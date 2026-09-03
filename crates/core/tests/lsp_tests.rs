//! LSP client (M14): drives a real `rust-analyzer` process through the
//! elisp-level protocol client in `crates/core/lisp/lsp.el`, over the
//! low-level transport in `crates/elisp/src/lsp.rs`.
//!
//! Skips (rather than fails) if `rust-analyzer` isn't on PATH, since a
//! machine without it shouldn't fail the whole suite over an optional
//! external tool -- everything else in this workspace has zero such
//! dependencies.

use elisp::printer::prin1_to_string;
use elisp::Interp;

fn have_rust_analyzer() -> bool {
    std::process::Command::new("rust-analyzer")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn setup() -> Interp {
    let mut interp = elisp::new_interp();
    core::init_editor(&mut interp);
    interp
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_lsp_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A tiny real cargo package so rust-analyzer has workspace context to
/// answer hover/definition without needing to index anything but itself.
fn write_scratch_project(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"lsp_smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let result = add(1, 2);\n    println!(\"{}\", result);\n}\n",
    )
    .unwrap();
    dir
}

/// Writes an executable shell script into DIR and returns its path as a
/// string, for M65's "deaf"/"chatty" fake-server tests: real `sh`
/// processes, not mocks, so the bounded-wait code under test talks to an
/// actual child process over an actual pipe, same as it would with a
/// real language server.
fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    path.to_str().unwrap().to_string()
}

