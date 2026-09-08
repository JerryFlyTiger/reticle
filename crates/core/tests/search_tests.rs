//! M82: `search-project`/`search-again`, streaming results into
//! `*search*` (`search.el`). Follows `compile_tests.rs`'s shape -- async
//! pump + buffer content -- as the closest existing precedent, but adds
//! coverage `compile_tests.rs` never needed: `compile.el` parses its
//! whole buffer ONCE at process exit (streamed chunks don't respect
//! line boundaries, so it just waits), while search results must be
//! navigable WHILE the job is still streaming, which means this file's
//! own incremental line-buffering (`search--feed-chunk`) is the thing
//! most worth trying to break.
//!
//! No real `rg` required for most of these -- `search-program`/
//! `search-arguments-literal` are swapped for a fake generator
//! (`printf`/`bash -c`, same technique `compile_tests.rs` already uses
//! for a fake compiler) so the suite doesn't depend on a real engine
//! being installed. A handful of tests genuinely need real `rg` (real
//! regex-engine behavior, or real command-not-found/bad-regex exit
//! codes, can't be faked by a canned-output generator) and skip
//! themselves if `rg` isn't on PATH -- same precedent this codebase
//! already has for `verible-verilog-ls` (see CLAUDE.md).
//!
//! Fix round R1 (independent cold-read review, post-M82): F1-F7 below
//! are new coverage/behavior added in this round; the file header
//! comments on each new/changed test say which finding they correspond
//! to.

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
    // M104 fix round: this file saves real .v files via
    // `apply_edits'/`search--edit-apply-to-file' (17 call sites), and
    // M104's `format-on-save' defaults to `t'. Every fixture here
    // happens to be invalid Verilog (e.g. "l1\nOLD\nl3\n"), and this
    // machine's `verible-verilog-format' happens to degrade gracefully
    // on a parse failure (exit 0, output unchanged) -- so today nothing
    // actually reformats. That is a coincidence of (a) what these
    // fixtures look like and (b) what one version of one external tool
    // does with bad input, not a property this file is actually
    // testing (it tests search/replace-across-files, not formatting).
    // `search_edit_apply_rolls_back_in_memory_edit_when_save_buffer_
    // fails' in particular relies on an `undo-boundary' placed after the
    // edit and before `save-buffer' -- if formatting ever DID touch the
    // buffer, that undo would roll back the formatting step instead of
    // the search edit under test. Opting out unconditionally, same as
    // `verilog_auto_tests.rs'/`lsp_save_close_tests.rs'/
    // `lsp_mode_tests.rs' before it.
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

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

/// Pump idle ticks until `pred` is true or the timeout hits. Same shape
/// as `compile_tests.rs`'s own helper of the same name.
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

fn no_search_procs_running(i: &mut Interp) -> bool {
    run(i, "search--procs") == "nil"
}

/// A scratch directory that deletes itself on drop -- same shape
/// `compile_tests.rs`'s `Scratch` uses.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "se_search_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn write(scratch: &std::path::Path, rel: &str, contents: &str) {
    let p = scratch.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, contents).unwrap();
}

/// Visit FILE (must already exist on disk) so `buffer-file-name' has
/// something real to resolve -- with no `.git' anywhere above a
/// `std::env::temp_dir()' scratch directory, `search--default-root'
/// falls back to the file's own directory, exactly the scratch
/// directory these tests want the search's working directory to be.
fn visit(i: &mut Interp, path: &std::path::Path) {
    let out = run(i, &format!("(find-file {:?})", path.to_str().unwrap()));
    assert!(
        !out.starts_with("ERROR"),
        "find-file {:?} failed: {}",
        path,
        out
    );
}

/// Point `search-program'/`search-arguments-literal' at a fake
/// generator (PROGRAM/ARGS, both spliced verbatim into the elisp source
/// via `{:?}`'s Rust-Debug escaping -- which happens to also be valid
/// elisp string-literal escaping for the plain ASCII text every caller
/// here uses, same backslash/quote rules). `search-project` (the
/// default, literal-mode command) is the one every fake-engine test
/// drives, so `search-arguments-literal' is the variable to override;
/// `search-arguments-regexp' is left at its real default for the
/// handful of tests that exercise `search-project-regexp' directly
/// against real `rg'.
fn set_fake_engine(i: &mut Interp, program: &str, args: &str) {
    run(i, &format!("(setq search-program {:?})", program));
    run(i, &format!("(setq search-arguments-literal {:?})", args));
}

/// A fake `printf'-based engine that prints LINES verbatim (one `%s\n'
/// each) and exits 0 -- the search pattern the user types is silently
/// discarded via the trailing `#' shell comment, same trick
/// `compile_tests.rs`'s `fake_compiler_printf' doesn't need (it never
/// appends anything after the user's command) but this file does,
/// since `search--start' always appends the shell-quoted pattern after
/// `search-arguments-literal'/`search-arguments-regexp'.
fn set_fake_printf_engine(i: &mut Interp, lines: &[&str]) {
    let quoted: Vec<String> = lines
        .iter()
        .map(|l| format!("'{}'", l.replace('\'', "'\\''")))
        .collect();
    let args = format!("'%s\\n' {}; true #", quoted.join(" "));
    set_fake_engine(i, "printf", &args);
}

/// Run `M-x search-project', typing PATTERN at the "Search (literal)
/// for:" prompt.
fn do_search(i: &mut Interp, ed: &Rc<RefCell<Editor>>, pattern: &str) {
    do_search_command(i, ed, "search-project", pattern);
}

/// Run `M-x search-project-regexp', typing PATTERN at the "Search
/// (regexp) for:" prompt.
fn do_search_regexp(i: &mut Interp, ed: &Rc<RefCell<Editor>>, pattern: &str) {
    do_search_command(i, ed, "search-project-regexp", pattern);
}

/// Run `M-x search-again' -- no prompt, so unlike `do_search'/`do_search_regexp'
/// this never opens a second minibuffer.
fn do_search_again(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    feed_keys(i, ed, "M-x").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-x should open a minibuffer prompt"
    );
    type_str(i, ed, "search-again");
    feed_keys(i, ed, "RET").unwrap();
}

fn do_search_command(i: &mut Interp, ed: &Rc<RefCell<Editor>>, command: &str, pattern: &str) {
    feed_keys(i, ed, "M-x").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-x should open a minibuffer prompt"
    );
    type_str(i, ed, command);
    feed_keys(i, ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "{} should open a second minibuffer prompt (Search ... for:)",
        command
    );
    type_str(i, ed, pattern);
    feed_keys(i, ed, "RET").unwrap();
}

fn search_buffer_string(i: &mut Interp) -> String {
    run(i, "(with-current-buffer \"*search*\" (buffer-string))")
}

// --- M83 helpers: search-edit-mode (C-x C-q / C-c C-c / C-c C-k) -------

/// `C-x C-q' while `*search*' is current -- enters `search-edit-mode'.
fn enter_edit_mode(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    run(i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(i, ed, "C-x C-q").unwrap();
}

/// `C-c C-c' while `*search*' is current -- applies edits.
fn apply_edits(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    run(i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(i, ed, "C-c C-c").unwrap();
}

/// `C-c C-k' while `*search*' is current -- discards edits.
fn discard_edits(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    run(i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(i, ed, "C-c C-k").unwrap();
}

/// Replace the ROW0 (0-based)-th line of `*search*' with NEW_TEXT --
/// only valid while the buffer is writable (`search-edit-mode').
fn set_search_line(i: &mut Interp, row0: usize, new_text: &str) {
    run(
        i,
        &format!(
            "(with-current-buffer \"*search*\" (goto-char (point-min)) (forward-line {}) \
             (delete-region (line-beginning-position) (line-end-position)) (insert {:?}))",
            row0, new_text
        ),
    );
}

fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {:?}: {}", path, e))
}

// --- 1. Three lines from a fake engine land in *search* verbatim -------

#[test]
fn search_streams_fake_engine_output_into_search_buffer() {
    let scratch = Scratch::new("basic");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "one\n");
    write(&scratch, "b.v", "two\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:one", "b.v:1:1:two", "a.v:2:2:three"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    assert_eq!(
        search_buffer_string(&mut i),
        "\"a.v:1:1:one\\nb.v:1:1:two\\na.v:2:2:three\\n\""
    );
    assert_eq!(run(&mut i, "(length search--results)"), "3");
}

// --- 2. A result line split across two separate poll chunks still ------
// --- parses identically to one delivered whole -- the core new --------
// --- engineering this milestone adds over compile.el's full-buffer- ---
// --- at-exit approach. -------------------------------------------------

#[test]
fn search_parses_a_result_line_split_across_two_chunks() {
    let scratch = Scratch::new("splitline");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "hello\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // Two separate `printf' calls with a real sleep between them force
    // `shell-process-poll' to hand back two distinct chunks -- the
    // first ending mid-line (no trailing newline at all).
    set_fake_engine(
        &mut i,
        "bash",
        "-c 'printf \"a.v:1:1:hel\"; sleep 0.3; printf \"lo\\n\"' #",
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    assert_eq!(search_buffer_string(&mut i), "\"a.v:1:1:hello\\n\"");
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "the split line must still parse into exactly one result"
    );
    let file = run(&mut i, "(aref (car search--results) 0)");
    assert!(file.ends_with("a.v\""), "result FILE must be a.v: {}", file);
}

// --- 3. An unparseable line is displayed but not jumpable --------------

#[test]
fn search_keeps_unparseable_lines_visible_but_not_jumpable() {
    let scratch = Scratch::new("unparseable");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "one\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(
        &mut i,
        &[
            "a warning banner with no FILE:LINE:COL shape",
            "a.v:1:1:hit",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    let content = search_buffer_string(&mut i);
    assert!(
        content.contains("a warning banner with no FILE:LINE:COL shape"),
        "unparseable line must still be displayed: {}",
        content
    );
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "only the one real result line should be jumpable"
    );
}

// --- 4. n/p cyclic navigation, plus the local keymap binding itself ----

#[test]
fn search_n_p_navigate_cyclically() {
    let scratch = Scratch::new("navcycle");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "line1\nline2\nline3\n");
    write(&scratch, "b.v", "x\ny\nz\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:one", "a.v:2:1:two", "b.v:1:1:three"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    // First jump via a REAL keypress while `*search*' is current --
    // exercises `search--ensure-output-buffer''s local keymap install,
    // not just the underlying `search-next-result' function.
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "n").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"a.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");

    // Remaining jumps via M-x (current buffer is now a.v, not
    // `*search*', so its local `n' binding isn't reachable by a plain
    // keypress here -- same reasoning `compile_tests.rs' already
    // documents for its own cross-file `compile-next-error' sequence).
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-next-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"a.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-next-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"b.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");

    // Wraps back to the first entry.
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-next-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"a.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");

    // `p' from the first entry wraps backward to the last.
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-previous-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"b.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");
}

// --- 5. RET jumps to the correct file/line/column, clamped past EOL ----

#[test]
fn search_ret_jumps_to_column_and_clamps_past_line_end() {
    let scratch = Scratch::new("column");
    write(&scratch, "main.sv", "// nothing\n");
    // Line 2 is short (5 chars) -- COL 19 must clamp to end-of-line.
    write(&scratch, "a.v", "line1\nshort\nline3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:3:msg one", "a.v:2:19:msg two"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-next-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"a.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");
    assert_eq!(
        run(&mut i, "(current-column)"),
        "2",
        "col 3 (1-based) -> point 2 chars in"
    );

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-next-result");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"a.v\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");
    assert_eq!(
        run(&mut i, "(current-column)"),
        "5",
        "col 19 must clamp to end of a 5-char line"
    );
}

// --- 6. Cross-file jump via RET at point in *search* --------------------

#[test]
fn search_goto_result_at_point_jumps_across_files() {
    let scratch = Scratch::new("crossfile");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nl2\n");
    write(&scratch, "b.v", "m1\nm2\nm3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:one", "b.v:3:1:two"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(forward-line 1)"); // second line: b.v:3:1:two
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"b.v\"",
        "RET must jump to the file named on the line point was on"
    );
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "3");
}

// --- 7. Per-tick insertion budget: 1000 lines, budget 10 -> drained ----
// --- over multiple ticks, never more than the budget in one tick. -----

