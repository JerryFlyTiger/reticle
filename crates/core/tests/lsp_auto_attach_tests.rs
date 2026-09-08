//! M63: automatic reuse of a live LSP connection for buffers other than
//! the one `M-x lsp' was run in -- `lsp-auto-attach', the pure judgment
//! function `lsp--auto-attach-client', the `find-file-hook' function
//! `lsp--maybe-auto-attach' (forward: buffers opened after a connection
//! already exists), and `lsp--auto-attach-backfill' (backward: buffers
//! that were already open before the first `M-x lsp' in the project).
//!
//! Same "cat" transport technique as `lsp_mode_tests.rs' (see that
//! file's own header and its M26 section): `cat' is registered as the
//! server command, `lsp--await' completes because `cat' echoes the
//! `initialize' request's id straight back, and `lsp-poll' drains the
//! didOpen frames the (fake) "server" echoed for inspection. This file
//! duplicates `lsp_mode_tests.rs's `scratch_dir'/`drain_frames' helpers
//! rather than sharing them across integration-test binaries (each
//! `tests/*.rs' file compiles to its own binary in this crate's test
//! harness).
//!
//! M123 fix round: this file's own `register_cat'/`lsp--await' path
//! DOES still depend on the echo, and was checked against M123 Part
//! A's dispatcher change rather than assumed safe just because the
//! suite stays green (`cargo test -p core --test lsp_auto_attach_tests'
//! run six times in a row, unfiltered, all 16/16) -- but it survives
//! for a reason worth recording, not because the defect `lsp_autostart_
//! tests.rs' hit doesn't apply here. It takes one extra round trip
//! now: `lsp--await' calls `lsp--dispatch' on every message it sees,
//! same as the async path. The FIRST message `cat' echoes back is the
//! real `initialize' request itself (id AND method both present) --
//! Part A's dispatcher correctly reads that as a server-initiated
//! REQUEST and answers it inline via `lsp--respond-to-request', which
//! sends a `MethodNotFound' error response (`{jsonrpc, id, error}', no
//! `method' key) back over the SAME connection. `cat' echoes THAT back
//! too, and this time it lands in the plain `id'-only branch (a real
//! response, no `method'), gets stashed in `lsp--client-pending', and
//! `lsp--await''s next poll finds it and returns `(gethash "result"
//! msg)' -- `nil', since an error response carries no `result' key.
//! That is the EXACT SAME `nil' `lsp--await' would have returned
//! before Part A existed (the OLD dispatcher's `id' branch would have
//! stashed the echoed `initialize' request itself, and `(gethash
//! "result" ...)' on a message with no `result' key is `nil' either
//! way) -- so every assertion in this file that only cares "did the
//! handshake complete, not what capabilities came back" is unaffected;
//! nothing here inspects `lsp--client-capabilities' or otherwise reads
//! meaning INTO that `nil'. Two hops through the connection instead of
//! one, same observable result -- which is also why `lsp_autostart_
//! tests.rs' broke and this file didn't: that file's ASYNC path
//! (`lsp-process-pending-all'/the autostart completion callback) reads
//! `(gethash "result" ...)' too, but only ever polls and dispatches
//! messages ONE AT A TIME across separate idle-tick-driven calls with
//! no blocking retry loop of its own -- there is no `lsp--await'-style
//! "keep going until MY id shows up" spin here to carry the completion
//! callback through that second hop, so the completion callback fires
//! (with a wrong-but-harmless nil-as-if-success reading) on the FIRST
//! echoed message and never sees the second at all... except Part A
//! changed what happens on THAT first message: it's now correctly
//! recognized as a request and answered instead of being handed to the
//! completion callback as if it were the reply, so the callback that
//! used to fire (wrongly, but effectively) on hop one now never fires
//! at all. Fixed in `lsp_autostart_tests.rs' by making the fake server
//! answer properly instead of echoing (see that file's own header).

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
            "reticle_lsp_auto_attach_{}_{}_{}",
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

/// A fresh scratch directory under the OS temp dir, unique per test run.
fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// Drain every frame `conn` has echoed back whose "method" is METHOD --
/// same technique as `lsp_mode_tests.rs's helper of the same name.
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

// ============================================================
// 1. find-file'ing a second buffer in the same project auto-attaches it.
// ============================================================

