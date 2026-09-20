# Mutation list for M139: evil's vim viewport family (C-f/C-b/C-e/C-y/C-d/C-u
# and the z prefix) in crates/core/lisp/evil.el, plus `window-row-start`.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m139.py
#
# Designed by the reviewer that cold-read the first batch. M7 was its predicted
# survivor (`z RET`'s first-non-blank was unobservable: the test started on an
# unindented line at column 0); the fix round moved that test to an indented
# line, so M7 is expected to FAIL now. Run from the main conversation.

PACKAGE = "core"
TEST_TARGET = "evil_tests"

MUTATIONS = [
    {
        "label": "M1 C-f page is h-1 rows instead of h-2",
        "file": "crates/core/lisp/evil.el",
        "old": '(defun evil-scroll-page-down ()  ; C-f\n  (interactive)\n  (let* ((n (max 1 (evil--total-count)))\n         (h (or (window-text-height) 1))\n         (step (max 1 (- h 2)))',
        "new": '(defun evil-scroll-page-down ()  ; C-f\n  (interactive)\n  (let* ((n (max 1 (evil--total-count)))\n         (h (or (window-text-height) 1))\n         (step (max 1 (- h 1)))',
        "test": "c_f_advances_by_h_minus_2_rows_landing_first_non_blank",
    },
    {
        "label": "M2 C-b lands the cursor one row past the bottom",
        "file": "crates/core/lisp/evil.el",
        "old": "(goto-char (window-row-start (1- h)))",
        "new": "(goto-char (window-row-start h))",
        "test": "c_b_retreats_by_h_minus_2_rows_landing_last_visible_row",
    },
    {
        "label": "M3 C-e never moves point when it leaves the window",
        "file": "crates/core/lisp/evil.el",
        "old": "(evil--goto-row-keep-column new-start",
        "new": "(ignore new-start",
        "test": "c_e_scrolls_without_moving_point_until_point_would_leave_the_window",
    },
    {
        "label": "M4 C-d forgets its count",
        "file": "crates/core/lisp/evil.el",
        "old": "(setq-local evil-scroll-count n)",
        "new": "nil",
        "test": "c_d_explicit_count_scrolls_and_is_remembered",
    },
    {
        "label": "M5 C-d scrolls past the last full page",
        "file": "crates/core/lisp/evil.el",
        "old": "(set-window-start nil (min candidate max-start) t)",
        "new": "(set-window-start nil candidate t)",
        "test": "c_d_never_scrolls_past_a_full_last_page_but_the_cursor_keeps_moving",
    },
    {
        "label": "M6 zz centres one row off",
        "file": "crates/core/lisp/evil.el",
        "old": '(defun evil-scroll-line-to-center ()  ; zz\n  (interactive)\n  (evil--z-goto-count)\n  (recenter (/ (1- (or (window-text-height) 1)) 2)))',
        "new": '(defun evil-scroll-line-to-center ()  ; zz\n  (interactive)\n  (evil--z-goto-count)\n  (recenter (/ (1+ (or (window-text-height) 1)) 2)))',
        "test": "zz_centers_and_clamps_at_both_ends",
    },
    {
        "label": "M7 z RET does not go to the first non-blank",
        "file": "crates/core/lisp/evil.el",
        "old": '  (recenter 0)\n  (goto-char (evil--pos-first-non-blank)))',
        "new": '  (recenter 0))',
        "test": "z_ret_z_dot_z_dash_add_first_non_blank_on_top_of_zt_zz_zb",
    },
    {
        "label": "M8 operator-pending runs the scroll instead of rejecting it",
        "file": "crates/core/lisp/evil.el",
        "old": "(define-key evil--op-pending-map key 'evil--op-invalid)",
        "new": "(define-key evil--op-pending-map key cmd)",
        "test": "d_c_d_cancels_the_operator_and_touches_nothing",
    },
    {
        "label": "M9 window-row-start clamps to the start instead of the last row",
        "file": "crates/core/src/redisplay.rs",
        "package": "core",
        "test_target": "viewport_tests",
        "old": "            .unwrap_or_else(|| ctx.eob_row_start(&b))\n    } else {\n        ctx.rows_backward(&b, window_start, (-n) as usize)",
        "new": "            .unwrap_or(window_start)\n    } else {\n        ctx.rows_backward(&b, window_start, (-n) as usize)",
        "test": "window_row_start_walks_and_clamps_at_both_ends",
    },
    {
        "label": "M10 the z prefix is not claimed from the op-pending catchall",
        "file": "crates/core/lisp/evil.el",
        "old": '?g ?u ?U ?~ ?z)',
        "new": '?g ?u ?U ?~)',
        "test": "d_z_z_cancels_the_operator_the_z_prefix_survives_the_op_pending_catchall",
    },
{
        "label": "M11 the last-row guard and the floor are both removed (second fix round)",
        "file": "crates/core/lisp/evil.el",
        "old": """      (when (and next-row-start (> next-row-start target) (< (1- next-row-start) limit))
        (setq limit (1- next-row-start)))
      (goto-char (max target (min (+ (point) col) limit))))))""",
        "new": """      (when (and next-row-start (< (1- next-row-start) limit))
        (setq limit (1- next-row-start)))
      (goto-char (min (+ (point) col) limit)))))""",
        "test": "c_e_onto_the_true_last_row_keeps_point_at_or_after_the_window_start",
        "note": "Either half alone is caught by the other (the floor covers a missing guard), so both go together: this is the first fix round's shape, which put point one character above the window start.",
    },
]
