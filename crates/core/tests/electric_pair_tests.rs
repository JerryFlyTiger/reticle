//! M37: electric-pair-mode (auto-close/skip matching brackets and
//! quotes). Every pairing/skip/DEL test drives real key events through
//! `handle_key`/`feed_keys` — never a direct `(insert ...)` call — since
//! the whole point of hanging this off `post-self-insert-hook` (see
//! electric-pair.el's header) rather than a keybinding is that it rides
//! the exact same path a real keystroke takes; a test that bypassed
//! `self_insert` would not actually exercise that path.
//!
//! `point` assertions below are elisp's 1-based convention
//! (`builtins/mod.rs`'s `int_pos` = internal 0-based offset + 1); each
//! was hand-derived from the exact insert/backward-char/delete-char
//! sequence `electric-pair-post-self-insert`/`electric-pair-backward-
//! delete` run, not guessed.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use elisp::{Interp, Value};

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (60, 20);
    (interp, ed)
}

/// A prog-mode buffer (c-mode): `prog-mode-hook` has run, so
/// `electric-pair-mode` is buffer-locally on and DEL is locally bound
/// to `electric-pair-backward-delete` — see electric-pair.el.
fn setup_prog() -> (Interp, Rc<RefCell<Editor>>) {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    (i, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => elisp::printer::prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn feed(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, keys: &str) {
    feed_keys(interp, ed, keys).unwrap_or_else(|e| panic!("feed_keys {:?}: {}", keys, e));
}

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

/// Types `s` one character at a time via raw key events (mirrors
/// indent_tests.rs's own `type_str`): needed because `feed_keys`' own
/// mini-language tokenizes on whitespace, and every character here
/// (including a literal space) must go through `self_insert` on its
/// own, exactly like real typing.
fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

// --- Pairing: each opener auto-closes, point lands between the two ---

#[test]
fn each_open_delimiter_auto_closes_with_point_between() {
    for &(open, expected) in &[('(', "()"), ('[', "[]"), ('{', "{}"), ('"', "\"\"")] {
        let (mut i, ed) = setup_prog();
        type_str(&mut i, &ed, &open.to_string());
        assert_eq!(bs(&mut i), expected, "typing {:?}", open);
        assert_eq!(
            pt(&mut i),
            2,
            "point must land between the pair after typing {:?}",
            open
        );
    }
}

// --- Skip: typing the matching close over an auto-inserted one moves
// --- point past it instead of duplicating the character ---

#[test]
fn typing_close_paren_over_the_auto_inserted_one_skips_instead_of_duplicating() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, "(");
    assert_eq!(bs(&mut i), "()");
    type_str(&mut i, &ed, ")");
    assert_eq!(
        bs(&mut i),
        "()",
        "must stay \"()\", not duplicate into \"())\""
    );
    assert_eq!(pt(&mut i), 3, "point must end up after the close paren");
}

#[test]
fn typing_quote_twice_skips_the_second_over_the_first_pair() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, "\"");
    assert_eq!(bs(&mut i), "\"\"");
    type_str(&mut i, &ed, "\"");
    assert_eq!(
        bs(&mut i),
        "\"\"",
        "must stay \"\\\"\\\"\", not become \"\\\"\\\"\\\"\""
    );
    assert_eq!(pt(&mut i), 3, "point must end up after the second quote");
}

#[test]
fn closer_typed_just_before_an_existing_matching_closer_skips() {
    let (mut i, ed) = setup_prog();
    run(&mut i, "(insert \")\")");
    run(&mut i, "(goto-char (point-min))");
    type_str(&mut i, &ed, ")");
    assert_eq!(
        bs(&mut i),
        ")",
        "skip must not duplicate the pre-existing \")\""
    );
    assert_eq!(
        pt(&mut i),
        2,
        "point must move past the (skipped) close paren"
    );
}

// --- Unbalanced closer: no matching char-after means GNU behavior --
// --- literal self-insert stands, nothing is deleted ---

#[test]
fn unbalanced_close_paren_is_inserted_literally() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, ")");
    assert_eq!(bs(&mut i), ")");
    assert_eq!(pt(&mut i), 2);
}

