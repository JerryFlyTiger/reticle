//! M49: `textDocument/documentHighlight` -- `lsp-highlight-at-point`
//! (`C-c l h`), `lsp-highlight-clear` (`C-c l H`), `lsp-next-highlight`/
//! `lsp-previous-highlight` (`C-c l N`/`C-c l P`). Same "fake the
//! boundary, exercise our own wiring" discipline as `lsp_action_tests.rs`
//! -- `lsp-request-async` is stubbed and its captured callback invoked
//! by hand with a fabricated reply, no real LSP server involved.
//!
//! Response shape (a bare array of `{"range": {...}}`, no `kind`) mirrors
//! what `verible-verilog-ls` actually sent in the M49 pre-flight probe
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
            "reticle_lsp_highlight_{}_{}_{}",
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

/// A scratch file containing "hello world\n" -- plain ASCII, so buffer
/// positions, LSP `character` offsets, and UTF-16 code units all
/// coincide.
fn write_hello_world_scratch(tag: &str) -> (Scratch, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello world\n").unwrap();
    (dir, file)
}

/// A scratch file containing "\u{1F600}foo bar\n" -- an astral-plane
/// emoji (U+1F600, 1 Unicode scalar value but 2 UTF-16 code units)
/// followed by "foo bar". Buffer positions (1-based) are:
///   1: the emoji   2: f   3: o   4: o   5: (space)   6: b  7: a  8: r  9: \n
/// Scalar-value LSP `character` offsets (this editor's native, WRONG
/// per the LSP spec) count the emoji as 1 unit; UTF-16 offsets (correct)
/// count it as 2 -- the two disagree by exactly 1 for every position
/// after the emoji, which is what the UTF-16 coordinate tests below
/// pin down. "foo" occupies buffer positions 2..5 (chars 2,3,4), i.e.
/// UTF-16 characters 2..5 on line 0 (2 for the emoji + 0/1/2 for how
/// far into "foo").
fn write_emoji_scratch(tag: &str) -> (Scratch, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "\u{1F600}foo bar\n").unwrap();
    (dir, file)
}

/// Common setup shared by every test in this file: FILE opened, a REAL
/// `lsp--client` struct (`:conn nil`) as `lsp--buffer-client`, and
/// `lsp--sync-buffer-now` stubbed to a no-op flag-setter -- same shape
/// as `lsp_action_tests.rs`'s `setup_client_buffer`.
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

/// (start . end) buffer-position pairs of every `'lsp-highlight'-tagged
/// overlay in the current buffer, ascending by start.
fn highlight_overlay_spans(interp: &mut Interp) -> String {
    run(
        interp,
        "(sort (let (out)
                 (dolist (ov (overlays-in (point-min) (point-max)))
                   (when (overlay-get ov 'lsp-highlight)
                     (push (cons (overlay-start ov) (overlay-end ov)) out)))
                 out)
               (lambda (a b) (< (car a) (car b))))",
    )
}

fn highlight_count(interp: &mut Interp) -> String {
    run(
        interp,
        "(length (let (out)
                    (dolist (ov (overlays-in (point-min) (point-max)))
                      (when (overlay-get ov 'lsp-highlight)
                        (push ov out)))
                    out))",
    )
}

fn documenthighlight_reply(range_json: &str) -> String {
    format!("[{{\"range\":{range_json}}}]")
}

// ============================================================
// Request shape / UTF-16 coordinates
// ============================================================

#[test]
fn highlight_sends_method_and_position_params() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("req_shape");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)"); // 0-based (line 0, char 2)
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");

    assert_eq!(run(&mut i, "test--synced"), "t");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentHighlight\""
    );
    let params = "(nth 2 test--captured)";
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"uri\" (gethash \"textDocument\" {params}))")
        ),
        run(&mut i, "(lsp--path-to-uri (buffer-file-name))")
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
        "2"
    );
}

/// The coordinate-system correctness test: point sits right after the
/// astral-plane emoji, where the scalar-value offset (1) and the
/// UTF-16 offset (2) disagree. Only the UTF-16 number may appear in
/// the outgoing request.
#[test]
fn highlight_request_uses_utf16_not_scalar_character_offset() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_emoji_scratch("req_utf16");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 2)"); // right after the emoji, before "foo"
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");

    let params = "(nth 2 test--captured)";
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"character\" (gethash \"position\" {params}))")
        ),
        "2",
        "must be the UTF-16 code-unit offset (2), not the scalar-value offset (1)"
    );
}

// ============================================================
// Building overlays from a reply
// ============================================================

/// The reverse-conversion counterpart of the UTF-16 request test: the
/// server's range uses UTF-16 offsets spanning "foo" (2..5), which must
/// map back to buffer positions 2..5 -- the scalar-value inverse would
/// misplace the start by one position (landing on 'o' instead of 'f').
#[test]
fn highlight_reply_builds_overlay_at_correct_utf16_reverse_converted_position() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_emoji_scratch("reply_utf16");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 2)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":2},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(highlight_count(&mut i), "1");
    assert_eq!(highlight_overlay_spans(&mut i), "((2 . 5))");
}

