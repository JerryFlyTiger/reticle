//! M53: an isearch session's `start`/`origin`/`origin_byte` are offsets
//! into whatever buffer was current when the session started. Nothing
//! used to invalidate the session when elisp swapped the current buffer
//! out from under it (`switch-to-buffer`, `kill-buffer`) mid-session --
//! see the `Isearch::buffer` field and the interception point at the top
//! of `commands::handle_key` (right before `if ed.borrow().isearch.is_some()`).
//!
//! Run on a big-stack thread (matches `crates/elisp/tests/gc_tests.rs`)
//! since the interpreter can recurse deeply.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
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

fn with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(f)
        .expect("spawn failed")
        .join()
        .expect("test thread panicked");
}

#[test]
fn switch_to_buffer_mid_session_then_c_g_does_not_move_point() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        // Buffer A ("first", the scratch buffer created by init): long
        // enough that a stale offset lands somewhere plausible-looking
        // (not just clamped to 0), same shape as the repro in the spec.
        run(&mut i, "(insert \"alpha body gamma delta epsilon\")");
        run(&mut i, "(goto-char 15)");
        assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");

        feed_keys(&mut i, &ed, "C-s b o d y").unwrap();
        assert!(ed.borrow().isearch.is_some(), "session should be open");

        // Buffer B: distinct content, distinct point.
        run(&mut i, "(switch-to-buffer \"other\")");
        run(&mut i, "(insert \"0123456789abcdefghij\")");
        run(&mut i, "(goto-char 5)");
        let point_before_cg = run(&mut i, "(point)");
        assert_eq!(point_before_cg, "5");

        feed_keys(&mut i, &ed, "C-g").unwrap();

        assert!(
            ed.borrow().isearch.is_none(),
            "switching buffers mid-session must end the stale session"
        );
        assert_eq!(run(&mut i, "(buffer-name)"), "\"other\"");
        assert_eq!(
            run(&mut i, "(point)"),
            point_before_cg,
            "C-g on a stale session must NOT move point in the new buffer"
        );
    });
}

#[test]
fn switch_to_buffer_mid_session_then_ordinary_key_inserts_normally() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"alpha body gamma delta epsilon\")");
        run(&mut i, "(goto-char 15)");

        feed_keys(&mut i, &ed, "C-s b o d y").unwrap();
        assert!(ed.borrow().isearch.is_some());

        run(&mut i, "(switch-to-buffer \"other\")");
        run(&mut i, "(insert \"0123456789abcdefghij\")");
        run(&mut i, "(goto-char (point-max))");

        // A self-inserting key must go into the new buffer as ordinary
        // text, not be swallowed as an isearch query character.
        feed_keys(&mut i, &ed, "z").unwrap();

        assert!(
            ed.borrow().isearch.is_none(),
            "the stale session must not still be intercepting keys"
        );
        assert_eq!(run(&mut i, "(buffer-string)"), "\"0123456789abcdefghijz\"");
    });
}