#[test]
fn find_file_in_same_project_auto_attaches_the_new_buffer() {
    let mut i = setup();
    let dir = scratch_dir("same_project");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        2
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 2. No connection ever started: auto-attach never runs `lsp' machinery.
// ============================================================

// M88: this only proves `find-file` itself never spawns -- `lsp--autostart-
// tick` (the idle-tick step M88 adds) is the thing that actually spawns a
// server now, and it is gated on `lsp--frontend-started`, which nothing in
// this test file's `setup()` ever sets. So read this test's name narrowly:
// "find-file never spawns", not "the editor never spawns".
#[test]
fn find_file_with_no_prior_connection_touches_nothing() {
    let mut i = setup();
    let dir = scratch_dir("no_connection");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "lsp--connections"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
}

// ============================================================
// 3. Different project root: no auto-attach across projects.
// ============================================================

#[test]
fn find_file_in_a_different_project_does_not_auto_attach() {
    let mut i = setup();
    let root = scratch_dir("cross_project");
    let dir1 = root.join("dir1");
    let dir2 = root.join("dir2");
    std::fs::create_dir_all(dir1.join(".git")).unwrap();
    std::fs::create_dir_all(dir2.join(".git")).unwrap();
    let file_a = dir1.join("a.rs");
    let file_c = dir2.join("c.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_c, "fn c() {}\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // Capture into a global before switching buffers -- lsp--buffer-client
    // is buffer-local, so a lazily-re-evaluated expression referencing it
    // would read the WRONG buffer's (nil) value once file_c is current.
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 4. Same directory, but a mode with no lsp-server-alist entry.
// ============================================================

#[test]
fn find_file_with_no_server_registered_for_its_mode_does_not_auto_attach() {
    let mut i = setup();
    let dir = scratch_dir("no_mode_entry");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_notes = dir.join("notes.txt");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_notes, "hello\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_notes.to_str().unwrap()),
    );
    // .txt has no auto-mode-alist entry, so it stays fundamental-mode,
    // which has no lsp-server-alist entry by default.
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 5. lsp-auto-attach nil disables the whole mechanism.
// ============================================================

#[test]
fn lsp_auto_attach_nil_disables_auto_attaching() {
    let mut i = setup();
    let dir = scratch_dir("auto_attach_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(&mut i, "(setq lsp-auto-attach nil)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 6. Manual `M-x lsp' in an already auto-attached buffer just reports.
// ============================================================

#[test]
fn manual_lsp_after_auto_attach_reports_already_connected_without_a_second_did_open() {
    let mut i = setup();
    let dir = scratch_dir("already_connected");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil"); // auto-attached

    // Manually re-running `lsp' in the auto-attached buffer must just
    // report, not re-didOpen.
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: already connected to cat\"");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        2 // one for a.rs (manual), one for b.rs (auto-attach) -- not 3
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 7. Guard order: lsp--connections nil short-circuits before
//    lsp--project-root is ever called.
// ============================================================

#[test]
fn no_connections_short_circuits_before_project_root_is_consulted() {
    let mut i = setup();
    let dir = scratch_dir("guard_order");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, "(setq test--project-root-calls 0)");
    ok(
        &mut i,
        "(fset 'lsp--project-root
               (lambda (f) (setq test--project-root-calls (1+ test--project-root-calls)) f))",
    );

    // No lsp--connections at all: the hook must bail out before ever
    // calling lsp--project-root.
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "test--project-root-calls"), "0");
}

// ============================================================
// 8. lsp--auto-attach-client is a pure function that rejects /ssh: paths.
// ============================================================

#[test]
fn auto_attach_client_rejects_a_remote_path_without_touching_any_buffer() {
    let mut i = setup();
    // A non-nil lsp--connections and a registered mode, so if the /ssh:
    // guard weren't checked before lsp--project-root, this would at
    // least reach that far (and, on a real /ssh: path, start shelling
    // out) -- the point of this test is that it never gets there.
    ok(
        &mut i,
        "(setq lsp--connections (list (cons (cons \"cat\" \"/whatever\") 'fake-client)))",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--auto-attach-client \"/ssh:h:/x/a.rs\" 'rust-mode)"
        ),
        "nil"
    );
}

