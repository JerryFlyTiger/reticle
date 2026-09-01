# M71's *Help* pass. The main list and header explanation are in
# dev/mutations/m71.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-help.py -p core \
#         --test-target help_tests

PACKAGE = "core"
TEST_TARGET = "help_tests"

MUTATIONS = [
    {
        "label": "M5 revert the clearing in describe-bindings",
        "file": "crates/core/lisp/simple.el",
        "old": "      (set-buffer-modified-p nil)\n      (set-buffer-read-only t))))",
        "new": "      (set-buffer-read-only t))))",
        "expect_fail": ["describe_bindings_help_buffer_is_not_marked_modified"],
        "note": "M67 explicitly deferred this; M71 is what finally closed it.",
    },
]
