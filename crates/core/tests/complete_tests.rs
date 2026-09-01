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

fn tab(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    handle_key(interp, ed, Key::Char(9));
}

fn mb_input(ed: &Rc<RefCell<Editor>>) -> String {
    ed.borrow()
        .minibuffer
        .as_ref()
        .map(|m| m.input.clone())
        .unwrap_or_default()
}

fn popup(ed: &Rc<RefCell<Editor>>) -> Option<(Vec<String>, usize)> {
    ed.borrow()
        .minibuffer
        .as_ref()
        .and_then(|m| m.completion.as_ref())
        .map(|c| (c.candidates.clone(), c.selected))
}

/// The M21 panel's accept strings and selection.
fn panel(ed: &Rc<RefCell<Editor>>) -> Option<(Vec<String>, usize)> {
    ed.borrow()
        .minibuffer
        .as_ref()
        .and_then(|m| m.panel.as_ref())
        .map(|p| {
            (
                p.rows.iter().map(|r| r.accept.clone()).collect(),
                p.selected,
            )
        })
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
            "se_complete_{}_{}_{}",
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

/// A scratch dir with a fixed set of entries for file completion.
fn fixture_dir(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(dir.join("subdir")).unwrap();
    std::fs::write(dir.join("alpha.txt"), "A").unwrap();
    std::fs::write(dir.join("alphabet.txt"), "B").unwrap();
    std::fs::write(dir.join("beta.txt"), "C").unwrap();
    dir
}

#[test]
fn find_file_prefills_default_directory() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("prefill");
    // Visit a file so the current buffer has a directory.
    run(
        &mut i,
        &format!(
            "(find-file-internal {:?})",
            dir.join("beta.txt").to_str().unwrap()
        ),
    );
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let input = mb_input(&ed);
    assert!(input.ends_with('/'), "prefill should end with /: {}", input);
    assert!(
        input.contains(&format!("se_complete_prefill_{}", std::process::id())),
        "prefill should be the visited file's directory: {}",
        input
    );
    // Cursor sits at the end of the prefill.
    let cursor = ed.borrow().minibuffer.as_ref().unwrap().cursor;
    assert_eq!(cursor, input.chars().count());
}

#[test]
fn tab_completes_common_prefix_then_lists() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("lcp");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    // The panel opens with the minibuffer (M21).
    assert!(panel(&ed).is_some(), "panel should open immediately");
    // Replace the prefill by typing an absolute path (GNU // shadowing
    // applies at submit; completion works on the raw text after '/').
    type_str(&mut i, &ed, &format!("{}/alp", dir.to_str().unwrap()));
    // Typing filtered the panel down to the two matches.
    let (cands, sel) = panel(&ed).expect("panel");
    assert_eq!(
        cands,
        vec!["alpha.txt".to_string(), "alphabet.txt".to_string()]
    );
    assert_eq!(sel, 0);
    tab(&mut i, &ed);
    // "alpha.txt" vs "alphabet.txt" → extends to the common prefix.
    assert!(
        mb_input(&ed).ends_with("/alpha"),
        "LCP extension failed: {}",
        mb_input(&ed)
    );
    // Second TAB: no further extension → cycles the panel selection.
    tab(&mut i, &ed);
    assert_eq!(panel(&ed).unwrap().1, 1);
    // RET accepts the highlighted candidate and, for a file, submits.
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "file selection should submit"
    );
    assert_eq!(run(&mut i, "(buffer-name)"), "\"alphabet.txt\"");
}

#[test]
fn unique_match_completes_fully() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("uniq");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    type_str(&mut i, &ed, &format!("{}/bet", dir.to_str().unwrap()));
    tab(&mut i, &ed);
    assert!(
        mb_input(&ed).ends_with("/beta.txt"),
        "unique completion failed: {}",
        mb_input(&ed)
    );
    assert!(popup(&ed).is_none());
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"beta.txt\"");
}

#[test]
fn directory_completion_descends() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("dir");
    std::fs::write(dir.join("subdir/inner.txt"), "I").unwrap();
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    type_str(&mut i, &ed, &format!("{}/sub", dir.to_str().unwrap()));
    tab(&mut i, &ed);
    // Completes to "subdir/" and stays in the minibuffer.
    assert!(
        mb_input(&ed).ends_with("/subdir/"),
        "dir completion failed: {}",
        mb_input(&ed)
    );
    assert!(ed.borrow().minibuffer.is_some());
    // The panel now lists the subdirectory's contents.
    assert_eq!(panel(&ed).unwrap().0, vec!["inner.txt".to_string()]);
    // TAB inside the directory uniquely completes its single entry.
    tab(&mut i, &ed);
    assert!(
        mb_input(&ed).ends_with("/subdir/inner.txt"),
        "descend failed: {}",
        mb_input(&ed)
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"inner.txt\"");
}

