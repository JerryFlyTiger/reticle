//! M60: TUI screen ownership — nothing may bypass the grid and write
//! straight to the terminal. `elisp::bglog` is the collector these
//! writers route through instead; `core::idle_tick` drains it into
//! `*background-output*`. See `crates/elisp/src/bglog.rs`'s module doc
//! for the motivating bug (an LSP server's stderr banner permanently
//! corrupting the TUI screen).
//!
//! Known coverage gaps, documented rather than faked (project convention:
//! known gaps get written down, not silently left as untested code):
//!
//! - The `highlight.rs` `eprintln!` → `bglog::push` conversion (M60 item
//!   3) has no test here. Triggering it requires making tree-sitter's
//!   `Parser::set_language` or `Query::new` fail for one of the nine
//!   vendored grammars/queries — there's no test seam to inject that
//!   failure (the query text is a compile-time `include_str!` and the
//!   grammar comes from a linked crate), and deliberately shipping a
//!   broken query file just to hit this path would violate the
//!   "vendored queries are hand-maintained, audited files" invariant
//!   `highlight.rs`'s own tests rely on elsewhere. The collector itself
//!   (`bglog::push`/`drain`) is exercised directly by
//!   `crates/elisp/src/bglog.rs`'s own unit tests and by the worker/LSP
//!   tests below, which is the part of that code path this crate can
//!   actually reach.
//! - `browse-url`'s `Stdio::null()` fix (`crates/core/src/builtins/
//!   ui.rs`) has no test here either: no test in this crate calls out to
//!   the system `open` command (platform-specific, has side effects
//!   outside the process, and nothing about its *output* is meant to be
//!   observed — the fix is just "don't let its stdio leak").
//! - `crates/elisp/src/jit.rs`'s four `JIT_DEBUG`-gated `eprintln!`s are
//!   the same category of "writes straight to the terminal" as
//!   everything else this milestone routed through `bglog`, but M60
//!   deliberately leaves them alone: they only fire when a user has
//!   explicitly set `JIT_DEBUG` in their environment to watch JIT trace
//!   output live in a terminal, which is the entire point of that output
//!   existing — routing it into a buffer instead would defeat the
//!   feature, not just relocate it. Not an oversight; a scope boundary.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

// `elisp::bglog` is a process-global `static`, not per-Interp state (see
// its module doc), and `cargo test` runs tests in this file concurrently
// by default. Without serialization, two tests polling/draining it at
// once would steal each other's lines -- this guard is grabbed at the
// top of every test that touches bglog, cargo's own per-binary test
// parallelism handles the rest (only tests in *this* file need it: they
// share nothing with other test binaries' bglog state at the process
// level since each `cargo test` integration test file is its own
// process).
static BGLOG_TEST_LOCK: Mutex<()> = Mutex::new(());

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

