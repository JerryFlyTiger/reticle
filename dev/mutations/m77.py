# M77 -- wall-clock timeout for `remote::run()`.
#
# Source of this list: the reviewer designed six entries A-F from a cold
# read of the diff; the main conversation turned them into a runnable config
# and added M7. The reviewer stated outright that entries D/E/F **could not
# be guarded** against the version it read (no test would go red); they only
# became guardable after the fix-up round added message assertions and a
# large-payload test -- so the point of these three entries is to **verify
# the fix-up round really closed the gap**, not a routine check.
#
# How to run (two passes, different targets):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m77.py -p core \
#         --test-target ssh_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m77-lib.py -p core \
#         --test-target lib
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# ## Why the stdin entry lives in a separate config
#
# `write_file`'s remote command **produces almost no stdout** (just a short
# status/echo), so moving the stdin write back onto the calling thread
# **doesn't** cause the classic bidirectional-pipe deadlock -- the child
# process keeps consuming stdin, so the writer never blocks. Locking it up
# would need a command that both "emits a lot of stdout" and "needs to
# consume a lot of stdin" at the same time, and none of the nine remote
# primitives look like that. The fix-up round therefore added a
# `#[cfg(test)]` test in `remote.rs` that calls the private `run()` directly
# with `cat` (600 KB in both directions), and that's the entry that actually
# pins down the design decision of the stdin thread -- and it's a `--lib`
# target.
#
# **This fact is itself worth recording**: the spec at the time required
# that "the large-payload test being added must also be self-verifiable by
# turning red", and that assumption doesn't hold here; forcing it through
# would only have produced a test that looks like it guards something but
# actually doesn't.

PACKAGE = "core"
TEST_TARGET = "ssh_tests"

MUTATIONS = [
    {
        "label": "A `0` is no longer an escape hatch for \"disable the timeout\"",
        "file": "crates/core/src/remote.rs",
        "old": """    if secs <= 0.0 {
        None""",
        "new": """    if secs < 0.0 {
        None""",
        "expect_fail": ["remote_timeout_zero_disables_the_deadline"],
    },
    {
        "label": "B disables the deadline entirely (equivalent to before M77: waits forever once connected)",
        "file": "crates/core/src/remote.rs",
        "old": "    let deadline = remote_timeout().map(|d| Instant::now() + d);",
        "new": "    let deadline: Option<Instant> = None;",
        "expect_fail": [
            "remote_read_timeout_is_not_unbounded_wait",
            "remote_save_timeout_leaves_buffer_modified",
            "remote_save_timeout_after_remote_already_committed_is_an_honest_false_negative",
            "remote_file_exists_p_timeout_is_an_error_not_nil",
            "remote_file_directory_p_timeout_is_an_error_not_nil",
            "remote_timeout_actually_kills_the_ssh_client",
        ],
    },
    {
        "label": "C1 `is_dir` swallows an io-layer failure into false (timeout -> \"not a directory\")",
        "file": "crates/core/src/remote.rs",
        "old": """        &format!("test -d {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .map_err(|e| run_err(&e, "stat"))""",
        "new": """        &format!("test -d {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .or_else(|_| Ok(false))""",
        "expect_fail": ["remote_file_directory_p_timeout_is_an_error_not_nil"],
    },
    {
        "label": "C2 `exists` swallows an io-layer failure into false (timeout -> \"file does not exist\")",
        "file": "crates/core/src/remote.rs",
        "old": """        &format!("test -e {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .map_err(|e| run_err(&e, "stat"))""",
        "new": """        &format!("test -e {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .or_else(|_| Ok(false))""",
        "expect_fail": ["remote_file_exists_p_timeout_is_an_error_not_nil"],
    },
    {
        "label": "D remove write_file's dedicated timeout message, falls back to the generic run_err",
        "file": "crates/core/src/remote.rs",
        "old": """        if e.kind() == std::io::ErrorKind::TimedOut {""",
        "new": """        if false {""",
        "expect_fail": ["remote_save_timeout_leaves_buffer_modified"],
    },
    {
        "label": "E stdout/stderr not drained while waiting (the shape the doc comment says will deadlock)",
        "file": "crates/core/src/remote.rs",
        "old": """    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });""",
        "new": """    // MUTATION E: switch to "only read the pipes once the child has exited".
    // The .join() call shape is preserved; only *when* the read actually
    // happens moves to after the wait loop -- the exact design run()'s doc
    // comment says will deadlock.
    struct LazyPipe<R: std::io::Read>(R);
    impl<R: std::io::Read> LazyPipe<R> {
        fn join(mut self) -> Result<Vec<u8>, ()> {
            let mut buf = Vec::new();
            let _ = self.0.read_to_end(&mut buf);
            Ok(buf)
        }
    }
    let stdout_thread = LazyPipe(stdout_pipe);
    let stderr_thread = LazyPipe(stderr_pipe);""",
        "expect_fail": ["remote_read_large_payload_does_not_deadlock"],
    },
]
