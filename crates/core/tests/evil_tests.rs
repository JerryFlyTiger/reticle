//! M29: evil.el — the vim-style modal keybinding layer built on the
//! M28 emulation-keymap foundation (see evil_foundation_tests.rs).
//!
//! Every test asserts both buffer content (`buffer-string`) and point
//! position, per the M29 plan's acceptance bar. `setup_evil` mirrors
//! `evil_foundation_tests.rs`'s `setup`, plus inserting fixture text,
//! resetting point to `point-min`, and turning evil-mode on.

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

/// Raw buffer content (unlike `run`, not the `prin1` printed form —
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
            "se_evil_{}_{}_{}",
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

/// The echo area's rendered text (redisplay.rs's bottom row — mirrors
/// `gui_features_tests.rs`'s `row_text`, just always reading the LAST
/// row instead of taking one as a parameter, since the echo area is
/// always the frame's final line).
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

// A 3-line fixture with no trailing newline, reused by several motion
// tests. Positions (1-based `point` convention):
//   h(1)e(2)l(3)l(4)o(5) (6)w(7)o(8)r(9)l(10)d(11)\n(12)
//   s(13)e(14)c(15)o(16)n(17)d(18) (19)l(20)i(21)n(22)e(23)\n(24)
//   t(25)h(26)i(27)r(28)d(29) (30)l(31)i(32)n(33)e(34)   point-max=35
const LINES3: &str = "hello world\nsecond line\nthird line";

// ---------------------------------------------------------------------
// Motions
// ---------------------------------------------------------------------

#[test]
fn h_moves_left_within_the_line_and_stops_at_bol() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "l l l"); // point -> 4
    feed(&mut i, &ed, "h");
    assert_eq!(pt(&mut i), 3);
    assert_eq!(bs(&mut i), LINES3);
    // h at bol doesn't cross into the previous line.
    feed(&mut i, &ed, "h h h h h h h h h h");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn l_moves_right_with_a_count() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "3 l");
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn j_and_k_move_by_line_keeping_column_best_effort() {
    let (mut i, ed) = setup_evil(LINES3);
    // Column 10: valid in "hello world" (11 chars) and "second line"
    // (11 chars), but "third line" is only 10 chars (columns 0-9) —
    // exercises the clamp, then the sticky goal column recovering it.
    feed(&mut i, &ed, "1 0 l");
    assert_eq!(pt(&mut i), 11);
    feed(&mut i, &ed, "j"); // line 2 ("second line"), same column
    assert_eq!(pt(&mut i), 23);
    feed(&mut i, &ed, "j"); // line 3 ("third line", shorter — clamped to eol)
    assert_eq!(pt(&mut i), 35);
    feed(&mut i, &ed, "k"); // goal column (10) survives the clamp
    assert_eq!(pt(&mut i), 23);
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn w_moves_to_the_start_of_the_next_word() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "w");
    assert_eq!(pt(&mut i), 7); // "world"
    feed(&mut i, &ed, "w");
    assert_eq!(pt(&mut i), 13); // "second" (crosses the newline)
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn b_moves_to_the_start_of_the_previous_word() {
    let (mut i, ed) = setup_evil(LINES3);
    run(&mut i, "(goto-char 13)"); // start of "second"
    feed(&mut i, &ed, "b");
    assert_eq!(pt(&mut i), 7); // "world"
    feed(&mut i, &ed, "b");
    assert_eq!(pt(&mut i), 1); // "hello"
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn e_moves_to_the_end_of_the_word() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "e");
    assert_eq!(pt(&mut i), 5); // last char of "hello"
    feed(&mut i, &ed, "e");
    assert_eq!(pt(&mut i), 11); // last char of "world"
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn big_word_motions_treat_punctuation_as_part_of_the_word() {
    let (mut i, ed) = setup_evil("foo.bar baz");
    feed(&mut i, &ed, "w"); // small w stops at the '.' boundary
    assert_eq!(pt(&mut i), 4);
    run(&mut i, "(goto-char 1)");
    feed(&mut i, &ed, "W"); // big W jumps straight to "baz" (space-delimited)
    assert_eq!(pt(&mut i), 9);
    // Small b (the 3-class word/punct/space model) stops at "bar", NOT
    // all the way back at "foo" -- the '.' is its own punct class.
    feed(&mut i, &ed, "b");
    assert_eq!(pt(&mut i), 5);
    // Big B (2-class blank/non-blank) crosses the punctuation and
    // lands all the way back at "foo".
    feed(&mut i, &ed, "B");
    assert_eq!(pt(&mut i), 1);
    run(&mut i, "(goto-char 1)");
    feed(&mut i, &ed, "E");
    assert_eq!(pt(&mut i), 7); // end of "foo.bar"
    assert_eq!(bs(&mut i), "foo.bar baz");
}

#[test]
fn zero_goes_to_column_zero_but_only_when_not_accumulating_a_count() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "l l l l l l"); // column 6
    feed(&mut i, &ed, "0");
    assert_eq!(pt(&mut i), 1);
    // "10" as a count: 1 then 0 both accumulate (0 is only bol when no
    // count has started yet).
    feed(&mut i, &ed, "1 0 l");
    assert_eq!(pt(&mut i), 11); // moved right 10 from column 0
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn caret_goes_to_the_first_non_blank_character() {
    let (mut i, ed) = setup_evil("   indented\nplain");
    feed(&mut i, &ed, "^");
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), "   indented\nplain");
}

#[test]
fn dollar_goes_to_end_of_line_with_count_extending_lines() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "$");
    assert_eq!(pt(&mut i), 12); // end of "hello world"
    run(&mut i, "(goto-char 1)");
    feed(&mut i, &ed, "3 $");
    assert_eq!(pt(&mut i), 35); // end of the 3rd line
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn gg_goes_to_the_first_line_first_non_blank() {
    let (mut i, ed) = setup_evil("  one\ntwo\nthree");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "g g");
    assert_eq!(pt(&mut i), 3); // "  one" — skips the leading spaces
    assert_eq!(bs(&mut i), "  one\ntwo\nthree");
}

#[test]
fn g_goes_to_the_last_line_with_no_count() {
    let (mut i, ed) = setup_evil(LINES3);
    feed(&mut i, &ed, "G");
    assert_eq!(pt(&mut i), 25); // start of "third line"
    assert_eq!(bs(&mut i), LINES3);
}

#[test]
fn g_with_a_count_goes_to_that_line() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour\nfive\nsix");
    feed(&mut i, &ed, "5 G");
    assert_eq!(pt(&mut i), 20); // start of "five"
    assert_eq!(bs(&mut i), "one\ntwo\nthree\nfour\nfive\nsix");
}

#[test]
fn paragraph_motions_stop_at_blank_lines() {
    let (mut i, ed) = setup_evil("alpha\nbeta\n\ngamma\ndelta");
    feed(&mut i, &ed, "}");
    assert_eq!(pt(&mut i), 12); // the blank line
    feed(&mut i, &ed, "}");
    assert_eq!(pt(&mut i), 24); // point-max: no further blank line
    feed(&mut i, &ed, "{");
    assert_eq!(pt(&mut i), 12);
    feed(&mut i, &ed, "{");
    assert_eq!(pt(&mut i), 1); // point-min
    assert_eq!(bs(&mut i), "alpha\nbeta\n\ngamma\ndelta");
}

#[test]
fn f_finds_a_character_forward_landing_on_it() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    feed(&mut i, &ed, "f X");
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn capital_f_finds_a_character_backward() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "F X");
    assert_eq!(pt(&mut i), 8);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn t_stops_just_before_the_character() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    feed(&mut i, &ed, "t X");
    assert_eq!(pt(&mut i), 3);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn capital_t_stops_just_after_the_character_going_backward() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "T X");
    assert_eq!(pt(&mut i), 9);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn capital_f_with_a_count_lands_on_the_nth_occurrence_backward() {
    // Bare-find-with-a-count counterpart to
    // `bare_count_before_find_lands_on_the_nth_occurrence' (which only
    // covers the forward `f'), going through `evil-find-char-backward'
    // instead, to confirm the M44 eager-count-read fix landed on ALL
    // four arm commands, not just `f'.
    let (mut i, ed) = setup_evil("abcXdefXghiXjklXmno");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "3 F X");
    assert_eq!(
        bs(&mut i),
        "abcXdefXghiXjklXmno",
        "a bare find must not edit"
    );
    assert_eq!(
        pt(&mut i),
        8,
        "3FX from the end must land on the THIRD X counting backward"
    );
}

#[test]
fn semicolon_repeats_the_last_find_forward() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    feed(&mut i, &ed, "f X");
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, ";");
    assert_eq!(pt(&mut i), 8);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn comma_repeats_the_last_find_reversed() {
    let (mut i, ed) = setup_evil("abcXdefXghi");
    feed(&mut i, &ed, "f X"); // -> 4
    feed(&mut i, &ed, ";"); // -> 8
    feed(&mut i, &ed, ","); // reversed: search backward -> 4
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), "abcXdefXghi");
}

#[test]
fn find_char_esc_cancels_without_moving_or_inserting() {
    let (mut i, ed) = setup_evil("abcXdef");
    feed(&mut i, &ed, "f ESC");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(bs(&mut i), "abcXdef");
}

// ---------------------------------------------------------------------
// Operators + motions
// ---------------------------------------------------------------------

#[test]
fn dw_deletes_to_the_start_of_the_next_word() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d w");
    assert_eq!(bs(&mut i), "world");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn de_deletes_through_the_end_of_the_word_inclusive() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d e");
    assert_eq!(bs(&mut i), " world");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn d_dollar_deletes_to_end_of_line() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "l l l"); // point at 4 ('l' of hel-l-o)
    feed(&mut i, &ed, "d $");
    assert_eq!(bs(&mut i), "hel");
    assert_eq!(pt(&mut i), 4);
}

#[test]
fn dd_deletes_the_whole_current_line() {
    let (mut i, ed) = setup_evil("line1\nline2\nline3");
    feed(&mut i, &ed, "d d");
    assert_eq!(bs(&mut i), "line2\nline3");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn two_dd_deletes_two_lines() {
    let (mut i, ed) = setup_evil("line1\nline2\nline3");
    feed(&mut i, &ed, "2 d d");
    assert_eq!(bs(&mut i), "line3");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn d2w_and_2dw_delete_the_same_two_words() {
    let (mut i, ed) = setup_evil("one two three four");
    feed(&mut i, &ed, "d 2 w");
    assert_eq!(bs(&mut i), "three four");
    assert_eq!(pt(&mut i), 1);

    let (mut i2, ed2) = setup_evil("one two three four");
    feed(&mut i2, &ed2, "2 d w");
    assert_eq!(bs(&mut i2), "three four");
    assert_eq!(pt(&mut i2), 1);
}

#[test]
fn cw_deletes_the_word_and_enters_insert_state() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "c w");
    assert_eq!(bs(&mut i), " world");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "h i");
    assert_eq!(bs(&mut i), "hi world");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn yy_then_p_pastes_the_line_below() {
    let (mut i, ed) = setup_evil("alpha\nbeta");
    feed(&mut i, &ed, "y y");
    assert_eq!(bs(&mut i), "alpha\nbeta", "yank must not modify the buffer");
    assert_eq!(pt(&mut i), 1, "yank must not move point");
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "alpha\nalpha\nbeta");
    assert_eq!(pt(&mut i), 7); // first-non-blank of the newly pasted line
}

