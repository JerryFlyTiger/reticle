//! M42 Part I: vim marks (`m`/`` ` ``/`'`) and named registers
//! (`"a`-`"z`) -- see evil.el's "M42: marks"/"M42: named registers"
//! sections. `setup_evil`/`feed`/`bs`/`pt`/`echo_row_text` mirror
//! evil_tests.rs's own helpers exactly (same setup/run/feed_keys
//! pattern, copied rather than shared since integration test binaries
//! can't import each other's private helpers).

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use core::redisplay::render;
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

// ---------------------------------------------------------------------
// Part I-A: marks (m / ` / ')
// ---------------------------------------------------------------------

#[test]
fn backtick_mark_survives_an_edit_before_it_via_marker_adjustment() {
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "l l l"); // point -> 4 ('d')
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "m a");
    feed(&mut i, &ed, "0"); // back to point 1
    assert_eq!(pt(&mut i), 1);
    // Insert text BEFORE the mark: a plain marker (not a frozen
    // integer) must shift along with it.
    run(&mut i, "(insert \"XY\")");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "` a");
    assert_eq!(bs(&mut i), "XYabcdef");
    assert_eq!(
        pt(&mut i),
        6,
        "the mark must have shifted with the insertion and still land on 'd'"
    );
}

#[test]
fn quote_mark_lands_on_the_first_non_blank_of_its_line() {
    let (mut i, ed) = setup_evil("first\n   indented\nthird");
    // Positions: f(1)i(2)r(3)s(4)t(5)\n(6)
    //            sp(7)sp(8)sp(9)i(10)n(11)d(12)e(13)n(14)t(15)e(16)d(17)\n(18)
    //            t(19)h(20)i(21)r(22)d(23)   point-max=24
    run(&mut i, "(goto-char 14)"); // mid-word, NOT the first non-blank
    feed(&mut i, &ed, "m a");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "' a");
    assert_eq!(
        pt(&mut i),
        10,
        "' must land on the first non-blank of the mark's line, not the mark's own column"
    );
}

#[test]
fn backtick_mark_as_operator_target_deletes_exclusive() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "w"); // point -> 7 ('w' of "world")
    feed(&mut i, &ed, "m a");
    feed(&mut i, &ed, "0"); // back to point 1
    feed(&mut i, &ed, "d ` a");
    assert_eq!(
        bs(&mut i),
        "world",
        "`` d`a '' must delete UP TO but not including the mark (exclusive)"
    );
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn quote_mark_as_operator_target_deletes_whole_lines() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour");
    feed(&mut i, &ed, "j j"); // point -> line3 "three"
    feed(&mut i, &ed, "m a");
    feed(&mut i, &ed, "g g"); // back to line1
    feed(&mut i, &ed, "d ' a");
    assert_eq!(
        bs(&mut i),
        "four",
        "`` d'a '' must delete WHOLE LINES from point's line through the mark's line"
    );
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn missing_mark_as_motion_and_as_operator_target_messages_and_cancels() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "` x"); // plain motion, no mark 'x' set
    assert_eq!(pt(&mut i), 1, "a failed jump must not move point");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(echo_row_text(&i, &ed), "Mark not set: x");

    feed(&mut i, &ed, "d ` x"); // same missing mark, as an operator target
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "the pending delete must be cancelled"
    );
    assert_eq!(run(&mut i, "evil--pending-operator"), "nil");
    assert_eq!(bs(&mut i), "hello world", "nothing must be deleted");
    assert_eq!(echo_row_text(&i, &ed), "Mark not set: x");
}

#[test]
fn marks_are_isolated_per_buffer() {
    let (mut i, ed) = setup_evil("aaaa");
    feed(&mut i, &ed, "m a"); // mark a at point 1 in *scratch*

    run(&mut i, "(get-buffer-create \"evil-marks-other\")");
    run(&mut i, "(switch-to-buffer \"evil-marks-other\")");
    // A brand new buffer created via `switch-to-buffer` (not
    // `find-file`) has no buffer-local evil state yet -- normally
    // `evil--maybe-init-current-buffer` runs from `post-command-hook`
    // on the first REAL command, but the setup below runs plain
    // `eval_source` calls (no command loop), so it's forced here.
    run(&mut i, "(evil--maybe-init-current-buffer)");
    run(&mut i, "(insert \"bbbb\")");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "l l"); // point -> 3
    feed(&mut i, &ed, "m a"); // this buffer's OWN mark a, at point 3

    feed(&mut i, &ed, "0");
    feed(&mut i, &ed, "` a");
    assert_eq!(pt(&mut i), 3, "\"other\"'s own mark a");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"evil-marks-other\"");

    run(&mut i, "(switch-to-buffer \"*scratch*\")");
    feed(&mut i, &ed, "` a");
    assert_eq!(
        pt(&mut i),
        1,
        "*scratch*'s own mark a must be unaffected by the other buffer's mark a"
    );
}

