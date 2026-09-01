//! M31: dabbrev.el -- buffer-internal text completion. The engine
//! itself (prefix scanning, candidate collection/ordering, session/
//! cycling) lives in `crates/core/lisp/dabbrev.el`; evil.el adds the
//! insert-state `C-n'/`C-p' entry points (`evil-complete-next'/
//! `evil-complete-previous') on top of it, and `M-/' (`dabbrev-expand')
//! is bound globally. See both files' headers for the full design.
//!
//! `setup_evil`/`feed`/`bs`/`pt` mirror evil_tests.rs's helpers of the
//! same names. Buffer positions below are computed from fixture string
//! lengths (`.len()') rather than hand-counted, to keep the arithmetic
//! honest.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
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
            "se_dabbrev_{}_{}_{}",
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

/// Raw buffer content (unlike `run`, not the `prin1` printed form).
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

fn echo(ed: &Rc<RefCell<Editor>>) -> String {
    ed.borrow().echo.clone().unwrap_or_default()
}

fn goto(interp: &mut Interp, pos: i64) {
    run(interp, &format!("(goto-char {})", pos));
}

/// Type STR one character at a time (mirrors how every other test in
/// this codebase feeds typed text, e.g. evil_tests.rs's `"i a b c"' --
/// `feed_keys' tokenizes on whitespace, so a single space-joined string
/// can't type multi-char text as one token).
fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    let spaced = s
        .chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    feed(interp, ed, &spaced);
}

// ---------------------------------------------------------------------
// Prefix boundaries
// ---------------------------------------------------------------------

#[test]
fn empty_prefix_after_a_non_prefix_char_does_nothing() {
    let (mut i, ed) = setup_evil("foo(");
    goto(&mut i, "foo(".len() as i64 + 1); // right after "("
    feed(&mut i, &ed, "i");
    feed(&mut i, &ed, "C-n");
    assert_eq!(echo(&ed), "No dynamic expansion possible here");
    assert_eq!(bs(&mut i), "foo(");
    assert_eq!(pt(&mut i), 5);
}

#[test]
fn empty_prefix_at_absolute_buffer_start_does_nothing() {
    let (mut i, ed) = setup_evil("foo"); // point already at point-min
    feed(&mut i, &ed, "i");
    feed(&mut i, &ed, "C-n");
    assert_eq!(echo(&ed), "No dynamic expansion possible here");
    assert_eq!(bs(&mut i), "foo");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn prefix_scan_stops_at_the_start_of_the_line_not_the_previous_line() {
    // "printhis\n" then an empty second line where "pri" gets typed --
    // if the backward scan wrongly crossed the newline, the effective
    // prefix would be something other than "pri" and this specific
    // candidate wouldn't come out.
    let fixture = "printhis\n";
    let (mut i, ed) = setup_evil(fixture);
    goto(&mut i, fixture.len() as i64 + 1); // the empty second line
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-p"); // forward direction: nearest BEFORE first
    assert_eq!(bs(&mut i), "printhis\nprinthis");
    assert_eq!(pt(&mut i), "printhis\nprinthis".len() as i64 + 1);
}

#[test]
fn prefix_reaching_all_the_way_back_to_point_min_works() {
    // A blank first line, "pri" typed at the very start of the buffer
    // (prefix-start ends up being `point-min' itself, not just "the
    // start of a line").
    let (mut i, ed) = setup_evil("\nprintln\n");
    feed(&mut i, &ed, "i"); // point is already point-min
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "println\nprintln\n");
    assert_eq!(pt(&mut i), "println".len() as i64 + 1);
}

#[test]
fn prefix_containing_underscore_and_hyphen() {
    let fixture = "foo_bar-baz\n\n";
    let (mut i, ed) = setup_evil(fixture);
    goto(&mut i, fixture.len() as i64 + 1); // the trailing blank line
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "foo_bar-");
    feed(&mut i, &ed, "C-p"); // nearest before
    assert_eq!(bs(&mut i), "foo_bar-baz\n\nfoo_bar-baz");
}

// ---------------------------------------------------------------------
// Direction: C-n (nearest AFTER point first) vs C-p (nearest BEFORE)
// ---------------------------------------------------------------------

