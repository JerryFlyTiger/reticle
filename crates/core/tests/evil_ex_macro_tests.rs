//! M42 Part II: `:s` substitute (evil.el's "M42-II: :s substitute"
//! section) and `q`/`@` keyboard macros (evil.el's "M42-II: keyboard
//! macros" section, backed by the Rust-side kbd-macro engine in
//! builtins/ui.rs and the recording tap in commands.rs's `handle_key`).
//! `setup_evil`/`feed`/`bs`/`pt`/`echo_row_text`/`type_text` mirror
//! evil_tests.rs's/evil_marks_registers_tests.rs's own helpers exactly
//! (same setup/run/feed_keys pattern, copied rather than shared since
//! integration test binaries can't import each other's private
//! helpers).

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

/// Turns S into a space-separated `feed`/`parse_kbd` token sequence
/// (each character its own token, a literal space becoming the `SPC`
/// token) -- lets a test type arbitrary text, backslashes and all,
/// through the REAL minibuffer/recording key path instead of round-
/// tripping it through the elisp reader's own string-escaping (which
/// would need a third, easy-to-get-wrong layer of backslash-doubling
/// on top of this parser's own `\/` delimiter escape and the regex
/// engine's `\(`/`\)`/`\1` syntax).
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

// ---------------------------------------------------------------------
// Part II-A: :s substitute
// ---------------------------------------------------------------------

#[test]
fn s_on_the_current_line_replaces_only_the_first_occurrence() {
    let (mut i, ed) = setup_evil("cat cat cat");
    run(&mut i, "(evil--run-ex \"s/cat/dog/\")");
    assert_eq!(bs(&mut i), "dog cat cat");
    assert_eq!(
        pt(&mut i),
        1,
        "point must land on the first-non-blank of the substituted line"
    );
    assert_eq!(
        echo_row_text(&i, &ed),
        "Substituted 1 occurrence(s) on 1 line(s)"
    );
}

#[test]
fn s_with_g_flag_replaces_every_occurrence_on_the_line() {
    let (mut i, ed) = setup_evil("cat cat cat");
    run(&mut i, "(evil--run-ex \"s/cat/dog/g\")");
    assert_eq!(bs(&mut i), "dog dog dog");
    assert_eq!(
        echo_row_text(&i, &ed),
        "Substituted 3 occurrence(s) on 1 line(s)"
    );
}

#[test]
fn percent_s_replaces_across_the_whole_buffer() {
    let (mut i, ed) = setup_evil("cat\ndog cat\ncat cat");
    run(&mut i, "(evil--run-ex \"%s/cat/X/g\")");
    assert_eq!(bs(&mut i), "X\ndog X\nX X");
    assert_eq!(
        echo_row_text(&i, &ed),
        "Substituted 4 occurrence(s) on 3 line(s)"
    );
    // Last substituted line is line 3 ("X X"), first-non-blank = its
    // own start: X(1) \n(2) d(3)o(4)g(5) SP(6)X(7) \n(8) X(9) SP(10)X(11).
    assert_eq!(pt(&mut i), 9);
}

#[test]
fn numeric_range_restricts_substitution_to_those_lines() {
    let (mut i, ed) = setup_evil("cat\ncat\ncat\ncat\ncat");
    run(&mut i, "(evil--run-ex \"2,4s/cat/dog/\")");
    assert_eq!(bs(&mut i), "cat\ndog\ndog\ndog\ncat");
    let _ = ed;
}

#[test]
fn backslash_group_and_ampersand_expand_in_the_replacement_template() {
    let (mut i, ed) = setup_evil("foo bar");
    // \1/\2 = the two capture groups, \& = the whole match ("foo bar").
    // Typed through the real minibuffer (see `type_text`'s own doc
    // comment) so the backslashes reach `evil--run-ex' exactly as a
    // user would type them, with no elisp-reader-escaping to get wrong.
    feed(
        &mut i,
        &ed,
        &format!(": {} RET", type_text(r"s/\(foo\) \(bar\)/[\2-\1] \&/")),
    );
    assert_eq!(bs(&mut i), "[bar-foo] foo bar");
}

