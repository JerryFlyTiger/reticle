//! M103: `display-buffer`/`pop-to-buffer` (window.el) and the five
//! window-introspection primitives they're built out of
//! (`window-list`/`window-buffer`/`set-window-buffer`/`select-window`/
//! `get-buffer-window`, builtins/ui.rs).
//!
//! Core regression this milestone exists for: before it, EVERY "show a
//! buffer" path (`switch-to-buffer-internal`) unconditionally overwrote
//! the selected window, so `C-h b` on a single-window frame made the
//! file being edited disappear entirely. Test 1 below is the direct
//! repro/guard for that.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
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

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

/// Every window id -> the name of the buffer it shows, as `(id . name)`
/// pairs sorted by id -- the one query most of these tests lean on,
/// since "window-count went up" alone can't tell you WHICH buffer ended
/// up WHERE.
fn window_buffer_map(interp: &mut Interp) -> String {
    run(
        interp,
        "(mapcar (lambda (w) (cons w (with-current-buffer-internal (window-buffer w) (lambda () (buffer-name))))) (window-list))",
    )
}

/// A scratch directory + file on disk, cleaned up best-effort by the
/// test process exiting (parallel test binary, same pattern
/// `core_tests.rs`'s own `Scratch` uses).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_window_display_{}_{}_{}",
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

// =======================================================================
// 1. Core symptom regression guard
// =======================================================================

#[test]
fn c_h_b_splits_instead_of_overwriting_the_file_being_edited() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(insert \"module alu; endmodule\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "C-h b").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "C-h b must split, not overwrite, the sole window"
    );
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"alu.sv\""),
        "the file being edited must still be visible in SOME window, got: {}",
        map
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Help*\"",
        "the selected window must show *Help*"
    );
}

// =======================================================================
// 2. Already two windows: reuse, don't split again
// =======================================================================

#[test]
fn c_h_b_with_two_windows_already_reuses_the_other_one() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(insert \"module alu; endmodule\")");
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    let selected_before = run(&mut i, "(selected-window)");
    let selected_buffer_before = run(&mut i, "(buffer-name)");

    feed_keys(&mut i, &ed, "C-h b").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "must not split a third window into existence"
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");
    // The window that was selected before C-h b must still show
    // whatever it showed before -- untouched.
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(with-current-buffer-internal (window-buffer {}) (lambda () (buffer-name)))",
                selected_before
            )
        ),
        selected_buffer_before,
        "the window that was selected before C-h b must be left alone"
    );
}

// =======================================================================
// 3. Already displayed: reuse that exact window, no new one
// =======================================================================

#[test]
fn displaying_an_already_shown_buffer_reuses_its_window() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-h b").unwrap(); // *Help* now shown, window-count 2
    assert_eq!(run(&mut i, "(window-count)"), "2");
    let help_window = run(&mut i, "(get-buffer-window \"*Help*\")");
    assert_ne!(help_window, "nil");

    // Move selection elsewhere, then ask to display *Help* again.
    feed_keys(&mut i, &ed, "C-x o").unwrap();
    ok(&mut i, "(pop-to-buffer \"*Help*\")");

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "must not create a second window for a buffer already on screen"
    );
    assert_eq!(
        run(&mut i, "(get-buffer-window \"*Help*\")"),
        help_window,
        "must reuse the SAME window *Help* was already in"
    );
}

// =======================================================================
// 3b. Fix round (cold-review defect 1): a SECOND helper buffer display,
// after a split, must not evict the file window it just protected.
// `display-buffer' step 4 (multiple windows, selected one is NOT a
// helper window) used to always pick "the next window after selected"
// with no regard for what it holds -- right after `C-h b' splits, the
// selected window IS the new helper window, so "next after selected"
// is exactly the user's file window.
// =======================================================================