#[test]
fn yw_then_capital_p_pastes_the_word_before_point() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "y w");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, "w"); // move to "world"
    feed(&mut i, &ed, "P");
    assert_eq!(bs(&mut i), "hello hello world");
    // The killed text is "hello " (yw includes the trailing space, per
    // the same exclusive `w' boundary dw/yw share); charwise P rests on
    // the LAST character of the pasted text -- the space before "world".
    assert_eq!(pt(&mut i), 12);
}

// ---------------------------------------------------------------------
// Text objects
// ---------------------------------------------------------------------

#[test]
fn diw_deletes_the_inner_word_under_point() {
    let (mut i, ed) = setup_evil("foo bar baz");
    feed(&mut i, &ed, "w"); // point on "bar"
    feed(&mut i, &ed, "d i w");
    assert_eq!(bs(&mut i), "foo  baz");
    assert_eq!(pt(&mut i), 5);
}

#[test]
fn daw_deletes_the_word_with_trailing_space() {
    let (mut i, ed) = setup_evil("foo bar baz");
    feed(&mut i, &ed, "w"); // point on "bar"
    feed(&mut i, &ed, "d a w");
    assert_eq!(bs(&mut i), "foo baz");
    assert_eq!(pt(&mut i), 5);
}

#[test]
fn di_quote_deletes_inside_the_quotes() {
    let (mut i, ed) = setup_evil("say \"hello world\" now");
    feed(&mut i, &ed, "d i \"");
    assert_eq!(bs(&mut i), "say \"\" now");
    assert_eq!(pt(&mut i), 6);
}

#[test]
fn da_paren_deletes_including_the_parens() {
    let (mut i, ed) = setup_evil("call(x, y) done");
    run(&mut i, "(goto-char 6)"); // inside the parens, on 'x'
    feed(&mut i, &ed, "d a (");
    assert_eq!(bs(&mut i), "call done");
    assert_eq!(pt(&mut i), 5);
}

#[test]
fn ci_brace_changes_inside_the_braces() {
    let (mut i, ed) = setup_evil("fn main() { old body }");
    run(&mut i, "(goto-char 15)"); // inside the braces
    feed(&mut i, &ed, "c i {");
    // i{ spans everything strictly between the braces, including the
    // interior leading/trailing spaces (unlike the whitespace-sensitive
    // `iw'): " old body " is deleted entirely, leaving "{}" adjacent.
    assert_eq!(bs(&mut i), "fn main() {}");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "fn main() {x}");
}

#[test]
fn di_bracket_deletes_inside_square_brackets_across_the_line() {
    let (mut i, ed) = setup_evil("xs = [1, 2, 3]\ndone");
    run(&mut i, "(goto-char 8)"); // inside the brackets, on '1'
    feed(&mut i, &ed, "d i [");
    assert_eq!(bs(&mut i), "xs = []\ndone");
    assert_eq!(pt(&mut i), 7);
}

// ---------------------------------------------------------------------
// Simple edits
// ---------------------------------------------------------------------

#[test]
fn x_deletes_the_character_under_point() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn three_x_deletes_three_characters() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "3 x");
    assert_eq!(bs(&mut i), "lo");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn capital_x_deletes_the_character_before_point() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "l l l"); // point at 4 (between the two "l"s)
    feed(&mut i, &ed, "X"); // deletes the character just before point
    assert_eq!(bs(&mut i), "helo");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn capital_d_deletes_to_end_of_line() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "l l l");
    feed(&mut i, &ed, "D");
    assert_eq!(bs(&mut i), "hel");
    assert_eq!(pt(&mut i), 4);
}

#[test]
fn capital_c_changes_to_end_of_line() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "l l l");
    feed(&mut i, &ed, "C");
    assert_eq!(bs(&mut i), "hel");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "help");
}

#[test]
fn capital_y_yanks_the_whole_line_not_to_end_of_line() {
    let (mut i, ed) = setup_evil("hello\nworld");
    feed(&mut i, &ed, "l l l"); // mid-line, shouldn't matter for Y
    feed(&mut i, &ed, "Y");
    assert_eq!(bs(&mut i), "hello\nworld");
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "hello\nhello\nworld");
}

#[test]
fn capital_s_changes_the_whole_line() {
    let (mut i, ed) = setup_evil("hello\nworld");
    feed(&mut i, &ed, "l l l");
    feed(&mut i, &ed, "S");
    assert_eq!(bs(&mut i), "\nworld");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "h i");
    assert_eq!(bs(&mut i), "hi\nworld");
}

#[test]
fn capital_j_joins_the_next_line_with_one_space() {
    let (mut i, ed) = setup_evil("hello\n   world");
    feed(&mut i, &ed, "J");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 6);
}

#[test]
fn r_replaces_the_character_under_point() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "r z");
    assert_eq!(bs(&mut i), "zello");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn count_before_r_replaces_that_many_characters() {
    // Pins `evil-replace-char''s existing eager-`evil--total-count'-read
    // (the M29-era pattern the M44 fix now mirrors for f/F/t/T -- see
    // `evil--do-replace-char') so a later change to the shared count
    // machinery can't quietly break it without a test noticing.
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "3 r y");
    assert_eq!(bs(&mut i), "yyylo");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn r_then_esc_cancels_without_changing_anything() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "r ESC");
    assert_eq!(bs(&mut i), "hello");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn tilde_flips_case_and_advances() {
    let (mut i, ed) = setup_evil("hELLO");
    feed(&mut i, &ed, "~");
    assert_eq!(bs(&mut i), "HELLO");
    assert_eq!(pt(&mut i), 2);
    feed(&mut i, &ed, "~ ~ ~ ~");
    assert_eq!(bs(&mut i), "Hello");
    assert_eq!(pt(&mut i), 6);
}

// ---------------------------------------------------------------------
// Undo / redo
// ---------------------------------------------------------------------

#[test]
fn u_restores_content_after_dd() {
    let (mut i, ed) = setup_evil("line1\nline2");
    feed(&mut i, &ed, "d d");
    assert_eq!(bs(&mut i), "line2");
    feed(&mut i, &ed, "u");
    assert_eq!(bs(&mut i), "line1\nline2");
}

#[test]
fn u_then_c_r_redoes_the_undone_change() {
    let (mut i, ed) = setup_evil("line1\nline2");
    feed(&mut i, &ed, "d d");
    assert_eq!(bs(&mut i), "line2");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "line1\nline2",
        "undo should have restored line1"
    );
    feed(&mut i, &ed, "C-r");
    assert_eq!(bs(&mut i), "line2", "redo should reapply the dd");
}

// NOTE on scope: consecutive `u` presses chain correctly arbitrarily
// far back (verified below), and a single C-r after any number of u's
// correctly redoes the most recent change (also verified below and in
// `u_then_c_r_redoes_the_undone_change`) -- this is the plan's actual
// requirement. A SECOND consecutive C-r does not currently chain to a
// second redo: `evil-redo''s marker trick (see its docstring) can only
// force last-command to NOT be "undo" (needed so the first redo grabs
// the freshly-appended redo-inverse at the tail rather than continuing
// backward) -- it cannot make last-command literally BE "undo" again
// afterward, which is what a second redo would need in order to
// *continue* via `pending_undo' the same way consecutive `u' presses
// do. There is no elisp accessor for last-command in this codebase,
// and synthesizing "undo" would mean actually re-running undo (an
// unwanted second step). Documented here rather than silently dropped.
#[test]
fn double_undo_chains_backward_then_a_single_redo_reapplies_the_latest_change() {
    let (mut i, ed) = setup_evil("aaa\nbbb\nccc");
    feed(&mut i, &ed, "d d"); // -> "bbb\nccc"
    feed(&mut i, &ed, "d d"); // -> "ccc"
    assert_eq!(bs(&mut i), "ccc");
    feed(&mut i, &ed, "u");
    assert_eq!(bs(&mut i), "bbb\nccc");
    feed(&mut i, &ed, "u"); // chains backward past the first undo
    assert_eq!(bs(&mut i), "aaa\nbbb\nccc");
    feed(&mut i, &ed, "C-r");
    assert_eq!(bs(&mut i), "bbb\nccc"); // redoes the most recent undo
}

// ---------------------------------------------------------------------
// Insert-state entry points
// ---------------------------------------------------------------------

#[test]
fn i_inserts_before_point() {
    let (mut i, ed) = setup_evil("bc");
    feed(&mut i, &ed, "i a");
    assert_eq!(bs(&mut i), "abc");
    assert_eq!(pt(&mut i), 2);
    assert_eq!(run(&mut i, "evil--state"), "insert");
}

#[test]
fn a_inserts_after_point() {
    let (mut i, ed) = setup_evil("ac");
    feed(&mut i, &ed, "a b");
    assert_eq!(bs(&mut i), "abc");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn capital_i_inserts_at_first_non_blank() {
    let (mut i, ed) = setup_evil("  bc");
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "I a");
    assert_eq!(bs(&mut i), "  abc");
    assert_eq!(pt(&mut i), 4);
}

#[test]
fn capital_a_inserts_at_end_of_line() {
    let (mut i, ed) = setup_evil("ab");
    run(&mut i, "(goto-char 1)");
    feed(&mut i, &ed, "A c");
    assert_eq!(bs(&mut i), "abc");
    assert_eq!(pt(&mut i), 4);
}

#[test]
fn o_opens_a_new_line_below_and_inserts() {
    let (mut i, ed) = setup_evil("first\nlast");
    feed(&mut i, &ed, "o x");
    assert_eq!(bs(&mut i), "first\nx\nlast");
    assert_eq!(pt(&mut i), 8);
}

#[test]
fn capital_o_opens_a_new_line_above_and_inserts() {
    let (mut i, ed) = setup_evil("first\nlast");
    feed(&mut i, &ed, "j"); // point on "last"
    feed(&mut i, &ed, "O x");
    assert_eq!(bs(&mut i), "first\nx\nlast");
    assert_eq!(pt(&mut i), 8);
}

#[test]
fn esc_in_insert_state_moves_point_left_one_and_returns_to_normal() {
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i a b c");
    assert_eq!(bs(&mut i), "abc");
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "ESC");
    assert_eq!(pt(&mut i), 3);
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

#[test]
fn esc_in_insert_state_does_not_move_left_at_bol() {
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i");
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, "ESC");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(bs(&mut i), "");
}

// ---------------------------------------------------------------------
// Visual state
// ---------------------------------------------------------------------

#[test]
fn visual_char_motion_then_delete() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v l l l"); // select "hell" (point 1..4, extend to 4)
    assert_eq!(run(&mut i, "evil--state"), "visual");
    feed(&mut i, &ed, "d");
    assert_eq!(bs(&mut i), "o world");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