#[test]
fn zero_hits_reports_pattern_not_found_and_touches_nothing() {
    let (mut i, ed) = setup_evil("hello world");
    run(&mut i, "(evil--run-ex \"s/xyz/abc/\")");
    assert_eq!(echo_row_text(&i, &ed), "Pattern not found: xyz");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(pt(&mut i), 1);
}

#[test]
fn empty_pattern_reuses_the_last_search_string() {
    let (mut i, ed) = setup_evil("cat bat cat");
    run(&mut i, "(isearch-set-last \"cat\" t)");
    run(&mut i, "(evil--run-ex \"s//dog/\")");
    assert_eq!(bs(&mut i), "dog bat cat");
    let _ = ed;
}

#[test]
fn s_sets_last_search_so_n_finds_the_next_match() {
    let (mut i, ed) = setup_evil("cat bat cat");
    run(&mut i, "(evil--run-ex \"s/cat/dog/\")");
    assert_eq!(bs(&mut i), "dog bat cat");
    assert_eq!(
        run(&mut i, "(isearch-last-string)"),
        "\"cat\"",
        ":s must have written the pattern into Editor::last_search"
    );
    feed(&mut i, &ed, "n");
    // "dog bat cat": d(1)o(2)g(3) SP(4)b(5)a(6)t(7) SP(8)c(9)a(10)t(11);
    // search-forward lands just past the match, per its own convention.
    assert_eq!(pt(&mut i), 12);
}

#[test]
fn whole_percent_s_undoes_in_a_single_step() {
    // Driven through the REAL `:' command dispatch (not a direct
    // `evil--run-ex' call): `execute_command''s own preamble is what
    // inserts the undo boundary separating this buffer's setup content
    // from the :s edits about to run -- a direct function call skips
    // that dispatch entirely, which would make a SINGLE `u' revert the
    // setup insert too (an artifact of this test's own plumbing, not a
    // real bug -- see the module doc's file-header precedent for why
    // this file otherwise prefers direct `evil--run-ex' calls).
    let (mut i, ed) = setup_evil("cat\ncat\ncat");
    feed(&mut i, &ed, &format!(": {} RET", type_text("%s/cat/dog/")));
    assert_eq!(bs(&mut i), "dog\ndog\ndog");
    feed(&mut i, &ed, "u");
    assert_eq!(
        bs(&mut i),
        "cat\ncat\ncat",
        "one `u' must revert the WHOLE :%s as a single undo group"
    );
}

#[test]
fn visual_v_selecting_three_lines_then_a_bare_s_only_touches_those_lines() {
    let (mut i, ed) = setup_evil("cat\ncat\ncat\ncat\ncat");
    feed(&mut i, &ed, "V j j"); // linewise-select lines 1-3
    feed(&mut i, &ed, &format!(": {} RET", type_text("s/cat/dog/")));
    assert_eq!(
        bs(&mut i),
        "dog\ndog\ndog\ncat\ncat",
        "an explicit-range-less `:s' right after visual must auto-apply \
         the just-left selection, touching only its 3 lines"
    );
}

#[test]
fn explicit_visual_marks_range_can_still_be_typed_directly() {
    let (mut i, ed) = setup_evil("cat\ncat\ncat\ncat\ncat");
    // Simulates a visual-selection snapshot from some earlier command
    // (see `evil--ex-visual-range''s own doc comment: only
    // `evil-ex-from-visual' ever writes it) without re-driving the
    // whole visual-mode dance -- this test is specifically about RANGE
    // RESOLUTION, already isolated from the visual-plumbing test above.
    run(&mut i, "(setq evil--ex-visual-range (cons 1 3))");
    run(&mut i, "(evil--run-ex \"'<,'>s/cat/dog/\")");
    assert_eq!(bs(&mut i), "dog\ndog\ndog\ncat\ncat");
    let _ = ed;
}