#[test]
fn setting_the_same_mark_name_again_overwrites_it() {
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "m a"); // mark a at point 1
    feed(&mut i, &ed, "l l l"); // point -> 4
    feed(&mut i, &ed, "m a"); // same name again: must overwrite, not shadow-and-leak
    feed(&mut i, &ed, "0");
    feed(&mut i, &ed, "` a");
    assert_eq!(
        pt(&mut i),
        4,
        "the second `m a` must overwrite the first, not coexist with it"
    );
}

#[test]
fn set_mark_and_goto_mark_reject_a_non_lowercase_name() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "m 5"); // '5' is not a-z
    assert_eq!(echo_row_text(&i, &ed), "Marks must be a-z");
    feed(&mut i, &ed, "` 5"); // never set: still reported as missing
    assert_eq!(echo_row_text(&i, &ed), "Mark not set: 5");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(bs(&mut i), "hello");
}

// ---------------------------------------------------------------------
// Part I-B: named registers ("{a-z})
// ---------------------------------------------------------------------

#[test]
fn named_write_and_read_are_independent_of_the_unnamed_kill_ring() {
    let (mut i, ed) = setup_evil("aaa\nbbb\nccc");
    feed(&mut i, &ed, "\" a y y"); // register a = "aaa\n"; unnamed = "aaa\n" too
    feed(&mut i, &ed, "j"); // point -> line2 "bbb"
    let point_before = pt(&mut i);
    feed(&mut i, &ed, "y y"); // plain yank: unnamed becomes "bbb\n", register a untouched
    assert_eq!(pt(&mut i), point_before, "yy must not move point");

    // Named paste: must read register a ("aaa\n"), NOT the now-different unnamed.
    feed(&mut i, &ed, "\" a p");
    assert_eq!(bs(&mut i), "aaa\nbbb\naaa\nccc");

    run(&mut i, "(undo)");
    assert_eq!(bs(&mut i), "aaa\nbbb\nccc");
    run(&mut i, &format!("(goto-char {})", point_before));

    // Plain paste: must read the unnamed kill-ring ("bbb\n"), proving
    // the earlier named write did not consume or replace it.
    feed(&mut i, &ed, "p");
    assert_eq!(
        bs(&mut i),
        "aaa\nbbb\nbbb\nccc",
        "plain p must still read the UNNAMED kill-ring (\"bbb\\n\"), not register a"
    );
}

#[test]
fn named_delete_writes_the_register_and_the_unnamed_kill_ring() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree");
    feed(&mut i, &ed, "\" a d d");
    assert_eq!(bs(&mut i), "two\nthree");
    assert_eq!(pt(&mut i), 1);

    // A plain unnamed `p` proves the kill-ring was ALSO updated by the
    // named delete (vim: a named write always updates unnamed too).
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "two\none\nthree");
}

#[test]
fn empty_register_read_messages_and_does_not_touch_the_buffer() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "\" z p");
    assert_eq!(echo_row_text(&i, &ed), "Nothing in register z");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn use_register_rejects_a_non_lowercase_name() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "\" 5"); // '5' is not a-z
    assert_eq!(echo_row_text(&i, &ed), "Registers must be a-z");
    assert_eq!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "a rejected name must not leave a stale pending register armed"
    );
    // yy/p must behave exactly as if `" 5` had never been typed.
    feed(&mut i, &ed, "y y");
    feed(&mut i, &ed, "p");
    // "hello" has no trailing newline, so the linewise `yy`/`p` pair
    // adds one (see `evil--paste-linewise`'s `eobp` branch) -- the
    // point of this assertion is just that `p` pasted a SECOND
    // "hello" at all, proving `" 5` left no stale pending register
    // behind to interfere with the plain, unnamed `yy`/`p` that follow.
    assert_eq!(bs(&mut i), "hello\nhello\n");
}

#[test]
fn meta_w_does_not_degrade_a_named_register_paste() {
    let (mut i, ed) = setup_evil("hello\nworld");
    feed(&mut i, &ed, "\" a y y"); // register a = "hello\n" (linewise)

    // A native, evil-unaware kill/yank command (M-w) changes the
    // UNNAMED kill-ring's top to something charwise -- this is the
    // existing M29 staleness mechanism degrading unnamed `p`/`P`.
    feed(&mut i, &ed, "v l l"); // select "hel" (visual, sets a real mark)
    feed(&mut i, &ed, "ESC"); // exit visual: mark position stays, just inactive
    feed(&mut i, &ed, "M-w"); // kill-ring-save over the stale mark..point region
    assert_eq!(
        run(&mut i, "(evil--current-yank-linewise-p)"),
        "nil",
        "M-w must have made the UNNAMED kill-ring/linewise-flag pairing stale"
    );

    // The named register must be completely unaffected: "ap still
    // pastes register a's own content, still linewise.
    feed(&mut i, &ed, "\" a p");
    assert_eq!(bs(&mut i), "hello\nhello\nworld");
    assert_eq!(pt(&mut i), 7);
}

