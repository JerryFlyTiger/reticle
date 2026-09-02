//! Golden-master safety net for the upcoming `redisplay.rs` refactor
//! (collapsing five "how wide is this text" implementations and two
//! line-wrapping implementations into one each).
//!
//! **Ordering matters more than anything else in this file.** This
//! fixture was captured from `redisplay.rs` *before* that refactor
//! exists. If it were ever regenerated from post-refactor code, it
//! would stop asserting anything -- it would just record whatever the
//! new code does, which is the exact failure mode `CLAUDE.md` already
//! records ("the three gates cannot see a spec that matches the wrong
//! code"). So: the checked-in fixture
//! (`fixtures/layout_golden.txt`) is authoritative. Regenerating it is
//! an explicit, deliberate act -- run with `RETICLE_BLESS_GOLDEN=1` set
//! (any non-empty value), e.g.:
//!
//!     RETICLE_BLESS_GOLDEN=1 cargo test -p core --test layout_golden_tests
//!
//! Do that ONLY when you have independently confirmed the new rendering
//! is correct (or, after the refactor lands, that it is
//! byte-identical). A plain `cargo test` run NEVER writes the fixture,
//! even on mismatch -- it only compares and fails.
//!
//! ## What's excluded and why
//!
//! Nothing is excluded as "inherently non-deterministic" -- every field
//! `render()` produces here (cell text, cell style, cursor, window
//! layout) is a pure function of interpreter/editor state with no
//! wall-clock, filesystem-ordering, or hash-map-iteration input.
//!
//! One caveat worth being honest about: several scenarios `find-file`
//! real material under `demo/rtl/`, and the mode line's title segment
//! shows that file's *absolute* path. That path is stable across runs
//! on the same checkout (deterministic for this test's purpose: the
//! same machine, before and after the refactor) but is NOT portable to
//! a different clone location -- cloning this repo somewhere else would
//! shift every mode-line row that shows a path and require reblessing.
//! That's an accepted limitation, not a bug in the test: this fixture's
//! job is to guard one specific refactor on one working tree, not to be
//! a portable cross-machine artifact.
//!
//! ## Row-text encoding
//!
//! Each row is serialized as exactly `grid.cols` characters (one per
//! `Cell`), Debug-quoted (`{:?}`) so trailing spaces and any stray
//! control bytes are visible in a diff rather than silently eaten by a
//! text editor. A `Cell` with `continuation == true` always carries
//! `ch == ' '` (see `Grid::put_wide`), which would make it visually
//! indistinguishable from a real blank cell if we just printed `ch`.
//! To keep that information instead of silently dropping it, every
//! continuation cell is printed as `CONT_MARK` (U+2504, a box-drawing
//! dash unlikely to appear in RTL source or English prose) in place of
//! its `ch`.
//!
//! ## Style encoding
//!
//! Printing every cell's full `Style` would make this file a wall of
//! repeated `Style::default()`s (most cells use the default face).
//! Instead, for each cell whose style differs from `Style::default()`
//! in any field, one `style r=.. c=..: <fields>` line lists only the
//! fields that actually differ.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use core::redisplay::{render, Grid, Style};
use elisp::Interp;

fn setup(cols: usize, rows: usize) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (cols, rows);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) {
    if let Err(flow) = interp.eval_source(src) {
        panic!("eval of {:?} failed: {}", src, interp.describe_flow(&flow));
    }
}

fn demo_path(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo")).join(rel)
}

