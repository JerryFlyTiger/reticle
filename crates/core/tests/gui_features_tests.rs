//! M16: renderer-level tests for the modern-UI features — line-number
//! gutter (+ diagnostic dots), hl-line, region highlight, segmented
//! modeline, themes, and the extended face model. All observable
//! through the character grid, no GUI needed.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use core::redisplay::{render, Underline};
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (50, 8);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn row_text(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let grid = render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
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
            "reticle_m16_{}_{}_{}",
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

#[test]
fn line_number_gutter_renders_and_respects_toggle() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"alpha\\nbeta\\ngamma\")");
    // Off by default: text starts at column 0.
    assert!(row_text(&i, &ed, 0).starts_with("alpha"));
    run(&mut i, "(setq display-line-numbers t)");
    let r0 = row_text(&i, &ed, 0);
    let r1 = row_text(&i, &ed, 1);
    assert!(r0.contains('1') && r0.contains("alpha"), "row0: {:?}", r0);
    assert!(r1.contains('2') && r1.contains("beta"), "row1: {:?}", r1);
    // Numbers precede the text.
    assert!(r0.find('1').unwrap() < r0.find("alpha").unwrap());
}

#[test]
fn diagnostics_show_gutter_dot_and_modeline_count() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"line one\\nline two\\n\")");
    run(&mut i, "(setq display-line-numbers t)");
    // Inject gutter data the way lsp.el does.
    run(
        &mut i,
        "(lsp--set-buffer-diagnostics (current-buffer) '((0 . 1)))",
    );
    let r0 = row_text(&i, &ed, 0);
    assert!(r0.contains('●'), "expected gutter dot on line 1: {:?}", r0);
    let r1 = row_text(&i, &ed, 1);
    assert!(!r1.contains('●'), "no dot on line 2: {:?}", r1);
    // Modeline (row = frame rows - 2) shows the count.
    let mode = row_text(&i, &ed, 6);
    assert!(mode.contains("!1"), "modeline: {:?}", mode);
}

#[test]
fn hl_line_tints_only_the_cursor_row() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char 5)"); // on line 2
    run(&mut i, "(setq hl-line-mode t)");
    let grid = render(&i, &ed);
    let bg_of = |row: usize| grid.lines[row][0].style.bg;
    assert!(bg_of(1).is_some(), "cursor row should be tinted");
    assert_eq!(bg_of(0), None, "other rows untinted");
    assert_eq!(bg_of(2), None);
}

/// M32: `hl-line-mode' now defaults to t (M16 originally shipped it
/// off) -- this is the same scenario as `hl_line_tints_only_the_cursor_
/// row' above, just with NO explicit `(setq hl-line-mode t)', pinning
/// the new default itself rather than the tinting logic (already
/// covered by the other test).
#[test]
fn hl_line_mode_is_on_by_default() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char 5)"); // on line 2
    let grid = render(&i, &ed);
    // Assert the exact hl-line face color (M112: dracula theme #363848,
    // the default), not just is_some(): a wrong face (e.g. region's)
    // must fail here (M32 review: the Option-only assertions couldn't
    // tell).
    assert_eq!(
        grid.lines[1][0].style.bg,
        Some((0x36, 0x38, 0x48)),
        "cursor row should carry the hl-line face bg with no setq at all"
    );
}

/// The everyday combination the default flip makes common: the cursor
/// row is one end of an active region, so region and hl-line meet on
/// the same row. Region must win inside the selection; hl-line covers
/// the rest of the row (M32 review finding 2).
#[test]
fn region_wins_over_hl_line_inside_the_selection_on_the_same_row() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(set-mark-command)");
    run(&mut i, "(goto-char 6)"); // region [1,6), point on the same row
    let grid = render(&i, &ed);
    assert_eq!(
        grid.lines[0][0].style.bg,
        Some((0x44, 0x47, 0x5a)),
        "inside the region: the region face (M107: dracula #44475a), not hl-line"
    );
    assert_eq!(
        grid.lines[0][8].style.bg,
        Some((0x36, 0x38, 0x48)),
        "outside the region on the cursor row: the hl-line face (M112: #363848)"
    );
}

#[test]
fn hl_line_mode_nil_disables_the_tint() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char 5)");
    run(&mut i, "(setq hl-line-mode nil)");
    let grid = render(&i, &ed);
    assert_eq!(
        grid.lines[1][0].style.bg, None,
        "explicitly disabled hl-line-mode must not tint"
    );
}

/// Existing semantics (M16, unchanged by M32's default flip): tinting
/// is gated on `is_selected', so a window that isn't the selected one
/// never gets it, even showing the very same buffer and point a
/// selected window would tint.
#[test]
fn hl_line_does_not_tint_a_non_selected_window() {
    let (mut i, ed) = setup(); // frame (50, 8)
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char 5)"); // on line 2 ("bbb")
    run(&mut i, "(split-window-below)");
    // `split-window-internal' keeps the ORIGINAL window selected (as
    // the top pane) and puts the new, unselected window below it --
    // both showing the same buffer at the same point. windows_height =
    // 8 - 1 (echo row) = 7; top pane gets ah = 7/2 = 3 rows (2 text
    // rows [0,1] + modeline [2]), bottom gets the remaining 4 (3 text
    // rows [3,4,5] + modeline [6]). Line 2 ("bbb") lands at row 1 in
    // the (selected) top pane and at row 4 in the (unselected) bottom
    // pane.
    let grid = render(&i, &ed);
    assert!(
        grid.lines[1][0].style.bg.is_some(),
        "selected (top) window's cursor row IS tinted"
    );
    assert_eq!(
        grid.lines[4][0].style.bg, None,
        "non-selected (bottom) window's own copy of the same buffer/point must NOT be tinted"
    );
}

#[test]
fn active_region_gets_the_region_background() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    // This test is about region highlighting, not hl-line; turn off
    // hl-line-mode (on by default since M32) so the "outside region
    // untinted" assertion below isn't confused by the current-line tint
    // -- point ends up on this same row (see the M25-era
    // `lsp_diagnostics_decorate_the_visiting_buffer' precedent just
    // below, which does the same for `display-line-numbers').
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(set-mark 1)");
    // Activate the mark and move point to 6: region covers "hello".
    {
        let e = ed.borrow();
        let mut b = e.current.borrow_mut();
        b.mark = Some(0);
        b.mark_active = true;
        b.point = 5;
    }
    let grid = render(&i, &ed);
    assert!(grid.lines[0][0].style.bg.is_some(), "region start tinted");
    assert!(grid.lines[0][4].style.bg.is_some(), "region end tinted");
    assert_eq!(grid.lines[0][6].style.bg, None, "outside region untinted");
}

/// M116 render-level tests: nothing before these drove `render` with
/// `show-trailing-whitespace` on and inspected the resulting grid --
/// a cold review demonstrated the hole concretely (deleting the whole
/// highlight block, or hardcoding the point-at-end-of-line exemption to
/// `false`, passed every test that existed before these). Dracula is
/// this editor's default theme (`default_theme_on_startup_is_dracula`);
/// its `trailing-whitespace` background is `#7e4f47` (themes.el).
const DRACULA_TRAILING_WS_BG: (u8, u8, u8) = (0x7e, 0x4f, 0x47);

#[test]
fn trailing_whitespace_highlights_the_run_at_end_of_line() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"foo   \\nbar\")");
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(setq-local show-trailing-whitespace t)");
    run(&mut i, "(goto-char 1)"); // line 1, not at its own end
    let grid = render(&i, &ed);
    assert_eq!(
        grid.lines[0][2].style.bg, None,
        "the 'o' itself is not trailing whitespace"
    );
    for c in 3..6 {
        assert_eq!(
            grid.lines[0][c].style.bg,
            Some(DRACULA_TRAILING_WS_BG),
            "column {c} is inside the trailing run"
        );
    }
}

#[test]
fn trailing_whitespace_point_at_line_end_is_exempted() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"foo   \\nbar\")");
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(setq-local show-trailing-whitespace t)");
    run(&mut i, "(goto-char 7)"); // char index 6 -- the line's own end (the '\n')
    let grid = render(&i, &ed);
    for c in 3..6 {
        assert_eq!(
            grid.lines[0][c].style.bg, None,
            "GNU's own exclusion: point at THIS line's end exempts it,              column {c}"
        );
    }
}

#[test]
fn trailing_whitespace_exemption_is_recomputed_on_a_later_line() {
    // The exemption is computed once before the per-character loop and then
    // recomputed at every '\n'. A test that puts point on the FIRST line
    // exercises only the initial computation, and that is what the sibling
    // test above does -- so mutating the recompute to a constant `false`
    // survived the entire suite until this test existed. Found by the
    // mutation runner, not by reading.
    let (mut i, ed) = setup();
    // chars: f0 o1 o2 \n3 b4 a5 r6 sp7 sp8 sp9 \n10 b11 a12 z13
    run(&mut i, "(insert \"foo\\nbar   \\nbaz\")");
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(setq-local show-trailing-whitespace t)");
    // point at char index 10 -- the SECOND line's own end (its '\n')
    run(&mut i, "(goto-char 11)");
    let grid = render(&i, &ed);
    for c in 3..6 {
        assert_eq!(
            grid.lines[1][c].style.bg, None,
            "point at line 2's end must exempt line 2's trailing run, column {c}"
        );
    }
}

#[test]
fn trailing_whitespace_leaves_a_clean_line_untouched() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"foo\\nbar   \")"); // trailing ws only on line 2
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(setq-local show-trailing-whitespace t)");
    run(&mut i, "(goto-char 1)");
    let grid = render(&i, &ed);
    for c in 0..3 {
        assert_eq!(
            grid.lines[0][c].style.bg, None,
            "line 1 (\"foo\") has no trailing whitespace at all, column {c}"
        );
    }
}

