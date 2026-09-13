# M135 (the fixed-sleep false-red family in `lsp_autostart_tests.rs') mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m135.py \
#         -p core --test-target lsp_autostart_tests
#
# Two test targets are involved, so several entries carry their own
# `test_target': Part A's primitive is exercised from `lsp_tests', everything
# else from `lsp_autostart_tests'.
#
# One deletion-style entry per FEATURE, per M117's rule. M135's effects are
# numbered E1..E8 here, NOT F1..F8: `F<N>' is already taken, twice over, in the
# very files this list mutates -- it numbers the test sections in
# `lsp_autostart_tests.rs' (`F1 (high)', `F8 review fix', ...) and it numbers
# this milestone's own fix-round items in the comments there (`M135 fix round
# (F2 ...)', `(F4 ...)'). A first cut of this file used F1..F8 for effects, and
# seven of the eight collided with an existing, unrelated meaning in the same
# files. Caught by the second trailing cold read.
#
#   E1  the delivered-event counter existing at all (D1), and `Died' counting
#       as a delivery (D2)
#   E2  the six fixed sleeps being gone -- i.e. the guard that keeps them gone
#       (D3)
#   E3  F9's wait being NON-consuming (D4)
#   E4  F10's fixture only dying after it has read the whole framed
#       `initialize', plus F10's counter-based wait (D5, D6)
#   E5  the deaf-server test no longer racing its own 1s deadline (D7)
#   E6  the settle-then-final-drain that closes the "extra frame sent AFTER
#       the awaited one" hole, at BOTH of its call sites (D8, D10)
#   E7  the memory ordering of the counter: add AFTER send, not before (D9)
#   E8  the fix round's 10s -> 30s ceiling on Part A's own waits (D11)
#
# D10 and D11 were added by the trailing cold read, and the reason they were
# missing is worth keeping: F2 is applied at TWO independent call sites and the
# first cut of this file covered only the helper, while the header above
# claimed one entry per effect. That is the M116 shape -- some of a
# milestone's sites covered, the rest not, with a completeness claim on top.
# A completeness rule does not check itself.
#
# One effect deliberately has NO entry, because it cannot be one -- and it is
# not in the E list above either, since it arrived in the fix round: the
# widening of the guard's marker from `std::thread::sleep(' to `thread::sleep('.
# Showing that it does anything needs TWO coordinated edits (add
# `use std::thread;' AND introduce an unqualified call), while an entry here is
# a single replacement. Measured by hand instead, on 2026-09-13, with a backup
# plus a targeted edit plus `touch' to restore:
#
#   widened marker (shipped):  FAILED, naming
#     (458, "        thread::sleep(std::time::Duration::from_millis(1500));")
#   pre-widening marker, same line:  ok. 1 passed
#
# So the narrow marker let an unqualified long sleep through in silence, and the
# widening is load-bearing rather than decorative. The second trailing cold read
# repeated the experiment independently and got the same pair, at line 459 --
# its own scratch edit sat one line higher. Recorded here because "no entry" and
# "no thought" are indistinguishable to a later reader.
#
# D6 through D11 are entered with `"expect": "survived"' and a reason. They are
# in the list because M117's rule is that an omission is indistinguishable, to
# anyone reading only this file, from nobody having thought about it.
#
# What this milestone's effect fundamentally is, and why the usual deletion
# question reads oddly here: reverting M135 wholesale does NOT turn any test
# red on an idle machine -- that is the defect being fixed. The observable is
# the opposite of a failing test: it is the four-way parallel run of the same
# binary, and the full gate itself.
#
#   baseline (HEAD 8f51dea): 4x parallel -> 17/100 red; and
#                            `cargo test --workspace --no-fail-fast' ->
#                            2630 passed / 1 FAILED
#                            (idle_tick_completes_a_near_deadline_handshake_
#                             before_reaping_it)
#   after M135:              4x parallel -> 104/104 green (twice, plus 40/40
#                            for F10 alone); full gate 3 runs ->
#                            2635 passed / 0 failed each, at load 9-13.8
#
# D3 is what makes F2 answerable by a NAMED test rather than only by that
# experiment: it reinstates a long sleep and the guard must go red.
#
# Every entry is replacement-style. This project's measured lesson (M83's
# `C-x s' prefix, M85's four-in-one-round) is that insertion-style mutations
# miss their target. D3 is a replacement whose new text happens to be longer
# than its old text; that is not the failure mode the rule is about -- the
# guard reads the file as TEXT, so a line it can see cannot be shadowed away.