#[test]
fn c_h_b_then_shell_command_swaps_help_for_output_without_evicting_the_file() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(insert \"module alu; endmodule\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");

    // A second helper buffer, displayed while *Help*'s split window is
    // still selected -- this is the exact repro sequence from the
    // cold-review report (real TUI via dev/tui-drive.py: C-h b, then
    // M-! echo ...).
    ok(&mut i, "(get-buffer-create \"*Shell Command Output*\")");
    ok(&mut i, "(pop-to-buffer \"*Shell Command Output*\")");

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "must not spawn a third window"
    );
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"alu.sv\""),
        "the file buffer must still be visible in SOME window, got: {}",
        map
    );
    assert!(
        !map.contains("\"*Help*\""),
        "*Help* must have been swapped out of the helper window it was \
         displayed in, got: {}",
        map
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Shell Command Output*\"",
        "the selected window must now show the new helper buffer"
    );
}

#[test]
fn shell_command_then_c_h_b_swaps_output_for_help_without_evicting_the_file() {
    // Reverse order of the above, per the coordinator's report: "反過來
    // (先 M-! 再 C-h b) 同樣重現".
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(insert \"module alu; endmodule\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    ok(&mut i, "(get-buffer-create \"*Shell Command Output*\")");
    ok(&mut i, "(pop-to-buffer \"*Shell Command Output*\")");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Shell Command Output*\"");

    feed_keys(&mut i, &ed, "C-h b").unwrap();

    assert_eq!(run(&mut i, "(window-count)"), "2");
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"alu.sv\""),
        "the file buffer must still be visible in SOME window, got: {}",
        map
    );
    assert!(
        !map.contains("\"*Shell Command Output*\""),
        "the shell-output buffer must have been swapped out, got: {}",
        map
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");
}

#[test]
fn c_x_2_with_two_different_buffers_then_c_h_b_does_not_evict_the_file_window() {
    // Blind spot in the pre-fix-round test suite, per the coordinator's
    // report: `c_h_b_with_two_windows_already_reuses_the_other_one'
    // used `C-x 2' (both windows show the SAME buffer), so overwriting
    // either one loses nothing -- it can never catch step 4 evicting
    // the wrong content. This test puts a DIFFERENT buffer in each
    // window first, THEN moves selection back to the window the user
    // is actually editing (alu.sv) before invoking C-h b -- "the
    // buffer being edited" can only mean whatever is selected/current
    // AT THE MOMENT C-h b is invoked (command dispatch guarantees
    // current-buffer == selected window's buffer); step 4 always picks
    // a window OTHER than the selected one
    // (`window--other-display-window'/`window--next-after-selected'
    // both explicitly exclude it), so THIS window must survive.
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(insert \"module alu; endmodule\")");
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    feed_keys(&mut i, &ed, "C-x o").unwrap();
    ok(&mut i, "(get-buffer-create \"other-buf\")");
    ok(&mut i, "(switch-to-buffer-internal \"other-buf\")");
    feed_keys(&mut i, &ed, "C-x o").unwrap(); // back to the alu.sv window
    assert_eq!(run(&mut i, "(buffer-name)"), "\"alu.sv\"");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert!(window_buffer_map(&mut i).contains("\"other-buf\""));

    feed_keys(&mut i, &ed, "C-h b").unwrap();

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(
        window_buffer_map(&mut i).contains("\"alu.sv\""),
        "the buffer being edited must not be the one C-h b overwrites, got: {}",
        window_buffer_map(&mut i)
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");
}

// =======================================================================
// 4. Frame too small to split either way: overwrite fallback
// =======================================================================

#[test]
fn display_buffer_overwrites_when_frame_is_too_small_to_split() {
    let (mut i, ed) = setup();
    // WINDOW_MIN_HEIGHT=2, WINDOW_MIN_WIDTH=4 (redisplay.rs). height=4
    // total -> windows_height=3 < 2*2 (vertical split fails); width=8 ->
    // raw=8 < 2*4+1=9 (horizontal split fails too, sep=1 since width>2).
    ed.borrow_mut().frame = (8, 4);
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    let result = run(&mut i, "(display-buffer \"*Help*\")");
    assert!(
        !result.starts_with("ERROR"),
        "display-buffer must not signal, got: {}",
        result
    );
    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "too small to split -- must fall back to overwriting, not split anyway"
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Help*\"",
        "the fallback overwrite must still actually show the buffer"
    );
}

