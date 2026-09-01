//! M45 (evil wave 3): the `C-w` window prefix and the `g`-series
//! motions/operators -- see evil.el's "C-w window prefix (M45)"
//! section and the M45 entries scattered through the `g`-prefix
//! keymap-population section (`g d`/`g _`/`g e`/`g E`/`g v`/`g i`/
//! `g J`/`g ~`/`g u`/`g U`). `setup_evil`/`feed`/`run`/`bs`/`pt`/
//! `echo_row_text` mirror evil_marks_registers_tests.rs's own helpers
//! exactly (same setup/run/feed_keys pattern, copied rather than
//! shared since integration test binaries can't import each other's
//! private helpers); `row_text`/`find_row` mirror evil_foundation_
//! tests.rs's grid helpers, `row_cols` mirrors completion_popup_
//! tests.rs's own (needed to tell a split window's own half of a row
//! apart from its neighbor's).

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use core::redisplay::{render, Grid};
use elisp::{Interp, Value};

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (50, 8);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => elisp::printer::prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// A fresh buffer preloaded with TEXT, point at the start, evil-mode on.
fn setup_evil(text: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", text));
    run(&mut i, "(goto-char (point-min))");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    (i, ed)
}

fn feed(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, keys: &str) {
    feed_keys(interp, ed, keys).unwrap_or_else(|e| panic!("feed_keys {:?}: {}", keys, e));
}

/// Raw buffer content (unlike `run`, not the `prin1` printed form --
/// needed to compare multi-line text without hand-escaping newlines).
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

/// The echo area's rendered text (mirrors evil_tests.rs's own helper).
fn echo_row_text(interp: &Interp, ed: &Rc<RefCell<Editor>>) -> String {
    let grid = render(interp, ed);
    let row = grid.rows - 1;
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// One grid row's text, trailing spaces trimmed (mirrors
/// evil_foundation_tests.rs's/gui_features_tests.rs's own `row_text`).
fn row_text(grid: &Grid, row: usize) -> String {
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The (first) row whose rendered text contains `needle` (mirrors
/// evil_foundation_tests.rs's/modes_tests.rs's own `find_row`).
fn find_row(grid: &Grid, needle: &str) -> usize {
    (0..grid.lines.len())
        .find(|&r| row_text(grid, r).contains(needle))
        .unwrap_or_else(|| {
            let dump: Vec<String> = (0..grid.lines.len()).map(|r| row_text(grid, r)).collect();
            panic!("no row contains {:?}; grid:\n{}", needle, dump.join("\n"))
        })
}

/// Characters in one grid row between columns `[start, end)`, trailing
/// spaces NOT trimmed -- needed to inspect just one split window's own
/// slice of a row without the neighboring window's content on the same
/// row washing out a `trim_end` (mirrors completion_popup_tests.rs's
/// own `row_cols`).
fn row_cols(
    interp: &Interp,
    ed: &Rc<RefCell<Editor>>,
    row: usize,
    start: usize,
    end: usize,
) -> String {
    let grid = render(interp, ed);
    grid.lines[row][start..end]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect()
}

// =======================================================================
// Part A: the `C-w' window prefix
// =======================================================================

#[test]
fn ctrl_w_s_splits_the_frame_stacked_top_and_bottom() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "C-w s");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    let grid = render(&i, &ed);
    let top = find_row(&grid, "hello");
    let bottom = (top + 1..grid.lines.len()).find(|&r| row_text(&grid, r).contains("hello"));
    assert!(
        bottom.is_some(),
        "the SAME buffer's text must appear in a SECOND row below the first \
         (stacked, not side-by-side): grid = {:?}",
        (0..grid.lines.len())
            .map(|r| row_text(&grid, r))
            .collect::<Vec<_>>()
    );
}

#[test]
fn ctrl_w_v_splits_the_frame_side_by_side() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "C-w v");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    // frame (50, 8): a 1-col separator, left width (50-1)/2 = 24, right
    // starts at col 25 -- see `redisplay::compute_rects''s own math.
    assert!(row_cols(&i, &ed, 0, 0, 24).contains("hello"), "left pane");
    assert!(row_cols(&i, &ed, 0, 25, 50).contains("hello"), "right pane");
}

