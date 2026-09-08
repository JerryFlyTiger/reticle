//! M22: TRAMP-style /ssh: end-to-end tests — no network. An `ssh` shim
//! (a shell script that drops the option/host arguments and runs the
//! command locally) stands in for the real binary via
//! RETICLE_SSH_BIN, so the full editor path — parse, transport,
//! dired dispatch, save — is exercised for real.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

/// RETICLE_SSH_BIN is process-global state — serialize the tests
/// that set it.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    // M104 fix round: some tests here save real .v files over the
    // fake-ssh transport (see e.g. the manual_e2e-style save-buffer
    // call), and M104's `format-on-save' defaults to `t'. The fixture
    // content happens not to be valid Verilog, and this machine's
    // `verible-verilog-format' happens to exit 0 with unchanged output
    // on a parse failure -- so nothing actually reformats today, but
    // that is a coincidence of the fixture content and one external
    // tool's current behavior, not something this file (remote-file
    // save/kill semantics) is actually testing. Opting out
    // unconditionally, same as `verilog_auto_tests.rs'/
    // `lsp_save_close_tests.rs'/`lsp_mode_tests.rs'/`search_tests.rs'
    // before it.
    let r = interp.eval_source("(setq format-on-save nil)");
    assert!(r.is_ok(), "setq format-on-save nil failed");
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
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
            "se_ssh_{}_{}_{}",
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

/// Fixture dir with files plus the ssh shim; sets RETICLE_SSH_BIN.
fn fixture(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("plain.txt"), "remote contents").unwrap();
    std::fs::write(dir.join("other.txt"), "other").unwrap();
    let shim = dir.join("fake-ssh");
    std::fs::write(
        &shim,
        "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\nexec /bin/sh -c \"$1\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
    dir
}

#[test]
fn remote_read_edit_save_roundtrip() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("rw");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"remote contents\"");
    // The buffer remembers the full remote path.
    assert_eq!(run(&mut i, "(buffer-file-name)"), format!("\"{}\"", remote));
    // Edit and save over the transport.
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!"
    );
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "nil");
}

#[test]
fn remote_new_file_and_write() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("new");
    let remote = format!("/ssh:fake@host:{}/created.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    run(&mut i, "(insert \"fresh\")");
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("created.txt")).unwrap(),
        "fresh"
    );
}

#[test]
fn remote_directory_opens_dired_and_visits() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("dired");
    let remote = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    // Landed in a read-only dired buffer listing the remote entries.
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.contains("plain.txt") && text.contains("subdir"),
        "listing: {}",
        text
    );
    // default-directory carries the remote prefix.
    let dd = run(&mut i, "(default-directory)");
    assert!(
        dd.starts_with("\"/ssh:fake@host:"),
        "default-directory: {}",
        dd
    );
    // RET on the plain.txt row opens the remote file.
    let rows: Vec<String> = {
        let t = text.trim_matches('"');
        t.split("\\n").map(|s| s.to_string()).collect()
    };
    let target = rows
        .iter()
        .position(|r| r.ends_with("plain.txt") && !r.contains("->"))
        .expect("plain.txt row");
    run(&mut i, "(goto-char (point-min))");
    for _ in 0..target {
        feed_keys(&mut i, &ed, "n").unwrap();
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"remote contents\"");
    assert!(run(&mut i, "(buffer-file-name)").contains("/ssh:fake@host:"));
}

#[test]
fn typing_ssh_path_over_prefill_shadows() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("shadow");
    // C-x C-f prefills a local directory; typing /ssh:... on top of it
    // must shadow the prefix (the // junction) and reach the remote.
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    for c in remote.chars() {
        core::commands::handle_key(&mut i, &ed, core::commands::Key::Char(c as i64));
    }
    // Remote input: the panel deliberately lists nothing.
    let rows = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .map(|p| p.rows.len())
        .unwrap_or(0);
    assert_eq!(rows, 0, "no remote completion in the panel");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"remote contents\"");
}

#[test]
fn remote_connect_failure_reports_cleanly() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("fail");
    // A shim that always fails, as an unreachable host would.
    let shim = dir.join("fail-ssh");
    std::fs::write(
        &shim,
        "#!/bin/sh\necho 'Connection refused' >&2\nexit 255\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
    let out = run(&mut i, "(find-file-internal \"/ssh:down@host:/etc/hosts\")");
    assert!(out.starts_with("ERROR:"), "expected clean error: {}", out);
    assert!(out.contains("Connection refused"), "error detail: {}", out);
}

/// Row index of the first row naming NAME, skipping a symlink's " ->
/// target" row (see dired_ops_tests.rs's identical helper).
fn row_index(rows: &[String], name: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(name) && !r.contains("->"))
        .unwrap_or_else(|| panic!("no row containing {}: {:?}", name, rows))
}

fn rows_of(i: &mut Interp) -> Vec<String> {
    let listing = run(i, "(buffer-string)");
    listing
        .trim_matches('"')
        .split("\\n")
        .map(|s| s.to_string())
        .collect()
}

// --- M41: dired file operations over the /ssh: transport ------------

#[test]
fn remote_dired_delete_removes_shim_target() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("ops-del");
    let remote = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(dired {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    let idx = row_index(&rows_of(&mut i), "plain.txt");
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "D").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Delete 1 file(s)?"), "echo: {}", echo);
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Deleted 1 file(s)");
    assert!(
        !dir.join("plain.txt").exists(),
        "the shim should have deleted the local file `rm` ran against"
    );
}

