//! M48: `lsp-code-action-at-point` (Part A/B) and `lsp-rename` (Part C),
//! plus the `C-c l` key bindings (Part D). No real LSP server involved
//! -- `lsp-request-async`/`completing-read`/`read-string` are stubbed,
//! same "fake the boundary, exercise our own wiring" discipline as
//! `lsp_format_tests.rs`'s M46 fix-round tests (see that file's own
//! comment on why `lsp-request-async` is stubbed rather than routed
//! through `lsp--await`).
//!
//! Response payload SHAPES here (codeAction's `edit`, rename's
//! `WorkspaceEdit`, a diagnostic's `range`/`severity`/`source`) mirror
//! what `verible-verilog-ls` actually sent in the M48 pre-flight probe
//! against the real binary (see PLAN.md), not a guess at the LSP spec.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
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
            "reticle_lsp_action_{}_{}_{}",
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

/// A file inside a `Scratch` directory -- derefs to the file's own path
/// (matching every existing call site's `file.to_str()`/`file.parent()`/
/// etc.), while the `Scratch` field's `Drop` cleans up the directory it
/// lives in when this value goes out of scope.
struct ScratchFile {
    file: std::path::PathBuf,
    _dir: Scratch,
}

impl std::ops::Deref for ScratchFile {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.file
    }
}

impl AsRef<std::path::Path> for ScratchFile {
    fn as_ref(&self) -> &std::path::Path {
        &self.file
    }
}

/// A scratch file containing "hello world\n" -- plain ASCII, so buffer
/// positions, LSP `character` offsets, and UTF-16 code units all
/// coincide, keeping every position in these tests a small hand-checked
/// number.
fn write_hello_world_scratch(tag: &str) -> ScratchFile {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello world\n").unwrap();
    ScratchFile { file, _dir: dir }
}

/// A scratch file containing "foo foo\n" -- two non-overlapping "foo"
/// occurrences (chars 0..3 and 4..7), the fixture `lsp-rename`'s
/// multi-edit tests apply a rename across.
fn write_foo_foo_scratch(tag: &str) -> ScratchFile {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "foo foo\n").unwrap();
    ScratchFile { file, _dir: dir }
}

/// Common setup shared by every `lsp-code-action-at-point`/`lsp-rename`
/// test: FILE opened, a REAL `lsp--client` struct (`:conn nil`) as
/// `lsp--buffer-client` -- unlike `lsp_format_tests.rs`'s bare-symbol
/// stand-ins, `lsp--diagnostics-at-point` calls `lsp-diagnostics`, which
/// reads `lsp--client-diagnostics` through the real `cl-defstruct`
/// accessor, so a non-struct value there would signal wrong-type --
/// and `lsp--sync-buffer-now` stubbed to a no-op flag-setter.
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

fn set_diagnostics(interp: &mut Interp, uri_expr: &str, diags_json: &str) {
    ok(
        interp,
        &format!(
            "(setf (lsp--client-diagnostics client)
                   (list (cons {uri_expr} (json-parse-string {diags_json:?}))))"
        ),
    );
}

// ============================================================
// Part A: lsp--diagnostics-at-point
// ============================================================

#[test]
fn diagnostics_at_point_containment_zero_width_and_half_open_boundary() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("diag_at_point");
    setup_client_buffer(&mut i, &file);

    // A covers "hello" (chars 0..5, i.e. buffer positions 1..6 -- point
    // 6, the space, is the exclusive END and must NOT be covered). B is
    // zero-width at char 11 (the newline) -- covers ONLY point 12.
    set_diagnostics(
        &mut i,
        "(lsp--path-to-uri (buffer-file-name))",
        "[{\"message\":\"A\",\"severity\":1,\
           \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}},\
          {\"message\":\"B\",\"severity\":1,\
           \"range\":{\"start\":{\"line\":0,\"character\":11},\"end\":{\"line\":0,\"character\":11}}}]",
    );

    ok(&mut i, "(goto-char 3)"); // inside "hello"
    assert_eq!(run(&mut i, "(length (lsp--diagnostics-at-point))"), "1");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"message\" (car (lsp--diagnostics-at-point)))"
        ),
        "\"A\""
    );

    ok(&mut i, "(goto-char 6)"); // the space right after "hello" -- END is exclusive
    assert_eq!(
        run(&mut i, "(lsp--diagnostics-at-point)"),
        "nil",
        "half-open range: END itself must not be covered"
    );

    ok(&mut i, "(goto-char 12)"); // the zero-width diagnostic's own position
    assert_eq!(run(&mut i, "(length (lsp--diagnostics-at-point))"), "1");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"message\" (car (lsp--diagnostics-at-point)))"
        ),
        "\"B\""
    );

    ok(&mut i, "(goto-char 13)"); // past everything
    assert_eq!(run(&mut i, "(lsp--diagnostics-at-point)"), "nil");
}