#[test]
fn ctrl_w_w_and_ctrl_w_ctrl_w_cycle_the_selected_window() {
    let (mut i, ed) = setup_evil("");
    run(&mut i, "(switch-to-buffer \"one\")");
    run(&mut i, "(split-window-below)");
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer \"two\")");
    // `switch-to-buffer' just now ran via a raw `eval_source' call, not
    // through the real command-dispatch pipeline -- so post-command-hook
    // (and with it, `evil--maybe-init-current-buffer''s lazy per-buffer
    // `evil--state' init) never fired for the new buffer "two". Calling
    // `(evil-mode 1)' again is a harmless, idempotent no-op as far as
    // the ALREADY-initialized buffers go, but it EAGERLY (re)initializes
    // every buffer in `(buffer-list)' -- "two" included -- so the
    // `feed_keys' calls below dispatch through evil's own keymaps
    // rather than whatever "two" would otherwise still have (nothing).
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(other-window 1)"); // back to "one" (still selected)
    assert_eq!(run(&mut i, "(buffer-name)"), "\"one\"");

    feed(&mut i, &ed, "C-w w");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"two\"");
    feed(&mut i, &ed, "C-w w");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"one\"");
    feed(&mut i, &ed, "C-w C-w");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"two\"");
}

#[test]
fn ctrl_w_c_deletes_the_window_and_errors_on_the_sole_survivor() {
    let (mut i, ed) = setup_evil("hello");
    run(&mut i, "(split-window-below)");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    feed(&mut i, &ed, "C-w c");
    assert_eq!(run(&mut i, "(window-count)"), "1");

    // vim's E444: deleting the LAST window must error (echoed, not a
    // panic) and leave the window untouched -- see `delete-window'
    // (builtins/ui.rs).
    feed(&mut i, &ed, "C-w c");
    assert_eq!(
        run(&mut i, "(window-count)"),
        "1",
        "the sole window must survive"
    );
    assert!(
        echo_row_text(&i, &ed).contains("Attempt to delete the sole ordinary window"),
        "expected the sole-window error echoed, got {:?}",
        echo_row_text(&i, &ed)
    );
}

#[test]
fn ctrl_w_q_closes_just_the_window_with_several_open() {
    // The sole-window-quit unsaved-changes guard is already covered by
    // evil_tests.rs's own `ex_q_*' tests (`evil-ex-quit' is the same
    // function `:q' already runs) -- this only needs to pin that `C-w
    // q' reaches `evil-ex-quit' at all, via the multi-window case.
    let (mut i, ed) = setup_evil("hello");
    run(&mut i, "(split-window-below)");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    feed(&mut i, &ed, "C-w q");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert!(
        !ed.borrow().quit,
        "closing one of several windows via C-w q must not quit the editor"
    );
}

#[test]
fn ctrl_w_o_keeps_only_the_selected_window() {
    let (mut i, ed) = setup_evil("");
    run(&mut i, "(switch-to-buffer \"keep\")");
    run(&mut i, "(split-window-below)");
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer \"gone\")");
    run(&mut i, "(evil-mode 1)"); // see the C-w w/C-w C-w test's comment
    run(&mut i, "(other-window 1)"); // back to "keep"
    assert_eq!(run(&mut i, "(buffer-name)"), "\"keep\"");
    assert_eq!(run(&mut i, "(window-count)"), "2");

    feed(&mut i, &ed, "C-w o");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"keep\"");
}

