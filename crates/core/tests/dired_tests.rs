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
            "se_dired_{}_{}_{}",
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
/// executable, and a symlink.
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

#[test]
fn find_file_on_directory_enters_dired() {
    let (mut i, ed) = setup();
    let dir = fixture("enter");
    run(
        &mut i,
        &format!("(find-file-internal {:?})", dir.to_str().unwrap()),
    );
    // Buffer named after the directory's last component, read-only,
    // listing with a permissions column.
    let name = run(&mut i, "(buffer-name)");
    assert!(name.contains("se_dired_enter"), "buffer name: {}", name);
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    let text = run(&mut i, "(buffer-string)");
    assert!(text.contains("plain.txt"), "listing missing file: {}", text);
    assert!(text.contains("subdir"), "listing missing dir: {}", text);
    assert!(text.contains("rw"), "no perms column: {}", text);
    // Overlay faces attached.
    assert_ne!(run(&mut i, "(overlays-in (point-min) (point-max))"), "nil");
    let _ = ed;
}

#[test]
fn read_only_blocks_typing_and_elisp() {
    let (mut i, ed) = setup();
    let dir = fixture("ro");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let before = run(&mut i, "(buffer-string)");
    // Typing echoes read-only and changes nothing ('z' is unbound in dired).
    type_str(&mut i, &ed, "z");
    assert_eq!(run(&mut i, "(buffer-string)"), before);
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("read-only"), "echo: {}", echo);
    // Bare elisp insert signals; under inhibit-read-only it succeeds.
    assert!(run(&mut i, "(insert \"x\")").starts_with("ERROR:"));
    assert!(!run(
        &mut i,
        "(let ((inhibit-read-only t)) (insert \"x\") (buffer-string))"
    )
    .starts_with("ERROR:"));
}

#[test]
fn navigate_and_visit() {
    let (mut i, ed) = setup();
    let dir = fixture("nav");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    // Rows: header, ".", "..", then alphabetical: link.txt (unix),
    // plain.txt, run.sh, subdir. Find plain.txt's line via elisp.
    run(&mut i, "(goto-char (point-min))");
    // Move onto the plain.txt row using n (next-line) from the top.
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let target_row = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .expect("plain.txt row");
    for _ in 0..target_row {
        feed_keys(&mut i, &ed, "n").unwrap();
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"hello dired\"");

    // RET on subdir row recurses into a new dired buffer.
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let target_row = rows
        .iter()
        .position(|r| r.ends_with("subdir"))
        .expect("subdir row");
    run(&mut i, "(goto-char (point-min))");
    for _ in 0..target_row {
        feed_keys(&mut i, &ed, "n").unwrap();
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"subdir\"");
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    let text = run(&mut i, "(buffer-string)");
    assert!(text.contains("inner.txt"), "subdir listing: {}", text);

    // ^ goes back up.
    feed_keys(&mut i, &ed, "^").unwrap();
    let name = run(&mut i, "(buffer-name)");
    assert!(name.contains("se_dired_nav"), "after ^: {}", name);
}

#[test]
fn revert_sees_new_files() {
    let (mut i, ed) = setup();
    let dir = fixture("revert");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!run(&mut i, "(buffer-string)").contains("newfile.txt"));
    std::fs::write(dir.join("newfile.txt"), "new").unwrap();
    feed_keys(&mut i, &ed, "g").unwrap();
    assert!(run(&mut i, "(buffer-string)").contains("newfile.txt"));
}

#[test]
fn modeline_shows_dir_slash_name() {
    let (mut i, ed) = setup();
    let dir = fixture("mode");
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    let grid = core::redisplay::render(&i, &ed);
    let rows = grid.lines.len();
    // Modeline is the second-to-last row (echo is last).
    let modeline: String = grid.lines[rows - 2]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    assert!(
        modeline.contains(&format!(
            "{}/plain.txt",
            dir.file_name().unwrap().to_str().unwrap()
        )),
        "modeline: {}",
        modeline
    );
}

