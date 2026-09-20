//! M138: viewport primitives (`window-start`, `set-window-start`,
//! `pos-visible-in-window-p`, `window-text-height`) and the Emacs paging
//! commands built on top of them (`recenter`, `recenter-top-bottom`,
//! `scroll-up-command`, `scroll-down-command`). Every numeric expectation
//! below is quoted from a real GNU Emacs 30.2 run captured under
//! `dev/gnu-scroll/` (the `-gui` scripts; the plain batch one has no
//! redisplay and is NOT the reference -- see that directory's README).
//!
//! M139 adds one test for `(window-row-start N &optional WINDOW)`, the
//! only Rust addition evil's viewport family (`crates/core/lisp/evil.el`)
//! needed -- it has no independent vim reference number of its own since
//! it is a pure "walk N rows from window-start" helper, not a vim command.
//!
//! No shared fixture module (project convention: each test file brings
//! its own helpers), modelled on `inline_diagnostics_tests.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use core::redisplay::render;
use elisp::printer::prin1_to_string;
use elisp::Interp;

/// Frame sized so `text_rows == rows - 2` for a single, unsplit window
/// (the echo row eats one, the window's own mode line eats another --
/// same arithmetic `inline_diagnostics_tests.rs`'s `text_rows` documents).
/// `setup(80, 23)` gives `text_rows == 21`, matching the GNU frame every
/// number in this file was measured against.
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

/// `(LINE SEV . "MSG")` per entry -- same shape `inline_diagnostics_tests.rs`'s
/// `set_diags` builds, for test 16's block-row row-counting case.
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

/// A 100-line buffer ("line 1" .. "line 100"), each terminated by a
/// newline -- so line 101 is the empty end-of-buffer line, exactly the
/// shape `dev/gnu-scroll/gnu-scroll-gui.el` builds. Every GNU number in
/// this file's doc comments was measured against this exact buffer.
fn hundred_lines(interp: &mut Interp) {
    let mut text = String::new();
    for n in 1..=100 {
        text.push_str(&format!("line {}\n", n));
    }
    let r = run(interp, &format!("(insert {:?})", text));
    assert!(
        !r.starts_with("ERROR"),
        "hundred_lines insert failed: {}",
        r
    );
    let r = run(interp, "(goto-char (point-min))");
    assert!(!r.starts_with("ERROR"), "goto-char failed: {}", r);
}

fn ws_line(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) -> usize {
    let editor = ed.borrow();
    let sel = editor.selected_window;
    let win = &editor.windows[&sel];
    let n = win.buffer.borrow().text.line_number(win.window_start);
    let _ = interp;
    n
}

fn pt_line(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) -> usize {
    let editor = ed.borrow();
    let sel = editor.selected_window;
    let buf = editor.windows[&sel].buffer.clone();
    let p = buf.borrow().point;
    let n = buf.borrow().text.line_number(p);
    let _ = interp;
    n
}

fn goto_line(interp: &mut Interp, line: usize) {
    let r = run(
        interp,
        &format!(
            "(progn (goto-char (point-min)) (forward-line {}))",
            line - 1
        ),
    );
    assert!(!r.starts_with("ERROR"), "goto_line failed: {}", r);
}

// --- 1-3: scroll-up-command sweep (dev/gnu-scroll/gnu-scroll-gui.el) ---

#[test]
fn scroll_up_from_top_moves_start_and_point_to_line_20() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 20);
    assert_eq!(pt_line(&mut i, &ed), 20);
}

#[test]
fn two_scroll_ups_reach_39_then_58() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    let expected = [20, 39, 58, 77, 96];
    for &want in &expected {
        let r = run(&mut i, "(scroll-up-command)");
        assert_eq!(r, "nil", "{}", r);
        assert_eq!(ws_line(&mut i, &ed), want);
    }
    // Every further press signals end-of-buffer and leaves start at 96.
    for _ in 0..2 {
        let r = run(&mut i, "(scroll-up-command)");
        assert_eq!(r, "ERROR: End of buffer", "{}", r);
        assert_eq!(ws_line(&mut i, &ed), 96);
    }
}

