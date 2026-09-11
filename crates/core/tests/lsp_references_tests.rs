//! M59: `lsp-references-at-point` (`textDocument/references`, bound
//! `M-?`). Same "fake the boundary, exercise our own wiring" discipline
//! as `lsp_action_tests.rs` (M48) -- `lsp-request-async`/
//! `completing-read` are stubbed, no real LSP server involved. Response
//! payload SHAPES (a `Location` vector, each with `uri`/`range`) mirror
//! the real `textDocument/references` LSP spec shape, matching what
//! `verible-verilog-ls` actually sent in the M59 pre-flight probe (see
//! PLAN.md) -- accurate but ALWAYS empty without a `verible.filelist`
//! in the project root, and never including the declaration site.

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
            "reticle_lsp_refs_{}_{}_{}",
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

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, content).unwrap();
    file
}

fn setup_client_buffer(interp: &mut Interp, file: &std::path::Path) {
    ok(interp, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(interp, "(setq client (make-lsp--client :conn nil))");
    ok(interp, "(setq-local lsp--buffer-client client)");
    ok(interp, "(setq test--synced nil)");
    ok(
        interp,
        "(fset 'lsp--sync-buffer-now (lambda () (setq test--synced t)))",
    );
}

fn capture_request_async(interp: &mut Interp) {
    ok(interp, "(setq test--captured nil)");
    ok(
        interp,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
}

fn capture_messages(interp: &mut Interp) {
    ok(interp, "(setq test--messages nil)");
    ok(
        interp,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

fn set_client_command(interp: &mut Interp, command: &str) {
    ok(
        interp,
        &format!("(setf (lsp--client-command client) {command:?})"),
    );
}

fn set_client_root(interp: &mut Interp, root: &std::path::Path) {
    ok(
        interp,
        &format!(
            "(setf (lsp--client-root client) {:?})",
            root.to_str().unwrap()
        ),
    );
}

fn stub_read_string_returning(interp: &mut Interp, value: &str) {
    ok(
        interp,
        &format!(
            "(fset 'read-string (lambda (prompt callback &optional initial) (funcall callback {value:?})))"
        ),
    );
}

fn invoke_captured_callback(interp: &mut Interp, reply_json: &str) {
    ok(
        interp,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply_json:?}))"),
    );
}

// ============================================================
// 1. request shape
// ============================================================

#[test]
fn sends_references_request_with_uri_position_and_include_declaration() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("req_shape");
    let file = write_file(&dir, "t.txt", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 7)"); // "world" starts at buffer pos 7
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/references\""
    );
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"uri\" (gethash \"textDocument\" (nth 2 test--captured)))"
        ),
        uri
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"line\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "0"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "6"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"includeDeclaration\" (gethash \"context\" (nth 2 test--captured)))"
        ),
        "t"
    );
}

// ============================================================
// 2/3/4. empty-result messages
// ============================================================

#[test]
fn empty_result_non_verilog_file_shows_plain_message_no_picker() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_plain");
    let file = write_file(&dir, "t.txt", "hello world\n");
    setup_client_buffer(&mut i, &file);
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    // M131: even a non-Verilog file's empty answer now names the root
    // that was actually used, so a narrow/misrooted answer is never
    // silently indistinguishable from "genuinely no references". Exact
    // comparison, same shape as 3 of its 4 sibling
    // `empty_result_verilog_file_*' tests below -- a dynamic root
    // still allows an exact match, and a loose `contains'-only check
    // would miss stray text appended after the root. (The 4th sibling,
    // `..._no_filelist_verible_command_names_verible_filelist_and_
    // root', uses `assert!(...contains...)' instead, checking the
    // `verible.filelist' clause and the root as two separate
    // substrings rather than one exact string.)
    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        format!("\"No references found (project root: {})\"", root)
    );
}

#[test]
fn empty_result_verilog_file_no_filelist_verible_command_names_verible_filelist_and_root() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_sv_no_filelist_verible");
    let file = write_file(&dir, "t.sv", "module t; endmodule\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "verible-verilog-ls");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains("verible.filelist"),
        "expected mention of verible.filelist, got {msg:?}"
    );
    assert!(
        msg.contains(&root),
        "expected the computed root {root:?} in message, got {msg:?}"
    );
}

