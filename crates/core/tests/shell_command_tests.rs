use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

fn buffer_text(interp: &mut Interp) -> String {
    run(interp, "(buffer-string)")
}

/// Pump idle ticks until `pred` is true or the timeout hits.
fn pump_until(
    interp: &mut Interp,
    timeout: Duration,
    mut pred: impl FnMut(&mut Interp) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        core::idle_tick(interp, Duration::from_millis(0));
        if pred(interp) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn no_procs_running(i: &mut Interp) -> bool {
    run(i, "shell-command--procs") == "nil"
}

/// Run `shell-command` (M-!) with CMD typed into the minibuffer prompt.
fn do_shell_command(i: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: &str) {
    feed_keys(i, ed, "M-!").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-! should open a minibuffer prompt"
    );
    type_str(i, ed, cmd);
    feed_keys(i, ed, "RET").unwrap();
}

fn do_async_shell_command(i: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: &str) {
    feed_keys(i, ed, "M-&").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-& should open a minibuffer prompt"
    );
    type_str(i, ed, cmd);
    feed_keys(i, ed, "RET").unwrap();
}

fn do_shell_command_on_region(i: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: &str) {
    feed_keys(i, ed, "M-|").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-| should open a minibuffer prompt"
    );
    type_str(i, ed, cmd);
    feed_keys(i, ed, "RET").unwrap();
}

// --- 1. M-! with output ------------------------------------------------

#[test]
fn shell_command_with_output_lands_in_output_buffer() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_shell_command(&mut i, &ed, "echo hello-m79");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    let text = run(
        &mut i,
        "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-string)))",
    );
    assert!(
        text.contains("hello-m79"),
        "expected output in *Shell Command Output*: {}",
        text
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Shell Command Output*\"",
        "buffer with output must be shown"
    );
}

// --- 2. M-! with no output, exit 0: current buffer unchanged -----------

#[test]
fn shell_command_with_no_output_does_not_switch_buffers() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_shell_command(&mut i, &ed, "true");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*scratch*\"",
        "silent success must not switch windows -- this is the whole point of M-!'s special case"
    );
}

// --- M-& shows the output buffer immediately, unlike M-! ---------------

#[test]
fn async_shell_command_shows_buffer_immediately() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_async_shell_command(&mut i, &ed, "echo async-m79");
    // Unlike `shell-command', the buffer switch happens at launch time,
    // before the process has necessarily produced any output yet.
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Shell Command Output*\"",
        "M-& must show the output buffer right away"
    );
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("async-m79"),
        "expected streamed output: {}",
        text
    );
}

// --- 3. M-! non-zero exit: exit code visible ---------------------------

#[test]
fn shell_command_nonzero_exit_reports_code() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_shell_command(&mut i, &ed, "exit 7");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    let msg = ed.borrow().echo.clone().unwrap_or_default();
    // Review R5: this used to be `msg.contains('7')`, which passes as long as
    // the digit 7 shows up anywhere -- including in a path, a byte count, or a
    // completely different sentence. Pin the whole message instead.
    assert_eq!(msg, "Shell command exited abnormally with code 7");
}

// --- 4. M-| success: region replaced with stdout ------------------------

#[test]
fn shell_command_on_region_replaces_with_stdout() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "tr a-z A-Z");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("HELLO world"),
        "region should be replaced with uppercased stdout: {}",
        text
    );
}

// --- 5. M-| success with stderr: stderr must not enter the buffer ------

#[test]
fn shell_command_on_region_success_keeps_stderr_out_of_buffer() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "echo -n REPLACED; echo warning-noise >&2");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("REPLACED"),
        "region must be replaced with stdout: {}",
        text
    );
    assert!(
        !text.contains("warning-noise"),
        "stderr must never enter the buffer content: {}",
        text
    );
    let out = run(
        &mut i,
        "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-string)))",
    );
    assert!(
        out.contains("warning-noise"),
        "stderr should still be visible in the output buffer: {}",
        out
    );
}

// --- 6. M-| failure: region untouched -----------------------------------

#[test]
fn shell_command_on_region_failure_leaves_region_untouched() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    let before = buffer_text(&mut i);
    do_shell_command_on_region(&mut i, &ed, "echo should-not-appear; exit 9");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let after = buffer_text(&mut i);
    assert_eq!(
        before, after,
        "a non-zero exit must leave the region byte-for-byte unchanged"
    );
}