#[test]
fn trailing_whitespace_off_by_default_draws_nothing() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"foo   \")");
    run(&mut i, "(setq hl-line-mode nil)");
    run(&mut i, "(goto-char 1)");
    let grid = render(&i, &ed);
    for c in 0..6 {
        assert_eq!(
            grid.lines[0][c].style.bg, None,
            "show-trailing-whitespace defaults off outside prog-mode, column {c}"
        );
    }
}

/// GNU review fix (see redisplay.rs's ordering comment): trailing
/// whitespace must stay visible INSIDE an active region, not be erased
/// by it -- verified against real Emacs (`-nw -Q`) driving a region
/// across a line's own trailing run and decoding the raw ANSI. `mark`
/// is placed past the trailing run (char index 6, right before the
/// newline) and `point` at the buffer start (index 0) rather than the
/// other way around, specifically so the point-at-line-end exemption
/// does NOT fire here -- this test is about the region/trailing-ws
/// priority, not about re-testing the exemption above.
#[test]
fn trailing_whitespace_wins_over_region_where_they_overlap() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"foo   \\nbar\")");
    run(&mut i, "(setq-local show-trailing-whitespace t)");
    {
        let e = ed.borrow();
        let mut b = e.current.borrow_mut();
        b.mark = Some(6);
        b.mark_active = true;
        b.point = 0;
    }
    let grid = render(&i, &ed);
    assert_eq!(
        grid.lines[0][4].style.bg,
        Some(DRACULA_TRAILING_WS_BG),
        "trailing-whitespace must win over an active region where they          overlap -- selecting old trailing spaces further up a line one          is editing is the ordinary case, not an exotic one"
    );
}

#[test]
fn segmented_modeline_shows_name_position_and_lsp_state() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"x\")");
    let mode = row_text(&i, &ed, 6);
    assert!(mode.contains("*scratch*"), "modeline: {:?}", mode);
    assert!(mode.contains("L1:1"), "modeline: {:?}", mode);
    assert!(mode.contains('*'), "modified marker: {:?}", mode);
    assert!(!mode.contains("LSP"), "no LSP yet: {:?}", mode);
    // M63: the LSP segment reads the buffer-local `lsp--buffer-client',
    // not the old global `lsp--clients' -- setting the latter here used
    // to be enough (that was the bug: it lit the segment for every
    // buffer, not just attached ones), so this now sets the buffer-local
    // slot the mode line actually consults.
    run(&mut i, "(setq-local lsp--buffer-client 'fake)");
    let mode = row_text(&i, &ed, 6);
    assert!(mode.contains("LSP"), "modeline: {:?}", mode);

    // M63: per-buffer, not per-editor -- switching to a second buffer
    // with no client of its own must NOT show "LSP", even though the
    // first buffer (still open) has one. This is exactly what the old
    // global `lsp--clients' read got wrong (see the comment above).
    run(&mut i, "(switch-to-buffer-internal \"*no-lsp*\")");
    let mode = row_text(&i, &ed, 6);
    assert!(!mode.contains("LSP"), "no client here: {:?}", mode);
}

#[test]
fn synthetic_buffers_never_show_the_modified_marker_on_the_modeline() {
    // M71: dired / *eshell* / *ielm* / *Help* are all machine-generated
    // buffers with nothing a user could edit or lose -- `*' on their
    // mode line is pure noise. G1 (this test) checks all four don't
    // show it; the "clearing `*' still works for a REAL edit" side of
    // the contract (G2) is already covered by
    // `segmented_modeline_shows_name_position_and_lsp_state' above,
    // which asserts `*scratch*''s mode line DOES contain `*' after
    // `(insert "x")' -- without that half of the contract, a "never
    // show `*' at all" mis-fix would make this test pass too.
    let (mut i, ed) = setup();

    // The `*' modified marker (`redisplay.rs': `modified_marker = "
    // *"') is appended directly after the buffer's title with no
    // separating space of its own -- so its on-screen shape is always
    // "{title} *". A plain `mode.contains(" *")' would false-positive
    // on buffers like `*eshell*' whose NAME already starts with `*'
    // (the mode line pads a leading space before the title too, so
    // even an unmodified `*eshell*' renders " *eshell* ..." -- the
    // leading " *" there is the buffer's own name, not the marker).
    // Check for the marker's actual on-screen shape instead.
    let assert_clean = |i: &mut Interp, ed: &Rc<RefCell<Editor>>, label: &str| {
        let name = run(i, "(buffer-name)");
        let name = name.trim_matches('"');
        let mode = row_text(i, ed, 6);
        let marker = format!("{} *", name);
        assert!(
            !mode.contains(&marker),
            "{} buffer's mode line shows the modified marker: {:?}",
            label,
            mode
        );
    };

    let dir = Scratch::new("g1_modeline");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plain.txt"), "hello").unwrap();
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    // M71 fix round: the dired half of this test used to only open the
    // buffer, which exercises `dired--fill' (already covered since
    // M68) but never `dired--set-mark-char' -- the choke point THIS
    // milestone actually added. Mark the entry so the assertion below
    // touches the new code, not just the old one.
    feed_keys(&mut i, &ed, "m").unwrap();
    assert_clean(&mut i, &ed, "dired");

    run(&mut i, "(eshell)");
    assert_clean(&mut i, &ed, "*eshell*");

    run(&mut i, "(ielm)");
    assert_clean(&mut i, &ed, "*ielm*");

    run(&mut i, "(describe-bindings)");
    assert_clean(&mut i, &ed, "*Help*");
}

#[test]
fn themes_switch_the_default_face_live() {
    let (mut i, ed) = setup();
    // M107: Dracula is the default (was `dark`).
    let dracula_bg = core::redisplay::frame_base_style(&i, &ed.borrow()).bg;
    assert_eq!(
        dracula_bg,
        Some((0x28, 0x2a, 0x36)),
        "dracula theme is the default"
    );
    run(&mut i, "(load-theme 'light)");
    let light = core::redisplay::frame_base_style(&i, &ed.borrow());
    assert_eq!(light.bg, Some((0xfb, 0xfb, 0xfd)));
    assert_eq!(light.fg, Some((0x2c, 0x31, 0x3a)));
    // And back.
    run(&mut i, "(load-theme 'dracula)");
    assert_eq!(
        core::redisplay::frame_base_style(&i, &ed.borrow()).bg,
        Some((0x28, 0x2a, 0x36))
    );
    assert_eq!(run(&mut i, "current-theme"), "dracula");
}

