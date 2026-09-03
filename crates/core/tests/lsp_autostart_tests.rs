//! M88: automatic asynchronous startup of an LSP server on first open --
//! `lsp-autostart`, `lsp-autostart-timeout`, `lsp--autostart-pending`,
//! `lsp--autostart-tried`, `lsp--autostart-begin`, and the idle-tick step
//! `lsp--autostart-tick`, all in `lsp.el`.
//!
//! Same "cat" transport technique as `lsp_auto_attach_tests.rs' (see that
//! file's own header): `cat' is registered as the server command, and the
//! `initialize' request's id echoes straight back, completing the
//! asynchronous handshake via the ordinary `lsp-process-pending-all' pump
//! -- no `lsp--await' anywhere on this path. For the "deaf server" tests
//! (spawns, reads stdin, never writes anything back), `sh -c "cat >
//! /dev/null"' stands in: a real, genuinely alive child process that will
//! never answer.
//!
//! `lsp--autostart-tick' no-ops entirely until `lsp--frontend-started' is
//! set -- every test below sets it explicitly with `(setq lsp--frontend-
//! started t)' unless the test is specifically about that gate (test 7).
//!
//! Fix-round note (F review; G4 fix round 3 -- do not restate a count of
//! call sites here, it will go stale again the next time a test is
//! added or removed; grep for `dispatch_one_pending' instead): every
//! test below that needs to observe an outgoing frame (a `didOpen', or
//! a still-pending client) right after completing a handshake calls the
//! `dispatch_one_pending' helper instead of `lsp-process-pending-all'.
//! That workaround exists because `cat' echoes bytes back essentially
//! instantly: `lsp-process-pending-all''s ordinary drain-until-empty
//! loop, mid-dispatch of the `initialize' reply, would otherwise
//! immediately re-poll and silently swallow the `didOpen' the
//! completion callback sends synchronously from inside that same
//! dispatch -- an artifact of this specific fake transport, not
//! something a real server (which never echoes its own notifications
//! back) can trigger. The consequence: this file does NOT end-to-end
//! exercise the production drain-and-re-poll path (`lsp-process-
//! pending-all') completing an autostart handshake and sending its
//! `didOpen' in the same call -- `idle_tick_completes_a_near_deadline_
//! handshake_before_reaping_it' (the only test here that drives
//! `core::idle_tick' directly) DOES go through the real `lsp-process-
//! pending-all' end to end, but does not itself assert on `didOpen'
//! framing -- only on which of "attached" vs "reaped" won.

use elisp::printer::prin1_to_string;
use elisp::Interp;

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

fn ok(interp: &mut Interp, src: &str) -> String {
    let r = run(interp, src);
    assert!(!r.starts_with("ERROR"), "{:?} failed: {}", src, r);
    r
}

/// A scratch directory that deletes itself on drop -- same shape as
/// `lsp_auto_attach_tests.rs`'s `Scratch` (see its own doc comment for
/// why cleanup lives in `Drop`, not the last line of each test body).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_lsp_autostart_{}_{}_{}",
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

fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// Drain every frame `conn` has echoed back whose "method" is METHOD --
/// same technique as `lsp_auto_attach_tests.rs`'s helper of the same name.
fn drain_frames(i: &mut Interp, conn_expr: &str, method: &str) -> usize {
    let src = format!(
        "(let ((frames nil) (msg t))
           (while msg
             (setq msg (lsp-poll {conn}))
             (when (and msg (equal (gethash \"method\" msg nil) {method:?}))
               (setq frames (cons msg frames))))
           (setq test--frames (nreverse frames))
           (length test--frames))",
        conn = conn_expr,
        method = method,
    );
    ok(i, &src)
        .parse()
        .expect("frame count should print as an integer")
}

fn register_cat(i: &mut Interp) {
    ok(
        i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
}

fn set_frontend_started(i: &mut Interp) {
    ok(i, "(setq lsp--frontend-started t)");
}

/// Poll exactly ONE message off the (single) pending autostart's
/// connection and dispatch it -- unlike `lsp-process-pending-all`, this
/// does not loop until the queue is empty, so it can't self-consume a
/// message the completion callback it triggers goes on to send. That
/// self-consumption is a real risk ONLY because the fake transport
/// (`cat`) echoes bytes back essentially instantly -- a real server
/// never echoes its own `didOpen` back at all, so `lsp-process-pending-
/// all`'s draining loop is exactly right in production; it's just the
/// wrong tool for observing one specific outgoing frame against this
/// particular test double.
fn dispatch_one_pending(i: &mut Interp) {
    ok(
        i,
        "(setq test--pending-client (nth 1 (cdr (car lsp--autostart-pending))))",
    );
    ok(
        i,
        "(let ((msg (lsp-poll (lsp--client-conn test--pending-client))))
           (when (hash-table-p msg) (lsp--dispatch test--pending-client msg)))",
    );
}

/// Same message-capturing shape used by `lsp_action_tests.rs`,
/// `lsp_format_tests.rs`, `lsp_highlight_tests.rs`, `lsp_references_tests.rs`.
fn capture_messages(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

// ============================================================
// 1. Happy path: idle tick spawns, completion attaches the current
//    buffer, and backfill also attaches another already-open buffer in
//    the same project.
// ============================================================

#[test]
fn happy_path_spawns_attaches_and_backfills() {
    let mut i = setup();
    let dir = scratch_dir("happy_path");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // b.rs opened first -- no connection exists yet, so it just sits
    // there unattached.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    // a.rs opened second and left current -- this is the buffer whose
    // idle tick triggers the autostart.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    set_frontend_started(&mut i);
    ok(&mut i, "(lsp--autostart-tick)");

    // Spawned, but NOT yet attached (I2): the handshake is still
    // in-flight, so lsp--buffer-client stays nil until the completion
    // callback runs.
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Let cat's echoed initialize reply arrive, which fires the
    // completion closure synchronously from inside `dispatch_one_pending`.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );
    ok(&mut i, "(setq test--client lsp--buffer-client)");

    // b.rs -- never itself ticked -- must have been backfilled too.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--client)"), "t");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(&mut i, "test--conn", "textDocument/didOpen"),
        2,
        "both a.rs and b.rs should have been didOpen'd"
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 2. Deaf server: spawns, reads, never writes. The editor stays
//    responsive across many idle ticks; after the deadline the pending
//    entry is reaped, lsp-kill runs, one message is emitted, the entry
//    lands in lsp--autostart-tried, and no second spawn happens.
// ============================================================

#[test]
fn deaf_server_is_reaped_after_the_deadline_with_no_retry() {
    let mut i = setup();
    let dir = scratch_dir("deaf_server");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist
             (cons 'rust-mode (list \"sh\" \"-c\" \"cat > /dev/null\")))",
    );
    ok(&mut i, "(setq lsp-autostart-timeout 1)");
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Several idle ticks well inside the 1s deadline: the entry must
    // stay pending, and ordinary editing must keep working -- nothing on
    // this path blocks. F8 review fix: bound the NON-sleep work of each
    // iteration (the sleep itself is the test's own pacing, not part of
    // what "responsive" claims) so the "stays responsive" comment is an
    // observation, not just a description of which state variables get
    // checked afterward. 2s of slack per iteration is generous -- a
    // genuinely blocked call here would mean `lsp-kill`'s `child.wait()`
    // (or something worse) hanging on a deaf server that hasn't even
    // been killed yet, not a plausible slow-but-fine case.
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let tick_start = std::time::Instant::now();
        ok(&mut i, "(lsp-process-pending-all)");
        ok(&mut i, "(lsp--autostart-tick)");
        ok(&mut i, "(insert \"x\")");
        let tick_elapsed = tick_start.elapsed();
        assert!(
            tick_elapsed < std::time::Duration::from_secs(2),
            "one idle-tick-shaped iteration took {:?} against a deaf \
             server well before its deadline -- the editor is supposed \
             to stay responsive here",
            tick_elapsed
        );
    }
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "lsp--autostart-tried"), "nil");

    // Push well past the 1s deadline, then tick again -- this is the
    // call that reaps the entry, i.e. the one that reaches `lsp-kill`'s
    // blocking `child.wait()` (documented at this file's own M88
    // section header as the one honest exception to "nothing on this
    // path blocks"). Bounded generously: `sh -c "cat > /dev/null"` is a
    // real, well-behaved child that responds to SIGKILL immediately, so
    // this reap should be on the order of milliseconds, not seconds.
    std::thread::sleep(std::time::Duration::from_millis(1200));
    let reap_start = std::time::Instant::now();
    ok(&mut i, "(lsp-process-pending-all)");
    ok(&mut i, "(lsp--autostart-tick)");
    let reap_elapsed = reap_start.elapsed();
    assert!(
        reap_elapsed < std::time::Duration::from_secs(5),
        "reaping the deaf server took {:?} -- lsp-kill's child.wait() \
         may be stuck",
        reap_elapsed
    );

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(lsp-live-p test--pending-conn)"), "nil");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("gave up"),
        "expected a give-up message, got: {}",
        messages
    );
    assert_eq!(run(&mut i, "(length test--messages)"), "1");

    // No second spawn: another tick with the same buffer current must
    // not touch lsp--autostart-pending or spawn a new client.
    let clients_before = run(&mut i, "(length lsp--clients)");
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), clients_before);
}

// ============================================================
// 3. Missing binary: one message, no panic, entry in the give-up list,
//    never retried.
// ============================================================

#[test]
fn missing_binary_is_recorded_and_never_retried() {
    let mut i = setup();
    let dir = scratch_dir("missing_binary");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist
             (cons 'rust-mode (list \"reticle-lsp-autostart-test-no-such-binary-88\")))",
    );
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    let r = run(&mut i, "(lsp--autostart-tick)");
    assert!(!r.starts_with("ERROR"), "tick signaled: {}", r);
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length test--messages)"), "1");

    // A second tick with the same buffer current must not retry.
    let r2 = run(&mut i, "(lsp--autostart-tick)");
    assert!(!r2.starts_with("ERROR"), "second tick signaled: {}", r2);
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length test--messages)"), "1");
}

