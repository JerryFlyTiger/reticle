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
            "se_eshell_{}_{}_{}",
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

#[test]
fn eshell_opens_with_prompt() {
    let (mut i, _ed) = setup();
    run(&mut i, "(eshell)");
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*eshell*\"");
    let text = buffer_text(&mut i);
    assert!(text.contains(" $ "), "prompt missing: {}", text);
}

#[test]
fn a_freshly_opened_eshell_buffer_is_not_marked_modified() {
    // M71: the welcome message + first prompt are inserted via
    // `insert', which sets the modified flag -- `*eshell*' has no file
    // to save, so a `*' on it would only ever mean "you ran a command",
    // never "you have unsaved work". `eshell--insert-prompt' clears the
    // flag after every insertion; this covers the very first one.
    let (mut i, _ed) = setup();
    run(&mut i, "(eshell)");
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "a freshly opened *eshell* buffer must not be modified"
    );
}

#[test]
fn eshell_stays_unmodified_after_a_command_and_new_prompt() {
    // Uses `pwd' (an internal command, see `eshell_pwd_cd_clear' above)
    // rather than an external process -- it runs synchronously so the
    // test doesn't need `pump_until', and it still exercises the exact
    // path under test: `eshell--insert-prompt' running again after a
    // command's output was inserted.
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "pwd");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "running a command and getting a fresh prompt must not leave *eshell* modified"
    );
}

#[test]
fn eshell_stays_unmodified_after_incomplete_elisp_input() {
    // Fix round: `eshell--eval-elisp''s `incomplete' branch (unbalanced
    // parens, RET pressed) is a stable state distinct from "typing
    // mid-word" -- M71's fix round added `set-buffer-modified-p' to
    // it, matching the same branch in `ielm-return'. Assert both the
    // flag and that the buffer really grew a line.
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    let before = buffer_text(&mut i);
    type_str(&mut i, &ed, "(+ 3");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let after = buffer_text(&mut i);
    assert_ne!(
        after, before,
        "RET on incomplete input should insert a newline"
    );
    assert!(
        after.contains("(+ 3\\n"),
        "expected the unbalanced input followed by a newline: {}",
        after
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "RET on incomplete elisp input is a stable state (not mid-typing) and must not leave *eshell* modified"
    );
}

#[test]
fn eshell_evaluates_elisp() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "(+ 20 22)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = buffer_text(&mut i);
    assert!(text.contains("(+ 20 22)\\n42\\n"), "elisp eval: {}", text);
}

#[test]
fn eshell_pwd_cd_clear() {
    let (mut i, ed) = setup();
    let dir = Scratch::new("cd");
    std::fs::create_dir_all(&dir).unwrap();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "pwd");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let before = run(&mut i, "(default-directory)");
    assert!(buffer_text(&mut i).contains(&before.trim_matches('"').to_string()));
    // cd into the temp dir; prompt and default-directory follow.
    type_str(&mut i, &ed, &format!("cd {}", dir.to_str().unwrap()));
    feed_keys(&mut i, &ed, "RET").unwrap();
    let dd = run(&mut i, "(default-directory)");
    assert!(dd.contains("se_eshell_cd"), "cd failed: {}", dd);
    // cd to a bogus dir reports and keeps the old one.
    type_str(&mut i, &ed, "cd /no/such/dir/zzz");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(buffer_text(&mut i).contains("no such directory"));
    // clear empties the buffer down to a fresh prompt.
    type_str(&mut i, &ed, "clear");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = buffer_text(&mut i);
    assert!(
        !text.contains("no such directory"),
        "clear failed: {}",
        text
    );
    assert!(text.contains(" $ "));
}

#[test]
fn external_command_streams_output() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "echo hello-from-shell");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        run(&mut i, "eshell--procs") != "nil",
        "process should be registered"
    );
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        run(i, "(buffer-string)").contains("hello-from-shell\\n")
    });
    assert!(ok, "output never arrived: {}", buffer_text(&mut i));
    // Prompt came back, no [exit] line for status 0.
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        run(i, "eshell--procs") == "nil"
    });
    assert!(ok, "process never finished");
    let text = buffer_text(&mut i);
    assert!(
        !text.contains("[exit"),
        "clean exit must not print [exit]: {}",
        text
    );
    assert!(
        text.trim_matches('"').ends_with(" $ "),
        "prompt back: {}",
        text
    );
}