#[test]
fn remote_dired_rename_moves_shim_target() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("ops-ren");
    let remote = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(dired {:?})", remote));
    let idx = row_index(&rows_of(&mut i), "plain.txt");
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "R").unwrap();
    assert!(ed.borrow().minibuffer.is_some(), "R should prompt");
    // Type the full remote destination over the /ssh: prefill -- the
    // `//' junction at the boundary shadows the prefill, same trick as
    // `typing_ssh_path_over_prefill_shadows' above.
    let dest = format!("/ssh:fake@host:{}/renamed.txt", dir.to_str().unwrap());
    for c in dest.chars() {
        handle_key(&mut i, &ed, Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.starts_with("Renamed "), "echo: {}", echo);
    assert!(!dir.join("plain.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("renamed.txt")).unwrap(),
        "remote contents"
    );
}

// --- M41 review #3: remote dest-is-directory now matches the shell ------

#[test]
fn remote_dired_copy_into_existing_dir_lands_at_basename() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("ops-copy-dir");
    let remote = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(dired {:?})", remote));
    let idx = row_index(&rows_of(&mut i), "plain.txt");
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "C").unwrap();
    assert!(ed.borrow().minibuffer.is_some(), "C should prompt");
    // `subdir` already exists remotely (fixture) -- file-directory-p must
    // now probe that over the shim (`test -d`) instead of always saying
    // "no" for a /ssh: destination, so the file lands inside it, and the
    // echo agrees with what the shim's real `cp` did.
    let dest = format!("/ssh:fake@host:{}/subdir", dir.to_str().unwrap());
    for c in dest.chars() {
        handle_key(&mut i, &ed, Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Copied 1 file(s)", "echo: {}", echo);
    assert_eq!(
        std::fs::read_to_string(dir.join("subdir/plain.txt")).unwrap(),
        "remote contents"
    );
    assert!(
        dir.join("plain.txt").exists(),
        "source should survive a copy"
    );
}

#[test]
fn remote_dired_rename_into_existing_dir_updates_visiting_buffer_and_saves() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("ops-ren-dir");
    let remote_file = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote_file));
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    let remote_dir = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(dired {:?})", remote_dir));
    let idx = row_index(&rows_of(&mut i), "plain.txt");
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "R").unwrap();
    let dest = format!("/ssh:fake@host:{}/subdir", dir.to_str().unwrap());
    for c in dest.chars() {
        handle_key(&mut i, &ed, Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Renamed 1 file(s)", "echo: {}", echo);

    // The visiting buffer's `file` must be the real final location
    // (dir/subdir/plain.txt), not the bare directory dired renamed into
    // -- otherwise `save-buffer` below would try `cat > <a directory>`.
    let expected_remote_path = format!("/ssh:fake@host:{}/subdir/plain.txt", dir.to_str().unwrap());
    let info = run(
        &mut i,
        &format!(
            "(let ((b (get-file-buffer {:?}))) (list (buffer-file-name b) (buffer-name b)))",
            expected_remote_path
        ),
    );
    assert!(info.contains(&expected_remote_path), "info: {}", info);
    assert!(info.contains("\"plain.txt\""), "info: {}", info);

    run(
        &mut i,
        &format!("(set-buffer (get-file-buffer {:?}))", expected_remote_path),
    );
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    let save_out = run(&mut i, "(save-buffer)");
    assert!(!save_out.starts_with("ERROR:"), "save-buffer: {}", save_out);
    assert_eq!(
        std::fs::read_to_string(dir.join("subdir/plain.txt")).unwrap(),
        "remote contents!"
    );
}

#[test]
fn cross_boundary_copy_local_to_remote_errors_cleanly() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("cross");
    let local_src = dir.join("plain.txt");
    let remote_dst = format!("/ssh:fake@host:{}/copy.txt", dir.to_str().unwrap());
    let out = run(
        &mut i,
        &format!(
            "(copy-file {:?} {:?})",
            local_src.to_str().unwrap(),
            remote_dst
        ),
    );
    assert!(out.starts_with("ERROR:"), "out: {}", out);
    assert!(
        out.contains("same host"),
        "expected a same-host boundary error: {}",
        out
    );
}

// --- M75: save-buffer's on-disk-conflict guard now covers /ssh: too ----
//
// M62 added the guard for local files only; these are its remote
// counterparts, mirroring `save_kill_hooks_tests.rs`'s local versions
// (see that file's `save_buffer_refuses_to_clobber_an_external_change`
// and friends).

/// Replace a shim's remote-side effects by directly overwriting the
/// fixture file that the /ssh: path maps to on disk -- this stands in
/// for "someone else changed the file on the remote host" without
/// needing a second ssh session.
fn external_write(dir: &std::path::Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).unwrap();
}

/// The digest has to catch a change that leaves the file's LENGTH untouched.
/// That case is the entire reason `DiskState::RemoteContent` stores a content
/// digest rather than the `ls -ld` line or an mtime/size pair -- both of which
/// were measured and rejected precisely because a same-length edit is invisible
/// to them (see that type's doc).
///
/// It needs its own test because every OTHER remote-save test here happens to
/// change the length as well, so the `len` field alone carries their
/// assertions. M75's mutation run proved the gap: making `content_digest`
/// return a constant left all 20 tests green.
#[test]
fn remote_save_same_length_different_content_is_detected() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-samelen");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    // Both strings are exactly 15 bytes -- only the digest can tell them apart.
    external_write(&dir, "plain.txt", "totally other!!");

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "a same-length external change must still be a conflict: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "totally other!!",
        "the other writer's content must survive"
    );
}

#[test]
fn remote_save_refuses_to_clobber_external_change() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-conflict");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    external_write(&dir, "plain.txt", "changed on disk by someone else");

    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected an error, got: {}", r);
    assert!(
        r.contains("changed on disk"),
        "message should mention the conflict: {}",
        r
    );
    // The core assertion: the external content must survive untouched.
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "changed on disk by someone else"
    );
}

#[test]
fn remote_save_second_attempt_overwrites_after_warning() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-force-retry");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    external_write(&dir, "plain.txt", "changed on disk by someone else");

    let r1 = run(&mut i, "(save-buffer)");
    assert!(r1.starts_with("ERROR"), "expected a refusal first: {}", r1);
    // A PLAIN retry -- no FORCE flag. Being warned once is what earns the
    // overwrite, via `save_conflict_ack`; `(save-buffer t)` is a separate
    // path with its own test (`remote_save_force_overwrites_on_first_try`).
    // This test used to pass `t` here, which meant it never exercised the
    // ack path at all: M75's mutation run caught that by deleting the ack
    // write and watching this test stay green.
    let r2 = run(&mut i, "(save-buffer)");
    assert!(
        !r2.starts_with("ERROR"),
        "retry after warning failed: {}",
        r2
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!"
    );
}

#[test]
fn remote_save_ack_binds_observed_state_not_path() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-ack-rebind");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    external_write(&dir, "plain.txt", "first external change");
    let r1 = run(&mut i, "(save-buffer)");
    assert!(r1.starts_with("ERROR"), "expected first refusal: {}", r1);

    // A SECOND, different external edit lands before the retry -- the
    // ack from the first refusal must not cover this new state.
    external_write(&dir, "plain.txt", "second external change");
    let r2 = run(&mut i, "(save-buffer)");
    assert!(
        r2.starts_with("ERROR"),
        "the stale ack must not authorize overwriting the SECOND external change: {}",
        r2
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "second external change"
    );
}

#[test]
fn remote_save_no_false_positive_when_untouched() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-clean");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "unexpected refusal: {}", r);
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!"
    );
}

#[test]
fn remote_save_twice_in_a_row_does_not_false_positive() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-twice");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    let r1 = run(&mut i, "(save-buffer)");
    assert!(!r1.starts_with("ERROR"), "first save failed: {}", r1);
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!"
    );

    // Second save right after: the baseline must have been refreshed by
    // the first write, or this falsely reports a conflict against our
    // own save.
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"?\")");
    let r2 = run(&mut i, "(save-buffer)");
    assert!(!r2.starts_with("ERROR"), "second save failed: {}", r2);
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!?"
    );

    // The real property under test isn't observable from elisp (there's
    // no way to peek at `disk_state`, and a false "second save didn't
    // error" result is indistinguishable from a correct one on its own
    // -- e.g. if the post-write refresh degraded to `Unknown` instead of
    // a real `RemoteContent` digest, saves 1 and 2 above would look
    // exactly this same way). So force the refreshed baseline to prove
    // itself: an external change AFTER the second save must still be
    // caught by a THIRD save. If the refresh after save 2 had silently
    // reverted to `Unknown`, this would fall into the "no baseline,
    // never a conflict" policy and be wrongly allowed through.
    external_write(&dir, "plain.txt", "changed after the second save");
    let r3 = run(&mut i, "(save-buffer)");
    assert!(
        r3.starts_with("ERROR"),
        "the baseline refreshed by save 2 must still detect a THIRD change: {}",
        r3
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "changed after the second save",
        "a refused third save must not clobber the external change"
    );
}

#[test]
fn remote_save_force_overwrites_on_first_try() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-force-first");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    external_write(&dir, "plain.txt", "changed on disk by someone else");

    // FORCE skips the guard outright -- no refusal at all, unlike the
    // retry-after-refusal path above.
    let r = run(&mut i, "(save-buffer t)");
    assert!(!r.starts_with("ERROR"), "force save failed: {}", r);
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!"
    );
}

