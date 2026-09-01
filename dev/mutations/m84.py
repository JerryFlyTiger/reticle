# M84 (orderless out-of-order matching + `M-x` filter-while-typing) mutation
# list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m84.py \
#         -p core --test-target completing_read_tests
#
# Some entries need to be run against the `complete_tests` target, see each
# entry's note -- **when the wrong target is used, the harness reports
# NOTESTS, not SURVIVED**, don't read that as a coverage gap (M83 hit this).
#
# This milestone's most important defect cannot be verified by mutation, so
# it's recorded separately here:
# **`COMPUTE_COUNT` was originally a process-global `AtomicUsize`, while
# `SESSION_CACHE` is `thread_local!` -- the mismatched scope let T11's
# cross-time equality assertion get contaminated by parallel tests.** Its
# observable consequence is **probabilistic**, not "some test is guaranteed
# to go red":
#   - Running the whole binary: the coordinator got 12/12 green (which is
#     exactly why it slipped past the first check)
#   - **Filtered invocation** (the targeted iteration this project's
#     CLAUDE.md encourages): the cold-reader got 20 red out of 30 runs, the
#     coordinator got 5 red out of 15; after the fix (making the counter
#     thread_local too), the coordinator got 0 red across 20 runs.
# The way to verify this is by **re-running**, not by reverting a line:
#     for i in $(seq 1 20); do cargo test -p core --test completing_read_tests \
#       -- symbol_source_candidate_list_is_computed_once_per_session meta_x orderless; done
# Lesson: **the choice of invocation itself decides the conclusion**.
# "Paste test numbers together with the command" isn't enough on its own --
# things the full gate can't catch will be hit every day by ordinary
# targeted iteration.

PACKAGE = "core"
TEST_TARGET = "completing_read_tests"

