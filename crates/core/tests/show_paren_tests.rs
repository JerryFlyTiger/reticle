//! M113: matching-bracket highlighting (GNU's `show-paren-mode`).
//!
//! Mirrors `rainbow_delimiters_tests.rs`'s worker-thread test pattern
//! (real major-mode function to turn the highlight engine on, then
//! `tick_until` pumps `core::idle_tick` past the debounce so the
//! background parse actually finishes) since the bracket pairing this
//! milestone highlights is computed by that exact same parse
//! (`highlight.rs`'s `collect_rainbow_spans`/`Engine::matching_pair`).
//!
//! But assertions here go through the rendered grid
//! (`gui_features_tests.rs`'s pattern for `hl-line`/`region`), not
//! through overlays: `show-paren-mode`'s highlight is a per-frame
//! direct write in `render_window`, not an overlay, so there is no
//! overlay to inspect -- the grid is the only observable surface.
//!
//! `display-line-numbers` is turned off in every fixture below (even
//! though the major-mode functions used here turn it on by default via
//! `prog-mode-hook`, same as `rainbow-delimiters-mode`) purely to keep
//! column arithmetic trivial: with it off, a fixture's Nth character
//! (0-based) always lands at grid column N, row R, with R the number of
//! `\n`s before it -- no gutter width to account for.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use core::redisplay::render;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(src: &str, mode: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (60, 8);
    run(&mut interp, &format!("(insert {:?})", src));
    run(&mut interp, &format!("({})", mode));
    run(&mut interp, "(setq-local display-line-numbers nil)");
    // hl-line-mode is also on by default (M32) and would tint the
    // cursor row's ENTIRE width wherever nothing else set a background
    // -- since every fixture here is short enough that the cursor's row
    // is the only row, that would mask a `None` (no paren highlight)
    // assertion with hl-line's own color instead. Turned off so these
    // tests observe show-paren-mode's own behavior in isolation;
    // `region_wins_over_hl_line_...`-style interaction is
    // hl-line's own test file's job, not this one's.
    run(&mut interp, "(setq hl-line-mode nil)");
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// Pump ticks until `pred` returns "t", bounded by a 30s hang-guard
/// deadline rather than a fixed loop count (M151; see
/// `highlight_tests.rs`'s helper for the full rationale) -- identical
/// contract to `rainbow_delimiters_tests.rs`'s own helper of the same
/// name.
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

