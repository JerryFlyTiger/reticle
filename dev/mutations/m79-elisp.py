# M79 (one-shot shell command entry point)'s Rust primitives pass.
# The main list (elisp command layer) is in dev/mutations/m79-core.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m79-elisp.py \
#         -p elisp --test-target shell_tests
#
# Split into two because the harness only takes one package + one test
# target per run, and this milestone spans crates/elisp (process
# primitives) and crates/core (command layer).
#
# List designed by an independent reviewer, executed by the main
# conversation (implementers don't verify their own fix). M5 was added by
# the fix-up round: the reviewer pointed out that the `exit_reported` guard
# had no test watching it at the time (every polling loop stops as soon as
# it sees the first Exit), and it only became observable after the fix-up
# round added `exit_reported_exactly_once`.

PACKAGE = "elisp"
TEST_TARGET = "shell_tests"

MUTATIONS = [
    {
        "label": "M1 merged mode changed to return a tagged cons (breaks eshell's backward compatibility)",
        "file": "crates/elisp/src/shell.rs",
        "old": "            PollOutput::Merged(s) => Ok(Value::string(s)),",
        "new": '            PollOutput::Merged(s) => Ok(Value::cons(\n                Value::Sym(i.intern("stdout")),\n                Value::string(s),\n            )),',
        "test": "start_shell_process_two_args_unchanged",
        # This same entry should also turn
        # merged_default_and_explicit_merged_symbol_agree red.
        "note": (
            "eshell.el:154 is the only existing caller, and it relies on "
            "the \"raw string\" shape. The entire reason the STREAMS flag "
            "exists is to avoid touching it."
        ),
    },
    {
        "label": "M2 stream tag wired wrong: stdout gets poured into the stderr bucket",
        "file": "crates/elisp/src/shell.rs",
        "old": "                        StreamKind::Stdout => self.stdout_buf.push_str(&s),",
        "new": "                        StreamKind::Stdout => self.stderr_buf.push_str(&s),",
        "test": "separate_mode_splits_stdout_and_stderr",
        "note": "The direct consequence of the routing being wired wrong is that M-| would replace the user's RTL with stderr.",
    },
    {
        "label": "M3 STDIN is never written to the child process at all",
        "file": "crates/elisp/src/shell.rs",
        "old": "                let _ = stdin.write_all(&data);",
        "new": "                let _ = &data;",
        "test": "stdin_string_is_delivered",
        "note": "The entire M-| path depends on this one line.",
    },
    {
        "label": "M4 the 'separate symbol doesn't actually select Separate mode",
        "file": "crates/elisp/src/shell.rs",
        "old": '                    Some("separate") => Streams::Separate,',
        "new": '                    Some("separate") => Streams::Merged,',
        "test": "separate_mode_splits_stdout_and_stderr",
        "note": "The flag is accepted but has no effect -- exactly the shape that would never surface unless a test exercises it.",
    },
    {
        "label": "M5 remove the exit-sent-only-once guard (only became observable after the fix-up round)",
        "file": "crates/elisp/src/shell.rs",
        "old": "            if !self.exit_reported {\n                self.exit_reported = true;",
        "new": "            if true {\n                self.exit_reported = true;",
        "test": "exit_reported_exactly_once",
        "note": (
            "The reviewer originally flagged this as \"unverifiable\": every "
            "polling loop stops right after seeing the first Exit, so "
            "removing the guard wouldn't turn any test red. The fix-up "
            "round added a test that \"polling ten more times after exit "
            "must all return nil\", only after which it became a real guard."
        ),
    },
]

# Honestly recorded: the following are unguarded by any mutation.
#
# - The "non-blocking" property of `poll()` after switching to `try_wait()` /
#   `is_finished()`: no test can prove it never blocks. Verifying it would
#   require constructing a child process where "both pipes are closed but
#   the process refuses to finish" and measuring poll()'s latency, which is
#   a timing-dependent test, less reliable than the risk it's meant to
#   guard against. Honestly recorded here, not claimed as covered.
#
# - `kill()` (around shell.rs:334) is still a blocking `child.wait()` +
#   unconditional `join()`. The implementer's justification was "that's the
#   user-initiated abort path, not the idle tick" -- **that justification is
#   wrong**: the output-cap path is the pump's `shell-command--count` calling
#   `shell-process-kill`, and the pump runs inside `idle_tick`. In practice
#   `wait()` returns immediately after SIGKILL and the writer thread gets
#   EPIPE right away, so the bound does hold; but the reasoning behind a
#   bound has to be written correctly, since this project relies on exactly
#   this kind of reasoning to judge risk.
