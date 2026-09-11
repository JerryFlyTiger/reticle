# M127 (leading-comment-before-a-wrap-node indentation fix) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m127-indent.py \
#         -p core --test-target indent_tests
#
# The fix adds one new adjustment, `indent--verilog-comment-before-wrap-
# node-adjust' (indent.el), threaded into `indent--query-pos-and-depth' via
# a new WRAP-DEPTH-ADJUST-FN parameter, wired up only for
# `verilog-indent-line'. Every entry below targets that adjustment, its
# helper `indent--treesit-next-sibling', or the wiring between them; none
# of these were run through `dev/mutate.py' by the implementer (that tool
# is reserved for the main conversation, per this project's own delegation
# rule).

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    {
        # Deletion-style entry: removes the WHOLE effect of the fix (the
        # adjustment never applies to anything), not a boundary
        # perturbation. All three "leading comment gets a wrap step"
        # tests go red; naming the port-connections one, since it is the
        # exact repro from the M127 spec.
        "label": "D1 the comment-before-wrap-node adjustment never fires at all",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "  (if (and (member (treesit-node-type node) '(\"one_line_comment\" \"block_comment\"))\n"
            "           (let ((next (indent--verilog-comment-after-skipping-comments node)))\n"
            "             (and next\n"
            "                  (member (treesit-node-type next)\n"
            "                          indent--verilog-comment-wrap-sibling-types))))\n"
            "      1\n"
            "    0))"
        ),
        "new": "  (ignore node)\n  0)",
        "test": "verilog_leading_comment_inside_wrapped_port_connections_gets_wrap_step",
    },
    {
        # Boundary entry: the must-not-fire case. Widening the sibling-
        # type set to include ordinary statement/instantiation wrapper
        # types would make the "comment above the whole instantiation"
        # case wrongly gain a wrap step too. This is the obvious way to
        # get this fix wrong (spec's own warning), so it gets its own
        # entry rather than relying on the deletion entry above to also
        # catch it.
        "label": "D2 the sibling-type set is widened to also match module_instantiation",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "(defvar indent--verilog-comment-wrap-sibling-types\n"
            "  '(\"list_of_port_connections\" \"list_of_parameter_value_assignments\"\n"
            "    \"list_of_arguments\")"
        ),
        "new": (
            "(defvar indent--verilog-comment-wrap-sibling-types\n"
            "  '(\"list_of_port_connections\" \"list_of_parameter_value_assignments\"\n"
            "    \"list_of_arguments\" \"module_instantiation\")"
        ),
        "test": "verilog_comment_above_whole_instantiation_does_not_get_a_wrap_step",
        "note": (
            "Widening the type set to also match `module_instantiation' makes a comment "
            "sitting ABOVE an entire instantiation (whose own next sibling by index really "
            "is `module_instantiation') wrongly gain a wrap step too, moving it from column "
            "2 to column 6 -- the exact must-not-fire shape the M127 spec calls out as the "
            "obvious way to get this fix wrong."
        ),
    },
    {
        # The negative control: the ANSI header wrap types must NEVER be
        # added to the sibling-type set (a header's leading comment is
        # already a genuine descendant of the list node, so adding a step
        # here would double it). This entry adds `list_of_port_
        # declarations' to prove the negative-control test actually
        # notices the double-count.
        "label": "D3 list_of_port_declarations wrongly added to the sibling-type set (double-counts the ANSI header)",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "  '(\"list_of_port_connections\" \"list_of_parameter_value_assignments\"\n"
            "    \"list_of_arguments\")"
        ),
        "new": (
            "  '(\"list_of_port_connections\" \"list_of_parameter_value_assignments\"\n"
            "    \"list_of_arguments\" \"list_of_port_declarations\")"
        ),
        "test": "verilog_leading_comment_inside_ansi_header_port_list_is_unaffected",
        "expect": "SURVIVED",
        "note": (
            "DECLARED SURVIVOR, and the reason is the interesting part. This entry was "
            "written expecting a FAIL, and it SURVIVED when the main conversation ran it "
            "on 2026-09-09. The mutation does land (the runner enforces a unique match), "
            "but it is semantically inert: the adjustment fires only when a comment's own "
            "NEXT SIBLING is a member of this set, and an ANSI header's leading comment is "
            "a genuine CHILD of `list_of_port_declarations' rather than a sibling of it -- "
            "so no comment anywhere ever has that node as its next sibling, and adding it "
            "to the set changes nothing. The same argument holds for the other three "
            "excluded header types. Consequence worth stating plainly: the four negative-"
            "control tests do guard the general path against a regression, but they CANNOT "
            "detect a wrongly-widened sibling-type set -- that exclusion is defensive and "
            "unfalsifiable, and no test can be written that would kill this mutation."
        ),
    },
    {
        # Helper-function entry: if `indent--treesit-next-sibling' always
        # returns nil (as if there were never a next sibling), the whole
        # adjustment can never fire for any of the three wrap types --
        # same observable failure as D1, but targets the helper
        # specifically rather than the adjust function's own membership
        # check.
        "label": "D4 indent--treesit-next-sibling always returns nil",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "(defun indent--treesit-next-sibling (node)\n"
            "  \"NODE's immediate next sibling by child index within its own PARENT, or"
        ),
        "new": (
            "(defun indent--treesit-next-sibling (node)\n"
            "  (ignore node)\n"
            "  nil)\n"
            "(defun indent--treesit-next-sibling--unused (node)\n"
            "  \"NODE's immediate next sibling by child index within its own PARENT, or"
        ),
        "test": "verilog_leading_comment_inside_wrapped_argument_list_gets_wrap_step",
    },
    {
        # Wiring entry: if WRAP-DEPTH-ADJUST-FN is never passed from
        # `verilog-indent-line', the adjustment function itself is
        # correct but never invoked -- a different failure point than D1
        # (which breaks the function's own logic) or D4 (which breaks its
        # helper), catching a regression where a future edit to
        # `verilog-indent-line' silently drops the argument.
        "label": "D5 verilog-indent-line stops passing the wrap-depth-adjust-fn argument",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "  (or (indent--treesit-depth-column\n"
            "       'verilog indent--verilog-closers #'indent--verilog-depth-adjust\n"
            "       #'indent--verilog-comment-before-wrap-node-adjust)\n"
            "      (indent--copy-previous-indentation)))"
        ),
        "new": (
            "  (or (indent--treesit-depth-column\n"
            "       'verilog indent--verilog-closers #'indent--verilog-depth-adjust)\n"
            "      (indent--copy-previous-indentation)))"
        ),
        "test": "verilog_leading_comment_inside_wrapped_parameter_value_assignment_list_gets_wrap_step",
    },
    {
        # Fix-round entry (trailing review finding 1): deletion-style for
        # the RUN-of-comments handling specifically -- reverts
        # `indent--verilog-comment-after-skipping-comments' to the
        # pre-fix-round behaviour (checking only the IMMEDIATE next
        # sibling, never skipping past a further comment). A single
        # leading comment is unaffected (still correctly the LAST
        # comment's own case), but the FIRST of two or more consecutive
        # leading comments regresses to block depth. Hand-verified during
        # this fix round (file backup + targeted Edit + `touch', reverted
        # after): left "2", right "6".
        "label": "D6 the comment-skipping walk is reverted to immediate-sibling-only (breaks multi-comment runs)",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "  (let ((n (indent--treesit-next-sibling node)))\n"
            "    (while (and n (member (treesit-node-type n) '(\"one_line_comment\" \"block_comment\")))\n"
            "      (setq n (indent--treesit-next-sibling n)))\n"
            "    n))"
        ),
        "new": "  (indent--treesit-next-sibling node))",
        "test": "verilog_two_leading_comments_inside_wrapped_port_connections_both_get_wrap_step",
        "note": (
            "Hand-verified during the fix round (file backup + Edit + touch, reverted after): "
            "with this mutation, the FIRST of the test's two leading comments computes 2 "
            "instead of 6; the SECOND (whose own immediate next sibling already is the wrap "
            "node) is unaffected."
        ),
    },
    {
        # Fix-round entry (reviewer's item 5): the INTERNAL forwarding
        # call inside `indent--treesit-depth-column' to
        # `indent--query-pos-and-depth' is a distinct break point from D5
        # above -- D5 mutates the OUTER call site in
        # `verilog-indent-line' (which never even passes a
        # WRAP-DEPTH-ADJUST-FN argument to `indent--treesit-depth-column'
        # in the first place); this one mutates the plumbing one layer
        # further in, dropping the 6th argument on the way to
        # `indent--query-pos-and-depth' even though `verilog-indent-line'
        # still passes it correctly to `indent--treesit-depth-column'.
        "label": "D7 indent--treesit-depth-column stops forwarding wrap-depth-adjust-fn to indent--query-pos-and-depth",
        "file": "crates/core/lisp/indent.el",
        "old": (
            "           (r (indent--query-pos-and-depth\n"
            "               lang-sym block-types closers wrap-types depth-adjust-fn\n"
            "               wrap-depth-adjust-fn)))"
        ),
        "new": (
            "           (r (indent--query-pos-and-depth\n"
            "               lang-sym block-types closers wrap-types depth-adjust-fn)))"
        ),
        "test": "verilog_leading_comment_inside_wrapped_port_connections_gets_wrap_step",
    },
]