#[test]
fn ctrl_w_l_and_h_move_across_a_side_by_side_split() {
    let (mut i, ed) = setup_evil("");
    run(&mut i, "(switch-to-buffer \"left\")");
    run(&mut i, "(split-window-right)"); // C-x 3 equivalent
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer \"right\")");
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(other-window 1)"); // back to "left"
    assert_eq!(run(&mut i, "(buffer-name)"), "\"left\"");

    feed(&mut i, &ed, "C-w l");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"right\"");
    feed(&mut i, &ed, "C-w h");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"left\"");
}

#[test]
fn ctrl_w_j_and_k_move_across_a_stacked_split() {
    let (mut i, ed) = setup_evil("");
    run(&mut i, "(switch-to-buffer \"top\")");
    run(&mut i, "(split-window-below)"); // C-x 2 equivalent
    run(&mut i, "(other-window 1)");
    run(&mut i, "(switch-to-buffer \"bottom\")");
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(other-window 1)"); // back to "top"
    assert_eq!(run(&mut i, "(buffer-name)"), "\"top\"");

    feed(&mut i, &ed, "C-w j");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"bottom\"");
    feed(&mut i, &ed, "C-w k");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"top\"");
}

#[test]
fn ctrl_w_hjkl_walk_a_full_loop_around_a_nested_2x2_grid() {
    // Mutation check (M45 spec): this nested layout is what actually
    // exercises `window_in_direction''s overlap tie-break (builtins/
    // ui.rs) rather than letting the tie-break's OWN fallback (smallest
    // id) accidentally produce the right answer regardless -- in
    // particular the BR -h-> BL leg below: with `tl=0 tr=1 br=2 bl=3'
    // (this construction's own id assignment order), a "drop the
    // overlap check, just take the smallest id" mutation would send
    // that leg to `tl' (id 0) instead of `bl' (id 3), and that
    // assertion would catch it.
    let (mut i, ed) = setup_evil("");
    ed.borrow_mut().frame = (60, 20); // headroom for four panes
    run(&mut i, "(switch-to-buffer \"tl\")");
    run(&mut i, "(split-window-right)"); // tl(sel) | (copy)         id0 id1
    run(&mut i, "(other-window 1)"); // -> id1
    run(&mut i, "(switch-to-buffer \"tr\")"); // tl | tr
    run(&mut i, "(split-window-below)"); // tr(sel) splits down       id2
    run(&mut i, "(other-window 1)"); // id1 -> id2
    run(&mut i, "(switch-to-buffer \"br\")"); // tl | tr / br
    run(&mut i, "(other-window 1)"); // id2 -> id0 ((0-2) mod 3 = 1)
    run(&mut i, "(split-window-below)"); // tl(sel) splits down       id3
    run(&mut i, "(other-window 3)"); // id0 -> id3 ((3-0) mod 4 = 3)
    run(&mut i, "(switch-to-buffer \"bl\")"); // tl/bl | tr/br
    run(&mut i, "(evil-mode 1)"); // see the C-w w/C-w C-w test's comment
    run(&mut i, "(other-window 1)"); // id3 -> id0 ((0-3) mod 4 = 1)
    assert_eq!(run(&mut i, "(buffer-name)"), "\"tl\"");
    assert_eq!(run(&mut i, "(window-count)"), "4");

    feed(&mut i, &ed, "C-w l");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"tr\"", "TL -l-> TR");
    feed(&mut i, &ed, "C-w j");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"br\"", "TR -j-> BR");
    feed(&mut i, &ed, "C-w h");
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"bl\"",
        "BR -h-> BL (the overlap tie-break leg, see this test's own comment)"
    );
    feed(&mut i, &ed, "C-w k");
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"tl\"",
        "BL -k-> TL, back to the start"
    );
}

#[test]
fn ctrl_w_direction_with_no_window_there_messages_and_leaves_selection_alone() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "C-w h"); // only window: nothing to its left
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert_eq!(echo_row_text(&i, &ed), "No window left");
}