#[test]
fn remote_save_conflict_when_deleted_remotely() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-deleted");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    std::fs::remove_file(dir.join("plain.txt")).unwrap();

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "deleting the remote file must be treated as a conflict, not silently recreated: {}",
        r
    );
    assert!(r.contains("changed on disk"), "message: {}", r);
    assert!(
        !dir.join("plain.txt").exists(),
        "a refused save must not recreate the file"
    );
}

#[test]
fn remote_new_file_saves_cleanly_when_uncontested() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-new-clean");
    let remote = format!("/ssh:fake@host:{}/created.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(insert \"fresh\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "unexpected refusal: {}", r);
    assert_eq!(
        std::fs::read_to_string(dir.join("created.txt")).unwrap(),
        "fresh"
    );
}

#[test]
fn remote_new_file_conflict_when_someone_else_creates_it_first() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-new-race");
    let remote = format!("/ssh:fake@host:{}/created.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(insert \"fresh\")");
    // Someone else races us and creates the file first.
    external_write(&dir, "created.txt", "someone else's file");

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "a file that appeared since open must be a conflict: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("created.txt")).unwrap(),
        "someone else's file"
    );
}

#[test]
fn remote_save_guard_failure_refuses_instead_of_writing() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("rsave-guard-fail");
    // A brand-new remote file (`disk_state` starts as `Absent`), not an
    // existing one. This matters for what this test can actually catch:
    // with an existing file (`RemoteContent` baseline), a guard bug that
    // swallows the reread's error into "file doesn't exist" would
    // *coincidentally* still refuse the save (Absent-vs-RemoteContent
    // looks like a conflict) even though it got there for the wrong
    // reason -- the assertions below wouldn't be able to tell the two
    // apart. Starting from `Absent` closes that hole: a guard bug that
    // maps "reread failed" to "treat as if the file doesn't exist"
    // hits the "nobody raced us" branch here and proceeds to write,
    // which the assertions below WILL catch.
    let remote = format!("/ssh:fake@host:{}/created.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(insert \"fresh\")");

    // Swap in a shim that fails every command EXCEPT a `>`-redirected
    // one (i.e. `write_file`'s `cat > path`) -- this isolates the guard
    // from the write it's supposed to gate. An earlier version of this
    // test used a shim that failed unconditionally, which also broke
    // `write_file` itself: removing the guard entirely (e.g.
    // `read_file(&rp).map_err(...)?` weakened to
    // `.unwrap_or(None)`) still made this test pass, because the save
    // failed downstream at the write step regardless of whether the
    // guard ran. Confirmed by mutation: reverting the guard to
    // `.unwrap_or(None)` with the OLD shim left this test green.
    let dead_shim = dir.join("dead-ssh");
    std::fs::write(
        &dead_shim,
        "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\ncase \"$1\" in\n  *'>'*) exec /bin/sh -c \"$1\" ;;\nesac\nexit 255\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dead_shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", dead_shim.to_str().unwrap());

    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "a guard that can't run must refuse the save, not proceed: {}",
        r
    );
    assert!(
        !dir.join("created.txt").exists(),
        "the guard failing must not let the write through and create the file"
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "t",
        "a refused save must leave the buffer's modified flag alone"
    );
}

// --- M76: write_file is length-gated + write-through, not `mv` --------
//
// M76's FIRST attempt staged into a temp file and `mv`'d it over PATH,
// betting a dropped connection kills the remote command before `mv`
// runs. That bet is wrong for real ssh: without a pty, a dropped
// connection sends a clean EOF to the remote command's stdin, not
// SIGHUP -- `cat` reading a closed pipe returns 0, so `cat > "$t" &&
// mv "$t" path` cheerfully commits a truncated temp file. Measured on
// a real TUI: editor said `Wrote`, remote file was a clean-looking
// half a file -- worse than the pre-M76 bug, which was at least loud.
//
// The design here instead length-gates the transfer before ever
// touching PATH, and commits by write-through (`cat "$t" > path`)
// rather than `mv`, because `mv` was measured to regress symlinks,
// hardlinks, ownership, and `-`-prefixed filenames. See `write_file`'s
// doc comment in `remote.rs` for the full writeup.
//
// Every test below states, in its doc comment, why it goes red against
// the OLD `cat > path` shape -- reviewer flagged that four of the five
// original M76 tests stayed green even with the entire fix reverted.

/// Build a shim that truncates every remote command's stdin to N bytes
/// via a clean close (`dd`), THEN runs the real compound command with
/// whatever exit code it produces -- modeling what a dropped ssh
/// connection actually does without a pty (clean EOF, not a killed
/// process or a forced nonzero exit). Using a nonzero-exit-code shim
/// here would test a scenario that doesn't happen with real ssh and
/// was exactly how M76's first attempt got measured as "safe" when it
/// wasn't.
fn swap_shim_truncated(dir: &std::path::Path, n: usize) {
    let shim = dir.join("truncated-ssh");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\ndd bs=1 count={} 2>/dev/null | /bin/sh -c \"$1\"\n",
            n
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
}

/// Make every invocation of CMD_NAME on the remote fail (`exit 1`),
/// while every other command still runs for real: prepends a directory
/// holding only a stub CMD_NAME to PATH before running the compound
/// command. Narrow on purpose -- unlike a shim that fails everything,
/// this can't make a test pass for the wrong reason (M75's mistake).
/// An ssh shim that runs the remote command for real (so it genuinely
/// commits) and then forces an exit status the script itself can never
/// produce -- standing in for the remote being interrupted from OUTSIDE:
/// ssh's own 255 on a dropped connection, a signal death, an OOM kill.
///
/// M76 tail review: without this, every non-zero status was answered with
/// "Remote file untouched", which is a promise the client cannot keep once
/// the status did not come from the script's own three exits.
fn swap_shim_forcing_exit_code(dir: &std::path::Path, code: i32) {
    let shim = dir.join(format!("forced-{}-ssh", code));
    std::fs::write(
        &shim,
        format!(
            // Only the WRITE command gets the forced status. Forcing it on
            // every invocation would also hit M75's pre-save reread, so the
            // conflict guard would refuse before the write ever ran and the
            // test would pass for the wrong reason -- the exact shape M75's
            // mutation run caught four times. `.reticle-save-` appears only in
            // the write command's staged-file name, and is matched with its
            // leading dot and trailing dash rather than as a bare substring.
            // That anchoring dates from when the marker was still `se-save`,
            // which an ordinary remote path contained by accident -- the dash
            // in `parse-save.v` supplied the `se-save` (M77 tail review,
            // finding 3; all three shims here were widened the same way, this
            // M76 one included). The current marker is long enough that no
            // such collision is plausible, but the anchoring is kept: it costs
            // nothing, and the next rename could be short again.
            "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\ncase \"$1\" in\n  *.reticle-save-*) /bin/sh -c \"$1\"; exit {} ;;\n  *) exec /bin/sh -c \"$1\" ;;\nesac\n",
            code
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
}

fn swap_shim_dying_command(dir: &std::path::Path, cmd_name: &str) {
    let fakebin = dir.join(format!("fakebin-{}", cmd_name));
    std::fs::create_dir_all(&fakebin).unwrap();
    std::fs::write(fakebin.join(cmd_name), "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            fakebin.join(cmd_name),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let shim = dir.join(format!("dying-{}-ssh", cmd_name));
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\nPATH=\"{}:$PATH\" exec /bin/sh -c \"$1\"\n",
            fakebin.to_str().unwrap()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
}

/// List entries directly in DIR whose name contains `.reticle-save-` -- the
/// crash-safe write's temp-file marker. Used to assert presence/absence
/// of leftovers.
/// Points `$TMPDIR` at a private directory for the duration of a test, and
/// restores the previous value on drop (including during a panic unwind).
///
/// The two tests that assert "the fallback left nothing behind in TMPDIR"
/// originally scanned the real, shared `std::env::temp_dir()` for a global
/// `reticle-save.*` prefix. That makes the assertion depend on
/// everything that has ever run on the machine: M76's own mutation runs
/// deliberately break the cleanup step, so a single mutation pass left a
/// stale `reticle-save.<pid>` behind and every later run of these two
/// tests failed on it -- including the mutation runner's baseline check,
/// which then refused to run at all. The temp dir the editor picks is
/// whatever `$TMPDIR` says, and the ssh shim is a child of this test
/// process, so pointing that at a per-test directory makes the assertion
/// hermetic instead of history-dependent.
struct TmpdirGuard(Option<std::ffi::OsString>);

impl TmpdirGuard {
    fn set(dir: &std::path::Path) -> TmpdirGuard {
        let prev = std::env::var_os("TMPDIR");
        std::env::set_var("TMPDIR", dir);
        TmpdirGuard(prev)
    }
}

impl Drop for TmpdirGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("TMPDIR", v),
            None => std::env::remove_var("TMPDIR"),
        }
    }
}