// ============================================================
// 4. Two buffers in one project produce exactly one spawn (I3).
// ============================================================

#[test]
fn two_buffers_in_one_project_produce_exactly_one_spawn() {
    let mut i = setup();
    let dir = scratch_dir("one_spawn");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Tick with a.rs current: begins the one and only spawn.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), "1");

    // Switch to b.rs (already open -- find-file-hook does not re-run)
    // and tick again: same project/command, so it must find the
    // existing pending entry rather than spawning its own.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), "1");

    // Let it complete; both buffers must end up on the SAME client.
    std::thread::sleep(std::time::Duration::from_millis(300));
    ok(&mut i, "(lsp-process-pending-all)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil"); // b.rs
    ok(&mut i, "(setq test--client-b lsp--buffer-client)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--client-b)"), "t");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--client-b))");
}

// ============================================================
// 5. lsp-autostart nil: never spawns.
// ============================================================

#[test]
fn lsp_autostart_nil_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("autostart_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);
    ok(&mut i, "(setq lsp-autostart nil)");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 6. lsp-auto-attach nil: never spawns (D6's coupling).
// ============================================================

#[test]
fn lsp_auto_attach_nil_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("auto_attach_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);
    ok(&mut i, "(setq lsp-auto-attach nil)");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 7. Frontend flag never set: never spawns. This documents why every
//    other integration test file in this crate -- which calls
//    `find-file-internal`/`eval_source` directly and never goes through
//    a real `run_tui`/`run_gui` loop -- can never trigger an autostart
//    spawn no matter how many idle-tick-shaped calls it happens to make.
// ============================================================

#[test]
fn frontend_flag_never_set_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("no_frontend_flag");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--frontend-started"), "nil");

    // Several ticks, exactly as the deadline-driven tests above do --
    // none of them may do anything at all.
    for _ in 0..5 {
        ok(&mut i, "(lsp--autostart-tick)");
    }
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 8. didChange cannot precede didOpen (I2): edit the buffer while the
//    autostart is pending, then let it complete, and assert the
//    didOpen carries the edited text and that no didChange was sent
//    before it -- one connection drain, one pass, since `lsp-poll` is
//    destructive and a second `drain_frames` call on the same
//    connection would find nothing left.
// ============================================================

#[test]
fn did_change_cannot_precede_did_open() {
    let mut i = setup();
    let dir = scratch_dir("no_early_did_change");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    // A deliberately DELAYED echo, not the ordinary `register_cat`
    // helper -- forecloses a real race by construction rather than
    // papering over its symptom. `cat` alone echoes the `initialize`
    // request back essentially instantly, and under default (parallel)
    // thread count, several spawned test processes contending for CPU
    // at once, that echo can occasionally land fast enough for the
    // THREE bare `lsp-process-pending-all` pumps below (no sleep
    // before them, by design -- see their own comment) to already
    // drain and dispatch it, completing the handshake from INSIDE that
    // same drain-until-empty loop -- which then self-consumes the
    // `didOpen` the completion callback sends, the exact failure mode
    // `dispatch_one_pending` exists to avoid elsewhere in this file.
    // A 1s delay before the fake server echoes anything guarantees
    // those three quick pumps see nothing, so this test's own
    // subsequent sleep + `dispatch_one_pending` (not
    // `lsp-process-pending-all`) is deterministically what completes
    // the handshake, every time.
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"sh\" \"-c\" \"sleep 1 && exec cat\")))",
    );

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Edit while the handshake is still in flight, and run several
    // pumps -- lsp--sync-buffer-now's own buffer walk runs on every one
    // of them, and I2 says it must be a no-op here (lsp--buffer-client
    // is still nil), so no didChange can be sent for this edit. The
    // delayed transport (above) guarantees these three can't
    // accidentally also complete the handshake.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"\\n// edited while pending\\n\")");
    for _ in 0..3 {
        ok(&mut i, "(lsp-process-pending-all)");
    }
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    std::thread::sleep(std::time::Duration::from_millis(1300));
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    std::thread::sleep(std::time::Duration::from_millis(300));
    // One pass over the connection's whole queue, classifying every
    // frame by method, so nothing is lost to a second (empty) drain.
    let src = "(let ((opens nil) (changes nil) (msg t))
                 (while msg
                   (setq msg (lsp-poll test--conn))
                   (when msg
                     (let ((m (gethash \"method\" msg nil)))
                       (cond
                        ((equal m \"textDocument/didOpen\") (push msg opens))
                        ((equal m \"textDocument/didChange\") (push msg changes))))))
                 (setq test--opens (nreverse opens))
                 (setq test--changes (nreverse changes))
                 (list (length test--opens) (length test--changes)))";
    let counts = ok(&mut i, src);
    assert_eq!(
        counts, "(1 0)",
        "expected exactly one didOpen and no didChange"
    );

    let text = ok(
        &mut i,
        "(gethash \"text\" (gethash \"textDocument\"
             (gethash \"params\" (car test--opens))))",
    );
    assert!(
        text.contains("edited while pending"),
        "didOpen text missing the edit made while autostart was pending: {}",
        text
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 9. A publishDiagnostics arriving BEFORE our initialize response
//    decorates the buffer without panicking -- a state that has never
//    existed before this milestone.
// ============================================================

#[test]
fn publish_diagnostics_before_initialize_response_does_not_panic() {
    let mut i = setup();
    let dir = scratch_dir("early_diagnostics");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-client (nth 1 (cdr (car lsp--autostart-pending))))",
    );

    let uri_str = format!("file://{}", file.to_str().unwrap());
    let json_body = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":{:?},\"diagnostics\":[]}}}}",
        uri_str
    );
    let msg_expr = format!("(json-parse-string {:?})", json_body);
    let r = run(
        &mut i,
        &format!("(lsp--dispatch test--pending-client {})", msg_expr),
    );
    assert!(!r.starts_with("ERROR"), "dispatch signaled: {}", r);

    // Diagnostics landed on the still-pending client despite no
    // completed handshake yet.
    assert_ne!(
        run(
            &mut i,
            "(lsp-diagnostics test--pending-client (buffer-file-name))"
        ),
        ""
    );

    // The rest of the handshake must still complete normally afterward.
    std::thread::sleep(std::time::Duration::from_millis(300));
    ok(&mut i, "(lsp-process-pending-all)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 10. The mode line shows the pending state and then the attached
//     state (D7): `lsp--autostart-pending-here` -- the buffer-local
//     signal `redisplay.rs` reads via the same `buffer_var_on` it
//     already uses for `lsp--buffer-client` -- is non-nil (keyed
//     `(COMMAND . ROOT)`) while pending, and back to nil once
//     `lsp--buffer-client` itself is set.
// ============================================================

#[test]
fn mode_line_signal_goes_pending_then_attached() {
    let mut i = setup();
    let dir = scratch_dir("mode_line_signal");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");
    ok(&mut i, "(lsp--autostart-tick)");

    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    let pending_here = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        pending_here, "nil",
        "expected a pending marker while in flight"
    );
    assert!(
        pending_here.contains("cat"),
        "pending marker should carry the command: {}",
        pending_here
    );

    std::thread::sleep(std::time::Duration::from_millis(300));
    ok(&mut i, "(lsp-process-pending-all)");

    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 11. F1 (high): an explicit `M-x lsp' racing an in-flight autostart
//     for the SAME (COMMAND . ROOT) must cancel the pending autostart
//     and win deterministically -- not spawn a second, leaked server
//     that the autostart's later completion would silently shadow.
// ============================================================

#[test]
fn m_x_lsp_cancels_an_in_flight_autostart_and_wins() {
    let mut i = setup();
    let dir = scratch_dir("f1_manual_wins");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Autostart begins first -- pending, not yet attached.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--autostart-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );
    assert_eq!(run(&mut i, "(lsp-live-p test--autostart-conn)"), "t");

    // `M-x lsp' races it and must win: cancel the pending autostart,
    // spawn its own (synchronous) connection, and attach the buffer.
    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "(lsp) signaled: {}", r);
    assert_eq!(r, "\"LSP: connected to cat\"");

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(
        run(&mut i, "(lsp-live-p test--autostart-conn)"),
        "nil",
        "the preempted autostart connection must have been killed"
    );
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    ok(&mut i, "(setq test--manual-client lsp--buffer-client)");

    // No zombie completion sneaks a second entry in later: pump/tick a
    // few more times (any reply the killed connection's channel might
    // still have buffered must not resurrect it -- see the completion
    // closure's own defensive branches).
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        ok(&mut i, "(lsp-process-pending-all)");
        ok(&mut i, "(lsp--autostart-tick)");
    }
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    assert_eq!(
        run(&mut i, "(eq lsp--buffer-client test--manual-client)"),
        "t"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--manual-client))");
}

// ============================================================
// 12. F1 (high): the completion closure's own defense -- if a
//     connection for (COMMAND . ROOT) already exists by the time it
//     fires, it must kill its own connection and attach nothing rather
//     than pushing a duplicate onto lsp--connections.
// ============================================================

#[test]
fn autostart_completion_defers_to_an_already_registered_connection() {
    let mut i = setup();
    let dir = scratch_dir("f1_completion_defensive");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Simulate a connection for the same key having already been
    // registered through some other path (this is exactly the state
    // `lsp--get-connection' would see), WITHOUT going through
    // `lsp--autostart-cancel-pending' -- so the pending entry this
    // closure is about to look up is still technically present, and
    // the defense under test is the SECOND cond branch (`lsp--get-
    // connection' already answers), not the first. Must be a REAL,
    // live connection: `lsp--get-connection' self-heals a dead/fake
    // one (prunes it and returns nil), which would silently defeat
    // this test by falling through to the ordinary success path
    // instead of exercising the defensive branch at all.
    ok(&mut i, "(setq test--other-conn (lsp-start \"cat\" nil))");
    ok(
        &mut i,
        "(setq test--other-client (make-lsp--client :conn test--other-conn :command \"cat\"))",
    );
    ok(
        &mut i,
        "(push (cons (cons \"cat\" (lsp--project-root (buffer-file-name))) \
                test--other-client)
              lsp--connections)",
    );

    // Now let the pending handshake's reply arrive and dispatch.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);

    // The pending entry must be gone, the buffer must NOT be attached
    // to the autostart's own client (nothing should have attached it
    // at all -- the already-registered connection is a stand-in with a
    // nil conn, unusable for a real attach), and lsp--connections must
    // still hold exactly the one (stand-in) entry -- no duplicate.
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    assert_eq!(
        run(
            &mut i,
            "(eq (cdr (car lsp--connections)) test--other-client)"
        ),
        "t",
        "the pre-existing connection must not have been replaced"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--pending-conn)"),
        "nil",
        "the autostart's own (now-redundant) connection must have been killed"
    );

    ok(&mut i, "(lsp-kill test--other-conn)");
}

// ============================================================
// 13. F5 (low): the pending marker must clear by its STORED key, not
//     by re-deriving "does this buffer still match" -- otherwise a
//     buffer whose major mode changes away from any `lsp-server-alist'
//     entry during the handshake window keeps showing `LSP…' forever.
// ============================================================

#[test]
fn pending_marker_clears_even_after_the_buffer_stops_matching() {
    let mut i = setup();
    let dir = scratch_dir("f5_sticky_marker");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    // The buffer's major mode changes away from any `lsp-server-alist'
    // entry WHILE the handshake is still in flight -- the exact window
    // where the pre-fix "re-derive match at clear time" logic would
    // have silently skipped clearing this buffer's marker forever.
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");

    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);

    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the pending marker must clear on completion even though this \
         buffer no longer matches any lsp-server-alist entry"
    );
    // Consistent with backfill's own exact-mode requirement: a buffer
    // that no longer matches the connecting mode is not attached.
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--pending-client))");
}

// ============================================================
// 14. F7 (low-medium): the pending marker must use the SAME exact-mode
//     test `lsp--auto-attach-backfill' uses, not "any mode whose
//     lsp-server-alist entry happens to share the same COMMAND" --
//     otherwise a buffer in a sibling mode (c-mode/c++-mode both -> the
//     same command here) shows `LSP…' as a promise backfill will never
//     keep.
// ============================================================

#[test]
fn pending_marker_requires_exact_mode_like_backfill_does() {
    let mut i = setup();
    let dir = scratch_dir("f7_exact_mode");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_c = dir.join("a.c");
    let file_cpp = dir.join("b.cpp");
    std::fs::write(&file_c, "int main(void) { return 0; }\n").unwrap();
    std::fs::write(&file_cpp, "int main() { return 0; }\n").unwrap();
    // Two DIFFERENT modes sharing the SAME command -- the exact shape
    // of the default `lsp-server-alist' (c-mode/c++-mode both -> clangd)
    // the reviewer's finding was about.
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'c-mode (list \"cat\")))",
    );
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'c++-mode (list \"cat\")))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'c++-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'c-mode)");
    set_frontend_started(&mut i);

    // c-mode buffer (current) triggers the autostart.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the triggering c-mode buffer itself must be marked"
    );

    // The c++-mode buffer, same project/command, must NOT be marked --
    // backfill will never attach it (exact-mode mismatch), so the
    // pending marker must not promise that it will.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "a same-command, different-mode buffer must not show LSP… \
         pending -- backfill will never attach it"
    );

    // Let the handshake complete and confirm backfill's own behavior
    // matches: c-mode attached, c++-mode buffer left alone.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--f7-conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "backfill must not attach the c++-mode buffer to a c-mode connection"
    );
    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    ok(&mut i, "(lsp-kill test--f7-conn)");
}

