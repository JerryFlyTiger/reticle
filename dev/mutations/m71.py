# M71 -- the `*` marker means something again: synthetic buffers no longer
# claim to have unsaved changes.
#
# Source of this list: the reviewer designed 5 entries (M1-M5) from a cold
# read of the diff; the main conversation added M6/M7 (the `incomplete`
# branch that only grew during the fix-up round -- those two lines didn't
# exist yet when the reviewer read the diff).
#
# How to run (four passes, since `--test-target` applies to the whole config):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71.py -p core \
#         --test-target dired_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-eshell.py -p core \
#         --test-target eshell_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-ielm.py -p core \
#         --test-target ielm_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-help.py -p core \
#         --test-target help_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this). All of `crates/core/lisp/*.el` is
# compiled into the binary via include_str!, so these elisp mutations also
# require a recompile to take effect.
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans.
#
# ## M2 is a "looks symmetric, actually only half-fixed" type (designed by
#    the reviewer)
#
# It wraps the clearing in `(when (eq ch ?\s) ...)` -- the flag is only
# cleared when "removing a mark", not when stamping `*`/`D`. This guard
# looks completely reasonable ("the flag only needs restoring when a mark is
# removed"), and **testing only unmark would pass**. What it proves: all
# four marking commands must be tested for this type of bug to be caught.

PACKAGE = "core"
TEST_TARGET = "dired_tests"

MUTATIONS = [
    {
        "label": "M1 revert the clearing on the dired marking path",
        "file": "crates/core/lisp/dired.el",
        "old": "    ;; meaningful signal; clear it again here.\n    (set-buffer-modified-p nil)))",
        "new": "    ;; meaningful signal; clear it again here.\n    (when nil (set-buffer-modified-p nil))))",
        "expect_fail": [
            "marking_a_file_does_not_mark_the_buffer_modified",
            "all_four_marking_commands_leave_the_buffer_unmodified",
            "marking_then_reverting_stays_unmodified_and_keeps_the_mark",
        ],
        "note": "Turns insert into a function's tail expression, so the "
                "following set-buffer-modified-p falls into a dead function "
                "nobody calls -- equivalent to reverting it, without any "
                "unbalanced parentheses.",
    },
    {
        "label": "M2 the flag is cleared only when removing a mark (looks symmetric, actually only half-fixed)",
        "file": "crates/core/lisp/dired.el",
        "old": "    (set-buffer-modified-p nil)))\n\n(defun dired--marks-remove",
        "new": "    (when (eq ch ?\\s) (set-buffer-modified-p nil))))\n\n(defun dired--marks-remove",
        "expect_fail": [
            "marking_a_file_does_not_mark_the_buffer_modified",
            "all_four_marking_commands_leave_the_buffer_unmodified",
            "marking_then_reverting_stays_unmodified_and_keeps_the_mark",
        ],
        "note": "Designed by the reviewer. Testing only unmark passes; only "
                "testing mark turns it red -- proves all four commands must "
                "be tested.",
    },
]
