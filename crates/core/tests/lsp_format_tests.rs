//! M46 Part B: `replace-region-contents`. No LSP server involved — these
//! exercise the builtin directly against synthetic buffers, mirroring
//! the measured pathologies (`crates/core/src/builtins/editing.rs`
//! docstring) that motivated diffing instead of delete-then-insert.

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
            "reticle_lsp_format_{}_{}_{}",
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

/// A fresh scratch directory under the OS temp dir, unique per test run
/// (mirrors `lsp_mode_tests.rs`'s helper of the same name -- the
/// interactive formatting/navigation commands below need a real
/// `buffer-file-name`, same as that file's hover/definition tests do).
fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// 200 numbered lines, `\n`-terminated, so line 5 and line 100 are easy
/// to address by construction.
fn make_200_lines() -> String {
    (0..200).map(|i| format!("line{i}\n")).collect()
}

/// The real distinguishing behavior of hunk-diffing vs. naive
/// delete-region+insert: call `replace-region-contents` over the whole
/// buffer (as `lsp-format-buffer` does — the server's TextEdit range
/// typically covers the entire document) where only line 5 actually
/// differs, and by a length-preserving edit so point's absolute offset
/// has no reason to move even by a shift. A naive whole-region replace
/// would delete the entire buffer (collapsing point to `point-min`) and
/// reinsert it; the hunk-based path only touches line 5's few changed
/// characters, so point on line 100 must come through *exactly*
/// unchanged, not just "close".
#[test]
fn point_survives_a_change_elsewhere_in_the_buffer() {
    let (mut i, _ed) = setup();
    let old_lines: Vec<String> = (0..200).map(|n| format!("line{n}")).collect();
    let mut new_lines = old_lines.clone();
    new_lines[4] = "xine4".to_string(); // same length as "line4"
    let old_text: String = old_lines.iter().map(|l| format!("{l}\n")).collect();
    let new_text: String = new_lines.iter().map(|l| format!("{l}\n")).collect();
    assert_eq!(old_text.len(), new_text.len());

    run(&mut i, &format!("(insert {old_text:?})"));
    // Put point at column 3 of line 100.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 99)");
    run(&mut i, "(forward-char 3)");
    let point_before = run(&mut i, "(point)");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "100");

    let result = run(
        &mut i,
        &format!("(replace-region-contents (point-min) (point-max) {new_text:?})"),
    );
    assert_eq!(result, "t", "expected an incremental (non-'full') apply");

    assert_eq!(run(&mut i, &format!("(= (point) {point_before})")), "t");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "100");
}

#[test]
fn overlay_survives_a_change_elsewhere_and_stays_visible_to_overlays_in() {
    let (mut i, _ed) = setup();
    run(&mut i, &format!("(insert {:?})", make_200_lines()));

    // Overlay on line 100.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 99)");
    run(&mut i, "(setq ov-start (line-beginning-position))");
    run(&mut i, "(setq ov-end (line-end-position))");
    run(&mut i, "(setq ov (make-overlay ov-start ov-end))");
    run(&mut i, "(overlay-put ov 'lsp-diag t)");

    // Change only line 5.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 4)");
    run(&mut i, "(setq beg (line-beginning-position))");
    run(&mut i, "(setq end (line-end-position))");
    run(
        &mut i,
        "(replace-region-contents beg end \"line4  (changed)\")",
    );

    // Overlay shifted by exactly the same delta as ov-start/ov-end would
    // have shifted (line 5's change adds 12 chars: "line4  (changed)" is
    // 17 chars vs "line4" 5 chars = +12).
    let delta = "line4  (changed)".len() as i64 - "line4".len() as i64;
    assert_eq!(
        run(
            &mut i,
            &format!("(= (overlay-start ov) (+ ov-start {delta}))")
        ),
        "t"
    );
    assert_eq!(
        run(&mut i, &format!("(= (overlay-end ov) (+ ov-end {delta}))")),
        "t"
    );
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1"
    );
}

/// Regression lock: directly reproduces the naive `delete-region` +
/// `insert` pathology that motivated `replace-region-contents` in the
/// first place (measured: overlay start/end swap and `overlays-in` goes
/// blind to the surviving overlay). If someone "simplifies"
/// `replace-region-contents` back to delete-then-insert, this test
/// documents exactly what breaks.
#[test]
fn naive_delete_then_insert_loses_the_overlay_from_overlays_in() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"hello world\\n\")");
    run(&mut i, "(setq ov (make-overlay 1 6))");
    run(&mut i, "(overlay-put ov 'lsp-diag t)");
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1"
    );

    run(&mut i, "(delete-region (point-min) (point-max))");
    run(&mut i, "(insert \"goodbye world\\n\")");

    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "0"
    );
}

#[test]
fn multi_hunk_change_is_a_single_undo_step() {
    let (mut i, _ed) = setup();
    let original = "aaa\nbbb\nccc\nddd\neee\n";
    run(&mut i, &format!("(insert {original:?})"));
    run(&mut i, "(undo-boundary)");

    // Three separate hunks in one replace-region-contents call.
    let new_text = "AAA\nbbb\nCCC\nddd\nEEE\n";
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(replace-region-contents (point-min) (point-max) {new_text:?})"),
    );
    assert_eq!(run(&mut i, "(buffer-string)"), format!("{new_text:?}"));

    run(&mut i, "(undo-internal)");
    assert_eq!(run(&mut i, "(buffer-string)"), format!("{original:?}"));
}