const COUNT_HL: &str = "(let ((n 0))
   (dolist (ov (overlays-in (point-min) (point-max)) nil)
     (when (overlay-get ov 'treesit-hl) (setq n (1+ n))))
   n)";

/// 1-based elisp buffer position of the first occurrence of `needle` in
/// `src` (pure-ASCII fixtures only, so byte offset == char offset).
fn goto_pos(src: &str, needle: &str) -> usize {
    src.find(needle)
        .unwrap_or_else(|| panic!("{:?} not found in {:?}", needle, src))
        + 1
}

/// (row, col), both 0-based, of the first occurrence of `needle` in
/// `src` -- the grid cell that character lands on, given
/// `display-line-numbers` is off in every fixture here (see this file's
/// header).
fn grid_pos(src: &str, needle: &str) -> (usize, usize) {
    let byte_off = src
        .find(needle)
        .unwrap_or_else(|| panic!("{:?} not found in {:?}", needle, src));
    let before = &src[..byte_off];
    let row = before.matches('\n').count();
    let col = match before.rfind('\n') {
        Some(nl) => byte_off - nl - 1,
        None => byte_off,
    };
    (row, col)
}

const DRACULA_SHOW_PAREN_MATCH: (u8, u8, u8) = (0x73, 0x5c, 0x49); // M113 review fix round: re-derived, see themes.el

fn bg_at(
    interp: &Interp,
    ed: &Rc<RefCell<Editor>>,
    row: usize,
    col: usize,
) -> Option<(u8, u8, u8)> {
    render(interp, ed).lines[row][col].style.bg
}

// --- Matched pair: point before the opener, point after the closer ---

#[test]
fn point_before_the_opener_highlights_both_members() {
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Outer pair: point immediately before the outermost "(".
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    let (r_open, c_open) = grid_pos(src, "(a");
    let (r_close, c_close) = (0, src.len() - 1); // the final ")"
    assert_eq!(
        bg_at(&i, &ed, r_open, c_open),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "point right before the outer opener must highlight it"
    );
    assert_eq!(
        bg_at(&i, &ed, r_close, c_close),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "and highlight its matching closer too, per GNU's default behavior"
    );
}

#[test]
fn point_after_the_closer_highlights_both_members() {
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point right after the final ")" (goto-char to one past its 1-based
    // position, i.e. end of buffer).
    run(&mut i, &format!("(goto-char {})", src.len() + 1));
    let (r_open, c_open) = grid_pos(src, "(a");
    let (r_close, c_close) = (0, src.len() - 1);
    assert_eq!(
        bg_at(&i, &ed, r_close, c_close),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "point right after the outer closer must highlight it"
    );
    assert_eq!(
        bg_at(&i, &ed, r_open, c_open),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "and its matching opener too"
    );
}

// --- Nested pairs: the innermost pair adjacent to point, not the outer one ---

#[test]
fn point_before_an_inner_opener_highlights_only_the_inner_pair() {
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(b")));
    let (r_inner_open, c_inner_open) = grid_pos(src, "(b");
    let (r_inner_close, c_inner_close) = grid_pos(src, ") d"); // the inner ")"
    let (r_outer_open, c_outer_open) = grid_pos(src, "(a");
    let (r_outer_close, c_outer_close) = (0, src.len() - 1);

    assert_eq!(
        bg_at(&i, &ed, r_inner_open, c_inner_open),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "inner opener must be highlighted"
    );
    assert_eq!(
        bg_at(&i, &ed, r_inner_close, c_inner_close),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "inner closer must be highlighted"
    );
    assert_eq!(
        bg_at(&i, &ed, r_outer_open, c_outer_open),
        None,
        "the OUTER pair must not be highlighted just because it also encloses point"
    );
    assert_eq!(
        bg_at(&i, &ed, r_outer_close, c_outer_close),
        None,
        "outer closer must not be highlighted either"
    );
}

// --- Point just inside a bracket (not facing it) gets nothing, matching GNU ---

#[test]
fn point_just_inside_the_opener_highlights_nothing() {
    // Verified against real `emacs -Q --batch`: point immediately AFTER
    // an opener (i.e. between "(" and the next char, not before the
    // "(" itself) does not trigger show-paren-mode's highlight.
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // One past the outer "(" -- point sits between "(" and "a".
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a") + 1));
    let (r_open, c_open) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r_open, c_open),
        None,
        "point just inside an opener (not facing it) must not highlight"
    );
}

#[test]
fn point_on_the_closer_itself_highlights_nothing() {
    // Mirror of `point_just_inside_the_opener_highlights_nothing` for the
    // closer side. `Engine::matching_pair`'s predicate
    // (`highlight.rs:201`) is `open == point || close + 1 == point`,
    // both using 0-based byte offsets (internal `Buffer::point` is
    // 0-based; `get_pos` converts the elisp 1-based arg with `i - 1`).
    // Point sitting exactly ON the closer -- internal point == close,
    // i.e. the cursor faces the closer from before it, not past it --
    // satisfies neither arm: it isn't `== open`, and it isn't `== close
    // + 1` (that's one further along, immediately AFTER the closer).
    // Observed against this engine (not GNU): no highlight, matching
    // the "inside/on a bracket, not facing across it" family this
    // project also confirms for the opener side.
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // The final ")" is the last char of `src`; its 0-based byte offset
    // is `src.len() - 1`, so `goto-char` to the 1-based position
    // `src.len()` lands internal point exactly ON it (not past it).
    let outer_close_1based = src.len();
    run(&mut i, &format!("(goto-char {})", outer_close_1based));
    let (r, c) = (0, src.len() - 1);
    assert_eq!(
        bg_at(&i, &ed, r, c),
        None,
        "point sitting exactly on the closer (facing it, not past it) must not highlight"
    );
}

// --- A bracket inside a string/comment is not a real bracket token ---

#[test]
fn bracket_inside_a_c_string_literal_never_matches() {
    let src = "const char *s = \"a(b\";\nint f(int x) { return (x); }\n";
    let (mut i, ed) = setup(src, "c-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point right before the "(" that's really just string text.
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(b")));
    let (r, c) = grid_pos(src, "(b");
    assert_eq!(
        bg_at(&i, &ed, r, c),
        None,
        "a \"(\" that's really just string TEXT must never be matched/highlighted"
    );

    // Sanity check the fixture actually exercises the engine: a real
    // structural paren elsewhere DOES match.
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(int x)")));
    let (r2, c2) = grid_pos(src, "(int x)");
    assert_eq!(
        bg_at(&i, &ed, r2, c2),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "a real structural paren elsewhere in the same buffer must still match"
    );
}

// --- An unmatched bracket highlights nothing (this project's deliberate
// divergence from GNU's mismatch face -- see highlight.rs's
// `Engine::matching_pair` doc comment) ---

#[test]
fn unmatched_bracket_highlights_nothing() {
    let src = "(a b";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    let (r, c) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r, c),
        None,
        "an unmatched opener must highlight nothing (not GNU's mismatch face -- \
         this project has none, see the M113 report)"
    );
}

// --- Mode off produces no highlight ---

#[test]
fn mode_off_produces_no_highlight() {
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, "(setq show-paren-mode nil)");

    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    let (r, c) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r, c),
        None,
        "show-paren-mode nil must produce no highlight even for an otherwise-matched pair"
    );
}

