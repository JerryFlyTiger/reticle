# Mutation list for M141's second half: merged process output shares one
# pipe between stdout and stderr (crates/elisp/src/shell.rs). The compile.el
# half is dev/mutations/m141.py; the runner takes one package per pass.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m141_shell.py
#
# Designed by the third cold read (the shell.rs batch). Run from the main
# conversation only, with no agent running.
#
# Not listed because no test can see it: the `StreamKind` tag passed to the
# merged-mode reader thread. `poll()`'s Merged branch ignores the tag by
# design, so Stdout -> Stderr there changes nothing observable.

PACKAGE = "elisp"
TEST_TARGET = "shell_tests"

MUTATIONS = [
    {
        "label": "S1 whole single-pipe effect: merged mode takes the two-pipe path",
        "file": "crates/elisp/src/shell.rs",
        "old": "        let merged_pipe = match streams {",
        "new": "        let merged_pipe = match Streams::Separate {",
        "test": "merged_mode_preserves_write_order",
    },
    {
        "label": "S2 merged mode expects two EOFs from its one reader",
        "file": "crates/elisp/src/shell.rs",
        "old": "                reader_thread(reader, StreamKind::Stdout, tx);\n                1\n",
        "new": "                reader_thread(reader, StreamKind::Stdout, tx);\n                2\n",
        "test": "merged_mode_preserves_write_order",
    },
    {
        "label": "S3 stderr not wired to the shared pipe",
        "file": "crates/elisp/src/shell.rs",
        "old": "                cmd.stderr(Stdio::from(writer_clone));",
        "new": "                cmd.stderr(Stdio::piped());",
        "test": "merged_mode_with_stdin_reports_exit_and_full_output",
    },
]