// M42 review fix (Severity-1 #2): an explicit numeric RANGE address
// outside [1,`evil--total-lines'] (real vim's E16 "Invalid range") must
// abort the WHOLE `:s' with zero edits, not silently `forward-line'-
// clamp into revisiting an already-collected line a second time (see
// `evil--ex-resolve-range-endpoint''s own doc comment for the
// mechanism this used to fall through to: address `0' clamped to line
// 1 the same as address `1', so `:0,1s' collected -- and then applied
// -- the SAME match twice).

#[test]
fn a_zero_address_in_a_range_is_rejected_as_invalid_and_edits_nothing() {
    let (mut i, ed) = setup_evil("cat dog");
    run(&mut i, "(evil--run-ex \"0,1s/cat/X/\")");
    assert_eq!(echo_row_text(&i, &ed), "Invalid range");
    assert_eq!(
        bs(&mut i),
        "cat dog",
        "an out-of-range explicit address must abort BEFORE any edit, not \
         apply the in-range side of the pair and skip only the bad one"
    );
}

#[test]
fn a_too_high_address_in_a_range_is_rejected_as_invalid_and_edits_nothing() {
    let (mut i, ed) = setup_evil("cat dog");
    run(&mut i, "(evil--run-ex \"5,99s/cat/X/\")");
    assert_eq!(echo_row_text(&i, &ed), "Invalid range");
    assert_eq!(bs(&mut i), "cat dog");
}

#[test]
fn a_visual_snapshot_range_past_a_shrunk_buffer_clamps_instead_of_erroring() {
    let (mut i, ed) = setup_evil("cat\ncat\ncat\ncat\ncat");
    // Snapshot taken while the buffer still had 5 lines...
    run(&mut i, "(setq evil--ex-visual-range (cons 1 5))");
    // ...then the buffer shrinks to 2 lines before `:s' actually runs --
    // an ordinary, expected staleness (see `evil--ex-visual-range''s own
    // doc comment), NOT a typo, so unlike the explicit-numeric case
    // above this must degrade gracefully rather than erroring.
    run(&mut i, "(erase-buffer)");
    run(&mut i, "(insert \"cat\\ncat\")");
    run(&mut i, "(evil--run-ex \"'<,'>s/cat/dog/\")");
    assert_eq!(
        bs(&mut i),
        "dog\ndog",
        "the stale snapshot's end (5) must clamp to the buffer's new total \
         (2), not report \"Invalid range\""
    );
    let _ = ed;
}

#[test]
fn bare_s_auto_apply_after_visual_also_clamps_a_snapshot_that_outgrew_the_buffer() {
    // The `evil--ex-from-visual' auto-apply shortcut (empty RANGE-STR
    // right after visual) reads the identical `evil--ex-visual-range'
    // snapshot as the explicit `\\='<,\\='>' syntax above, but through a
    // SEPARATE code path (`evil--ex-clamp-visual-range', not
    // `evil--ex-resolve-range-endpoint') -- this proves that path clamps
    // too, rather than only the explicitly-typed-address one. BOTH
    // endpoints (3 and 5) are chosen past the 2-line buffer -- an
    // unclamped BEG of 3 makes `evil--ex-collect-substitutions' walk
    // straight past every real line (`evil--goto-line' overshoots
    // forward to `point-max' and never revisits an earlier, in-range
    // line the way an UNDER-shoot to line 0 would), so this is the one
    // shape of staleness where the difference between "clamped" and
    // "not clamped" actually changes the outcome -- an END-only
    // overshoot (BEG still valid) would collect the identical matches
    // either way and wouldn't prove the clamp does anything.
    let (mut i, ed) = setup_evil("cat\ncat");
    // Simulates a from-visual snapshot+flag pair (see the direct-state
    // precedent in `explicit_visual_marks_range_can_still_be_typed_
    // directly' above) taken while the buffer still had >=5 lines and
    // the selection started at line 3, now stale against the current
    // 2-line buffer.
    run(&mut i, "(setq evil--ex-visual-range (cons 3 5))");
    run(&mut i, "(setq evil--ex-from-visual t)");
    run(&mut i, "(evil--run-ex \"s/cat/dog/\")");
    assert_eq!(
        bs(&mut i),
        "cat\ndog",
        "the auto-apply shortcut must clamp the stale snapshot (3,5) down \
         to the buffer's current last line (2,2), touching only that line"
    );
    let _ = ed;
}