fn reticle_save_temp_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".reticle-save-"))
        .collect()
}

/// An exit status the script never produces means the remote was cut off
/// from outside -- possibly partway through the commit. Claiming the file
/// is untouched there would be a promise about the exact scenario this
/// milestone exists for, so the message has to say it doesn't know.
///
/// Red before the M76 tail-review fix: the `code != 0` arm answered EVERY
/// non-zero status with "Remote file untouched", including this one, while
/// the commit had in fact already happened.
#[test]
fn remote_write_unknown_exit_code_does_not_promise_the_file_is_untouched() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-unknown-code");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    // 42 is not one of the script's own exits (0 / 1 / 3).
    swap_shim_forcing_exit_code(&dir, 42);

    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected a reported failure: {}", r);
    // The commit really did land -- that is the whole point: the status
    // is unknown to us, not a guarantee that nothing happened.
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!",
        "the forced exit code must not stop the command from committing"
    );
    assert!(
        !r.contains("untouched"),
        "must not promise the file is untouched when the status is unknown: {}",
        r
    );
    assert!(
        r.contains("unknown"),
        "the message should say the status is unknown: {}",
        r
    );
}

/// Scenario 1 (the milestone's core safety property): a connection that
/// drops mid-transfer delivers a clean EOF partway through TEXT, not a
/// killed remote process. The length gate must see fewer bytes than
/// promised and refuse to commit.
///
/// Red under the OLD `cat > path`: there's no length check at all --
/// `cat` just writes whatever it got (5 of 16 bytes) straight into
/// PATH, so the file ends up truncated to "remot" instead of surviving
/// untouched, and the FIRST M76 attempt's `mv`-based version is red for
/// the same reason described above (`cat` sees a clean EOF, returns 0,
/// `mv` commits the truncated temp file).
///
/// Also covers scenario 9's REJECTION side (the SUCCESS side is covered
/// by the several "no leftover temp file" assertions elsewhere): this
/// exercises the length-gate's own `rm -f "$t"` cleanup, which nothing
/// else in this file reaches. Reviewer found NONE of the (then) nine
/// tests would go red if that `rm -f` were deleted -- these two
/// assertions close that gap.
#[test]
fn remote_write_early_eof_is_rejected() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-eof");
    let private_tmp = dir.join("tmpdir");
    std::fs::create_dir_all(&private_tmp).unwrap();
    let _tmp = TmpdirGuard::set(&private_tmp);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    // "remote contents!" is 16 bytes; cut the stream off at 5.
    swap_shim_truncated(&dir, 5);
    let r = run(&mut i, "(save-buffer)");
    assert!(r.starts_with("ERROR"), "expected a save failure: {}", r);
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents",
        "an early EOF must never reach the write-through commit"
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "t",
        "a rejected save must leave the buffer's modified flag alone"
    );
    assert!(
        reticle_save_temp_files(&dir).is_empty(),
        "the length gate's rejection must clean up its same-directory temp file"
    );
    // Deliberately unfiltered. `private_tmp` is created empty by this test
    // and TMPDIR points at it, so *anything* left here is a leftover. An
    // earlier version filtered on the `reticle-save.` prefix, which made the
    // assertion vacuous: rename the marker in the product and this still
    // passes, because there are still no `reticle-save.*` entries. A mutation
    // run confirmed exactly that (rename-tail M2 SURVIVED).
    //
    // Note what this still does NOT cover: the fallback temp file's *name*.
    // A successful save deletes it, so no assertion made after the fact can
    // see what it was called. Observing the name would need a shim that
    // captures the command mid-flight, or a forced failure down the
    // "staged copy kept at $t" branch. Recorded rather than papered over.
    let tmp_leftovers: Vec<_> = std::fs::read_dir(&private_tmp)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        tmp_leftovers.is_empty(),
        "the length gate's rejection must not leave a TMPDIR fallback temp file either: {:?}",
        tmp_leftovers
    );
}

/// Reviewer finding (fault injection): the commit branch used to be
/// `if cat "$t" > path; then rm -f "$t"; else ...; fi` with no explicit
/// `exit 0` -- so the WHOLE script's exit code was that of the LAST
/// command run, `rm -f`, not of the `cat` that actually decided success.
/// A cleanup `rm` that itself fails (stale permissions on the temp
/// file's directory, a racing cleanup sweep, anything) then makes a
/// save that fully succeeded get reported as a failure: `write_file`
/// takes the `code != 0` branch and tells the user "Remote file
/// untouched", which is false -- the content is already committed.
/// Worse, the M75 save-conflict guard never refreshes its baseline on
/// what it believes was a failed save, so the NEXT save spuriously
/// reports "changed on disk by someone else" against content that is,
/// in fact, this buffer's own last save.
#[test]
fn remote_write_success_is_not_masked_by_a_failing_cleanup() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-cleanup-fails");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    // The commit's OWN `rm -f "$t"` cleanup fails; the write-through
    // `cat "$t" > path` immediately before it still succeeded.
    swap_shim_dying_command(&dir, "rm");
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "a failing CLEANUP must not be reported as a failed save: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!",
        "the write-through commit already landed before cleanup ran"
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "a save the code believes succeeded must clear the modified flag"
    );
}