#[test]
fn extended_face_model_parses_wave_underline_and_slant() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"squiggle here\")");
    run(
        &mut i,
        "(let ((ov (make-overlay 1 9)))
           (overlay-put ov 'face '(:underline (:style wave :color \"#f44747\") :slant italic)))",
    );
    let grid = render(&i, &ed);
    let cell = grid.lines[0][0];
    assert_eq!(cell.style.underline, Underline::Wave);
    assert_eq!(cell.style.underline_color, Some((0xf4, 0x47, 0x47)));
    assert!(cell.style.italic);
    // Past the overlay: clean.
    assert_eq!(grid.lines[0][10].style.underline, Underline::None);
}

#[test]
fn lsp_diagnostics_decorate_the_visiting_buffer() {
    let (mut i, _ed) = setup();
    // A real file-visiting buffer, decorated via the same path lsp.el
    // uses when a publishDiagnostics notification arrives.
    let dir = Scratch::new("diag");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diag.rs");
    std::fs::write(&path, "fn broken( {}\nok line\n").unwrap();
    run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    // This test is about the diagnostics squiggle, not the gutter; turn
    // off the line-number gutter that rust-mode's prog-mode-hook now
    // turns on by default (M24), so the hardcoded grid columns below
    // keep lining up with the buffer text at column 0.
    run(&mut i, "(setq-local display-line-numbers nil)");
    let uri = format!("file://{}", path.to_str().unwrap());
    let fake = format!(
        "(lsp--dispatch (make-lsp--client :conn nil) (json-parse-string
           \"{{\\\"method\\\":\\\"textDocument/publishDiagnostics\\\",\\\"params\\\":{{\\\"uri\\\":\\\"{}\\\",\\\"diagnostics\\\":[{{\\\"message\\\":\\\"expected type\\\",\\\"severity\\\":1,\\\"range\\\":{{\\\"start\\\":{{\\\"line\\\":0,\\\"character\\\":3}},\\\"end\\\":{{\\\"line\\\":0,\\\"character\\\":9}}}}}}]}}}}\"))",
        uri
    );
    let r = run(&mut i, &fake);
    assert!(!r.starts_with("ERROR"), "dispatch failed: {}", r);
    // The squiggle overlay exists over "broken" with a wave underline.
    let has_squiggle = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 (point-max)) hit)
             (when (overlay-get ov 'lsp-diag) (setq hit t))))",
    );
    assert_eq!(has_squiggle, "t");
    let grid_check = {
        let ed2 = core::editor::editor(&i);
        let grid = render(&i, &ed2);
        grid.lines[0][4].style.underline == Underline::Wave
    };
    assert!(grid_check, "wave underline should reach the grid");
}

/// `modes_tests.rs`'s own helper, copied rather than shared -- there is
/// no common test-fixture module in this project (see CLAUDE.md's test
/// conventions: "no shared fixture...each file brings its own helper").
fn find_row(grid: &core::redisplay::Grid, needle: &str) -> usize {
    (0..grid.lines.len())
        .find(|&r| {
            grid.lines[r]
                .iter()
                .filter(|c| !c.continuation)
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .contains(needle)
        })
        .unwrap_or_else(|| {
            let dump: Vec<String> = (0..grid.lines.len())
                .map(|r| {
                    grid.lines[r]
                        .iter()
                        .filter(|c| !c.continuation)
                        .map(|c| c.ch)
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect();
            panic!("no row contains {:?}; grid:\n{}", needle, dump.join("\n"))
        })
}

/// M69 I1: a narrow split window (40 cols is a realistic `C-x 3` half of
/// an 80-col terminal) used to lose `L:C' entirely when a longer RTL
/// filename was in the title -- see PLAN.md M69, symptom 1.
#[test]
fn m69_narrow_frame_keeps_line_number() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (40, 8);
    run(&mut i, "(insert \"x\")");
    run(&mut i, "(rename-buffer \"bus/axi4_lite_arbiter.sv\" t)");
    let grid = render(&i, &ed);
    let row = find_row(&grid, "L1:");
    let text: String = grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    let lc_pos = text.find("L1:").expect("L1: present");
    // M69 review F7: a single space before "L1:" only proves the gap
    // didn't hit exactly 0 -- it doesn't catch the gap collapsing to 1
    // column. `redisplay.rs`'s private `ML_GAP` const is 2; this test
    // can't see that constant from here, so the `2` below is a literal
    // that has to be kept in sync with it by hand.
    assert!(
        text[..lc_pos].ends_with("  "),
        "left and right segments must keep the full 2-column gap (ML_GAP): {:?}",
        text
    );
}

/// M69 I2/I3: a CJK directory name in the mode line's right segment
/// (dired's `default_directory') must produce a real double-width cell
/// pair -- `put_wide' plus a `continuation' cell -- not a bare `put'
/// whose declared width silently disagreed with what got drawn. `I2`
/// checks the `continuation' flag directly (not `row_text', which
/// filters continuation cells out and would hide exactly the bug this
/// milestone fixes); `I3` checks the column-count consequence of the
/// same bug (a row that's short or long by however many wide chars got
/// mis-drawn).
#[test]
fn m69_dired_cjk_dir_right_segment_wide_and_column_exact() {
    let dir = Scratch::new("m69_cjk");
    let cjk = dir.join("專案");
    std::fs::create_dir_all(&cjk).unwrap();
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (100, 8);
    run(&mut i, &format!("(dired {:?})", cjk.to_str().unwrap()));
    let grid = render(&i, &ed);
    let row = find_row(&grid, "dired-mode");

    // I2: find the CJK char's cell and confirm the next cell is a
    // continuation cell.
    let cjk_char = '專';
    let col = (0..grid.cols)
        .find(|&c| grid.lines[row][c].ch == cjk_char)
        .unwrap_or_else(|| {
            panic!("no cell with {:?} in mode line row: {:?}", cjk_char, {
                grid.lines[row].iter().map(|c| c.ch).collect::<String>()
            })
        });
    assert!(
        grid.lines[row][col + 1].continuation,
        "cell after the CJK char must be marked continuation"
    );

    // I3: total display columns of non-continuation cells in the row
    // must equal the frame width -- no drift from an under- or
    // over-counted wide char.
    let total: usize = grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| {
            unicode_width::UnicodeWidthChar::width(c.ch)
                .unwrap_or(1)
                .max(1)
        })
        .sum();
    assert_eq!(
        total, grid.cols,
        "row must account for exactly frame-width columns"
    );
}

/// Fix 2 (mouse-support milestone review): `fill_chrome_runs` used to
/// `continue` straight past any `Cell::continuation` cell -- the second
/// half of a double-width character -- without folding it into the
/// still-open run's `cols`. For buffer text `RunBuilder::push` already
/// keeps `cols` separate from char count (see its doc); chrome painting
/// (the mode line here) drew a wide CJK glyph directly into `grid.lines`
/// with no `RunBuilder` involved at all, so its continuation cell was
/// claimed by *no* run -- a one-column gap in `grid.runs` for that row.
/// Reuses the exact CJK-dired-mode-line scenario `m69_dired_cjk_dir_
/// right_segment_wide_and_column_exact` above already exercises for
/// `grid.lines`, but asserts the stronger, general invariant on
/// `grid.runs` instead: runs on a row must tile every column from 0 to
/// `grid.cols` exactly -- no gaps, no overlaps -- which is what the next
/// milestone (reconstructing each row from `grid.runs` to shape/draw it)
/// depends on.
#[test]
fn chrome_runs_cover_every_column_of_a_row_with_a_wide_char() {
    let dir = Scratch::new("fix2_cjk");
    let cjk = dir.join("專案");
    std::fs::create_dir_all(&cjk).unwrap();
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (100, 8);
    run(&mut i, &format!("(dired {:?})", cjk.to_str().unwrap()));
    let grid = render(&i, &ed);
    let row = find_row(&grid, "dired-mode");

    let mut row_runs: Vec<&core::redisplay::PaintRun> =
        grid.runs.iter().filter(|r| r.row == row).collect();
    row_runs.sort_by_key(|r| r.col);
    assert!(!row_runs.is_empty(), "mode line row must have chrome runs");

    let mut next_col = 0usize;
    for r in &row_runs {
        assert_eq!(
            r.col, next_col,
            "gap or overlap in row {row}'s runs before col {next_col}: {row_runs:#?}"
        );
        next_col += r.cols;
    }
    assert_eq!(
        next_col, grid.cols,
        "runs on row {row} must tile all {} columns, covered only {next_col}: {row_runs:#?}",
        grid.cols
    );
}

/// M-visual-quality: `severity_color` must resolve the diagnostic dot's
/// color from the `diagnostic-error` face rather than a hard-coded
/// tuple. Diagnostics are set via `lsp--set-buffer-diagnostics`, the
/// same elisp entry point `diagnostics_show_gutter_dot_and_modeline_
/// count' above uses, rather than the full LSP dispatch plumbing
/// `lsp_diagnostics_decorate_the_visiting_buffer' exercises -- this
/// test is only about the color.
#[test]
fn severity_color_resolves_from_the_diagnostic_error_face() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"line one\\n\")");
    run(&mut i, "(setq display-line-numbers t)");
    run(
        &mut i,
        "(set-face 'diagnostic-error :foreground \"#112233\")",
    );
    run(
        &mut i,
        "(lsp--set-buffer-diagnostics (current-buffer) '((0 . 1)))",
    );
    let grid = render(&i, &ed);
    let dot_col = (0..grid.cols)
        .find(|&c| grid.lines[0][c].ch == '●')
        .expect("diagnostic dot should be drawn in the gutter");
    assert_eq!(
        grid.lines[0][dot_col].style.fg,
        Some((0x11, 0x22, 0x33)),
        "dot color must come from the diagnostic-error face"
    );
}

/// Same setup as above, but without any custom `set-face` override --
/// switching between the built-in dark and light themes must itself
/// change the dot's color, proving it's theme-driven and not still a
/// build-time constant that merely happens to be readable through
/// `face_or`.
#[test]
fn severity_color_differs_between_dark_and_light_themes() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"line one\\n\")");
    run(&mut i, "(setq display-line-numbers t)");
    run(
        &mut i,
        "(lsp--set-buffer-diagnostics (current-buffer) '((0 . 1)))",
    );
    let dark_grid = render(&i, &ed);
    let dark_col = (0..dark_grid.cols)
        .find(|&c| dark_grid.lines[0][c].ch == '●')
        .expect("dot present under dark theme");
    let dark_color = dark_grid.lines[0][dark_col].style.fg;

    run(&mut i, "(load-theme 'light)");
    let light_grid = render(&i, &ed);
    let light_col = (0..light_grid.cols)
        .find(|&c| light_grid.lines[0][c].ch == '●')
        .expect("dot present under light theme");
    let light_color = light_grid.lines[0][light_col].style.fg;

    assert!(dark_color.is_some());
    assert!(light_color.is_some());
    assert_ne!(
        dark_color, light_color,
        "diagnostic dot color must change with the theme"
    );
}

/// M-visual-quality: the echo row picks up the `echo-area` face instead
/// of the old `Style::default()`.
#[test]
fn echo_row_picks_up_the_echo_area_face() {
    let (mut i, ed) = setup();
    run(&mut i, "(set-face 'echo-area :foreground \"#445566\")");
    run(&mut i, "(message \"hello there\")");
    let grid = render(&i, &ed);
    let echo_row = grid.lines.len() - 1;
    let col = (0..grid.cols)
        .find(|&c| grid.lines[echo_row][c].ch == 'h')
        .expect("message text should be drawn in the echo row");
    assert_eq!(
        grid.lines[echo_row][col].style.fg,
        Some((0x44, 0x55, 0x66)),
        "echo row must carry the echo-area face's foreground"
    );
}

// M112 review: the two theme-parsing tests below (`every_theme_defines_
// the_same_set_of_faces` and `theme_chrome_faces_clear_the_visibility_
// bar`) used to each carry their own copy of "find one theme's body in
// themes.el's source", and the shared idiom cut a theme's body at the
// next literal `"\n(defun "` rather than at that theme's own closing
// paren -- for `theme--light` that ran roughly 3,300 characters into
// `theme--dracula`'s header comment, which is to say clean through the
// whole of it, since no other `"\n(defun "` occurs before its end. (An
// earlier version of this comment said "about 370", a dropped digit
// caught by cold reading -- measured by replaying the removed logic
// against the file's actual bytes.) Harmless only because no comment
// in the file happens to contain `(set-face '...)`-shaped text.
// Factored into one paren-matching helper, shared by both tests plus
// the org-face-color test below, that stops at the defun's own actual
// end.

/// Discover every `(defun theme--NAME (...)` top-level form in
/// `themes.el`'s source, instead of a hard-coded list of names -- so a
/// theme added later is picked up automatically rather than silently
/// skipped by every test that uses this.
fn theme_defun_names(src: &str) -> Vec<String> {
    src.match_indices("(defun theme--")
        .map(|(i, m)| {
            let rest = &src[i + m.len()..];
            let end = rest.find(' ').unwrap_or(rest.len());
            format!("theme--{}", &rest[..end])
        })
        .collect()
}

