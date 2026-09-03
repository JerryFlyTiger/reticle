# Mutation list for the demo fix (`3caac40'), designed by the reviewer that
# cold-read that commit AFTER it was already committed and pushed.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m99-demo.py \
#         -p core --test-target demo_smoke_tests
#
# Context: adding an AUTO_TEMPLATE above `u_arbiter' put the literal strings
# `.req_ready_o', `.gnt_req_o' and the rest into the buffer as comment text,
# which silently turned nine whole-buffer substring assertions in
# `arbiter_autoinst_expands_then_deletes' into tautologies. The fix scopes
# every assertion to the instantiation's own port list. These two mutations
# ask whether the scoped version actually detects a broken expansion.
#
# The reviewer also designed a third item that is deliberately NOT in this
# list: reverting the test's scoping (back to whole-buffer `expanded' /
# `deleted') is expected to SURVIVE, because that vacuousness is exactly what
# the commit exists to describe. Running it would produce a SURVIVED that
# means "correct", which is the reading this project has been burned by
# before -- so it stays documented here rather than executable.

PACKAGE = "core"
TEST_TARGET = "demo_smoke_tests"

MUTATIONS = [
    {
        "label": "M1 AUTOINST expands to nothing (the connection lines are never inserted)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        (when lines\n          (goto-char (treesit-node-end comment))\n          (insert \"\\n\" (string-join lines \"\\n\")))\n        1))))",
        "new": "        (when nil\n          (goto-char (treesit-node-end comment))\n          (insert \"\\n\" (string-join lines \"\\n\")))\n        1))))",
        "test": "arbiter_autoinst_expands_then_deletes",
        "note": (
            "Still returns 1, so the `verilog-auto: 1 inst,' echo assertion "
            "stays green and the failure has to come from the scoped port "
            "assertions -- which is the point."
        ),
    },
    {
        "label": "M2 the segment start marker no longer matches, so the slice guard must fire",
        "file": "crates/core/tests/demo_smoke_tests.rs",
        "old": "    let start_marker = \"u_arbiter (\";",
        "new": "    let start_marker = \"u_arbiterX (\";",
        "test": "arbiter_autoinst_expands_then_deletes",
    },
]