/// Scenario 2: PATH is a symlink. Write-through must go through it
/// (updating whatever it points at) rather than replacing it with a
/// plain file.
///
/// Red under `mv`-based staging (M76's first attempt): `mv "$t" path`
/// unlinks PATH and relinks the name to the temp file's inode, which
/// replaces the symlink with a plain file -- the link and whatever else
/// pointed at its target stop seeing updates.
///
/// NOT red under the OLD `cat > path`: that shape writes through the
/// redirect too (no injected truncation here to trip it up), so it
/// passes this test unchanged. This test's only job is guarding
/// against the `mv`-based regression, not against the pre-M76 bug --
/// scenario 1 (`remote_write_early_eof_is_rejected`) is what catches
/// that one.
#[test]
fn remote_write_through_symlink_updates_target_not_the_link() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-symlink");
    std::fs::write(dir.join("real-target.txt"), "original").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("real-target.txt"), dir.join("via-link.txt")).unwrap();
    let remote = format!("/ssh:fake@host:{}/via-link.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-string)"), "\"original\"");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();

    let link_meta = std::fs::symlink_metadata(dir.join("via-link.txt")).unwrap();
    assert!(
        link_meta.file_type().is_symlink(),
        "via-link.txt must still be a symlink after saving through it"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("real-target.txt")).unwrap(),
        "original!",
        "the symlink's target must carry the new content"
    );
}

/// Scenario 3: PATH is one of two hardlinked names. Write-through opens
/// PATH and truncates/rewrites the SAME inode, so the link count and
/// the other name's content must both survive.
///
/// Red under `mv`-based staging: `mv "$t" path` unlinks PATH from the
/// shared inode and relinks the name onto the temp file's (new) inode,
/// so the other hardlinked name stops seeing the update and nlink on
/// the surviving inode drops.
#[test]
fn remote_write_through_hardlink_preserves_link_count() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-hardlink");
    std::fs::write(dir.join("orig-name.txt"), "shared").unwrap();
    std::fs::hard_link(dir.join("orig-name.txt"), dir.join("other-name.txt")).unwrap();
    let remote = format!("/ssh:fake@host:{}/other-name.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(dir.join("other-name.txt")).unwrap();
        assert_eq!(meta.nlink(), 2, "the hardlink must survive the save");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("orig-name.txt")).unwrap(),
        "shared!",
        "the OTHER hardlinked name must see the update (same inode)"
    );
}

/// Scenario 4: PATH's directory is read-only but PATH itself is
/// writable (GNU's own `file-precious-flag` docstring calls this out as
/// a case its fallback handles). The primary same-directory temp file
/// can't be created there, so the write must fall back to `$TMPDIR`
/// and still land the save.
///
/// Red under the OLD `cat > path`: irrelevant here, since `cat > path`
/// never creates a same-directory temp file to begin with -- included
/// to prove the new fallback logic actually works, not as a regression
/// check against the old code.
///
/// Caveat (generic to Unix permission tests, not specific to this
/// one): if these tests are ever run as root, `chmod 0o555` on `rodir`
/// is a no-op as far as write access goes -- root ignores the
/// permission bits -- so the primary same-directory temp file would be
/// created successfully, the TMPDIR fallback would never engage, and
/// this test would still pass, but for the wrong reason (it wouldn't
/// actually be testing the fallback anymore).
#[test]
fn remote_write_falls_back_to_tmpdir_when_directory_is_read_only() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("crash-safe-rodir");
    let private_tmp = dir.join("tmpdir");
    std::fs::create_dir_all(&private_tmp).unwrap();
    let _tmp = TmpdirGuard::set(&private_tmp);
    let rodir = dir.join("rodir");
    std::fs::create_dir_all(&rodir).unwrap();
    std::fs::write(rodir.join("ro-target.txt"), "original").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            rodir.join("ro-target.txt"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        std::fs::set_permissions(&rodir, std::fs::Permissions::from_mode(0o555)).unwrap();
    }
    let remote = format!("/ssh:fake@host:{}/ro-target.txt", rodir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    let r = run(&mut i, "(save-buffer)");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rodir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(
        !r.starts_with("ERROR"),
        "save via TMPDIR fallback failed: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(rodir.join("ro-target.txt")).unwrap(),
        "original!"
    );
    assert!(
        reticle_save_temp_files(&rodir).is_empty(),
        "no temp file should be left in the read-only dir"
    );
    // Deliberately unfiltered. `private_tmp` is created empty by this test
    // and TMPDIR points at it, so *anything* left here is a leftover. An
    // earlier version filtered on the `reticle-save.` prefix, which made the
    // assertion vacuous: rename the marker in the product and this still
    // passes, because there are still no `reticle-save.*` entries. A mutation
    // run confirmed exactly that (rename-tail M2 SURVIVED).
    //
    // Note what this still does NOT cover: the fallback temp file's *name*.
    // A successful save deletes it, so no assertion made after the fact can
    // see what it was called. Observing the name would need a shim that
    // captures the command mid-flight, or a forced failure down the
    // "staged copy kept at $t" branch. Recorded rather than papered over.
    let tmp_leftovers: Vec<_> = std::fs::read_dir(&private_tmp)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        tmp_leftovers.is_empty(),
        "no leftovers of any name in TMPDIR: {:?}",
        tmp_leftovers
    );
}

/// Scenario 5: a filename starting with `-` (a real RTL pattern --
/// e.g. a testbench named `-rf.v` after some naming convention). It
/// must never be handed to `mv`/`cp`/`rm` as a bare positional argument,
/// where getopt would parse it as an option.
///
/// Red under `mv`-based staging: `mv "$t" -rf.v` gets `-rf.v` parsed by
/// `mv`'s getopt as flags, not a destination, so the save silently does
/// the wrong thing (or errors) every time. Write-through never passes
/// PATH as a `mv`/`cp`/`rm` argument -- only as a shell redirect target
/// -- so this can't happen.
#[test]
fn remote_write_dash_prefixed_filename_is_not_parsed_as_an_option() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("crash-safe-dashname");
    std::fs::write(dir.join("-rf.v"), "original").unwrap();
    let remote = format!("/ssh:fake@host:{}/-rf.v", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-string)"), "\"original\"");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"!\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "save of a `-`-prefixed name failed: {}",
        r
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("-rf.v")).unwrap(),
        "original!"
    );
    assert!(
        reticle_save_temp_files(&dir).is_empty(),
        "no leftover temp file after a successful save"
    );
}

/// Scenario 6: an empty buffer (0 bytes) must not be treated as a
/// truncation by the length gate.
///
/// Red under `mv`-based staging in a different way than the others:
/// not wrong on its own, but included because a naive length gate
/// implementation could special-case "n == 0 means something went
/// wrong" -- this pins that 0 is a legitimate length.
#[test]
fn remote_write_empty_buffer_succeeds() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-empty");
    let remote = format!("/ssh:fake@host:{}/empty.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("empty.txt")).unwrap(), "");
    assert!(
        reticle_save_temp_files(&dir).is_empty(),
        "no leftover temp file after a successful save"
    );
}

/// Scenario 7: an ordinary existing file. Write-through must rewrite
/// the SAME inode (not `mv` a new one over it) and preserve its mode.
///
/// Red under `mv`-based staging: `mv` gives PATH the temp file's inode
/// (a new one, created fresh by `cat >`), so the inode number changes
/// even though nothing else about this case looks wrong from the
/// outside -- this is the assertion that catches it. Red under the OLD
/// `cat > path` for mode: `cat >` on its own leaves whatever mode the
/// shell's default umask gives a newly truncated-and-reopened file
/// only if the file didn't already exist; since PATH already exists
/// here `cat > path` actually preserves the mode too (it doesn't
/// recreate the inode) -- so this assertion alone doesn't discriminate
/// the OLD code, but the inode-identity assertion does discriminate
/// M76's own FIRST (`mv`-based) attempt, which is the regression this
/// milestone's rewrite is actually guarding against.
#[test]
fn remote_write_preserves_inode_and_mode_on_existing_file() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("crash-safe-inode");
    let path = dir.join("plain.txt");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    #[cfg(unix)]
    let ino_before = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&path).unwrap().ino()
    };
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "remote contents!");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(
            meta.ino(),
            ino_before,
            "write-through must reuse PATH's inode"
        );
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o640,
            "mode must survive the save"
        );
    }
    assert!(
        reticle_save_temp_files(&dir).is_empty(),
        "no leftover temp file after a successful save"
    );
}