#[test]
fn highlight_reply_with_n_ranges_builds_n_tagged_overlays() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("reply_n");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    // "hello" (chars 0..5, buffer 1..6) and "world" (chars 6..11, buffer 7..12).
    let reply = "[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}},\
                  {\"range\":{\"start\":{\"line\":0,\"character\":6},\"end\":{\"line\":0,\"character\":11}}}]";
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(highlight_count(&mut i), "2");
    assert_eq!(highlight_overlay_spans(&mut i), "((1 . 6) (7 . 12))");
}

/// A server may legally reply with a zero-width range (`start == end`
/// -- same shape the diagnostics side already has to handle, see
/// `lsp--decorate-buffer`). `make-overlay FROM FROM` would be
/// invisible (nothing to draw), so `lsp-highlight-at-point` must widen
/// it by one character, same as `lsp--decorate-buffer` does. Chosen
/// position (buffer 4, character 3 on "hello world\n") is well short of
/// the buffer's end, so the widened endpoint (5) cannot be clamped back
/// down to 4 by `make-overlay`'s own end-of-buffer clamp -- see this
/// function's own docstring in `lsp.el` for why that clamp is a known,
/// separate, unaddressed limitation at the true end of the buffer.
#[test]
fn highlight_zero_width_range_widens_to_a_visible_one_character_overlay() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("zero_width");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":3},\"end\":{\"line\":0,\"character\":3}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(highlight_count(&mut i), "1");
    assert_eq!(
        highlight_overlay_spans(&mut i),
        "((4 . 5))",
        "a zero-width range must still widen to a visible overlay, not collapse to nothing"
    );
}

#[test]
fn highlight_second_request_does_not_accumulate_overlays_from_the_first() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("no_accumulate");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply1 = "[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}},\
                   {\"range\":{\"start\":{\"line\":0,\"character\":6},\"end\":{\"line\":0,\"character\":11}}}]";
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply1:?}))"),
    );
    assert_eq!(highlight_count(&mut i), "2");

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply2 = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply2:?}))"),
    );

    assert_eq!(
        highlight_count(&mut i),
        "1",
        "the second reply's overlay count must win, not add to the first"
    );
}

// ============================================================
// Empty replies (three wire shapes) clear old highlights
// ============================================================

#[test]
fn highlight_empty_array_reply_clears_old_highlights_and_messages() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("empty_array");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );
    assert_eq!(highlight_count(&mut i), "1", "sanity: old highlight exists");

    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-highlight-at-point)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured) (json-parse-string \"[]\"))",
    );

    assert_eq!(
        highlight_count(&mut i),
        "0",
        "old highlight must be cleared"
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No highlights here\""
    );
}

#[test]
fn highlight_lisp_nil_reply_clears_old_highlights_and_messages() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("nil_reply");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );
    assert_eq!(highlight_count(&mut i), "1", "sanity: old highlight exists");

    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-highlight-at-point)");
    // A JSON-RPC error response (no "result" key) delivers Lisp `nil`
    // to the callback -- see `lsp.el`'s M46 note on `lsp--dispatch`.
    ok(&mut i, "(funcall (nth 3 test--captured) nil)");

    assert_eq!(
        highlight_count(&mut i),
        "0",
        "old highlight must be cleared"
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No highlights here\""
    );
}

#[test]
fn highlight_json_null_reply_clears_old_highlights_and_messages() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("null_reply");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );
    assert_eq!(highlight_count(&mut i), "1", "sanity: old highlight exists");

    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-highlight-at-point)");
    // A successful reply of the JSON literal `null` parses to the
    // symbol `:null` (per `crates/elisp/src/json.rs`'s `from_json`).
    ok(&mut i, "(funcall (nth 3 test--captured) :null)");

    assert_eq!(
        highlight_count(&mut i),
        "0",
        "old highlight must be cleared"
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"No highlights here\""
    );
}

// ============================================================
// Buffer-identity guard
// ============================================================