/// A single valid Content-Length-framed `initialize` response (id 1,
/// empty result), then keeps stdin open forever via `cat` so the process
/// stays alive (and `lsp-live-p` stays `t`) instead of exiting and
/// muddying a "did the timeout logic fire, not a death" assertion.
const ECHO_ONE_SCRIPT: &str = "#!/bin/sh
msg='{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'
len=$(printf '%s' \"$msg\" | wc -c)
printf 'Content-Length: %d\\r\\n\\r\\n%s' \"$len\" \"$msg\"
cat >/dev/null
";

/// Never answers anything, but keeps sending well-formed
/// `$/progress`-shaped notifications every 50ms forever -- the "chatty
/// server" M65 asks for: a naive per-message-reset of the wait budget
/// would never time out against this, since a new message always
/// arrives before any fixed per-call timeout elapses.
// A server that talks constantly but never answers anything: an endless
// stream of well-formed `$/progress' notifications. What it exists to
// prove is that `lsp--await''s budget is a TOTAL deadline, not a
// per-receive one -- a talkative server must not be able to postpone
// the timeout forever just by saying something before each slice runs
// out.
//
// The message and its Content-Length are computed ONCE, outside the
// loop, on purpose. An earlier version recomputed `len' with `wc -c'
// every iteration, forking two processes per message (`wc' plus the
// command substitution's subshell) on top of `sleep''s own fork. That
// made the send rate load-dependent, and the mutation this test is the
// observation point for (make the budget reset every iteration ->
// `lsp--await' should then never return) only hangs while the server
// keeps beating the budget: under load a single iteration could exceed
// the test's timeout, the loop would take its ordinary timeout exit,
// and the test PASSED WITH THE DEFENCE REMOVED. That is exactly the
// false-negative shape mutation testing is supposed to catch, and it
// showed up as a mutation that survived in a full battery run while
// hanging (correctly) when run alone -- the difference was machine load,
// not the code under test. Fixed on both sides: no per-message forks
// here, and a test timeout (below) two orders of magnitude larger than
// the interval, so a gap would have to be ~40x the nominal one to break
// the observation.
const CHATTY_SCRIPT: &str = "#!/bin/sh
msg='{\"jsonrpc\":\"2.0\",\"method\":\"$/progress\",\"params\":{}}'
len=${#msg}
while true; do
  printf 'Content-Length: %d\\r\\n\\r\\n%s' \"$len\" \"$msg\"
  sleep 0.05
done
";

/// A "deaf" server: reads stdin to EOF forever, discarding it, and
/// never writes anything back -- the "connects fine, replies to
/// nothing" shape M65 is about.
///
/// M65 review round 2: this used to also `echo $$` its own PID to a
/// file the test polled for, so tests could independently confirm via
/// `kill -0` that the process was really gone (not just this crate's
/// own `alive` bookkeeping). That PID FILE was itself a race: under the
/// default (parallel) test runner, a starved shell can be SIGKILLed
/// before it ever reaches its first line, so the file legitimately
/// never appears no matter how long a test polls for it -- confirmed by
/// running this suite with the default thread count repeatedly, which
/// failed the two tests depending on that file on all but one of 12
/// runs. `lsp-connection-pid` (`crates/elisp/src/lsp.rs`) replaces it:
/// the PID comes straight from `Child::id()`, recorded at spawn time on
/// the Rust side, with no dependency on the child ever being scheduled.
const DEAF_SCRIPT: &str = "#!/bin/sh
cat >/dev/null
";

fn write_deaf_script(dir: &std::path::Path) -> String {
    write_script(dir, "deaf.sh", DEAF_SCRIPT)
}

/// Never touches stdin at all -- unlike `DEAF_SCRIPT`, closing its
/// stdin (what `Drop for LspConnection` does before `child.wait()`) has
/// NO effect on this process; only an actual signal (`kill()`) stops it.
///
/// M65 review round 3: `lsp_connection_killed_purely_by_drop_when_
/// unreachable` used `DEAF_SCRIPT` (`cat >/dev/null`), and a mutation
/// check (deleting `let _ = self.child.kill();` from `Drop for
/// LspConnection`, keeping only `self.stdin = None; self.child.wait();`)
/// left that test GREEN -- because `cat` exits on its own the moment
/// its stdin hits EOF, which `self.stdin = None` alone already causes.
/// The test was therefore observing "did dropping the stdin handle
/// happen", not "did `kill()` get called" -- exactly the line the
/// reviewer flagged as the highest-risk, most test-worthy one. This
/// script closes that gap: with `kill()` removed, `child.wait()` in the
/// destructor blocks forever against a process that will never exit on
/// its own, which is the actual failure mode M65 exists to prevent (a
/// server that ignores stdin closing).
const IGNORES_STDIN_SCRIPT: &str = "#!/bin/sh
while true; do
  sleep 1
done
";

/// Evaluates `(lsp-connection-pid EXPR)` and parses the result as a
/// `u32`. EXPR is any elisp expression evaluating to a connection
/// (typically a bound variable, e.g. `"conn"`).
fn conn_pid(i: &mut Interp, expr: &str) -> u32 {
    let r = run(i, &format!("(lsp-connection-pid {})", expr));
    r.parse().unwrap_or_else(|_| {
        panic!(
            "(lsp-connection-pid {}) did not return an integer: {}",
            expr, r
        )
    })
}

/// OS-level (not this crate's own bookkeeping) check that PID is no
/// longer a live process, polling `kill -0` for up to TIMEOUT rather
/// than a single fixed `sleep` -- `kill()` + `wait()` reaping isn't
/// instantaneous, but it should be fast, and a poll loop proves that
/// without baking in an arbitrary sleep length as "the" answer.
/// Whether PID names a live process right now.
///
/// M65 tail review: every `wait_until_pid_dead` assertion below is
/// one-sided -- it only ever checks that a pid STOPS being alive. A
/// `lsp-connection-pid` that returned a wrong-but-plausible number (say,
/// the real pid + 1) would sail through all of them, because an
/// unrelated pid is usually not a live process either, so "it's dead"
/// is true from the first poll. Asserting the pid is alive BEFORE the
/// kill is what makes those tests actually depend on `pid()` returning
/// the right value.
fn pid_is_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wait_until_pid_dead(pid: u32, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !alive {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn lsp_wait_timeout_returns_nil_without_message_and_stays_alive() {
    let dir = Scratch::new("lsp_wait_nil");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_deaf_script(&dir);

    let mut i = setup();
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    let pid = conn_pid(&mut i, "conn");
    // Nothing was ever sent, so a bounded wait must return nil, not hang
    // and not report the (still-alive) server as dead.
    assert_eq!(run(&mut i, "(lsp-wait conn 0.2)"), "nil");
    assert_eq!(run(&mut i, "(lsp-live-p conn)"), "t");
    run(&mut i, "(lsp-kill conn)");
    assert_eq!(run(&mut i, "(lsp-live-p conn)"), "nil");
    assert!(
        wait_until_pid_dead(pid, std::time::Duration::from_secs(2)),
        "process {} should be dead (OS-level) after (lsp-kill conn), not just bookkeeping",
        pid
    );
}

#[test]
fn lsp_wait_returns_message_with_or_without_timeout_arg() {
    let dir = Scratch::new("lsp_wait_message");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_script(&dir, "echo_one.sh", ECHO_ONE_SCRIPT);

    // With an explicit TIMEOUT: a message that's already available (or
    // arrives well within the budget) is returned exactly as before --
    // TIMEOUT only bounds the ABSENCE of a message, never delays one
    // that's ready.
    let mut i = setup();
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    assert_eq!(run(&mut i, "(gethash \"id\" (lsp-wait conn 5))"), "1");
    run(&mut i, "(lsp-kill conn)");

    // With TIMEOUT omitted: unchanged pre-M65 behavior (unbounded
    // block), exercised here with a message that's ready immediately so
    // the test itself can't hang.
    let mut i2 = setup();
    let r = run(&mut i2, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    assert_eq!(run(&mut i2, "(gethash \"id\" (lsp-wait conn))"), "1");
    run(&mut i2, "(lsp-kill conn)");
}

#[test]
fn lsp_connect_deaf_server_times_out_and_kills_the_connection() {
    let dir = Scratch::new("lsp_connect_deaf");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_deaf_script(&dir);

    let mut i = setup();
    // Capture every connection `lsp-kill` is called on, forwarding to
    // the real primitive, so the test can assert `lsp-connect`'s M65
    // failure path actually kills the connection it started -- without
    // this, a failed `lsp-connect` gives the caller no handle back to
    // check.
    run(
        &mut i,
        "(defvar test--killed-conns nil)
         (fset 'lsp-kill-orig (symbol-function 'lsp-kill))
         (defun lsp-kill (c) (setq test--killed-conns (cons c test--killed-conns)) (lsp-kill-orig c))",
    );
    // M65 review round 2: `lsp-initialize-timeout` no longer needs to be
    // razor-thin -- the PID used for the OS-level death check below now
    // comes straight from `lsp-connection-pid` (`Child::id()`, recorded
    // at spawn time), not from the deaf script racing this timeout to
    // write a PID file before it might get killed. 0.3s is kept anyway
    // (rather than widened) simply so the test itself stays fast; it is
    // no longer load-bearing for correctness the way it was before.
    run(&mut i, "(setq lsp-initialize-timeout 0.3)");
    let r = run(
        &mut i,
        &format!(
            r#"(condition-case err
                   (progn (lsp-connect {:?} nil nil) "no-error")
                 (error (format "%S" err)))"#,
            script
        ),
    );
    assert!(
        r.to_lowercase().contains("timed out") || r.to_lowercase().contains("timeout"),
        "expected a timeout error, got: {}",
        r
    );
    assert_eq!(
        run(&mut i, "(length test--killed-conns)"),
        "1",
        "lsp-connect should kill exactly the one connection it started"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p (car test--killed-conns))"),
        "nil",
        "the killed connection should report dead"
    );
    // No half-built client left behind in the registry the idle tick
    // pumps every connection through.
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    // M65 review: `alive` above is this crate's own bookkeeping, and
    // `child.kill()`'s `Result` is discarded (`let _ =`) in
    // `LspConnection::kill` -- prove the OS agrees the process is
    // really gone, not just that we believe it. The pid comes from the
    // very connection object `lsp-kill` was actually called on (via the
    // monkey-patch above), fetched AFTER the kill -- `Child::id()`
    // stays valid regardless of whether the process is still alive.
    let pid = conn_pid(&mut i, "(car test--killed-conns)");
    assert!(
        wait_until_pid_dead(pid, std::time::Duration::from_secs(2)),
        "orphaned deaf server (pid {}) still running after lsp-connect's failure path",
        pid
    );
}

#[test]
fn lsp_command_deaf_server_leaves_no_stale_connections_entry() {
    let dir = Scratch::new("lsp_command_deaf");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let file = dir.join("src/scratch.sh");
    std::fs::write(&file, "#!/bin/sh\necho deaf-server-scratch\n").unwrap();
    let script = write_deaf_script(&dir);

    let mut i = setup();
    // Same monkey-patch as `lsp_connect_deaf_server_times_out_and_kills_
    // the_connection`: `(lsp)` never hands the connection object back to
    // the caller, so this is the only way to get a handle onto it for
    // `lsp-connection-pid` afterward.
    run(
        &mut i,
        "(defvar test--killed-conns nil)
         (fset 'lsp-kill-orig (symbol-function 'lsp-kill))
         (defun lsp-kill (c) (setq test--killed-conns (cons c test--killed-conns)) (lsp-kill-orig c))",
    );
    run(&mut i, "(setq lsp-initialize-timeout 0.3)");
    run(
        &mut i,
        &format!(
            r#"(setq lsp-server-alist (list (cons 'sh-mode (list {:?}))))"#,
            script
        ),
    );
    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", file.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);

    let msg = run(&mut i, "(lsp)");
    // "failed to start" alone is `lsp`'s catch-all prefix for ANY
    // connect failure (spawn error, malformed reply, timeout, ...) --
    // near-tautologically true here, so it proves nothing about WHICH
    // failure fired. Require the actual timeout evidence instead.
    assert!(
        msg.to_lowercase().contains("timed out") || msg.to_lowercase().contains("timeout"),
        "expected lsp's own timeout-specific failure message, got: {}",
        msg
    );
    assert_eq!(
        run(&mut i, "lsp--connections"),
        "nil",
        "a failed M-x lsp must not leave a half-connected entry in lsp--connections"
    );
    assert_eq!(
        run(&mut i, "lsp--clients"),
        "nil",
        "a failed M-x lsp must not leave a half-connected entry in lsp--clients either"
    );
    assert_eq!(
        run(&mut i, "(length test--killed-conns)"),
        "1",
        "a failed M-x lsp should kill exactly the one connection it started"
    );
    let pid = conn_pid(&mut i, "(car test--killed-conns)");
    assert!(
        wait_until_pid_dead(pid, std::time::Duration::from_secs(2)),
        "orphaned deaf server (pid {}) still running after a failed M-x lsp",
        pid
    );
}

#[test]
fn lsp_await_timeout_budget_survives_a_chatty_never_answering_server() {
    let dir = Scratch::new("lsp_await_chatty");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_script(&dir, "chatty.sh", CHATTY_SCRIPT);

    let mut i = setup();
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    run(&mut i, "(setq client (make-lsp--client :conn conn))");

    let start = std::time::Instant::now();
    // id 99999 is never answered; the script only ever sends
    // `$/progress` notifications (no id), so `lsp--dispatch` drops every
    // one of them and `lsp--await' keeps looping. A per-message-reset
    // bug would see a new message roughly every 50ms and never notice
    // the deadline has long passed -- it would loop forever.
    //
    // The budget is 2s rather than something snappier like 0.3s for the
    // mutation-observability reason spelled out on `CHATTY_SCRIPT': the
    // defence is only observable while the server keeps talking faster
    // than the budget, so the gap between "message interval" (~50ms) and
    // "budget" is what buys that observation robustness against machine
    // load. 2s costs this one test 2 seconds and makes a false negative
    // need a ~40x scheduling stall instead of a ~6x one.
    let r = run(
        &mut i,
        r#"(condition-case err (lsp--await client 99999 2.0) (error (format "%S" err)))"#,
    );
    let elapsed = start.elapsed();
    run(&mut i, "(lsp-kill conn)");

    assert!(
        r.to_lowercase().contains("timed out") || r.to_lowercase().contains("timeout"),
        "expected a timeout error, got: {}",
        r
    );
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "lsp--await took {:?}, budget should have expired around 2s -- \
         looks like the chatty server reset the wait budget instead of it \
         being an absolute deadline",
        elapsed
    );
}

