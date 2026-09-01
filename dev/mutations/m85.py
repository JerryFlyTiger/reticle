# M85 (`*search*` live filtering + `M-.` jump to module declaration)
# mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m85.py \
#         -p core --test-target search_tests
#
# Some entries need to run against a different target, see each entry's
# note. **When the wrong target is used, the harness reports NOTESTS, not
# SURVIVED**, don't read that as a coverage gap (M83 hit this).
#
# M9-M11 target three high-severity findings caught on a cold read. They
# share a common shape worth recording here: **a new feature invalidates an
# existing mechanism's implicit assumption, while none of those three spots
# of code were changed at all.**
#   - `search--result-at-buffer-pos`'s exact-equality comparison is always
#     correct in a world where "every result is shown" (every entry's
#     position is currently accurate). Once filtering arrives, "a hidden
#     entry's position" becomes a value that's meaningless yet still
#     participates in the comparison. The original comment even explicitly
#     claimed this was harmless and gave reasoning for it -- that reasoning
#     is wrong.
#   - `n`/`p` traverse all results, which in a world without filtering is
#     the same as traversing what's on screen.
#   - `(buffer-name)` is used to check "am I inside *search*", which never
#     needed to be more precise in a world without a global input hook; and
#     opening the minibuffer doesn't change the current buffer.
# Same family as M84's TAB defect (see M11 in dev/mutations/m84.py).

PACKAGE = "core"
TEST_TARGET = "search_tests"