#[test]
fn empty_result_verilog_file_names_the_filelist_ancestor_not_the_nearer_git() {
    // M93: dir/verible.filelist (outer) plus dir/sub/.git/ (nearer the
    // buffer). Before M93, `lsp--project-root' stopped at `sub' (the
    // nearer `.git'), so this message would have named `sub' even
    // though a real `verible.filelist' exists two directories up --
    // true but useless, since the file it points at is empty. After
    // M93 the same function call resolves to `dir', so the message
    // must name `dir', not `sub'.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_sv_filelist_outranks_git");
    std::fs::create_dir_all(dir.join("sub/.git")).unwrap();
    std::fs::write(dir.join("verible.filelist"), "sub/t.sv\n").unwrap();
    let file = write_file(&dir.join("sub"), "t.sv", "module t; endmodule\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "verible-verilog-ls");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    assert_eq!(
        root,
        dir.to_str().unwrap(),
        "lsp--project-root should have resolved to the filelist ancestor, not the nearer .git"
    );

    // The buffer's directory (`sub') is a naive nearest-marker walk's
    // answer; the actually-used root is `dir' itself, and the filelist
    // lives directly in it -- so verible.filelist WAS found, and the
    // message must name `dir' (M131) with no "no verible.filelist in
    // ..." clause (which would be a lie: the file is right there in
    // the root that was actually used), not the nearer `sub'.
    let msg = run(&mut i, "(car test--messages)");
    assert_eq!(
        msg,
        format!("\"No references found (project root: {})\"", root),
        "got {msg:?}"
    );
}

#[test]
fn empty_result_verilog_file_with_filelist_shows_plain_message() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_sv_with_filelist");
    let file = write_file(&dir, "t.sv", "module t; endmodule\n");
    write_file(&dir, "verible.filelist", "t.sv\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "verible-verilog-ls");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        format!("\"No references found (project root: {})\"", root)
    );
}

#[test]
fn empty_result_verilog_file_no_filelist_slang_command_shows_plain_message() {
    // M59 fix 1 (reviewer round): a slang-server-connected buffer must
    // NEVER be told to look for verible.filelist -- slang-server's own
    // empty-cross-file-reference failure mode is a wrong project root
    // (missing `.slang` marker), unrelated to a verible-only file.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_sv_no_filelist_slang");
    let file = write_file(&dir, "t.sv", "module t; endmodule\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "slang-server");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    let msg = run(&mut i, "(car test--messages)");
    assert_eq!(
        msg,
        format!("\"No references found (project root: {})\"", root),
        "got {msg:?}"
    );
    assert!(
        !msg.to_lowercase().contains("verible"),
        "a slang-server client's empty-result message must never mention verible, got {msg:?}"
    );
}

#[test]
fn empty_result_verilog_file_no_filelist_nil_command_shows_plain_message() {
    // M59 fix 1: an unknown client command (nil, e.g. a client that
    // predates the `command' field, or a bare test stub) must not be
    // guessed as verible -- silence over a wrong guess.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_sv_no_filelist_nil_cmd");
    let file = write_file(&dir, "t.sv", "module t; endmodule\n");
    setup_client_buffer(&mut i, &file);
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    let msg = run(&mut i, "(car test--messages)");
    assert_eq!(
        msg,
        format!("\"No references found (project root: {})\"", root),
        "got {msg:?}"
    );
}

#[test]
fn empty_result_vh_file_no_filelist_verible_command_names_verible_filelist() {
    // M59 fix 2 (reviewer round): `.vh' is part of the Verilog family
    // too (modes.el's `auto-mode-alist', verilog-auto.el's
    // `verilog-auto--library-file-name-p', verilog-nav.el's header all
    // agree on `.v'/`.vh'/`.sv'/`.svh') -- `lsp--verilog-buffer-p' had
    // dropped it.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_vh_no_filelist");
    let file = write_file(&dir, "t.vh", "`define X 1\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "verible-verilog-ls");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains("verible.filelist"),
        "expected mention of verible.filelist for a .vh file, got {msg:?}"
    );
}

// ============================================================
// 5. exactly one result -> direct jump, M-, returns
// ============================================================

#[test]
fn single_result_jumps_directly_and_m_dot_comma_returns() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("single");
    let file = write_file(&dir, "t.txt", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    // "world" starts at buffer position 7 (1-based).
    assert_eq!(run(&mut i, "(point)"), "7");

    ok(&mut i, "(lsp-pop-definition-stack)");
    assert_eq!(run(&mut i, "(point)"), "1");
}

// ============================================================
// 6. multiple results -> exact candidate list, sorted
// ============================================================

#[test]
fn multiple_results_build_exact_sorted_candidate_list() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("multi");
    let file_t = write_file(&dir, "t.sv", "hello world\n");
    write_file(&dir, "other.sv", "second reference here\n");
    setup_client_buffer(&mut i, &file_t);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let uri_other = run(
        &mut i,
        &format!(
            "(lsp--path-to-uri {:?})",
            dir.join("other.sv").to_str().unwrap()
        ),
    );
    let reply = format!(
        "[{{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}},\
          {{\"uri\":{uri_other},\"range\":{{\"start\":{{\"line\":0,\"character\":7}},\"end\":{{\"line\":0,\"character\":16}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    assert_eq!(
        run(&mut i, "(nth 2 test--cr-args)"),
        "t",
        "REQUIRE-MATCH must be t"
    );
    assert_eq!(
        run(&mut i, "(nth 1 test--cr-args)"),
        "(\"other.sv:1:8  second reference here\" \"t.sv:1:7  hello world\")"
    );
}