#[test]
fn cxcf_prefill_uses_buffer_default_directory() {
    let (mut i, ed) = setup();
    let dir = fixture("dd");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    // From a dired buffer, C-x C-f starts in that directory.
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let input = ed.borrow().minibuffer.as_ref().unwrap().input.clone();
    assert!(
        input.contains(dir.file_name().unwrap().to_str().unwrap()),
        "prefill: {}",
        input
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

#[test]
fn dired_key_binding_c_x_d() {
    let (mut i, ed) = setup();
    let dir = fixture("cxd");
    feed_keys(&mut i, &ed, "C-x d").unwrap();
    assert!(ed.borrow().minibuffer.is_some(), "C-x d should prompt");
    type_str(&mut i, &ed, dir.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    assert!(run(&mut i, "(buffer-string)").contains("plain.txt"));
}

// --- M68: buffer identity, quit target, cursor landing -------------------

#[test]
fn two_directories_sharing_a_basename_get_separate_buffers() {
    let (mut i, _ed) = setup();
    let p1 = Scratch::new("same1");
    let p2 = Scratch::new("same2");
    std::fs::create_dir_all(p1.join("core")).unwrap();
    std::fs::create_dir_all(p2.join("core")).unwrap();
    std::fs::write(p1.join("core/unique1.txt"), "one").unwrap();
    std::fs::write(p2.join("core/unique2.txt"), "two").unwrap();

    run(
        &mut i,
        &format!("(dired {:?})", p1.join("core").to_str().unwrap()),
    );
    let name1 = run(&mut i, "(buffer-name)");
    let dir1 = run(&mut i, "dired--dir");
    let text1 = run(&mut i, "(buffer-string)");
    assert!(text1.contains("unique1.txt"), "text1: {}", text1);

    run(
        &mut i,
        &format!("(dired {:?})", p2.join("core").to_str().unwrap()),
    );
    let name2 = run(&mut i, "(buffer-name)");
    let dir2 = run(&mut i, "dired--dir");
    let text2 = run(&mut i, "(buffer-string)");
    assert!(text2.contains("unique2.txt"), "text2: {}", text2);

    assert_eq!(name1, "\"core\"");
    assert_eq!(name2, "\"core<2>\"");
    assert_ne!(dir1, dir2, "each dired buffer must keep its own dired--dir");

    // Re-visiting p1's core must NOT have been clobbered by p2's dired.
    run(&mut i, &format!("(set-buffer {})", name1));
    let text1_again = run(&mut i, "(buffer-string)");
    assert!(
        text1_again.contains("unique1.txt") && !text1_again.contains("unique2.txt"),
        "p1's dired buffer got contaminated: {}",
        text1_again
    );
}

#[test]
fn dired_does_not_clobber_a_file_buffer_sharing_the_directorys_basename() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("clobber");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("core"), "module core; endmodule\n").unwrap();
    std::fs::create_dir_all(dir.join("sub/core")).unwrap();
    std::fs::write(dir.join("sub/core/inner.txt"), "inner").unwrap();

    // Open the FILE named "core" first, like a user editing it.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("core").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"core\"");
    let before_count = run(&mut i, "(length (buffer-list))");

    // M-x dired into a DIRECTORY also named "core" must get its own buffer,
    // not take over the file buffer above.
    run(
        &mut i,
        &format!("(dired {:?})", dir.join("sub/core").to_str().unwrap()),
    );
    let dired_name = run(&mut i, "(buffer-name)");
    assert_ne!(
        dired_name, "\"core\"",
        "dired must not steal the file buffer's name"
    );
    let after_count = run(&mut i, "(length (buffer-list))");
    assert_ne!(
        before_count, after_count,
        "dired must have created a new buffer"
    );

    let info = run(
        &mut i,
        &format!(
            "(let ((b (get-file-buffer {:?})))
               (list (buffer-file-name b)
                     (with-current-buffer b (buffer-string))))",
            dir.join("core").to_str().unwrap()
        ),
    );
    assert!(
        info.contains("module core; endmodule"),
        "the file buffer's contents must survive untouched: {}",
        info
    );
}