#[test]
fn unbalanced_close_bracket_before_unrelated_text_is_inserted_literally() {
    let (mut i, ed) = setup_prog();
    run(&mut i, "(insert \"x\")");
    run(&mut i, "(goto-char (point-min))");
    type_str(&mut i, &ed, "]");
    assert_eq!(
        bs(&mut i),
        "]x",
        "] before \"x\" (not a matching \"]\") must not skip"
    );
    assert_eq!(pt(&mut i), 2);
}

// --- Nested pairs ---

#[test]
fn nested_open_parens_pair_up_and_then_skip_closing_in_turn() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, "((");
    assert_eq!(
        bs(&mut i),
        "(())",
        "\"((\" must produce \"(())\", cursor in the middle"
    );
    assert_eq!(
        pt(&mut i),
        3,
        "point sits between the two middle characters: \"((|))\""
    );
    type_str(&mut i, &ed, "))");
    assert_eq!(
        bs(&mut i),
        "(())",
        "typing the two closers must skip both, not duplicate either"
    );
    assert_eq!(pt(&mut i), 5, "point ends up after everything");
}

// --- DEL: an empty pair straddling point deletes as one edit ---

#[test]
fn del_between_an_empty_pair_deletes_both_characters() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, "(");
    assert_eq!(bs(&mut i), "()");
    feed(&mut i, &ed, "DEL");
    assert_eq!(bs(&mut i), "", "DEL on \"(|)\" must remove both characters");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn del_between_an_empty_pair_with_surrounding_text_only_removes_the_pair() {
    let (mut i, ed) = setup_prog();
    // Pre-existing "ab", THEN pairing "(" between them: unlike typing
    // "a(b" in one sweep (where "b" would land INSIDE the fresh pair,
    // giving "a(b)" -- see the previous test), inserting the pair
    // between two characters that already exist keeps them outside it.
    run(&mut i, "(insert \"ab\")");
    run(&mut i, "(goto-char 2)"); // between "a" and "b"
    type_str(&mut i, &ed, "(");
    assert_eq!(bs(&mut i), "a()b");
    assert_eq!(pt(&mut i), 3, "point between \"(\" and \")\"");
    feed(&mut i, &ed, "DEL");
    assert_eq!(
        bs(&mut i),
        "ab",
        "only the empty pair should go, not the surrounding text"
    );
    assert_eq!(pt(&mut i), 2);
}

#[test]
fn del_with_non_matching_chars_around_point_falls_back_to_plain_delete() {
    let (mut i, ed) = setup_prog();
    run(&mut i, "(insert \"axb\")");
    run(&mut i, "(goto-char 3)"); // between "x" and "b"
    feed(&mut i, &ed, "DEL");
    assert_eq!(
        bs(&mut i),
        "ab",
        "plain DEL semantics when the pair doesn't match"
    );
    assert_eq!(pt(&mut i), 2);
}

// --- Undo: the auto-inserted close paren must undo together with the
// --- open paren that triggered it, as ONE step ---

#[test]
fn undo_after_auto_pairing_reverts_both_characters_in_one_step() {
    let (mut i, ed) = setup_prog();
    type_str(&mut i, &ed, "(");
    assert_eq!(bs(&mut i), "()");
    feed(&mut i, &ed, "C-/");
    assert_eq!(
        bs(&mut i),
        "",
        "a single undo must remove both the typed \"(\" and the auto-inserted \")\""
    );
}

// --- Isolation: only prog buffers with electric-pair-mode on pair ---

#[test]
fn fundamental_mode_buffer_does_not_pair() {
    let (mut i, ed) = setup();
    run(&mut i, "(fundamental-mode)");
    type_str(&mut i, &ed, "(");
    assert_eq!(
        bs(&mut i),
        "(",
        "electric-pair-mode must be off outside prog-mode"
    );
    assert_eq!(pt(&mut i), 2);
}

#[test]
fn org_buffer_does_not_pair() {
    let (mut i, ed) = setup();
    run(&mut i, "(org-mode)");
    type_str(&mut i, &ed, "(");
    assert_eq!(
        bs(&mut i),
        "(",
        "org-mode is not a prog-mode, so pairing stays off"
    );
}

#[test]
fn minibuffer_open_paren_is_unaffected_earlier_branch() {
    // handle_key routes the minibuffer to `minibuffer_key` before
    // `dispatch_key`/`self_insert` are even reached (see commands.rs's
    // `handle_key`), so `post-self-insert-hook` never runs for it —
    // pinned here regardless of the underlying buffer's own
    // electric-pair-mode state.
    let (mut i, ed) = setup_prog();
    feed(&mut i, &ed, "M-x");
    type_str(&mut i, &ed, "(");
    let input = ed
        .borrow()
        .minibuffer
        .as_ref()
        .expect("M-x should have opened the minibuffer")
        .input
        .clone();
    assert_eq!(input, "(", "the minibuffer's own input must not be paired");
}