/// M48 fix round: every existing boundary test above hits at most one
/// diagnostic per point. `lsp--diagnostics-at-point` returns ALL
/// diagnostics covering point, in the server's own reply order --
/// `lsp--code-action-context` sends every one of them back to the
/// server, and `lsp--code-action-range` (a separate function, not
/// exercised here) takes only `(car diags)`. This test constructs two
/// overlapping diagnostics at the same point and pins down both halves
/// of that contract on `lsp--diagnostics-at-point` itself.
#[test]
fn diagnostics_at_point_returns_every_diagnostic_covering_an_overlapping_point_in_server_order() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("diag_at_point_overlap");
    setup_client_buffer(&mut i, &file);

    // A covers "hello" (chars 0..5); B covers "hell" (chars 0..4) --
    // both cover char 2 (buffer position 3). Server order is A, B.
    set_diagnostics(
        &mut i,
        "(lsp--path-to-uri (buffer-file-name))",
        "[{\"message\":\"A\",\"severity\":1,\
           \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}},\
          {\"message\":\"B\",\"severity\":1,\
           \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":4}}}]",
    );

    ok(&mut i, "(goto-char 3)"); // inside both ranges
    assert_eq!(run(&mut i, "(length (lsp--diagnostics-at-point))"), "2");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"message\" (nth 0 (lsp--diagnostics-at-point)))"
        ),
        "\"A\"",
        "server order must be preserved, not re-sorted"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"message\" (nth 1 (lsp--diagnostics-at-point)))"
        ),
        "\"B\""
    );
}

// ============================================================
// Part B: lsp-code-action-at-point
// ============================================================

/// M47's `lsp--symbol-alist` and M48's `lsp--code-action-alist` are
/// separately-written, structurally-identical functions -- see
/// `lsp_async_tests.rs`'s `symbol_alist_disambiguates_repeated_names_...`
/// for the sibling test. Every existing codeAction fixture in this file
/// uses distinct titles, so none of them exercises the " (N)" suffix
/// this function adds on a title collision. Pure function: fed
/// hand-built action hash-tables directly, no JSON round trip needed.
#[test]
fn code_action_alist_disambiguates_repeated_titles_and_each_resolves_to_its_own_action() {
    let (mut i, _ed) = setup();
    ok(
        &mut i,
        "(setq a1 (make-hash-table)) (puthash \"title\" \"Fix\" a1) (puthash \"id\" 1 a1)",
    );
    ok(
        &mut i,
        "(setq a2 (make-hash-table)) (puthash \"title\" \"Fix\" a2) (puthash \"id\" 2 a2)",
    );
    ok(
        &mut i,
        "(setq a3 (make-hash-table)) (puthash \"title\" \"Reformat\" a3) (puthash \"id\" 3 a3)",
    );
    ok(&mut i, "(setq actions (list a1 a2 a3))");
    ok(&mut i, "(setq alist (lsp--code-action-alist actions))");

    assert_eq!(
        run(&mut i, "(mapcar #'car alist)"),
        "(\"Fix (1)\" \"Fix (2)\" \"Reformat\")"
    );
    // Each disambiguated display resolves back to the ACTION it came
    // from (checked via the "id" field baked into each fixture), not
    // just the first "Fix".
    assert_eq!(
        run(&mut i, "(gethash \"id\" (cdr (assoc \"Fix (1)\" alist)))"),
        "1"
    );
    assert_eq!(
        run(&mut i, "(gethash \"id\" (cdr (assoc \"Fix (2)\" alist)))"),
        "2"
    );
    assert_eq!(
        run(&mut i, "(gethash \"id\" (cdr (assoc \"Reformat\" alist)))"),
        "3"
    );
}

