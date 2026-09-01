//! M40-4: the cursor-anchored LSP completion popup -- `CompletionPopup`
//! (editor.rs), the `show-completion-popup`/`hide-completion-popup`/
//! `completion-popup-active-p` builtins (builtins/ui.rs), the
//! `completion_popup_key` branch in `commands::handle_key`, its grid
//! rendering (redisplay.rs), and `lsp.el`'s `lsp-completion-at-point`/
//! `completion-at-point` wiring (`C-M-i`).
//!
//! Same fset-stub discipline as `lsp_mode_tests.rs`'s hover/definition
//! tests: `lsp-request-async` is replaced with a capturing lambda so the
//! request/callback *logic* (0-based position, prefix scan and filter,
//! staleness guards) is exercised without a real server or subprocess. A
//! plain symbol (`'fake-client`) stands in for a live `lsp--client`,
//! passing `lsp--live-buffer-client`'s alive-gate untouched -- see that
//! function's own doc comment in lsp.el.
//!
//! Rendering assertions read the character grid directly (`row_text`),
//! same approach as `gui_features_tests.rs`'s `row_text` -- no GUI/TUI
//! needed, since M40-4 draws the popup onto the grid both frontends
//! share.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use elisp::{Interp, Value};

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (60, 20);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => elisp::printer::prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn ok(interp: &mut Interp, src: &str) -> String {
    let r = run(interp, src);
    assert!(!r.starts_with("ERROR"), "{:?} failed: {}", src, r);
    r
}

fn feed(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, keys: &str) {
    feed_keys(interp, ed, keys).unwrap_or_else(|e| panic!("feed_keys {:?}: {}", keys, e));
}

/// Types `s` one character at a time via raw key events (electric-pair
/// tests' / indent tests' own `type_str`): needed so each character goes
/// through `self_insert` exactly like real typing, not a batch `insert`.
fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

fn bs(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-string)") {
        Ok(Value::Str(s)) => (*s).clone(),
        other => panic!(
            "(buffer-string) didn't return a string: {:?}",
            other.is_ok()
        ),
    }
}

