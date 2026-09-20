//! M79 (Rust primitives half): tests for `crates/elisp/src/shell.rs`'s
//! `start-shell-process` / `shell-process-poll` / `shell-process-kill` /
//! `shell-process-live-p`.
//!
//! Covers:
//! - the pre-M79 two-argument call shape is byte-for-byte unchanged
//!   (`start_shell_process_two_args_unchanged`), since
//!   `crates/core/lisp/eshell.el` is the only existing caller and this
//!   milestone must not disturb it;
//! - the new optional STDIN argument, including the "nil still means
//!   EOF" case and a large (>1 MiB, bigger than a pipe buffer) payload
//!   round-tripping intact through `cat` without truncation or
//!   reordering;
//! - the new optional STREAMS argument (`merged` default vs
//!   `separate`), and that an invalid STREAMS value errors;
//! - non-zero exit codes, that `(exit . CODE)` is delivered exactly once
//!   (subsequent polls return nil, not a second exit event), and that a
//!   child which never reads a stdin payload bigger than a pipe buffer
//!   (so `write_all` is guaranteed to still have unwritten data when the
//!   child exits and closes its end) doesn't hang or panic the writer
//!   thread on EPIPE.
//!
//! All polling loops are bounded (a fixed retry count with a short sleep
//! between attempts) rather than infinite, per project convention: a
//! hung test should fail loudly, not hang the test binary.

use elisp::interp::Interp;
use elisp::printer::prin1_to_string;
use elisp::value::Value;
use std::time::Duration;

/// Run `src` to completion on a big-stack thread and return its printed
/// result (or "ERROR: ..." on a signal), same shape as the other
/// integration test files in this crate.
fn run(src: &str) -> String {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut interp = elisp::new_interp();
            match interp.eval_source(&src) {
                Ok(v) => prin1_to_string(&interp, &v),
                Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
            }
        })
        .expect("spawn failed")
        .join()
        .expect("eval thread panicked")
}

/// Like `run`, but keeps the interpreter alive across a closure so a
/// test can spawn a process, then poll it repeatedly against the same
/// bound `proc` variable. Runs on a big-stack thread for consistency
/// with the rest of the suite.
fn with_interp<F: FnOnce(&mut Interp) + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut interp = elisp::new_interp();
            f(&mut interp);
        })
        .expect("spawn failed")
        .join()
        .expect("test thread panicked");
}

fn eval(interp: &mut Interp, src: &str) -> Value {
    match interp.eval_source(src) {
        Ok(v) => v,
        Err(flow) => panic!("eval {:?} failed: {}", src, interp.describe_flow(&flow)),
    }
}

fn eval_str(interp: &mut Interp, src: &str) -> String {
    let v = eval(interp, src);
    prin1_to_string(interp, &v)
}

