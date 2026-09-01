# M81 (regex backtracking switched to an explicit stack + a work budget)
# mutation list.
#
#     PYTHONUNBUFFERED=1 python3 -u dev/mutate.py --config dev/mutations/m81.py \
#         -p elisp --test-target regex_tests
#     PYTHONUNBUFFERED=1 python3 -u dev/mutate.py --config dev/mutations/m81-core.py \
#         -p core --test-target core_tests
#
# Split into two because the harness only takes one package + one target
# per run, and the tests for the three noerror entries live in crates/core.
# This file is the elisp pass.
#
# M80's lesson is already baked in: when the harness gets killed, `finally`
# doesn't run, and **untracked files don't show up in git status**. All nine
# files M81 touches are tracked (modified), so `git diff` can see any
# leftovers this time -- but still, as a rule, after a kill, confirm by
# counting the hit count of every `old` string one by one.
#
# This machine's practical window is about 3 entries (each needs a
# recompile). Run in batches + `--skip-baseline` (the main conversation
# already verified green by hand) + `python3 -u` (otherwise the log is empty
# when killed).

PACKAGE = "elisp"
TEST_TARGET = "regex_tests"

MUTATIONS = [
    {
        "label": "M1 remove the per-step steps increment (reverts the hole from before fix R3)",
        "file": "crates/elisp/src/regex.rs",
        "old": "            scratch.steps += 1;",
        "new": "            // scratch.steps += 1;",
        "test": "fixed_width_repeat_without_split_or_jump_still_bounded_by_budget",
        "note": (
            "A high-severity hole caught on a cold read: `\\{n\\}`'s "
            "mandatory repeat portion gets expanded by compile_node into a "
            "**flat instruction sequence with no Split and no Jump**, while "
            "the first version only counted on the backward edge. The "
            "reviewer tested `.\\{45000\\}zzz` against a 60KB haystack: it "
            "ran for 1.563 seconds, steps and frames stayed at 0 the whole "
            "time, and neither budget ever triggered -- 1.5 seconds of "
            "editor unresponsiveness, exactly what this milestone claims to "
            "eliminate."
        ),
    },
    {
        "label": "M2 remove the budget comparison before match_at returns (the other half of the same hole)",
        "file": "crates/elisp/src/regex.rs",
        "old": "        if scratch.steps > scratch.max_steps {\n            return Err(RegexLimit);\n        }\n        if matched {",
        "new": "        if matched {",
        "test": "fixed_width_repeat_without_split_or_jump_still_bounded_by_budget",
        "note": (
            "Fix R3 comes in two halves: incrementing per step (M1) + "
            "comparing on the backward edge **and before match_at returns**. "
            "Without this half, the flat instruction stream accumulates "
            "cost but that cost is never compared against anything. "
            "**Each half alone is sufficient** (removing either half, the "
            "other still catches this test) -- this is the "
            "defense-in-depth shape recorded back in M80, which is why M1 "
            "and M2 are listed separately, and each should turn red on its "
            "own; if one SURVIVES, first check whether the other half "
            "caught it."
        ),
    },
    {
        "label": "M3 frame cap reverts to a fixed value (reverts the misclassification from before fix R4)",
        "file": "crates/elisp/src/regex.rs",
        "old": "const FRAME_BUDGET_PER_BYTE: usize = 2;",
        "new": "const FRAME_BUDGET_PER_BYTE: usize = 0;",
        "test": "long_linear_match_is_not_misclassified_as_too_complex",
        "note": (
            "A second high-severity hole caught on a cold read: the first "
            "version's MAX_FRAMES was a fixed 1 million, not scaled to the "
            "haystack, and on the success path frames are never popped. So "
            "a purely linear, never-backtracking `[^;]*` crossing 1 "
            "million characters gets classified as exceeding the limit -- "
            "contradicting the module's own stated contract of \"linear is "
            "fine, quadratic isn't\". The reviewer tested: 999,000 "
            "succeeds, 1,500,000 fails, the boundary comes entirely from "
            "that fixed constant.\n"
            "**Worse, the first version's regression test pinned this "
            "wrong behavior down as the expected behavior** -- it used a "
            "linear `[^;]*` against 2 million characters, and it passed "
            "because it hit the fixed cap, not because the pattern is "
            "pathological. The fix-up round changed all four tests to use "
            "a genuinely catastrophic-backtracking `\\(a+\\)+z`."
        ),
    },
    {
        "label": "M4 Split's push/take order swapped (greedy and lazy get completely flipped)",
        "file": "crates/elisp/src/regex.rs",
        "old": "                    scratch.frames.push(Frame {\n                        pc: b,\n                        pos: pos as u32,\n                        undo_len: scratch.journal.len() as u32,\n                    });\n                    pc = a as usize;",
        "new": "                    scratch.frames.push(Frame {\n                        pc: a,\n                        pos: pos as u32,\n                        undo_len: scratch.journal.len() as u32,\n                    });\n                    pc = b as usize;",
        "test": "greedy_vs_lazy",
        "note": (
            "Priority is decided by how compile_node fills in (a,b); run() "
            "must \"push b, take a\". Swapped, `<.*>` against \"<a><b>\" "
            "returns the short `<a>`, and `<.*?>` returns the whole string "
            "-- both assertions flip at once. This is an existing test, not "
            "added this time."
        ),
    },
    {
        "label": "M5 undo journal over-reverts (truncation condition changed to clear the whole thing)",
        "file": "crates/elisp/src/regex.rs",
        "old": "        while scratch.journal.len() > frame.undo_len as usize {",
        "new": "        while !scratch.journal.is_empty() {",
        "test": "capture_positions_correct_across_two_separately_popped_frames",
        "note": (
            "This is the one place in the whole rewrite that can quietly "
            "change the match result (not a crash -- regex silently "
            "matches wrong, which poisons evil's :s, dabbrev, org, and "
            "verilog-auto downstream). Over-reverting wipes out capture data "
            "belonging to an earlier, still-valid frame too.\n"
            "The cold read did a manual derivation + three adversarial "
            "cases + 2000 rounds of differential fuzzing on this spot and "
            "found no problem; this mutation turns \"found no problem\" "
            "into \"a guard is actually watching\".\n"
            "**The first version pointed at "
            "capture_position_correct_after_backtracking_through_save and "
            "reported SURVIVED -- yet the mutation did hit its target.** "
            "That test uses `\\(a+\\)ab`, with only one capture group; "
            "with a single group, the journal only ever has entries from "
            "one frame's section, so \"truncate to frame.undo_len\" and "
            "\"clear the whole thing\" are the same operation, making the "
            "mutation a no-op on this input. Pointing it instead at the "
            "newly added two-group test (`\\(a*\\)\\(a*\\)ab` against "
            "\"aaab\", which needs reverting across two frames popped in "
            "sequence) is what distinguishes them.\n"
            "**This is the fifth cause of SURVIVED: the test exists, looks "
            "right, but can't tell the two apart.** Different from M79's "
            "\"the test guards a different guard\" -- there, a different "
            "piece of code made it green; here, the test data itself is too "
            "weak, and the two branches behave identically on that input."
        ),
    },
    {
        "label": "M6 regexp-too-complex no longer a subclass of error",
        "file": "crates/elisp/src/interp.rs",
        "old": "                self.syms.regexp_too_complex,",
        "new": "                self.syms.wrong_type_argument,",
        "test": "regexp_too_complex_is_caught_by_a_plain_error_handler",
        "note": (
            "**Deliberately the opposite** of elisp-timeout: the latter is "
            "deliberately not a subclass of error (timeout_tests.rs:85 "
            "states outright that this is load-bearing semantics), because "
            "it means \"the user wants to cancel\"; regexp-too-complex means "
            "\"this particular call failed\", and the caller is entitled to "
            "degrade gracefully on its own."
        ),
    },
    {
        "label": "M7 signal carries the message as data (reverts the echo-area duplication bug)",
        "file": "crates/elisp/src/interp.rs",
        "old": "        let e = self.syms.regexp_too_complex;\n        self.signal(e, vec![])",
        "new": "        let e = self.syms.regexp_too_complex;\n        let msg = self.plist_get(e, self.syms.error_message);\n        self.signal(e, vec![msg])",
        "test": "regexp_too_complex_message_is_not_duplicated",
        "note": (
            "What the main conversation saw in a real editor: "
            "\"Regexp match gave up: pattern is too expensive on this "
            "text: Regexp match gave...\" -- the same sentence strung "
            "together twice, and the echo area's budget is only 72 "
            "characters (recorded in M70/M77). The cause is treating the "
            "error's own error-message property as the signal's data, "
            "while describe_flow's format is \"message: data\". The control "
            "group elisp_timeout uses signal(e, vec![])."
        ),
    },
    {
        "label": "M8 replace_all's budget reverts to resetting on every match",
        "file": "crates/elisp/src/regex.rs",
        "old": "            .search_with(target, pos, &mut scratch)",
        "new": "            .search(target, pos)",
        "test": "replace_all_shares_budget_across_its_whole_loop_not_per_match",
        "note": (
            "A third hole caught on a cold read: the public search() "
            "allocates a fresh Scratch on every call, so `:s///g`'s "
            "(replace_all's) outer loop **gets a brand-new budget every "
            "round** -- the claim of \"the whole call is bounded\" only "
            "holds within a single search() call, and :s///g is exactly the "
            "command that motivated this milestone. The implementer "
            "measured the gap before making the fix: the old shape ran "
            "ok=true for 9 calls, the shared-budget shape ran ok=false for "
            "7 calls."
        ),
    },
    {
        "label": "M9 split-string's budget reverts to resetting on every iteration",
        "file": "crates/elisp/src/builtins/misc.rs",
        "old": "                .search_with(&s, pos, &mut scratch)",
        "new": "                .search(&s, pos)",
        "test": "split_string_shares_budget_across_its_whole_loop_not_per_separator_match",
        "note": (
            "**A third occurrence of the same bug, while my spec only named "
            "two spots** (replace_all and re-search-backward). The "
            "implementer flagged this itself and didn't expand the scope "
            "unilaterally -- correctly handled. It was only added in the "
            "third round.\n"
            "Lesson: this shape of bug is especially easy to miss, because "
            "the three spots look nothing alike (the elisp layer, a core "
            "builtin, a misc builtin). **Naming spots one by one will miss "
            "some -- what's needed is a complete list** -- in the third "
            "round I required rg to list every `.search(` call site and "
            "annotate each one, only that confirmed there was no fourth "
            "spot."
        ),
    },
    {
        "label": "M10 split-string's conversion path for a single call exceeding the limit",
        "file": "crates/elisp/src/builtins/misc.rs",
        "old": "                .map_err(|_| i.regexp_too_complex())?;",
        "new": "                .unwrap_or(None);",
        "test": "split_string_signals_regexp_too_complex_not_swallowed",
        "note": (
            "**A different guard** from M9: M9 guards cross-iteration "
            "accumulation, this one guards whether a single call exceeding "
            "the limit is correctly converted to a signal (instead of "
            "being silently swallowed into \"no separator\"). Both tests "
            "should stay, with the difference written in the comment."
        ),
    },
]