#[test]
fn code_action_sends_diagnostic_range_and_context_when_point_is_on_a_diagnostic() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_params_hit");
    setup_client_buffer(&mut i, &file);
    set_diagnostics(
        &mut i,
        "(lsp--path-to-uri (buffer-file-name))",
        "[{\"message\":\"A\",\"severity\":1,\
           \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}}]",
    );
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");

    assert_eq!(run(&mut i, "test--synced"), "t");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/codeAction\""
    );
    let params = "(nth 2 test--captured)";
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"character\" (gethash \"start\" (gethash \"range\" {params})))")
        ),
        "0"
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"character\" (gethash \"end\" (gethash \"range\" {params})))")
        ),
        "5"
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(length (gethash \"diagnostics\" (gethash \"context\" {params})))")
        ),
        "1"
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(gethash \"message\" (aref (gethash \"diagnostics\" (gethash \"context\" {params})) 0))"
            )
        ),
        "\"A\""
    );
}

#[test]
fn code_action_falls_back_to_zero_width_range_and_empty_context_without_a_diagnostic() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_params_miss");
    setup_client_buffer(&mut i, &file);
    // No diagnostics registered at all.
    ok(&mut i, "(goto-char 3)"); // 0-based (line 0, char 2)
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");

    let params = "(nth 2 test--captured)";
    let start = format!("(gethash \"start\" (gethash \"range\" {params}))");
    let end = format!("(gethash \"end\" (gethash \"range\" {params}))");
    assert_eq!(run(&mut i, &format!("(gethash \"line\" {start})")), "0");
    assert_eq!(
        run(&mut i, &format!("(gethash \"character\" {start})")),
        "2"
    );
    assert_eq!(run(&mut i, &format!("(gethash \"line\" {end})")), "0");
    assert_eq!(run(&mut i, &format!("(gethash \"character\" {end})")), "2");
    assert_eq!(
        run(
            &mut i,
            &format!("(length (gethash \"diagnostics\" (gethash \"context\" {params})))")
        ),
        "0"
    );
}

/// M48 fix round: `lsp-code-action-at-point`'s `cond` has two
/// precondition branches before it ever touches the network -- no
/// client connected, and no file-name -- that no existing test in this
/// file reaches (`grep` for these two message strings turns up zero
/// hits before this test). Modeled on `lsp_mode_tests.rs`'s
/// `hover_at_point_reports_when_no_client_is_connected` and
/// `lsp_command_reports_no_file_without_signaling`.
#[test]
fn code_action_at_point_reports_when_no_client_is_connected() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_noclient");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    // No `lsp--buffer-client` set at all -- `lsp--live-buffer-client`
    // returns nil for it.
    assert_eq!(
        run(&mut i, "(lsp-code-action-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
}

#[test]
fn code_action_at_point_reports_when_buffer_is_not_visiting_a_file() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"*ca-nofile*\")");
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    assert_eq!(
        run(&mut i, "(lsp-code-action-at-point)"),
        "\"Buffer is not visiting a file\""
    );
}

#[test]
fn code_action_no_actions_messages_for_empty_and_null_replies() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_none");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured) (json-parse-string \"[]\"))",
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No code actions here\""
    );

    // A JSON-RPC error response (no "result" key at all) delivers Lisp
    // `nil` to the callback -- see the file header's M46 note on
    // `lsp--dispatch`. Defensive coverage for that shape too, even
    // though `(not (and (vectorp result) ...))` treats it identically
    // to `:null` below.
    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-code-action-at-point)");
    ok(&mut i, "(funcall (nth 3 test--captured) nil)");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No code actions here\""
    );

    // A successful reply whose `result` is the JSON literal `null`
    // parses (per `crates/elisp/src/json.rs`'s `from_json`) to the
    // symbol `:null`, NOT Lisp `nil` -- this is the shape a real
    // `textDocument/codeAction` reply of `null` actually takes on the
    // wire (a server that found nothing may legally reply this way
    // instead of an empty array).
    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-code-action-at-point)");
    ok(&mut i, "(funcall (nth 3 test--captured) :null)");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No code actions here\""
    );
}

#[test]
fn code_action_single_usable_action_applies_immediately_and_announces_its_title() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_single");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"kind\":\"quickfix\",\"isPreferred\":true,\
           \"edit\":{{\"changes\":{{{uri}:[\
             {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
           ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), "\"Hello world\\n\"");
    assert_eq!(run(&mut i, "(car test--messages)"), "\"Capitalize\"");
}