/// Scenario 8: PATH is actually a directory. The commit step
/// (`cat "$t" > path`) must fail honestly (`cat: Is a directory`), not
/// report success while leaving the directory untouched.
///
/// Driven directly through `remote::write_file` -- `find-file-internal`
/// on a directory opens dired, not a text buffer, so there's no elisp
/// path that lets a save target a directory; this exercises the same
/// remote command a stale/racing buffer could otherwise hit.
#[test]
fn remote_write_target_is_directory_fails_honestly() {
    let _guard = lock();
    let dir = fixture("crash-safe-dirtarget");
    // `fixture` already created `dir/subdir`.
    let rp = core::remote::RemotePath {
        host: "fake@host".to_string(),
        path: dir.join("subdir").to_str().unwrap().to_string(),
    };
    let r = core::remote::write_file(&rp, "won't fit in a directory");
    assert!(
        r.is_err(),
        "writing to a directory must not report success: {:?}",
        r
    );
    let msg = r.unwrap_err();
    assert!(
        msg.contains("staged copy kept"),
        "a commit-step failure must point at the staged copy: {}",
        msg
    );
    // Scenario 9 (failure-path leftover): a commit-step failure must
    // NOT clean up the temp file -- it holds the user's unsaved work.
    let leftovers = reticle_save_temp_files(&dir);
    assert!(
        !leftovers.is_empty(),
        "a failed commit must leave the staged copy behind, not discard it"
    );
}

// ---------------------------------------------------------------------
// M77: wall-clock timeout around `remote::run()`.
// ---------------------------------------------------------------------

/// RETICLE_REMOTE_TIMEOUT is process-global state, same shape as
/// `TmpdirGuard` above (set for the test's duration, restore the previous
/// value on drop -- including during a panic unwind -- rather than just
/// unconditionally removing it, since a later test in the same binary
/// might rely on inheriting whatever the environment had before).
struct RemoteTimeoutGuard(Option<std::ffi::OsString>);

impl RemoteTimeoutGuard {
    fn set(secs: &str) -> RemoteTimeoutGuard {
        let prev = std::env::var_os("RETICLE_REMOTE_TIMEOUT");
        std::env::set_var("RETICLE_REMOTE_TIMEOUT", secs);
        RemoteTimeoutGuard(prev)
    }
}

impl Drop for RemoteTimeoutGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => std::env::set_var("RETICLE_REMOTE_TIMEOUT", v),
            None => std::env::remove_var("RETICLE_REMOTE_TIMEOUT"),
        }
    }
}

/// A shim that never answers: ignores whatever command it was asked to
/// run and `exec`s straight into `sleep 5`. `exec` is load-bearing here,
/// not `sleep 5 &` -- `child.kill()` only reaches the direct child of
/// `Command::spawn`, and without `exec` that child is this shim's own
/// `/bin/sh`, which would leave `sleep` as an orphan that outlives the
/// kill and the test (see M77's spec: verified against the leak this
/// shape avoids). 5s, not 30s, so a kill that somehow fails to land
/// still doesn't stall the suite for long.
fn hang_shim(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let shim = dir.join(name);
    std::fs::write(&shim, "#!/bin/sh\nexec sleep 5\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim
}

/// Like `hang_shim`, but only hangs for the WRITE command (matched on
/// `.reticle-save-`, same trick `run_then_hang_shim` uses) -- every other
/// command, in particular the M75 save-conflict guard's pre-save `test
/// -e`/`cat` probe, answers normally and fast. Needed to reach
/// `write_file`'s OWN timeout message: a shim that hangs on every
/// invocation (plain `hang_shim`) times out at the pre-save probe
/// instead, which never even calls `write_file`.
fn hang_only_for_write_shim(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let shim = dir.join(name);
    std::fs::write(
        &shim,
        "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\ncase \"$1\" in\n  *.reticle-save-*) exec sleep 5 ;;\n  *) exec /bin/sh -c \"$1\" ;;\nesac\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim
}

/// Same as `hang_shim`, but first writes its own (pre-`exec`) pid to
/// PIDFILE. Since `exec` replaces the process image without changing the
/// pid, the pid recorded here is the pid of the `sleep` the shell becomes
/// -- exactly the process `child.kill()` is supposed to reach. Mirrors
/// `LspConnection::pid()`'s reasoning (`crates/elisp/src/lsp.rs`): read
/// the pid from the process itself rather than trusting bookkeeping.
fn hang_shim_with_pidfile(
    dir: &std::path::Path,
    name: &str,
    pidfile: &std::path::Path,
) -> std::path::PathBuf {
    let shim = dir.join(name);
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\necho $$ > {}\nexec sleep 5\n",
            shell_quote_for_shim(pidfile)
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim
}

/// A shim that genuinely runs the requested command (so it can commit for
/// real), and only THEN hangs: models "the remote finished the work but
/// the ack never made it back" -- the exact case M77's write-timeout doc
/// comment says is an honest false negative, not data loss.
///
/// Only the WRITE command gets the hang-after-running treatment (matched
/// by `.reticle-save-`, the write-through commit's own temp-file marker -- same
/// trick `swap_shim_forcing_exit_code` above uses and for the same
/// reason): the M75 save-conflict guard's own pre-save `test -e`/`cat`
/// probe goes through this same `run()` two more times before the write
/// ever happens, and if THOSE hung too, the save would fail at the probe
/// instead of ever reaching the write this test means to exercise.
fn run_then_hang_shim(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let shim = dir.join(name);
    std::fs::write(
        &shim,
        "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\ncase \"$1\" in\n  *.reticle-save-*) /bin/sh -c \"$1\"; exec sleep 5 ;;\n  *) exec /bin/sh -c \"$1\" ;;\nesac\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim
}

/// A shim that answers for real, but only after a delay -- used to prove
/// `RETICLE_REMOTE_TIMEOUT=0` really disables the deadline: a timeout
/// shorter than this delay would otherwise fire and kill it.
fn slow_answer_shim(dir: &std::path::Path, name: &str, delay_secs: &str) -> std::path::PathBuf {
    let shim = dir.join(name);
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\nsleep {}\nexec /bin/sh -c \"$1\"\n",
            delay_secs
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    shim
}

/// Single-quote a path for embedding in a shell script written to disk
/// (distinct from `remote::shell_quote`, which is private to the crate
/// under test and quotes a REMOTE path, not a local one this test harness
/// is writing out).
fn shell_quote_for_shim(p: &std::path::Path) -> String {
    format!("'{}'", p.to_str().unwrap().replace('\'', r"'\''"))
}

fn set_shim(shim: &std::path::Path) {
    std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());
}

/// Verifiable, well-past-64-KB content: `n` lines of `"line NNNNNN\n"`
/// (12 bytes each), so a test can assert total byte count plus the exact
/// first/last line without having to diff the whole thing. Used by the
/// M77 review's large-payload tests below.
fn big_numbered_content(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 12);
    for n in 0..lines {
        s.push_str(&format!("line {:06}\n", n));
    }
    s
}

/// A hung remote must not freeze the editor forever: `find-file` on a
/// dead-slow `/ssh:` path has to fail within the configured timeout, not
/// hang. The `< 5s` wall-clock assertion is the actual proof the timeout
/// fired at all -- without it, a regression back to unbounded waiting
/// would still "pass" a mere `.contains("timed out")` check by waiting
/// out the test harness's own default timeout instead.
#[test]
fn remote_read_timeout_is_not_unbounded_wait() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("timeout-read");
    let _timeout = RemoteTimeoutGuard::set("0.3");
    let shim = hang_shim(&dir, "hang-ssh");
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    let start = std::time::Instant::now();
    let r = run(&mut i, &format!("(find-file-internal {:?})", remote));
    let elapsed = start.elapsed();
    assert!(r.starts_with("ERROR"), "expected a timeout error: {}", r);
    assert!(
        r.to_lowercase().contains("timed out"),
        "error should say timed out: {}",
        r
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "find-file on a hung remote took {:?}, timeout should have fired well under 5s",
        elapsed
    );
}

