//! M80: `compile`/`recompile`, and the `next-error`/`previous-error`
//! dispatcher (`compile.el`). Follows shell_command_tests.rs's shape --
//! async pump + buffer content -- as the closest existing precedent
//! (compile.el reuses two of that file's helpers directly).
//!
//! No real `iverilog`/`verible` here (the task must not depend on
//! either being installed) -- a fake "compiler" is a `printf` line per
//! test, run against real files created under a scratch directory so
//! the disk-existence check (`compile--parse-error-line`) has something
//! real to check against.

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

fn no_compile_procs_running(i: &mut Interp) -> bool {
    run(i, "compile--procs") == "nil"
}

/// A scratch directory that deletes itself on drop -- same shape
/// dabbrev_tests.rs's `Scratch` uses (`Drop` runs during unwind too, so
/// a failing assertion still cleans up).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "se_compile_{}_{}_{}",
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

/// Visit FILE (must already exist on disk) so `buffer-file-name' /
/// `lsp--project-root' have something real to resolve -- with no
/// project-root marker anywhere above a `std::env::temp_dir()' scratch
/// directory, `lsp--project-root' falls back to the file's own
/// directory, which is exactly the scratch directory these tests want
/// `compile''s working directory to be.
fn visit(i: &mut Interp, path: &std::path::Path) {
    let out = run(i, &format!("(find-file {:?})", path.to_str().unwrap()));
    assert!(
        !out.starts_with("ERROR"),
        "find-file {:?} failed: {}",
        path,
        out
    );
}

/// Run `M-x compile', typing CMD at the "Compile command:" prompt.
fn do_compile(i: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: &str) {
    feed_keys(i, ed, "M-x").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-x should open a minibuffer prompt"
    );
    type_str(i, ed, "compile");
    feed_keys(i, ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "compile should open a second minibuffer prompt (Compile command:)"
    );
    // The prompt is pre-filled with the current `compile-command' (see
    // `compile''s own doc comment), cursor at the end -- clear it first
    // (`C-a C-k') so the typed CMD *replaces* rather than appends after
    // whatever the previous test/invocation left behind.
    feed_keys(i, ed, "C-a").unwrap();
    feed_keys(i, ed, "C-k").unwrap();
    type_str(i, ed, cmd);
    feed_keys(i, ed, "RET").unwrap();
}

fn do_recompile(i: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    feed_keys(i, ed, "M-x").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    type_str(i, ed, "recompile");
    feed_keys(i, ed, "RET").unwrap();
}

/// The four real-tool error-line shapes captured in compile.el's header
/// comment, plus one line naming a file that does NOT exist on disk
/// (must be dropped by the existence check, not parsed as an error).
fn fake_compiler_printf(_scratch: &std::path::Path) -> String {
    "printf '%s\\n' \
     'styled.sv:2:15-19: Explicitly define a storage type ... [Style: constants][explicit-parameter-storage-type]' \
     'top.sv:11: error: Unknown module type: nonexistent_module' \
     'bad.v:3: syntax error' \
     'bus/axi4_lite_arbiter.sv:35: sorry: Overriding the default variable lifetime is not yet supported.' \
     'nosuchfile.v:3: error: x'; true"
        .to_string()
}

fn write(scratch: &std::path::Path, rel: &str, contents: &str) {
    let p = scratch.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, contents).unwrap();
}

fn compile_dir_str(i: &mut Interp) -> String {
    run(i, "compile--dir")
}

// --- 1-2. All four real formats parse; nonexistent file is dropped -----

#[test]
fn compile_parses_all_four_real_formats_and_drops_nonexistent_file() {
    let scratch = Scratch::new("parse");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "styled.sv", "// nothing\n");
    write(&scratch, "top.sv", "// nothing\n");
    write(&scratch, "bad.v", "// nothing\n");
    write(&scratch, "bus/axi4_lite_arbiter.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    do_compile(&mut i, &ed, &fake_compiler_printf(&scratch));
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");
    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "4",
        "expected exactly 4 real errors, nosuchfile.v dropped: {}",
        n
    );

    let severities = run(&mut i, "(mapcar (lambda (e) (aref e 3)) compile--errors)");
    assert_eq!(
        severities, "(note error error sorry)",
        "severity classification: {}",
        severities
    );

    let cols = run(&mut i, "(mapcar (lambda (e) (aref e 2)) compile--errors)");
    assert_eq!(
        cols, "(15 nil nil nil)",
        "column parsing (verible range keeps only start): {}",
        cols
    );

    let lines = run(&mut i, "(mapcar (lambda (e) (aref e 1)) compile--errors)");
    assert_eq!(lines, "(2 11 3 35)", "line numbers: {}", lines);
}