#[test]
fn identical_content_is_a_no_op() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"unchanged text\\n\")");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-char 3)");
    let point_before = run(&mut i, "(point)");
    let tick_before = run(&mut i, "(buffer-modified-tick)");

    let result = run(
        &mut i,
        "(replace-region-contents (point-min) (point-max) \"unchanged text\\n\")",
    );
    assert_eq!(result, "nil");
    assert_eq!(run(&mut i, "(buffer-modified-tick)"), tick_before);
    assert_eq!(run(&mut i, "(point)"), point_before);
}

#[test]
fn max_cost_zero_forces_full_replace_but_text_is_still_correct() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"one\\ntwo\\nthree\\n\")");
    let result = run(
        &mut i,
        "(replace-region-contents (point-min) (point-max) \"ONE\\ntwo\\nthree\\n\" 0)",
    );
    assert_eq!(result, "full");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"ONE\\ntwo\\nthree\\n\"");
}

/// Architect-flagged coverage gap: nothing previously caught
/// `replace-region-contents` (or any editing path) bypassing
/// `editor::edit_delete`/`edit_insert` and calling `Buffer::delete`/
/// `insert` directly, which would leave other windows' `window_start`
/// stale. Two windows on the same buffer, a change before the second
/// window's scroll position, and we assert that window's `window_start`
/// shifted by exactly the size delta — reading the `Editor`/`Window`
/// structs directly since there's no elisp accessor for `window_start`.
#[test]
fn window_start_stays_in_sync_across_windows() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", make_200_lines()));

    // Split, and manually scroll the second window well past the edit
    // point (line 5) so its window_start must shift when line 5 grows.
    run(&mut i, "(split-window-internal nil)");
    run(&mut i, "(other-window 1)");

    // Compute the char offset of line 150 (well past line 5) and use it
    // as this window's window_start.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 149)");
    let line150_pos: usize = run(&mut i, "(point)").parse().unwrap();
    {
        let mut editor = ed.borrow_mut();
        let sel = editor.selected_window;
        editor.windows.get_mut(&sel).unwrap().window_start = line150_pos - 1; // 0-based
    }
    run(&mut i, "(other-window 1)"); // back to first window

    // Now edit line 5 (grows by some delta) from the first window.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 4)");
    run(&mut i, "(setq beg (line-beginning-position))");
    run(&mut i, "(setq end (line-end-position))");
    run(
        &mut i,
        "(replace-region-contents beg end \"line4-extended-content\")",
    );
    let delta = "line4-extended-content".len() as i64 - "line4".len() as i64;

    let other_window_start = {
        let editor = ed.borrow();
        let mut ids: Vec<usize> = editor.windows.keys().copied().collect();
        ids.sort_unstable();
        let other_id = ids
            .into_iter()
            .find(|id| *id != editor.selected_window)
            .expect("split created a second window");
        editor.windows[&other_id].window_start as i64
    };
    assert_eq!(other_window_start, (line150_pos as i64 - 1) + delta);
}

#[test]
fn non_ascii_offsets_are_character_based_not_byte_based() {
    let (mut i, _ed) = setup();
    // CJK comment line, each 中/文 is 3 bytes but 1 char.
    run(
        &mut i,
        "(insert \"// 中文註解\\nmodule top;\\nendmodule\\n\")",
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(setq beg (line-beginning-position))");
    run(&mut i, "(setq end (line-end-position))");
    let result = run(
        &mut i,
        "(replace-region-contents beg end \"// 中文註解已更新\")",
    );
    // Must succeed (not panic / not silently corrupt), and the rest of
    // the buffer must be untouched.
    assert_ne!(result, "ERROR");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 1)");
    assert_eq!(
        run(&mut i, "(buffer-substring (point) (line-end-position))"),
        "\"module top;\""
    );
}

/// M46 Part C: `lsp--pos-at-utf16`/`lsp--utf16-character-at` must round
/// trip on a line containing an astral-plane character (emoji, here),
/// AND must disagree with the pre-M46 `lsp--pos-at`/
/// `lsp--line-character-at` on the very same input -- proving the new
/// functions actually changed behavior (correctly count the emoji as 2
/// UTF-16 units) rather than just being the old logic under a new name.
#[test]
fn utf16_position_functions_round_trip_and_differ_from_the_char_based_ones() {
    let (mut i, _ed) = setup();
    // "x😀y" -- 'x' (1 char, 1 unit), the emoji (1 char, 2 UTF-16 units),
    // 'y' (1 char, 1 unit). The char immediately after the emoji sits at
    // char-offset 2 but UTF-16 offset 3.
    run(&mut i, "(insert \"x😀y\\nsecond line\\n\")");

    // Round trip: UTF-16 character 3 (right after the emoji) converts
    // to a buffer position and back to the same UTF-16 character.
    let pos = run(&mut i, "(lsp--pos-at-utf16 0 3)");
    assert_eq!(
        run(&mut i, &format!("(lsp--utf16-character-at {pos})")),
        "3"
    );
    // That buffer position is 'y' (the char right after the emoji).
    assert_eq!(run(&mut i, &format!("(= (char-after {pos}) ?y)")), "t");

    // The pre-M46 char-based function, given the SAME UTF-16 offset (3),
    // overshoots past 'y' entirely (there are only 3 characters total on
    // the line) -- it treats 3 as a scalar-value offset, landing one
    // character further than `lsp--pos-at-utf16` does. This is the
    // concrete disagreement the file header's M46 note describes.
    let old_pos = run(&mut i, "(lsp--pos-at 0 3)");
    assert_ne!(
        old_pos, pos,
        "old and new position functions must disagree here"
    );
}