// ============================================================
// 7. same-file multiple hits -> file read once
// ============================================================

#[test]
fn same_file_multiple_hits_reads_the_file_only_once() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("dedup_read");
    let file = write_file(&dir, "t.sv", "hello world\nfoo bar\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback (car collection))))",
    );
    ok(
        &mut i,
        "(setq test--fc-orig (symbol-function 'file-contents-as-string))",
    );
    ok(&mut i, "(setq test--fc-calls 0)");
    ok(
        &mut i,
        "(fset 'file-contents-as-string
               (lambda (path)
                 (setq test--fc-calls (1+ test--fc-calls))
                 (funcall test--fc-orig path)))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}},\
          {{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":1,\"character\":0}},\"end\":{{\"line\":1,\"character\":3}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    assert_eq!(
        run(&mut i, "test--fc-calls"),
        "1",
        "two hits in the same file must read it exactly once"
    );
}

// ============================================================
// 8. unreadable target file/out-of-range line degrades gracefully
// ============================================================

#[test]
fn unreadable_target_degrades_to_no_snippet_without_aborting() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("unreadable");
    let file = write_file(&dir, "t.sv", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let missing_path = dir.join("does-not-exist.sv");
    let uri_missing = run(
        &mut i,
        &format!("(lsp--path-to-uri {:?})", missing_path.to_str().unwrap()),
    );
    let reply = format!(
        "[{{\"uri\":{uri_missing},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":1}}}}}},\
          {{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let collection = run(&mut i, "(nth 1 test--cr-args)");
    assert!(
        collection.contains("does-not-exist.sv:1:1\""),
        "missing file's candidate must have no snippet, got {collection:?}"
    );
    assert!(
        !collection.contains("does-not-exist.sv:1:1  "),
        "missing file's candidate must not have a trailing snippet separator, got {collection:?}"
    );
    assert!(
        collection.contains("t.sv:1:1  hello world"),
        "the readable candidate must still carry its snippet, got {collection:?}"
    );
}

// ============================================================
// 9. UTF-16 correctness
// ============================================================

#[test]
fn utf16_character_offset_lands_on_the_correct_column() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("utf16");
    // U+1F600 (an astral-plane emoji, 2 UTF-16 code units) then a space
    // then "hello" -- LSP `character` 3 (2 units for the emoji + 1 for
    // the space) must land exactly on 'h', not one character short/long
    // the way the older, non-UTF-16-aware `lsp--pos-at' would (it
    // treats `character' as a raw buffer-char count: `forward-char 3'
    // from line start lands on 'e', one past 'h').
    let file = write_file(&dir, "t.sv", "\u{1F600} hello\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":3}},\"end\":{{\"line\":0,\"character\":8}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    // Buffer position 3 (1-based): pos1 = emoji, pos2 = space, pos3 = 'h'.
    assert_eq!(run(&mut i, "(point)"), "3");
    assert_eq!(run(&mut i, "(char-after (point))"), "104"); // ?h
}