// ============================================================
// 9. didOpen failing rolls the attach back to a clean nil state.
// ============================================================

#[test]
fn did_open_failure_rolls_back_buffer_client_and_synced_tick() {
    let mut i = setup();
    let dir = scratch_dir("did_open_fails");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");

    ok(
        &mut i,
        "(fset 'lsp-did-open (lambda (&rest _) (error \"boom\")))",
    );

    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "M-x lsp signaled: {}", r);
    assert!(r.contains("failed to start"), "message: {}", r);
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "lsp--last-synced-tick"), "nil");
}

// ============================================================
// 10. Backfill: buffers opened before `M-x lsp' get attached too.
// ============================================================

#[test]
fn lsp_backfills_buffers_already_open_before_it_was_run() {
    let mut i = setup();
    let dir = scratch_dir("backfill");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // Both buffers opened first, with no connection at all yet.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil"); // b.rs

    // Switch to a.rs and connect there.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    let conn_expr = "(lsp--client-conn lsp--buffer-client)".to_string();

    // b.rs, which was already open, must now be attached too.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(drain_frames(&mut i, &conn_expr, "textDocument/didOpen"), 2);

    ok(&mut i, &format!("(lsp-kill {})", conn_expr));
}

// ============================================================
// 11. Coordinator round 2, fix 1: the hook's own `let*` binding
//     (`lsp--auto-attach-client', which is NOT wrapped in a
//     `condition-case') must not let a malformed `lsp-server-alist'
//     entry leak an error out of `find-file' on every visit.
// ============================================================

#[test]
fn hook_swallows_a_malformed_server_alist_entry_without_signaling_or_attaching() {
    let mut i = setup();
    let dir = scratch_dir("bad_server_alist");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.badx");
    std::fs::write(&file, "hi\n").unwrap();

    // A mode whose `auto-mode-alist' function sets a mode symbol with a
    // MALFORMED lsp-server-alist entry: `(cons 'lsp-test--bad-mode
    // "my-cmd")' instead of `(cons 'lsp-test--bad-mode (list "my-cmd"))'
    // -- exactly the shape `lsp--server-for-mode' hands back as a bare
    // string, not a (COMMAND . ARGS) cons. `(car "my-cmd")' then signals
    // wrong-type-argument inside `lsp--auto-attach-client'.
    ok(
        &mut i,
        "(defun lsp-test--bad-mode () (major-mode-internal-set 'lsp-test--bad-mode))",
    );
    ok(
        &mut i,
        "(add-to-list 'auto-mode-alist '(\"\\\\.badx\\\\'\" . lsp-test--bad-mode))",
    );
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'lsp-test--bad-mode \"my-cmd\"))",
    );
    // A live connection so `lsp--connections' is non-nil (guard 2 must
    // NOT short-circuit before the malformed entry is reached, or this
    // repro doesn't exercise the bug at all).
    ok(
        &mut i,
        "(setq lsp--connections (list (cons (cons \"my-cmd\" \"/whatever\") 'fake-client)))",
    );

    // Capture the interpreter's raw output sink (what `run_hook_by_name'
    // writes hook errors to, via `Interp::out' -- NOT the same channel
    // as `Editor::echo'/`message', which M62's own precedent already
    // established `eval_source'-driven tests can't observe: `Editor::
    // echo' is only ever written from inside `call_command'. `Interp::
    // out' is different -- `run_hook_by_name' calls it directly, so it
    // IS visible here, and capturing it gives a real repro instead of
    // the weaker "did find-file signal" fallback.
    let captured = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let sink = captured.clone();
    i.output = Some(Box::new(move |s: &str| sink.borrow_mut().push_str(s)));

    let r = run(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    assert!(!r.starts_with("ERROR"), "find-file itself signaled: {}", r);
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    let out = captured.borrow().clone();
    // `find-file-hook' is the load-bearing needle: `run_hook_by_name'
    // formats an escaping hook error as "Error in {hook} ({fn}): ..."
    // (commands.rs), so the hook's NAME is what appears. The tail review
    // caught the first version of this assertion also testing for
    // "wrong-type-argument" -- that one is DEAD, because the printed
    // text comes from the error symbol's `error-message' property,
    // which reads "Wrong type argument" (spaces, capitalised), never the
    // symbol's own spelling. It was true whether the fix was in or out
    // and contributed nothing. Kept here as the actual on-screen text so
    // the assertion still covers both halves of the message.
    assert!(
        !out.contains("find-file-hook") && !out.contains("Wrong type argument"),
        "hook error leaked to output: {:?}",
        out
    );
}