#[test]
fn display_buffer_falls_back_to_horizontal_split_when_vertical_would_fail() {
    let (mut i, ed) = setup();
    // Genuinely different from the previous test (fix round, cold
    // review: the earlier version of this test used the SAME (8, 4)
    // frame as `display_buffer_overwrites_when_frame_is_too_small_to_
    // split' above, so it exercised nothing beyond a weaker copy of
    // that assertion -- the vertical-fails/horizontal-succeeds
    // fallback path had no coverage at all). WINDOW_MIN_HEIGHT=2,
    // WINDOW_MIN_WIDTH=4 (redisplay.rs): height=4 total ->
    // windows_height=3 < 2*2 (vertical split fails), but width=20 ->
    // raw=20 >= 2*4+1=9 (horizontal split succeeds).
    ed.borrow_mut().frame = (20, 4);
    ok(&mut i, "(get-buffer-create \"b.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"b.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    ok(&mut i, "(display-buffer \"*Help*\")");

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "vertical split must fail here, but horizontal split must succeed \
         -- display-buffer must not fall all the way through to overwrite"
    );
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"b.sv\"") && map.contains("\"*Help*\""),
        "both the original buffer and *Help* must be visible, got: {}",
        map
    );
}

// =======================================================================
// 5/6/7. `q` and the per-window `created-for-display' flag
// (`window-created-for-display-p', ui.rs -- see editor.rs's
// `Window::created_for_display' for why this moved off a buffer-local
// elisp variable during M103's third fix round).
// =======================================================================

#[test]
fn q_deletes_the_window_display_buffer_created_for_it() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");

    feed_keys(&mut i, &ed, "q").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "q must delete the window C-h b created"
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"alu.sv\"",
        "must land back on the buffer that was being edited"
    );
}

#[test]
fn q_does_not_delete_a_window_it_did_not_create() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    feed_keys(&mut i, &ed, "C-x 2").unwrap(); // 2 pre-existing windows
    assert_eq!(run(&mut i, "(window-count)"), "2");

    feed_keys(&mut i, &ed, "C-h b").unwrap(); // reuses the other window
    assert_eq!(run(&mut i, "(window-count)"), "2");

    feed_keys(&mut i, &ed, "q").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "q must NOT delete a window that was already there before C-h b"
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"alu.sv\"",
        "the reused window must switch back to quit-source"
    );
}

#[test]
fn window_created_for_display_flag_is_cleared_so_a_second_round_trip_works() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");

    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "1");

    // Second round trip: if the flag from the first round survived,
    // this `q` would delete a window nobody just created.
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "stale window--created-for-display flag must not survive a full q round trip"
    );
}

// =======================================================================
// 7b. Fix round (cold-review defect 2): `q' must close the window the
// user is actually looking at, not unconditionally jump to whichever
// window the flag names and delete THAT one instead.
// =======================================================================

fn parse_window_ids(s: &str) -> Vec<i64> {
    s.trim_start_matches('(')
        .trim_end_matches(')')
        .split_whitespace()
        .map(|tok| tok.parse::<i64>().expect("window id"))
        .collect()
}

