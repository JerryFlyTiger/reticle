//! M118: the sticky scope header (F1), the scope breadcrumb (F2), and
//! their shared foundation, `highlight::Engine::scope_chain`.
//!
//! Follows `show_paren_tests.rs`'s pattern throughout: a real major-mode
//! function turns the highlight engine on, `tick_until` pumps
//! `core::idle_tick` past the debounce so the background parse actually
//! finishes, and `Engine::scope_chain`/the rendered `Grid` are the two
//! observable surfaces (the chain itself for the foundation tests, the
//! grid for F1's header rows -- F1 is a per-frame direct grid write,
//! not an overlay, so there is no overlay to inspect).
//!
//! `display-line-numbers` is off in every fixture except
//! `header_rows_gutter_carries_no_line_number` (the one test that
//! needs a gutter to inspect), same reasoning `show_paren_tests.rs`
//! gives: with it off, column 0 of the text area is always `tx`, so
//! there is no gutter width to account for in the rest of these tests.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use core::redisplay::render;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(src: &str, mode: &str, cols: usize, rows: usize) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (cols, rows);
    run(&mut interp, &format!("(insert {:?})", src));
    run(&mut interp, &format!("({})", mode));
    run(&mut interp, "(setq-local display-line-numbers nil)");
    run(&mut interp, "(setq hl-line-mode nil)");
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// Same contract as `show_paren_tests.rs`'s helper of the same name.
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    for _ in 0..200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

const COUNT_HL: &str = "(let ((n 0))
   (dolist (ov (overlays-in (point-min) (point-max)) nil)
     (when (overlay-get ov 'treesit-hl) (setq n (1+ n))))
   n)";

/// 0-based byte (== char, every fixture here is pure ASCII) offset of
/// the start of `line0` (0-based) in `src`.
fn line_start_offset(src: &str, line0: usize) -> usize {
    let mut off = 0;
    for (i, line) in src.split('\n').enumerate() {
        if i == line0 {
            return off;
        }
        off += line.len() + 1;
    }
    src.len()
}

/// Point directly at `line0`/`col0` (0-based), bypassing `goto-char`'s
/// 1-based elisp interface -- same direct-field-set precedent
/// `show_paren_tests.rs`'s `region_wins_over_the_paren_highlight_...`
/// test uses.
fn set_point(ed: &Rc<RefCell<Editor>>, src: &str, line0: usize, col0: usize) -> usize {
    let pos = line_start_offset(src, line0) + col0;
    ed.borrow().current.borrow_mut().point = pos;
    pos
}

/// Pin the selected window's `window_start` to `line0` (0-based) and
/// mark it scroll-pinned at `point` -- the same mechanism a real mouse
/// wheel event uses (`Window::scroll_pin`'s own doc), which makes
/// `render_window` skip `ensure_point_visible`'s recentre for this
/// frame as long as point doesn't move. Used here to get exact,
/// reproducible row offsets for the header-row tests without depending
/// on `ensure_point_visible`'s own recentring heuristics.
fn pin_window_start(ed: &Rc<RefCell<Editor>>, src: &str, line0: usize, point: usize) {
    let win_id = ed.borrow().selected_window;
    let start = line_start_offset(src, line0);
    let mut e = ed.borrow_mut();
    let w = e.windows.get_mut(&win_id).expect("selected window exists");
    w.window_start = start;
    w.scroll_pin = Some(point);
}

/// Same as `pin_window_start` above, but `window_start` is an exact char
/// offset the caller computed rather than a line's own start -- needed
/// for the FIX-2 wrapped-line regression test below, where
/// `window_start` deliberately sits mid-line (a soft-wrap continuation
/// row), not at a line boundary.
fn pin_window_start_offset(ed: &Rc<RefCell<Editor>>, start_offset: usize, point: usize) {
    let win_id = ed.borrow().selected_window;
    let mut e = ed.borrow_mut();
    let w = e.windows.get_mut(&win_id).expect("selected window exists");
    w.window_start = start_offset;
    w.scroll_pin = Some(point);
}

fn grid_row_text(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let g = render(interp, ed);
    g.lines[row]
        .iter()
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

// =====================================================================
// Foundation: `Engine::scope_chain`
// =====================================================================

#[test]
fn chain_inside_a_module_instantiation_in_real_soc_top_sv() {
    // Real file content, read rather than invented -- soc_top.sv (see
    // M118's own spec) has no `always_ff` at its own top level (it
    // lives in the submodules), but it DOES have several
    // `module_instantiation`s directly inside the module, which is
    // exactly the same "nested scope" shape the milestone spec's
    // `always_ff`-inside-`module` example describes.
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl/top/soc_top.sv"
    ))
    .expect("demo/rtl/top/soc_top.sv");
    let (mut i, ed) = setup(&src, "verilog-mode", 80, 20);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point inside u_regfile's own port list (".clk_i    (clk_i),").
    let line0 = src
        .lines()
        .position(|l| l.contains(".clk_i    (clk_i)"))
        .expect("u_regfile's .clk_i line");
    let pos = set_point(&ed, &src, line0, 4);

    let buf = ed.borrow().current.clone();
    let chain = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    let kinds_labels: Vec<(&str, &str)> = chain
        .iter()
        .map(|s| (s.kind.as_str(), s.label.as_str()))
        .collect();
    assert_eq!(
        kinds_labels,
        vec![
            ("module_declaration", "soc_top"),
            ("module_instantiation", "u_regfile"),
        ],
        "expected exactly [module_declaration soc_top, module_instantiation \
         u_regfile], outermost first; got {kinds_labels:?}"
    );
}

#[test]
fn chain_at_top_level_of_the_file_is_empty() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl/top/soc_top.sv"
    ))
    .expect("demo/rtl/top/soc_top.sv");
    let (mut i, ed) = setup(&src, "verilog-mode", 80, 20);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Line 0 is the file's very first comment line, well before the
    // `module` keyword -- outside every construct.
    let pos = set_point(&ed, &src, 0, 0);
    let buf = ed.borrow().current.clone();
    let chain = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    assert!(
        chain.is_empty(),
        "point before the module keyword must have an empty chain, got {:?}",
        chain.iter().map(|s| &s.label).collect::<Vec<_>>()
    );
}

