# Mutation list for the one M99 branch whose only guard lives outside the
# milestone's own test files.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m99-gui.py \
#         -p core --test-target gui_features_tests
#
# `lsp--diagnostics-for-uri' walks `lsp--effective-buffer-clients' and appends
# the publishing CLIENT if it isn't already there. That append looks dead in
# ordinary use -- a client attached by `M-x lsp' or autostart is always in the
# list -- and the only thing that exercises it is a pre-existing GUI test that
# publishes from a synthetic client never attached to the buffer. Kept in its
# own config file precisely so a main conversation running the other two lists
# cannot quietly skip it.

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "M10 a publishing client that was never attached is dropped from the walk",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (unless (memq client raw-clients)\n        (setq raw-clients (append raw-clients (list client))))",
        "new": "      (when nil\n        (setq raw-clients (append raw-clients (list client))))",
        "test": "lsp_diagnostics_decorate_the_visiting_buffer",
    },
]