#[test]
fn kill_buffer_mid_session_then_key_does_not_search_another_buffer() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(switch-to-buffer \"victim\")");
        run(&mut i, "(insert \"alpha body gamma delta epsilon\")");
        run(&mut i, "(goto-char 15)");

        feed_keys(&mut i, &ed, "C-s b o d y").unwrap();
        assert!(ed.borrow().isearch.is_some());

        // Switch away first (kill-buffer normally requires it not be
        // current, and this also matches the spec's "kill the buffer
        // being searched" scenario without killing the current buffer).
        // Deliberately CONTAINS "body" (the query) positioned AFTER the
        // session's origin offset (15, from "victim") -- if a stale
        // session were to keep searching this buffer forward from that
        // stale offset, it would find this match and move point, so
        // this assertion actually has power to catch a regression
        // instead of passing vacuously on a buffer with no possible
        // match (reviewer finding, M53 second round).
        run(&mut i, "(switch-to-buffer \"survivor\")");
        run(&mut i, "(insert \"0123456789abcdefghij body\")");
        run(&mut i, "(goto-char 5)");
        run(&mut i, "(kill-buffer \"victim\")");

        let point_before = run(&mut i, "(point)");
        feed_keys(&mut i, &ed, "C-s").unwrap();

        // Either the session was already invalidated by the buffer swap
        // above (before kill-buffer even ran), or it survives the swap
        // and gets invalidated here -- either way, this key must not
        // silently search "survivor" using "victim"'s stale offsets, and
        // must not resurrect a session pointed at a buffer that no
        // longer exists.
        assert!(
            ed.borrow().isearch.is_none()
                || ed
                    .borrow()
                    .isearch
                    .as_ref()
                    .unwrap()
                    .buffer
                    .upgrade()
                    .is_some(),
            "must not leave a session referencing a killed buffer"
        );
        assert_eq!(
            run(&mut i, "(point)"),
            point_before,
            "a stray C-s after kill-buffer must not move point via a stale session"
        );
    });
}

#[test]
fn switching_away_and_back_to_the_same_buffer_keeps_the_session_alive() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(
            &mut i,
            "(insert \"the quick brown fox jumps over the lazy dog\")",
        );
        run(&mut i, "(goto-char (point-min))");
        assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");

        feed_keys(&mut i, &ed, "C-s t h e").unwrap();
        assert!(ed.borrow().isearch.is_some());
        let first_match = run(&mut i, "(point)");

        // Switch away, then switch straight back to the SAME buffer --
        // deliberately not treated as invalidation (see the module doc
        // and the comment at the interception point in commands.rs).
        run(&mut i, "(switch-to-buffer \"other\")");
        run(&mut i, "(switch-to-buffer \"*scratch*\")");
        assert!(
            ed.borrow().isearch.is_some(),
            "switching away and back to the SAME buffer must not end the session"
        );

        // A repeat (C-s again) should still work against the live
        // session and find the next occurrence of "the".
        feed_keys(&mut i, &ed, "C-s").unwrap();
        let second_match = run(&mut i, "(point)");
        assert_ne!(first_match, second_match);
    });
}

#[test]
fn isearch_active_p_is_nil_in_the_gap_after_switching_buffers() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"alpha body gamma delta epsilon\")");
        run(&mut i, "(goto-char 15)");

        feed_keys(&mut i, &ed, "C-s b o d y").unwrap();
        assert_eq!(run(&mut i, "(isearch-active-p)"), "t");

        // Gap: the current buffer has already been swapped by elisp, but
        // no isearch keystroke has reached `handle_key` yet to lazily
        // clear the now-stale session out of `editor.isearch`. A caller
        // reading `isearch.is_some()` directly here would wrongly see a
        // live session (M53 second round, reviewer finding).
        run(&mut i, "(switch-to-buffer \"other\")");
        assert_eq!(
            run(&mut i, "(isearch-active-p)"),
            "nil",
            "isearch-active-p must not report a session stale for the new current buffer"
        );
    });
}

#[test]
fn ordinary_isearch_ret_and_c_g_are_unaffected() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"the quick brown fox\")");
        run(&mut i, "(goto-char (point-min))");

        feed_keys(&mut i, &ed, "C-s b r o w n").unwrap();
        assert_eq!(run(&mut i, "(point)"), "16"); // just after "brown"
        feed_keys(&mut i, &ed, "RET").unwrap();
        assert!(ed.borrow().isearch.is_none());
        assert_eq!(run(&mut i, "(point)"), "16");

        run(&mut i, "(goto-char 1)");
        feed_keys(&mut i, &ed, "C-s x y z n o m a t c h").unwrap();
        feed_keys(&mut i, &ed, "C-g").unwrap();
        assert!(ed.borrow().isearch.is_none());
        assert_eq!(run(&mut i, "(point)"), "1");
    });
}