// ============================================================
// 10. two-layer staleness
// ============================================================

#[test]
fn stale_buffer_switch_before_reply_lands_does_nothing() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("stale_a");
    let file = write_file(&dir, "t.txt", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    // Switch away before the reply lands.
    ok(&mut i, "(get-buffer-create \"*scratch-elsewhere*\")");
    ok(&mut i, "(switch-to-buffer \"*scratch-elsewhere*\")");

    let file_uri = format!("\"file://{}\"", file.to_str().unwrap());
    let reply = format!(
        "[{{\"uri\":{file_uri},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch-elsewhere*\"");
    assert!(
        run(&mut i, "test--messages") == "nil",
        "a stale reply must not even message"
    );
}

#[test]
fn stale_buffer_switch_while_picker_open_does_not_jump_on_pick() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("stale_b");
    let file_t = write_file(&dir, "t.sv", "hello world\n");
    write_file(&dir, "other.sv", "second reference here\n");
    setup_client_buffer(&mut i, &file_t);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-callback nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-callback callback)))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let uri_other = run(
        &mut i,
        &format!(
            "(lsp--path-to-uri {:?})",
            dir.join("other.sv").to_str().unwrap()
        ),
    );
    let reply = format!(
        "[{{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}},\
          {{\"uri\":{uri_other},\"range\":{{\"start\":{{\"line\":0,\"character\":7}},\"end\":{{\"line\":0,\"character\":16}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    // Picker is "open" (callback captured, not yet invoked). Switch away.
    ok(&mut i, "(get-buffer-create \"*scratch-elsewhere*\")");
    ok(&mut i, "(switch-to-buffer \"*scratch-elsewhere*\")");

    // Now the user picks -- must NOT jump into t.sv/other.sv.
    ok(
        &mut i,
        "(funcall test--cr-callback \"t.sv:1:7  hello world\")",
    );

    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch-elsewhere*\"");
}

// ============================================================
// Reviewer round: malformed Location elements must be skipped, not
// abort the whole reply (fix 3), and a readable file whose reply LINE
// is past its own end must degrade the same way an unreadable file
// does (fix 4).
// ============================================================

#[test]
fn a_malformed_location_is_skipped_the_rest_of_the_reply_still_works() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("malformed_loc");
    let file_t = write_file(&dir, "t.sv", "hello world\n");
    write_file(&dir, "other.sv", "second reference here\n");
    setup_client_buffer(&mut i, &file_t);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let uri_other = run(
        &mut i,
        &format!(
            "(lsp--path-to-uri {:?})",
            dir.join("other.sv").to_str().unwrap()
        ),
    );
    // Middle element is malformed: `range' is a string, not a
    // hash-table -- `gethash "start"' on it would signal
    // wrong-type-argument if reached un-guarded.
    let reply = format!(
        "[{{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}},\
          {{\"uri\":{uri_other},\"range\":\"not-a-range\"}},\
          {{\"uri\":{uri_other},\"range\":{{\"start\":{{\"line\":0,\"character\":7}},\"end\":{{\"line\":0,\"character\":16}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    assert_eq!(
        run(&mut i, "(nth 1 test--cr-args)"),
        "(\"other.sv:1:8  second reference here\" \"t.sv:1:7  hello world\")",
        "the malformed middle element must be skipped, the other two kept"
    );
}