#[test]
fn visual_line_j_then_delete_deletes_two_lines() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree");
    feed(&mut i, &ed, "V j");
    assert_eq!(run(&mut i, "evil--state"), "visual");
    feed(&mut i, &ed, "d");
    assert_eq!(bs(&mut i), "three");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn visual_o_swaps_point_and_mark() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v l l l l"); // mark=1(0-based)/point moved to 5
    let point_at_far_end = pt(&mut i);
    feed(&mut i, &ed, "o");
    // After swapping, point should now be at the original mark (1) —
    // and a further "o" should bring it right back.
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, "o");
    assert_eq!(pt(&mut i), point_at_far_end);
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn esc_in_visual_state_clears_the_mark_and_returns_to_normal() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v l l"); // point: 1 -> 3
    feed(&mut i, &ed, "ESC");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(pt(&mut i), 3, "ESC in visual state doesn't move point");
    assert_eq!(bs(&mut i), "hello world");
    // The keymap really did switch back to normal-map: "x" now deletes
    // a single character under point (the normal-state binding) rather
    // than acting on a leftover visual selection.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "helo world");
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn visual_char_yank_moves_point_to_selection_start() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "w"); // point on "world"
    feed(&mut i, &ed, "v l l");
    feed(&mut i, &ed, "y");
    assert_eq!(bs(&mut i), "hello world", "yank must not modify the buffer");
    assert_eq!(pt(&mut i), 7, "visual yank moves point to selection start");
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

// ---------------------------------------------------------------------
// State machine edges: operator-pending cancel, emacs state, mode off
// ---------------------------------------------------------------------

#[test]
fn esc_in_operator_pending_cancels_back_to_normal() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "ESC");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(bs(&mut i), "hello world");
    // Normal dispatch resumes immediately afterward.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello world");
}

#[test]
fn c_g_in_operator_pending_cancels_back_to_normal() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "2 d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "C-g");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(run(&mut i, "evil--count"), "nil");
}

#[test]
fn interactive_spec_failure_in_operator_pending_returns_to_normal_state() {
    // M48 Part E repro (PLAN.md "known unfixed latent issues"): `process_pending`'s
    // `'r'` branch used to `return` straight out of `handle_key` on a
    // missing mark, skipping `call_command`'s post-command-hook/
    // `last_command` tail entirely -- so an emulation layer that resets
    // its own pending state from `post-command-hook` (evil's
    // `evil--post-command`, which cancels a stuck `operator-pending` on
    // the second consecutive firing while still in that state -- see its
    // own docstring) never got a chance to run at all for this key.
    //
    // `C-w` is unbound in `evil--op-pending-map` (only normal state binds
    // it, as the window prefix; it's not one of the printable ASCII keys
    // `evil--op-pending-map`'s catchall covers either), so from
    // operator-pending it falls through the emulation keymap to the
    // global keymap's `kill-region` binding -- whose `"r"` interactive
    // spec fails here since no mark is set, exactly the failure path
    // under test.
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "C-w");
    assert_eq!(bs(&mut i), "hello world", "kill-region must not have run");
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "post-command-hook must still run on interactive-spec collection \
         failure so evil's pending operator doesn't get stuck"
    );
    // Dispatch resumes normally afterward.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello world");
}

#[test]
fn interactive_form_eval_error_in_operator_pending_returns_to_normal_state() {
    // Found by the M48 fix-round reviewer: the same class of "aborted
    // without finishing" bug also exists in `execute_command`'s
    // `InteractiveSpec::Form` branch -- when `elisp::eval::eval` fails to
    // evaluate the interactive form, the old code just `echo`ed a message
    // and returned without calling `finish_command`, the same structural
    // problem as the `'r'`-no-mark case above: evil's
    // `evil--post-command` never gets a chance to run on this path, and
    // operator-pending gets stuck. Reproduced with a test command using
    // `(interactive (error ...))`, bound to `C-q` (not occupied by
    // `evil--op-pending-map` or any existing binding in this project;
    // the catchall only covers printable characters 32..126, and `C-q`
    // is a control character, so like `C-w` it falls through to the
    // global keymap).
    let (mut i, ed) = setup_evil("hello world");
    run(
        &mut i,
        "(defun se--test-form-error-cmd ()\n\
           (interactive (error \"boom\"))\n\
           (insert \"SHOULD-NOT-RUN\"))",
    );
    run(&mut i, "(global-set-key \"C-q\" 'se--test-form-error-cmd)");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "C-q");
    assert_eq!(
        bs(&mut i),
        "hello world",
        "se--test-form-error-cmd's body must not have run"
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "post-command-hook must still run when the interactive FORM's own \
         eval fails, so evil's pending operator doesn't get stuck"
    );
    // Dispatch resumes normally afterward.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello world");
}

#[test]
fn minibuffer_arg_conversion_error_in_operator_pending_returns_to_normal_state() {
    // Same class of bug as the two tests above, this time in
    // `submit_minibuffer': when `convert_minibuffer_arg' fails (e.g. `n'
    // given non-numeric text), the old code only `echo'd the message and
    // dropped the whole pending-args collection without ever calling
    // `finish_command' -- so from operator-pending, evil's own
    // `evil--post-command' reset never fired either. `goto-line' (`"n"'
    // spec, bound globally to `M-g M-g') is real, already-shipped
    // machinery for exercising this -- no test-only command needed here,
    // unlike the FORM-eval case above where nothing built-in errors out
    // of its interactive form itself.
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "M-g M-g");
    assert!(
        ed.borrow().minibuffer.is_some(),
        "goto-line's \"n\" spec must have opened the minibuffer prompt"
    );
    feed(&mut i, &ed, "a b c RET"); // non-numeric -> convert_minibuffer_arg Err
    assert!(
        ed.borrow().minibuffer.is_none(),
        "the failed submission must still close the minibuffer"
    );
    assert_eq!(
        bs(&mut i),
        "hello world",
        "goto-line must never have run its body"
    );
    assert_eq!(
        pt(&mut i),
        1,
        "point must be untouched by the failed goto-line"
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "post-command-hook must still run when convert_minibuffer_arg fails, \
         so evil's pending operator doesn't get stuck"
    );
    // Dispatch resumes normally afterward.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello world");
}

// M50: minibuffer ESC deliberately does NOT run `keyboard-quit-hook`
// (see the comment on the ESC branch in `commands::minibuffer_key`) so
// that evil's `evil--on-keyboard-quit' -> `evil-insert-exit' -- bound on
// that hook -- never fires for it. C-g still runs the hook, matching
// upstream evil's wider "C-g in Insert state behaves like ESC" meaning.
// These two tests pin that split down to user-visible evil state.

#[test]
fn minibuffer_esc_in_insert_state_leaves_insert_state_alone() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "i");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "M-x");
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-x should have opened the minibuffer"
    );
    feed(&mut i, &ed, "ESC");
    assert!(
        ed.borrow().minibuffer.is_none(),
        "ESC should have cancelled the minibuffer"
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "insert",
        "ESC cancelling the minibuffer must not also kick evil out of \
         Insert state -- that's keyboard-quit-hook's C-g-flavored wide \
         semantics, which ESC deliberately does not run"
    );
}

#[test]
fn minibuffer_c_g_in_insert_state_exits_to_normal_state() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "i");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "M-x");
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-x should have opened the minibuffer"
    );
    feed(&mut i, &ed, "C-g");
    assert!(
        ed.borrow().minibuffer.is_none(),
        "C-g should have cancelled the minibuffer"
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "C-g cancelling the minibuffer must still run keyboard-quit-hook, \
         which exits Insert state via evil-insert-exit -- existing \
         behavior, must not regress"
    );
}

#[test]
fn emacs_state_buffer_j_is_dired_next_line_not_an_evil_motion() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("dired");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    std::fs::write(dir.join("b.txt"), "b").unwrap();
    let opened = run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "dired failed: {}", opened);
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "evil--state"), "emacs");
    let before_text = bs(&mut i);
    let before_point = pt(&mut i);
    feed(&mut i, &ed, "j");
    // M130: dired's own local map now binds "j" to `next-line' (dired.el),
    // so this must move point down a line without touching the buffer
    // text at all -- confirming dispatch went through dired's LOCAL map
    // (which happens to also resolve to `next-line', same as `n') rather
    // than falling through to evil's normal-state motion. The guard
    // against this buffer silently dropping OUT of `emacs' state (which
    // would let evil's own normal-state `j' land here and make this
    // assertion pass for the wrong reason) is the `assert_eq!' just
    // above, against `evil--state' directly -- not a separate list-
    // membership test (M130 fix round FIX-5: no such test exists in
    // this file; grepping for it turns up nothing).
    assert_ne!(pt(&mut i), before_point, "j must move point down a line");
    assert_eq!(bs(&mut i), before_text, "j must not alter buffer text");
}

#[test]
fn evil_mode_off_restores_plain_self_insert() {
    let (mut i, ed) = setup_evil("ab\ncd");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "j"); // evil: moves down a line, buffer unchanged
    assert_eq!(bs(&mut i), "ab\ncd");
    assert_ne!(pt(&mut i), 1);
    run(&mut i, "(evil-mode -1)");
    assert_eq!(run(&mut i, "(local-variable-p 'emulation-keymap)"), "t");
    assert_eq!(run(&mut i, "emulation-keymap"), "nil");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "j"); // now an ordinary self-insert
    assert_eq!(bs(&mut i), "jab\ncd");
    assert_eq!(pt(&mut i), 2);
}

// ---------------------------------------------------------------------
// Count boundaries
// ---------------------------------------------------------------------

#[test]
fn nine_j_moves_down_nine_lines_or_clamps_at_the_last_line() {
    let (mut i, ed) = setup_evil("l1\nl2\nl3\nl4\nl5");
    feed(&mut i, &ed, "9 j");
    // Only 4 more lines exist; clamps to the last line without erroring.
    assert_eq!(pt(&mut i), 13); // start of "l5"
    assert_eq!(bs(&mut i), "l1\nl2\nl3\nl4\nl5");
}

#[test]
fn hundred_j_does_not_crash_and_clamps() {
    let (mut i, ed) = setup_evil("l1\nl2\nl3");
    feed(&mut i, &ed, "1 0 0 j");
    assert_eq!(pt(&mut i), 7); // start of "l3", the last line
    assert_eq!(bs(&mut i), "l1\nl2\nl3");
}

#[test]
fn count_is_consumed_and_does_not_leak_into_the_next_command() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "3 l"); // point -> 4, count consumed
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "l"); // plain, no leftover count: moves by 1 only
    assert_eq!(pt(&mut i), 5);
    assert_eq!(bs(&mut i), "hello world");
}

// =======================================================================
// Review-round fixes (post-implementation code review)
// =======================================================================

// --- Issue 1 (high severity): d/c/y + f/F/t/T was completely broken ---
// `evil-find-char-forward' (etc.) arming `capture-next-key' is itself a
// complete command, so its own completion fired post-command-hook
// before the target character even arrived; `evil--op-pending-fresh'
// was already stale from the "d" keypress, so that firing looked like
// a stray key and cancelled the operator out from under the pending
// capture. Fixed by re-arming the flag in all four entry points.

const FIND_FIXTURE: &str = "abcXdefXghi";
// a(1) b(2) c(3) X(4) d(5) e(6) f(7) X(8) g(9) h(10) i(11); point-max=12

#[test]
fn dfx_deletes_through_the_found_character() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    feed(&mut i, &ed, "d f X");
    assert_eq!(bs(&mut i), "defXghi");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn dtx_deletes_up_to_but_not_including_the_found_character() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    feed(&mut i, &ed, "d t X");
    assert_eq!(bs(&mut i), "XdefXghi");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn cfx_deletes_through_the_found_character_and_enters_insert() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    feed(&mut i, &ed, "c f X");
    assert_eq!(bs(&mut i), "defXghi");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "Z");
    assert_eq!(bs(&mut i), "ZdefXghi");
}

