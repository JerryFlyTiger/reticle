# M82 (background search + `*search*` results buffer) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m82.py \
#         -p core --test-target search_tests
#
# M1-M10 were designed by an independent reviewer from a cold read of the
# diff; M11-M15 were added by the main conversation after the fix-up round
# -- the five guards added by the fix-up round (exit-code classification,
# literal/regex mode, empty-string sanity check, shell escaping, key
# prefix) didn't exist yet when the reviewer read the diff, so its list
# couldn't hit them.
#
# **M2 / M3 / M5 are the highlights of this list, for the opposite reason
# from the other entries**: the reviewer's expected outcome for these three
# is **SURVIVED (PASS)**, because it verified directly that those three
# guards "existed at the time but no test could hit them" -- deleting the
# guard entirely wouldn't turn any of the original 12 tests red. The
# fix-up round added one test for each of these three spots. So **these
# three entries must now flip to FAIL**; if they still PASS, that means the
# added tests still don't hit the same guard, which is a signal to report,
# not "no problem".
#
# M4's observable consequence could be either FAIL or a timeout: once the
# budget is set to 0, QUEUE never drains, and the pump loop in the test
# will spin until its own timeout before asserting failure. Both outcomes
# count as expected; seeing a HANG doesn't mean the harness is broken.

PACKAGE = "core"
TEST_TARGET = "search_tests"

