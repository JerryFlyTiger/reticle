//! M37: rainbow-delimiters -- matching brackets colored by nesting
//! depth. Mirrors highlight_tests.rs's own worker-thread test pattern
//! (`tick_until` pumping `core::idle_tick` past the debounce), since
//! rainbow spans are computed by the exact same background parse this
//! engine already runs for font-lock. Unlike highlight_tests.rs, fixture
//! buffers here go through a REAL major-mode function (`c-mode'/
//! `emacs-lisp-mode') rather than a bare `(treesit-highlight-mode ...)'
//! call: only `prog-mode-hook' (run by the major-mode function, see
//! modes.el's `treesit--prog-mode-setup') turns `rainbow-delimiters-
//! mode' on, and that's exactly the default-on-in-prog-buffers behavior
//! under test.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(&mut interp, &format!("(insert {:?})", src));
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
/// contract to highlight_tests.rs's own helper of the same name.
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

/// 1-based buffer position of the first occurrence of `needle` in `src`
/// (pure-ASCII fixtures only, so byte offset == char offset == the
/// elisp 1-based position after the usual +1). Computed instead of
/// hand-counted so a typo in a long fixture string can't silently
/// desync a position from what's actually in the buffer.
fn pos_of(src: &str, needle: &str) -> usize {
    src.find(needle)
        .unwrap_or_else(|| panic!("{:?} not found in {:?}", needle, src))
        + 1
}