// ============================================================
// 15. F9 (medium): drive the REAL `core::idle_tick` (not manual elisp
//     calls in a test-chosen order) through the near-deadline race the
//     design depends on -- the pump (`lsp-process-pending-all`) runs
//     before the reap step (`lsp--autostart-tick`) inside `idle_tick`,
//     so a handshake whose deadline has already technically passed by
//     the time its reply is sitting in the connection's channel is
//     still completed, not reaped out from under itself.
//
//     The deadline is forced into the past directly (rather than raced
//     against real wall-clock timing) so this is deterministic, not
//     flaky: what's under test is ORDERING WITHIN ONE `idle_tick` CALL,
//     not real-time scheduling.
// ============================================================

#[test]
fn idle_tick_completes_a_near_deadline_handshake_before_reaping_it() {
    let mut i = setup();
    let dir = scratch_dir("f9_idle_tick_ordering");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");

    // The real Rust entry point, not `(setq lsp--frontend-started t)`
    // by hand.
    core::frontend_started(&mut i);

    // First real idle_tick: no pending entry yet, so this begins the
    // autostart (lsp--autostart-tick runs as part of it).
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");

    // Let cat actually echo the initialize request back into the
    // connection's channel.
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Force the deadline into the past -- "just barely made its
    // deadline" without depending on real-time scheduling to land the
    // race exactly right.
    ok(
        &mut i,
        "(setf (nth 2 (cdr (car lsp--autostart-pending))) (- (float-time) 5))",
    );

    // One more real idle_tick: `lsp-process-pending-all` (called first,
    // inside `idle_tick`) must drain and dispatch the already-buffered
    // reply -- completing the handshake -- BEFORE `lsp--autostart-tick`
    // (called after it, also inside `idle_tick`) ever gets a chance to
    // reap the (by-then-already-past-deadline) entry.
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    assert_eq!(
        run(&mut i, "lsp--autostart-pending"),
        "nil",
        "the handshake must have completed (removing the pending entry)"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-tried"),
        "nil",
        "it must NOT have been reaped -- if the ordering were reversed, \
         this would show one give-up entry instead"
    );
    assert_ne!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the buffer must have been attached by the completion's backfill"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 16. F10 (low): the "spawned fine, then died before the deadline"
//     reap branch (`(not (lsp-live-p conn))`) is distinct from both
//     "spawn failed outright" (test 3) and "deadline passed while
//     alive" (test 2) -- exercise it on its own, with a deadline set
//     far enough out that only the liveness check can be what reaps it.
// ============================================================

#[test]
fn dead_process_before_deadline_is_reaped_via_liveness_not_deadline() {
    let mut i = setup();
    let dir = scratch_dir("f10_dead_not_deadline");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    // Spawns successfully, then exits immediately -- distinct from a
    // missing binary (test 3, lsp-start itself fails) and distinct from
    // a deaf server that stays alive (test 2, only the deadline reaps
    // it).
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"sh\" \"-c\" \"exit 1\")))",
    );
    // Deliberately huge: if this test passes, the deadline branch is
    // structurally incapable of having fired -- only liveness could
    // have reaped the entry.
    ok(&mut i, "(setq lsp-autostart-timeout 100)");
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    core::frontend_started(&mut i);

    // Through the real `core::idle_tick`, not manual elisp calls: the
    // `lsp-live-p` check in `lsp--autostart-tick`'s reap loop only
    // reflects reality once the connection has actually been POLLED at
    // least once since the child died -- `LspConnection::is_alive` is a
    // cached flag flipped by the reader thread's `Died` event, observed
    // only via `lsp-poll`/`lsp-wait`, not by any synchronous OS-level
    // check. `idle_tick` calls `lsp-process-pending-all` (which polls
    // every live client, this one included) before `lsp--autostart-
    // tick` runs its reap loop -- driving this test through `idle_tick`
    // rather than `(lsp--autostart-tick)` alone is what makes the
    // liveness check actually see the death, exactly as production
    // does every tick.
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");

    // Give the child a moment to actually exit.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let before = std::time::Instant::now();
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    let elapsed = before.elapsed();

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("gave up"),
        "expected a give-up message, got: {}",
        messages
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "reap took {:?}, nowhere near instant -- suspicious given a \
         100s deadline that structurally cannot have been the cause",
        elapsed
    );
}