// --- 3. compile-next-error jumps across files ---------------------------

#[test]
fn compile_next_error_jumps_across_files() {
    let scratch = Scratch::new("crossfile");
    write(&scratch, "main.sv", "// nothing\n");
    write(
        &scratch,
        "top.sv",
        "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\nline11 bad\n",
    );
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'top.sv:11: error: Unknown module type: nonexistent_module'; true"
        .to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile-next-error");
    feed_keys(&mut i, &ed, "RET").unwrap();

    let name = run(&mut i, "(buffer-name)");
    assert_eq!(
        name, "\"top.sv\"",
        "compile-next-error must switch to the file the error is in: {}",
        name
    );
    let line = run(&mut i, "(line-number-at-pos)");
    assert_eq!(line, "11", "point must land on the error's line: {}", line);
}

// --- 4-5. Column jump, and clamping past line length --------------------

#[test]
fn compile_next_error_jumps_to_column_and_clamps_past_line_end() {
    let scratch = Scratch::new("column");
    write(&scratch, "main.sv", "// nothing\n");
    // Line 2 is short (5 chars) -- the second error's COL (19) must
    // clamp to end-of-line, not run onto line 3.
    write(&scratch, "styled.sv", "line1\nshort\nline3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'styled.sv:1:3: msg one' 'styled.sv:2:19: msg two'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile-next-error");
    feed_keys(&mut i, &ed, "RET").unwrap();
    // Error 1: line 1 "line1", col 3 (1-based) -> point 3 chars into
    // the line, i.e. at offset (1-based col) 3.
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");
    assert_eq!(
        run(&mut i, "(current-column)"),
        "2",
        "col 3 (1-based) is 2 chars past line start"
    );

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile-next-error");
    feed_keys(&mut i, &ed, "RET").unwrap();
    // Error 2: line 2 "short" (5 chars), col 19 -- way past the line's
    // end -- must clamp to line-end, not spill onto line 3.
    assert_eq!(
        run(&mut i, "(line-number-at-pos)"),
        "2",
        "must stay on line 2, not overshoot to line 3"
    );
    assert_eq!(
        run(&mut i, "(current-column)"),
        "5",
        "clamped to line-end-position (5 chars)"
    );
}

// --- 6. Wraparound -------------------------------------------------------

#[test]
fn compile_next_error_wraps_to_first_after_last() {
    let scratch = Scratch::new("wrap");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nl2\nl3\n");
    write(&scratch, "b.v", "l1\nl2\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'a.v:1: error: e1' 'b.v:2: error: e2'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    for _ in 0..3 {
        feed_keys(&mut i, &ed, "M-x").unwrap();
        type_str(&mut i, &ed, "compile-next-error");
        feed_keys(&mut i, &ed, "RET").unwrap();
    }
    // 3 calls over a 2-entry list: entry0, entry1, wrap back to entry0.
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"a.v\"",
        "third call must wrap back to the first error"
    );
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "1");
}

// --- 7. M-g n dispatcher: compile errors take priority over LSP --------

#[test]
fn next_error_dispatcher_prefers_compile_errors_when_present() {
    let scratch = Scratch::new("dispatch");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nl2\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'a.v:1: error: e1'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    feed_keys(&mut i, &ed, "M-g n").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"a.v\"",
        "M-g n must walk compile errors when present"
    );
    assert!(
        ed.borrow()
            .echo
            .as_deref()
            .unwrap_or("")
            .starts_with("[compile]"),
        "echo must be prefixed [compile]: {:?}",
        ed.borrow().echo
    );
}

#[test]
fn next_error_dispatcher_falls_back_to_lsp_diagnostics_when_no_compile_errors() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    // No compile job has ever run (compile--errors is nil): the
    // dispatcher must fall through to `next-diagnostic', which reports
    // "no LSP server connected" for a buffer with no client -- the
    // observable proof the fallback path ran, without standing up a
    // real LSP server.
    feed_keys(&mut i, &ed, "M-g n").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.starts_with("[lsp]"),
        "echo must be prefixed [lsp] when falling back: {:?}",
        echo
    );
    assert!(
        echo.contains("No LSP server connected"),
        "fallback must actually reach next-diagnostic's own message: {:?}",
        echo
    );
}

// --- 8-9. recompile reuses the last command/dir; compile remembers it --