#[test]
fn chain_in_a_plain_text_buffer_with_no_grammar_is_empty() {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(
        &mut interp,
        "(insert \"just some plain text, no major mode\")",
    );
    // No `treesit-highlight-mode` call at all -- `fundamental-mode` (the
    // default) never enables the highlight engine for this buffer, so
    // `Engine::scope_chain` must find no entry for it in `states` and
    // return empty immediately, not panic or block.
    let buf = ed.borrow().current.clone();
    let chain = ed
        .borrow()
        .hl
        .as_ref()
        .map(|hl| hl.scope_chain(&buf, 3))
        .unwrap_or_default();
    assert!(chain.is_empty());
}

#[test]
fn generation_guard_returns_empty_for_a_stale_cache() {
    // Mirrors `show_paren_tests.rs`'s
    // `stale_cache_after_an_edit_elsewhere_yields_no_highlight_until_
    // reparse_lands`: an edit bumps `edit_ticks` immediately, the
    // worker only catches up a debounce interval later, and
    // `scope_chain` must return empty during that window rather than
    // trust the now-stale cached `Scope` offsets.
    let src = "module m;\n  wire w;\nendmodule\n";
    let (mut i, ed) = setup(src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, src, 1, 2); // inside the module, on "wire w;"
    let buf = ed.borrow().current.clone();
    let before = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    assert!(
        !before.is_empty(),
        "sanity: point inside the module has a non-empty chain before the edit"
    );

    // Edit elsewhere without moving point -- bumps `edit_ticks`.
    run(
        &mut i,
        "(save-excursion (goto-char 1) (insert \"// x\\n\"))",
    );

    // No `idle_tick` here: this is deliberately the race window before
    // the debounced reparse lands.
    let stale = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    assert!(
        stale.is_empty(),
        "a stale cached parse must yield an empty chain, not stale offsets; got {:?}",
        stale.iter().map(|s| &s.label).collect::<Vec<_>>()
    );
}