#[test]
fn q_from_a_plain_split_clone_switches_buffer_in_place_without_deleting_anything() {
    // M103, third fix round: this test's EXPECTED VALUE changed when the
    // `created-for-display' flag moved from a buffer-local elisp
    // variable to a per-WINDOW field (`Window::created_for_display',
    // editor.rs). Under the old (buffer-local) model, a single flag
    // could not tell window B (the ORIGINAL helper window `display-
    // buffer' created) apart from window C (a plain `C-x 2' clone of B,
    // sharing the same *Help* buffer) -- the flag necessarily "belonged"
    // to whichever of the two happened to be recorded, so `q' pressed in
    // C used to delete a window (either C itself, per the fix in an
    // earlier round, or B, per a later regression -- see PLAN.md's M103
    // record). Under the NEW (per-window) model, C is simply never
    // flagged -- a plain split never sets `created_for_display' on
    // either side, only `display-buffer' does, and only on the window
    // IT creates -- so `q' in C takes the plain "switch this window's
    // buffer back to quit-source" path, same as it would in any window
    // that isn't a `display-buffer' creation. This is also what GNU
    // does for a window with no `quit-restore' parameter: switch the
    // buffer, don't delete the window.
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    // C-h b splits off window B for *Help* (flagged); B stays selected.
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    let window_b = run(&mut i, "(selected-window)");
    assert_eq!(run(&mut i, "(window-created-for-display-p)"), "t");

    // C-x 2, run FROM B, clones *Help* into a brand new window C -- a
    // PLAIN split, never flagged. B stays selected (matches
    // `window_split_and_navigate' in core_tests.rs: splitting never
    // moves selection off the original window).
    let before_ids = parse_window_ids(&run(&mut i, "(window-list)"));
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "3");
    let after_ids = parse_window_ids(&run(&mut i, "(window-list)"));
    let window_c = *after_ids
        .iter()
        .find(|id| !before_ids.contains(id))
        .expect("C-x 2 must have created exactly one new window id");
    assert_eq!(
        run(
            &mut i,
            &format!("(window-created-for-display-p {})", window_c)
        ),
        "nil",
        "a plain split must never flag the window it creates"
    );

    // Move selection to C -- the coordinator's repro: "C-x o 切到 C".
    ok(&mut i, &format!("(select-window {})", window_c));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");

    feed_keys(&mut i, &ed, "q").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "3",
        "q in an unflagged window must not delete anything"
    );
    let remaining = parse_window_ids(&run(&mut i, "(window-list)"));
    assert!(
        remaining.contains(&window_c),
        "window C itself must still exist (buffer switched in place, not deleted), \
         remaining: {:?}",
        remaining
    );
    assert!(
        remaining.contains(&window_b.parse::<i64>().unwrap()),
        "window B must still exist untouched, remaining: {:?}",
        remaining
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(with-current-buffer-internal (window-buffer {}) (lambda () (buffer-name)))",
                window_c
            )
        ),
        "\"alu.sv\"",
        "window C must have switched ITS OWN buffer back to quit-source (alu.sv)"
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(with-current-buffer-internal (window-buffer {}) (lambda () (buffer-name)))",
                window_b
            )
        ),
        "\"*Help*\"",
        "window B must still show *Help*, untouched by q pressed in C"
    );
}

#[test]
fn two_separately_created_helper_windows_each_close_on_their_own_q() {
    // Two DIFFERENT helper buffers, each getting its OWN split (not
    // reusing each other's window, since they're displayed while more
    // than one window already exists would trigger step 4's reuse --
    // this test instead returns to a single window between the two, so
    // each `pop-to-buffer' call takes the "exactly one window" split
    // path and flags its own new window). Each `q' must delete exactly
    // the window it created, landing back on a single window at the end.
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    ok(&mut i, "(pop-to-buffer \"*Help*\")");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(window-created-for-display-p)"), "t");
    ok(&mut i, "(quit-source-return)");
    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "q on the first helper window must delete it"
    );
    assert_eq!(
        run(&mut i, "(get-buffer-window \"*Help*\")"),
        "nil",
        "the first helper window's own id must not resurface as still showing *Help*"
    );

    ok(&mut i, "(pop-to-buffer \"*ielm*\")");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert_eq!(run(&mut i, "(window-created-for-display-p)"), "t");
    ok(&mut i, "(quit-source-return)");
    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "q on the second helper window must delete it too"
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"alu.sv\"",
        "must land back on the original file buffer after both round trips"
    );
}

// =======================================================================
// 8. pop-to-buffer selects, display-buffer does not
// =======================================================================

#[test]
fn pop_to_buffer_selects_the_new_window() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    let before = run(&mut i, "(selected-window)");

    let win = ok(&mut i, "(pop-to-buffer \"*Help*\")");

    assert_eq!(
        run(&mut i, "(selected-window)"),
        win,
        "pop-to-buffer must select the window it displayed into"
    );
    assert_ne!(run(&mut i, "(selected-window)"), before);
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");
}

#[test]
fn display_buffer_does_not_select_the_new_window() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    let before = run(&mut i, "(selected-window)");
    let before_buffer = run(&mut i, "(buffer-name)");

    let win = ok(&mut i, "(display-buffer \"*Help*\")");

    assert_eq!(
        run(&mut i, "(selected-window)"),
        before,
        "display-buffer must NOT move selection"
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        before_buffer,
        "current buffer must be unchanged by a pure display-buffer call"
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(with-current-buffer-internal (window-buffer {}) (lambda () (buffer-name)))",
                win
            )
        ),
        "\"*Help*\"",
        "but the returned window must actually show the buffer"
    );
}