#[test]
fn search_respects_per_tick_line_budget() {
    let scratch = Scratch::new("budget");
    write(&scratch, "main.sv", "// nothing\n");
    // `search--parse-result-line' requires FILE to exist on disk (same
    // ambiguity guard `compile--parse-error-line' uses, see search.el's
    // header) -- without this, every generated `f.v:...' line would
    // fail to parse and `search--results' would never grow at all,
    // masking the very budget behavior this test exists to observe.
    write(&scratch, "f.v", "placeholder\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    run(&mut i, "(setq search-max-lines-per-tick 10)");
    set_fake_engine(
        &mut i,
        "bash",
        "-c 'for n in $(seq 1 1000); do printf \"f.v:%d:1:x\\n\" \"$n\"; done' #",
    );
    do_search(&mut i, &ed, "x");

    let mut prev = 0usize;
    let mut saw_partial = false;
    let start = Instant::now();
    loop {
        core::idle_tick(&mut i, Duration::from_millis(0));
        let n: usize = run(&mut i, "(length search--results)")
            .trim()
            .parse()
            .expect("(length search--results) must print an integer");
        assert!(
            n <= prev + 10,
            "a single tick inserted more than the budget: {} -> {}",
            prev,
            n
        );
        if n > 0 && n < 1000 {
            saw_partial = true;
        }
        prev = n;
        if no_search_procs_running(&mut i) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "search job never finished draining"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        saw_partial,
        "expected to observe an intermediate (not-yet-complete) result count"
    );
    assert_eq!(prev, 1000);
}

// --- 8. search-max-results truncates: process killed, banner shown,
// --- EXACTLY N results inserted (F4: off-by-one, tightened assertion) --

#[test]
fn search_max_results_truncates_and_kills_process() {
    let scratch = Scratch::new("truncate");
    write(&scratch, "main.sv", "// nothing\n");
    // See `search_respects_per_tick_line_budget''s identical comment --
    // `f.v' must exist for the generated lines to actually register in
    // `search--results', or the count assertion below is vacuous.
    write(&scratch, "f.v", "placeholder\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    run(&mut i, "(setq search-max-results 50)");
    set_fake_engine(
        &mut i,
        "bash",
        "-c 'for n in $(seq 1 500); do printf \"f.v:%d:1:x\\n\" \"$n\"; done' #",
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(10), no_search_procs_running);
    assert!(ok, "truncated search job never finished");

    // F4 (fix round R1): the cap is EXACTLY 50, not "50 or 51" -- an
    // earlier version of `search--drain-queue' compared with `>' instead
    // of `>=' and let exactly one extra line through every time. A
    // `<= 51' assertion here could not tell the fixed behavior apart
    // from the off-by-one bug, so this is tightened to an exact value.
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "50",
        "search-max-results=50 must admit EXACTLY 50 results, not 51"
    );
    let content = search_buffer_string(&mut i);
    assert!(
        content.contains("truncated"),
        "buffer must carry a truncation notice: {}",
        content
    );
}

// --- 9. has_async_work sees search--procs -------------------------------

#[test]
fn has_async_work_true_for_search_procs() {
    let (mut i, _ed) = setup();
    assert!(!core::has_async_work(&mut i), "no jobs running yet");
    run(&mut i, "(setq search--procs (list 1))");
    assert!(
        core::has_async_work(&mut i),
        "search--procs must count as async work"
    );
    run(&mut i, "(setq search--procs nil)");
}

// --- 10. Starting a second search kills the first ------------------------

#[test]
fn starting_second_search_kills_first() {
    let scratch = Scratch::new("secondkill");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_engine(&mut i, "bash", "-c 'sleep 30' #");
    do_search(&mut i, &ed, "x");
    let started = pump_until(&mut i, Duration::from_secs(2), |i| {
        run(i, "search--procs") != "nil"
    });
    assert!(started, "first search never registered a running process");
    run(
        &mut i,
        "(setq test--first-proc (aref (car search--procs) 0))",
    );
    assert_eq!(
        run(&mut i, "(shell-process-live-p test--first-proc)"),
        "t",
        "first search's process must actually be alive before the second starts"
    );

    set_fake_printf_engine(&mut i, &["a.v:1:1:x"]);
    do_search(&mut i, &ed, "y");
    assert_eq!(
        run(&mut i, "(shell-process-live-p test--first-proc)"),
        "nil",
        "starting a second search must kill the first process immediately"
    );
}

// --- 11. Real-rg end-to-end, skipped if rg isn't on PATH ------------------

#[test]
fn search_e2e_with_real_rg_if_available() {
    let has_rg = std::process::Command::new("rg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_rg {
        eprintln!("search_e2e_with_real_rg_if_available: skipping, `rg` not on PATH");
        return;
    }
    let scratch = Scratch::new("e2e");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "hit.v", "hello world\nneedle here\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // `search-program'/`search-arguments-literal' are left at their
    // real defaults here -- this is one of the tests meant to exercise
    // them for real.
    do_search(&mut i, &ed, "needle");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "real rg search never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "1");
    let file = run(&mut i, "(aref (car search--results) 0)");
    assert!(
        file.ends_with("hit.v\""),
        "expected the match to be in hit.v: {}",
        file
    );
}

// --- 12. Output lands even when *search* isn't the current buffer -------

#[test]
fn search_output_lands_when_search_buffer_is_not_current() {
    let scratch = Scratch::new("notcurrent");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "one\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:one"]);
    do_search(&mut i, &ed, "x");

    // Immediately switch away -- the whole point of this milestone is
    // that the user can keep editing while the search streams in.
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");

    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    // Current buffer must be UNCHANGED by the pump...
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*scratch*\"");
    // ...yet the result must still have landed.
    assert_eq!(search_buffer_string(&mut i), "\"a.v:1:1:one\\n\"");
    assert_eq!(run(&mut i, "(length search--results)"), "1");
}

// =========================================================================
// Fix round R1 (independent cold-read review, post-M82 initial landing):
// F1 (exit code discarded), F3 (empty-pattern guard), F4 (already
// tightened above, test 8), F5 (zero/negative per-tick budget hangs),
// F6 (three zero-coverage defense lines), F7 (literal-vs-regexp modes).
// =========================================================================

fn has_rg() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// --- F1a. A nonexistent `search-program' gets an explicit abnormal-exit
// --- message naming the shell's own "command not found" code, not the
// --- generic "Search finished" every exit code used to produce. -------

#[test]
fn search_reports_abnormal_exit_when_program_not_found() {
    // Repro before this fix: pointing `search-program' at a command that
    // doesn't exist printed the exact same "Search finished (0 results)"
    // as a real, successful, merely-empty search -- nothing distinguished
    // a typo'd program name from an ordinary zero-hit search.
    let scratch = Scratch::new("badprogram");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    run(
        &mut i,
        "(setq search-program \"se-definitely-not-a-real-command-xyz\")",
    );
    run(&mut i, "(setq search-arguments-literal \"\")");
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("abnormally") && echo.contains("127"),
        "expected an explicit abnormal-exit message naming the shell's \
         \"command not found\" exit code (127): {:?}",
        echo
    );
}

// --- F1b. Exit code 1 (rg's own "ran fine, matched nothing" convention)
// --- gets its own distinct message, not folded into either the normal
// --- "finished (N results)" or a scary "exited abnormally" message. ---

#[test]
fn search_reports_no_matches_message_for_exit_code_one() {
    let scratch = Scratch::new("exit1");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_engine(&mut i, "bash", "-c 'exit 1' #");
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("no matches"),
        "exit code 1 must be reported as a normal \"no matches\" outcome, \
         not folded into a generic finished/abnormal message: {:?}",
        echo
    );
    assert!(
        !echo.contains("abnormally"),
        "exit code 1 must NOT be reported as abnormal: {:?}",
        echo
    );
}

// --- F1c. A genuinely invalid regex (real rg exit 2) is reported as an
// --- abnormal exit, distinct from both 0 and 1. Requires real rg -- a
// --- fake generator can't misparse a regex it never actually reads. ---

#[test]
fn search_regexp_reports_abnormal_exit_for_invalid_regex_if_rg_available() {
    if !has_rg() {
        eprintln!(
            "search_regexp_reports_abnormal_exit_for_invalid_regex_if_rg_available: \
             skipping, `rg` not on PATH"
        );
        return;
    }
    let scratch = Scratch::new("badregex");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // Unbalanced paren -- a syntactically invalid regex real `rg' exits
    // 2 on, but which means nothing special to `-F' literal mode (this
    // is exactly why `search-project-regexp' is the command under test
    // here, not `search-project').
    do_search_regexp(&mut i, &ed, "foo(bar");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("abnormally"),
        "an invalid regex must be reported as an abnormal exit, not a \
         plain \"finished\" message: {:?}",
        echo
    );
}

// --- F3. An empty pattern is rejected outright -- no process started. --

#[test]
fn search_project_rejects_empty_pattern_without_starting_a_process() {
    let scratch = Scratch::new("emptypattern");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // Repro before this fix: submitting the prompt with an empty string
    // (just pressing RET, typing nothing) built `rg ... -- ''', which
    // matched every line of every file under ROOT and raced straight
    // into `search-max-results' with no indication anything unusual had
    // even been asked for.
    do_search(&mut i, &ed, "");
    assert_eq!(
        run(&mut i, "search--procs"),
        "nil",
        "an empty pattern must never start a search process"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("empty"),
        "expected an explicit empty-pattern rejection message: {:?}",
        echo
    );
}

// --- F5. search-max-lines-per-tick <= 0 must be clamped, not hang. -----

#[test]
fn search_max_lines_per_tick_zero_is_clamped_and_does_not_hang() {
    let scratch = Scratch::new("budgetzero");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "f.v", "placeholder\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // Repro before this fix: with the budget at 0, `search--drain-
    // queue''s own `while' condition (`< n budget') is false on its
    // very first check every tick, QUEUE never drains even one line no
    // matter how many idle ticks run, the job's entry never leaves
    // `search--procs', and `has_async_work' reports async work forever
    // -- `pump_until' below would time out, not just run slowly.
    run(&mut i, "(setq search-max-lines-per-tick 0)");
    set_fake_printf_engine(&mut i, &["f.v:1:1:one", "f.v:1:1:two"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(
        ok,
        "search-max-lines-per-tick=0 must be clamped to at least 1, not hang forever"
    );
    assert_eq!(run(&mut i, "(length search--results)"), "2");
}

// --- F6a. The final line, with NO trailing newline, still gets flushed
// --- and inserted when the process exits -- every pre-existing fake
// --- engine's last line ended in `\n', so this path had zero coverage.

#[test]
fn search_flushes_final_line_with_no_trailing_newline_at_process_exit() {
    let scratch = Scratch::new("noeolflush");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "one\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // No trailing `\n' at all -- the process exits immediately after.
    set_fake_engine(&mut i, "bash", "-c 'printf \"a.v:1:1:noeol\"' #");
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    assert_eq!(
        search_buffer_string(&mut i),
        "\"a.v:1:1:noeol\\n\"",
        "the final line must still be flushed and inserted (with this \
         file's own trailing newline) even though the engine itself \
         never wrote one"
    );
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "the flushed final line must still parse into a result"
    );
}

// --- F6b. A line shaped exactly like FILE:LINE:COL: but naming a
// --- nonexistent file is displayed but not jumpable -- the pre-existing
// --- "unparseable line" test used a banner with no FILE:LINE:COL shape
// --- at all, which only exercises the regexp-mismatch path, never the
// --- SEPARATE `file-exists-p' guard. ------------------------------------

#[test]
fn search_line_shaped_like_a_result_but_naming_a_nonexistent_file_is_not_jumpable() {
    let scratch = Scratch::new("nonexistentfile");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "real.v", "hit\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(
        &mut i,
        &[
            "nosuchfile.v:1:1:shaped-like-a-result",
            "real.v:1:1:actual-hit",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    let content = search_buffer_string(&mut i);
    assert!(
        content.contains("nosuchfile.v:1:1:shaped-like-a-result"),
        "the well-shaped-but-nonexistent-file line must still be displayed: {}",
        content
    );
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "only the line naming a REAL file should be jumpable"
    );
    let file = run(&mut i, "(aref (car search--results) 0)");
    assert!(
        file.ends_with("real.v\""),
        "the one jumpable result must be real.v: {}",
        file
    );
}

// --- F6c. `search--shell-quote' protects a PATTERN containing shell
// --- metacharacters (single quote, whitespace, `;', backtick, `$') both
// --- from injection AND from corruption -- every pre-existing test
// --- used a trivial one-word pattern (`x'/`y'/`needle'). ---------------

#[test]
fn search_shell_quote_protects_special_characters_from_injection() {
    let scratch = Scratch::new("shellquote");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // A fake "echo" engine: `$1' is PATTERN exactly as `search--shell-
    // quote' placed it on the command line -- `_' is the conventional
    // bash `$0' placeholder, so the (shell-quoted) PATTERN `search--
    // start' appends lands at `$1'.
    set_fake_engine(&mut i, "bash", "-c 'printf \"echo.v:1:1:%s\\n\" \"$1\"' _");
    let pattern = "it's a test; `touch INJECTED`; $(touch INJECTED2)";
    do_search(&mut i, &ed, pattern);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    let expected_line = format!("echo.v:1:1:{}", pattern);
    let content = search_buffer_string(&mut i);
    assert!(
        content.contains(&expected_line),
        "PATTERN must reach the engine byte-for-byte as a single \
         argument: expected {:?} inside {}",
        expected_line,
        content
    );
    assert!(
        !scratch.join("INJECTED").exists(),
        "backtick command substitution embedded in PATTERN must NOT have executed"
    );
    assert!(
        !scratch.join("INJECTED2").exists(),
        "$(...) command substitution embedded in PATTERN must NOT have executed"
    );
}

// --- F7a. Default (`search-project', literal) mode does NOT treat a
// --- Verilog bus width `[7:0]' as a regex bracket expression. ----------

#[test]
fn search_project_literal_mode_does_not_treat_bus_width_as_char_class_if_rg_available() {
    if !has_rg() {
        eprintln!(
            "search_project_literal_mode_does_not_treat_bus_width_as_char_class_if_rg_available: \
             skipping, `rg` not on PATH"
        );
        return;
    }
    let scratch = Scratch::new("literalbus");
    write(&scratch, "main.sv", "// nothing\n");
    // Line 1 contains the literal text `[7:0]'; line 2 contains none of
    // it but DOES contain a lone `7' -- `[7:0]' misread as a regex
    // bracket expression (chars `7'/`:'/`0', not a range) would ALSO
    // match line 2.
    write(&scratch, "bus.v", "input [7:0] data;\nassign flag = 7;\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    do_search(&mut i, &ed, "[7:0]");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "literal search never finished");
    let lines = run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)");
    assert_eq!(
        lines, "(1)",
        "default literal mode must match ONLY the exact bus-width text \
         on line 1, never line 2's lone digit 7: {}",
        lines
    );
}

// --- F7b. Explicit `search-project-regexp' DOES treat `[7:0]' as a
// --- regex bracket expression -- the opt-in counterpart to F7a. --------

#[test]
fn search_project_regexp_mode_treats_bracket_expression_as_regex_if_rg_available() {
    if !has_rg() {
        eprintln!(
            "search_project_regexp_mode_treats_bracket_expression_as_regex_if_rg_available: \
             skipping, `rg` not on PATH"
        );
        return;
    }
    let scratch = Scratch::new("regexbus");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "bus.v", "input [7:0] data;\nassign flag = 7;\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    do_search_regexp(&mut i, &ed, "[7:0]");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "regexp search never finished");
    let lines = run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)");
    assert!(
        lines.contains('2'),
        "regexp mode must treat `[7:0]' as a bracket expression matching \
         the lone digit `7' on line 2 too, proving it was actually run \
         as a regex: {}",
        lines
    );
}