#[test]
fn highlight_reply_delivered_in_a_different_buffer_builds_no_overlays() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("stale_buf");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)");
    ok(&mut i, "(generate-new-buffer \"*hl-other*\")");
    ok(&mut i, "(switch-to-buffer-internal \"*hl-other*\")");
    // *hl-other* needs its own real text: an empty buffer clamps every
    // position to 1, so a defeated guard's overlay would land as a
    // zero-width (1 . 1) overlay there -- which `overlays-in` does not
    // report at all (confirmed by hand against this editor's overlay
    // implementation), making the assertion below pass whether or not
    // the guard actually ran. Giving *hl-other* the same "hello world\n"
    // text as the real file makes the range convert to a normal,
    // detectable (1 . 6) span if the guard is defeated.
    ok(&mut i, "(insert \"hello world\\n\")");

    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );

    assert_eq!(
        run(&mut i, "test--messages"),
        "nil",
        "no message in either buffer"
    );
    // The buffer-identity guard must stop the overlay from being built
    // at all -- not just "not in the original buffer". `make-overlay`
    // with no explicit buffer argument defaults to the CURRENT buffer,
    // which is `*hl-other*` at this point in the test -- if the guard
    // were missing (or defeated), the overlay would land here, and a
    // check that only ever looks at the original file buffer's count
    // (already 0 in both the guarded and unguarded case) would never
    // notice.
    assert_eq!(
        highlight_count(&mut i),
        "0",
        "the reply must not have built an overlay in *hl-other* either"
    );
    ok(
        &mut i,
        &format!(
            "(switch-to-buffer-internal {:?})",
            file.file_name().unwrap().to_str().unwrap()
        ),
    );
    assert_eq!(highlight_count(&mut i), "0");
}

// ============================================================
// Guard paths: no client / no file
// ============================================================

#[test]
fn highlight_at_point_reports_when_no_client_is_connected() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("noclient");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    assert_eq!(
        run(&mut i, "(lsp-highlight-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
}

#[test]
fn highlight_at_point_reports_when_buffer_is_not_visiting_a_file() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"*hl-nofile*\")");
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    assert_eq!(
        run(&mut i, "(lsp-highlight-at-point)"),
        "\"Buffer is not visiting a file\""
    );
}

/// Pins the `(unless quiet (message ...))` guard on the *other* branch of
/// `lsp-highlight-at-point`'s `cond' -- the one taken when the buffer has
/// a live client but no file name (`highlight_at_point_reports_when_buffer_is_not_visiting_a_file`
/// only exercises this with QUIET nil). Reviewer round 2 found this half
/// had zero coverage: reverting `(unless quiet (message ...))' to a bare
/// `(message ...)' here left all 32 tests in this file passing.
#[test]
fn manual_highlight_at_point_with_quiet_suppresses_messages_when_no_file_name() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"*hl-quiet-nofile*\")");
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    capture_request_async(&mut i);
    capture_messages(&mut i);

    ok(&mut i, "(lsp-highlight-at-point t)");

    assert_eq!(run(&mut i, "test--captured"), "nil");
    assert_eq!(run(&mut i, "test--messages"), "nil");

    // Control: the same buffer, same guard, but QUIET nil -- the message
    // must still fire, confirming the `unless quiet` didn't also swallow
    // the non-quiet path's control flow.
    ok(&mut i, "(lsp-highlight-at-point)");
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Buffer is not visiting a file\""
    );
}

// ============================================================
// lsp-highlight-clear
// ============================================================

#[test]
fn highlight_clear_removes_only_lsp_highlight_tagged_overlays() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("clear_selective");
    setup_client_buffer(&mut i, &file);

    ok(
        &mut i,
        "(setq diag-ov (make-overlay 1 3)) (overlay-put diag-ov 'lsp-diag t)",
    );
    ok(
        &mut i,
        "(setq hl-ov (make-overlay 4 6)) (overlay-put hl-ov 'lsp-highlight t)",
    );

    ok(&mut i, "(lsp-highlight-clear)");

    assert_eq!(highlight_count(&mut i), "0");
    assert_eq!(
        run(
            &mut i,
            "(length (let (out)
                        (dolist (ov (overlays-in (point-min) (point-max)))
                          (when (overlay-get ov 'lsp-diag) (push ov out)))
                        out))"
        ),
        "1",
        "the lsp-diag overlay must survive lsp-highlight-clear"
    );
}

/// Pins T2 (M51 second round): `lsp-highlight-clear' called on a buffer
/// with no live client must not record `lsp--idle-highlight-last-point',
/// otherwise a later `M-x lsp' connection at that same unmoved point
/// would silently swallow the first idle auto-trigger until the user
/// moves point at least once. See `lsp-highlight-clear''s docstring for
/// the narrow, deliberately-accepted counterpart case this fix does not
/// address.
#[test]
fn highlight_clear_on_client_free_buffer_does_not_create_a_dead_zone() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("t2_no_dead_zone");
    // No client yet -- unlike setup_client_buffer, just find-file.
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(goto-char 3)");
    ok(&mut i, "(lsp-highlight-clear)"); // no client: must not record last-point

    // Now connect a client, same point, and tick.
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    ok(&mut i, "(setq test--synced nil)");
    ok(
        &mut i,
        "(fset 'lsp--sync-buffer-now (lambda () (setq test--synced t)))",
    );
    capture_request_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentHighlight\"",
        "the first auto-trigger after connecting must not be suppressed by \
         a stale last-point recorded while there was no client"
    );
}

// ============================================================
// lsp-next-highlight / lsp-previous-highlight
// ============================================================

