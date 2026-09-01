use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_org_{}_{}_{}",
            std::process::id(),
            rand_suffix(),
            unique_seq()
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

fn setup_org(content: &str) -> (Interp, Rc<RefCell<Editor>>, Scratch) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    let dir = Scratch::new();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.org");
    std::fs::write(&path, content).unwrap();
    let src = format!("(find-file-internal {:?})", path.to_str().unwrap());
    interp
        .eval_source(&src)
        .map_err(|_| ())
        .expect("find-file failed");
    (interp, ed, dir)
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
}

/// Process-wide monotonic counter: parallel tests in the same process
/// share a pid and can land on the same nanosecond, so the timestamp
/// alone occasionally collides — this guarantees uniqueness.
fn unique_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn grid_row(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

const SAMPLE: &str = "\
* Heading one
body text a
body text b
** Child heading
child body
* Heading two
tail text
";

#[test]
fn org_mode_auto_activates() {
    let (mut i, _ed, _dir) = setup_org(SAMPLE);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "org-mode");
}

#[test]
fn folding_with_tab() {
    let (mut i, ed, _dir) = setup_org(SAMPLE);
    ed.borrow_mut().frame = (40, 10);
    run(&mut i, "(goto-char (point-min))");
    assert_eq!(grid_row(&i, &ed, 0), "* Heading one");
    assert_eq!(grid_row(&i, &ed, 1), "body text a");
    // TAB folds the whole subtree (including the child).
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(grid_row(&i, &ed, 0), "* Heading one...");
    assert_eq!(grid_row(&i, &ed, 1), "* Heading two");
    // TAB again unfolds.
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(grid_row(&i, &ed, 1), "body text a");
    // Folding the child hides only the child body.
    run(
        &mut i,
        "(progn (goto-char (point-min)) (search-forward \"** Child\"))",
    );
    feed_keys(&mut i, &ed, "TAB").unwrap();
    assert_eq!(grid_row(&i, &ed, 3), "** Child heading...");
    assert_eq!(grid_row(&i, &ed, 4), "* Heading two");
}

#[test]
fn todo_cycling() {
    let (mut i, ed, _dir) = setup_org(SAMPLE);
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-c C-t").unwrap();
    assert!(run(&mut i, "(buffer-string)").starts_with("\"* TODO Heading one"));
    feed_keys(&mut i, &ed, "C-c C-t").unwrap();
    assert!(run(&mut i, "(buffer-string)").starts_with("\"* DONE Heading one"));
    feed_keys(&mut i, &ed, "C-c C-t").unwrap();
    assert!(run(&mut i, "(buffer-string)").starts_with("\"* Heading one"));
}

#[test]
fn heading_insert_and_navigation() {
    let (mut i, ed, _dir) = setup_org(SAMPLE);
    // M-RET after the child heading inserts a same-level heading.
    run(
        &mut i,
        "(progn (goto-char (point-min)) (search-forward \"** Child\"))",
    );
    feed_keys(&mut i, &ed, "M-RET").unwrap();
    run(&mut i, "(insert \"New child\")");
    assert!(run(&mut i, "(buffer-string)").contains("** Child heading\\n** New child\\nchild body"));
    // C-c C-n / C-c C-p move between headings.
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-c C-n").unwrap();
    assert_eq!(run(&mut i, "(org--line-string)"), "\"** Child heading\"");
    feed_keys(&mut i, &ed, "C-c C-p").unwrap();
    assert_eq!(run(&mut i, "(org--line-string)"), "\"* Heading one\"");
}

#[test]
fn table_alignment_with_cjk() {
    let (mut i, ed, _dir) = setup_org("| Name | Qty |\n|-\n| 中文名稱 | 2 |\n| ab | 100 |\n");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-c C-c").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(text.contains("| Name     | Qty |"), "got: {}", text);
    assert!(text.contains("|----------+-----|"), "got: {}", text);
    assert!(text.contains("| 中文名稱 | 2   |"), "got: {}", text);
    assert!(text.contains("| ab       | 100 |"), "got: {}", text);
}