#[test]
fn scroll_up_arg_3_and_minus_3() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(scroll-up-command)"); // start 20, point 20
    run(&mut i, "(scroll-up-command)"); // start 39, point 39
    run(&mut i, "(scroll-up-command 3)");
    assert_eq!(ws_line(&mut i, &ed), 42);
    assert_eq!(pt_line(&mut i, &ed), 42);
    run(&mut i, "(scroll-up-command -3)");
    assert_eq!(ws_line(&mut i, &ed), 39);
    assert_eq!(pt_line(&mut i, &ed), 42);
}

// --- 4-5: scroll-down-command ---

// Fix round item 5: the previous versions of both tests asserted point
// line 20 without ever having driven the buffer through the sequence
// that would put it there -- neither number was ever actually exercised
// by GNU's own measured trace (`dev/gnu-scroll/gnu-scroll-gui.el`'s
// `C-v #1/#2/arg 3/arg -3/M-v #1/M-v arg 3` run). Reproduce that exact
// sequence and assert both start and point at every step.
#[test]
fn scroll_down_moves_point_to_bottom_row_40() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(scroll-up-command)"); // C-v #1
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (20, 20));
    run(&mut i, "(scroll-up-command)"); // C-v #2
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (39, 39));
    run(&mut i, "(scroll-up-command 3)"); // C-v arg 3
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (42, 42));
    run(&mut i, "(scroll-up-command -3)"); // C-v arg -3
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (39, 42));
    let r = run(&mut i, "(scroll-down-command)"); // M-v #1
    assert_eq!(r, "nil", "{}", r);
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (20, 40));
}

#[test]
fn scroll_down_arg_3_to_17_point_37() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(scroll-up-command)"); // C-v #1
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (20, 20));
    run(&mut i, "(scroll-up-command)"); // C-v #2
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (39, 39));
    run(&mut i, "(scroll-up-command 3)"); // C-v arg 3
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (42, 42));
    run(&mut i, "(scroll-up-command -3)"); // C-v arg -3
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (39, 42));
    run(&mut i, "(scroll-down-command)"); // M-v #1
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (20, 40));
    run(&mut i, "(scroll-down-command 3)"); // M-v arg 3
    assert_eq!((ws_line(&mut i, &ed), pt_line(&mut i, &ed)), (17, 37));
}

#[test]
fn scroll_down_at_top_signals_beginning_of_buffer_even_with_point_on_line_6() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    let r = run(&mut i, "(scroll-down-command)");
    assert_eq!(r, "ERROR: Beginning of buffer", "{}", r);
    goto_line(&mut i, 6);
    render(&i, &ed);
    let r = run(&mut i, "(scroll-down-command)");
    assert_eq!(r, "ERROR: Beginning of buffer", "{}", r);
}

// --- 7: boundary behaviour at the end of the buffer ---

#[test]
fn scroll_up_boundary_81_allowed_82_signals() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);

    goto_line(&mut i, 81);
    render(&i, &ed);
    // window-start tracks point via ensure_point_visible's recentre; force
    // it explicitly so the scroll's own starting point is exactly line 81.
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 100);

    goto_line(&mut i, 82);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "ERROR: End of buffer", "{}", r);

    goto_line(&mut i, 83);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "ERROR: End of buffer", "{}", r);

    goto_line(&mut i, 80);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 99);

    goto_line(&mut i, 70);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let before = ws_line(&mut i, &ed);
    let r = run(&mut i, "(scroll-up-command 50)");
    assert_eq!(r, "ERROR: End of buffer", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), before, "nothing should move");
}

// --- 8: scroll-down clamps at the top ---

#[test]
fn scroll_down_large_arg_clamps_to_top_without_error() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);

    goto_line(&mut i, 30);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-down-command 30)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 1);
    assert_eq!(pt_line(&mut i, &ed), 21);

    goto_line(&mut i, 30);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-down-command 50)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 1);
    assert_eq!(pt_line(&mut i, &ed), 21);

    goto_line(&mut i, 2);
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point) t)");
    let r = run(&mut i, "(scroll-down-command 30)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 1);
    assert_eq!(pt_line(&mut i, &ed), 2);
}