#[test]
fn rust_chain_impl_and_function() {
    let src = "impl Foo {\n    fn bar(&self) -> i32 {\n        1\n    }\n}\n";
    let (mut i, ed) = setup(src, "rust-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, src, 2, 4); // the "1" inside bar's body
    let buf = ed.borrow().current.clone();
    let chain = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    let kinds_labels: Vec<(&str, &str)> = chain
        .iter()
        .map(|s| (s.kind.as_str(), s.label.as_str()))
        .collect();
    assert_eq!(
        kinds_labels,
        vec![("impl_item", "impl Foo"), ("function_item", "bar")]
    );
}

#[test]
fn python_chain_class_and_function() {
    let src = "class Foo:\n    def bar(self):\n        return 1\n";
    let (mut i, ed) = setup(src, "python-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, src, 2, 8); // "return 1" inside bar's body
    let buf = ed.borrow().current.clone();
    let chain = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    let kinds_labels: Vec<(&str, &str)> = chain
        .iter()
        .map(|s| (s.kind.as_str(), s.label.as_str()))
        .collect();
    assert_eq!(
        kinds_labels,
        vec![("class_definition", "Foo"), ("function_definition", "bar")]
    );
}

// =====================================================================
// F1: sticky scope header rows in the rendered `Grid`
// =====================================================================

/// Synthetic Verilog, laid out so every header-row test below can pin
/// `window_start`/point to an exact, hand-computed line number instead
/// of depending on `ensure_point_visible`'s own recentring heuristics.
/// Line numbers (0-based), the numbers every test below is built from:
///
/// ```text
///  0  module m;
///  1  ..20  wire w0; .. wire w19;
/// 21  sub inst_u (
/// 22      .a(w0)
/// 23  );
/// 24  endmodule
/// ```
///
/// `module_declaration` starts at line 0; `module_instantiation`
/// (`inst_u`) starts at line 21 and ends on line 23 -- short enough
/// that a window scrolled past its OWN start line while point is still
/// inside it (lines 22-23) is a real, reachable state, which is exactly
/// what several tests below need.
fn scroll_fixture() -> String {
    let mut src = String::from("module m;\n");
    for i in 0..20 {
        src.push_str(&format!("  wire w{i};\n"));
    }
    src.push_str("  sub inst_u (\n");
    src.push_str("    .a(w0)\n");
    src.push_str("  );\n");
    src.push_str("endmodule\n");
    src
}

#[test]
fn header_row_spells_the_scrolled_off_module_source_line() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point on line 18 ("wire w17;"), inside the module but not yet
    // inside `inst_u` (which only starts at line 21) -- a 1-deep chain.
    let pos = set_point(&ed, &src, 18, 2);
    pin_window_start(&ed, &src, 15, pos); // window shows lines 15..24
    let rect_row = render(&i, &ed).windows[0].row;

    assert_eq!(
        grid_row_text(&i, &ed, rect_row),
        "module m;",
        "the top row must be pinned to the module's own source line, \
         which has scrolled off the top of the window"
    );
}

#[test]
fn scope_header_nil_leaves_the_row_as_ordinary_buffer_text() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, "(setq scope-header nil)");

    let pos = set_point(&ed, &src, 18, 2);
    pin_window_start(&ed, &src, 15, pos);
    let rect_row = render(&i, &ed).windows[0].row;

    assert_eq!(
        grid_row_text(&i, &ed, rect_row),
        "  wire w14;",
        "scope-header nil must leave the window's real top row (line 15, \
         \"wire w14;\") untouched -- no pinning at all"
    );
}

#[test]
fn max_lines_one_with_a_two_deep_chain_keeps_exactly_one_innermost_row() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, "(setq scope-header-max-lines 1)");

    // Point on line 23 (");", the instantiation's closing line, still
    // inside its char range), window scrolled to line 22 -- BOTH
    // module_declaration (line 0) and module_instantiation (line 21)
    // have scrolled off (0 < 22 and 21 < 22), a genuine 2-deep chain.
    let pos = set_point(&ed, &src, 23, 0);
    pin_window_start(&ed, &src, 22, pos);
    let rect_row = render(&i, &ed).windows[0].row;

    assert_eq!(
        grid_row_text(&i, &ed, rect_row),
        "  sub inst_u (",
        "max-lines 1 must keep the INNERMOST construct (the instantiation), \
         not the outer module"
    );
    // And there must be exactly one such row: the row right below it is
    // ordinary buffer text (line 23 itself, the window's second visible
    // row), not a second header row.
    assert_eq!(grid_row_text(&i, &ed, rect_row + 1), "  );");
}