// ============================================================
// 12. Coordinator round 2, fix 2: backfill's "already attached" guard
//     must not mistake a buffer pointing at a DEAD client for one that's
//     genuinely covered -- it should re-attach it to the fresh
//     connection instead of leaving it stuck on the corpse.
// ============================================================

#[test]
fn backfill_reattaches_a_buffer_whose_old_connection_died() {
    let mut i = setup();
    let dir = scratch_dir("backfill_dead_client");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // b.rs connects first and gets its own (first) connection, C1.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // Kill C1's underlying process/connection while staying in b.rs, so
    // `lsp--buffer-client' here is left pointing at a DEAD client --
    // exactly what `lsp--live-buffer-client' is for, and exactly what a
    // truthy-only guard (`(not lsp--buffer-client)') can't tell apart
    // from a live one.
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");

    // a.rs, opened AFTER C1 died: the hook's own lookup goes through
    // `lsp--get-connection', which prunes the dead entry and finds
    // nothing to reuse, so a.rs is NOT auto-attached here.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Manual `M-x lsp' in a.rs: no reusable entry (pruned above), so
    // this spawns a fresh connection C2 and attaches a.rs to it, then
    // backfills every other buffer C2 now covers -- including b.rs,
    // which is stuck on the corpse of C1.
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(&mut i, "(setq test--c2 lsp--buffer-client)");

    // b.rs must now point at C2, not still at dead C1. Revisiting b.rs
    // via `find-file' just switches buffers (already open, no hook
    // re-run) so this reads exactly the state backfill left behind.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--c2)"), "t");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--c2))");
}

// ============================================================
// 13. Coordinator round 2, fix 3: a strong observation point for the
//     `/ssh:' guard's position in `lsp--auto-attach-client''s guard
//     order -- a regressed order would still eventually return nil (the
//     existing test above still passes), but only after walking every
//     ancestor directory and shelling out `ssh' with a 5s ConnectTimeout
//     per marker. Counting `lsp--project-root' calls makes a regression
//     fail immediately instead of after tens of seconds of real ssh
//     attempts to a nonexistent host.
// ============================================================

#[test]
fn ssh_guard_short_circuits_before_project_root_is_consulted() {
    let mut i = setup();
    ok(
        &mut i,
        "(setq lsp--connections (list (cons (cons \"cat\" \"/whatever\") 'fake-client)))",
    );
    ok(&mut i, "(setq test--project-root-calls 0)");
    ok(
        &mut i,
        "(fset 'lsp--project-root
               (lambda (f) (setq test--project-root-calls (1+ test--project-root-calls)) f))",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--auto-attach-client \"/ssh:h:/x/a.rs\" 'rust-mode)"
        ),
        "nil"
    );
    assert_eq!(run(&mut i, "test--project-root-calls"), "0");
}

// ============================================================
// 14. Coordinator round 2, fix 4a: backfill must not re-didOpen a
//     buffer that a PRIOR hook attach (or prior backfill) already
//     covered -- no existing test triggers backfill more than once, so
//     this gap had no observation point at all.
// ============================================================

