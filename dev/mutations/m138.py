# Mutation list for M138: viewport primitives (`window-start`,
# `set-window-start`, `pos-visible-in-window-p`, `recenter`, `scroll-up-command`,
# `scroll-down-command`, `recenter-top-bottom`, `last-command`) in
# crates/core/src/redisplay.rs, builtins/ui.rs, commands.rs, simple.el.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m138.py
#
# Designed by the reviewer that cold-read the first batch; two entries it
# expected to SURVIVE (block rows in the backward walk, a hardcoded
# `next-screen-context-lines`) got tests in the fix round and are expected to
# FAIL now. M11 covers the fix round's own high-severity fix (the backward walk
# from a wrap-continuation row). Run from the main conversation.

PACKAGE = "core"
TEST_TARGET = "viewport_tests"

MUTATIONS = [
    {
        "label": "M1 the pin is never set after a viewport move",
        "file": "crates/core/src/redisplay.rs",
        "old": "win.scroll_pin = if set_pin { Some(new_point) } else { None };",
        "new": "win.scroll_pin = None;",
        "test": "scroll_pin_survives_render_until_point_moves",
        "expect": "survived",
        "note": (
            "Ran 2026-09-13: SURVIVED, and that is the honest answer, not a "
            "missed mutation. Every M138 op leaves point on screen (scroll "
            "moves it to the top/bottom row, recenter keeps it, "
            "set-window-start moves it to the middle row), and "
            "ensure_point_visible never moves a start that already shows "
            "point. The pin only matters for an op that leaves point "
            "off-screen -- the mouse wheel, gui_features_tests.rs:1777 -- so "
            "here it is symmetry with that path, not a load-bearing part."
        ),
    },
    {
        "label": "M2 rows_backward ignores block rows",
        "file": "crates/core/src/redisplay.rs",
        "old": "let block = self.block_rows.get(&prev_line0).copied().unwrap_or(0);",
        "new": "let block = 0usize;",
        "test": "scroll_down_and_recenter_count_block_rows_above",
    },
    {
        "label": "M3 end-of-buffer boundary off by one",
        "file": "crates/core/src/redisplay.rs",
        "old": "Some(t) if t < point_max => t,",
        "new": "Some(t) if t <= point_max => t,",
        "test": "scroll_up_boundary_81_allowed_82_signals",
    },
    {
        "label": "M4 scroll_down never moves point to the bottom row",
        "file": "crates/core/src/redisplay.rs",
        "old": '''    let new_start = ctx.rows_backward(&b, window_start, n);
    let mut new_point = point;
    if ctx.rows_between(&b, new_start, new_point) >= text_rows {''',
        "new": '''    let new_start = ctx.rows_backward(&b, window_start, n);
    let mut new_point = point;
    if false {''',
        "test": "scroll_down_large_arg_clamps_to_top_without_error",
    },
    {
        "label": "M5 recenter negative arg collapsed to middle",
        "file": "crates/core/src/redisplay.rs",
        "old": '''        Some(a) => {
            if -a <= h {
                h + a
            } else {
                h / 2
            }
        }''',
        "new": '''        Some(_a) => h / 2,''',
        "test": "recenter_table",
    },
    {
        "label": "M6 last-command never mirrored to elisp",
        "file": "crates/core/src/commands.rs",
        "old": '''    // `execute_command`'s comment on why `this-command` is set there.
    if let Some(id) = interp.intern_soft("last-command") {''',
        "new": '''    // `execute_command`'s comment on why `this-command` is set there.
    if let Some(id) = interp.intern_soft("last-command").filter(|_| false) {''',
        "test": "last_command_is_set_by_the_dispatcher",
    },
    {
        "label": "M7 describe_flow keeps the raw symbol name",
        "file": "crates/elisp/src/interp.rs",
        "old": '"end-of-buffer" => "End of buffer".to_string(),',
        "new": '"end-of-buffer" => "end-of-buffer".to_string(),',
        "test": "end_of_buffer_error_renders",
    },
    {
        "label": "M8 set-window-start moves an off-screen point to the top, not the middle",
        "file": "crates/core/src/redisplay.rs",
        "old": "let mid = text_rows / 2;",
        "new": "let mid = 0;",
        "test": "window_start_round_trips_and_set_window_start_moves_offscreen_point_to_middle",
    },
    {
        "label": "M9 scroll_up hardcodes next-screen-context-lines",
        "file": "crates/core/src/redisplay.rs",
        "old": '''            let ctx_lines = var_int(interp, "next-screen-context-lines", 2);
            ((text_rows as i64) - ctx_lines).max(1) as usize
        }
    };
    let ctx = RowCtx::for_window(interp, &ed.borrow(), win_id, &buf);
    let b = buf.borrow();
    let point_max = b.text.len();''',
        "new": '''            let ctx_lines = 2;
            ((text_rows as i64) - ctx_lines).max(1) as usize
        }
    };
    let ctx = RowCtx::for_window(interp, &ed.borrow(), win_id, &buf);
    let b = buf.borrow();
    let point_max = b.text.len();''',
        "test": "next_screen_context_lines",
    },
    {
        "label": "M10 recenter-top-bottom never restarts its cycle",
        "file": "crates/core/lisp/simple.el",
        "old": "(if (eq last-command 'recenter-top-bottom)",
        "new": "(if t",
        "test": "recenter_top_bottom_cycles_40_50_30_40_and_restarts_after_another_command",
    },
    {
        "label": "M11 rows_backward forgets the rows above start inside its own line",
        "file": "crates/core/src/redisplay.rs",
        "old": "        if n <= within {",
        "new": "        if false {",
        "test": "rows_backward_from_a_wrap_continuation_row",
    },
]
