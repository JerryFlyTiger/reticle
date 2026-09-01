# M80 (compile / recompile / next-error) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m80.py \
#         -p core --test-target compile_tests
#
# M1-M5 were designed by an independent reviewer. M6-M9 were added by the
# main conversation after the fix-up round, specifically targeting four
# fixes caught on a cold read:
#   M6  reverts two guards at once -> **the whole test process SIGABRTs**
#       (not an assertion failure)
#   M6b/M6c are **the same mutation** (enlarging the prefix cap), differing
#       only in which test they're aimed at: M6b hits data with an early
#       colon -> SURVIVED; M6c hits purely colonless data -> SIGABRT. The
#       same mutation producing two different outcomes is exactly what
#       clarifies "what removing the trailing `.*` actually protects against"
#   M7 removes the lower-bound clamp on the column -> COL=0 jumps to the
#       previous line
#   M8 disables the output cap -> the truncation notice stops appearing
#   M9 scrambles the order of severity's cond clauses -> messages with
#       overlapping keywords get misclassified
#
# One note specifically about M6: its "failure" mode is the process getting
# killed by SIGABRT, which cargo reports as a test failure. The harness
# scoring it as FAIL is correct, but **the raw output you'll see is a stack
# overflow, not an assertion message** -- don't mistake that for the harness
# being broken.

PACKAGE = "core"
TEST_TARGET = "compile_tests"