// --- 9: after end-of-buffer, C-v still signals ---

#[test]
fn after_end_of_buffer_scroll_up_signals() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    run(&mut i, "(goto-char (point-max))");
    render(&i, &ed); // ensure_point_visible recentres on the last page
                     // Fix round item 7: GNU's own measured trace names the resulting
                     // start explicitly ("after M->" -> ws=91). Asserted here rather than
                     // adjusted to whatever this editor happens to compute -- if this
                     // fails, that is an `ensure_point_visible`/`recenter` divergence from
                     // GNU worth knowing about, not a wrong expectation.
    assert_eq!(ws_line(&mut i, &ed), 91);
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "ERROR: End of buffer", "{}", r);
}

// --- 10: recenter table (dev/gnu-scroll/gnu-scroll-gui2.el) ---

#[test]
fn recenter_table() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);

    let cases: &[(usize, Option<i64>, usize)] = &[
        (50, None, 40),
        (50, Some(0), 50),
        (50, Some(-1), 30),
        (50, Some(3), 47),
        (50, Some(-3), 32),
        (50, Some(25), 30),
        (50, Some(-25), 40),
        (3, None, 1),
        (3, Some(-1), 1),
        (99, None, 89),
        (99, Some(0), 99),
        (99, Some(-1), 79),
    ];
    for &(point_line, arg, want) in cases {
        // `recenter` computes purely from point (matching GNU: it does
        // not consult the window's previous start at all), so no
        // window-start setup is needed here.
        goto_line(&mut i, point_line);
        let src = match arg {
            None => "(recenter)".to_string(),
            Some(a) => format!("(recenter {})", a),
        };
        let r = run(&mut i, &src);
        assert_eq!(r, "t", "{} on point line {}: {}", src, point_line, r);
        assert_eq!(
            ws_line(&mut i, &ed),
            want,
            "{} on point line {}",
            src,
            point_line
        );
    }

    // Point on line 31 with start 20 -> (recenter) -> 21 (recenter ignores
    // the window's previous start entirely, same as GNU).
    goto_line(&mut i, 31);
    let r = run(&mut i, "(recenter)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 21);
}

// --- 11-12: recenter-top-bottom cycling, driven through the key path ---

#[test]
fn recenter_top_bottom_cycles_40_50_30_40_and_restarts_after_another_command() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    goto_line(&mut i, 50);

    let expected = [40, 50, 30, 40];
    for &want in &expected {
        feed_keys(&mut i, &ed, "C-l").expect("C-l");
        assert_eq!(ws_line(&mut i, &ed), want);
    }

    // An unrelated command in between resets the cycle to top (middle).
    handle_key(&mut i, &ed, Key::Sym("right".to_string()));
    goto_line(&mut i, 50);
    feed_keys(&mut i, &ed, "C-l").expect("C-l");
    assert_eq!(ws_line(&mut i, &ed), 40);
}

#[test]
fn recenter_top_bottom_with_arg_2_gives_48() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    goto_line(&mut i, 50);
    let r = run(&mut i, "(recenter-top-bottom 2)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 48);
}

// --- 13: pos-visible-in-window-p ---

#[test]
fn pos_visible_in_window_p_after_scroll() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(scroll-up-command)"); // start 20, point 20
    let r = run(&mut i, "(pos-visible-in-window-p (point))");
    assert_eq!(r, "t", "{}", r);
    let r = run(&mut i, "(pos-visible-in-window-p (point-min))");
    assert_eq!(r, "nil", "{}", r);
}

// --- 14: window-start round-trips; set-window-start moves offscreen point ---