#[test]
fn lsp_wait_extreme_timeout_values_never_panic() {
    // M65 review: `need_wait_duration' used to go through
    // `Duration::from_secs_f64', which panics on `f64::MAX`/huge
    // floats/non-finite input -- unwinding the whole process, unsaved
    // edits included, with no `catch_unwind' anywhere in the repo to
    // stop it. None of the values below may panic.
    let dir = Scratch::new("lsp_wait_extremes");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_deaf_script(&dir);

    let mut i = setup();
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);

    assert_eq!(
        run(&mut i, "(lsp-wait conn -1)"),
        "nil",
        "negative timeout: single non-blocking check, no message available"
    );
    assert_eq!(
        run(&mut i, "(lsp-wait conn 0)"),
        "nil",
        "zero timeout: single non-blocking check, no message available"
    );
    run(&mut i, "(lsp-kill conn)");

    // Huge-timeout cases (bignum, and the exact `1e20` value the
    // reviewer's standalone repro panicked on): `need_wait_duration'
    // maps these to `Duration::MAX' (~584 billion years), which this
    // test emphatically cannot afford to actually wait out. Instead of
    // a deaf server, this uses one that exits IMMEDIATELY with no
    // reply: the reader thread hits EOF at once and enqueues
    // `LspEvent::Died' on the channel before `lsp-wait' ever calls
    // `recv_timeout' -- `mpsc::Receiver::recv_timeout' returns as soon
    // as a message is already queued, regardless of how large the
    // timeout argument is, so this returns almost instantly REGARDLESS
    // of whether the Duration conversion is correct. What it actually
    // proves is narrower but still exactly the point: building a
    // `Duration' from a bignum/`1e20' TIMEOUT and handing it to
    // `recv_timeout' does not panic. (A true "did the value clamp to
    // something sane rather than error out" check would need a
    // reachable never-answering server and is intentionally not
    // attempted here, since that upper bound is unobservable in a test
    // that must finish in finite time.)
    let dies_at_once = |i: &mut elisp::Interp, tag: &str, timeout_expr: &str| {
        let dir = Scratch::new(tag);
        std::fs::create_dir_all(&*dir).unwrap();
        let script = write_script(&dir, "dies.sh", "#!/bin/sh\nexit 0\n");
        let r = run(i, &format!("(setq conn (lsp-start {:?}))", script));
        assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
        let r = run(i, &format!("(lsp-wait conn {})", timeout_expr));
        assert!(
            !r.starts_with("ERROR: lisp error"),
            "TIMEOUT {} should not itself be treated as a Lisp error, got: {}",
            timeout_expr,
            r
        );
        // M65 tail review: nothing anywhere asserted the SHAPE `lsp-wait`
        // returns when the server dies during a bounded wait -- death was
        // only ever checked indirectly through `lsp-live-p`, so swapping
        // `dead_value(i)` for `Value::Nil` in that arm would have gone
        // unnoticed. It matters precisely because M65 gave `nil` a second
        // meaning (timeout): `lsp--await` reads a `nil` as "budget
        // expired" and signals, so a death reported as `nil` would be
        // mis-told to the user as a timeout.
        assert!(
            r.contains("died"),
            "a server that exits during a bounded wait must come back as \
             the death value, not as the timeout `nil' -- got: {}",
            r
        );
        run(i, "(lsp-kill conn)");
    };
    dies_at_once(
        &mut i,
        "lsp_wait_extremes_bignum",
        "(* 100000000000000000000 100000000000000000000)",
    );
    dies_at_once(&mut i, "lsp_wait_extremes_1e20", "1e20");
}

