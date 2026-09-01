//! M24: major-mode infrastructure — `auto-mode-alist`, `normal-mode`,
//! `prog-mode-hook`, `rust-mode`, and the buffer-local
//! `display-line-numbers` gutter. See `crates/core/lisp/modes.el` and
//! `redisplay.rs`'s `buffer_var_on`/`editor::buffer_local_value`.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use core::redisplay::render;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (60, 10);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
}

/// Process-wide monotonic counter (see org_tests.rs's `unique_seq`):
/// parallel tests in the same process share a pid and can land on the
/// same nanosecond, so the timestamp alone occasionally collides.
fn unique_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
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
            "reticle_modes_{}_{}_{}_{}",
            tag,
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

fn temp_dir(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_and_open(
    interp: &mut Interp,
    dir: &std::path::Path,
    name: &str,
    contents: &str,
) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    run(
        interp,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    )
}

/// Plain-text row rendering (row_text pattern from gui_features_tests.rs):
/// trailing spaces trimmed, wide-char continuation cells dropped.
fn row_text(grid: &core::redisplay::Grid, row: usize) -> String {
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The (first) row whose rendered text contains `needle` — used instead
/// of hardcoded row arithmetic so these tests don't have to duplicate
/// render()'s window/modeline geometry math.
fn find_row(grid: &core::redisplay::Grid, needle: &str) -> usize {
    (0..grid.lines.len())
        .find(|&r| row_text(grid, r).contains(needle))
        .unwrap_or_else(|| {
            let dump: Vec<String> = (0..grid.lines.len()).map(|r| row_text(grid, r)).collect();
            panic!("no row contains {:?}; grid:\n{}", needle, dump.join("\n"))
        })
}

#[test]
fn rust_file_gets_rust_mode_gutter_and_modeline_name() {
    let (mut i, ed) = setup();
    // Wider than `setup`'s 60 columns on purpose. This test's title is
    // "reticle_modes_rust_<pid>_<timestamp>_<n>/main.rs" -- 48 columns
    // of temp-dir fixture -- and M69's mode line drops the mode name before
    // it drops "L:C" when the two don't both fit. At 60 columns that ladder
    // fires and the name this test looks for is (correctly) gone. The
    // fixture's length is a harness artifact, not a real buffer title, so
    // the frame is widened rather than the ladder reordered.
    ed.borrow_mut().frame = (100, 10);
    let dir = temp_dir("rust");
    let r = write_and_open(&mut i, &dir, "main.rs", "fn main() {\n    1;\n}\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "rust-mode");

    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "fn main");
    let text = row_text(&grid, r0);
    let digit_pos = text.find('1');
    let text_pos = text.find("fn main");
    assert!(
        digit_pos.is_some() && digit_pos.unwrap() < text_pos.unwrap(),
        "expected a line-number gutter before the text: {:?}",
        text
    );

    let mode_row = find_row(&grid, "rust-mode");
    assert!(row_text(&grid, mode_row).contains("rust-mode"));
}

#[test]
fn plain_text_file_gets_fundamental_mode_and_no_gutter() {
    let (mut i, ed) = setup();
    let dir = temp_dir("txt");
    let r = write_and_open(&mut i, &dir, "notes.txt", "hello there\nsecond line\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "fundamental-mode");

    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "hello there");
    assert!(
        row_text(&grid, r0).starts_with("hello there"),
        "expected no gutter before the text: {:?}",
        row_text(&grid, r0)
    );
}

/// Migration guard: org.el used to turn on org-mode via an ad hoc
/// find-file-hook lambda; it now registers "\\.org\\'" in
/// auto-mode-alist like every other mode. org_tests.rs is the deep
/// coverage for org-mode itself — this only guards the dispatch path.
#[test]
fn org_file_still_gets_org_mode_after_migration_to_auto_mode_alist() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("org");
    let r = write_and_open(&mut i, &dir, "notes.org", "* heading\ntext\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "org-mode");
}

#[test]
fn user_can_register_a_custom_auto_mode_alist_entry() {
    let (mut i, _ed) = setup();
    run(&mut i, "(defvar my-mode-ran nil)");
    run(
        &mut i,
        "(defun my-custom-mode ()
           (major-mode-internal-set 'my-custom-mode)
           (setq my-mode-ran t))",
    );
    // Exercised with the exact GNU idiom from the task spec, including
    // the \\' end-of-string anchor.
    let r = run(
        &mut i,
        r#"(add-to-list 'auto-mode-alist '("\\.mycustom\\'" . my-custom-mode))"#,
    );
    assert!(!r.starts_with("ERROR"), "add-to-list failed: {}", r);

    let dir = temp_dir("custom");
    let opened = write_and_open(&mut i, &dir, "thing.mycustom", "data\n");
    assert!(!opened.starts_with("ERROR"), "find-file failed: {}", opened);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "my-custom-mode");
    assert_eq!(run(&mut i, "my-mode-ran"), "t");
}