#[test]
fn a_line_number_past_a_readable_files_own_end_degrades_to_no_snippet() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("line_oob_readable");
    // Only 1 line -- line 5 in the reply is past its end, but the file
    // itself is perfectly readable (unlike the existing "unreadable
    // target" test, which hits the OTHER disjunct of
    // `lsp--reference-line-text''s `or').
    let file_t = write_file(&dir, "t.sv", "hello world\n");
    setup_client_buffer(&mut i, &file_t);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":5,\"character\":0}},\"end\":{{\"line\":5,\"character\":1}}}}}},\
          {{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let collection = run(&mut i, "(nth 1 test--cr-args)");
    assert!(
        collection.contains("t.sv:6:1\""),
        "the out-of-range line's candidate must have no snippet, got {collection:?}"
    );
    assert!(
        !collection.contains("t.sv:6:1  "),
        "the out-of-range candidate must not have a trailing snippet separator, got {collection:?}"
    );
    assert!(
        collection.contains("t.sv:1:1  hello world"),
        "the in-range candidate must still carry its snippet, got {collection:?}"
    );
}

// ============================================================
// Tail-review round, fix A: a non-empty reply where EVERY element is
// malformed must not fall into a picker with an empty, require-match-t
// collection (permanently unsubmittable, `C-g` only escape) -- it must
// message a distinct "no USABLE references" note and never call
// `completing-read' at all.
// ============================================================

#[test]
fn all_malformed_reply_messages_instead_of_opening_an_empty_picker() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("all_malformed");
    let file = write_file(&dir, "t.sv", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    // A non-empty vector (2 elements), but both malformed: `range' is a
    // string, not a hash-table.
    let reply = "[{\"uri\":\"file:///a.sv\",\"range\":\"nope\"},\
                   {\"uri\":\"file:///b.sv\",\"range\":\"also-nope\"}]";
    invoke_captured_callback(&mut i, reply);

    let msg = run(&mut i, "(car test--messages)");
    assert_eq!(
        msg, "\"No usable references here (2 malformed entries in the reply)\"",
        "got {msg:?}"
    );
    assert!(
        !msg.to_lowercase().contains("verible"),
        "this message's cause is a malformed reply, not a missing verible.filelist -- \
         must not reuse `lsp--references-empty-message', got {msg:?}"
    );
}

#[test]
fn malformed_plus_one_valid_still_jumps_directly_no_picker() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("malformed_plus_one");
    let file = write_file(&dir, "t.sv", "hello world\n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":\"file:///malformed.sv\",\"range\":\"nope\"}},\
          {{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    // "world" starts at buffer position 7 (1-based) -- the single valid
    // entry's own direct-jump path must still fire, not the picker.
    assert_eq!(run(&mut i, "(point)"), "7");
}

// ============================================================
// M131: the project-root label -- success/empty/rename -- and the
// outside-root advisory sweep.
// ============================================================

#[test]
fn references_success_message_names_the_root_actually_sent() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("success_names_root");
    let file_t = write_file(&dir, "t.sv", "hello world\n");
    write_file(&dir, "other.sv", "second reference here\n");
    setup_client_buffer(&mut i, &file_t);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_t = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let uri_other = run(
        &mut i,
        &format!(
            "(lsp--path-to-uri {:?})",
            dir.join("other.sv").to_str().unwrap()
        ),
    );
    let reply = format!(
        "[{{\"uri\":{uri_t},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}},\
          {{\"uri\":{uri_other},\"range\":{{\"start\":{{\"line\":0,\"character\":7}},\"end\":{{\"line\":0,\"character\":16}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let root: String = root.trim_matches('"').to_string();
    let prompt = run(&mut i, "(nth 0 test--cr-args)");
    assert!(
        prompt.contains("2 reference"),
        "expected the reference/file count in the picker prompt, got {prompt:?}"
    );
    assert!(
        prompt.contains(&format!("project root: {}", root)),
        "expected the actual root {root:?} in the picker prompt, got {prompt:?}"
    );
}