/// `worker-start` (see `elisp::worker::WorkerHandle::spawn`) normally
/// re-execs `current_exe()` in `--worker` mode -- correct when the
/// *running* process is `reticle` itself, but wrong for a `cargo
/// test` binary (there's no `--worker` mode for a test harness to enter,
/// and `current_exe()` inside a test resolves to the test binary, not
/// `reticle`). Points `RETICLE_WORKER_EXE` (see
/// `WorkerHandle::spawn`'s M60 testability note) at the real
/// already-built `reticle` binary so
/// `worker_stderr_is_captured_not_inherited` can spawn a real worker.
///
/// Deliberately does NOT invoke `cargo build` itself -- same shape as
/// `crates/elisp/tests/module_tests.rs`'s `demo_module_path()`, which
/// faces the identical "test needs a cargo-built artifact" problem and
/// resolves it by checking for the artifact and panicking with build
/// instructions rather than shelling out to cargo from inside a test:
/// a nested `cargo build` leaks its own stdout past the test harness's
/// capture, and silently ignores whatever profile/flags (`--release`,
/// `--offline`, a non-default `CARGO_TARGET_DIR`, ...) the *outer* `cargo
/// test` invocation was run with. `cargo test --workspace` already
/// builds the `reticle` bin target as a normal dependency of the
/// build graph, so this doesn't make the ordinary path any more manual
/// than `demo_module_path()` already is.
///
/// Caller must hold `BGLOG_TEST_LOCK` for the duration this env var
/// stays set (i.e. for the rest of the calling test): `set_var` mutating
/// process environment while another thread might read it (e.g. a
/// sibling test in this same binary spawning a child process, which
/// reads the whole environment) is the kind of thing that's only safe
/// because every test in this file takes `BGLOG_TEST_LOCK` before doing
/// anything else, so no two tests in this process ever run concurrently.
///
/// Known gap, left as-is (M60 review, round 3): a *scoped* invocation
/// like `cargo test -p core --test background_output_tests` does not
/// rebuild the root package's `reticle` bin (verified by mtime: it
/// is not touched by that command), so if a stale binary is already
/// sitting in `target/{debug,release}` this can hand `worker-start` a
/// build that predates the very worker.rs changes under test, and the
/// test would pass against the wrong binary rather than failing loudly.
/// This is the same class of pre-existing gap `crates/elisp/tests/
/// module_tests.rs`'s `demo_module_path()` already has for
/// `demo-module`; not something introduced here.
fn worker_exe_path() -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .expect("crates/core/Cargo.toml should be two levels under the workspace root");
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    // Prefer whatever profile *this test binary itself* was compiled
    // with before falling back to the other one -- otherwise, if both a
    // fresh `target/release/reticle` and a stale `target/debug/
    // reticle` happen to exist, `cargo test --release` would silently
    // pick up the stale debug binary (fixed profile order used to always
    // try "debug" first regardless of how the test was built).
    let profiles: [&str; 2] = if cfg!(debug_assertions) {
        ["debug", "release"]
    } else {
        ["release", "debug"]
    };
    for profile in profiles {
        let p = target_dir.join(profile).join("reticle");
        if p.is_file() {
            return p;
        }
    }
    panic!(
        "reticle binary not found in target/{{debug,release}}; \
         run `cargo build --bin reticle` first"
    );
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// Poll `elisp::bglog::drain()` (which empties the collector each call)
/// until some accumulated line matches `pred`, or `timeout` elapses.
/// Background threads (worker/LSP stderr readers) are asynchronous, so
/// tests can't assume a single drain sees the data.
fn wait_for_bglog_line(pred: impl Fn(&str) -> bool, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        for line in elisp::bglog::drain() {
            if pred(&line) {
                return true;
            }
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn worker_stderr_is_captured_not_inherited() {
    // Drain any leftovers from another test sharing this process-global
    // collector (elisp::bglog is a `static`, not per-Interp state).
    let _guard = BGLOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Safe to mutate process env here: see `worker_exe_path`'s doc for
    // why holding `BGLOG_TEST_LOCK` (acquired above, held for this
    // entire test) is what makes this not a data race with sibling
    // tests in this binary.
    std::env::set_var("RETICLE_WORKER_EXE", worker_exe_path());
    let _ = elisp::bglog::drain();

    let (mut i, _ed) = setup();
    run(&mut i, "(setq w (worker-start))");
    // worker.rs's `run_worker_loop` redirects the worker's own
    // `interp.output` to eprint! (its stdout is the framed-sexp
    // protocol channel), so a `message` call in the worker process ends
    // up on the worker's stderr, which M60 pipes into bglog under the
    // "worker" tag.
    run(&mut i, "(worker-eval w '(message \"m60-worker-marker\"))");
    // Drive completion so the worker's request is actually processed
    // (worker-eval is fire-and-forget; the response itself is irrelevant
    // here, only the stderr side effect is under test).
    run(&mut i, "(worker-wait w)");

    let found = wait_for_bglog_line(
        |l| l.contains("m60-worker-marker"),
        std::time::Duration::from_secs(5),
    );
    assert!(found, "expected the worker's stderr message to reach bglog");
    run(&mut i, "(worker-kill w)");
}

#[test]
fn lsp_server_stderr_is_captured_not_inherited() {
    let _guard = BGLOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = elisp::bglog::drain();

    let (mut i, _ed) = setup();
    // A fake "server": writes one line to stderr, then sleeps briefly so
    // the reader thread has time to observe it before the process exits.
    // No real LSP handshake attempted -- only the stderr-piping path is
    // under test (see M60's directive not to depend on `initialize`
    // succeeding).
    run(
        &mut i,
        r#"(setq conn (lsp-start "sh" (list "-c" "echo m60-lsp-marker 1>&2; sleep 0.3")))"#,
    );
    assert!(
        !run(&mut i, "conn").starts_with("ERROR"),
        "lsp-start (sh -c ...) should always succeed: sh is assumed present"
    );

    let found = wait_for_bglog_line(
        |l| l.contains("m60-lsp-marker"),
        std::time::Duration::from_secs(5),
    );
    assert!(
        found,
        "expected the LSP server's stderr line to reach bglog"
    );
}

#[test]
fn idle_tick_drains_into_background_output_buffer_and_notifies_once() {
    let _guard = BGLOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = elisp::bglog::drain();

    let (mut i, ed) = setup();
    assert_eq!(run(&mut i, "(get-buffer \"*background-output*\")"), "nil");

    elisp::bglog::push("t", "first batch line");
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    assert_ne!(run(&mut i, "(get-buffer \"*background-output*\")"), "nil");
    let text = run(
        &mut i,
        "(with-current-buffer \"*background-output*\" (buffer-string))",
    );
    assert!(
        text.contains("[t] first batch line"),
        "buffer text was: {text}"
    );
    let echo_after_first = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo_after_first.contains("*background-output*"),
        "expected the one-time notice, got: {echo_after_first}"
    );

    // Clear the echo area to something else, then push + tick again: the
    // buffer should grow, but the notice must NOT fire a second time.
    run(&mut i, "(message \"unrelated\")");
    elisp::bglog::push("t", "second batch line");
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    let text2 = run(
        &mut i,
        "(with-current-buffer \"*background-output*\" (buffer-string))",
    );
    assert!(text2.contains("first batch line"));
    assert!(text2.contains("second batch line"));
    let echo_after_second = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(
        echo_after_second, "unrelated",
        "second drain must not re-fire the one-time notice"
    );
}

#[test]
fn background_output_buffer_is_bounded_and_keeps_the_tail() {
    let _guard = BGLOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = elisp::bglog::drain();

    let (mut i, ed) = setup();

    // `elisp::bglog` itself caps at 500 lines per push()-then-drain()
    // cycle, so getting past `core::MAX_BACKGROUND_OUTPUT_LINES`
    // requires many idle ticks -- exactly the "long session, continuous
    // background chatter" shape this cap exists for (see the comment at
    // `core::MAX_BACKGROUND_OUTPUT_LINES`'s definition).
    const BATCH_SIZE: usize = 400;
    const BATCHES: usize = 14; // 14 * 400 = 5600 lines pushed in total
    let total_pushed = BATCHES * BATCH_SIZE;
    let cap = core::MAX_BACKGROUND_OUTPUT_LINES;
    assert!(
        total_pushed > cap,
        "test setup bug: this test needs to push past the cap ({total_pushed} <= {cap})"
    );

    for batch in 0..BATCHES {
        for n in 0..BATCH_SIZE {
            elisp::bglog::push("t", &format!("line-{batch}-{n}"));
        }
        core::idle_tick(&mut i, std::time::Duration::ZERO);
    }

    let buf = ed
        .borrow()
        .find_buffer("*background-output*")
        .expect("buffer should exist after the first tick");
    let text = buf.borrow().search_text();

    // Exact count, not just an upper bound: an "excess" or "cut point"
    // calculation that trims MORE than it should (e.g. an off-by-N in
    // how many lines are dropped) would still satisfy `line_count <=
    // cap` and still leave the tail line intact, so a `<=` check alone
    // cannot tell "trimmed correctly" from "over-trimmed" apart. Since
    // every batch is smaller than `cap`, the buffer is always a
    // contiguous suffix of everything ever pushed and settles at
    // exactly `cap` lines once `total_pushed > cap`.
    let line_count = text.matches('\n').count();
    assert_eq!(
        line_count, cap,
        "expected the buffer to settle at exactly the cap, not more or fewer"
    );

    // Which absolute line (0-indexed over the whole push sequence) is
    // the oldest survivor, worked out independently of the trimming
    // code under test: with every batch smaller than the cap, the final
    // buffer is exactly the last `cap` of the `total_pushed` lines ever
    // pushed, i.e. absolute lines [total_pushed - cap, total_pushed).
    let first_surviving_index = total_pushed - cap;
    let first_batch = first_surviving_index / BATCH_SIZE;
    let first_n = first_surviving_index % BATCH_SIZE;
    let first_line = format!("[t] line-{first_batch}-{first_n}\n");
    let last_line = format!("[t] line-{}-{}\n", BATCHES - 1, BATCH_SIZE - 1);

    // Boundary correctness, not just presence: a cut point that's off
    // by even one character (e.g. eating the first char of the next
    // line, or leaving one char of the trimmed line behind) would still
    // let a `contains(...)` substring check pass on a mangled line, so
    // this checks the survivor appears as a COMPLETE, cleanly-bounded
    // line -- the buffer's first bytes are exactly this line, not a
    // fragment of it.
    assert!(
        text.starts_with(&first_line),
        "buffer should start with the complete line {first_line:?}; \
         actual buffer head: {:?}",
        &text[..text.len().min(60)]
    );
    assert!(
        text.ends_with(&last_line),
        "buffer should end with the complete most-recent line {last_line:?}; \
         actual buffer tail: {:?}",
        &text[text.len().saturating_sub(60)..]
    );
}

#[test]
fn background_log_drain_primitive_works_without_an_editor() {
    let _guard = BGLOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = elisp::bglog::drain();
    // Batch-mode ("--repl"/"--eval"/"--script") has no editor, so
    // idle_tick never runs; `background-log-drain` is the escape hatch.
    let mut interp = elisp::new_interp(); // no core::init_editor
    elisp::bglog::push("batch", "hello from batch mode");
    let out = run(&mut interp, "(background-log-drain)");
    assert!(out.contains("hello from batch mode"), "got: {out}");
    // Drained: a second call sees nothing left.
    assert_eq!(run(&mut interp, "(background-log-drain)"), "nil");
}
