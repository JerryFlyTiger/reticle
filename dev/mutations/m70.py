# M70 -- horizontal scrolling of the echo row: minibuffer no longer types blind.
#
# Source of this list: the reviewer designed 5 entries (Mut-A..Mut-E) from a
# cold read of the diff, of which Mut-D/Mut-E were **used to prove a coverage
# gap** (no test would FAIL at the time); only after the fix-up round added
# those two tests did they become real verification points. The main
# conversation added M6/M7 separately (code that only grew during the fix-up
# round).
#
# How to run (three passes, since `--test-target` applies to the whole
# config; the guards span three test targets):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m70.py -p core \
#         --test-target completing_read_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m70-lib.py -p core \
#         --test-target lib
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m70-isearch.py -p core \
#         --test-target isearch_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans.
#
# ## One item the reviewer explicitly stated "cannot be verified by
#    mutation", honestly recorded here
#
# The cursor-scrolling line `mb_caret.saturating_sub(off).min(win - 1)` has
# **two clamps that are each unobservable when reverted individually**:
# `echo_scroll_off`'s own invariant (`off <= caret < off + win`) degenerately
# couples the two -- `off > 0` implies `caret >= win`, so `.min` alone already
# produces the correct answer; conversely, the invariant also guarantees
# `.min` is never triggered. Only removing the whole line is observable
# (= M5), and what that catches is overflow protection, not the scrolling
# logic itself. **"Text scrolling" and "cursor scrolling" are not two truly
# independently testable guards in this implementation**, and that isn't
# claimed as covered.

PACKAGE = "core"
TEST_TARGET = "completing_read_tests"

MUTATIONS = [
    {
        "label": "M1 the offset formula removes the \"pin caret to the right edge\" term (Mut-A)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    caret.saturating_sub(win - 1).min(max_off)",
        "new": "    caret.min(max_off)",
        "expect_fail": ["m70_i4_c_a_returns_to_left_edge"],
        "note": "The reviewer verified this algebraically: what this entry "
                "cannot be caught by is e7's invariant scan (the mutated "
                "formula still satisfies all three inequalities for every "
                "caret<=total); only an integration test catches it. This "
                "is exactly an instance of 'a property test passing doesn't mean the behavior is correct'.",
    },
    {
        "label": "M2 wide character crossing the right boundary changed to <= (looks symmetric, actually wrong)",
        "file": "crates/core/src/redisplay.rs",
        "old": "                    if idx + 1 < off + win {",
        "new": "                    if idx + 1 <= off + win {",
        "expect_fail": ["m70_i8_wide_char_split_at_right_scroll_edge_draws_blank"],
        "note": "The consequence isn't a panic (put_wide has its own "
                "bounds check), it is that the last column renders the "
                "first half of a wide character while the second half is "
                "silently dropped. When the reviewer designed this entry "
                "no test would FAIL yet, and that survival was itself the "
                "coverage gap it reported.",
    },
    {
        "label": "M3 echo row reverted to being able to write the last column (reverts fix A from the fix-up round)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let win = cols.saturating_sub(1);",
        "new": "    let win = cols;",
        "expect_fail": ["m70_message_never_writes_the_echo_rows_last_column"],
        "note": "That cell is the terminal's bottom-right corner. "
                "frontend-tui never disables auto-wrap (searching the "
                "whole crate finds no ?7l/DisableLineWrap), so writing "
                "into it enters pending-wrap. Before this change, the "
                "echo loop guarded it with `ecol + w >= cols`, and it has "
                "never historically been written to -- this mutation "
                "guards a pre-existing invariant that was never written "
                "down.",
    },
    {
        "label": "M4 mb_caret reverted to summing two separately-expanded segments (reverts fix B from the fix-up round)",
        "file": "crates/core/src/redisplay.rs",
        "old": "        let input_prefix: String = mb.input.chars().take(mb.cursor).collect();\n"
               "        echo_cells(&format!(\"{}{}\", mb.prompt, input_prefix)).len()",
        "new": "        let input_prefix: String = mb.input.chars().take(mb.cursor).collect();\n"
               "        echo_cells(&mb.prompt).len() + echo_cells(&input_prefix).len()",
        "expect_fail": ["m70_caret_tab_stop"],
        "note": "echo_cells always starts col at 0 internally, so when "
                "expanded separately, a tab in the input prefix uses the "
                "wrong tab-stop baseline. With prompt=\"M-x \" (4 columns) "
                "plus a prefix of \"\\t\", it is off by 4 columns.",
    },
    {
        "label": "M5 the whole line with two clamps removed for the cursor (the only kind the reviewer says is observable)",
        "file": "crates/core/src/redisplay.rs",
        "old": "        grid.cursor = (echo_row, mb_caret.saturating_sub(off).min(win - 1));",
        "new": "        grid.cursor = (echo_row, mb_caret);",
        "expect_fail": ["m70_i3_cursor_visible_and_aligned_with_input_end"],
        "note": "See file header: what this catches is overflow "
                "protection, not the scrolling logic itself. Reverting "
                "either clamp individually is unobservable, already "
                "honestly recorded, and no fake mutation was added just "
                "to pad the count.",
    },
]