/// M46 Part D: `lsp--apply-text-edits` must reject overlapping
/// `TextEdit`s rather than silently applying them back-to-front (which
/// would eat data). Two edits both touching character 2 of the line.
#[test]
fn apply_text_edits_rejects_overlapping_edits() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"abcdefghij\\n\")");
    let edits = "[{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":4}},\"newText\":\"WXYZ\"},\
                  {\"range\":{\"start\":{\"line\":0,\"character\":2},\"end\":{\"line\":0,\"character\":6}},\"newText\":\"1234\"}]";
    let result = run(
        &mut i,
        &format!("(lsp--apply-text-edits (json-parse-string {edits:?}))"),
    );
    assert!(
        result.starts_with("ERROR"),
        "expected overlapping edits to signal an error, got {result}"
    );
    // And the buffer must be untouched -- the check runs before any
    // edit is applied.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abcdefghij\\n\"");
}

/// M46 Part D: the common case -- a single TextEdit covering the whole
/// document (what `verible-verilog-ls` actually returns for
/// `textDocument/formatting`, per the M46 spec) goes through the exact
/// same code path as many small edits, no special-casing.
#[test]
fn apply_text_edits_handles_a_single_whole_document_edit() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"module top;\\nlogic a;\\nendmodule\\n\")");
    let new_text = "module top;\n  logic a;\nendmodule\n";
    let edits = format!(
        "[{{\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":3,\"character\":0}}}},\"newText\":{new_text:?}}}]"
    );
    run(
        &mut i,
        &format!("(lsp--apply-text-edits (json-parse-string {edits:?}))"),
    );
    assert_eq!(run(&mut i, "(buffer-string)"), format!("{new_text:?}"));
}

// ============================================================
// M46 fix round -- A1/A2: the two order-sensitive properties that were
// previously unguarded (a length-preserving multi-hunk test can't tell
// front-to-back apart from back-to-front application; these use
// length-CHANGING hunks/edits scattered across the buffer specifically
// so getting the order backwards corrupts the result).
// ============================================================

/// A1: `replace-region-contents` with 3+ hunks, each changing length
/// differently (grow, shrink, grow), scattered across the buffer.
/// Mutation-verified: reverting `hunks.iter().rev()` to `hunks.iter()`
/// in `crates/core/src/builtins/editing.rs` makes this FAIL (see the
/// implementer's report for the actual run).
#[test]
fn multi_hunk_length_changing_edits_apply_in_correct_order() {
    let (mut i, _ed) = setup();
    let old = "line0\nline1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\n";
    let new =
        "line0\nline1 GROWS A LOT\nline2\nX\nline4\nline5\nline6 shrinks a bit\nline7\nline8\nline9\n";
    run(&mut i, &format!("(insert {old:?})"));
    let result = run(
        &mut i,
        &format!("(replace-region-contents (point-min) (point-max) {new:?})"),
    );
    assert_eq!(result, "t", "expected an incremental (non-'full') apply");
    assert_eq!(run(&mut i, "(buffer-string)"), format!("{new:?}"));
}

/// A2: `lsp--apply-text-edits` with 3 non-overlapping, length-changing
/// top-level TextEdits. Mutation-verified: flipping the sort comparator
/// in `lsp--apply-text-edits` (`lsp.el`) from `>` to `<` makes this
/// FAIL (see the implementer's report for the actual run).
#[test]
fn apply_text_edits_multiple_length_changing_edits_apply_in_correct_order() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        "(insert \"line0\\nline1\\nline2\\nline3\\nline4\\nline5\\n\")",
    );
    let edits = "[\
      {\"range\":{\"start\":{\"line\":1,\"character\":0},\"end\":{\"line\":1,\"character\":5}},\"newText\":\"line1 GROWS\"},\
      {\"range\":{\"start\":{\"line\":3,\"character\":0},\"end\":{\"line\":3,\"character\":5}},\"newText\":\"X\"},\
      {\"range\":{\"start\":{\"line\":5,\"character\":0},\"end\":{\"line\":5,\"character\":5}},\"newText\":\"line5 grows even more\"}\
    ]";
    run(
        &mut i,
        &format!("(lsp--apply-text-edits (json-parse-string {edits:?}))"),
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        "\"line0\\nline1 GROWS\\nline2\\nX\\nline4\\nline5 grows even more\\n\""
    );
}

// ============================================================
// M46 fix round -- B3: the missing positive overlap-check case.
// ============================================================

/// Adjacent-but-not-overlapping edits (one's END equals the next one's
/// START) must NOT signal an error, and must apply correctly.
/// Mutation-verified: changing the overlap check's `>` to `>=` in
/// `lsp--apply-text-edits` makes this FAIL (see the implementer's
/// report for the actual run).
#[test]
fn apply_text_edits_adjacent_but_not_overlapping_edits_are_accepted() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"abcdefghij\\n\")");
    let edits = "[\
      {\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":4}},\"newText\":\"WXYZ\"},\
      {\"range\":{\"start\":{\"line\":0,\"character\":4},\"end\":{\"line\":0,\"character\":8}},\"newText\":\"1234\"}\
    ]";
    let result = run(
        &mut i,
        &format!("(lsp--apply-text-edits (json-parse-string {edits:?}))"),
    );
    assert_ne!(
        result, "ERROR",
        "adjacent (non-overlapping) edits must not signal"
    );
    assert_eq!(run(&mut i, "(buffer-string)"), "\"WXYZ1234ij\\n\"");
}

