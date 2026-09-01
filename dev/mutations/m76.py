# M76 -- remote save: length gate + write-through commit (crash-safe).
#
# Source of this list: the second-round reviewer (cold-reading the
# "length gate + write-through" design) designed M1-M3 and M5; M4 and M6 are
# points that only became guardable after the fix-up round added tests; M7
# was added by the main conversation.
#
# How to run:
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m76.py -p core \
#         --test-target ssh_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# ## This milestone's design was replaced once; the list applies only to
#    the final design
#
# The first version was "same-directory temp file + atomic `mv` rename". It
# turned out **worse than the original bug under the real ssh failure
# model**: when the connection drops, the remote command's stdin receives a
# **clean EOF** (no tty means no SIGHUP), so `cat` returns 0 and `mv` commits
# anyway -- tested live in a real TUI: the editor reported `Wrote`, while a
# 79-line RTL file became a half-written 21-line file. **The real guard is
# the length gate, not the atomic rename.**
#
# ## Something the reviewer stated outright and the main conversation
#    confirmed: most tests only lock in "don't regress to the mv version"
#
# M1 (reverting entirely to the most primitive `cat > path`) only turns two
# tests red; the symlink / hardlink / dash-prefixed-filename / inode /
# TMPDIR / empty-buffer tests **all stay green under M1**, because the old
# implementation was already write-through and satisfies those properties
# too. **This isn't a coverage gap, guarding against the mv version is
# exactly those tests' job**, honestly recorded here so "only two go red"
# isn't mistaken for a broken list.

PACKAGE = "core"
TEST_TARGET = "ssh_tests"

MUTATIONS = [
    {
        "label": "M1 revert entirely to the original `cat > path` (no temp file, no length gate)",
        "file": "crates/core/src/remote.rs",
        "old": """    let cmd = format!(
        "n={n}; t={stem_q}.$$; ( : > \\"$t\\" ) 2>/dev/null || t=${{TMPDIR:-/tmp}}/reticle-save.$$\\n\\
         cat > \\"$t\\" && [ \\"$(wc -c < \\"$t\\")\\" -eq \\"$n\\" ] || {{ rm -f \\"$t\\"; exit 1; }}\\n\\
         if cat \\"$t\\" > {q}; then rm -f \\"$t\\"; exit 0; \\
         elif [ -r \\"$t\\" ]; then echo \\"staged copy kept at $t\\" >&2; exit 3; \\
         else echo \\"staged copy lost\\" >&2; exit 3; fi"
    );""",
        "new": """    let _ = (&n, &stem_q);
    let cmd = format!("cat > {q}");""",
        "expect_fail": [
            "remote_write_early_eof_is_rejected",
            "remote_write_target_is_directory_fails_honestly",
        ],
        "note": "The other seven tests are expected to **stay green** (see "
                "file header). This entry is the existence proof for the "
                "whole milestone.",
    },
    {
        "label": "M2 commit reverted to `mv` (rebuilds the first version's commit mechanism, length gate kept)",
        "file": "crates/core/src/remote.rs",
        "old": "         if cat \\\"$t\\\" > {q}; then rm -f \\\"$t\\\"; exit 0; \\",
        "new": "         if mv \\\"$t\\\" {q}; then exit 0; \\",
        "expect_fail": [
            "remote_write_through_symlink_updates_target_not_the_link",
            "remote_write_through_hardlink_preserves_link_count",
            "remote_write_dash_prefixed_filename_is_not_parsed_as_an_option",
            "remote_write_preserves_inode_and_mode_on_existing_file",
        ],
        "note": "Four verified regressions in mv semantics. The reviewer "
                "also pointed out a fifth: when the destination is an "
                "existing directory, `mv` silently moves the file into it "
                "instead of erroring, so "
                "`..._target_is_directory_fails_honestly` might also go red.",
    },
    {
        "label": "M3 length gate always true (the core guard stops working)",
        "file": "crates/core/src/remote.rs",
        "old": "[ \\\"$(wc -c < \\\"$t\\\")\\\" -eq \\\"$n\\\" ]",
        "new": "[ \\\"$n\\\" -eq \\\"$n\\\" ]",
        "expect_fail": ["remote_write_early_eof_is_rejected"],
        "note": "A real ssh disconnect = a clean EOF = `cat` returns 0, so without this gate, a half-written file gets committed.",
    },
    {
        "label": "M4 the temp file is not cleaned up when the gate rejects it",
        "file": "crates/core/src/remote.rs",
        "old": "{{ rm -f \\\"$t\\\"; exit 1; }}",
        "new": "{{ exit 1; }}",
        "expect_fail": ["remote_write_early_eof_is_rejected"],
        "note": "Only became guardable after the fix-up round added the "
                "\"no leftover in either the same directory or TMPDIR\" "
                "assertion; before that the reviewer explicitly flagged "
                "this as black-box unobservable.",
    },
    {
        "label": "M5 the success branch loses its explicit exit 0 (the exit code gets hijacked by the cleanup rm)",
        "file": "crates/core/src/remote.rs",
        "old": "then rm -f \\\"$t\\\"; exit 0; \\",
        "new": "then rm -f \\\"$t\\\"; \\",
        "expect_fail": ["remote_write_success_is_not_masked_by_a_failing_cleanup"],
        "note": "The reviewer injected a fake `rm` (always exit 1) and "
                "tested live: the content lands correctly, yet it reports "
                "\"Remote file untouched\" -- a lie, and one that also "
                "cascades into M75's baseline getting stuck at the old "
                "value, causing the next save to falsely report an "
                "external conflict.",
    },
    {
        "label": "M6 does not distinguish whether the temp file still exists on a commit failure (always claims kept)",
        "file": "crates/core/src/remote.rs",
        "old": "         elif [ -r \\\"$t\\\" ]; then echo \\\"staged copy kept at $t\\\" >&2; exit 3; \\",
        "new": "         elif true; then echo \\\"staged copy kept at $t\\\" >&2; exit 3; \\",
        "expect_fail": [],
        "note": "**Expected SURVIVED**, honestly recorded: no test can "
                "construct the narrow window of \"gate passes, but $t "
                "disappears before the commit\" (that would need an "
                "external delete inserted between two lines of the same "
                "script). The reviewer reproduced the mechanism itself in "
                "plain shell, but it is black-box unobservable in the "
                "existing test set.",
    },
    {
        "label": "M7 (trailing re-review) an unknown exit code also claims \"the file was never touched\"",
        "file": "crates/core/src/remote.rs",
        "old": "    if code != 0 && code != 1 {",
        "new": "    if false {",
        "expect_fail": ["remote_write_unknown_exit_code_does_not_promise_the_file_is_untouched"],
        "note": "Found by the trailing reviewer: the Rust side only "
                "special-cases code==3, and applies \"Remote file "
                "untouched\" to every other non-zero value (ssh's own "
                "255, a signal-killed -1, an OOM kill) -- but that claim "
                "only holds for the exit 1 the script itself emits. If the "
                "remote gets killed externally **mid-commit**, the code the "
                "client sees is neither 1 nor 3, yet we tell the user the "
                "file is fine -- **exactly the scenario this milestone "
                "exists for**.",
    },
]