#[test]
fn yfx_then_p_pastes_the_yanked_span_without_moving_point() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    feed(&mut i, &ed, "y f X");
    assert_eq!(bs(&mut i), FIND_FIXTURE, "yank must not modify the buffer");
    assert_eq!(pt(&mut i), 1, "yank must not move point");
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "aabcXbcXdefXghi");
    assert_eq!(pt(&mut i), 5);
}

#[test]
fn d_capital_f_x_deletes_backward_through_the_found_character() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "d F X");
    assert_eq!(bs(&mut i), "abcXdef");
    assert_eq!(pt(&mut i), 8);
}

#[test]
fn d_capital_t_x_deletes_backward_up_to_but_not_including_the_character() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, "d T X");
    assert_eq!(bs(&mut i), "abcXdefX");
    assert_eq!(pt(&mut i), 9);
}

#[test]
fn count_before_operator_extends_a_find_to_the_nth_occurrence() {
    let (mut i, ed) = setup_evil("abcXdefXghiXjkl");
    feed(&mut i, &ed, "2 d f X");
    assert_eq!(bs(&mut i), "ghiXjkl");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn bare_count_before_find_lands_on_the_nth_occurrence() {
    // M44 bug: a bare (operator-less) "3fX" used to silently drop its
    // count -- `evil-find-char-forward' arming `capture-next-key' is
    // itself a complete command, so its own `evil--post-command' pass
    // ran the count-staleness cleanup (see :1317) and cleared
    // `evil--count' before the target character "X" ever arrived to let
    // `evil--handle-find' read it via `evil--total-count'.
    let (mut i, ed) = setup_evil("abcXdefXghiXjklXmno");
    feed(&mut i, &ed, "3 f X");
    assert_eq!(
        bs(&mut i),
        "abcXdefXghiXjklXmno",
        "a bare find must not edit"
    );
    assert_eq!(
        pt(&mut i),
        12,
        "3fX must land on the THIRD X, not the first"
    );
}

#[test]
fn count_typed_after_the_operator_still_extends_a_find_to_the_nth_occurrence() {
    // M44 bug: unlike "2dfX" (count typed BEFORE the operator, which
    // `evil--op-start' safely moves into `evil--op-count' -- see
    // `count_before_operator_extends_a_find_to_the_nth_occurrence'
    // above), "d2fX" types the count AFTER "d" has already started the
    // operator. The "2" still lands in `evil--count' (`evil--digit'
    // doesn't distinguish operator-pending from normal state), and the
    // SAME staleness cleanup that drops a bare find's count also drops
    // this one when "f" arms its capture. Both orders are supposed to
    // be equivalent "count=2 dfX" -- same fixture/assertions as the
    // "2dfX" test above.
    let (mut i, ed) = setup_evil("abcXdefXghiXjkl");
    feed(&mut i, &ed, "d 2 f X");
    assert_eq!(bs(&mut i), "ghiXjkl");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn d_f_esc_cancels_the_capture_and_the_operator_leaving_the_buffer_untouched() {
    let (mut i, ed) = setup_evil(FIND_FIXTURE);
    feed(&mut i, &ed, "d f");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "ESC"); // delivered to the f-capture callback as 27
    assert_eq!(bs(&mut i), FIND_FIXTURE);
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "normal");
    // Dispatch resumes normally afterward (state didn't get stuck).
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "bcXdefXghi");
}

// --- Issue 2 (medium-high): unbound printable key in operator-pending
// used to self-insert before the safety net cancelled the operator ---

#[test]
fn an_unbound_printable_key_in_operator_pending_does_not_self_insert() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d p"); // "p" is not an operator-pending motion
    assert_eq!(bs(&mut i), "hello world", "must not have inserted 'p'");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

#[test]
fn a_named_key_in_operator_pending_does_not_crash() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    // Arrow keys aren't operator-pending motions; documented (file
    // header) as a delayed (not immediate) recovery since a Key::Sym
    // can't self-insert and so never reaches post-command-hook. The
    // property under test here is just "doesn't crash / doesn't touch
    // the buffer", not immediate state recovery.
    feed(&mut i, &ed, "<up>");
    assert_eq!(bs(&mut i), "hello world");
}

// --- Issue 3 (M29: documented, not fixed -- M30: fixed as a stretch
// goal). c/s/S/C/o/O now group their delete/newline with the text
// subsequently typed into ONE undo step, via the `undo-amalgamate-
// boundary' mechanism (see evil.el's `evil--operator-apply' and
// `evil-open-below'/`evil-open-above', and the builtin's own docstring
// in editing.rs). This is the "deliberate update" the M29 comment on
// the old pinned test anticipated -- it now pins the MERGED behavior
// instead of the old two-step one. ---

#[test]
fn cw_then_typing_then_esc_then_a_single_undo_reverts_both() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "c w");
    assert_eq!(bs(&mut i), " world");
    feed(&mut i, &ed, "h i");
    assert_eq!(bs(&mut i), "hi world");
    feed(&mut i, &ed, "ESC");
    assert_eq!(bs(&mut i), "hi world");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "hello world",
        "a single undo should revert BOTH the deletion and the typed insertion"
    );
}

#[test]
fn cw_with_nothing_typed_before_esc_is_still_a_single_undo_step() {
    // No text ever got typed, so there's nothing to merge WITH -- a
    // single `u' should just revert the deletion, same as before M30
    // (guards against the suppression flag ever affecting a session
    // where no self-insert came along to consume it).
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "c w");
    assert_eq!(bs(&mut i), " world");
    feed(&mut i, &ed, "ESC");
    feed(&mut i, &ed, "u");
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn s_then_typing_then_esc_then_a_single_undo_reverts_both() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "s");
    assert_eq!(bs(&mut i), "ello");
    feed(&mut i, &ed, "X ESC");
    assert_eq!(bs(&mut i), "Xello");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "hello",
        "s + typed text + ESC should be one undo group"
    );
}

#[test]
fn o_then_typing_then_esc_then_a_single_undo_reverts_both() {
    let (mut i, ed) = setup_evil("first\nlast");
    feed(&mut i, &ed, "o");
    feed(&mut i, &ed, "x y z");
    assert_eq!(bs(&mut i), "first\nxyz\nlast");
    feed(&mut i, &ed, "ESC");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "first\nlast",
        "a single undo should revert BOTH the opened line and the typed text"
    );
}

#[test]
fn capital_o_then_typing_then_esc_then_a_single_undo_reverts_both() {
    let (mut i, ed) = setup_evil("first\nlast");
    feed(&mut i, &ed, "j"); // point on "last"
    feed(&mut i, &ed, "O");
    feed(&mut i, &ed, "x y z");
    assert_eq!(bs(&mut i), "first\nxyz\nlast");
    feed(&mut i, &ed, "ESC");
    feed(&mut i, &ed, "u");
    assert_eq!(bs(&mut i), "first\nlast");
}

#[test]
fn c_g_after_a_change_operator_clears_the_undo_amalgamation_flag() {
    // M30 review fix (issue 3, medium severity): C-g reaching
    // `evil-insert-exit' via `keyboard-quit-hook' (insert state's own
    // C-g handling, see `evil--on-keyboard-quit') never passes through
    // `execute_command' at all -- that hook runs via a plain
    // `apply_function' (see `run_hook_by_name' in commands.rs), not
    // the command loop -- so a pending `undo-amalgamate-boundary'
    // suppression armed by the `c' operator just before this C-g was
    // left uncleared without this fix: it would silently merge some
    // LATER, unrelated self-insert into the undo group `c'/C-g had
    // already abandoned. `handle_key''s top-level C-g branch now
    // clears the flag itself (the capture-cancel branch does too, for
    // the same reason, though this test only exercises the top-level
    // one -- there's no capture-next-key active during plain insert-
    // state typing).
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "c w"); // deletes "hello" entirely, enters insert, arms the flag
    assert_eq!(bs(&mut i), "");
    feed(&mut i, &ed, "C-g"); // abandons the change via keyboard-quit-hook, not execute_command
    assert_eq!(run(&mut i, "evil--state"), "normal");
    // M34: normal state now sets buffer-local `inhibit-self-insert', so
    // "z" (unbound in evil--normal-map, the very fact this test used to
    // exploit) no longer self-inserts there -- flip the flag directly,
    // bypassing evil.el's state machine, so this probe still reaches a
    // BARE `self_insert' call, i.e. one NOT preceded by any
    // `execute_command' (which would itself clear
    // `suppress_next_undo_boundary' and so mask exactly the fix this test
    // exists to pin), without disabling evil-mode altogether -- that
    // would also unbind "u" below, which is bound to `undo' only via
    // evil--normal-map.
    run(&mut i, "(setq-local inhibit-self-insert nil)");
    feed(&mut i, &ed, "z"); // now an ordinary self-insert again
    assert_eq!(bs(&mut i), "z");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "",
        "a single undo should revert ONLY the unrelated \"z\", not the abandoned cw too"
    );
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "hello",
        "a second, separate undo restores the change C-g abandoned"
    );
}

#[test]
fn plain_typing_with_no_preceding_operator_is_unaffected_by_amalgamation() {
    // Sanity check: ordinary insert-state typing (no delete/newline to
    // merge with) still gets its own fresh undo boundary the moment
    // insert state is entered, exactly as before M30.
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i a b c ESC");
    assert_eq!(bs(&mut i), "abc");
    feed(&mut i, &ed, "u");
    assert_eq!(bs(&mut i), "");
}

// --- Issue 4 (medium): evil--yank-linewise must not trust a stale flag
// when the kill-ring's top changed via a non-evil-aware command ---

#[test]
fn a_native_kill_between_evil_yank_and_paste_is_treated_as_charwise() {
    let (mut i, ed) = setup_evil("xxx\nbbb");
    feed(&mut i, &ed, "y y"); // linewise yank of "xxx\n"; flag=t, text="xxx\n"
    assert_eq!(bs(&mut i), "xxx\nbbb");
    // A native, non-evil-aware kill-ring-save (what M-w calls
    // internally) changes the kill-ring's top without touching evil's
    // own bookkeeping -- simulates the reviewer's "yy" then "M-w" repro.
    run(&mut i, "(kill-ring-save-internal 5 8)"); // saves "bbb" (charwise, no newline)
    assert_eq!(run(&mut i, "(current-kill)"), "\"bbb\"");
    feed(&mut i, &ed, "p");
    // Correct (charwise, since the flag is now recognized as stale):
    // "bbb" inserted right after point. The stale-flag bug would
    // instead have force-pasted "bbb" as a WHOLE NEW LINE, giving
    // "xxx\nbbb\nbbb" (three lines) -- structurally different, so this
    // assertion would fail loudly if the fix regressed.
    assert_eq!(bs(&mut i), "xbbbxx\nbbb");
    assert_eq!(pt(&mut i), 4);
}

// --- Issue 5 (medium): a half-typed count must not survive an
// unrelated intervening command or linger indefinitely ---