MUTATIONS = [
    {
        "label": "M1 remove the disk-existence check (false positives get through)",
        "file": "crates/core/lisp/compile.el",
        "old": "        (when (file-exists-p file)",
        "new": "        (when t",
        "test": "compile_parses_all_four_real_formats_and_drops_nonexistent_file",
        "note": (
            "The existence check is what makes replacing GNU's giant "
            "per-tool table with a single generic regexp work -- without "
            "it, any ordinary output line that looks like `foo:12: bar` "
            "becomes a fake error."
        ),
    },
    {
        "label": "M2 remove the column's upper-bound clamp (spills onto the next line)",
        "file": "crates/core/lisp/compile.el",
        "old": "      (goto-char (max bol (min (+ bol (1- col)) (line-end-position)))))))",
        "new": "      (goto-char (+ bol (1- col))))))",
        "test": "compile_next_error_jumps_to_column_and_clamps_past_line_end",
        "note": "This is the first place in the whole codebase that jumps by column, and both ends need clamping.",
    },
    {
        "label": "M3 the dispatcher always goes to LSP, never to compile errors",
        "file": "crates/core/lisp/compile.el",
        "old": "  (if (and compile--errors (get-buffer compile-output-buffer-name))\n      (compile-next-error)",
        "new": "  (if nil\n      (compile-next-error)",
        "test": "next_error_dispatcher_prefers_compile_errors_when_present",
        "note": (
            "Dispatch logic that shares the keybinding but not the data "
            "structure. GNU has two separate key sets, M-g M-n and M-g n, "
            "leaving the user to remember which one has focus."
        ),
    },
    {
        "label": "M4 has_async_work misses compile--procs",
        "file": "crates/core/src/lib.rs",
        "old": '    // M80: `compile\'/`recompile\'s own job list.\n    let compile_procs = interp.intern("compile--procs");',
        "new": '    // M80: `compile\'/`recompile\'s own job list.\n    let compile_procs = interp.intern("compile--procs-disabled");',
        "test": "has_async_work_true_for_shell_command_and_compile_procs",
        "note": None,
    },
    {
        "label": "M5 has_async_work misses shell-command--procs (reverts a defect left over from M79)",
        "file": "crates/core/src/lib.rs",
        "old": '    let shell_command_procs = interp.intern("shell-command--procs");',
        "new": '    let shell_command_procs = interp.intern("shell-command--procs-disabled");',
        "test": "has_async_work_true_for_shell_command_and_compile_procs",
        "note": (
            "This isn't an M80 fix, it's an M79 defect that M80 fixed in "
            "passing: without it, the frontend wouldn't switch to fast "
            "polling for M-!/M-&/M-|. Turned up incidentally during "
            "reconnaissance, not on any checklist."
        ),
    },
    {
        "label": "M6 revert two guards at once (fatal: the whole process SIGABRTs)",
        "file": "crates/core/lisp/compile.el",
        "old": '  (let* ((bound (min compile--line-prefix-limit (length line)))\n         (prefix (substring line 0 bound)))\n    (when (string-match compile--error-prefix-regexp prefix)\n      (let* ((file-raw (match-string 1 prefix))',
        "new": '  (let* ((bound (length line))\n         (prefix line))\n    (when (string-match (concat compile--error-prefix-regexp "\\\\(.*\\\\)$") prefix)\n      (let* ((file-raw (match-string 1 prefix))',
        "test": "compile_survives_an_8192_char_line_and_still_parses_the_rest",
        "note": (
            "**The first version of M6 reverted only the input-length cap "
            "and reported SURVIVED -- yet the mutation did hit its target. "
            "Digging in revealed the main conversation's understanding of "
            "which change actually fixed the crash was wrong.**\n"
            "The main conversation tested this directly: the new prefix "
            "pattern (no trailing `.*`) matches a 65536-character line "
            "normally, no crash; the old full pattern (with trailing `.*`) "
            "already stack-overflows into SIGABRT on a 3000-character line. "
            "What actually fixed the crash was removing the trailing `.*` "
            "(the message is now taken via substring instead), not the "
            "input cap.\n"
            "**The two guards are not equivalent -- I got this wrong in my "
            "first version too, and the trailing re-review disproved it "
            "experimentally.**\n"
            "I originally wrote \"each guard alone is sufficient\". The "
            "reviewer fed the NEW pattern (no trailing `.*`) a long string "
            "with **absolutely no colon**: 2500 survived, both 2800 and "
            "3000 SIGABRTed, almost the same threshold as the old pattern. "
            "The mechanism is that `[^:\\n]+` is itself a greedy "
            "quantifier, and against input with no colon to stop at, it "
            "eats all the way to the end and then backtracks character by "
            "character; the backtracking depth is O(N) regardless of "
            "whether there's a trailing `.*`.\n"
            "**The correct statement**: `compile--line-prefix-limit` is the "
            "only guard that's necessary in the general case; removing the "
            "trailing `.*` only helps in the specific case where the input "
            "genuinely has an early colon -- all four real-world samples "
            "happen to satisfy that, which is why this mistake never "
            "surfaced against the samples.\n"
            "This mutation reverts both guards at once, so it still hits "
            "its target; this entry changes the whole start of the "
            "function (the harness's old/new can be multi-line blocks), "
            "reverting both the cap and the pattern together is what makes "
            "it hit.\n"
            "**The expected failure mode is SIGABRT, not an assertion "
            "message** -- don't mistake that for the harness being broken."
        ),
    },
    {
        "label": "M6b enlarges only the input cap, aimed at data with an early colon (SURVIVED)",
        "file": "crates/core/lisp/compile.el",
        "old": "(defvar compile--line-prefix-limit 300",
        "new": "(defvar compile--line-prefix-limit 1000000",
        "test": "compile_survives_an_8192_char_line_and_still_parses_the_rest",
        "expect": "SURVIVED",
        "note": (
            "**Deliberately marked expected SURVIVED, but the reason it "
            "SURVIVES isn't what my first version said.**\n"
            "I originally wrote \"enlarging the cap alone won't blow up, "
            "because the pattern no longer has a trailing `.*`\" -- that's "
            "wrong (see M6's note). It SURVIVES only because the "
            "corresponding test's data has an **early colon** "
            "(`\"top.sv:11: error: \" + \"x\"*8192`), so `[^:\\n]+` stops "
            "at the first colon and the backtracking depth is independent "
            "of line length. **This mutation doesn't test the thing it "
            "claims to test** -- switch to colonless input and it would "
            "SIGABRT. See M6c."
        ),
    },
    {
        "label": "M6c large cap + purely colonless input (exposes \"each guard is independent\")",
        "file": "crates/core/lisp/compile.el",
        "old": "(defvar compile--line-prefix-limit 300",
        "new": "(defvar compile--line-prefix-limit 1000000",
        "test": "compile_parse_error_line_survives_long_colonless_prefix",
        "note": (
            "**The same mutation** as M6b, differing only in which test it "
            "targets -- M6b targets data with an early colon (SURVIVED), "
            "this one targets purely colonless data (expected SIGABRT). The "
            "same mutation producing opposite outcomes on two tests is "
            "exactly what clarifies what removing `.*` actually protects "
            "against: it only helps when there's an early colon, and the "
            "cap is necessary regardless.\n"
            "**For this to hold, the test side first needs an input that's "
            "\"under the default cap, purely colonless, and long enough\"** "
            "(added by the fix-up round's R8); without that test, this "
            "mutation has nowhere to land."
        ),
    },
    {
        "label": "M7 remove the column's lower-bound clamp (COL=0 jumps to the previous line)",
        "file": "crates/core/lisp/compile.el",
        "old": "(max bol (min (+ bol (1- col)) (line-end-position)))",
        "new": "(min (+ bol (1- col)) (line-end-position))",
        "test": "compile_next_error_col_zero_clamps_to_current_line_start",
        "note": (
            "The regexp's COL group is `[0-9]+`, legally accepting \"0\"; "
            "`(1- 0)` = -1, and for an error not on line 1, that makes "
            "point land on the previous line's last position -- exactly "
            "violating this function's own docstring claim of \"never "
            "spills onto another line\"."
        ),
    },
    {
        "label": "M8 disable the output cap",
        "file": "crates/core/lisp/compile.el",
        "old": "    (> total compile-max-output-chars)))",
        "new": "    (> total (+ compile-max-output-chars 999999999))))",
        "test": "compile_output_cap_kills_process_and_reports_truncation",
        "note": (
            "This test was added by the main conversation: the implementer "
            "honestly flagged that R3 had no dedicated test, and M79 "
            "already had one honest gap of the same kind, which shouldn't "
            "be allowed to accumulate a second one."
        ),
    },
    {
        "label": "M9 scramble the order of severity's cond clauses",
        "file": "crates/core/lisp/compile.el",
        "old": '   ((string-match-p "\\\\berror\\\\b" msg) \'error)',
        "new": '   ((string-match-p "\\\\bnever-matches-anything\\\\b" msg) \'error)',
        "test": "compile_severity_prefers_error_over_warning_when_message_has_both_keywords",
        "note": (
            "A cold read pointed out: none of the four sample messages "
            "contains two or more keywords at once, so scrambling the cond "
            "order wouldn't turn any existing test red. This only became a "
            "real guard after the fix-up round added a fixture with "
            "overlapping keywords."
        ),
    },
]

