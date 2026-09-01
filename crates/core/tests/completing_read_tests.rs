//! M47: `read-from-minibuffer`/`read-string`/`completing-read`/
//! `y-or-n-p` and the `with-read-string`/`with-completing-read`/
//! `with-y-or-n` macro sugar over them. Style follows
//! `complete_tests.rs`/`panel_tests.rs`: `setup()` builds an
//! `(Interp, Rc<RefCell<Editor>>)`, `run()` evaluates elisp and prints
//! the result, `type_str`/`feed_keys` drive the key loop, and small
//! per-file helpers read back minibuffer/panel state.

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

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

fn mb_open(ed: &Rc<RefCell<Editor>>) -> bool {
    ed.borrow().minibuffer.is_some()
}

fn mb_input(ed: &Rc<RefCell<Editor>>) -> String {
    ed.borrow()
        .minibuffer
        .as_ref()
        .map(|m| m.input.clone())
        .unwrap_or_default()
}

fn mb_note(ed: &Rc<RefCell<Editor>>) -> Option<String> {
    ed.borrow().minibuffer.as_ref().and_then(|m| m.note.clone())
}

fn panel_accepts(ed: &Rc<RefCell<Editor>>) -> Vec<String> {
    ed.borrow()
        .minibuffer
        .as_ref()
        .and_then(|m| m.panel.as_ref())
        .map(|p| p.rows.iter().map(|r| r.accept.clone()).collect())
        .unwrap_or_default()
}

fn capturing(ed: &Rc<RefCell<Editor>>) -> bool {
    ed.borrow().key_capture.is_some()
}

// M70: echo row helpers (mirrors `completion_popup_tests.rs`'s
// `row_text`/`cursor_row`) -- the echo row is always the last grid row,
// i.e. `frame.1 - 1`.
fn echo_row_text(interp: &Interp, ed: &Rc<RefCell<Editor>>) -> String {
    let row = ed.borrow().frame.1 - 1;
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn echo_cursor_col(interp: &Interp, ed: &Rc<RefCell<Editor>>) -> usize {
    core::redisplay::render(interp, ed).cursor.1
}

// 1. `read-string`'s callback receives the typed string.
#[test]
fn read_string_callback_gets_typed_input() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (read-string \"Name: \" (lambda (s) (setq result s)))",
    );
    assert!(mb_open(&ed));
    type_str(&mut i, &ed, "hello");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(!mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "\"hello\"");
}

// 2. INITIAL prefills, cursor at the end; typing appends.
#[test]
fn initial_prefills_with_cursor_at_end() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (read-string \"Name: \" (lambda (s) (setq result s)) \"abc\")",
    );
    assert_eq!(mb_input(&ed), "abc");
    type_str(&mut i, &ed, "def");
    assert_eq!(mb_input(&ed), "abcdef");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "result"), "\"abcdef\"");
}

// 3. `completing-read` opens the panel and lists candidates; RET
// accepts the selected (first, by default) row.
#[test]
fn completing_read_opens_panel_and_ret_accepts_row() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(\"alpha\" \"beta\" \"gamma\") (lambda (s) (setq result s)))",
    );
    assert_eq!(panel_accepts(&ed), vec!["alpha", "beta", "gamma"]);
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "result"), "\"alpha\"");
}

// 4. Live filtering: every keystroke narrows the panel rows.
#[test]
fn typing_filters_panel_rows_live() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(completing-read \"Pick: \" '(\"apple\" \"apricot\" \"banana\") (lambda (s) (setq result s)))",
    );
    assert_eq!(panel_accepts(&ed), vec!["apple", "apricot", "banana"]);
    type_str(&mut i, &ed, "ap");
    assert_eq!(panel_accepts(&ed), vec!["apple", "apricot"]);
    type_str(&mut i, &ed, "p");
    assert_eq!(panel_accepts(&ed), vec!["apple"]);
}

// 5. Prefix matches sort before substring matches, and within each
// group the collection's own order is kept (no alphabetizing).
#[test]
fn prefix_matches_rank_before_substring_matches() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(completing-read \"Pick: \" '(\"scatter\" \"cat\" \"dog\") (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "cat");
    // "cat" itself is a prefix match; "scatter" only a substring match
    // (contains "cat" but doesn't start with it); "dog" doesn't match
    // at all. Prefix group first, substring group after, each in the
    // collection's own order.
    assert_eq!(panel_accepts(&ed), vec!["cat", "scatter"]);
}

// 5b. The "keep collection order, don't alphabetize" claim from test 5
// needs a collection that isn't already alphabetical to be a real
// check -- otherwise a stray `.sort()` would go undetected. No typing
// happens here, so every candidate is a prefix match of "" and the
// whole group must come back exactly as given.
#[test]
fn collection_order_preserved_when_not_alphabetical() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(completing-read \"Pick: \" '(\"zebra\" \"apple\" \"mango\") (lambda (s) (setq result s)))",
    );
    assert_eq!(panel_accepts(&ed), vec!["zebra", "apple", "mango"]);
}

// 6. TAB expands to the longest common prefix across the filtered set.
#[test]
fn tab_expands_longest_common_prefix() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(completing-read \"Pick: \" '(\"prefix-alpha\" \"prefix-beta\") (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "pre");
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(mb_input(&ed), "prefix-");
}

// 7. REQUIRE-MATCH blocks RET on a non-matching input: minibuffer stays
// open, note shows, callback is never invoked.
#[test]
fn require_match_blocks_ret_on_mismatch() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (completing-read \"Pick: \" '(\"alpha\" \"beta\") (lambda (s) (setq result s)) t)",
    );
    type_str(&mut i, &ed, "zzz");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(mb_open(&ed));
    assert_eq!(mb_note(&ed).as_deref(), Some(" [No match]"));
    assert_eq!(run(&mut i, "result"), "unset");
}

// 8. Same gate, but via C-j — the panel-bypass escape hatch must not
// smuggle a non-matching literal past REQUIRE-MATCH.
#[test]
fn require_match_blocks_c_j_on_mismatch() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (completing-read \"Pick: \" '(\"alpha\" \"beta\") (lambda (s) (setq result s)) t)",
    );
    type_str(&mut i, &ed, "zzz");
    feed_keys(&mut i, &ed, "C-j").unwrap();
    assert!(mb_open(&ed));
    assert_eq!(mb_note(&ed).as_deref(), Some(" [No match]"));
    assert_eq!(run(&mut i, "result"), "unset");
}

