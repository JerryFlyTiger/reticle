//! M40 part 1: before-save-hook, after-save-hook, kill-buffer-hook --
//! the save/kill hook infrastructure the rest of M40 (LSP didSave/
//! didClose, verilog-auto-on-save) hangs off. All three run off the
//! keystroke path (see commands.rs's KEYSTROKE_HOOKS doc comment),
//! same as find-file-hook, so they're unbudgeted here too.

use std::cell::RefCell;
use std::rc::Rc;

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

fn ok(interp: &mut Interp, src: &str) -> String {
    let r = run(interp, src);
    assert!(!r.starts_with("ERROR"), "{:?} failed: {}", src, r);
    r
}

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_save_kill_hooks_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A fresh scratch directory under the OS temp dir, unique per test run.
fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

#[test]
fn before_save_hook_edits_reach_disk() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("before_edit");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(
        &mut i,
        "(add-hook 'before-save-hook
           (lambda () (goto-char (point-max)) (insert \" world\")))",
    );
    ok(&mut i, "(save-buffer)");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "hello world");
}

#[test]
fn after_save_hook_sees_modified_flag_already_cleared() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("after_flag");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(insert \"!\")");
    ok(
        &mut i,
        "(add-hook 'after-save-hook
           (lambda () (setq test--modified-in-hook (buffer-modified-p))))",
    );
    ok(&mut i, "(save-buffer)");

    assert_eq!(run(&mut i, "test--modified-in-hook"), "nil");
}

#[test]
fn before_and_after_save_hooks_run_in_order() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("order");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(setq test--order nil)");
    ok(
        &mut i,
        "(add-hook 'before-save-hook
           (lambda () (setq test--order (cons 'before test--order))))",
    );
    ok(
        &mut i,
        "(add-hook 'after-save-hook
           (lambda () (setq test--order (cons 'after test--order))))",
    );
    ok(&mut i, "(save-buffer)");

    // test--order is built by consing onto the front, so the most
    // recent (after) is first once reversed back to arrival order.
    assert_eq!(run(&mut i, "(reverse test--order)"), "(before after)");
}

#[test]
fn before_save_hook_error_does_not_abort_the_save() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("hook_error");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    ok(
        &mut i,
        "(add-hook 'before-save-hook (lambda () (error \"boom\")))",
    );
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer signaled: {}", r);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "hello!");
}

#[test]
fn kill_buffer_hook_sees_the_dying_buffer_as_current_when_killed_from_elsewhere() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"target\")");
    ok(&mut i, "(get-buffer-create \"elsewhere\")");
    ok(&mut i, "(set-buffer \"elsewhere\")");
    ok(
        &mut i,
        "(add-hook 'kill-buffer-hook
           (lambda () (setq test--killed-name (buffer-name))))",
    );
    ok(&mut i, "(kill-buffer \"target\")");

    assert_eq!(run(&mut i, "test--killed-name"), "\"target\"");
    assert_eq!(run(&mut i, "(get-buffer \"target\")"), "nil");
    // The hook ran with target current, but the caller's buffer is
    // restored afterward.
    assert_eq!(run(&mut i, "(buffer-name)"), "\"elsewhere\"");
}

#[test]
fn kill_buffer_hook_runs_when_killing_the_current_buffer() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"target\")");
    ok(&mut i, "(set-buffer \"target\")");
    ok(
        &mut i,
        "(add-hook 'kill-buffer-hook
           (lambda () (setq test--killed-name (buffer-name))))",
    );
    ok(&mut i, "(kill-buffer)");

    assert_eq!(run(&mut i, "test--killed-name"), "\"target\"");
    assert_eq!(run(&mut i, "(get-buffer \"target\")"), "nil");
}