/// The exact source text of one `(defun theme--NAME (...) ...)` form,
/// from its own opening paren to its own matching closing paren --
/// found by tracking paren depth (skipping `;`-to-end-of-line comments
/// and `"..."` string contents, neither of which can hide a real paren
/// in this file: no hex color or docstring here contains one), not by
/// guessing where the next theme's defun starts.
fn theme_defun_body<'a>(src: &'a str, defun: &str) -> &'a str {
    let needle = format!("(defun {defun} (");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("{defun} not found in themes.el"));
    let bytes = src.as_bytes();
    let mut i = start;
    let mut depth: i32 = 0;
    let mut in_string = false;
    loop {
        assert!(
            i < bytes.len(),
            "{defun}: ran off the end of the file with unbalanced parens"
        );
        let c = bytes[i];
        if in_string {
            if c == b'"' {
                in_string = false;
            }
        } else {
            match c {
                b';' => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'"' => in_string = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[start..=i];
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
}

/// Every `(set-face 'FACE ...)` name defined directly inside one
/// theme's body.
fn faces_set_by(body: &str) -> std::collections::BTreeSet<String> {
    body.match_indices("(set-face '")
        .map(|(i, m)| {
            let rest = &body[i + m.len()..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')')
                .unwrap_or(rest.len());
            rest[..end].to_string()
        })
        .collect()
}

/// Pull the value of one `:foreground`/`:background` keyword off one
/// theme's `(set-face 'FACE ...)` call. `indent-guide`/`scroll-bar` are
/// stored under `:foreground` in this codebase even though they're
/// rendered as a fill color, not text -- callers pass whichever keyword
/// each face actually uses.
fn face_color(body: &str, face: &str, keyword: &str) -> String {
    let start = body
        .find(&format!("(set-face '{face} "))
        .unwrap_or_else(|| panic!("(set-face '{face} ...) not found"));
    let rest = &body[start..];
    let end = rest
        .find(')')
        .unwrap_or_else(|| panic!("no closing paren for (set-face '{face} ...)"));
    let call = &rest[..end];
    let kw_start = call
        .find(&format!(":{keyword} \""))
        .unwrap_or_else(|| panic!("{face} has no :{keyword} in {call}"))
        + keyword.len()
        + 3;
    let kw_rest = &call[kw_start..];
    let kw_end = kw_rest.find('"').unwrap();
    kw_rest[..kw_end].to_string()
}

/// Every `#rrggbb` hex literal appearing anywhere in a theme's body,
/// lower-cased. Used to check that a face's color is drawn from the
/// theme's own palette rather than fat-fingered or copy-pasted from a
/// different theme.
/// Every `#RRGGBB` literal in `body`, lower-cased.
///
/// Two things this deliberately does NOT do, both found by cold reading
/// the first version:
///
/// * It does not slice a fixed seven bytes off `body` at each `#`. That
///   panics with "byte index is not a char boundary" the day someone
///   puts a multi-byte character within six bytes after a `#` in a
///   theme comment. There is exactly one non-ASCII character in
///   `themes.el` today and it happens to sit outside every theme body,
///   which is not a property worth relying on.
/// * It does not accept a run of more than six hex digits. An
///   eight-digit `#RRGGBBAA` appears in `theme--dracula`'s indent-guide
///   comment (`#FFFFFF1A`, Dracula's published white-at-10% guide
///   colour), and taking its leading six would have inserted a
///   `#ffffff` into the derived palette that no face actually uses --
///   inert today, but a false-pass waiting for a fat-fingered face
///   colour whose value happens to match some comment's leading six.
fn all_hex_colors_in(body: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for (start, _) in body.match_indices('#') {
        let run: String = body[start + 1..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if run.len() == 6 {
            out.insert(format!("#{}", run.to_ascii_lowercase()));
        }
    }
    out
}

/// M107: every built-in theme (`theme--dark`, `theme--light`,
/// `theme--dracula`, `theme--xcode`, `theme--vscode`, and any future
/// `theme--*` defun) must define the exact same set of face names --
/// catches a future edit that adds a face to one theme's function and
/// forgets another, silently leaving that face unstyled whenever the
/// forgotten theme is active. This supersedes the old M-visual-quality
/// version, which hard-coded just `theme--dark` and `theme--light` by
/// name -- M107 added three more theme functions that version would
/// never have looked at, so a face dropped from any of them would have
/// gone undetected.
#[test]
fn every_theme_defines_the_same_set_of_faces() {
    // This reads the source rather than the running face table, and that
    // is not squeamishness -- the obvious runtime version of this test is
    // vacuous. `themes.el` calls `(load-theme 'dracula)` at eval time, so
    // by the time a test can call `(load-theme 'light)` the face table
    // already holds everything the dracula theme set. Loading another
    // theme on top only ever adds or overwrites, never removes, so any
    // later theme's face set is unconditionally a superset of every
    // theme loaded before it, and deleting a face from one theme's own
    // defun is invisible. Caught by mutation C3 in dev/mutations/m86.py,
    // which survived the earlier runtime version of this test.
    let src = include_str!("../lisp/themes.el");

    let theme_names = theme_defun_names(src);
    assert!(
        theme_names.len() >= 5,
        "expected at least 5 theme--* defuns (dark, light, dracula, xcode, \
         vscode), the parser found only {:?}",
        theme_names
    );

    let face_sets: Vec<(String, std::collections::BTreeSet<String>)> = theme_names
        .iter()
        .map(|name| (name.clone(), faces_set_by(theme_defun_body(src, name))))
        .collect();

    for (name, faces) in &face_sets {
        assert!(
            !faces.is_empty(),
            "parsed no faces at all for {name} -- the parser, not the theme, is broken"
        );
    }

    let (reference_name, reference_faces) = &face_sets[0];
    for (name, faces) in &face_sets[1..] {
        let missing: Vec<_> = reference_faces.difference(faces).collect();
        let extra: Vec<_> = faces.difference(reference_faces).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "face sets differ between {reference_name} and {name}: \
             missing-from-{name}={missing:?} extra-in-{name}={extra:?}"
        );
    }

    // A floor, because comparing the themes only against EACH OTHER is
    // blind in one direction: a face missing from all five is perfectly
    // consistent, and this test would report nothing.
    //
    // That is not hypothetical. A cold reviewer named it as a theoretical
    // gap during M112; during M116 it happened. Two new faces were added
    // to the Rust side and to no theme at all, so they silently fell back
    // to hard-coded defaults and stopped following the active theme --
    // exactly the `org-*` defect M112 had just finished fixing -- and this
    // test stayed green throughout, because 57 == 57 == 57 == 57 == 57.
    //
    // Same lesson as `dev/test_pixdiff.py`'s MINIMUM_TESTS floor: a check
    // that decides what to compare has to fail loudly when it decides to
    // compare nothing. Raise this number when faces are added; lower it
    // only in a commit that says which face went away and why.
    // M118: +1 for `scope-header` (sticky scope header / breadcrumb).
    const MINIMUM_FACES: usize = 60;
    for (name, faces) in &face_sets {
        assert!(
            faces.len() >= MINIMUM_FACES,
            "{name} defines {} faces, expected at least {MINIMUM_FACES}. Either a \
             face was deliberately removed (lower the floor in a commit that says \
             which and why) or one was added to the Rust side and to no theme, \
             which the set comparison above cannot see because it only checks the \
             themes against each other",
            faces.len()
        );
    }
}

// WCAG 2.x relative luminance / contrast ratio -- see
// https://www.w3.org/TR/WCAG21/#dfn-relative-luminance. No such helper
// existed anywhere in the tree; this is a pure function, so per this
// project's testing conventions it belongs here, not in product code.
//
// The formula is exactly WCAG's, but the numeric bands the test below
// checks against (roughly 1.20-1.45) are NOT a WCAG conformance claim:
// WCAG's own threshold for non-text UI components is 3:1, well above
// every value here. This project's editor chrome is deliberately quiet
// -- these bands are this project's own design targets, using the WCAG
// formula only as a consistent ruler for "how far a color sits from
// the background", not as a pass/fail accessibility check.
fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let c = c as f64 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let hex = hex.trim_start_matches('#');
    (
        u8::from_str_radix(&hex[0..2], 16).unwrap(),
        u8::from_str_radix(&hex[2..4], 16).unwrap(),
        u8::from_str_radix(&hex[4..6], 16).unwrap(),
    )
}

fn contrast_ratio(a: &str, b: &str) -> f64 {
    let (l1, l2) = (
        relative_luminance(hex_to_rgb(a)),
        relative_luminance(hex_to_rgb(b)),
    );
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// M112: the chrome-visibility fix. `hl-line`, `indent-guide`,
/// `scroll-bar`, `mode-line`, and `mode-line-inactive` used to be set so
/// close to each theme's own background (measured from a screenshot:
/// contrast ratios of 1.03-1.16, several within three RGB units per
/// channel of the background) that they were invisible in practice even
/// though the drawing code ran every frame. This computes the same
/// relative-luminance contrast ratio the milestone was scoped from
/// (see `relative_luminance' above for what these numbers do and don't
/// claim) and checks every theme landed in the target bands, reading
/// colors straight out of themes.el's source the same way
/// `every_theme_defines_the_same_set_of_faces' above does (parsing, not
/// the live face table, for the same reason that test gives: loading a
/// second theme only ever adds/overwrites the runtime table, so a
/// runtime read can't tell a theme's own value from one left over from
/// `load-theme''s startup call).
/// `themes.el` states, per theme, that `completions-selected` and
/// `panel-selected` reuse `region`'s own background, so that a chosen
/// completion row and an active selection read as the same "this one"
/// affordance. That was true in all five themes and asserted nowhere.
///
/// It nearly stopped being true: M112's fix round raised `light`'s
/// `region` substantially (it was capping how strong that theme's
/// indent guide and scrollbar could be), and these two faces had to
/// move with it. A cold reviewer named this as the one value in that
/// diff for which no mutation could be designed, because no test read
/// it. This is that test.
/// `show-paren-match` must stay visually apart from the two other
/// surfaces that can sit under point: `region` and `hl-line`.
///
/// This exists because the claim was twice made in prose and twice
/// wrong. M113 shipped a 20% blend of each theme's own
/// `diagnostic-warning` into its background; a cold read measured
/// Dracula's at **1.011** against `region` -- the closest two faces this
/// project shipped anywhere. The fix round re-derived Dracula and then
/// asserted, again without measuring, that the other four "already clear
/// a reasonable margin". A second cold read measured them: `light` was
/// **1.051** against `hl-line`, which is worse in practice than the
/// Dracula case it followed, because `hl-line-mode` is on by default and
/// point is nearly always on the current line.
///
/// So the bar is 1.30 against both, and it is checked here rather than
/// claimed in a comment. The blend fraction differs per theme as a
/// result -- it is whatever that theme's own warning hue needs.
#[test]
fn show_paren_match_separates_from_region_and_hl_line() {
    let src = include_str!("../lisp/themes.el");
    const BAR: f64 = 1.30;

    for name in [
        "theme--dark",
        "theme--light",
        "theme--dracula",
        "theme--xcode",
        "theme--vscode",
    ] {
        let body = theme_defun_body(src, name);
        let paren = face_color(body, "show-paren-match", "background");
        for other in ["region", "hl-line"] {
            let c = face_color(body, other, "background");
            let r = contrast_ratio(&paren, &c);
            assert!(
                r >= BAR,
                "{name}: show-paren-match {paren} is only {r:.3} from {other} {c}; \
                 the bar is {BAR} and a highlight that vanishes into the \
                 selection or the current line is not a highlight"
            );
        }
    }
}

/// M116 sibling of `show_paren_match_separates_from_region_and_hl_line`,
/// same bar and same reasoning: `trailing-whitespace' is a genuine grid
/// cell background (`redisplay.rs` sets `style.bg` in the per-character
/// loop, same mechanism `show-paren-match'/`region' use), and it can sit
/// on point's own current line -- wherever point is not at that line's
/// end, per `show-trailing-whitespace''s own exclusion rule -- at the
/// same time `hl-line-mode' tints the rest of that row, and inside an
/// active selection at the same time as `region' (both of which fully
/// overwrite it where they overlap, exactly the way `show-paren-match'
/// is overwritten by `region' -- see redisplay.rs's ordering comments).
/// Without this bar, a theme could pick a trailing-whitespace color that
/// reads identically to the current-line tint it usually sits inside,
/// making the "look here, verible will reject this" signal invisible in
/// exactly the situation (typing on a line with old trailing spaces
/// further up it) it exists to catch.
///
/// `fill-column-indicator' is deliberately NOT covered by this test: it
/// is never a grid-cell background at all -- `frontend-gui/src/lib.rs`
/// paints it as a standalone pixel rectangle over the finished frame,
/// the same mechanism `indent-guide' uses (see that block's own
/// comment), not a `Style::bg' a per-character loop can be racing
/// `region'/`hl-line'/`show-paren-match' to set. Its own visual-overlap
/// concern (colliding with `indent-guide', not with `region'/`hl-line')
/// is checked below as a separate `vs-indent-guide' comment recorded
/// next to each theme's `fill-column-indicator' definition, not by a
/// test in this file, because -- unlike a `Style::bg' collision, which
/// is a per-cell either/or `redisplay.rs` resolves at render time -- an
/// indent guide and the fill-column ruler landing on the same column is
/// a per-buffer-content coincidence with no fixed resolution to assert
/// on beyond "paint order picks a winner", which the `lib.rs` comment
/// at that block already states plainly.
#[test]
fn trailing_whitespace_separates_from_region_and_hl_line() {
    let src = include_str!("../lisp/themes.el");
    const BAR: f64 = 1.30;

    for name in [
        "theme--dark",
        "theme--light",
        "theme--dracula",
        "theme--xcode",
        "theme--vscode",
    ] {
        let body = theme_defun_body(src, name);
        let tw = face_color(body, "trailing-whitespace", "background");
        for other in ["region", "hl-line"] {
            let c = face_color(body, other, "background");
            let r = contrast_ratio(&tw, &c);
            assert!(
                r >= BAR,
                "{name}: trailing-whitespace {tw} is only {r:.3} from {other} {c}; \
                 the bar is {BAR} and a highlight that vanishes into the \
                 selection or the current line is not a highlight"
            );
        }
    }
}

/// M116 review fix: `fill-column-indicator` is a pixel overlay, not a
/// grid-cell background (see the comment on the test above for why it's
/// excluded from THAT one), but it is painted immediately after
/// `indent-guide` in `frontend-gui/src/lib.rs` and both are the exact
/// same kind of overlay (a thin rule filled from a `:foreground` value)
/// -- so a future theme edit could still make the ruler vanish into the
/// guides on any column both land on. Same bar and formula as every
/// other separation test in this file.
#[test]
fn fill_column_indicator_separates_from_indent_guide() {
    let src = include_str!("../lisp/themes.el");
    const BAR: f64 = 1.30;

    for name in [
        "theme--dark",
        "theme--light",
        "theme--dracula",
        "theme--xcode",
        "theme--vscode",
    ] {
        let body = theme_defun_body(src, name);
        let fc = face_color(body, "fill-column-indicator", "foreground");
        let ig = face_color(body, "indent-guide", "foreground");
        let r = contrast_ratio(&fc, &ig);
        assert!(
            r >= BAR,
            "{name}: fill-column-indicator {fc} is only {r:.3} from indent-guide              {ig}; the bar is {BAR} and a ruler that vanishes into the indent              guides on the columns where they coincide is not a ruler"
        );
    }
}

#[test]
fn selected_row_backgrounds_mirror_the_selection() {
    let src = include_str!("../lisp/themes.el");

    for name in [
        "theme--dark",
        "theme--light",
        "theme--dracula",
        "theme--xcode",
        "theme--vscode",
    ] {
        let body = theme_defun_body(src, name);
        let region = face_color(body, "region", "background");
        for face in ["completions-selected", "panel-selected"] {
            let got = face_color(body, face, "background");
            assert_eq!(
                got, region,
                "{name}: {face}'s background {got} must mirror region's {region} -- \
                 a selected row and an active selection are the same affordance, \
                 and themes.el says so in prose next to both"
            );
        }
    }
}

#[test]
fn theme_chrome_faces_clear_the_visibility_bar() {
    let src = include_str!("../lisp/themes.el");

    for name in [
        "theme--dark",
        "theme--light",
        "theme--dracula",
        "theme--xcode",
        "theme--vscode",
    ] {
        let body = theme_defun_body(src, name);
        let bg = face_color(body, "default", "background");
        let region = face_color(body, "region", "background");
        let hl_line = face_color(body, "hl-line", "background");
        let indent_guide = face_color(body, "indent-guide", "foreground");
        let scroll_bar = face_color(body, "scroll-bar", "foreground");
        let mode_line = face_color(body, "mode-line", "background");
        let mode_line_inactive = face_color(body, "mode-line-inactive", "background");

        let r_region = contrast_ratio(&region, &bg);
        let r_hl_line = contrast_ratio(&hl_line, &bg);
        let r_indent_guide = contrast_ratio(&indent_guide, &bg);
        let r_scroll_bar = contrast_ratio(&scroll_bar, &bg);
        let r_mode_line = contrast_ratio(&mode_line, &bg);
        let r_mode_line_inactive = contrast_ratio(&mode_line_inactive, &bg);

        // hl-line must never read as loud as an actual selection, in
        // every theme without exception -- this is a correctness rule,
        // not just a visibility target.
        assert!(
            r_hl_line < r_region,
            "{name}: hl-line ({r_hl_line:.3}) must be strictly weaker than region ({r_region:.3})"
        );

        // theme--xcode's hl-line is excluded from the visibility band on
        // purpose: it's disclosed in themes.el as copied verbatim from
        // Xcode's own .xccolortheme Current Line color (verified against
        // the installed application, not just trusted -- see that
        // face's own comment in themes.el), and M112's own scope rules
        // forbid overriding a value marked as upstream-sourced.
        if name != "theme--xcode" {
            assert!(
                (1.20..=1.35).contains(&r_hl_line),
                "{name}: hl-line ratio {r_hl_line:.3} outside the 1.20-1.35 target band"
            );
        }

        // indent-guide: this used to carry a theme--light-specific
        // carve-out here, because that theme's own `region' was only
        // ~1.30 (far weaker than every other theme's), capping how
        // strong indent-guide/scroll-bar could get below it. That was
        // fixed at the source (`region' itself raised to ~1.73, see its
        // own comment in themes.el) rather than special-cased in this
        // test, so every theme now shares one band with no exception.
        assert!(
            (1.30..=1.45).contains(&r_indent_guide),
            "{name}: indent-guide ratio {r_indent_guide:.3} outside the 1.30-1.45 target band"
        );

        // scroll-bar: at least as visible as indent-guide, no louder
        // than the selection -- regardless of which of indent-guide's
        // and region's ratios happens to be larger in this theme.
        let (lo, hi) = if r_indent_guide < r_region {
            (r_indent_guide, r_region)
        } else {
            (r_region, r_indent_guide)
        };
        assert!(
            r_scroll_bar >= lo - 0.001 && r_scroll_bar <= hi + 0.001,
            "{name}: scroll-bar ratio {r_scroll_bar:.3} not between indent-guide's \
             {r_indent_guide:.3} and region's {r_region:.3}"
        );

        // mode-line is a distinct chrome surface in every theme.
        assert!(
            r_mode_line >= 1.20,
            "{name}: mode-line ratio {r_mode_line:.3} below the 1.20 chrome-visibility bar"
        );

        // mode-line-inactive is also a distinct surface (M112 review:
        // this face is a named target of the milestone and was
        // previously untested here at all), but it must read as LESS
        // prominent than the active mode-line, with a real margin --
        // an inactive window's status line outshining the active one's
        // is backwards. A first attempt at `theme--dark' got this
        // exactly backwards by 0.0007 while its own comment claimed the
        // opposite; 0.02 is comfortably larger than that kind of
        // rounding-distance mistake.
        assert!(
            (1.05..=1.30).contains(&r_mode_line_inactive),
            "{name}: mode-line-inactive ratio {r_mode_line_inactive:.3} outside the 1.05-1.30 band"
        );
        assert!(
            r_mode_line - r_mode_line_inactive >= 0.02,
            "{name}: mode-line-inactive ({r_mode_line_inactive:.3}) must be visibly weaker than \
             mode-line ({r_mode_line:.3}), margin was only {:.4}",
            r_mode_line - r_mode_line_inactive
        );
    }
}

/// M112 review: nothing checked org-face *colors*, only their names
/// (`every_theme_defines_the_same_set_of_faces' above only proves the
/// nine org faces exist in every theme, not that their values are
/// sane) -- a fat-fingered hex or a color accidentally copy-pasted from
/// a different theme's org section would pass every other check here.
/// Each theme's nine org faces are documented as reusing that same
/// theme's own existing palette (see e.g. `theme--dark''s org-faces
/// comment), so this asserts exactly that property: every org face's
/// color also appears somewhere else in that same theme's body.
#[test]
fn org_face_colors_are_drawn_from_their_own_theme() {
    let src = include_str!("../lisp/themes.el");
    let org_faces = [
        "org-level-1",
        "org-level-2",
        "org-level-3",
        "org-level-4",
        "org-todo",
        "org-done",
        "org-table",
        "org-date",
        "org-link",
    ];

    for name in theme_defun_names(src) {
        let body = theme_defun_body(src, &name);
        let palette = all_hex_colors_in(body);
        for face in org_faces {
            let color = face_color(body, face, "foreground");
            // Every org face's color must occur at least twice in the
            // body: once on its own `org-*' line, and at least once
            // more on some other face's line -- i.e. it is a reuse of
            // an existing role color, not a one-off value unique to
            // the org face (which is exactly what a fat-fingered or
            // cross-theme-copied hex would look like).
            let occurrences = body.matches(&color).count();
            assert!(
                palette.contains(&color) && occurrences >= 2,
                "{name}: {face}'s color {color} does not reappear elsewhere in this theme's own \
                 body (found {occurrences} occurrence(s)) -- looks fat-fingered or copied from a \
                 different theme"
            );
        }
    }
}

/// GUI fix 2: `Grid::windows` is the layout metadata the GUI reads to
/// stop assuming a single, full-height, ungutlered window (its indent
/// guides, hairline separators, and scrollbar all used to hard-code that
/// shape). Split setup and row math mirror
/// `hl_line_does_not_tint_a_non_selected_window` above, which already
/// worked this arithmetic out: frame (50, 8), windows_height = 7,
/// `split-window-below` gives the top pane height 3 (text rows 0-1,
/// mode line at row 2) and the bottom pane height 4 (text rows 3-5, mode
/// line at row 6).
#[test]
fn published_window_layout_matches_a_split_frame() {
    let (mut i, ed) = setup(); // frame (50, 8)
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(split-window-below)");
    let grid = render(&i, &ed);

    assert_eq!(grid.windows.len(), 2, "one entry per window pane");

    let top = grid
        .windows
        .iter()
        .find(|w| w.row == 0)
        .expect("top pane's entry");
    assert_eq!(top.col, 0);
    assert_eq!(top.rows, 3);
    assert_eq!(top.cols, 50);
    assert_eq!(top.mode_line_row, 2, "top pane's mode line is grid row 2");

    let bottom = grid
        .windows
        .iter()
        .find(|w| w.row == 3)
        .expect("bottom pane's entry");
    assert_eq!(bottom.col, 0);
    assert_eq!(bottom.rows, 4);
    assert_eq!(bottom.cols, 50);
    assert_eq!(
        bottom.mode_line_row, 6,
        "bottom pane's mode line is grid row 6"
    );

    assert_ne!(
        top.win_id, bottom.win_id,
        "the two panes must be distinguishable by window id"
    );
}

/// Fix B (trailing review): `published_window_layout_matches_a_split_frame`
/// above never turns on `display-line-numbers`, so `gutter_cols` is 0 in
/// both windows it checks and no assertion ever reads that field --
/// replacing `gutter_cols: gutter_w` with `gutter_cols: 0` in
/// `render_window` passed that test outright. This turns the gutter on and
/// checks `gutter_cols` against the grid itself: the column where each
/// window's text actually starts (found by locating the known, unindented
/// text `"aaa"`/`"ddd"` in each pane's first row), not a hard-coded
/// constant that could drift with the same bug the field exists to guard
/// against.
#[test]
fn published_window_layout_reports_the_real_gutter_width() {
    let (mut i, ed) = setup(); // frame (50, 8)
    run(&mut i, "(setq display-line-numbers t)");
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    // Point back to the top of the buffer so both panes' `window_start`
    // scrolls to show "aaa" on their first row -- `insert` above leaves
    // point at the end, and with only 2 text rows per pane (frame (50, 8)
    // split in two), `ensure_point_visible` would otherwise scroll each
    // pane down to keep point (the last line) on screen.
    run(&mut i, "(goto-char (point-min))");
    // Both panes show the same buffer (split-window-below doesn't clone
    // it), so both display the same "aaa\nbbb\nccc" text and get the same
    // digit-width gutter -- this still exercises two separately published
    // `WindowLayout` entries, at two different `rect`s, each computing its
    // own `gutter_w`.
    run(&mut i, "(split-window-below)");
    let grid = render(&i, &ed);

    assert_eq!(grid.windows.len(), 2, "one entry per window pane");

    let top = grid
        .windows
        .iter()
        .find(|w| w.row == 0)
        .expect("top pane's entry");
    let top_row: String = grid.lines[top.row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    let top_text_col = top_row
        .find('a')
        .expect("top pane's text ('aaa') must be on its first row");
    assert!(
        top.gutter_cols > 0,
        "line numbers are on: gutter must be nonzero"
    );
    assert_eq!(
        top.gutter_cols, top_text_col,
        "gutter_cols must match where the top pane's text actually starts in the grid"
    );

    let bottom = grid
        .windows
        .iter()
        .find(|w| w.row == 3)
        .expect("bottom pane's entry");
    let bottom_row: String = grid.lines[bottom.row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    let bottom_text_col = bottom_row
        .find('a')
        .expect("bottom pane's text ('aaa') must be on its first row");
    assert!(
        bottom.gutter_cols > 0,
        "line numbers are on: gutter must be nonzero"
    );
    assert_eq!(
        bottom.gutter_cols, bottom_text_col,
        "gutter_cols must match where the bottom pane's text actually starts in the grid"
    );
}

// ---------------------------------------------------------------------
// Mouse-support milestone, task 1: `Grid::runs` / `Grid::buffer_pos_at`.
// ---------------------------------------------------------------------

/// The selected (only) window's text-area origin in grid coordinates:
/// `(row, col)` of that window's first text row/column, using
/// `grid.windows` rather than assuming `(0, 0)` -- true for every test
/// below since `setup()`'s frame is unsplit and line numbers are off by
/// default, but stated explicitly so a future edit to this helper can't
/// silently drift from what the layout actually is.
fn text_origin(grid: &core::redisplay::Grid) -> (usize, usize) {
    let w = &grid.windows[0];
    (w.row, w.col + w.gutter_cols)
}

#[test]
fn buffer_pos_at_maps_plain_text_click_to_byte() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello\\nworld\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // "hello\nworld\n": h=0 e=1 l=2 l=3 o=4 \n=5 w=6 o=7 r=8 l=9 d=10 \n=11
    assert_eq!(grid.buffer_pos_at(row0, col0 + 2), Some(2));
    assert_eq!(grid.buffer_pos_at(row0 + 1, col0), Some(6));
}

#[test]
fn buffer_pos_at_past_end_of_line_clamps_to_line_end() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hi\\nworld\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // "hi" is 2 bytes (0..2); clicking well past it on the same row must
    // land at 2 (right before the '\n'), not spill onto row 1's 'w'.
    assert_eq!(grid.buffer_pos_at(row0, col0 + 20), Some(2));
}

#[test]
fn buffer_pos_at_empty_line_maps_to_its_own_start() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"a\\n\\nb\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // "a\n\nb\n": a=0 \n=1 (blank line starts at 2) \n=2 b=3 \n=4
    assert_eq!(grid.buffer_pos_at(row0 + 1, col0), Some(2));
}

#[test]
fn buffer_pos_at_tab_maps_every_column_to_the_tab_byte() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"\\tX\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // A tab at column 0 expands to display columns 0..8; 'X' follows at
    // byte 1. Every column inside the tab's span must map to byte 0 --
    // the naive proportional mapping the milestone spec warns about
    // would instead walk off past the tab's single source byte.
    assert_eq!(grid.buffer_pos_at(row0, col0), Some(0));
    assert_eq!(grid.buffer_pos_at(row0, col0 + 5), Some(0));
    assert_eq!(grid.buffer_pos_at(row0, col0 + 8), Some(1));
}