// 9. REQUIRE-MATCH nil: any string submits.
#[test]
fn no_require_match_accepts_anything() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(\"alpha\" \"beta\") (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "not-in-list");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(!mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "\"not-in-list\"");
}

// 10. ESC and C-g both cancel without invoking the callback.
#[test]
fn esc_and_c_g_cancel_without_callback() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (read-string \"Name: \" (lambda (s) (setq result s)))",
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert!(!mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "unset");

    run(
        &mut i,
        "(setq result 'unset) (read-string \"Name: \" (lambda (s) (setq result s)))",
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert!(!mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "unset");
}

// 11. Nested reads: a callback opening a second `completing-read` works
// normally, and the second submission's callback fires too.
#[test]
fn nested_completing_read_from_within_a_callback() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq outer nil) (setq inner nil)
         (completing-read \"Outer: \" '(\"a\" \"b\") (lambda (x)
           (setq outer x)
           (completing-read \"Inner: \" '(\"c\" \"d\") (lambda (y) (setq inner y)))))",
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(mb_open(&ed));
    assert_eq!(panel_accepts(&ed), vec!["c", "d"]);
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(!mb_open(&ed));
    assert_eq!(run(&mut i, "outer"), "\"a\"");
    assert_eq!(run(&mut i, "inner"), "\"c\"");
}

// 12. Calling `completing-read` while a minibuffer is already open
// signals an error and leaves the original prompt/pending untouched;
// submitting afterward still runs the first callback.
#[test]
fn nested_call_outside_a_callback_signals_and_leaves_original_intact() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq r nil) (read-string \"First: \" (lambda (s) (setq r s)))",
    );
    assert!(mb_open(&ed));
    let err = run(
        &mut i,
        "(setq r2 nil) (completing-read \"Second: \" '(\"x\") (lambda (s) (setq r2 s)))",
    );
    assert!(err.starts_with("ERROR"), "expected an error, got {err}");
    assert!(mb_open(&ed));
    assert_eq!(ed.borrow().minibuffer.as_ref().unwrap().prompt, "First: ");
    type_str(&mut i, &ed, "hi");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "r"), "\"hi\"");
    assert_eq!(run(&mut i, "r2"), "nil");
}

// 12b. `read-string`/`read-from-minibuffer` have their own copy of the
// reentrancy guard (they don't share code with `completing-read`'s
// check) -- exercise it directly so a guard dropped from just this
// path wouldn't go unnoticed.
#[test]
fn read_string_reentrancy_guard_signals_and_leaves_original_intact() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq r nil) (read-string \"First: \" (lambda (s) (setq r s)))",
    );
    assert!(mb_open(&ed));
    let err = run(
        &mut i,
        "(setq r2 nil) (read-string \"Second: \" (lambda (s) (setq r2 s)))",
    );
    assert!(err.starts_with("ERROR"), "expected an error, got {err}");
    assert!(mb_open(&ed));
    assert_eq!(ed.borrow().minibuffer.as_ref().unwrap().prompt, "First: ");
    type_str(&mut i, &ed, "hi");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "r"), "\"hi\"");
    assert_eq!(run(&mut i, "r2"), "nil");
}

// 13. An empty collection with REQUIRE-MATCH set can never be
// submitted, by either RET or C-j.
#[test]
fn empty_collection_with_require_match_never_submits() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (completing-read \"Pick: \" nil (lambda (s) (setq result s)) t)",
    );
    type_str(&mut i, &ed, "anything");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "unset");
    feed_keys(&mut i, &ed, "C-j").unwrap();
    assert!(mb_open(&ed));
    assert_eq!(run(&mut i, "result"), "unset");
}

// 14. `y-or-n-p`: y -> t, n -> nil, other keys are ignored (still
// capturing) until the user actually answers.
#[test]
fn y_or_n_p_answers_and_ignores_other_keys() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (y-or-n-p \"Proceed? \" (lambda (v) (setq result v)))",
    );
    assert!(capturing(&ed));
    handle_key(&mut i, &ed, Key::Char('x' as i64));
    assert!(capturing(&ed));
    assert_eq!(run(&mut i, "result"), "unset");
    handle_key(&mut i, &ed, Key::Char('y' as i64));
    assert!(!capturing(&ed));
    assert_eq!(run(&mut i, "result"), "t");

    run(
        &mut i,
        "(setq result 'unset) (y-or-n-p \"Proceed? \" (lambda (v) (setq result v)))",
    );
    handle_key(&mut i, &ed, Key::Char('n' as i64));
    assert_eq!(run(&mut i, "result"), "nil");
}

// 15. The three macros expand usably, and their lambdas close over the
// lexical environment they were written in.
#[test]
fn macros_work_and_close_over_lexical_scope() {
    let (mut i, ed) = setup();

    run(
        &mut i,
        "(setq log nil)
         (let ((tag 'from-read-string))
           (with-read-string (s \"Name: \")
             (setq log (list tag s))))",
    );
    type_str(&mut i, &ed, "bob");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "log"), "(from-read-string \"bob\")");

    run(
        &mut i,
        "(setq log nil)
         (let ((tag 'from-completing-read))
           (with-completing-read (s \"Pick: \" '(\"x\" \"y\") t)
             (setq log (list tag s))))",
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "log"), "(from-completing-read \"x\")");

    run(
        &mut i,
        "(setq log nil)
         (let ((tag 'from-y-or-n))
           (with-y-or-n (v \"OK? \")
             (setq log (list tag v))))",
    );
    handle_key(&mut i, &ed, Key::Char('y' as i64));
    assert_eq!(run(&mut i, "log"), "(from-y-or-n t)");
}

// 16 (Part D). M-p recalls history, M-n walks back to the live-edge
// stash, and different HISTORY-KEYs don't share a ring.
#[test]
fn history_m_p_m_n_and_key_isolation() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    type_str(&mut i, &ed, "first");
    feed_keys(&mut i, &ed, "RET").unwrap();

    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    type_str(&mut i, &ed, "second");
    feed_keys(&mut i, &ed, "RET").unwrap();

    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    type_str(&mut i, &ed, "partial");
    feed_keys(&mut i, &ed, "M-p").unwrap();
    assert_eq!(mb_input(&ed), "second");
    feed_keys(&mut i, &ed, "M-p").unwrap();
    assert_eq!(mb_input(&ed), "first");
    feed_keys(&mut i, &ed, "M-n").unwrap();
    assert_eq!(mb_input(&ed), "second");
    feed_keys(&mut i, &ed, "M-n").unwrap();
    assert_eq!(mb_input(&ed), "partial");
    feed_keys(&mut i, &ed, "RET").unwrap();

    // A different HISTORY-KEY has its own, independent ring.
    run(
        &mut i,
        "(setq r nil) (read-string \"B: \" (lambda (s) (setq r s)) nil \"keyB\")",
    );
    type_str(&mut i, &ed, "b-value");
    feed_keys(&mut i, &ed, "RET").unwrap();

    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)) nil \"keyA\")",
    );
    feed_keys(&mut i, &ed, "M-p").unwrap();
    // "keyA" has never been submitted before, so M-p has nothing to
    // recall -- input stays empty, unaffected by "keyB"'s ring.
    assert_eq!(mb_input(&ed), "");
    feed_keys(&mut i, &ed, "RET").unwrap();
}

// 17. Fix round: RET on a `Source::Custom` panel (completing-read)
// submits the user's own exact-match input over the highlighted row.
// `custom_filter` deliberately does NOT sort (see its doc comment), so
// unlike `Source::File`/`Source::Buffer` an exact match is not
// guaranteed to land on row 0 -- collection ("alphabet" "alpha") with
// "alphabet" listed first means typing "alpha" and hitting RET must
// still submit "alpha", not the highlighted "alphabet".
#[test]
fn literal_exact_match_wins_over_highlighted_row_for_custom_source() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(\"alphabet\" \"alpha\") (lambda (s) (setq result s)))",
    );
    assert_eq!(panel_accepts(&ed), vec!["alphabet", "alpha"]);
    type_str(&mut i, &ed, "alpha");
    // Untouched selection still highlights row 0 ("alphabet").
    assert_eq!(
        ed.borrow()
            .minibuffer
            .as_ref()
            .unwrap()
            .panel
            .as_ref()
            .unwrap()
            .selected,
        0
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "result"), "\"alpha\"");
}

