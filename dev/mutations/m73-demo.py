# M73's second pass: the two guards for the demo showcase material run on
# the `demo_smoke_tests` target, and `mutate.py` only takes one target per
# run, so this is split into a separate list. See m73.py for the main list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m73-demo.py -p core \
#         --test-target demo_smoke_tests

PACKAGE = "core"
TEST_TARGET = "demo_smoke_tests"

MUTATIONS = [
    {
        "label": "D1 remove the integration call (the demo showcase material side)",
        "file": "crates/core/lisp/modes.el",
        "old": "  (indent--maybe-detect-width)\n  (run-hooks 'prog-mode-hook)",
        "new": "  (run-hooks 'prog-mode-hook)",
        "expect_fail": [
            "demo_rtl_verilog_files_detect_two_space_width_except_the_undersampled_svh",
            "opening_a_new_line_in_alu_sv_body_lands_at_column_two_matching_verible",
        ],
        "note": "These two guard the repo's own showcase claims: files under "
                "demo/rtl open with a step of 2, and opening a new line in "
                "alu.sv lands in the column verible expects.",
    },
]