#[test]
fn references_root_label_uses_the_clients_stored_root_not_a_recomputation() {
    // The client's own STORED root (`lsp--client-root', M131 Part A)
    // deliberately differs from what a FRESH `lsp--project-root' call
    // for FILE would compute right now -- simulating a connection whose
    // `rootUri' was decided once, at `lsp-connect' time, and then
    // reused via `lsp--connections''s own `(COMMAND . ROOT)' key
    // (`lsp--get-connection') for as long as the connection lives. The
    // label must show the value ACTUALLY SENT to the server, never a
    // value recomputed after the fact.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("root_label_actual_a");
    let stored_root_dir = scratch_dir("root_label_stored_b");
    let file = write_file(&dir, "t.sv", "hello world\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &stored_root_dir);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":6}},\"end\":{{\"line\":0,\"character\":11}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let computed_root = run(&mut i, "(lsp--project-root (buffer-file-name))");
    let computed_root: String = computed_root.trim_matches('"').to_string();
    assert_ne!(
        computed_root,
        stored_root_dir.to_str().unwrap(),
        "test setup must actually diverge -- otherwise this test can't tell the two apart"
    );

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains(stored_root_dir.to_str().unwrap()),
        "expected the client's own STORED root, got {msg:?}"
    );
    assert!(
        !msg.contains(&computed_root),
        "must not silently fall back to a freshly recomputed root, got {msg:?}"
    );
}

#[test]
fn references_reports_verilog_files_outside_the_root_that_contain_the_name() {
    // repo/ (`.git' -- `search--find-root''s own scan boundary)
    //   proj/t.sv    -- the LSP project root actually used, contains
    //                   the symbol and is the only file in the reply.
    //   other/other.sv -- OUTSIDE proj, also mentions the same text,
    //                     but the (stubbed) server never reported it.
    let (mut i, _ed) = setup();
    let repo = scratch_dir("outside_sweep_repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let proj = repo.join("proj");
    let file = write_file(&proj, "t.sv", "WidgetName37 foo\n");
    write_file(&repo.join("other"), "other.sv", "WidgetName37 bar\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &proj);
    ok(&mut i, "(goto-char 1)"); // point is on "WidgetName37"
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":12}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains("1 Verilog file"),
        "expected the outside-root sweep to report exactly one file, got {msg:?}"
    );
    assert!(
        msg.contains("WidgetName37"),
        "expected the swept-for symbol named in the message, got {msg:?}"
    );
    assert!(
        msg.contains("search-project"),
        "expected the search-project escape hatch named, got {msg:?}"
    );
}

#[test]
fn references_outside_root_sweep_does_not_run_for_non_verilog_buffers() {
    // FIX-8 (M131 fix round): the QUERYING buffer here is `t.rs', not
    // Verilog -- `lsp--verilog-buffer-p' must gate the sweep off
    // entirely, even though the outside file below is real, is `.sv',
    // and genuinely does contain the swept-for text (same shape as
    // `references_reports_verilog_files_outside_the_root_that_contain_
    // the_name' above, but for a non-Verilog querying buffer). Without
    // the gate this would report "1 Verilog file..." despite the
    // symbol itself being a Rust identifier, not Verilog at all.
    let (mut i, _ed) = setup();
    let repo = scratch_dir("outside_sweep_non_verilog_buffer");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let proj = repo.join("proj");
    let file = write_file(&proj, "t.rs", "RustIdentifier55 foo\n");
    write_file(&repo.join("other"), "other.sv", "RustIdentifier55 bar\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &proj);
    ok(&mut i, "(goto-char 1)"); // point is on "RustIdentifier55"
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":16}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        !msg.contains("Verilog file"),
        "a non-Verilog querying buffer must never run the outside-root sweep \
         at all, even when a real matching Verilog file exists outside the \
         root, got {msg:?}"
    );
}

#[test]
fn references_outside_root_sweep_ignores_non_verilog_files() {
    // Same shape as the test above, except the OUTSIDE file this time
    // is `.md'/`.py'/`.el'/`.rs' -- none of them Verilog, all of them
    // must be excluded from the sweep's own count.
    let (mut i, _ed) = setup();
    let repo = scratch_dir("outside_sweep_non_verilog");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let proj = repo.join("proj");
    let file = write_file(&proj, "t.sv", "GadgetXyz99 foo\n");
    write_file(&repo.join("other"), "notes.md", "GadgetXyz99 in prose\n");
    write_file(&repo.join("other"), "gen.py", "# GadgetXyz99\n");
    write_file(&repo.join("other"), "helper.el", ";; GadgetXyz99\n");
    write_file(&repo.join("other"), "tests.rs", "// GadgetXyz99\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &proj);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":11}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        !msg.contains("Verilog file"),
        "no Verilog file exists outside the root -- the .md/.py/.el/.rs \
         hits must not be counted, got {msg:?}"
    );
}