// 17b. Same setup, but the user moved the selection (e.g. an arrow key)
// before RET -- the highlighted row must still win even though the
// typed input happens to exactly match a different candidate. This
// guards the fix in test 17 from overreaching: `chosen` must gate it.
#[test]
fn arrow_key_selection_wins_over_exact_literal_match() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(\"alphabet\" \"alpha\") (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "alpha");
    // Move the highlight down to "alpha" and back up to "alphabet" so
    // the selection is explicitly touched (`chosen = true`) while
    // still resting on row 0.
    feed_keys(&mut i, &ed, "down").unwrap();
    feed_keys(&mut i, &ed, "up").unwrap();
    assert_eq!(
        ed.borrow()
            .minibuffer
            .as_ref()
            .unwrap()
            .panel
            .as_ref()
            .unwrap()
            .selected,
        0
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    // Selection was touched, so the highlighted row ("alphabet") wins
    // even though "alpha" is an exact match too.
    assert_eq!(run(&mut i, "result"), "\"alphabet\"");
}

// 17c. Non-exact input on a `Source::Custom` panel still takes the
// highlighted row 0, unaffected by the fix.
#[test]
fn non_exact_custom_input_still_takes_highlighted_row() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(\"alphabet\" \"alpha\") (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "alph");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "result"), "\"alphabet\"");
}

// 18. Fix round: submitting the same string to the same history ring
// twice in a row is deduplicated -- the ring keeps only one entry, not
// two adjacent copies.
#[test]
fn push_minibuffer_history_dedupes_adjacent_repeat() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    type_str(&mut i, &ed, "same");
    feed_keys(&mut i, &ed, "RET").unwrap();

    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    type_str(&mut i, &ed, "same");
    feed_keys(&mut i, &ed, "RET").unwrap();

    // A third read: M-p should recall "same" once, and a second M-p
    // should find nothing further back (the ring holds only one entry).
    run(
        &mut i,
        "(setq r nil) (read-string \"A: \" (lambda (s) (setq r s)))",
    );
    feed_keys(&mut i, &ed, "M-p").unwrap();
    assert_eq!(mb_input(&ed), "same");
    feed_keys(&mut i, &ed, "M-p").unwrap();
    // Still "same" -- there's nothing older to walk back to.
    assert_eq!(mb_input(&ed), "same");
    feed_keys(&mut i, &ed, "RET").unwrap();
}

// 19. Fix round: a symbol list works as COLLECTION -- elements are read
// by their print name (`collection_arg`, ui.rs).
#[test]
fn symbol_collection_works_as_completing_read_source() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" '(alpha beta) (lambda (s) (setq result s)))",
    );
    assert_eq!(panel_accepts(&ed), vec!["alpha", "beta"]);
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "result"), "\"alpha\"");
}

// 20. Fix round: a COLLECTION element that's neither a string nor a
// symbol signals wrong-type-argument instead of panicking or silently
// dropping it.
#[test]
fn non_string_non_symbol_collection_element_signals_wrong_type() {
    let (mut i, ed) = setup();
    let err = run(
        &mut i,
        "(completing-read \"Pick: \" '(\"ok\" 42) (lambda (s) s))",
    );
    assert!(err.starts_with("ERROR"), "expected an error, got {err}");
    assert!(!mb_open(&ed));
}

// 21. Fix round: C-g cancels `y-or-n-p` without invoking the callback
// (mirrors test 14's y/n/other-key coverage, which never exercised
// C-g).
#[test]
fn y_or_n_p_c_g_cancels_without_callback() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result 'unset) (y-or-n-p \"Proceed? \" (lambda (v) (setq result v)))",
    );
    assert!(capturing(&ed));
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert!(!capturing(&ed));
    assert_eq!(run(&mut i, "result"), "unset");
}

// M70 review fix (finding 1): the echo row is the terminal's LAST row,
// so its last column is the screen's bottom-right cell -- writing to
// it is a scroll hazard on any terminal that hasn't disabled autowrap
// (`frontend-tui`'s setup never does). This isn't a minibuffer case
// (hence living up here rather than beside the read-string tests
// below), but it goes through the exact same echo-row drawing code, so
// it belongs in this file's echo-row coverage rather than a fourth
// test file. A plain `(message ...)` -- no minibuffer, no isearch --
// exactly `cols` (80) characters long used to have its 80th character
// silently dropped by the pre-M70 `if ecol + w >= cols { break; }`
// guard (one column short of `cols`, never written down as an
// invariant); this pins that as deliberate, not an accident of a
// stale `break`.
#[test]
fn m70_message_never_writes_the_echo_rows_last_column() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 24);
    let msg: String = (0..80).map(|n| char::from(b'A' + (n % 26) as u8)).collect();
    run(&mut i, &format!("(message \"{msg}\")"));
    let grid = core::redisplay::render(&i, &ed);
    let row = 23;
    let last_cell = &grid.lines[row][79];
    assert_ne!(
        last_cell.ch,
        msg.chars().nth(79).unwrap(),
        "the echo row's last column must never be written"
    );
    assert_eq!(
        last_cell.ch, ' ',
        "last column should be left as the grid's default blank cell"
    );
}