#[test]
fn code_action_multiple_usable_actions_open_a_picker_and_apply_the_pick() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_multi");
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
                 (funcall callback \"Shout\")))",
    );

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}},\
         {{\"title\":\"Shout\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"HELLO\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(
        run(&mut i, "(nth 1 test--cr-args)"),
        "(\"Capitalize\" \"Shout\")"
    );
    assert_eq!(
        run(&mut i, "(nth 2 test--cr-args)"),
        "t",
        "REQUIRE-MATCH must be t"
    );
    // The picked action ("Shout"), not the first one, was applied.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"HELLO world\\n\"");
    assert_eq!(run(&mut i, "(car test--messages)"), "\"Shout\"");
}

#[test]
fn code_action_elements_without_edit_are_skipped_and_counted() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_skip_edit");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    // A command-only element (no "edit") ahead of the one usable action.
    let reply = format!(
        "[{{\"title\":\"Run fixer\",\"command\":{{\"title\":\"Run fixer\",\"command\":\"verible.fix\"}}}},\
          {{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
            {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
          ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), "\"Hello world\\n\"");
    assert!(
        run(&mut i, "(car test--messages)").contains("Capitalize")
            && run(&mut i, "(car test--messages)").contains('1'),
        "expected the title plus a skipped-count note, got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn code_action_no_applicable_actions_when_every_element_lacks_an_edit() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_all_commands");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
                  (json-parse-string \"[{\\\"title\\\":\\\"Run fixer\\\",\\\"command\\\":{\\\"command\\\":\\\"x\\\"}}]\"))",
    );

    assert!(
        run(&mut i, "(car test--messages)").contains("No applicable"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn code_action_edit_touching_another_file_is_skipped_and_reported() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_other_file");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Rename module\",\"edit\":{{\"changes\":{{\
           {uri}:[{{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}],\
           \"file:///elsewhere/other.sv\":[{{\"newText\":\"x\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":1}}}}}}]\
         }}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    // This buffer's own edit still applied...
    assert_eq!(run(&mut i, "(buffer-string)"), "\"Hello world\\n\"");
    // ...and the other file's is only reported, not silently dropped.
    assert!(
        run(&mut i, "(car test--messages)").contains("other file"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn code_action_discards_a_stale_reply_if_the_buffer_changed_since_the_request() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_stale_tick");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"// typed while waiting\\n\")");
    let buffer_after_typing = run(&mut i, "(buffer-string)");

    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), buffer_after_typing);
    assert!(
        run(&mut i, "(car test--messages)").contains("stale")
            || run(&mut i, "(car test--messages)").contains("changed"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn code_action_first_staleness_layer_ignores_a_reply_delivered_in_a_different_buffer() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_stale_buf1");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-code-action-at-point)");
    ok(&mut i, "(generate-new-buffer \"*ca-other*\")");
    ok(&mut i, "(switch-to-buffer-internal \"*ca-other*\")");

    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    // buffer-file-name above was captured before switching, still the
    // scratch file's -- fine, only used to build the reply's uri.
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    // No message at all -- the reply was silently discarded, same
    // discipline as `lsp-format-buffer`'s own buffer-switch case.
    assert_eq!(run(&mut i, "test--messages"), "nil");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*ca-other*\"");
}

#[test]
fn code_action_second_staleness_layer_ignores_a_pick_made_after_switching_buffers() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_stale_buf2");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(generate-new-buffer \"*ca-other2*\")");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (switch-to-buffer-internal \"*ca-other2*\")
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}},\
         {{\"title\":\"Shout\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"HELLO\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-name)"), "\"*ca-other2*\"");
    ok(
        &mut i,
        &format!(
            "(switch-to-buffer-internal {:?})",
            file.file_name().unwrap().to_str().unwrap()
        ),
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        "\"hello world\\n\"",
        "must not have applied the pick made after switching away"
    );
}

