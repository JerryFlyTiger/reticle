# M77 -- the stdin guard, watched from the `--lib` target.
#
# The reason for a separate config is in `m77.py`'s header: `write_file`'s
# remote command produces almost no stdout, so the bidirectional-pipe
# deadlock cannot be reached through the public API; what actually pins down
# the stdin thread is the unit test in `crates/core/src/remote.rs`'s
# `#[cfg(test)]` that calls the private `run()` directly with `cat` (600 KB
# in both directions), and `dev/mutate.py` only takes one target per run.
#
# How to run:
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m77-lib.py -p core \
#         --test-target lib
#
# **Expected outcome is `hang`, not `FAIL`**: under the pre-M77 shape,
# `cat`'s stdout fills up with nobody draining it -> cat blocks on writing ->
# we block on writing stdin -> both sides wait on each other, and **this
# happens before entering the wait loop**, so the timeout branch hasn't even
# started counting and will never trigger. This is exactly why "stdin also
# needs its own thread": wrapping just the wait isn't enough.
#
# ## The first version of this mutation was wrong (2026-08-23, a mistake the
#    main conversation made itself)
#
# The first version only replaced the `let stdin_thread = ...` block with a
# synchronous `write_all`, **without moving the reader thread's position**.
# As a result, after the mutation, stdout was still drained the whole time,
# `cat` never blocked, and the test PASSed in 0.18 seconds -- the harness
# honestly reported `SURVIVED`, and the correct reading was "**this mutation
# didn't reproduce the failure shape it claimed to reproduce**", not "the
# test can't guard it".
#
# Same category of lesson as the five items in `dev/fake-ssh.py`'s header:
# **a mutation is itself a model that can be wrong**. "SURVIVED" has two
# possible causes -- the guard really isn't watched, or the mutation never
# hit the guard at all -- and the harness cannot tell them apart. Before
# interpreting a SURVIVED, first ask "did my mutation actually reproduce
# that failure shape".

PACKAGE = "core"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "F stdin written synchronously, with nobody draining stdout in the meantime (the true pre-M77 shape)",
        "file": "crates/core/src/remote.rs",
        "old": """    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });
    let stdin_thread = stdin_data.map(|data| {
        // Borrowed from the caller; owned so it can move into the thread.
        let data = data.to_vec();
        let mut stdin = child.stdin.take().expect("piped stdin");
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
            // `stdin` drops here, closing the pipe -- the remote `cat`
            // sees EOF and finishes. Load-bearing: see the doc comment
            // above.
        })
    });""",
        "new": """    // MUTATION F: the pre-M77 shape -- write all of stdin synchronously on the
    // calling thread, during which **nobody is draining stdout/stderr** (the
    // reader threads are moved to after the write completes).
    if let Some(data) = stdin_data {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let _ = stdin.write_all(data);
    }
    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });
    let stdin_thread: Option<std::thread::JoinHandle<()>> = None;""",
        "expect": "hang",
        "timeout": 90,
        "test": "run_stdin_and_stdout_both_large_does_not_deadlock",
    },
    {
        # Trailing re-review finding 1. The original display function only
        # did "round to 3 decimal places", and the doc claimed that pinned
        # the width cap at `"86400"` (5 characters) -- **only true at
        # integer boundary values**. `12345.6789` prints as `12345.679`
        # (9 characters), and paired with `delete`/`rename`, two 6-character
        # verbs, that's 73 characters, one over the 72-character budget, and
        # the overflow lands exactly on the closing phrase
        # "it may still be running there".
        #
        # Worse: the "budget guard" test added at the time itself missed this
        # shape (its value list only had integer boundaries and pure
        # decimals), so that regression **would not have been caught**. The
        # fix drops the decimal outright at 100 seconds and above, and adds
        # `12345.6789` / `86399.9994` / `99.9994` to the value list. This
        # mutation reverts the fix, proving the added values really do guard
        # against it.
        "label": "G display seconds no longer drops the decimal for large values (message overflows the echo-area budget)",
        "file": "crates/core/src/remote.rs",
        "old": "    if rounded.fract() == 0.0 || rounded >= 100.0 {",
        "new": "    if rounded.fract() == 0.0 {",
        "expect_fail": ["timeout_message_text_fits_the_echo_area_budget"],
    },
]