#[test]
fn recompile_reuses_last_command_and_directory_without_prompting() {
    let scratch = Scratch::new("recompile");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'a.v:1: error: e1'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "first compile job never finished");
    let dir_after_compile = compile_dir_str(&mut i);

    // Switch to an unrelated buffer with a DIFFERENT project root, so a
    // stale/rederived directory would show up as a behavior change --
    // `recompile' must still use the directory from the ORIGINAL
    // `compile' call, not whatever buffer is current now.
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");

    do_recompile(&mut i, &ed);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "recompile job never finished");
    assert_eq!(
        compile_dir_str(&mut i),
        dir_after_compile,
        "recompile must reuse the same directory"
    );
    assert_eq!(
        run(&mut i, "(length compile--errors)"),
        "1",
        "recompile must have rerun the same command"
    );
}

#[test]
fn compile_prefills_prompt_with_current_compile_command() {
    let scratch = Scratch::new("prefill");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // First compile sets `compile-command'.
    do_compile(&mut i, &ed, "printf '%s\\n' 'set once'");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "first compile job never finished");
    assert_eq!(
        run(&mut i, "compile-command"),
        "\"printf '%s\\\\n' 'set once'\""
    );

    // Second `M-x compile', accepting the prefilled INITIAL with a bare
    // RET (type nothing) -- must resubmit the SAME command.
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(ed.borrow().minibuffer.is_some());
    assert_eq!(
        ed.borrow().minibuffer.as_ref().unwrap().input,
        "printf '%s\\n' 'set once'",
        "prompt must be pre-filled with the current compile-command"
    );
    feed_keys(&mut i, &ed, "RET").unwrap();
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "second compile job never finished");
}

// --- 10. *compilation* is never left marked modified during streaming --

#[test]
fn compilation_buffer_not_marked_modified_during_streaming() {
    let scratch = Scratch::new("modified");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    do_compile(&mut i, &ed, "printf '%s\\n' 'hello-m80'");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");
    let modified = run(
        &mut i,
        "(with-current-buffer-internal \"*compilation*\" (lambda () (buffer-modified-p)))",
    );
    assert_eq!(
        modified, "nil",
        "*compilation* must not be left marked modified"
    );
}

// --- 11. compilation-mode is an evil emacs-state mode -------------------

#[test]
fn compilation_mode_is_an_evil_emacs_state_mode() {
    let (mut i, _ed) = setup();
    let member = run(&mut i, "(memq 'compilation-mode evil-emacs-state-modes)");
    assert_ne!(
        member, "nil",
        "compilation-mode must be in evil-emacs-state-modes"
    );
}

// --- 12. M-g n/p bound to the dispatcher --------------------------------

#[test]
fn m_g_n_and_p_bound_to_dispatcher() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\M-g ?n))"),
        "(command next-error global)"
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\M-g ?p))"),
        "(command previous-error global)"
    );
}

// --- 13. has_async_work sees shell-command--procs and compile--procs ---

#[test]
fn has_async_work_true_for_shell_command_and_compile_procs() {
    let (mut i, _ed) = setup();
    assert!(!core::has_async_work(&mut i), "no jobs running yet");
    run(&mut i, "(setq shell-command--procs (list 1))");
    assert!(
        core::has_async_work(&mut i),
        "shell-command--procs must count as async work"
    );
    run(&mut i, "(setq shell-command--procs nil)");
    run(&mut i, "(setq compile--procs (list 1))");
    assert!(
        core::has_async_work(&mut i),
        "compile--procs must count as async work"
    );
    run(&mut i, "(setq compile--procs nil)");
}

// --- R1 (fix round): a very long output line must not crash the process,
// and must not poison parsing of the other, real error lines around it ---

#[test]
fn compile_survives_an_8192_char_line_and_still_parses_the_rest() {
    let scratch = Scratch::new("longline");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "a.v", "l1\nl2\n");
    write(&scratch, "b.v", "l1\nl2\nl3\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // A pathological 8192-char line that itself starts with something
    // that could look like a FILE:LINE:COL header IF the regex engine
    // backtracked through the whole thing (which is exactly what used
    // to crash the process pre-fix, R1) -- plus two ordinary short
    // error lines before and after it, to prove the long line doesn't
    // poison parsing of its neighbors.
    let long_junk = "x".repeat(8192);
    let cmd = format!(
        "printf '%s\\n' 'a.v:1: error: e1' 'top.sv:11: error: {}' 'b.v:2: error: e2'; true",
        long_junk
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(
        ok,
        "compile job never finished -- process must survive the long line"
    );
    // (b) no abort: reaching this line at all is the proof -- a
    // SIGABRT would have killed the whole test process before any
    // assertion below could run.
    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "2",
        "exactly the 2 real short errors must be parsed; the long line's \
         own fake \"top.sv:11: error: ...\" header DOES match \
         compile--error-prefix-regexp (its header is short -- only the \
         MESSAGE that follows is 8192 chars, and that part is now \
         `substring'-sliced, not regex-matched, so it costs nothing to \
         skip past), but \"top.sv\" was never created in this scratch \
         dir, so the existence check drops it: {}",
        n
    );
    let files = run(&mut i, "(mapcar (lambda (e) (aref e 0)) compile--errors)");
    assert!(
        files.contains("a.v") && files.contains("b.v") && !files.contains("top.sv"),
        "the long line's fake \"top.sv\" header must not have been parsed as an error: {}",
        files
    );
}