/// "printhis" before the typing position (the blank 2nd line), "println"
/// after it.
fn before_after_fixture() -> (String, i64) {
    let text = "printhis\n\nprintln\n".to_string();
    let blank_pos = "printhis\n".len() as i64 + 1;
    (text, blank_pos)
}

#[test]
fn evil_insert_c_n_expands_to_the_nearest_match_after_point() {
    let (text, blank_pos) = before_after_fixture();
    let (mut i, ed) = setup_evil(&text);
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "printhis\nprintln\nprintln\n");
}

#[test]
fn evil_insert_c_p_expands_to_the_nearest_match_before_point() {
    let (text, blank_pos) = before_after_fixture();
    let (mut i, ed) = setup_evil(&text);
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-p");
    assert_eq!(bs(&mut i), "printhis\nprinthis\nprintln\n");
}

#[test]
fn m_slash_uses_the_same_direction_as_c_p_nearest_before_first() {
    let (text, blank_pos) = before_after_fixture();
    let (mut i, ed) = setup_evil(&text);
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "M-/");
    assert_eq!(bs(&mut i), "printhis\nprinthis\nprintln\n");
}

// ---------------------------------------------------------------------
// Rotation / cycling / wrap-around
// ---------------------------------------------------------------------

#[test]
fn repeated_c_n_rotates_through_candidates_then_wraps_to_original_then_repeats() {
    // Only "after" candidates, so C-n's order is exactly [println, printout].
    let (mut i, ed) = setup_evil("\nprintln\nprintout\n");
    feed(&mut i, &ed, "i"); // point-min, the blank first line
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n"); // -> println (1st candidate)
    assert_eq!(bs(&mut i), "println\nprintln\nprintout\n");
    feed(&mut i, &ed, "C-n"); // -> printout (2nd candidate)
    assert_eq!(bs(&mut i), "printout\nprintln\nprintout\n");
    feed(&mut i, &ed, "C-n"); // -> back to original "pri"
    assert_eq!(bs(&mut i), "pri\nprintln\nprintout\n");
    assert_eq!(echo(&ed), "(back to original)");
    feed(&mut i, &ed, "C-n"); // -> println again, cycle repeats
    assert_eq!(bs(&mut i), "println\nprintln\nprintout\n");
}

#[test]
fn duplicate_occurrences_of_the_same_word_count_as_one_candidate() {
    let l1 = "println\n";
    let l2 = "println\n";
    let blank_pos = (l1.len() + l2.len()) as i64 + 1; // the blank 3rd line
    let (mut i, ed) = setup_evil(&format!("{}{}\nprintln\n", l1, l2));
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n"); // only ONE distinct candidate: println
    assert_eq!(bs(&mut i), "println\nprintln\nprintln\nprintln\n");
    feed(&mut i, &ed, "C-n"); // straight to "back to original" -- no 2nd candidate
    assert_eq!(bs(&mut i), "println\nprintln\npri\nprintln\n");
    assert_eq!(echo(&ed), "(back to original)");
}

// ---------------------------------------------------------------------
// Session interruption
// ---------------------------------------------------------------------

fn interruption_fixture() -> (String, i64) {
    let text = "println\n\nfoo\n".to_string();
    let blank_pos = "println\n".len() as i64 + 1;
    (text, blank_pos)
}

#[test]
fn moving_point_away_and_back_starts_a_fresh_session() {
    let (text, blank_pos) = interruption_fixture();
    let (mut i, ed) = setup_evil(&text);
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n"); // expands to "println"
    assert_eq!(bs(&mut i), "println\nprintln\nfoo\n");
    run(&mut i, "(goto-char (point-min))"); // moved away out-of-band
    feed(&mut i, &ed, "C-n"); // must NOT continue the old session
    assert_eq!(
        echo(&ed),
        "No dynamic expansion possible here",
        "point-min has an empty prefix -- a continued session would have \
         rotated/wrapped instead of reporting this"
    );
    assert_eq!(
        bs(&mut i),
        "println\nprintln\nfoo\n",
        "buffer must be untouched"
    );
}

