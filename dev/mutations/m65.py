# M65 -- LSP waits must be bounded, and no orphan server left behind on failure.
#
# Source of this list: the reviewer designed 8 entries from a cold read of
# the diff, but that batch was written against the code as it was **before
# the fix-up round**; two fix-up rounds changed `need_seconds` to
# `need_wait_duration`, removed `.max(0.0)`, widened the scope of
# `condition-case`, and replaced the whole PID-file mechanism with the new
# `lsp-connection-pid`. So this file is a rewrite by the main conversation
# against the current state: the reviewer's **observation-point design** is
# preserved, updated to the current code, plus the fix points added by the
# fix-up round (M4/M5 -- those two spots didn't exist yet when it read the
# diff).
#
# How to run (everything is the same crate and test file, no need to split
# into passes):
#     dev/mutate.py --config dev/mutations/m65.py -p core --test-target lsp_tests
#
# **Must be run as a background task, not in the foreground**: the M64 run
# hit the 10-minute limit in the foreground and got SIGKILLed, `finally`
# didn't run, and the source code was left in a mutated state (see the M64
# section in PLAN.md). This list has two more entries expected to HANG,
# making it even easier to run out the clock.
#
# ## The lesson taught by M6: a mutation's "observability" can itself be
#    load-dependent
#
# M6 initially survived when run as part of the full batch but correctly
# HANGed when run alone, and **both runs recompiled** (so this isn't the
# false conclusion of "the mutation never made it into the binary"). The
# real reason is that the mutated code **only waits forever if the fake
# server keeps beating the budget every round**: the old `CHATTY_SCRIPT`
# forked a `wc -c` and then a `sleep` every round, and when the machine gets
# busy, a single round could exceed the test's 0.3-second budget, so even
# the code with its guard removed would time out normally and the test would
# stay green.
#
# The fix is on the test side: message length is now computed once outside
# the loop (no more per-message fork), and the test's budget was widened
# from 0.3 seconds to 2 seconds. Together, this stretched the gap between
# "message interval" and "budget" from ~6x to ~40x. **Lesson: when designing
# a mutation's observation point, ask "what timing conditions does this
# observation require to hold" -- an observation point that only holds under
# a race will silently report "the guard doesn't exist" under load.**
#
# For this reason `dev/mutate.py` also gained instrumentation: every entry
# now prints how long it took and which crates got recompiled, and flags
# "this entry's conclusion is untrustworthy" directly if nothing recompiled
# (all of `crates/core/lisp/*.el` gets compiled into the binary via
# `include_str!`, so elisp mutations also require a recompile to take
# effect).
#
# ## Two entries expected to "hang" rather than "fail an assertion"
#
# The observable consequence of reverting M5 and M6 is that **the test never
# returns**, not a red failure -- this is exactly the kind of failure mode
# this milestone is fixing, so its mutations naturally look like this too.
# mutate.py's `expect: "hang"` exists exactly for this (lesson 5 in the file
# header); both entries were given individually shorter timeouts so one
# hanging doesn't eat up ten minutes.
#
# ## One entry expected to survive (honestly recorded, not a gap)
#
# M7: the `(max remaining 0)` that `lsp--await` passes to `lsp-wait` is pure
# defense in depth -- the Rust side's `need_wait_duration`
# (`crates/elisp/src/lsp.rs:303`) already converts any `<= 0` to
# `Duration::ZERO`. Removing this elisp-side `max` produces identical
# behavior. Marked `expect: survived` so "this layer has no target" becomes
# a visible fact in the list, rather than a surprise after running it.
#
# ## One fix point with no target, and no test forced into existence for it
#
# The section of `condition-case` covering the `initialized` notification
# (the review finding discussed in the comment at `lsp.el:676-700`) **has no
# corresponding test**: observing it would need a fake server that dies the
# instant it finishes answering initialize, and no existing fixture does
# that. Honestly recorded here, not claimed as covered.

PACKAGE = "core"
TEST_TARGET = "lsp_tests"