// =========================================================================
// Fix round R2 (coordinator decision): the whole search family moved
// from two flat keys (`C-c s'/`C-c S') to a `C-c s' PREFIX -- `C-c s s'
// (search-project), `C-c s r' (search-project-regexp), `C-c s a'
// (search-again) -- mirroring `C-c l ...' (the LSP family). `C-c s'
// itself must NEVER be bound directly to a command again: `lookup_in'
// (commands.rs) stops dispatch the instant a key sequence resolves to
// `Lookup::Command', so a direct `C-c s' -> command binding would make
// `C-c s r'/`C-c s s'/`C-c s a' permanently unreachable no matter what
// they're bound to (see `simple.el''s own comment at these bindings for
// the exact mechanism, and `help--shadowed-p''s doc comment above that).
// =========================================================================

#[test]
fn search_c_c_s_is_a_prefix_not_a_command() {
    let (mut i, _ed) = setup();
    // `(command ...)' here (instead of `(prefix ...)') is exactly the
    // regression this whole fix round exists to prevent: it would mean
    // `C-c s' was bound straight to a command again, silently making
    // every longer `C-c s ...' sequence below unreachable.
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?s))"),
        "(prefix nil global)"
    );
}

#[test]
fn search_c_c_s_s_reaches_search_project() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?s ?s))"),
        "(command search-project global)"
    );
}

#[test]
fn search_c_c_s_r_key_reaches_search_project_regexp() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?s ?r))"),
        "(command search-project-regexp global)"
    );
}

#[test]
fn search_c_c_s_a_reaches_search_again() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?s ?a))"),
        "(command search-again global)"
    );
}

// --- A real keypress sequence, not just `lookup-key', actually reaches
// --- `search-project-regexp' and opens its own "Search (regexp) for:"
// --- prompt (proven the same way `do_search_command' proves it for
// --- `M-x': a second minibuffer opens). ---------------------------------

#[test]
fn search_pressing_c_c_s_r_opens_search_project_regexp_prompt() {
    let scratch = Scratch::new("keyregexp");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // `true' is a real, always-present no-op command -- avoids both a
    // real `rg' dependency and leaving an orphaned search process
    // running past this test (see below, this test pumps to completion).
    set_fake_engine(&mut i, "true", "");
    feed_keys(&mut i, &ed, "C-c s r").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "C-c s r must reach search-project-regexp and open its prompt"
    );
    type_str(&mut i, &ed, "x");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "search--last-regexp-p"),
        "t",
        "the C-c s r path must have gone through search-project-regexp \
         (regexp-p = t), not search-project"
    );
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
}

// =========================================================================
// Fix round R3 (mutation testing, main conversation): reverting
// `evil-emacs-state-modes' (evil.el) to NOT include `search-mode' left
// `search_n_p_navigate_cyclically' PASSING -- that test's real `n'
// keypress never goes through evil normal-state at all (`setup' here
// never calls `(evil-mode 1)'), so the entry this milestone added to
// `evil-emacs-state-modes' had zero test coverage. T1 mirrors
// `compile_tests.rs''s `compilation_mode_is_an_evil_emacs_state_mode'
// (static list membership); T2 is the behavioral test that actually
// drives evil-mode and a real keypress -- modeled on `help_tests.rs''s
// `describe_bindings_q_reachable_under_evil_normal_state', the existing
// precedent for "real evil-mode 1, buffer's own local key must still
// win over evil normal-state's same key".
// =========================================================================

// --- T1. search-mode is in evil-emacs-state-modes (static list check) --

#[test]
fn search_mode_is_an_evil_emacs_state_mode() {
    let (mut i, _ed) = setup();
    let member = run(&mut i, "(memq 'search-mode evil-emacs-state-modes)");
    assert_ne!(
        member, "nil",
        "search-mode must be in evil-emacs-state-modes"
    );
}

// --- T2. With evil-mode actually turned on, a real `n' keypress in
// --- `*search*' still reaches `search-next-result', not evil normal-
// --- state's own `n' (`evil-search-next', evil.el:3524). ---------------

#[test]
fn search_n_still_navigates_results_with_evil_mode_enabled() {
    let scratch = Scratch::new("evilnav");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "line1\nline2\n");
    write(&scratch, "b.v", "x\ny\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);

    set_fake_printf_engine(&mut i, &["a.v:1:1:one", "b.v:1:1:two"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    assert_eq!(
        run(&mut i, "evil--state"),
        "emacs",
        "*search* must start in evil's `emacs' state (evil-emacs-state-modes) \
         so its own local `n'/`p'/RET' bindings aren't shadowed by the \
         normal-state emulation keymap"
    );

    // If `search-mode' were NOT on `evil-emacs-state-modes', this `n'
    // would instead be `evil-search-next' (evil.el's normal-state map),
    // which has no notion of `search--results' at all and would not
    // move point to a different buffer -- `buffer-name' would still be
    // "*search*" here, not "a.v".
    feed_keys(&mut i, &ed, "n").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"a.v\"",
        "n under evil's normal state must still reach search-next-result, \
         not get intercepted by evil's normal-state `n' (evil-search-next)"
    );
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");
}

// =========================================================================
// Fix round R3 (tail review, main conversation): G1 -- `search-again's
// own function body had ZERO test coverage across all 29 pre-existing
// tests (`search_c_c_s_a_reaches_search_again' only proves the KEY
// resolves to the symbol, never actually calls it). This round's own
// F2 addition (`search--last-regexp-p' literal/regexp mode carry-
// through) was entirely riding on that untested body.
// =========================================================================

// --- G1a. `search-again' actually reruns the LAST pattern against the
// --- LAST root -- not whatever the CURRENT buffer's root would be. -----

#[test]
fn search_again_reruns_last_pattern_and_root() {
    let root_a = Scratch::new("againroota");
    write(&root_a, "main.sv", "// nothing\n");
    // `marker.v' exists ONLY under root A -- if `search-again' wrongly
    // recomputed the root from whatever buffer is CURRENT (instead of
    // reusing `search--last-root'), this relative FILE would fail
    // `file-exists-p' after switching to a buffer under root B below,
    // and `search--results' would stay empty even though the same text
    // is still printed into the buffer either way.
    write(&root_a, "marker.v", "hit\n");
    let root_b = Scratch::new("againrootb");
    write(&root_b, "main2.sv", "// nothing\n");

    let (mut i, ed) = setup();
    visit(&mut i, &root_a.join("main.sv"));
    // Echo-back engine: `$1' is PATTERN exactly as `search--shell-quote'
    // placed it -- lets this test observe which PATTERN actually ran,
    // not just canned output (same technique as `search_shell_quote_
    // protects_special_characters_from_injection').
    set_fake_engine(
        &mut i,
        "bash",
        "-c 'printf \"marker.v:1:1:%s\\n\" \"$1\"' _",
    );
    do_search(&mut i, &ed, "first-pattern");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "first search never finished");
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "first search must resolve marker.v under root A"
    );

    // Switch to a buffer under a COMPLETELY DIFFERENT root before
    // rerunning -- `search--default-root' would compute root B here if
    // `search-again' mistakenly called it instead of reusing state.
    visit(&mut i, &root_b.join("main2.sv"));

    do_search_again(&mut i, &ed);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search-again job never finished");

    assert_eq!(
        search_buffer_string(&mut i),
        "\"marker.v:1:1:first-pattern\\n\"",
        "search-again must rerun the SAME pattern (\"first-pattern\"), \
         not a fresh/empty one"
    );
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "1",
        "search-again must still resolve marker.v against root A (the \
         LAST root), not root B (the CURRENT buffer's root) -- 0 here \
         would mean it recomputed the root instead of reusing it"
    );
}

// --- G1b/c. `search-again' reuses the literal-vs-regexp MODE from the
// --- invocation it's repeating, in both directions. Requires real `rg'
// --- -- the mode only has an observable effect via a real regex engine
// --- (see F7's own `[7:0]' bracket-expression fixture, reused here). ---

#[test]
fn search_again_reuses_regexp_mode_if_rg_available() {
    if !has_rg() {
        eprintln!("search_again_reuses_regexp_mode_if_rg_available: skipping, `rg` not on PATH");
        return;
    }
    let scratch = Scratch::new("againregexp");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "bus.v", "input [7:0] data;\nassign flag = 7;\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));

    do_search_regexp(&mut i, &ed, "[7:0]");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "regexp search never finished");
    let lines = run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)");
    assert!(
        lines.contains('2'),
        "sanity check: regexp mode must match line 2's lone digit: {}",
        lines
    );

    do_search_again(&mut i, &ed);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search-again job never finished");
    let lines = run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)");
    assert!(
        lines.contains('2'),
        "search-again must rerun in REGEXP mode, not fall back to \
         literal -- line 2 (only matched under regexp) must still \
         appear: {}",
        lines
    );
}

#[test]
fn search_again_reuses_literal_mode_if_rg_available() {
    if !has_rg() {
        eprintln!("search_again_reuses_literal_mode_if_rg_available: skipping, `rg` not on PATH");
        return;
    }
    let scratch = Scratch::new("againliteral");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "bus.v", "input [7:0] data;\nassign flag = 7;\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));

    do_search(&mut i, &ed, "[7:0]");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "literal search never finished");
    assert_eq!(
        run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)"),
        "(1)",
        "sanity check: literal mode must match ONLY line 1"
    );

    do_search_again(&mut i, &ed);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search-again job never finished");
    assert_eq!(
        run(&mut i, "(mapcar (lambda (e) (aref e 1)) search--results)"),
        "(1)",
        "search-again must rerun in LITERAL mode, not switch to regexp \
         -- line 2 (only matched under regexp) must NOT appear"
    );
}

// --- G1d. search-again before any search has ever run. -----------------

#[test]
fn search_again_without_prior_search_reports_message_and_starts_nothing() {
    let (mut i, ed) = setup();
    do_search_again(&mut i, &ed);
    assert_eq!(
        run(&mut i, "search--procs"),
        "nil",
        "search-again with no prior search must never start a process"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("No previous search"),
        "expected an explicit \"no previous search\" message: {:?}",
        echo
    );
}