#[test]
fn typing_another_character_after_an_expansion_starts_a_fresh_session() {
    let (text, blank_pos) = interruption_fixture();
    let (mut i, ed) = setup_evil(&text);
    goto(&mut i, blank_pos);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n"); // -> "println"
    assert_eq!(bs(&mut i), "println\nprintln\nfoo\n");
    type_str(&mut i, &ed, "x"); // extends the text -- self-check must fail now
    feed(&mut i, &ed, "C-n");
    assert_eq!(
        echo(&ed),
        "No dynamic expansion for \"printlnx\" found",
        "the new prefix is \"printlnx\", proving a FRESH session started \
         rather than continuing the old \"pri\" one"
    );
    assert_eq!(bs(&mut i), "println\nprintlnx\nfoo\n");
}

// ---------------------------------------------------------------------
// No candidates
// ---------------------------------------------------------------------

#[test]
fn no_candidates_echoes_a_message_and_leaves_the_buffer_untouched() {
    // Trailing space so the typed "zzz" doesn't fuse with "world" into
    // one long prefix -- see `dabbrev--prefix-char-p'.
    let fixture = "hello world ";
    let (mut i, ed) = setup_evil(fixture);
    goto(&mut i, fixture.len() as i64 + 1);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "zzz");
    feed(&mut i, &ed, "C-n");
    assert_eq!(echo(&ed), "No dynamic expansion for \"zzz\" found");
    assert_eq!(bs(&mut i), "hello world zzz");
}

// ---------------------------------------------------------------------
// M-/ and evil state interaction
// ---------------------------------------------------------------------

#[test]
fn m_slash_falls_through_to_dabbrev_expand_in_evil_normal_state() {
    // Decision (see dabbrev.el's file header): `M-/' has no binding in
    // `evil--normal-map', and evil's emulation keymap is only ever
    // consulted AHEAD of local/global keymaps, never as a full
    // replacement -- so a plain `M-/' in NORMAL state falls through to
    // the SAME global `dabbrev-expand' binding it would hit with
    // evil-mode off. Accepted rather than shadowed: vim has no
    // standalone normal-state text-completion key for this to preempt,
    // and it matches upstream evil's own "let unclaimed keys fall
    // through" philosophy.
    let fixture = "printhis\npri";
    let (mut i, ed) = setup_evil(fixture);
    // Still in NORMAL state (setup_evil never enters insert).
    assert_eq!(run(&mut i, "evil--state"), "normal");
    goto(&mut i, fixture.len() as i64 + 1);
    feed(&mut i, &ed, "M-/");
    assert_eq!(
        bs(&mut i),
        "printhis\nprinthis",
        "M-/ expanded even though evil-mode was in normal state"
    );
}

#[test]
fn evil_mode_off_c_n_reverts_to_next_line_and_m_slash_still_works() {
    let (mut i, ed) = setup_evil("ab\ncd");
    run(&mut i, "(evil-mode -1)");
    // With evil-mode off, `emulation-keymap' is nil everywhere, so C-n
    // falls straight to simple.el's global binding: plain `next-line'.
    feed(&mut i, &ed, "C-n");
    assert_eq!(
        pt(&mut i),
        4,
        "C-n moved to the next line, column 0 -> position of 'c'"
    );
    assert_eq!(
        bs(&mut i),
        "ab\ncd",
        "buffer untouched -- this was a motion, not a completion"
    );

    // M-/ is unaffected by evil-mode either way -- it was always a
    // plain global binding.
    let fixture = "printhis\npri";
    let (mut i2, ed2) = setup_evil(fixture);
    run(&mut i2, "(evil-mode -1)");
    goto(&mut i2, fixture.len() as i64 + 1);
    feed(&mut i2, &ed2, "M-/");
    assert_eq!(bs(&mut i2), "printhis\nprinthis");
}

// ---------------------------------------------------------------------
// Dot-repeat integration
// ---------------------------------------------------------------------

#[test]
fn dot_repeat_replays_the_completed_text_not_just_the_typed_prefix() {
    let (mut i, ed) = setup_evil("xxx yyy\nprintln");
    // Change the first word ("xxx") via ciw, type "pri", expand with
    // C-n to "println", then leave insert state.
    feed(&mut i, &ed, "c i w");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "println yyy\nprintln");
    feed(&mut i, &ed, "ESC");
    // Move onto the second word ("yyy") and replay.
    goto(&mut i, "println ".len() as i64 + 1); // the 'y' of "yyy"
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "println println\nprintln",
        "dot-repeat replayed the FINAL text (post-completion), not just \"pri\""
    );
}