// ============================================================
// M46 fix round -- B2: `lsp--formatting-options` standalone.
// ============================================================

#[test]
fn formatting_options_reads_standard_indent_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(setq standard-indent-width 2)");
    assert_eq!(
        run(&mut i, "(gethash \"tabSize\" (lsp--formatting-options))"),
        "2"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"insertSpaces\" (lsp--formatting-options))"
        ),
        "t"
    );
    // Changing the variable changes the output -- proves this reads the
    // variable rather than a baked-in constant that happens to equal
    // the default of 4.
    run(&mut i, "(setq standard-indent-width 8)");
    assert_eq!(
        run(&mut i, "(gethash \"tabSize\" (lsp--formatting-options))"),
        "8"
    );
}

// ============================================================
// M46 fix round -- C1: `lsp--flatten-symbols` preorder, checked BEFORE
// the caller's sort papers over the bug.
// ============================================================

/// Direct test of `lsp--flatten-symbols`'s own output order (not
/// `lsp--document-symbol-positions`, which sorts and would mask the
/// double-reversal bug the M46 review found). One top-level symbol with
/// two children must come out as (top, a, f) -- document/preorder --
/// not (top, f, a).
#[test]
fn flatten_symbols_preserves_preorder_before_any_sort() {
    let (mut i, _ed) = setup();
    let payload = "[{\"name\":\"top\",\"kind\":6,\
                       \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":3,\"character\":9}},\
                       \"selectionRange\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":10}},\
                       \"children\":[\
                         {\"name\":\"a\",\"kind\":13,\
                          \"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":16}},\
                          \"selectionRange\":{\"start\":{\"line\":1,\"character\":14},\"end\":{\"line\":1,\"character\":15}}},\
                         {\"name\":\"f\",\"kind\":12,\
                          \"range\":{\"start\":{\"line\":2,\"character\":2},\"end\":{\"line\":2,\"character\":47}},\
                          \"selectionRange\":{\"start\":{\"line\":2,\"character\":15},\"end\":{\"line\":2,\"character\":16}}}\
                       ]}]";
    run(
        &mut i,
        &format!("(setq flat (lsp--flatten-symbols (json-parse-string {payload:?})))"),
    );
    assert_eq!(run(&mut i, "(length flat)"), "3");
    assert_eq!(run(&mut i, "(nth 1 (nth 0 flat))"), "\"top\"");
    assert_eq!(
        run(&mut i, "(nth 1 (nth 1 flat))"),
        "\"a\"",
        "child order reversed -- the M46 double-reversal bug"
    );
    assert_eq!(run(&mut i, "(nth 1 (nth 2 flat))"), "\"f\"");
}

// ============================================================
// M46 fix round -- B1: `lsp-format-buffer`/`lsp-format-region`/
// `lsp-next-symbol`/`lsp-previous-symbol` wiring. `lsp-request-async`
// is stubbed (never `lsp--await`, matching `lsp_mode_tests.rs`'s
// hover/definition pattern): the fake client `'fake-client` is a bare
// symbol, not an `lsp--client` struct, so if any of these commands were
// changed to go through `lsp--await`/`lsp--request` instead, the real
// `lsp--client-conn` accessor would signal a wrong-type error on it --
// there is nothing else in this test file that would make that path
// succeed. That's what makes these tests fail loudly if the "must go
// through `lsp-request-async`" requirement regresses.
// ============================================================

fn write_verilog_scratch_file(tag: &str) -> ScratchFile {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    std::fs::write(
        &file,
        "module top;\n  logic [3:0] a;\n  function int f(int x); return x+1; endfunction\nendmodule\n",
    )
    .unwrap();
    ScratchFile { file, _dir: dir }
}

#[test]
fn format_buffer_sends_correct_method_params_and_syncs_first() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_buf");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");

    ok(&mut i, "(setq test--synced nil)");
    ok(
        &mut i,
        "(fset 'lsp--sync-buffer-now (lambda () (setq test--synced t)))",
    );
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );

    ok(&mut i, "(lsp-format-buffer)");

    assert_eq!(
        run(&mut i, "test--synced"),
        "t",
        "must sync the buffer before sending the request"
    );
    assert_eq!(run(&mut i, "(car test--captured)"), "fake-client");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/formatting\""
    );
    assert_eq!(
        run(
            &mut i,
            "(string-prefix-p \"file://\" (gethash \"uri\" (gethash \"textDocument\" (nth 2 test--captured))))"
        ),
        "t"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"tabSize\" (gethash \"options\" (nth 2 test--captured)))"
        ),
        "4"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"insertSpaces\" (gethash \"options\" (nth 2 test--captured)))"
        ),
        "t"
    );

    // Deliver the reply and confirm it actually gets applied. The
    // scratch file has 4 lines (0..3), so the whole-document range runs
    // through line 4 character 0.
    let new_text = "module top;\n  logic [3:0] a;\n  function int f(int x);\n    return x+1;\n  endfunction\nendmodule\n";
    let edits = format!(
        "[{{\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":4,\"character\":0}}}},\"newText\":{new_text:?}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {edits:?}))"),
    );
    assert_eq!(run(&mut i, "(buffer-string)"), format!("{new_text:?}"));
}