// --- G3. `search-arguments-literal'/`search-arguments-regexp' must
// --- share every flag except `--fixed-strings' -- a future shared flag
// --- (e.g. `--hidden') added to only one of the two long, near-
// --- identical strings would silently drift them apart otherwise. -----

#[test]
fn search_arguments_literal_and_regexp_share_the_same_flags_except_fixed_strings() {
    let (mut i, _ed) = setup();
    let literal_str = run(&mut i, "search-arguments-literal");
    let regexp_str = run(&mut i, "search-arguments-regexp");
    let literal_unquoted = literal_str.trim_matches('"');
    let regexp_unquoted = regexp_str.trim_matches('"');

    let mut literal_flags: Vec<&str> = literal_unquoted
        .split_whitespace()
        .filter(|f| *f != "--fixed-strings")
        .collect();
    let mut regexp_flags: Vec<&str> = regexp_unquoted.split_whitespace().collect();
    literal_flags.sort_unstable();
    regexp_flags.sort_unstable();

    assert_eq!(
        literal_flags, regexp_flags,
        "search-arguments-literal (minus --fixed-strings) and \
         search-arguments-regexp must have identical flag sets -- \
         literal={:?} regexp={:?}",
        literal_str, regexp_str
    );
    assert!(
        literal_unquoted.contains("--fixed-strings"),
        "sanity check: literal must actually contain --fixed-strings: {}",
        literal_str
    );
}

// =========================================================================
// M83: editable search results (wgrep-level). `C-x C-q' enters `search-
// edit-mode', `C-c C-c' applies, `C-c C-k' discards. See search.el's
// own header ("--- M83: editable search results (wgrep-level) ---")
// for the full D1-D11 design this suite exercises.
// =========================================================================

// --- 1. One edited line -> disk content correct -------------------------

#[test]
fn search_edit_apply_writes_one_changed_line_to_disk() {
    let scratch = Scratch::new("edit1");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "line1\nOLDTEXT\nline3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLDTEXT"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEWTEXT");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "line1\nNEWTEXT\nline3\n",
        "the edited line must land on disk, at the same line, unchanged elsewhere"
    );
}

// --- 2. Edits spanning multiple files all apply -------------------------

#[test]
fn search_edit_apply_writes_across_multiple_files() {
    let scratch = Scratch::new("edit2");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD_A\nl3\n");
    write(&scratch, "b.v", "l1\nOLD_B\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD_A", "b.v:2:1:OLD_B"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW_A");
    set_search_line(&mut i, 1, "b.v:2:1:NEW_B");
    apply_edits(&mut i, &ed);

    assert_eq!(read_file(&scratch.join("a.v")), "l1\nNEW_A\nl3\n");
    assert_eq!(read_file(&scratch.join("b.v")), "l1\nNEW_B\nl3\n");
}

// --- 3. Multiple edits to the SAME file all apply correctly -------------
// --- (D5: descending-line application -- see this test's own note on ---
// --- what it does and doesn't prove about that specific mechanism). ----

#[test]
fn search_edit_apply_writes_multiple_lines_in_the_same_file() {
    // NOTE (honest limitation of this test): this implementation edits
    // each target line via `(goto-char (point-min)) (forward-line ...)'
    // computed FRESH for every edit, and never inserts/deletes a
    // newline character -- so line numbers of OTHER lines in the same
    // file never shift regardless of processing order. This test proves
    // the OUTCOME D5 asks for (multiple same-file edits all land
    // correctly), not that descending order specifically is what makes
    // it work -- with this particular line-based (not offset-based)
    // editing strategy, ascending order would produce the identical
    // result. D5's sort is still implemented exactly as specified.
    let scratch = Scratch::new("edit3");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD2\nOLD3\nl4\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD2", "a.v:3:1:OLD3"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW2");
    set_search_line(&mut i, 1, "a.v:3:1:NEW3");
    apply_edits(&mut i, &ed);

    assert_eq!(read_file(&scratch.join("a.v")), "l1\nNEW2\nNEW3\nl4\n");
}

// --- 4. Untouched lines are byte-for-byte unchanged ----------------------

#[test]
fn search_edit_apply_leaves_untouched_lines_byte_identical() {
    let scratch = Scratch::new("edit4");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "keep1\nOLD\nkeep3\nkeep4\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "keep1\nNEW\nkeep3\nkeep4\n",
        "lines 1/3/4 must be byte-identical to the original; only line 2 changes"
    );
}

// --- 5. Target line changed externally since the search -> skipped, ----
// --- reported, other edits still apply. ----------------------------------

#[test]
fn search_edit_apply_skips_line_changed_externally_since_search() {
    let scratch = Scratch::new("edit5");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD_A\nl3\n");
    write(&scratch, "b.v", "l1\nOLD_B\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD_A", "b.v:2:1:OLD_B"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW_A"); // will conflict
    set_search_line(&mut i, 1, "b.v:2:1:NEW_B"); // will NOT conflict

    // External modification: NOT via any buffer this editor knows
    // about -- a real filesystem write behind this editor's back,
    // landing on the exact line `a.v''s edit targets.
    std::fs::write(scratch.join("a.v"), "l1\nTAMPERED\nl3\n").unwrap();

    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nTAMPERED\nl3\n",
        "a.v's conflicting edit must be skipped, not overwrite the external change"
    );
    assert_eq!(
        read_file(&scratch.join("b.v")),
        "l1\nNEW_B\nl3\n",
        "b.v's unrelated edit must still apply"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("skipped"),
        "report must mention the skip: {:?}",
        echo
    );
}

// --- 6. Target already open with unrelated unsaved edits -> buffer and -
// --- disk never diverge (the CORE reason D1 requires find-file+save- ---
// --- buffer instead of any direct-to-disk write). ------------------------

#[test]
fn search_edit_apply_preserves_unrelated_unsaved_edits_in_already_open_buffer() {
    let scratch = Scratch::new("edit6");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    // Open a.v directly and make an UNRELATED, UNSAVED edit (append a
    // whole new line at the end) -- this is exactly the scenario a
    // direct-to-disk write (bypassing the buffer) would silently lose.
    visit(&mut i, &scratch.join("a.v"));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"EXTRA\\n\")");
    assert_eq!(run(&mut i, "(buffer-modified-p)"), "t");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW");
    apply_edits(&mut i, &ed);

    let disk = read_file(&scratch.join("a.v"));
    assert!(
        disk.contains("NEW"),
        "the intended edit must be on disk: {:?}",
        disk
    );
    assert!(
        disk.contains("EXTRA"),
        "the user's own prior unsaved edit must NOT be lost: {:?}",
        disk
    );
    assert_eq!(disk, "l1\nNEW\nl3\nEXTRA\n");
}

// --- 7. A corrupted header is reported, not silently dropped -----------

#[test]
fn search_edit_apply_reports_broken_prefix_without_silently_dropping() {
    let scratch = Scratch::new("edit7");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    // Overwrite the WHOLE line, header included -- no longer a
    // FILE:LINE:COL: shape at all.
    set_search_line(&mut i, 0, "totally not a header anymore");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nOLD\nl3\n",
        "a.v must be untouched -- nothing was applicable for the corrupted line"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("corrupted") || echo.contains("skipped"),
        "report must mention the broken prefix: {:?}",
        echo
    );
}

// --- 8. C-c C-k discards: disk untouched, buffer read-only again --------

#[test]
fn search_edit_discard_reverts_buffer_and_touches_no_files() {
    let scratch = Scratch::new("edit8");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    let before = search_buffer_string(&mut i);
    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW");
    discard_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nOLD\nl3\n",
        "discard must never touch any file on disk"
    );
    assert_eq!(
        search_buffer_string(&mut i),
        before,
        "discard must restore *search* to exactly its pre-edit text"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (buffer-read-only-p))"
        ),
        "t",
        "discard must return *search* to the read-only view state"
    );
}

// --- 9. Entering edit mode while a search is still running is refused --

#[test]
fn search_edit_mode_refused_while_search_running() {
    let scratch = Scratch::new("edit9");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_engine(&mut i, "bash", "-c 'sleep 30' #");
    do_search(&mut i, &ed, "x");
    let started = pump_until(&mut i, Duration::from_secs(2), |i| {
        run(i, "search--procs") != "nil"
    });
    assert!(started, "search never registered a running process");

    enter_edit_mode(&mut i, &ed);
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("running"),
        "expected a rejection message mentioning the search is still running: {:?}",
        echo
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (buffer-read-only-p))"
        ),
        "t",
        "*search* must remain read-only -- edit mode must not have been entered"
    );

    // Cleanup: don't leave a 30s `sleep' orphan running past this test.
    run(
        &mut i,
        "(dolist (e search--procs) (shell-process-kill (aref e 0)))",
    );
}

// --- 10. Applying produces a SINGLE undo group per target buffer -------
// --- G2 (fix round R3): the ORIGINAL version of this test used a
// --- freshly `find-file'd buffer with an EMPTY undo history, where
// --- `undo-boundary' is a documented no-op (buffer.rs) -- deleting both
// --- of F1's `undo-boundary' calls would not have changed this test's
// --- outcome at all. Rewritten to actually construct the scenario F1
// --- exists for: the target buffer ALREADY open, with an UNTERMINATED
// --- prior edit (no trailing boundary) -- exactly what a real user
// --- mid-typing looks like.

#[test]
fn search_edit_apply_produces_a_single_undo_group_per_target_buffer() {
    let scratch = Scratch::new("edit10");
    write(&scratch, "main.sv", "// nothing\n");
    let original = "l1\nOLD1\nOLD2\nl4\n";
    write(&scratch, "a.v", original);
    let (mut i, ed) = setup();

    // a.v is ALREADY open, with text typed directly (a raw `insert'
    // call, not a real command dispatch -- so, like a real user's
    // still-in-progress edit, it has NO trailing undo boundary of its
    // own yet).
    visit(&mut i, &scratch.join("a.v"));
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"USERTYPED\\n\")");
    let with_user_edit = "l1\nOLD1\nOLD2\nl4\nUSERTYPED\n";
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        format!("{:?}", with_user_edit)
    );

    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD1", "a.v:3:1:OLD2"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW1");
    set_search_line(&mut i, 1, "a.v:3:1:NEW2");
    apply_edits(&mut i, &ed);
    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nNEW1\nNEW2\nl4\nUSERTYPED\n"
    );

    // ONE undo must revert ONLY the batch write -- F1's own boundaries
    // must have closed the earlier "USERTYPED" group off first, so this
    // undo call stops there and never touches it.
    run(&mut i, "(with-current-buffer \"a.v\" (undo))");
    assert_eq!(
        run(&mut i, "(with-current-buffer \"a.v\" (buffer-string))"),
        format!("{:?}", with_user_edit),
        "a single undo must revert ONLY the batch write, leaving the \
         user's own prior (unterminated) edit intact"
    );
}

// --- G2 tail review (mutation-found gap): the CLOSING boundary (AFTER
// --- the dolist, BEFORE `save-buffer') has no test of its own -- the
// --- test above only exercises the OPENING boundary (a PRIOR,
// --- unterminated edit merging INTO the batch). This is the mirror
// --- direction: text typed AFTER apply must not merge BACKWARD into
// --- the batch either.

#[test]
fn search_edit_apply_closing_boundary_protects_edits_typed_after_apply() {
    let scratch = Scratch::new("g2c");
    write(&scratch, "main.sv", "// nothing\n");
    let original = "l1\nOLD1\nOLD2\nl4\n";
    write(&scratch, "a.v", original);
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD1", "a.v:3:1:OLD2"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW1");
    set_search_line(&mut i, 1, "a.v:3:1:NEW2");
    apply_edits(&mut i, &ed);
    let after_apply = "l1\nNEW1\nNEW2\nl4\n";
    assert_eq!(read_file(&scratch.join("a.v")), after_apply);

    // AFTER apply, the user keeps typing into the SAME buffer -- a raw
    // `insert' call, no trailing boundary of its own yet, exactly the
    // shape a real still-in-progress edit has.
    run(
        &mut i,
        "(with-current-buffer \"a.v\" (goto-char (point-max)) (insert \"AFTERTYPED\\n\"))",
    );
    let with_after_edit = "l1\nNEW1\nNEW2\nl4\nAFTERTYPED\n";
    assert_eq!(
        run(&mut i, "(with-current-buffer \"a.v\" (buffer-string))"),
        format!("{:?}", with_after_edit)
    );

    // ONE undo must revert ONLY the "AFTERTYPED" text -- the CLOSING
    // boundary (search.el, right after the dolist) must have already
    // sealed the batch write off, so this undo stops there and never
    // reaches back into the batch.
    run(&mut i, "(with-current-buffer \"a.v\" (undo))");
    assert_eq!(
        run(&mut i, "(with-current-buffer \"a.v\" (buffer-string))"),
        format!("{:?}", after_apply),
        "a single undo must revert ONLY the text typed after apply, \
         leaving the batch write intact"
    );
}