#[test]
fn references_outside_root_sweep_never_adds_entries_to_the_result_list() {
    // The outside-root sweep is advisory-only text: even though
    // `other/other.sv' textually contains the same identifier, it must
    // NEVER show up as a candidate in the picker's own COLLECTION --
    // only the server's actual reply may do that.
    let (mut i, _ed) = setup();
    let repo = scratch_dir("outside_sweep_never_pollutes_list");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let proj = repo.join("proj");
    let file_a = write_file(&proj, "a.sv", "ThingCounter7 x\n");
    write_file(&proj, "b.sv", "ThingCounter7 y\n");
    write_file(&repo.join("other"), "other.sv", "ThingCounter7 z\n");
    setup_client_buffer(&mut i, &file_a);
    set_client_root(&mut i, &proj);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri_a = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let uri_b = run(
        &mut i,
        &format!(
            "(lsp--path-to-uri {:?})",
            proj.join("b.sv").to_str().unwrap()
        ),
    );
    let reply = format!(
        "[{{\"uri\":{uri_a},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":13}}}}}},\
          {{\"uri\":{uri_b},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":13}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let collection = run(&mut i, "(nth 1 test--cr-args)");
    assert!(
        !collection.contains("other.sv"),
        "the text-only outside-root hit must never appear in the actual \
         candidate list, got {collection:?}"
    );
    let msg = run(&mut i, "(nth 0 test--cr-args)");
    assert!(
        msg.contains("1 Verilog file"),
        "the sweep's own advisory count must still appear in the prompt, got {msg:?}"
    );
}

#[test]
fn references_symbol_at_point_includes_dollar_for_system_functions() {
    // FIX-2 (M131 fix round): `lsp--symbol-at-point' now reuses
    // `verilog-complete--ident-char-p' (`verilog-complete.el'), which
    // treats `$' as an identifier character -- SystemVerilog allows it
    // inside a simple identifier, and every system function/task
    // (`$clog2', `$bits', `$signed', ...) starts with one. Point on
    // `$clog2' below must sweep for the FULL `$clog2', never the
    // truncated `clog2' a plain `[A-Za-z0-9_]' class would produce.
    let (mut i, _ed) = setup();
    let repo = scratch_dir("symbol_dollar_sysfunc");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let proj = repo.join("proj");
    let file = write_file(&proj, "t.sv", "$clog2(WIDTH)\n");
    write_file(&repo.join("other"), "other.sv", "$clog2(DEPTH)\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &proj);
    ok(&mut i, "(goto-char 3)"); // inside "$clog2", on the "c"
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
    );

    ok(&mut i, "(lsp-references-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":6}}}}}}]"
    );
    invoke_captured_callback(&mut i, &reply);

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains("1 Verilog file"),
        "expected the outside file found via the FULL \"$clog2\" (proving `$' \
         was treated as part of the identifier), got {msg:?}"
    );
}