// ---------------------------------------------------------------------
// Part II-B: keyboard macros (q / @)
// ---------------------------------------------------------------------

#[test]
fn qa_records_ciw_then_typed_text_then_esc_and_at_a_replays_elsewhere() {
    let (mut i, ed) = setup_evil("foo bar baz");
    feed(&mut i, &ed, "w"); // point on "bar"
    feed(&mut i, &ed, "q a"); // arm register a; NOT yet recording
    assert_eq!(run(&mut i, "(defining-kbd-macro-p)"), "t");
    feed(&mut i, &ed, "c i w");
    feed(&mut i, &ed, &format!("{} ESC", type_text("abc")));
    feed(&mut i, &ed, "q"); // stop; the stopping `q' itself must be trimmed
    assert_eq!(run(&mut i, "(defining-kbd-macro-p)"), "nil");
    assert_eq!(run(&mut i, "(kbd-macro-p ?a)"), "t");
    assert_eq!(bs(&mut i), "foo abc baz");

    feed(&mut i, &ed, "w"); // move onto "baz"
    feed(&mut i, &ed, "@ a");
    assert_eq!(
        bs(&mut i),
        "foo abc abc",
        "@a must replay ciw+abc+ESC (not the STOP `q') at the new point"
    );
}

#[test]
fn a_count_prefixed_replay_runs_the_macro_that_many_times() {
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "q b"); // arm register b
    feed(&mut i, &ed, "x"); // records AND executes: deletes 'a'
    feed(&mut i, &ed, "q");
    assert_eq!(bs(&mut i), "bcdef");

    feed(&mut i, &ed, "3 @ b"); // 3 more deletes: 'b','c','d'
    assert_eq!(bs(&mut i), "ef");
}

#[test]
fn at_at_replays_the_most_recently_used_register() {
    let (mut i, ed) = setup_evil("abcdef");
    feed(&mut i, &ed, "q c"); // arm register c
    feed(&mut i, &ed, "x"); // deletes 'a'
    feed(&mut i, &ed, "q");
    assert_eq!(bs(&mut i), "bcdef");

    feed(&mut i, &ed, "@ c"); // deletes 'b'; remembers register c
    assert_eq!(bs(&mut i), "cdef");
    feed(&mut i, &ed, "@ @"); // replays c again
    assert_eq!(bs(&mut i), "def");
}

#[test]
fn a_macro_containing_a_colon_ex_command_replays_correctly() {
    let (mut i, ed) = setup_evil("cat\ncat");
    feed(&mut i, &ed, "q e"); // arm register e
    feed(&mut i, &ed, &format!(": {} RET", type_text("s/cat/dog/")));
    feed(&mut i, &ed, "q");
    assert_eq!(bs(&mut i), "dog\ncat");

    feed(&mut i, &ed, "j"); // move to line 2
    feed(&mut i, &ed, "@ e");
    assert_eq!(
        bs(&mut i),
        "dog\ndog",
        "the recorded `:' ex command must replay end-to-end through the \
         minibuffer, not just the raw ':' keystroke"
    );
}

#[test]
fn recording_at_g_inside_another_macro_does_not_double_record_gs_expansion() {
    let (mut i, ed) = setup_evil("ABCDEF");
    // g = "x" (delete-char); executes once for real while recording it.
    feed(&mut i, &ed, "q g x q");
    assert_eq!(bs(&mut i), "BCDEF");

    // f = "@g" (a DYNAMIC reference, if the recording tap correctly
    // skips g's own replayed keys -- see the tap's own doc comment in
    // commands.rs). Recording this executes g once for real too.
    feed(&mut i, &ed, "q f @ g q");
    assert_eq!(bs(&mut i), "CDEF");

    // Redefine g to something else entirely (movement, not deletion).
    // If f had (incorrectly) double-recorded g's OLD expansion ('x')
    // as a literal keystroke, replaying f below would delete another
    // character; if f correctly stored only the two keys '@'/'g', it
    // re-dispatches to CURRENT g and just moves point instead.
    feed(&mut i, &ed, "q g l q");
    assert_eq!(bs(&mut i), "CDEF", "re-recording g must not itself edit");
    let before = pt(&mut i);

    feed(&mut i, &ed, "@ f");
    assert_eq!(
        bs(&mut i),
        "CDEF",
        "@f must have re-dispatched to g's CURRENT ('l', no-op-on-text) \
         definition, not a stale, double-recorded 'x'"
    );
    assert_eq!(
        pt(&mut i),
        before + 1,
        "g's CURRENT body ('l') must have run"
    );
}