#[test]
fn show_paren_mode_is_on_by_default() {
    let src = "(a)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // No explicit `(setq show-paren-mode t)` -- pinning the default itself.
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    let (r, c) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r, c),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "show-paren-mode must default to on, with no setq at all"
    );
}

// --- Review fix round: stale cache (edit elsewhere while point rests on
// an already-matched bracket, before the debounced reparse lands) ---

#[test]
fn stale_cache_after_an_edit_elsewhere_yields_no_highlight_until_reparse_lands() {
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point on the outer "(" -- pair (0, 10) gets cached.
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    let (r_open, c_open) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r_open, c_open),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "sanity: the pair is highlighted before the edit"
    );

    // An edit elsewhere that does NOT move point -- bumps `edit_ticks`
    // without invalidating or shifting the cached pairs. 1-based
    // position 6 == 0-based index 5, the space between "(b" and "c".
    run(&mut i, "(save-excursion (goto-char 6) (insert \"QQQQQ\"))");

    // Render immediately, deliberately with NO `idle_tick` call here --
    // this is exactly the race window before the debounced reparse
    // lands. In the edited text ("(a (bQQQQQ c) d)"), column 10 (the
    // stale cached close position) is now a SPACE, not ")".
    assert_eq!(
        bg_at(&i, &ed, 0, 10),
        None,
        "a stale cached pair must not paint the highlight on a non-bracket \
         character while the reparse is still in flight"
    );
    // The opener, still genuinely "(" in the edited text too, must also
    // not be highlighted during the stale window: `matching_pair` must
    // distrust the WHOLE cached generation, not selectively trust the
    // half that happens to still look right.
    assert_eq!(
        bg_at(&i, &ed, r_open, c_open),
        None,
        "the opener must not be highlighted either while the cache is stale"
    );

    // Self-heals once the reparse actually lands. NOT `tick_until` with
    // `COUNT_HL > 0` as the predicate: that's already true from the
    // parse before the edit (this fixture never drops to zero
    // highlighted spans), so it would return on the very first tick
    // without ever actually waiting for a NEW parse generation. Poll
    // the highlight itself instead.
    let mut healed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        core::idle_tick(&mut i, std::time::Duration::ZERO);
        if bg_at(&i, &ed, r_open, c_open) == Some(DRACULA_SHOW_PAREN_MATCH) {
            healed = true;
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    assert!(
        healed,
        "once the reparse lands, the pair must be highlighted correctly again"
    );
}

// --- Review fix round: `is_selected` gate, exercised with a real split ---