#[test]
fn buffer_pos_at_wide_char_continuation_maps_to_char_start() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"\u{754c}X\\n\")"); // 界 (U+754C, 3 UTF-8 bytes, display width 2) then X
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // Both display columns of the wide char map to its own start byte
    // (0), not to two different bytes -- the continuation cell has no
    // byte of its own.
    assert_eq!(grid.buffer_pos_at(row0, col0), Some(0));
    assert_eq!(grid.buffer_pos_at(row0, col0 + 1), Some(0));
    // 'X' starts right after the wide char's 3 UTF-8 bytes.
    assert_eq!(grid.buffer_pos_at(row0, col0 + 2), Some(3));
}

#[test]
fn buffer_pos_at_chrome_row_returns_none() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let mode_row = grid.windows[0].mode_line_row;
    // The mode line has no buffer behind any of its columns.
    assert_eq!(grid.buffer_pos_at(mode_row, 0), None);
}

#[test]
fn buffer_pos_at_outside_any_run_row_returns_none() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hi\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    // A row well past everything painted (`rows` is only 8; this is
    // deliberately out of range) has no runs at all.
    assert_eq!(grid.buffer_pos_at(grid.rows + 5, 0), None);
}

#[test]
fn paint_runs_cover_buffer_text_with_byte_accurate_src() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    let hello_run = grid
        .runs
        .iter()
        .find(|r| r.row == row0 && r.col == col0 && r.src.is_some())
        .expect("a src-tagged run must cover the buffer text row");
    assert_eq!(hello_run.text, "hello");
    assert_eq!(hello_run.src, Some(0..5));
    // Chrome on the same frame (e.g. the mode line) must carry no src.
    assert!(grid
        .runs
        .iter()
        .filter(|r| r.row == grid.windows[0].mode_line_row)
        .all(|r| r.src.is_none()));
}