MUTATIONS = [
    {
        "label": "M1 view keymap loses `/`",
        "file": "crates/core/lisp/search.el",
        "old": "    (define-key map \"/\" 'search-filter-start)",
        "new": "",
        "test": "search_filter_narrows_to_matching_results_across_files",
        "note": "Without this line, `/` becomes self-insert, and the filter prompt can't open at all.",
    },
    {
        "label": "M2 edit keymap also binds `/` (violates D8)",
        "file": "crates/core/lisp/search.el",
        # The first version tried to shadow it by "inserting a stub with
        # the same name before the real definition", which **completely
        # missed its target** -- both elisp's defun and Rust's defun()
        # registration let a later definition override an earlier one, so
        # the real function was untouched. Changed to directly adding a `/`
        # binding in the edit keymap.
        "old": "    (define-key map \"C-c C-c\" 'search-edit-apply)",
        "new": "    (define-key map \"C-c C-c\" 'search-edit-apply)\n    (define-key map \"/\" 'search-filter-start)",
        "test": "search_filter_key_is_not_bound_in_edit_mode_keymap",
        "note": (
            "Note this mutation doesn't add the binding directly (that "
            "would touch multiple lines of keymap construction), it makes "
            "the edit keymap's installer function a no-op shell -- edit "
            "mode therefore inherits view keymap's `/`. If the harness "
            "reports a compile or runtime error rather than a test FAIL, "
            "that still counts as FAIL, but the raw output looks different."
        ),
    },
    {
        "label": "M3 the filter comparison always holds (N/M count breaks)",
        "file": "crates/core/lisp/search.el",
        "old": "    (message \"Filter: showing %d/%d result%s\" shown total (if (= total 1) \"\" \"s\"))))",
        "new": "    (message \"Filter: showing %d/%d result%s\" total total (if (= total 1) \"\" \"s\"))))",
        "test": "search_filter_narrows_live_as_you_type_before_ret",
        "note": "D4: while a filter is active, it must display \"N out of M total\", otherwise the user would think search only found this many.",
    },
    {
        "label": "M4 `search--insert-line` no longer branches (streaming results ignore the filter)",
        "file": "crates/core/lisp/search.el",
        # Same as above, the first version's shadow stub missed its target.
        # Changed to directly forcing the branch to always take the old
        # path. `if search--filter` occurs twice in this file (:969 the
        # insertion branch, :1064 navigation filtering), so the second
        # version's anchor was non-unique and the harness blocked it.
        # Adding the next line makes it unique.
        "old": "  (if search--filter\n      (search--insert-filtered-line line dir)",
        "new": "  (if nil\n      (search--insert-filtered-line line dir)",
        "test": "search_filter_set_mid_stream_governs_later_results_too",
        "note": "D6: setting a filter while a search is mid-run must decide whether new streaming results get in based on the current condition.",
    },
    {
        "label": "M5 the `orderless-rank` primitive collapses into a boolean (no longer distinguishes rank 0/1)",
        "file": "crates/core/src/builtins/ui.rs",
        # Same as above, the first version's shadow registration missed
        # its target (a later registration overrides an earlier one).
        # Changed to directly collapsing rank into 1.
        "old": "            Some(rank) => Value::Int(rank as i64),",
        "new": "            Some(_) => Value::Int(1),",
        "test": "orderless_rank_primitive_returns_nil_0_1_faithfully",
        "note": (
            "**Must be run with `--test-target completing_read_tests`.** "
            "The spec states outright that it must faithfully return "
            "`nil`/`0`/`1`, not simplify into a boolean -- sorting later "
            "relies on the rank."
        ),
    },
    {
        "label": "M6 the hook does not pass the current input as its argument",
        "file": "crates/core/src/commands.rs",
        "old": "        run_hook_by_name_with_arg(interp, \"minibuffer-input-changed-hook\", Value::string(input));",
        "new": "        run_hook_by_name_with_arg(interp, \"minibuffer-input-changed-hook\", Value::Nil);",
        "test": "minibuffer_input_changed_hook_fires_with_current_input_while_typing",
        "note": "Without the input, the consumer has no way to filter.",
    },
    {
        "label": "M7 `M-.`'s messages swapped (says the wrong thing on a miss)",
        "file": "crates/core/lisp/search.el",
        "old": "\"Not on a module name\"",
        "new": "\"No search result on this line\"",
        "test": "search_m_dot_reports_a_message_when_hit_is_not_a_module_name",
        "note": "D7: a not-found case must give a clear message, not stay silently unresponsive -- a wrong message is just as misleading.",
    },
    {
        "label": "M8 the composition when entering edit mode after filtering (D5, first half)",
        "file": "crates/core/lisp/search.el",
        # The first version targeted `search--nav-results`, the wrong spot
        # (and the shadow stub never landed either). D5's first half comes
        # from "the screen only has visible lines", so the target should be
        # render's filtering check: making render skip filtering and draw
        # every entry, so edit mode's snapshot ends up containing the
        # filtered-out lines too.
        "old": "          (if (search--matching-p e)",
        "new": "          (if t",
        "test": "search_filter_then_edit_mode_applies_only_to_visible_lines",
        "note": (
            "Confirmed on a cold read that D5's first half **holds "
            "naturally** (a consequence of M83's row-index snapshot "
            "mechanism), not a guarantee newly added in this branch. This "
            "entry targets \"it still holds now\". If SURVIVED, that means "
            "the test guards something else and needs to be reported."
        ),
    },
    {
        "label": "M9 [high] a hidden entry keeps a stale BUFFER-POS (cold-read finding 1)",
        "file": "crates/core/lisp/search.el",
        "old": "            (aset e 3 nil))))",
        "new": "            nil)))",
        "test": "search_filter_ret_does_not_jump_to_a_stale_hidden_entrys_position",
        "note": (
            "**After filtering, `RET` silently jumps to the wrong file.** "
            "A hidden entry keeps the position from the last time it was "
            "shown, and lookup does an exact-equality, newest-first scan "
            "against every entry -- a non-monotonic filter transition can "
            "make a stale value exactly equal to a visible entry's new "
            "position, with the stale one scanned first. **The reproduction "
            "conditions are narrower than intuition suggests**: plain "
            "backspace-to-empty then retyping doesn't reproduce it (every "
            "position gets correctly recomputed when passing through an "
            "empty string); it requires an AND comparison across two "
            "tokens plus cursor movement to construct."
        ),
    },
    {
        "label": "M10 [high] `n`/`p` ignore the filter (cold-read finding 2)",
        "file": "crates/core/lisp/search.el",
        "old": "  (let ((ordered (search--ordered-results)))\n    (if search--filter",
        "new": "  (let ((ordered (search--ordered-results)))\n    (if nil",
        "test": "search_filter_n_stays_within_visible_subset",
        "note": (
            "When the screen shows \"1/50\", pressing `n` still cycles "
            "among all 50 entries, jumping to a file that isn't even on "
            "screen -- directly defeating the purpose of this feature."
        ),
    },
    {
        "label": "M11 [high] remove the prompt guard (the hook gets hijacked by an unrelated prompt, cold-read finding 3)",
        "file": "crates/core/lisp/search.el",
        "old": "  (when (and (equal (minibuffer-prompt) search--filter-prompt)",
        "new": "  (when (and t",
        "test": "search_filter_not_hijacked_by_a_later_unrelated_prompt",
        "note": (
            "Opening the minibuffer **doesn't change the current buffer**, "
            "so `(buffer-name)` is still `*search*` while any prompt is "
            "open on top of `*search*`; and `search--filter`, once set, is "
            "never cleared. Consequence: after closing the prompt following "
            "a filter, pressing `M-x` and typing overwrites the filter "
            "condition on every keystroke with that unrelated text and "
            "redraws, with no warning at all. **The fix uses the "
            "point-in-time-checkable fact `(minibuffer-prompt)`, not a "
            "flag** -- a flag would run into the existing gap that \"ESC "
            "closing the minibuffer gives no notification at all\" and "
            "could never be cleared."
        ),
    },
    {
        "label": "M12 [trailing] navigation index is not recalibrated after a filter changes composition",
        "file": "crates/core/lisp/search.el",
        "old": "      (search--recalibrate-current-index old-nav))))",
        "new": "      nil)))",
        "test": "search_filter_changes_composition_recalibrates_current_index",
        "note": (
            "**F2's fix was incomplete, caught by the trailing re-review.** "
            "Before M85, the navigation target only grew monotonically (new "
            "results consed onto the front, appended to the tail after "
            "reversing), and existing entries' positions never changed, so "
            "`search--current-index`, a plain numeric index, was naturally "
            "stable. F2 changed the navigation target to a list whose "
            "**composition** changes with the filter, and the index's "
            "numeric semantics can then drift: with five entries A-E, "
            "pressing `n` three times stops at C (index=2); after filtering "
            "down to {A,C,E}, the index is still 2, and pressing `n` gives "
            "`(mod 3 3)=0` -> jumps to A instead of E. **No crash, silently "
            "jumps to the wrong place.**"
        ),
    },
    {
        "label": "M13 [trailing] parse failures are no longer recorded (the hygiene detector's second mode stops working)",
        "file": "crates/core/tests/lisp_hygiene_tests.rs",
        "old": "                parse_failures.push(ParseFailure {",
        "new": "                let _ = ParseFailure {",
        "test": "scan_file_detects_and_reports_a_genuinely_unparseable_file",
        "note": (
            "**Must be run with `--test-target lisp_hygiene_tests`.** This "
            "entry targets a coverage gap in the detector itself: the newly "
            "added parse-failure reporting path had only ever been "
            "verified against \"a clean tree produces zero false "
            "positives\", never against \"does it actually catch a genuine "
            "breakage\". If this mutation causes a compile error, the "
            "harness will report it and skip it, which is not a coverage "
            "gap."
        ),
    },
]
