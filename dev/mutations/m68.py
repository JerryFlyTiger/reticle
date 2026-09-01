# M68 -- dired's buffer identity, return target, and cursor landing spot.
#
# Source of this list: the reviewer designed 8 entries from a cold read of
# the diff; this file adopts as-is the ones expressible as "revert a single
# line", plus two more added by the main conversation (M8/M9) that it
# explicitly stated "cannot be constructed by reverting a line".
#
# How to run (two passes, since `--test-target` applies to the whole config;
# the findings span two test files):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m68.py -p core \
#         --test-target dired_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m68-help.py -p core \
#         --test-target help_tests
#
# **`PYTHONUNBUFFERED=1` is not optional.** During M67, 12 entries were run
# in the background and got killed, leaving a 0-byte log and losing the
# whole batch of work, precisely because this was missing (Python
# block-buffers stdout when it's redirected). **And it must be run in
# batches** (`--only`, about 3 minutes per batch of 3 entries) -- don't
# throw all 10 entries into the background at once.
#
# All of `crates/core/lisp/*.el` is compiled into the binary via
# `include_str!` (`crates/core/src/lib.rs:22`), so even though this whole
# list is elisp, every entry still requires a recompile to take effect, and
# NOBUILD applies to them equally.
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans (it only recognizes
# `label`/`file`/`old`/`new`/`expect`/`test`/`timeout`). No `test` filter is
# set, so every entry runs the whole target test file -- FAIL means "at
# least one went red", which incidentally catches collateral damage too.
#
# ## The most important entry in this list is M1
#
# It reverts the buffer lookup to the pre-M68 "find by name", which
# corresponds to **the path involving data loss**: an ordinary file buffer
# named `core` gets hijacked by dired, `buffer-file-name` still points at
# the original file, and pressing `C-x C-s` writes the directory listing
# into it. The main conversation established this only after actually
# destroying a file in a real TUI. If M1 survives, it means that guard
# isn't watched by any test at all.
#
# ## Two entries deliberately designed to "prove semantics" rather than
#    "prove a call happened"
#
# Neither M7 nor M8 is "remove some guard" -- instead, **the semantics are
# swapped for a version that looks plausible but is wrong**: M7 makes
# `quit-source` remember only the last hop instead of inheriting the
# original source; M8 makes `*Help*` record itself as the source every
# single time. Neither makes the code look broken, they only make `q` land
# in the wrong place -- exactly the lesson learned from M66, "ask which
# claim a mutation actually proves".

PACKAGE = "core"
TEST_TARGET = "dired_tests"

MUTATIONS = [
    {
        "label": "M1 buffer lookup reverted to finding by name (= reverts the data-loss path)",
        "file": "crates/core/lisp/dired.el",
        "old": "         (buf (or (dired--buffer-for dir) (generate-new-buffer name))))",
        "new": "         (buf name))",
        "expect_fail": [
            "two_directories_sharing_a_basename_get_separate_buffers",
            "dired_does_not_clobber_a_file_buffer_sharing_the_directorys_basename",
        ],
    },
    {
        "label": "M2 dired--buffer-for never finds anything (creates new instead of reusing)",
        "file": "crates/core/lisp/dired.el",
        "old": "      (when (and (not hit) (equal (buffer-local-value 'dired--dir b) dir))",
        "new": "      (when (and nil (not hit) (equal (buffer-local-value 'dired--dir b) dir))",
        "expect_fail": ["reusing_a_dired_buffer_keeps_marks_and_prunes_stale_ones"],
    },
    {
        "label": "M3 marks unconditionally reset (removes the local-variable-p guard)",
        "file": "crates/core/lisp/dired.el",
        "old": """    (unless (local-variable-p 'dired--marks)
      (setq-local dired--marks nil))""",
        "new": "    (setq-local dired--marks nil)",
        "expect_fail": ["reusing_a_dired_buffer_keeps_marks_and_prunes_stale_ones"],
    },
    {
        "label": "M4 remove cursor positioning after entering a directory (reviewer originally couldn't verify this, should be verifiable after the fix-up round strengthened it)",
        "file": "crates/core/lisp/dired.el",
        "old": "    (dired--goto-first-real-entry)\n    (set-buffer-read-only t)))",
        "new": "    (set-buffer-read-only t)))",
        "expect_fail": ["dired_lands_on_first_real_entry_not_dot"],
        "note": (
            "Honestly recorded by the reviewer: before the fix-up round, "
            "this mutation would **not** turn any test red, because the "
            "assertion at the time was `entry != \".\" && entry != \"..\"`, "
            "and with the positioning removed, point stops on the header "
            "line, `dired--entry-at-point` returns nil, `(car nil)` prints "
            "\"nil\", and both inequalities still hold. It only became "
            "observable after the fix-up round changed it to an exact "
            "comparison against the fixture's first real entry."
        ),
    },
    {
        "label": "M5 remove landing back on the child directory just left after ^",
        "file": "crates/core/lisp/dired.el",
        "old": "    (dired parent)\n    (dired--goto-name child)))",
        "new": "    (dired parent)))",
        "expect_fail": ["up_directory_lands_on_the_child_just_left"],
    },
    {
        "label": "M6 remove dired-revert's line-number clamping (reviewer originally couldn't verify this, should be verifiable after the fix-up round added a test)",
        "file": "crates/core/lisp/dired.el",
        "old": """    (let ((max-line (+ dired--header-lines (length dired--files))))
      (when (> (line-number-at-pos) max-line)
        (goto-char (point-min))
        (forward-line (1- max-line))))))""",
        "new": "    ))",
        "expect_fail": ["revert_clamps_point_when_the_last_row_disappears"],
        "note": "Same as M4: before the fix-up round added a test, deleting this whole block wouldn't turn any test red.",
    },
    {
        "label": "M7 quit-source remembers only the last hop, doesn't inherit the original source (proves semantics, not a call)",
        "file": "crates/core/lisp/dired.el",
        "old": "         (source (quit-source-of-current))",
        "new": "         (source (current-buffer))",
        "expect_fail": ["q_returns_to_source_through_a_dired_chain"],
        "note": (
            "This isn't removing a guard, it's swapping in a version that "
            "looks plausible but is wrong: `q` stops at the intermediate "
            "dired layer instead of going all the way back to the original "
            "file. The code doesn't look broken, only the behavior is wrong."
        ),
    },
    {
        "label": "M9 remove the set-buffer-modified-p at the end of dired--fill",
        "file": "crates/core/lisp/dired.el",
        "old": "    (set-buffer-modified-p nil)))",
        "new": "    ))",
        "expect_fail": ["a_freshly_filled_dired_buffer_is_not_marked_modified"],
        "note": (
            "A probe added by the main conversation, **not on the "
            "reviewer's list**. The first run was SURVIVED -- the line "
            "exists but no test watches it. The main conversation therefore "
            "added "
            "`a_freshly_filled_dired_buffer_is_not_marked_modified` (which "
            "verifies both the first-open and reuse paths), and after "
            "re-running, this entry FAILs as expected. **This test was "
            "written by the main conversation and has not been cold-read.**"
        ),
    },
    {
        "label": "M11 add a local-variable-p guard to quit-source (= applies marks' pattern)",
        "file": "crates/core/lisp/dired.el",
        "old": "    (setq-local quit-source source)",
        "new": "    (unless (local-variable-p 'quit-source)\n      (setq-local quit-source source))",
        "expect_fail": ["quit_source_updates_to_the_most_recent_caller_on_reuse"],
        "note": (
            "Added by the trailing re-review. This is **not** \"remove some "
            "guard\", it's **adding a guard that looks symmetric but is "
            "actually wrong** -- `dired--marks` has a `local-variable-p` "
            "guard (hand-accumulated content that should be preserved on "
            "reuse), while `quit-source` deliberately has none (it records "
            "\"who invoked this instance this time\", and should be updated "
            "on reuse).\n\n"
            "Why this specifically deserves guarding: **the first round's "
            "cold-read reviewer proposed adding exactly this guard** (the "
            "main conversation tested it and judged it a false alarm, "
            "keeping the current behavior). The trailing re-review then "
            "found that in the only existing test involving a reuse chain, "
            "the old and new `quit-source` happen to be equal, so **the two "
            "versions produce indistinguishable results** -- meaning a "
            "wrong change that had already been proposed once had nothing "
            "blocking it. Go by whatever FAILED line the harness actually "
            "prints for the test function name."
        ),
    },
    {
        "label": "M10 quit-source does not fall back to *scratch* when the target is dead",
        "file": "crates/core/lisp/simple.el",
        "old": "  (let ((target (if (and quit-source (memq quit-source (buffer-list)))",
        "new": "  (let ((target (if (and quit-source t)",
        "expect_fail": ["q_falls_back_to_scratch_when_the_source_buffer_is_gone"],
        "note": (
            "The test function name is a guess (the fix-up round reported "
            "that a test covers this scenario but didn't paste the exact "
            "function name); go by whatever FAILED line the harness "
            "actually prints, not the name written here."
        ),
    },
]