# Honestly recorded: the following are unguarded by any mutation, don't
# claim they're covered.
#
# - **The optimization that skips the journal push when `frames.is_empty()`
#   is correctness-neutral**, unobservable by any functional test. The cold
#   read derived this step by step: backtrack() only ever consumes journal
#   entries after some live frame's undo_len, and any **later** frame's
#   undo_len is recorded only after those "redundant" pushes, so it's never
#   reachable. Removing this guard produces bit-identical behavior, just
#   with extra allocation. **This is "unverifiable", not "unguarded".**
#
# - **The `usize` -> `u32` truncation** (Frame's pc/pos/undo_len) silently
#   misbehaves for haystacks over 4GB. The target use case is Verilog source
#   code, so this isn't realistic, but noted anyway.
#
# - **This entry was originally something I got wrong, kept as a record.**
#   The first version said "`.\{n\}`'s n cap is 1000 (blocked at the parse
#   stage), so a single `\{n\}` expands to at most 1000 copies; M1's test
#   using `.\{45000\}` achieves that via nesting". **Both halves are
#   wrong**, and the trailing re-review caught it by reading the code plus
#   testing directly: `parse_counts`'s 1000 check **only blocks `\{n,m\}`**
#   (with a comma and an explicit m) -- `\{n\}` (no comma) returns
#   immediately at `if !self.eat(',') { return ... }`, and `\{n,\}` (open
#   upper bound) also returns before the check -- neither is bounded at all.
#   Tested directly: `.\{20000000\}` compiles successfully (51ms),
#   `.\{5000000,\}` compiles successfully (13ms). And M1's test uses a
#   **single, non-nested** `.\{45000\}`, not nesting. I wrote this from
#   assumption, without reading `parse_counts`. **M80 had just recorded
#   that "honest records themselves also need verification", and this is
#   the same mistake happening again** -- errors in an honesty field are
#   especially dangerous because they make later readers underestimate the
#   risk (believing the 1000 protection exists). This gap has been folded
#   into fix R11.
#
# - **The performance claim**: the implementer's first version reported
#   "100k iterations of a 52-byte line dropped from 8-11ms to 5.68ms",
#   attributed to scratch reuse. The cold read measured 198ns/call (3x the
#   claimed 56ns), and pointed out structurally that Scratch is **freshly
#   allocated on every external search() call**, not shared across calls, so
#   that causal explanation cannot account for a speedup across 100,000
#   independent calls. **The implementer has withdrawn that comparative
#   conclusion** (it measured 58-70ns three times itself, consistent and
#   credible, but couldn't reproduce the reviewer's 198ns, and couldn't
#   re-measure the pre-fix baseline either -- that would require checking
#   out the pre-M81 commit). **There is currently no controlled
#   before/after comparison on the same machine**; performance neutrality is
#   supported only by "two reproducible number pairs (1.038 vs 1.088ms,
#   760.79 vs 758.76ms) show no regression". Honestly recorded here.