// --- G1. save-buffer failing after edits already landed in memory ------
// --- must roll those edits back, not leave the buffer silently dirty --
// --- with a batch that was just reported as "not applied". -------------

#[test]
fn search_edit_apply_rolls_back_in_memory_edit_when_save_buffer_fails() {
    let scratch = Scratch::new("g1");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    // Pre-open a.v so `find-file' inside apply REUSES it (its disk_state
    // baseline is captured HERE, before the external tamper below) --
    // if `find-file' instead read the file fresh, the baseline would
    // already reflect the tamper and this scenario couldn't happen.
    visit(&mut i, &scratch.join("a.v"));

    // External modification behind this editor's back, to a DIFFERENT
    // line than the one about to be edited -- D4's own per-line check
    // on line 2 still passes (line 2 itself is untouched here), but the
    // file's mtime/size now disagree with the buffer's stale baseline,
    // so `save-buffer''s OWN whole-file conflict guard refuses the
    // write AFTER the in-memory edit has already been made.
    std::fs::write(scratch.join("a.v"), "l1\nOLD\nEXTERNAL\n").unwrap();

    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nOLD\nEXTERNAL\n",
        "save-buffer must have refused -- disk must be untouched"
    );
    // G1's own point: the in-memory edit must be ROLLED BACK, not left
    // sitting dirty -- otherwise a LATER, unrelated save (the user's own
    // C-x C-s, or a retried apply) would silently persist it anyway.
    assert_eq!(
        run(&mut i, "(with-current-buffer \"a.v\" (buffer-string))"),
        "\"l1\\nOLD\\nl3\\n\"",
        "the in-memory edit must be rolled back after save-buffer fails"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("skipped"),
        "report must mention the skip: {:?}",
        echo
    );
}

// --- 11. *search* is read-only outside edit mode ------------------------

#[test]
fn search_buffer_is_read_only_outside_edit_mode() {
    let scratch = Scratch::new("edit11");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "hit\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hit"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    let out = run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-max)) (insert \"x\"))",
    );
    assert!(
        out.starts_with("ERROR"),
        "insert into *search* outside edit mode must be refused: {}",
        out
    );
    assert!(
        out.contains("read-only")
            || out.contains("Read-only")
            || out.contains("Buffer is read-only"),
        "error must mention read-only: {}",
        out
    );
}

// --- 12. M82's streaming insert still works now that *search* defaults -
// --- to read-only (D8's own load-bearing `inhibit-read-only' spots). ---

#[test]
fn search_streaming_insert_still_works_after_buffer_is_read_only_by_default() {
    let scratch = Scratch::new("edit12");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "hit\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hit"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (buffer-read-only-p))"
        ),
        "t",
        "*search* must be read-only by default (D8)"
    );
    assert_eq!(search_buffer_string(&mut i), "\"a.v:1:1:hit\\n\"");
    assert_eq!(run(&mut i, "(length search--results)"), "1");
}

// --- 13. search-edit-mode: evil static list + real behavior ------------

#[test]
fn search_edit_mode_is_an_evil_emacs_state_mode() {
    let (mut i, _ed) = setup();
    let member = run(&mut i, "(memq 'search-edit-mode evil-emacs-state-modes)");
    assert_ne!(
        member, "nil",
        "search-edit-mode must be in evil-emacs-state-modes"
    );
}

#[test]
fn search_edit_mode_allows_direct_typing_with_evil_mode_enabled() {
    let scratch = Scratch::new("edit13");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);

    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    assert_eq!(
        run(&mut i, "evil--state"),
        "emacs",
        "search-edit-mode must start in evil's `emacs' state so ordinary \
         typing self-inserts instead of being read as vim motions/commands"
    );

    // Move point to the end of the buffer and type a plain character.
    // F8 (fix round R2, tail review -- CORRECTED comment, the previous
    // version's claim below was WRONG): `*search*' being on `*search*'
    // is itself buffer-local and set ONCE per buffer, the first time
    // evil ever looks at it (`evil--maybe-init-current-buffer', guarded
    // by `(not (local-variable-p 'evil--state))', evil.el) -- by the
    // time this test reaches `search-edit-mode', THIS buffer's
    // `evil--state' was already pinned to `emacs' back when it was
    // still plain `search-mode' (which was ALREADY on `evil-emacs-
    // state-modes' since M82). Removing `search-edit-mode' from that
    // list would therefore change NOTHING observable here -- the SAME
    // buffer object never gets re-evaluated. This assertion's real
    // value is proving typing works correctly in `search-edit-mode'
    // (self-insert, not a stray vim motion); D11's actual regression
    // guard is the static list-membership check,
    // `search_edit_mode_is_an_evil_emacs_state_mode' above -- THAT
    // test does fail if `search-edit-mode' is removed from the list,
    // because it never depends on any buffer's already-pinned state.
    run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-max)))",
    );
    let before = search_buffer_string(&mut i);
    feed_keys(&mut i, &ed, "x").unwrap();
    let after = search_buffer_string(&mut i);
    // Buffer ended in a trailing newline (the streamed result line's own
    // "\n"), so a self-inserted `x' at point-max lands right after it,
    // with no newline of its own following -- the prin1'd string simply
    // grows by one `x' right before its closing quote.
    let expected = format!("{}x\"", &before[..before.len() - 1]);
    assert_eq!(
        after, expected,
        "expected `x' to have been self-inserted at the end of the buffer \
         (not e.g. evil-delete-char firing instead, if normal state were \
         active): before={:?} after={:?}",
        before, after
    );
}

// --- 14. Partial failure (one file read-only) does not abort the batch -

#[test]
fn search_edit_apply_partial_failure_does_not_abort_batch() {
    let scratch = Scratch::new("edit14");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD_A\nl3\n");
    write(&scratch, "b.v", "l1\nOLD_B\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));

    // b.v is already open and made read-only -- its write must fail
    // without aborting a.v's.
    visit(&mut i, &scratch.join("b.v"));
    run(&mut i, "(set-buffer-read-only t)");
    visit(&mut i, &scratch.join("main.sv"));

    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD_A", "b.v:2:1:OLD_B"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW_A");
    set_search_line(&mut i, 1, "b.v:2:1:NEW_B");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nNEW_A\nl3\n",
        "a.v's edit must still apply even though b.v's failed"
    );
    assert_eq!(
        read_file(&scratch.join("b.v")),
        "l1\nOLD_B\nl3\n",
        "b.v must be untouched -- it was read-only"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Applied 1"),
        "report must show exactly 1 line applied: {:?}",
        echo
    );
}

// =========================================================================
// Fix round R2 (tail review, main conversation): F2/F3/F6 below. F4/F5/F7
// could NOT be meaningfully tested this round -- `search-edit-apply's
// actual disk-write path is blocked by a newly-surfaced, exhaustively
// isolated `crates/elisp' compiler defect (reported separately, out of
// this file's scope to fix): calling `search-edit-apply'/`search--edit-
// apply-to-file' AS LOADED FROM THIS FILE throws "Symbol's value as
// variable is void: n" -- `n' names nothing in either function's
// current body. The identical logic, `eval'd as a dynamically-defined
// clone in the SAME already-fully-loaded interpreter, runs correctly
// every time. See search.el's own `search-edit-apply' docstring
// ("BLOCKING ISSUE") for the full isolation trail.
// =========================================================================

// --- F2. search-project/search-project-regexp/search-again all refuse
// --- to start a new search while `*search*' is mid-edit. ---------------

#[test]
fn search_project_refuses_while_editing() {
    let scratch = Scratch::new("f2a");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    enter_edit_mode(&mut i, &ed);

    let before = search_buffer_string(&mut i);
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-project");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "search-project must be refused BEFORE ever opening the pattern prompt"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("being edited"),
        "expected an explicit mid-edit rejection message: {:?}",
        echo
    );
    assert_eq!(
        search_buffer_string(&mut i),
        before,
        "*search* must be untouched -- the new search must never have started"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-edit-mode",
        "*search* must still be mid-edit, not reset"
    );
}

#[test]
fn search_project_regexp_refuses_while_editing() {
    let scratch = Scratch::new("f2b");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    enter_edit_mode(&mut i, &ed);

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-project-regexp");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "search-project-regexp must be refused BEFORE ever opening the pattern prompt"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("being edited"),
        "expected an explicit mid-edit rejection message: {:?}",
        echo
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-edit-mode"
    );
}

#[test]
fn search_again_refuses_while_editing() {
    let scratch = Scratch::new("f2c");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    enter_edit_mode(&mut i, &ed);

    do_search_again(&mut i, &ed);
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("being edited"),
        "expected an explicit mid-edit rejection message, not \"No previous search\": {:?}",
        echo
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-edit-mode"
    );
}

// --- F3. M-x can call any of the three search-edit-* commands from ------
// --- anywhere / any state -- each must check its own preconditions -----
// --- instead of trusting the local keymap to have enforced them. -------

#[test]
fn search_edit_discard_via_m_x_while_not_editing_does_not_erase_results() {
    let scratch = Scratch::new("f3a");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    // NOT in edit mode -- *search* is still the plain read-only view.

    let before = search_buffer_string(&mut i);
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-edit-discard");
    feed_keys(&mut i, &ed, "RET").unwrap();

    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Not currently editing"),
        "expected a rejection message: {:?}",
        echo
    );
    assert_eq!(
        search_buffer_string(&mut i),
        before,
        "real search results must NOT be erased by a discard while not editing"
    );
}

#[test]
fn search_edit_apply_via_m_x_while_not_editing_is_refused() {
    let scratch = Scratch::new("f3b");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-edit-apply");
    feed_keys(&mut i, &ed, "RET").unwrap();

    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Not currently editing"),
        "expected a rejection message: {:?}",
        echo
    );
    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nOLD\nl3\n",
        "a.v must be untouched -- apply must have been refused before doing anything"
    );
}

#[test]
fn search_edit_mode_via_m_x_from_wrong_buffer_is_refused() {
    let scratch = Scratch::new("f3c");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    // Switch AWAY from *search* before invoking the command via M-x.
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-edit-mode");
    feed_keys(&mut i, &ed, "RET").unwrap();

    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Not in a *search* buffer"),
        "expected a rejection message: {:?}",
        echo
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-mode",
        "*search* must remain in the read-only view state"
    );
}

#[test]
fn search_edit_mode_twice_does_not_resnapshot() {
    let scratch = Scratch::new("f3d");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:EDITED");
    // Entering edit mode a SECOND time (already editing) must not
    // re-snapshot the NOW-edited text as the new "original" baseline --
    // that would make `search-edit-discard' restore the WRONG (already
    // modified) text instead of the true pre-edit original. `C-x C-q'
    // itself isn't bound in the EDIT keymap (only the view keymap has
    // it), so this second attempt goes through `M-x' instead, matching
    // F3's own "M-x can reach this from anywhere" concern.
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-edit-mode");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Already editing"),
        "expected a rejection message: {:?}",
        echo
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" search--edit-original-text)"
        ),
        "\"a.v:2:1:OLD\\n\"",
        "the snapshot must still be the TRUE original, not the in-progress edit"
    );
}

// --- F6. search--reset forces *search* back to the read-only view ------
// --- state even if it's somehow (still) in search-edit-mode -- a -------
// --- direct, unit-level test of `search--reset' itself: F2 above -------
// --- means the public commands can no longer reach this path, so this --
// --- calls `search--reset' directly, bypassing them on purpose. --------

#[test]
fn search_reset_forces_view_mode_even_if_search_edit_mode_was_left_installed() {
    let scratch = Scratch::new("f6");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    enter_edit_mode(&mut i, &ed);
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-edit-mode"
    );

    // Direct, unguarded call -- F2's public-command guards live in
    // search-project/search-project-regexp/search-again, NOT in
    // search--reset itself, so this reaches search--reset's own
    // defense-in-depth force-view-keymap behavior on purpose.
    run(&mut i, "(search--reset)");

    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (major-mode-internal-get))"
        ),
        "search-mode",
        "search--reset must force *search* back to the read-only view state"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" (buffer-read-only-p))"
        ),
        "t"
    );
    assert_eq!(
        run(
            &mut i,
            "(with-current-buffer \"*search*\" search--edit-original-text)"
        ),
        "nil",
        "the stale edit snapshot must be cleared too"
    );
}

// =========================================================================
// R3 (fix round R3, tail review): F5/F7 -- the two tests deferred from
// the previous fix round while a real `crates/elisp' reader bug (three
// unescaped `"' characters closing a docstring early, see search.el's
// own header history) was being chased down. Both now run against the
// row-indexed comparison logic exactly as designed in M83's own header
// (D2).
// =========================================================================