#[test]
fn ctrl_g_cancels_a_pending_register_before_it_is_consumed() {
    let (mut i, ed) = setup_evil("hello\nworld");
    feed(&mut i, &ed, "\" a"); // arm register a; state stays normal throughout
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_ne!(run(&mut i, "evil--pending-register"), "nil");

    feed(&mut i, &ed, "C-g");
    assert_eq!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "C-g must clear the pending register even though state never left normal"
    );

    // A subsequent yy must NOT have written into register a.
    feed(&mut i, &ed, "y y");
    feed(&mut i, &ed, "\" a p");
    assert_eq!(echo_row_text(&i, &ed), "Nothing in register a");
    assert_eq!(bs(&mut i), "hello\nworld", "nothing should have pasted");
}

#[test]
fn ctrl_g_cancels_a_pending_register_armed_mid_operator() {
    let (mut i, ed) = setup_evil("one\ntwo");
    feed(&mut i, &ed, "\" a"); // arm register a
    feed(&mut i, &ed, "d"); // enter operator-pending with delete
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "C-g");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(run(&mut i, "evil--pending-register"), "nil");
    assert_eq!(
        bs(&mut i),
        "one\ntwo",
        "the delete must have been cancelled entirely"
    );

    feed(&mut i, &ed, "y y"); // plain yank, must not land in register a
    feed(&mut i, &ed, "\" a p");
    assert_eq!(echo_row_text(&i, &ed), "Nothing in register a");
}

#[test]
fn dot_repeat_after_named_delete_freezes_the_register_name_but_rereads_content() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour");
    feed(&mut i, &ed, "\" a d d");
    assert_eq!(bs(&mut i), "two\nthree\nfour");
    assert_eq!(pt(&mut i), 1);

    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "three\nfour",
        ". must redo \"delete current line into register a\" at the NEW point"
    );
    assert_eq!(pt(&mut i), 1);

    // Register a must now hold the SECOND deleted line ("two\n"), not
    // the first ("one\n") -- proving the replay re-armed the SAME
    // register name and `evil--maybe-write-register' overwrote it fresh.
    feed(&mut i, &ed, "\" a p");
    assert_eq!(bs(&mut i), "three\ntwo\nfour");
    assert_eq!(pt(&mut i), 7);
}

// M42 review fix (Severity-1 #1): a pending register prefix must NOT
// survive an unrelated intervening command, but MUST survive any
// number of visual-selection-extending keys before the operator that
// actually consumes it -- see `evil--post-command''s new staleness
// check and `evil--pending-register''s own doc comment (Global state
// section) for the full timeline this pair of tests exercises.

#[test]
fn pending_register_expires_after_an_unrelated_command_before_a_bare_paste() {
    let (mut i, ed) = setup_evil("REGVAL\nworld");
    feed(&mut i, &ed, "\" a y y"); // register a = "REGVAL\n"; unnamed = "REGVAL\n" too
    feed(&mut i, &ed, "j"); // point -> line2 "world"
    feed(&mut i, &ed, "y y"); // plain yank: unnamed becomes "world\n", register a untouched
    feed(&mut i, &ed, "\" a"); // arm register a again
    feed(&mut i, &ed, "l"); // unrelated motion -- must expire the armed register right away
    assert_eq!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "an unrelated command must drop the armed register immediately, not \
         survive one whole extra command's grace period"
    );

    feed(&mut i, &ed, "p"); // bare paste: must read the UNNAMED register, not `a`
    assert_eq!(
        bs(&mut i),
        // Trailing "\n": `evil--paste-linewise' pasting after the LAST
        // line of a buffer with no final newline of its own first
        // inserts one to open the new line (see its own `eobp' branch)
        // -- unrelated to this test's own assertion, which is about
        // WHICH register's text landed, not the exact newline count.
        "REGVAL\nworld\nworld\n",
        "`\"a` then an unrelated `l` then a bare `p` must paste the unnamed \
         register (\"world\"), not the stale register a (\"REGVAL\")"
    );
}

#[test]
fn pending_register_survives_a_visual_selection_extended_before_the_operator() {
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "\" a"); // arm register a
    feed(&mut i, &ed, "v"); // enter visual char state
    assert_ne!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "entering visual state must not itself expire the register armed before it"
    );
    feed(&mut i, &ed, "l l"); // extend the selection two keys, still no operator yet
    assert_ne!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "extending a visual selection must not expire a register armed before \
         entering visual -- only the operator that finally applies it may consume it"
    );

    feed(&mut i, &ed, "d"); // visual delete: consumes the still-armed register
    assert_eq!(
        bs(&mut i),
        "def",
        "\"abc\" (positions 1-3) must have been deleted"
    );
    assert_eq!(
        run(&mut i, "(cadr (assq ?a evil--registers))"),
        "\"abc\"",
        "register a must have received the deleted selection's text"
    );
    assert_eq!(
        run(&mut i, "evil--pending-register"),
        "nil",
        "the operator must have consumed (cleared) the pending register"
    );
}