/// A save that times out must leave the buffer exactly as an unsaved,
/// still-modified buffer -- not silently clear the modified flag, and
/// not claim `Wrote` anywhere the user could see it.
#[test]
fn remote_save_timeout_leaves_buffer_modified() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("timeout-write");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    assert_eq!(run(&mut i, "(buffer-modified-p)"), "t");
    // A more generous timeout than the immediate-hang tests above use:
    // unlike `hang_shim`, this shim has to actually run TWO real commands
    // for real first (the M75 pre-save guard's `test -e` and `cat` probe)
    // before the write step it's actually meant to hang on -- measured
    // (coordinator's own run) up to a 427ms outlier for a trivial shim's
    // fork/exec under this sandbox's syscall-interception overhead, so a
    // 300ms budget across two such invocations is not safely above that.
    let _timeout = RemoteTimeoutGuard::set("2.0");
    // Must hang only for the write step, not the M75 pre-save probe too
    // -- otherwise the save fails at the probe, never reaches
    // `write_file`, and the assertions below on write_file's OWN message
    // text would be checking the wrong error.
    let shim = hang_only_for_write_shim(&dir, "hang-write-ssh");
    set_shim(&shim);
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "expected a save timeout error: {}",
        r
    );
    assert!(
        !r.contains("Wrote"),
        "a timed-out save must never claim success: {}",
        r
    );
    // M77 review: this is `write_file`'s OWN timeout message, not
    // `run_err`'s generic one -- it exists specifically to say both "we
    // didn't confirm a save" and "we don't know what's really there" in
    // the same breath. Pinning down the actual text: deleting that
    // special case and falling back to the generic `run_err` message
    // left every other assertion here green, so only a text assertion
    // catches it going missing again.
    assert!(
        r.contains("NOT saved"),
        "must say the save wasn't confirmed: {}",
        r
    );
    assert!(
        r.contains("remote state unknown"),
        "must say the remote's real state is unknown: {}",
        r
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "t",
        "a timed-out save must leave the modified flag alone"
    );
}

/// Pins down the accepted false negative documented on `write_file`: if
/// the remote actually finishes committing the write and only the final
/// status round-trip is what's slow, killing the local ssh at the
/// timeout still reports failure to the user even though the remote file
/// is, in fact, already correct. This is deliberate (see the doc comment
/// on `write_file` in `remote.rs`) -- fail-closed reporting, not data
/// loss -- and this test exists to keep that fact honest: if a future
/// change makes this scenario report success instead, that would be nice,
/// but if it makes the remote file silently NOT match what was sent,
/// that's the actual regression to watch for.
#[test]
fn remote_save_timeout_after_remote_already_committed_is_an_honest_false_negative() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("timeout-write-committed");
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    feed_keys(&mut i, &ed, "M-> !").unwrap();
    // A generous timeout, not the 0.3s other tests here use: unlike a
    // shim that hangs immediately, this one has to run the FULL write
    // command for real first (temp file, `wc`, write-through commit) --
    // under this sandbox's syscall-interception overhead that measurably
    // eats into a short deadline (measured: the same real work that takes
    // ~40ms unsandboxed can still be mid-flight past 300ms here), so a
    // short deadline risks killing the write before it ever finishes,
    // which would defeat the point of this test (proving the ALREADY-
    // COMMITTED case, not yet another kill-before-it-runs case).
    let _timeout = RemoteTimeoutGuard::set("2.0");
    let shim = run_then_hang_shim(&dir, "commit-then-hang-ssh");
    set_shim(&shim);
    let r = run(&mut i, "(save-buffer)");
    assert!(
        r.starts_with("ERROR"),
        "the client must report failure -- it never saw the remote's ack: {}",
        r
    );
    assert!(!r.contains("Wrote"), "must not claim success: {}", r);
    assert!(
        r.contains("NOT saved") && r.contains("remote state unknown"),
        "this is the scenario write_file's own timeout message exists for \
         -- it must say both \"not saved\" and \"state unknown\": {}",
        r
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "t",
        "the client doesn't know it succeeded, so the buffer must stay modified"
    );
    // The time window this test exists to pin down: the remote command
    // actually ran to completion (proved by the file on disk being the
    // NEW content) even though the editor reports failure.
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "remote contents!",
        "the remote write genuinely committed even though we report a timeout"
    );
}

/// `file-exists-p` must not answer "nil" for "I couldn't find out" -- that
/// would make `find-file` treat a merely-slow remote as "definitely a new
/// file", the exact silent lie M77's spec calls out.
#[test]
fn remote_file_exists_p_timeout_is_an_error_not_nil() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("timeout-exists");
    let _timeout = RemoteTimeoutGuard::set("0.3");
    let shim = hang_shim(&dir, "hang-ssh");
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    let r = run(&mut i, &format!("(file-exists-p {:?})", remote));
    assert!(
        r.starts_with("ERROR"),
        "a timeout must not be reported as nil: {}",
        r
    );
    assert!(r.to_lowercase().contains("timed out"), "{}", r);
}

/// Same as above for `file-directory-p` -- dired's own destination checks
/// rely on this NOT silently reading as "not a directory" when the remote
/// is merely unresponsive.
#[test]
fn remote_file_directory_p_timeout_is_an_error_not_nil() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("timeout-isdir");
    let _timeout = RemoteTimeoutGuard::set("0.3");
    let shim = hang_shim(&dir, "hang-ssh");
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/subdir", dir.to_str().unwrap());
    let r = run(&mut i, &format!("(file-directory-p {:?})", remote));
    assert!(
        r.starts_with("ERROR"),
        "a timeout must not be reported as nil: {}",
        r
    );
    assert!(r.to_lowercase().contains("timed out"), "{}", r);
}

