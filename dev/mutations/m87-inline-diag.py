# M87 stage 3 (inline diagnostic rows) mutation list -- the `core` half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-inline-diag.py \
#         -p core --test-target inline_diagnostics_tests
#
# The `frontend-gui` half lives in dev/mutations/m87-inline-diag-gui.py and
# runs against a different target; the harness takes one package/target per
# run, so the two lists are separate files.
#
# All entries are replacement-style. Insertion-style mutations have
# repeatedly missed their target on this project (a later definition
# overwrites an earlier one), so nothing here tries to shadow a function.
#
# D3 deliberately widens the budget guard's threshold to infinity rather
# than deleting the `break`: deleting it would also delete the loop label's
# only use and stop compiling, which is a failed mutation, not a mutation.

PACKAGE = "core"
TEST_TARGET = "inline_diagnostics_tests"

MUTATIONS = [
    {
        "label": "D1 block rows stop being marked as 75% scale",
        "file": "crates/core/src/redisplay.rs",
        "old": "                grid.row_scale[r] = 75;",
        "new": "                grid.row_scale[r] = 100;",
        "test": "block_row_has_scale_75_and_kind_block_every_other_row_stays_default",
    },
    {
        "label": "D2 block rows stop being marked as RowKind::Block",
        "file": "crates/core/src/redisplay.rs",
        "old": "                grid.row_kind[r] = RowKind::Block;",
        "new": "                grid.row_kind[r] = RowKind::Text;",
        "test": "block_row_has_scale_75_and_kind_block_every_other_row_stays_default",
    },
    {
        "label": "D3 the text_rows budget guard never fires",
        "file": "crates/core/src/redisplay.rs",
        "old": "                if *row + 1 >= text_rows {",
        "new": "                if *row + 1 >= usize::MAX {",
        "test": "no_budget_left_drops_the_row_and_never_exceeds_text_rows",
    },
    {
        "label": "D4 the three-row cap on a multi-line message becomes four",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let mut lines: Vec<String> = all.iter().take(3).map(|s| s.to_string()).collect();",
        "new": "    let mut lines: Vec<String> = all.iter().take(4).map(|s| s.to_string()).collect();",
        "test": "multiline_message_splits_and_caps_at_three_rows_with_ellipsis",
    },
    {
        "label": "D5 block-row text is no longer italic",
        "file": "crates/core/src/redisplay.rs",
        "old": "                    italic: true,",
        "new": "                    italic: false,",
        "test": "block_row_style_is_italic_and_uses_the_severity_color",
    },
    {
        "label": "D6 (F6 fix) blank interior message lines are no longer skipped",
        "file": "crates/core/src/redisplay.rs",
        "old": "                if line_text.trim().is_empty() {",
        "new": "                if line_text.len() == usize::MAX {",
        "test": "blank_interior_message_line_is_skipped_not_shown_as_an_empty_row",
    },
    {
        "label": "D7 block rows are placed at a frame row instead of the window's own row",
        "file": "crates/core/src/redisplay.rs",
        "old": "                let r = rect.row + *row;",
        "new": "                let r = *row;",
        "test": "split_window_diagnostic_lands_only_inside_its_own_windows_rect",
    },
    {
        "label": "D8 the inline-diagnostics variable is ignored (always on)",
        "file": "crates/core/src/redisplay.rs",
        "old": '    let inline_diag_on = var_on(interp, "inline-diagnostics");',
        "new": "    let inline_diag_on = true;",
        "test": "inline_diagnostics_nil_reproduces_the_grid_exactly",
    },
    {
        "label": "D9 truncation never happens (message returned whole)",
        "file": "crates/core/src/redisplay.rs",
        # `if ml_width(s) <= budget {` alone appears twice (ml_truncate at
        # :612 shares the shape), so the anchor carries the fn signature.
        "old": "fn diag_truncate_tail(s: &str, budget: usize) -> String {\n    if ml_width(s) <= budget {",
        "new": "fn diag_truncate_tail(s: &str, budget: usize) -> String {\n    if true {",
        "test": "message_wider_than_the_window_is_truncated_with_ellipsis_not_wrapped",
    },
    {
        "label": "D10 (F1/D12 fix) kill-buffer stops dropping the diagnostics entry",
        "file": "crates/core/src/builtins/buffers.rs",
        "old": "        ed.borrow_mut()\n            .diagnostics\n            .remove(&(Rc::as_ptr(&target) as usize));",
        "new": "        let _ = Rc::as_ptr(&target);",
        "test": "killing_a_buffer_drops_its_diagnostics_entry",
    },
    {
        "label": "D11 the new (LINE SEV . MSG) shape drops the message text",
        "file": "crates/core/src/builtins/buffers.rs",
        "old": "                            Value::Str(s) => (*s).clone(),",
        "new": "                            Value::Str(_) => String::new(),",
        "test": "diagnostic_adds_one_row_directly_below_its_line_and_shifts_later_lines_down",
    },
]