// M70: echo row horizontal scrolling. `read-string`'s INITIAL prefills
// a deep RTL path (72 columns of prompt+path, matching the repro in
// M70's spec section 1 exactly) on an 80-column frame, then a 34-char
// filename is typed on top of it.

const M70_PROMPT: &str = "Find file: ";
const M70_DEEP_PATH: &str = "~/projects/chipname/rtl/subsystem/interconnect/axi/axi4_lite/";
const M70_FILENAME: &str = "axi4_lite_to_apb_bridge_wrapper.sv";

fn m70_open_deep_path_prompt(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    ed.borrow_mut().frame = (80, 24);
    run(
        i,
        &format!(
            "(setq result nil) (read-string \"{}\" (lambda (s) (setq result s)) \"{}\")",
            M70_PROMPT, M70_DEEP_PATH
        ),
    );
}

// I1: this is the repro. Typing past column 80 must keep updating the
// echo row on every single keystroke -- the pre-M70 bug went silent
// after the 8th character.
#[test]
fn m70_i1_typing_past_80_cols_keeps_updating_echo_row() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    let mut prev = echo_row_text(&i, &ed);
    for c in M70_FILENAME.chars() {
        type_str(&mut i, &ed, &c.to_string());
        let cur = echo_row_text(&i, &ed);
        assert_ne!(cur, prev, "echo row did not change after typing {c:?}");
        prev = cur;
    }
}

// I2: once fully typed, the echo row's tail is the just-typed filename
// (not the truncated-from-column-0 view the old code stuck on).
#[test]
fn m70_i2_end_of_echo_row_shows_tail_of_input() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    assert!(echo_row_text(&i, &ed).ends_with(M70_FILENAME));
}

// I3: the cursor stays on screen and lands exactly one column past the
// last-drawn character of the input.
#[test]
fn m70_i3_cursor_visible_and_aligned_with_input_end() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    let col = echo_cursor_col(&i, &ed);
    assert!(col < 80);
    // Caret sits at the very end of a 106-cell text over an 80-column
    // frame -- `echo_scroll_off`'s "caret at end" case puts it at the
    // last visible column. That's column 78, not 79: the echo row's
    // usable width is `cols - 1` (79), so the last visible column is
    // index 78 -- column 79 (the grid's actual last column) is never
    // drawn or pointed at (M70 review finding 1 -- see `win`'s own
    // comment in `redisplay.rs`).
    assert_eq!(col, 78);
}

// I3b (M70 review finding 7): I3's `col == 78` alone doesn't prove the
// scroll offset is being computed correctly -- it only proves the
// cursor ends up at the rightmost visible column when the caret is at
// the very end of a long text, which the "no scrolling, just clamp to
// `cols - 1`" pre-M70 shape would also produce for THIS one geometry.
// Landing the caret somewhere in the middle of a long, already-scrolled
// input -- not at either edge -- pins down the offset itself, not just
// the clamp.
#[test]
fn m70_i3b_mid_window_cursor_matches_scroll_offset() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    feed_keys(&mut i, &ed, "C-a").unwrap();
    for _ in 0..70 {
        feed_keys(&mut i, &ed, "C-f").unwrap();
    }
    // caret = prompt (11 cells) + 70 input chars = 81; total = 106;
    // win = 79 (`cols - 1`). `echo_scroll_off(106, 81, 79)`: caret is
    // past `win - 1` (78) but total is past `win` too, so this is
    // still the "scrolled" branch: `max_off = 107 - 79 = 28`,
    // `off = (81 - 78).min(28) = 3`, `cursor = 81 - 3 = 78`.
    assert_eq!(echo_cursor_col(&i, &ed), 78);
}

// I4: `C-a` snaps the view back to the left edge -- prompt visible from
// column 0, no `…`, cursor sitting right after the prompt.
#[test]
fn m70_i4_c_a_returns_to_left_edge() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    feed_keys(&mut i, &ed, "C-a").unwrap();
    let text = echo_row_text(&i, &ed);
    assert!(text.starts_with(M70_PROMPT), "text={text:?}");
    assert!(!text.contains('…'), "text={text:?}");
    assert_eq!(echo_cursor_col(&i, &ed), M70_PROMPT.chars().count());
}

// I5: `C-e` from there goes back to the tail view (same shape as I2/I3).
#[test]
fn m70_i5_c_e_returns_to_tail() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "C-e").unwrap();
    assert!(echo_row_text(&i, &ed).ends_with(M70_FILENAME));
    assert_eq!(echo_cursor_col(&i, &ed), 78);
}

// I6: while scrolled (cursor at the end of the long input), the first
// visible column is the left-edge `…` indicator.
#[test]
fn m70_i6_left_edge_indicator_while_scrolled() {
    let (mut i, ed) = setup();
    m70_open_deep_path_prompt(&mut i, &ed);
    type_str(&mut i, &ed, M70_FILENAME);
    let text = echo_row_text(&i, &ed);
    assert_eq!(text.chars().next(), Some('…'), "text={text:?}");
}

// I7: short prompt/input that never overflows the frame renders exactly
// as it did before M70 -- prompt at column 0, no `…`, cursor right
// after the input.
#[test]
fn m70_i7_short_input_unaffected() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 24);
    run(
        &mut i,
        "(setq result nil) (read-string \"Q: \" (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "abc");
    let text = echo_row_text(&i, &ed);
    assert_eq!(text, "Q: abc");
    assert!(!text.contains('…'));
    assert_eq!(echo_cursor_col(&i, &ed), "Q: abc".chars().count());
}