fn pt(interp: &mut Interp) -> i64 {
    match interp.eval_source("(point)") {
        Ok(Value::Int(n)) => n,
        other => panic!("(point) didn't return an int: {:?}", other.is_ok()),
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
            "reticle_completion_popup_{}_{}_{}",
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

/// A fresh scratch directory under the OS temp dir, unique per test run
/// (mirrors `lsp_mode_tests.rs`'s helper of the same name).
fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// A buffer visiting TEXT on disk, with a stand-in LSP client attached
/// (`lsp--live-buffer-client`'s alive-gate passes a plain symbol
/// through untouched -- see its doc comment) so `lsp-completion-at-
/// point` takes the async-request branch instead of "no client
/// connected". Returns the scratch directory too, for cleanup.
fn setup_connected(text: &str) -> (Interp, Rc<RefCell<Editor>>, Scratch) {
    let mut i = elisp::new_interp();
    let ed = core::init_editor(&mut i);
    ed.borrow_mut().frame = (60, 20);
    let dir = scratch_dir("t");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, text).unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    (i, ed, dir)
}

/// Replace `lsp-request-async` with a lambda that captures its
/// arguments into the global `test--captured` (client method params
/// callback) instead of sending anything -- same isolation
/// `lsp_mode_tests.rs`'s hover/definition tests use.
fn stub_capture(interp: &mut Interp) {
    ok(interp, "(setq test--captured nil)");
    ok(
        interp,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
}

/// One grid row's text, trailing spaces trimmed (mirrors
/// `gui_features_tests.rs`'s `row_text`).
fn row_text(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Characters in one grid row between columns `[start, end)`, trailing
/// spaces NOT trimmed (unlike `row_text`) -- needed to inspect just one
/// split window's own slice of a row without the neighboring window's
/// (or the popup's) content on the same row washing out a `trim_end`.
fn row_cols(
    interp: &Interp,
    ed: &Rc<RefCell<Editor>>,
    row: usize,
    start: usize,
    end: usize,
) -> String {
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row][start..end]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect()
}

fn cursor_row(interp: &Interp, ed: &Rc<RefCell<Editor>>) -> usize {
    core::redisplay::render(interp, ed).cursor.0
}

/// One `(LABEL INSERT START FILTER)` entry of `show-completion-popup`'s
/// ITEMS argument (M44-3), as elisp source text -- START is a plain
/// integer (1-based buffer position, same convention as every other
/// position argument, see `get_pos`).
fn item_src(label: &str, insert: &str, start: i64, filter: &str) -> String {
    format!("(list {:?} {:?} {} {:?})", label, insert, start, filter)
}

/// `(list ITEM...)` for a whole ITEMS argument, built from `item_src`
/// entries -- the fixture shape every direct `show-completion-popup`
/// call in this file now needs (M44-3 replaced the old `(LABEL
/// . INSERT)` conses with per-item `(LABEL INSERT START FILTER)`
/// lists).
fn items_src(entries: &[(&str, &str, i64, &str)]) -> String {
    format!(
        "(list {})",
        entries
            .iter()
            .map(|(l, ins, s, f)| item_src(l, ins, *s, f))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// `(show-completion-popup ITEMS PREFIX-START)` elisp source text, built
/// from `items_src` -- the fixture shape every direct `show-completion-
/// popup` call in this file now needs (M44 review fix #1: PREFIX-START
/// is a new required argument, a plain integer 1-based buffer position
/// same as every `item_src` START -- see `CompletionPopup::
/// prefix_start`'s doc comment in editor.rs for what it anchors).
fn show_popup_src(entries: &[(&str, &str, i64, &str)], prefix_start: i64) -> String {
    format!(
        "(show-completion-popup {} {})",
        items_src(entries),
        prefix_start
    )
}

/// A prog-mode (c-mode) buffer: `prog-mode-hook` has run, so
/// `electric-pair-mode` is buffer-locally on (electric_pair_tests.rs's
/// own `setup_prog`) -- needed for the electric-pair/popup interaction
/// test below.
fn setup_prog() -> (Interp, Rc<RefCell<Editor>>) {
    let (mut i, ed) = setup();
    ok(&mut i, "(c-mode)");
    (i, ed)
}

// ============================================================
// 1. C-M-i sends a 0-based position and opens the popup on answer
// ============================================================

#[test]
fn c_m_i_sends_0_based_position_and_opens_popup_on_answer() {
    let (mut i, ed, _dir) = setup_connected("hello");
    ok(&mut i, "(goto-char (point-max))"); // right after "hello"
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    assert_eq!(run(&mut i, "(car test--captured)"), "fake-client");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/completion\""
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
        "5"
    );

    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"hello_world\\\"}]\"))",
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(text.contains("hello_world"), "row {}: {:?}", row, text);
}

// ============================================================
// 2. C-n/C-p cycle selection (wrapping); RET accepts
// ============================================================

#[test]
fn c_n_cycles_selection_wrapping_and_ret_accepts() {
    let (mut i, ed) = setup();
    type_str(&mut i, &ed, "fo");
    ok(
        &mut i,
        &show_popup_src(
            &[("alpha", "Alpha", 1, "alpha"), ("beta", "Beta", 1, "beta")],
            1,
        ),
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    feed(&mut i, &ed, "C-n"); // alpha -> beta
    feed(&mut i, &ed, "C-n"); // beta -> wraps back to alpha
    feed(&mut i, &ed, "RET");

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(bs(&mut i), "Alpha");
    assert_eq!(pt(&mut i), 6);
}

#[test]
fn c_p_wraps_backward_and_tab_also_accepts() {
    let (mut i, ed) = setup();
    type_str(&mut i, &ed, "fo");
    ok(
        &mut i,
        &show_popup_src(
            &[("alpha", "Alpha", 1, "alpha"), ("beta", "Beta", 1, "beta")],
            1,
        ),
    );
    feed(&mut i, &ed, "C-p"); // alpha -> wraps backward to beta
    feed(&mut i, &ed, "TAB");

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(bs(&mut i), "Beta");
    assert_eq!(pt(&mut i), 5);
}

// ============================================================
// 3. Prefix filtering: only the matching candidate is listed and
//    accepting replaces just the prefix, not the whole insert on top
// ============================================================

#[test]
fn prefix_filters_candidates_and_accept_replaces_just_the_prefix() {
    let (mut i, ed, _dir) = setup_connected("");
    type_str(&mut i, &ed, "fo");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"foo_bar\\\"},{\\\"label\\\":\\\"baz\\\"}]\"))",
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(text.contains("foo_bar"), "row {}: {:?}", row, text);
    assert!(
        !text.contains("baz"),
        "baz should have been filtered out by the \"fo\" prefix: {:?}",
        text
    );

    feed(&mut i, &ed, "RET");
    // "fo" is replaced by "foo_bar", not appended to ("fofoo_bar").
    assert_eq!(bs(&mut i), "foo_bar");
    assert_eq!(pt(&mut i), 8);
}

// ============================================================
// 4. ESC closes the popup without leaving insert state
// ============================================================

#[test]
fn esc_closes_popup_without_leaving_insert_state() {
    let (mut i, ed) = setup();
    ok(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "i");
    assert_eq!(run(&mut i, "evil--state"), "insert");

    ok(&mut i, &show_popup_src(&[("a", "A", 1, "a")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    feed(&mut i, &ed, "ESC");

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(
        run(&mut i, "evil--state"),
        "insert",
        "the first ESC must only close the popup, not exit insert state"
    );
    assert_eq!(bs(&mut i), "");
}

// ============================================================
// 5. M44-3: a printable key now KEEPS the popup open and filters
//    (replaces the old M40-4 "any other key closes" test for
//    self-insert characters -- see the split-off "other key" test
//    right below for the part of that old test's behavior that DID
//    survive unchanged)
// ============================================================

#[test]
fn printable_key_keeps_popup_open_and_filters_candidates() {
    let (mut i, ed) = setup();
    ok(
        &mut i,
        &show_popup_src(
            &[
                ("apple", "APPLE", 1, "apple"),
                ("banana", "BANANA", 1, "banana"),
            ],
            1,
        ),
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    type_str(&mut i, &ed, "a");

    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "t",
        "a printable key must keep the popup open, not close it"
    );
    assert_eq!(
        bs(&mut i),
        "a",
        "the character must still land via ordinary self-insert"
    );
    assert_eq!(pt(&mut i), 2);
    // "banana" no longer starts with "a" -> filtered out, "apple" stays.
    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(text.contains("apple"), "row: {:?}", text);
    assert!(!text.contains("banana"), "row: {:?}", text);
}

// ============================================================
// 5b. A non-printable, non-DEL key still closes the popup immediately
//     and falls through -- unchanged from M40-4 (`completion_popup_key`'s
//     three-way split's third bucket, "keep today's behaviour")
// ============================================================

#[test]
fn other_key_c_x_still_closes_popup_and_falls_through() {
    let (mut i, ed) = setup();
    ok(&mut i, &show_popup_src(&[("a", "A", 1, "a")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // C-x is a prefix key (C-x b, C-x C-f, ...) -- neither self-insert
    // nor DEL, so it hits the wildcard bucket: close immediately, then
    // fall through to its own (unrelated) prefix dispatch.
    feed(&mut i, &ed, "C-x");

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 6. No client: C-M-i falls through to dabbrev-expand
// ============================================================

#[test]
fn c_m_i_falls_back_to_dabbrev_expand_when_no_client_is_connected() {
    let (mut i, ed) = setup();
    // Same fixture as dabbrev_tests.rs's `evil_insert_c_p_expands_...`:
    // `dabbrev-expand`'s own traditional direction (nearest match
    // BEFORE point first) matches evil's C-p, not C-n -- see evil.el's
    // M31 section header.
    ok(&mut i, "(insert \"printhis\\n\\nprintln\\n\")");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 1)"); // the blank line
    type_str(&mut i, &ed, "pri");

    feed(&mut i, &ed, "C-M-i");

    assert_eq!(bs(&mut i), "printhis\nprinthis\nprintln\n");
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 7. Null result: "No completions" message, no popup
// ============================================================

#[test]
fn null_result_shows_no_completions_message_and_no_popup() {
    let (mut i, ed, _dir) = setup_connected("foo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    let out = run(&mut i, "(funcall (nth 3 test--captured) nil)");
    assert_eq!(out, "\"No completions\"");
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 8. Render geometry: near the window bottom, popup renders ABOVE
// ============================================================

#[test]
fn popup_renders_above_cursor_when_window_bottom_is_near() {
    let (mut i, ed) = setup();
    // rows=6 -> windows_height=5 -> window rect height=5 -> text_rows=4
    // (grid rows 0..3), modeline row=4, echo row=5.
    ed.borrow_mut().frame = (40, 6);
    ok(&mut i, "(insert \"a\\nb\\nc\\nHERE\")");
    ok(&mut i, "(goto-char (point-max))"); // end of "HERE", the 4th (last) text row

    let cr = cursor_row(&i, &ed);
    assert_eq!(cr, 3, "sanity: cursor should sit on the last text row");

    ok(
        &mut i,
        "(show-completion-popup
               (list (list \"AAA\" \"aaa\" (point) \"AAA\")
                     (list \"BBB\" \"bbb\" (point) \"BBB\")
                     (list \"CCC\" \"ccc\" (point) \"CCC\"))
               (point))",
    );

    // below_room (echo_row 5 - (cursor_row 3 + 1) = 1) is less than the
    // 3 rows needed, so the popup flips above: rows 0..2.
    assert!(
        row_text(&i, &ed, 0).contains("AAA"),
        "row0: {:?}",
        row_text(&i, &ed, 0)
    );
    assert!(
        row_text(&i, &ed, 1).contains("BBB"),
        "row1: {:?}",
        row_text(&i, &ed, 1)
    );
    assert!(
        row_text(&i, &ed, 2).contains("CCC"),
        "row2: {:?}",
        row_text(&i, &ed, 2)
    );
    // Row 3 is the cursor's own buffer line, left untouched.
    assert!(row_text(&i, &ed, 3).contains("HERE"));
    // Never drawn below -- confirms this is really "above", not "below".
    assert!(!row_text(&i, &ed, 4).contains("AAA"));
    assert!(!row_text(&i, &ed, 5).contains("AAA"));
}

// ============================================================
// 8b. Render geometry (M40-4 review issue #2): more candidates than fit
//     above the cursor must be CLAMPED, never overrun the cursor's own
//     row or the modeline
// ============================================================

#[test]
fn popup_row_count_is_clamped_to_the_window_and_never_covers_cursor_or_modeline() {
    let (mut i, ed) = setup();
    // Same (40, 6) frame as test 8: windows_height=5 -> window rect
    // height=5 -> text_rows=4 (grid rows 0..3), modeline row=4, echo
    // row=5.
    ed.borrow_mut().frame = (40, 6);
    ok(&mut i, "(insert \"a\\nb\\nc\\nHERE\")");
    ok(&mut i, "(goto-char (point-max))"); // end of "HERE", the last text row
    let cr = cursor_row(&i, &ed);
    assert_eq!(cr, 3, "sanity: cursor should sit on the last text row");

    // Capture the cursor's row and the modeline row BEFORE the popup, so
    // the assertions below don't have to guess at exact modeline text.
    let cursor_line_before = row_text(&i, &ed, 3);
    let mode_line_before = row_text(&i, &ed, 4);
    assert!(cursor_line_before.contains("HERE"));

    // 5 candidates: below_room is 0 (the cursor sits on the window's
    // last text row) and above_room is only 3 (rows 0..2), so with the
    // #2 fix the popup must clamp to 3 rows -- rows 0..2 -- and skip
    // "DDD"/"EEE" entirely, rather than the pre-fix behavior of
    // painting 5 rows (0..4) and overrunning both the cursor's row (3)
    // and the modeline (4).
    ok(
        &mut i,
        "(show-completion-popup
               (list (list \"AAA\" \"aaa\" (point) \"AAA\")
                     (list \"BBB\" \"bbb\" (point) \"BBB\")
                     (list \"CCC\" \"ccc\" (point) \"CCC\")
                     (list \"DDD\" \"ddd\" (point) \"DDD\")
                     (list \"EEE\" \"eee\" (point) \"EEE\"))
               (point))",
    );

    assert!(
        row_text(&i, &ed, 0).contains("AAA"),
        "row0: {:?}",
        row_text(&i, &ed, 0)
    );
    assert!(
        row_text(&i, &ed, 1).contains("BBB"),
        "row1: {:?}",
        row_text(&i, &ed, 1)
    );
    assert!(
        row_text(&i, &ed, 2).contains("CCC"),
        "row2: {:?}",
        row_text(&i, &ed, 2)
    );
    // Clamped: DDD/EEE never fit anywhere and must not be drawn at all.
    for row in 0..6 {
        let text = row_text(&i, &ed, row);
        assert!(
            !text.contains("DDD"),
            "row{} unexpectedly has DDD: {:?}",
            row,
            text
        );
        assert!(
            !text.contains("EEE"),
            "row{} unexpectedly has EEE: {:?}",
            row,
            text
        );
    }
    // The cursor's own row and the modeline must be untouched.
    assert_eq!(
        row_text(&i, &ed, 3),
        cursor_line_before,
        "the popup must never paint over the cursor's own row"
    );
    assert_eq!(
        row_text(&i, &ed, 4),
        mode_line_before,
        "the popup must never paint over the modeline"
    );
}

// ============================================================
// 8c. CJK label width (M40-4 review issue #4): popup width must use
//     DISPLAY width (double-width-aware), not char count, so a CJK
//     label isn't truncated to roughly half itself
// ============================================================

#[test]
fn cjk_label_is_not_truncated_by_a_char_count_vs_display_width_mismatch() {
    let (mut i, ed) = setup();
    ok(&mut i, "(insert \"x\")");
    ok(&mut i, "(goto-char (point-max))");

    // "介面設定" is 4 chars, each 2 columns wide (display width 8).
    // Pre-fix, `max_label` used `.chars().count()` (=4) for the popup's
    // column width while the draw loop measured display width via
    // `char_width` -- so the loop's own `col + w > width` guard broke
    // out after only 2 of the 4 characters.
    ok(
        &mut i,
        "(show-completion-popup (list (list \"介面設定\" \"設定\" (point) \"介面設定\")) (point))",
    );

    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(
        text.contains("介面設定"),
        "CJK label must render in full, row {}: {:?}",
        row,
        text
    );
}

// ============================================================
// 9a. M44-3: a reply arriving after MORE TYPING WITHIN THE SAME
//     identifier run is no longer stale -- it opens, already filtered
//     against the extra typing (the "request-time race" flavor of
//     filter-as-you-type). Replaces the M40-4 `stale_callback_after_
//     more_typing_does_not_open_popup` test's "more typing = stale"
//     premise for THIS case; see 9b below for the case that's still
//     discarded.
// ============================================================

#[test]
fn reply_after_more_typing_in_the_same_identifier_run_opens_filtered() {
    let (mut i, ed, _dir) = setup_connected("fo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    // Still identifier characters -- the run the request was about
    // never ended, just grew.
    type_str(&mut i, &ed, "o");

    // `show-completion-popup`'s own return value is always nil whether
    // or not it actually opened anything (see builtins/ui.rs), so the
    // reliable signal that the staleness `when' guard passed is
    // `completion-popup-active-p', not this funcall's return value.
    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"foo\\\"},{\\\"label\\\":\\\"bar\\\"}]\"))",
    );
    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "t",
        "typing within the same identifier run must not be treated as stale"
    );
    // "foo" now fully typed -> "bar" must already be filtered out.
    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(text.contains("foo"), "row: {:?}", text);
    assert!(!text.contains("bar"), "row: {:?}", text);
}

// ============================================================
// 9b. A reply arriving after the identifier run has ENDED (a non-prefix
//     character, e.g. a space, was typed) is still discarded silently --
//     the M44-3 identifier-run check catches this exactly like the old
//     (buffer, tick, point) triple did.
// ============================================================

#[test]
fn reply_after_typing_a_non_prefix_char_is_still_discarded() {
    let (mut i, ed, _dir) = setup_connected("fo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    // A space breaks the identifier run the request was about.
    type_str(&mut i, &ed, " ");

    let out = run(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"foo\\\"}]\"))",
    );
    assert_eq!(
        out, "nil",
        "a reply after the identifier run ended must still be discarded"
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 10. inhibit-self-insert guard: normal-state-by-the-time-it-answers
//     blocks the popup even when buffer/tick/point never moved
// ============================================================

#[test]
fn inhibit_self_insert_guard_blocks_popup_even_when_not_stale() {
    let (mut i, ed, _dir) = setup_connected("fo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    // Simulates having left insert state (evil ESC back to normal)
    // WITHOUT otherwise touching the buffer, so buffer/tick/point still
    // match exactly what the request captured -- isolates this guard
    // (`show-completion-popup' itself, M34's `inhibit-self-insert') from
    // the elisp-side staleness check covered by case 9 above.
    ok(&mut i, "(setq-local inhibit-self-insert t)");
    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"foo\\\"}]\"))",
    );

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 11. C-g clears the popup
// ============================================================

#[test]
fn c_g_clears_the_popup() {
    let (mut i, ed) = setup();
    type_str(&mut i, &ed, "fo");
    ok(&mut i, &show_popup_src(&[("a", "A", 1, "a")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    feed(&mut i, &ed, "C-g");

    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ============================================================
// 12. Async reply arriving after the user opened the minibuffer must
//     not hijack it (M40-4 review issue #3(a))
// ============================================================

#[test]
fn async_reply_arriving_while_minibuffer_is_open_does_not_open_the_popup() {
    let (mut i, ed, _dir) = setup_connected("fo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    // The user doesn't wait around for the server: M-x opens the
    // minibuffer before the reply arrives. Unlike case 9 (more typing),
    // M-x never touches the buffer -- buffer/tick/point are exactly
    // what the request captured, so lsp.el's own staleness `when' guard
    // would NOT catch this. It's `show-completion-popup' itself that
    // must refuse to open on top of an active minibuffer.
    feed(&mut i, &ed, "M-x");
    assert!(
        ed.borrow().minibuffer.is_some(),
        "sanity: M-x should have opened the minibuffer"
    );

    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"[{\\\"label\\\":\\\"foo\\\"}]\"))",
    );
    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "nil",
        "the popup must not open on top of an active minibuffer"
    );
    assert!(
        ed.borrow().minibuffer.is_some(),
        "the minibuffer must still be open"
    );

    // The minibuffer must still receive its own keys normally.
    type_str(&mut i, &ed, "beginning-of-b");
    handle_key(&mut i, &ed, Key::Char(9)); // TAB completes the command name
    assert_eq!(
        ed.borrow().minibuffer.as_ref().unwrap().input,
        "beginning-of-buffer"
    );
    feed(&mut i, &ed, "RET");
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(run(&mut i, "(point)"), "1");
}

// ============================================================
// 13. Even a popup that ends up open anyway must never hijack the
//     minibuffer's own keys (M40-4 review issue #3(b), defense in depth
//     for #3(a) above)
// ============================================================

#[test]
fn completion_popup_key_defends_the_minibuffer_even_if_a_popup_is_forced_open() {
    let (mut i, ed) = setup();
    ok(&mut i, "(insert \"background text\")");
    feed(&mut i, &ed, "M-x");
    assert!(
        ed.borrow().minibuffer.is_some(),
        "sanity: M-x should have opened the minibuffer"
    );

    // Force a popup open directly, bypassing `show-completion-popup`'s
    // own guard (case 12) entirely -- this pins the SECOND, independent
    // guard in `completion_popup_key` itself.
    ed.borrow_mut().completion_popup = Some(core::editor::CompletionPopup {
        items: vec![core::editor::PopupItem {
            label: "a".to_string(),
            insert: "A".to_string(),
            start: 1,
            filter: "a".to_string(),
        }],
        prefix_start: 1,
        selected: 0,
        incomplete: false,
        tick: 0,
    });

    // TAB hits the `Key::Char(9) => accept_completion` branch in
    // `completion_popup_key`. Without the #3(b) guard this would
    // delete-and-insert into the BACKGROUND buffer (not the minibuffer)
    // and consume the key before `minibuffer_key` ever saw it -- exactly
    // the "RET/TAB inserts into the wrong buffer" failure review issue
    // #3 describes.
    handle_key(&mut i, &ed, Key::Char(9));

    assert!(
        ed.borrow().completion_popup.is_none(),
        "the stale popup must be cleared once a minibuffer key reaches completion_popup_key"
    );
    assert_eq!(
        bs(&mut i),
        "background text",
        "TAB must not have run accept_completion against the background buffer"
    );
    assert!(
        ed.borrow().minibuffer.is_some(),
        "the minibuffer must still be open"
    );
}

// ============================================================
// 14. accept_completion is its own undo step (M40-4 review issue #5)
// ============================================================

#[test]
fn accept_completion_is_its_own_undo_step() {
    let (mut i, ed) = setup();
    // Typing "fo" accumulates `consec_inserts` without cutting a
    // boundary between the two characters.
    type_str(&mut i, &ed, "fo");
    ok(
        &mut i,
        &show_popup_src(&[("foo_bar", "foo_bar", 1, "foo_bar")], 1),
    );
    feed(&mut i, &ed, "RET"); // accept: "fo" -> "foo_bar"
    assert_eq!(bs(&mut i), "foo_bar");
    // More typing right after acceptance.
    type_str(&mut i, &ed, "baz");
    assert_eq!(bs(&mut i), "foo_barbaz");

    // Real key dispatch (C-/ -> `undo`, bound in simple.el), not a bare
    // `(undo)` eval: consecutive-undo chaining reads `editor.last_command`
    // (see `undo-internal` in builtins/editing.rs), which only
    // `execute_command`'s `call_command` sets -- a raw `eval_source`
    // call bypasses that entirely and would just toggle the same group
    // back and forth instead of walking further back.

    // First undo: only the "baz" typed after acceptance comes back off.
    feed(&mut i, &ed, "C-/");
    assert_eq!(
        bs(&mut i),
        "foo_bar",
        "first undo must remove only the post-acceptance typing"
    );
    // Second undo: the accepted candidate itself comes back off, to "fo".
    feed(&mut i, &ed, "C-/");
    assert_eq!(
        bs(&mut i),
        "fo",
        "second undo must remove only the accepted candidate"
    );
    // Third undo: the originally typed "fo" comes back off too.
    feed(&mut i, &ed, "C-/");
    assert_eq!(
        bs(&mut i),
        "",
        "third undo must remove the originally typed text"
    );
}

// ============================================================
// 15. Horizontal clamp (M40-4 review issue #2, second pass): a popup
//     anchored near a split window's right edge must not paint into
//     the neighboring window
// ============================================================

#[test]
fn popup_horizontal_clamp_does_not_paint_into_the_neighboring_split_window() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (50, 10);
    ok(&mut i, "(switch-to-buffer \"left\")");
    // 20 'a's on line 1 (cursor lands at column 20 right after it), then
    // a distinctive marker on line 2 -- rendered on BOTH split windows'
    // row 1 (same shared buffer, see `split_selected` in editor.rs), so
    // it doubles as a "was this row painted over" probe for the RIGHT
    // window specifically.
    ok(&mut i, "(insert \"aaaaaaaaaaaaaaaaaaaa\\nMARKERTEXT\")");
    ok(&mut i, "(goto-char 21)"); // end of line 1, before the newline

    let cr = cursor_row(&i, &ed);
    assert_eq!(cr, 0, "sanity: cursor should be on row 0");

    // `split-window-right` leaves the ORIGINAL window selected (it
    // becomes the split's "a"/left leaf -- see `split_selected` in
    // editor.rs), which is exactly the "selected window is the
    // narrower left one" setup this issue needs.
    ok(&mut i, "(split-window-right)");

    // Same arithmetic as `compute_rects`'s horizontal-split branch
    // (redisplay.rs): frame width 50, separator is 1 column since
    // width > 2 -> left window width = (50 - 1) / 2 = 24 (columns
    // 0..23), right window starts at column 24 + 1 = 25.
    let right_win_start = 25;
    let marker_before = row_cols(&i, &ed, 1, right_win_start, 50);
    assert!(
        marker_before.contains("MARKERTEXT"),
        "sanity: the right window's own row 1 must show MARKERTEXT \
         before the popup opens: {:?}",
        marker_before
    );

    // A 15-column-wide label anchored at cursor column 20 in a
    // 24-column-wide left window: `cursor_col + width` (35) is well
    // under the FRAME's width (50), so the pre-fix `cols`-based (i.e.
    // frame-width-based) clamp never shifted it at all -- it painted
    // absolute columns [20, 35), stomping through the separator and 10
    // columns into the right window (which starts at column 25),
    // exactly covering "MARKERTEXT" (10 characters, columns 25..34).
    ok(
        &mut i,
        "(show-completion-popup (list (list \"AAAAAAAAAAAAAAA\" \"x\" (point) \"AAAAAAAAAAAAAAA\")) (point))",
    );

    let marker_after = row_cols(&i, &ed, 1, right_win_start, 50);
    assert_eq!(
        marker_after, marker_before,
        "the popup must not paint into the neighboring (right) split window's own row"
    );
}

// ============================================================
// M44-3 new coverage below (D section of the milestone spec): textEdit,
// sortText, filter-as-you-type, pure-movement close, isIncomplete,
// electric-pair interaction, snippet literal insert, dabbrev regression.
// ============================================================

// ------------------------------------------------------------
// 19. textEdit: a candidate's own `textEdit.range.start' overrides the
//     ordinary prefix-start for THAT candidate only; a plain candidate
//     mixed into the same list keeps using the prefix-based start.
// ------------------------------------------------------------

#[test]
fn text_edit_start_overrides_prefix_start_for_its_own_candidate_only() {
    let (mut i, ed, _dir) = setup_connected("obj.");
    ok(&mut i, "(goto-char (point-max))"); // right after "obj."
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");
    // Sanity: "." breaks the identifier run, so the ordinary prefix scan
    // sees an EMPTY prefix here -- the interesting case for `textEdit'.
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "4"
    );

    ok(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "[{\"label\":\"whole\",\"filterText\":\"obj.\",\"textEdit\":{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":4}},\"newText\":\"REPLACED\"}},{\"label\":\"member\",\"insertText\":\"MEMBER\"}]"))"#,
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // Both survived their own (DIFFERENT) filter spans: "member"'s
    // empty-prefix span, and "whole"'s "obj."-wide `textEdit' span.
    // sortText is absent from both -> sorted by label: "member" < "whole".
    let row0 = cursor_row(&i, &ed) + 1;
    assert!(row_text(&i, &ed, row0).contains("member"));
    assert!(row_text(&i, &ed, row0 + 1).contains("whole"));

    feed(&mut i, &ed, "C-n"); // member -> whole
    feed(&mut i, &ed, "RET");

    // The accepted candidate's OWN `textEdit.range.start' (buffer
    // position 1, before "obj." entirely) is what got deleted -- not the
    // empty-prefix span the ordinary scan would have used.
    assert_eq!(bs(&mut i), "REPLACED");
}

// ------------------------------------------------------------
// 20. sortText: candidates sort by sortText (falling back to label),
//     stably (ties keep server order).
// ------------------------------------------------------------

#[test]
fn sort_text_orders_candidates_and_falls_back_to_label_stably() {
    let (mut i, ed, _dir) = setup_connected("");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    ok(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "[{\"label\":\"charlie\",\"sortText\":\"2\"},{\"label\":\"alpha\",\"sortText\":\"9\"},{\"label\":\"bravo\"},{\"label\":\"delta\",\"sortText\":\"2\"},{\"label\":\"echo\",\"sortText\":\"1\"}]"))"#,
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // "1" < "2" < "9" < "bravo" (ASCII digits sort before letters);
    // "charlie"/"delta" tie at sortText "2" -> stable, server order kept.
    let row0 = cursor_row(&i, &ed) + 1;
    let order = ["echo", "charlie", "delta", "alpha", "bravo"];
    for (offset, label) in order.iter().enumerate() {
        let text = row_text(&i, &ed, row0 + offset);
        assert!(
            text.contains(label),
            "row {}: expected {:?}, got {:?}",
            row0 + offset,
            label,
            text
        );
    }
}

// ------------------------------------------------------------
// 21. Filter-as-you-type: typing narrows candidates and resets
//     `selected' to 0; a character matching nothing closes the popup.
// ------------------------------------------------------------

#[test]
fn typing_narrows_candidates_resets_selection_and_closes_when_nothing_matches() {
    let (mut i, ed) = setup();
    ok(
        &mut i,
        &show_popup_src(
            &[
                ("apple", "APPLE", 1, "apple"),
                ("banana", "BANANA", 1, "banana"),
            ],
            1,
        ),
    );
    // Move off the default selection (0) so a later reset-to-0 is
    // actually observable (via what RET ends up accepting).
    feed(&mut i, &ed, "C-n"); // apple -> banana
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    type_str(&mut i, &ed, "a"); // both still start with "a"
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    type_str(&mut i, &ed, "p"); // only "apple" starts with "ap" now
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");
    let row = cursor_row(&i, &ed) + 1;
    let text = row_text(&i, &ed, row);
    assert!(text.contains("apple"), "row: {:?}", text);
    assert!(!text.contains("banana"), "row: {:?}", text);

    feed(&mut i, &ed, "RET");
    // Had `selected' not been reset to 0 by the narrow, this would
    // either panic (stale index 1 into a 1-element vec) or accept the
    // wrong candidate.
    assert_eq!(bs(&mut i), "APPLE");
}

#[test]
fn typing_a_char_matching_no_candidate_closes_the_popup() {
    let (mut i, ed) = setup();
    ok(
        &mut i,
        &show_popup_src(&[("apple", "APPLE", 1, "apple")], 1),
    );
    type_str(&mut i, &ed, "z"); // "apple" doesn't start with "z"
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(
        bs(&mut i),
        "z",
        "the character must still land via ordinary self-insert"
    );
}

// ------------------------------------------------------------
// 22. Backspace re-filters with a shorter typed span (without
//     spuriously dropping a survivor) and closes once point backs up
//     past the candidate's own start.
//
// Note (spec deviation, see the final report): a candidate dropped by a
// FORWARD keystroke's filter mismatch cannot be "brought back" by a
// LATER backspace under this popup's data shape -- `PopupItem`s are
// removed from `CompletionPopup::items` (the very Vec `render_lsp_
// completion_popup` draws from) as soon as they stop matching, and
// there is no separate "original full candidate list" to re-derive from
// without either a new field (M44-3 spec A.1's `PopupItem`/
// `CompletionPopup` field list is exhaustive) or making redisplay
// re-filter every frame (spec A.3 calls for "zero additional render
// change"). What IS true, and what this test pins instead: a backspace
// event re-filters with the shorter typed span and does not spuriously
// exclude a candidate that still matches it.
// ------------------------------------------------------------

#[test]
fn backspace_refilters_with_a_shorter_span_and_closes_past_the_start() {
    let (mut i, ed) = setup();
    ok(&mut i, "(insert \"xx\")"); // leading text before the candidate's start
    ok(&mut i, "(goto-char (point-max))");
    // PREFIX-START (3) coincides with the item's own START here -- both
    // sit right after "xx", same as a real `textEdit'-less candidate
    // would have it.
    ok(&mut i, &show_popup_src(&[("foo", "FOO", 3, "foo")], 3));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    type_str(&mut i, &ed, "fo"); // typed "fo" -- still a prefix of "foo"
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // Backspacing within the typed span re-filters with a SHORTER typed
    // string ("f") -- the survivor must not be spuriously dropped just
    // because the buffer changed (this is the exact same
    // `refilter_completion_popup` path self-insert uses, not a special
    // case for DEL).
    feed(&mut i, &ed, "DEL");
    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "t",
        "a surviving candidate must not be dropped by backspacing within its own typed span"
    );

    // Two more backspaces: buffer back to "xx" (point == prefix_start,
    // typed == "" -- still trivially a prefix match), then ONE more
    // crosses `point < prefix_start' and closes the popup outright --
    // nothing left it could possibly still be replacing.
    feed(&mut i, &ed, "DEL");
    feed(&mut i, &ed, "DEL");
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(bs(&mut i), "x");
}

// ------------------------------------------------------------
// 23. Pure movement (no edit) closes the popup -- vim/VS Code convention.
//
// Note: an ordinary motion key like C-b or <left> never actually reaches
// this codepath -- `completion_popup_key`'s three-way split (M44-3)
// only lets a self-insert character or DEL fall through unconsumed;
// every OTHER key (C-b/<left> included) still closes the popup
// immediately in `completion_popup_key` itself, unchanged from M40-4
// (see `other_key_c_x_still_closes_popup_and_falls_through`). The tail
// refilter's "tick unchanged but point moved" branch is reachable only
// when a self-insert-SHAPED key (`is_self_insert_char` tests the
// character's shape, not its actual keymap binding) happens to be bound
// to a pure-motion command instead of self-insert -- exercised here via
// a local rebinding, the only way to reach it given today's split.
// ------------------------------------------------------------

#[test]
fn pure_movement_closes_the_popup() {
    let (mut i, ed) = setup();
    ok(&mut i, "(insert \"ab\")");
    ok(&mut i, "(local-set-key \"j\" 'backward-char)");
    ok(&mut i, &show_popup_src(&[("x", "X", 1, "x")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // "j" is self-insert-SHAPED (`is_self_insert_char` doesn't consult
    // keymaps), so `completion_popup_key` lets it fall through
    // unconsumed same as any other printable character -- but it's
    // locally bound to a pure motion here, so `dispatch_key` runs
    // `backward-char`, not `self_insert`: no edit, `edit_ticks`
    // unchanged, only `point` moves.
    type_str(&mut i, &ed, "j");
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(pt(&mut i), 2, "the motion itself must still have happened");
}

// ------------------------------------------------------------
// 24. isIncomplete: the popup remembers it; typing further re-requests
//     completion instead of narrowing the existing (partial) list.
// ------------------------------------------------------------

#[test]
fn incomplete_popup_requeries_instead_of_narrowing_on_further_typing() {
    let (mut i, ed, _dir) = setup_connected("fo");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "2"
    );

    ok(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "{\"isIncomplete\":true,\"items\":[{\"label\":\"foo\"}]}"))"#,
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // Re-stub so the SECOND (re-)request can be captured independently
    // of the first.
    stub_capture(&mut i);
    type_str(&mut i, &ed, "o"); // buffer "foo" -- triggers the requery

    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "nil",
        "an incomplete popup must close rather than narrow itself further"
    );
    assert_eq!(
        run(&mut i, "(car test--captured)"),
        "fake-client",
        "lsp-request-async must have been called again"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "3",
        "the new request's position must reflect the buffer AFTER the extra typing"
    );
}

// ------------------------------------------------------------
// 25. electric-pair interaction: auto-closing a bracket still works
//     exactly as it does with no popup open, and the popup itself
//     closes via the ordinary refilter rule ("(" matches no identifier
//     filter text).
// ------------------------------------------------------------

#[test]
fn electric_pair_auto_close_still_works_and_closes_the_popup_via_refilter() {
    let (mut i, ed) = setup_prog();
    ok(&mut i, "(insert \"fo\")");
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, &show_popup_src(&[("foo", "foo", 1, "foo")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    type_str(&mut i, &ed, "(");

    // electric-pair's own behavior is untouched by the popup being open.
    assert_eq!(bs(&mut i), "fo()");
    assert_eq!(pt(&mut i), 4);
    // "(" is not part of "foo"'s filter text -- refilter drops every
    // candidate and closes the popup, the same "all excluded -> close"
    // rule any other non-matching character would hit.
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
}

// ------------------------------------------------------------
// 26. Snippet syntax in `textEdit.newText' is inserted literally (v1
//     does not parse/expand it) -- also exercises newText's priority
//     over insertText.
// ------------------------------------------------------------

#[test]
fn snippet_syntax_in_new_text_is_inserted_literally() {
    let (mut i, ed, _dir) = setup_connected("");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    ok(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "[{\"label\":\"fn\",\"insertText\":\"ignored\",\"textEdit\":{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":0}},\"newText\":\"fn ${1:name}() {}\"}}]"))"#,
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");
    feed(&mut i, &ed, "RET");
    assert_eq!(
        bs(&mut i),
        "fn ${1:name}() {}",
        "snippet syntax must be inserted as literal text (v1), and newText must win over insertText"
    );
}

// ------------------------------------------------------------
// 27. dabbrev C-n/C-p regression: unaffected once a completion popup
//     that was opened has since closed.
// ------------------------------------------------------------

#[test]
fn dabbrev_c_n_c_p_unaffected_once_the_completion_popup_has_closed() {
    let (mut i, ed) = setup();
    ok(&mut i, "(evil-mode 1)");
    ok(&mut i, "(insert \"printhis\\n\\nprintln\\n\")");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 1)"); // the blank line
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");

    // Open and close a completion popup first -- M44-3's own DEL/
    // self-insert/refilter plumbing must not leave anything behind that
    // shadows dabbrev's C-n/C-p once the popup itself is gone.
    ok(&mut i, &show_popup_src(&[("x", "X", 1, "x")], 1));
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");
    feed(&mut i, &ed, "ESC"); // closes the popup without leaving insert state
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "nil");
    assert_eq!(run(&mut i, "evil--state"), "insert");

    feed(&mut i, &ed, "C-p"); // dabbrev-expand's traditional direction
    assert_eq!(bs(&mut i), "printhis\nprinthis\nprintln\n");
}

// ============================================================
// M44 review fixes below: #1 (filter anchor decoupled from deletion
// range) and #2 (a malformed item must not sink the whole reply).
// ============================================================

// ------------------------------------------------------------
// 28. M44 review fix #1: a postfix-shaped candidate -- `textEdit`
//     replaces the receiver span ("obj."), but `filterText` covers only
//     the inserted keyword ("if"), the exact shape rust-analyzer's
//     `.if`/`.match`/`.unwrap` postfix templates use -- must survive
//     BOTH the initial filter and further typing while the popup is
//     open. M44-3 originally anchored both checks at the candidate's
//     OWN `textEdit` start (well before "obj."), so "obj." (and later
//     "obj.i") was compared against filterText "if" and never prefix-
//     matched -- the candidate silently vanished, "No completions".
//     Mutation: reverting either filter step back to the per-item
//     `start` (`item.start` in `refilter_completion_popup`, or `start`
//     instead of `prefix-start` in `lsp--completion-items`) makes this
//     FAIL -- the popup would never open (or would close on "i").
// ------------------------------------------------------------

#[test]
fn postfix_shaped_candidate_survives_filtering_anchored_at_the_popup_prefix() {
    let (mut i, ed, _dir) = setup_connected("obj.");
    ok(&mut i, "(goto-char (point-max))"); // right after "obj."
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");
    // Sanity: "." breaks the identifier run -- empty prefix, the
    // interesting case for a postfix `textEdit` (same setup as test 19).
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "4"
    );

    ok(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "[{\"label\":\"if\",\"filterText\":\"if\",\"textEdit\":{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":4}},\"newText\":\"if obj\"}}]"))"#,
    );
    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "t",
        "the postfix candidate must survive the initial filter even though \
         its filterText (\"if\") isn't a prefix of the receiver text (\"obj.\")"
    );
    let row = cursor_row(&i, &ed) + 1;
    assert!(
        row_text(&i, &ed, row).contains("if"),
        "row: {:?}",
        row_text(&i, &ed, row)
    );

    // Typing "i" narrows against the SAME popup-wide prefix span
    // (empty, captured when the popup opened), not the candidate's own
    // (much earlier) textEdit start -- "if".starts_with("i").
    type_str(&mut i, &ed, "i");
    assert_eq!(
        run(&mut i, "(completion-popup-active-p)"),
        "t",
        "typing \"i\" must still match \"if\" via the popup-wide prefix, \
         not the candidate's own textEdit start"
    );

    feed(&mut i, &ed, "RET");
    // The candidate's OWN textEdit start (position 1, before "obj."
    // entirely) is still what accept deletes -- "obj." plus the "i"
    // just typed, all replaced by newText. Per-item `start` keeps its
    // role for the deletion range; only the FILTER anchor moved.
    assert_eq!(bs(&mut i), "if obj");
}

// ------------------------------------------------------------
// 29. M44 review fix #2: a malformed element in the server's reply
//     (JSON `null`, e.g. a server bug or a lossy proxy) must not sink
//     the WHOLE reply -- only that one element is skipped, every
//     well-formed candidate around it still reaches the popup.
//     `lsp--completion-items` used to push every raw element (including
//     non-hash-tables) and sort BEFORE filtering by `hash-table-p`;
//     `sort`'s key function calls `gethash` unconditionally, so it
//     signaled on the first non-hash-table element and lost "foo" and
//     "bar" right along with it. Mutation: moving the `hash-table-p`
//     filter back to after `sort` makes this FAIL with an escaped
//     "Wrong type argument" error instead of a two-candidate popup.
// ------------------------------------------------------------

#[test]
fn malformed_item_in_the_reply_is_skipped_not_fatal_to_the_whole_list() {
    let (mut i, ed, _dir) = setup_connected("");
    ok(&mut i, "(goto-char (point-max))");
    stub_capture(&mut i);
    feed(&mut i, &ed, "C-M-i");

    let out = run(
        &mut i,
        r#"(funcall (nth 3 test--captured)
               (json-parse-string "[{\"label\":\"foo\"},null,{\"label\":\"bar\"}]"))"#,
    );
    assert!(
        !out.starts_with("ERROR"),
        "a malformed element must not signal and lose the whole reply: {}",
        out
    );
    assert_eq!(run(&mut i, "(completion-popup-active-p)"), "t");

    // sortText absent from both survivors -> sorted by label: "bar" < "foo".
    let row0 = cursor_row(&i, &ed) + 1;
    assert!(
        row_text(&i, &ed, row0).contains("bar"),
        "row0: {:?}",
        row_text(&i, &ed, row0)
    );
    assert!(
        row_text(&i, &ed, row0 + 1).contains("foo"),
        "row1: {:?}",
        row_text(&i, &ed, row0 + 1)
    );
}