// ---------------------------------------------------------------------
// Undo (pinned, current behavior -- see the comment for the mechanism)
// ---------------------------------------------------------------------

#[test]
fn undo_after_an_expansion_reverts_just_the_expansion_first() {
    // `execute_command' (commands.rs) pushes a fresh undo boundary
    // before EVERY dispatched command, including `evil-complete-next'
    // itself -- so the delete-region+insert a completion performs land
    // in their OWN undo group, separate from the self-inserted "pri"
    // that preceded them (even though THAT group is amalgamated with
    // the preceding `ciw''s delete, per evil.el's `undo-amalgamate-
    // boundary' mechanism -- see `cw_then_typing_then_esc_then_a_single_
    // undo_reverts_both' in evil_tests.rs for that half). One `u' right
    // after a completion therefore undoes ONLY the completion, landing
    // back on the typed prefix; a second `u' then reverts the `ciw'
    // deletion + prefix typing as the usual single group.
    let (mut i, ed) = setup_evil("xxx\nprintln");
    feed(&mut i, &ed, "c i w");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "println\nprintln");
    feed(&mut i, &ed, "ESC");

    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "pri\nprintln",
        "first undo: just the completion"
    );

    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "xxx\nprintln",
        "second undo: the ciw + typed prefix group"
    );
}

// ---------------------------------------------------------------------
// Review fixes: false session continuation (point+text alone lied)
// ---------------------------------------------------------------------

#[test]
fn typing_one_more_character_then_esc_does_not_falsely_continue_the_session() {
    // Reproduces the reviewer-found bug: C-n expands "pri" to "println",
    // then ONE more (arbitrary) character is typed, then ESC -- whose
    // own `backward-char' silently realigns point back to exactly
    // LAST-POINT (where the completion left it), and a plain
    // point-equal + text-equal check cannot tell that apart from
    // "nothing happened since": `(buffer-substring PREFIX-START
    // LAST-POINT)' never even looks at the newly typed character, which
    // sits exactly AT LAST-POINT, one past the checked range. The buggy
    // behavior this used to cause: a following M-/ "continues" the
    // stale session, rotates to "back to original", and
    // `delete-region's only the checked span ("println"), splicing the
    // untouched extra character right back onto "pri" -- silently
    // producing "priz" while every checked condition looked consistent.
    // Fixed by `buffer-modified-tick' (any edit bumps it, this one
    // included) -- see dabbrev.el's "Session / cycling" section.
    let (mut i, ed) = setup_evil("xxx\nprintln");
    feed(&mut i, &ed, "c i w");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "println\nprintln");
    type_str(&mut i, &ed, "z"); // buffer: "printlnz\nprintln", point past 'z'
    feed(&mut i, &ed, "ESC"); // backward-char realigns point to right before 'z'
    assert_eq!(
        bs(&mut i),
        "printlnz\nprintln",
        "ESC itself never mutates the buffer"
    );

    feed(&mut i, &ed, "M-/");
    assert_eq!(
        bs(&mut i),
        "printlnz\nprintln",
        "must NOT have rewritten \"println\" back to \"pri\", stranding \"z\" as \"priz\""
    );
    assert_eq!(
        echo(&ed),
        "No dynamic expansion for \"println\" found",
        "a genuinely fresh session started against the NEW prefix \"println\" \
         (point sits right after it, before the 'z') and found no other match"
    );
}

#[test]
fn switching_buffers_does_not_falsely_continue_the_session() {
    // The cross-buffer twin of the bug above: the session had no buffer
    // identity of its own, so switching to a DIFFERENT buffer that
    // happens to coincide on (PREFIX-START, LAST-POINT, LAST-TEXT) used
    // to "continue" -- rotating with the OLD buffer's candidate list
    // and mutating the NEW buffer via stale positions. Fixed by
    // recording `(current-buffer)' in the session and `eq'-checking it.
    let (mut i, ed) = setup_evil("xxx\nprintln");
    feed(&mut i, &ed, "c i w");
    type_str(&mut i, &ed, "pri");
    feed(&mut i, &ed, "C-n");
    assert_eq!(bs(&mut i), "println\nprintln");
    assert_eq!(pt(&mut i), 8, "sanity: LAST-POINT is 8");
    feed(&mut i, &ed, "ESC");

    // A second buffer whose content/point coincidentally matches the
    // stale session's recorded (PREFIX-START=1, LAST-POINT=8,
    // LAST-TEXT="println").
    run(&mut i, "(switch-to-buffer \"other\")");
    run(&mut i, "(insert \"println\")"); // 7 chars -- point lands at 8, same as LAST-POINT
    assert_eq!(
        pt(&mut i),
        8,
        "sanity: coincidentally the same point as buffer A's session"
    );

    feed(&mut i, &ed, "M-/");
    assert_eq!(
        bs(&mut i),
        "println",
        "must NOT have treated buffer A's stale session as still live in THIS buffer"
    );
    assert_eq!(echo(&ed), "No dynamic expansion for \"println\" found");
}

