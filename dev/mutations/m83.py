# M83 (editable search results / wgrep-level) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m83.py \
#         -p core --test-target search_tests
#
# M1-M13 were designed across two independent rounds of cold reading (the
# first round + a trailing re-review); M14-M15 were added by the main
# conversation.
#
# **Five entries in this list have the opposite point from the others.**
# The trailing re-review's expectation for M1/M2/M11/M12 is **SURVIVED** --
# it verified directly that those guards "existed at the time but no test
# could hit them":
#   - The two undo-boundary entries: `buffer.rs:434-438`'s
#     `undo_boundary()` is a **no-op** when the undo history is empty
#     (`None` is treated as "already at a boundary"), and the only test
#     touching undo at the time used a brand-new buffer opened on the spot
#     via `find-file`, with completely empty history -> both calls are
#     no-ops, and removing them changes nothing at all. The one test that
#     actually constructed "the target buffer already has unfinished
#     editing beforehand" never called `(undo)` at all.
#   - The FILE/COL entries: in the triplet-comparison fixture, the two
#     results already had the same FILE and COL, so the LINE check caught
#     the mismatch first, meaning removing FILE or COL alone still passed
#     all green.
# The last round added a test for each of these four spots (rewriting the
# undo test to first create unfinished history, then press undo; adding
# dedicated row-shift fixtures for FILE and COL respectively). **So these
# four entries must now flip to FAIL**; if they're still SURVIVED, that
# means the added tests still don't hit the same guard, which is a signal
# to report, not a pass.
#
# M14 is this milestone's root-cause regression: an unescaped pair of
# double quotes closes a docstring early, and the leftover prose becomes
# body forms (one of them a bare symbol `n` sitting **before** the
# `condition-case`), so calling the function throws immediately and the
# disk is never touched. The reader silently accepts it because the total
# quote count is even, `defun` succeeds, and there's no load-time error at
# all. This entry targets the hygiene test added for this; **its invocation
# differs from the other entries** (`--test-target lisp_hygiene_tests`),
# see that entry's note.

PACKAGE = "core"
TEST_TARGET = "search_tests"