#[test]
fn kill_buffer_hook_killing_the_original_current_buffer_falls_back_to_a_live_buffer() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"A\")");
    ok(&mut i, "(get-buffer-create \"B\")");
    ok(&mut i, "(set-buffer \"A\")");
    // A "kill related buffers together" hook is a natural thing to
    // write, and exactly the shape review issue #1 flagged: while
    // `(kill-buffer "B")` is switched onto B to run the hook, the hook
    // kills A -- the CALLER's original current buffer -- out from
    // under it.
    ok(
        &mut i,
        "(add-hook 'kill-buffer-hook
           (lambda () (when (equal (buffer-name) \"B\") (kill-buffer \"A\"))))",
    );
    let r = run(&mut i, "(kill-buffer \"B\")");
    assert!(!r.starts_with("ERROR"), "kill-buffer signaled: {}", r);

    assert_eq!(run(&mut i, "(get-buffer \"A\")"), "nil");
    assert_eq!(run(&mut i, "(get-buffer \"B\")"), "nil");

    // current-buffer must be a live, findable buffer -- not the zombie
    // A that switching back to a no-longer-`ed.buffers`-resident
    // "original" would install (A's name field still reads "A" even
    // after removal, but it would no longer be findable by that name).
    let cur_name = run(&mut i, "(buffer-name)");
    assert_ne!(
        cur_name, "\"A\"",
        "current buffer must not be the killed original A"
    );
    assert_ne!(
        run(&mut i, "(get-buffer (buffer-name))"),
        "nil",
        "current buffer {} must still be a real, findable buffer",
        cur_name
    );
}

#[test]
fn save_buffer_on_a_non_visiting_buffer_errors_without_running_before_save_hook() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(get-buffer-create \"no-file\")");
    ok(&mut i, "(set-buffer \"no-file\")");
    ok(&mut i, "(setq test--before-ran nil)");
    ok(
        &mut i,
        "(add-hook 'before-save-hook (lambda () (setq test--before-ran t)))",
    );
    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected an error, got: {}", r);
    assert!(r.contains("not visiting a file"), "message: {}", r);
    assert_eq!(run(&mut i, "test--before-ran"), "nil");
}

// M62: save-buffer's on-disk-conflict guard.

#[test]
fn save_buffer_pins_a_baseline_and_a_second_untouched_save_still_succeeds() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_repeat_save");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "first save failed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");

    // Second save, disk untouched since the first save's baseline was
    // recorded -- must not falsely report a conflict.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"?\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "second save failed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!?");
}

#[test]
fn save_buffer_refuses_to_clobber_an_external_change() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_conflict");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(insert \"!\")");
    // External tool changes the file's length (and hence mtime OR size
    // differs from the baseline recorded at find-file-internal time --
    // a differing length guarantees the size check alone catches it
    // even if mtime granularity is too coarse to move within the same
    // test run).
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected an error, got: {}", r);
    assert!(
        r.contains("changed on disk"),
        "message should mention the conflict: {}",
        r
    );
    // The core assertion: the external content must survive untouched.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on disk by someone else"
    );
}

#[test]
fn save_buffer_force_overwrites_after_a_conflict() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_force");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected the plain save to be refused: {}",
        r
    );

    let r = run(&mut i, "(save-buffer t)");
    assert!(!r.starts_with("ERROR"), "forced save failed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");
}

/// FORCE on its own, with NO prior refusal to lean on.
///
/// This test exists because a mutation survived. Flipping
/// `let force = opt(a, 0).truthy();` to `let force = false;` left
/// `save_buffer_force_overwrites_after_a_conflict` still passing: that test
/// runs a plain `(save-buffer)` first, and the refusal it triggers sets
/// `save_conflict_ack`, which by itself is enough to let the NEXT save
/// through. So the FORCE argument was never actually the thing under test
/// there -- the ack path masked it. Here nothing is refused first, so the
/// only thing that can carry the save past the guard is FORCE.
#[test]
fn save_buffer_force_works_without_a_prior_refusal() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_force_no_ack");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    // No plain `(save-buffer)` first -- `save_conflict_ack` is still nil.
    let r = run(&mut i, "(save-buffer t)");
    assert!(!r.starts_with("ERROR"), "forced save failed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");
}

#[test]
fn save_buffer_retrying_right_after_a_refusal_succeeds() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_ack_retry");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected the first save to be refused: {}",
        r
    );

    // No FORCE argument this time -- just calling save-buffer again on
    // the SAME buffer for the SAME path is itself the confirmation.
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "second (ack'd) save failed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");
}

#[test]
fn save_buffer_conflict_ack_does_not_leak_across_buffers() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_ack_cross_buffer");
    std::fs::create_dir_all(&dir).unwrap();
    let path_a = dir.join("a.txt");
    let path_b = dir.join("b.txt");
    std::fs::write(&path_a, "hello a").unwrap();
    std::fs::write(&path_b, "hello b").unwrap();

    // Buffer A: gets refused, then confirmed by retrying -- this is the
    // "already acked" state that must stay local to A.
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path_a.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path_a, "changed a on disk").unwrap();
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected A's first save to be refused: {}",
        r
    );

    // Buffer B: also stale on disk, but has never been asked about it.
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path_b.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path_b, "changed b on disk").unwrap();

    // B's FIRST save must still be refused -- A's confirmation must not
    // leak over just because save-buffer happens to run again next.
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "B's first save must be refused independently of A's ack, got: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path_b).unwrap(),
        "changed b on disk"
    );

    // A, meanwhile, is still ack'd and can retry successfully.
    ok(
        &mut i,
        &format!(
            "(set-buffer {:?})",
            path_a.file_name().unwrap().to_str().unwrap()
        ),
    );
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "A's ack'd retry should still succeed: {}",
        r
    );
    assert_eq!(std::fs::read_to_string(&path_a).unwrap(), "hello a!");
}