// --- F5. A whole-row deletion inside *search* (v1 does not support
// --- this, see the "v1 does not include" list) must NEVER cross-wire
// --- one result's text onto ANOTHER result's file/line coordinates --
// --- the highest-risk, previously-untested arm of the (FILE, LINE,
// --- COL) triple comparison in `search-edit-apply--do'. -----------------

#[test]
fn search_edit_apply_does_not_cross_wire_lines_when_a_search_row_is_deleted() {
    let scratch = Scratch::new("f5");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nAT_LINE2\nl3\nl4\nAT_LINE5\nl6\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:AT_LINE2", "a.v:5:1:AT_LINE5"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    // Delete the FIRST result row (and its newline) entirely from
    // *search* -- v1 does not support inserting/deleting whole rows
    // (search.el's own header), but nothing stops the user from typing
    // this anyway, and the ROW-INDEXED comparison must not silently
    // treat row 1's now-shifted-up text as if it were row 0's edit.
    run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-min)) \
         (delete-region (line-beginning-position) (progn (forward-line 1) (point))))",
    );
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nAT_LINE2\nl3\nl4\nAT_LINE5\nl6\n",
        "a.v must be completely untouched -- row 1's original text must \
         never get written to row 0's (line 2's) coordinates"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("skipped") || echo.contains("corrupted"),
        "the row shift must be reported, not silently ignored: {:?}",
        echo
    );
}

// --- G3a. FILE-only regression: same LINE+COL, different FILE -- the
// --- existing row-shift test above happens to have identical FILE and
// --- COL for both rows (only LINE differs), so removing the FILE
// --- check alone would NOT fail it. This constructs a row shift where
// --- FILE is the ONLY component that differs. ---------------------------

#[test]
fn search_edit_apply_does_not_cross_wire_files_with_same_line_and_col() {
    let scratch = Scratch::new("g3a");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nORIG_A\nl3\n");
    write(&scratch, "b.v", "l1\nORIG_B\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // SAME line (2) and SAME column (1) for both rows -- only FILE
    // differs.
    set_fake_printf_engine(&mut i, &["a.v:2:1:ORIG_A", "b.v:2:1:ORIG_B"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    // Edit row 1 (b.v's) BEFORE deleting row 0 -- so its text differs
    // from row 0's original, making a cross-write observable.
    set_search_line(&mut i, 1, "b.v:2:1:EDITED_B");
    run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-min)) \
         (delete-region (line-beginning-position) (progn (forward-line 1) (point))))",
    );
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nORIG_A\nl3\n",
        "a.v must be untouched -- b.v's edited text must never be \
         written to a.v's line 2, even though LINE and COL happen to match"
    );
}

// --- G3b. COL-only regression: same FILE+LINE, different COL -- two
// --- results on the SAME line (two matches on one line is realistic
// --- `rg' output), only COL differs. -------------------------------------

#[test]
fn search_edit_apply_does_not_cross_wire_columns_with_same_file_and_line() {
    let scratch = Scratch::new("g3b");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nSAME_LINE_TEXT\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // SAME file and SAME line (2) for both rows, matching TEXT (as a
    // real two-matches-on-one-line `rg' result would report) -- only
    // COL differs (5 vs 15).
    set_fake_printf_engine(
        &mut i,
        &["a.v:2:5:SAME_LINE_TEXT", "a.v:2:15:SAME_LINE_TEXT"],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    // Edit row 1 (col 15's) BEFORE deleting row 0 -- so its text now
    // differs from row 0's original snapshot text.
    set_search_line(&mut i, 1, "a.v:2:15:EDITED_TEXT");
    run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-min)) \
         (delete-region (line-beginning-position) (progn (forward-line 1) (point))))",
    );
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nSAME_LINE_TEXT\nl3\n",
        "a.v must be untouched -- col 15's edited text must never be \
         written using col 5's tracked original, even though FILE and \
         LINE happen to match"
    );
}

// --- F7. Same-file D4 conflict on ONE line does not block an unrelated,
// --- unconflicted edit to ANOTHER line in the SAME file -- D4's own
// --- docstring claims this ("two different lines are independent
// --- facts"); the cross-file version of this was already tested
// --- (tests 5/14), the same-file version was not. ------------------------

#[test]
fn search_edit_apply_same_file_one_line_conflict_other_line_still_applies() {
    let scratch = Scratch::new("f7");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nOLD_A\nl3\nOLD_B\nl5\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:OLD_A", "a.v:4:1:OLD_B"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:NEW_A");
    set_search_line(&mut i, 1, "a.v:4:1:NEW_B");

    // External modification: tamper with JUST line 2, behind this
    // editor's back -- line 4 is untouched.
    std::fs::write(scratch.join("a.v"), "l1\nTAMPERED\nl3\nOLD_B\nl5\n").unwrap();

    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("a.v")),
        "l1\nTAMPERED\nl3\nNEW_B\nl5\n",
        "line 2's conflicting edit must be skipped (TAMPERED preserved) \
         while line 4's unconflicted edit in the SAME file still applies, \
         via the SAME save-buffer call"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("skipped"),
        "report must mention the line-2 skip: {:?}",
        echo
    );
}

// --- G4. Applying to a file deleted since the search must never ---------
// --- recreate it -- `find-file' on a nonexistent path opens an EMPTY ---
// --- buffer rather than erroring, and if the result's ORIG-TEXT is -----
// --- itself empty (a blank line the pattern matched), D4's own --------
// --- `string=' check would otherwise consider that a match. -------------

#[test]
fn search_edit_apply_does_not_recreate_a_deleted_file() {
    let scratch = Scratch::new("g4");
    write(&scratch, "main.sv", "// nothing\n");
    // Line 2 is blank -- ORIG-TEXT for this result will be the empty
    // string, matching the empty buffer `find-file' would otherwise
    // silently hand back for a deleted path.
    write(&scratch, "a.v", "l1\n\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:2:1:"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    // Delete the file entirely, behind this editor's back, after the
    // search already ran and recorded this result.
    std::fs::remove_file(scratch.join("a.v")).unwrap();
    assert!(!scratch.join("a.v").exists());

    enter_edit_mode(&mut i, &ed);
    set_search_line(&mut i, 0, "a.v:2:1:RESURRECTED");
    apply_edits(&mut i, &ed);

    assert!(
        !scratch.join("a.v").exists(),
        "a deleted file must NOT be recreated by applying an edit to it"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("no longer exists"),
        "report must mention the file is gone: {:?}",
        echo
    );
}

// ============================================================
// M85 D1: `minibuffer-input-changed-hook` -- general-purpose
// notification, not tied to search at all. See simple.el's own doc
// comment for why this needed to exist (the M21 completion-panel
// refresh path never fires for a plain `read-string' prompt).
// ============================================================

#[test]
fn minibuffer_input_changed_hook_fires_with_current_input_while_typing() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq se--seen nil) \
         (add-hook 'minibuffer-input-changed-hook \
                    (lambda (s) (setq se--seen (cons s se--seen))))",
    );
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-again");
    // `M-x`'s own minibuffer is a plain prompt with no completion panel
    // candidates matching yet at every prefix -- exactly the shape a
    // panel-gated notification would miss.
    let seen = run(&mut i, "(reverse se--seen)");
    assert_eq!(
        seen, "(\"s\" \"se\" \"sea\" \"sear\" \"searc\" \"search\" \"search-\" \"search-a\" \"search-ag\" \"search-aga\" \"search-agai\" \"search-again\")",
        "hook must fire once per keystroke, each time with the input SO FAR: {}",
        seen
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
}

#[test]
fn minibuffer_input_changed_hook_does_not_fire_on_keys_that_leave_input_unchanged() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(setq se--n 0) \
         (add-hook 'minibuffer-input-changed-hook (lambda (s) (setq se--n (1+ se--n))))",
    );
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-again");
    assert_eq!(run(&mut i, "se--n"), "12");
    // Left/right cursor motion moves point but never touches `mb.input`
    // -- must not trip the hook.
    feed_keys(&mut i, &ed, "C-b").unwrap();
    feed_keys(&mut i, &ed, "C-f").unwrap();
    assert_eq!(
        run(&mut i, "se--n"),
        "12",
        "cursor motion alone must not fire the hook"
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
}

// ============================================================
// M85 D7: `M-.` in `*search*` jumps to a module's own DECLARATION,
// not the hit line -- reusing `verilog-goto-module-at-point'
// (verilog-nav.el, M55) at the hit's real file:line:col rather than
// reimplementing any module-name detection here.
// ============================================================

fn goto_search_bol(i: &mut Interp, row0: usize) {
    run(
        i,
        &format!(
            "(with-current-buffer \"*search*\" (goto-char (point-min)) (forward-line {}))",
            row0
        ),
    );
}

#[test]
fn search_m_dot_jumps_to_module_declaration_of_hit_at_point() {
    let scratch = Scratch::new("m85_mdot");
    write(
        &scratch,
        "top.sv",
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr, output rd);\nendmodule\n",
    );
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("top.sv"));
    // Column 3 is where `fifo` starts on line 2 (1-based, `rg`
    // convention) -- "  fifo u_f (.wr(w));".
    set_fake_printf_engine(&mut i, &["top.sv:2:3:  fifo u_f (.wr(w));"]);
    do_search(&mut i, &ed, "fifo");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "1");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    goto_search_bol(&mut i, 0);
    let r = run(&mut i, "(search-goto-module-declaration)");
    assert!(!r.starts_with("ERROR"), "{}", r);

    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"top.sv\"",
        "must have switched to the file the module is declared in"
    );
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = run(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 4),
    );
    assert_eq!(
        name, "\"fifo\"",
        "point must land exactly on the DECLARATION's own name, not the hit line"
    );
}

#[test]
fn search_m_dot_reports_a_message_when_hit_is_not_a_module_name() {
    let scratch = Scratch::new("m85_mdot_miss");
    write(&scratch, "notes.txt", "line one\nfifo is a kind of queue\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("notes.txt"));
    set_fake_printf_engine(&mut i, &["notes.txt:2:1:fifo is a kind of queue"]);
    do_search(&mut i, &ed, "fifo");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "1");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    goto_search_bol(&mut i, 0);
    let file_before = run(&mut i, "(buffer-name)");
    run(&mut i, "(search-goto-module-declaration)");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("Not on a module name"),
        "must report a message, not silently do nothing: {:?}",
        echo
    );
    // A plain-text buffer has no `'verilog' treesit parser at all, so
    // this must be the "not applicable" branch, not a jump: the hit's
    // OWN file/line is where `search--goto-result' already landed
    // before the module check ran, and that jump itself is unaffected
    // by this test's assertion -- only confirming a message was ALSO
    // produced, and that the miss didn't ERROR out instead.
    let _ = file_before;
}

#[test]
fn search_m_dot_reports_no_result_when_point_is_not_on_a_result_line() {
    let scratch = Scratch::new("m85_mdot_norow");
    write(&scratch, "top.sv", "module top;\nendmodule\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("top.sv"));
    set_fake_printf_engine(&mut i, &["top.sv:1:1:module top;"]);
    do_search(&mut i, &ed, "top");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    // Move point to the buffer's own end -- past the single result
    // line, onto the trailing (possibly blank) line `search--insert-
    // line' always appends after it.
    run(
        &mut i,
        "(with-current-buffer \"*search*\" (goto-char (point-max)))",
    );
    run(&mut i, "(search-goto-module-declaration)");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("No search result on this line"), "{:?}", echo);
}

// ============================================================
// M85 D2-D6: `*search*`'s live result filter (`/`) -- redraw from
// memory (`search--results'), never a re-search, using the same
// orderless matcher (`orderless-rank', M84/M85) `completing-read'
// already uses, applied against each result's WHOLE line text
// (`FILE:LINE:COL:TEXT').
// ============================================================

/// `/' in `*search*' -- opens the live-filter prompt. Requires
/// `*search*' to already be current (mirrors `enter_edit_mode''s own
/// precondition comment).
fn start_filter(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    run(i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(i, ed, "/").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "/ should open a Filter: prompt"
    );
}

fn submit_filter(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    feed_keys(i, ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "RET should close the Filter: prompt"
    );
}

// --- 1/3/4. Narrowing, multi-token orderless, empty filter shows all --

#[test]
fn search_filter_narrows_to_matching_results_across_files() {
    let scratch = Scratch::new("filter_narrow");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "module alu (input clk);\n");
    write(&scratch, "b.v", "module fifo (input clk);\n");
    write(&scratch, "c.v", "// nothing about alu here\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(
        &mut i,
        &[
            "a.v:1:1:module alu (input clk);",
            "b.v:1:1:module fifo (input clk);",
            "c.v:1:1:// nothing about alu here",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "3");

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "alu");
    let buf = search_buffer_string(&mut i);
    assert!(
        buf.contains("a.v:1:1:"),
        "a.v (contains \"alu\") must show: {:?}",
        buf
    );
    assert!(
        buf.contains("c.v:1:1:"),
        "c.v (contains \"alu\") must show: {:?}",
        buf
    );
    assert!(
        !buf.contains("b.v:1:1:"),
        "b.v (no \"alu\" anywhere in its line) must be hidden: {:?}",
        buf
    );
    submit_filter(&mut i, &ed);
}