#[test]
fn window_start_round_trips_and_set_window_start_moves_offscreen_point_to_middle() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);

    let r = run(&mut i, "(goto-char 5) (set-window-start nil (point))");
    let _ = r;
    let r = run(&mut i, "(progn (goto-char (point-min)) (window-start))");
    assert_eq!(r, "1", "{}", r);

    goto_line(&mut i, 41);
    let start_of_line1 = "(save-excursion (goto-char (point-min)) (point))";
    run(
        &mut i,
        &format!("(set-window-start nil {})", start_of_line1),
    );
    assert_eq!(ws_line(&mut i, &ed), 1);
    assert_eq!(pt_line(&mut i, &ed), 11);

    goto_line(&mut i, 1);
    goto_line(&mut i, 50);
    let start_of_line50 = "(save-excursion (goto-char (point-min)) (forward-line 49) (point))";
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(set-window-start nil {})", start_of_line50),
    );
    assert_eq!(ws_line(&mut i, &ed), 50);
    assert_eq!(pt_line(&mut i, &ed), 60);

    // A point already visible under the new start must stay put.
    run(&mut i, "(set-window-start nil (point-min))");
    goto_line(&mut i, 5);
    run(&mut i, "(set-window-start nil (point-min))");
    assert_eq!(pt_line(&mut i, &ed), 5);
}

// --- 15: set-window-start snaps to the visual row start ---

#[test]
fn set_window_start_snaps_to_the_visual_row_start() {
    let (mut i, ed) = setup(40, 10); // cols = 40 for the text area
    let long_line = "x".repeat(100);
    run(&mut i, &format!("(insert {:?})", long_line));
    run(&mut i, "(insert \"\\nline 2\\nline 3\\n\")");
    // A position inside the second wrapped row of line 1 (row 0: chars
    // 0..40, row 1: chars 40..80).
    run(&mut i, "(goto-char 45)");
    run(&mut i, "(set-window-start nil (point))");
    let editor = ed.borrow();
    let sel = editor.selected_window;
    let ws = editor.windows[&sel].window_start;
    drop(editor);
    // The last column is reserved for the wrap continuation marker
    // (`wraps_before`: `col + w > cols - 1`), so at cols=40 each wrapped
    // row holds 39 characters: row 0 is chars 0..39, row 1 starts at 39.
    assert_eq!(
        ws, 39,
        "window-start should snap to row 1's own start, not line 1's start (0)"
    );
}

// --- 16: scroll-up counts M87 stage 3 block rows ---

#[test]
fn scroll_up_counts_block_rows() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    run(&mut i, "(setq inline-diagnostics t)");
    // Two single-line diagnostics on 0-based line 4 (line 5, 1-based).
    set_diags(&mut i, &[(4, 1, "one"), (4, 2, "two")]);
    render(&i, &ed);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(set-window-start nil (point-min) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(
        ws_line(&mut i, &ed),
        18,
        "rows 1-4=lines1-4, row4=line5, block rows 5-6, row7=line6, row19=line18"
    );
}

// --- 17: scroll-up counts wrapped rows ---

#[test]
fn scroll_up_counts_wrapped_rows() {
    let (mut i, ed) = setup(40, 23); // cols = 40 for the text area
    let mut text = String::new();
    text.push_str("line 1\nline 2\n");
    text.push_str(&"y".repeat(100)); // line 3: 100 chars = 3 wrapped rows @ cols 40
    text.push('\n');
    for n in 4..=100 {
        text.push_str(&format!("line {}\n", n));
    }
    run(&mut i, &format!("(insert {:?})", text));
    run(&mut i, "(goto-char (point-min))");
    render(&i, &ed);
    run(&mut i, "(set-window-start nil (point-min) t)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 18);
}

// --- 18: scroll_pin survives render until point moves ---

#[test]
fn scroll_pin_survives_render_until_point_moves() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(scroll-up-command)"); // start 20, point 20
    render(&i, &ed);
    render(&i, &ed);
    assert_eq!(ws_line(&mut i, &ed), 20, "the pin must survive re-render");
    run(&mut i, "(forward-line 1)"); // point 21, still visible from start 20
    render(&i, &ed);
    assert_eq!(
        ws_line(&mut i, &ed),
        20,
        "start must stay put while point remains visible"
    );
}

// --- 19: keys are bound ---