MUTATIONS = [
    {
        "label": "M1 empty input no longer returns rank 0 early",
        "file": "crates/core/src/complete.rs",
        "old": "    if tokens.is_empty() {",
        "new": "    if false {",
        "test": "orderless_rank_empty_input_matches_everything_at_rank_zero",
        "note": (
            "After removal, `tokens[0]` indexes out of bounds. The FAIL "
            "shows up as a panic rather than a mismatched assertion; the "
            "harness scoring it FAIL is correct."
        ),
    },
    {
        "label": "M2 rank is always 1 (prefix no longer takes priority)",
        "file": "crates/core/src/complete.rs",
        "old": "    Some(if smart_case_starts_with(cand, tokens[0]) {\n        0\n    } else {\n        1",
        "new": "    Some(if false {\n        0\n    } else {\n        1",
        "test": "prefix_matches_rank_before_substring_matches",
        "note": (
            "This entry is D2's acceptance criterion -- **single-token "
            "input behavior must be exactly identical to before M84**. "
            "`single_token_input_matches_pre_m84_prefix_then_substring_behavior` "
            "should also go red."
        ),
    },
    {
        "label": "M3 token matching changed from \"all must hit\" to \"any one hitting is enough\"",
        "file": "crates/core/src/complete.rs",
        "old": "    if !tokens.iter().all(|t| smart_case_contains(cand, t)) {",
        "new": "    if !tokens.iter().any(|t| smart_case_contains(cand, t)) {",
        "test": "orderless_excludes_candidates_missing_any_token",
        "note": "orderless's core semantics: every space-separated token must match, order doesn't matter.",
    },
    {
        "label": "M4 smart case stops working (always case-insensitive)",
        "file": "crates/core/src/complete.rs",
        "old": "    if needle.chars().any(|c| c.is_uppercase()) {\n        hay.contains(needle)",
        "new": "    if false {\n        hay.contains(needle)",
        "test": "orderless_smart_case_per_token",
        "note": "Consistent with M82's `rg --smart-case`: a token containing an uppercase letter makes matching case-sensitive.",
    },
    {
        "label": "M5 `custom_filter`'s two tiers swapped (substring ranked before prefix)",
        "file": "crates/core/src/complete.rs",
        "old": "            Some(0) => prefix.push(c.clone()),",
        "new": "            Some(0) => substr.push(c.clone()),",
        "test": "prefix_matches_rank_before_substring_matches",
        "note": "Custom sources need to preserve the collection's original order, with the prefix tier coming first (LSP document order).",
    },
    {
        "label": "M6 `filter_sorted` drops the substring tier (reverts to pure prefix matching from before M84)",
        "file": "crates/core/src/complete.rs",
        "old": "            Some(_) => substr.push(n.clone()),",
        "new": "            Some(_) => {}",
        "test": "meta_x_substring_matches_a_real_command_name",
        "note": (
            "Before M84, `filter_sorted` **only had `starts_with`**, so "
            "typing `buffer` at `M-x` couldn't find `switch-to-buffer`. "
            "This entry verifies that gap was really closed."
        ),
    },
    {
        "label": "M7 `panel::refresh` removes the Command/Function/Symbol branch",
        "file": "crates/core/src/panel.rs",
        "old": "        s @ (Source::Command | Source::Function | Source::Symbol) => {",
        "new": "        s @ (Source::Command | Source::Function) if false => {",
        "test": "meta_x_filters_live_without_tab",
        "note": (
            "M84's second goal: `M-x` filters while typing. Without this "
            "branch, all three sources revert to the old TAB popup -- "
            "candidates get cleared while typing, and TAB has to be "
            "pressed again."
        ),
    },
    {
        "label": "M8 `buffer_rows` reverts to pure prefix (C-x b loses orderless)",
        "file": "crates/core/src/panel.rs",
        "old": "        .filter(|b| complete::orderless_rank(&b.borrow().name, input).is_some())",
        "new": "        .filter(|b| b.borrow().name.starts_with(input))",
        "test": "buffer_source_matches_substring_and_out_of_order_tokens",
        "note": (
            "**A functional gap caught on a cold read**: the first "
            "version never touched `buffer_rows` at all, so `C-x b` got "
            "none of orderless, while `filter_sorted`'s doc claims to "
            "serve the Buffer source -- that path is simply never taken. "
            "Documentation and code each hold on their own but contradict "
            "each other."
        ),
    },
    {
        "label": "M9 remove the candidate list's session cache (recomputed on every keystroke)",
        "file": "crates/core/src/complete.rs",
        "old": "    if let Some(names) = SESSION_CACHE.with(|c| {",
        "new": "    if let Some(names) = None.map(|x: Rc<Vec<String>>| x).or_else(|| SESSION_CACHE.with(|c| {",
        "test": "symbol_source_candidate_list_is_computed_once_per_session",
        "note": (
            "`command_names` scans the whole symbol table and calls "
            "`is_command` on every symbol; the cost used to only be paid "
            "when TAB was pressed, and after being wired into the panel it "
            "would be paid on every keystroke. **If this mutation causes a "
            "compile error, the harness will report it and skip it, which "
            "is not a coverage gap.**"
        ),
    },
    {
        "label": "M10 remove the cache reset when starting a new session",
        "file": "crates/core/src/commands.rs",
        "old": "                crate::complete::reset_session_cache();",
        "new": "                ();",
        "test": "command_source_cache_resets_between_sessions_of_the_same_kind",
        "note": (
            "**Original expectation on a cold read was SURVIVED for this "
            "entry**: T11's session sequence changes kind every time, and "
            "`cached_or_compute`'s kind mismatch already forces a "
            "recompute regardless of this line. The fix-up round added a "
            "test for \"two consecutive sessions of the same kind, with a "
            "new command defined in between\". **So it must now FAIL.**"
        ),
    },
    {
        "label": "M11 remove TAB expansion's \"lcp must start with stem\" guard",
        "file": "crates/core/src/commands.rs",
        "old": "        if lcp.chars().count() > stem.chars().count() && lcp.starts_with(&stem) {",
        "new": "        if lcp.chars().count() > stem.chars().count() {",
        "test": "meta_x_tab_does_not_replace_input_with_unrelated_lcp",
        "note": (
            "**A real defect caught by the trailing re-review.** The "
            "original conditional only compared character counts and never "
            "checked whether lcp is actually an extension of stem -- in a "
            "world of pure prefix matching, that assumption was guaranteed "
            "by the nature of the matcher itself, so it was never written "
            "as an explicit check. As soon as orderless arrived, that "
            "guarantee vanished, and that conditional -- unchanged a "
            "single character -- went from correct to **silently "
            "replacing the whole string the user typed with an unrelated "
            "string** (typing `clkr rst` turns into `se-review-clkrstz`, "
            "zero overlapping characters, no warning). Unreachable before "
            "M84 (multi-token wouldn't match anything); F2 then wired "
            "`C-x b` into the same path.\n"
            "`buffer_tab_does_not_replace_input_with_unrelated_lcp` should "
            "also go red."
        ),
    },
]