#[test]
fn normal_state_ctrl_w_is_the_window_prefix_not_kill_region() {
    let (mut i, ed) = setup_evil("hello world");
    run(&mut i, "(set-mark 1)");
    run(&mut i, "(goto-char 6)"); // mark..point = "hello", an active region
    feed(&mut i, &ed, "C-w s");
    assert_eq!(
        bs(&mut i),
        "hello world",
        "normal-state C-w must not kill the region"
    );
    assert_eq!(
        run(&mut i, "(window-count)"),
        "2",
        "... it must be the window-split prefix instead"
    );
}

#[test]
fn insert_state_ctrl_w_still_falls_back_to_the_global_kill_region() {
    // Pins the CURRENT (v1) scope, per evil.el's own file-header note:
    // vim's `i_CTRL-W' (delete word before point) is not implemented,
    // so insert state's C-w is deliberately left bound to whatever the
    // global keymap already gives it (`kill-region', simple.el) --
    // `evil--insert-map' never binds C-w to anything of its own (see
    // the C-w window-prefix section's own header comment).
    let (mut i, ed) = setup_evil("hello world");
    run(&mut i, "(set-mark 1)");
    run(&mut i, "(goto-char 6)"); // region = "hello"
    feed(&mut i, &ed, "i");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "C-w");
    assert_eq!(
        bs(&mut i),
        " world",
        "insert-state C-w must still kill the active region via the global binding"
    );
}

// =======================================================================
// Part B: the `g'-series motions and operators
// =======================================================================

// --- g_ (last non-blank) ------------------------------------------------

#[test]
fn g_underscore_lands_on_the_last_non_blank_of_the_line() {
    let (mut i, ed) = setup_evil("hello   \nworld");
    feed(&mut i, &ed, "g _");
    assert_eq!(
        pt(&mut i),
        5,
        "must land ON 'o', skipping the trailing spaces"
    );
}

#[test]
fn count_before_g_underscore_moves_down_first() {
    let (mut i, ed) = setup_evil("hello   \nworld");
    feed(&mut i, &ed, "2 g _");
    assert_eq!(pt(&mut i), 14, "line 2's own last non-blank ('d' of world)");
}

#[test]
fn d_g_underscore_deletes_inclusive_of_the_last_non_blank_char() {
    let (mut i, ed) = setup_evil("hello   \nworld");
    feed(&mut i, &ed, "d g _");
    assert_eq!(
        bs(&mut i),
        "   \nworld",
        "must delete THROUGH 'o'; trailing spaces stay"
    );
    assert_eq!(pt(&mut i), 1);
}

// --- ge / gE (backward to the end of the previous word/WORD) -----------

#[test]
fn ge_lands_on_the_end_of_the_previous_word_stopping_at_a_punctuation_run() {
    // "abc foo.bar": word-wise, '.' is its own (punct) run -- `ge' from
    // "bar" must land ON the '.', not skip past it to "foo".
    let (mut i, ed) = setup_evil("abc foo.bar");
    run(&mut i, "(goto-char 9)"); // 'b' of "bar"
    feed(&mut i, &ed, "g e");
    assert_eq!(pt(&mut i), 8, "must land on the '.' itself");
}

#[test]
fn g_capital_e_treats_the_punctuation_run_as_part_of_one_whole_word() {
    // Same fixture, WORD-wise this time: "foo.bar" is ONE WORD (only
    // whitespace separates WORDs), so `gE' must skip all of it and land
    // on the end of "abc" -- the ge/gE contrast this fixture exists for.
    let (mut i, ed) = setup_evil("abc foo.bar");
    run(&mut i, "(goto-char 9)"); // 'b' of "bar"
    feed(&mut i, &ed, "g E");
    assert_eq!(
        pt(&mut i),
        3,
        "must skip the whole 'foo.bar' WORD to the end of 'abc'"
    );
}

#[test]
fn count_before_ge_repeats_it() {
    let (mut i, ed) = setup_evil("one two three");
    run(&mut i, "(goto-char 13)"); // the last 'e' of "three"
    feed(&mut i, &ed, "2 g e");
    assert_eq!(
        pt(&mut i),
        3,
        "2ge must land on the end of 'one', skipping 'two'"
    );
}

