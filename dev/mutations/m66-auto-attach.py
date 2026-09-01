# M66's second pass: the entry that lands on the `lsp_auto_attach_tests` test
# file.
#
# The only reason this is split into two files is that `dev/mutate.py`'s
# `--test-target` applies to the whole config (`m60.py` / `m60-elisp.py` is
# the same precedent). See the header of `m66.py` for content and how to run.
#
#     dev/mutate.py --config dev/mutations/m66-auto-attach.py -p core \
#         --test-target lsp_auto_attach_tests
#
# This entry verifies that "after extracting the inline `string-prefix-p`
# into the shared predicate `lsp--remote-path-p`, the auto-attach side's
# behavior didn't drift along with it" -- a target for the refactor, not for
# new functionality.

PACKAGE = "core"
TEST_TARGET = "lsp_auto_attach_tests"

MUTATIONS = [
    {
        "label": "M4 auto-attach's remote guard returns t instead of nil",
        "file": "crates/core/lisp/lsp.el",
        "old": """   ((lsp--remote-path-p file) nil)""",
        "new": """   ((lsp--remote-path-p file) t)""",
        # The branch still short-circuits at the same place (so it won't
        # trigger lsp--project-root or issue real ssh calls), it's just that
        # the return value is wrong. The existing test expects "nil".
        "test": "auto_attach_client_rejects_a_remote_path_without_touching_any_buffer",
    },
]