/// M48 review fix-round gap: `code_action_discards_a_stale_reply_if_
/// the_buffer_changed_since_the_request` above only exercises the
/// SINGLE-action path, where there is no delay between the reply
/// landing and `lsp--apply-code-action` running -- "check at the top
/// of the callback" and "check right before applying, inside
/// `lsp--apply-code-action`" behave identically there. And
/// `..._second_staleness_layer_ignores_a_pick_made_after_switching_
/// buffers` above only exercises a buffer SWITCH, which the picker's
/// own `(eq (current-buffer) buf)` catches before `lsp--apply-code-
/// action` is ever reached, so the tick check inside it never fires
/// either. Neither test can tell "checked before applying" apart from
/// "checked at the top of the callback, before the picker even opens".
///
/// This test can: it types into the SAME buffer (no switch) from
/// inside the `completing-read` stub, after the reply has landed but
/// before the pick is applied. If the tick check were hoisted to the
/// top of `lsp-code-action-at-point`'s callback (before the picker
/// runs), it would see the old, still-current tick and let the pick
/// through; only a check placed where it actually is today --
/// immediately before the write, inside `lsp--apply-code-action` --
/// catches typing that happens during the picker itself.
#[test]
fn code_action_second_staleness_layer_catches_typing_in_the_same_buffer_during_the_picker() {
    let (mut i, _ed) = setup();
    let file = write_hello_world_scratch("ca_stale_typed_same_buf");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (goto-char (point-max))
                 (insert \"// typed during picker\\n\")
                 (funcall callback (car collection))))",
    );

    ok(&mut i, "(lsp-code-action-at-point)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "[{{\"title\":\"Capitalize\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"Hello\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}},\
         {{\"title\":\"Shout\",\"edit\":{{\"changes\":{{{uri}:[\
           {{\"newText\":\"HELLO\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":5}}}}}}\
         ]}}}}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    // The typed line is still there and untouched by the pick -- the
    // edit was discarded, not applied on top of the fresh text.
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        "\"hello world\\n// typed during picker\\n\""
    );
    assert!(
        run(&mut i, "(car test--messages)").contains("stale")
            || run(&mut i, "(car test--messages)").contains("changed"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

// ============================================================
// Part C: lsp-rename
// ============================================================

fn stub_read_string_returning(interp: &mut Interp, name: &str) {
    ok(
        interp,
        &format!(
            "(fset 'read-string
                   (lambda (prompt callback &optional initial)
                     (funcall callback {name:?})))"
        ),
    );
}

/// M48 fix round: same precondition-branch gap as `lsp-code-action-at-
/// point`'s two tests above -- `lsp-rename`'s own `cond` has the
/// identical two branches before `with-read-string` is even reached,
/// and no existing test in this file hits either.
#[test]
fn rename_reports_when_no_client_is_connected() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_noclient");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    assert_eq!(
        run(&mut i, "(lsp-rename)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
}

#[test]
fn rename_reports_when_buffer_is_not_visiting_a_file() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"*rn-nofile*\")");
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    assert_eq!(
        run(&mut i, "(lsp-rename)"),
        "\"Buffer is not visiting a file\""
    );
}

/// M48 fix round: `lsp-goto-symbol-by-name` has a FIRST staleness
/// check right when the picker opens (`(eq (current-buffer) buf)`
/// guarding whether to open it at all) in addition to its second one
/// inside the callback -- and `lsp-code-action-at-point` has the
/// equivalent pair, both covered by tests in this file. `lsp-rename`'s
/// analogous first check is the `(eq (current-buffer) buf)` guarding
/// whether `with-read-string`'s own callback does anything at all
/// (BUF captured before `with-read-string` opens, per the function's
/// own docstring) -- none of `lsp-rename`'s six existing tests stub
/// `read-string` to switch buffers before calling back, so that guard
/// has never been exercised.
#[test]
fn rename_first_staleness_layer_ignores_a_name_submitted_after_switching_buffers() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_stale_buf1");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);
    ok(&mut i, "(generate-new-buffer \"*rn-other*\")");
    ok(
        &mut i,
        "(fset 'read-string
               (lambda (prompt callback &optional initial)
                 (switch-to-buffer-internal \"*rn-other*\")
                 (funcall callback \"bar\")))",
    );

    ok(&mut i, "(lsp-rename)");

    // The guard fired before `lsp--sync-buffer-now`/`lsp-request-async`
    // ever ran -- no request was sent, and nothing was applied.
    assert_eq!(run(&mut i, "test--captured"), "nil");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*rn-other*\"");
    ok(
        &mut i,
        &format!(
            "(switch-to-buffer-internal {:?})",
            file.file_name().unwrap().to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-string)"), "\"foo foo\\n\"");
}

#[test]
fn rename_sends_position_params_and_new_name() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_params");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 2)"); // 0-based (line 0, char 1), inside the first "foo"
    stub_read_string_returning(&mut i, "bar");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");

    assert_eq!(run(&mut i, "test--synced"), "t");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/rename\""
    );
    let params = "(nth 2 test--captured)";
    assert_eq!(
        run(&mut i, &format!("(gethash \"newName\" {params})")),
        "\"bar\""
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"line\" (gethash \"position\" {params}))")
        ),
        "0"
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"character\" (gethash \"position\" {params}))")
        ),
        "1"
    );
}