MUTATIONS = [
    {
        "label": "M1 line buffering treats a leftover partial line as a complete line",
        "file": "crates/core/lisp/search.el",
        "old": "    (aset entry 3 (append (aref entry 3) (nreverse complete)))\n    (aset entry 2 (car rest))))",
        "new": '    (aset entry 3 (append (aref entry 3) lines))\n    (aset entry 2 "")))',
        "test": "search_parses_a_result_line_split_across_two_chunks",
        "note": (
            "Incremental parsing is this milestone's one new mechanism. "
            "compile.el states outright that it chose to wait until the "
            "process ends and parse the whole thing at once specifically "
            "to avoid \"chunks not aligning to line boundaries\"; search "
            "cannot afford to wait."
        ),
    },
    {
        "label": "M2 remove the disk-existence guard [reviewer originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "        (when (file-exists-p file)",
        "new": "        (when t",
        "test": "search_line_shaped_like_a_result_but_naming_a_nonexistent_file_is_not_jumpable",
        "note": (
            "Confirmed on a cold read: the original \"not jumpable\" test's "
            "counterexample was banner text that doesn't look like a "
            "result line at all, taking the regex-mismatch path, which "
            "never reaches this guard. The fix-up round added a test for "
            "\"shape matches but the file doesn't exist\". **Must now FAIL.**"
        ),
    },
    {
        "label": "M3 remove the leftover-line flush at process exit [reviewer originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "            (when (and (aref entry 5) (> (length (aref entry 2)) 0))",
        "new": "            (when nil",
        "test": "search_flushes_final_line_with_no_trailing_newline_at_process_exit",
        "note": (
            "Confirmed on a cold read: the fake engine output in all 12 "
            "original tests had a trailing newline on the last line, so "
            "whether the last hit disappears was completely unobservable. "
            "**Must now FAIL.**"
        ),
    },
    {
        "label": "M4 remove the lower-bound clamp on the per-tick budget (setting it to 0 hangs forever)",
        "file": "crates/core/lisp/search.el",
        "old": "  (let ((n 0) (truncated nil) (budget (max 1 search-max-lines-per-tick)))",
        "new": "  (let ((n 0) (truncated nil) (budget search-max-lines-per-tick))",
        "test": "search_max_lines_per_tick_zero_is_clamped_and_does_not_hang",
        "note": "The symptom is that QUEUE never drains, the job is never removed, and has_async_work stays true forever.",
    },
    {
        "label": "M5 max-results reverted to an off-by-one > [reviewer originally expected SURVIVED]",
        "file": "crates/core/lisp/search.el",
        "old": "        (when (>= (aref entry 4) search-max-results)",
        "new": "        (when (> (aref entry 4) search-max-results)",
        "test": "search_max_results_truncates_and_kills_process",
        "note": (
            "Confirmed on a cold read: the original assertion was "
            "`n <= 51` (cap=50), which cannot distinguish inclusive from "
            "exclusive. The fix-up round tightened it to an exact value. "
            "**Must now FAIL.**"
        ),
    },
    {
        "label": "M6 does not kill the old job when starting a new search",
        "file": "crates/core/lisp/search.el",
        "old": "  (dolist (entry search--procs)\n    (shell-process-kill (aref entry 0)))",
        "new": "  (dolist (entry search--procs)\n    (ignore entry))",
        "test": "starting_second_search_kills_first",
        "note": None,
    },
    {
        "label": "M7 has_async_work misses search--procs (drops to slow 500ms polling)",
        "file": "crates/core/src/lib.rs",
        "old": '    let search_procs = interp.intern("search--procs");',
        "new": '    let search_procs = interp.intern("search--procs-disabled");',
        "test": "has_async_work_true_for_search_procs",
        "note": "The comment in lib.rs records that M79 already missed once on this manually maintained list.",
    },
    {
        "label": "M8 SEARCH_EL's content is emptied (the whole lisp file never loads)",
        "file": "crates/core/src/lib.rs",
        "old": 'pub const SEARCH_EL: &str = include_str!("../lisp/search.el");',
        "new": 'pub const SEARCH_EL: &str = "";',
        "test": "search_streams_fake_engine_output_into_search_buffer",
        "note": "Verifies loading is actually wired up, not incidentally working thanks to a definition elsewhere.",
    },
    {
        "label": "M9 idle_tick does not call search's pump",
        "file": "crates/core/src/lib.rs",
        "old": '    let _ = interp.eval_source("(search-process-pending-all)");',
        "new": '    let _ = interp.eval_source("(ignore)");',
        "test": "search_streams_fake_engine_output_into_search_buffer",
        "note": "Output getting back to the main loop relies on idle-tick polling, not callback notification.",
    },
    {
        "label": "M10 remove search-mode from evil-emacs-state-modes",
        "file": "crates/core/lisp/evil.el",
        "old": "(defvar evil-emacs-state-modes '(dired-mode eshell-mode ielm-mode help-mode shell-command-mode compilation-mode search-mode)",
        "new": "(defvar evil-emacs-state-modes '(dired-mode eshell-mode ielm-mode help-mode shell-command-mode compilation-mode)",
        # The first version pointed at `search_n_p_navigate_cyclically`,
        # and it actually SURVIVED -- that test's setup() never enables evil
        # at all (`search_tests.rs:35-39` only does new_interp + init_editor),
        # so its "real key press n" never goes through evil normal-state,
        # and this guard's presence or absence doesn't affect the outcome.
        # **This is a genuine coverage gap, not a mutation missing its
        # target** -- the reviewer had asserted at the time "actually
        # covered by a real key press, not an empty claim", and this
        # entry overturned it. After a test was added, it was switched to
        # point at the one that actually enables `(evil-mode 1)`.
        "test": "search_n_still_navigates_results_with_evil_mode_enabled",
        "note": (
            "Without it on this list, evil normal-state's `n` "
            "(evil-search-next) would hijack `n` inside `*search*`."
        ),
    },
    {
        "label": "M11 PATTERN interpolated directly without shell escaping (injection)",
        "file": "crates/core/lisp/search.el",
        "old": '           (cmd (format "%s %s %s" search-program args (search--shell-quote pattern)))',
        "new": '           (cmd (format "%s %s \'%s\'" search-program args pattern))',
        "test": "search_shell_quote_protects_special_characters_from_injection",
        "note": "A guard added by the fix-up round; didn't exist when the reviewer read the diff.",
    },
    {
        "label": "M12 every exit code is treated as a normal finish",
        "file": "crates/core/lisp/search.el",
        "old": "(search--report-finish (aref entry 6) (length search--results))",
        "new": "(search--report-finish 0 (length search--results))",
        "test": "search_reports_abnormal_exit_when_program_not_found",
        "note": "Added by the fix-up round. Previously rg returning 2 (regex error) or 127 (command not found) both printed finished.",
    },
    {
        "label": "M13 remove the sanity check against an empty-string PATTERN",
        "file": "crates/core/lisp/search.el",
        "old": "  (if (string-empty-p pattern)",
        "new": "  (if nil",
        "test": "search_project_rejects_empty_pattern_without_starting_a_process",
        "note": "Added by the fix-up round. An empty string would make rg match every line of every file in the whole tree.",
    },
    {
        "label": "M14 literal mode loses --fixed-strings (reverts to regex interpretation)",
        "file": "crates/core/lisp/search.el",
        "old": '  "--line-number --column --no-heading --color never --smart-case --fixed-strings --"',
        "new": '  "--line-number --column --no-heading --color never --smart-case --"',
        "test": "search_project_literal_mode_does_not_treat_bus_width_as_char_class_if_rg_available",
        "note": (
            "Added by the fix-up round, and the entry with the biggest "
            "impact on the target user for this milestone: `[7:0]` treated "
            "as a character class doesn't error, it just quietly returns "
            "the wrong result set."
        ),
    },
    {
        "label": "M16 search-again does not reuse the previous literal/regexp mode",
        "file": "crates/core/lisp/search.el",
        "old": "(search--start search--last-pattern search--last-root search--last-regexp-p)",
        "new": "(search--start search--last-pattern search--last-root nil)",
        "test": "search_again_reuses_regexp_mode_if_rg_available",
        "note": (
            "Designed by the trailing re-review. Its expectation at handoff "
            "was **SURVIVED** -- because at the time none of the 29 tests "
            "actually called into `search-again`'s function body; that key "
            "test only verified it resolved to a symbol. This mutation's "
            "original purpose was to **demonstrate that the coverage gap "
            "exists**, not to verify a guard. After the last round added "
            "five search-again tests, **it must now flip to FAIL**; if "
            "still SURVIVED, that means the added tests still don't touch "
            "the mode-carrying section."
        ),
    },
    {
        "label": "M15 bind C-c s back to a command (making the three-key sequence unreachable)",
        "file": "crates/core/lisp/simple.el",
        # The insertion point must come **after all three bindings**. The
        # first version inserted it after `C-c s s`, and the following two
        # lines for `C-c s r`/`C-c s a` rebuilt the prefix again -- the
        # mutation got overwritten by its own two following lines, and it
        # actually SURVIVED. That was "the mutation missed", not a
        # coverage gap.
        "old": "(global-set-key \"C-c s a\" 'search-again)",
        "new": "(global-set-key \"C-c s a\" 'search-again)\n(global-set-key \"C-c s\" 'search-project)",
        "test": "search_c_c_s_is_a_prefix_not_a_command",
        "note": (
            "The mechanism recorded in simple.el itself: once a single key "
            "resolves to Lookup::Command, dispatch stops right there, and a "
            "longer key sequence can never be reached."
        ),
    },
]