// --- R2 (fix round): COL=0 must not spill onto the PREVIOUS line -------

#[test]
fn compile_next_error_col_zero_clamps_to_current_line_start() {
    let scratch = Scratch::new("colzero");
    write(&scratch, "main.sv", "// nothing\n");
    // Line 2 is the target -- if COL=0 isn't clamped, `(1- 0)' = -1
    // would land point on the LAST character of line 1 instead.
    write(&scratch, "a.v", "line-one\nline-two\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'a.v:2:0: msg'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile-next-error");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "(line-number-at-pos)"),
        "2",
        "COL=0 must stay on line 2, not spill onto line 1"
    );
    assert_eq!(
        run(&mut i, "(current-column)"),
        "0",
        "COL=0 clamps to the line's own start"
    );
}

// --- R4 (fix round): `compile--severity''s cond ORDER is load-bearing --

#[test]
fn compile_severity_prefers_error_over_warning_when_message_has_both_keywords() {
    let scratch = Scratch::new("severity-order");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "x.v", "l1\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'x.v:1: warning: this error is ignorable'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");
    let severity = run(&mut i, "(aref (car compile--errors) 3)");
    assert_eq!(
        severity, "error",
        "a message containing both \"error\" and \"warning\" must classify \
         as 'error (checked first in compile--severity's cond) -- reverting \
         that cond's order to check 'warning first must turn this red: {}",
        severity
    );
}

// --- R1 (fix round), directly: a line whose "FILE" segment itself has no
// colon anywhere near `compile--line-prefix-limit' characters in must not
// crash and must be treated as ordinary output (no match) -----------------

#[test]
fn compile_parse_error_line_survives_long_colonless_prefix() {
    let (mut i, _ed) = setup();
    let src = "(compile--parse-error-line (make-string 8192 ?x) 1 \"/tmp\")";
    let r = run(&mut i, src);
    assert_eq!(
        r, "nil",
        "8192 chars with no colon at all must not match, and must not crash \
         reaching this assertion at all is the crash-survival proof: {}",
        r
    );
}

// --- R7/R8 (fix round, tail review): pin that `compile--line-prefix-
// limit' is the defense actually in effect for colon-free input --------
//
// Tail review disproved this file's earlier claim that dropping the
// trailing `.*' from `compile--error-prefix-regexp' was ALSO
// independent protection: run WITHOUT the prefix bound,
// `compile--error-prefix-regexp' alone still SIGABRTs against a
// colon-free string at N=2800/N=5000 (survives at N=2500) -- almost
// the exact same threshold as the original, unsplit pattern. The only
// thing standing between a colon-free line and that crash is
// `compile--line-prefix-limit' actually being applied before the
// regexp ever sees the string.
//
// IMPORTANT LIMITATION, stated plainly so nobody mistakes this test for
// more coverage than it has: this test does NOT touch
// `compile--line-prefix-limit' -- it exercises whatever the DEFAULT
// value already is. It therefore proves the limit is being applied at
// all (a regression that stopped applying it entirely, e.g. reverting
// `compile--parse-error-line' back to matching the whole LINE, would
// turn this red on a long enough input), but it does NOT prove the
// limit's VALUE is small enough to avoid the crash threshold -- if
// someone later raises `compile--line-prefix-limit' to, say, 100000,
// this test keeps passing unchanged (its input never gets anywhere
// near a value that large) while the real crash risk at that limit
// value would be very real. Catching THAT case is a mutation-testing
// job (raise the limit, feed colon-free input past the new, larger
// threshold, expect SIGABRT), not something this fixed-length unit
// test can pin -- see the mutation list for M80 for that scenario;
// it is deliberately not duplicated here.
#[test]
fn compile_colonless_5000_chars_survives_under_default_prefix_limit() {
    let (mut i, _ed) = setup();
    let src = "(compile--parse-error-line (make-string 5000 ?x) 1 \"/tmp\")";
    let r = run(&mut i, src);
    assert_eq!(
        r, "nil",
        "5000 colon-free chars under the DEFAULT compile--line-prefix-limit \
         must not match and must not crash: {}",
        r
    );
}