#[test]
fn nothing_scrolled_off_at_the_top_of_the_file_produces_no_header_row() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, &src, 3, 2);
    pin_window_start(&ed, &src, 0, pos); // window starts at the very top
    let rect_row = render(&i, &ed).windows[0].row;

    assert_eq!(
        grid_row_text(&i, &ed, rect_row),
        "module m;",
        "with window_start at line 0, the module's own opening line is \
         ALREADY on screen, so it must render as an ordinary row, not a \
         pinned header row (this happens to read identically either way -- \
         the real assertion is buffer_pos_at below)"
    );
    let g = render(&i, &ed);
    assert!(
        g.buffer_pos_at(rect_row, 0).is_some(),
        "the top row must be real buffer text (a mapped position), not a \
         chrome header row, when nothing has scrolled off"
    );
}

#[test]
fn point_on_the_first_visible_row_caps_the_header_to_zero_rows() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Same 2-deep-chain position as the max-lines test above, but this
    // time point sits on window_start's own row (point_row 0) --
    // n.min(point_row) must cap the header to zero rows so it can never
    // cover the row point is painted on.
    let pos = set_point(&ed, &src, 22, 4);
    pin_window_start(&ed, &src, 22, pos);
    let rect_row = render(&i, &ed).windows[0].row;

    let g = render(&i, &ed);
    assert!(
        g.buffer_pos_at(rect_row, 0).is_some(),
        "point on the window's own first row must leave that row as \
         ordinary buffer text -- zero header rows"
    );
}

#[test]
fn point_on_the_second_visible_row_with_a_two_deep_chain_allows_one_header_row() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point one row below window_start (point_row 1) with the same
    // 2-deep chain -- the cap allows exactly one header row now.
    let pos = set_point(&ed, &src, 23, 0);
    pin_window_start(&ed, &src, 22, pos);
    let rect_row = render(&i, &ed).windows[0].row;

    let g = render(&i, &ed);
    assert!(
        g.buffer_pos_at(rect_row, 0).is_none(),
        "the top row must be a pinned header row (no buffer position) \
         once point has moved one row past window_start"
    );
    assert!(
        g.buffer_pos_at(rect_row + 1, 0).is_some(),
        "and only ONE such row -- point's own row (row 1) must be \
         ordinary buffer text"
    );
}

#[test]
fn header_row_never_maps_back_to_a_buffer_position() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, &src, 18, 2);
    pin_window_start(&ed, &src, 15, pos);
    let g = render(&i, &ed);
    let rect_row = g.windows[0].row;
    let rect_col = g.windows[0].col;
    let cols = g.windows[0].cols;

    for col in rect_col..rect_col + cols {
        assert_eq!(
            g.buffer_pos_at(rect_row, col),
            None,
            "column {col} of the header row must not map to any buffer \
             position -- a click there must never move point"
        );
    }
}

