# M62 -- No check for disk having changed before saving.
#
# The history of this list is worth writing down, since it's itself a case
# study of the "who verifies whom" rule:
#
# 1. What the reviewer cold-read was the **first version** of the fix (using
#    `Editor::last_command` to detect "pressed again"). Of the 8 entries it
#    handed off, it flagged 3 as black-box unobservable itself -- because the
#    first version's force path had no tests at all, and that's exactly the
#    path where it also found two real bugs (`last_command` is global state,
#    would erroneously match across buffers).
# 2. The fix-up round switched the mechanism to a one-time token owned by the
#    buffer itself, and the target lines for those 3 entries no longer
#    existed; the main conversation rewrote them against the new code per the
#    reviewer's original intent.
# 3. First live run: 5 matched expectations, 5 survived. Of these, M8/M9
#    surviving was a **mutation designed wrong by the main conversation** --
#    changing `disk_state` to `Unknown`, when `Unknown` never counts as a
#    conflict, effectively turns the check off rather than simulating "stale
#    baseline". The failure direction that came to mind was "the value gets
#    written wrong", when what actually needed simulating was "never written
#    at all". This is the same blind spot recorded in M60.
# 4. M4 surviving was a **genuine test gap**:
#    `save_buffer_force_overwrites_after_a_conflict` first runs a
#    `(save-buffer)` that gets refused, and the ack set by that refusal alone
#    is enough for the next call to pass, so whether FORCE is actually read
#    was never tested. `save_buffer_force_works_without_a_prior_refusal` was
#    added.
# 5. The trailing re-review **overturned the main conversation's
#    "unobservable" verdict on two other surviving entries**, and reached
#    that conclusion by actually experimenting (see below). Both entries now
#    have tests pinning them down and are no longer black-box.
#
# **Everything in this file from item 3 onward has not been cold-read** (the
# "one round limit" of the loop), recorded honestly here, not pretended to
# have been reviewed.
#
# How to run:
#     dev/mutate.py --config dev/mutations/m62.py
#
# TEST_TARGET left empty: the list spans two integration test binaries,
# save_kill_hooks_tests and evil_tests, and the shared TEST_TARGET can only
# point at one, so leaving it empty is the only way to hit both.
#
# ## Two overturned "unobservable" verdicts (findings from the trailing
#    re-review, recorded to avoid repeating the mistake)
#
# **(a) The size comparison.** The main conversation originally judged: this
# machine is APFS with nanosecond mtime, so a subsequent external write must
# move mtime, making the size comparison forever a redundant second check,
# unobservable on this machine. **Wrong.** `std::fs::File::set_modified` is a
# stable API that can wind mtime back exactly to the baseline while keeping a
# different size. Tested directly:
#     baseline               mtime=...895624242 size=5
#     after external write   mtime=...896106570 size=32
#     after set_modified     mtime=...895624242 size=32   <- mtime equals baseline, size differs
# Lesson: before concluding "this environment can't produce that state", ask
# first "is there an API that can directly force the environment into that
# state" -- don't reason only from the probability of it occurring naturally.
#
# **(b) The ack binds to the path.** The main conversation originally
# conflated "doesn't leak across buffers" with "the same buffer changes
# path", treating both as structural guarantees. The former holds (ack is a
# field of `Buffer`, B can't read A's), but the latter **does not hold** --
# `rename-file` (`files.rs:349-350`) changes the buffer's `.file` without
# touching `save_conflict_ack`, so the path comparison is a line that does
# real work and can be tested.
#
# ## Still honestly recorded as unobservable (not listed)
#
# 1. **"ack doesn't leak across buffers"**: ack is a field of `Buffer`,
#    per-buffer isolation is a **structural** guarantee, not something
#    maintained by any one line of code.
#    `save_buffer_conflict_ack_does_not_leak_across_buffers` is a
#    **regression guard rail** (blocking someone in the future from moving
#    ack to `Editor`), not a mutation-provable assertion. The trailing
#    re-review looked at this independently and agrees.
# 2. **Remote `/ssh:` always skips the check**: `remote.rs` has no single-file
#    stat primitive, so `disk_state` is always `Unknown`. Reverting that
#    `is_none()` guard would send remote paths into a local metadata lookup,
#    and remote tests use fake paths, making the behavior unpredictable --
#    not listed.
# 3. **The non-atomic window between check and write**: observing it would
#    require inserting an external write between two system calls, which the
#    existing test infrastructure cannot do. Already documented in the
#    comments in `editing.rs` as a deliberately unsolved gap.