// ---------------------------------------------------------------------
// Review fixes: `\b' disagreeing with the dabbrev prefix character class
// ---------------------------------------------------------------------

#[test]
fn hyphen_prefixed_candidate_is_found() {
    // Bug: a plain regex `\b' does not treat a space/newline-to-`-'
    // transition as a boundary (`-' isn't a "word" character to the
    // regex engine either), so a prefix that itself STARTS with `-'
    // used to never find real candidates at all. Fixed by dropping `\b'
    // from the pattern and filtering matches with
    // `dabbrev--at-word-start-p' instead, which uses this file's OWN
    // prefix character class (`-' included).
    let fixture = "-foobar\n";
    let (mut i, ed) = setup_evil(fixture);
    goto(&mut i, fixture.len() as i64 + 1); // the blank second line
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "-foo");
    feed(&mut i, &ed, "C-p"); // nearest before
    assert_eq!(bs(&mut i), "-foobar\n-foobar");
}

#[test]
fn does_not_offer_a_fake_candidate_carved_out_of_a_larger_hyphenated_identifier() {
    // Bug, the more insidious direction (this codebase's own source is
    // full of `xxx--yyy' names, so this is easy to hit by accident):
    // plain `\b' DOES consider the spot right after a `-' a boundary
    // (`-' is "non-word", the letter after it is), so completing "sess"
    // used to offer "session" carved out of the MIDDLE of
    // "dabbrev--session" -- wrong, because this file's own prefix class
    // treats `-' as part of an identifier, so "dabbrev--session" is ONE
    // token whose real start is "d", not "s". Expected outcome, derived
    // from that same character class: NO candidate at all -- "sess" is
    // not a prefix of the whole token "dabbrev--session" either, so
    // there is nothing correct to offer, not even the whole token.
    let fixture = "dabbrev--session\n";
    let (mut i, ed) = setup_evil(fixture);
    goto(&mut i, fixture.len() as i64 + 1);
    feed(&mut i, &ed, "i");
    type_str(&mut i, &ed, "sess");
    feed(&mut i, &ed, "C-n");
    assert_eq!(echo(&ed), "No dynamic expansion for \"sess\" found");
    assert_eq!(
        bs(&mut i),
        "dabbrev--session\nsess",
        "buffer must be untouched"
    );
}

// ---------------------------------------------------------------------
// Review fix: emacs-state buffer (evil-mode ON globally, THIS buffer's
// own state is 'emacs) -- a different code path from evil-mode being
// globally off.
// ---------------------------------------------------------------------

#[test]
fn emacs_state_buffer_c_n_is_next_line_not_completion() {
    // `evil_mode_off_c_n_reverts_to_next_line_and_m_slash_still_works'
    // above exercises the GLOBAL evil-mode toggle (`emulation-keymap'
    // nil for every buffer). This is a different mechanism entirely:
    // evil-mode stays ON globally, but a dired buffer's OWN
    // `evil--state' is `emacs' (see `evil-emacs-state-modes'), which
    // makes `emulation-keymap' nil for just THIS buffer -- C-n must
    // fall through to plain `next-line' here too.
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
    let before_line = run(&mut i, "(line-number-at-pos)");
    feed(&mut i, &ed, "C-n");
    let after_line = run(&mut i, "(line-number-at-pos)");
    assert_eq!(
        bs(&mut i),
        before_text,
        "C-n must not attempt a completion in an emacs-state buffer"
    );
    assert_ne!(
        after_line, before_line,
        "C-n moved to the next line (plain next-line) -- emulation-keymap was nil for THIS buffer"
    );
}