MUTATIONS = [
    {
        "label": "M1 remove the undo boundary before editing [trailing re-review originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "            (undo-boundary)\n            (dolist (e sorted)",
        "new": "            (dolist (e sorted)",
        "test": "search_edit_apply_produces_a_single_undo_group_per_target_buffer",
        "note": (
            "Separates the user's own unfinished editing from this batch of "
            "writes. Without it, a single `(undo)` would revert both "
            "together. **Must now FAIL.**"
        ),
    },
    {
        "label": "M2 remove the undo boundary after editing [trailing re-review originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "            (undo-boundary))\n            (when (> applied 0)",
        "new": "            )\n            (when (> applied 0)",
        "test": "search_edit_apply_produces_a_single_undo_group_per_target_buffer",
        "note": "Closes off this batch of writes on its own, so the user's subsequent typing doesn't get merged back into it.",
    },
    {
        "label": "M3 remove the triplet's FILE equality check [trailing re-review originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "                         (string= (nth 0 cparsed) (nth 0 oparsed))\n",
        "new": "",
        "test": "search_edit_apply_does_not_cross_wire_files_with_same_line_and_col",
        "note": (
            "Blocks \"writing file B's text at file A's coordinates\" when "
            "row indices shift. The old fixture's two results had the same "
            "FILE, so the LINE check caught it first, meaning this couldn't "
            "hit its target. **Must now FAIL.**"
        ),
    },
    {
        "label": "M4 remove the triplet's LINE equality check",
        "file": "crates/core/lisp/search.el",
        "old": "                         (= (nth 1 cparsed) (nth 1 oparsed))\n",
        "new": "",
        "test": "search_edit_apply_does_not_cross_wire_lines_when_a_search_row_is_deleted",
        "note": "The most dangerous data-corruption shape in this milestone: line A's new text gets applied to line B's coordinates.",
    },
    {
        "label": "M5 remove the triplet's COL equality check [trailing re-review originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "                         (= (nth 2 cparsed) (nth 2 oparsed)))",
        "new": "                         t)",
        "test": "search_edit_apply_does_not_cross_wire_columns_with_same_file_and_line",
        "note": "Same as M3, the old fixture's COL was all 1. **Must now FAIL.**",
    },
    {
        "label": "M6 remove G4's \"does the file still exist\" gate",
        "file": "crates/core/lisp/search.el",
        "old": "        (if (not (file-exists-p file))",
        "new": "        (if nil",
        "test": "search_edit_apply_does_not_recreate_a_deleted_file",
        "note": (
            "`find-file` returns an **empty buffer** for a nonexistent "
            "path instead of erroring, and if some result's TEXT happens "
            "to be an empty string, D4 would compare an empty string to an "
            "empty string and call them consistent, writing a deleted file "
            "back onto disk out of nowhere."
        ),
    },
    {
        "label": "M7 remove G1's in-memory rollback",
        "file": "crates/core/lisp/search.el",
        "old": "       (when (and buf (> applied 0))\n         (setq skips (search--edit-rollback buf file skips)))",
        "new": "       (when nil\n         (setq skips (search--edit-rollback buf file skips)))",
        "test": "search_edit_apply_rolls_back_in_memory_edit_when_save_buffer_fails",
        "note": (
            "Without it, when `save-buffer` fails the user is told \"0 "
            "lines applied\", but that buffer is already dirty with this "
            "batch of edits -- any subsequent unrelated save would quietly "
            "write the \"reported as failed\" change to disk. Same "
            "severity as the silent contamination D1(c) guards against, "
            "just in the opposite direction."
        ),
    },
    {
        "label": "M8 `search--require-search-buffer` always lets it through",
        "file": "crates/core/lisp/search.el",
        "old": "(defun search--require-search-buffer ()",
        "new": "(defun search--require-search-buffer () t)\n(defun search--require-search-buffer--unused ()",
        "test": "search_edit_mode_via_m_x_from_wrong_buffer_is_refused",
        "note": "`M-x` can bypass the buffer-local keymap and call these commands from any buffer.",
    },
    {
        "label": "M9 the three search commands lose \"refuse to start while editing\" (F2)",
        "file": "crates/core/lisp/search.el",
        # The `(if (search--editing-p)` at the three call sites looks
        # identical (1543/1558); the first version's anchor occurred twice
        # and was blocked by the harness. Switched to `search-again`'s cond
        # branch instead -- that one's phrasing differs and is unique.
        "old": "   ((search--editing-p) (message \"%s\" search--edit-blocked-message))\n",
        "new": "",
        "test": "search_again_refuses_while_editing",
        "note": (
            "Without it, a new search would unconditionally `erase-buffer` "
            "and wipe out edits the user has already typed and is about to "
            "write back to disk. This is the mirror image of D9 (forbidding "
            "entry into edit mode while a search is running)."
        ),
    },
    {
        "label": "M10 `search--reset` does not clear the edit snapshot",
        "file": "crates/core/lisp/search.el",
        "old": "  (setq search--edit-original-text nil)\n  (let ((buf (get-buffer search-output-buffer-name)))",
        "new": "  (let ((buf (get-buffer search-output-buffer-name)))",
        "test": "search_reset_forces_view_mode_even_if_search_edit_mode_was_left_installed",
        "note": None,
    },
    {
        "label": "M11 remove `search--edit-apply-to-file`'s condition-case (a partial failure aborts the whole batch)",
        "file": "crates/core/lisp/search.el",
        "old": "    (condition-case err",
        "new": "    (progn-not-condition-case",
        "test": "search_edit_apply_partial_failure_does_not_abort_batch",
        "note": (
            "Follows dired's precedent of tolerating failures item by "
            "item. Note this mutation directly breaks the function (calls "
            "the nonexistent `progn-not-condition-case`), so the FAIL "
            "shows up as an error rather than a mismatched assertion -- the "
            "harness scoring it FAIL is correct, but the raw output you'll "
            "see is void-function."
        ),
    },
    {
        "label": "M12 `search-edit-mode` no longer refuses while a search is mid-run (D9)",
        "file": "crates/core/lisp/search.el",
        # D9's gate is the first cond clause inside `search-edit-mode`, not
        # an `if`. The first version's anchor occurred 0 times and was
        # blocked by the harness.
        "old": "   (search--procs\n    (message \"Cannot edit search results while a search is still running\"))",
        "new": "   (nil\n    (message \"Cannot edit search results while a search is still running\"))",
        "test": "search_edit_mode_refused_while_search_running",
        "note": "The pump keeps inserting at the end of the buffer, which conflicts with the snapshot being edited.",
    },
    {
        "label": "M13 remove search-edit-mode from `evil-emacs-state-modes`",
        "file": "crates/core/lisp/evil.el",
        "old": "search-mode search-edit-mode)",
        "new": "search-mode)",
        "test": "search_edit_mode_is_an_evil_emacs_state_mode",
        "note": (
            "Already pointed out by the trailing re-review: the other "
            "\"behavioral\" test "
            "(`search_edit_mode_allows_direct_typing_with_evil_mode_enabled`) "
            "**cannot hit this entry** -- `*search*`'s evil state is set "
            "once per buffer, and gets pinned to emacs by `search-mode` "
            "well before edit mode is even entered. What guards this is "
            "the static list-membership test."
        ),
    },
    {
        "label": "M14 put back the unescaped quote pair from the root cause (hygiene test regression)",
        "file": "crates/core/lisp/search.el",
        "old": "from `split-string' on a literal newline separator, see",
        "new": 'from `split-string\' on "\\n", see',
        "test": "no_bare_string_in_defun_body_past_the_docstring_slot",
        "note": (
            "**Must be run standalone with `--test-target "
            "lisp_hygiene_tests`.** It reproduces this milestone's root "
            "cause: an unescaped double quote in a docstring closes the "
            "docstring early, and the leftover prose becomes body forms. "
            "The total quote count is even, so the reader silently accepts "
            "it and `defun` succeeds, only blowing up when the function is "
            "**called**. This mutation should also turn the whole apply "
            "path of `search_tests` red at the same time (that's exactly "
            "the original symptom)."
        ),
    },
]
