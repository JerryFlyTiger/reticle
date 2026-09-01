//! M41: dired file operations — marking (m/u/U/d) and the operation
//! commands built on top of it (x/D/C/R), plus the underlying
//! delete-file/delete-directory/copy-file/rename-file primitives.
//! Local-only; the /ssh: shim-backed equivalents live in ssh_tests.rs.

use std::cell::RefCell;
use std::rc::Rc;

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
            "se_dired_ops_{}_{}_{}",
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

/// Temp dir with a file, a subdirectory (containing a file), an
/// executable, and a symlink — same shape as dired_tests.rs's fixture.
fn fixture(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("plain.txt"), "hello dired").unwrap();
    std::fs::write(dir.join("subdir/inner.txt"), "inner").unwrap();
    std::fs::write(dir.join("run.sh"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        std::os::unix::fs::symlink(dir.join("plain.txt"), dir.join("link.txt")).unwrap();
    }
    dir
}

/// `(buffer-string)` split back into plain lines (undoing prin1's
/// escaping) — dired.el never parses this back itself, but the tests
/// need to read the mark column and row order somehow.
fn rows(i: &mut Interp) -> Vec<String> {
    let listing = run(i, "(buffer-string)");
    listing
        .trim_matches('"')
        .split("\\n")
        .map(|s| s.to_string())
        .collect()
}

/// Absolute row index (including the header) of the row naming NAME —
/// excluding a symlink's " -> target" row, whose target name could
/// otherwise collide with NAME.
fn row_index(rows: &[String], name: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(name) && !r.contains("->"))
        .unwrap_or_else(|| panic!("no row containing {}: {:?}", name, rows))
}

fn goto_row(i: &mut Interp, name: &str) {
    let idx = row_index(&rows(i), name);
    run(
        i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
}

fn dired_open(i: &mut Interp, dir: &std::path::Path) {
    run(i, &format!("(dired {:?})", dir.to_str().unwrap()));
}

fn line_no(i: &mut Interp) -> i64 {
    run(i, "(line-number-at-pos)").parse().unwrap()
}

// --- 1/2: marking ------------------------------------------------------

#[test]
fn mark_unmark_and_unmark_all() {
    let (mut i, ed) = setup();
    let dir = fixture("mark");
    dired_open(&mut i, &dir);

    goto_row(&mut i, "plain.txt");
    let idx = row_index(&rows(&mut i), "plain.txt");
    let ln_before = line_no(&mut i);
    feed_keys(&mut i, &ed, "m").unwrap();
    let row = rows(&mut i)[idx].clone();
    assert!(row.starts_with('*'), "row after mark: {:?}", row);
    assert_eq!(line_no(&mut i), ln_before + 1, "point should move down");
    assert_eq!(
        run(&mut i, "(cdr (assoc \"plain.txt\" dired--marks))"),
        "42" // ?* == 42
    );

    // Unmark: go back to the row and press u.
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "u").unwrap();
    let row = rows(&mut i)[idx].clone();
    assert!(row.starts_with(' '), "row after unmark: {:?}", row);
    assert_eq!(run(&mut i, "(assoc \"plain.txt\" dired--marks)"), "nil");

    // Mark two files, then U clears both.
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "m").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "m").unwrap();
    assert_eq!(run(&mut i, "(length dired--marks)"), "2");
    feed_keys(&mut i, &ed, "U").unwrap();
    assert_eq!(run(&mut i, "dired--marks"), "nil");
    for name in ["plain.txt", "run.sh"] {
        let idx = row_index(&rows(&mut i), name);
        assert!(
            rows(&mut i)[idx].starts_with(' '),
            "{} still marked after U",
            name
        );
    }
}