#[test]
fn references_symbol_at_point_expands_from_the_middle_or_end_of_the_identifier() {
    // FIX-4 (M131 fix round): every outside-root-sweep test above put
    // point at buffer position 1 -- the identifier's own LEFTMOST
    // character -- so `lsp--symbol-at-point''s backward-expansion loop
    // ((while (verilog-complete--ident-char-p (char-before))
    // (backward-char))) ran ZERO iterations in every one of them. Point
    // sitting in the MIDDLE, or right at the END, of an identifier --
    // the common real-world `M-?' invocation -- is what actually
    // exercises that loop. "MidEndIdent12" spans buffer positions 1-13
    // (1-based); position 7 sits in the middle (both loops run),
    // position 14 sits one past the last character (only the backward
    // loop runs; the forward loop sees the following space and stops
    // immediately) -- the exact opposite split from every test above.
    for point in [7, 14] {
        let (mut i, _ed) = setup();
        let repo = scratch_dir(&format!("symbol_mid_end_{point}"));
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let proj = repo.join("proj");
        let file = write_file(&proj, "t.sv", "MidEndIdent12 foo\n");
        write_file(&repo.join("other"), "other.sv", "MidEndIdent12 bar\n");
        setup_client_buffer(&mut i, &file);
        set_client_root(&mut i, &proj);
        ok(&mut i, &format!("(goto-char {point})"));
        capture_messages(&mut i);
        capture_request_async(&mut i);
        ok(
            &mut i,
            "(fset 'completing-read (lambda (&rest _) (error \"picker must not open\")))",
        );

        ok(&mut i, "(lsp-references-at-point)");
        let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
        let reply = format!(
            "[{{\"uri\":{uri},\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":13}}}}}}]"
        );
        invoke_captured_callback(&mut i, &reply);

        let msg = run(&mut i, "(car test--messages)");
        assert!(
            msg.contains("1 Verilog file"),
            "point={point}: expected the outside file actually found -- this only \
             happens if `lsp--symbol-at-point' recovered the FULL identifier \
             \"MidEndIdent12\", not a truncated substring, got {msg:?}"
        );
    }
}

#[test]
fn references_empty_message_names_the_root_for_a_non_verible_server() {
    // Death-code fix: the OLD three-way gate required COMMAND to name
    // verible before naming the root at ALL -- so a non-verible server
    // (slang-server, or any other) got nothing but the bare "No
    // references found", even though ROOT is exactly as useful (or
    // misleading) regardless of which server answered. Uses a root
    // that diverges from `lsp--project-root' (M131 Part A), so this
    // also confirms the empty path reads the STORED root, not a
    // recomputed one.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("empty_names_root_other_server");
    let stored_root_dir = scratch_dir("empty_names_root_other_server_stored");
    let file = write_file(&dir, "t.sv", "module t; endmodule\n");
    setup_client_buffer(&mut i, &file);
    set_client_command(&mut i, "slang-server");
    set_client_root(&mut i, &stored_root_dir);
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-references-at-point)");
    invoke_captured_callback(&mut i, "[]");

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains(stored_root_dir.to_str().unwrap()),
        "expected the client's own stored root named, got {msg:?}"
    );
    assert!(
        !msg.to_lowercase().contains("verible"),
        "a non-verible server's empty message must never mention verible, got {msg:?}"
    );
}

#[test]
fn rename_skipped_message_names_the_root() {
    // `lsp-rename''s own safety valve applies only to the current
    // buffer's edits and reports how many it skipped in other files
    // (`lsp.el''s `lsp-rename', "also skipped N edit(s)..." /
    // "touches other file(s) only" messages) -- that count is built
    // from a single buffer's own connection, which can be scoped
    // narrower than the real project, so an undercount must at least
    // be visible: the root actually used has to be named.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("rename_skip_names_root");
    let stored_root_dir = scratch_dir("rename_skip_names_root_stored");
    let file = write_file(&dir, "foo.sv", "foo foo\n");
    setup_client_buffer(&mut i, &file);
    set_client_root(&mut i, &stored_root_dir);
    ok(&mut i, "(goto-char 1)");
    stub_read_string_returning(&mut i, "bar");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "{{\"changes\":{{\
           {uri}:[{{\"newText\":\"bar\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":3}}}}}}],\
           \"file:///elsewhere/other.sv\":[{{\"newText\":\"x\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":1}}}}}}]\
         }}}}"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    let msg = run(&mut i, "(car test--messages)");
    assert!(
        msg.contains(stored_root_dir.to_str().unwrap()),
        "expected the client's own stored root named in the rename skip message, got {msg:?}"
    );
}