#[test]
fn header_rows_gutter_carries_no_line_number() {
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    run(&mut i, "(setq-local display-line-numbers t)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let pos = set_point(&ed, &src, 18, 2);
    pin_window_start(&ed, &src, 15, pos);
    let g = render(&i, &ed);
    let rect_row = g.windows[0].row;
    let rect_col = g.windows[0].col;
    let gutter_w = g.windows[0].gutter_cols;
    assert!(gutter_w > 0, "sanity: the gutter must actually be showing");

    let gutter_text: String = g.lines[rect_row][rect_col..rect_col + gutter_w]
        .iter()
        .map(|c| c.ch)
        .collect();
    assert!(
        !gutter_text.chars().any(|c| c.is_ascii_digit()),
        "the header row's gutter must carry no line number, got {gutter_text:?}"
    );
}

// =====================================================================
// F2: the scope breadcrumb.
//
// `compose_mode_line`'s own formatting/priority-order rules are pure
// and covered by unit tests in `redisplay.rs`'s `#[cfg(test)]` block
// (the required test list's items 13-16). Those tests construct
// `ModeLineParts` directly, though, so none of them would notice a
// break in the WIRING between `render_window` and that function --
// e.g. `render_window` never calling `scope_chain`, or never passing
// the result into `ModeLineParts::breadcrumb` at all. This test closes
// that gap the way `gui_features_tests.rs`'s
// `segmented_modeline_shows_name_position_and_lsp_state` closes it for
// the LSP segment: read the mode line's OWN row out of a real rendered
// `Grid`, from real buffer content parsed by the real highlight engine
// -- see this project's CLAUDE.md on "feature and tests not wired" for
// why a unit test on the pure half is not sufficient on its own.
// =====================================================================

#[test]
fn breadcrumb_actually_appears_on_the_rendered_mode_line_row() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl/top/soc_top.sv"
    ))
    .expect("demo/rtl/top/soc_top.sv");
    let (mut i, ed) = setup(&src, "verilog-mode", 80, 20);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let line0 = src
        .lines()
        .position(|l| l.contains(".clk_i    (clk_i)"))
        .expect("u_regfile's .clk_i line");
    set_point(&ed, &src, line0, 4);

    let g = render(&i, &ed);
    let mode_row = g.windows[0].mode_line_row;
    let mode_text: String = g.lines[mode_row].iter().map(|c| c.ch).collect();
    assert!(
        mode_text.contains("[soc_top > u_regfile]"),
        "the mode line's own rendered row must show the breadcrumb for \
         real content parsed by the real engine, not just in a unit test \
         that hand-builds ModeLineParts: {mode_text:?}"
    );

    // And the nil toggle actually reaches this same wiring, not just
    // `compose_mode_line`'s own parameter.
    run(&mut i, "(setq scope-breadcrumb nil)");
    let g2 = render(&i, &ed);
    let mode_text2: String = g2.lines[mode_row].iter().map(|c| c.ch).collect();
    assert!(
        !mode_text2.contains('['),
        "scope-breadcrumb nil must remove the breadcrumb from the real \
         rendered mode line: {mode_text2:?}"
    );
}

// =====================================================================
// Review fix round (cold review of the initial M118 diff): FIX-1,
// FIX-2, FIX-4.
// =====================================================================

#[test]
fn header_cleanup_does_not_corrupt_a_neighbouring_split_window() {
    // FIX-1: `render_window`'s header-cleanup `retain` used to bound
    // only by ROW, and `grid.runs` is frame-global -- on a side-by-side
    // split, both windows share the same absolute rows, so the RIGHT
    // window's cleanup deleted the LEFT window's already-pushed text
    // runs for those rows too (same rows, different columns). The
    // glyphs still looked right (`grid.put` already wrote them
    // directly), but `Grid::buffer_pos_at` found no run left and
    // returned `None` -- a click on the left window's top rows would
    // stop moving point.
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 120, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // `split_selected` (editor.rs) keeps the ORIGINAL window selected as
    // side `a` (left) and gives the new window (side `b`, right) the
    // next id -- `compute_rects` always renders side `a` first
    // regardless of which side ends up selected, so selecting the RIGHT
    // window below and giving it a header reproduces the exact ordering
    // the bug needs: left's runs pushed first, right's cleanup running
    // second.
    let left_id = ed.borrow().selected_window;
    run(&mut i, "(split-window-right)");
    let right_id = {
        let mut ids: Vec<usize> = ed.borrow().windows.keys().copied().collect();
        ids.sort_unstable();
        *ids.last().expect("split created a second window")
    };
    assert_ne!(
        left_id, right_id,
        "sanity: split must create a distinct window id"
    );

    // Select the right window FIRST -- `select_window` (editor.rs) syncs
    // the OUTGOING selected window's saved `point` from the live buffer
    // as one of its side effects, so setting the left window's fields
    // before this call would just get overwritten by it.
    run(&mut i, &format!("(select-window {right_id})"));

    // Left window: parked at the very top -- ordinary buffer text, no
    // header row of its own.
    {
        let mut e = ed.borrow_mut();
        let w = e.windows.get_mut(&left_id).unwrap();
        w.window_start = 0;
        w.point = 0;
    }

    // Pin the (now-selected) right window's scroll deep enough to show a
    // header row -- same fixture/offsets as
    // `header_row_spells_the_scrolled_off_module_source_line` above.
    let pos = set_point(&ed, &src, 18, 2);
    pin_window_start(&ed, &src, 15, pos);

    let g = render(&i, &ed);
    let left_rect = *g
        .windows
        .iter()
        .find(|w| w.win_id == left_id)
        .expect("left window has a layout entry");
    let right_rect = *g
        .windows
        .iter()
        .find(|w| w.win_id == right_id)
        .expect("right window has a layout entry");
    assert_eq!(
        left_rect.row, right_rect.row,
        "sanity: a side-by-side split puts both windows' top row at the \
         same absolute frame row -- the exact condition FIX-1 needs"
    );

    assert!(
        g.buffer_pos_at(left_rect.row, left_rect.col).is_some(),
        "the LEFT window's own top row must still map to a buffer \
         position -- the right window's header-cleanup `retain` must not \
         delete a neighbouring window's already-pushed runs for the same \
         absolute rows"
    );
    assert!(
        g.buffer_pos_at(right_rect.row, right_rect.col).is_none(),
        "sanity: the right window's own header row must still have no \
         buffer position"
    );
}

