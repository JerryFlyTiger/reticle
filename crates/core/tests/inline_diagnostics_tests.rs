//! M87 stage 3: inline diagnostic block rows. `Grid` gains a per-row
//! scale (`row_scale`) and kind (`row_kind`); core turns each LSP
//! diagnostic into one or more extra "block" rows drawn directly under
//! the buffer line it belongs to, consuming the window's own `text_rows`
//! budget the same way a wrap-continuation row does (D4). See
//! `redisplay.rs`'s `render_window`/`emit_block_rows` for the
//! implementation and `PLAN.md`'s M87 stage 3 entry for the design
//! decisions (D1-D12) these tests are named after.
//!
//! No shared fixture module (project convention: each test file brings
//! its own helpers).

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use core::redisplay::{render, Grid, RowKind};
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(cols: usize, rows: usize) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (cols, rows);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// `(LINE SEV . "MSG")` per entry -- the same shape `lsp--decorate-
/// buffer` builds (`(cons line (cons sev msg))`, which the list reader's
/// dotted-pair notation collapses to this form).
fn set_diags(interp: &mut Interp, items: &[(usize, u8, &str)]) {
    let body: String = items
        .iter()
        .map(|(line, sev, msg)| format!("({} {} . {:?})", line, sev, msg))
        .collect::<Vec<_>>()
        .join(" ");
    let r = run(
        interp,
        &format!("(lsp--set-buffer-diagnostics (current-buffer) '({}))", body),
    );
    assert!(!r.starts_with("ERROR"), "set_diags failed: {}", r);
}

fn row_text(grid: &Grid, row: usize) -> String {
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn find_row(grid: &Grid, needle: &str) -> Option<usize> {
    (0..grid.lines.len()).find(|&r| row_text(grid, r).contains(needle))
}

/// Frame sized so `text_rows == rows - 2` (single, unsplit window: the
/// echo row eats one, the window's own mode line eats another -- see
/// `frame_layout`'s doc). Every test below picks its own `(cols, rows)`
/// and derives `text_rows` from this, rather than hard-coding it, so a
/// change to that arithmetic elsewhere would fail loudly here instead of
/// silently miscounting.
fn text_rows(rows: usize) -> usize {
    rows - 2
}

// 1. + 2. -----------------------------------------------------------

#[test]
fn diagnostic_adds_one_row_directly_below_its_line_and_shifts_later_lines_down() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\\nline three\")");
    set_diags(&mut i, &[(0, 1, "boom")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").expect("line one visible");
    assert!(
        row_text(&grid, r0 + 1).contains("boom"),
        "row after line one should carry the diagnostic: {:?}",
        row_text(&grid, r0 + 1)
    );
    assert!(
        row_text(&grid, r0 + 2).contains("line two"),
        "line two must shift down by exactly one row: {:?}",
        row_text(&grid, r0 + 2)
    );
    assert!(row_text(&grid, r0 + 3).contains("line three"));
    // F1 (M87 stage 3 fix round): the block row's message text must be
    // covered by exactly ONE run, not two byte-identical runs stacked on
    // the same columns (the manual `grid.runs.push` in `emit_block_rows`
    // used to duplicate whatever `fill_chrome_runs`'s post-pass already
    // synthesized). The block row legitimately has a SECOND, different
    // run too -- the trailing blank padding past the message, a
    // different style -- so this counts runs whose text contains "boom"
    // specifically, not every run on the row.
    let block_row = r0 + 1;
    let boom_runs: Vec<_> = grid
        .runs
        .iter()
        .filter(|r| r.row == block_row && r.text.contains("boom"))
        .collect();
    assert_eq!(
        boom_runs.len(),
        1,
        "expected exactly one run carrying the message text, got {:#?}",
        boom_runs
    );
}

#[test]
fn window_shows_exactly_one_fewer_line_of_text_and_grid_rows_is_unchanged() {
    let (mut i, ed) = setup(40, 10);
    let tr = text_rows(10);
    // "ZZ" prefix (not "L", the modeline's own `L{line}:{col}` segment
    // uses that letter and would collide with a naive substring search).
    let lines: Vec<String> = (1..=tr).map(|n| format!("ZZ{n}")).collect();
    run(&mut i, &format!("(insert {:?})", lines.join("\n")));
    let plain = render(&i, &ed);
    assert_eq!(plain.rows, 10);
    assert!(
        find_row(&plain, &format!("ZZ{tr}")).is_some(),
        "without a diagnostic every one of the {tr} lines should be visible"
    );

    set_diags(&mut i, &[(0, 1, "boom")]);
    let diag = render(&i, &ed);
    assert_eq!(diag.rows, 10, "grid.rows must not change");
    assert!(
        find_row(&diag, &format!("ZZ{tr}")).is_none(),
        "the block row's budget must come out of text_rows -- the last line should now be scrolled off"
    );
    assert!(find_row(&diag, &format!("ZZ{}", tr - 1)).is_some());
}

// 3. ------------------------------------------------------------------

#[test]
fn block_row_has_scale_75_and_kind_block_every_other_row_stays_default() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    set_diags(&mut i, &[(0, 1, "boom")]);
    let grid = render(&i, &ed);
    assert_eq!(grid.row_scale.len(), grid.rows);
    assert_eq!(grid.row_kind.len(), grid.rows);
    let r0 = find_row(&grid, "line one").unwrap();
    let block_row = r0 + 1;
    for r in 0..grid.rows {
        if r == block_row {
            assert_eq!(grid.row_scale[r], 75, "block row scale");
            assert_eq!(grid.row_kind[r], RowKind::Block, "block row kind");
        } else {
            assert_eq!(grid.row_scale[r], 100, "row {r} scale");
            assert_eq!(grid.row_kind[r], RowKind::Text, "row {r} kind");
        }
    }
}

