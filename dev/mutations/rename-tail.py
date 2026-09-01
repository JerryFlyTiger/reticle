# Mutation list for the 2026-08-31 rename tail: the remote-save temp-file
# markers that the blanket `small_emacs` -> `reticle` substitution missed,
# because they were spelled with hyphens (`.se-save-`, `small-emacs-save`)
# rather than underscores.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/rename-tail.py \
#         -p core --test-target ssh_tests
#
# Designed by the reviewer that cold-read the tail diff; run from the main
# conversation, because an implementer must not verify its own fix.
#
# What these are for. The rename touched the product and its tests in the same
# pass, so both sides could have been changed consistently *wrong* and every
# gate would still be green -- fmt, clippy and the full suite all passed while
# the marker was still called `.se-save-`. The only question worth asking is
# whether the tests actually observe the string the product emits, or merely
# agree with themselves. Each mutation below breaks the agreement on exactly
# one side and names the test that must go red.
#
# M1/M2 desynchronise the product from the tests. M3 desynchronises the test
# helper from the product. M4-M6 desynchronise the `sh` shim globs, which is
# the subtler class: a shim whose `case` no longer matches the write command
# falls through to its unconditional branch, so the fault it exists to inject
# is never injected and the test passes for the wrong reason.

# Result, 2026-09-01: 5 of 6 FAIL as expected. **M2 SURVIVED**, and it is a
# genuine gap, not a mutation that missed -- all six were pre-checked to match
# exactly once before the run. The test that should have caught it filtered
# TMPDIR entries by the `reticle-save.` prefix before asserting the list was
# empty, which is true by construction once the product stops using that
# prefix. The filter has since been removed so the assertion catches a leftover
# under any name, but the fallback file's *name* remains unobservable: a
# successful save deletes it. Recorded in the test comment rather than papered
# over. Re-running this list should still report M2 as SURVIVED.

PACKAGE = "core"
TEST_TARGET = "ssh_tests"

MUTATIONS = [
    {
        "label": "M1 product primary marker back to the old name",
        "file": "crates/core/src/remote.rs",
        "old": "{}/.reticle-save-{}",
        "new": "{}/.se-save-{}",
        "test": "remote_write_target_is_directory_fails_honestly",
        "note": (
            "The staged copy written next to the target file on the remote "
            "host. That test asserts a leftover staged copy IS found after a "
            "failed write; the helper looks for `.reticle-save-`, so if the "
            "product emits the old name the leftover becomes invisible and "
            "the assertion fails."
        ),
    },
    {
        "label": "M2 product TMPDIR fallback marker back to the old name",
        "file": "crates/core/src/remote.rs",
        "old": "/reticle-save.$$",
        "new": "/small-emacs-save.$$",
        "test": "remote_write_falls_back_to_tmpdir_when_directory_is_read_only",
        "note": (
            "The fallback used when the target's own directory is not "
            "writable. The test filters TMPDIR entries by the "
            "`reticle-save.` prefix."
        ),
    },
    {
        "label": "M3 test helper filter back to the old name",
        "file": "crates/core/tests/ssh_tests.rs",
        "old": '.contains(".reticle-save-")',
        "new": '.contains(".se-save-")',
        "test": "remote_write_target_is_directory_fails_honestly",
        "note": (
            "Mirror of M1 on the test side: the helper now searches for a "
            "substring the product never emits, so it reports no leftovers."
        ),
    },
    {
        "label": "M4 shim glob for exit-code injection back to the old name",
        "file": "crates/core/tests/ssh_tests.rs",
        "old": r'*.reticle-save-*) /bin/sh -c \"$1\"; exit {} ;;',
        "new": r'*.se-save-*) /bin/sh -c \"$1\"; exit {} ;;',
        "test": "remote_write_unknown_exit_code_does_not_promise_the_file_is_untouched",
        "note": (
            "The shim injects exit code 42 only on the write command. If its "
            "`case` stops matching, the command runs normally, the save "
            "succeeds, and the expected ERROR result never appears."
        ),
    },
    {
        "label": "M5 shim glob for hang-on-write back to the old name",
        "file": "crates/core/tests/ssh_tests.rs",
        "old": "*.reticle-save-*) exec sleep 5 ;;",
        "new": "*.se-save-*) exec sleep 5 ;;",
        "test": "remote_save_timeout_leaves_buffer_modified",
        "note": "Without a match the write never hangs, so no timeout occurs.",
    },
    {
        "label": "M6 shim glob for commit-then-hang back to the old name",
        "file": "crates/core/tests/ssh_tests.rs",
        "old": r'*.reticle-save-*) /bin/sh -c \"$1\"; exec sleep 5 ;;',
        "new": r'*.se-save-*) /bin/sh -c \"$1\"; exec sleep 5 ;;',
        "test": "remote_save_timeout_after_remote_already_committed_is_an_honest_false_negative",
        "note": (
            "Same shape as M5, but this shim commits first and then hangs, "
            "which is the case where a timeout is an honest false negative."
        ),
    },
]