// ============================================================
// 17. G2 (fix round 3): the pending marker must be set AND cleared on
//     the non-current buffer, not just the one that happens to be
//     current when `lsp--autostart-mark-pending` runs. Every earlier
//     test with two buffers sharing a pending entry (1, 4, 14) only
//     ever inspected `lsp--autostart-pending-here` in whichever buffer
//     was current at the time -- this one switches buffers specifically
//     to read the OTHER one, both while pending and after it clears.
//     Mutation this must catch: restricting the clear (or the mark) to
//     `(eq buf (current-buffer))'.
// ============================================================

#[test]
fn pending_marker_is_set_and_cleared_on_the_non_current_buffer_too() {
    let mut i = setup();
    let dir = scratch_dir("g2_non_current_marker");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // b.rs opened first -- it will be the NON-current buffer for the
    // rest of this test.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    // a.rs opened second and left current -- triggers the autostart.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the current (triggering) buffer must be marked"
    );

    // Switch to b.rs (already open, no hook re-run) and read ITS marker
    // while the handshake is still pending.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    let b_pending_marker = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        b_pending_marker, "nil",
        "the NON-current buffer (b.rs) must also be marked while pending"
    );
    assert!(
        b_pending_marker.contains("cat"),
        "b.rs's pending marker should carry the command: {}",
        b_pending_marker
    );

    // Switch back to a.rs (current, so dispatch_one_pending's own
    // (nth 1 (cdr (car lsp--autostart-pending))) lookup is unaffected by
    // which buffer is current -- it reads the pending alist, not any
    // buffer-local state) and let the handshake complete.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "a.rs's own marker must have cleared"
    );
    ok(
        &mut i,
        "(setq test--g2-conn (lsp--client-conn lsp--buffer-client))",
    );

    // Switch to b.rs again and confirm ITS marker cleared too -- this is
    // the assertion the "restrict clear to (eq buf (current-buffer))"
    // mutation would falsify: b.rs was never current during the clear
    // call above, so a buggy clear would leave its marker stuck at
    // `("cat" . ROOT)' forever.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the NON-current buffer's marker must have cleared too"
    );

    ok(&mut i, "(lsp-kill test--g2-conn)");
}