#[test]
fn format_region_sends_range_params_from_the_active_region() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_region");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");

    // Select line 1 (0-based) -- "  logic [3:0] a;" -- as the region.
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 1)");
    ok(&mut i, "(set-mark (line-beginning-position))");
    ok(&mut i, "(goto-char (line-end-position))");

    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-format-region)");

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/rangeFormatting\""
    );
    let params_range = "(gethash \"range\" (nth 2 test--captured))";
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"line\" (gethash \"start\" {params_range}))")
        ),
        "1"
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"character\" (gethash \"start\" {params_range}))")
        ),
        "0"
    );
    assert_eq!(
        run(
            &mut i,
            &format!("(gethash \"line\" (gethash \"end\" {params_range}))")
        ),
        "1"
    );
}

#[test]
fn next_symbol_sends_document_symbol_and_jumps_with_wraparound() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("next_sym");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");

    let payload = "[{\"name\":\"top\",\"kind\":6,\
                       \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":3,\"character\":9}},\
                       \"selectionRange\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":10}},\
                       \"children\":[\
                         {\"name\":\"a\",\"kind\":13,\
                          \"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":16}},\
                          \"selectionRange\":{\"start\":{\"line\":1,\"character\":14},\"end\":{\"line\":1,\"character\":15}}},\
                         {\"name\":\"f\",\"kind\":12,\
                          \"range\":{\"start\":{\"line\":2,\"character\":2},\"end\":{\"line\":2,\"character\":47}},\
                          \"selectionRange\":{\"start\":{\"line\":2,\"character\":15},\"end\":{\"line\":2,\"character\":16}}}\
                       ]}]";

    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );

    // First call: request sent, no "position"/"range" in params -- just
    // textDocument.
    ok(&mut i, "(lsp-next-symbol)");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentSymbol\""
    );
    assert_eq!(
        run(&mut i, "(gethash \"range\" (nth 2 test--captured))"),
        "nil"
    );
    assert_eq!(
        run(&mut i, "(gethash \"position\" (nth 2 test--captured))"),
        "nil"
    );

    // Deliver: point (at point-min) jumps forward to the first symbol, "top".
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {payload:?}))"),
    );
    let top_pos = run(&mut i, "(lsp--pos-at-utf16 0 7)");
    assert_eq!(run(&mut i, "(point)"), top_pos);

    // Next: jumps forward to "a".
    ok(&mut i, "(lsp-next-symbol)");
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {payload:?}))"),
    );
    let a_pos = run(&mut i, "(lsp--pos-at-utf16 1 14)");
    assert_eq!(run(&mut i, "(point)"), a_pos);

    // Next: jumps forward to "f".
    ok(&mut i, "(lsp-next-symbol)");
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {payload:?}))"),
    );
    let f_pos = run(&mut i, "(lsp--pos-at-utf16 2 15)");
    assert_eq!(run(&mut i, "(point)"), f_pos);

    // Next again from the last symbol: wraps around to "top".
    ok(&mut i, "(lsp-next-symbol)");
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {payload:?}))"),
    );
    assert_eq!(
        run(&mut i, "(point)"),
        top_pos,
        "must wrap to the first symbol"
    );
}

#[test]
fn previous_symbol_wraps_to_the_last_symbol() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("prev_sym");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");

    let payload = "[{\"name\":\"top\",\"kind\":6,\
                       \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":3,\"character\":9}},\
                       \"selectionRange\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":10}},\
                       \"children\":[\
                         {\"name\":\"a\",\"kind\":13,\
                          \"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":16}},\
                          \"selectionRange\":{\"start\":{\"line\":1,\"character\":14},\"end\":{\"line\":1,\"character\":15}}},\
                         {\"name\":\"f\",\"kind\":12,\
                          \"range\":{\"start\":{\"line\":2,\"character\":2},\"end\":{\"line\":2,\"character\":47}},\
                          \"selectionRange\":{\"start\":{\"line\":2,\"character\":15},\"end\":{\"line\":2,\"character\":16}}}\
                       ]}]";

    // Point at the very start -- there's nothing before it, so
    // lsp-previous-symbol must wrap to the LAST symbol, "f".
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-previous-symbol)");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentSymbol\""
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {payload:?}))"),
    );
    let f_pos = run(&mut i, "(lsp--pos-at-utf16 2 15)");
    assert_eq!(
        run(&mut i, "(point)"),
        f_pos,
        "must wrap to the last symbol"
    );
}

// ============================================================
// M46 fix round -- C2: staleness must be checked against buffer content
// (`buffer-modified-tick`), not just buffer identity -- typing while a
// format request is in flight must not let the stale reply clobber it.
// ============================================================

#[test]
fn format_buffer_discards_a_stale_reply_if_the_buffer_changed_since_the_request() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("stale_fmt");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(setq test--messages nil)");
    ok(
        &mut i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );

    ok(&mut i, "(lsp-format-buffer)");

    // The user keeps typing while the (fake) server is still "computing".
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"// typed while waiting\\n\")");
    let buffer_after_typing = run(&mut i, "(buffer-string)");

    // The reply finally lands, describing edits computed against the
    // OLD (pre-typing) snapshot.
    let new_text = "module top;\n  logic [3:0] a;\n  function int f(int x);\n    return x+1;\n  endfunction\nendmodule\n";
    let edits = format!(
        "[{{\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":3,\"character\":0}}}},\"newText\":{new_text:?}}}]"
    );
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {edits:?}))"),
    );

    // The stale edits must NOT have been applied -- the buffer keeps
    // exactly what the user typed, untouched.
    assert_eq!(run(&mut i, "(buffer-string)"), buffer_after_typing);
    assert!(
        run(&mut i, "(car test--messages)").contains("stale")
            || run(&mut i, "(car test--messages)").contains("changed"),
        "expected a user-visible message about the discarded stale reply, got {:?}",
        run(&mut i, "test--messages")
    );
}