# Honestly recorded: the following are unguarded by any mutation, don't
# claim they're covered.
#
# - **`regex.rs`'s recursive backtracking itself was not fixed**, only the
#   input on compile's side was made bounded. Any elisp-layer code that does
#   `string-match` on a long string **can still kill the editor**. This is a
#   pre-existing landmine, not introduced by M80; M80 is only the first
#   place that feeds "an external tool's output of arbitrary length"
#   directly into it. `regex.rs`'s own fuzz test only generates haystacks of
#   0-15 characters (regex.rs:1793-1809), and this magnitude has never been
#   tested. Already recorded as a next-step candidate in PLAN.md.
#
# - **Errors get silently dropped when a tool changes directory on its own**
#   (`make -C sub` prints a relative sub-directory path, and the existence
#   check fails). This is the price paid for replacing GNU's giant table
#   with "generic regexp + existence check"; already written into
#   compile.el's Known gaps, but has no test.
#
# - **Error locations are stored as raw line numbers, not markers**. If the
#   target source file is edited after compile finishes and M-g n is
#   pressed, it jumps to a stale line number. Same category as M72/M79,
#   already documented, no test.
#
# - **The dispatcher's `[lsp]` prefix relies on an undeclared return-value
#   contract from lsp.el**. Only the "no live client" branch is tested; the
#   two branches "client present with diagnostics" and "client present
#   without diagnostics" need a real LSP server to start, which the
#   existing test suite structurally cannot reach.
