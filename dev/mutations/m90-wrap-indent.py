# M90 (continuation-line wrap indentation) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m90-wrap-indent.py \
#         -p core --test-target indent_tests
#
# L1 and L5 are the pair that matter: they are the two different ways a future
# change could quietly fold the wrap step back into the block multiply, which
# is the exact mistake the measured verible behaviour rules out. Both must be
# caught by the width-8 test, which is the milestone's load-bearing assertion.
#
# The `(and depth wrap-types ...)` short-circuit that keeps the other five
# languages doing no extra work is deliberately NOT listed: with `wrap-types`
# nil, removing the guard still computes zero, so no test can distinguish the
# two. That is recorded in the function's own docstring rather than faked here.

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    {
        "label": "L1 the wrap step becomes the block step (4 -> 2)",
        "file": "crates/core/lisp/indent.el",
        "old": "(defvar indent-wrap-width 4",
        "new": "(defvar indent-wrap-width 2",
        "test": "verilog_wrapped_port_list_continuation_lands_at_wrap_step_not_block_step",
    },
    {
        "label": "L2 the wrap step is read from the block width instead of its own variable",
        "file": "crates/core/lisp/indent.el",
        "old": "           (* (nth 2 r) indent-wrap-width))))))",
        "new": "           (* (nth 2 r) standard-indent-width))))))",
        "test": "verilog_wrapped_port_list_continuation_wrap_step_is_decoupled_from_block_width",
        "note": (
            "This is the mutation the milestone exists to prevent. With the "
            "buffer's indent width at 8 the decoupling test expects 12; "
            "reading the block width instead gives 16."
        ),
    },
    {
        "label": "L3 instantiation port lists stop getting a wrap step",
        "file": "crates/core/lisp/indent.el",
        "old": '  \'((verilog . ("list_of_port_connections" "list_of_parameter_value_assignments"',
        "new": '  \'((verilog . ("list_of_parameter_value_assignments"',
        "test": "verilog_wrapped_port_list_continuation_lands_at_wrap_step_not_block_step",
    },
    {
        "label": "L4 argument lists stop getting a wrap step",
        "file": "crates/core/lisp/indent.el",
        "old": '                "list_of_arguments")))',
        "new": '                )))',
        "test": "verilog_wrapped_call_argument_list_continuation_gets_a_wrap_step",
    },
]
