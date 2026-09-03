# M92 (AUTO_TEMPLATE for AUTOINST) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m92-auto-template.py \
#         -p core --test-target verilog_auto_tests
#
# Q1 is the one that matters most. The trailing cold read found that the
# regex-captured EXPR backtracked to the rightmost `)' on the line, so a
# comment containing a paren folded the separator and half the comment into
# the connection -- a WRONG connection, silently, which is worse than the
# dropped rule the round before it. The fix replaced the capture with a
# depth-counting balance scan, and this mutation puts the old behaviour back.
#
# Worth recording next to it: the implementer's own first draft of those tests
# used substring assertions, which passed against the very defect they were
# written for -- the corrupted output still CONTAINS `(finished)'. That was
# caught by running the mutation and watching it stay green, then tightened to
# require exact line termination. A mutation that survives is not always a
# missing test; sometimes it is a test that cannot see.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "Q1 EXPR is taken to the last paren on the line instead of the balanced one",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(defun verilog-auto--balanced-paren-end (s open)",
        "new": "(defun verilog-auto--balanced-paren-end-unused (s open)",
        "test": "autotemplate_exact_trailing_comment_containing_a_paren_does_not_corrupt_expr",
        "note": (
            "Renaming the definition makes every call site hit a void "
            "function, which is a blunt instrument but a genuine one: the "
            "balance scan is the fix, and nothing else provides it. A "
            "subtler replacement would have to reimplement the old greedy "
            "capture inline, which is a rewrite rather than a mutation."
        ),
    },
    {
        "label": "Q2 exact rules stop winning over wildcard rules",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "    (if exact\n        exact\n",
        "new": "    (if nil\n        exact\n",
        "test": "autotemplate_exact_wins_over_wildcard_for_same_port",
    },
    {
        "label": "Q3 wildcard patterns stop being anchored (substring matches leak in)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                        (push (cons (concat "^" name "$") expr) wild))',
        "new": "                        (push (cons name expr) wild))",
        "test": "autotemplate_wildcard_anchoring_prevents_substring_match",
    },
    {
        "label": "Q4 template lookup prefers a following template over a preceding one",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "    (or before after)))",
        "new": "    (or after before)))",
        "test": "autotemplate_backward_lookup_wins_when_templates_appear_both_sides",
    },
    {
        "label": "Q5 AUTOWIRE goes back to seeing only the first instance of a comma-separated declaration",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '             (hiers (verilog-auto--find-all-of-type mi "hierarchical_instance")))',
        "new": '             (hiers (list (verilog-auto--find-first-of-type mi "hierarchical_instance"))))',
        "test": "autowire_multi_instance_comma_form_sees_every_instance",
    },
    {
        "label": "Q6 a template whose body cannot be extracted goes back to failing silently",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '          (push (format "AUTO_TEMPLATE for module %s: comment found but its own body could not be extracted',
        "new": '          (ignore (format "AUTO_TEMPLATE for module %s: comment found but its own body could not be extracted',
        "test": "autotemplate_nested_block_comment_truncation_is_reported_not_silent",
    },
    {
        "label": "Q7 parse warnings carry over from one verilog-auto run into the next",
        "file": "crates/core/lisp/verilog-auto.el",
        # Rebinding it to its own value still creates a FRESH let binding, so
        # the outer value is untouched either way and the mutation survived.
        # Renaming the binding is what actually leaves the real variable
        # global, so pushes accumulate across runs.
        "old": "          (verilog-auto--template-parse-warnings nil)",
        "new": "          (verilog-auto--template-parse-warnings-unbound nil)",
        "test": "autotemplate_parse_warnings_reset_across_two_verilog_auto_runs",
    },
]