#[test]
fn d_g_e_deletes_inclusive_back_to_the_previous_words_end() {
    let (mut i, ed) = setup_evil("foo bar");
    run(&mut i, "(goto-char 5)"); // 'b' of "bar"
    feed(&mut i, &ed, "d g e");
    assert_eq!(bs(&mut i), "foar");
    assert_eq!(pt(&mut i), 3);
}

// --- gv (reselect the last visual selection) ----------------------------

#[test]
fn gv_reselects_the_last_char_visual_selection() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v l l l"); // mark=1, point 1->2->3->4
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "ESC");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    run(&mut i, "(goto-char 1)"); // move point away before reselecting

    feed(&mut i, &ed, "g v");
    assert_eq!(run(&mut i, "evil--state"), "visual");
    assert_eq!(run(&mut i, "evil--visual-type"), "char");
    assert_eq!(run(&mut i, "(mark)"), "1");
    assert_eq!(pt(&mut i), 4);
}

#[test]
fn gv_reselects_the_last_linewise_visual_selection() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree");
    feed(&mut i, &ed, "V j"); // linewise, mark on line1, point -> line2
    feed(&mut i, &ed, "ESC");
    run(&mut i, "(goto-char (point-max))");

    feed(&mut i, &ed, "g v");
    assert_eq!(run(&mut i, "evil--state"), "visual");
    assert_eq!(run(&mut i, "evil--visual-type"), "line");
}

#[test]
fn gv_after_a_visual_operator_still_restores_via_the_apply_path_snapshot() {
    // The M45 spec's own trap: `evil--visual-apply' (the operator path
    // -- visual `d'/`c'/`y'/`gu'/...) never calls `evil--visual-exit' at
    // all, so it needs its OWN identical snapshot (see `evil--last-
    // visual''s docstring). Dropping that second snapshot site is
    // exactly the mutation this test exists to catch.
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "v l l"); // mark=1, point 1->2->3
    assert_eq!(pt(&mut i), 3);
    feed(&mut i, &ed, "d"); // visual delete: never touches evil--visual-exit
    assert_eq!(bs(&mut i), "def");
    assert_eq!(run(&mut i, "evil--state"), "normal");

    feed(&mut i, &ed, "g v");
    assert_eq!(
        run(&mut i, "evil--state"),
        "visual",
        "gv must work after an operator ended visual state, not just after ESC"
    );
    assert_eq!(run(&mut i, "(mark)"), "1");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn gv_with_no_prior_visual_selection_messages() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "g v");
    assert_eq!(echo_row_text(&i, &ed), "No previous visual selection");
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

// --- gi (resume the last insert) ----------------------------------------

#[test]
fn gi_resumes_insert_at_the_last_exit_position() {
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i a b c");
    assert_eq!(bs(&mut i), "abc");
    feed(&mut i, &ed, "ESC");
    assert_eq!(
        pt(&mut i),
        3,
        "ESC backs up one column off the insert-exit position"
    );
    run(&mut i, "(goto-char 1)");

    feed(&mut i, &ed, "g i");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    assert_eq!(
        pt(&mut i),
        4,
        "gi must resume exactly where insert was left, not the ESC-adjusted column"
    );
    feed(&mut i, &ed, "d");
    assert_eq!(bs(&mut i), "abcd");
}

#[test]
fn gi_with_no_prior_insert_session_behaves_like_plain_i() {
    let (mut i, ed) = setup_evil("hello");
    run(&mut i, "(goto-char 3)");
    feed(&mut i, &ed, "g i");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    assert_eq!(
        pt(&mut i),
        3,
        "no history: point must stay put, same as plain `i'"
    );
}