/// "hello world\n" with two highlights: "hello" (buffer 1..6) and
/// "world" (buffer 7..12).
fn setup_two_highlights(interp: &mut Interp, tag: &str) -> (Scratch, std::path::PathBuf) {
    let (dir, file) = write_hello_world_scratch(tag);
    setup_client_buffer(interp, &file);
    ok(interp, "(goto-char 1)");
    capture_request_async(interp);
    ok(interp, "(lsp-highlight-at-point)");
    let reply = "[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}},\
                  {\"range\":{\"start\":{\"line\":0,\"character\":6},\"end\":{\"line\":0,\"character\":11}}}]";
    ok(
        interp,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );
    (dir, file)
}

#[test]
fn next_highlight_jumps_forward_and_wraps() {
    let (mut i, _ed) = setup();
    let (_scratch, _file) = setup_two_highlights(&mut i, "next_fwd");

    ok(&mut i, "(goto-char 3)"); // inside "hello"
    ok(&mut i, "(lsp-next-highlight)");
    assert_eq!(
        run(&mut i, "(point)"),
        "7",
        "jumps to the start of \"world\""
    );

    capture_messages(&mut i);
    ok(&mut i, "(lsp-next-highlight)");
    assert_eq!(
        run(&mut i, "(point)"),
        "1",
        "wraps to the start of \"hello\""
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Wrapped to first highlight\""
    );
}

#[test]
fn previous_highlight_jumps_backward_and_wraps() {
    let (mut i, _ed) = setup();
    let (_scratch, _file) = setup_two_highlights(&mut i, "prev_bwd");

    // At "hello"'s own start (1): nothing starts before it, so this
    // wraps to the last highlight, "world" (start 7).
    ok(&mut i, "(goto-char 1)");
    capture_messages(&mut i);
    ok(&mut i, "(lsp-previous-highlight)");
    assert_eq!(
        run(&mut i, "(point)"),
        "7",
        "wraps to the start of \"world\""
    );
    assert_eq!(
        run(&mut i, "(car test--messages)"),
        "\"Wrapped to last highlight\""
    );

    // From "world"'s own start (7): "hello" (start 1) is the nearest
    // highlight starting before it -- no wrap needed this time.
    ok(&mut i, "(lsp-previous-highlight)");
    assert_eq!(
        run(&mut i, "(point)"),
        "1",
        "jumps to the start of \"hello\""
    );
}

#[test]
fn next_and_previous_highlight_message_when_none_are_drawn() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("none_drawn");
    setup_client_buffer(&mut i, &file);
    capture_messages(&mut i);

    ok(&mut i, "(lsp-next-highlight)");
    assert_eq!(run(&mut i, "(car test--messages)"), "\"No highlights\"");

    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-previous-highlight)");
    assert_eq!(run(&mut i, "(car test--messages)"), "\"No highlights\"");
}

// ============================================================
// C-c l key bindings
// ============================================================

#[test]
fn c_c_l_prefix_bindings_reach_every_m49_highlight_command() {
    let (mut i, ed) = setup();
    for (keys, fname) in [
        ("C-c l h", "lsp-highlight-at-point"),
        ("C-c l H", "lsp-highlight-clear"),
        ("C-c l N", "lsp-next-highlight"),
        ("C-c l P", "lsp-previous-highlight"),
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

// ============================================================
// M51: idle auto-trigger (`lsp--idle-highlight-tick`)
// ============================================================

/// Like `capture_request_async`, but appends every call instead of
/// overwriting -- needed for the D6 out-of-order-reply test, which must
/// hold onto both requests' callbacks at once.
fn capture_all_requests_async(interp: &mut Interp) {
    ok(interp, "(setq test--captured-all nil)");
    ok(
        interp,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured-all
                       (append test--captured-all
                               (list (list client method params callback))))
                 99))",
    );
}

#[test]
fn idle_tick_below_threshold_sends_no_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_below_threshold");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 100)"); // default delay is 300

    assert_eq!(run(&mut i, "test--captured"), "nil");
}

#[test]
fn idle_tick_at_threshold_at_new_point_sends_one_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_at_threshold");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentHighlight\""
    );
}

#[test]
fn idle_tick_disabled_by_nil_delay_sends_no_request_even_at_huge_quiet_ms() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_disabled");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    ok(&mut i, "(setq lsp-idle-highlight-delay-ms nil)");
    capture_request_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 1000000)");

    assert_eq!(run(&mut i, "test--captured"), "nil");
}

#[test]
fn idle_tick_twice_at_same_point_sends_only_one_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_same_point_twice");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_all_requests_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");
    ok(&mut i, "(lsp--idle-highlight-tick 500)");

    assert_eq!(run(&mut i, "(length test--captured-all)"), "1");
}

#[test]
fn idle_tick_after_point_moves_sends_a_second_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_point_moves");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_all_requests_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");
    ok(&mut i, "(goto-char 7)");
    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(run(&mut i, "(length test--captured-all)"), "2");
}

#[test]
fn idle_tick_with_no_client_sends_no_request_and_no_message() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_no_client");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);
    capture_messages(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(run(&mut i, "test--captured"), "nil");
    assert_eq!(run(&mut i, "test--messages"), "nil");
}