MUTATIONS = [
    {
        "label": 'D1 the delivered counter always reads 0 (the whole of Part A)',
        "file": "crates/elisp/src/lsp.rs",
        "old": '        self.delivered.load(std::sync::atomic::Ordering::SeqCst)',
        "new": '        0',
        "test": 'events_delivered_starts_at_zero_and_increments_when_a_message_arrives',
        "test_target": "lsp_tests",
        # F9 and F10 both wait on this counter, so with it stuck at 0 they
        # burn their full 30s ceiling before panicking. Generous cap.
        "timeout": 900,
    },
    {
        "label": 'D2 `Died\' is delivered but not counted',
        "file": "crates/elisp/src/lsp.rs",
        "old": """                        if tx.send(LspEvent::Died).is_ok() {
                            delivered_reader.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }""",
        "new": """                        if tx.send(LspEvent::Died).is_ok() {
                            // D2: a Died delivery no longer bumps the counter.
                        }""",
        "test": 'events_delivered_counts_died_as_an_event',
        "test_target": "lsp_tests",
        "timeout": 900,
    },
    {
        "label": 'D3 a 1500ms sleep is back in the file (does the guard see it?)',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": 'fn pump_until(i: &mut Interp, pred_elisp: &str, what: &str) {',
        "new": """fn pump_until(i: &mut Interp, pred_elisp: &str, what: &str) {
    std::thread::sleep(std::time::Duration::from_millis(1500));""",
        "test": 'guard_no_long_thread_sleeps_in_this_file',
    },
    {
        "label": 'D4 F9 pumps while waiting (consuming the very reply under test)',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '        let n: i64 = ok(&mut i, "(lsp-events-delivered test--f9-conn)")',
        "new": '        let n: i64 = ok(&mut i, "(progn (lsp-process-pending-all) (lsp-events-delivered test--f9-conn))")',
        "test": 'idle_tick_completes_a_near_deadline_handshake_before_reaping_it',
    },
    {
        "label": 'D5 F10 waits on bare `lsp-live-p\' (a cached flag nothing flips)',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '        let n: i64 = ok(&mut i, "(lsp-events-delivered test--f10-conn)")',
        "new": '        let n: i64 = ok(&mut i, "(if (lsp-live-p test--f10-conn) 0 1)")',
        "test": 'dead_process_before_deadline_is_reaped_via_liveness_not_deadline',
        "timeout": 900,
    },
    {
        "label": 'D6 F10\'s server dies immediately again (EPIPE on the `initialize\' write)',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '        DIES_AFTER_READING_INITIALIZE_SCRIPT,',
        "new": '        "#!/bin/sh\\nexit 1\\n",',
        "test": 'dead_process_before_deadline_is_reaped_via_liveness_not_deadline',
        # Probabilistic: the implementer measured the intermediate
        # `sh -c "read line; exit 1"' shape red 1 time in 40 under 4x parallel
        # load, and the original `exit 1' shape red 1/4 under the same load.
        # An idle single run may well stay green, so a SURVIVED here is not a
        # coverage gap -- it means the machine was not loaded enough. The
        # honest observable for this one is the parallel experiment named in
        # this file's header.
        "expect": "survived",
        "timeout": 900,
    },
    {
        "label": 'D7 the deaf-server test goes back to racing its own 1s deadline',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '    ok(&mut i, "(setq lsp-autostart-timeout 30)");',
        "new": '    ok(&mut i, "(setq lsp-autostart-timeout 1)");',
        "test": 'deaf_server_is_reaped_after_the_deadline_with_no_retry',
        # Same shape as D6: the 5x100ms responsiveness loop has to overrun 1s
        # of wall clock before the entry is reaped early, which needs load.
        # Declared survived rather than left out.
        "expect": "survived",
    },
    {
        "label": 'D8 the settle + final drain pass is gone (an extra frame sent after the awaited one is never seen)',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '    std::thread::sleep(std::time::Duration::from_millis(50));\n    total += drain_frames(i, conn_expr, method);\n    total\n}',
        "new": '    total\n}',
        "test": 'happy_path_spawns_attaches_and_backfills',
        # Declared survivor, and it was measured twice: by hand during the fix
        # round (all 26 tests stayed green with the settle and final pass cut
        # out) and by this entry. The hole it closes needs a hypothetical bug --
        # a completion callback emitting a stray frame right AFTER the didOpen --
        # so nothing in the current tree can turn it red. Kept because the fixed
        # 1500ms sleep this milestone removed DID cover that case, which makes
        # this a deliberate piece of defensive coverage rather than an oversight.
        #
        # A first cut of this entry renamed the function instead, which only
        # broke its call sites: the target did not compile and the runner
        # recorded FAIL. A compile error is not a mutation result -- it says
        # nothing about coverage. Replaced with a mutation that actually lands.
        "expect": "survived",
    },
    {
        "label": 'D9 the counter is bumped BEFORE the send, not after',
        "file": "crates/elisp/src/lsp.rs",
        "old": '                    Ok(Some(s)) => {\n                        if tx.send(LspEvent::Message(s)).is_err() {\n                            break;\n                        }\n                        // M135: bump the counter ONLY after `send`\n                        // returns Ok -- so "count >= k" implies "k events\n                        // are already sitting in the channel, ready for\n                        // `lsp-poll` to pick up right now". Doing it the\n                        // other way around (add then send) would let a\n                        // waiter see the count go up before the message\n                        // is actually enqueued, and fall through too\n                        // early.\n                        delivered_reader.fetch_add(1, std::sync::atomic::Ordering::SeqCst);\n',
        "new": '                    Ok(Some(s)) => {\n                        delivered_reader.fetch_add(1, std::sync::atomic::Ordering::SeqCst);\n                        if tx.send(LspEvent::Message(s)).is_err() {\n                            break;\n                        }\n                        // M135: bump the counter ONLY after `send`\n                        // returns Ok -- so "count >= k" implies "k events\n                        // are already sitting in the channel, ready for\n                        // `lsp-poll` to pick up right now". Doing it the\n                        // other way around (add then send) would let a\n                        // waiter see the count go up before the message\n                        // is actually enqueued, and fall through too\n                        // early.\n',
        "test": 'events_delivered_starts_at_zero_and_increments_when_a_message_arrives',
        "test_target": "lsp_tests",
        # The window this opens (count visible before the message is in the
        # queue) is nanoseconds wide, against a 5ms polling interval -- five
        # to six orders of magnitude apart. No test in this repo can make it
        # observable, and pretending otherwise would be the "mutation list
        # that lies" failure mode. Recorded as survived, with the reason,
        # because the ordering is load-bearing for the docstring's claim.
        # Note this moves the add; it does not add a second one, which would
        # go red for double-counting instead of for the ordering.
        "expect": "survived",
        "timeout": 900,
    },
    {
        "label": 'D10 the settle + final classification pass is gone at the SECOND call site',
        "file": "crates/core/tests/lsp_autostart_tests.rs",
        "old": '    std::thread::sleep(std::time::Duration::from_millis(50));\n    ok(&mut i, classify_one_pass);\n',
        "new": '    // D10: settle + final classification pass removed at this call site.\n',
        "test": 'did_change_cannot_precede_did_open',
        # F2 was applied at TWO independent call sites -- `drain_frames_until'
        # (D8) and, separately, inline in this test. The first cut of this file
        # carried an entry for the helper only. That is the M116 shape exactly:
        # one of a milestone's sites covered, the other not, and the header
        # claiming one entry per feature. Caught by the trailing cold read.
        # Same expectation and same reason as D8: the hole needs a
        # hypothetical bug (a stray frame emitted AFTER the didOpen) to be
        # visible, so nothing here can turn it red.
        "expect": "survived",
    },
    {
        "label": 'D11 Part A\'s one wait ceiling goes back to the 10s that was observed timing out',
        "file": "crates/core/tests/lsp_tests.rs",
        "old": 'const EVENTS_DELIVERED_CEILING: std::time::Duration = std::time::Duration::from_secs(30);',
        "new": 'const EVENTS_DELIVERED_CEILING: std::time::Duration = std::time::Duration::from_secs(10);',
        "test": 'events_delivered_starts_at_zero_and_increments_when_a_message_arrives',
        "test_target": "lsp_tests",
        # The fix-round change F1 was a 10s -> 30s ceiling, after the first cold
        # read actually watched two of these tests time out together at 10s
        # while three `cargo test --workspace' runs shared the machine (load
        # 4.19-6.65), green on three immediate retries. Probabilistic like D6/D7:
        # an idle single run at 10s almost certainly stays green, so a SURVIVED
        # here means the machine was not loaded, not that the ceiling is fine.
        # Entered rather than omitted, because the first trailing cold read found
        # this effect had no entry at all -- and then the SECOND one found this
        # entry reached only one of the three call sites that each spelled the
        # ceiling out. The fix was not two more entries: the helper now owns the
        # ceiling as a single `const', so there is one place to mutate and the
        # three-way disagreement is unwritable. Same move as M132's, where the
        # shared root was removed as a parameter rather than computed twice.
        "expect": "survived",
    },
]