// =======================================================================
// 9/10. C-x 4 f / C-x 4 b
// =======================================================================

#[test]
fn c_x_4_f_opens_a_file_in_another_window() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    let dir = Scratch::new("c_x_4_f");
    std::fs::create_dir_all(&*dir).unwrap();
    let path = dir.join("other.txt");
    std::fs::write(&path, "other file contents").unwrap();

    feed_keys(&mut i, &ed, "C-x 4 f").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, path.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "C-x 4 f must split into a second window"
    );
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"alu.sv\""),
        "the original buffer must still be visible somewhere, got: {}",
        map
    );
    assert_eq!(run(&mut i, "(buffer-string)"), "\"other file contents\"");
}

#[test]
fn c_x_4_b_switches_to_a_buffer_in_another_window() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(get-buffer-create \"other-buf\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "C-x 4 b").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, "other-buf");
    feed_keys(&mut i, &ed, "RET").unwrap();

    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "C-x 4 b must split into a second window"
    );
    let map = window_buffer_map(&mut i);
    assert!(
        map.contains("\"alu.sv\""),
        "the original buffer must still be visible somewhere, got: {}",
        map
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"other-buf\"");
}

// =======================================================================
// 11. Direct builtin tests
// =======================================================================

#[test]
fn window_list_length_and_content() {
    let (mut i, ed) = setup();
    assert_eq!(run(&mut i, "(window-list)"), "(0)");
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    let list = run(&mut i, "(window-list)");
    assert_eq!(list, "(0 1)");
}

#[test]
fn window_buffer_reports_the_buffer_shown_in_a_given_window() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    feed_keys(&mut i, &ed, "C-x o").unwrap();
    ok(&mut i, "(get-buffer-create \"other-buf\")");
    ok(&mut i, "(switch-to-buffer-internal \"other-buf\")");

    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 0) (lambda () (buffer-name)))"
        ),
        "\"alu.sv\""
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 1) (lambda () (buffer-name)))"
        ),
        "\"other-buf\""
    );
    assert_eq!(run(&mut i, "(window-buffer 99)"), "nil");
}

#[test]
fn set_window_buffer_does_not_move_selection() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(selected-window)"), "0");
    ok(&mut i, "(get-buffer-create \"other-buf\")");

    ok(&mut i, "(set-window-buffer 1 \"other-buf\")");

    assert_eq!(
        run(&mut i, "(selected-window)"),
        "0",
        "set-window-buffer must not change the selected window"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 1) (lambda () (buffer-name)))"
        ),
        "\"other-buf\""
    );
    assert_eq!(run(&mut i, "(set-window-buffer 99 \"other-buf\")"), "nil");
}

#[test]
fn select_window_moves_selection_and_reports_missing_windows() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-x 2").unwrap();
    assert_eq!(run(&mut i, "(selected-window)"), "0");

    assert_eq!(run(&mut i, "(select-window 1)"), "t");
    assert_eq!(run(&mut i, "(selected-window)"), "1");
    assert_eq!(run(&mut i, "(select-window 99)"), "nil");
    assert_eq!(
        run(&mut i, "(selected-window)"),
        "1",
        "a failed select-window must not change the selection"
    );
}

#[test]
fn get_buffer_window_finds_and_fails_to_find() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    ok(&mut i, "(get-buffer-create \"never-shown\")");

    assert_eq!(run(&mut i, "(get-buffer-window \"alu.sv\")"), "0");
    assert_eq!(run(&mut i, "(get-buffer-window \"never-shown\")"), "nil");
}

// =======================================================================
// 12. eshell / ielm / compile / async shell / search regression guards --
// each one must still leave the original buffer visible somewhere.
// =======================================================================

#[test]
fn eshell_leaves_the_original_buffer_visible() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    ok(&mut i, "(eshell)");

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*eshell*\"");
}

#[test]
fn ielm_leaves_the_original_buffer_visible() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    ok(&mut i, "(ielm)");

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*ielm*\"");
}