#[test]
fn a_wrapped_declaration_line_pins_once_the_window_scrolls_past_its_middle() {
    // FIX-2: the old filter compared LINE NUMBERS
    // (`s.start_line < window_start_line`). This editor soft-wraps, so
    // `window_start` can sit mid-line on a continuation row; when that
    // happens, the construct's own `start_line` and the window's start
    // line compare EQUAL, and the old filter said "still on screen" even
    // though the beginning of the line has visually scrolled off the
    // top of the window.
    //
    // Isolated on purpose: a single top-level `package_declaration`
    // whose own declaration line is made deliberately long, with point
    // resting on the line right after it (not inside any NESTED scope,
    // so the chain has exactly one entry -- an enclosing `module`, if
    // there were one, would already get pinned by its OWN start_line
    // being a full line above, masking exactly the bug this test needs
    // to isolate).
    let long_name = "this_is_a_very_long_package_name_used_only_to_make_the_declaration_line_long_enough_to_still_be_the_current_source_line_at_a_column_past_one_hundred";
    let src = format!("package {long_name};\n  int x;\nendpackage\n");
    assert!(
        src.find('\n').unwrap() > 100,
        "sanity: the package declaration line must be over 100 chars long"
    );

    let (mut i, ed) = setup(&src, "verilog-mode", 60, 10);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point on "int x;" (line 1) -- chain is exactly [package_declaration].
    let pos = set_point(&ed, &src, 1, 2);
    let buf = ed.borrow().current.clone();
    let chain = ed.borrow().hl.as_ref().unwrap().scope_chain(&buf, pos);
    assert_eq!(
        chain.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
        vec!["package_declaration"],
        "sanity: exactly one scope in the chain, isolating FIX-2 from any \
         nesting that would mask it"
    );

    // `window_start` at char offset 100 -- still on line 0 (the package
    // line is >100 chars), but well past its own start: a wrapped
    // continuation row, not the line's own first row.
    pin_window_start_offset(&ed, 100, pos);
    let g = render(&i, &ed);
    let rect_row = g.windows[0].row;

    assert!(
        g.buffer_pos_at(rect_row, 0).is_none(),
        "the package declaration's beginning has scrolled off (window_start \
         is 100 chars into its own line), so the top row must be a pinned \
         header row -- no buffer position"
    );
}