#[test]
fn dot_and_dotdot_cannot_be_marked_or_flagged() {
    let (mut i, ed) = setup();
    let dir = fixture("dots");
    dired_open(&mut i, &dir);
    // M68: dired's default landing is now the first REAL entry, not
    // "." -- navigate onto "." explicitly (ls -al order puts . then ..
    // first) before exercising the "." guard below.
    run(&mut i, "(progn (goto-char (point-min)) (forward-line 1))");
    let ln_before = line_no(&mut i);
    feed_keys(&mut i, &ed, "m").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Cannot mark ."), "echo: {}", echo);
    assert_eq!(run(&mut i, "dired--marks"), "nil");
    assert_eq!(line_no(&mut i), ln_before, "point must not move");

    run(&mut i, "(forward-line 1)"); // onto ".."
    feed_keys(&mut i, &ed, "d").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Cannot flag .."), "echo: {}", echo);
    assert_eq!(run(&mut i, "dired--marks"), "nil");
}

// --- 3: flag + x, confirm y/n -------------------------------------------

#[test]
fn flagged_delete_confirm_yes_and_no() {
    let (mut i, ed) = setup();
    let dir = fixture("flagx");
    dired_open(&mut i, &dir);

    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "d").unwrap();
    let idx = row_index(&rows(&mut i), "plain.txt");
    assert!(rows(&mut i)[idx].starts_with('D'));

    // First attempt: decline.
    feed_keys(&mut i, &ed, "x").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Delete 1 file(s)?"), "echo: {}", echo);
    feed_keys(&mut i, &ed, "n").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "(No deletions performed)");
    assert!(dir.join("plain.txt").exists(), "file wrongly deleted");
    assert!(rows(&mut i)
        .iter()
        .any(|r| r.contains("plain.txt") && !r.contains("->")));

    // Second attempt: confirm.
    feed_keys(&mut i, &ed, "x").unwrap();
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Deleted 1 file(s)");
    assert!(!dir.join("plain.txt").exists(), "file not deleted");
    assert!(!rows(&mut i)
        .iter()
        .any(|r| r.contains("plain.txt") && !r.contains("->")));
}

#[test]
fn flagged_delete_with_nothing_flagged_is_a_no_op() {
    let (mut i, ed) = setup();
    let dir = fixture("flagx-empty");
    dired_open(&mut i, &dir);
    feed_keys(&mut i, &ed, "x").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "(No deletions requested)");
}

// --- 4: D on point vs marks ---------------------------------------------

#[test]
fn do_delete_uses_point_when_unmarked() {
    let (mut i, ed) = setup();
    let dir = fixture("del-point");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "D").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Delete 1 file(s)?"), "echo: {}", echo);
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Deleted 1 file(s)");
    assert!(!dir.join("run.sh").exists());
}

#[test]
fn do_delete_uses_marks_when_present() {
    let (mut i, ed) = setup();
    let dir = fixture("del-marks");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "m").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "m").unwrap();
    // Point is now away from both marked rows; D must still use the marks.
    feed_keys(&mut i, &ed, "D").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("Delete 2 file(s)?"), "echo: {}", echo);
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Deleted 2 file(s)");
    assert!(!dir.join("plain.txt").exists());
    assert!(!dir.join("run.sh").exists());
}

// --- 5: recursive directory delete --------------------------------------

#[test]
fn do_delete_recurses_into_marked_directory() {
    let (mut i, ed) = setup();
    let dir = fixture("del-dir");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "subdir");
    feed_keys(&mut i, &ed, "D").unwrap();
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Deleted 1 file(s)");
    assert!(!dir.join("subdir").exists());
}

// --- 6: copy -------------------------------------------------------------

#[test]
fn copy_single_file_to_literal_destination_name() {
    let (mut i, ed) = setup();
    let dir = fixture("copy-lit");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    let dest = dir.join("copy_of_plain.txt");
    feed_keys(&mut i, &ed, "C").unwrap();
    assert!(ed.borrow().minibuffer.is_some(), "C should prompt");
    type_str(&mut i, &ed, dest.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.starts_with("Copied "), "echo: {}", echo);
    assert!(echo.contains("->"), "echo: {}", echo);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello dired");
    assert!(
        dir.join("plain.txt").exists(),
        "source should survive a copy"
    );
}