PACKAGE = "core"

MUTATIONS = [
    {
        "label": "M1 Known baseline comparison entirely disabled (checks neither mtime nor size)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        m.modified().ok() != Some(t) || m.len() != n\n",
        "new": "                        false\n",
        "test": "save_buffer_refuses_to_clobber_an_external_change",
    },
    {
        # The first version pointed at
        # `save_buffer_refuses_to_clobber_an_external_change` (relying on
        # natural time passing, mtime is bound to move), which SURVIVED.
        # Changed to point at the test that uses `File::set_modified` to wind
        # mtime back to the baseline, leaving only size different.
        "label": "M2 remove only the size comparison, rely on mtime alone",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        m.modified().ok() != Some(t) || m.len() != n\n",
        "new": "                        m.modified().ok() != Some(t)\n",
        "test": "save_buffer_catches_a_size_change_even_when_mtime_is_rolled_back_to_match",
    },
    {
        "label": "M3 the Absent branch doesn't count as a conflict (a new file created by someone else first gets silently clobbered)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                    crate::buffer::DiskState::Absent => true,\n",
        "new": "                    crate::buffer::DiskState::Absent => false,\n",
        "test": "save_buffer_new_file_then_created_out_from_under_it_is_a_conflict",
    },
    {
        # `let force = opt(a, 0).truthy();` occurs twice on its own (the other
        # site is FORCE for `save-buffers-kill-terminal`), so the defun line
        # must be included as context, otherwise a single change would hit
        # two builtins at once and the failure reason couldn't be attributed
        # (mutate.py rule 3).
        #
        # The first version pointed at
        # `save_buffer_force_overwrites_after_a_conflict`, and it SURVIVED in
        # a live run -- that test first runs a `(save-buffer)` that gets
        # refused, and the ack set by that refusal alone is enough for the
        # next call to pass, so whether FORCE is actually read was never
        # tested. Changed to point at a new test that doesn't trigger a
        # refusal first.
        "label": "M4 the FORCE argument is ignored (`:w!` and (save-buffer t) stop working)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": '    defun(interp, "save-buffer", 0, Some(1), |i, a| {\n        let force = opt(a, 0).truthy();\n',
        "new": '    defun(interp, "save-buffer", 0, Some(1), |i, a| {\n        let force = false;\n',
        "test": "save_buffer_force_works_without_a_prior_refusal",
    },
    {
        "label": "M5 ack is never honored (retrying after a refusal is refused again, force path effectively doesn't exist)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                    let already_acked =\n                        b.borrow().save_conflict_ack == Some((path.clone(), observed));\n",
        "new": "                    let already_acked = false;\n",
        "test": "save_buffer_retrying_right_after_a_refusal_succeeds",
    },
    {
        # A gap found by the trailing re-review: the old ack design bound only
        # to the path and stayed valid until the next successful write, so
        # "refused -> an unrelated external change happens in the meantime ->
        # retry" would silently clobber a version the user was never warned
        # about. This mutation removes the state comparison, reverting to the
        # old behavior.
        "label": "M6 ack compares only the path, not the disk state (reverts to the old design: a second external change gets silently clobbered)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        b.borrow().save_conflict_ack == Some((path.clone(), observed));\n",
        "new": "                        b.borrow().save_conflict_ack.as_ref().map(|(p, _)| p.clone())\n                            == Some(path.clone());\n",
        "test": "save_buffer_a_second_external_change_before_the_retry_is_refused_again",
    },
    {
        # Reversed direction: compares only state, not path. `rename-file`
        # changes `.file` but doesn't touch ack, so the path comparison is a
        # line that does real work.
        "label": "M7 ack compares only disk state, not path (ack gets erroneously honored after a rename)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "                        b.borrow().save_conflict_ack == Some((path.clone(), observed));\n",
        "new": "                        b.borrow().save_conflict_ack.as_ref().map(|(_, s)| *s) == Some(observed);\n",
        "test": "save_buffer_conflict_ack_does_not_survive_a_rename_to_a_different_path",
    },
    {
        # **Confirmed SURVIVED in a live run, and this time it's not a design
        # mistake -- that line has genuinely become redundant.** In round one
        # (ack bound only to the path) it FAILed as expected. After round two
        # changed ack to bind to "path + the disk state at the time", not
        # clearing ack no longer causes an erroneous pass: after a successful
        # write, `disk_state` gets refreshed, so if there's a conflict next
        # time, the currently observed state will necessarily differ from the
        # old state stored in ack, the ack comparison misses, and it's
        # refused anyway. For it to hit, the disk would need to return
        # **exactly** to the state at the time of the previous refusal (same
        # mtime and size), which requires deliberately constructing it via
        # `set_modified`, not a naturally occurring scenario. Conclusion:
        # this line is now defense in depth, kept (it makes the "ack is
        # one-time-only" invariant explicit), but honestly recorded as
        # **unobservable**, without forcing a test to be manufactured for it.
        # As an aside, the reason
        # `save_buffer_conflict_ack_is_one_time_only` passes now is no longer
        # the mechanism its name describes but the state comparison -- the
        # name is a bit misleading now, but the assertion is still correct.
        "label": "M8 ack not cleared after a successful save (already covered by the state comparison, expected to survive)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "            bb.save_conflict_ack = None;\n",
        "new": "",
        "test": "save_buffer_conflict_ack_is_one_time_only",
    },
    {
        # The first version wrote `if false` (so the else branch's `Unknown`
        # gets taken), which SURVIVED -- `Unknown` never counts as a
        # conflict, which effectively **turns the check off** rather than
        # making the baseline stale. To test "doesn't refresh", the field
        # must keep the pre-save Known value, so the whole assignment is
        # deleted.
        "label": "M9 disk_state is not refreshed after writing (second save falsely reports a conflict)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """            bb.disk_state = if crate::remote::parse(&path).is_none() {
                std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| {
                        m.modified()
                            .ok()
                            .map(|t| crate::buffer::DiskState::Known(t, m.len()))
                    })
                    .unwrap_or(crate::buffer::DiskState::Unknown)
            } else {
                crate::buffer::DiskState::Unknown
            };
""",
        "new": "",
        "test": "save_buffer_pins_a_baseline_and_a_second_untouched_save_still_succeeds",
    },
    {
        # Same as M9: the first version's change to `Unknown` turned the
        # check off instead of "didn't record a baseline", which SURVIVED.
        # Deleting the assignment leaves it at its initial value `Absent`,
        # and `Absent` + the file now existing = a conflict.
        "label": "M10 Known baseline not recorded when reading the file (stays at Absent, false report on the very first save)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """                disk_state = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| {
                        m.modified()
                            .ok()
                            .map(|t| crate::buffer::DiskState::Known(t, m.len()))
                    })
                    .unwrap_or(crate::buffer::DiskState::Unknown);
