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
    // Assert the exact hl-line face color (dark theme #20232a), not
    // just is_some(): a wrong face (e.g. region's) must fail here
    // (M32 review: the Option-only assertions couldn't tell).
    assert_eq!(
        grid.lines[1][0].style.bg,
        Some((0x20, 0x23, 0x2a)),
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
        Some((0x2c, 0x44, 0x63)),
        "inside the region: the region face (dark #2c4463), not hl-line"
    );
    assert_eq!(
        grid.lines[0][8].style.bg,
        Some((0x20, 0x23, 0x2a)),
        "outside the region on the cursor row: the hl-line face"
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
    let dark_bg = core::redisplay::frame_base_style(&i, &ed.borrow()).bg;
    assert_eq!(
        dark_bg,
        Some((0x19, 0x1b, 0x20)),
        "dark theme is the default"
    );
    run(&mut i, "(load-theme 'light)");
    let light = core::redisplay::frame_base_style(&i, &ed.borrow());
    assert_eq!(light.bg, Some((0xfb, 0xfb, 0xfd)));
    assert_eq!(light.fg, Some((0x2c, 0x31, 0x3a)));
    // And back.
    run(&mut i, "(load-theme 'dark)");
    assert_eq!(
        core::redisplay::frame_base_style(&i, &ed.borrow()).bg,
        Some((0x19, 0x1b, 0x20))
    );
    assert_eq!(run(&mut i, "current-theme"), "dark");
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

/// M-visual-quality: both built-in themes must define the exact same
/// set of face names -- catches a future edit that adds a face to one
/// theme's function and forgets the other, silently leaving that face
/// unstyled whenever the forgotten theme is active.
#[test]
fn both_themes_define_the_same_set_of_faces() {
    // This reads the source rather than the running face table, and that
    // is not squeamishness -- the obvious runtime version of this test is
    // vacuous. `themes.el` calls `(load-theme 'dark)` at eval time, so by
    // the time a test can call `(load-theme 'light)` the face table
    // already holds everything the dark theme set. Loading light on top
    // only ever adds or overwrites, never removes, so the light set is
    // unconditionally a superset of the dark one and deleting a face from
    // `theme--light` is invisible. Caught by mutation C3 in
    // dev/mutations/m86.py, which survived the earlier runtime version of
    // this test.
    let src = include_str!("../lisp/themes.el");

    fn faces_set_by(src: &str, defun: &str) -> std::collections::BTreeSet<String> {
        let body = src
            .split_once(&format!("(defun {defun} ("))
            .unwrap_or_else(|| panic!("{defun} not found in themes.el"))
            .1;
        // Each theme is one top-level defun, so the next one starts at the
        // next column-zero `(defun`; take everything before that.
        let body = body.split("\n(defun ").next().unwrap_or(body);
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

    let dark_names = faces_set_by(src, "theme--dark");
    let light_names = faces_set_by(src, "theme--light");
    assert!(
        !dark_names.is_empty() && !light_names.is_empty(),
        "parsed no faces at all -- the parser, not the themes, is broken \
         (dark={}, light={})",
        dark_names.len(),
        light_names.len()
    );

    let dark_only: Vec<_> = dark_names.difference(&light_names).collect();
    let light_only: Vec<_> = light_names.difference(&dark_names).collect();
    assert!(
        dark_only.is_empty() && light_only.is_empty(),
        "face sets differ: dark-only={:?} light-only={:?}",
        dark_only,
        light_only
    );
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
