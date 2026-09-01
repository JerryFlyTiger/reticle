# M75 -- remote saves no longer silently clobber someone else's changes either.
#
# Source of this list: the reviewer designed 9 entries (M1-M9) from a cold
# read of the diff. **M1 and M2 are the two entries the reviewer explicitly
# predicted "will not FAIL"** -- it alleges that those two tests would still
# pass even if the guard under test were broken (whatever makes them pass
# has been substituted by something else). The main conversation ran these
# two first to verify whether the allegation holds, then decided on a fix.
#
# How to run:
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m75.py -p core \
#         --test-target ssh_tests
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).

PACKAGE = "core"
TEST_TARGET = "ssh_tests"

MUTATIONS = [
    {
        "label": "M1 the guard swallows the error on a read failure and writes anyway (reviewer predicts this will not FAIL)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "            match crate::remote::read_file(&rp).map_err(|e| i.error(e))? {",
        "new": "            match crate::remote::read_file(&rp).unwrap_or(None) {",
        "expect_fail": ["remote_save_guard_failure_refuses_instead_of_writing"],
        "note": "The reviewer's allegation: that test uses a shim where "
                "every command exits 255, and the same shim also makes the "
                "downstream write_file fail, so even if the guard swallows "
                "the error, save-buffer still returns ERROR and the file "
                "still never gets touched -- both assertions still hold.",
    },
    {
        "label": "M2 digest not refreshed after a write, falls back to Unknown (reviewer predicts this will not FAIL)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """                crate::buffer::DiskState::RemoteContent {
                    digest: crate::buffer::content_digest(text.as_bytes()),
                    len: text.len() as u64,
                }
            } else {""",
        "new": """                crate::buffer::DiskState::Unknown
            } else {""",
        "expect_fail": ["remote_save_twice_in_a_row_does_not_false_positive"],
        "note": "The reviewer's allegation: falling back to Unknown lands "
                "in the conflict check's `_ => false` arm, so the second "
                "save still succeeds, identical to the observable result "
                "of a correctly refreshed digest.",
    },
    {
        "label": "M3 remote: someone else creating a file of the same name first no longer counts as a conflict",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """                        crate::buffer::DiskState::RemoteContent { .. } => disk_state != observed,
                        crate::buffer::DiskState::Absent => true,""",
        "new": """                        crate::buffer::DiskState::RemoteContent { .. } => disk_state != observed,
                        crate::buffer::DiskState::Absent => false,""",
        "expect_fail": ["remote_new_file_conflict_when_someone_else_creates_it_first"],
    },
    {
        "label": "M4 remote: a different digest no longer counts as a conflict either (the guard's main check)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        crate::buffer::DiskState::RemoteContent { .. } => disk_state != observed,",
        "new": "                        crate::buffer::DiskState::RemoteContent { .. } => false,",
        "expect_fail": [
            "remote_save_refuses_to_clobber_external_change",
            "remote_save_ack_binds_observed_state_not_path",
            "remote_save_twice_in_a_row_does_not_false_positive",
        ],
    },
    {
        "label": "M5 remote: the file being deleted after opening no longer counts as a conflict",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                    if matches!(disk_state, crate::buffer::DiskState::RemoteContent { .. })",
        "new": "                    if matches!(disk_state, crate::buffer::DiskState::Absent)",
        "expect_fail": ["remote_save_conflict_when_deleted_remotely"],
    },
    {
        "label": "M6 the baseline is not written when opening a remote file (buffer stays at Buffer::new's Unknown)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """    buf.borrow_mut().disk_state = disk_state;
    buf.borrow_mut().file = Some(full.to_string());""",
        "new": """    buf.borrow_mut().file = Some(full.to_string());""",
        "expect_fail": [
            "remote_save_refuses_to_clobber_external_change",
            "remote_save_conflict_when_deleted_remotely",
            "remote_new_file_conflict_when_someone_else_creates_it_first",
        ],
        "note": "Verifies the reviewer's finding 3: if a remote buffer "
                "stays at Unknown, the guard silently fails. This entry "
                "confirms existing tests catch that consequence (even "
                "though no production path currently causes it). Note that "
                "`old` only occurs at the find_file_remote site (the other "
                "occurrence at :920 has different indentation).",
    },
    {
        "label": "M7 ack is not recorded on a refusal (a retry can never pass)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    b.borrow_mut().save_conflict_ack = Some((path.to_string(), observed));\n    Err(i.error(format!(",
        "new": "    Err(i.error(format!(",
        "expect_fail": ["remote_save_second_attempt_overwrites_after_warning"],
        "note": "An extracted shared helper. If ack isn't recorded, \"save again after being warned\" would always be refused.",
    },
    {
        "label": "M8 ack binds only to the path, not to the observed state",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    let already_acked = b.borrow().save_conflict_ack == Some((path.to_string(), observed));",
        "new": "    let already_acked = b.borrow().save_conflict_ack.as_ref().map(|(p, _)| p.as_str()) == Some(path);",
        "expect_fail": ["remote_save_ack_binds_observed_state_not_path"],
        "note": "The property designed into M62's second version: if a "
                "second, unrelated external change lands after a refusal, "
                "retrying the save must still be refused, and must not "
                "clobber a change the user was never warned about.",
    },
    {
        "label": "M9 (trailing re-review) content_digest degenerates to a constant (only len is left guarding it)",
        "file": "crates/core/src/buffer.rs",
        "old": """    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {""",
        "new": """    let mut hash = FNV_OFFSET_BASIS;
    if bytes.len() < usize::MAX {
        return 0;
    }
    #[allow(unreachable_code)]
    for &b in bytes {""",
        "expect_fail": ["remote_save_same_length_different_content_is_detected"],
        "note": "Found by the trailing reviewer: every existing fixture's "
                "\"baseline vs. external change\" differs in length, so "
                "RemoteContent's len field alone is enough to hold up every "
                "assertion, and digest's content-sensitivity **has never "
                "been tested** -- and \"content changed but length "
                "unchanged\" is exactly the reason the `ls -l` approach was "
                "rejected in the first place. The first run was SURVIVED; "
                "it only flipped red after a same-length, different-content "
                "test was added.",
    },
    {
        "label": "M10 (trailing re-review) the already-acked short-circuit stops working (records ack but still doesn't pass)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "    if already_acked {\n        return Ok(());\n    }\n",
        "new": "",
        "expect_fail": ["remote_save_second_attempt_overwrites_after_warning"],
        "note": "Different from M7: M7 tests \"was ack recorded at all\", "
                "this entry tests \"once recorded, is the comparison "
                "actually used to short-circuit\". This shares the same "
                "helper locally, so save_kill_hooks_tests's retry test is "
                "expected to go red at the same time.",
    },
    {
        "label": "M11 (trailing re-review) the initial baseline digest is computed wrong when opening a file",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """        Some(text) => crate::buffer::DiskState::RemoteContent {
            digest: crate::buffer::content_digest(text.as_bytes()),""",
        "new": """        Some(text) => crate::buffer::DiskState::RemoteContent {
            digest: 0xdead,""",
        "expect_fail": ["remote_save_no_false_positive_when_untouched"],
        "note": "M1-M8 all touch the conditional branches and ack; none of them touch the line where find_file_remote constructs the initial baseline.",
    },
]