#[test]
fn dot_repeat_after_gi_inserts_at_point_not_the_stale_last_insert_pos() {
    // Review fix (M45 wave-3 follow-up #1): `g i''s dot-repeat REPLAY
    // entry must be `evil-insert' (insert wherever point already is when
    // `.' runs), NOT `evil-goto-last-insert' itself. `evil--last-insert-
    // pos' is a buffer-local var this SAME session's own exit overwrites
    // (`evil--finish-insert-session' sets it unconditionally), so
    // replaying `evil-goto-last-insert' would re-read a stale, self-
    // referential absolute position instead of continuing at `.''s own
    // point -- unlike every other i/a/I/A/o/O replay entry in this file,
    // which all recompute their target from the CURRENT point.
    let (mut i, ed) = setup_evil("xy");
    feed(&mut i, &ed, "g i"); // no history yet: point stays at 1 (see the test above)
    feed(&mut i, &ed, "A");
    feed(&mut i, &ed, "ESC");
    assert_eq!(bs(&mut i), "Axy");
    // `evil--last-insert-pos' is now 2 -- where this session's ESC ran.
    run(&mut i, "(goto-char 3)"); // between 'x' and 'y'

    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "AxAy",
        "dot-repeat must insert at point (3); replaying `evil-goto-last-\
         insert' instead jumps back to the stale last-insert-pos (2) and \
         produces \"AAxy\""
    );
}

// --- gJ (join without inserting a space) ---------------------------------

#[test]
fn g_capital_j_joins_without_inserting_a_space() {
    let (mut i, ed) = setup_evil("hello\n   world");
    feed(&mut i, &ed, "g J");
    assert_eq!(
        bs(&mut i),
        "hello   world",
        "gJ deletes only the newline itself, no whitespace cleanup"
    );
}

#[test]
fn capital_j_inserts_a_space_for_contrast_with_g_capital_j() {
    let (mut i, ed) = setup_evil("hello\n   world");
    feed(&mut i, &ed, "J");
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn count_before_g_capital_j_joins_that_many_lines() {
    let (mut i, ed) = setup_evil("a\nb\nc\nd\ne");
    feed(&mut i, &ed, "3 g J"); // n = max(2,3) = 3, joins 2 times
    assert_eq!(bs(&mut i), "abc\nd\ne");
}

// --- g~ / gu / gU (case operators) ---------------------------------------

#[test]
fn guw_downcases_a_word() {
    let (mut i, ed) = setup_evil("HELLO world");
    feed(&mut i, &ed, "g u w");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn g_capital_u_w_upcases_a_word() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "g U w");
    assert_eq!(bs(&mut i), "HELLO world");
}

#[test]
fn g_tilde_w_toggles_the_case_of_a_word() {
    let (mut i, ed) = setup_evil("Hello world");
    feed(&mut i, &ed, "g ~ w");
    assert_eq!(bs(&mut i), "hELLO world");
}