#[test]
fn keys_are_bound() {
    let (mut i, _ed) = setup(80, 23);
    for (key, cmd) in [
        ("?\\C-v", "scroll-up-command"),
        ("?\\M-v", "scroll-down-command"),
        ("'next", "scroll-up-command"),
        ("'prior", "scroll-down-command"),
        ("?\\C-l", "recenter-top-bottom"),
    ] {
        let r = run(&mut i, &format!("(lookup-key (list {}))", key));
        assert!(
            r.contains(cmd),
            "expected {} bound to {}, got {}",
            key,
            cmd,
            r
        );
    }
}

// --- 20: last-command is set by the dispatcher ---

#[test]
fn last_command_is_set_by_the_dispatcher() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    feed_keys(&mut i, &ed, "C-v").expect("C-v");
    let r = run(&mut i, "last-command");
    assert_eq!(r, "scroll-up-command", "{}", r);
}

// --- 21: end-of-buffer error rendering ---

#[test]
fn end_of_buffer_error_renders_as_end_of_buffer_message() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    run(&mut i, "(goto-char (point-max))");
    render(&i, &ed);
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "ERROR: End of buffer", "{}", r);

    run(&mut i, "(goto-char (point-min))");
    render(&i, &ed);
    let r = run(&mut i, "(scroll-down-command)");
    assert_eq!(r, "ERROR: Beginning of buffer", "{}", r);
}

// --- Fix round item 1: rows_backward must count a wrap-continuation
// row's own line, not just previous logical lines ---

fn window_start_of(ed: &Rc<RefCell<Editor>>) -> usize {
    let editor = ed.borrow();
    let sel = editor.selected_window;
    editor.windows[&sel].window_start
}

#[test]
fn rows_backward_from_a_wrap_continuation_row() {
    // cols = 40 -> each wrapped row holds 39 chars (the last column is
    // reserved for the continuation marker), so a 200-char line wraps to
    // rows starting at 0, 39, 78, 117, 156, 195.
    let (mut i, ed) = setup(40, 10);
    let long_line = "z".repeat(200);
    run(&mut i, &format!("(insert {:?})", long_line));
    run(&mut i, "(goto-char 90)"); // 0-based 89, inside the third row (78..116)
    run(&mut i, "(set-window-start nil (point))");
    render(&i, &ed);
    assert_eq!(
        window_start_of(&ed),
        78,
        "setup: start must snap to the third row"
    );
    let r = run(&mut i, "(scroll-down-command 1)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(
        window_start_of(&ed),
        39,
        "must land on the second wrapped row's start, not line 1's start (0)"
    );
}

#[test]
fn recenter_on_a_wrapped_line_counts_its_own_rows() {
    let (mut i, ed) = setup(40, 10); // text_rows = 8
    let long_line = "z".repeat(200);
    run(&mut i, &format!("(insert {:?})", long_line));
    run(&mut i, "(goto-char 91)"); // 0-based 90, inside the third row (78..116)
    let r = run(&mut i, "(recenter 0)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        window_start_of(&ed),
        78,
        "(recenter 0) must put point's OWN row at the top, not line 1's start"
    );
    let r = run(&mut i, "(recenter 2)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        window_start_of(&ed),
        0,
        "(recenter 2) must put point's row (78) exactly 2 rows below the new top (0)"
    );
}

#[test]
fn recenter_on_a_short_then_wrapped_then_short_buffer_counts_the_wrapped_lines_rows() {
    let (mut i, ed) = setup(40, 10);
    // line 1 (short) at 0..7, line 2 (200 chars, 6 wrapped rows: relative
    // offsets 0,39,78,117,156,195) at 7..208, line 3 (short) at 208..215.
    let mut text = String::from("line 1\n");
    text.push_str(&"y".repeat(200));
    text.push_str("\nline 3\n");
    run(&mut i, &format!("(insert {:?})", text));
    run(&mut i, "(goto-char 209)"); // 0-based 208: line 3's own start
    let r = run(&mut i, "(recenter 1)");
    assert_eq!(r, "t", "{}", r);
    // rows_backward(line3_start=208, 1): line 3 contributes 0 rows above
    // itself, so the previous-lines loop consumes line 2's 6 rows
    // (wrap_rows=6, block=0); surplus = 6 - 1 = 5, and walking forward 5
    // rows from line 2's start (7) lands on its 6th (last) wrapped row,
    // at offset 7 + 195 = 202.
    assert_eq!(
        window_start_of(&ed),
        202,
        "the top row must be line 2's LAST wrapped row, not line 2's own start"
    );
}