// The completion-popup modal-session guard (`builtins/ui.rs`) is the
// other reader of the session besides `handle_key` and
// `isearch-active-p`. An async LSP reply arriving in the gap -- after
// elisp switched buffers, before the next keystroke let `handle_key`
// clear the stale session -- must not be mistaken for "a modal session
// is up" and silently dropped: the reply is for the buffer the user is
// in now, and no real isearch is running there.
//
// `show-completion-popup` returns nil whether or not it opened anything
// (see builtins/ui.rs), so the signal has to be
// `completion-popup-active-p', not the funcall's value -- same reason
// completion_popup_tests.rs gives.
#[test]
fn completion_popup_not_swallowed_by_a_stale_session_after_switching_buffers() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"alpha body gamma delta epsilon\")");
        run(&mut i, "(goto-char 15)");
        run(&mut i, "(setq other (generate-new-buffer \"other\"))");
        run(&mut i, "(with-current-buffer other (insert \"abc\"))");

        feed_keys(&mut i, &ed, "C-s b o d y").unwrap();
        assert!(ed.borrow().isearch.is_some(), "session should be live here");

        // elisp switches out from under the session; no key has arrived
        // since, so `handle_key` has not cleared it yet.
        run(&mut i, "(switch-to-buffer other)");
        assert!(
            ed.borrow().isearch.is_some(),
            "invalidation is lazy -- the raw Option is still Some in the gap"
        );

        run(
            &mut i,
            "(show-completion-popup (list (list \"foo\" \"foo\" (point) \"\")) (point))",
        );
        assert_eq!(
            run(&mut i, "(completion-popup-active-p)"),
            "t",
            "a session stale for the current buffer must not swallow the reply"
        );
    });
}

// M70 S1: isearch shares the echo row's drawing path with the
// minibuffer (`redisplay.rs`'s single echo-area block), but has no
// `cursor` field of its own -- point is always at the end of the typed
// query (see `Isearch`'s own doc comment). A query long enough to
// overflow an 80-column frame must still scroll to keep the tail (the
// characters just typed) visible, exactly like the minibuffer case in
// `completing_read_tests.rs`'s I1-I3.
#[test]
fn s1_long_query_scrolls_echo_row_to_show_tail() {
    with_big_stack(|| {
        let (mut i, ed) = setup();
        ed.borrow_mut().frame = (80, 24);
        run(&mut i, "(insert \"just some scratch buffer text\")");
        feed_keys(&mut i, &ed, "C-s").unwrap();
        assert!(ed.borrow().isearch.is_some());

        // 120 non-matching, non-periodic characters: sixty two-digit
        // numbers "00".."59" concatenated. Long enough to overflow 80
        // columns even after the "Failing I-search: " prefix (18
        // cols), never matching the buffer text (no digits in it) so
        // the session stays open the whole way through, and -- unlike
        // a repeated character, which any ≥20-char window of trailing
        // copies would satisfy whether or not the row actually
        // scrolled (M70 review finding 3) -- strictly increasing so
        // every 20-char substring of the whole query is unique. A
        // window that DIDN'T follow the caret would show query[..60]
        // (numbers "00".."29"ish) instead of the query's own tail, and
        // that can never equal the true tail below.
        let query: String = (0..60).map(|n: u32| format!("{n:02}")).collect();
        for c in query.chars() {
            feed_keys(&mut i, &ed, &c.to_string()).unwrap();
        }
        assert!(
            ed.borrow().isearch.is_some(),
            "session should still be open"
        );

        let row = 23;
        let grid = core::redisplay::render(&i, &ed);
        let text: String = grid.lines[row]
            .iter()
            .filter(|c| !c.continuation)
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string();
        let tail: String = query.chars().skip(query.len() - 20).collect();
        assert!(
            text.ends_with(&tail),
            "echo row did not scroll to the tail: text={text:?} tail={tail:?}"
        );
    });
}