#[test]
fn non_selected_window_showing_the_same_buffer_and_point_gets_no_highlight() {
    let src = "(a)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, &format!("(goto-char {})", goto_pos(src, "(a")));
    run(&mut i, "(split-window-below)");
    // Same layout math as gui_features_tests.rs's `hl_line_does_not_
    // tint_a_non_selected_window`: frame (60, 8) -> windows_height 7 ->
    // `split-window-internal` keeps the ORIGINAL window selected as the
    // top pane (rows [0,1] text, [2] modeline); the new, unselected
    // window is the bottom pane (rows [3,4,5] text, [6] modeline).
    // Both show the same buffer at the same point.
    let (r, c) = grid_pos(src, "(a");
    assert_eq!(
        bg_at(&i, &ed, r, c),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "selected (top) window shows the highlight"
    );
    assert_eq!(
        bg_at(&i, &ed, r + 3, c),
        None,
        "non-selected (bottom) window's own copy of the same buffer/point must NOT be highlighted"
    );
}

// --- Review fix round: precedence between the paren highlight and an
// active region where they overlap (redisplay.rs:2408-2412 writes the
// paren bg, then the region block right after it) ---

#[test]
fn region_wins_over_the_paren_highlight_where_they_overlap() {
    // This test pins TODAY'S observed precedence -- it does not assert
    // that this is the GNU-correct answer. GNU-parity of this
    // precedence was not established here; see `redisplay.rs`'s
    // `region_bg.or(style.bg)` (the mechanism this test exercises).
    //
    // Rather than hardcode a second copy of the theme's region color,
    // this compares the OVERLAP cell (region + paren both apply) against
    // a REGION-ONLY cell (region applies, no paren) in the exact same
    // buffer state. If region wins, the two cells read back identical;
    // if the paren highlight won instead, the overlap cell would show
    // `DRACULA_SHOW_PAREN_MATCH` while the region-only cell would not.
    let src = "(a (b c) d)";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Point on the outer opener -- internal (0-based) point == 0, the
    // same adjacency `point_before_the_opener_highlights_both_members`
    // uses, so the paren highlight is active at offset 0 absent any
    // region. Set directly on the buffer (not via `goto-char`, which
    // would also have to satisfy the mark/region setup below in one
    // shot) then activate a mark-active region [0, 2) that covers BOTH
    // the opener (offset 0, paren-highlighted) and "a" (offset 1,
    // plain text, region-only).
    {
        let e = ed.borrow();
        let mut b = e.current.borrow_mut();
        b.point = 0;
        b.mark = Some(2);
        b.mark_active = true;
    }

    let (r_open, c_open) = grid_pos(src, "(a"); // offset 0: region + paren overlap
    let (r_plain, c_plain) = (r_open, c_open + 1); // offset 1, "a": region only
    let overlap_bg = bg_at(&i, &ed, r_open, c_open);
    let region_only_bg = bg_at(&i, &ed, r_plain, c_plain);

    assert!(
        region_only_bg.is_some(),
        "sanity: the region itself must be visible"
    );
    assert_ne!(
        overlap_bg,
        Some(DRACULA_SHOW_PAREN_MATCH),
        "sanity: confirm the paren highlight's own color didn't leak through unchanged"
    );
    assert_eq!(
        overlap_bg, region_only_bg,
        "region must win over the paren highlight where they overlap (today's observed precedence)"
    );
}

// --- Review fix round: tie-break at a close/open boundary ---

#[test]
fn tie_break_at_a_close_open_boundary_picks_the_earlier_closing_pair() {
    // Point sits exactly between the first pair's closer and the second
    // pair's opener, satisfying BOTH pairs' adjacency condition at once
    // ("point after a closer" for the first, "point before an opener"
    // for the second). Real GNU Emacs picks the EARLIER-CLOSING pair
    // here -- verified against real `emacs -Q --batch` on this exact
    // text ("()()", point 3): `show-paren--overlay`/`-1` come back as
    // (1 2)/(2 3), i.e. the FIRST "()", not the second. See highlight.rs's
    // ORDERING INVARIANT comment on `pairs` for why `Engine::
    // matching_pair`'s `.find()` reproduces that.
    let src = "()()";
    let (mut i, ed) = setup(src, "emacs-lisp-mode");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // 1-based position 3 == 0-based index 2, the boundary between the
    // first ")" (index 1) and the second "(" (index 2).
    run(&mut i, "(goto-char 3)");
    assert_eq!(
        bg_at(&i, &ed, 0, 0),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "the FIRST pair's opener (earlier-closing pair) must be highlighted"
    );
    assert_eq!(
        bg_at(&i, &ed, 0, 1),
        Some(DRACULA_SHOW_PAREN_MATCH),
        "the FIRST pair's closer must be highlighted"
    );
    assert_eq!(
        bg_at(&i, &ed, 0, 2),
        None,
        "the SECOND pair's opener must NOT be highlighted (loses the tie-break)"
    );
    assert_eq!(
        bg_at(&i, &ed, 0, 3),
        None,
        "the SECOND pair's closer must NOT be highlighted"
    );
}