fn no_shell_procs_running(i: &mut Interp) -> bool {
    run(i, "shell-command--procs") == "nil"
}

fn no_compile_procs_running(i: &mut Interp) -> bool {
    run(i, "compile--procs") == "nil"
}

fn no_search_procs_running(i: &mut Interp) -> bool {
    run(i, "search--procs") == "nil"
}

fn pump_until(
    interp: &mut Interp,
    timeout: std::time::Duration,
    mut pred: impl FnMut(&mut Interp) -> bool,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        core::idle_tick(interp, std::time::Duration::from_millis(0));
        if pred(interp) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[test]
fn async_shell_command_leaves_the_original_buffer_visible() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "M-!").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, "echo hello-m103");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let finished = pump_until(
        &mut i,
        std::time::Duration::from_secs(5),
        no_shell_procs_running,
    );
    assert!(finished, "shell process never finished");

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Shell Command Output*\"");
}

#[test]
fn compile_leaves_the_original_buffer_visible() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, "echo compiling-m103");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let finished = pump_until(
        &mut i,
        std::time::Duration::from_secs(5),
        no_compile_procs_running,
    );
    assert!(finished, "compile process never finished");

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*compilation*\"");
}

#[test]
fn search_leaves_the_original_buffer_visible() {
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    // A literal grep for a string that will not be found is enough to
    // exercise the display path without depending on `rg` being
    // installed or any real repository content -- the "no matches"
    // report still goes through `shell-command--maybe-show`.
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-project");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, "zzz-no-such-token-m103-zzz");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let finished = pump_until(
        &mut i,
        std::time::Duration::from_secs(5),
        no_search_procs_running,
    );
    assert!(finished, "search process never finished");

    assert_eq!(run(&mut i, "(window-count)"), "2");
    assert!(window_buffer_map(&mut i).contains("\"alu.sv\""));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*search*\"");
}

// =======================================================================
// 13. Fix round (mutation-testing gap): step 1 (reuse a window already
// showing the buffer) and step 4's "prefer another helper window"
// clause both survived mutation with only two windows in play, because
// with two windows the window `window--next-after-selected' would pick
// is ALSO the one either of those two clauses would pick -- the
// clauses only diverge from the "next after selected" fallback with
// THREE windows. Both tests below are built with three.
// =======================================================================

#[test]
fn display_buffer_reuses_the_window_already_showing_it_even_when_a_different_window_is_next() {
    // Layout: A (id 0, selected, buffer F), B (id 1, buffer Y), C (id 2,
    // buffer X). `window--next-after-selected' from A lands on B (the
    // next id after 0), NOT on C -- so if step 1 (reuse a window
    // already showing the requested buffer) were disabled, `display-
    // buffer' would fall through to the "next after selected"/"only
    // window" logic and evict B's buffer Y instead of returning the
    // window that already shows X untouched.
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"F\")");
    ok(&mut i, "(switch-to-buffer-internal \"F\")");
    ok(&mut i, "(split-window-internal nil)"); // id 1, clone of F
    ok(&mut i, "(split-window-internal t)"); // id 2, clone of F
    assert_eq!(run(&mut i, "(window-count)"), "3");
    assert_eq!(
        run(&mut i, "(selected-window)"),
        "0",
        "splitting must never move selection off the original window"
    );
    ok(&mut i, "(get-buffer-create \"Y\")");
    ok(&mut i, "(get-buffer-create \"X\")");
    ok(&mut i, "(set-window-buffer 1 \"Y\")");
    ok(&mut i, "(set-window-buffer 2 \"X\")");
    // Sanity check on the layout itself: confirms the two policies
    // (step 1's "reuse existing" vs. the "next after selected"
    // fallback) actually diverge here -- next-after-selected(A) is B
    // (id 1), while X already lives in C (id 2), a DIFFERENT window.
    assert_eq!(
        run(&mut i, "(window--next-after-selected)"),
        "1",
        "precondition: next-after-selected from A must be B, not C, or \
         this test can't tell step 1 apart from the fallback"
    );

    let result = ok(&mut i, "(display-buffer \"X\")");

    assert_eq!(
        result, "2",
        "display-buffer must reuse C (already showing X), not evict B"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 1) (lambda () (buffer-name)))"
        ),
        "\"Y\"",
        "B must still show Y -- untouched"
    );
    assert_eq!(
        run(&mut i, "(window-count)"),
        "3",
        "must not create a fourth window either"
    );
}