#[test]
fn accepting_directory_candidate_keeps_minibuffer_open() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("keep");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    type_str(&mut i, &ed, &format!("{}/", dir.to_str().unwrap()));
    // Empty stem: the panel lists all 4 entries ("."/".." excluded).
    let (cands, _) = panel(&ed).expect("panel");
    assert_eq!(cands.len(), 4, "expected all entries: {:?}", cands);
    // Arrow down to "subdir/" and accept it.
    let target = cands.iter().position(|c| c == "subdir/").unwrap();
    for _ in 0..target {
        feed_keys(&mut i, &ed, "<down>").unwrap();
    }
    assert_eq!(panel(&ed).unwrap().1, target);
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "directory accept must not submit"
    );
    assert!(
        mb_input(&ed).ends_with("/subdir/"),
        "input: {}",
        mb_input(&ed)
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

#[test]
fn no_match_shows_note_and_keeps_input() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("nomatch");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let typed = format!("{}/zzz", dir.to_str().unwrap());
    type_str(&mut i, &ed, &typed);
    tab(&mut i, &ed);
    assert!(mb_input(&ed).ends_with("/zzz"), "input must be unchanged");
    let note = ed.borrow().minibuffer.as_ref().unwrap().note.clone();
    assert_eq!(note.as_deref(), Some(" [No match]"));
    assert!(popup(&ed).is_none());
    // Typing clears the note.
    type_str(&mut i, &ed, "x");
    assert!(ed.borrow().minibuffer.as_ref().unwrap().note.is_none());
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

#[test]
fn meta_x_completes_command_names() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"abc\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "beginning-of-b");
    tab(&mut i, &ed);
    assert_eq!(mb_input(&ed), "beginning-of-buffer");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_none());
    assert_eq!(run(&mut i, "(point)"), "1");
}

// M84: M-x now lists candidates through the M21 panel as they're typed
// (no TAB needed) rather than through the old TAB-triggered popup --
// renamed from `meta_x_popup_excludes_non_commands` since "popup" no
// longer describes what this exercises.
#[test]
fn meta_x_panel_excludes_non_commands() {
    let (mut i, ed) = setup();
    // A command and a plain function sharing a prefix.
    run(
        &mut i,
        "(progn
           (defun se-test-cmd-one () (interactive) 1)
           (defun se-test-cmd-two () (interactive) 2)
           (defun se-test-cmd-plain () 3))",
    );
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "se-test-cmd-");
    // No TAB: the panel must already be filtered from typing alone.
    let (cands, _) = panel(&ed).expect("panel should be open for two commands");
    assert_eq!(
        cands,
        vec!["se-test-cmd-one".to_string(), "se-test-cmd-two".to_string()]
    );
    // RET runs the highlighted command immediately.
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_none());
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// M84 D5: the panel (type-as-you-filter) and TAB (longest-common-prefix
// expansion) are orthogonal -- TAB must still do its own job on this
// source now that it no longer also opens a popup here.
#[test]
fn meta_x_tab_still_expands_common_prefix_with_panel_open() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(progn
           (defun se-test-cmd-one () (interactive) 1)
           (defun se-test-cmd-two () (interactive) 2))",
    );
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "se-test-cmd-");
    // Two candidates share "se-test-cmd-" as their longest common
    // prefix already -- TAB has nothing further to add.
    tab(&mut i, &ed);
    assert_eq!(mb_input(&ed), "se-test-cmd-");
    // Narrow to a single candidate; TAB now completes the full name.
    type_str(&mut i, &ed, "o");
    tab(&mut i, &ed);
    assert_eq!(mb_input(&ed), "se-test-cmd-one");
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

#[test]
fn buffer_name_completion() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"zebra-one\")");
    run(&mut i, "(get-buffer-create \"zebra-two\")");
    feed_keys(&mut i, &ed, "C-x b").unwrap();
    // The panel opens immediately, current buffer first and selected.
    let (cands, sel) = panel(&ed).expect("panel");
    assert_eq!(cands[0], "*scratch*", "current buffer first: {:?}", cands);
    assert_eq!(sel, 0);
    type_str(&mut i, &ed, "zeb");
    tab(&mut i, &ed);
    assert_eq!(mb_input(&ed), "zebra-");
    let (cands, _) = panel(&ed).expect("panel");
    assert_eq!(
        cands,
        vec!["zebra-one".to_string(), "zebra-two".to_string()]
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"zebra-one\"");
}

#[test]
fn double_slash_shadows_prefilled_directory() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("shadow");
    let path = dir.join("beta.txt");
    // Prefill points at the cwd; typing an absolute path on top of it
    // (the "//" junction) must open the typed path, like GNU Emacs.
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    let prefill = mb_input(&ed);
    assert!(!prefill.is_empty());
    type_str(&mut i, &ed, path.to_str().unwrap());
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"C\"");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"beta.txt\"");
}

#[test]
fn panel_renders_in_grid() {
    let (mut i, ed) = setup();
    let dir = fixture_dir("render");
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    type_str(&mut i, &ed, &format!("{}/alp", dir.to_str().unwrap()));
    let grid = core::redisplay::render(&i, &ed);
    let rows = grid.lines.len();
    let text_row = |r: usize| -> String {
        grid.lines[r]
            .iter()
            .filter(|c| !c.continuation)
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    };
    // Panel occupies the bottom third above the echo row: separator
    // line, then the two ls -al style candidate rows.
    let panel_h = (rows / 3).max(4);
    let sep_row = rows - 1 - panel_h;
    assert!(
        text_row(sep_row).starts_with('─'),
        "separator: {}",
        text_row(sep_row)
    );
    let r1 = text_row(sep_row + 1);
    let r2 = text_row(sep_row + 2);
    assert!(
        r1.contains("alpha.txt") && r1.contains("rw"),
        "row1: {}",
        r1
    );
    assert!(r2.contains("alphabet.txt"), "row2: {}", r2);
}