#[test]
fn lsp_connection_killed_purely_by_drop_when_unreachable() {
    // M65 review: every other kill-on-failure test here goes through an
    // EXPLICIT `lsp-kill` call (directly, or via `lsp-connect`'s M65
    // failure path). None of them exercise `Drop for LspConnection`
    // itself -- the fallback that's supposed to catch everything else,
    // and the one the reviewer flagged as having no coverage at all.
    // IGNORES_STDIN_SCRIPT, not DEAF_SCRIPT: `DEAF_SCRIPT` (`cat
    // >/dev/null`) exits on its own the instant its stdin sees EOF,
    // which `Drop`'s `self.stdin = None` alone already causes -- so a
    // mutation check removing `kill()` from `Drop for LspConnection'
    // left this test green even though the actual `kill()` call was
    // never exercised (M65 review round 3). A script that never reads
    // its stdin at all is the only way to make `kill()` load-bearing
    // here: without it, `child.wait()` in the destructor has nothing
    // that will ever make the process exit on its own.
    let dir = Scratch::new("lsp_drop_kill");
    std::fs::create_dir_all(&*dir).unwrap();
    let script = write_script(&dir, "ignores_stdin.sh", IGNORES_STDIN_SCRIPT);

    let mut i = setup();
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    let pid = conn_pid(&mut i, "conn");
    // Two-sided: the pid must be a LIVE process before the drop, or the
    // "it's dead afterwards" assertion below proves nothing about
    // `kill()` (see `pid_is_alive`). This is also what makes
    // `lsp-connection-pid` returning the wrong number observable at all.
    assert!(
        pid_is_alive(pid),
        "lsp-connection-pid returned {} but no such process is running -- \
         the post-drop death assertion below would pass vacuously",
        pid
    );

    // Drop the only Lisp reference to the connection -- no `lsp-kill`,
    // no `lsp-connect` failure path, nothing else in this interp holds
    // it (never pushed onto `lsp--clients`/`lsp--connections`, never
    // wrapped in a `lsp--client`). `Value::Ext`'s payload is a plain
    // `Rc` (see `crates/elisp/src/value.rs`), not something requiring a
    // tracing sweep to reclaim an acyclic object -- `(garbage-collect)`
    // only exists to break REFERENCE CYCLES (see `crates/elisp/src/
    // gc.rs`'s module doc) -- so this is included for completeness/
    // documentation of that fact, not because it's expected to be
    // load-bearing for this particular drop.
    run(&mut i, "(setq conn nil)");
    run(&mut i, "(garbage-collect)");

    assert!(
        wait_until_pid_dead(pid, std::time::Duration::from_secs(2)),
        "orphaned deaf server (pid {}) still running after its only Lisp \
         reference was dropped -- Drop for LspConnection did not fire or \
         did not kill it",
        pid
    );
}