#[test]
fn search_filter_multi_token_orderless_any_order() {
    let scratch = Scratch::new("filter_multitoken");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    write(&scratch, "b.v", "x\n");
    write(&scratch, "c.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(
        &mut i,
        &[
            "a.v:1:1:module alu (input clk);",
            "b.v:1:1:module fifo (input clk);",
            "c.v:1:1:// nothing about alu here",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    // "clk alu" -- tokens in the OPPOSITE order from how they appear in
    // a.v's own line ("... alu ... clk ...") -- orderless must not care.
    type_str(&mut i, &ed, "clk alu");
    let buf = search_buffer_string(&mut i);
    assert!(
        buf.contains("a.v:1:1:"),
        "a.v has both tokens (order reversed from INPUT) -- must show: {:?}",
        buf
    );
    assert!(
        !buf.contains("b.v:1:1:"),
        "b.v has \"clk\" but not \"alu\" -- must be hidden: {:?}",
        buf
    );
    assert!(
        !buf.contains("c.v:1:1:"),
        "c.v has \"alu\" but not \"clk\" -- must be hidden: {:?}",
        buf
    );
    submit_filter(&mut i, &ed);
}

#[test]
fn search_filter_empty_shows_everything() {
    let scratch = Scratch::new("filter_empty");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    write(&scratch, "b.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:one", "b.v:1:1:two"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    // Opening `/' alone (D2: even at an empty filter, `search--filter'
    // is now a non-nil "" rather than nil) must not hide anything.
    start_filter(&mut i, &ed);
    let buf = search_buffer_string(&mut i);
    assert!(buf.contains("a.v:1:1:"), "{:?}", buf);
    assert!(buf.contains("b.v:1:1:"), "{:?}", buf);
    submit_filter(&mut i, &ed);
}

// --- 2/5. Live per-keystroke narrowing (no RET needed) + the N/M count -

#[test]
fn search_filter_narrows_live_as_you_type_before_ret() {
    let scratch = Scratch::new("filter_live");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "keep.v", "x\n");
    write(&scratch, "drop.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // "xyz123" appears only in keep.v's own line; drop.v's line shares
    // not even a single one of those characters, so narrowing should
    // already be visible after the very FIRST keystroke, well before
    // the filter text is anywhere near complete.
    set_fake_printf_engine(
        &mut i,
        &[
            "keep.v:1:1:target_xyz123_thing",
            "drop.v:1:1:totally_unrelated_stuff",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    // Not yet typed anything -- both still visible.
    let buf0 = search_buffer_string(&mut i);
    assert!(buf0.contains("keep.v:1:1:") && buf0.contains("drop.v:1:1:"));

    type_str(&mut i, &ed, "x");
    let buf1 = search_buffer_string(&mut i);
    assert!(
        buf1.contains("keep.v:1:1:"),
        "keep.v's line has an \"x\" -- must still show: {:?}",
        buf1
    );
    assert!(
        !buf1.contains("drop.v:1:1:"),
        "drop.v's line has no \"x\" anywhere -- must ALREADY be hidden \
         after a single keystroke, without pressing RET: {:?}",
        buf1
    );
    // D4: the visible/total count must be reported without RET too.
    let echo1 = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo1.contains("1/2"),
        "must report 1 shown out of 2 total: {:?}",
        echo1
    );

    type_str(&mut i, &ed, "yz123");
    let buf2 = search_buffer_string(&mut i);
    assert!(buf2.contains("keep.v:1:1:") && !buf2.contains("drop.v:1:1:"));
    let echo2 = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo2.contains("1/2"), "{:?}", echo2);

    submit_filter(&mut i, &ed);
}

// --- 6. THE payload: filter narrows, THEN edit-apply touches only the -
// --- lines still visible -- the whole point of this milestone. --------

#[test]
fn search_filter_then_edit_mode_applies_only_to_visible_lines() {
    let scratch = Scratch::new("filter_then_edit");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "keep.v", "OLD_KEEP\n");
    write(&scratch, "hide.v", "OLD_HIDE\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["keep.v:1:1:OLD_KEEP", "hide.v:1:1:OLD_HIDE"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "keep");
    let filtered = search_buffer_string(&mut i);
    assert!(filtered.contains("keep.v:1:1:") && !filtered.contains("hide.v:1:1:"));
    submit_filter(&mut i, &ed);

    // `*search*' is still current after the filter prompt closes (no
    // buffer switch happened, only the minibuffer overlay closed) --
    // `C-x C-q' reaches `search-edit-mode' directly.
    feed_keys(&mut i, &ed, "C-x C-q").unwrap();
    assert_eq!(
        run(&mut i, "(major-mode-internal-get)"),
        "search-edit-mode",
        "C-x C-q must reach search-edit-mode with *search* still current"
    );
    // Exactly one row is visible/editable now -- hide.v's own row was
    // never even part of the snapshot `search-edit-mode' just took.
    set_search_line(&mut i, 0, "keep.v:1:1:NEW_KEEP");
    apply_edits(&mut i, &ed);

    assert_eq!(
        read_file(&scratch.join("keep.v")),
        "NEW_KEEP\n",
        "the VISIBLE (post-filter) line's edit must apply"
    );
    assert_eq!(
        read_file(&scratch.join("hide.v")),
        "OLD_HIDE\n",
        "the FILTERED-OUT line must be untouched -- it was never even \
         on screen to edit"
    );
}

// --- 7. The other, asymmetric direction: `/` is refused mid-edit ------

#[test]
fn search_filter_refused_while_editing() {
    let scratch = Scratch::new("filter_refused_editing");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hello"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "search-edit-mode");

    // D8: `/' is bound to `search-filter-start' ONLY in the view
    // keymap, never in `search-edit-mode''s own (writable) keymap --
    // pressing it here would just self-insert a literal "/" into the
    // buffer being edited, not reach the command at all. The guard
    // this test exercises is `search-filter-start''s OWN internal
    // check, reached the same way `search-edit-apply-via-m-x-while-
    // not-editing-is-refused' reaches ITS guard: `M-x' directly, which
    // bypasses whichever local keymap happens to be installed.
    let before = search_buffer_string(&mut i);
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "search-filter-start");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "must NOT open a filter prompt while editing"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("edit") || echo.contains("discard"),
        "must report why it refused: {:?}",
        echo
    );
    assert_eq!(
        run(&mut i, "(major-mode-internal-get)"),
        "search-edit-mode",
        "still editing -- unaffected by the refused filter attempt"
    );
    assert_eq!(
        search_buffer_string(&mut i),
        before,
        "buffer content must be untouched by the refused filter attempt"
    );
    discard_edits(&mut i, &ed);
}

#[test]
fn search_filter_key_is_not_bound_in_edit_mode_keymap() {
    // Two SEPARATE assertions, each pinning a different half of D8/D5:
    //
    // 1. (lookup-key) directly inspects the keymap layers themselves
    //    (emulation/local/global -- same mechanism `search_c_c_s_is_a_
    //    prefix_not_a_command' above already uses) and asserts `/' has
    //    NO binding at all while `*search*' is in `search-edit-mode'.
    //    This is the ONLY thing that can tell "`/' isn't bound here"
    //    apart from "`/' IS bound here but whatever it's bound to
    //    refused to do anything" -- a behavioral assertion (did a
    //    minibuffer open?) can't distinguish the two, since `search-
    //    filter-start' itself already refuses while editing (D5) and
    //    would produce the identical "no minibuffer opened" outcome
    //    either way. Review finding (independent cold read, post-M85
    //    fix round): mutating `search--install-edit-keymap' to ADD
    //    `(define-key map "/" 'search-filter-start)' left the OLD
    //    version of this test (behavior-only) passing -- `/' really
    //    was reaching `search-filter-start', which then hit its own
    //    internal `search-edit-mode' guard and produced the exact same
    //    "no minibuffer" outcome the test was checking for. This
    //    `lookup-key' assertion is what actually catches that mutation.
    // 2. The behavioral assertion (pressing `/' via `feed_keys' opens
    //    no minibuffer) is KEPT alongside it -- it pins the guard
    //    described in `search-filter-start''s own doc comment (D5:
    //    refuses outright while editing) for the case some FUTURE
    //    change binds `/' in the edit keymap anyway; without it, a
    //    regression that added the binding AND removed the internal
    //    guard would only be caught by assertion 1, one layer short of
    //    the actual user-visible behavior this test's name promises.
    let scratch = Scratch::new("filter_key_unbound_editing");
    write(
        &scratch,
        "main.sv",
        "// nothing
",
    );
    write(
        &scratch, "a.v", "x
",
    );
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hello"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    enter_edit_mode(&mut i, &ed);

    // Assertion 1: the keymap itself has no binding for `/' anywhere in
    // the emulation/local/global layer stack right now.
    assert_eq!(
        run(&mut i, "(lookup-key (list ?/))"),
        "nil",
        "`/' must not be bound to anything (not `search-filter-start', \
         not anything else) while *search* is in search-edit-mode"
    );

    // Assertion 2: pressing `/' produces no user-visible effect either
    // (no minibuffer opens) -- see this test's own header comment for
    // why both assertions are kept.
    feed_keys(&mut i, &ed, "/").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "`/' must not open a filter prompt while editing"
    );
    discard_edits(&mut i, &ed);
}

// --- 8. Filter set while a search is still streaming -- new results ---
// --- decide visibility against the CURRENT filter, not the search's ---
// --- own completion. ----------------------------------------------------

#[test]
fn search_filter_set_mid_stream_governs_later_results_too() {
    let scratch = Scratch::new("filter_mid_stream");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "keep1.v", "x\n");
    write(&scratch, "keep2.v", "x\n");
    write(&scratch, "hide1.v", "x\n");
    write(&scratch, "hide2.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // One line drained per idle tick -- keeps the job's own `search--
    // procs' entry alive across several ticks so a filter can be
    // engaged WHILE lines are still streaming in, not after the job
    // has already finished.
    run(&mut i, "(setq search-max-lines-per-tick 1)");
    set_fake_printf_engine(
        &mut i,
        &[
            "keep1.v:1:1:KEEP one",
            "hide1.v:1:1:nope one",
            "keep2.v:1:1:KEEP two",
            "hide2.v:1:1:nope two",
        ],
    );
    do_search(&mut i, &ed, "x");

    // Drain exactly the FIRST line (keep1.v), then engage the filter
    // while the job is still running (search--procs non-nil).
    let ok = pump_until(&mut i, Duration::from_secs(5), |ip| {
        run(ip, "(length search--results)") == "1"
    });
    assert!(ok, "first line never landed");
    assert!(
        !no_search_procs_running(&mut i),
        "job must still be running when the filter is engaged"
    );

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "KEEP");
    submit_filter(&mut i, &ed);

    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(
        run(&mut i, "(length search--results)"),
        "4",
        "all 4 lines must still be RECORDED regardless of filter"
    );
    let buf = search_buffer_string(&mut i);
    assert!(buf.contains("keep1.v:1:1:") && buf.contains("keep2.v:1:1:"));
    assert!(
        !buf.contains("hide1.v:1:1:") && !buf.contains("hide2.v:1:1:"),
        "lines that streamed in AFTER the filter was set must ALSO be \
         hidden if they don't match it, not just the ones already on \
         screen when filtering started: {:?}",
        buf
    );
}

// ============================================================
// M85 fix round (independent cold-read review): F1/F2/F3.
// ============================================================

// --- F1 (HIGH): a non-monotonic filter change leaves a HIDDEN entry's -
// --- stale BUFFER-POS able to collide with a currently-visible one's --
// --- NEW position, so RET can silently jump to the wrong file. --------