// --- R3's output cap: the implementer honestly flagged there was no
// dedicated test for it, so the coordinator added this one. ----------
//
// Uses a throttled producer (40 bytes / 20ms) instead of `yes`: M79
// already measured that a single `shell-process-poll` call drains the
// whole mpsc channel, so against a full-speed producer that one call
// can take unbounded time (see the comment on the same-named test in
// shell_command_tests.rs). That's a property of the shared primitive,
// not something this milestone's logic can fix.
#[test]
fn compile_output_cap_kills_process_and_reports_truncation() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(setq compile-max-output-chars 200)");
    do_compile(
        &mut i,
        &ed,
        "bash -c 'while true; do printf 0123456789012345678901234567890123456789; sleep 0.02; done'",
    );
    let ok = pump_until(&mut i, Duration::from_secs(10), no_compile_procs_running);
    assert!(
        ok,
        "capped compile job should be killed and reaped promptly"
    );
    let text = run(
        &mut i,
        "(with-current-buffer-internal \"*compilation*\" (lambda () (buffer-string)))",
    );
    assert!(
        text.contains("truncated"),
        "expected a truncation notice in *compilation*: {}",
        text
    );
}

// F3 (fix round): the truncation path (`compile-process-pending-all''s
// output-cap branch) must also report a dropped count, not just
// `compile--finish' -- drives the same cap as the test above, but with
// a header-shaped line naming a nonexistent file (error severity)
// emitted before the cap is crossed.
#[test]
fn compile_output_cap_truncation_message_reports_dropped_count() {
    let (mut i, ed) = setup();
    run(&mut i, "(get-buffer-create \"*scratch*\")");
    run(&mut i, "(switch-to-buffer-internal \"*scratch*\")");
    run(&mut i, "(setq compile-max-output-chars 200)");
    do_compile(
        &mut i,
        &ed,
        "bash -c 'printf \"missing.v:3: error: gone\\n\"; \
         while true; do printf 0123456789012345678901234567890123456789; sleep 0.02; done'",
    );
    let ok = pump_until(&mut i, Duration::from_secs(10), no_compile_procs_running);
    assert!(
        ok,
        "capped compile job should be killed and reaped promptly"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("truncated"),
        "truncation message must still say truncated: {:?}",
        echo
    );
    assert!(
        echo.contains("1 error line names a file that does not exist"),
        "truncation message must ALSO report the dropped count \
         (missing.v:3 never resolves to a real file): {:?}",
        echo
    );
}

// --- M130: vim-style j/k motion in compilation's local keymap -----------

#[test]
fn compilation_j_and_k_move_without_inserting() {
    let scratch = Scratch::new("jk_compile");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    run(&mut i, "(evil-mode 1)");
    do_compile(&mut i, &ed, "printf '%s\\n' one two three");
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    run(&mut i, "(switch-to-buffer-internal \"*compilation*\")");
    assert_eq!(
        run(&mut i, "evil--state"),
        "emacs",
        "*compilation* must start in evil's `emacs' state"
    );
    let before = run(&mut i, "(buffer-string)");
    run(&mut i, "(goto-char (point-min))");
    let line0 = run(&mut i, "(line-number-at-pos)");
    feed_keys(&mut i, &ed, "j").unwrap();
    let line1 = run(&mut i, "(line-number-at-pos)");
    assert_ne!(line1, line0, "j must move down a line");
    feed_keys(&mut i, &ed, "k").unwrap();
    assert_eq!(
        run(&mut i, "(line-number-at-pos)"),
        line0,
        "k must move back up to the original line"
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before,
        "j/k must never insert text into *compilation*"
    );
    // M130 fix round FIX-4: `*compilation*' is a plain scrolling-output
    // text buffer (no trailing row past the last line of real content
    // the way dired/search-mode have -- see those two modes' own `G'
    // fix comments), so plain `end-of-buffer' semantics are correct
    // here and need no special-case landing function. Still worth its
    // own assertion: nothing else in this test presses `G' at all, so a
    // regression that dropped or broke the binding would go undetected.
    feed_keys(&mut i, &ed, "G").unwrap();
    assert_eq!(
        run(&mut i, "(point)"),
        run(&mut i, "(point-max)"),
        "G must move point to the end of *compilation*"
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before,
        "G must never insert text into *compilation*"
    );
}

// --- M141: `M-x compile' follows a build's own directory changes --------
//
// T1: gmake-style "Entering directory '...'" / "Leaving directory '...'"
// (plain apostrophe both ends) around an error line whose FILE only
// exists under the announced subdirectory.