// ============================================================
// M57: capability gate for `textDocument/formatting'/`rangeFormatting'.
// `stub_client_with_capabilities`-style helper, self-contained per this
// file's own convention (no cross-file sharing -- see
// `verilog_complete_tests.rs`'s helper of the same shape).
// ============================================================

/// A `lsp--client' whose `conn' is nil (passes `lsp--live-buffer-client's
/// alive gate untouched, same convention `lsp_mode_tests.rs'/
/// `verilog_complete_tests.rs' use) and whose `capabilities' slot is
/// CAPS-EXPR, a raw elisp expression evaluated in the caller's buffer.
fn stub_client_with_capabilities(interp: &mut Interp, caps_expr: &str) {
    ok(
        interp,
        &format!(
            "(setq-local lsp--buffer-client (make-lsp--client :conn nil :capabilities {}))",
            caps_expr
        ),
    );
}

/// Stubs `lsp-request-async' to capture its call (or record that it was
/// never called) into `test--captured'/`test--request-sent'.
fn stub_request_async(i: &mut Interp) {
    ok(i, "(setq test--captured nil)");
    ok(i, "(setq test--request-sent nil)");
    ok(
        i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--request-sent t)
                 (setq test--captured (list client method params callback))
                 99))",
    );
}

/// Stubs `message' to accumulate every call (most recent first) into
/// `test--messages'.
fn stub_message(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

#[test]
fn format_buffer_key_absent_blocks_the_request_and_messages() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_buf_absent");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(&mut i, "(make-hash-table)");
    stub_request_async(&mut i);
    stub_message(&mut i);

    ok(&mut i, "(lsp-format-buffer)");

    assert_eq!(run(&mut i, "test--request-sent"), "nil");
    assert!(
        run(&mut i, "(car test--messages)").contains("documentFormattingProvider"),
        "expected a message naming the missing capability, got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn format_region_key_absent_blocks_the_request_and_messages() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_region_absent");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(&mut i, "(make-hash-table)");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    stub_request_async(&mut i);
    stub_message(&mut i);

    ok(&mut i, "(lsp-format-region)");

    assert_eq!(run(&mut i, "test--request-sent"), "nil");
    assert!(
        run(&mut i, "(car test--messages)").contains("documentRangeFormattingProvider"),
        "expected a message naming the missing capability, got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn format_region_key_present_but_false_still_sends_the_request() {
    // Region counterpart of `format_buffer_key_present_but_false_...`:
    // only the REGION key is present (and falsy) -- the BUFFER key is
    // absent entirely, so this also proves the two commands don't share
    // one key (a falsy-but-present buffer key would pass just as well,
    // hiding a copy-paste-the-wrong-key bug).
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_region_false");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"documentRangeFormattingProvider\" :false h) h)",
    );
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-region)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn format_region_capabilities_nil_still_sends_the_request() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_region_nil_caps");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(&mut i, "nil");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-region)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn format_region_non_struct_client_still_sends_the_request() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_region_fake_client");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-region)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn format_buffer_and_format_region_use_independent_capability_keys() {
    // Direct cross-check that the two commands query DIFFERENT keys:
    // only "documentFormattingProvider" (the BUFFER key) is present.
    // If `lsp-format-region` were (by mistake) gated on that same key
    // instead of its own "documentRangeFormattingProvider", it would
    // send here too -- this catches exactly that copy-paste-the-wrong-
    // key mutation, which none of the single-command tests above can
    // (each only ever populates the one key it's testing).
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_independent_keys");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"documentFormattingProvider\" t h) h)",
    );
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    stub_request_async(&mut i);
    stub_message(&mut i);

    ok(&mut i, "(lsp-format-buffer)");
    assert_eq!(
        run(&mut i, "test--request-sent"),
        "t",
        "buffer key is present -- lsp-format-buffer must send"
    );

    ok(&mut i, "(setq test--request-sent nil)");
    ok(&mut i, "(setq test--messages nil)");
    ok(&mut i, "(lsp-format-region)");
    assert_eq!(
        run(&mut i, "test--request-sent"),
        "nil",
        "region key is absent -- lsp-format-region must NOT send, even \
         though the (different) buffer key is present"
    );
    assert!(
        run(&mut i, "(car test--messages)").contains("documentRangeFormattingProvider"),
        "expected a message naming the missing REGION capability, got {:?}",
        run(&mut i, "test--messages")
    );
}

#[test]
fn format_buffer_key_present_but_false_still_sends_the_request() {
    // Mirrors the M46 `hoverProvider: false' finding -- a falsy value is
    // not evidence of anything, so the gate must not read the value.
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_buf_false");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"documentFormattingProvider\" :false h) h)",
    );
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-buffer)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn format_buffer_capabilities_nil_still_sends_the_request() {
    // A client that predates the `capabilities' field (or a genuinely
    // empty/malformed `initialize' reply) must not regress to M46's
    // pre-gate behavior of never sending anything.
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_buf_nil_caps");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    stub_client_with_capabilities(&mut i, "nil");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-buffer)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn format_buffer_non_struct_client_still_sends_the_request() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("fmt_buf_fake_client");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    stub_request_async(&mut i);

    ok(&mut i, "(lsp-format-buffer)");

    assert_eq!(run(&mut i, "test--request-sent"), "t");
}