#[test]
fn idle_tick_on_non_file_buffer_sends_no_request_and_no_message() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"*hl-idle-nofile*\")");
    ok(&mut i, "(setq client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client client)");
    capture_request_async(&mut i);
    capture_messages(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(run(&mut i, "test--captured"), "nil");
    assert_eq!(run(&mut i, "test--messages"), "nil");
}

/// Pins the `(unless quiet (message ...))` guards themselves: called
/// with QUIET non-nil (as only `lsp--idle-highlight-tick` normally
/// would) on a buffer with no client connected, `lsp-highlight-at-point`
/// must neither send a request nor emit either of its two `message`
/// calls. Without this test neither `unless quiet` guard has a mutation
/// target, since `lsp--idle-highlight-tick` itself already filters out
/// "no client"/"non-file buffer" before ever calling with QUIET=t.
#[test]
fn manual_highlight_at_point_with_quiet_suppresses_messages_when_no_client() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_manual_quiet_no_client");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    capture_request_async(&mut i);
    capture_messages(&mut i);

    ok(&mut i, "(lsp-highlight-at-point t)");

    assert_eq!(run(&mut i, "test--captured"), "nil");
    assert_eq!(run(&mut i, "test--messages"), "nil");
}

#[test]
fn idle_tick_after_manual_highlight_at_same_point_sends_no_second_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_after_manual");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_all_requests_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)"); // manual, interactive-style call
    ok(&mut i, "(lsp--idle-highlight-tick 300)");

    assert_eq!(
        run(&mut i, "(length test--captured-all)"),
        "1",
        "the manual call must have set lsp--idle-highlight-last-point"
    );
}

#[test]
fn idle_tick_empty_reply_clears_overlays_without_messaging() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_empty_reply_quiet");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    // First: a real reply draws one overlay.
    ok(&mut i, "(lsp-highlight-at-point)");
    let reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {reply:?}))"),
    );
    assert_eq!(highlight_count(&mut i), "1");

    // Then: move point, quiet-tick, and reply empty.
    ok(&mut i, "(goto-char 8)");
    capture_messages(&mut i);
    ok(&mut i, "(lsp--idle-highlight-tick 300)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured) (json-parse-string \"[]\"))",
    );

    assert_eq!(highlight_count(&mut i), "0");
    assert_eq!(run(&mut i, "test--messages"), "nil");
}

/// Pins that `lsp-highlight-clear` records the *current* point (position
/// B) in `lsp--idle-highlight-last-point`, not just whatever point the
/// last request happened to leave there. Point is moved from A (where
/// the request was sent) to B *before* calling `lsp-highlight-clear`,
/// so if that call didn't update the variable, the idle tick at B would
/// still see the stale "last point" A (!= B) and wrongly fire a request.
#[test]
fn idle_tick_after_highlight_clear_at_same_point_sends_no_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_after_clear");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)"); // position A
    capture_request_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)"); // request sent from A
    ok(&mut i, "(goto-char 7)"); // move to position B ("world")
    ok(&mut i, "(setq test--captured nil)");
    ok(&mut i, "(lsp-highlight-clear)"); // must record B as last-point
    ok(&mut i, "(lsp--idle-highlight-tick 300)"); // ticked while still at B

    assert_eq!(run(&mut i, "test--captured"), "nil");
}

/// D6: the callback only has a buffer-identity guard, not a
/// request-recency guard, so if two requests are in flight from the
/// same buffer and their replies arrive out of order, the stale first
/// reply's callback runs *after* the fresh second reply's and
/// overwrites its highlights with the wrong (earlier) position's
/// ranges. This pins the request-serial guard (D6) that prevents it.
#[test]
fn stale_out_of_order_reply_does_not_clobber_the_newer_requests_highlights() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("idle_stale_reply");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 1)"); // "hello"
    capture_all_requests_async(&mut i);

    ok(&mut i, "(lsp-highlight-at-point)"); // request #1, at point 1
    ok(&mut i, "(goto-char 7)"); // "world"
    ok(&mut i, "(lsp-highlight-at-point)"); // request #2, at point 7

    assert_eq!(run(&mut i, "(length test--captured-all)"), "2");

    let hello_reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":5}}",
    );
    let world_reply = documenthighlight_reply(
        "{\"start\":{\"line\":0,\"character\":6},\"end\":{\"line\":0,\"character\":11}}",
    );
    // Deliver out of order: request #2's (newer) reply first, then
    // request #1's (stale) reply.
    ok(
        &mut i,
        &format!(
            "(funcall (nth 3 (nth 1 test--captured-all)) (json-parse-string {world_reply:?}))"
        ),
    );
    ok(
        &mut i,
        &format!(
            "(funcall (nth 3 (nth 0 test--captured-all)) (json-parse-string {hello_reply:?}))"
        ),
    );

    assert_eq!(
        highlight_overlay_spans(&mut i),
        "((7 . 12))",
        "the stale reply for point 1 (\"hello\", 1..6) must not overwrite \
         the newer request's highlight at point 7 (\"world\", 7..12)"
    );
}