/// The `rainbow-delimiters-depth-N-face` symbol name of the overlay at
/// 1-based buffer position `pos`, or `None` if there isn't one (either
/// no overlay at all, or an overlay whose face isn't a rainbow face --
/// e.g. font-lock-string-face over a bracket character sitting inside a
/// string literal).
fn rainbow_face_at(interp: &mut Interp, pos: usize) -> Option<String> {
    let src = format!(
        "(let (face)
           (dolist (ov (overlays-in {p} {p1}) face)
             (let ((f (overlay-get ov 'face)))
               (when (and f (symbolp f) (string-prefix-p \"rainbow-delimiters-depth-\" (symbol-name f)))
                 (setq face (symbol-name f))))))",
        p = pos,
        p1 = pos + 1
    );
    match interp.eval_source(&src) {
        Ok(elisp::Value::Str(s)) => Some((*s).clone()),
        Ok(elisp::Value::Nil) => None,
        other => panic!(
            "rainbow_face_at({}) unexpected result: {:?}",
            pos,
            other.is_ok()
        ),
    }
}

// --- Depth coloring, nesting, and pairing ---

#[test]
fn nested_three_levels_get_depth_1_2_3_faces_and_each_pair_shares_one_color() {
    let src = "(a (b (c)))";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(emacs-lisp-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let open1 = pos_of(src, "(a");
    let open2 = pos_of(src, "(b");
    let open3 = pos_of(src, "(c");
    // The three closers, right to left: "(c)" 's, then "(b ...)" 's,
    // then the outermost.
    let close3 = pos_of(src, "))"); // first char of "))" closes level 3
    let close2 = close3 + 1; // second char of "))" closes level 2
    let close1 = pos_of(src, ")))") + 2; // third char of ")))" closes level 1

    assert_eq!(
        rainbow_face_at(&mut i, open1).as_deref(),
        Some("rainbow-delimiters-depth-1-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, open2).as_deref(),
        Some("rainbow-delimiters-depth-2-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, open3).as_deref(),
        Some("rainbow-delimiters-depth-3-face")
    );
    // Matching open/close pairs share exactly one color.
    assert_eq!(
        rainbow_face_at(&mut i, close1),
        rainbow_face_at(&mut i, open1)
    );
    assert_eq!(
        rainbow_face_at(&mut i, close2),
        rainbow_face_at(&mut i, open2)
    );
    assert_eq!(
        rainbow_face_at(&mut i, close3),
        rainbow_face_at(&mut i, open3)
    );
}

#[test]
fn sibling_pairs_at_the_same_depth_share_color() {
    let src = "(a) (b)";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(emacs-lisp-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let first_open = pos_of(src, "(a");
    let second_open = pos_of(src, "(b");
    let first = rainbow_face_at(&mut i, first_open);
    let second = rainbow_face_at(&mut i, second_open);
    assert_eq!(first.as_deref(), Some("rainbow-delimiters-depth-1-face"));
    assert_eq!(
        first, second,
        "two top-level pairs must get the same depth-1 color"
    );
}

#[test]
fn tenth_level_of_nesting_wraps_back_to_depth_one_face() {
    let src = format!("{}x{}", "(".repeat(10), ")".repeat(10));
    let (mut i, _ed) = setup(&src);
    run(&mut i, "(emacs-lisp-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let outermost_open = 1; // 1st "(" -- depth 1
    let innermost_open = 10; // 10th "(" -- depth 10, wraps to depth-1's face
    assert_eq!(
        rainbow_face_at(&mut i, outermost_open).as_deref(),
        Some("rainbow-delimiters-depth-1-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, innermost_open).as_deref(),
        Some("rainbow-delimiters-depth-1-face"),
        "depth 10 must wrap around to the same face as depth 1"
    );
    // And depth 9 (one level out from the innermost) must NOT collide
    // with either -- otherwise the wraparound could be a coincidence of
    // everything mapping to one face rather than a genuine 9-cycle.
    assert_eq!(
        rainbow_face_at(&mut i, 9).as_deref(),
        Some("rainbow-delimiters-depth-9-face")
    );
}

// --- String/comment literals never contribute a bracket token ---

#[test]
fn bracket_character_inside_a_c_string_literal_gets_no_rainbow_overlay() {
    let src = "const char *s = \"a(b\";\nint f(int x) { return (x); }\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(c-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let paren_in_string = pos_of(src, "a(b");
    assert_eq!(
        rainbow_face_at(&mut i, paren_in_string),
        None,
        "a \"(\" that's really just string TEXT must not be treated as a bracket token"
    );
    // Sanity check the fixture actually exercises the engine: a real
    // structural paren elsewhere in the same buffer DOES get one.
    let real_paren = pos_of(src, "(int x)");
    assert!(rainbow_face_at(&mut i, real_paren).is_some());
}

#[test]
fn bracket_character_inside_a_c_comment_gets_no_rainbow_overlay() {
    let src = "// a(b comment\nint f(int x) { return (x); }\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(c-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let paren_in_comment = pos_of(src, "a(b");
    assert_eq!(
        rainbow_face_at(&mut i, paren_in_comment),
        None,
        "a \"(\" inside a // comment must not be treated as a bracket token"
    );
    let real_paren = pos_of(src, "(int x)");
    assert!(rainbow_face_at(&mut i, real_paren).is_some());
}

// --- Buffer-local toggle, applied at materialization time ---

#[test]
fn toggling_mode_off_removes_rainbow_overlays_next_tick_without_reparsing() {
    let src = "int f(int x) { return (x); }\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(c-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let open = pos_of(src, "(int x)");
    assert!(
        rainbow_face_at(&mut i, open).is_some(),
        "rainbow-delimiters-mode is on by default in a prog buffer"
    );
    let total_before = run(&mut i, COUNT_HL).parse::<i64>().unwrap();

    run(&mut i, "(setq-local rainbow-delimiters-mode nil)");
    // One tick, no edit: the worker thread never re-parses (nothing
    // invalidated `edit_ticks`) -- this exercises the materialization-
    // time gate in `apply_visible`, not a fresh worker computation.
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    assert_eq!(
        rainbow_face_at(&mut i, open),
        None,
        "toggling the mode off must remove the rainbow overlay on the very next tick"
    );
    let total_after = run(&mut i, COUNT_HL).parse::<i64>().unwrap();
    assert!(
        total_after < total_before,
        "some treesit-hl overlays (the rainbow ones) must have been dropped"
    );
    assert!(
        total_after > 0,
        "ordinary keyword/type highlighting overlays must be unaffected by the rainbow toggle"
    );
}

#[test]
fn toggling_mode_back_on_restores_rainbow_overlays_next_tick() {
    let src = "int f(int x) { return (x); }\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(c-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    run(&mut i, "(setq-local rainbow-delimiters-mode nil)");
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    let open = pos_of(src, "(int x)");
    assert_eq!(rainbow_face_at(&mut i, open), None);

    run(&mut i, "(setq-local rainbow-delimiters-mode t)");
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    assert!(
        rainbow_face_at(&mut i, open).is_some(),
        "turning the mode back on must restore the overlay on the next tick too"
    );
}

// --- M37 review fix round: sibling-matching stack replaces the linear
// counter (see highlight.rs's `collect_rainbow_spans` doc comment) -------
//
// The review's original repro was: an unclosed `(` mid-buffer (typing
// transient / pasted fragment / manually-deleted closer) followed by a
// later, syntactically-complete, unrelated top-level defun -- claiming
// the linear counter's leaked depth would miscolor that later defun all
// the way to EOF. Empirically dumping the actual parse tree (a throwaway
// probe binary, built the same way this codebase's now-deleted
// `crates/core/examples/dump_tree.rs` once was, then discarded) for the
// review's own literal text --
//
//   (defun f ()
//     (foo
//   (defun g ()
//     (+ 1 2)))
//
// -- shows tree-sitter-elisp recovers this with ZERO `ERROR` nodes and
// exactly one synthesized MISSING `)` at EOF (closing `f`): `g` parses as
// a perfectly ordinary, three-deep NESTED child of `foo`'s argument list
// (`f` > `foo`'s list > `g`), and `(+ 1 2)` four-deep under that. Both the
// old linear counter AND the new sibling-matching stack agree completely
// on every position in that exact text -- verified with both algorithms
// implemented side by side against the real tree, byte for byte. That
// text cannot demonstrate the fix because it was never actually a
// tree-shape bug for elisp specifically: every one of elisp's bracket
// families (`list` via `(` `)`, `vector` via `[` `]`) grammatically
// accepts `sexp*`, with no distinguished "block" delimiter the way Rust's
// `{}` differs from its `()` -- so tree-sitter-elisp's recovery for a
// dangling opener always prefers "keep nesting, synthesize one closer at
// EOF" (cost: one MISSING token, however much follows) over "orphan this
// token and resume at the top level". Confirmed across several other
// fixture shapes tried the same way (a dangling opener directly in a
// param list, a dangling `[`, an unterminated string, a stray extra
// closer before the dangling one) -- none produced the sibling-ejecting
// recovery the review's hypothesis assumed; elisp's grammar structurally
// doesn't have that failure mode.
//
// What DOES force tree-sitter-elisp into a genuine `ERROR` node -- and so
// gives the fix something real to prove -- is a MISMATCHED bracket KIND:
// a `]` typed where a `)` was meant (plausible fat-finger, or pasting a
// fragment from a language that uses `]`). The old linear counter's guard
// was blind to bracket kind (`")" | "]" | "}" if depth > 0`, no check
// that the specific closer matches the specific opener), so it treated
// that stray `]` as a perfectly valid pop -- decrementing `depth` one
// level too many and cascading into THREE broken pairs for the rest of
// the buffer, including leaving the buffer's own final, real, visible
// closing paren completely UNCOLORED (not just miscolored -- the guard's
// `depth > 0` went false early, so nothing painted it at all). The tests
// below pin the fixed behavior for exactly this shape, in both elisp and
// a structurally-distinct second grammar (Rust, whose `{}` block
// delimiter really is distinct from `()`, so it reaches the same
// `ERROR`-node recovery by a more direct route -- no bracket-kind typo
// needed, just a dangling `(` inside an otherwise-normal function body).

#[test]
fn elisp_stray_closer_of_the_wrong_kind_is_uncolored_and_does_not_leak_into_later_siblings() {
    // Tree shape (dump-verified): `f`'s own `function_definition` node
    // directly contains `(`, its `()` arglist, and a `list` node for
    // `(foo ...)`. THAT list directly contains `(`, `foo`, and a second
    // `list` for the dangling, initially-unclosed `(` -- which in turn
    // directly contains: the dangling `(` itself, an `ERROR` node holding
    // just the stray `]` (parsed into its own node because a `list`
    // grammatically expects `)`, never `]`, so `]` cannot continue it),
    // `g`'s whole `function_definition`, and finally a REAL closing `)`
    // (a sibling of the dangling `(`, both children of that inner list --
    // tree-sitter still finds a legitimate way to close it once `g`'s own
    // well-formed form is behind it). `g` is thus four bracket-levels
    // deep (f > foo's-list > the-dangling-(''s-list > g), one level
    // deeper than the review's original text, because of that extra
    // wrapper list the mismatched bracket forced into existence.
    let src = "(defun f ()\n  (foo (bar]\n(defun g ()\n  (+ 1 2)))))\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(emacs-lisp-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let f_open = pos_of(src, "(defun f");
    let foo_open = pos_of(src, "(foo (");
    let dangling_open = pos_of(src, "(bar]");
    let stray_bracket = dangling_open + 4; // the "]" itself
    let g_open = pos_of(src, "(defun g");
    let inner_plus_open = pos_of(src, "(+ 1 2)");
    let inner_plus_close = inner_plus_open + 6; // "(+ 1 2)" is 7 chars
    let g_close = inner_plus_open + 7; // closes g, right after its body
    let dangling_close = inner_plus_open + 8; // closes the once-dangling "("
    let foo_close = inner_plus_open + 9; // closes "(foo ...)"
    let f_close = inner_plus_open + 10; // closes f -- a REAL char here,
                                        // not a synthesized MISSING one,
                                        // since this fixture supplies
                                        // exactly enough real closers to
                                        // balance every opener.

    assert_eq!(
        rainbow_face_at(&mut i, stray_bracket),
        None,
        "a \"]\" that can't close the \"(\" it's sitting next to (wrong kind, its own \
         ERROR node) must be left uncolored -- the old linear counter's kind-blind guard \
         wrongly treated any \")\"/\"]\"/\"}}\" as a valid pop whenever depth > 0"
    );

    // f's own pair: the whole point of the fix. The old algorithm left
    // f_close completely UNCOLORED (guard's depth>0 check went false
    // early because of the phantom decrement from the stray "]"); the
    // fix restores it to f_open's own color.
    let f_open_face = rainbow_face_at(&mut i, f_open);
    assert_eq!(
        f_open_face.as_deref(),
        Some("rainbow-delimiters-depth-1-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, f_close),
        f_open_face,
        "f's own closing paren must share f's own opening paren's color, despite the \
         mismatched-kind stray closer sitting inside f's body"
    );

    let foo_open_face = rainbow_face_at(&mut i, foo_open);
    assert_eq!(
        foo_open_face.as_deref(),
        Some("rainbow-delimiters-depth-2-face")
    );
    assert_eq!(rainbow_face_at(&mut i, foo_close), foo_open_face);

    let dangling_open_face = rainbow_face_at(&mut i, dangling_open);
    assert_eq!(
        dangling_open_face.as_deref(),
        Some("rainbow-delimiters-depth-3-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, dangling_close),
        dangling_open_face,
        "the ORIGINALLY-dangling \"(\" still ends up correctly paired with its real \
         closer once tree-sitter's own recovery supplies one"
    );

    let g_open_face = rainbow_face_at(&mut i, g_open);
    assert_eq!(
        g_open_face.as_deref(),
        Some("rainbow-delimiters-depth-4-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, g_close),
        g_open_face,
        "g's own parens must share one color even though g sits inside the trouble \
         spot's recovered subtree"
    );

    let inner_plus_open_face = rainbow_face_at(&mut i, inner_plus_open);
    assert_eq!(
        inner_plus_open_face.as_deref(),
        Some("rainbow-delimiters-depth-5-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, inner_plus_close),
        inner_plus_open_face
    );
}

#[test]
fn rust_dangling_open_paren_in_error_node_no_longer_shifts_a_later_sibling_fns_depth() {
    // Tree shape (dump-verified): `source_file`'s two DIRECT children are
    // `f`'s and `g`'s `function_item` nodes -- genuine top-level siblings,
    // unlike the elisp case above, because Rust's grammar distinguishes
    // the `{}` block delimiter from `()` expression grouping. Inside f's
    // `block` node, the dangling `(` parses into its own `ERROR` node (a
    // sibling of `identifier "foo"` and the block's real closing `}`) --
    // tree-sitter simply drops it as noise rather than trying to nest
    // anything inside it, since nothing valid can follow to fill an
    // argument list there. `g` is consequently a true, unrelated sibling
    // at the SAME depth as `f`, and its own braces/parens should read
    // exactly like any other fresh top-level fn's.
    let src = "fn f() {\n    foo(\n}\nfn g(y: i32) { let z = y; }\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(rust-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let f_brace_open = pos_of(src, "{\n    foo(");
    let dangling_open = pos_of(src, "(\n}");
    let f_brace_close = dangling_open + 2; // the "}" right after the dangling "("
    let g_params_open = pos_of(src, "(y: i32)");
    let g_params_close = g_params_open + 7; // "(y: i32)" is 8 chars
    let g_brace_open = pos_of(src, "{ let");
    let g_brace_close = pos_of(src, "y; }") + 3;

    let f_brace_open_face = rainbow_face_at(&mut i, f_brace_open);
    assert_eq!(
        f_brace_open_face.as_deref(),
        Some("rainbow-delimiters-depth-1-face")
    );
    assert_eq!(
        rainbow_face_at(&mut i, f_brace_close),
        f_brace_open_face,
        "f's own {{ and }} must share one color -- the old linear counter instead left \
         f's \"}}\" one level deeper than f's \"{{\" (colored depth-2, not depth-1), because \
         the dangling \"(\" inside the ERROR node had incremented depth with no way back \
         down for it"
    );
    assert!(
        rainbow_face_at(&mut i, dangling_open).is_some(),
        "the dangling \"(\" itself is still a real opener token and still gets colored \
         (only closers get guarded against a stack/kind mismatch)"
    );

    // g is a fresh top-level sibling: its own { and ( must read exactly
    // like f's did, both back at depth-1 -- "( is normal" per the review,
    // i.e. no leaked offset from f's unrelated trouble.
    assert_eq!(
        rainbow_face_at(&mut i, g_params_open).as_deref(),
        Some("rainbow-delimiters-depth-1-face"),
        "g's own parameter-list paren must be depth-1, same as f's, not depth-2"
    );
    assert_eq!(
        rainbow_face_at(&mut i, g_params_close),
        rainbow_face_at(&mut i, g_params_open)
    );
    let g_brace_open_face = rainbow_face_at(&mut i, g_brace_open);
    assert_eq!(g_brace_open_face, f_brace_open_face);
    assert_eq!(
        rainbow_face_at(&mut i, g_brace_close),
        g_brace_open_face,
        "g's own {{ and }} must share one color too"
    );
}

#[test]
fn stray_closer_with_no_matching_opener_is_left_uncolored_and_does_not_shift_later_siblings() {
    // No error-recovery subtlety here -- a plain over-closed buffer, the
    // guard case the ORIGINAL linear counter already handled correctly
    // (`depth > 0` was false) and the new sibling-matching stack covers
    // the same way (`open` is empty, so nothing can be a "top match").
    // Pinned as its own test since no such case previously existed in
    // this file despite the doc comment always having claimed the
    // behavior.
    let src = "(a)) (b)\n";
    let (mut i, _ed) = setup(src);
    run(&mut i, "(emacs-lisp-mode)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let stray_close = pos_of(src, "(a))") + 3; // the SECOND ")" -- over-closes
    assert_eq!(
        rainbow_face_at(&mut i, stray_close),
        None,
        "a closer with nothing left open must not be colored"
    );

    let second_open = pos_of(src, "(b)");
    let second_close = second_open + 2;
    assert_eq!(
        rainbow_face_at(&mut i, second_open).as_deref(),
        Some("rainbow-delimiters-depth-1-face"),
        "the stray over-close must not shift a later, unrelated pair's depth"
    );
    assert_eq!(
        rainbow_face_at(&mut i, second_close),
        rainbow_face_at(&mut i, second_open)
    );
}