#[test]
fn at_at_self_recursion_aborts_cleanly_at_the_depth_cap() {
    // 32 levels of fully-nested handle_key -> apply_function -> eval ->
    // ... -> execute-kbd-macro -> handle_key native recursion is deep
    // enough to overflow a DEFAULT thread stack in an unoptimized debug
    // build -- not a bug introduced here, but this codebase's existing,
    // documented characteristic of its tree-walking elisp evaluator
    // (see eval_tests.rs's/timeout_tests.rs's own `run` helpers, and
    // src/main.rs, all of which already run on an explicit 512MB-stack
    // thread for exactly this reason -- "matching how the real binary
    // evaluates", per eval_tests.rs's own comment). Mirroring that
    // precedent here, rather than shrinking the depth cap to fit a
    // default stack, keeps the cap at the value the M42 spec actually
    // asks for (32) and matches how the real interactive binary would
    // run this same scenario.
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(|| {
            let (mut i, ed) = setup_evil("PQRST");
            // h = "@@" -- recorded WITHOUT ever actually replaying
            // during its own recording (`evil--last-macro-register' is
            // still nil at record time, so each `@' just arms/echoes
            // "No previous macro").
            feed(&mut i, &ed, "q h @ @ q");
            assert_eq!(run(&mut i, "(kbd-macro-p ?h)"), "t");

            // Replaying h now DOES have a target (itself, via
            // `evil--last-macro-register' = h) -- must recurse up to
            // the Rust-side 32-deep cap and unwind cleanly rather than
            // hang or overflow the stack.
            feed(&mut i, &ed, "@ h");
            assert_eq!(
                bs(&mut i),
                "PQRST",
                "self-recursion never touches buffer text"
            );

            // Prove `macro_replay_depth' was actually restored to 0
            // (not left stuck by the recursion): a FRESH recording
            // only captures keys while depth is 0 (see the recording
            // tap) -- if depth were stuck, z's own 'x' keystroke would
            // silently fail to record, and @z would replay as a no-op
            // instead of deleting a second character.
            feed(&mut i, &ed, "q z x q"); // records z=['x'], deletes 'P' for real
            assert_eq!(bs(&mut i), "QRST");
            feed(&mut i, &ed, "@ z");
            assert_eq!(
                bs(&mut i),
                "RST",
                "z's recording (and replay) must be unaffected by the \
                 earlier self-recursion -- proves macro_replay_depth \
                 returned to 0"
            );
        })
        .expect("spawn failed")
        .join()
        .expect("big-stack test thread panicked");
}

#[test]
fn q_then_escape_cancels_without_leaving_a_half_recorded_state() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "q ESC");
    assert_eq!(run(&mut i, "(defining-kbd-macro-p)"), "nil");
    assert_eq!(bs(&mut i), "hello");
    // Normal typing must still work -- nothing left the buffer/state stuck.
    feed(&mut i, &ed, "x");
    assert_eq!(bs(&mut i), "ello");
}

#[test]
fn replaying_an_undefined_register_messages_and_touches_nothing() {
    let (mut i, ed) = setup_evil("hello");
    feed(&mut i, &ed, "@ z"); // register z was never recorded
    assert_eq!(echo_row_text(&i, &ed), "No macro in register z");
    assert_eq!(bs(&mut i), "hello");
    assert_eq!(pt(&mut i), 1);
}