// ============================================================
// M51 F2: end-to-end through `core::idle_tick` (Rust), not just the
// elisp `lsp--idle-highlight-tick` the other M51 tests call directly.
// Pins the Rust->elisp wiring in `lib.rs` (the `Duration` -> millisecond
// conversion and the `eval_source` call that dispatches into elisp).
// ============================================================

#[test]
fn rust_idle_tick_over_threshold_triggers_a_documenthighlight_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("rust_idle_tick_over_threshold");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    core::idle_tick(&mut i, std::time::Duration::from_millis(500)); // > default 300ms

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentHighlight\""
    );
}

#[test]
fn rust_idle_tick_below_threshold_sends_no_request() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("rust_idle_tick_below_threshold");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    capture_request_async(&mut i);

    core::idle_tick(&mut i, std::time::Duration::from_millis(100)); // < default 300ms

    assert_eq!(run(&mut i, "test--captured"), "nil");
}

/// Pins the *ceiling value* of the `Duration` -> millisecond clamp in
/// `lib.rs`'s `idle_tick` (M51 second round, T4), not just that ordinary
/// small delays pass through unclamped. `lsp-idle-highlight-delay-ms` is
/// set above the old, wrong ceiling (3_600_000) but below the correct one
/// (`u32::MAX` = 4_294_967_295); a `Duration` above that threshold sends a
/// request under the correct clamp but is silently, permanently swallowed
/// under the old one -- exactly the failure mode F4 fixed.
#[test]
fn rust_idle_tick_with_threshold_above_the_old_wrong_clamp_still_triggers() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_hello_world_scratch("rust_idle_tick_high_threshold");
    setup_client_buffer(&mut i, &file);
    ok(&mut i, "(goto-char 3)");
    ok(&mut i, "(setq lsp-idle-highlight-delay-ms 100000000)"); // 100_000_000 > 3_600_000
    capture_request_async(&mut i);

    core::idle_tick(&mut i, std::time::Duration::from_millis(200_000_000)); // > threshold, < u32::MAX

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentHighlight\""
    );
}

#[test]
fn idle_highlight_last_point_is_buffer_local() {
    let (mut i, _ed) = setup();
    let (_scratch_a, file_a) = write_hello_world_scratch("idle_buffer_local_a");
    let (_scratch_b, file_b) = write_hello_world_scratch("idle_buffer_local_b");
    setup_client_buffer(&mut i, &file_a);
    ok(&mut i, "(goto-char 3)");
    capture_all_requests_async(&mut i);

    ok(&mut i, "(lsp--idle-highlight-tick 300)"); // buffer A: 1 request

    setup_client_buffer(&mut i, &file_b);
    ok(&mut i, "(goto-char 3)");
    ok(&mut i, "(lsp--idle-highlight-tick 300)"); // buffer B: same point, but a different buffer

    assert_eq!(
        run(&mut i, "(length test--captured-all)"),
        "2",
        "lsp--idle-highlight-last-point must be buffer-local"
    );
}

// ============================================================
// M99: `lsp-merge-diagnostics-from-all-clients` -- merged decoration
// and navigation across two attached clients.
// ============================================================

/// Same helper as `lsp_mode_tests.rs`'s `install_test_publish_helper`,
/// copied rather than shared (this project's tests bring their own
/// helpers; see CLAUDE.md's testing conventions). Dispatches a
/// `textDocument/publishDiagnostics` notification for CLIENT with a
/// single diagnostic at 0-based LINE, so `lsp--merge-diagnostics' stores
/// it exactly as it would from a real server.
fn install_test_publish_helper(interp: &mut Interp) {
    ok(
        interp,
        r#"(defun test--publish (client uri msg line)
             (let ((h (make-hash-table)) (p (make-hash-table)))
               (puthash "uri" uri p)
               (puthash "diagnostics"
                        (json-parse-string
                         (format "[{\"range\":{\"start\":{\"line\":%d,\"character\":0},\"end\":{\"line\":%d,\"character\":1}},\"message\":\"%s\"}]"
                                 line line msg))
                        p)
               (puthash "method" "textDocument/publishDiagnostics" h)
               (puthash "params" p h)
               (lsp--dispatch client h)))"#,
    );
}

/// Count of `'lsp-diag'-tagged overlays in the current buffer -- the
/// diagnostics analogue of this file's own `highlight_overlay_spans'/
/// `highlight_count', for `'lsp-highlight'.
fn diag_overlay_count(interp: &mut Interp) -> String {
    run(
        interp,
        "(length (let (out)
                    (dolist (ov (overlays-in (point-min) (point-max)))
                      (when (overlay-get ov 'lsp-diag)
                        (push ov out)))
                    out))",
    )
}

fn write_three_line_scratch(tag: &str) -> (Scratch, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "line0\nline1\nline2\n").unwrap();
    (dir, file)
}