// --- Fix round item 2: set-window-start's NOFORCE ---

#[test]
fn set_window_start_noforce_leaves_point_and_lets_render_recentre() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    let start_of_line50 = "(save-excursion (goto-char (point-min)) (forward-line 49) (point))";

    // NOFORCE t: point must stay exactly where it was, and the start is
    // NOT pinned -- since point (line 1) is now far outside the region
    // starting at line 50, the very next render recentres back onto it.
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(set-window-start nil {} t)", start_of_line50),
    );
    assert_eq!(pt_line(&mut i, &ed), 1, "NOFORCE must not move point");
    assert_eq!(ws_line(&mut i, &ed), 50);
    render(&i, &ed);
    assert_eq!(
        ws_line(&mut i, &ed),
        1,
        "an unpinned start must let ensure_point_visible recentre back onto point"
    );

    // Contrast: NOFORCE nil (omitted) forces point onto the new region
    // and pins the start across a render.
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(set-window-start nil {})", start_of_line50),
    );
    assert_eq!(ws_line(&mut i, &ed), 50);
    assert_eq!(pt_line(&mut i, &ed), 60);
    render(&i, &ed);
    render(&i, &ed);
    assert_eq!(
        ws_line(&mut i, &ed),
        50,
        "the pin must survive re-render when NOFORCE is nil"
    );
}

// --- Fix round item 3: an elisp (setq this-command ...) override must
// win over the Rust-side field when last-command is set ---

#[test]
fn command_overriding_this_command_updates_last_command() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    run(
        &mut i,
        "(defun m138-test-cmd () (interactive) (setq this-command 'foo))",
    );
    run(&mut i, "(global-set-key \"C-c C-t\" 'm138-test-cmd)");
    feed_keys(&mut i, &ed, "C-c C-t").expect("C-c C-t");
    let r = run(&mut i, "last-command");
    assert_eq!(r, "foo", "{}", r);
}

// --- Fix round item 4: pos-visible-in-window-p's nil POS must use the
// NAMED window's own point, not the currently selected window's ---

#[test]
fn pos_visible_in_window_p_nil_pos_uses_the_named_windows_own_point() {
    let (mut i, ed) = setup(80, 46);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(split-window-below)");
    render(&i, &ed);
    let sel = run(&mut i, "(selected-window)");
    let ids_str = run(&mut i, "(window-list)");
    let other = ids_str
        .trim_matches(|c| c == '(' || c == ')')
        .split_whitespace()
        .find(|s| *s != sel)
        .expect("two windows after split")
        .to_string();

    // Diverge the two windows' points WITHOUT re-rendering after the
    // last switch, so OTHER's own `window_start` stays wherever it was
    // before its point moved. Otherwise redisplay would re-sync every
    // window to its own point and both checks would trivially answer
    // `t`, masking the bug this test targets.
    run(&mut i, &format!("(select-window {})", other));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, &format!("(select-window {})", sel));
    run(&mut i, "(goto-char (point-min))");

    let r_self = run(&mut i, &format!("(pos-visible-in-window-p nil {})", sel));
    let r_other = run(&mut i, &format!("(pos-visible-in-window-p nil {})", other));
    assert_eq!(r_self, "t", "{}", r_self);
    assert_eq!(
        r_other, "nil",
        "OTHER's own point (point-max) must be checked, not the selected window's (point-min)"
    );
}

// --- Fix round item 6: rows_backward's block-row term ---