#[test]
fn count_does_not_survive_an_intervening_command_like_switching_buffers() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "5"); // start a count, never followed by a motion
    assert_eq!(run(&mut i, "evil--count"), "5");
    // A real command runs through the actual command loop (so
    // post-command-hook fires) without ever touching evil--count --
    // switching to a different buffer and back, the reviewer's exact
    // repro. `command-execute' on a fresh lambda drives this through
    // the real command loop without navigating the minibuffer/panel UI.
    run(
        &mut i,
        "(command-execute (lambda () (interactive) \
           (switch-to-buffer-internal \"evil-count-other\")))",
    );
    run(
        &mut i,
        "(command-execute (lambda () (interactive) \
           (switch-to-buffer-internal \"*scratch*\")))",
    );
    assert_eq!(
        run(&mut i, "evil--count"),
        "nil",
        "the stale count must not survive an unrelated command"
    );
    feed(&mut i, &ed, "3 l");
    assert_eq!(pt(&mut i), 4, "moved by exactly 3, not 53");
    assert_eq!(bs(&mut i), "hello world");
}

#[test]
fn esc_in_normal_state_clears_a_half_typed_count() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "5");
    assert_eq!(run(&mut i, "evil--count"), "5");
    feed(&mut i, &ed, "ESC");
    assert_eq!(run(&mut i, "evil--count"), "nil");
    feed(&mut i, &ed, "3 l");
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), "hello world");
}

// --- Issue 6 (low): evil-mode's on-toggle must sync cursor-type (was
// cursor-shape pre-M32) to the buffer that's actually current, not
// whichever is last in buffer-list ---

#[test]
fn evil_mode_on_syncs_cursor_type_to_the_current_buffer_not_the_last_in_buffer_list() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("cursor");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    // dired's buffer is created (and so appended to buffer-list) AFTER
    // *scratch*, so it's last; switching back to *scratch* makes it
    // current again before evil-mode turns on.
    let opened = run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "dired failed: {}", opened);
    run(&mut i, "(switch-to-buffer \"*scratch*\")");
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");
    // `evil--state' can only read "normal" if the final refresh after
    // the on-toggle's buffer-list loop (see `evil-mode') landed on
    // *scratch*'s OWN state, not dired's. And post-review cursor-type
    // discriminates again: dired (emacs state) maps to nil, *scratch*
    // (normal) to 'box -- so 'box here proves the sync used the
    // CURRENT buffer, not the last one the loop visited.
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(
        run(&mut i, "cursor-type"),
        "box",
        "must reflect *scratch* (current, normal -> box), not dired (emacs -> nil)"
    );
    let _ = ed;
}

// --- Issue 7 (low): C-g in insert state should behave like ESC ---

#[test]
fn c_g_in_insert_state_returns_to_normal_like_esc() {
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i a b c");
    assert_eq!(bs(&mut i), "abc");
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "C-g");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(pt(&mut i), 3, "moved left one, like ESC");
    assert_eq!(
        bs(&mut i),
        "abc",
        "C-g doesn't undo the typed text, just exits"
    );
}

// =======================================================================
// M30: Ex commands (`:'), search bridge (`/ ? n N'), dot-repeat (`.')
// =======================================================================

/// Turn arbitrary text into a `feed_keys' description: each character
/// becomes its own space-separated token (a literal space becomes
/// "SPC", `parse_kbd''s spelling for it) -- the same one-token-per-
/// character shape the rest of this file already writes by hand for
/// short sequences (e.g. "h i"), just generated for longer strings
/// (ex-command text, isearch queries, inserted text).
fn type_text(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == ' ' {
                "SPC".to_string()
            } else {
                c.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- Ex commands (`:') -----------------------------------------------
//
// Per the M29-tests-file precedent this section follows (see the
// module doc / the M30 plan): most of the ex-parser's own logic is
// exercised by calling `evil--run-ex' directly (skipping the
// minibuffer's key-by-key UI entirely -- these are unit tests of the
// parser/dispatcher, not of minibuffer plumbing), with a SMALL number
// of true end-to-end tests (typing `:' and letting `feed_keys' drive
// the minibuffer) proving the plumbing itself -- opening the prompt,
// reaching `evil-ex-command', and (for visual state) leaving visual
// first -- actually works.

#[test]
fn ex_w_saves_the_current_buffer() {
    let dir = Scratch::new("ex_w");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    let opened = run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "find-file failed: {}", opened);
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"XYZ\")");
    let r = run(&mut i, "(evil--run-ex \"w\")");
    assert!(!r.starts_with("ERROR"), "evil--run-ex \"w\" failed: {}", r);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "oldXYZ");
    let _ = ed;
}

#[test]
fn ex_w_bang_overwrites_after_a_plain_w_is_refused_on_conflict() {
    // M62: `:w' checks for an on-disk conflict; `:w!' forces past it.
    let dir = Scratch::new("ex_w_bang");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, _ed) = setup();
    let opened = run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "find-file failed: {}", opened);
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"XYZ\")");

    // Someone else changes the file on disk before we save.
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(evil--run-ex \"w\")");
    assert!(
        r.starts_with("ERROR"),
        "expected plain :w to be refused, got: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on disk by someone else"
    );

    let r = run(&mut i, "(evil--run-ex \"w!\")");
    assert!(
        !r.starts_with("ERROR"),
        ":w! should force the save, got: {}",
        r
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "oldXYZ");
}

#[test]
fn ex_q_with_one_window_and_nothing_unsaved_quits() {
    let (mut i, ed) = setup_evil("hello");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    run(&mut i, "(evil--run-ex \"q\")");
    // `hello' lives in a buffer with no associated file (like
    // *scratch*), so `save-buffers-kill-terminal''s unsaved-guard
    // (file-backed AND modified) never triggers -- a plain `:q' quits
    // immediately, same as real vim closing the only window on an
    // unmodified/file-less buffer.
    assert!(
        ed.borrow().quit,
        "plain :q with nothing unsaved should quit"
    );
}

#[test]
fn ex_q_with_unsaved_file_backed_changes_warns_and_does_not_quit() {
    let dir = Scratch::new("ex_q");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(insert \"more\")");
    run(&mut i, "(evil--run-ex \"q\")");
    assert!(
        !ed.borrow().quit,
        ":q must not quit with unsaved file-backed changes"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Unsaved"),
        "expected an Unsaved warning, got {:?}",
        echo
    );
}

#[test]
fn ex_q_pressed_twice_with_unsaved_changes_warns_both_times_never_quits() {
    // M30 review fix (issue 1, high severity -- silent data loss): an
    // earlier `evil-ex-quit' implementation routed through
    // `command-execute', which -- as an ORDINARY side effect of
    // running ANY command -- set `last-command' to `save-buffers-
    // kill-terminal' even on the FIRST plain `:q'. That poisoned the
    // "press again to force" check for the SECOND `:q' (still typed
    // WITHOUT a `!', still expecting just another warning) into
    // silently quitting and discarding the unsaved buffer instead.
    // `evil-ex-quit' now calls `save-buffers-kill-terminal' as a plain
    // funcall (never touching `last-command' at all), so a repeated,
    // un-banged `:q' must warn EVERY time, with no way to force it
    // short of the actual `:q!'.
    let dir = Scratch::new("ex_q_twice");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(insert \"more\")");
    run(&mut i, "(evil--run-ex \"q\")");
    assert!(!ed.borrow().quit, "first :q must only warn");
    assert!(ed
        .borrow()
        .echo
        .clone()
        .unwrap_or_default()
        .contains("Unsaved"));
    run(&mut i, "(evil--run-ex \"q\")");
    assert!(
        !ed.borrow().quit,
        "a SECOND plain :q (no !) must ALSO only warn, never silently force-quit"
    );
    assert!(
        ed.borrow()
            .echo
            .clone()
            .unwrap_or_default()
            .contains("Unsaved"),
        "the second :q should still show the Unsaved warning, got {:?}",
        ed.borrow().echo
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "old", "nothing should have been saved or lost");
}

#[test]
fn ex_q_warning_does_not_cross_arm_a_following_real_c_x_c_c() {
    // The cross-armed half of the same bug: the nested `command-
    // execute' hack didn't just poison a SECOND `:q' -- it poisoned
    // `last-command' for ANY following command, including the real,
    // interactively-bound `C-x C-c'. After an ex `:q' warns, the very
    // next REAL C-x C-c press must still warn the first time too, not
    // silently quit because of leftover state from the ex command.
    let dir = Scratch::new("ex_q_cross_arm");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(insert \"more\")");
    run(&mut i, "(evil--run-ex \"q\")");
    assert!(!ed.borrow().quit, "the ex :q must only warn");
    feed(&mut i, &ed, "C-x C-c");
    assert!(
        !ed.borrow().quit,
        "a real C-x C-c right after an ex :q warning must ALSO warn first, \
         not silently quit due to cross-armed state"
    );
    assert!(ed
        .borrow()
        .echo
        .clone()
        .unwrap_or_default()
        .contains("Unsaved"));
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "old");
}

#[test]
fn ex_q_force_quits_past_the_unsaved_guard_and_discards_the_change() {
    let dir = Scratch::new("ex_qforce");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(insert \"more\")");
    run(&mut i, "(evil--run-ex \"q!\")");
    assert!(
        ed.borrow().quit,
        ":q! should force quit even with unsaved changes"
    );
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk, "old",
        ":q! must not have saved the discarded change"
    );
}

#[test]
fn ex_wq_saves_then_quits() {
    let dir = Scratch::new("ex_wq");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"more\")");
    run(&mut i, "(evil--run-ex \"wq\")");
    assert!(ed.borrow().quit, ":wq should quit after saving");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "oldmore");
}

/// `:wq` must NOT quit when the save is refused.
///
/// M62 leaned on this: the on-disk-conflict guard aborts `save-buffer` by
/// signalling, and `evil--run-ex`'s `wq` arm is a plain two-call body --
/// `(save-buffer) (evil-ex-quit)` -- with no `condition-case` around it. The
/// signal therefore aborts the whole cond clause and `evil-ex-quit` never
/// runs, which is exactly what we want: a save that failed must not take the
/// editor down with the buffer's edits unwritten. That is also why the guard
/// returns `Err` rather than echoing a message and returning nil -- the nil
/// version would let `:wq` quit WITHOUT saving, which is worse than the
/// original bug M62 set out to fix.
///
/// The signal assertion below is NOT decoration -- it is the only half of
/// this test that actually pins the `Err` decision. The first version
/// asserted only `!quit`, and a mutation (guard returns `Ok(nil)` instead of
/// `Err`) SURVIVED it: with the save silently doing nothing the buffer stays
/// modified, so `evil-ex-quit` -> `save-buffers-kill-terminal` refuses to
/// quit on its own "Unsaved: ... — C-x C-c again" guard. Two independent
/// guards both keep `quit` false, so `!quit` cannot tell them apart.
///
/// Note it asserts on the value `evil--run-ex` returns, not on the echo
/// area: these tests drive elisp through `eval_source`, which never goes
/// through `call_command`, and `Editor::echo` is only written there. Asking
/// the echo area what happened returns whatever unrelated message was left
/// in it (observed: `"Theme: dark"`).
#[test]
fn ex_wq_does_not_quit_when_the_save_is_refused() {
    let dir = Scratch::new("ex_wq_refused");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"more\")");
    // An external tool rewrites the file behind the editor's back.
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(evil--run-ex \"wq\")");
    assert!(
        !ed.borrow().quit,
        ":wq must not quit when save-buffer was refused"
    );
    assert!(
        r.starts_with("ERROR") && r.contains("has changed on disk"),
        "the conflict must propagate out of :wq as a signal, not be swallowed: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on disk by someone else",
        "the external change must still be on disk"
    );
}