#[test]
fn a_two_row_window_never_becomes_entirely_header() {
    // FIX-4: `n.min(text_rows.saturating_sub(1))` (redisplay.rs) is
    // meant to guarantee a window with just ONE text row (`rows == 2`:
    // one text row + one mode-line row) never has that single row
    // replaced by a header -- every other fixture in this file uses a
    // 10-row window, so this clamp had no test at all.
    //
    // Must be a NON-selected window: for a SELECTED window, the
    // point-row cap (`n = n.min(point_row)`) already forces `n` to 0 on
    // its own whenever `text_rows == 1` (the cursor's row is clamped to
    // `text_rows - 1 == 0`, so `point_row` is always 0), which would
    // make this test pass even with the two-row clamp itself deleted or
    // off-by-one'd. Only a background window skips that cap entirely
    // (`render_window`'s `if is_selected { ... }` block), leaving the
    // two-row clamp as the SOLE thing standing between a 2-deep
    // scrolled-off chain and a fully-header window with zero visible
    // text rows.
    let src = scroll_fixture();
    let (mut i, ed) = setup(&src, "verilog-mode", 60, 12);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, "(setq scope-header-max-lines 3)");

    // `(split-window-below 2)` gives the ORIGINAL (still-selected)
    // window exactly 2 rows (one text row + mode line); the new window
    // below it takes the rest. `(other-window)` then moves selection to
    // that new window, leaving the original 2-row window as the
    // background one this test needs.
    let top_id = ed.borrow().selected_window;
    run(&mut i, "(split-window-below 2)");
    run(&mut i, "(other-window)");
    assert_ne!(
        ed.borrow().selected_window,
        top_id,
        "sanity: selection moved off the top window"
    );

    // Same 2-deep-chain position `max_lines_one_with_a_two_deep_chain_...`
    // above uses -- both `module_declaration` and `module_instantiation`
    // have scrolled off, so without the clamp `n` would want 2 rows, more
    // than this window's single available text row.
    let pos = line_start_offset(&src, 23);
    {
        let mut e = ed.borrow_mut();
        let w = e.windows.get_mut(&top_id).expect("top window exists");
        w.window_start = line_start_offset(&src, 22);
        // Scroll-pinned for the same reason `pin_window_start` does it:
        // without the pin, `render_window` runs `ensure_point_visible`
        // for this window too and recentres `window_start` forward off
        // line 22, so the state asserted against would not be the state
        // this comment claims. (Trailing cold read, M118: the first
        // version of this fixture left `scroll_pin` at `None`. The test
        // still guarded its clamp -- both scopes stayed above the
        // recentred start either way -- but it was guarding a position
        // nobody had written down, one fixture tweak away from silently
        // exercising a shallower chain than its name promises.)
        w.scroll_pin = Some(pos);
        w.point = pos;
    }

    let g = render(&i, &ed);
    let top_rect = *g
        .windows
        .iter()
        .find(|w| w.win_id == top_id)
        .expect("top window has a layout entry");
    assert_eq!(
        top_rect.rows, 2,
        "sanity: this window really is the 2-row (one text row) case"
    );
    assert!(
        g.buffer_pos_at(top_rect.row, top_rect.col).is_some(),
        "a 2-row background window with a 2-deep scrolled-off chain must \
         still keep its one text row as ordinary buffer text -- it must \
         never become entirely header"
    );
}

// =====================================================================
// M122: breadcrumb chains from real files, for the five M121 kinds
// (modport/covergroup/program/class-constructor/concurrent-assertion)
// plus nested if-generate-inside-for-generate, none of which had a
// real-file test before this milestone (M118's own test above was the
// only real-file chain test, and it predates all six of these kinds
// having any real material).
// =====================================================================

fn read_demo(rel: &str) -> String {
    let path =
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{:?}: {}", path, e))
}

fn chain_at(src: &str, mode: &str, needle: &str, col: usize) -> Vec<(String, String)> {
    let (mut i, ed) = setup(src, mode, 80, 20);
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    let line0 = src
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{:?} not found in source", needle));
    let pos = set_point(&ed, src, line0, col);
    let buf = ed.borrow().current.clone();
    let e = ed.borrow();
    let chain = e.hl.as_ref().unwrap().scope_chain(&buf, pos);
    chain
        .iter()
        .map(|s| (s.kind.clone(), s.label.clone()))
        .collect()
}