#[test]
fn unsupported_features_note_lists_both_missing_commands() {
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(&mut i, "(make-hash-table)");
    let note = run(
        &mut i,
        "(lsp--unsupported-features-note lsp--buffer-client)",
    );
    // Full literal comparison -- including the "unsupported: " prefix
    // and the ", "-joined format -- not just substring checks, so a
    // format regression (wrong separator, dropped prefix, ...) shows up
    // here rather than passing silently.
    assert_eq!(
        note,
        "\"unsupported: lsp-format-buffer, lsp-format-region\""
    );
}

#[test]
fn unsupported_features_note_only_buffer_key_absent_omits_region() {
    // Asymmetric case: only "documentRangeFormattingProvider" (the
    // REGION key) is present, so only lsp-format-buffer is missing.
    // Catches the two `lsp--gated-features-alist' entries being
    // copy-pasted to point at the same key -- that mutation would make
    // this note ALSO mention lsp-format-region, which the assertion
    // below explicitly rejects.
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"documentRangeFormattingProvider\" t h) h)",
    );
    let note = run(
        &mut i,
        "(lsp--unsupported-features-note lsp--buffer-client)",
    );
    assert!(note.contains("lsp-format-buffer"), "note = {note}");
    assert!(!note.contains("lsp-format-region"), "note = {note}");
}

#[test]
fn unsupported_features_note_only_region_key_absent_omits_buffer() {
    // Mirror of the test above: only "documentFormattingProvider" (the
    // BUFFER key) is present, so only lsp-format-region is missing.
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"documentFormattingProvider\" t h) h)",
    );
    let note = run(
        &mut i,
        "(lsp--unsupported-features-note lsp--buffer-client)",
    );
    assert!(note.contains("lsp-format-region"), "note = {note}");
    assert!(!note.contains("lsp-format-buffer"), "note = {note}");
}

#[test]
fn unsupported_features_note_nil_when_capabilities_complete() {
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table)))
           (puthash \"documentFormattingProvider\" t h)
           (puthash \"documentRangeFormattingProvider\" t h)
           h)",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--unsupported-features-note lsp--buffer-client)"
        ),
        "nil"
    );
}

#[test]
fn unsupported_features_note_nil_when_capabilities_are_nil() {
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(&mut i, "nil");
    assert_eq!(
        run(
            &mut i,
            "(lsp--unsupported-features-note lsp--buffer-client)"
        ),
        "nil"
    );
}

#[test]
fn unsupported_features_note_nil_when_keys_present_but_false() {
    let (mut i, _ed) = setup();
    stub_client_with_capabilities(
        &mut i,
        "(let ((h (make-hash-table)))
           (puthash \"documentFormattingProvider\" :false h)
           (puthash \"documentRangeFormattingProvider\" :false h)
           h)",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--unsupported-features-note lsp--buffer-client)"
        ),
        "nil"
    );
}

// ============================================================
// M57 e2e: real servers, following `lsp_mode_tests.rs`'s
// `manual_e2e_verible_verilog_ls_...` convention -- PATH-check-only skip
// (no `#[ignore]`), since these exercise this editor's primary-language
// server. `demo/rtl` is the project root deliberately (not the edited
// file's own directory) -- `.slang'/`verible.filelist' markers, and
// `lsp--project-root' picking the wrong root is a documented failure
// mode (see `lsp-server-alist`'s own docstring on `slang-server`
// indexing).
// ============================================================

/// Whether CMD resolves to a real executable on PATH -- same technique
/// as `lsp_mode_tests.rs`'s `have_on_path` (spawn with a harmless flag,
/// kill immediately; this file keeps its own copy per this repo's
/// no-shared-test-helpers convention).
fn have_on_path(cmd: &str) -> bool {
    match std::process::Command::new(cmd)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            true
        }
        Err(_) => false,
    }
}

fn demo_rtl_root() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo/rtl"))
}

// No polling loop needed here, unlike the hover/definition e2e tests in
// `lsp_mode_tests.rs`: `lsp-connect`'s `initialize` round trip is
// SYNCHRONOUS (`lsp--await`, see the file header's standing note on
// why), so by the time `(lsp)` returns, `lsp--client-capabilities` is
// already populated and `lsp--unsupported-features-note` has already
// run once (`lsp`'s own connect-time call) -- both are readable
// immediately off `(lsp)`'s own return value / the client it left in
// `lsp--buffer-client`.

#[test]
fn manual_e2e_verible_verilog_ls_advertises_formatting_note_is_nil() {
    if !have_on_path("verible-verilog-ls") {
        eprintln!("skipping: verible-verilog-ls not on PATH");
        return;
    }
    let (mut i, _ed) = setup();
    let root = demo_rtl_root();
    let file = root.join("core").join("alu.sv");
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"verible-verilog-ls\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    let note = run(
        &mut i,
        "(lsp--unsupported-features-note lsp--buffer-client)",
    );
    println!("note => {note}");

    // Kill the server BEFORE asserting: a panic here would skip the
    // teardown, and the surviving child inherits this test binary's
    // stdout -- `cargo test` then waits forever for an EOF that never
    // comes, so the whole suite HANGS instead of reporting a failure.
    // Measured 2026-08-11 while running M57's mutation list: mutating
    // the connect-time note away turned this file's slang-server e2e
    // from FAIL into a 200s timeout plus an orphaned server process.
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");

    assert_eq!(r, "\"LSP: connected to verible-verilog-ls\"");
    assert_eq!(note, "nil", "verible-verilog-ls advertises formatting");
}