#[test]
fn search_filter_ret_does_not_jump_to_a_stale_hidden_entrys_position() {
    // F1 repro construction note: a NAIVE "type unique_B then
    // backspace-to-empty then type unique_A" sequence does NOT
    // reproduce this bug -- the empty-input intermediate state matches
    // EVERY entry (D2's own empty-filter convention), which re-renders
    // and re-positions BOTH entries correctly, erasing the staleness
    // before it can collide. This repro instead uses a two-TOKEN
    // filter and cursor movement (`C-a'/`C-e', both ordinary user
    // actions) so B is NEVER re-shown (and so never gets a fresh
    // position) from the moment A starts matching onward.
    let scratch = Scratch::new("f1_stale_pos");
    write(
        &scratch,
        "main.sv",
        "// nothing
",
    );
    write(
        &scratch, "aaa.v", "x
",
    );
    write(
        &scratch, "bbb.v", "x
",
    );
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // B (bbb.v) is NEWER than A (aaa.v) -- `search--results' is
    // newest-first, so B sits ahead of A in the scan order `search--
    // result-at-buffer-pos' walks.
    set_fake_printf_engine(
        &mut i,
        &["aaa.v:1:1:same unique_A", "bbb.v:1:1:same unique_B"],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    // Narrow to B only -- B's own BUFFER-POS is recomputed to line 1
    // (the only line shown); A is hidden.
    type_str(&mut i, &ed, "unique_B");
    let buf_b_only = search_buffer_string(&mut i);
    assert!(
        buf_b_only.contains("bbb.v:1:1:") && !buf_b_only.contains("aaa.v:1:1:"),
        "{:?}",
        buf_b_only
    );

    // Prepend a SECOND token at the FRONT of the input (not appended
    // after -- `C-a' moves to the start first) -- the filter becomes
    // "unique_A unique_B", a two-token AND-match neither line
    // satisfies (no line contains both substrings), so at this exact
    // moment BOTH entries are hidden -- but B's own BUFFER-POS is
    // simply never touched again from here on, since it never matches
    // again from this point forward.
    feed_keys(&mut i, &ed, "C-a").unwrap();
    type_str(&mut i, &ed, "unique_A ");
    let buf_neither = search_buffer_string(&mut i);
    assert_eq!(
        buf_neither, "\"\"",
        "two-token AND-filter must match neither line yet (empty *search* \
         buffer, printed as an empty elisp string)"
    );

    // Now trim the SECOND token (originally "unique_B") from the END,
    // one character at a time (`C-e' then repeated backspace) --
    // ordinary editing, never touching the FIRST token ("unique_A").
    // The moment the trailing "B" is removed, the second token becomes
    // a prefix ("unique_") that A's OWN text also contains (A's line
    // ends in "unique_A", which itself starts with "unique_") -- so A
    // starts matching (both of ITS required tokens present), while B
    // never regains a match (it never contains "unique_A" at all).
    // This is the crux of the repro: A flips from hidden to VISIBLE
    // and gets a FRESH "first line" position, while B has been hidden,
    // untouched, since the "unique_B"-only state above -- if B's
    // BUFFER-POS is left stale instead of cleared, it can equal
    // (and, per this exact construction, DOES equal) A's brand new
    // position.
    feed_keys(&mut i, &ed, "C-e").unwrap();
    for _ in 0..9 {
        feed_keys(&mut i, &ed, "DEL").unwrap();
    }
    let buf_a_only = search_buffer_string(&mut i);
    assert!(
        buf_a_only.contains("aaa.v:1:1:") && !buf_a_only.contains("bbb.v:1:1:"),
        "{:?}",
        buf_a_only
    );
    submit_filter(&mut i, &ed);

    // The ONLY line on screen right now is A's -- point RET at it.
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "RET").unwrap();

    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"aaa.v\"",
        "RET on the only VISIBLE line must jump to aaa.v (A), not \
         bbb.v (B) -- B is hidden and its BUFFER-POS is a stale \
         leftover from when it was last shown, which must not collide \
         with A's own freshly-recomputed position"
    );
}

// --- F2 (HIGH): `n'/`p' must stay within the VISIBLE (filtered) subset -

#[test]
fn search_filter_n_stays_within_visible_subset() {
    let scratch = Scratch::new("f2_n_filtered");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "keep.v", "x\n");
    write(&scratch, "hide1.v", "x\n");
    write(&scratch, "hide2.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(
        &mut i,
        &["hide1.v:1:1:nope", "keep.v:1:1:KEEP", "hide2.v:1:1:nope"],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "3");

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "KEEP");
    let buf = search_buffer_string(&mut i);
    assert!(buf.contains("keep.v:1:1:") && !buf.contains("hide1.v") && !buf.contains("hide2.v"));
    submit_filter(&mut i, &ed);

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    for _ in 0..3 {
        feed_keys(&mut i, &ed, "n").unwrap();
        assert_eq!(
            run(&mut i, "(buffer-name)"),
            "\"keep.v\"",
            "n must stay on the only VISIBLE result -- must never jump \
             to a hidden hide1.v/hide2.v"
        );
    }
}

// --- F3 (HIGH): a LATER, unrelated minibuffer prompt on `*search*' ----
// --- must not hijack `search--filter' just because it wasn't cleared. -

#[test]
fn search_filter_not_hijacked_by_a_later_unrelated_prompt() {
    let scratch = Scratch::new("f3_hijack");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hello"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "hello");
    submit_filter(&mut i, &ed);
    let filter_before = run(&mut i, "search--filter");
    let buf_before = search_buffer_string(&mut i);

    // `*search*' is STILL the current buffer (opening a minibuffer
    // never changes it) -- open an entirely unrelated prompt on top of
    // it and type into it, exactly the way `search--filter-input-
    // changed' must NOT mistake for its own filter session.
    feed_keys(&mut i, &ed, "M-x").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(&mut i, &ed, "search-again");

    assert_eq!(
        run(&mut i, "search--filter"),
        filter_before,
        "an unrelated M-x prompt typed while *search* is current must \
         NOT overwrite search--filter"
    );
    assert_eq!(
        search_buffer_string(&mut i),
        buf_before,
        "*search*'s own content must not be redrawn by an unrelated \
         prompt's keystrokes"
    );
    // Close it without actually running anything.
    feed_keys(&mut i, &ed, "C-g").unwrap();
}

// ============================================================
// M85 fix round (independent cold-read review): F6/F7/F9 -- pinned
// CURRENT behavior (see search.el's own "M85 v1 does not include /
// known gaps" section for why each is a documented gap, not fixed
// here).
// ============================================================

// --- F6: the truncation banner is swallowed under an active filter, ---
// --- but the echo-area message still fires. ----------------------------

#[test]
fn search_filter_swallows_the_truncation_banner_but_keeps_the_echo_message() {
    let scratch = Scratch::new("f6_truncate_filtered");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "f.v", "placeholder\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    run(&mut i, "(setq search-max-results 5)");
    run(&mut i, "(setq search-max-lines-per-tick 1)");
    set_fake_engine(
        &mut i,
        "bash",
        "-c 'for n in $(seq 1 50); do printf \"f.v:%d:1:MATCHME\\n\" \"$n\"; done' #",
    );
    do_search(&mut i, &ed, "x");

    // Engage the filter WHILE the job is still running and still under
    // the cap, so every subsequent streamed line (including the
    // eventual truncation banner) goes through the filtered insert
    // path.
    let ok = pump_until(&mut i, Duration::from_secs(5), |ip| {
        run(ip, "(length search--results)") != "0"
    });
    assert!(ok, "first line never landed");
    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "MATCHME");
    submit_filter(&mut i, &ed);

    let ok = pump_until(&mut i, Duration::from_secs(10), no_search_procs_running);
    assert!(ok, "truncated search job never finished");

    let content = search_buffer_string(&mut i);
    assert!(
        !content.contains("truncated"),
        "F6 (documented gap): under an active filter, the permanent \
         truncation banner is CURRENTLY swallowed (it fails to parse \
         as a result line, so `search--insert-filtered-line' drops it \
         outright) -- this assertion pins that current behavior; \
         seeing \"truncated\" here would mean the gap was fixed and \
         this test (and search.el's F6 note) needs updating: {:?}",
        content
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("truncated"),
        "the echo-area message must still fire even though the \
         buffer's own permanent banner does not: {:?}",
        echo
    );
}

// --- F7: C-g mid-filter leaves the partial filter in place, no revert -

#[test]
fn search_filter_c_g_leaves_the_partial_filter_in_place() {
    let scratch = Scratch::new("f7_c_g");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "keep.v", "x\n");
    write(&scratch, "hide.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["keep.v:1:1:KEEP", "hide.v:1:1:nope"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "KEEP");
    let buf_mid_filter = search_buffer_string(&mut i);
    assert!(buf_mid_filter.contains("keep.v:1:1:") && !buf_mid_filter.contains("hide.v:1:1:"));

    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "C-g must close the filter prompt"
    );

    // F7 (documented gap): NOT reverted to the pre-`/' state (both
    // results visible) -- still shows only the partially-filtered
    // subset from the moment C-g was pressed.
    assert_eq!(
        run(&mut i, "search--filter"),
        "\"KEEP\"",
        "F7 (documented gap): search--filter is left at its partial \
         value after C-g, not reverted to nil"
    );
    assert_eq!(
        search_buffer_string(&mut i),
        buf_mid_filter,
        "F7 (documented gap): *search*'s content is left at the \
         partially-filtered state after C-g, not restored to showing \
         every result"
    );
}

// --- F9: search--render leaves point at the buffer's end, not on the --
// --- line that was under point before the redraw. ----------------------

#[test]
fn search_filter_render_leaves_point_at_buffer_end() {
    let scratch = Scratch::new("f9_point");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    write(&scratch, "b.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    set_fake_printf_engine(&mut i, &["a.v:1:1:hello", "b.v:1:1:hello"]);
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    run(&mut i, "(goto-char (point-min))");
    let point_before = run(&mut i, "(point)");
    assert_ne!(point_before, run(&mut i, "(point-max)"));

    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "hello");
    submit_filter(&mut i, &ed);

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    assert_eq!(
        run(&mut i, "(point)"),
        run(&mut i, "(point-max)"),
        "F9 (documented gap): search--render leaves point at the \
         buffer's end after a redraw, not preserved on whatever line \
         was under point beforehand"
    );
}

// ============================================================
// M85 fix round (tail re-review): H1 -- `search--current-index` must
// be recalibrated against the SAME entry when a filter change alters
// `search--nav-results`'s own composition, not left at a stale
// numeric position that now names an unrelated entry.
// ============================================================

#[test]
fn search_filter_changes_composition_recalibrates_current_index() {
    let scratch = Scratch::new("h1_recalibrate");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "x\n");
    write(&scratch, "b.v", "x\n");
    write(&scratch, "c.v", "x\n");
    write(&scratch, "d.v", "x\n");
    write(&scratch, "e.v", "x\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // Only a.v/c.v/e.v's own lines contain "KEEP" -- b.v/d.v don't
    // share even the uppercase "K" with it (smart-case makes "KEEP" a
    // case-SENSITIVE token), so filtering to "KEEP" narrows the
    // 5-entry nav list straight to {a.v, c.v, e.v} in one keystroke.
    set_fake_printf_engine(
        &mut i,
        &[
            "a.v:1:1:KEEP_A",
            "b.v:1:1:plain_b",
            "c.v:1:1:KEEP_C",
            "d.v:1:1:plain_d",
            "e.v:1:1:KEEP_E",
        ],
    );
    do_search(&mut i, &ed, "x");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_search_procs_running);
    assert!(ok, "search job never finished");
    assert_eq!(run(&mut i, "(length search--results)"), "5");

    // First `n' via a real keypress while `*search*' is current;
    // the remaining two via `M-x' (current buffer has moved to the
    // jump target after the first, so `n's own local binding on
    // `*search*' isn't reachable by a plain keypress anymore -- same
    // pattern `search_n_p_navigate_cyclically' already uses).
    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "n").unwrap();
    for _ in 0..2 {
        feed_keys(&mut i, &ed, "M-x").unwrap();
        type_str(&mut i, &ed, "search-next-result");
        feed_keys(&mut i, &ed, "RET").unwrap();
    }
    // Three `n' presses from a fresh (nil) index land on the THIRD
    // on-screen entry, c.v (index 2: a.v=0, b.v=1, c.v=2).
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"c.v\"",
        "sanity: three n presses must land on c.v before filtering"
    );

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    start_filter(&mut i, &ed);
    type_str(&mut i, &ed, "KEEP");
    let buf = search_buffer_string(&mut i);
    assert!(
        buf.contains("a.v:1:1:") && buf.contains("c.v:1:1:") && buf.contains("e.v:1:1:"),
        "{:?}",
        buf
    );
    assert!(
        !buf.contains("b.v:1:1:") && !buf.contains("d.v:1:1:"),
        "{:?}",
        buf
    );
    submit_filter(&mut i, &ed);

    run(&mut i, "(switch-to-buffer-internal \"*search*\")");
    feed_keys(&mut i, &ed, "n").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"e.v\"",
        "H1: after filtering narrows the nav list to {{a.v, c.v, e.v}} \\
         while standing on c.v, `search--current-index' must be \\
         recalibrated to STILL point at c.v (now at position 1 in the \\
         3-entry nav list, not the stale position 2) -- the next `n' \\
         from there must land on e.v (the next VISIBLE entry after \\
         c.v), not silently wrap back to a.v via the unrecalibrated \\
         stale index"
    );
}
