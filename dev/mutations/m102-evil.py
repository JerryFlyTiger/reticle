# M102's evil half: the `C-w + - > < =` bindings live in evil.el and their
# tests are in evil_wave3_tests.rs, a different --test target.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m102-evil.py \
#         -p core --test-target evil_wave3_tests

PACKAGE = "core"
TEST_TARGET = "evil_wave3_tests"

MUTATIONS = [
    {
        "label": "R7 C-w + no longer enlarges",
        "file": "crates/core/lisp/evil.el",
        "old": "(defun evil-window-increase-height ()  ; C-w +\n  (interactive)\n  (enlarge-window))",
        "new": "(defun evil-window-increase-height ()  ; C-w +\n  (interactive)\n  (ignore))",
        "test": "ctrl_w_plus_grows_the_selected_window",
    },
    {
        "label": "R8 C-w = no longer balances",
        "file": "crates/core/lisp/evil.el",
        "old": "(defun evil-window-balance ()  ; C-w =\n  (interactive)\n  (balance-windows))",
        "new": "(defun evil-window-balance ()  ; C-w =\n  (interactive)\n  (ignore))",
        "test": "ctrl_w_equals_balances_all_windows",
    },
]