#[test]
fn manual_e2e_slang_server_note_mentions_formatting() {
    if !have_on_path("slang-server") {
        eprintln!("skipping: slang-server not on PATH");
        return;
    }
    let (mut i, _ed) = setup();
    let root = demo_rtl_root();
    let file = root.join("core").join("alu.sv");
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"slang-server\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    let note = run(
        &mut i,
        "(lsp--unsupported-features-note lsp--buffer-client)",
    );
    println!("note => {note}");

    // Teardown before the assertions -- see the verible e2e above for
    // why (a panic before this line leaks the server and hangs the
    // whole suite instead of failing it).
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");

    assert!(
        r.starts_with("\"LSP: connected to slang-server"),
        "unexpected connect message: {r}"
    );
    assert!(
        r.contains("lsp-format-buffer") && r.contains("lsp-format-region"),
        "connect message doesn't report the missing formatting capability: {r}"
    );
    assert_ne!(note, "nil", "slang-server has no formatting support");
    assert!(
        note.contains("lsp-format-buffer") || note.contains("lsp-format-region"),
        "note doesn't mention formatting: {note}"
    );
}

// ============================================================
// M48 Part F: `lsp-goto-symbol-by-name` (M47) had no integration test at
// all before this -- only the pure `lsp--symbol-alist` helper was
// covered (lsp_async_tests.rs). M48's `lsp-code-action-at-point`/
// `lsp-rename` copy this exact "async request -> callback -> open
// `with-completing-read` picker -> second staleness check -> act" shape,
// so this pins down that the pattern itself works end to end before
// anything new is built on top of it. `completing-read` is stubbed
// (never driven by real minibuffer keystrokes) so the picker's own
// selection UI/history/panel machinery -- exercised elsewhere -- isn't
// what's under test here; only `lsp-goto-symbol-by-name`'s own wiring
// into it is.
// ============================================================

const THREE_SYMBOL_PAYLOAD: &str = "[{\"name\":\"top\",\"kind\":6,\
                       \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":3,\"character\":9}},\
                       \"selectionRange\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":10}},\
                       \"children\":[\
                         {\"name\":\"a\",\"kind\":13,\
                          \"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":16}},\
                          \"selectionRange\":{\"start\":{\"line\":1,\"character\":14},\"end\":{\"line\":1,\"character\":15}}},\
                         {\"name\":\"f\",\"kind\":12,\
                          \"range\":{\"start\":{\"line\":2,\"character\":2},\"end\":{\"line\":2,\"character\":47}},\
                          \"selectionRange\":{\"start\":{\"line\":2,\"character\":15},\"end\":{\"line\":2,\"character\":16}}}\
                       ]}]";

#[test]
fn goto_symbol_by_name_full_round_trip_through_the_completing_read_picker() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("goto_sym");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");

    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(setq test--cr-args nil)");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (setq test--cr-args (list prompt collection require-match))
                 (funcall callback \"f\")))",
    );

    ok(&mut i, "(lsp-goto-symbol-by-name)");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/documentSymbol\""
    );

    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {THREE_SYMBOL_PAYLOAD:?}))"),
    );

    // The picker got every symbol's name, in document order, with
    // REQUIRE-MATCH set (M47's `lsp--symbol-alist` contract).
    assert_eq!(
        run(&mut i, "(nth 1 test--cr-args)"),
        "(\"top\" \"a\" \"f\")"
    );
    assert_eq!(run(&mut i, "(nth 2 test--cr-args)"), "t");

    // Picking "f" moved point to its selectionRange.start.
    let f_pos = run(&mut i, "(lsp--pos-at-utf16 2 15)");
    assert_eq!(run(&mut i, "(point)"), f_pos);
}

#[test]
fn goto_symbol_by_name_second_staleness_check_ignores_a_pick_made_after_switching_buffers() {
    let (mut i, _ed) = setup();
    let file = write_verilog_scratch_file("goto_sym_stale");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");
    ok(&mut i, "(goto-char (point-min))");
    let point_before = run(&mut i, "(point)");

    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    // The user switches to another buffer WHILE the picker is up (the
    // gap `with-completing-read`'s own doc comment calls out -- an
    // arbitrary amount of real time/typing can pass between the picker
    // opening and this callback running), then picks a symbol anyway.
    ok(&mut i, "(generate-new-buffer \"*goto-sym-other*\")");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (switch-to-buffer-internal \"*goto-sym-other*\")
                 (funcall callback \"f\")))",
    );

    ok(&mut i, "(lsp-goto-symbol-by-name)");
    ok(
        &mut i,
        &format!("(funcall (nth 3 test--captured) (json-parse-string {THREE_SYMBOL_PAYLOAD:?}))"),
    );

    // Still in the other buffer, at whatever point it started at -- the
    // stale pick must not have jumped anywhere in the original buffer.
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*goto-sym-other*\"");
    ok(
        &mut i,
        &format!(
            "(switch-to-buffer-internal {:?})",
            file.file_name().unwrap().to_str().unwrap()
        ),
    );
    assert_eq!(
        run(&mut i, "(point)"),
        point_before,
        "must not have jumped in the original buffer either"
    );
}