#[test]
fn compile_follows_gmake_entering_directory_apostrophe_style() {
    let scratch = Scratch::new("dirtrack_gmake");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "sub/bad.v", "line1\nline2\nsyntax bad\n");
    let sub = scratch.join("sub");
    let sub_str = sub.to_str().unwrap();
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = format!(
        "printf '%s\\n' \"Entering directory '{0}'\" 'bad.v:3: syntax error' \"Leaving directory '{0}'\"; true",
        sub_str
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "1",
        "expected bad.v:3 to resolve under the announced sub directory: {}",
        n
    );
    let expected_file = format!("\"{}\"", sub.join("bad.v").to_str().unwrap());
    let file = run(&mut i, "(aref (car compile--errors) 0)");
    assert_eq!(
        file, expected_file,
        "bad.v must resolve against the announced 'Entering directory' path, not the job dir: {}",
        file
    );

    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "compile-next-error");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-name)"),
        "\"bad.v\"",
        "M-g n must actually visit sub/bad.v"
    );
}

// T2: macOS make 3.81 `-w' style -- backquote-open, apostrophe-close.

#[test]
fn compile_follows_make_entering_directory_backquote_style() {
    let scratch = Scratch::new("dirtrack_backquote");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "sub/bad.v", "line1\nline2\nsyntax bad\n");
    let sub = scratch.join("sub");
    let sub_str = sub.to_str().unwrap();
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = format!(
        "printf '%s\\n' \"make[1]: Entering directory \\`{0}'\" 'bad.v:3: syntax error' \"make[1]: Leaving directory \\`{0}'\"; true",
        sub_str
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(n, "1", "backquote-open style must also be tracked: {}", n);
    let expected_file = format!("\"{}\"", sub.join("bad.v").to_str().unwrap());
    let file = run(&mut i, "(aref (car compile--errors) 0)");
    assert_eq!(file, expected_file, "wrong resolved file: {}", file);
}

// T3: nested make -C, real files at each level -- pins reticle's own
// (GNU-diverging) directory-stack resolution rule for every entry A-F
// from the task spec's synthetic log.

#[test]
fn compile_tracks_nested_directory_changes_with_real_files_at_each_level() {
    let scratch = Scratch::new("dirtrack_nested");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "top.v", "line1\n");
    write(&scratch, "sim/s.v", "line1\n");
    write(&scratch, "sim/deep/d.v", "line1\n");
    let root_str = scratch.to_str().unwrap();
    let sim = scratch.join("sim");
    let sim_str = sim.to_str().unwrap();
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = format!(
        "printf '%s\\n' \
         \"Entering directory '{root}'\" \
         'top.v:1: error: A' \
         \"Entering directory '{sim}'\" \
         's.v:1: error: B' \
         \"Entering directory 'deep'\" \
         'd.v:1: error: C' \
         \"Leaving directory 'deep'\" \
         's.v:1: error: D' \
         \"Leaving directory '{sim}'\" \
         'top.v:1: error: E' \
         \"Leaving directory '{root}'\" \
         'top.v:1: error: F'; true",
        root = root_str,
        sim = sim_str,
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(n, "6", "expected all 6 entries A-F to parse: {}", n);

    let p_top = format!("\"{}\"", scratch.join("top.v").to_str().unwrap());
    let p_s = format!("\"{}\"", scratch.join("sim/s.v").to_str().unwrap());
    let p_d = format!("\"{}\"", scratch.join("sim/deep/d.v").to_str().unwrap());
    let expected = [&p_top, &p_s, &p_d, &p_s, &p_top, &p_top];
    for (idx, exp) in expected.iter().enumerate() {
        let got = run(&mut i, &format!("(aref (nth {} compile--errors) 0)", idx));
        assert_eq!(
            &got, *exp,
            "entry {} (A-F, 0-based) resolved to the wrong directory: got {}, want {}",
            idx, got, exp
        );
    }
}

// T4: `Leaving' with an empty stack is a no-op, not an error, and
// directory tracking still works afterward -- proven by an `Entering'
// AFTER the stray `Leaving' whose error's file exists ONLY under the
// announced subdirectory (fix round F4: the original fixture put its
// error file in the JOB dir, which an entirely-deleted M141 would also
// resolve correctly, so it passed with the whole feature removed).
//
// The `(when stack (setq stack (cdr stack)))' guard around the pop is
// not itself observable in this interpreter: `(cdr nil)' is `nil'
// (`crates/elisp/src/value.rs:241-246'), so removing the `when' guard
// entirely changes no behavior -- `(setq stack (cdr stack))' against an
// empty STACK already leaves it `nil'. What this test actually pins is
// that a stray `Leaving' doesn't error/hang the job and that later
// `Entering'/error lines are still processed normally.

