# M74 -- decouple AUTOARG's continuation-line indent from the block's step
# (fixes a regression introduced by M73).
#
# Source of this list: the reviewer designed 3 entries from a cold read of
# the diff, all adopted and executed as-is by the main conversation.
#
# How to run (one pass is enough, all guards are in verilog_auto_tests):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m74.py -p core \
#         --test-target verilog_auto_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# The three fields `expect_fail` / `expect_pass` / `note` are **not read** by
# `mutate.py`, they're purely for humans.
#
# ## One entry the reviewer explicitly stated is "structurally unobservable",
#    honestly recorded here and not put on the list
#
# Swapping the order of the two arguments to
# `(concat (verilog-auto--line-indent ...) (make-string verilog-auto-wrap-width ?\s))`
# produces **byte-for-byte identical output** -- both sides are pure
# whitespace strings, so order doesn't affect content. This isn't a coverage
# gap, this shape is inherently unobservable. **Do not design a mutation for
# it.**

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "M1 wrap width reverted to reading standard-indent-width (fully reverts M73's regression)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                            (make-string verilog-auto-wrap-width ?\\s)))",
        "new": "                            (make-string standard-indent-width ?\\s)))",
        "expect_fail": [
            "autoarg_wrap_width_independent_of_detected_2_space_block_step",
            "autoarg_wrap_width_independent_of_detected_3_space_block_step",
            "autoarg_wrap_width_reads_verilog_auto_wrap_width_variable",
            "autoarg_wrap_width_adds_to_module_line_own_indent",
        ],
        "note": "The reviewer also pointed out that two **old** AUTOARG "
                "tests stay green under this mutation -- they go through "
                "insert and never trigger M73's detection, so "
                "standard-indent-width stays at 4, which coincidentally "
                "equals the wrap default. **The old tests are blind to "
                "this regression**, which is exactly why new tests were "
                "needed.",
    },
    {
        "label": "M2 wrap width default changed from 4 to 2",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(defvar verilog-auto-wrap-width 4",
        "new": "(defvar verilog-auto-wrap-width 2",
        "expect_fail": [
            "autoarg_wrap_width_independent_of_detected_2_space_block_step",
            "autoarg_wrap_width_independent_of_detected_3_space_block_step",
            "autoarg_wrap_width_adds_to_module_line_own_indent",
            "autoarg_nonansi_groups_and_no_trailing_comma",
        ],
        "note": "Deliberately used to confirm that \"default 4\" is pinned "
                "down by **both new and old tests together**, not just the "
                "new ones. `..._reads_verilog_auto_wrap_width_variable` "
                "sets itself to 8 via setq and is unaffected by the default.",
    },
    {
        "label": "M3 ignore the variable, hard-code 4 (the implementation no longer really reads that defvar)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                            (make-string verilog-auto-wrap-width ?\\s)))",
        "new": "                            (make-string 4 ?\\s)))",
        "expect_fail": ["autoarg_wrap_width_reads_verilog_auto_wrap_width_variable"],
        "note": "Confirms that test isn't redundant with the other three -- it's the only one guarding that the variable is genuinely wired up.",
    },
]