#[test]
fn reusing_a_dired_buffer_keeps_marks_and_prunes_stale_ones() {
    let (mut i, ed) = setup();
    let dir = fixture("reuse-marks");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    // Mark plain.txt.
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "m").unwrap();
    assert_eq!(run(&mut i, "(length dired--marks)"), "1");

    // Delete run.sh out from under dired's feet, then re-enter the SAME
    // directory (reuse path).
    std::fs::remove_file(dir.join("run.sh")).unwrap();
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));

    assert_ne!(
        run(&mut i, "(assoc \"plain.txt\" dired--marks)"),
        "nil",
        "plain.txt's mark must survive re-entering the same directory"
    );
    assert!(!run(&mut i, "(buffer-string)").contains("run.sh"));
}

#[test]
fn q_returns_to_source_through_a_dired_chain() {
    let (mut i, ed) = setup();
    let dir = fixture("chain");
    // Open a file first, like a user editing it.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    // C-x d into the directory.
    feed_keys(&mut i, &ed, "C-x d").unwrap();
    type_str(&mut i, &ed, dir.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();

    // RET into subdir.
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows.iter().position(|r| r.ends_with("subdir")).unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"subdir\"");

    // ^ back up.
    feed_keys(&mut i, &ed, "^").unwrap();

    // q must go straight back to plain.txt, not to the intermediate
    // top-level dired buffer.
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");
}

#[test]
fn q_follows_a_renamed_source_buffer() {
    let (mut i, ed) = setup();
    let dir = fixture("rename-source");
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    // Rename the source buffer while dired is displayed.
    run(
        &mut i,
        "(with-current-buffer \"plain.txt\" (rename-buffer \"renamed-src\"))",
    );
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"renamed-src\"",
        "q must follow the source buffer object even after a rename"
    );
}

#[test]
fn q_falls_back_to_scratch_when_source_was_killed() {
    let (mut i, ed) = setup();
    let dir = fixture("killed-source");
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    run(&mut i, "(kill-buffer \"plain.txt\")");
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");
}

/// M68 tail review guard: the header note's own worked example --
/// `quit-source' must be OVERWRITTEN unconditionally on reuse (unlike
/// `dired--marks', which is preserved via a `local-variable-p' guard).
/// The chain test (`q_returns_to_source_through_a_dired_chain' above)
/// can't tell this apart from a `(unless (local-variable-p
/// 'quit-source) ...)' guard, because in that test the recomputed
/// `quit-source' happens to equal the existing one. Here it must
/// differ: A -> `(dired D1)' -> q -> A; switch to B; `(dired D1)' again
/// (confirmed a REUSE, not a fresh/deduped buffer); q must land on B,
/// the most recent caller -- not the stale A a guard would have kept.
#[test]
fn quit_source_updates_to_the_most_recent_caller_on_reuse() {
    let (mut i, ed) = setup();
    let dir = fixture("quit-source-reuse");

    // A: a file buffer.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("plain.txt").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    // (dired D1) from A -- quit-source(D1) is A.
    run(
        &mut i,
        &format!("(dired {:?})", dir.join("subdir").to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"subdir\"");

    // q -> A.
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"plain.txt\"");

    // Switch to B: a DIFFERENT file buffer.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("run.sh").to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"run.sh\"");
    // Snapshot the buffer count right BEFORE the second (dired D1) --
    // NOT back at the first one, which was taken before B's own file
    // buffer existed, so comparing against it would count B itself as a
    // (bogus) extra dired buffer.
    let count_before_second_open = run(&mut i, "(length (buffer-list))");

    // (dired D1) again -- must REUSE the same buffer (no "subdir<2>"),
    // not alias/dedup onto a new one.
    run(
        &mut i,
        &format!("(dired {:?})", dir.join("subdir").to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"subdir\"",
        "second entry must reuse the SAME buffer, not create a differently-named one"
    );
    let count_after_second_open = run(&mut i, "(length (buffer-list))");
    assert_eq!(
        count_before_second_open, count_after_second_open,
        "second entry into D1 must not have created a new buffer"
    );

    // q must now land on B -- the most recent caller -- not the stale A.
    feed_keys(&mut i, &ed, "q").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"run.sh\"",
        "quit-source must have been updated to B on the second (dired D1), not kept as the stale A"
    );
}