// --- 7. M-| replacement goes through undo --------------------------------

#[test]
fn shell_command_on_region_replacement_is_undoable() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "tr a-z A-Z");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    assert!(buffer_text(&mut i).contains("HELLO world"));
    run(&mut i, "(undo)");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("hello world") && !text.contains("HELLO"),
        "undo should restore the pre-substitution text: {}",
        text
    );
}

// --- 8. M-| with no mark set: no error signal, just a message ----------

#[test]
fn shell_command_on_region_without_mark_reports_message_not_error() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"no region here\")");
    // No `set-mark' called -- `region-beginning' would signal.
    let result = run(&mut i, "(shell-command-on-region)");
    assert!(
        !result.starts_with("ERROR"),
        "calling with no mark set must not signal an elisp error: {}",
        result
    );
    assert!(
        ed.borrow().minibuffer.is_none(),
        "no minibuffer prompt should have opened when there is no region"
    );
}

// --- 9. output buffer stays unmodified while streaming -------------------

#[test]
fn shell_command_output_buffer_not_marked_modified_while_streaming() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_shell_command(&mut i, &ed, "echo chunk; sleep 1");
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        let t = run(
            i,
            "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-string)))",
        );
        t.matches("chunk").count() >= 1
    });
    assert!(ok, "output never arrived");
    assert_ne!(
        run(&mut i, "shell-command--procs"),
        "nil",
        "the process must still be running for this to test the streaming window"
    );
    let modified = run(
        &mut i,
        "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-modified-p)))",
    );
    assert_eq!(
        modified, "nil",
        "streamed output must not leave the output buffer showing the modified marker"
    );
    pump_until(&mut i, Duration::from_secs(5), no_procs_running);
}

// --- 10. output cap: process killed, truncation message shown ----------

#[test]
fn shell_command_output_cap_kills_process_and_reports_truncation() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    // Force a tiny cap. Deliberately NOT `yes' here: `shell-process-poll'
    // (shell.rs) drains its whole mpsc channel in one call, looping
    // until it sees the channel momentarily empty -- piped against
    // `yes' (an unthrottled, max-speed producer) that single poll call
    // can take an unbounded amount of wall-clock time under load, since
    // the channel may never go empty while `yes' keeps flooding it.
    // Confirmed by hand: even the pre-fix-round code (a direct,
    // synchronous `kill' the instant the cap trips, no extra draining)
    // hung the same way against `yes' on this machine under load,
    // so this is a property of the shared (out-of-scope, unmodified)
    // `poll' primitive combined with an untamed producer, not something
    // this milestone's own logic can fix. A steady, throttled producer
    // (40 bytes every 20ms) still blows well past a 200-char cap in a
    // few hundred milliseconds, without ever handing `poll' an
    // unbounded backlog to drain in one call.
    run(&mut i, "(setq shell-command-max-output-chars 200)");
    do_shell_command(
        &mut i,
        &ed,
        "bash -c 'while true; do printf 0123456789012345678901234567890123456789; sleep 0.02; done'",
    );
    let ok = pump_until(&mut i, Duration::from_secs(10), no_procs_running);
    assert!(ok, "capped process should be killed and reaped promptly");
    let text = run(
        &mut i,
        "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-string)))",
    );
    assert!(
        text.contains("truncat"),
        "expected a truncation message: {}",
        text
    );
}

// --- 11. default-directory: command runs in the originating buffer's dir

#[test]
fn shell_command_runs_in_original_buffer_directory() {
    let (mut i, ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "se_shellcmd_dir_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(
        &mut i,
        &format!("(set-default-directory {:?})", dir.to_str().unwrap()),
    );
    do_shell_command(&mut i, &ed, "pwd");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    let text = run(
        &mut i,
        "(with-current-buffer-internal \"*Shell Command Output*\" (lambda () (buffer-string)))",
    );
    // Resolve symlinks (macOS $TMPDIR is often a symlink) the same way
    // the child's own `pwd' would have to for the assertion to hold on
    // every platform this runs on.
    let canon = std::fs::canonicalize(&dir).unwrap();
    assert!(
        text.contains(canon.file_name().unwrap().to_str().unwrap()),
        "expected pwd output to be inside the buffer's default-directory: {} (dir={:?})",
        text,
        canon
    );
    std::fs::remove_dir_all(&dir).ok();
}