#[test]
fn ex_x_behaves_like_wq() {
    let dir = Scratch::new("ex_x");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"Z\")");
    run(&mut i, "(evil--run-ex \"x\")");
    assert!(ed.borrow().quit, ":x should quit after saving");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "oldZ");
}

#[test]
fn ex_q_with_multiple_windows_closes_just_the_window() {
    let (mut i, ed) = setup_evil("hello");
    run(&mut i, "(split-window-below)");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    run(&mut i, "(evil--run-ex \"q\")");
    assert_eq!(run(&mut i, "(window-count)"), "1");
    assert!(
        !ed.borrow().quit,
        "closing one of several windows must not quit the editor"
    );
}

#[test]
fn ex_e_opens_a_file() {
    let dir = Scratch::new("ex_e");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("target.txt");
    std::fs::write(&path, "target contents").unwrap();
    let (mut i, ed) = setup_evil("hello");
    let arg = format!("e {}", path.to_str().unwrap());
    let cmd = format!("(evil--run-ex {:?})", arg);
    let r = run(&mut i, &cmd);
    assert!(
        !r.starts_with("ERROR"),
        "evil--run-ex {:?} failed: {}",
        arg,
        r
    );
    assert_eq!(bs(&mut i), "target contents");
    let _ = ed;
}

#[test]
fn ex_e_with_no_path_echoes_a_message_and_does_not_error() {
    let (mut i, ed) = setup_evil("hello");
    let r = run(&mut i, "(evil--run-ex \"e\")");
    assert!(!r.starts_with("ERROR"), "{}", r);
    assert_eq!(bs(&mut i), "hello", "no file should have been visited");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("file name required"), "got {:?}", echo);
}

#[test]
fn ex_numeric_goes_to_that_line() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour");
    run(&mut i, "(evil--run-ex \"3\")");
    assert_eq!(pt(&mut i), 9); // first non-blank of "three"
    let _ = ed;
}

#[test]
fn ex_dollar_goes_to_the_last_line() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour");
    run(&mut i, "(evil--run-ex \"$\")");
    assert_eq!(pt(&mut i), 15); // first non-blank of "four"
    let _ = ed;
}

#[test]
fn ex_unknown_command_echoes_not_an_editor_command() {
    let (mut i, ed) = setup_evil("hello");
    run(&mut i, "(evil--run-ex \"bogus\")");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Not an editor command: bogus");
}

#[test]
fn ex_empty_input_is_a_no_op() {
    let (mut i, ed) = setup_evil("hello");
    let before_pt = pt(&mut i);
    run(&mut i, "(evil--run-ex \"\")");
    assert_eq!(bs(&mut i), "hello");
    assert_eq!(pt(&mut i), before_pt);
    let _ = ed;
}

#[test]
fn colon_opens_the_ex_minibuffer_with_a_bare_colon_prompt() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, ":");
    let prompt = ed.borrow().minibuffer.as_ref().map(|mb| mb.prompt.clone());
    assert_eq!(prompt.as_deref(), Some(":"));
    feed(&mut i, &ed, "ESC");
    assert!(ed.borrow().minibuffer.is_none());
}

#[test]
fn colon_w_end_to_end_via_the_minibuffer_saves() {
    let dir = Scratch::new("colon_w");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.txt");
    std::fs::write(&path, "old").unwrap();
    let (mut i, ed) = setup();
    run(&mut i, &format!("(find-file {:?})", path.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"Z\")");
    feed(&mut i, &ed, ": w RET");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "oldZ");
}

#[test]
fn visual_colon_returns_to_normal_before_the_prompt_and_has_no_range_support() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v l l");
    assert_eq!(run(&mut i, "evil--state"), "visual");
    feed(&mut i, &ed, ":");
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        "visual state must be left before/while the prompt opens"
    );
    assert!(
        ed.borrow().minibuffer.is_some(),
        "the : prompt should now be open"
    );
    feed(&mut i, &ed, "ESC");
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(
        bs(&mut i),
        "hello world",
        "no range/command should have run"
    );
}

// --- Search bridge (`/ ? n N') -----------------------------------------

const SEARCH_FIXTURE: &str = "foo bar foo baz foo qux";
// 1-based points: foo(1-3) bar(5-7) foo(9-11) baz(13-15) foo(17-19)
// qux(21-23); point-max = 24.

#[test]
fn slash_search_lands_after_the_first_match() {
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    feed(&mut i, &ed, &format!("/ {} RET", type_text("foo")));
    assert_eq!(pt(&mut i), 4);
    assert_eq!(bs(&mut i), SEARCH_FIXTURE);
}

#[test]
fn question_mark_search_lands_on_the_start_of_the_match_going_backward() {
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, &format!("? {} RET", type_text("foo")));
    assert_eq!(pt(&mut i), 17);
    assert_eq!(bs(&mut i), SEARCH_FIXTURE);
}

#[test]
fn n_repeats_the_last_search_forward_and_wraps() {
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    feed(&mut i, &ed, &format!("/ {} RET", type_text("foo")));
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "n");
    assert_eq!(pt(&mut i), 12);
    feed(&mut i, &ed, "n");
    assert_eq!(pt(&mut i), 20);
    feed(&mut i, &ed, "n"); // no more forward -- wraps to the first match
    assert_eq!(pt(&mut i), 4);
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("wrapped"),
        "expected a wrap notice, got {:?}",
        echo
    );
    assert_eq!(
        bs(&mut i),
        SEARCH_FIXTURE,
        "n/N must never modify the buffer"
    );
}

#[test]
fn capital_n_reverses_the_search_direction_and_wraps() {
    // `N' repeats in the OPPOSITE direction from how the search
    // originally ran (vim semantics) -- after a backward `?', `N'
    // searches FORWARD, landing past the match (Emacs-style, like `/'
    // itself) rather than continuing backward the way `n' would (see
    // `n_after_a_backward_search_continues_backward').
    //
    // Position derivation: `?' from point-max lands on the THIRD
    // "foo" (positions 17-19, 1-based; see SEARCH_FIXTURE's own
    // comment), backward convention = START = 17. `N' reverses to
    // forward; searching forward from there finds nothing (there is
    // no fourth "foo"), so this SINGLE `N' both reverses AND wraps in
    // one step, landing on the FIRST "foo" (positions 1-3), forward
    // convention = END = 4.
    //
    // An earlier, BUGGY version of this test asserted `N' lands back
    // at 20 instead -- that was the bug (issue 2 in the M30 review):
    // `find_forward'/`find_backward' are inclusive of the boundary
    // point already rests on, so a naive reversal immediately re-finds
    // the SAME third "foo" it started at (its end, 20, since the
    // candidate search this time is forward) and reports that
    // self-match as if it were "the next occurrence" -- wrong, and not
    // real vim semantics (real vim would find nothing forward of the
    // last match and wrap to the first one, exactly what's asserted
    // below). Fixed via `evil--last-match-span' in evil.el, which
    // detects "the candidate equals the span I'm already at the edge
    // of" and searches past it once more before accepting.
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, &format!("? {} RET", type_text("foo")));
    assert_eq!(
        pt(&mut i),
        17,
        "? lands on the third \"foo\" (start, backward convention)"
    );
    feed(&mut i, &ed, "N");
    assert_eq!(
        pt(&mut i),
        4,
        "N reverses to forward; nothing lies forward of the third \"foo\" so this \
         wraps directly to the first \"foo\" (end, forward convention) -- NOT a \
         self-match on the third \"foo\" it started at"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("wrapped"),
        "expected a wrap notice, got {:?}",
        echo
    );
    // A second `N': still forward (`N' always reverses relative to the
    // ORIGINAL `?', not relative to the previous `N'), continuing from
    // the first "foo" (point 4) -> lands on the second "foo"
    // (positions 9-11), forward convention = END = 12. No wrap this
    // time (a genuinely later match exists within the buffer).
    feed(&mut i, &ed, "N");
    assert_eq!(pt(&mut i), 12);
    assert_eq!(
        bs(&mut i),
        SEARCH_FIXTURE,
        "n/N must never modify the buffer"
    );
}

#[test]
fn capital_n_after_n_reverses_to_the_earlier_match_without_wrapping() {
    // The non-wrapping shape of the same fix: `/' lands on the first
    // "foo" (end, 4); `n' continues forward to the second "foo" (end,
    // 12); `N' then reverses to backward FROM there -- a naive
    // backward search from 12 immediately re-matches the SAME second
    // "foo" (its start, 9, is exactly the boundary `find_backward'
    // treats as reachable from position 12), so without the fix this
    // would report position 9 as "the previous match" when it's
    // really just the one already under point. The fix must search
    // PAST it and land on the genuinely earlier, FIRST "foo" (start,
    // 1) instead -- entirely without needing to wrap, since a real
    // earlier match exists in the buffer.
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    feed(&mut i, &ed, &format!("/ {} RET", type_text("foo")));
    assert_eq!(pt(&mut i), 4);
    feed(&mut i, &ed, "n");
    assert_eq!(pt(&mut i), 12);
    feed(&mut i, &ed, "N");
    assert_eq!(
        pt(&mut i),
        1,
        "N should reverse to backward and land on the FIRST foo, not re-match the second"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        !echo.contains("wrapped"),
        "a genuinely earlier match exists in the buffer -- no wrap should be needed, got echo {:?}",
        echo
    );
}

#[test]
fn n_after_a_backward_search_continues_backward() {
    // `n' repeats in the ORIGINAL search's direction, not always
    // forward -- after a `?' search, `n' must also search backward.
    let (mut i, ed) = setup_evil(SEARCH_FIXTURE);
    run(&mut i, "(goto-char (point-max))");
    feed(&mut i, &ed, &format!("? {} RET", type_text("foo")));
    assert_eq!(pt(&mut i), 17);
    feed(&mut i, &ed, "n");
    assert_eq!(
        pt(&mut i),
        9,
        "n after ? should continue backward, not reverse"
    );
}

#[test]
fn n_with_only_one_occurrence_in_the_buffer_wraps_back_to_itself() {
    // A buffer with exactly ONE match for the search string: `n' has
    // nothing else to find in either direction, so it must wrap back
    // onto that SAME match (not error, hang, or infinite-loop trying
    // to find a "different" one) and still echo the wrap notice.
    // "only" is the sole match: a(1)l(2)p(3)h(4)a(5) (6)o(7)n(8)l(9)
    // y(10) (11)b(12)r(13)a(14)v(15)o(16) -- end (forward convention)
    // = 11.
    let (mut i, ed) = setup_evil("alpha only bravo");
    feed(&mut i, &ed, &format!("/ {} RET", type_text("only")));
    assert_eq!(pt(&mut i), 11);
    feed(&mut i, &ed, "n");
    assert_eq!(
        pt(&mut i),
        11,
        "n with a single match in the buffer wraps back onto it"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("wrapped"),
        "expected a wrap notice, got {:?}",
        echo
    );
    assert_eq!(
        bs(&mut i),
        "alpha only bravo",
        "n must never modify the buffer"
    );
}

// --- Dot-repeat (`.') ---------------------------------------------------

#[test]
fn dw_then_dot_deletes_another_word() {
    let (mut i, ed) = setup_evil("one two three four");
    feed(&mut i, &ed, "d w");
    assert_eq!(bs(&mut i), "two three four");
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "three four");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn x_then_dot_deletes_another_character() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello");
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "llo");
}