// ============================================================
// 18. G3 (fix round 3): cancelling an in-flight autostart for ONE
//     project must not disturb a simultaneously pending autostart for a
//     DIFFERENT project. `lsp--autostart-cancel-pending' looks up an
//     exact `(command . root)' key; a mutation that widened this to
//     "just take the first pending entry" (or otherwise ignored ROOT)
//     would pass every other test here, since none of them have two
//     live pending entries under different keys at once.
// ============================================================

#[test]
fn cancelling_one_projects_autostart_leaves_a_different_projects_alone() {
    let mut i = setup();
    let root = scratch_dir("g3_two_projects");
    let dir1 = root.join("proj1");
    let dir2 = root.join("proj2");
    std::fs::create_dir_all(dir1.join(".git")).unwrap();
    std::fs::create_dir_all(dir2.join(".git")).unwrap();
    let file1 = dir1.join("a.rs");
    let file2 = dir2.join("b.rs");
    std::fs::write(&file1, "fn a() {}\n").unwrap();
    std::fs::write(&file2, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // Buffer 1: project 1, autostart begins FIRST -- its entry ends up
    // at the TAIL of `lsp--autostart-pending' (each new entry is
    // `push'ed onto the front).
    ok(
        &mut i,
        &format!("(find-file {:?})", file1.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--proj1-pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Buffer 2: a DIFFERENT project (different root), autostart begins
    // SECOND -- its entry ends up at the HEAD. Deliberately: this is
    // what makes the "just take the first pending entry" mutation
    // observable below -- cancelling project 1 (the one at the TAIL)
    // must not let a "take the head instead" bug pass by coincidentally
    // hitting the right entry anyway.
    ok(
        &mut i,
        &format!("(find-file {:?})", file2.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "2");
    let proj2_marker_before = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(proj2_marker_before, "nil");

    // `M-x lsp' in buffer 1's own project (the TAIL entry) must cancel
    // ONLY buffer 1's pending entry, not buffer 2's -- even though
    // buffer 2's entry is the one `car'/`(first ...)' would hand back.
    ok(
        &mut i,
        &format!("(find-file {:?})", file1.to_str().unwrap()),
    );
    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "(lsp) signaled: {}", r);
    assert_eq!(r, "\"LSP: connected to cat\"");

    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "exactly buffer 1's own pending entry must have been cancelled"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "buffer 1's own marker must be cleared (it's attached now)"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--proj1-pending-conn)"),
        "nil",
        "buffer 1's preempted autostart connection must be dead"
    );
    ok(
        &mut i,
        "(setq test--proj1-conn (lsp--client-conn lsp--buffer-client))",
    );

    // Buffer 2's pending entry, marker, and connection must be
    // untouched -- this is the assertion a "take the first entry"
    // mutation falsifies: it would have killed buffer 2's connection
    // (the one actually at the head) instead of buffer 1's.
    ok(
        &mut i,
        &format!("(find-file {:?})", file2.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "project 2's pending entry must still be there"
    );
    let proj2_marker_after = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        proj2_marker_after, "nil",
        "project 2's own marker must survive project 1's cancellation"
    );
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--proj2-pending-conn (nth 0 (cdr (assoc (cons \"cat\" (lsp--project-root (buffer-file-name))) lsp--autostart-pending))))",
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--proj2-pending-conn)"),
        "t",
        "project 2's autostart connection must still be alive"
    );

    // Let project 2's own handshake complete too, for cleanup and as a
    // final sanity check that it was never disturbed.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--proj2-conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(&mut i, "(lsp-kill test--proj1-conn)");
    ok(&mut i, "(lsp-kill test--proj2-conn)");
}

// ============================================================
// M94: a second server per buffer, routed by capability -- the
// autostart-side half. This is the test the silent-sink bug needed:
// before M94, `lsp--autostart-maybe-begin' short-circuited on `(not
// (lsp--live-buffer-client))', so the instant a primary attached, a
// SECONDARY (`lsp-secondary-server-alist') could never autostart for
// that buffer again, for the whole session, with no error and no
// message.
// ============================================================

#[test]
fn secondary_autostart_is_not_blocked_by_an_already_attached_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_secondary_not_blocked");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // The primary autostarts and completes first -- no secondary is
    // registered for rust-mode yet.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_eq!(
        run(&mut i, "(and (lsp--live-buffer-client) t)"),
        "t",
        "the primary must now be live-attached"
    );
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "1");

    // Register a SECONDARY for the same mode, a DIFFERENT command, and
    // tick again. The old `(not (lsp--live-buffer-client))' guard would
    // have skipped this call entirely from here on, forever, since the
    // primary is already live.
    ok(
        &mut i,
        "(add-to-list 'lsp-secondary-server-alist (cons 'rust-mode (list \"sh\" \"-c\" \"cat\")))",
    );
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "the secondary's own autostart must have begun despite the primary already being live"
    );
    assert_eq!(
        run(&mut i, "(car (car (car lsp--autostart-pending)))"),
        "\"sh\"",
        "the pending entry must be keyed on the SECONDARY's own command"
    );

    // Let it complete too: both clients end up attached, and the
    // primary is untouched.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");
    assert_eq!(
        run(
            &mut i,
            "(equal (lsp--client-command lsp--buffer-client) \"cat\")"
        ),
        "t",
        "the primary must still be the original cat client, not the secondary"
    );

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}

