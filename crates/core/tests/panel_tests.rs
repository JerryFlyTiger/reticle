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

fn panel_accepts(ed: &Rc<RefCell<Editor>>) -> Vec<String> {
    ed.borrow()
        .minibuffer
        .as_ref()
        .and_then(|m| m.panel.as_ref())
        .map(|p| p.rows.iter().map(|r| r.accept.clone()).collect())
        .unwrap_or_default()
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
            "se_panel_{}_{}_{}",
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

fn fixture(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("alpha.txt"), "A").unwrap();
    std::fs::write(dir.join("beta.txt"), "B").unwrap();
    dir
}

#[test]
fn panel_opens_immediately_and_windows_shrink() {
    let (mut i, ed) = setup();
    // Baseline grid without a panel.
    run(&mut i, "(insert \"line-one\")");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    assert!(ed.borrow().minibuffer.as_ref().unwrap().panel.is_some());
    let grid = core::redisplay::render(&i, &ed);
    let rows = grid.lines.len();
    let panel_h = (rows / 3).max(4);
    // The window's modeline moved up: it sits just above the separator.
    let modeline_row = rows - 2 - panel_h;
    let modeline: String = grid.lines[modeline_row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    assert!(
        modeline.contains("*scratch*"),
        "modeline at shrunk row: {}",
        modeline
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
    // Panel gone after C-g.
    assert!(ed.borrow().minibuffer.is_none());
}

#[test]
fn ret_on_prefilled_directory_opens_dired() {
    let (mut i, ed) = setup();
    let dir = fixture("dired");
    // Visit a file inside the fixture dir so the prefill points there.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("alpha.txt").to_str().unwrap()
        ),
    );
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    // Untouched selection + empty stem → RET submits the directory
    // itself → dired, not the first listed file.
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-read-only-p)"), "t");
    assert!(run(&mut i, "(buffer-string)").contains("beta.txt"));
}

#[test]
fn arrows_select_and_ret_opens_file() {
    let (mut i, ed) = setup();
    let dir = fixture("arrows");
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("alpha.txt").to_str().unwrap()
        ),
    );
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let cands = panel_accepts(&ed);
    // alpha.txt, beta.txt, subdir/ — sorted.
    assert_eq!(cands, vec!["alpha.txt", "beta.txt", "subdir/"]);
    // Down to beta.txt, RET opens it directly.
    feed_keys(&mut i, &ed, "<down>").unwrap();
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(run(&mut i, "(buffer-name)"), "\"beta.txt\"");
}

#[test]
fn up_arrow_wraps_selection() {
    let (mut i, ed) = setup();
    let dir = fixture("wrap");
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("alpha.txt").to_str().unwrap()
        ),
    );
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    feed_keys(&mut i, &ed, "<up>").unwrap();
    let sel = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .unwrap()
        .selected;
    assert_eq!(sel, 2, "up from 0 wraps to the last row");
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

#[test]
fn c_j_submits_literal_input() {
    let (mut i, ed) = setup();
    let dir = fixture("cj");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    // "alph" is a prefix of alpha.txt; C-j must create "alph" itself.
    type_str(&mut i, &ed, &format!("{}/alph", dir.to_str().unwrap()));
    assert_eq!(panel_accepts(&ed), vec!["alpha.txt"]);
    feed_keys(&mut i, &ed, "C-j").unwrap();
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(run(&mut i, "(buffer-name)"), "\"alph\"");
}

#[test]
fn new_file_via_empty_panel() {
    let (mut i, ed) = setup();
    let dir = fixture("newf");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    type_str(
        &mut i,
        &ed,
        &format!("{}/brandnew.txt", dir.to_str().unwrap()),
    );
    assert!(panel_accepts(&ed).is_empty(), "no matches for a new name");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"brandnew.txt\"");
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
}

#[test]
fn buffer_panel_lists_current_first_and_kills() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"aux-one\")");
    run(&mut i, "(get-buffer-create \"aux-two\")");
    // C-x b: current buffer is the first row and preselected.
    feed_keys(&mut i, &ed, "C-x b").unwrap();
    let cands = panel_accepts(&ed);
    assert_eq!(cands[0], "*scratch*");
    // Down selects aux-one; RET switches to it.
    feed_keys(&mut i, &ed, "<down>").unwrap();
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"aux-one\"");

    // C-x k: panel lists buffers; select aux-two and kill it.
    feed_keys(&mut i, &ed, "C-x k").unwrap();
    let cands = panel_accepts(&ed);
    assert_eq!(cands[0], "aux-one", "current buffer first in kill list");
    let target = cands.iter().position(|c| c == "aux-two").unwrap();
    for _ in 0..target {
        feed_keys(&mut i, &ed, "<down>").unwrap();
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(get-buffer \"aux-two\")"), "nil");
}

#[test]
fn buffer_panel_rows_carry_metadata_segments() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"dirty\")"); // *scratch* modified
    feed_keys(&mut i, &ed, "C-x b").unwrap();
    let has_modified_seg = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .unwrap()
        .rows[0]
        .segments
        .iter()
        .any(|(t, f)| t.contains('*') && *f == "panel-modified");
    assert!(has_modified_seg, "modified marker segment expected");
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// M61 T10: `.file` is now always absolute (find-file-internal
// normalizes it); the buffer-list panel's place column shows that
// absolute path verbatim when it's outside $HOME (`scratch_dir`-style
// fixtures under the OS temp dir aren't, on any of this project's
// supported platforms). This pins both "panel reads the normalized
// `.file`" and "no rewriting happens when the path isn't under HOME".
#[test]
fn buffer_panel_place_shows_full_absolute_path_outside_home() {
    let (mut i, ed) = setup();
    let dir = fixture("place_abs");
    let file = dir.join("alpha.txt");
    run(
        &mut i,
        &format!("(find-file-internal {:?})", file.to_str().unwrap()),
    );
    feed_keys(&mut i, &ed, "C-x b").unwrap();
    let row = ed
        .borrow()
        .minibuffer
        .as_ref()
        .unwrap()
        .panel
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .find(|r| r.accept == "alpha.txt")
        .unwrap()
        .segments
        .clone();
    let place_seg = row.iter().find(|(_, f)| *f == "panel-buffer-file");
    assert!(place_seg.is_some(), "expected a panel-buffer-file segment");
    let expected = format!("  {}", file.to_str().unwrap());
    assert_eq!(place_seg.unwrap().0, expected);
    feed_keys(&mut i, &ed, "C-g").unwrap();
}
