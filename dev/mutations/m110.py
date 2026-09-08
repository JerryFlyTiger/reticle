# Mutation list for M110 (`kill-whole-line`). Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m110.py \
#         -p core --test-target core_tests
#
# The milestone's own history is the reason several of these exist: the first
# implementation followed a spec that asserted a GNU rule from memory, and that
# invented rule ("when the last line has no newline, back up and take the
# PRECEDING line's newline") both diverged from real GNU Emacs 30.2 and
# corrupted the buffer whenever point sat on the implicit trailing empty line.
# K1 and K4 are the guards against that class returning.

PACKAGE = "core"
TEST_TARGET = "core_tests"

MUTATIONS = [
    {
        # The first attempt at K1 mutated `if le < len` to `if le <= len`.
        # It SURVIVED, and the two-part check on a survivor showed why: the
        # mutation lands, but `pos` then runs one past the end and every
        # consumer clamps, so nothing observable changes. Replaced with a
        # mutation of the clamp itself, which is observable.
        "label": "K1 the end-of-buffer clamp drops the last character",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        pos = len;\n                        break;",
        "new": "                        pos = len.saturating_sub(1);\n                        break;",
        "test": "kill_whole_line_last_line_no_trailing_newline",
    },
    {
        "label": "K2 a count of zero falls through to the backward branch",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "            } else if n == 0 {",
        "new": "            } else if n == -99999 {",
        "test": "kill_whole_line_count_zero_excludes_trailing_newline",
    },
    {
        "label": "K3 the backward kill stops short of the preceding newline",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                pos = pos.saturating_sub(1);",
        "new": "                pos = pos.saturating_sub(0);",
        "test": "kill_whole_line_negative_count_kills_backward_with_preceding_newline",
    },
    {
        # Declared an expected survivor after checking, not after guessing.
        # Removing the guard leaves behaviour identical: `Buffer::delete`
        # over an empty range removes nothing and `Editor::kill_new`
        # (editor.rs) returns early on an empty string, so no kill-ring
        # entry appears either. Nothing downstream can tell the two apart --
        # and the `edit_ticks` bump is invisible too, but not for the reason
        # an earlier version of this comment claimed (that `derive_edit`
        # reports "no change" for byte-identical text since M109). The real
        # reason predates M109 and sits one layer earlier: `Buffer::delete`
        # (buffer.rs) returns immediately when the removed text is empty,
        # *before* `edit_ticks += 1` runs, so there is no bump to observe in
        # the first place -- `treesit::parse`'s generation check hits its
        # cache without ever reaching `derive_edit`. The guard is redundant
        # defensive code; it is kept because it states the intent at the
        # point of decision, but it must not be claimed as covered.
        #
        # Loose end, noted rather than closed: `Editor::kill_append`
        # (editor.rs) does not special-case an empty string the way
        # `kill_new` does -- if the guard's removal were reached through the
        # append path instead of the `kill_new` path, it would still run
        # `kill_ring_yank = kill_ring.len() - 1`. Neither the reviewer nor
        # the author could construct an observable difference from that, so
        # it stays a loose end rather than a claimed gap.
        "expect": "survived",
        "label": "K4 the empty-range guard is removed (expected to SURVIVE by construction)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "        if start == end {\n            return Ok(Value::Nil);\n        }\n        let text = edit_delete(&ed, &b, start, end);\n        // Consecutive kill-whole-lines append to the same kill-ring entry.",
        "new": "        if false {\n            return Ok(Value::Nil);\n        }\n        let text = edit_delete(&ed, &b, start, end);\n        // Consecutive kill-whole-lines append to the same kill-ring entry.",
        "test": "kill_whole_line_point_max_with_trailing_newline_is_noop",
    },
    {
        "label": "K5 the append check names the wrong command",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "matches!(&editor.last_command, Value::Sym(id) if i.sym_name(*id) == \"kill-whole-line\")",
        "new": "matches!(&editor.last_command, Value::Sym(id) if i.sym_name(*id) == \"kill-line\")",
        "test": "kill_whole_line_consecutive_calls_append_one_kill_ring_entry",
    },
    {
        "label": "K6 the read-only guard is removed",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "        super::check_writable(i, &b)?;\n        let ed = ed_handle(i);\n        let (start, end) = {\n            let bb = b.borrow();\n            let len = bb.text.len();",
        "new": "        let ed = ed_handle(i);\n        let (start, end) = {\n            let bb = b.borrow();\n            let len = bb.text.len();",
        "test": "kill_whole_line_refuses_on_read_only_buffer",
    },
    {
        "label": "K7 the kill starts at point rather than at the line's beginning",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "            let bol = bb.text.line_start(bb.point);",
        "new": "            let bol = bb.point;",
        "test": "kill_whole_line_point_mid_line",
    },
]