#[test]
fn three_x_then_dot_repeats_with_the_recorded_count_of_three() {
    let (mut i, ed) = setup_evil("abcdefgh");
    feed(&mut i, &ed, "3 x");
    assert_eq!(bs(&mut i), "defgh");
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "gh",
        ". must reuse the recorded count of 3, not default to 1"
    );
}

#[test]
fn ciw_then_typed_text_then_esc_then_dot_replays_elsewhere() {
    let (mut i, ed) = setup_evil("foo bar baz");
    feed(&mut i, &ed, "w"); // point on "bar"
    feed(&mut i, &ed, "c i w");
    assert_eq!(bs(&mut i), "foo  baz");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, &format!("{} ESC", type_text("abc")));
    assert_eq!(bs(&mut i), "foo abc baz");
    assert_eq!(run(&mut i, "evil--state"), "normal");
    feed(&mut i, &ed, "w"); // move onto "baz"
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "foo abc abc",
        ". should redo ciw (delete the word under point) then retype \"abc\""
    );
    assert_eq!(
        run(&mut i, "evil--state"),
        "normal",
        ". must leave insert state again, like the original ESC"
    );
}

#[test]
fn p_then_dot_pastes_again() {
    let (mut i, ed) = setup_evil("alpha\nbeta");
    feed(&mut i, &ed, "y y");
    feed(&mut i, &ed, "p");
    assert_eq!(bs(&mut i), "alpha\nalpha\nbeta");
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "alpha\nalpha\nalpha\nbeta");
}

// --- M30 review fix (issue 4): yank must never be dot-repeatable -------
//
// An early M30 draft recorded `Y' (and, by the same shared code path,
// any operator+motion/text-object combo with the `yank' operator --
// yw/y$/yiw/...) for dot-repeat, reasoning it was explicitly listed
// alongside the delete/change simple edits. Review caught that this is
// actively harmful, not just inauthentic: replaying a yank at a NEW
// position silently overwrites the kill-ring with different text,
// discarding whatever the user last yanked on purpose. Real vim's `.'
// never repeats a yank at all -- `evil--record-change' now refuses to
// record while `evil--pending-operator' is `yank', so `evil--last-
// change' simply keeps whatever the last actual delete/change was.

#[test]
fn yank_motion_is_never_recorded_for_dot_repeat() {
    let (mut i, ed) = setup_evil("one two three four five");
    feed(&mut i, &ed, "d w");
    assert_eq!(bs(&mut i), "two three four five");
    feed(&mut i, &ed, "y w");
    assert_eq!(
        bs(&mut i),
        "two three four five",
        "yank must not modify the buffer"
    );
    assert_eq!(
        run(&mut i, "(current-kill)"),
        "\"two \"",
        "yw should still update the kill-ring like any yank"
    );
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "three four five",
        ". must replay the earlier dw, NOT the intervening yw -- had yw been \
         replayed instead, the buffer would be unchanged here since a yank never \
         modifies the buffer"
    );
}

#[test]
fn capital_y_yank_line_is_never_recorded_for_dot_repeat() {
    let (mut i, ed) = setup_evil("one\ntwo\nthree");
    feed(&mut i, &ed, "d d");
    assert_eq!(bs(&mut i), "two\nthree");
    feed(&mut i, &ed, "Y");
    assert_eq!(bs(&mut i), "two\nthree", "Y must not modify the buffer");
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "three",
        ". must still replay the earlier dd, not Y -- Y is never dot-repeatable"
    );
}

#[test]
fn yiw_text_object_is_never_recorded_for_dot_repeat() {
    let (mut i, ed) = setup_evil("aaa bbb ccc");
    feed(&mut i, &ed, "d w"); // records dw: deletes "aaa "
    assert_eq!(bs(&mut i), "bbb ccc");
    feed(&mut i, &ed, "y i w"); // yanks "bbb" in place -- must not overwrite last-change
    assert_eq!(bs(&mut i), "bbb ccc");
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "ccc",
        ". must replay dw again (deleting \"bbb \"), not yiw"
    );
}

#[test]
fn undo_does_not_change_what_dot_repeats() {
    let (mut i, ed) = setup_evil("one two three");
    feed(&mut i, &ed, "d w");
    assert_eq!(bs(&mut i), "two three");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "one two three",
        "undo should restore the deleted word"
    );
    // Normalize point (undo's own point-placement isn't what this test
    // is about) before checking that `.' is STILL "delete a word".
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "two three",
        "`.' must still replay dw -- u must not have touched evil--last-change"
    );
}

#[test]
fn o_then_typed_text_then_esc_then_dot_opens_another_line_elsewhere() {
    let (mut i, ed) = setup_evil("first\nlast");
    feed(&mut i, &ed, &format!("o {} ESC", type_text("x")));
    assert_eq!(bs(&mut i), "first\nx\nlast");
    feed(&mut i, &ed, "j"); // move onto "last"
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "first\nx\nlast\nx");
}

#[test]
fn count_before_dot_repeats_the_whole_change_that_many_times() {
    // Stretch goal: `3.' replays the recorded change THREE TIMES in a
    // row (not "override the recorded count with 3", real vim's own
    // `:help .' semantics -- see `evil-repeat-change''s docstring).
    let (mut i, ed) = setup_evil("abcdefgh");
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "bcdefgh");
    feed(&mut i, &ed, "3 .");
    assert_eq!(bs(&mut i), "efgh");
}

#[test]
fn r_then_dot_replays_with_the_same_captured_character() {
    // `r''s dot-repeat is recorded in `evil--do-replace-char' itself
    // (not the generic `evil--replay-thunk' shape every OTHER simple
    // edit uses): the captured replacement character has to be reused
    // directly, not re-captured via another `capture-next-key'.
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "r z");
    assert_eq!(bs(&mut i), "zello");
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, "l l"); // move onto the first "l" (point 3)
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "zezlo",
        ". should replace the character under point with the SAME captured 'z'"
    );
    assert_eq!(pt(&mut i), 3);
}

#[test]
fn d_count_after_operator_then_f_then_dot_replays_with_the_frozen_count() {
    // M44 regression guard: the count typed AFTER the operator (the
    // "2" of "d 2 f X", the exact ordering M44 fixes -- see
    // `count_typed_after_the_operator_still_extends_a_find_to_the_nth_
    // occurrence' above) must be frozen into `evil--replay-find-thunk'
    // as N=2, not silently re-read as N=1 on replay. A BARE "3fX" can't
    // pin this: per the file's dot-repeat rules, a bare (operator-less)
    // find is a pure motion and is never recorded as `evil--last-
    // change' at all, so there'd be nothing for "." to replay.
    let (mut i, ed) = setup_evil("abcXdefXghiXjklXmnoXpqrXstu");
    feed(&mut i, &ed, "d 2 f X");
    assert_eq!(bs(&mut i), "ghiXjklXmnoXpqrXstu");
    assert_eq!(pt(&mut i), 1);
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "mnoXpqrXstu",
        ". must reuse the frozen count of 2, deleting through the SECOND \
         X from the new point (not just the first, which is what a \
         regression back to N=1 would produce)"
    );
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn capital_s_then_typed_text_then_esc_then_dot_replays_on_another_line() {
    // S (change-line) shares `evil--op-current-lines' with dd/cc/yy/Y
    // -- exercises that the insert-session hookup also fires correctly
    // through THAT path (linewise), not just the charwise operator+
    // motion path `ciw' already covers.
    let (mut i, ed) = setup_evil("aaa\nbbb\nccc");
    feed(&mut i, &ed, "S");
    assert_eq!(bs(&mut i), "\nbbb\nccc");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed(&mut i, &ed, "X ESC");
    assert_eq!(bs(&mut i), "X\nbbb\nccc");
    feed(&mut i, &ed, "j");
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "X\nX\nccc");
}

#[test]
fn capital_j_then_dot_repeats_the_join_elsewhere() {
    // J captures its count BEFORE the `(max 2 ...)' clamp (`evil-join'
    // records `n0', not the clamped `n') -- confirms the replay
    // reproduces "join with count 1" (i.e. join 2 lines), not
    // literally "join `(max 2 1)' lines" baked in as a constant 2
    // \(same numeric value here, but recorded via the pre-clamp count,
    // per the function's own docstring/implementation).
    let (mut i, ed) = setup_evil("one\ntwo\nthree\nfour");
    feed(&mut i, &ed, "J");
    assert_eq!(bs(&mut i), "one two\nthree\nfour");
    run(&mut i, "(goto-char 9)"); // start of "three"
    feed(&mut i, &ed, ".");
    assert_eq!(bs(&mut i), "one two\nthree four");
}

// =======================================================================
// M32: visual feedback -- the `-- INSERT --'/`-- VISUAL --' echo
// indicator, `cursor-type' unification, and evil-mode-off cleanup.
// (hl-line-mode's own default-on behavior is exercised in
// `gui_features_tests.rs', alongside the rest of the M16 render tests
// it builds on.)
// =======================================================================

#[test]
fn echo_area_shows_a_vim_style_state_indicator_that_disappears_in_normal_state() {
    let (mut i, ed) = setup_evil("hello world");
    // Startup's `(load-theme 'dracula)' (themes.el, M107) leaves an
    // unrelated "Theme: dracula" `editor.echo' message sitting from
    // BEFORE any real keypress -- ordinarily cleared by the first
    // `handle_key' call (see commands.rs), same as it is here.
    ed.borrow_mut().echo = None;
    assert_eq!(run(&mut i, "evil--state"), "normal");
    assert_eq!(
        echo_row_text(&i, &ed),
        "",
        "normal state shows no indicator"
    );
    feed(&mut i, &ed, "i");
    assert_eq!(echo_row_text(&i, &ed), "-- INSERT --");
    feed(&mut i, &ed, "ESC");
    assert_eq!(echo_row_text(&i, &ed), "", "back to normal, indicator gone");
    feed(&mut i, &ed, "v");
    assert_eq!(echo_row_text(&i, &ed), "-- VISUAL --");
    feed(&mut i, &ed, "v"); // same-type `v' toggles visual back off
    assert_eq!(echo_row_text(&i, &ed), "");
    feed(&mut i, &ed, "V");
    assert_eq!(echo_row_text(&i, &ed), "-- VISUAL LINE --");
    feed(&mut i, &ed, "ESC");
    assert_eq!(echo_row_text(&i, &ed), "");
}

#[test]
fn a_message_during_insert_shows_over_the_indicator_then_it_returns_on_the_next_key() {
    // Exact vim behavior this pins: a real message (dabbrev's "no
    // expansion" echo, via C-n -- M31) temporarily COVERS the "--
    // INSERT --" indicator; once that message is cleared (every key
    // clears `editor.echo' first -- see commands.rs's `handle_key'),
    // the indicator reappears on its own, with no further evil.el
    // involvement needed (see redisplay.rs's three-layer echo area).
    let (mut i, ed) = setup_evil("");
    feed(&mut i, &ed, "i");
    assert_eq!(echo_row_text(&i, &ed), "-- INSERT --");
    feed(&mut i, &ed, "C-n"); // dabbrev completion; nothing to expand here
    assert_eq!(
        echo_row_text(&i, &ed),
        "No dynamic expansion possible here",
        "a real message must win over the indicator"
    );
    feed(&mut i, &ed, "x"); // any next key clears the message
    assert_eq!(
        echo_row_text(&i, &ed),
        "-- INSERT --",
        "the indicator must return once the message is gone"
    );
}

