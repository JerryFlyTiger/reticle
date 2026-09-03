# M88 mutation list -- the mode-line half, which runs against a different target.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m88-autostart-lib.py \
#         -p core --test-target lib
#
# The integration half lives in dev/mutations/m88-autostart.py.

PACKAGE = "core"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "B1 (F2) the pending mode-line label is indistinguishable from the attached one",
        "file": "crates/core/src/redisplay.rs",
        "old": '        LspState::Pending => Some("LSP\u2026"),',
        "new": '        LspState::Pending => Some("LSP"),',
        "test": "u15_lsp_state_labels_and_width",
        "note": (
            "The cold read flagged this arm as unreachable from the suite: "
            "nothing rendered a pending mode line, so a wrong label or a "
            "wrong width was invisible. The fix round added the test this "
            "mutation now has something to fail."
        ),
    },
]