// I8 (M70 review finding 5): a wide (CJK) character split by the RIGHT
// edge of the scrolled window -- its first half the last visible
// column, its second half one column past the window -- must draw
// blank there, not the wide char's first half with its second half
// silently dropped. Nothing exercised this before: `redisplay.rs`'s
// `if idx + 1 < off + win` could be weakened to `<=` (drawing the
// truncated half) without any test noticing.
//
// Geometry, worked out algebraically rather than by trial and error:
// whenever `caret >= win - 1` (78, here) `echo_scroll_off` picks
// `off = caret - (win - 1)`, which makes `off + win - 1 == caret`
// always -- the rightmost visible column is always exactly the cell
// AT the caret's own index. So placing the wide char's first half at
// cell index `caret` (i.e. immediately after wherever the cursor
// sits, with only plain ASCII before it) puts it exactly on that
// rightmost column, with its second half one past the window edge.
//
// prompt "P: " = 3 cells. 87 "x" chars before the cursor -> caret =
// 3 + 87 = 90 (>= 78, so the property above holds). The wide char
// "寬" sits immediately after the cursor; 30 "y" chars trail it so
// `total` (122) stays comfortably above `win` (79).
#[test]
fn m70_i8_wide_char_split_at_right_scroll_edge_draws_blank() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 24);
    let initial = format!("{}{}{}", "x".repeat(87), '寬', "y".repeat(30));
    run(
        &mut i,
        &format!(
            "(setq result nil) (read-string \"P: \" (lambda (s) (setq result s)) \"{initial}\")"
        ),
    );
    feed_keys(&mut i, &ed, "C-a").unwrap();
    for _ in 0..87 {
        feed_keys(&mut i, &ed, "C-f").unwrap();
    }
    assert_eq!(echo_cursor_col(&i, &ed), 78, "sanity: caret at column 78");

    let grid = core::redisplay::render(&i, &ed);
    let row = 23;
    // Column 77 (idx 89): the last "x" before the wide char -- sanity
    // that the geometry above lines up.
    assert_eq!(grid.lines[row][77].ch, 'x', "sanity: column 77 is 'x'");
    // Column 78 (idx 90): the wide char's first half -- must be blank,
    // not '寬', since its second half (idx 91) would fall outside the
    // window.
    assert_eq!(
        grid.lines[row][78].ch, ' ',
        "wide char split by the right scroll edge must draw blank, not its first half"
    );
    assert!(!grid.lines[row][78].continuation);
}

// M70 K*: minibuffer point-movement keys (`C-a`/`C-e`/`C-b`/`C-f`,
// arrow keys, `C-k`, backspace) had zero regression coverage anywhere
// in the test suite before this milestone, despite M70's scrolling
// correctness depending directly on their semantics. `mb_input` after
// typing at a moved cursor is the observable: it shows where the next
// inserted character actually landed.

// K1: `C-b`/`C-f` each move the cursor by exactly one character.
#[test]
fn m70_k1_c_b_c_f_move_one_char() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "ac");
    feed_keys(&mut i, &ed, "C-b").unwrap(); // cursor: a[c] -> between a and c
    type_str(&mut i, &ed, "b");
    assert_eq!(mb_input(&ed), "abc");
}

// M70 review finding 2: the caret must come from ONE expansion of
// prompt-plus-input-prefix, not from two independent expansions summed.
// `echo_cells` always counts columns from zero, so a tab inside the
// input prefix lands on the wrong tab stop when that prefix is expanded
// on its own: after a 4-column prompt the tab really spans 4 columns
// (column 4 up to the column-8 stop), but expanded from zero it
// measures 8, putting the caret at column 12 instead of 8.
//
// A literal tab can't be typed in -- `commands.rs`'s minibuffer
// self-insert arm excludes it -- so it arrives as an INITIAL, which is
// also the shape a completion candidate or an `M-p` history entry can
// carry. The fix-round change had no test until mutation M4 survived;
// this is that test.
#[test]
fn m70_caret_tab_stop_uses_a_single_expansion() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (80, 24);
    run(&mut i, "(read-string \"M-x \" (lambda (s) s) \"\tx\")");
    assert!(mb_open(&ed), "minibuffer did not open");
    assert_eq!(mb_input(&ed), "\tx");
    // `C-a` then `C-f` leaves the cursor just past the tab, so the
    // caret is exactly the width of "M-x " + that tab.
    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    assert_eq!(
        echo_cursor_col(&i, &ed),
        8,
        "caret must use the merged expansion's tab stop"
    );
}

// K2: `C-a` moves to the start -- a subsequent insert lands there.
#[test]
fn m70_k2_c_a_moves_to_start() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "bc");
    feed_keys(&mut i, &ed, "C-a").unwrap();
    type_str(&mut i, &ed, "a");
    assert_eq!(mb_input(&ed), "abc");
}

// K3: `C-e` moves back to the end -- a subsequent insert appends.
#[test]
fn m70_k3_c_e_moves_to_end() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "ab");
    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "C-e").unwrap();
    type_str(&mut i, &ed, "c");
    assert_eq!(mb_input(&ed), "abc");
}

// K4: the left/right arrow keys behave the same as `C-b`/`C-f`.
#[test]
fn m70_k4_arrow_keys_match_c_b_c_f() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "ac");
    feed_keys(&mut i, &ed, "<left>").unwrap();
    type_str(&mut i, &ed, "b");
    assert_eq!(mb_input(&ed), "abc");

    let (mut i2, ed2) = setup();
    run(
        &mut i2,
        "(read-string \"P: \" (lambda (s) (setq result s)))",
    );
    type_str(&mut i2, &ed2, "ac");
    feed_keys(&mut i2, &ed2, "C-b").unwrap();
    type_str(&mut i2, &ed2, "b");
    assert_eq!(mb_input(&ed2), mb_input(&ed));

    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "<right>").unwrap();
    feed_keys(&mut i, &ed, "<right>").unwrap();
    feed_keys(&mut i, &ed, "<right>").unwrap();
    type_str(&mut i, &ed, "d");
    assert_eq!(mb_input(&ed), "abcd");
}

// K5: `C-b` at the start and `C-f` at the end don't under/overflow --
// repeated presses don't panic and leave the cursor position (and
// input) unchanged.
#[test]
fn m70_k5_boundary_presses_do_not_panic_or_move() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "ab");
    feed_keys(&mut i, &ed, "C-a").unwrap();
    for _ in 0..5 {
        feed_keys(&mut i, &ed, "C-b").unwrap();
    }
    type_str(&mut i, &ed, "X");
    assert_eq!(mb_input(&ed), "Xab");

    feed_keys(&mut i, &ed, "C-e").unwrap();
    for _ in 0..5 {
        feed_keys(&mut i, &ed, "C-f").unwrap();
    }
    type_str(&mut i, &ed, "Y");
    assert_eq!(mb_input(&ed), "XabY");
}

// K6: `C-k` truncates from the cursor to the end of the input.
#[test]
fn m70_k6_c_k_truncates_from_cursor() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "abcdef");
    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    feed_keys(&mut i, &ed, "C-k").unwrap();
    assert_eq!(mb_input(&ed), "ab");
}

