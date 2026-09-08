# Mutation list for M103 (display-buffer / pop-to-buffer). Designed by the
# reviewer in step 4, extended by the main conversation with the fix round's
# own two branches (F1/F2), and run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m103.py \
#         -p core --test-target window_display_tests
#
# F1 is the one that matters: without the "the selected window is itself a
# helper this mechanism created, so reuse it" branch, the second helper
# display evicts the file window -- the milestone's own symptom, reproduced
# in two keystrokes. The original test suite passed with that defect present.

PACKAGE = "core"
TEST_TARGET = "window_display_tests"

MUTATIONS = [
    {
        "label": "F1 display-buffer no longer reuses a helper window it is sitting in",
        "file": "crates/core/lisp/window.el",
        "old": "     ((window-created-for-display-p (selected-window))",
        "new": "     ((eq nil (selected-window))",
        "test": "c_h_b_then_shell_command_swaps_help_for_output_without_evicting_the_file",
    },
    {
        "label": "F2 display-buffer stops preferring another helper window",
        "file": "crates/core/lisp/window.el",
        "old": "      (let ((target (or (window--other-display-window (selected-window))\n                         (window--next-after-selected))))",
        "new": "      (let ((target (window--next-after-selected)))",
        # Survived the first run: with only two windows the preferred helper
        # window and the next-after-selected fallback are the same window.
        # The test named here builds the three-window layout where they differ.
        "test": "display_buffer_prefers_another_helper_window_over_the_next_after_selected_fallback",
    },
    {
        "label": "R1 shell-command output goes back to overwriting the selected window",
        "file": "crates/core/lisp/shell-command.el",
        "old": "  (pop-to-buffer buf)",
        "new": "  (switch-to-buffer-internal buf)",
    },
    {
        "label": "R2 eshell goes back to overwriting the selected window",
        "file": "crates/core/lisp/eshell.el",
        "old": "    (pop-to-buffer \"*eshell*\")",
        "new": "    (switch-to-buffer-internal \"*eshell*\")",
        "test": "eshell_leaves_the_original_buffer_visible",
    },
    {
        "label": "R3 ielm goes back to overwriting the selected window",
        "file": "crates/core/lisp/ielm.el",
        "old": "  (pop-to-buffer \"*ielm*\")",
        "new": "  (switch-to-buffer-internal \"*ielm*\")",
        "test": "ielm_leaves_the_original_buffer_visible",
    },
    {
        "label": "R4 describe-bindings goes back to overwriting the selected window",
        "file": "crates/core/lisp/simple.el",
        "old": "      (pop-to-buffer \"*Help*\")",
        "new": "      (switch-to-buffer-internal \"*Help*\")",
        "test": "c_h_b_splits_instead_of_overwriting_the_file_being_edited",
    },
    {
        "label": "R5 display-buffer stops reusing a window that already shows the buffer",
        "file": "crates/core/lisp/window.el",
        "old": "     (existing existing)",
        "new": "     ((eq existing 'never) existing)",
        # Same shape as F2: two windows cannot tell "reuse the window already
        # showing it" apart from the fallback, because they pick the same one.
        "test": "display_buffer_reuses_the_window_already_showing_it_even_when_a_different_window_is_next",
    },
    {
        "label": "R7 the created-for-display flag is never set on a split",
        "file": "crates/core/lisp/window.el",
        "old": "            (set-window-created-for-display new-win t)",
        "new": "            (set-window-created-for-display new-win nil)",
        "test": "q_deletes_the_window_display_buffer_created_for_it",
    },
    {
        "label": "R10 pop-to-buffer no longer selects the window it displayed in",
        "file": "crates/core/lisp/window.el",
        "old": "      (select-window win))",
        "new": "      win)",
        "test": "pop_to_buffer_selects_the_new_window",
    },
    {
        "label": "F3 quit-source-return deletes a window it did not create",
        "file": "crates/core/lisp/simple.el",
        "old": "  (if (and (window-created-for-display-p (selected-window))",
        "new": "  (if (and t",
        "test": "q_does_not_delete_a_window_it_did_not_create",
    },
    {
        "label": "F4 the per-window flag is never read back, so q never deletes",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "            Some(win) => Ok(Value::bool(win.created_for_display, i.syms.t)),",
        "new": "            Some(_win) => Ok(Value::Nil),",
        "test": "q_deletes_the_window_display_buffer_created_for_it",
    },
]