""",
        "new": "",
        "test": "save_buffer_pins_a_baseline_and_a_second_untouched_save_still_succeeds",
    },
    {
        # This entry verifies the design decision that "aborting must use
        # Err, not echo + Ok(nil)". `evil--run-ex`'s wq branch is two plain
        # calls, `(save-buffer) (evil-ex-quit)`, with no condition-case
        # around it: a signal aborts the whole cond clause, so a failed save
        # means it doesn't quit. If changed to return nil, `:wq` would **quit
        # without saving**, worse than the original bug.
        "label": "M12 returns Ok(nil) instead of Err on conflict (reverts to the \"echo then return nil\" design)",
        "file": "crates/core/src/builtins/editing.rs",
        "old": """                        return Err(i.error(format!(
                            "{} has changed on disk since it was read — save again to overwrite",
                            path
                        )));
""",
        "new": "                        return Ok(Value::Nil);\n",
        "test": "ex_wq_does_not_quit_when_the_save_is_refused",
    },
    {
        "label": "M11 evil's `:w!` branch removed",
        "file": "crates/core/lisp/evil.el",
        "old": "           ((string= cmd \"w!\") (save-buffer t))\n",
        "new": "",
        "test": "ex_w_bang_overwrites_after_a_plain_w_is_refused_on_conflict",
    },
]