/// `add-to-list` must not duplicate an entry that's already `member` of
/// the list (GNU semantics), and it must actually mutate the variable
/// (not just its local view of it).
#[test]
fn add_to_list_is_idempotent_and_mutates_the_symbol_value() {
    let (mut i, _ed) = setup();
    run(&mut i, "(defvar amtl-test-list '(b c))");
    assert_eq!(run(&mut i, "(add-to-list 'amtl-test-list 'a)"), "(a b c)");
    assert_eq!(run(&mut i, "amtl-test-list"), "(a b c)");
    // Re-adding an existing element is a no-op.
    assert_eq!(run(&mut i, "(add-to-list 'amtl-test-list 'b)"), "(a b c)");
    assert_eq!(run(&mut i, "amtl-test-list"), "(a b c)");
}

/// prog-mode-hook's default "(setq-local display-line-numbers t)" is
/// just a hook function; the user can remove it like any other and get
/// prog-mode buffers with no gutter. (M36 added a second default hook
/// function, the RET->newline-and-indent local binding -- see
/// indent.el -- so this clears prog-mode-hook entirely rather than
/// assuming a single entry; the point of this test is still "the
/// defaults are ordinary, removable hook functions", not "there is
/// exactly one".)
#[test]
fn prog_mode_hook_default_can_be_overridden_by_the_user() {
    let (mut i, ed) = setup();
    assert_eq!(run(&mut i, "(setq prog-mode-hook nil)"), "nil");
    assert_eq!(run(&mut i, "prog-mode-hook"), "nil");

    let dir = temp_dir("noln");
    let r = write_and_open(&mut i, &dir, "c.rs", "fn c() {}\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "rust-mode");

    let grid = render(&i, &ed);
    let r0 = find_row(&grid, "fn c");
    assert!(
        row_text(&grid, r0).starts_with("fn c"),
        "expected no gutter once prog-mode-hook's default is removed: {:?}",
        row_text(&grid, r0)
    );
}

/// The core M24 regression guard: `display-line-numbers` is commonly
/// buffer-local (prog-mode-hook sets it per-buffer), and two windows can
/// show two buffers with different local values at once. `render_window`
/// must read each window's toggle from the buffer it's painting, not
/// from whichever buffer happens to be current.
#[test]
fn buffer_local_line_numbers_render_independently_per_window() {
    let (mut i, ed) = setup();

    // Window A (selected window, pre-split): a buffer with an explicit
    // local line-numbers toggle.
    run(&mut i, "(switch-to-buffer \"alpha\")");
    run(&mut i, "(insert \"alpha one\\nalpha two\")");
    run(&mut i, "(setq-local display-line-numbers t)");

    run(&mut i, "(split-window-below)");
    run(&mut i, "(other-window 1)");

    // Window B (the new window): a fresh buffer with no local binding
    // at all — display-line-numbers is nil globally by default.
    run(&mut i, "(switch-to-buffer \"beta\")");
    run(&mut i, "(insert \"beta text\")");
    assert_eq!(
        run(&mut i, "(local-variable-p 'display-line-numbers)"),
        "nil"
    );

    let grid = render(&i, &ed);
    let alpha_row = find_row(&grid, "alpha one");
    let beta_row = find_row(&grid, "beta text");
    let alpha_text = row_text(&grid, alpha_row);
    let beta_text = row_text(&grid, beta_row);
    let digit_pos = alpha_text.find('1');
    let alpha_pos = alpha_text.find("alpha one");
    assert!(
        digit_pos.is_some() && digit_pos.unwrap() < alpha_pos.unwrap(),
        "alpha's window should have a line-number gutter: {:?}",
        alpha_text
    );
    assert!(
        beta_text.starts_with("beta text"),
        "beta's window should have no gutter: {:?}",
        beta_text
    );

    // Switch the CURRENT BUFFER (not the selected window — set-buffer,
    // not switch-to-buffer) to beta and re-render. Alpha's window still
    // shows alpha, but alpha is no longer the current buffer, so its
    // local display-line-numbers value now lives only in alpha's own
    // `locals` map, not the global cell (which now holds beta's, i.e.
    // effectively nothing local). A read that naively consulted the
    // global cell for every window would wrongly report alpha's gutter
    // gone; buffer_var_on must still get it right per-window.
    run(&mut i, "(set-buffer \"beta\")");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"beta\"");
    let grid2 = render(&i, &ed);
    let alpha_row2 = find_row(&grid2, "alpha one");
    let alpha_text2 = row_text(&grid2, alpha_row2);
    let digit_pos2 = alpha_text2.find('1');
    let alpha_pos2 = alpha_text2.find("alpha one");
    assert!(
        digit_pos2.is_some() && digit_pos2.unwrap() < alpha_pos2.unwrap(),
        "alpha's gutter must survive an unrelated current-buffer switch: {:?}",
        alpha_text2
    );
}