// K7: backspace deletes the character immediately before the cursor,
// not whatever's at the end of the input.
#[test]
fn m70_k7_backspace_deletes_before_cursor() {
    let (mut i, ed) = setup();
    run(&mut i, "(read-string \"P: \" (lambda (s) (setq result s)))");
    type_str(&mut i, &ed, "abcdef");
    feed_keys(&mut i, &ed, "C-a").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    feed_keys(&mut i, &ed, "DEL").unwrap();
    assert_eq!(mb_input(&ed), "acdef");
}

// M84: orderless-style multi-token matching and M-x type-as-you-filter.
// T1/T2/T3/T4/T5/T6 exercise `custom_filter` (via `completing-read`,
// which never sorts -- collection order is preserved, matching the
// existing test 5/5b style above); T7/T8/T9 exercise the M-x
// (`Source::Command`) panel path added by D4/D5; T10 pins TAB's LCP
// expansion still working through the panel; T11 observes the D6 cache.

// T1/T2: "clk rst" matches candidates containing both tokens regardless
// of order or adjacency, and token order in the input doesn't matter.
#[test]
fn orderless_multi_token_matches_regardless_of_order() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" \
         '(\"clk_rst_ctrl\" \"rst_clk_sync\" \"clk_only\" \"unrelated\") \
         (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "clk rst");
    let forward: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        forward,
        ["clk_rst_ctrl", "rst_clk_sync"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );

    // Cancel and reopen to type the tokens in the opposite order.
    feed_keys(&mut i, &ed, "C-g").unwrap();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" \
         '(\"clk_rst_ctrl\" \"rst_clk_sync\" \"clk_only\" \"unrelated\") \
         (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "rst clk");
    let backward: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        forward, backward,
        "token order in the input must not change the matched set"
    );
}

// T3: a candidate missing any one token is excluded.
#[test]
fn orderless_excludes_candidates_missing_any_token() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" \
         '(\"clk_rst_ctrl\" \"clk_only\" \"rst_only\") \
         (lambda (s) (setq result s)))",
    );
    type_str(&mut i, &ed, "clk rst");
    assert_eq!(panel_accepts(&ed), vec!["clk_rst_ctrl"]);
}

// T4: smart case -- an all-lowercase token matches a mixed-case
// candidate case-insensitively, but a token containing an uppercase
// letter only matches case-sensitively (M82's `rg --smart-case`
// convention).
#[test]
fn orderless_smart_case_per_token() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" \
         '(\"ClkDomain\" \"clkdomain\") (lambda (s) (setq result s)))",
    );
    // All-lowercase input: case-insensitive, matches both.
    type_str(&mut i, &ed, "clk");
    assert_eq!(panel_accepts(&ed), vec!["ClkDomain", "clkdomain"]);
    feed_keys(&mut i, &ed, "C-g").unwrap();

    run(
        &mut i,
        "(setq result nil) (completing-read \"Pick: \" \
         '(\"ClkDomain\" \"clkdomain\") (lambda (s) (setq result s)))",
    );
    // A capital letter in the input: case-sensitive, only the
    // exact-case candidate matches.
    type_str(&mut i, &ed, "Clk");
    assert_eq!(panel_accepts(&ed), vec!["ClkDomain"]);
}

// T5: empty input matches every candidate (already covered implicitly
// by tests 3/5b above via the initial, untyped panel listing, but
// asserted directly here against `orderless_rank` itself since that's
// the shared contract D2/D3 both build on).
#[test]
fn orderless_rank_empty_input_matches_everything_at_rank_zero() {
    assert_eq!(core::complete::orderless_rank("anything", ""), Some(0));
    assert_eq!(core::complete::orderless_rank("", ""), Some(0));
}

// --- M85: `(orderless-rank CANDIDATE INPUT)` -- the elisp-visible ------
// --- primitive wrapping `orderless_rank` unchanged, so elisp callers --
// --- outside the minibuffer (e.g. `*search*`'s live filter, search.el) -
// --- get the identical semantics tested above through --------------
// --- `completing-read`, without a second, hand-rolled implementation. -