#[test]
fn scroll_down_and_recenter_count_block_rows_above() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    run(&mut i, "(setq inline-diagnostics t)");
    // Same setup as test 16: two single-line diagnostics under 0-based
    // line 4 (line 5, 1-based) -- lines 1-4 = rows 0-3, line 5 = row 4,
    // block rows = rows 5-6, line 6 = row 7, ..., row 19 = line 18. A
    // plain `C-v` from the top therefore lands on line 18, not 20.
    set_diags(&mut i, &[(4, 1, "one"), (4, 2, "two")]);
    render(&i, &ed);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(set-window-start nil (point-min) t)");
    run(&mut i, "(scroll-up-command)");
    assert_eq!(
        ws_line(&mut i, &ed),
        18,
        "setup: C-v from the top lands on line 18"
    );

    // (scroll-down-command) with no arg: n = text_rows - next-screen-
    // context-lines = 21 - 2 = 19. rows_backward(line18, 19): line 18
    // contributes 0 rows above itself (it's a plain line start), so the
    // previous-lines loop must walk back through lines 17..6 (12 lines,
    // 12 rows) + line 5's 1 wrap row + its 2 block rows (3 rows) + lines
    // 4..1 (4 lines outside; but only 19-12-3=4 rows of budget remain,
    // exactly lines 4,3,2,1's 4 rows) = 12+3+4 = 19 rows exactly, landing
    // on line 1's own start.
    let r = run(&mut i, "(scroll-down-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(
        ws_line(&mut i, &ed),
        1,
        "19 rows back from line 18 (17 lines + 2 block rows) lands on line 1"
    );

    // recenter from a point below the diagnostics: point on line 20,
    // (recenter) (rows_above = text_rows/2 = 10) walks back 10 rows from
    // line 20 -- lines 19..11 is 9 rows (9 lines), leaving 1 row of
    // budget, which lands inside line 10's own single row (no block rows
    // in this range), giving line 10.
    goto_line(&mut i, 20);
    let r = run(&mut i, "(recenter)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 10);

    // (recenter -1) from line 20: rows_above = h + (-1) = 20. Walking
    // back 20 rows from line 20 crosses line 5's block rows: lines
    // 19..6 is 14 rows (14 lines), leaving 6; line 5 contributes 1 wrap
    // row + 2 block rows = 3, leaving 3; lines 4..2 are 3 more rows,
    // leaving 0 exactly at line 2's own start.
    let r = run(&mut i, "(recenter -1)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(ws_line(&mut i, &ed), 2);
}

// --- next-screen-context-lines is actually observed ---

#[test]
fn next_screen_context_lines_changes_the_default_scroll_amount() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);
    run(&mut i, "(setq next-screen-context-lines 5)");
    let r = run(&mut i, "(scroll-up-command)");
    assert_eq!(r, "nil", "{}", r);
    assert_eq!(
        ws_line(&mut i, &ed),
        17,
        "21 - 5 = 16 rows forward from line 1 = line 17"
    );
}

// --- M139: window-row-start (forward, backward-clamp, forward-clamp) ---

#[test]
fn window_row_start_walks_and_clamps_at_both_ends() {
    let (mut i, ed) = setup(80, 23);
    hundred_lines(&mut i);
    render(&i, &ed);

    // window-start is line 1: 5 rows forward is line 6's own start.
    let r = run(&mut i, "(window-row-start 5)");
    goto_line(&mut i, 6);
    let expect = run(&mut i, "(line-beginning-position)");
    assert_eq!(r, expect);

    // Negative past row 0 clamps to window-start itself (point-min here).
    let r = run(&mut i, "(window-row-start -3)");
    assert_eq!(r, run(&mut i, "(point-min)"));

    // Forward past the end of the buffer clamps to the last (EOB) row,
    // not an error and not an unclamped position.
    let r = run(&mut i, "(window-row-start 1000)");
    assert_eq!(r, run(&mut i, "(point-max)"));

    // After the window-start itself moves (via recenter), window-row-start
    // walks from the NEW start, and the optional WINDOW argument (nil ==
    // selected window here) resolves the same way window-start's does.
    goto_line(&mut i, 50);
    run(&mut i, "(recenter 0)");
    assert_eq!(
        ws_line(&mut i, &ed),
        50,
        "setup: recenter 0 puts line 50 at the top"
    );
    let r = run(&mut i, "(window-row-start 3 nil)");
    goto_line(&mut i, 53);
    let expect = run(&mut i, "(line-beginning-position)");
    assert_eq!(r, expect);
}
