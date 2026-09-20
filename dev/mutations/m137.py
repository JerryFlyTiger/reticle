# Mutation list for M137: point-follow (`ensure_point_visible` and its
# helpers in crates/core/src/redisplay.rs) now counts the inline-diagnostic
# block rows (M87 stage 3) that `emit_block_rows` draws under a line. Before
# it, `M->` in the GUI left point off-screen by exactly the number of block
# rows between window start and point, with the cursor drawn clamped on the
# last row (screenshot, 2026-09-13, `demo/rtl/top/soc_top.sv` with 9 rows
# under line 117).
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m137.py
#
# Designed by the reviewer that cold-read the diff; M2's test was added by
# the main conversation afterwards because the reviewer showed M2 would
# survive the implementer's four tests (single-line messages, for which
# "number of diagnostics" and "rows drawn" coincide). Run from the main
# conversation, never by the implementer.

PACKAGE = "core"
TEST_TARGET = "inline_diagnostics_tests"

MUTATIONS = [
    {
        "label": "M1 whole effect: the count never reaches the scan",
        "file": "crates/core/src/redisplay.rs",
        "old": "row += 1 + block_rows.get(&line0).copied().unwrap_or(0);",
        "new": "row += 1;",
        "test": "point_at_end_of_buffer_stays_visible_below_a_diagnostic_block",
        "note": "This is the pre-M137 behaviour verbatim: block rows are drawn but never counted.",
    },
    {
        "label": "M2 count desynchronised from the draw (diagnostics, not rows)",
        "file": "crates/core/src/redisplay.rs",
        "old": "let n = block_row_lines(diags).len();",
        "new": "let n = diags.len();",
        "test": "point_follow_counts_drawn_block_rows_not_diagnostics",
        "note": (
            "A blank interior line is skipped and a 4-line message is capped "
            "at 3 rows, so 2 diagnostics draw 5 rows; counting 2 leaves point "
            "off-screen by 3."
        ),
    },
    {
        "label": "M3 next_row_start_ex never reports a newline",
        "file": "crates/core/src/redisplay.rs",
        "old": "return Some((p + 1, true));",
        "new": "return Some((p + 1, false));",
        "test": "far_jump_recenter_accounts_for_block_rows",
    },
    {
        "label": "M4 line0 bookkeeping frozen (every line looks like line 0)",
        "file": "crates/core/src/redisplay.rs",
        "old": "line0 += 1;",
        "new": "let _ = line0;",
        "test": "point_at_end_of_buffer_stays_visible_below_a_diagnostic_block",
    },
    {
        "label": "M5 recenter path sees an empty map",
        "file": "crates/core/src/redisplay.rs",
        "old": "let half = (text_rows / 2).max(1);",
        "new": "let half = (text_rows / 2).max(1);\n    let block_rows = &std::collections::HashMap::new();",
        "test": "far_jump_recenter_accounts_for_block_rows",
        "note": "Shadows recenter's own parameter; the smooth path in ensure_point_visible is untouched.",
    },
    {
        "label": "M6 the old test without its point-min move",
        "file": "crates/core/tests/inline_diagnostics_tests.rs",
        "old": '''    // of text_rows, not out of nowhere.
    run(&mut i, "(goto-char (point-min))");
''',
        "new": '''    // of text_rows, not out of nowhere.
''',
        "test": "window_shows_exactly_one_fewer_line_of_text_and_grid_rows_is_unchanged",
        "note": (
            "With point left on the last line, the fixed scan correctly pulls "
            "window start down to keep it visible, so the test's own 'last "
            "line scrolled off' assertion fails: the edit is load-bearing."
        ),
    },
]