#[test]
fn rename_applies_multiple_edits_and_reports_the_count() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_multi");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    stub_read_string_returning(&mut i, "bar");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");
    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "{{\"changes\":{{{uri}:[\
           {{\"newText\":\"bar\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":3}}}}}},\
           {{\"newText\":\"bar\",\"range\":{{\"start\":{{\"line\":0,\"character\":4}},\"end\":{{\"line\":0,\"character\":7}}}}}}\
         ]}}}}"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), "\"bar bar\\n\"");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Renamed 2 occurrence(s)\""
    );
}

#[test]
fn rename_empty_changes_and_null_result_both_message_not_available() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_empty");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    stub_read_string_returning(&mut i, "bar");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured) (json-parse-string \"{\\\"changes\\\":{}}\"))",
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Rename not available here\""
    );

    // A JSON-RPC error response (no "result" key) delivers Lisp `nil`
    // to the callback -- see the file header's M46 note on
    // `lsp--dispatch`. Defensive coverage for that shape too.
    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-rename)");
    ok(&mut i, "(funcall (nth 3 test--captured) nil)");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Rename not available here\""
    );

    // A successful reply of the JSON literal `null` parses to the
    // symbol `:null` (per `crates/elisp/src/json.rs`'s `from_json`),
    // not Lisp `nil` -- the actual wire shape of a `textDocument/
    // rename` reply that found nothing to rename.
    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-rename)");
    ok(&mut i, "(funcall (nth 3 test--captured) :null)");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Rename not available here\""
    );
}

#[test]
fn rename_edits_touching_only_another_file_are_reported_as_not_applied() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_other_only");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    stub_read_string_returning(&mut i, "bar");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");
    let reply = "{\"changes\":{\"file:///elsewhere/other.sv\":[\
        {\"newText\":\"x\",\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":1}}}\
      ]}}";
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), "\"foo foo\\n\"");
    assert!(
        run(&mut i, "(car test--messages)").contains("other file"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn rename_touching_this_file_and_another_applies_here_and_reports_the_skip() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_mixed");
    setup_client_buffer(&mut i, &file);
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

    assert_eq!(run(&mut i, "(buffer-string)"), "\"bar foo\\n\"");
    assert!(
        run(&mut i, "(car test--messages)").contains('1')
            && run(&mut i, "(car test--messages)").contains("other file"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn rename_discards_a_stale_reply_if_the_buffer_changed_since_the_request() {
    let (mut i, _ed) = setup();
    let file = write_foo_foo_scratch("rn_stale");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    stub_read_string_returning(&mut i, "bar");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-rename)");
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"// typed while waiting\\n\")");
    let buffer_after_typing = run(&mut i, "(buffer-string)");

    let uri = run(&mut i, "(lsp--path-to-uri (buffer-file-name))");
    let reply = format!(
        "{{\"changes\":{{{uri}:[\
           {{\"newText\":\"bar\",\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":3}}}}}}\
         ]}}}}"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(run(&mut i, "(buffer-string)"), buffer_after_typing);
    assert!(
        run(&mut i, "(car test--messages)").contains("stale")
            || run(&mut i, "(car test--messages)").contains("changed"),
        "got {:?}",
        run(&mut i, "test--messages")
    );
}

// ============================================================
// Part D: C-c l key bindings
// ============================================================

#[test]
fn c_c_l_prefix_bindings_reach_every_m48_and_earlier_lsp_command() {
    let (mut i, ed) = setup();
    for (keys, fname) in [
        ("C-c l a", "lsp-code-action-at-point"),
        ("C-c l r", "lsp-rename"),
        ("C-c l f", "lsp-format-buffer"),
        ("C-c l F", "lsp-format-region"),
        ("C-c l s", "lsp-goto-symbol-by-name"),
        ("C-c l n", "lsp-next-symbol"),
        ("C-c l p", "lsp-previous-symbol"),
    ] {
        ok(&mut i, "(setq test--ran nil)");
        ok(
            &mut i,
            &format!("(fset '{fname} (lambda (&rest _) (interactive) (setq test--ran t)))"),
        );
        feed_keys(&mut i, &ed, keys).unwrap_or_else(|e| panic!("feed_keys {keys:?}: {e}"));
        assert_eq!(run(&mut i, "test--ran"), "t", "{keys} must reach {fname}");
    }
}