#[test]
fn copy_single_file_into_existing_directory() {
    let (mut i, ed) = setup();
    let dir = fixture("copy-dir");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "C").unwrap();
    type_str(&mut i, &ed, dir.join("subdir").to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Copied 1 file(s)");
    assert_eq!(
        std::fs::read_to_string(dir.join("subdir/plain.txt")).unwrap(),
        "hello dired"
    );
}

#[test]
fn copy_multiple_marked_to_non_directory_errors() {
    let (mut i, ed) = setup();
    let dir = fixture("copy-multi-err");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "m").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "m").unwrap();
    feed_keys(&mut i, &ed, "C").unwrap();
    let dest = dir.join("not-a-dir.txt");
    type_str(&mut i, &ed, dest.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "Destination must be a directory for multiple files");
    assert!(!dest.exists());
    assert!(dir.join("plain.txt").exists());
    assert!(dir.join("run.sh").exists());
}

// --- 7: rename syncs a visiting buffer -----------------------------------

#[test]
fn rename_updates_visiting_buffer_file_and_name() {
    let (mut i, ed) = setup();
    let dir = fixture("rename-buf");
    // Open the file directly first, like a user editing it.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    let new_path = dir.join("renamed.txt");
    feed_keys(&mut i, &ed, "R").unwrap();
    type_str(&mut i, &ed, new_path.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.starts_with("Renamed "), "echo: {}", echo);

    assert!(!dir.join("plain.txt").exists());
    assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "hello dired");

    let new_path_str = new_path.to_str().unwrap();
    let info = run(
        &mut i,
        &format!(
            "(let ((b (get-file-buffer {:?}))) (list (buffer-file-name b) (buffer-name b)))",
            new_path_str
        ),
    );
    assert!(info.contains(new_path_str), "info: {}", info);
    assert!(info.contains("\"renamed.txt\""), "info: {}", info);
}

// --- 8: revert keeps live marks, drops stale ones ------------------------

#[test]
fn revert_keeps_marks_for_survivors_drops_marks_for_deleted() {
    let (mut i, ed) = setup();
    let dir = fixture("revert-marks");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "m").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "m").unwrap();
    assert_eq!(run(&mut i, "(length dired--marks)"), "2");

    // Delete run.sh from under dired's feet (not via dired itself).
    std::fs::remove_file(dir.join("run.sh")).unwrap();
    feed_keys(&mut i, &ed, "g").unwrap();

    assert_eq!(run(&mut i, "(assoc \"run.sh\" dired--marks)"), "nil");
    assert_ne!(run(&mut i, "(assoc \"plain.txt\" dired--marks)"), "nil");
    let idx = row_index(&rows(&mut i), "plain.txt");
    assert!(
        rows(&mut i)[idx].starts_with('*'),
        "plain.txt's mark should survive revert"
    );
    assert!(!rows(&mut i).iter().any(|r| r.contains("run.sh")));
}

// --- 9: primitive error paths --------------------------------------------

#[test]
fn delete_file_missing_target_errors_cleanly() {
    let (mut i, _ed) = setup();
    let dir = fixture("prim-del-err");
    let missing = dir.join("does-not-exist.txt");
    let out = run(
        &mut i,
        &format!("(delete-file {:?})", missing.to_str().unwrap()),
    );
    assert!(out.starts_with("ERROR:"), "out: {}", out);
}

#[test]
fn copy_file_on_a_directory_errors_cleanly() {
    let (mut i, _ed) = setup();
    let dir = fixture("prim-copy-err");
    let out = run(
        &mut i,
        &format!(
            "(copy-file {:?} {:?})",
            dir.join("subdir").to_str().unwrap(),
            dir.join("copy-dest").to_str().unwrap()
        ),
    );
    assert!(out.starts_with("ERROR:"), "out: {}", out);
    assert!(!dir.join("copy-dest").exists());
}

// --- 10: copy-file src == dst (review #1) --------------------------------

#[test]
fn copy_file_same_path_errors_and_preserves_content() {
    let (mut i, _ed) = setup();
    let dir = fixture("prim-copy-same");
    let p = dir.join("plain.txt");
    let out = run(
        &mut i,
        &format!(
            "(copy-file {:?} {:?})",
            p.to_str().unwrap(),
            p.to_str().unwrap()
        ),
    );
    assert!(out.starts_with("ERROR:"), "out: {}", out);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "hello dired",
        "src==dst copy-file must not truncate the file"
    );
}