// --- 12. shell-command-mode is in evil-emacs-state-modes ----------------

#[test]
fn shell_command_mode_is_an_evil_emacs_state_mode() {
    let (mut i, _ed) = setup();
    let result = run(&mut i, "(memq 'shell-command-mode evil-emacs-state-modes)");
    assert_ne!(
        result, "nil",
        "shell-command-mode must be in evil-emacs-state-modes, else evil's normal-state \
         keymap outranks the local `q' binding and it never fires"
    );
}

// --- 13. the three keys are bound to the right commands in global map ---

#[test]
fn shell_command_keys_bound_in_global_keymap() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\M-!))"),
        "(command shell-command global)"
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\M-|))"),
        "(command shell-command-on-region global)"
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\M-&))"),
        "(command async-shell-command global)"
    );
}

// --- R1 (fix round): concurrent edit during M-| must not corrupt data --
//
// The job is asynchronous (see file header); the user can keep editing
// the SAME buffer while it runs. Storing the region as two bare integer
// offsets means an edit that happens before the process exits silently
// shifts where those offsets actually point in the buffer -- the
// eventual `delete-region'+`insert' then destroys whatever the user
// just typed and/or replaces the wrong span. This must use markers
// (which `buffer.rs' keeps adjusted through every insert/delete)
// instead of raw integers.

#[test]
fn shell_command_on_region_survives_concurrent_edit_at_region_start() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    // region = "hello" (1..6).
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "sleep 0.3; tr a-z A-Z");
    // Concurrent edit: user types "XXXXX" at the very start of the
    // buffer (position 1 = the region's own start) while the process is
    // still running.
    run(&mut i, "(goto-char 1) (insert \"XXXXX\")");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("XXXXX"),
        "the user's own concurrent edit must survive: {}",
        text
    );
    assert!(
        text.contains("HELLO"),
        "the region's own content should still get transformed by the \
         command (uppercased), landing right after the user's edit: {}",
        text
    );
    assert_eq!(
        text.trim_matches('"'),
        "XXXXXHELLO world",
        "expected the marker-based region to track the concurrent insert \
         exactly: {}",
        text
    );
}

// --- R2 (fix round): `q' must return to the buffer the command was ------
// invoked from, not whatever buffer happens to be current when the idle
// pump gets around to displaying the output (`shell-command''s
// has-output path and `shell-command-on-region''s failure path are both
// only ever discovered from the pump, arbitrarily long after the user
// may have switched away).

#[test]
fn shell_command_quit_source_recorded_at_invocation_not_at_pump_time() {
    let (mut i, ed) = setup();
    // M103: pre-split into two windows so `shell-command--maybe-show`'s
    // `pop-to-buffer` REUSES the other window (`display-buffer` step 2)
    // instead of splitting one. This test is specifically about
    // `quit-source` being captured AT INVOCATION TIME, not about
    // `quit-source-return`'s window-aware "delete the window `display-
    // buffer` created for me" branch (that branch is covered by
    // `window_display_tests.rs`) -- if a split happened here instead,
    // `q` would correctly delete the newly created window and land back
    // on whatever the OTHER (pre-existing) window already showed
    // (`*other*`, since that window's own buffer was never touched),
    // which is the right answer for THAT mechanism but would silently
    // stop exercising the invocation-time-capture behavior this test
    // exists to guard.
    run(&mut i, "(split-window-below)");
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    do_shell_command(&mut i, &ed, "sleep 0.3; echo done-m79");
    // Switch away before the process (and thus the "has output, show
    // the buffer" decision) has had a chance to complete.
    run(&mut i, "(get-buffer-create \"*other*\")");
    run(&mut i, "(switch-to-buffer-internal \"*other*\")");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*Shell Command Output*\"",
        "output buffer should have been shown once the process finished"
    );
    run(&mut i, "(quit-source-return)");
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"*scratch*\"",
        "`q' must return to *scratch* (where M-! was invoked), not *other* \
         (whatever was current when the pump happened to notice the process \
         had finished)"
    );
}