#[test]
fn merge_diagnostics_default_paints_the_union_of_two_attached_clients() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_union");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);

    ok(&mut i, "(setq primary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq secondary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list primary secondary))",
    );

    ok(
        &mut i,
        "(test--publish primary (lsp--path-to-uri (buffer-file-name)) \"from primary\" 0)",
    );
    assert_eq!(
        diag_overlay_count(&mut i),
        "1",
        "the primary's own publish must decorate the buffer"
    );

    ok(
        &mut i,
        "(test--publish secondary (lsp--path-to-uri (buffer-file-name)) \"from secondary\" 1)",
    );
    assert_eq!(
        diag_overlay_count(&mut i),
        "2",
        "lsp-merge-diagnostics-from-all-clients defaults to t: the \
         secondary's publish must ADD to the buffer's decoration, not \
         replace or be rejected by it"
    );
}

#[test]
fn merge_diagnostics_set_to_nil_restores_authoritative_only_painting() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_nil");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);
    ok(&mut i, "(setq lsp-merge-diagnostics-from-all-clients nil)");

    ok(&mut i, "(setq primary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq secondary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list primary secondary))",
    );

    ok(
        &mut i,
        "(test--publish primary (lsp--path-to-uri (buffer-file-name)) \"from primary\" 0)",
    );
    assert_eq!(diag_overlay_count(&mut i), "1");

    ok(
        &mut i,
        "(test--publish secondary (lsp--path-to-uri (buffer-file-name)) \"from secondary\" 1)",
    );
    assert_eq!(
        diag_overlay_count(&mut i),
        "1",
        "with the merge variable nil, a non-primary publish must not \
         repaint the buffer's decoration at all -- the exact pre-M99 \
         (M94) behavior"
    );
}

#[test]
fn single_client_painting_is_identical_regardless_of_the_merge_setting() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_single_client");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);
    ok(&mut i, "(setq only (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client only)");

    ok(
        &mut i,
        "(test--publish only (lsp--path-to-uri (buffer-file-name)) \"solo\" 0)",
    );
    assert_eq!(diag_overlay_count(&mut i), "1");

    ok(&mut i, "(setq lsp-merge-diagnostics-from-all-clients nil)");
    ok(
        &mut i,
        "(test--publish only (lsp--path-to-uri (buffer-file-name)) \"solo again\" 1)",
    );
    assert_eq!(
        diag_overlay_count(&mut i),
        "1",
        "a single attached client's own decoration must not depend on \
         lsp-merge-diagnostics-from-all-clients either way"
    );
}

#[test]
fn next_diagnostic_reaches_a_secondary_clients_diagnostic_when_merged() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_navigation");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);

    ok(&mut i, "(setq primary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq secondary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list primary secondary))",
    );
    // Only the SECONDARY ever publishes -- if `next-diagnostic' only
    // ever looked at the primary's own stored diagnostics (pre-M99), it
    // would report "No diagnostics" even though a squiggle is on
    // screen.
    ok(
        &mut i,
        "(test--publish secondary (lsp--path-to-uri (buffer-file-name)) \"secondary only\" 1)",
    );

    ok(&mut i, "(goto-char (point-min))");
    assert_eq!(run(&mut i, "(next-diagnostic)"), "\"secondary only\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");
}

#[test]
fn same_client_two_diagnostics_sharing_start_severity_and_message_are_both_painted() {
    // F1/F2 regression guard: the dedup key used to be a (start line,
    // start character, severity, message) 4-tuple, which does NOT
    // include `range.end' -- two genuinely distinct diagnostics that
    // happen to start at the same place, with the same severity and
    // the same message text, but different END positions, must both
    // still be painted. Content-level dedup can't tell that case apart
    // from the same diagnostic being walked twice, so dedup must
    // operate on the CLIENT list, not on diagnostic content.
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_same_key_diff_end");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq only (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client only)");
    ok(
        &mut i,
        r#"(defun test--publish-two (client uri msg line end1 end2)
             (let ((h (make-hash-table)) (p (make-hash-table)))
               (puthash "uri" uri p)
               (puthash "diagnostics"
                        (json-parse-string
                         (format "[{\"range\":{\"start\":{\"line\":%d,\"character\":0},\"end\":{\"line\":%d,\"character\":%d}},\"severity\":1,\"message\":\"%s\"},{\"range\":{\"start\":{\"line\":%d,\"character\":0},\"end\":{\"line\":%d,\"character\":%d}},\"severity\":1,\"message\":\"%s\"}]"
                                 line line end1 msg line line end2 msg))
                        p)
               (puthash "method" "textDocument/publishDiagnostics" h)
               (puthash "params" p h)
               (lsp--dispatch client h)))"#,
    );

    // Two diagnostics, identical start/severity/message, differing only
    // in `range.end.character' (1 vs 2), published in a single
    // publishDiagnostics notification -- exactly what a real server
    // could legitimately send for two distinct overlapping-start issues.
    ok(
        &mut i,
        "(test--publish-two only (lsp--path-to-uri (buffer-file-name)) \"same\" 0 1 2)",
    );

    assert_eq!(
        diag_overlay_count(&mut i),
        "2",
        "two diagnostics with the same start/severity/message but \
         different range.end must both be painted, not deduped away"
    );
}

