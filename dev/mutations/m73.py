# M73 -- indent width follows the file (detecting `standard-indent-width`
# from file content).
#
# Source of this list: the reviewer designed 7 entries (M1-M7) from a cold
# read of the diff; the main conversation added M8-M10, which guard **product
# code that only grew during the fix-up round** (skipping CRLF blank lines,
# the `integerp` check, and the fixture added for the tab-lines branch the
# reviewer flagged as "currently zero coverage") -- those three didn't exist
# yet when the reviewer read the diff.
#
# How to run (two passes, since the guards span two test targets):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m73.py -p core \
#         --test-target indent_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m73-demo.py -p core \
#         --test-target demo_smoke_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans.
#
# ## Two tests "deliberately meant to survive", honestly recorded here and
#    not put on the list
#
# `find_file_four_space_sv_sets_width_to_four` and
# `find_file_tab_indented_sv_keeps_mode_default` **survive** M1 (removing the
# integration call), and this is by design: verilog-mode's default is
# already 4, so "detected as 4" and "fell back to the default 4" are
# indistinguishable from the `standard-indent-width` observation point. The
# reviewer originally flagged them as fake tests; the fix-up round's
# resolution was to keep them but add a direct assertion on
# `(indent--detect-width)` to each, letting them at least distinguish the two
# states, and to write into the test comments that **they are not a signal
# for the integration point**. The integration point is guarded by the four
# entries listed under M1. **Do not design a mutation expecting FAIL for
# these.**

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    {
        "label": "M1 remove the integration call (the detection result never gets applied to the buffer)",
        "file": "crates/core/lisp/modes.el",
        "old": "  (indent--maybe-detect-width)\n  (run-hooks 'prog-mode-hook)",
        "new": "  (run-hooks 'prog-mode-hook)",
        "expect_fail": [
            "find_file_two_space_sv_opens_new_lines_at_detected_width",
            "find_file_two_buffers_keep_independent_widths",
            "manually_switching_to_verilog_mode_reruns_detection",
            "user_hook_setq_local_wins_over_detection",
        ],
        "note": "The wiring for the whole milestone. All four expected "
                "values are different from the mode default of 4, so all "
                "should go red.",
    },
    {
        "label": "M2 the `indent-detect-width` switch stops working (always detects, cannot be turned off)",
        "file": "crates/core/lisp/indent.el",
        "old": "  (when indent-detect-width\n    (let ((width (indent--detect-width)))",
        "new": "  (when t\n    (let ((width (indent--detect-width)))",
        "expect_fail": ["find_file_two_space_sv_with_detection_disabled_keeps_mode_default"],
        "note": "The user's opt-out.",
    },
    {
        "label": "M3 detection moved to after run-hooks (overwrites the user hook's explicit setting)",
        "file": "crates/core/lisp/modes.el",
        "old": "  (indent--maybe-detect-width)\n  (run-hooks 'prog-mode-hook)\n  (run-hooks hook))",
        "new": "  (run-hooks 'prog-mode-hook)\n  (run-hooks hook)\n  (indent--maybe-detect-width))",
        "expect_fail": ["user_hook_setq_local_wins_over_detection"],
        "note": "Priority order: an explicit setting must win over a "
                "guess. Reversing the order makes it lose.",
    },
    {
        "label": "M4 the divisibility-rate threshold raised from 90% to 100%",
        "file": "crates/core/lisp/indent.el",
        "old": "                 (when (>= (* 10 ok) (* 9 total))",
        "new": "                 (when (>= (* 10 ok) (* 10 total))",
        "expect_fail": ["detect_width_one_odd_line_among_nine_still_detects_two"],
        "note": "**This entry was SURVIVED on the first run**, and the "
                "reason the reviewer gave was wrong: it originally pointed "
                "at `detect_width_soc_top_shape...`, claiming that fixture "
                "deliberately makes only 8/11 samples divisible by 2 -- "
                "in reality that fixture's samples are all even (2 divides "
                "100% of them), and what it guards is candidate ordering "
                "and \"4 does not divide 2\", not the threshold. The 90% "
                "constant had zero coverage at the time. The main "
                "conversation therefore added "
                "`..._one_odd_line_among_nine_still_detects_two` (9/10, "
                "sitting exactly on the threshold) and "
                "`..._two_odd_lines_among_ten_declines` (8/10, bracketing "
                "it from below), only after which this entry became FAIL.",
    },
    {
        "label": "M5 candidate width order reversed (narrowest tried first)",
        "file": "crates/core/lisp/indent.el",
        "old": "(defconst indent--detect-width-candidates '(8 4 3 2)",
        "new": "(defconst indent--detect-width-candidates '(2 3 4 8)",
        "expect_fail": ["detect_width_eight_space_content"],
        "note": "For 8-space content, 2/4/8 all divide it, so order decides the answer.",
    },
    {
        "label": "M6 remove the range check from `set-indent-width`",
        "file": "crates/core/lisp/indent.el",
        "old": "    ((or (< width 1) (> width 16))\n     (error \"Indent width out of range (1..16): %d\" width))",
        "new": "    ((and nil (or (< width 1) (> width 16)))\n     (error \"Indent width out of range (1..16): %d\" width))",
        "expect_fail": ["set_indent_width_changes_value_and_rejects_out_of_range"],
    },
    {
        "label": "M7 the minimum sample count lowered from 5 to 3",
        "file": "crates/core/lisp/indent.el",
        "old": "          ((< total indent--detect-min-samples) nil)",
        "new": "          ((< total 3) nil)",
        "expect_fail": ["detect_width_fewer_than_min_samples_returns_nil"],
        "note": "That fixture has 4 samples: 4<5 is nil (returns nil), "
                "while 4<3 is false, so it would compute an answer.",
    },
    # --- The following three entries guard product code from the fix-up
    #     round; they did not exist when the reviewer read the diff ---
    {
        "label": "M8 (fix-up round) CRLF blank lines no longer skipped",
        "file": "crates/core/lisp/indent.el",
        "old": "          (while (and (< rest eol) (eq (char-after rest) ?\\r))",
        "new": "          (while (and (< rest eol) (eq (char-after rest) ?\\a))",
        "expect_fail": ["detect_width_crlf_blank_line_with_trailing_spaces_matches_lf_twin"],
        "note": "The main conversation reproduced this in a real TUI: a "
                "new line in the CRLF version of alu.sv falls back to "
                "column 4.",
    },
    {
        "label": "M9 (fix-up round) remove the integerp check from `set-indent-width`",
        "file": "crates/core/lisp/indent.el",
        "old": "    ((not (integerp width))\n     (error \"Indent width must be an integer: %S\" width))",
        "new": "    ((and nil (not (integerp width)))\n     (error \"Indent width must be an integer: %S\" width))",
        "expect_fail": ["set_indent_width_rejects_a_float"],
        "note": "A float would pass the range check, and every indent "
                "afterward would then hit wrong-type-argument integerp.",
    },
    {
        "label": "M10 (fix-up round) the tab-lines-majority branch stops working",
        "file": "crates/core/lisp/indent.el",
        "old": "          ((> tab-lines total) nil)",
        "new": "          ((> tab-lines (* 1000 total)) nil)",
        "expect_fail": ["detect_width_tab_lines_outnumbering_space_samples_returns_nil"],
        "note": "The reviewer pointed out the old fixture made this entry "
                "and min-samples both hold at once, so the failure "
                "couldn't be attributed; the fixture added by the fix-up "
                "round (6 tab lines + 5 2-space lines) can only be "
                "explained by this branch returning nil.",
    },
    {
        "label": "M11 (trailing re-review) blank-line detection uses the position before the CR-skip",
        "file": "crates/core/lisp/indent.el",
        "old": "          (unless (= rest eol) ; blank (incl. CRLF-blank) line -> skip",
        "new": "          (unless (= end eol) ; blank (incl. CRLF-blank) line -> skip",
        "expect_fail": ["detect_width_crlf_blank_line_with_trailing_spaces_matches_lf_twin"],
        "note": "The trailing reviewer points out that this entry hits "
                "the comparison line actually changed by the fix more "
                "directly than M8 (`?\\r` to `?\\a`); the two have "
                "different attack surfaces, both are kept.",
    },
    {
        "label": "M12 (trailing re-review) the threshold's >= changed to > (boundary value no longer counts as passing)",
        "file": "crates/core/lisp/indent.el",
        "old": "                 (when (>= (* 10 ok) (* 9 total))",
        "new": "                 (when (> (* 10 ok) (* 9 total))",
        "expect_fail": ["detect_width_one_odd_line_among_nine_still_detects_two"],
        "note": "A different attack surface from M4 (changing the "
                "constant 9->10): this entry verifies the operator "
                "itself, and that fixture happens to sit exactly on the "
                "90 = 90 boundary.",
    },
]
