# M72 -- the cursor in another window no longer silently drifts
# (Window.point / window_start shifting).
#
# Source of this list: the reviewer designed 6 entries (M1-M6) from a cold
# read of the diff; the main conversation added M7/M8 -- those two guard
# things that only grew during the fix-up round (`erase-buffer` switching to
# edit_delete, and the cross-buffer isolation test), which didn't exist yet
# when the reviewer read the diff.
#
# How to run (one pass is enough, all guards are in core_tests):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m72.py -p core \
#         --test-target core_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans.
#
# ## A group the reviewer explicitly stated is "structurally unobservable",
#    honestly recorded here and not put on the list
#
# The two equality boundaries in the delete branch (`win.point >= end` vs.
# `> end`, and `win.point > start` vs. `>= start`) are **unobservable by any
# test**: `end = start + n`, so at `point == end`, "shift" and "clamp to
# start" compute the same value; at `point == start`, "assign start" and
# "leave unchanged" also compute the same value. This is an algebraic
# property of this shape of clamp, not a defect -- `Buffer`'s own
# `adjust_positions_delete` and `window_start`'s delete threshold have the
# same property. **Do not design a mutation expecting FAIL for these, that
# slot would be wasted.**
#
# ## Another entry known to survive, and deliberately not added
#
# `window_start`'s insertion threshold is `>` (while `point`'s is `>=`), with
# zero test coverage at the boundary; changing it to `>=` wouldn't turn any
# test red. **Deliberately not added**: there's no repro backing it, and
# pinning down a choice nobody has argued for isn't right either. The full
# reasoning is written in the header of `adjust_windows_for_edit`.

PACKAGE = "core"
TEST_TARGET = "core_tests"

MUTATIONS = [
    {
        "label": "M1 insertion threshold changed from >= to > (point no longer follows text inserted at the same position)",
        "file": "crates/core/src/editor.rs",
        "old": "                if win.point >= pos {",
        "new": "                if win.point > pos {",
        "expect_fail": ["window_point_resyncs_correctly_through_a_multi_edit_undo_group"],
        "note": "The reviewer worked this out by hand: of the seven tests, "
                "only this one hits the pos == win.point equality case "
                "(and it's a mid-test assertion before the undo). Looks "
                "like it's off by a single equals sign.",
    },
    {
        "label": "M2 clamps to the end of the delete range instead of the start",
        "file": "crates/core/src/editor.rs",
        "old": "                    win.point = start;",
        "new": "                    win.point = end;",
        "expect_fail": [
            "window_point_clamps_when_delete_range_covers_it",
            "erase_buffer_syncs_other_window_point_and_window_start",
        ],
        "note": "Clamping to end lands on an out-of-range position after "
                "the delete (the text has already shortened). W7 also "
                "goes red alongside it -- `erase-buffer` deletes [0, len), "
                "so the frozen point necessarily falls inside the range "
                "and takes the same clamp branch (live run: 2 tests red, "
                "consistent with this).",
    },
    {
        "label": "M3 remove .rev() from the undo group replay order",
        "file": "crates/core/src/buffer.rs",
        "old": "        for entry in group.into_iter().rev() {",
        "new": "        for entry in group.into_iter() {",
        "expect_fail": ["window_point_resyncs_correctly_through_a_multi_edit_undo_group"],
        "note": "The reviewer points out this entry's blast radius is "
                "bigger than window sync -- it breaks undo's own text "
                "replay, so more than one test is expected to go red. "
                "Listed here to confirm that \"order\" is being watched.",
    },
    {
        "label": "M4 undo's window shifting applied in reverse order",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                for edit in edits {",
        "new": "                for edit in edits.into_iter().rev() {",
        "expect_fail": ["window_point_resyncs_correctly_through_a_multi_edit_undo_group"],
        "note": "**W5 (undo of a single entry) cannot catch this** -- "
                "reversing a Vec with only one element is a no-op. Only "
                "W5b (two edits at different positions in a group) can "
                "guard it. This is exactly why the main conversation "
                "specifically requested W5b in the first place, and the "
                "reviewer confirmed by hand that it really discriminates.",
    },
    {
        "label": "M5 undo does not shift windows at all (reverts to the pre-fix state)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                for edit in edits {\n"
               "                    adjust_windows_for_edit(&ed, &b, edit);\n"
               "                }",
        "new": "                for edit in edits {\n"
               "                    let _ = edit;\n"
               "                }",
        "expect_fail": [
            "window_point_and_window_start_both_resync_on_undo",
            "window_point_resyncs_correctly_through_a_multi_edit_undo_group",
        ],
        "note": "This is exactly the second drift the main conversation "
                "reproduced (pressing u in window B, window A drifts from "
                "L69:9 to L69:8).",
    },
    {
        "label": "M6 the insert branch does not shift point at all (reverts to the pre-M72 state)",
        "file": "crates/core/src/editor.rs",
        "old": "                if win.point >= pos {\n"
               "                    win.point += n;\n"
               "                }",
        "new": "                if win.point >= pos {\n"
               "                    let _ = n;\n"
               "                }",
        "expect_fail": [
            "window_point_shifts_forward_past_insert_before_it",
            "window_point_end_to_end_across_window_switch",
            "window_point_and_window_start_both_resync_on_undo",
            "window_point_resyncs_correctly_through_a_multi_edit_undo_group",
        ],
        "note": "The reviewer points out W3 (\"a later edit has no effect\") "
                "**cannot catch this** -- a point that never moves always "
                "satisfies \"unchanged\". W3 guards the opposite direction "
                "of error (unconditional shifting). The two undo tests also "
                "go red, **and they blow up on a mid-test assertion before "
                "undo is even called** -- the insertion-threshold defect "
                "spills over into the undo path, because both of those "
                "tests do a forward insert before the undo (live run: 4 "
                "tests red, consistent with this).",
    },
    {
        "label": "M7 erase-buffer reverted to calling Buffer::delete directly (reverts fix A from the fix-up round)",
        "file": "crates/core/src/builtins/buffers.rs",
        "old": "        crate::editor::edit_delete(&ed, &b, 0, len);",
        "new": "        let _ = &ed;\n        b.borrow_mut().delete(0, len);",
        "expect_fail": ["erase_buffer_syncs_other_window_point_and_window_start"],
        "note": "Reviewer finding 1: this was the one direct-call site "
                "the whole crate missed after consolidation, and it's used "
                "by dired's re-read, eshell's clear, and describe-bindings.",
    },
    {
        "label": "M8 remove the cross-buffer filter (every window gets shifted)",
        "file": "crates/core/src/editor.rs",
        "old": "        if !Rc::ptr_eq(&win.buffer, buf) {\n            continue;\n        }",
        "new": "        if false {\n            continue;\n        }",
        "expect_fail": ["window_point_unaffected_by_edits_in_a_different_buffer"],
        "note": "The reviewer points out that before the fix-up round **no "
                "test at all** used a second buffer, so removing this "
                "filter wouldn't turn any test red. W8 was added exactly "
                "for this.",
    },
]