#[test]
fn connect_and_shutdown() {
    if !have_rust_analyzer() {
        eprintln!("skipping: rust-analyzer not on PATH");
        return;
    }
    let dir = write_scratch_project("connect_and_shutdown");
    let mut i = setup();
    let src = format!(
        "(setq client (lsp-connect \"rust-analyzer\" nil {:?}))",
        dir.to_str().unwrap()
    );
    let r = run(&mut i, &src);
    assert!(!r.starts_with("ERROR"), "lsp-connect failed: {}", r);
    assert_eq!(
        run(&mut i, "(lsp-connection-p (lsp--client-conn client))"),
        "t"
    );
    assert_eq!(run(&mut i, "(lsp-live-p (lsp--client-conn client))"), "t");
    run(&mut i, "(lsp-shutdown client)");
}

// Verified working (see PLAN.md M14): passes reliably run alone, even
// with only a handful of retries. Ignored by default because it's
// genuinely flaky when many test binaries run in parallel (plain
// `cargo test --workspace`/`cargo test -p core`, no other change) --
// rust-analyzer's own internal `cargo`/`rustc` subprocess spawning
// starts intermittently failing under that resource contention, not
// because of anything in this client. Run deliberately with
// `cargo test -p core --test lsp_tests -- --ignored`.
#[test]
#[ignore]
fn hover_and_definition_against_real_rust_analyzer() {
    if !have_rust_analyzer() {
        eprintln!("skipping: rust-analyzer not on PATH");
        return;
    }
    let dir = write_scratch_project("hover_and_definition");
    let main_rs = dir.join("src/main.rs");
    let text = std::fs::read_to_string(&main_rs).unwrap();
    let mut i = setup();

    let connect_src = format!(
        "(setq client (lsp-connect \"rust-analyzer\" nil {:?}))",
        dir.to_str().unwrap()
    );
    let r = run(&mut i, &connect_src);
    assert!(!r.starts_with("ERROR"), "lsp-connect failed: {}", r);

    let open_src = format!(
        "(lsp-did-open client {:?} {:?})",
        main_rs.to_str().unwrap(),
        text
    );
    let r = run(&mut i, &open_src);
    assert!(!r.starts_with("ERROR"), "lsp-did-open failed: {}", r);

    // rust-analyzer needs a moment to index even this tiny project;
    // real editor clients all retry hover/definition for the same
    // reason. Bounded, not an indefinite hang: `lsp--await` always
    // returns (either an actual answer or a genuine `null` result), so
    // each attempt completes quickly regardless.
    let hover_src = format!("(lsp-hover client {:?} 5 18)", main_rs.to_str().unwrap());
    let mut hover = String::new();
    for attempt in 0..60 {
        hover = run(&mut i, &hover_src);
        if hover != "nil" && !hover.starts_with("ERROR") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        eprintln!("hover attempt {attempt}: {hover}");
    }
    assert!(!hover.starts_with("ERROR"), "lsp-hover failed: {}", hover);
    assert_ne!(
        hover, "nil",
        "rust-analyzer never returned hover info after retrying"
    );
    assert!(hover.contains("i32"), "hover result: {}", hover);

    let def_src = format!(
        "(lsp-definition client {:?} 5 18)",
        main_rs.to_str().unwrap()
    );
    let mut def = String::new();
    for attempt in 0..60 {
        def = run(&mut i, &def_src);
        if def != "nil" && !def.starts_with("ERROR") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        eprintln!("definition attempt {attempt}: {def}");
    }
    assert!(!def.starts_with("ERROR"), "lsp-definition failed: {}", def);
    assert_ne!(
        def, "nil",
        "rust-analyzer never returned a definition after retrying"
    );
    // `add` is defined starting on line 0 (0-based) of main.rs.
    assert!(
        def.contains("main.rs") && def.ends_with(" . 0)"),
        "definition result: {}",
        def
    );

    run(&mut i, "(lsp-shutdown client)");
}