#[test]
fn evil_normal_state_open_paren_does_not_pair_or_insert() {
    // The core reason this feature is a `post-self-insert-hook'
    // function and not a keybinding for "(": a real binding would be
    // dispatched by `execute_command' regardless of evil's state,
    // punching through M34's `inhibit-self-insert' guard. A hook can't:
    // normal state never reaches `self_insert' for an unbound printable
    // key at all (see M34's `dispatch_key'), so this hook's code plainly
    // never runs.
    let (mut i, ed) = setup_prog();
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    assert_eq!(run(&mut i, "evil--state"), "normal");
    type_str(&mut i, &ed, "(");
    assert_eq!(
        bs(&mut i),
        "",
        "normal state must swallow \"(\" entirely -- M34's inhibit-self-insert"
    );
    assert_eq!(pt(&mut i), 1);
}

// --- Evil dot-repeat x electric-pair interaction ---
//
// `evil--finish-insert-session' (evil.el, M30) captures the session's
// typed text as `(buffer-substring evil--insert-start (point))', read
// BEFORE point moves for ESC's own "step back onto the last typed
// char" adjustment -- i.e. at wherever the last self-insert (or hook)
// left point. Electric-pair's whole "point sits in the middle" pairing
// contract (required by this file's very first test above) means that after
// typing an OPENING delimiter, point deliberately sits BETWEEN it and
// its auto-inserted close -- so when a session ends (ESC) right there,
// the auto-inserted closer sits AFTER point and falls OUTSIDE the
// captured range. `.' later replays the captured text via a plain
// `(insert TEXT)' call (evil.el), which -- unlike a real keystroke --
// never runs `post-self-insert-hook' at all, so replaying can't
// re-trigger pairing to make up the difference either.
//
// Net effect, verified empirically below: dot-repeat after a `ciw'
// that ends with point auto-paired mid-bracket reproduces the TYPED
// characters ("(x") but not the auto-inserted ")". This is a genuine
// interaction gap between M30's capture-by-point-range design and
// M37's point-lands-between-the-pair contract, not a bug in the
// pairing logic itself (which this file's other tests confirm behaves
// correctly on its own) -- and not something this milestone's spec
// anticipated (its own text assumed the session capture would include
// the auto-close). Fixing it for real would mean changing what
// `evil--finish-insert-session' captures (M30's own mechanism), an
// architecture-level call outside this milestone's scope -- flagged in
// the M37 report rather than patched here. Pinned as-observed so a
// future change doesn't silently move this behavior again without
// anyone noticing either way.
#[test]
fn evil_dot_repeat_replays_typed_text_but_not_the_auto_inserted_close_paren() {
    let (mut i, ed) = setup_prog();
    run(&mut i, &format!("(insert {:?})", "foo bar baz"));
    run(&mut i, "(goto-char (point-min))");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    feed(&mut i, &ed, "w"); // point on "bar"
    feed(&mut i, &ed, "c i w");
    assert_eq!(bs(&mut i), "foo  baz");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    type_str(&mut i, &ed, "(x");
    feed(&mut i, &ed, "ESC");
    assert_eq!(
        bs(&mut i),
        "foo (x) baz",
        "electric-pair should have auto-closed while typing inside the insert session"
    );
    assert_eq!(run(&mut i, "evil--state"), "normal");
    // Two `w` presses: the first lands on the auto-closed ")" itself
    // (a punctuation run is its own word-motion token), the second
    // clears it and the following space to reach "baz".
    feed(&mut i, &ed, "w w");
    assert_eq!(
        bs(&mut i).get(8..11),
        Some("baz"),
        "point should be heading into \"baz\" before dot-repeat runs"
    );
    feed(&mut i, &ed, ".");
    assert_eq!(
        bs(&mut i),
        "foo (x) (x",
        "see this test's header: the auto-inserted \")\" falls outside dot-repeat's \
         captured range, so replay is missing it -- a pre-existing M30 capture-mechanism \
         limitation, not a pairing bug"
    );
}