#[test]
fn count_before_guw_extends_the_word_motion() {
    let (mut i, ed) = setup_evil("ONE TWO THREE FOUR FIVE");
    feed(&mut i, &ed, "3 g u w");
    assert_eq!(bs(&mut i), "one two three FOUR FIVE");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn guu_downcases_the_whole_line() {
    let (mut i, ed) = setup_evil("Hello World");
    feed(&mut i, &ed, "g u u");
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn g_capital_u_u_upcases_the_whole_line() {
    let (mut i, ed) = setup_evil("Hello World");
    feed(&mut i, &ed, "g U U");
    assert_eq!(bs(&mut i), "HELLO WORLD");
}

#[test]
fn g_tilde_tilde_toggles_the_whole_line() {
    let (mut i, ed) = setup_evil("Hello World");
    feed(&mut i, &ed, "g ~ ~");
    assert_eq!(bs(&mut i), "hELLO wORLD");
}

#[test]
fn gugu_spelling_doubles_the_same_as_guu() {
    let (mut i, ed) = setup_evil("Hello World");
    feed(&mut i, &ed, "g u g u");
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn visual_line_g_capital_u_upcases_the_selected_lines() {
    let (mut i, ed) = setup_evil("hello\nworld");
    feed(&mut i, &ed, "V g U");
    assert_eq!(bs(&mut i), "HELLO\nworld");
}

#[test]
fn du_is_guarded_and_does_not_fall_through_to_a_whole_line_delete() {
    // The M45 spec's other named trap: `u'/`U'/`~' are bound at the TOP
    // LEVEL of `evil--op-pending-map' (vim's `guu'/`gUU'/`g~~' doubling
    // shortcuts), a keymap slot shared across every pending operator --
    // without `evil--case-doubled''s guard on `evil--pending-operator'
    // actually being a case op, `du' (`u' is not a motion or text
    // object here) would silently delete the WHOLE current line instead
    // of just cancelling the pending `delete', the same as any other
    // unclaimed operator-pending key does.
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d u");
    assert_eq!(
        bs(&mut i),
        "hello world",
        "the guard must cancel, not fall through to a whole-line delete"
    );
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(run(&mut i, "evil--pending-operator"), "nil");
    assert!(
        !echo_row_text(&i, &ed).is_empty(),
        "expected a cancellation message"
    );
}

#[test]
fn g_capital_u_w_then_dot_repeat_reapplies_elsewhere() {
    let (mut i, ed) = setup_evil("hello world foo");
    feed(&mut i, &ed, "g U w");
    assert_eq!(bs(&mut i), "HELLO world foo");
    run(&mut i, "(goto-char 7)"); // start of "world"
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "HELLO WORLD foo");
}

#[test]
fn case_operators_never_touch_the_kill_ring() {
    let (mut i, ed) = setup_evil("aaa\nbbb");
    feed(&mut i, &ed, "\" a y y"); // register a = unnamed kill-ring = "aaa\n"
    feed(&mut i, &ed, "j");
    feed(&mut i, &ed, "g u w"); // no visible change, but must not touch the kill-ring
    feed(&mut i, &ed, "p");
    assert_eq!(
        bs(&mut i),
        "aaa\nbbb\naaa\n",
        "plain p must still paste the EARLIER yy's content, proving guw left the kill-ring alone"
    );
}

#[test]
fn case_operators_consume_but_never_write_a_pending_named_register() {
    // Pins the case-op branch's own local invariant (see its comment in
    // evil.el): a pending named-register prefix (`\"a') is consumed by
    // the SAME `evil--operator-apply' call that used it, written or not
    // -- vim's case operators have no register form, so it must never
    // end up written. NOTE: the bare `(setq-local evil--pending-
    // register nil)' this branch does is, empirically, redundant with
    // `evil--post-command''s own pre-existing (M42) stale-register
    // safety net -- that net already clears any leftover pending-
    // register on the very next post-command-hook cycle whenever no
    // operator is left pending and state isn't visual, which is always
    // true immediately after a case op. So this specific line's removal
    // is NOT independently observable through feed_keys black-box
    // testing (verified directly); this test still pins the correct
    // externally-observed behavior (nil afterward, never written), and
    // a more severe mutation of the SAME bug class -- breaking the
    // `(memq op '(toggle-case downcase upcase))' dispatch so case ops
    // fall through to the `t' (delete) branch instead -- IS caught, by
    // this test and nine others in this file (it starts touching the
    // kill-ring and the buffer content wrongly).
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "\" a g u w");
    assert_eq!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "the armed register must be consumed by the case-op apply"
    );
    feed(&mut i, &ed, "\" a p");
    assert_eq!(
        echo_row_text(&i, &ed),
        "Nothing in register a",
        "vim's case operators never write a register"
    );
}

// --- gd (go to definition) ------------------------------------------------

#[test]
fn gd_with_no_lsp_client_messages() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "g d");
    // The frame is only 50 columns wide (`setup`'s default), so the
    // full message is clipped in the echo row -- `contains` on the
    // stable prefix, same as the other echoed-error assertions above.
    assert!(
        echo_row_text(&i, &ed).contains("No LSP server connected in this buffer"),
        "got {:?}",
        echo_row_text(&i, &ed)
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "gd must not enter op-pending or visual state"
    );
}