#[test]
fn orderless_rank_primitive_returns_nil_0_1_faithfully() {
    let (mut i, _ed) = setup();
    // No match at all -- nil, not `false`/0.
    assert_eq!(
        run(&mut i, r#"(orderless-rank "clk_alu_top" "xyz")"#),
        "nil"
    );
    // First token is a prefix of the candidate -- rank 0.
    assert_eq!(run(&mut i, r#"(orderless-rank "alu_top" "alu")"#), "0");
    // Matches, but not by the first-token-is-a-prefix rule -- rank 1.
    assert_eq!(run(&mut i, r#"(orderless-rank "clk_alu_top" "alu")"#), "1");
}

#[test]
fn orderless_rank_primitive_multi_token_any_order() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, r#"(orderless-rank "clk_alu_top" "alu clk")"#),
        "1",
        "both tokens present, any order -- must match even though \"alu\" \
         comes before \"clk\" in the candidate but after it in INPUT"
    );
    assert_eq!(
        run(&mut i, r#"(orderless-rank "clk_alu_top" "alu missing")"#),
        "nil",
        "one token absent -- must not match regardless of the other"
    );
}

#[test]
fn orderless_rank_primitive_smart_case_per_token() {
    let (mut i, _ed) = setup();
    // An all-lowercase token is case-INsensitive.
    assert_eq!(run(&mut i, r#"(orderless-rank "ALU_TOP" "alu")"#), "0");
    // A token containing an uppercase letter is case-sensitive -- must
    // NOT match a candidate that only differs in case.
    assert_eq!(run(&mut i, r#"(orderless-rank "alu_top" "ALU")"#), "nil");
    assert_eq!(run(&mut i, r#"(orderless-rank "ALU_top" "ALU")"#), "0");
}

#[test]
fn orderless_rank_primitive_empty_or_whitespace_input_matches_everything() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, r#"(orderless-rank "anything" "")"#), "0");
    assert_eq!(run(&mut i, r#"(orderless-rank "anything" "   ")"#), "0");
    assert_eq!(run(&mut i, r#"(orderless-rank "" "")"#), "0");
}

// T6: single-token input behaves exactly as the pre-M84 prefix/substring
// split did -- pinned directly against `custom_filter`, in addition to
// `prefix_matches_rank_before_substring_matches` above which already
// covers this end-to-end through `completing-read`.
#[test]
fn single_token_input_matches_pre_m84_prefix_then_substring_behavior() {
    let cands = ["scatter", "cat", "dog"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        core::complete::custom_filter(&cands, "cat"),
        vec!["cat".to_string(), "scatter".to_string()]
    );
}

fn open_meta_x(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    feed_keys(i, ed, "M-x").unwrap();
}

// T7: M-x is type-as-you-filter now -- the panel has content after a
// single keystroke, with no TAB press.
#[test]
fn meta_x_filters_live_without_tab() {
    let (mut i, ed) = setup();
    open_meta_x(&mut i, &ed);
    // Before typing anything, every command is listed.
    assert!(
        !panel_accepts(&ed).is_empty(),
        "M-x should list candidates before any key is typed"
    );
    type_str(&mut i, &ed, "b");
    assert!(
        !panel_accepts(&ed).is_empty(),
        "one keystroke into M-x should already have filtered, non-empty panel rows"
    );
    assert!(
        panel_accepts(&ed).iter().all(|c| c.contains('b')),
        "every listed row after typing \"b\" should contain it: {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// T8: M-x's orderless substring matching finds a real command by an
// interior substring of its name, not just a prefix.
#[test]
fn meta_x_substring_matches_a_real_command_name() {
    let (mut i, ed) = setup();
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "buffer");
    assert!(
        panel_accepts(&ed).contains(&"switch-to-buffer".to_string()),
        "expected \"switch-to-buffer\" among M-x candidates for \"buffer\": {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// T9: the panel's up/down selection works for the Command source (the
// same machinery File/Buffer/Custom already had).
#[test]
fn meta_x_panel_arrow_key_selection_works() {
    let (mut i, ed) = setup();
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "buffer");
    let before = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .unwrap()
        .selected;
    feed_keys(&mut i, &ed, "down").unwrap();
    let after = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .unwrap()
        .selected;
    assert_ne!(before, after, "down should move the M-x panel selection");
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// T10: TAB still expands to the longest common prefix through the panel
// path (D5: the panel and TAB's LCP expansion are orthogonal -- moving
// M-x onto the panel must not have dropped TAB's own job).
#[test]
fn meta_x_tab_still_expands_longest_common_prefix() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun se-t10-cmd-one () (interactive) 1)
         (defun se-t10-cmd-two () (interactive) 2)",
    );
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-t10-cmd-");
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(mb_input(&ed), "se-t10-cmd-");
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// T11 (D6): the Command/Function/Symbol candidate list is computed once
// per minibuffer session, not once per keystroke. There's no return
// value that distinguishes a cache hit from a recompute -- the
// candidate list looks identical either way -- so this reads
// `complete::debug_compute_count()`'s counter, added specifically as
// this test's observability hook (see its doc comment). H2 correction
// (M84 fix round): this used to say "process-wide counter" -- F1 made
// it thread-local, so it's really "this thread's own counter"; safe to
// read here because this whole test runs on one thread.
#[test]
fn symbol_source_candidate_list_is_computed_once_per_session() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun se-t11-symbol-cmd (s) (interactive \"S\") (setq result s))",
    );
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-t11-symbol-cmd");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        mb_open(&ed),
        "the command's own \"S\" prompt should have opened"
    );

    let before = core::complete::debug_compute_count();
    // Several more keystrokes in the SAME session must not trigger any
    // further recomputation of the underlying symbol list.
    type_str(&mut i, &ed, "se-t11");
    let mid = core::complete::debug_compute_count();
    type_str(&mut i, &ed, "-symbol-cmd");
    let after = core::complete::debug_compute_count();
    assert_eq!(
        (before, mid, after),
        (before, before, before),
        "typing within one session must not recompute the candidate list \
         (before={before}, mid={mid}, after={after})"
    );
    feed_keys(&mut i, &ed, "RET").unwrap();

    // A NEW session for the same source is allowed (expected) to
    // recompute once -- the cache is session-scoped, not permanent.
    run(
        &mut i,
        "(defun se-t11-symbol-cmd-2 (s) (interactive \"S\") (setq result s))",
    );
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-t11-symbol-cmd-2");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let after_new_session = core::complete::debug_compute_count();
    assert!(
        after_new_session > after,
        "a fresh minibuffer session is expected to recompute once \
         (before={after}, after new session={after_new_session})"
    );
}

// F2 (M84 fix round): `C-x b` (`Source::Buffer`) must also match through
// `orderless_rank` -- substring, any token order -- not the pre-fix
// single-token `starts_with` it was still on. Current-buffer-first
// ordering (already covered by `complete_tests.rs`'s
// `buffer_name_completion`) is untouched by this fix, so it isn't
// re-asserted here.
#[test]
fn buffer_source_matches_substring_and_out_of_order_tokens() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"clk-domain-rtl\")");
    run(&mut i, "(get-buffer-create \"rtl-clk-testbench\")");
    run(&mut i, "(get-buffer-create \"unrelated-buffer\")");
    feed_keys(&mut i, &ed, "C-x b").unwrap();

    // Substring, not just prefix: neither buffer name STARTS WITH
    // "domain", but one contains it.
    type_str(&mut i, &ed, "domain");
    assert_eq!(panel_accepts(&ed), vec!["clk-domain-rtl".to_string()]);
    feed_keys(&mut i, &ed, "C-g").unwrap();

    feed_keys(&mut i, &ed, "C-x b").unwrap();
    // Two tokens, out of the order they appear in either name.
    type_str(&mut i, &ed, "clk rtl");
    let matched: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        matched,
        ["clk-domain-rtl", "rtl-clk-testbench"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// F4 (M84 fix round): a direct test that `complete::reset_session_cache`
// (called from `commands.rs`'s `process_pending` each time a fresh
// minibuffer opens) is actually load-bearing. T11 above cycles through
// Command -> Symbol -> Command, which forces a recompute on every
// session regardless of whether the reset call runs at all (different
// `kind` always misses `cached_or_compute`'s single cache slot) --
// deleting the `reset_session_cache()` call site would NOT make T11
// fail. This test keeps the SAME kind (Command) across two consecutive
// sessions and defines a new command in between, so only the reset
// call (not a kind change) can make the new command visible.
//
// Mutation check (reported, not self-executed): deleting the
// `crate::complete::reset_session_cache();` call in `commands.rs`'s
// `process_pending` must turn this FAIL.
#[test]
fn command_source_cache_resets_between_sessions_of_the_same_kind() {
    let (mut i, ed) = setup();
    run(&mut i, "(defun se-f4-cmd-one () (interactive) 1)");
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-f4-cmd-");
    assert!(
        panel_accepts(&ed).contains(&"se-f4-cmd-one".to_string()),
        "sanity: the first command should be visible in its own session: {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert!(!mb_open(&ed));

    // A second command, defined strictly BETWEEN the two sessions.
    run(&mut i, "(defun se-f4-cmd-two () (interactive) 2)");
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-f4-cmd-");
    assert!(
        panel_accepts(&ed).contains(&"se-f4-cmd-two".to_string()),
        "a command defined between two M-x sessions of the SAME kind \
         must be visible in the second session, not served from a stale \
         cached list: {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// F5 (M84 fix round): a whitespace-ONLY input (not just empty) also
// yields zero tokens from `split_whitespace`, so it matches everything
// at rank 0 -- same as empty input, and a deliberate divergence from
// pre-M84 behavior (a plain `starts_with("   ")`/`contains("   ")`
// would have matched almost nothing). See `orderless_rank`'s doc
// comment for the coordinator's ruling to keep this behavior.
#[test]
fn orderless_whitespace_only_input_matches_everything() {
    let cands = ["alpha", "beta", "gamma"]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    assert_eq!(core::complete::custom_filter(&cands, "   "), cands);
    assert_eq!(core::complete::orderless_rank("anything", "   "), Some(0));
}

// F6 (M84 fix round): TAB's longest-common-prefix expansion is computed
// over the WHOLE filtered list (rank 0 + rank 1 mixed together,
// `longest_common_prefix` in `complete.rs`/`commands.rs`'s
// `minibuffer_tab`), not per-rank. A single rank-1 (substring-only, not
// prefix) candidate mixed into an otherwise-uniform-prefix set dilutes
// or destroys the common prefix. This isn't new logic (`longest_common_
// prefix` predates M84), but M84 is what put it behind M-x, a
// high-frequency TAB user. Recording ACTUAL behavior, not a fixed
// target -- see the coordinator's ruling in the fix-round request.
//
// H1 note (later fix round): this test's own lcp happens to still
// START WITH the typed stem ("se-f6-cmd"), so H1's `lcp.starts_with(
// &stem)` guard is a no-op here -- this test alone was never the H1
// repro. The strictly worse shape H1 actually fixed (lcp sharing NO
// characters with what was typed, so TAB would silently replace the
// input with something unrelated) is covered separately by
// `meta_x_tab_does_not_replace_input_with_unrelated_lcp` and
// `buffer_tab_does_not_replace_input_with_unrelated_lcp` below.
#[test]
fn meta_x_tab_lcp_is_diluted_by_a_mixed_in_substring_match() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(progn
           (defun se-f6-cmd-one () (interactive) 1)
           (defun se-f6-cmd-two () (interactive) 2)
           (defun se-f6-other-se-f6-cmd () (interactive) 3))",
    );
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "se-f6-cmd");
    // All three commands match "se-f6-cmd" (the first two as a prefix,
    // the third only as a substring -- it doesn't START with it).
    let matched: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        matched,
        ["se-f6-cmd-one", "se-f6-cmd-two", "se-f6-other-se-f6-cmd"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "sanity: all three should match \"se-f6-cmd\": {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "TAB").unwrap();
    // KNOWN GAP (recorded, not fixed): with the substring-only match
    // mixed in, the three candidates' longest common prefix collapses
    // to something far short of "se-f6-cmd-" (or empty) -- pinning
    // whatever it actually comes out to rather than asserting the
    // "nice" pre-fix-round-only-prefix-candidates behavior.
    assert_eq!(
        mb_input(&ed),
        "se-f6-cmd",
        "TAB expansion collapses to no expansion at all once a \
         substring-only match dilutes the common prefix"
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// H1 (M84 fix round): TAB's longest-common-prefix expansion used to
// only check LENGTH (`lcp.chars().count() > stem.chars().count()`),
// never whether `lcp` actually extends what the user typed. Under
// orderless matching the two can share zero characters -- reproduced
// with the coordinator's exact repro shape: input "clkr rst" (two
// tokens, each only a SUBSTRING match, not a prefix, of either
// candidate) against "se-review-clkrstz1"/"se-review-clkrstz2", whose
// longest common prefix is "se-review-clkrstz" (17 chars) -- longer
// than the 8-char stem but sharing no characters with it at all.
// Pre-fix, TAB would truncate the input to nothing and replace it with
// "se-review-clkrstz", silently discarding what the user typed with no
// `[No match]` or other signal. Fixed by requiring `lcp.starts_with(
// &stem)` before expanding.
#[test]
fn meta_x_tab_does_not_replace_input_with_unrelated_lcp() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(progn
           (defun se-review-clkrstz1 () (interactive) 1)
           (defun se-review-clkrstz2 () (interactive) 2))",
    );
    open_meta_x(&mut i, &ed);
    type_str(&mut i, &ed, "clkr rst");
    // Sanity: both candidates really do match (substring-only, per the
    // repro's shape) before TAB is even pressed.
    let matched: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        matched,
        ["se-review-clkrstz1", "se-review-clkrstz2"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "sanity: both should match \"clkr rst\" as substrings: {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(
        mb_input(&ed),
        "clkr rst",
        "TAB must not silently replace the user's typed query with an \
         unrelated longest-common-prefix string"
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// H1, same shape via `Source::Buffer` (`C-x b`) -- F2 is this bug's new
// entry point: before F2, `buffer_rows` filtered on a single
// `starts_with(input)` over the WHOLE input string, so a two-token,
// space-containing input like "clkr rst" would almost never match any
// buffer name at all (a name would need that literal substring,
// spaces included). F2's orderless rewrite made per-token substring
// matches (and therefore this repro shape) reachable through `C-x b`
// too.
#[test]
fn buffer_tab_does_not_replace_input_with_unrelated_lcp() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"se-review-clkrstz1\")");
    run(&mut i, "(get-buffer-create \"se-review-clkrstz2\")");
    feed_keys(&mut i, &ed, "C-x b").unwrap();
    type_str(&mut i, &ed, "clkr rst");
    let matched: std::collections::HashSet<String> = panel_accepts(&ed).into_iter().collect();
    assert_eq!(
        matched,
        ["se-review-clkrstz1", "se-review-clkrstz2"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "sanity: both buffers should match \"clkr rst\" as substrings: {:?}",
        panel_accepts(&ed)
    );
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(
        mb_input(&ed),
        "clkr rst",
        "TAB must not silently replace the user's typed query with an \
         unrelated longest-common-prefix string"
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}