/// The timeout must actually kill the local ssh client, not just give up
/// waiting on it and leak it running in the background.
///
/// Uses a 2s deadline, not the 0.3s other tests here use: the shim has to
/// actually run `echo $$ > pidfile` before it hangs, and under this
/// sandbox's syscall-interception overhead that first fork/exec was
/// measured to occasionally still be in flight past 300ms (the same
/// step takes ~40ms unsandboxed) -- a short deadline here risks killing
/// the shim before it ever gets to write its own pid down, which would
/// make this test fail for a reason that has nothing to do with whether
/// the kill itself works.
#[test]
fn remote_timeout_actually_kills_the_ssh_client() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("timeout-kill");
    let pidfile = dir.join("shim.pid");
    let _timeout = RemoteTimeoutGuard::set("2.0");
    let shim = hang_shim_with_pidfile(&dir, "hang-pid-ssh", &pidfile);
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    let r = run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert!(r.starts_with("ERROR"), "{}", r);
    // Give the pidfile a moment to appear and the kill a moment to land --
    // both happen well within the 5s ceiling `hang_shim` itself is bounded
    // by, and this is polling for a file/process state change, not a bare
    // sleep standing in for synchronization.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut pid: Option<i32> = None;
    while std::time::Instant::now() < deadline {
        if let Ok(s) = std::fs::read_to_string(&pidfile) {
            if let Ok(p) = s.trim().parse::<i32>() {
                pid = Some(p);
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pid = pid.expect("shim never wrote its pidfile");
    // `kill -0` on a reaped/dead pid reports back promptly (ESRCH); no
    // sleep needed beyond what the timeout itself already guaranteed
    // elapsed above.
    let status = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "pid {} should have been killed by the timeout, but kill -0 still finds it alive",
        pid
    );
}

/// `RETICLE_REMOTE_TIMEOUT=0` is a deliberate escape hatch back to
/// unbounded waiting. Proven with a shim that answers for real after
/// 0.5s: a timeout of 0.3s would kill it if the escape hatch weren't
/// working, so success here is only possible with the timeout genuinely
/// disabled.
#[test]
fn remote_timeout_zero_disables_the_deadline() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("timeout-disabled");
    let _timeout = RemoteTimeoutGuard::set("0");
    let shim = slow_answer_shim(&dir, "slow-ssh", "0.5");
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    let r = run(&mut i, &format!("(find-file-internal {:?})", remote));
    assert!(!r.starts_with("ERROR"), "expected success, got: {}", r);
    assert_eq!(run(&mut i, "(buffer-string)"), "\"remote contents\"");
}

/// Existing behavior this milestone must not disturb: a dead connection
/// (ssh's own exit 255) still reads as `Ok(false)`/nil, not an `Err` --
/// only an io-level failure (ssh not runnable at all, or the new
/// timeout) is newly surfaced as an error. `is_dir`/`exists` changed
/// signature from `bool` to `Result<bool, String>` in this milestone;
/// this pins the exit-255 case to the unchanged half of that behavior.
#[test]
fn remote_file_exists_p_false_on_dead_connection_not_error() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("dead-conn-exists");
    let shim = dir.join("dead-ssh");
    std::fs::write(
        &shim,
        "#!/bin/sh\necho 'Connection refused' >&2\nexit 255\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    set_shim(&shim);
    let remote = format!("/ssh:fake@host:{}/plain.txt", dir.to_str().unwrap());
    assert_eq!(run(&mut i, &format!("(file-exists-p {:?})", remote)), "nil");
    assert_eq!(
        run(&mut i, &format!("(file-directory-p {:?})", remote)),
        "nil"
    );
}

// ---------------------------------------------------------------------
// M77 review, finding 5: `run()`'s doc comment argues at length that
// draining stdout/stderr on background threads (instead of reading only
// after `try_wait` succeeds) is load-bearing once a remote command's
// output exceeds one pipe buffer (64 KB) -- and that reverting either
// half of that (reader threads, or the stdin writer thread) makes NO
// existing test fail, because every other test's payload is a few dozen
// bytes. These two tests are the executable backing for the READER-thread
// half of that claim, and only that half.
//
// They do NOT back the stdin-writer half, and saying otherwise was an
// overclaim (M77 tail review, finding 2; the coordinator's mutation run
// confirmed it): `write_file`'s remote script emits only a few bytes of
// status output, so it never fills the stdout pipe, so nothing pushes
// back on our stdin write -- move that write back onto the calling
// thread and `remote_write_large_payload_does_not_deadlock` still
// passes. The stdin half is pinned by
// `remote::tests::run_stdin_and_stdout_both_large_does_not_deadlock`
// instead, which drives the private `run()` against `cat` so the traffic
// is genuinely large in BOTH directions. This test stays as what it
// actually is: a byte-level correctness regression test for `write_file`
// at 586 KB, a size nothing else here covers.
// ---------------------------------------------------------------------

/// Read half: `cat`s a file well past 64 KB over the real (local-shim)
/// transport. Must complete promptly, not deadlock -- the `< 5s`
/// assertion is the actual proof, same reasoning as the timeout tests
/// above (a regression back to "read only after try_wait" would hang
/// this forever, which a bare `.contains` style check wouldn't catch
/// were it not for the wall-clock bound).
#[test]
fn remote_read_large_payload_does_not_deadlock() {
    let _guard = lock();
    let (mut i, _ed) = setup();
    let dir = fixture("large-read");
    // 50,000 * 12 bytes = ~586 KB, well past the 64 KB pipe buffer
    // `run()`'s doc comment calls out.
    let content = big_numbered_content(50_000);
    std::fs::write(dir.join("big.txt"), &content).unwrap();
    let remote = format!("/ssh:fake@host:{}/big.txt", dir.to_str().unwrap());
    let start = std::time::Instant::now();
    let r = run(&mut i, &format!("(find-file-internal {:?})", remote));
    let elapsed = start.elapsed();
    assert!(!r.starts_with("ERROR"), "unexpected error: {}", r);
    assert_eq!(
        run(&mut i, "(buffer-size)"),
        content.len().to_string(),
        "buffer size must match the file's real byte count"
    );
    assert_eq!(run(&mut i, "(buffer-substring 1 13)"), "\"line 000000\\n\"");
    let tail_start = content.len() - 11;
    assert_eq!(
        run(
            &mut i,
            &format!("(buffer-substring {} (point-max))", tail_start)
        ),
        "\"line 049999\\n\""
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "reading a {} byte remote file took {:?}, should be milliseconds \
         on a local shim -- a regression back to \"read stdout only after \
         try_wait succeeds\" would hang this instead",
        content.len(),
        elapsed
    );
}

/// Write half: saves a buffer well past 64 KB to a remote path over the
/// real (local-shim) transport. Must complete promptly, and the bytes
/// that land on "disk" must match exactly (M76's length gate happens to
/// double-check this too, but this test's own assertion doesn't rely on
/// that -- it rereads the file itself).
#[test]
fn remote_write_large_payload_does_not_deadlock() {
    let _guard = lock();
    let (mut i, ed) = setup();
    let dir = fixture("large-write");
    let remote = format!("/ssh:fake@host:{}/new-big.txt", dir.to_str().unwrap());
    run(&mut i, &format!("(find-file-internal {:?})", remote));
    let content = big_numbered_content(50_000);
    let r = run(&mut i, &format!("(insert {:?})", content));
    assert!(!r.starts_with("ERROR"), "insert failed: {}", r);
    let start = std::time::Instant::now();
    feed_keys(&mut i, &ed, "C-x C-s").unwrap();
    let elapsed = start.elapsed();
    let written = std::fs::read(dir.join("new-big.txt")).unwrap();
    assert_eq!(
        written.len(),
        content.len(),
        "byte count landed on the remote must match what was sent"
    );
    assert_eq!(String::from_utf8(written).unwrap(), content);
    assert_eq!(run(&mut i, "(buffer-modified-p)"), "nil");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "writing a {} byte remote file took {:?}, should be well under \
         5s on a local shim -- a regression back to a synchronous stdin \
         write on the calling thread would hang this instead",
        content.len(),
        elapsed
    );
}
