# M94 mutation list -- the autostart half, which runs against a different target.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m94-multi-client-autostart.py \
#         -p core --test-target lsp_autostart_tests
#
# The core half lives in dev/mutations/m94-multi-client.py.

PACKAGE = "core"
TEST_TARGET = "lsp_autostart_tests"

MUTATIONS = [
    {
        "label": "AC1 the pending marker goes back to a single slot (the secondary overwrites the primary)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                              (cons key lsp--autostart-pending-here)))",
        "new": "                              (list key)))",
        "test": "pending_marker_holds_both_keys_and_only_fully_clears_once_both_complete",
        "note": (
            "Restores the defect the trailing review found: with verilog-mode's "
            "own default config both handshakes start in one tick, so the "
            "secondary's key overwrote the primary's, and a secondary that "
            "completed first cleared the marker while the primary was still "
            "in flight -- the mode line then showed neither state."
        ),
    },
]