#[test]
fn buffer_pos_at_invisible_cjk_ellipsis_maps_every_column_to_the_hidden_span_start() {
    // Fix 1 (mouse-support milestone review): the invisible-region "..."
    // indicator is always the 3-byte ASCII literal "...". When the
    // overlay it stands in for hides exactly one CJK character -- also 3
    // UTF-8 bytes, though only 1 char -- `r.text.len() == byte_len`
    // coincides, and the old discriminator (`r.text.len() != byte_len`)
    // misread that as "proportional expansion" and walked "..."'s own
    // three ASCII characters, returning `src.start + 1` / `src.start + 2`
    // -- byte offsets that land on UTF-8 continuation bytes of the hidden
    // CJK character, not character boundaries. `PaintRun::atomic` fixes
    // this by making the run's non-proportional-ness an explicit flag
    // instead of an inferred coincidence.
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"a\u{754c}b\")"); // a 界 b -- 界 is U+754C, 3 bytes, 1 char
    run(
        &mut i,
        "(let ((ov (make-overlay 2 3))) (overlay-put ov 'invisible t))",
    );
    let grid = render(&i, &ed);
    let (row0, col0) = text_origin(&grid);
    // Row reads "a...b". The hidden span is the single CJK char, char
    // index 1..2, i.e. buffer bytes 1..4 (byte 0 is 'a').
    assert_eq!(grid.buffer_pos_at(row0, col0), Some(0)); // 'a'
    assert_eq!(grid.buffer_pos_at(row0, col0 + 1), Some(1)); // 1st '.' of "..."
    assert_eq!(grid.buffer_pos_at(row0, col0 + 2), Some(1)); // 2nd '.' -- must NOT be 2
    assert_eq!(grid.buffer_pos_at(row0, col0 + 3), Some(1)); // 3rd '.' -- must NOT be 3
    assert_eq!(grid.buffer_pos_at(row0, col0 + 4), Some(4)); // 'b'
}