#[test]
fn backfill_does_not_redundantly_did_open_an_already_attached_buffer() {
    let mut i = setup();
    let dir = scratch_dir("backfill_dedup");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    let file_c = dir.join("c.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    std::fs::write(&file_c, "fn c() {}\n").unwrap();
    register_cat(&mut i);

    // a.rs connects manually.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // Stash the connection while a.rs is current rather than re-deriving
    // it from `lsp--buffer-client' later: that variable is buffer-local,
    // so a lazily-evaluated expression reads whichever buffer happens to
    // be current at the time. It would work out here (c.rs ends up on
    // the same connection) but only by accident, and this file's other
    // tests treat that as a trap to avoid rather than to rely on.
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    // b.rs, opened next, gets auto-attached by the `find-file-hook'
    // path (its first, and only expected, didOpen).
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");

    // c.rs is opened with auto-attach OFF, so the hook does nothing for
    // it -- this is what lets a later manual `(lsp)' there trigger a
    // FRESH (second) backfill sweep instead of just reporting "already
    // connected" the way it would if the hook had already attached it.
    ok(&mut i, "(setq lsp-auto-attach nil)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(&mut i, "(setq lsp-auto-attach t)");

    // This is the SECOND backfill sweep (the first ran inside the a.rs
    // `(lsp)' call above). It must attach c.rs but leave a.rs/b.rs,
    // already live-attached, alone.
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(&mut i, "test--conn", "textDocument/didOpen");
    // Count how many didOpen frames named b.rs's URI -- must be exactly
    // 1 (from the hook), not 2 (hook + redundant second backfill).
    let b_uri = ok(
        &mut i,
        &format!("(lsp--path-to-uri {:?})", file_b.to_str().unwrap()),
    );
    let n_src = format!(
        "(let ((n 0))
           (dolist (f test--frames)
             (when (equal (gethash \"uri\" (gethash \"textDocument\" (gethash \"params\" f))) {b_uri})
               (setq n (1+ n))))
           n)",
        b_uri = b_uri
    );
    assert_eq!(
        run(&mut i, &n_src),
        "1",
        "b.rs must be didOpen'd exactly once"
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 15. Coordinator round 2, fix 4b (cheap addition): `lsp-auto-attach'
//     nil must also suppress BACKFILL, not just the `find-file-hook'
//     path -- existing test 5 only covers the hook (the buffer it
//     checks was opened AFTER auto-attach was turned off).
// ============================================================

#[test]
fn lsp_auto_attach_nil_also_suppresses_backfill() {
    let mut i = setup();
    let dir = scratch_dir("backfill_auto_attach_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i);

    // Both buffers opened first, with no connection at all yet (so the
    // hook is a no-op for both regardless of `lsp-auto-attach').
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");

    ok(&mut i, "(setq lsp-auto-attach nil)");

    // Switch back to a.rs and connect -- with auto-attach off, this
    // must NOT backfill b.rs even though it's the same project/command.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 16. Backfill must compare each buffer's OWN major mode, not just
//     assume the connecting buffer's.
//
//     Added by the coordinator after the mutation run: reverting the
//     `(eq (major-mode-internal-get) mode)' line in
//     `lsp--auto-attach-backfill' SURVIVED the whole 15-test suite.
//     That line is load-bearing and nothing was watching it.
//
//     Why it matters rather than being merely untested: the sweep hands
//     `lsp--auto-attach-client' the CONNECTING buffer's MODE, not the
//     mode of the buffer it is currently looking at. So with that guard
//     gone the lookup answers "yes, rust-mode has a cat server and its
//     connection matches" for a plain-text buffer that happens to sit in
//     the same project, and backfill attaches it -- sending the server a
//     didOpen for a document it has no business parsing. The equivalent
//     hook path is safe because it reads the mode off the buffer being
//     visited (test 4 covers that); backfill is the one that has to be
//     explicit.
// ============================================================

#[test]
fn backfill_skips_buffers_whose_own_major_mode_has_no_server() {
    let mut i = setup();
    let dir = scratch_dir("backfill_other_mode");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let notes = dir.join("notes.txt");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&notes, "just some prose\n").unwrap();
    register_cat(&mut i);

    // notes.txt is open first, in a mode with no `lsp-server-alist'
    // entry, and with no connection existing yet.
    ok(
        &mut i,
        &format!("(find-file {:?})", notes.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Connect from a.rs in the same project: this runs backfill over
    // every buffer, notes.txt included.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // Stash the connection NOW, while a.rs is still current: the checks
    // below switch to a buffer whose `lsp--buffer-client' is (and must
    // stay) nil, so a lazily-evaluated `(lsp--client-conn
    // lsp--buffer-client)' would blow up there instead of measuring
    // anything.
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    // notes.txt must still be untouched.
    ok(
        &mut i,
        &format!("(find-file {:?})", notes.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "backfill attached a fundamental-mode buffer using the connecting \
         buffer's mode instead of this buffer's own"
    );

    // And the server saw exactly one didOpen -- a.rs's, not notes.txt's.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(&mut i, "test--conn", "textDocument/didOpen"),
        1
    );

    ok(&mut i, "(lsp-kill test--conn)");
}
