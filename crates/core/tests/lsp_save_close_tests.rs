//! M40 part 2: `textDocument/didSave`/`textDocument/didClose`, sent by
//! `lsp--on-after-save`/`lsp--on-kill-buffer` off the M40 save/kill
//! hooks (`after-save-hook`/`kill-buffer-hook`). Same cat-echo
//! technique as `lsp_mode_tests.rs`'s M26/M35 sections (a real, always-
//! spawnable subprocess that byte-for-byte echoes frames back so the
//! outbound JSON-RPC can be inspected) and the same setup/run/ok/
//! scratch_dir helpers.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
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
            "reticle_lsp_save_close_{}_{}_{}",
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

/// Drain every frame `conn` has echoed back, in arrival order, into
/// `test--frames` -- mirrors `lsp_mode_tests.rs`'s `drain_frames`/
/// `drain_methods` helpers, just keeping both the full message and the
/// arrival order in one pass since these tests check both the
/// didChange/didSave (or didClose) sequence and a frame's contents.
fn drain_all(i: &mut Interp, conn_expr: &str) {
    let src = format!(
        "(let ((frames nil) (msg t))
           (while msg
             (setq msg (lsp-poll {conn}))
             (when msg (setq frames (cons msg frames))))
           (setq test--frames (nreverse frames)))",
        conn = conn_expr,
    );
    ok(i, &src);
}

fn methods_of_test_frames(i: &mut Interp) -> String {
    run(
        i,
        "(mapcar (lambda (m) (gethash \"method\" m nil)) test--frames)",
    )
}

#[test]
fn save_buffer_sends_did_save_preceded_by_a_did_change_with_no_text_field() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("didsave_basic");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    // Drain the handshake's "initialized" notification and didOpen so
    // only the save's own frames remain.
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_all(&mut i, "(lsp--client-conn lsp--buffer-client)");
    assert_eq!(
        methods_of_test_frames(&mut i),
        "(\"initialized\" \"textDocument/didOpen\")"
    );

    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer signaled: {}", r);

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_all(&mut i, "(lsp--client-conn lsp--buffer-client)");
    assert_eq!(
        methods_of_test_frames(&mut i),
        "(\"textDocument/didChange\" \"textDocument/didSave\")",
        "didSave must be sent exactly once, preceded by the didChange \
         that syncs the unsaved edit"
    );

    let expected_uri = ok(
        &mut i,
        &format!("(lsp--path-to-uri {:?})", file.to_str().unwrap()),
    );
    assert_eq!(
        ok(
            &mut i,
            "(gethash \"uri\" (gethash \"textDocument\" (gethash \"params\" (nth 1 test--frames))))"
        ),
        expected_uri,
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"text\" (gethash \"params\" (nth 1 test--frames)))"
        ),
        "nil",
        "v1 didSave must carry no text/includeText"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn kill_buffer_sends_did_close_with_the_right_uri() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("didclose_basic");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // lsp--buffer-client is buffer-local: stash the raw connection in a
    // global now so it's still reachable after the buffer (and its
    // buffer-local binding) is killed below.
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_all(&mut i, "test--conn");
    assert_eq!(
        methods_of_test_frames(&mut i),
        "(\"initialized\" \"textDocument/didOpen\")"
    );

    ok(&mut i, "(kill-buffer)");

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_all(&mut i, "test--conn");
    assert_eq!(
        methods_of_test_frames(&mut i),
        "(\"textDocument/didClose\")"
    );
    let expected_uri = ok(
        &mut i,
        &format!("(lsp--path-to-uri {:?})", file.to_str().unwrap()),
    );
    assert_eq!(
        ok(
            &mut i,
            "(gethash \"uri\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))"
        ),
        expected_uri,
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

#[test]
fn kill_buffer_drops_the_uri_from_the_clients_diagnostics_alist() {
    // M40-4 review issue #6: `lsp--on-kill-buffer' (lsp.el) claims that
    // didClose also `delq's the closed buffer's URI out of the client's
    // `lsp--client-diagnostics' alist -- pin that down; nothing
    // previously asserted it.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("didclose_diagnostics");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    // lsp--buffer-client is buffer-local: stash the client struct and
    // this buffer's URI in globals now so both are still reachable
    // after the buffer (and its buffer-local binding) is killed below.
    ok(&mut i, "(setq test--client lsp--buffer-client)");
    ok(
        &mut i,
        "(setq test--uri (lsp--path-to-uri (buffer-file-name)))",
    );

    // Same shape a real publishDiagnostics notification leaves behind
    // (see lsp_mode_tests.rs's diagnostic_navigation_jumps_... test).
    ok(
        &mut i,
        r#"(setf (lsp--client-diagnostics test--client)
               (list (cons test--uri
                           (json-parse-string "[{\"range\":{\"start\":{\"line\":0,\"character\":0}},\"message\":\"boom\"}]"))))"#,
    );
    assert_ne!(
        run(
            &mut i,
            "(assoc test--uri (lsp--client-diagnostics test--client))"
        ),
        "nil",
        "sanity: the diagnostics alist entry must exist before the kill"
    );

    ok(&mut i, "(kill-buffer)");

    assert_eq!(
        run(
            &mut i,
            "(assoc test--uri (lsp--client-diagnostics test--client))"
        ),
        "nil",
        "lsp--on-kill-buffer must delq the closed buffer's URI out of \
         the client's diagnostics alist"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

#[test]
fn ordinary_buffer_save_and_kill_without_a_client_are_unaffected() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("no_client");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", file.to_str().unwrap()),
    );
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer signaled: {}", r);
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.starts_with("Wrote "),
        "echo polluted by the LSP save hook: {:?}",
        echo
    );

    let r = run(&mut i, "(kill-buffer)");
    assert!(!r.starts_with("ERROR"), "kill-buffer signaled: {}", r);
}

#[test]
fn save_buffer_does_not_signal_after_the_server_process_has_died() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("dead_client");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");

    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer signaled: {}", r);

    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(
        on_disk.contains("fn b() {}"),
        "save must still succeed once the server is dead: {:?}",
        on_disk
    );
}