MUTATIONS = [
    {
        "label": "M1 Drop no longer kills (the destructor waits after only closing stdin)",
        "file": "crates/elisp/src/lsp.rs",
        "old": """        let _ = self.child.kill();
        self.stdin = None;
        let _ = self.child.wait();""",
        "new": """        self.stdin = None;
        let _ = self.child.wait();""",
        "test": "lsp_connection_killed_purely_by_drop_when_unreachable",
        # The live result is HANG rather than FAIL, and that's the correct
        # observable consequence: after the test fixture was switched to "an
        # infinite loop that never reads stdin at all", `Drop` without the
        # kill stalls forever on `child.wait()` -- exactly the failure mode
        # this milestone is guarding against. The first version's fixture
        # was `cat >/dev/null`, which exits on its own once stdin is closed,
        # so what that test observed as "the process is gone" back then was
        # the effect of closing the pipe, not of the kill, and the mutation
        # survived as a result. Only after swapping the fixture did it get a
        # real target.
        "expect": "hang",
        "timeout": 120,
    },
    {
        "label": "M2 a timeout is mistaken for the server dying",
        "file": "crates/elisp/src/lsp.rs",
        "old": """                Ok(None) => Ok(Value::Nil),""",
        "new": """                Ok(None) => Ok(dead_value(i)),""",
        "test": "lsp_wait_timeout_returns_nil_without_message_and_stays_alive",
    },
    {
        "label": "M3 the compatibility path for TIMEOUT = nil is changed to 0 seconds",
        "file": "crates/elisp/src/lsp.rs",
        "old": """            Value::Nil => None,""",
        "new": """            Value::Nil => Some(Duration::ZERO),""",
        "test": "lsp_wait_returns_message_with_or_without_timeout_arg",
    },
    {
        "label": "M4 reverts to the Duration constructor that can panic",
        "file": "crates/elisp/src/lsp.rs",
        "old": """    Ok(Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX))""",
        "new": """    Ok(Duration::from_secs_f64(secs))""",
        "test": "lsp_wait_extreme_timeout_values_never_panic",
    },
    {
        "label": "M5 remove the negative-value guard (keep only the NaN check)",
        "file": "crates/elisp/src/lsp.rs",
        "old": """    if secs.is_nan() || secs <= 0.0 {""",
        "new": """    if secs.is_nan() {""",
        # negative -> try_from_secs_f64 returns Err -> Duration::MAX -> waits
        # 58.4 billion years for a deaf server.
        "test": "lsp_wait_extreme_timeout_values_never_panic",
        "expect": "hang",
        "timeout": 90,
    },
    {
        "label": "M6 budget resets every round (a chatty server can stall indefinitely)",
        "file": "crates/core/lisp/lsp.el",
        "old": """        (let* ((remaining (- deadline (float-time)))""",
        "new": """        (let* ((remaining (or timeout lsp-initialize-timeout))""",
        "test": "lsp_await_timeout_budget_survives_a_chatty_never_answering_server",
        "expect": "hang",
        "timeout": 90,
    },
    {
        "label": "M7 remove elisp-side (max remaining 0) (expected to survive: already blocked on the Rust side)",
        "file": "crates/core/lisp/lsp.el",
        "old": """               (msg (lsp-wait (lsp--client-conn client) (max remaining 0))))""",
        "new": """               (msg (lsp-wait (lsp--client-conn client) remaining)))""",
        "test": "lsp_await_timeout_budget_survives_a_chatty_never_answering_server",
        "expect": "survived",
    },
    {
        # M9/M11 were added at the trailing re-review's specific request: the
        # first 8 entries don't cover "the shape of the value returned on
        # death" or "whether pid() really returns that process's id". Both
        # are claims specific to M65.
        "label": "M9 server death is reported as a timeout's nil",
        "file": "crates/elisp/src/lsp.rs",
        "old": """                Ok(Some(LspEvent::Died)) => Ok(dead_value(i)),""",
        "new": """                Ok(Some(LspEvent::Died)) => Ok(Value::Nil),""",
        # M65 gave nil a second meaning (timeout), so reporting death as nil
        # would make `lsp--await` lie to the user by saying "timed out". The
        # trailing re-review predicted this entry would survive; after
        # adding an assertion on the return shape, it should FAIL.
        "test": "lsp_wait_extreme_timeout_values_never_panic",
    },
    {
        "label": "M11 pid() returns an off-by-one wrong PID",
        "file": "crates/elisp/src/lsp.rs",
        "old": """        self.child.id()""",
        "new": """        self.child.id().wrapping_add(1)""",
        # The trailing re-review predicted: under a one-sided assertion that
        # only checks "died afterward", this entry would survive (an
        # off-by-one PID is usually not a live process anyway). After adding
        # the two-sided assertion "must be alive before the kill", it should
        # FAIL.
        "test": "lsp_connection_killed_purely_by_drop_when_unreachable",
        "timeout": 120,
    },
    {
        "label": "M8 no longer kills the connection when the handshake fails",
        "file": "crates/core/lisp/lsp.el",
        "old": """       (lsp-kill conn)
""",
        "new": "",
        "test": "lsp_connect_deaf_server_times_out_and_kills_the_connection",
        "timeout": 120,
    },
]