#[test]
fn chain_inside_a_modport_line_in_real_axi4_lite_if_sv() {
    let src = read_demo("rtl/bus/axi4_lite_if.sv");
    let chain = chain_at(&src, "verilog-mode", "modport master(", 4);
    assert_eq!(
        chain,
        vec![
            (
                "interface_declaration".to_string(),
                "axi4_lite_if".to_string()
            ),
            ("modport_declaration".to_string(), "master".to_string()),
        ],
        "expected [interface_declaration axi4_lite_if, modport_declaration \
         master], got {chain:?}"
    );
}

#[test]
fn chain_inside_a_concurrent_assertion_in_real_axi4_lite_if_sv() {
    let src = read_demo("rtl/bus/axi4_lite_if.sv");
    let chain = chain_at(&src, "verilog-mode", "a_aw_stable :", 2);
    assert_eq!(
        chain,
        vec![
            (
                "interface_declaration".to_string(),
                "axi4_lite_if".to_string()
            ),
            (
                "concurrent_assertion_item".to_string(),
                "a_aw_stable: assert property".to_string()
            ),
        ],
        "expected [interface_declaration axi4_lite_if, concurrent_assertion_item \
         a_aw_stable: assert property], got {chain:?}"
    );
}

#[test]
fn chain_inside_the_nested_if_generate_inside_the_for_generate_in_real_sram_bank_sv() {
    let src = read_demo("rtl/mem/sram_bank.sv");
    // "clk_gate u_clk_gate (" sits inside `g_gated_clk`'s `if (NumBanks
    // > 1)` generate arm, itself nested inside the `for (genvar b …)`
    // loop generate's own `g_bank` block -- the exact nested-generate
    // shape M119 had no real material for.
    let chain = chain_at(&src, "verilog-mode", "clk_gate u_clk_gate (", 6);
    assert_eq!(
        chain,
        vec![
            ("module_declaration".to_string(), "sram_bank".to_string()),
            (
                "loop_generate_construct".to_string(),
                "for generate".to_string()
            ),
            ("generate_block".to_string(), "g_bank".to_string()),
            (
                "if_generate_construct".to_string(),
                "if generate".to_string()
            ),
            ("generate_block".to_string(), "g_gated_clk".to_string()),
            ("module_instantiation".to_string(), "u_clk_gate".to_string()),
        ],
        "expected the full nested generate chain down to u_clk_gate, got {chain:?}"
    );
}

#[test]
fn chain_inside_the_class_constructor_in_real_soc_verif_pkg_sv() {
    let src = read_demo("verif/soc_verif_pkg.sv");
    let chain = chain_at(&src, "verilog-mode", "hits_q     = 0;", 6);
    assert_eq!(
        chain,
        vec![
            (
                "package_declaration".to_string(),
                "soc_verif_pkg".to_string()
            ),
            ("class_declaration".to_string(), "rw_checker".to_string()),
            (
                "class_constructor_declaration".to_string(),
                "new".to_string()
            ),
        ],
        "expected [package_declaration soc_verif_pkg, class_declaration \
         rw_checker, class_constructor_declaration new], got {chain:?}"
    );
}

#[test]
fn chain_inside_the_program_in_real_sram_bank_tb_sv() {
    let src = read_demo("verif/sram_bank_tb.sv");
    // "chk = new();" is directly inside the program's own top-level
    // `initial begin … end`, not inside one of its tasks, so the chain
    // stops at the program itself.
    let chain = chain_at(&src, "verilog-mode", "chk = new();", 4);
    assert_eq!(
        chain,
        vec![("program_declaration".to_string(), "axi_driver".to_string())],
        "expected [program_declaration axi_driver], got {chain:?}"
    );
}

#[test]
fn chain_inside_the_covergroup_in_real_sram_bank_tb_sv() {
    let src = read_demo("verif/sram_bank_tb.sv");
    let chain = chain_at(&src, "verilog-mode", "cp_wr_bank: coverpoint", 4);
    assert_eq!(
        chain,
        vec![
            ("module_declaration".to_string(), "sram_bank_tb".to_string()),
            (
                "covergroup_declaration".to_string(),
                "cg_axi_bank".to_string()
            ),
        ],
        "expected [module_declaration sram_bank_tb, covergroup_declaration \
         cg_axi_bank], got {chain:?}"
    );
}