/// The nil-fallback side of the same mechanism (M32 review finding 3):
/// in normal state there is no indicator, so a transient message must
/// clear back to an EMPTY echo row on the next key — not leave any
/// residue.
#[test]
fn a_message_in_normal_state_clears_back_to_an_empty_echo_row() {
    let (mut i, ed) = setup_evil("hello");
    ed.borrow_mut().echo = None; // shed startup theme message
    assert_eq!(echo_row_text(&i, &ed), "", "normal state: no indicator");
    run(&mut i, "(message \"transient\")");
    assert_eq!(echo_row_text(&i, &ed), "transient");
    feed(&mut i, &ed, "l"); // any key clears editor.echo
    assert_eq!(
        echo_row_text(&i, &ed),
        "",
        "no fallback in normal state: the row must be blank again"
    );
}

#[test]
fn cursor_type_reflects_each_evil_state() {
    let (mut i, ed) = setup_evil("hello world");
    assert_eq!(run(&mut i, "cursor-type"), "box", "normal state");
    feed(&mut i, &ed, "i");
    assert_eq!(run(&mut i, "cursor-type"), "bar", "insert state");
    feed(&mut i, &ed, "ESC");
    assert_eq!(run(&mut i, "cursor-type"), "box", "back to normal");
    feed(&mut i, &ed, "v");
    assert_eq!(run(&mut i, "cursor-type"), "box", "visual state");
    feed(&mut i, &ed, "v"); // toggle visual back off
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    assert_eq!(run(&mut i, "cursor-type"), "box", "operator-pending state");
    feed(&mut i, &ed, "w"); // completes "dw", back to normal
    assert_eq!(run(&mut i, "evil--state"), "normal");
    let _ = ed;
}

#[test]
fn evil_mode_off_clears_the_echo_indicator_and_hands_the_cursor_back() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "i");
    assert_eq!(echo_row_text(&i, &ed), "-- INSERT --");
    assert_eq!(run(&mut i, "cursor-type"), "bar");
    run(&mut i, "(evil-mode -1)");
    assert_eq!(
        echo_row_text(&i, &ed),
        "",
        "indicator must clear when evil-mode turns off"
    );
    // nil, not 'box: the M32 review caught 'box pinning a steady-block
    // DECSCUSR over the user's own terminal cursor after evil bowed
    // out — nil routes through the TUI's reset-on-clear path instead,
    // the exact symptom the M28 review fix exists to prevent.
    assert_eq!(
        run(&mut i, "cursor-type"),
        "nil",
        "cursor handed back to the terminal default"
    );
}

/// dired/eshell/ielm buffers (emacs state) are ones evil deliberately
/// does not manage — the cursor there must stay the terminal's own,
/// not evil's steady box (M32 review finding 1).
#[test]
fn emacs_state_buffer_leaves_the_cursor_to_the_terminal() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("cursor_nil");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    // Same establishment order as emacs_state_buffer_j_is_dired...:
    // dired first, then evil-mode 1, whose buffer-list pass assigns
    // dired its emacs state and whose final refresh reflects the
    // now-current dired buffer.
    let opened = run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "dired failed: {}", opened);
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "evil--state"), "emacs");
    assert_eq!(run(&mut i, "cursor-type"), "nil");
    let _ = ed;
}

// =======================================================================
// M34: `inhibit-self-insert` -- normal/visual/operator-pending must not
// let an unbound printable character (CJK text above all, since no
// fixed-size vim keymap could ever enumerate it, but also a plain
// unbound ASCII key like `q` -- the gap the M30 review already flagged
// for normal state specifically) fall through to plain self-insert. See
// simple.el's `inhibit-self-insert` docstring and evil.el's
// `evil--set-state`.
// =======================================================================

/// A single CJK character used throughout this section purely as "some
/// printable character no vim keymap enumerates" -- `parse_kbd`/
/// `feed_keys` (via `feed`) already support it with no change: a `char`
/// in Rust is one Unicode scalar value regardless of byte width, and
/// `keymap::parse_one` treats any single-`char` word (ASCII or not) as a
/// bare `Key::Char`, exactly like an ASCII letter -- verified by these
/// tests actually passing, rather than falling back to constructing
/// `Key::Char` and calling `handle_key` directly (also available, see
/// e.g. `complete_tests.rs`, but not needed here).
const CJK_CHAR: &str = "界";

#[test]
fn normal_state_blocks_an_unbound_cjk_character_from_self_inserting() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, CJK_CHAR);
    assert_eq!(bs(&mut i), "hello world", "must not have self-inserted");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(
        ed.borrow().echo.clone().as_deref(),
        Some("界 is undefined"),
        "same undefined-key echo format as any other unbound key"
    );
}

#[test]
fn normal_state_blocks_an_unbound_ascii_key_from_self_inserting() {
    // M30 review fix (a different issue in the same milestone) explicitly
    // noted normal state has no op-pending-style ASCII catch-all; M34
    // closes that gap via `inhibit-self-insert` rather than an ASCII
    // enumeration (which couldn't cover CJK anyway).
    //
    // `q` was this test's original example key (still unbound as of
    // M34) until M42-II claimed it for `evil-record-macro` -- `z' is
    // the replacement (real vim's fold/scroll-window prefix, none of
    // which this editor implements, so it stays genuinely unbound).
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "z"); // unbound in evil--normal-map
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 1);
    assert_eq!(ed.borrow().echo.clone().as_deref(), Some("z is undefined"));
}

#[test]
fn insert_state_allows_a_cjk_character_to_self_insert() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, &format!("i {CJK_CHAR}"));
    assert_eq!(bs(&mut i), "界hello world");
    assert_eq!(pt(&mut i), 2);
    assert_eq!(run(&mut i, "evil--state"), "insert");
}

#[test]
fn visual_state_blocks_a_cjk_character_from_self_inserting() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "v");
    assert_eq!(run(&mut i, "evil--state"), "visual");
    feed(&mut i, &ed, CJK_CHAR);
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(ed.borrow().echo.clone().as_deref(), Some("界 is undefined"));
}

/// `emacs` state (dired/eshell/ielm) must NOT set `inhibit-self-insert`
/// -- confirmed here via the ECHO MESSAGE, not just "buffer unchanged"
/// (dired is read-only regardless, so an unchanged buffer alone can't
/// distinguish "never attempted self-insert" from "attempted and was
/// turned away"): dispatch must still reach `self_insert`, which then
/// gets turned away by the read-only check (a DIFFERENT echo than the
/// undefined-key one) -- exactly the difference a wrongly-`t`
/// `inhibit-self-insert` for `emacs` state would erase.
#[test]
fn emacs_state_is_not_inhibited_from_attempting_self_insert() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("inhibit_emacs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    let opened = run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!opened.starts_with("ERROR"), "dired failed: {}", opened);
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "evil--state"), "emacs");
    let before = bs(&mut i);
    feed(&mut i, &ed, CJK_CHAR); // unbound in dired; blocked only by read-only-ness
    assert_eq!(bs(&mut i), before, "dired is read-only regardless");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("read-only"),
        "emacs state must leave inhibit-self-insert nil -- dispatch should still attempt \
         self-insert and be turned away by the read-only check, not the undefined-key echo \
         (got {:?})",
        echo
    );
}

#[test]
fn evil_mode_off_restores_plain_self_insert_for_cjk_and_unbound_ascii() {
    // `q` was this test's original ASCII example (still unbound as of
    // M34) until M42-II claimed it for `evil-record-macro` -- `z' is
    // the replacement (real vim's unimplemented fold/scroll prefix, so
    // it stays genuinely unbound both here and with evil-mode off).
    // Using a now-BOUND key here would still leave the buffer
    // unchanged (armed a macro-recording capture instead of inserting
    // -- coincidentally the same observable buffer state), but would
    // no longer be testing `inhibit-self-insert' at all.
    let (mut i, ed) = setup_evil("ab");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "z"); // blocked: unbound ASCII while evil's normal state is active
    assert_eq!(bs(&mut i), "ab");
    feed(&mut i, &ed, CJK_CHAR); // blocked: CJK, same reason
    assert_eq!(bs(&mut i), "ab");

    run(&mut i, "(evil-mode -1)");
    assert_eq!(run(&mut i, "(local-variable-p 'inhibit-self-insert)"), "t");
    assert_eq!(run(&mut i, "inhibit-self-insert"), "nil");

    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "z");
    assert_eq!(bs(&mut i), "zab", "z now self-inserts with evil off");
    assert_eq!(pt(&mut i), 2);

    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, CJK_CHAR);
    assert_eq!(
        bs(&mut i),
        "界zab",
        "CJK now self-inserts with evil off too"
    );
}

/// `inhibit-self-insert` must only ever gate the self-insert FALLBACK,
/// never the three keymap lookups themselves -- a real binding anywhere
/// in emulation/local/global still dispatches normally under evil's
/// normal state.
#[test]
fn normal_state_real_bindings_still_dispatch_under_inhibit_self_insert() {
    let (mut i, ed) = setup_evil("hello world");
    feed(&mut i, &ed, "C-x b");
    assert!(
        ed.borrow()
            .minibuffer
            .as_ref()
            .and_then(|m| m.panel.as_ref())
            .is_some(),
        "C-x b must still open the buffer-switch panel under evil's normal state"
    );
}

/// M120 E3: `evil--halfpage` (C-d/C-u) must use the SELECTED window's own
/// text height, not the frame's -- in a split those differ, and before
/// this fix C-d/C-u scrolled by half the frame even in a short window.
/// The frame is 41 rows tall (`windows_height` = 40, one row reserved for
/// the echo area), so the pre-fix `frame-height'/2 = 20. The window is
/// split down to 6 rows (`window-height` includes its own mode-line row,
/// so text height = 5); the correct half-page count is 5/2 = 2, not 20.
#[test]
fn evil_scroll_down_uses_selected_windows_height_not_frames() {
    // 20 lines of "x\n" (2 chars each) so point position is
    // `1 + line_index * 2`, letting the expected point be computed
    // arithmetically rather than pinned to a magic number.
    let text = "x\n".repeat(20);
    let (mut i, ed) = setup_evil(&text);
    ed.borrow_mut().frame = (50, 41);
    run(&mut i, "(goto-char (point-min))");
    assert_eq!(pt(&mut i), 1);

    run(&mut i, "(split-window-below 6)");
    assert_eq!(run(&mut i, "(window-count)"), "2");
    // The original (top) window stays selected after splitting, and is
    // the short one.
    assert_eq!(run(&mut i, "(window-height 0)"), "6");
    assert_eq!(run(&mut i, "(selected-window)"), "0");

    // Sanity check on the numbers this test's expectation depends on.
    assert_eq!(run(&mut i, "(frame-height)"), "41");
    assert_eq!(
        run(&mut i, "(max 1 (/ (frame-height) 2))"),
        "20",
        "pre-fix formula, kept here only to document what this test would \
         have wrongly accepted before M120"
    );

    run(&mut i, "(evil-scroll-down)");
    // Selected window's text height is 6 - 1 = 5; half of that is 2, so
    // point moves from line 1 to line 3 (0-indexed line 2): 1 + 2*2 = 5.
    assert_eq!(
        pt(&mut i),
        5,
        "C-d must scroll by half the SELECTED window's text height (2 \
         lines here), not half the frame's (20 lines)"
    );
}