#[test]
fn dired_lands_on_first_real_entry_not_dot() {
    let (mut i, _ed) = setup();
    let dir = fixture("landing");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let entry = run(&mut i, "(car (dired--entry-at-point))");
    // Exact match against the fixture's own alphabetically-first real
    // entry (`link.txt' on unix, where the fixture also creates a
    // symlink -- see `fixture''s own `#[cfg(unix)]' block), not just
    // "not . and not .." -- that weaker check still passes with `nil'
    // (i.e. point stuck on the header row, `dired--entry-at-point'
    // returning nil) if `dired--goto-first-real-entry' were never
    // called at all, since `(car nil)' prints as "nil", which is
    // neither "\".\"" nor "\"..\"".
    #[cfg(unix)]
    let expected = "\"link.txt\"";
    #[cfg(not(unix))]
    let expected = "\"plain.txt\"";
    assert_eq!(entry, expected, "landed on the wrong first real entry");
}

#[test]
fn a_freshly_filled_dired_buffer_is_not_marked_modified() {
    // `dired--fill' builds its listing with `erase-buffer' + `insert',
    // which sets the modified flag -- so without the explicit
    // `set-buffer-modified-p nil' at the end of `dired--fill', every
    // dired buffer permanently shows the mode line's `*' despite the
    // user never having typed anything. Found by a mutation probe (M9
    // in dev/mutations/m68.py) that survived the whole suite: the line
    // existed with nothing watching it.
    let (mut i, _ed) = setup();
    let dir = fixture("unmodified");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "a dired buffer that was only ever generated, never edited, must not be modified"
    );
    // Reuse goes through `dired--fill' again -- the flag must stay clear
    // on that path too, not just on first open.
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "re-entering an already-open dired buffer must not leave it modified either"
    );
}

// M71: `dired--fill' (covered above) only clears the flag on the
// open/revert path. Every interactive marking command goes through
// `dired--set-mark-char' instead, which does its own `insert' and used
// to leave the buffer marked modified again. D2-D4 below cover that
// path; each asserts BOTH the flag and the actual mark-column text, so
// a `dired-mark' that silently did nothing couldn't pass by accident.

#[test]
fn marking_a_file_does_not_mark_the_buffer_modified() {
    let (mut i, ed) = setup();
    let dir = fixture("mark-unmodified");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "m").unwrap();
    let row = run(&mut i, "(buffer-string)");
    let row = row.trim_matches('"').split("\\n").nth(idx).unwrap();
    assert!(row.starts_with('*'), "row after mark: {:?}", row);
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "marking a file is a dired-internal bookkeeping change, not user text -- must not set `*' on the mode line"
    );
}

#[test]
fn all_four_marking_commands_leave_the_buffer_unmodified() {
    let (mut i, ed) = setup();
    let dir = fixture("mark-commands-unmodified");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .unwrap();
    let goto = |i: &mut Interp| {
        run(
            i,
            &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
        );
    };
    let row_at = |i: &mut Interp| -> String {
        run(i, "(buffer-string)")
            .trim_matches('"')
            .split("\\n")
            .nth(idx)
            .unwrap()
            .to_string()
    };

    // dired-mark: 'm', row starts with '*'.
    goto(&mut i);
    feed_keys(&mut i, &ed, "m").unwrap();
    assert!(
        row_at(&mut i).starts_with('*'),
        "row after dired-mark: {:?}",
        row_at(&mut i)
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "after dired-mark"
    );

    // dired-unmark: 'u', row starts with a space.
    goto(&mut i);
    feed_keys(&mut i, &ed, "u").unwrap();
    assert!(
        row_at(&mut i).starts_with(' '),
        "row after dired-unmark: {:?}",
        row_at(&mut i)
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "after dired-unmark"
    );

    // dired-unmark-all-marks: mark first, then 'U' clears it.
    goto(&mut i);
    feed_keys(&mut i, &ed, "m").unwrap();
    feed_keys(&mut i, &ed, "U").unwrap();
    assert!(
        row_at(&mut i).starts_with(' '),
        "row after dired-unmark-all-marks: {:?}",
        row_at(&mut i)
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "after dired-unmark-all-marks"
    );

    // dired-flag-file-deletion: 'd', row starts with 'D'.
    goto(&mut i);
    feed_keys(&mut i, &ed, "d").unwrap();
    assert!(
        row_at(&mut i).starts_with('D'),
        "row after dired-flag-file-deletion: {:?}",
        row_at(&mut i)
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "after dired-flag-file-deletion"
    );
}