#[test]
fn compile_leaving_with_empty_stack_is_a_no_op() {
    let scratch = Scratch::new("dirtrack_empty_leave");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "sub/a.v", "line1\n");
    let sub = scratch.join("sub");
    let sub_str = sub.to_str().unwrap();
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = format!(
        "printf '%s\\n' \"Leaving directory '/somewhere/else'\" \"Entering directory '{0}'\" 'a.v:1: error: e1'; true",
        sub_str
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(
        ok,
        "compile job never finished -- a Leaving with empty stack must not error"
    );
    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(n, "1", "the error line after the stray Leaving: {}", n);
    let expected_file = format!("\"{}\"", sub.join("a.v").to_str().unwrap());
    let file = run(&mut i, "(aref (car compile--errors) 0)");
    assert_eq!(
        file, expected_file,
        "must resolve under sub/ (the Entering announced AFTER the stray \
         Leaving) -- proves tracking still works after the empty pop: {}",
        file
    );
}

// F1 (fix round): a real error line whose MESSAGE happens to contain the
// text "Entering directory '...'" must not be swallowed as a directory
// announcement -- the error header regexp must be tried FIRST, and only
// a line that does NOT match it may be treated as a directory line.

#[test]
fn compile_error_line_containing_entering_directory_text_is_not_swallowed() {
    let scratch = Scratch::new("f1_header_first");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "bad.v", "l1\nl2\nl3\n");
    write(&scratch, "a2.v", "l1\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd =
        "printf '%s\\n' \"bad.v:3: error: while Entering directory 'x'\" 'a2.v:1: error: e2'; true"
            .to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "2",
        "both lines are real errors -- the first must NOT be treated as a \
         directory-change announcement just because its MESSAGE contains \
         the words \"Entering directory '...'\": {}",
        n
    );
    let expected_bad = format!("\"{}\"", scratch.join("bad.v").to_str().unwrap());
    let expected_a2 = format!("\"{}\"", scratch.join("a2.v").to_str().unwrap());
    assert_eq!(
        run(&mut i, "(aref (nth 0 compile--errors) 0)"),
        expected_bad,
        "first entry must resolve against the job dir"
    );
    assert_eq!(
        run(&mut i, "(aref (nth 1 compile--errors) 0)"),
        expected_a2,
        "second entry must ALSO resolve against the job dir -- the false \
         directory match must not have pushed \"x\" onto the stack"
    );
}

// F2 (fix round): a timestamp-shaped line ("14:23:01: warning: ...")
// matches the FILE:LINE:COL header with FILE="14" -- an all-digit FILE
// that will (almost) never exist on disk must not inflate the dropped
// count.

#[test]
fn compile_timestamp_line_does_not_inflate_dropped_count() {
    let scratch = Scratch::new("f2_timestamp");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' '14:23:01: warning: disk usage high'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    assert_eq!(
        run(&mut i, "(length compile--errors)"),
        "0",
        "\"14\" does not exist as a file, so no entry should be created"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        !echo.contains("error line") && !echo.contains("error lines"),
        "a timestamp-shaped line must NOT be counted as a dropped error line: {:?}",
        echo
    );
}

// F7 (fix round): a dropped `sorry:' line (iverilog's unsupported-
// construct error, which stops the build) must be counted, same as
// 'error/'warning -- 'note is the only severity that stays uncounted.

#[test]
fn compile_dropped_sorry_line_is_counted() {
    let scratch = Scratch::new("f7_sorry");
    write(&scratch, "main.sv", "// nothing\n");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'missing.sv:35: sorry: not yet supported'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    assert_eq!(
        run(&mut i, "(length compile--errors)"),
        "0",
        "missing.sv does not exist, so no entry should be created"
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("1 error line names a file that does not exist"),
        "a dropped 'sorry line must be counted just like 'error/'warning: {:?}",
        echo
    );
}

// F8 (fix round): CRLF line endings -- a directory-announcement line
// ending in \r (before the buffer's own \n split) must still match
// `compile--directory-regexp''s `$' anchor.

#[test]
fn compile_follows_directory_change_with_crlf_line_endings() {
    let scratch = Scratch::new("f8_crlf");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "sub/bad.v", "line1\nline2\nsyntax bad\n");
    let sub = scratch.join("sub");
    let sub_str = sub.to_str().unwrap();
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    // `printf' emits a literal \r before each \n on the Entering/Leaving
    // lines only -- the error line itself stays plain \n, matching a
    // tool that CRLF-terminates its own directory chatter but not
    // necessarily every line (the worse case: if it did, the error
    // header regexp has no end-of-line anchor at all, so \r just joins
    // MESSAGE harmlessly, already true before this fix).
    let cmd = format!(
        "printf 'Entering directory %s\\r\\n' \"'{0}'\" && \
         printf '%s\\n' 'bad.v:3: syntax error' && \
         printf 'Leaving directory %s\\r\\n' \"'{0}'\"; true",
        sub_str
    );
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "1",
        "CRLF Entering/Leaving lines must still be tracked: {}",
        n
    );
    let expected_file = format!("\"{}\"", sub.join("bad.v").to_str().unwrap());
    let file = run(&mut i, "(aref (car compile--errors) 0)");
    assert_eq!(file, expected_file, "wrong resolved file: {}", file);
}

