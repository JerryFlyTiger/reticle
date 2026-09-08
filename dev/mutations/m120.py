# Mutation list for M120 (statements in this repo that were false).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m120.py -p core
#
# **This milestone is almost entirely prose**, and prose has no deletion
# test. Six items (E1, E2, E4, E5, E6) corrected comments, docstrings, a
# lint-waiver rationale and a PLAN.md inventory; reverting any of them
# changes no observable behaviour, so there is nothing to mutate and no
# coverage gap implied by that. Recording the fact here rather than
# leaving the file absent, because an absent mutation file reads --
# per the rule M117 wrote into CLAUDE.md -- exactly like nobody having
# thought about it.
#
# E3 is the one behaviour change: `evil--halfpage' used the FRAME's
# height, justified by a docstring claiming a window's height was not
# exposed to elisp. It is (`window-height', builtins/ui.rs). With one
# window the two are nearly equal, which is why this was invisible; in a
# split, C-d in a 10-row window scrolled half the frame. G1 is its
# deletion entry.
#
# The guard against a false conclusion here is that the mutation restores
# the EXACT pre-M120 expression, so a green run would mean the new test
# cannot tell the old behaviour from the new one.

PACKAGE = "core"
TEST_TARGET = "evil_tests"

MUTATIONS = [
    {
        "label": "G1 the half-page count goes back to half the FRAME (deletion entry: E3)",
        "file": "crates/core/lisp/evil.el",
        "old": "    (max 1 (/ (if wh (max 1 (1- wh)) (frame-height)) 2))))",
        "new": "    (max 1 (/ (frame-height) 2))))",
        "test": "evil_scroll_down_uses_selected_windows_height_not_frames",
    },
]