#[test]
fn marking_then_reverting_stays_unmodified_and_keeps_the_mark() {
    let (mut i, ed) = setup();
    let dir = fixture("mark-then-revert-unmodified");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "m").unwrap();
    assert_eq!(run(&mut i, "(buffer-modified-p)"), "nil", "before revert");

    feed_keys(&mut i, &ed, "g").unwrap();

    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "revert must not leave a marked dired buffer modified"
    );
    let row = run(&mut i, "(buffer-string)");
    let row = row.trim_matches('"').split("\\n").nth(idx).unwrap();
    assert!(
        row.starts_with('*'),
        "mark should survive revert: {:?}",
        row
    );
}

#[test]
fn dired_on_empty_directory_does_not_crash() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("empty");
    std::fs::create_dir_all(&*dir).unwrap();
    let out = run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    assert!(!out.starts_with("ERROR:"), "out: {}", out);
    // Only "."/".." exist -- landing must not error and must stay in range.
    assert!(run(&mut i, "(line-number-at-pos)").parse::<i64>().is_ok());
}

#[test]
fn up_directory_lands_on_the_child_just_left() {
    let (mut i, ed) = setup();
    let dir = fixture("updir");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows.iter().position(|r| r.ends_with("subdir")).unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"subdir\"");

    feed_keys(&mut i, &ed, "^").unwrap();
    let entry = run(&mut i, "(car (dired--entry-at-point))");
    assert_eq!(entry, "\"subdir\"");
}

#[test]
fn up_directory_at_root_does_not_crash() {
    let (mut i, ed) = setup();
    run(&mut i, "(dired \"/\")");
    let out = feed_keys(&mut i, &ed, "^");
    assert!(out.is_ok(), "^ at root errored: {:?}", out);
}

#[test]
fn revert_g_keeps_point_on_the_same_line() {
    let (mut i, ed) = setup();
    let dir = fixture("g-line");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows
        .iter()
        .position(|r| r.contains("plain.txt") && !r.contains("->"))
        .unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    let before = run(&mut i, "(line-number-at-pos)");
    feed_keys(&mut i, &ed, "g").unwrap();
    assert_eq!(run(&mut i, "(line-number-at-pos)"), before);
}

/// M68 review guard: `dired-revert's line-number clamp. Point starts on
/// the fixture's alphabetically LAST real row ("subdir", after link.txt/
/// plain.txt/run.sh); that row's own directory is removed out from under
/// dired before `g', so the saved line number no longer exists in the
/// shrunk listing. Without the clamp, `forward-line' still can't error
/// (it self-clamps at `point-max' -- see `editing.rs'), but it lands
/// PAST the last real row, on the buffer's own trailing empty line,
/// where `dired--entry-at-point' reads out of `dired--files' bounds and
/// returns nil.
#[test]
fn revert_clamps_point_when_the_last_row_disappears() {
    let (mut i, ed) = setup();
    let dir = fixture("revert-clamp");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    let listing = run(&mut i, "(buffer-string)");
    let rows: Vec<&str> = listing.trim_matches('"').split("\\n").collect();
    let idx = rows.iter().position(|r| r.ends_with("subdir")).unwrap();
    run(
        &mut i,
        &format!("(progn (goto-char (point-min)) (forward-line {}))", idx),
    );
    assert_eq!(run(&mut i, "(car (dired--entry-at-point))"), "\"subdir\"");

    std::fs::remove_dir_all(dir.join("subdir")).unwrap();
    feed_keys(&mut i, &ed, "g").unwrap();

    let line: i64 = run(&mut i, "(line-number-at-pos)").parse().unwrap();
    let max_line: i64 = run(&mut i, "(+ dired--header-lines (length dired--files))")
        .parse()
        .unwrap();
    assert!(
        line <= max_line,
        "point landed past the last real row: line {} > max {}",
        line,
        max_line
    );
    assert_ne!(
        run(&mut i, "(dired--entry-at-point)"),
        "nil",
        "point must still be sitting on a real row after the clamp"
    );
}