#[test]
fn merge_diagnostics_does_not_double_paint_a_client_listed_twice() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("merge_dedup");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);

    ok(&mut i, "(setq only (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client only)");
    // The SAME client object appears twice in `lsp--buffer-clients' --
    // this must not be painted twice.
    ok(&mut i, "(setq-local lsp--buffer-clients (list only only))");

    ok(
        &mut i,
        "(test--publish only (lsp--path-to-uri (buffer-file-name)) \"dup\" 0)",
    );
    assert_eq!(
        diag_overlay_count(&mut i),
        "1",
        "the same client appearing twice in lsp--buffer-clients must not \
         produce a duplicate overlay for the same diagnostic"
    );
}

// ============================================================
// M99 review round 2: `lsp--diagnostics-for-uri`'s RETURN ORDER.
// Every test above only ever asserts overlay COUNT; none of them
// observes order, and `next-diagnostic' re-sorts by buffer position
// before a caller ever sees it -- so a reversed return list was never
// caught. Order matters for real users: M87 stage 3's inline-row cap
// keeps whichever diagnostics come FIRST on a line, so which ones
// survive truncation depends on this function's return order.
// ============================================================

/// Publishes one `textDocument/publishDiagnostics' notification for
/// CLIENT carrying one diagnostic per (LINE . MESSAGE) pair in MSGS, in
/// MSGS's own order -- so a test can pin down the ORDER
/// `lsp--diagnostics-for-uri' returns, not just how many it returns.
fn install_test_publish_n_helper(interp: &mut Interp) {
    ok(
        interp,
        r#"(defun test--publish-n (client uri msgs)
             (let ((h (make-hash-table)) (p (make-hash-table)))
               (puthash "uri" uri p)
               (puthash "diagnostics"
                        (json-parse-string
                         (concat "["
                                 (mapconcat
                                  (lambda (pair)
                                    (format "{\"range\":{\"start\":{\"line\":%d,\"character\":0},\"end\":{\"line\":%d,\"character\":1}},\"severity\":1,\"message\":\"%s\"}"
                                            (car pair) (car pair) (cdr pair)))
                                  msgs ",")
                                 "]"))
                        p)
               (puthash "method" "textDocument/publishDiagnostics" h)
               (puthash "params" p h)
               (lsp--dispatch client h)))"#,
    );
}

/// The `"message"' field of every diagnostic `lsp--diagnostics-for-uri'
/// returns for CLIENT/URI, joined with `,' in the order returned --
/// order-observing counterpart to `diag_overlay_count' (which only
/// counts).
fn diagnostics_for_uri_message_order(interp: &mut Interp, client_expr: &str) -> String {
    run(
        interp,
        &format!(
            "(mapconcat (lambda (d) (gethash \"message\" d)) \
             (lsp--diagnostics-for-uri {} (lsp--path-to-uri (buffer-file-name))) \
             \",\")",
            client_expr
        ),
    )
}

#[test]
fn diagnostics_for_uri_nil_branch_preserves_publish_order() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("order_nil_branch");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_n_helper(&mut i);
    ok(&mut i, "(setq lsp-merge-diagnostics-from-all-clients nil)");
    ok(&mut i, "(setq only (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client only)");

    ok(
        &mut i,
        "(test--publish-n only (lsp--path-to-uri (buffer-file-name)) \
         (list (cons 0 \"first\") (cons 1 \"second\") (cons 2 \"third\")))",
    );

    assert_eq!(
        diagnostics_for_uri_message_order(&mut i, "only"),
        "\"first,second,third\"",
        "lsp-merge-diagnostics-from-all-clients nil: return order must \
         match publish order"
    );
}

#[test]
fn diagnostics_for_uri_merge_branch_orders_by_client_then_publish_order() {
    let (mut i, _ed) = setup();
    let (_scratch, file) = write_three_line_scratch("order_merge_branch");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_n_helper(&mut i);

    ok(&mut i, "(setq primary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq secondary (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list primary secondary))",
    );

    ok(
        &mut i,
        "(test--publish-n primary (lsp--path-to-uri (buffer-file-name)) \
         (list (cons 0 \"p1\") (cons 1 \"p2\")))",
    );
    ok(
        &mut i,
        "(test--publish-n secondary (lsp--path-to-uri (buffer-file-name)) \
         (list (cons 2 \"s1\")))",
    );

    assert_eq!(
        diagnostics_for_uri_message_order(&mut i, "primary"),
        "\"p1,p2,s1\"",
        "merged order must be: primary's own diagnostics in their \
         publish order, then secondary's in its publish order"
    );
}
