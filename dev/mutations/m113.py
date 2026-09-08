# Mutation list for M113 (matching-bracket highlight / show-paren-mode).
# Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m113.py \
#         -p core --test-target show_paren_tests
#
# One entry overrides the test target: the face-separation bar lives in
# gui_features_tests.rs because it is a property of the themes rather than of
# the highlight logic. The per-entry override exists for exactly this shape of
# list (added during M109, when U7's guard likewise lived in another binary).
#
# One entry is a declared expected survivor. It is the symmetric half of the
# MISSING-node guard: tree-sitter was observed to recover a stray closer by
# wrapping it in an ERROR node rather than synthesising a MISSING opener, in
# every grammar anyone tested (elisp, C, Rust, Verilog), so the opener-side
# guard is defensive and no fixture in the suite reaches it. Recorded rather
# than left looking covered.
#
# M117 appended P11-P13. P11 and P12 are wires nothing was watching: the
# closer-side half of the adjacency predicate (the opener-side half had a test,
# the closer-side half did not), and which of the paren highlight and the region
# wins where they overlap -- a precedence that no test anywhere in the repo
# pinned, so a refactor could have flipped it silently. P13 is the deletion
# entry this project now requires of every mutation list: it removes the
# feature's whole effect rather than perturbing a boundary inside it.

PACKAGE = "core"
TEST_TARGET = "show_paren_tests"

MUTATIONS = [
    {
        "label": "P1 the staleness guard is gone (paints on text that no longer exists)",
        "file": "crates/core/src/highlight.rs",
        "old": "        if *gen != buffer.borrow().edit_ticks {",
        "new": "        if false {",
        "test": "stale_cache_after_an_edit_elsewhere_yields_no_highlight_until_reparse_lands",
    },
    {
        "label": "P2 the after-a-closer adjacency loses its offset",
        "file": "crates/core/src/highlight.rs",
        "old": ".find(|&(open, close)| open == point || close + 1 == point)",
        "new": ".find(|&(open, close)| open == point || close == point)",
        "test": "point_after_the_closer_highlights_both_members",
    },
    {
        "label": "P3 the before-an-opener adjacency gains one",
        "file": "crates/core/src/highlight.rs",
        "old": ".find(|&(open, close)| open == point || close + 1 == point)\n",
        "new": ".find(|&(open, close)| open + 1 == point || close + 1 == point)\n",
        "test": "point_before_the_opener_highlights_both_members",
    },
    {
        "label": "P4 the tie-break picks the later pair instead of the earlier",
        "file": "crates/core/src/highlight.rs",
        "old": "            .find(|&(open, close)| open == point || close + 1 == point)",
        "new": "            .rev()\n            .find(|&(open, close)| open == point || close + 1 == point)",
        "test": "tie_break_at_a_close_open_boundary_picks_the_earlier_closing_pair",
    },
    {
        "label": "P5 a MISSING closer produces a phantom pair again",
        "file": "crates/core/src/highlight.rs",
        "old": "                        if !node.is_missing() && !open_missing {",
        "new": "                        if !open_missing {",
        "test": "unmatched_bracket_highlights_nothing",
    },
    {
        # Declared expected survivor -- see the header note.
        "expect": "survived",
        "label": "P6 the symmetric opener guard is removed (expected to SURVIVE)",
        "file": "crates/core/src/highlight.rs",
        "old": "                        if !node.is_missing() && !open_missing {\n",
        "new": "                        if !node.is_missing() {\n",
        "test": "unmatched_bracket_highlights_nothing",
    },
    {
        "label": "P7 a background window draws the highlight from its own saved point",
        "file": "crates/core/src/redisplay.rs",
        "old": "if is_selected && var_on(interp, \"show-paren-mode\") {",
        "new": "if var_on(interp, \"show-paren-mode\") || is_selected {",
        "test": "non_selected_window_showing_the_same_buffer_and_point_gets_no_highlight",
    },
    {
        "label": "P8 the mode toggle is ignored",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let paren_pair: Option<(usize, usize)> = if is_selected && var_on(interp, \"show-paren-mode\") {",
        "new": "    let paren_pair: Option<(usize, usize)> = if is_selected {",
        "test": "mode_off_produces_no_highlight",
    },
    {
        "label": "P9 the mode ships off by default",
        "file": "crates/core/lisp/simple.el",
        "old": "(defvar show-paren-mode t",
        "new": "(defvar show-paren-mode nil",
        "test": "show_paren_mode_is_on_by_default",
    },
    {
        "label": "P10 light's paren highlight sinks back into its current-line colour",
        "file": "crates/core/lisp/themes.el",
        "old": "(set-face 'show-paren-match :background \"#c8a471\")",
        "new": "(set-face 'show-paren-match :background \"#e9dccc\")",
        "test": "show_paren_match_separates_from_region_and_hl_line",
        "test_target": "gui_features_tests",
    },
    {
        "label": "P11 the closer-side adjacency case starts matching too",
        "file": "crates/core/src/highlight.rs",
        "old": "            .find(|&(open, close)| open == point || close + 1 == point)",
        "new": "            .find(|&(open, close)| open == point || close + 1 == point || close == point)",
        "test": "point_on_the_closer_itself_highlights_nothing",
    },
    {
        "label": "P12 the paren highlight starts beating the region where they overlap",
        "file": "crates/core/src/redisplay.rs",
        "old": "                style.bg = region_bg.or(style.bg);",
        "new": "                style.bg = style.bg.or(region_bg);",
        "test": "region_wins_over_the_paren_highlight_where_they_overlap",
    },
    {
        "label": "P13 DELETION: the highlight is never written to any cell",
        "file": "crates/core/src/redisplay.rs",
        "old": """        if let (Some((op, cp)), Some(bg)) = (paren_pair, paren_bg) {
            if pos == op || pos == cp {
                style.bg = Some(bg);
            }
        }""",
        "new": """        if let (Some((op, cp)), Some(bg)) = (paren_pair, paren_bg) {
            let _ = (op, cp, bg);
        }""",
        "test": "point_before_the_opener_highlights_both_members",
    },
]