// M71 tail review, finding 2: streamed stdout lands while the process
// is still running, so there is a window between the first chunk and
// the exit prompt that the prompt-side clear cannot cover -- without
// the clear in `eshell-process-pending-all''s string branch, *eshell*
// shows `*' for the whole length of a long command, which is neither
// of the two exceptions `eshell--insert-prompt''s comment documents.
// The assertion sits at exactly that point: output has landed and the
// process is still registered. `sleep 1' is what makes that window
// wide enough to observe rather than a race.
#[test]
fn eshell_stays_unmodified_while_a_command_is_still_streaming() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "echo chunk; sleep 1");
    feed_keys(&mut i, &ed, "RET").unwrap();
    // Two occurrences, not one: the echoed command line itself already
    // contains "chunk", so `contains` alone returns true before any
    // process output has arrived at all -- the window this test exists
    // to observe would be skipped entirely.
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        run(i, "(buffer-string)").matches("chunk").count() >= 2
    });
    assert!(ok, "output never arrived: {}", buffer_text(&mut i));
    assert_ne!(
        run(&mut i, "eshell--procs"),
        "nil",
        "the process must still be running for this to test the streaming window"
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "streamed output must not leave *eshell* showing the modified marker"
    );
    // Reap it so the test doesn't leave a live child behind.
    pump_until(&mut i, Duration::from_secs(5), |i| {
        run(i, "eshell--procs") == "nil"
    });
}

#[test]
fn stderr_merges_and_nonzero_exit_reports() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "echo oops >&2; exit 3");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let ok = pump_until(&mut i, Duration::from_secs(5), |i| {
        let t = run(i, "(buffer-string)");
        t.contains("oops") && t.contains("[exit 3]")
    });
    assert!(ok, "stderr/exit not reported: {}", buffer_text(&mut i));
}

#[test]
fn interrupt_kills_running_process() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "sleep 30");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(core::has_async_work(&mut i), "async work while running");
    let start = Instant::now();
    feed_keys(&mut i, &ed, "C-c C-c").unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "kill must not wait for sleep"
    );
    let text = buffer_text(&mut i);
    assert!(text.contains("[killed]"), "kill marker: {}", text);
    assert_eq!(run(&mut i, "eshell--procs"), "nil");
    assert!(!core::has_async_work(&mut i), "no async work after kill");
}

#[test]
fn history_recall() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "(+ 1 1)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    type_str(&mut i, &ed, "(+ 2 2)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    // M-p recalls the most recent command into the input region.
    feed_keys(&mut i, &ed, "M-p").unwrap();
    let text = buffer_text(&mut i);
    assert!(
        text.trim_matches('"').ends_with("(+ 2 2)"),
        "M-p once: {}",
        text
    );
    feed_keys(&mut i, &ed, "M-p").unwrap();
    let text = buffer_text(&mut i);
    assert!(
        text.trim_matches('"').ends_with("(+ 1 1)"),
        "M-p twice: {}",
        text
    );
    // M-n walks back forward.
    feed_keys(&mut i, &ed, "M-n").unwrap();
    let text = buffer_text(&mut i);
    assert!(text.trim_matches('"').ends_with("(+ 2 2)"), "M-n: {}", text);
    // Re-run the recalled line.
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = buffer_text(&mut i);
    assert!(text.contains("(+ 2 2)\\n4"), "recalled line runs: {}", text);
}

#[test]
fn multiline_elisp_continues() {
    let (mut i, ed) = setup();
    run(&mut i, "(eshell)");
    type_str(&mut i, &ed, "(+ 3");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(!buffer_text(&mut i).contains("Error"));
    type_str(&mut i, &ed, " 4)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        buffer_text(&mut i).contains("\\n7\\n"),
        "multiline: {}",
        buffer_text(&mut i)
    );
}

// M69: `eshell--abbrev-dir' had the same boundary-free-prefix bug as
// the Rust mode-line code (`string-prefix-p' alone, no separator
// check) -- a sibling directory whose name happens to start with
// $HOME's basename (e.g. HOME=/x/home, dir=/x/home2/rtl) used to get
// mis-abbreviated to "~2/rtl". These don't touch the `HOME' env var
// (a process-global that's a race under parallel tests) -- they derive
// everything from `(expand-file-name "~")', the same value the elisp
// function itself reads.

#[test]
fn eshell_abbrev_dir_sibling_of_home_not_mangled() {
    let (mut i, _ed) = setup();
    let out = run(
        &mut i,
        "(eshell--abbrev-dir (concat (expand-file-name \"~\") \"2/x\"))",
    );
    assert!(!out.contains("~2"), "sibling dir mangled: {}", out);
    let home = run(&mut i, "(expand-file-name \"~\")");
    let home_bare = home.trim_matches('"');
    assert_eq!(out, format!("\"{}2/x\"", home_bare));
}

#[test]
fn eshell_abbrev_dir_under_home_abbreviates() {
    let (mut i, _ed) = setup();
    let out = run(
        &mut i,
        "(eshell--abbrev-dir (concat (expand-file-name \"~\") \"/x\"))",
    );
    assert_eq!(out, "\"~/x\"");
}

#[test]
fn eshell_abbrev_dir_exactly_home() {
    let (mut i, _ed) = setup();
    let out = run(&mut i, "(eshell--abbrev-dir (expand-file-name \"~\"))");
    assert_eq!(out, "\"~\"");
}