#[test]
fn buffer_pos_at_result_is_always_a_char_boundary() {
    // The invariant Fix 1 exists to guarantee. An earlier version of this
    // test covered a 2-byte Latin-1 accented character, a 3-byte CJK
    // character, a tab, and a control character -- and passed even with
    // the pre-Fix-1 bug reinstated (`if r.atomic` forced to `if false`),
    // because NONE of those cases can produce a non-boundary byte via the
    // proportional branch:
    //   - A tab run is N spaces over 1 source byte. Walking it
    //     proportionally returns src.start, src.start+1, ... -- those run
    //     past src.end, so the BYTE OFFSET is wrong, but every value it
    //     produces still lands on a char boundary, because whatever comes
    //     after a tab in this buffer is ordinary ASCII.
    //   - A control-character run is "^A", two ASCII bytes over one
    //     source byte -- same story: wrong, but still boundary-aligned.
    //   - A CJK character painted directly (not hidden) is one run of 3
    //     bytes / 2 columns, and the proportional walk steps by
    //     `len_utf8()` per character in `r.text`, so for an un-merged
    //     single-character run it is boundary-correct by construction.
    // The ONLY construct that produces a genuine non-boundary byte is the
    // one that first exposed this defect: an invisible-region ellipsis
    // whose hidden span is multi-byte. `"..."` is three one-byte ASCII
    // characters, so walking IT proportionally hands back src.start+1 and
    // src.start+2 -- offsets into the middle of whatever multi-byte
    // character the overlay is hiding. Anyone strengthening this test
    // later needs to know that "covers tabs, control chars, and a CJK
    // character" is exactly the set of cases that let this bug through
    // once already -- the overlay-hidden span below is not optional
    // padding, it is the one case that actually exercises the invariant.
    let (mut i, ed) = setup();
    // Precomposed é (U+00E9, 2 bytes). Built via `char-to-string`/`concat`
    // rather than an elisp string escape for the control character: this
    // reader's string syntax has no `\uXXXX`/`\xNN` escape (a bare `\u`
    // is read as literal `u`), so a literal `"\u0001"` would silently
    // insert the six characters `u`, `0`, `0`, `0`, `1` instead of one
    // control byte. The trailing `a`, CJK char, `b` give the overlay
    // below something multi-byte to hide.
    run(
        &mut i,
        "(insert (concat (char-to-string ?\u{e9}) (char-to-string ?\u{754c}) \"\\tX\" (char-to-string 1) \"Ya\" (char-to-string ?\u{754c}) \"b\"))",
    );
    // 1-based points: 1=é 2=界 3=\t 4=X 5=^A 6=Y 7=a 8=界 9=b. Hide the
    // second CJK character (point 8..9) so its run renders as "..." --
    // the shape that reproduced Fix 1's panic.
    run(
        &mut i,
        "(let ((ov (make-overlay 8 9))) (overlay-put ov 'invisible t))",
    );
    let grid = render(&i, &ed);
    // Ground truth is the FULL underlying buffer text, including the
    // bytes the overlay hides -- `buffer_pos_at` answers in buffer byte
    // offsets, not in terms of what's currently visible on screen.
    let text = ed.borrow().current.borrow().text.to_string();
    // Sweep every row and every column in the grid, not just the ones
    // this test's author expects to matter -- that is the point of an
    // invariant test: it catches the case nobody thought of, the way the
    // original narrower version of this test did not.
    for row in 0..grid.rows {
        for col in 0..grid.cols {
            if let Some(byte) = grid.buffer_pos_at(row, col) {
                assert!(
                    byte <= text.len() && text.is_char_boundary(byte),
                    "row {row} col {col} mapped to non-boundary byte {byte} in {text:?}"
                );
            }
        }
    }
}

#[test]
fn buffer_pos_at_gutter_column_falls_through_to_line_end_not_none() {
    // Pinning test, not a design endorsement. `Grid::buffer_pos_at` has
    // no concept of "gutter" -- it only knows about runs, and gutter
    // columns carry `src: None` chrome runs (see `fill_chrome_runs`),
    // never a `Some`-src buffer-text run. So a gutter column matches NO
    // run's positive-width span, which sends `buffer_pos_at` down the
    // exact same "past the end of this row's text" fallback that a
    // genuine past-line-end click uses (see `buffer_pos_at_past_end_
    // of_line_clamps_to_line_end` above) -- it does NOT return `None`.
    //
    // This is why `pixel_to_buffer_pos` in `frontend-gui/src/lib.rs`
    // has its own explicit `col >= text_left` guard BEFORE ever calling
    // `buffer_pos_at`: without that guard, a click on the line-number
    // gutter would silently place point at the end of that line
    // instead of doing nothing (the milestone's guard-rail requirement:
    // "clicks outside any window's text area must do nothing"). The
    // GUI's own gutter-click test is `pixel_to_buffer_pos_gutter_
    // column_returns_none` in `frontend-gui/src/lib.rs`'s test module.
    //
    // This test pins today's fallback so a change to it doesn't slip
    // past unnoticed -- it does not assert that "line end" is the
    // *right* contract for a gutter click reaching `buffer_pos_at`
    // directly; an alternative (`None` for any column before the row's
    // first run) was considered and is not applied in this round.
    let (mut i, ed) = setup();
    run(&mut i, "(setq display-line-numbers t)");
    run(&mut i, "(insert \"hello\\n\")");
    let grid = core::redisplay::render(&i, &ed);
    let win = &grid.windows[0];
    assert!(win.gutter_cols > 0, "line numbers must be on for this pin");
    // Leftmost gutter column, on the window's first (only) text row.
    let gutter_col = win.col;
    // "hello" is 5 bytes; the fallback lands on the row's last run's
    // src.end, which is 5 -- right before the '\n', i.e. "end of line".
    assert_eq!(grid.buffer_pos_at(win.row, gutter_col), Some(5));
}

// ---------------------------------------------------------------------
// Fix 3 (mouse-support milestone review): wheel scroll must not
// self-cancel. `scroll_window_start` moves `window_start` without
// moving point; `render_window` calls `ensure_point_visible` on every
// frame, which used to recentre unconditionally whenever point drifted
// far enough from `window_start` -- undoing the scroll on the very next
// render. `Window::scroll_pin` fixes this: see its doc comment for the
// exact semantics.
// ---------------------------------------------------------------------