#[test]
fn dired_copy_into_own_directory_errors_and_preserves_content() {
    let (mut i, ed) = setup();
    let dir = fixture("copy-own-dir");
    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "C").unwrap();
    // Destination is the directory the marked file already lives in --
    // dired-do-copy resolves that to DEST/basename, which is exactly SRC.
    type_str(&mut i, &ed, dir.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("skipped") && echo.contains("plain.txt"),
        "echo: {}",
        echo
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "hello dired",
        "copying a file onto itself must not empty it"
    );
}

// --- 11: rename dedup must exclude the buffer being renamed (review #2) --

#[test]
fn rename_into_existing_dir_same_basename_does_not_dedup_self() {
    let (mut i, ed) = setup();
    let dir = fixture("rename-self-collide");
    // Open the file directly first, like a user editing it.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    dired_open(&mut i, &dir);
    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "R").unwrap();
    // Same basename, different directory -- the most common rename/move,
    // and the case the buggy dedup mistook for a name collision with
    // itself.
    type_str(&mut i, &ed, dir.join("subdir").to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.starts_with("Renamed "), "echo: {}", echo);

    let new_path = dir.join("subdir/plain.txt");
    assert!(new_path.exists());
    let new_path_str = new_path.to_str().unwrap();
    let info = run(
        &mut i,
        &format!(
            "(let ((b (get-file-buffer {:?}))) (list (buffer-file-name b) (buffer-name b)))",
            new_path_str
        ),
    );
    assert!(info.contains(new_path_str), "info: {}", info);
    assert!(info.contains("\"plain.txt\""), "info: {}", info);
    assert!(
        !info.contains("plain.txt<2>"),
        "buffer wrongly deduped against itself: {}",
        info
    );
}

// --- 12: batch delete skip-and-continue (review #4) -----------------------

#[test]
fn flagged_delete_skips_missing_file_but_deletes_the_rest_and_reverts() {
    let (mut i, ed) = setup();
    let dir = fixture("flagx-skip");
    dired_open(&mut i, &dir);

    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "d").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "d").unwrap();

    // Remove plain.txt out from under dired's feet before committing --
    // the flag is still recorded against it in dired--marks.
    std::fs::remove_file(dir.join("plain.txt")).unwrap();

    feed_keys(&mut i, &ed, "x").unwrap();
    feed_keys(&mut i, &ed, "y").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.starts_with("Deleted 1 file(s), skipped"),
        "echo: {}",
        echo
    );
    assert!(echo.contains("plain.txt"), "echo: {}", echo);
    assert!(
        !dir.join("run.sh").exists(),
        "run.sh should still be deleted despite plain.txt's failure"
    );
    // The listing was reverted: neither stale row is still shown.
    assert!(!rows(&mut i).iter().any(|r| r.contains("run.sh")));
}

#[test]
fn do_rename_skips_missing_file_but_renames_the_rest_and_reverts() {
    let (mut i, ed) = setup();
    let dir = fixture("ren-skip");
    dired_open(&mut i, &dir);

    goto_row(&mut i, "plain.txt");
    feed_keys(&mut i, &ed, "m").unwrap();
    goto_row(&mut i, "run.sh");
    feed_keys(&mut i, &ed, "m").unwrap();

    // Remove plain.txt out from under dired's feet before committing.
    std::fs::remove_file(dir.join("plain.txt")).unwrap();

    feed_keys(&mut i, &ed, "R").unwrap();
    type_str(&mut i, &ed, dir.join("subdir").to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.starts_with("Renamed 1 file(s), skipped"),
        "echo: {}",
        echo
    );
    assert!(echo.contains("plain.txt"), "echo: {}", echo);
    assert!(
        dir.join("subdir/run.sh").exists(),
        "run.sh should still be renamed despite plain.txt's failure"
    );
    assert!(!rows(&mut i).iter().any(|r| r.contains("run.sh")));
}