// --- R7 (fix round): concurrent edits AT the region's own boundary or --
// INSIDE it must not be silently absorbed into the replaced span. Markers
// here have no insertion-type distinction (`buffer.rs' shifts every
// marker at/after the insertion point the same way -- confirmed by
// reading `adjust_positions_insert'), so an edit landing at the region's
// end boundary or in its middle moves the END marker forward right along
// with it, silently pulling the new text into the doomed-to-be-deleted
// span. The fix must detect that the content under the markers no longer
// matches what was piped to the command, and refuse to replace.

#[test]
fn shell_command_on_region_concurrent_edit_at_end_boundary_is_preserved() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    // region = "hello" (1..6).
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "sleep 0.3; tr a-z A-Z");
    // Concurrent edit exactly at the region's END boundary (position 6,
    // right where "hello" ends and " world" begins).
    run(&mut i, "(goto-char 6) (insert \"END\")");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("END"),
        "the user's own concurrent edit at the region's end boundary must \
         survive, not get silently deleted along with the stale region: {}",
        text
    );
}

#[test]
fn shell_command_on_region_concurrent_edit_in_middle_is_preserved() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    // region = "hello" (1..6).
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "sleep 0.3; tr a-z A-Z");
    // Concurrent edit in the MIDDLE of the region (position 3, inside
    // "hello").
    run(&mut i, "(goto-char 3) (insert \"MID\")");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let text = buffer_text(&mut i);
    assert!(
        text.contains("MID"),
        "the user's own concurrent edit inside the region must survive, \
         not get silently deleted along with the stale region: {}",
        text
    );
}

// Pin down that the fix for the two cases above does NOT break the case
// R1 already fixed: an edit strictly BEFORE the region (at its very
// start boundary, pushing both markers forward together with content
// unchanged) must still replace normally.
#[test]
fn shell_command_on_region_concurrent_edit_before_region_still_replaces() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    do_shell_command_on_region(&mut i, &ed, "sleep 0.3; tr a-z A-Z");
    run(&mut i, "(goto-char 1) (insert \"XXXXX\")");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_procs_running);
    assert!(ok, "process never finished");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    let text = buffer_text(&mut i);
    assert_eq!(
        text.trim_matches('"'),
        "XXXXXHELLO world",
        "an edit strictly before the region must still replace normally \
         (region content itself unchanged, only shifted): {}",
        text
    );
}

// --- R9 (fix round): filter mode must stop promptly when the ORIGINAL --
// buffer is killed mid-stream, same aggressiveness 'view mode already had.

#[test]
fn shell_command_on_region_stops_promptly_when_original_buffer_killed_mid_stream() {
    let (mut i, ed) = setup();
    // A second, otherwise-unused buffer must exist before *scratch* gets
    // killed below: `kill-buffer' (buffers.rs) falls back to fabricating
    // a BRAND NEW same-named "*scratch*" only when NO other live buffer
    // exists (see `switch_to_a_live_buffer') -- that fresh buffer would
    // pass `(get-buffer "*scratch*")' again despite being a completely
    // different object than the one the marker/entry actually track,
    // defeating the very check this test exists to exercise.
    run(&mut i, "(get-buffer-create \"*keepalive*\")");
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1) (set-mark (point)) (goto-char 6)");
    // The command must produce NO output at all. An earlier version used
    // `echo chunk; sleep 5', and mutation M15 showed that version passed
    // even with R9's proactive check disabled: the chunk was still
    // sitting undelivered when the buffer got killed, so the next pump
    // tick polled it and R4's *reactive* per-chunk check did the killing.
    // The test was green for the wrong reason and covered R4, not R9.
    // With a silent process, no chunk ever arrives, so the reactive path
    // can never fire and only the proactive tick-top check can notice
    // the buffer is gone.
    do_shell_command_on_region(&mut i, &ed, "sleep 5");
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        run(i, "shell-command--procs") != "nil"
    });
    assert!(ok, "setup wait failed");
    run(&mut i, "(kill-buffer \"*scratch*\")");
    let start = Instant::now();
    let ok = pump_until(&mut i, Duration::from_secs(4), no_procs_running);
    assert!(
        ok,
        "process must be killed promptly once its original buffer dies, \
         not left running for the full `sleep 5'"
    );
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "killing the source buffer must stop the process quickly, not \
         wait out the remaining `sleep 5'"
    );
}