/// Poll `(shell-process-poll proc)` up to `max_polls` times (sleeping
/// briefly between attempts), returning every printed result seen
/// (skipping bare "nil"s) plus the loop's own diagnostic on timeout.
/// Bounded so a regression that hangs the process shows up as a test
/// failure, not a wedged test binary.
fn poll_until_exit(interp: &mut Interp, max_polls: u32) -> Vec<String> {
    let mut results = Vec::new();
    for _ in 0..max_polls {
        let s = eval_str(interp, "(shell-process-poll proc)");
        if s != "nil" {
            let is_exit = s.starts_with("(exit .");
            results.push(s);
            if is_exit {
                return results;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "shell-process-poll did not report exit within {} polls; results so far: {:?}",
        max_polls, results
    );
}

// ---------------------------------------------------------------------
// 1. Two-argument call shape (the pre-M79, still-only-real-caller shape)
// ---------------------------------------------------------------------

#[test]
fn start_shell_process_two_args_unchanged() {
    with_interp(|i| {
        eval(i, r#"(setq proc (start-shell-process "echo hi" "."))"#);
        let results = poll_until_exit(i, 200);
        // Bare string, not a cons -- merged mode is still the default
        // and still returns raw strings, exactly as before M79.
        assert_eq!(
            results,
            vec!["\"hi\\n\"".to_string(), "(exit . 0)".to_string()]
        );
    });
}

// ---------------------------------------------------------------------
// 2. STDIN argument
// ---------------------------------------------------------------------

#[test]
fn stdin_string_is_delivered() {
    with_interp(|i| {
        eval(i, r#"(setq proc (start-shell-process "cat" "." "hello"))"#);
        let results = poll_until_exit(i, 200);
        assert_eq!(
            results,
            vec!["\"hello\"".to_string(), "(exit . 0)".to_string()]
        );
    });
}

#[test]
fn stdin_nil_still_means_eof() {
    with_interp(|i| {
        eval(i, r#"(setq proc (start-shell-process "cat" "." nil))"#);
        let results = poll_until_exit(i, 200);
        // No output chunk at all: cat on a closed stdin produces
        // nothing before exiting, same as the pre-M79 Stdio::null().
        assert_eq!(results, vec!["(exit . 0)".to_string()]);
    });
}

/// Data-integrity regression test for the STDIN argument at a size well
/// past a single OS pipe buffer (~64 KiB): the full payload must arrive
/// at `cat`'s stdout intact -- no truncation, no reordering, no lost
/// bytes -- which requires the writer thread to keep feeding `write_all`
/// across however many partial writes the pipe forces, while the reader
/// threads drain stdout concurrently so neither side stalls the other.
///
/// This is NOT a deadlock test: unlike `crates/core/src/remote.rs`'s
/// `run()` (where the same thread writes stdin and then drains stdout,
/// so a big write truly can deadlock against a full stdout pipe), this
/// module's `spawn()` starts the reader threads BEFORE the stdin writer
/// (see the doc comments on `spawn()` and the file header in
/// `crates/elisp/src/shell.rs`), so they're already independently
/// draining output by the time the writer runs -- a synchronous write on
/// the calling thread would just be slow here, not stuck. Reviewer
/// finding (M79 tail review, R1): an earlier version of this test made
/// the deadlock claim in its name/doc without that being what the
/// assertions actually covered.
#[test]
fn large_stdin_payload_round_trips_intact() {
    with_interp(|i| {
        // Comfortably past any plausible pipe buffer: ~1.2 MiB.
        let payload: String = "0123456789\n".repeat(120_000);
        let id = i.intern("test-payload");
        i.set_sym_value(id, Value::string(payload.clone()));
        eval(
            i,
            r#"(setq proc (start-shell-process "cat" "." test-payload))"#,
        );

        let start = std::time::Instant::now();
        let mut got = String::new();
        let mut saw_exit = false;
        for _ in 0..1000 {
            let v = eval(i, "(shell-process-poll proc)");
            match &v {
                Value::Str(s) => got.push_str(s),
                Value::Cons(_) => {
                    // Either (stdout . STR) in a hypothetical future
                    // default, or (exit . CODE) here since this test
                    // uses the default merged mode -- only exit is a
                    // cons in that mode.
                    let printed = prin1_to_string(i, &v);
                    assert!(
                        printed.starts_with("(exit ."),
                        "unexpected cons in merged mode: {}",
                        printed
                    );
                    saw_exit = true;
                    break;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(saw_exit, "did not see exit within the poll budget");
        assert_eq!(got.len(), payload.len(), "payload size mismatch");
        assert_eq!(got, payload);
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "large stdin payload took {:?}, should be well under 10s -- a \
             regression back to a synchronous stdin write on the calling \
             thread would hang this instead",
            start.elapsed()
        );
    });
}

// ---------------------------------------------------------------------
// 3. STREAMS argument
// ---------------------------------------------------------------------

#[test]
fn separate_mode_splits_stdout_and_stderr() {
    with_interp(|i| {
        eval(
            i,
            r#"(setq proc (start-shell-process "echo out; echo err >&2" "." nil 'separate))"#,
        );
        let mut stdout_acc = String::new();
        let mut stderr_acc = String::new();
        let mut saw_exit = false;
        for _ in 0..200 {
            let v = eval(i, "(shell-process-poll proc)");
            match &v {
                Value::Nil => {}
                Value::Cons(_) => {
                    let printed = prin1_to_string(i, &v);
                    if printed.starts_with("(stdout . ") {
                        // Strip the tag + quoting to get at the raw text
                        // is unnecessary here; substring containment on
                        // the printed form is enough to prove the tag
                        // and content landed correctly.
                        stdout_acc.push_str(&printed);
                    } else if printed.starts_with("(stderr . ") {
                        stderr_acc.push_str(&printed);
                    } else if printed.starts_with("(exit . ") {
                        saw_exit = true;
                        break;
                    } else {
                        panic!("unexpected poll result: {}", printed);
                    }
                }
                other => panic!(
                    "unexpected non-cons, non-nil poll result: {}",
                    prin1_to_string(i, other)
                ),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(saw_exit, "did not see exit within the poll budget");
        assert!(
            stdout_acc.contains("out"),
            "stdout accumulator missing 'out': {}",
            stdout_acc
        );
        assert!(
            stderr_acc.contains("err"),
            "stderr accumulator missing 'err': {}",
            stderr_acc
        );
        // Not mixed into the same cons: stdout's accumulator never saw
        // the literal text "err" glued onto the same cons cell as "out"
        // (they're two entirely separate strings above), and vice
        // versa.
        assert!(!stdout_acc.contains("err"), "stdout leaked stderr text");
        assert!(!stderr_acc.contains("out"), "stderr leaked stdout text");
    });
}

#[test]
fn merged_default_and_explicit_merged_symbol_agree() {
    with_interp(|i| {
        eval(i, r#"(setq proc1 (start-shell-process "echo hi" "."))"#);
        eval(
            i,
            r#"(setq proc2 (start-shell-process "echo hi" "." nil 'merged))"#,
        );
        eval(i, "(setq proc proc1)");
        let default_results = poll_until_exit(i, 200);
        eval(i, "(setq proc proc2)");
        let explicit_results = poll_until_exit(i, 200);
        assert_eq!(default_results, explicit_results);
        assert_eq!(
            default_results,
            vec!["\"hi\\n\"".to_string(), "(exit . 0)".to_string()]
        );
    });
}

/// M141: `Merged` mode must preserve the child's write order. Before the
/// fix, stdout and stderr each got their own pipe and reader thread, and
/// the two threads raced to push onto one channel -- nothing serialized
/// them against each other, so interleaving was arrival order, not write
/// order. This asserts the exact expected sequence for N alternating
/// writes; see the milestone report for the pre-fix failure counts.
#[test]
fn merged_mode_preserves_write_order() {
    with_interp(|i| {
        eval(
            i,
            r#"(setq proc (start-shell-process "for i in $(seq 1 300); do echo o$i; echo e$i >&2; done" "."))"#,
        );
        let mut got = String::new();
        let mut saw_exit = false;
        // A deadline, not a fixed iteration count: under the full gate's
        // parallel load a 600-line shell loop can outlast a few seconds
        // (M135's false-red family), and running out still fails loudly
        // below.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let v = eval(i, "(shell-process-poll proc)");
            match &v {
                Value::Str(s) => got.push_str(s),
                Value::Cons(_) => {
                    let printed = prin1_to_string(i, &v);
                    assert!(
                        printed.starts_with("(exit ."),
                        "unexpected cons in merged mode: {}",
                        printed
                    );
                    saw_exit = true;
                    break;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(saw_exit, "did not see exit within the poll budget");

        let mut expected = String::new();
        for n in 1..=300 {
            expected.push_str(&format!("o{}\n", n));
            expected.push_str(&format!("e{}\n", n));
        }
        if got != expected {
            let got_lines: Vec<&str> = got.lines().collect();
            let expected_lines: Vec<&str> = expected.lines().collect();
            let mismatch = got_lines
                .iter()
                .zip(expected_lines.iter())
                .position(|(a, b)| a != b);
            let idx = mismatch.unwrap_or(got_lines.len().min(expected_lines.len()));
            let lo = idx.saturating_sub(3);
            let hi_got = (idx + 3).min(got_lines.len());
            let hi_exp = (idx + 3).min(expected_lines.len());
            panic!(
                "output out of order at line {}: got {:?}, expected {:?}",
                idx,
                &got_lines[lo..hi_got],
                &expected_lines[lo..hi_exp]
            );
        }
    });
}

/// M141: `Separate` mode must NOT be affected by the single-pipe change
/// to `Merged` mode -- stdout and stderr stay on their own pipes, so each
/// accumulator must equal exactly its own lines in order.
#[test]
fn separate_mode_keeps_streams_independent_and_ordered() {
    with_interp(|i| {
        eval(
            i,
            r#"(setq proc (start-shell-process "for i in $(seq 1 300); do echo o$i; echo e$i >&2; done" "." nil 'separate))"#,
        );
        let mut stdout_acc = String::new();
        let mut stderr_acc = String::new();
        let mut saw_exit = false;
        // Same deadline as `merged_mode_preserves_write_order`, for the
        // same 600-line loop under the full gate's load.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let v = eval(i, "(shell-process-poll proc)");
            match &v {
                Value::Nil => {}
                Value::Cons(_) => {
                    let tag = v.car();
                    let val = v.cdr();
                    let tag_name = match &tag {
                        Value::Sym(s) => i.sym_name(*s).to_string(),
                        _ => panic!("unexpected tag shape"),
                    };
                    match tag_name.as_str() {
                        "stdout" => {
                            if let Value::Str(s) = &val {
                                stdout_acc.push_str(s);
                            }
                        }
                        "stderr" => {
                            if let Value::Str(s) = &val {
                                stderr_acc.push_str(s);
                            }
                        }
                        "exit" => {
                            saw_exit = true;
                        }
                        other => panic!("unexpected tag in poll result: {}", other),
                    }
                    if saw_exit {
                        break;
                    }
                }
                other => panic!(
                    "unexpected non-cons, non-nil poll result: {}",
                    prin1_to_string(i, other)
                ),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(saw_exit, "did not see exit within the poll budget");

        let mut expected_stdout = String::new();
        let mut expected_stderr = String::new();
        for n in 1..=300 {
            expected_stdout.push_str(&format!("o{}\n", n));
            expected_stderr.push_str(&format!("e{}\n", n));
        }
        assert_eq!(stdout_acc, expected_stdout);
        assert_eq!(stderr_acc, expected_stderr);
    });
}

/// M141: the single-pipe change must not break EOF accounting when the
/// stdin writer thread is also present -- `cat` echoes stdin back on
/// stdout while a stderr write happens too, and the process must still
/// report exit exactly once with all output intact.
#[test]
fn merged_mode_with_stdin_reports_exit_and_full_output() {
    with_interp(|i| {
        let payload = "line one\nline two\nline three\n".to_string();
        let id = i.intern("test-payload");
        i.set_sym_value(id, Value::string(payload.clone()));
        eval(
            i,
            r#"(setq proc (start-shell-process "cat; echo done >&2" "." test-payload))"#,
        );
        // Polled directly rather than through `poll_until_exit`, so the
        // chunks are collected as raw strings: `cat` and `echo done` are two
        // separate writes, a poll can land between them, and joining
        // prin1-printed chunks would splice `""` into the text.
        //
        // What this guards is EOF accounting when the stdin writer thread
        // exists alongside the single pipe: all output arrives and exit is
        // reported. It is NOT an ordering test -- `cat; echo done` runs its
        // two writes one after the other, so they could not race even on
        // two pipes. `merged_mode_preserves_write_order` is the ordering
        // test. Exit-exactly-once past the first exit is
        // `exit_reported_exactly_once`'s job.
        let mut got = String::new();
        let mut exit = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let v = eval(i, "(shell-process-poll proc)");
            match &v {
                Value::Str(s) => got.push_str(s),
                Value::Cons(_) => {
                    exit = Some(prin1_to_string(i, &v));
                    break;
                }
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            exit.as_deref(),
            Some("(exit . 0)"),
            "no exit within the poll budget; output so far: {:?}",
            got
        );
        assert_eq!(got, "line one\nline two\nline three\ndone\n");
    });
}

#[test]
fn invalid_streams_value_errors() {
    let out = run(r#"(start-shell-process "echo hi" "." nil 'nonsense)"#);
    assert!(out.starts_with("ERROR"), "expected an error, got: {}", out);
}

// ---------------------------------------------------------------------
// 4. Exit codes
// ---------------------------------------------------------------------

#[test]
fn nonzero_exit_code_is_reported() {
    with_interp(|i| {
        eval(i, r#"(setq proc (start-shell-process "exit 3" "."))"#);
        let results = poll_until_exit(i, 200);
        assert_eq!(results, vec!["(exit . 3)".to_string()]);
    });
}

// ---------------------------------------------------------------------
// 5. Writer thread must not panic/hang on EPIPE
// ---------------------------------------------------------------------

/// `true` exits immediately without ever reading stdin. The payload here
/// is deliberately well past a single OS pipe buffer (~64 KiB is the
/// common size) so `write_all` is guaranteed to still have unwritten
/// data queued when `true` exits and the kernel tears down its end of
/// the pipe -- i.e. this test actually forces the EPIPE path, unlike an
/// earlier version of this test whose few-byte payload could complete
/// `write_all` before `true` even got scheduled, never touching EPIPE at
/// all (M79 tail review, R3). Either way (EPIPE hit or, in a race, not)
/// this must not panic or hang the poll loop.
#[test]
fn stdin_given_but_unread_does_not_hang_or_panic() {
    with_interp(|i| {
        // ~2 MiB: comfortably past any plausible pipe buffer size.
        let payload: String = "x".repeat(2 * 1024 * 1024);
        let id = i.intern("test-payload");
        i.set_sym_value(id, Value::string(payload));
        eval(
            i,
            r#"(setq proc (start-shell-process "true" "." test-payload))"#,
        );
        let results = poll_until_exit(i, 200);
        assert_eq!(results, vec!["(exit . 0)".to_string()]);
    });
}

// ---------------------------------------------------------------------
// 6. `(exit . CODE)` is delivered exactly once
// ---------------------------------------------------------------------

/// The `exit_reported` guard in `ShellProc::poll()` (`crates/elisp/src/
/// shell.rs`) is meant to ensure `(exit . CODE)` crosses into elisp
/// exactly once. Nothing else in this suite exercises polling PAST that
/// point -- `poll_until_exit` stops as soon as it sees the exit cons, and
/// the (not-yet-written) M79 elisp pump is expected to do the same -- so
/// without this test, deleting that guard would not turn any existing
/// test red (M79 tail review, R4).
#[test]
fn exit_reported_exactly_once() {
    with_interp(|i| {
        eval(i, r#"(setq proc (start-shell-process "exit 3" "."))"#);
        let first = poll_until_exit(i, 200);
        assert_eq!(first, vec!["(exit . 3)".to_string()]);
        // Poll several more times: no second exit event, no panic, and
        // every result is plain nil.
        for _ in 0..10 {
            let s = eval_str(i, "(shell-process-poll proc)");
            assert_eq!(s, "nil", "expected nil after the exit was already reported");
        }
    });
}