#[test]
fn display_buffer_prefers_another_helper_window_over_the_next_after_selected_fallback() {
    // Layout built in a deliberately non-obvious ORDER (see inline
    // comments) so that `window--next-after-selected' from the final
    // selected window (A) does NOT land on the flagged helper window H
    // -- if it did, this test couldn't distinguish step 4's "prefer
    // another window already flagged `window--created-for-display'"
    // clause from the plain "next after selected" fallback, since both
    // would agree on the answer (exactly the trap the coordinator
    // warned about: a naive C-h b then C-x 3 order makes H the very
    // next id after A and both policies collapse onto it).
    //
    // Final layout: B (id 0, buffer Y), H (id 1, buffer *Help*,
    // flagged `window--created-for-display' = 1), A (id 2, selected,
    // buffer F). Built by: start with a single window (required for
    // `pop-to-buffer' to actually SPLIT rather than reuse), `pop-to-
    // buffer' *Help* there (creates H as id 1 and selects it), split H
    // itself to create id 2 (becomes A), select id 2 and put F there,
    // and only THEN repurpose the original id-0 window (never
    // reselected since) into B with buffer Y. This makes A (id 2) the
    // LAST window in sorted order, so `window--next-after-selected'
    // WRAPS AROUND to the smallest id (0, which is B) instead of
    // landing on H (id 1) -- the two policies now disagree.
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(window-count)"), "1");
    let help_win = ok(&mut i, "(pop-to-buffer \"*Help*\")"); // id 1, selected
    assert_eq!(help_win, "1");
    assert_eq!(run(&mut i, "(selected-window)"), "1");

    ok(&mut i, "(split-window-internal t)"); // id 2, clone of *Help*, H stays selected
    ok(&mut i, "(get-buffer-create \"F\")");
    ok(&mut i, "(set-window-buffer 2 \"F\")");
    ok(&mut i, "(select-window 2)"); // A = id 2, now selected

    ok(&mut i, "(get-buffer-create \"Y\")");
    ok(&mut i, "(set-window-buffer 0 \"Y\")"); // B = id 0

    assert_eq!(run(&mut i, "(window-count)"), "3");
    assert_eq!(run(&mut i, "(selected-window)"), "2");
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 0) (lambda () (buffer-name)))"
        ),
        "\"Y\""
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 1) (lambda () (buffer-name)))"
        ),
        "\"*Help*\""
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 2) (lambda () (buffer-name)))"
        ),
        "\"F\""
    );
    // Precondition: confirms the divergence documented above -- from A
    // (id 2, the LAST id), next-after-selected wraps to id 0 (B), not
    // id 1 (H).
    assert_eq!(
        run(&mut i, "(window--next-after-selected)"),
        "0",
        "precondition: next-after-selected from A must wrap to B, not H, \
         or this test can't tell step 4's helper-window preference apart \
         from the fallback"
    );

    let result = ok(&mut i, "(display-buffer \"Z\")");

    assert_eq!(
        result, "1",
        "display-buffer must reuse H (already flagged as a helper window), \
         not evict B via the next-after-selected fallback"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 0) (lambda () (buffer-name)))"
        ),
        "\"Y\"",
        "B must still show Y -- untouched"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal (window-buffer 2) (lambda () (buffer-name)))"
        ),
        "\"F\"",
        "A must still show F -- display-buffer never touches the selected window"
    );
    assert_eq!(
        run(&mut i, "(window-count)"),
        "3",
        "must not create a fourth window either"
    );
}

// =======================================================================
// 14. Fix round (cold-review defect 1): step 2's window reuse must not
// let the OLD buffer's point clobber the NEW buffer's point via
// `editor::select_window`'s unconditional save-back.
// =======================================================================