#[test]
fn secondary_autostart_short_circuits_immediately_for_a_mode_with_no_secondary_registered() {
    // The perf-sensitive fast path (M93): a mode with NO secondary
    // entry must still skip `lsp--project-root' entirely once the
    // primary is already live-attached -- M94 must not have regressed
    // that for every OTHER mode just to fix Verilog's.
    let mut i = setup();
    let dir = scratch_dir("m94_no_secondary_fast_path");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_eq!(run(&mut i, "(and (lsp--live-buffer-client) t)"), "t");

    ok(&mut i, "(setq test--project-root-calls 0)");
    ok(
        &mut i,
        "(setq test--orig-project-root (symbol-function 'lsp--project-root))",
    );
    ok(
        &mut i,
        "(fset 'lsp--project-root
               (lambda (f) (setq test--project-root-calls (1+ test--project-root-calls))
                 (funcall test--orig-project-root f)))",
    );
    ok(&mut i, "(lsp--autostart-tick)");
    ok(&mut i, "(fset 'lsp--project-root test--orig-project-root)");
    assert_eq!(
        run(&mut i, "test--project-root-calls"),
        "0",
        "no secondary registered for rust-mode -- the fast path must still skip \
         lsp--project-root entirely once the primary is already live"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// M94 review fix round 2 (AA1/AA4): `lsp--autostart-pending-here` is a
// LIST now, not a single slot -- two independent handshakes (primary
// and secondary) pending on the SAME buffer at once must not have one
// clobber the other's marker, and the secondary completing FIRST must
// not clear the marker while the primary is still in flight. This also
// covers AA4's second request (a test that reads the marker's actual
// VALUE after a secondary-only handshake, not just `lsp--autostart-
// pending'/`lsp--buffer-clients' the way the round-1 M94 tests did --
// see this test's own assertions on `lsp--autostart-pending-here'
// below; no separate test is added for that half of AA4).
// ============================================================

#[test]
fn pending_marker_holds_both_keys_and_only_fully_clears_once_both_complete() {
    let mut i = setup();
    let dir = scratch_dir("aa1_two_pending_markers");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);
    ok(
        &mut i,
        "(add-to-list 'lsp-secondary-server-alist (cons 'rust-mode (list \"sh\" \"-c\" \"cat\")))",
    );

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Both the primary and the secondary autostart in the SAME tick.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "2");

    // Both keys must be present in the marker at once -- the AA1 bug
    // was the second `lsp--autostart-mark-pending' call unconditionally
    // overwriting the first's key.
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending-here)"),
        "2",
        "both the primary's and the secondary's pending keys must coexist"
    );
    let commands = run(
        &mut i,
        "(sort (mapcar 'car lsp--autostart-pending-here) 'string<)",
    );
    assert_eq!(commands, "(\"cat\" \"sh\")");

    // The secondary was BEGUN second, so it's at the HEAD of
    // `lsp--autostart-pending' (each new entry is `push'ed onto the
    // front) -- dispatching the head completes the SECONDARY first,
    // exactly the ordering AA1 is about: nothing orders two independent
    // subprocess startups, and this project's own `cat`-echo transport
    // makes the second-begun one answer first deterministically.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);

    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the secondary must never take the primary slot"
    );
    let after_secondary = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        after_secondary, "nil",
        "the marker must still report pending -- the primary's own handshake \
         is still in flight -- not go fully dark the instant the secondary \
         (whichever completed first) finishes"
    );
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending-here)"),
        "1",
        "only the secondary's own key must have been removed"
    );
    assert_eq!(
        run(
            &mut i,
            "(equal (car (car lsp--autostart-pending-here)) \"cat\")"
        ),
        "t",
        "the surviving key must be the primary's"
    );

    // Let the primary complete too.
    std::thread::sleep(std::time::Duration::from_millis(300));
    dispatch_one_pending(&mut i);
    assert_ne!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the primary must now be attached"
    );
    assert_eq!(
        run(
            &mut i,
            "(equal (lsp--client-command lsp--buffer-client) \"cat\")"
        ),
        "t"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "both handshakes are done -- the marker must be fully clear now"
    );
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}