/// M68 review guard: `expand-file-name' runs TWICE on a `/ssh:' path --
/// once in `find-file-internal' (editing.rs's own `expand_path'), then
/// again inside `dired''s own `(expand-file-name dir)' when it dispatches
/// a remote directory there. If that double expansion weren't idempotent,
/// `dired--dir' would come out differently depending on entry point, and
/// `dired--buffer-for' would silently fail to reuse the buffer.
#[test]
fn ssh_dired_and_find_file_internal_agree_on_dired_dir() {
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut i, _ed) = setup();
    let dir = Scratch::new("ssh-idem");
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("plain.txt"), "hi").unwrap();
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

    let remote = format!("/ssh:fake@host:{}", dir.to_str().unwrap());
    run(&mut i, &format!("(dired {:?})", remote));
    let dir1 = run(&mut i, "dired--dir");
    let count1 = run(&mut i, "(length (buffer-list))");

    run(&mut i, &format!("(find-file-internal {:?})", remote));
    let dir2 = run(&mut i, "dired--dir");
    let count2 = run(&mut i, "(length (buffer-list))");

    std::env::remove_var("RETICLE_SSH_BIN");

    // Positive assertion first: `dir1' must actually be a populated
    // `/ssh:' path, not the global default `nil' both entry points would
    // share if the shim or `remote::is_dir' silently failed to route
    // either call into the remote dired branch at all -- `assert_eq!`
    // alone would pass on `"nil" == "nil"` in that case and hide that
    // nothing was really exercised.
    assert!(
        dir1.starts_with("\"/ssh:"),
        "dired--dir must be a real /ssh: path, got: {}",
        dir1
    );
    assert_eq!(
        dir1, dir2,
        "dired--dir must be identical whether entered via `dired' or `find-file-internal'"
    );
    assert_eq!(
        count1, count2,
        "the second entry point must reuse the SAME dired buffer, not create a new one"
    );
}

// --- M130: vim-style j/k/G motion in dired's local keymap ---------------

#[test]
fn dired_j_and_k_move_by_line() {
    let (mut i, ed) = setup();
    let dir = fixture("jk");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "evil--state"), "emacs");
    let before_text = run(&mut i, "(buffer-string)");
    run(&mut i, "(goto-char (point-min))");
    let line0 = run(&mut i, "(line-number-at-pos)");
    feed_keys(&mut i, &ed, "j").unwrap();
    let line1 = run(&mut i, "(line-number-at-pos)");
    assert_ne!(line1, line0, "j must move down a line");
    feed_keys(&mut i, &ed, "k").unwrap();
    let line2 = run(&mut i, "(line-number-at-pos)");
    assert_eq!(line2, line0, "k must move back up to the original line");
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before_text,
        "j/k must never alter dired's listing text"
    );
}

#[test]
fn dired_capital_g_goes_to_last_line() {
    let (mut i, ed) = setup();
    let dir = fixture("capg");
    run(&mut i, &format!("(dired {:?})", dir.to_str().unwrap()));
    run(&mut i, "(evil-mode 1)");
    assert_eq!(run(&mut i, "evil--state"), "emacs");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "G").unwrap();
    // M130 fix round FIX-2: every row `dired-insert-listing' emits, the
    // LAST one included, ends in "\n" -- so a plain `end-of-buffer'
    // lands on the empty line PAST the last real entry, where
    // `dired--entry-at-point' computes a row index one past the end of
    // `dired--files' and `nth' returns nil. `G' must instead land ON
    // the last real entry, the same as GNU's own `dired-mode' `G'
    // convention.
    let entry = run(&mut i, "(dired--entry-at-point)");
    assert_ne!(
        entry, "nil",
        "G must land on a real entry, not the empty line past the last row"
    );
    let last_name = run(&mut i, "(car (car (last dired--files)))");
    let entry_name = run(&mut i, "(car (dired--entry-at-point))");
    assert_eq!(
        entry_name, last_name,
        "G must land specifically on the LAST file in the listing"
    );
}
