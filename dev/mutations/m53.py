# M53 mutation list (isearch session goes stale when the current buffer is
# swapped out).
#
# List designed by the reviewer, executed by the main conversation. Kept as a
# regression list: after changing the fields of `Isearch`, `Editor::isearch_live`,
# the staleness check in `handle_key`, or any `isearch_live()` read site, run
# `dev/mutate.py --config dev/mutations/m53.py` to confirm these guards are
# still being watched by tests. If an entry turns into SKIP, the code has
# drifted from the original string; update or delete the entry.
#
# The bug itself: `Isearch`'s start / origin / origin_byte are offsets into
# "whichever buffer was current when the search started", while GapBuffer's
# char_to_byte / byte_to_char clamp out-of-range offsets instead of erroring.
# Switching or killing the buffer mid-session in elisp makes C-g set point to
# a semantically unrelated position in the new buffer -- no crash, no error,
# a silent bug.
#
# Two spots that are easy to misjudge, written down here so they don't get
# re-derived next time:
#
# * M4 (removing the buffer identity comparison, treating any still-alive
#   buffer as a match) does **not** turn the kill_buffer_... test red. A
#   killed buffer's `Weak` already fails to upgrade, so that test exercises
#   the upgrade-failure path, not the identity comparison. This isn't a
#   mistake in the list -- the two guards each cover a different scenario,
#   which is exactly why both need to stay.
#
# * `isearch` has **four** read sites in total, and this list only watches
#   three of them. The fourth is the `completion_popup_key` modal guard in
#   `commands.rs` -- it also switched to `isearch_live()`, but given the
#   current call order inside `handle_key`, by the time execution reaches
#   that line `isearch` is always `None` (the staleness check always runs
#   first, and the only branch of `isearch_key` that doesn't return and keeps
#   the session alive itself calls `isearch_exit` first). Both versions of the
#   code produce identical results, so **no mutation can turn any test red
#   because of that line**. Honestly recorded as a black-box, unobservable
#   spot: it's a defense-in-depth guard for a future reordering of the call
#   sequence, enforced by code review rather than by tests. Don't hard-code
#   an entry just to pad the count -- that would just become permanent
#   SURVIVED noise.
#
# * M2 / M3 turn **all** tests red (the comparison is inverted, or the Weak
#   is left permanently dangling, which means even a normal session where the
#   buffer was never swapped gets cleared on the very next key). One
#   representative test is pinned per entry in the list, but the actual blast
#   radius is everything. This is also where the value of the "switch away
#   and back" and "ordinary isearch regression" tests lies: they have no
#   discriminating power against the whole fix being deleted, but they're the
#   first to break against the subtler bug of an inverted comparison.

PACKAGE = "core"
TEST_TARGET = "isearch_tests"

CMD = "crates/core/src/commands.rs"
ED = "crates/core/src/editor.rs"
UI = "crates/core/src/builtins/ui.rs"

MUTATIONS = [
    {
        "label": "M1 handle_key staleness clearing (whole block removed, reverts to pre-fix)",
        "file": CMD,
        "old": """        let stale = ed.borrow().isearch.is_some() && !ed.borrow().isearch_live();""",
        "new": """        let stale = false;""",
        "test": "switch_to_buffer_mid_session_then_c_g_does_not_move_point",
    },
    {
        "label": "M1b same as above, for the \"ordinary key should not be swallowed\" test",
        "file": CMD,
        "old": """        let stale = ed.borrow().isearch.is_some() && !ed.borrow().isearch_live();""",
        "new": """        let stale = false;""",
        "test": "switch_to_buffer_mid_session_then_ordinary_key_inserts_normally",
    },
    {
        "label": "M1c same as above, for the kill-buffer test",
        "file": CMD,
        "old": """        let stale = ed.borrow().isearch.is_some() && !ed.borrow().isearch_live();""",
        "new": """        let stale = false;""",
        "test": "kill_buffer_mid_session_then_key_does_not_search_another_buffer",
    },
    {
        "label": "M2 isearch_live comparison inverted (blast radius: all tests)",
        "file": ED,
        "old": """            .is_some_and(|s| matches!(s.buffer.upgrade(), Some(b) if Rc::ptr_eq(&b, &self.current)))""",
        "new": """            .is_some_and(|s| !matches!(s.buffer.upgrade(), Some(b) if Rc::ptr_eq(&b, &self.current)))""",
        # Deliberately pinned on "ordinary isearch is completely unaffected": the
        # most obvious consequence of an inverted comparison is that even a
        # normal session where the buffer was never swapped gets cleared, and
        # that's something users would hit every day.
        "test": "ordinary_isearch_ret_and_c_g_are_unaffected",
    },
    {
        "label": "M3 isearch_start does not record the buffer (Weak is always dangling)",
        "file": ED,
        "old": """        buffer: Rc::downgrade(&buf),""",
        "new": """        buffer: std::rc::Weak::new(),""",
        "test": "switching_away_and_back_to_the_same_buffer_keeps_the_session_alive",
    },
    {
        "label": "M4 remove buffer identity comparison (any still-alive buffer counts as a match)",
        "file": ED,
        "old": """            .is_some_and(|s| matches!(s.buffer.upgrade(), Some(b) if Rc::ptr_eq(&b, &self.current)))""",
        "new": """            .is_some_and(|s| matches!(s.buffer.upgrade(), Some(_b)))""",
        "test": "switch_to_buffer_mid_session_then_c_g_does_not_move_point",
    },
    {
        "label": "M5 isearch-active-p bypasses the staleness check (reverts to reading raw is_some)",
        "file": UI,
        "old": """        Ok(Value::bool(ed_handle(i).borrow().isearch_live(), i.syms.t))""",
        "new": """        Ok(Value::bool(
            ed_handle(i).borrow().isearch.is_some(),
            i.syms.t,
        ))""",
        "test": "isearch_active_p_is_nil_in_the_gap_after_switching_buffers",
    },
    {
        "label": "M6 completion popup modal guard bypasses the staleness check",
        "file": UI,
        "old": """            if editor.minibuffer.is_some() || editor.isearch_live() || editor.key_capture.is_some()""",
        "new": """            if editor.minibuffer.is_some() || editor.isearch.is_some() || editor.key_capture.is_some()""",
        "test": "completion_popup_not_swallowed_by_a_stale_session",
    },
]
