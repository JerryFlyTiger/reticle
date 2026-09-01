# M79 (one-shot shell command entry point)'s elisp command-layer pass.
# The Rust primitives pass is in dev/mutations/m79-elisp.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m79-core.py \
#         -p core --test-target shell_command_tests
#
# List designed by an independent reviewer, executed by the main
# conversation (implementers don't verify their own fix).
#
# M9 / M10 / M11 are three entries added only after the fix-up round,
# specifically targeting two real bugs and one diagnostic gap the reviewer
# mechanically reproduced:
#   M9  region reverts from a marker to a raw offset -> concurrent edits
#       destroy the user's text
#   M10 quit-source reverts to being captured "at display time" -> q returns
#       to the wrong buffer
#   M11 the bounded drain before kill is removed -> truncation diagnostics
#       miss output that's already buffered
# These three entries also double as verification points for "did the
# fix-up round actually fix it".

PACKAGE = "core"
TEST_TARGET = "shell_command_tests"

MUTATIONS = [
    {
        "label": "M1 still steals the window even with no output (breaks this milestone's value proposition)",
        "file": "crates/core/lisp/shell-command.el",
        "old": "        (if (and empty (integerp code) (= code 0))",
        "new": "        (if (and (not empty) (integerp code) (= code 0))",
        "test": "shell_command_with_no_output_does_not_switch_buffers",
        "note": (
            "Today, M-x eshell makes you lose the file you were editing "
            "regardless of whether there's output, and a clean lint run "
            "(no output) is exactly the most common case. This guard is the "
            "whole reason this milestone exists."
        ),
    },
    {
        "label": "M2 stderr gets concatenated into the text replacing the region (data corruption)",
        "file": "crates/core/lisp/shell-command.el",
        "old": "            (insert stdout))",
        "new": "            (insert (concat stdout stderr)))",
        "test": "shell_command_on_region_success_keeps_stderr_out_of_buffer",
        "note": "The sole reason the whole 'separate routing mechanism exists.",
    },
    {
        "label": "M3 a nonzero exit still replaces the region",
        "file": "crates/core/lisp/shell-command.el",
        "old": "     ((and (integerp code) (= code 0))",
        "new": "     ((and (integerp code) t)",
        "test": "shell_command_on_region_failure_leaves_region_untouched",
        "note": "GNU always replaces regardless of success or failure; this is deliberately different.",
    },
    {
        "label": "M4 the output cap stops working",
        "file": "crates/core/lisp/shell-command.el",
        "old": "    (> total shell-command-max-output-chars)))",
        "new": "    (> total (+ shell-command-max-output-chars 999999999))))",
        "test": "truncat",
        "note": (
            "shell.rs deliberately has no output cap (the division of labor "
            "is written in its file header), so this guard exists only at "
            "this layer. A user could type `M-! yes`."
        ),
    },
    {
        "label": "M5 default-directory's nil fallback removed",
        "file": "crates/core/lisp/shell-command.el",
        "old": "  (or (default-directory) (file-name-as-directory (expand-file-name \".\"))))",
        "new": "  (default-directory))",
        "test": "shell_command_with_output_lands_in_output_buffer",
        "note": (
            "A newly created buffer's default-directory is always nil "
            "(buffer.rs:369 doesn't inherit it), and this is exactly what "
            "turned 6/13 tests red for the implementer in the first round."
        ),
    },
    {
        "label": "M6 M-! bound to the wrong key",
        "file": "crates/core/lisp/simple.el",
        "old": "(global-set-key \"M-!\" 'shell-command)",
        "new": "(global-set-key \"M-1\" 'shell-command)",
        "test": "shell_command_keys_bound_in_global_keymap",
        "note": None,
    },
    {
        "label": "M7 shell-command-mode not added to evil-emacs-state-modes",
        "file": "crates/core/lisp/evil.el",
        "old": " help-mode shell-command-mode)",
        "new": " help-mode)",
        "test": "shell_command_mode_is_an_evil_emacs_state_mode",
        "note": (
            "Missing this means evil normal-state's keymap ranks above the "
            "local keymap, so the output buffer's q can never be reached. "
            "simple.el:254-262 documents this pitfall."
        ),
    },
    {
        "label": "M8 the idle pump was never wired up",
        "file": "crates/core/src/lib.rs",
        "old": '    let _ = interp.eval_source("(shell-command-process-pending-all)");',
        "new": '    // let _ = interp.eval_source("(shell-command-process-pending-all)");',
        "test": "shell_command_with_output_lands_in_output_buffer",
        "note": "Without wiring, all async output never appears; expected to fail as a timeout.",
    },
    {
        "label": "M9 region reverts from a marker to a raw offset (reverts the data corruption from before fix R1)",
        "file": "crates/core/lisp/shell-command.el",
        "old": "      (let ((beg-marker (copy-marker (region-beginning)))\n            (end-marker (copy-marker (region-end)))",
        "new": "      (let ((beg-marker (region-beginning))\n            (end-marker (region-end))",
        "test": "shell_command_on_region_survives_concurrent_edit_at_region_start",
        "note": (
            "A real bug the reviewer mechanically reproduced: the buffer is "
            "\"hello world\", the region is \"hello\", the user types XXXXX "
            "at the very front while the command is mid-run, and the result "
            "is \"HELLOhello world\" -- the text the user just typed is "
            "silently deleted and the original text is never transformed at "
            "all. PLAN.md M72 recorded this \"raw offset\" category before; "
            "this is the first time it was actually reproduced."
        ),
    },
    {
        "label": "M10 quit-source reverts to being captured at display time (reverts the behavior from before fix R2)",
        "file": "crates/core/lisp/shell-command.el",
        "old": (
            "            (shell-command--maybe-show buf (aref entry 9))\n"
            '            (message "Shell command %s"'
        ),
        "new": (
            "            (shell-command--maybe-show buf (quit-source-of-current))\n"
            '            (message "Shell command %s"'
        ),
        "test": "shell_command_quit_source_recorded_at_invocation_not_at_pump_time",
        "note": (
            "The first version's `old` string appeared 4 times in the file "
            "(all four pump-side call sites got the fix applied), and the "
            "harness correctly judged it non-unique and SKIPped -- that's a "
            "stale list, not an unwatched guard. Adding the next line's "
            "message as context makes it unique. Everywhere else in this "
            "repo that uses quit-source captures it synchronously at call "
            "time; this is the first one captured from the idle pump, which "
            "is why the first version ended up capturing whatever buffer "
            "the user happened to be in the instant the process finished."
        ),
    },
    {
        "label": "M11 remove the bounded drain before kill (reverts the diagnostic gap from before fix R3)",
        "file": "crates/core/lisp/shell-command.el",
        "old": "(defun shell-command--drain-before-kill (entry)",
        "new": "(defun shell-command--drain-before-kill-disabled (entry)",
        "test": "truncat",
        "note": (
            "Renaming it makes the caller unable to find it; the truncation "
            "path is expected to fail with void-function. If this reports "
            "SURVIVED, first check whether the caller's condition-case is "
            "swallowing it -- that would be \"the mutation hit but got "
            "swallowed\", not \"nobody guards it\"."
        ),
    },
    {
        "label": "M12 remove R7's content-comparison guard (reverts \"silent deletion\")",
        "file": "crates/core/lisp/shell-command.el",
        "old": "         ((not (string= (with-current-buffer orig-buf\n                          (buffer-substring beg end))\n                        snapshot))",
        "new": "         ((and nil (not (string= (with-current-buffer orig-buf\n                          (buffer-substring beg end))\n                        snapshot)))",
        "test": "shell_command_on_region_concurrent_edit_at_end_boundary_is_preserved",
        # This same entry should also turn
        # shell_command_on_region_concurrent_edit_in_middle_is_preserved red.
        "note": (
            "Found by the trailing re-review: R1's switch to markers only "
            "fixed the \"edit before the region\" case; \"insert at the end "
            "boundary or right in the middle\" actually got worse, going "
            "from \"misplaced but preserved\" to \"silently deleted\" -- and "
            "it doesn't go into undo either. The root cause is that "
            "buffer.rs:645-650's adjust_positions_insert unconditionally "
            "moves forward any marker with p >= pos, and beg and end both "
            "follow that same rule; this editor's markers have no "
            "insertion-type concept, so nothing stops end-marker from "
            "moving forward along with it."
        ),
    },
    {
        "label": "M13 R7's guard over-expands: even \"edited before\" now refuses to replace",
        "file": "crates/core/lisp/shell-command.el",
        "old": "                        snapshot))",
        "new": "                        (concat snapshot \"x\")))",
        "test": "shell_command_on_region_concurrent_edit_before_region_still_replaces",
        "note": (
            "The opposite direction of mistake: a guard added too broadly "
            "also blocks the case R1 originally fixed, and the user can "
            "never touch the buffer while the command is running again. "
            "Both directions need a test."
        ),
    },
    {
        "label": "M14 R8's character-count cap stops working (unbounded overshoot during the drain phase)",
        "file": "crates/core/lisp/shell-command.el",
        "old": "                (< drained drain-cap))",
        "new": "                (or t (< drained drain-cap)))",
        "test": "truncat",
        "expect": "SURVIVED",
        "note": (
            "**Deliberately marked expected SURVIVED**: the trailing "
            "re-review points out R8 adds a guard against the overshoot "
            "direction, but existing tests only verify \"the truncation "
            "message appears\", none of them measures \"how many characters "
            "actually got pulled in\". So removing the character cap won't "
            "turn any test red. Honestly recorded: this guard is currently "
            "black-box unobservable, not covered."
        ),
    },
    {
        "label": "M15 remove R9's per-tick liveness check",
        "file": "crates/core/lisp/shell-command.el",
        "old": "        (when (and (eq mode 'filter) (not (get-buffer (aref entry 3))))",
        "new": "        (when (and nil (eq mode 'filter) (not (get-buffer (aref entry 3))))",
        "test": "shell_command_on_region_stops_promptly_when_original_buffer_killed_mid_stream",
        "note": (
            "R4 only has a reactive check (only looks when a chunk "
            "arrives), which never triggers for a process quietly sleeping. "
            "R9 adds a proactive check at the start of the tick.\n"
            "**This entry reported SURVIVED on the first run, and the "
            "mutation did hit its target -- the problem was the test.** "
            "The original command was `echo chunk; sleep 5`, and the "
            "test's pump_until returned true on the very first tick, "
            "before that chunk had even been polled; the buffer was killed "
            "and only the next tick polled it, so **R4's reactive check** "
            "killed the process. The test was green, but for the wrong "
            "reason -- R4, not R9. Only after switching to a `sleep 5` with "
            "no output at all, so reactive never gets a chance to trigger, "
            "did this entry really turn red.\n"
            "Lesson: **a test can be green for the wrong reason**, and look "
            "exactly like it's green for the right one. SURVIVED's third "
            "possible cause (the first two are written in mutate.py's "
            "header: \"nobody guards it\" and \"the mutation missed\") is "
            "\"**the test is guarding a different guard**\"."
        ),
    },
]

# Honestly recorded: the following are unguarded by any mutation, don't
# claim they're covered.
#
# - `M-|` doesn't expand to whole lines under evil Visual Line (`V`)
#   selection -- region-beginning/region-end don't know about
#   `evil--visual-type`. This is a known gap (written in shell-command.el's
#   Known gaps), not a defect, so there's no test and no mutation for it.
#
# - The three commands share one output buffer, and a new command kills the
#   previous still-running process: this deliberate simplification has no
#   dedicated test.
#
# - The prose correctness of file headers and docstrings is never verified
#   by any test (same category of gap as M78).