#[test]
fn save_buffer_conflict_ack_is_one_time_only() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_ack_one_shot");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path, "changed on disk once").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected the first save to be refused: {}",
        r
    );
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "ack'd retry should succeed: {}", r);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");

    // The ack must have been consumed by that successful save -- a fresh
    // external change must be refused again, not silently let through.
    std::fs::write(&path, "changed on disk twice, longer this time").unwrap();
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "the second external change must be refused independently, got: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on disk twice, longer this time"
    );
}

#[test]
fn save_buffer_new_file_then_created_out_from_under_it_is_a_conflict() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_new_file_race");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    // No file yet -- find-file-internal takes the "new file" branch.

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(insert \"first content\")");

    // Straightforward case: still no file on disk, save succeeds.
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "save of a genuinely new file failed: {}",
        r
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first content");

    // Race variant: another buffer visits the not-yet-existing file,
    // then someone else creates it before the save.
    std::fs::create_dir_all(&dir).unwrap();
    let path2 = dir.join("t2.txt");
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path2.to_str().unwrap()),
    );
    ok(&mut i, "(insert \"race\")");
    std::fs::write(&path2, "someone else created this first").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected a conflict, got: {}", r);
    assert_eq!(
        std::fs::read_to_string(&path2).unwrap(),
        "someone else created this first"
    );
}

#[test]
fn save_buffer_after_rename_file_does_not_false_positive() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_rename");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    let renamed = dir.join("t2.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(
        &mut i,
        &format!(
            "(rename-file {:?} {:?})",
            path.to_str().unwrap(),
            renamed.to_str().unwrap()
        ),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save after rename failed: {}", r);
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "hello!");
}

#[test]
fn save_buffer_a_second_external_change_before_the_retry_is_refused_again() {
    // M62 tail review (finding 1): the ack must be tied to the disk
    // state observed at refusal time, not stay valid indefinitely until
    // the next successful write. Otherwise a SECOND, unrelated external
    // edit landing between the refusal and the retry would get
    // silently clobbered too, with no warning ever shown for THAT
    // change.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_second_external_change");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");

    // First external change -- gets the buffer refused and acked.
    std::fs::write(&path, "first external change").unwrap();
    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected the first refusal: {}", r);

    // Second, DIFFERENT external change before the user retries.
    std::fs::write(&path, "second external change, a different length").unwrap();
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "a second unrelated external change must be refused again, not silently passed by the stale ack: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "second external change, a different length"
    );

    // The user now retries against THIS state and it's accepted.
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "retry against the current state should succeed: {}",
        r
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello!");
}

#[test]
fn save_buffer_catches_a_size_change_even_when_mtime_is_rolled_back_to_match() {
    // M62 tail review: the mtime comparison alone is not reliably
    // observable in a fast test run (APFS mtime resolution can be too
    // coarse to move between two `std::fs::write` calls microseconds
    // apart) -- this pins that the SIZE half of the (mtime, size) pair
    // catches an external change on its own, by deliberately rolling
    // the mtime back to the baseline with `File::set_modified` while
    // leaving the size changed.
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_size_only");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();
    let t0 = std::fs::metadata(&path).unwrap().modified().unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");

    // External tool writes a different-length body, then rolls mtime
    // back to exactly the baseline -- only `size` still differs.
    std::fs::write(&path, "changed on disk, a much longer replacement body").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(t0)
        .unwrap();
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(after.modified().unwrap(), t0, "mtime roll-back didn't take");
    assert_ne!(
        after.len(),
        5,
        "size must actually differ from the baseline"
    );

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected the size-only mismatch to be caught, got: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on disk, a much longer replacement body"
    );
}

#[test]
fn save_buffer_conflict_ack_does_not_survive_a_rename_to_a_different_path() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("m62_ack_rename");
    std::fs::create_dir_all(&dir).unwrap();
    let path1 = dir.join("p1.txt");
    let path2 = dir.join("p2.txt");
    std::fs::write(&path1, "hello").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path1.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"!\")");
    std::fs::write(&path1, "changed on disk by someone else").unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected the first save (on path1) to be refused: {}",
        r
    );

    ok(
        &mut i,
        &format!(
            "(rename-file {:?} {:?})",
            path1.to_str().unwrap(),
            path2.to_str().unwrap()
        ),
    );

    // No further external change to path2 -- if the ack were keyed on
    // path alone (or ignored the path entirely), this retry would now
    // force-overwrite path2 despite the user never having been warned
    // about path2 specifically.
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "the stale ack from path1 must not authorize an overwrite of path2, got: {}",
        r
    );
}
