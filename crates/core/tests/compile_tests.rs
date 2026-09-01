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