fn find_file(interp: &mut Interp, rel: &str) {
    let path = demo_path(rel);
    run(
        interp,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
}

/// Stand-in for a continuation cell's `ch` in the serialized row text --
/// see the file-level doc comment's "Row-text encoding" section.
const CONT_MARK: char = '\u{2504}';

/// Only the fields of `s` that differ from `Style::default()`, in a
/// fixed order, compact form. `None` when `s` IS the default style.
fn style_diff(s: &Style) -> Option<String> {
    let base = Style::default();
    let mut parts = Vec::new();
    if s.fg != base.fg {
        parts.push(format!("fg={:?}", s.fg));
    }
    if s.bg != base.bg {
        parts.push(format!("bg={:?}", s.bg));
    }
    if s.bold != base.bold {
        parts.push(format!("bold={}", s.bold));
    }
    if s.italic != base.italic {
        parts.push(format!("italic={}", s.italic));
    }
    if s.underline != base.underline {
        parts.push(format!("underline={:?}", s.underline));
    }
    if s.underline_color != base.underline_color {
        parts.push(format!("underline_color={:?}", s.underline_color));
    }
    if s.reverse != base.reverse {
        parts.push(format!("reverse={}", s.reverse));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Serialize a `Grid` as described in the file-level doc comment.
fn serialize(grid: &Grid) -> String {
    let mut out = String::new();
    out.push_str(&format!("cols={} rows={}\n", grid.cols, grid.rows));
    out.push_str(&format!("cursor=({}, {})\n", grid.cursor.0, grid.cursor.1));
    out.push_str(&format!("windows={}\n", grid.windows.len()));
    for w in &grid.windows {
        out.push_str(&format!(
            "  win_id={} row={} col={} rows={} cols={} gutter_cols={} mode_line_row={}\n",
            w.win_id, w.row, w.col, w.rows, w.cols, w.gutter_cols, w.mode_line_row
        ));
    }
    for (r, row) in grid.lines.iter().enumerate() {
        let text: String = row
            .iter()
            .map(|c| if c.continuation { CONT_MARK } else { c.ch })
            .collect();
        out.push_str(&format!("row {:03}: {:?}\n", r, text));
        for (c, cell) in row.iter().enumerate() {
            if let Some(d) = style_diff(&cell.style) {
                out.push_str(&format!("  style r={} c={}: {}\n", r, c, d));
            }
        }
    }
    // Task 1/2 (mouse support): `grid.runs`, appended as its own section
    // after everything else this scenario already recorded above -- see
    // `dev/gui-shot.sh`'s sibling task spec for why this has to be a
    // pure append (existing rows/style lines must not move or change).
    // Style is printed the same compact way `style_diff` already prints
    // per-cell styles (`None` -> "default"), so a run's line doesn't
    // repeat the same wall of `Style::default()` fields the "Style
    // encoding" file-level doc comment already explains avoiding.
    out.push_str(&format!("runs={}\n", grid.runs.len()));
    for r in &grid.runs {
        let style_desc = style_diff(&r.style).unwrap_or_else(|| "default".to_string());
        let src_desc = match &r.src {
            Some(range) => format!("{}..{}", range.start, range.end),
            None => "none".to_string(),
        };
        out.push_str(&format!(
            "  run row={} col={} cols={} src={} style=[{}] text={:?}\n",
            r.row, r.col, r.cols, src_desc, style_desc, r.text
        ));
    }
    out
}

fn capture(interp: &Interp, ed: &Rc<RefCell<Editor>>, name: &str, out: &mut String) {
    let grid = render(interp, ed);
    out.push_str(&format!("=== {} ===\n", name));
    out.push_str(&serialize(&grid));
    out.push('\n');
}

// ---------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------

/// 1a. Real SystemVerilog (`demo/rtl/core/alu.sv`, has lines up to 86
/// columns) at a frame narrow enough (40 cols) to force wrapping.
fn scenario_sv_narrow_wrap(out: &mut String) {
    let (mut i, ed) = setup(40, 20);
    find_file(&mut i, "rtl/core/alu.sv");
    capture(&i, &ed, "sv_narrow_wrap", out);
}

/// 1b. Same file, wide enough that its longest line (86 cols) still
/// doesn't wrap.
fn scenario_sv_wide_no_wrap(out: &mut String) {
    let (mut i, ed) = setup(100, 15);
    find_file(&mut i, "rtl/core/alu.sv");
    capture(&i, &ed, "sv_wide_no_wrap", out);
}

/// Scenario 2: Line-number gutter off (the default) then on, same buffer.
fn scenario_gutter_off_and_on(out: &mut String) {
    let (mut i, ed) = setup(60, 12);
    find_file(&mut i, "rtl/core/regfile.sv");
    capture(&i, &ed, "gutter_off", out);
    run(&mut i, "(setq display-line-numbers t)");
    capture(&i, &ed, "gutter_on", out);
}

/// 3a. A vertical split (`split-window-right`) -- `windows` must carry
/// two entries side by side.
fn scenario_vertical_split(out: &mut String) {
    let (mut i, ed) = setup(90, 15);
    find_file(&mut i, "rtl/core/alu.sv");
    run(&mut i, "(split-window-right)");
    capture(&i, &ed, "vertical_split", out);
}

/// 3b. A horizontal split (`split-window-below`) -- two entries stacked,
/// each with its own mode line row.
fn scenario_horizontal_split(out: &mut String) {
    let (mut i, ed) = setup(60, 20);
    find_file(&mut i, "rtl/core/regfile.sv");
    run(&mut i, "(split-window-below)");
    capture(&i, &ed, "horizontal_split", out);
}

/// Scenario 4: Tabs, including one that lands exactly at the wrap boundary.
/// Frame is 30 cols (`cols - 1` == 29). 26 `x` chars put the column at
/// 26; a tab there has `char_width('\t', 26) == 8 - (26 % 8) == 6`, so
/// `col + w == 32 > 29` -- the wrap-before-draw branch in
/// `redisplay.rs`'s buffer loop fires, and the tab itself is drawn at
/// column 0 of the *next* row, not split across the boundary. The
/// second line has ordinary mid-line tabs for contrast.
fn scenario_tabs(out: &mut String) {
    let (mut i, ed) = setup(30, 10);
    run(
        &mut i,
        &format!("(insert \"{}\\tEND\\na\\tb\\tc\\n\")", "x".repeat(26)),
    );
    capture(&i, &ed, "tabs", out);
}

/// Scenario 5: CJK wide characters in ordinary buffer text: one mid-line (must
/// produce a `put_wide` + continuation-cell pair), and one placed so
/// that drawing it would straddle the wrap boundary -- 18 `a` chars put
/// the column at 18; frame is 20 cols (`cols - 1 == 19`), and
/// `char_width('界', 18) == 2`, so `18 + 2 == 20 > 19` forces the
/// buffer-text wrap-before-draw branch, moving `界` to column 0 of the
/// next row whole rather than splitting it. (The OTHER wide-char corner
/// case named in the milestone spec -- a wide char clipped in place by
/// a scrolled window's right edge -- has no buffer-text equivalent;
/// buffer text always wraps rather than horizontally scrolling, so that
/// case is exercised in `scenario_echo_area` below instead, which
/// mirrors the dedicated regression test for it,
/// `completing_read_tests.rs`'s `m70_i8_wide_char_split_at_right_
/// scroll_edge_draws_blank`.)
fn scenario_cjk(out: &mut String) {
    let (mut i, ed) = setup(20, 8);
    run(&mut i, "(insert \"中文 mid-line 界 end\\n\")");
    run(
        &mut i,
        &format!("(insert \"{}界bbbbb\\n\")", "a".repeat(18)),
    );
    capture(&i, &ed, "cjk", out);
}

/// Scenario 6: Control characters, which render as `^X` two-cell sequences.
///
/// Second line is deliberately positioned at the exact wrap-boundary
/// column: 28 `e`s put `col` at 28, and a control char's WIDTH (not
/// just its two-cell rendering) is `char_width`'s job -- `char_width(c,
/// 28) == 2` for `c < 0x20`, so `col + w == 30 == cols`, one past
/// `cols - 1`. That makes the draw loop's own wrap check
/// (`redisplay.rs`, the `if col + w > cols.saturating_sub(1)` right
/// before the character match) fire and move this control char to the
/// next row. If `char_width`'s control-char branch ever regresses to
/// return 1 instead of 2, `28 + 1 == 29`, which does NOT exceed `cols -
/// 1 == 29` -- the check no longer fires, the control char gets drawn
/// in place instead of wrapping, and (since the two `grid.put` calls
/// for `^`/`X` are unconditional once inside that match arm) its
/// SECOND cell lands exactly on the column the marker/next glyph would
/// otherwise occupy. Confirmed by hand: reverting `char_width`'s `c if
/// (c as u32) < 32 => 2` to `=> 1` changes this row's rendered text --
/// see the milestone's mutation-testing notes for the exact diff.
fn scenario_control_chars(out: &mut String) {
    let (mut i, ed) = setup(30, 8);
    // ?\C-a is 0x01, ?\C-b is 0x02; \x7f (DEL) is spliced in directly
    // from Rust since the elisp string reader has no `\x` escape (same
    // reason `help_tests.rs`'s `echo_del_also_renders_as_the_buffer_
    // path_does` does it this way).
    let ctrl3 = '\u{3}'; // C-c, distinct from C-a/C-b above so the two
                         // lines are independently identifiable in a diff
    let src = format!(
        "(insert ?\\C-a ?\\C-b \"mid{del}end\\n{e}{ctrl3}TAIL\\n\")",
        del = '\u{7f}',
        e = "e".repeat(28),
        ctrl3 = ctrl3
    );
    run(&mut i, &src);
    capture(&i, &ed, "control_chars", out);
}

/// Scenario 7: A long single line that wraps several times, exercising the `\`
/// continuation marker `redisplay.rs` draws in the grid's last column
/// on every wrapped row. Frame is 15 cols; the line is 60 chars, so it
/// wraps 4 times (14 content columns + 1 marker column per row).
fn scenario_long_wrap(out: &mut String) {
    let (mut i, ed) = setup(15, 10);
    run(&mut i, &format!("(insert \"{}\")", "x".repeat(60)));
    capture(&i, &ed, "long_wrap", out);
}

/// Scenario 8: Echo area / minibuffer, which has its own separate horizontal-
/// scrolling code path (`echo_cells`/`echo_scroll_off`, distinct from
/// the buffer-text wrap loop above). Two sub-cases: a plain `message`,
/// and the dedicated CJK-clipped-at-the-right-scroll-edge regression
/// (mirrors `completing_read_tests.rs`'s `m70_i8_...` test exactly, see
/// that test's own comment for the column arithmetic).
fn scenario_echo_area(out: &mut String) {
    let (mut i, ed) = setup(80, 24);
    run(&mut i, "(message \"hello from the echo area\")");
    capture(&i, &ed, "echo_message", out);

    let (mut i2, ed2) = setup(80, 24);
    let initial = format!("{}{}{}", "x".repeat(87), '寬', "y".repeat(30));
    run(
        &mut i2,
        &format!(
            "(setq result nil) (read-string \"P: \" (lambda (s) (setq result s)) \"{initial}\")"
        ),
    );
    feed_keys(&mut i2, &ed2, "C-a").unwrap();
    for _ in 0..87 {
        feed_keys(&mut i2, &ed2, "C-f").unwrap();
    }
    capture(&i2, &ed2, "echo_minibuffer_cjk_clip", out);
}

/// Scenario 9: Scrolling: `ensure_point_visible` must move `window_start` off 0,
/// which is the only way `next_row_start` (`redisplay.rs`'s lookahead
/// scanner used by `scan_forward_for_point`/`recenter`) actually
/// influences the rendered grid -- the scanner never draws anything
/// itself, it only decides which portion of the buffer the draw loop
/// (a separate, independent wrap computation) starts from. Without a
/// scenario that forces `window_start` away from 0, `next_row_start`'s
/// own arithmetic is *exercised* (it runs every render, even to decide
/// "does point already fit") but never *observed*: whatever it
/// computes is discarded the moment `scan_forward_for_point` finds
/// point already fits inside the still-window_start-0 view.
///
/// Frame is 24 cols (chosen because 24 is a multiple of 8, so a tab
/// can land exactly on the boundary described below same as the other
/// three character kinds) x 10 rows (`text_rows == 8`, frame rows minus
/// the window's own mode line and the shared echo row).
///
/// Four "trigger" lines are each built so that, at the column where
/// their one interesting character sits, `col + w == cols` exactly
/// (`cols == 24`):
/// - line 0: 24 plain `a`s -- the 24th one lands at `col == 23`,
///   `char_width('a', 23) == 1`, `23 + 1 == 24`.
/// - line 1: 16 `b`s then a tab -- `col == 16` (a tab stop already),
///   `char_width('\t', 16) == 8`, `16 + 8 == 24`.
/// - line 2: 22 `c`s then a CJK char (`界`) -- `col == 22`,
///   `char_width('界', 22) == 2`, `22 + 2 == 24`.
/// - line 3: 22 `d`s then a control char (`C-a`) -- `col == 22`,
///   `char_width(0x01, 22) == 2`, `22 + 2 == 24`.
///
/// `col + w == cols` is the ONE integer value where the real wrap
/// check (`col + w > cols - 1`, true whenever `col + w >= cols`) and
/// the mutated one from the coordinator's finding (`col + w > cols`,
/// true only when `col + w >= cols + 1`) disagree: real code wraps
/// (these 4 lines cost 2 visual rows each, 8 rows total), mutated code
/// does not (1 row each, 4 rows total) -- a 4-row deficit that
/// `scan_forward_for_point` accumulates while walking from
/// `window_start == 0` toward point.
///
/// Ten short filler lines (`F01`..`F10`, one row each under EITHER
/// version -- no wrapping-relevant character in them) follow. Point is
/// placed at the start of `F06` (`forward-line 9` from `point-min`:
/// lines 0-3 are the triggers, 4-8 are `F01`..`F05`, so line 9 is
/// `F06`). Real total rows from `window_start == 0` to that point: 8
/// (triggers) + 5 (`F01`..`F05`) == 13, which exceeds `text_rows ==
/// 8`, so real code MUST scroll. Mutated total: 4 + 5 == 9, which ALSO
/// exceeds 8, so mutated code scrolls too -- just by 4 fewer rows,
/// landing `window_start` on a different line than real code does. So
/// this scenario doesn't merely need "some" scrolling to happen; it's
/// built so BOTH versions scroll, to two different, individually
/// verifiable positions, which is what makes the divergence a
/// dependable content difference rather than a coincidental one.
fn scenario_scroll(out: &mut String) {
    let (mut i, ed) = setup(24, 10);
    let ctrl = '\u{1}'; // C-a
    let fillers = (1..=10)
        .map(|n| format!("F{:02}", n))
        .collect::<Vec<_>>()
        .join("\\n");
    let content = format!(
        "{a}\\n{b}\\t\\n{c}界\\n{d}{ctrl}\\n{fillers}",
        a = "a".repeat(24),
        b = "b".repeat(16),
        c = "c".repeat(22),
        d = "d".repeat(22),
        ctrl = ctrl,
        fillers = fillers,
    );
    run(&mut i, &format!("(insert \"{}\")", content));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 9)"); // start of F06 -- see doc comment
    capture(&i, &ed, "scroll_forces_window_start_off_zero", out);
}

// ---------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------

fn build_golden() -> String {
    let mut out = String::new();
    scenario_sv_narrow_wrap(&mut out);
    scenario_sv_wide_no_wrap(&mut out);
    scenario_gutter_off_and_on(&mut out);
    scenario_vertical_split(&mut out);
    scenario_horizontal_split(&mut out);
    scenario_tabs(&mut out);
    scenario_cjk(&mut out);
    scenario_control_chars(&mut out);
    scenario_long_wrap(&mut out);
    scenario_echo_area(&mut out);
    scenario_scroll(&mut out);
    out
}

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/layout_golden.txt"
);

/// Compares line-by-line so a mismatch points at the exact line rather
/// than dumping the whole (large) string.
fn compare(expected: &str, actual: &str) {
    if expected == actual {
        return;
    }
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let n = exp_lines.len().min(act_lines.len());
    for i in 0..n {
        if exp_lines[i] != act_lines[i] {
            panic!(
                "golden master mismatch at fixture line {}:\n  expected: {:?}\n  actual:   {:?}\n\n\
                 If this divergence is an INTENTIONAL result of the redisplay refactor \
                 (confirmed byte-identical rendering was not the goal here, or a real bug \
                 was fixed), regenerate with RETICLE_BLESS_GOLDEN=1. Otherwise this is the \
                 refactor changing rendering it wasn't supposed to change.",
                i + 1,
                exp_lines[i],
                act_lines[i]
            );
        }
    }
    if exp_lines.len() != act_lines.len() {
        panic!(
            "golden master mismatch: expected {} lines, got {} lines (diverged after the \
             common {}-line prefix)",
            exp_lines.len(),
            act_lines.len(),
            n
        );
    }
}

#[test]
fn layout_golden_master() {
    let actual = build_golden();
    if std::env::var("RETICLE_BLESS_GOLDEN")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        std::fs::write(FIXTURE_PATH, &actual)
            .unwrap_or_else(|e| panic!("failed to write {}: {}", FIXTURE_PATH, e));
        return;
    }
    let expected = std::fs::read_to_string(FIXTURE_PATH)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", FIXTURE_PATH, e));
    compare(&expected, &actual);
}