// 4. ------------------------------------------------------------------

#[test]
fn buffer_pos_at_is_none_for_every_column_of_a_block_row() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    set_diags(&mut i, &[(0, 1, "boom")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    let block_row = r0 + 1;
    assert_eq!(grid.row_kind[block_row], RowKind::Block);
    for c in 0..grid.cols {
        assert_eq!(
            grid.buffer_pos_at(block_row, c),
            None,
            "column {c} of block row {block_row} must map to no buffer position"
        );
    }
}

// 5. ------------------------------------------------------------------

#[test]
fn two_diagnostics_on_the_same_line_produce_two_rows_in_stored_order() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    set_diags(&mut i, &[(0, 1, "alpha issue"), (0, 2, "beta issue")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    assert!(row_text(&grid, r0 + 1).contains("alpha issue"));
    assert!(row_text(&grid, r0 + 2).contains("beta issue"));
    assert!(row_text(&grid, r0 + 3).contains("line two"));
}

// 6. ------------------------------------------------------------------

#[test]
fn multiline_message_splits_and_caps_at_three_rows_with_ellipsis() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    set_diags(&mut i, &[(0, 1, "m1\nm2\nm3\nm4")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    assert!(row_text(&grid, r0 + 1).contains("m1"));
    assert!(row_text(&grid, r0 + 2).contains("m2"));
    let third = row_text(&grid, r0 + 3);
    assert!(third.contains("m3"), "third row: {:?}", third);
    assert!(
        third.contains('\u{2026}'),
        "capped-off third row must carry the ellipsis: {:?}",
        third
    );
    assert!(
        !row_text(&grid, r0 + 4).contains("m4"),
        "a 4th message line must never reach the grid: {:?}",
        row_text(&grid, r0 + 4)
    );
    // And the following buffer line lands right after the capped-at-3
    // block rows, not after a would-be 4th.
    assert!(row_text(&grid, r0 + 4).contains("line two"));
}

// 7. ------------------------------------------------------------------

#[test]
fn message_wider_than_the_window_is_truncated_with_ellipsis_not_wrapped() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    let long = "x".repeat(80);
    set_diags(&mut i, &[(0, 1, &long)]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    let block = row_text(&grid, r0 + 1);
    assert!(block.contains('\u{2026}'), "must be truncated: {:?}", block);
    assert!(
        block.chars().count() <= grid.cols,
        "truncated row must fit in one row's width: {:?}",
        block
    );
    // Never wraps: the diagnostic must still be exactly one row, so
    // "line two" is the very next row.
    assert!(row_text(&grid, r0 + 2).contains("line two"));
}

// 8. ------------------------------------------------------------------

#[test]
fn diagnostic_on_a_line_scrolled_out_of_view_produces_no_row() {
    let (mut i, ed) = setup(40, 10);
    let tr = text_rows(10);
    // Many more lines than fit; diagnostic on line 0 (0-based).
    let n = tr * 4;
    let lines: Vec<String> = (1..=n).map(|k| format!("L{k}")).collect();
    run(&mut i, &format!("(insert {:?})", lines.join("\n")));
    set_diags(&mut i, &[(0, 1, "boom")]);
    // Move point to the very end and let normal point-visibility
    // scrolling push line 0 off screen.
    run(&mut i, "(goto-char (point-max))");
    let grid = render(&i, &ed);
    assert!(
        find_row(&grid, "L1").is_none(),
        "line 0 (\"L1\") itself must have scrolled out of view for this test to mean anything"
    );
    assert!(
        find_row(&grid, "boom").is_none(),
        "line 0 is scrolled out of view -- its diagnostic must not appear anywhere"
    );
}

// 9. ------------------------------------------------------------------

#[test]
fn diagnostic_on_a_wrapped_line_lands_after_the_last_continuation_row() {
    let (mut i, ed) = setup(20, 12);
    let long_line = "A".repeat(35); // wraps into 2 rows at cols=20
    run(&mut i, &format!("(insert \"{long_line}\\nNEXT LINE\")"));
    set_diags(&mut i, &[(0, 1, "WRAPMSG")]);
    let grid = render(&i, &ed);
    let wrap_row = find_row(&grid, "AAAA").expect("first visual row of the wrapped line");
    // The wrapped line's own second visual row is directly below the
    // first (still all 'A's, no other content in between).
    assert!(row_text(&grid, wrap_row + 1).contains('A'));
    let msg_row = find_row(&grid, "WRAPMSG").expect("diagnostic row");
    assert_eq!(
        msg_row,
        wrap_row + 2,
        "the block row must sit right after BOTH visual rows of the wrapped line"
    );
    assert!(row_text(&grid, msg_row + 1).contains("NEXT LINE"));
}

// 10. -----------------------------------------------------------------

#[test]
fn no_budget_left_drops_the_row_and_never_exceeds_text_rows() {
    let (mut i, ed) = setup(40, 10);
    let tr = text_rows(10);
    // Exactly `tr` lines, filling the window's text budget completely;
    // the diagnostic sits on the very last visible line.
    let lines: Vec<String> = (1..=tr).map(|n| format!("L{n}")).collect();
    run(&mut i, &format!("(insert {:?})", lines.join("\n")));
    set_diags(&mut i, &[(tr - 1, 1, "no room")]);
    let grid = render(&i, &ed);
    assert_eq!(grid.rows, 10);
    assert!(
        find_row(&grid, "no room").is_none(),
        "no budget left for a block row on the last visible line"
    );
    // Structural sanity: every row/kind vector still has the right
    // length and no row was marked Block (nothing was actually emitted).
    assert_eq!(grid.row_scale.len(), grid.rows);
    assert!(grid.row_kind.iter().all(|k| *k == RowKind::Text));
}

// 11. -----------------------------------------------------------------

#[test]
fn inline_diagnostics_nil_reproduces_the_grid_exactly() {
    // "Today" (pre-stage-3) baseline: a diagnostic set the OLD way
    // (`(LINE . SEVERITY)`, no message) still lights up the gutter dot
    // and the modeline count -- D7 requires those to keep working
    // unchanged -- but produces zero block rows (there is no message to
    // show), with the default `inline-diagnostics` (`t`). That's the
    // grid D11 says setting `inline-diagnostics` to `nil` must reproduce
    // byte-for-byte, NOT a grid with no diagnostic at all (which
    // wouldn't even have the gutter dot/modeline count, so comparing
    // against it would prove nothing about block rows specifically).
    let (mut i1, ed1) = setup(40, 10);
    run(&mut i1, "(insert \"line one\\nline two\\nline three\")");
    run(&mut i1, "(setq display-line-numbers t)");
    run(
        &mut i1,
        "(lsp--set-buffer-diagnostics (current-buffer) '((0 . 1)))",
    );
    let baseline = render(&i1, &ed1);

    let (mut i2, ed2) = setup(40, 10);
    run(&mut i2, "(insert \"line one\\nline two\\nline three\")");
    run(&mut i2, "(setq display-line-numbers t)");
    run(&mut i2, "(setq inline-diagnostics nil)");
    set_diags(&mut i2, &[(0, 1, "boom")]);
    let off = render(&i2, &ed2);

    assert_eq!(off.rows, baseline.rows);
    assert_eq!(off.cols, baseline.cols);
    assert_eq!(off.row_scale, baseline.row_scale);
    assert_eq!(off.row_kind, baseline.row_kind);
    for r in 0..baseline.rows {
        assert_eq!(row_text(&off, r), row_text(&baseline, r), "row {r} text");
    }
    assert_eq!(
        off.runs.len(),
        baseline.runs.len(),
        "run count must be identical with inline-diagnostics off"
    );
    // Sanity: this really is exercising the gutter dot / modeline count
    // path, not two grids that both happened to have zero diagnostics.
    assert!(row_text(&baseline, 0).contains('\u{25cf}'), "gutter dot");
    assert!(row_text(&baseline, 8).contains("!1"), "modeline count");
}

// 12. -----------------------------------------------------------------

#[test]
fn killing_a_buffer_drops_its_diagnostics_entry() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\")");
    set_diags(&mut i, &[(0, 1, "boom")]);
    assert_eq!(ed.borrow().diagnostics.len(), 1, "one buffer, one entry");

    let r = run(&mut i, "(kill-buffer (current-buffer))");
    assert!(!r.starts_with("ERROR"), "kill-buffer failed: {}", r);
    assert_eq!(
        ed.borrow().diagnostics.len(),
        0,
        "the killed buffer's diagnostics entry must be removed, not left stale"
    );

    // A freshly created buffer must not inherit anything.
    run(&mut i, "(switch-to-buffer-internal \"fresh\")");
    run(&mut i, "(insert \"line one\")");
    let grid = render(&i, &ed);
    assert!(
        !row_text(&grid, 1).contains('\u{258f}'),
        "new buffer must show no inline diagnostic: {:?}",
        row_text(&grid, 1)
    );
}

// F2 (M87 stage 3 fix round) --------------------------------------------

#[test]
fn block_row_style_is_italic_and_uses_the_severity_color() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    run(&mut i, "(setq display-line-numbers t)");
    set_diags(&mut i, &[(0, 1, "boom")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();

    // The gutter dot on line one's own row is colored via the exact
    // same `severity_color(sev)` call the block row's text uses --
    // cross-checking against it (rather than hard-coding a theme RGB
    // triple here) keeps this test theme-agnostic and still catches
    // "italic flipped to false" / "wrong severity color" mutations.
    let dot_col = (0..grid.cols)
        .find(|&c| grid.lines[r0][c].ch == '\u{25cf}')
        .expect("gutter dot on line one");
    let expected_fg = grid.lines[r0][dot_col].style.fg;
    assert!(expected_fg.is_some(), "gutter dot must have a color");

    let block_row = r0 + 1;
    let msg_col = (0..grid.cols)
        .find(|&c| {
            let cell = grid.lines[block_row][c];
            cell.ch != ' ' && cell.ch != '\u{258f}'
        })
        .expect("a message-text cell on the block row");
    let cell = grid.lines[block_row][msg_col];
    assert!(cell.style.italic, "block row text must be italic");
    assert_eq!(
        cell.style.fg, expected_fg,
        "block row text must use the same severity color as the gutter dot"
    );
}

// F6 (M87 stage 3 fix round), end to end ----------------------------------

#[test]
fn blank_interior_message_line_is_skipped_not_shown_as_an_empty_row() {
    let (mut i, ed) = setup(40, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    set_diags(&mut i, &[(0, 1, "real text\n   \nmore text")]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    // Two rows only -- "real text" then "more text" -- with the blank
    // interior line skipped, not a 3-row block with an empty middle row.
    assert!(row_text(&grid, r0 + 1).contains("real text"));
    assert!(row_text(&grid, r0 + 2).contains("more text"));
    assert!(row_text(&grid, r0 + 3).contains("line two"));
}

// F7 (M87 stage 3 fix round): a split window ------------------------------

#[test]
fn split_window_diagnostic_lands_only_inside_its_own_windows_rect() {
    let (mut i, ed) = setup(80, 20);
    run(&mut i, "(insert \"buffer A line one\")");
    // A horizontal split (stacked, distinct row ranges) -- unlike
    // `split-window-right`'s side-by-side windows, which would share
    // every row and make "landed inside window B's rect" untestable in
    // the row dimension alone.
    run(&mut i, "(split-window-below)");
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer-internal \"bufB\")");
    run(&mut i, "(insert \"buffer B line one\")");
    set_diags(&mut i, &[(0, 1, "boom")]);
    let grid = render(&i, &ed);

    assert_eq!(grid.windows.len(), 2, "must be a real split");
    let win_a = grid
        .windows
        .iter()
        .find(|w| (w.row..w.mode_line_row).any(|r| row_text(&grid, r).contains("buffer A")))
        .expect("window showing buffer A");
    let win_b = grid
        .windows
        .iter()
        .find(|w| (w.row..w.mode_line_row).any(|r| row_text(&grid, r).contains("buffer B")))
        .expect("window showing buffer B");
    assert_ne!(win_a.win_id, win_b.win_id);

    let block_row = find_row(&grid, "boom").expect("diagnostic row somewhere on screen");
    assert!(
        block_row >= win_b.row && block_row < win_b.mode_line_row,
        "block row {} must be inside window B's rect [{}, {})",
        block_row,
        win_b.row,
        win_b.mode_line_row
    );
    assert!(
        !(block_row >= win_a.row && block_row < win_a.mode_line_row),
        "block row must NOT be inside window A's rect"
    );
    // Window A itself is completely unaffected by B's diagnostic.
    assert!(row_text(&grid, win_a.row).contains("buffer A line one"));
}

// --- Trailing cold-read gap: a wide (CJK) character inside a message ---
//
// `diag_truncate_tail` budgets by `wide_char_width`, the paint loop
// chooses `put_wide` for a width-2 char (lead cell + a `continuation`
// cell with no character of its own), and `fill_chrome_runs` merges the
// lead and continuation cells into one run with `cols` counted in
// display columns, not chars -- none of the 15 tests above ever put a
// wide char in a message, so none of that was actually under test.

#[test]
fn wide_characters_in_a_message_occupy_their_real_display_width() {
    use core::redisplay::display_width::string_width_elisp;

    let (mut i, ed) = setup(60, 10);
    run(&mut i, "(insert \"line one\\nline two\")");
    // 4 CJK characters, each display-width 2 (8 columns total) but only
    // 4 chars -- the gap between "chars" and "columns" this test exists
    // to cover.
    let msg = "\u{4e2d}\u{6587}\u{8b66}\u{544a}"; // "中文警告"
    set_diags(&mut i, &[(0, 1, msg)]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "line one").unwrap();
    let block_row = r0 + 1;

    // The row's text (as chars, via `row_text`'s char collection) must
    // contain the message verbatim -- this window is wide enough that
    // no truncation happens.
    assert!(
        row_text(&grid, block_row).contains(msg),
        "block row must show the full message: {:?}",
        row_text(&grid, block_row)
    );

    // Cells occupied: prefix display width (4) + message display width
    // (8) == 12, NOT prefix chars (4) + message char count (4) == 8.
    // Each CJK char occupies its own lead cell plus one `continuation`
    // cell with no character of its own (`Grid::put_wide`).
    let prefix_w = string_width_elisp("  \u{258f} ");
    let msg_w = string_width_elisp(msg);
    assert_eq!(msg_w, 8, "sanity: 4 CJK chars at display-width 2 each");
    let total_w = prefix_w + msg_w;

    // The last occupied column is a continuation cell (the second half
    // of the last CJK char); the column right after it is unoccupied
    // padding, not part of the message at all.
    assert!(
        grid.lines[block_row][total_w - 1].continuation,
        "column {} (last message column) must be a continuation cell",
        total_w - 1
    );
    assert_eq!(grid.lines[block_row][total_w].ch, ' ');
    assert!(!grid.lines[block_row][total_w].continuation);

    // Exactly 4 continuation cells inside the message span -- one per
    // CJK character, no more, no fewer (a desync here would mean a
    // lead/continuation cell pair drifted apart).
    let cont_count = (prefix_w..total_w)
        .filter(|&c| grid.lines[block_row][c].continuation)
        .count();
    assert_eq!(cont_count, 4);

    // F1-style check, for the wide-char case specifically: exactly one
    // run covers the block row's text (prefix + message merged, same
    // style throughout), with `cols` counted in display columns -- not
    // two duplicate runs, and not a run whose `cols` undercounts because
    // a continuation cell got merged wrong.
    let full_text = format!("  \u{258f} {}", msg);
    let msg_runs: Vec<_> = grid
        .runs
        .iter()
        .filter(|r| r.row == block_row && r.text == full_text)
        .collect();
    assert_eq!(
        msg_runs.len(),
        1,
        "expected exactly one run carrying the full prefix+message text, got {:#?}",
        msg_runs
    );
    assert_eq!(msg_runs[0].col, 0);
    assert_eq!(
        msg_runs[0].cols, total_w,
        "run's `cols` must be the display width, not the char count"
    );
}

#[test]
fn a_wide_character_straddling_the_truncation_point_is_dropped_whole() {
    // Frame text width 8 (gutter off): prefix "  ▏ " is display-width 4,
    // leaving a truncation budget of 4. "AB" (2 narrow chars, width 2)
    // fits; the next character, "中" (width 2), would need columns 2..4
    // of that 4-column budget but the reserved ellipsis column means
    // only 3 are actually free for body text (`diag_truncate_tail`
    // reserves 1 of the budget's columns for the ellipsis) -- so "中"
    // straddles the cut exactly at the char level, not just the column
    // level.
    let (mut i, ed) = setup(8, 10);
    run(&mut i, "(insert \"x\\ny\")");
    let msg = "AB\u{4e2d}\u{6587}"; // "AB中文"
    set_diags(&mut i, &[(0, 1, msg)]);
    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "x").unwrap();
    let block_row = r0 + 1;
    let text = row_text(&grid, block_row);

    assert!(text.contains("AB"), "the two chars that DO fit: {:?}", text);
    assert!(
        text.contains('\u{2026}'),
        "must carry the ellipsis: {:?}",
        text
    );
    // The wide character that straddles the cut must be dropped WHOLE --
    // neither of its two halves (the char itself, or a stray orphaned
    // continuation cell with no lead) may appear.
    assert!(
        !text.contains('\u{4e2d}') && !text.contains('\u{6587}'),
        "a straddling wide char must never appear half-drawn: {:?}",
        text
    );
    let orphan_continuation = grid.lines[block_row]
        .iter()
        .enumerate()
        .any(|(c, cell)| cell.continuation && grid.lines[block_row][c - 1].ch == ' ');
    assert!(
        !orphan_continuation,
        "no continuation cell may exist without a real lead character before it: {:?}",
        grid.lines[block_row]
            .iter()
            .map(|c| (c.ch, c.continuation))
            .collect::<Vec<_>>()
    );

    // F1-style check again: still exactly one run for the block row's
    // (now-truncated) text, even with a wide char dropped mid-message.
    let msg_runs: Vec<_> = grid
        .runs
        .iter()
        .filter(|r| r.row == block_row && r.text.contains("AB"))
        .collect();
    assert_eq!(
        msg_runs.len(),
        1,
        "expected exactly one run carrying the truncated text, got {:#?}",
        msg_runs
    );
}