// --- M99: `lsp-start`'s third (CWD) argument -----------------------------

/// Writes a script that prints its own working directory into
/// OUTPUT_FILE (an absolute path baked into the script body at write
/// time, not passed as an argv entry), then behaves like `DEAF_SCRIPT`
/// (reads stdin to EOF, never exits on its own) so the connection stays
/// alive long enough for the test to poll for the file and then
/// `lsp-kill' it explicitly.
fn write_pwd_script(dir: &std::path::Path, name: &str, output_file: &std::path::Path) -> String {
    let body = format!(
        "#!/bin/sh\npwd > '{}'\ncat >/dev/null\n",
        output_file.to_str().unwrap()
    );
    write_script(dir, name, &body)
}

/// Polls PATH for up to ~2s (matching `wait_until_pid_dead`'s own
/// budget) and returns its trimmed contents, or an empty string if it
/// never appeared -- the caller asserts non-empty itself so a timeout
/// produces a readable failure message instead of a panic here.
fn wait_for_file_contents(path: &std::path::Path) -> String {
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(path) {
            return s.trim().to_string();
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    String::new()
}

#[test]
fn lsp_start_with_cwd_arg_sets_the_server_processs_working_directory() {
    let dir = Scratch::new("lsp_cwd_basic");
    std::fs::create_dir_all(&*dir).unwrap();
    let target_cwd = Scratch::new("lsp_cwd_basic_target");
    std::fs::create_dir_all(&*target_cwd).unwrap();
    let out_file = dir.join("pwd.txt");
    let script = write_pwd_script(&dir, "pwd.sh", &out_file);

    let mut i = setup();
    let r = run(
        &mut i,
        &format!(
            "(setq conn (lsp-start {:?} nil {:?}))",
            script,
            target_cwd.to_str().unwrap()
        ),
    );
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    let content = wait_for_file_contents(&out_file);
    run(&mut i, "(lsp-kill conn)");
    assert!(!content.is_empty(), "server never wrote its pwd");
    // macOS: /tmp is a symlink to /private/tmp -- canonicalize both sides
    // before comparing, or this is a guaranteed false negative there.
    let got = std::fs::canonicalize(&content)
        .unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", content, e));
    let want = std::fs::canonicalize(&*target_cwd).unwrap();
    assert_eq!(got, want, "server cwd did not match the CWD argument");
}

#[test]
fn lsp_start_without_cwd_arg_inherits_the_editor_process_cwd() {
    let dir = Scratch::new("lsp_cwd_default");
    std::fs::create_dir_all(&*dir).unwrap();
    let out_file = dir.join("pwd.txt");
    let script = write_pwd_script(&dir, "pwd.sh", &out_file);

    let mut i = setup();
    // Two-arg call, exactly as every pre-M99 caller in this file makes
    // it -- CWD must default to "inherit", not "some arbitrary
    // directory", for every one of those callers to keep behaving as
    // before.
    let r = run(&mut i, &format!("(setq conn (lsp-start {:?}))", script));
    assert!(!r.starts_with("ERROR"), "lsp-start failed: {}", r);
    let content = wait_for_file_contents(&out_file);
    run(&mut i, "(lsp-kill conn)");
    assert!(!content.is_empty(), "server never wrote its pwd");
    let got = std::fs::canonicalize(&content)
        .unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", content, e));
    let want = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    assert_eq!(
        got, want,
        "server cwd should default to the editor process's own cwd"
    );
}

#[test]
fn lsp_start_relative_cmd_containing_a_slash_still_resolves_against_the_editor_cwd_not_the_new_server_cwd(
) {
    // `Command::new` resolves a bare (no '/') name via PATH search,
    // unaffected by `current_dir` -- only a relative path CONTAINING a
    // separator is at risk of being resolved against the wrong
    // directory once `current_dir` is set (POSIX chdir-then-exec
    // ordering). This test's script path deliberately contains one
    // ("./DIRNAME/pwd.sh").
    let real_cwd = std::env::current_dir().unwrap();
    let rel_dir_name = format!(
        "reticle_lsp_relcmd_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    let script_dir = real_cwd.join(&rel_dir_name);
    std::fs::create_dir_all(&script_dir).unwrap();

    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
    let _cleanup = Cleanup(script_dir.clone());

    let target_cwd = Scratch::new("lsp_cwd_relcmd_target");
    std::fs::create_dir_all(&*target_cwd).unwrap();
    let out_file = script_dir.join("pwd.txt");
    write_pwd_script(&script_dir, "pwd.sh", &out_file);
    let rel_cmd = format!("./{}/pwd.sh", rel_dir_name);

    let mut i = setup();
    let r = run(
        &mut i,
        &format!(
            "(setq conn (lsp-start {:?} nil {:?}))",
            rel_cmd,
            target_cwd.to_str().unwrap()
        ),
    );
    assert!(
        !r.starts_with("ERROR"),
        "lsp-start failed (the relative cmd must still resolve against the \
         editor's own cwd, {:?}, not the new server cwd, {:?}): {}",
        real_cwd,
        target_cwd.to_str().unwrap(),
        r
    );
    let content = wait_for_file_contents(&out_file);
    run(&mut i, "(lsp-kill conn)");
    assert!(!content.is_empty(), "server never wrote its pwd");
    let got = std::fs::canonicalize(&content)
        .unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", content, e));
    let want = std::fs::canonicalize(&*target_cwd).unwrap();
    assert_eq!(
        got, want,
        "the spawned server's own cwd should still be the CWD argument, \
         even though its own executable path was relative"
    );
}

/// A pwd-recording server that also completes `lsp-connect''s
/// `initialize' handshake, so this can be driven through the elisp-level
/// `lsp-connect' (not just the Rust-level `lsp-start') -- writes its own
/// cwd to OUTPUT_FILE, then answers `initialize' (id 1) with an empty
/// result exactly like `ECHO_ONE_SCRIPT' (ignoring the request's actual
/// bytes, same as that script already does), then reads stdin to EOF
/// forever so the connection survives long enough for the test to poll
/// the file and `lsp-kill' explicitly.
fn write_pwd_and_handshake_script(
    dir: &std::path::Path,
    name: &str,
    output_file: &std::path::Path,
) -> String {
    let body = format!(
        "#!/bin/sh\n\
         pwd > '{}'\n\
         msg='{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}'\n\
         len=$(printf '%s' \"$msg\" | wc -c)\n\
         printf 'Content-Length: %d\\r\\n\\r\\n%s' \"$len\" \"$msg\"\n\
         cat >/dev/null\n",
        output_file.to_str().unwrap()
    );
    write_script(dir, name, &body)
}

#[test]
fn lsp_connect_passes_its_root_path_argument_through_as_the_servers_cwd() {
    // F6 (M99 review): the whole reason M99's `lsp-start' grew a CWD
    // argument is so `lsp-connect'/`lsp--autostart-begin' could pass
    // their project ROOT down to it -- but the three lsp-start-level
    // tests above only ever exercise `lsp-start' itself. This drives
    // the actual `lsp-connect' entry point end to end (real handshake,
    // real subprocess) and checks the spawned server's OWN cwd, not
    // just that `lsp-connect' succeeded.
    let dir = Scratch::new("lsp_connect_cwd_script");
    std::fs::create_dir_all(&*dir).unwrap();
    let target_cwd = Scratch::new("lsp_connect_cwd_root");
    std::fs::create_dir_all(&*target_cwd).unwrap();
    let out_file = dir.join("pwd.txt");
    let script = write_pwd_and_handshake_script(&dir, "pwd_handshake.sh", &out_file);

    let mut i = setup();
    let r = run(
        &mut i,
        &format!(
            "(setq client (lsp-connect {:?} nil {:?}))",
            script,
            target_cwd.to_str().unwrap()
        ),
    );
    assert!(!r.starts_with("ERROR"), "lsp-connect failed: {}", r);

    let content = wait_for_file_contents(&out_file);
    run(&mut i, "(lsp-kill (lsp--client-conn client))");
    assert!(!content.is_empty(), "server never wrote its pwd");
    // macOS: /tmp is a symlink to /private/tmp -- canonicalize both
    // sides before comparing, or this is a guaranteed false negative.
    let got = std::fs::canonicalize(&content)
        .unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", content, e));
    let want = std::fs::canonicalize(&*target_cwd).unwrap();
    assert_eq!(
        got, want,
        "lsp-connect's ROOT-PATH argument must reach lsp-start as the \
         spawned server's own cwd"
    );
}

#[test]
fn lsp_autostart_begin_passes_its_root_argument_through_as_the_servers_cwd() {
    // F6 (M99 review), "best effort" half: `lsp--autostart-begin' is
    // the OTHER call site that passes ROOT down to `lsp-start' as CWD
    // (`lsp-connect', above, is the first). Called directly here --
    // this function is a plain 4-arg function with no dependency on the
    // heavier `lsp-server-alist'/`find-file' autostart wiring that
    // lives in `lsp_autostart_tests.rs' -- so this does not need that
    // file's fixtures. The `initialize' reply is handled asynchronously
    // by the idle pump and is NOT awaited here; the cwd effect under
    // test happens synchronously inside `lsp-start' the moment the
    // process is spawned, before any reply could even arrive, so
    // there's nothing to drive or wait on beyond the spawned script
    // writing its own pwd.
    let dir = Scratch::new("lsp_autostart_cwd_script");
    std::fs::create_dir_all(&*dir).unwrap();
    let target_root = Scratch::new("lsp_autostart_cwd_root");
    std::fs::create_dir_all(&*target_root).unwrap();
    let out_file = dir.join("pwd.txt");
    let script = write_pwd_and_handshake_script(&dir, "pwd_handshake.sh", &out_file);

    let mut i = setup();
    let r = run(
        &mut i,
        &format!(
            "(lsp--autostart-begin {:?} nil {:?} 'fundamental-mode)",
            script,
            target_root.to_str().unwrap()
        ),
    );
    assert!(
        !r.starts_with("ERROR"),
        "lsp--autostart-begin failed: {}",
        r
    );

    let content = wait_for_file_contents(&out_file);
    // Clean up via the client this call pushed onto `lsp--clients' --
    // `lsp--autostart-begin' itself returns nil (it fires-and-forgets),
    // so the connection is reached through the global client list
    // rather than a returned handle.
    run(
        &mut i,
        "(dolist (c lsp--clients) (lsp-kill (lsp--client-conn c)))",
    );
    assert!(!content.is_empty(), "server never wrote its pwd");
    let got = std::fs::canonicalize(&content)
        .unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", content, e));
    let want = std::fs::canonicalize(&*target_root).unwrap();
    assert_eq!(
        got, want,
        "lsp--autostart-begin's ROOT argument must reach lsp-start as \
         the spawned server's own cwd"
    );
}