#[test]
fn timestamp_insert() {
    let (mut i, ed, _dir) = setup_org("note: ");
    run(&mut i, "(end-of-buffer)");
    feed_keys(&mut i, &ed, "C-c .").unwrap();
    let text = run(&mut i, "(buffer-string)");
    // <YYYY-MM-DD ...>
    assert!(text.contains("<20"), "got: {}", text);
    assert!(text.contains(">"), "got: {}", text);
}

#[test]
fn link_extraction() {
    let (mut i, _ed, _dir) = setup_org(SAMPLE);
    assert_eq!(
        run(
            &mut i,
            "(org--link-at \"see [[https://example.com][site]] here\" 8)"
        ),
        "\"https://example.com\""
    );
    assert_eq!(
        run(&mut i, "(org--link-at \"see [[notes.org]] here\" 8)"),
        "\"notes.org\""
    );
    assert_eq!(run(&mut i, "(org--link-at \"no link here\" 3)"), "nil");
}

#[test]
fn highlighting_faces() {
    let (mut i, ed, _dir) = setup_org("* TODO Task one\n| a | b |\n<2026-07-20 Sun>\n");
    let _ = &ed;
    // Heading line carries a level face and a TODO face overlay.
    assert_eq!(
        run(
            &mut i,
            "(let ((faces nil))
               (dolist (ov (overlays-in 1 16) faces)
                 (when (overlay-get ov 'org-face)
                   (push (overlay-get ov 'face) faces))))"
        )
        .contains("org-level-1")
        .to_string(),
        "true"
    );
    let all = run(
        &mut i,
        "(let ((faces nil))
           (dolist (ov (overlays-in (point-min) (point-max)) faces)
             (when (overlay-get ov 'org-face)
               (push (overlay-get ov 'face) faces))))",
    );
    assert!(all.contains("org-todo"), "faces: {}", all);
    assert!(all.contains("org-table"), "faces: {}", all);
    assert!(all.contains("org-date"), "faces: {}", all);
}

#[test]
fn org_mode_hook_runs() {
    let mut i = elisp::new_interp();
    let _ed = core::init_editor(&mut i);
    run(&mut i, "(setq hook-ran nil)");
    run(
        &mut i,
        "(add-hook 'org-mode-hook (lambda () (setq hook-ran t)))",
    );
    run(&mut i, "(org-mode)");
    assert_eq!(run(&mut i, "hook-ran"), "t");
}

/// M24 probe 2 (run deliberately: `cargo test --release -p core --test
/// org_tests -- --ignored --nocapture`): total cost of `(org-mode)` —
/// which fontifies the whole buffer synchronously, see `org--fontify-buffer`
/// in lisp/org.el — over a 20k-line org file mixing headings, TODO/tag
/// lines, a list with a link, a table, and plain prose. Recorded in the
/// M24 report alongside the line-number-cache probes.
#[test]
#[ignore]
fn measure_org_fontify_cost_20k_lines() {
    let mut src = String::new();
    for n in 0..20_000usize {
        match n % 20 {
            0 => src.push_str(&format!("* Heading {n}\n")),
            5 => src.push_str(&format!("** TODO Sub {n} :tag:\n")),
            10 => src.push_str(&format!(
                "- item {n} with [[https://example.com/{n}][a link]]\n"
            )),
            15 => src.push_str(&format!("| col{n} | col{} | col{} |\n", n + 1, n + 2)),
            _ => src.push_str(&format!(
                "body text line {n} — plain prose, nothing special, some 中文 mixed in.\n"
            )),
        }
    }
    let char_len = src.chars().count();

    // Insert directly through the buffer rather than via a temp file +
    // find-file-internal: this isolates the fontify cost from file I/O,
    // which is what the probe is measuring.
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    {
        let buf = ed.borrow().current.clone();
        buf.borrow_mut().insert(0, &src);
    }

    let start = std::time::Instant::now();
    run(&mut interp, "(org-mode)");
    let elapsed = start.elapsed();
    eprintln!("org-mode fontify over 20000 lines ({char_len} chars): {elapsed:?}");
}