#[test]
fn wheel_scroll_survives_the_next_render_when_point_does_not_move() {
    let (mut i, ed) = setup();
    // 60 short lines -- point stays at line 0 (char 0) throughout.
    let lines: Vec<String> = (0..60).map(|n| format!("line{n}")).collect();
    run(&mut i, &format!("(insert {:?})", lines.join("\n")));
    run(&mut i, "(goto-char (point-min))");
    // Establish window_start at 0 with an initial render.
    let _ = render(&i, &ed);
    let win_id = ed.borrow().selected_window;
    assert_eq!(ed.borrow().windows[&win_id].window_start, 0);

    // Scroll forward 20 lines -- point (still 0) is now far enough from
    // the new window_start that the OLD `ensure_point_visible` would
    // recentre it right back toward 0 on the very next render.
    core::redisplay::scroll_window_start(&ed, win_id, 20);
    let scrolled_start = ed.borrow().windows[&win_id].window_start;
    assert!(
        scrolled_start > 0,
        "scroll must have advanced window_start, got {scrolled_start}"
    );

    // Multiple renders in a row (simulating repeated frames with no
    // intervening command) must NOT snap window_start back toward point.
    for _ in 0..3 {
        let _ = render(&i, &ed);
        assert_eq!(
            ed.borrow().windows[&win_id].window_start,
            scrolled_start,
            "an explicit scroll must survive repeated renders while point hasn't moved"
        );
    }
}

#[test]
fn moving_point_after_a_wheel_scroll_resumes_normal_recentring() {
    let (mut i, ed) = setup();
    let lines: Vec<String> = (0..60).map(|n| format!("line{n}")).collect();
    run(&mut i, &format!("(insert {:?})", lines.join("\n")));
    run(&mut i, "(goto-char (point-min))");
    let _ = render(&i, &ed);
    let win_id = ed.borrow().selected_window;

    core::redisplay::scroll_window_start(&ed, win_id, 20);
    let scrolled_start = ed.borrow().windows[&win_id].window_start;
    let _ = render(&i, &ed); // pin holds -- point hasn't moved yet
    assert_eq!(ed.borrow().windows[&win_id].window_start, scrolled_start);

    // A real command moves point -- the pin must be dropped and
    // `ensure_point_visible` must resume making the window follow point,
    // same as any command that moves point off-screen.
    run(&mut i, "(goto-char (point-max))");
    let _ = render(&i, &ed);
    let after_move_start = ed.borrow().windows[&win_id].window_start;
    // point-max (near line 60) is far from `scrolled_start` (near line
    // 20) -- with the pin correctly dropped, `ensure_point_visible` must
    // have moved window_start again to bring point back on screen, so
    // it must differ from the pinned value the scroll left behind.
    assert_ne!(
        after_move_start, scrolled_start,
        "moving point must resume recentring, not keep the stale scrolled position"
    );
    let point = ed.borrow().current.borrow().point;
    assert!(
        point >= after_move_start,
        "point ({point}) must be visible (>= window_start {after_move_start}) again"
    );
}

// M105: user-selectable GUI font. These exercise the elisp side only
// (`crates/core/lisp/gui.el`) -- the embedded-bytes/atlas-rebuild side
// lives in `crates/frontend-gui/tests/font_tests.rs`, which is a
// separate crate this one has no dependency on.

#[test]
fn gui_font_and_related_vars_have_the_documented_defaults() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "gui-font"), "jetbrains-mono");
    assert_eq!(run(&mut i, "gui-font-family"), "nil");
    assert_eq!(run(&mut i, "gui-font-size"), "16");
    assert_eq!(run(&mut i, "gui-padding-x"), "10");
    assert_eq!(run(&mut i, "gui-padding-y"), "6");
    assert_eq!(run(&mut i, "gui-line-spacing"), "100");
    assert_eq!(run(&mut i, "gui-indent-guide-step"), "nil");
    assert_eq!(run(&mut i, "gui-cursor-blinks"), "10");
    assert_eq!(run(&mut i, "gui-debug-overlay"), "nil");
    assert_eq!(run(&mut i, "gui-opacity"), "100");
}

#[test]
fn set_font_is_interactive_and_updates_gui_font_via_completing_read() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(fboundp 'set-font)"), "t");
    // Stub `completing-read` the same way `format_tests.rs`'s
    // `format-set-style` tests do: `fset` a fixed-choice replacement,
    // since there is no real minibuffer in this headless test.
    run(&mut i, "(setq gui-font 'jetbrains-mono)");
    run(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback \"fira-code\")))",
    );
    run(&mut i, "(set-font)");
    assert_eq!(run(&mut i, "gui-font"), "fira-code");
}

#[test]
fn set_font_rejects_sf_mono_when_not_installed() {
    let (mut i, _ed) = setup();
    run(&mut i, "(setq gui-font 'jetbrains-mono)");
    run(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback \"sf-mono\")))",
    );
    // Force the "not installed" branch regardless of what's actually on
    // the machine running this test, by redefining the availability
    // check itself -- `gui--sf-mono-available-p` is a plain `defun`, and
    // `fset` overwrites it the same way `format-set-style`'s tests
    // overwrite `completing-read`.
    run(&mut i, "(fset 'gui--sf-mono-available-p (lambda () nil))");
    run(&mut i, "(set-font)");
    assert_eq!(
        run(&mut i, "gui-font"),
        "jetbrains-mono",
        "gui-font must be left unchanged when sf-mono isn't available"
    );
}

/// M107: all five built-in themes (the two pre-existing plus the three
/// new dark ones) actually load and take effect -- `current-theme` is
/// updated and the `default` face's fg/bg match that theme's real hex
/// values, not just "some color changed".
#[test]
fn all_five_themes_load_and_set_the_default_face() {
    let (mut i, ed) = setup();
    // A named triple rather than a bare tuple: clippy's type_complexity
    // fires on the inline form, and the names say which end is which.
    struct ThemeCase {
        name: &'static str,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    }
    let cases: [ThemeCase; 5] = [
        ThemeCase {
            name: "dracula",
            fg: (0xf8, 0xf8, 0xf2),
            bg: (0x28, 0x2a, 0x36),
        },
        ThemeCase {
            name: "xcode",
            fg: (0xff, 0xff, 0xff),
            bg: (0x1f, 0x1f, 0x24),
        },
        ThemeCase {
            name: "vscode",
            fg: (0xd4, 0xd4, 0xd4),
            bg: (0x1e, 0x1e, 0x1e),
        },
        ThemeCase {
            name: "dark",
            fg: (0xc5, 0xca, 0xd3),
            bg: (0x19, 0x1b, 0x20),
        },
        ThemeCase {
            name: "light",
            fg: (0x2c, 0x31, 0x3a),
            bg: (0xfb, 0xfb, 0xfd),
        },
    ];
    for ThemeCase { name, fg, bg } in cases {
        run(&mut i, &format!("(load-theme '{name})"));
        assert_eq!(
            run(&mut i, "current-theme"),
            name,
            "current-theme did not update after loading {name}"
        );
        let style = core::redisplay::frame_base_style(&i, &ed.borrow());
        assert_eq!(style.fg, Some(fg), "{name}: default face fg mismatch");
        assert_eq!(style.bg, Some(bg), "{name}: default face bg mismatch");
    }
}

/// M107: dracula must be the theme a freshly started interpreter is
/// already on, with no `load-theme` call from the test at all.
#[test]
fn default_theme_on_startup_is_dracula() {
    let (mut i, ed) = setup();
    assert_eq!(run(&mut i, "current-theme"), "dracula");
    let style = core::redisplay::frame_base_style(&i, &ed.borrow());
    assert_eq!(
        style.bg,
        Some((0x28, 0x2a, 0x36)),
        "default face background must be dracula's, with no load-theme call"
    );
}

/// M107: an unrecognized theme name is still rejected, and the error
/// message names all five known themes (not just the original two), so
/// a user mistyping a name gets a useful list of valid choices.
#[test]
fn load_theme_rejects_unknown_names_and_lists_all_five() {
    let (mut i, _ed) = setup();
    let result = run(&mut i, "(load-theme 'nonexistent-theme)");
    assert!(
        result.starts_with("ERROR"),
        "expected an error, got {result:?}"
    );
    for name in ["dracula", "xcode", "vscode", "dark", "light"] {
        assert!(
            result.contains(name),
            "error message should list {name:?} as a valid choice, got {result:?}"
        );
    }
}

/// M107 sanity check: none of the three new themes forgot to set a
/// background at all -- a face left completely unstyled would silently
/// fall back to whatever `Style::default()` renders as (typically pure
/// black or the terminal's own background), so require the three new
/// dark themes' `default` background to be neither pure black nor pure
/// white.
#[test]
fn new_dark_themes_have_a_real_background_not_black_or_white() {
    let (mut i, ed) = setup();
    for name in ["dracula", "xcode", "vscode"] {
        run(&mut i, &format!("(load-theme '{name})"));
        let bg = core::redisplay::frame_base_style(&i, &ed.borrow())
            .bg
            .unwrap_or_else(|| panic!("{name}: default face has no background at all"));
        assert_ne!(bg, (0x00, 0x00, 0x00), "{name}: background is pure black");
        assert_ne!(bg, (0xff, 0xff, 0xff), "{name}: background is pure white");
    }
}

/// M107: `load-theme` called interactively (no argument) prompts via
/// `completing-read` over the five theme names, the same pattern
/// `set-font`/`format-set-style` already use, and applies whichever
/// theme the user picked.
#[test]
fn load_theme_is_interactive_and_applies_the_chosen_theme() {
    let (mut i, ed) = setup();
    assert_eq!(run(&mut i, "current-theme"), "dracula");
    run(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback \"xcode\")))",
    );
    run(&mut i, "(load-theme)");
    assert_eq!(run(&mut i, "current-theme"), "xcode");
    assert_eq!(
        core::redisplay::frame_base_style(&i, &ed.borrow()).bg,
        Some((0x1f, 0x1f, 0x24))
    );
}