#[test]
fn step_2_reuse_preserves_the_new_buffers_own_point_not_the_old_buffers() {
    // Exact repro from the coordinator's real-code trace: *Help*'s
    // point is 3, the second buffer's point is 22 (point-max after a
    // 21-char insert) -- before the fix, `pop-to-buffer` on the second
    // buffer (which goes through display-buffer step 2, reusing the
    // SELECTED helper window `*Help*` is already in) left `(point)` at
    // 3, not 22.
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    // `describe-bindings` (not a bare `pop-to-buffer` on an empty
    // buffer) so `*Help*` actually has enough real content for
    // `goto-char 3` to land somewhere meaningful -- an empty buffer
    // clamps any goto-char to 1, which would make this test pass for
    // the wrong reason.
    ok(&mut i, "(describe-bindings)");
    let help_win = run(&mut i, "(get-buffer-window \"*Help*\")");
    assert_ne!(help_win, "nil");
    ok(&mut i, "(goto-char 3)");
    assert_eq!(run(&mut i, "(point)"), "3");

    ok(&mut i, "(get-buffer-create \"*Out*\")");
    ok(&mut i, "(with-current-buffer-internal \"*Out*\" (lambda () (insert \"AAAAAAAAAAAAAAAAAAAAA\") (goto-char (point-max))))");
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer-internal \"*Out*\" (lambda () (point)))"
        ),
        "22"
    );

    let win2 = ok(&mut i, "(pop-to-buffer \"*Out*\")");

    assert_eq!(
        win2, help_win,
        "step 2 must reuse the same (selected) helper window"
    );
    assert_eq!(
        run(&mut i, "(point)"),
        "22",
        "the NEW buffer's own point must survive the window reuse -- it \
         must NOT be clobbered by *Help*'s point (3)"
    );
}

// =======================================================================
// 15. Fix round (cold-review defect 2): `q` must not delete a window it
// did not create, even when the flag names a DIFFERENT window that
// also happens to show the same buffer.
// =======================================================================

#[test]
fn q_does_not_delete_the_users_own_window_when_flag_names_a_different_window() {
    // Repro: `C-h b' splits off window B for `*Help*' (flag -> B);
    // `C-x o' moves selection back to the ORIGINAL window A; `C-x b'
    // switches A's buffer to `*Help*' directly (`switch-to-buffer',
    // NOT `display-buffer' -- the flag is never touched by this path);
    // `q' is pressed with A selected. Without the `(eq window--
    // created-for-display (selected-window))' guard added to `quit-
    // source-return''s first branch, the first branch would have fired
    // (flag non-nil, A shows the current buffer, window-count > 1) and
    // deleted A -- the user's OWN window, not the one this mechanism
    // created.
    //
    // What this test asserts is exactly what was asked: A itself must
    // survive as a window. It deliberately does NOT assert `buffer-
    // name' after `q', because the SECOND (flag-named-window) branch's
    // own condition is independently satisfied here too (B still shows
    // `*Help*', and current-buffer -- A's buffer -- is ALSO `*Help*',
    // so the two are `eq'), so `q' instead deletes B and leaves A on
    // screen still showing `*Help*' rather than switching back to
    // `quit-source' in place -- worth recording honestly since it
    // means this specific window-count-2 case never reaches the
    // fallback branch either.
    let (mut i, ed) = setup();
    ok(&mut i, "(get-buffer-create \"alu.sv\")");
    ok(&mut i, "(switch-to-buffer-internal \"alu.sv\")");
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(run(&mut i, "(window-count)"), "2");
    let window_a = 0i64; // the original window is never assigned a new id

    feed_keys(&mut i, &ed, "C-x o").unwrap();
    assert_eq!(run(&mut i, "(selected-window)"), window_a.to_string());
    assert_eq!(run(&mut i, "(buffer-name)"), "\"alu.sv\"");

    feed_keys(&mut i, &ed, "C-x b").unwrap();
    type_str(&mut i, &ed, "*Help*");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*Help*\"");
    assert_eq!(run(&mut i, "(selected-window)"), window_a.to_string());

    feed_keys(&mut i, &ed, "q").unwrap();

    let remaining = parse_window_ids(&run(&mut i, "(window-list)"));
    assert!(
        remaining.contains(&window_a),
        "q must not delete window A (the user's own window, not one \
         this mechanism created), remaining: {:?}",
        remaining
    );
}