// T5: unannounced cd (plain `cd sub && tool', or make without `-w') is
// still dropped (this file's own header names it as the remaining known
// gap), but no longer silently -- the finish message must name the
// count. A second dropped-looking line whose message classifies as
// NONE of 'error/'warning/'sorry (`compile--severity': checks
// \berror\b, \bwarning\b, \bsorry\b in order, 'note catch-all) must NOT
// be counted -- 'sorry IS counted (fix round F7, see the dedicated
// `compile_dropped_sorry_line_is_counted' test above), only 'note stays
// uncounted.

#[test]
fn compile_unannounced_directory_change_is_dropped_but_counted_in_message() {
    let scratch = Scratch::new("dirtrack_unannounced");
    write(&scratch, "main.sv", "// nothing\n");
    // Neither bad.v nor foo.v exists anywhere in the scratch tree --
    // both header-match but both fail the existence check.
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = "printf '%s\\n' 'bad.v:3: syntax error' 'foo.v:3: note text'; true".to_string();
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(5), no_compile_procs_running);
    assert!(ok, "compile job never finished");

    assert_eq!(
        run(&mut i, "(length compile--errors)"),
        "0",
        "both lines' files are missing, so compile--errors must be empty"
    );
    // Confirm what the classifier actually does with "note text": it
    // matches none of \berror\b/\bwarning\b/\bsorry\b, so it falls into
    // the 'note catch-all -- verified directly via compile--severity
    // rather than assumed, per this milestone's own instructions.
    assert_eq!(
        run(&mut i, "(compile--severity \"note text\")"),
        "note",
        "\"note text\" must classify as 'note (no error/warning/sorry keyword), \
         so it must NOT be counted in the dropped total"
    );

    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("1 error line names a file that does not exist"),
        "finish message must report exactly 1 dropped line (bad.v's \
         \"syntax error\" classifies as 'error; foo.v's \"note text\" \
         does not count): {:?}",
        echo
    );
}

// T6: a real `make -w -C sub' run -- the directory-tracking fix must
// also work against a real make binary, not just synthetic printf
// output shaped like one.

#[test]
fn compile_follows_real_make_dash_c_directory_change() {
    let skip_env = std::env::var("RETICLE_SKIP_MAKE_TESTS").unwrap_or_default();
    let skip_requested = matches!(skip_env.as_str(), "1" | "true" | "yes");
    let make_available = std::process::Command::new("make")
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false);
    if !make_available {
        assert!(
            skip_requested,
            "`make` not found on PATH -- required by \
             compile_follows_real_make_dash_c_directory_change; set \
             RETICLE_SKIP_MAKE_TESTS=1/true/yes to skip this test deliberately"
        );
        return;
    }

    let scratch = Scratch::new("dirtrack_real_make");
    write(&scratch, "main.sv", "// nothing\n");
    write(&scratch, "sub/bad.v", "line1\nline2\nline3\n");
    write(
        &scratch,
        "sub/Makefile",
        "all:\n\tprintf 'bad.v:3: syntax error\\n' >&2; exit 1\n",
    );
    let sub = scratch.join("sub");
    let (mut i, ed) = setup();
    visit(&mut i, &scratch.join("main.sv"));
    let cmd = format!("make -w -C {}", sub.to_str().unwrap());
    do_compile(&mut i, &ed, &cmd);
    let ok = pump_until(&mut i, Duration::from_secs(10), no_compile_procs_running);
    assert!(ok, "real make job never finished");

    let n = run(&mut i, "(length compile--errors)");
    assert_eq!(
        n, "1",
        "expected the real make -C's directory announcement to be tracked: {}",
        n
    );
    // Real `make` calls `getcwd()` when reporting the directory it
    // entered, which on macOS resolves the `/var` -> `/private/var'
    // symlink `std::env::temp_dir()` itself leaves unresolved -- so the
    // expected path must be canonicalized the same way, not built from
    // `sub` directly (that comparison is exercised, unresolved, by the
    // synthetic-log tests above; this test's own point is proving a
    // REAL make binary's announcement is followed at all).
    let real_sub_bad_v = std::fs::canonicalize(sub.join("bad.v")).unwrap();
    let expected_file = format!("\"{}\"", real_sub_bad_v.to_str().unwrap());
    let file = run(&mut i, "(aref (car compile--errors) 0)");
    assert_eq!(file, expected_file, "wrong resolved file: {}", file);
}
