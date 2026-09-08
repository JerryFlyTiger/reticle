//! M104: style-selectable, save-time code formatting.
//!
//! Two layers under test:
//!   - `call-process-string` (elisp/src/shell.rs): the synchronous
//!     "feed stdin, collect stdout/stderr" Rust builtin, tested directly
//!     against real `/bin/*` tools -- the same technique
//!     `shell_command_tests.rs` uses for `start-shell-process`.
//!   - `format.el`: the elisp dispatch layer on top of it, tested with
//!     `/bin/cat`/`sed`/`sh` standing in for real formatters (so these
//!     tests don't depend on `verible-verilog-format`/`clang-format`
//!     being installed), plus one `fset`-faked LSP dispatch test
//!     mirroring `lsp_format_tests.rs`'s convention.
//!
//! Item 13 (three Verilog styles genuinely producing different output)
//! is gated on `verible-verilog-format` actually being on PATH, same
//! `have_on_path` technique as `lsp_format_tests.rs`/`lsp_mode_tests.rs`
//! (this file keeps its own copy, per this repo's no-shared-test-helpers
//! convention).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::{Interp, Value};

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

fn ok(interp: &mut Interp, src: &str) -> String {
    let r = run(interp, src);
    assert!(!r.starts_with("ERROR"), "{:?} failed: {}", src, r);
    r
}

fn bs(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-string)") {
        Ok(Value::Str(s)) => (*s).clone(),
        Ok(v) => panic!(
            "(buffer-string) didn't return a string: {}",
            prin1_to_string(interp, &v)
        ),
        Err(flow) => panic!("(buffer-string) errored: {}", interp.describe_flow(&flow)),
    }
}

/// Stubs `message' to accumulate every call (most recent first) into
/// `test--messages' -- same convention as `lsp_format_tests.rs's
/// `stub_message'.
fn stub_message(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

/// Whether CMD resolves to a real executable on PATH -- same technique
/// as `lsp_format_tests.rs'/`lsp_mode_tests.rs's `have_on_path` (spawn
/// with a harmless flag, kill immediately; this file keeps its own copy
/// per this repo's no-shared-test-helpers convention).
fn have_on_path(cmd: &str) -> bool {
    match std::process::Command::new(cmd)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            let _ = child.kill();
            let _ = child.wait();
            true
        }
        Err(_) => false,
    }
}

// =====================================================================
// `call-process-string` -- direct Rust builtin tests.
// =====================================================================

#[test]
fn call_process_string_cat_echoes_input_back_with_exit_zero() {
    let (mut i, _ed) = setup();
    let result = ok(
        &mut i,
        "(call-process-string \"/bin/cat\" nil \"hello there\")",
    );
    assert_eq!(result, "(0 \"hello there\" \"\")");
}

#[test]
fn call_process_string_reports_nonzero_exit() {
    let (mut i, _ed) = setup();
    let result = ok(
        &mut i,
        "(call-process-string \"/bin/sh\" (list \"-c\" \"exit 3\") \"\")",
    );
    assert_eq!(result, "(3 \"\" \"\")");
}

#[test]
fn call_process_string_collects_stderr_separately_from_stdout() {
    let (mut i, _ed) = setup();
    let result = ok(
        &mut i,
        "(call-process-string \"/bin/sh\" (list \"-c\" \"echo out; echo err >&2\") \"\")",
    );
    assert_eq!(result, "(0 \"out\\n\" \"err\\n\")");
}

/// `/bin/cat` blocks reading stdin until it sees EOF. If
/// `call-process-string` failed to close the write end after writing
/// INPUT, this would hang until the default 5000ms timeout and return
/// exit code -1 instead of 0 -- so a fast, exit-0 result here is exactly
/// what proves the pipe got closed.
#[test]
fn call_process_string_closes_stdin_so_cat_does_not_hang() {
    let (mut i, _ed) = setup();
    let start = Instant::now();
    let result = ok(&mut i, "(call-process-string \"/bin/cat\" nil \"abc\")");
    assert_eq!(result, "(0 \"abc\" \"\")");
    assert!(
        start.elapsed().as_secs() < 1,
        "cat should have returned almost instantly, took {:?}",
        start.elapsed()
    );
}

#[test]
fn call_process_string_timeout_kills_the_child_and_returns_minus_one_quickly() {
    let (mut i, _ed) = setup();
    let start = Instant::now();
    let result = ok(
        &mut i,
        "(call-process-string \"/bin/sh\" (list \"-c\" \"sleep 30\") \"\" 200)",
    );
    let elapsed = start.elapsed();
    assert!(
        result.starts_with("(-1 "),
        "expected exit code -1 on timeout, got {:?}",
        result
    );
    assert!(
        elapsed.as_secs() < 1,
        "a killed child should not take anywhere near the full 30s sleep, took {:?}",
        elapsed
    );
}

/// M104 fix round (cold-review defect): `/bin/sh -c "sleep 30"` execs
/// straight into `sleep` (confirmed with `ps` -- no fork happens), so it
/// can't tell "kill the whole process GROUP" apart from "kill only the
/// direct child" -- removing the `-pid` negation in `call_process_string`
/// wouldn't turn any existing test red.
///
/// This instead backgrounds a SEPARATE subshell (`(sleep 2; touch
/// MARKER) &`) before sh settles into its own long-running foreground
/// `sleep 30` (which is what actually keeps sh alive long enough for
/// the 100ms timeout below to fire while it's still running). Manually
/// verified with `ps`/`os.kill` before writing this (see the M104 fix
/// round report): killing ONLY sh's own pid leaves that backgrounded
/// subshell alive as a reparented orphan (own pid, own process image --
/// it doesn't need sh to still be alive to finish running `sleep 2;
/// touch MARKER` inside itself), and the marker DOES appear about 2
/// seconds later; killing the whole process group (`kill(-pgid, ...)`)
/// takes the orphan down too, and the marker never appears. An earlier
/// version of this test used `sleep 1 && touch MARKER` with no
/// backgrounding, which does NOT distinguish the two: `touch` there is
/// the SECOND half of a script `&&`-chained inside the very sh process
/// we kill, so once sh itself dies (which happens either way, since
/// sh's own pid is always in the signal's target set), nothing is left
/// alive to run `touch` regardless of whether `sleep` survived --
/// confirmed empirically to give the same (wrong, non-discriminating)
/// answer under both a real group-kill and a same-pid-only kill.
#[test]
fn call_process_string_timeout_kills_the_whole_process_group_not_just_the_direct_child() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_pgroup_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("survived");
    let cmd = format!("(sleep 2; touch {}) & sleep 30", marker.to_str().unwrap());

    ok(
        &mut i,
        &format!(
            "(call-process-string \"/bin/sh\" (list \"-c\" {:?}) \"\" 100)",
            cmd
        ),
    );
    // The backgrounded subshell would finish and `touch` the marker
    // about 2 seconds after spawn if it survived the kill; wait
    // comfortably past that before checking.
    std::thread::sleep(std::time::Duration::from_millis(3000));
    assert!(
        !marker.exists(),
        "the backgrounded subshell must have been killed along with its sh \
         parent's whole process group, but the marker file exists -- \
         process-group kill did not reach it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn call_process_string_missing_program_returns_nonzero_with_stderr_not_an_error() {
    let (mut i, _ed) = setup();
    ok(
        &mut i,
        "(setq test--r (call-process-string \"no-such-program-xyz-m104\" nil \"\"))",
    );
    assert_ne!(
        run(&mut i, "(nth 0 test--r)"),
        "0",
        "spawn failure must be reported as a non-zero exit code"
    );
    assert_eq!(
        run(&mut i, "(> (length (nth 2 test--r)) 0)"),
        "t",
        "stderr should be non-empty when the program can't even be spawned"
    );
}

// =====================================================================
// `format.el` -- provider dispatch, style tables, save integration.
// =====================================================================

/// Registers a throwaway major mode `format-test-mode' with an external
/// provider `(PROGRAM . ARGS-FUNCTION)`, so these tests don't depend on
/// any real formatter being installed.
fn use_external_test_mode(i: &mut Interp, program: &str, args_fn_body: &str) {
    ok(i, "(major-mode-internal-set 'format-test-mode)");
    ok(
        i,
        &format!("(defun format-test--args (style) {})", args_fn_body),
    );
    ok(
        i,
        &format!(
            "(setq format-provider-alist (format--alist-put format-provider-alist 'format-test-mode (cons {:?} 'format-test--args)))",
            program
        ),
    );
}

#[test]
fn format_buffer_external_provider_no_change_reports_already_formatted() {
    let (mut i, _ed) = setup();
    use_external_test_mode(&mut i, "/bin/cat", "nil");
    stub_message(&mut i);
    ok(&mut i, "(insert \"unchanged text\\n\")");
    ok(&mut i, "(format-buffer)");
    assert_eq!(bs(&mut i), "unchanged text\n");
    assert_eq!(run(&mut i, "(car test--messages)"), "\"Already formatted\"");
}

#[test]
fn format_buffer_external_provider_applies_changed_output() {
    let (mut i, _ed) = setup();
    // `sed` rewrites "old" to "NEW" -- proves the formatter's stdout,
    // not the original buffer text, ends up in the buffer.
    use_external_test_mode(&mut i, "sed", "(list \"s/old/NEW/\")");
    ok(&mut i, "(insert \"the old value\\n\")");
    ok(&mut i, "(format-buffer)");
    assert_eq!(bs(&mut i), "the NEW value\n");
}

#[test]
fn format_buffer_external_provider_failure_leaves_buffer_untouched_and_messages() {
    let (mut i, _ed) = setup();
    use_external_test_mode(
        &mut i,
        "/bin/sh",
        "(list \"-c\" \"cat >/dev/null; echo boom trouble >&2; exit 1\")",
    );
    stub_message(&mut i);
    ok(&mut i, "(insert \"do not touch me\\n\")");
    ok(&mut i, "(format-buffer)");
    assert_eq!(
        bs(&mut i),
        "do not touch me\n",
        "a non-zero exit must leave the buffer exactly as it was"
    );
    assert!(
        run(
            &mut i,
            "(string-search \"boom trouble\" (car test--messages))"
        ) != "nil",
        "expected the failure message to include stderr, got {:?}",
        run(&mut i, "test--messages")
    );
}

/// `replace-region-contents` (not delete+insert) is what makes this
/// pass: only the last line actually changes, so point sitting on line
/// 1 must not move at all.
#[test]
fn format_buffer_uses_minimal_edit_point_on_untouched_line_does_not_move() {
    let (mut i, _ed) = setup();
    use_external_test_mode(&mut i, "sed", "(list \"s/last/CHANGED/\")");
    ok(&mut i, "(insert \"line one\\nline two\\nlast line\\n\")");
    ok(&mut i, "(goto-char (point-min))");
    let before = ok(&mut i, "(point)");
    ok(&mut i, "(format-buffer)");
    assert_eq!(bs(&mut i), "line one\nline two\nCHANGED line\n");
    let after = ok(&mut i, "(point)");
    assert_eq!(
        before, after,
        "point on the untouched first line must not move"
    );
}

#[test]
fn format_buffer_no_provider_configured_is_a_message_only_noop() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");
    stub_message(&mut i);
    ok(&mut i, "(insert \"plain text\\n\")");
    ok(&mut i, "(format-buffer)");
    assert_eq!(bs(&mut i), "plain text\n");
    assert!(
        run(&mut i, "(> (length test--messages) 0)") == "t",
        "expected a message explaining nothing is configured"
    );
}

#[test]
fn format_region_external_provider_is_unsupported_and_leaves_buffer_untouched() {
    let (mut i, _ed) = setup();
    use_external_test_mode(&mut i, "sed", "(list \"s/x/y/\")");
    stub_message(&mut i);
    ok(&mut i, "(insert \"xxxx\\n\")");
    ok(&mut i, "(set-mark (point-min))");
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(format-region (region-beginning) (region-end))");
    assert_eq!(bs(&mut i), "xxxx\n");
    assert!(
        run(&mut i, "(string-search \"region\" (car test--messages))") != "nil",
        "expected a message saying region formatting isn't supported, got {:?}",
        run(&mut i, "test--messages")
    );
}

// --- Style tables: pure-function assertions on `format--verible-args' ---

#[test]
fn verible_args_verible_style_is_indentation_only() {
    let (mut i, _ed) = setup();
    assert_eq!(
        ok(&mut i, "(format--verible-args 'verible)"),
        "(\"--indentation_spaces=2\" \"-\")"
    );
}

#[test]
fn verible_args_aligned_style_sets_every_alignment_flag_to_align() {
    let (mut i, _ed) = setup();
    assert_eq!(
        ok(&mut i, "(format--verible-args 'aligned)"),
        "(\"--indentation_spaces=2\" \"--port_declarations_alignment=align\" \
         \"--named_port_alignment=align\" \"--named_parameter_alignment=align\" \
         \"--parameter_declaration_alignment=align\" \"--assignment_statement_alignment=align\" \
         \"--case_items_alignment=align\" \"--module_net_variable_alignment=align\" \"-\")"
    );
}

#[test]
fn verible_args_compact_style_sets_flush_left_and_widens_column_limit() {
    let (mut i, _ed) = setup();
    assert_eq!(
        ok(&mut i, "(format--verible-args 'compact)"),
        "(\"--indentation_spaces=2\" \"--column_limit=120\" \
         \"--port_declarations_alignment=flush-left\" \"--named_port_alignment=flush-left\" \
         \"--named_parameter_alignment=flush-left\" \"--parameter_declaration_alignment=flush-left\" \
         \"--assignment_statement_alignment=flush-left\" \"--case_items_alignment=flush-left\" \
         \"--module_net_variable_alignment=flush-left\" \"-\")"
    );
}

#[test]
fn verible_args_three_styles_are_pairwise_different() {
    let (mut i, _ed) = setup();
    let verible = ok(&mut i, "(format--verible-args 'verible)");
    let aligned = ok(&mut i, "(format--verible-args 'aligned)");
    let compact = ok(&mut i, "(format--verible-args 'compact)");
    assert_ne!(verible, aligned);
    assert_ne!(verible, compact);
    assert_ne!(aligned, compact);
}

// --- `format--clang-args' -- pure-function assertions on `-assume-filename' ---

/// M104 mutation round (SURVIVED 2): `manual_e2e_clang_format_style_
/// file_finds_the_projects_own_clang_format' also passes `call-process-
/// string' the buffer's own directory as its DIR argument (a SEPARATE
/// defense, in `format--run-external'), so clang-format finds
/// `.clang-format' via its own CWD search regardless of whether
/// `-assume-filename' is present at all -- removing `-assume-filename'
/// alone does not turn that end-to-end test red. This isolates just the
/// `-assume-filename' half with a direct, pure-function assertion.
#[test]
fn clang_args_includes_assume_filename_when_buffer_has_a_file() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_clang_args_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.c");
    std::fs::write(&path, "int main() {}\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    let args = ok(&mut i, "(format--clang-args 'file)");
    let expected_flag = format!("-assume-filename={}", path.to_str().unwrap());
    assert!(
        run(
            &mut i,
            &format!("(member {:?} (format--clang-args 'file))", expected_flag)
        ) != "nil",
        "expected {:?} in {}",
        expected_flag,
        args
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn clang_args_omits_assume_filename_when_buffer_has_no_file() {
    let (mut i, _ed) = setup();
    // A fresh scratch buffer never visited a file, so `(buffer-file-
    // name)' is nil.
    let args = ok(&mut i, "(format--clang-args 'file)");
    assert!(
        !args.contains("-assume-filename"),
        "expected no -assume-filename flag when the buffer has no file, got {}",
        args
    );
}

// --- `format-set-style' ---

#[test]
fn format_set_style_updates_the_style_alist_and_messages() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    stub_message(&mut i);
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback \"aligned\")))",
    );
    ok(&mut i, "(format-set-style)");
    assert_eq!(
        run(&mut i, "(cdr (assq 'verilog-mode format-style-alist))"),
        "aligned"
    );
    assert!(
        run(&mut i, "(string-search \"aligned\" (car test--messages))") != "nil",
        "expected the new style to be named in the message, got {:?}",
        run(&mut i, "test--messages")
    );
    // And the args-function for verilog-mode now reflects the new style.
    assert_eq!(
        ok(
            &mut i,
            "(format--verible-args (cdr (assq 'verilog-mode format-style-alist)))"
        ),
        ok(&mut i, "(format--verible-args 'aligned)")
    );
}

/// M104 fix round (cold-review coverage gap): `format-style-alist''s
/// own docstring claims it's GLOBAL, keyed by mode, not buffer-local --
/// but nothing had ever actually looked at it from a SECOND buffer.
/// Changing `setq' to `setq-local' in `format-set-style' would not have
/// turned any pre-existing test red.
#[test]
fn format_set_style_is_visible_from_a_second_buffer_of_the_same_mode() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(switch-to-buffer-internal \"fmt-style-buf-a\")");
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    ok(&mut i, "(switch-to-buffer-internal \"fmt-style-buf-b\")");
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");

    // Back to buffer A: pick "aligned" there.
    ok(&mut i, "(switch-to-buffer-internal \"fmt-style-buf-a\")");
    ok(
        &mut i,
        "(fset 'completing-read
               (lambda (prompt collection callback require-match &optional initial)
                 (funcall callback \"aligned\")))",
    );
    ok(&mut i, "(format-set-style)");
    assert_eq!(
        run(&mut i, "(cdr (assq 'verilog-mode format-style-alist))"),
        "aligned"
    );

    // Buffer B never ran `format-set-style' itself -- if the style had
    // been written `setq-local' instead of `setq', it would still read
    // whatever verilog-mode's default was (`verible'), not the new
    // buffer-A-set value.
    ok(&mut i, "(switch-to-buffer-internal \"fmt-style-buf-b\")");
    assert_eq!(
        run(&mut i, "(cdr (assq 'verilog-mode format-style-alist))"),
        "aligned",
        "format-style-alist must be visible from a second buffer of the same mode"
    );
}

#[test]
fn format_set_style_no_choices_for_mode_is_message_only() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");
    stub_message(&mut i);
    ok(&mut i, "(format-set-style)");
    assert!(
        run(&mut i, "(> (length test--messages) 0)") == "t",
        "expected a message explaining no style choices exist"
    );
}

// --- Format on save ---

#[test]
fn format_on_save_formats_before_writing_to_disk() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_on_save_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "the old value\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    use_external_test_mode(&mut i, "sed", "(list \"s/old/NEW/\")");
    ok(&mut i, "(save-buffer)");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "the NEW value\n");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn format_on_save_disabled_leaves_content_unformatted() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_on_save_off_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "the old value\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    use_external_test_mode(&mut i, "sed", "(list \"s/old/NEW/\")");
    ok(&mut i, "(setq format-on-save nil)");
    ok(&mut i, "(save-buffer)");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(on_disk, "the old value\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// The single most important formatting-on-save property: a formatter
/// crash/failure must never block the save itself.
#[test]
fn format_on_save_formatter_failure_still_saves_the_file() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_on_save_fail_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "original\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    use_external_test_mode(
        &mut i,
        "/bin/sh",
        "(list \"-c\" \"cat >/dev/null; exit 1\")",
    );
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"appended\\n\")");
    ok(&mut i, "(save-buffer)");

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk, "original\nappended\n",
        "the save must go through even though the formatter failed"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// M104 fix round (cold-review coverage gap): every existing failure
/// path exercised so far is an external PROCESS exiting non-zero, which
/// only ever reaches `message' -- nothing had ever made `format-buffer'
/// itself signal an elisp `error' (e.g. a bug inside an ARGS-FUNCTION),
/// so `format--maybe-on-save''s `condition-case' had no test proving it
/// actually catches anything. Here the ARGS-FUNCTION itself signals
/// before `call-process-string' is ever invoked -- the most direct way
/// to make `format-buffer' raise a real elisp error rather than just
/// message one.
#[test]
fn format_on_save_args_function_error_does_not_block_the_save() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_on_save_signal_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "original\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    use_external_test_mode(&mut i, "/bin/cat", "(error \"boom from args-fn\")");
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"appended\\n\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(
        !r.starts_with("ERROR"),
        "save-buffer must not propagate the ARGS-FUNCTION's error, got {:?}",
        r
    );

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk, "original\nappended\n",
        "the save must go through even though the ARGS-FUNCTION signaled an error"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// M104 mutation round (SURVIVED 1): `before-save-hook''s own runner
/// (`run_hook_by_name_with_arg', commands.rs) already catches any error
/// a hook function signals and just echoes it -- it never lets one
/// escape to `save-buffer' itself. That means the test above, which
/// goes through a real `(save-buffer)', cannot tell whether
/// `format--maybe-on-save''s OWN `condition-case' is even still there:
/// removing it entirely does not turn that test red, because the hook
/// runner's safety net silently covers for it. This calls
/// `format--maybe-on-save' directly (bypassing `save-buffer'/the hook
/// runner entirely) and asserts it returns normally rather than
/// signaling -- removing its `condition-case' turns this into
/// `signalled', while the test above stays green either way.
#[test]
fn format_maybe_on_save_itself_does_not_signal_when_the_args_function_errors() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_maybe_on_save_signal_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "original\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    use_external_test_mode(&mut i, "/bin/cat", "(error \"boom from args-fn\")");
    ok(&mut i, "(insert \"more\n\")");

    let result = ok(
        &mut i,
        "(condition-case e
           (progn (format--maybe-on-save) 'returned-normally)
           (error 'signalled))",
    );
    assert_eq!(
        result, "returned-normally",
        "format--maybe-on-save's own condition-case must catch the ARGS-FUNCTION's error \
         itself, independent of before-save-hook's own runner also catching it"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn format_on_save_non_target_mode_is_a_true_noop() {
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_on_save_noop_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "plain\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");
    stub_message(&mut i);
    // Call the hook function directly rather than `(save-buffer)` --
    // `save-buffer` itself unconditionally messages "Wrote FILE", which
    // would drown out the one thing this test cares about: whether
    // `format--maybe-on-save' itself does anything at all for a mode
    // with no formatter configured.
    ok(&mut i, "(format--maybe-on-save)");

    assert_eq!(
        run(&mut i, "test--messages"),
        "nil",
        "fundamental-mode has no formatter, so format--maybe-on-save must not even message"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// --- LSP provider dispatch ---

#[test]
fn format_buffer_lsp_provider_dispatches_to_lsp_format_buffer() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(setq test--lsp-format-buffer-called nil)");
    ok(
        &mut i,
        "(fset 'lsp-format-buffer (lambda () (setq test--lsp-format-buffer-called t)))",
    );
    ok(&mut i, "(format-buffer)");
    assert_eq!(run(&mut i, "test--lsp-format-buffer-called"), "t");
}

// --- verilog-mode's own default indent width (M104's modes.el change) ---

#[test]
fn verilog_mode_default_indent_width_is_two() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(verilog-mode)");
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
}

// =====================================================================
// End-to-end, gated on `verible-verilog-format` actually being on PATH.
// =====================================================================

// Named port connections of visibly different lengths (`.clk(clk)` vs.
// `.a(a)`/`.b(b)`) plus a case with a short and a long label (`0:` vs.
// `default:`) -- confirmed by hand against the real binary (see this
// test's own commands in the M104 implementation report) to be the
// smallest shape where `aligned' (columns padded), `verible' (the
// upstream `infer' default, which here happens to ALSO align the named
// ports) and `compact' (explicitly flush-left) produce three distinct
// outputs -- a flatter snippet like a single always_comb with no named
// ports left `verible'/`compact' byte-identical, since `infer' had
// nothing worth aligning to begin with.
const MESSY_SV: &str = "module m #(parameter int W = 4, parameter int Depth = 16) (input logic clk, input logic [W-1:0] a, output logic [W-1:0] b);\n  always_comb begin\n    case (a)\n      0: b = 1;\n      default: b = 0;\n    endcase\n  end\n\n  sub_module #(.WIDTH(W), .DEPTH(Depth)) u_sub (.clk(clk), .a(a), .b(b));\nendmodule\n";

/// M104 fix round (real defect, found by cold review): before this
/// fix, `call_process_string' never set the child's working directory,
/// so `clang-format -style=file' searched for a project `.clang-format'
/// starting from wherever the EDITOR itself happened to be running
/// from -- not from anywhere near the file actually being formatted.
/// Confirmed manually against the real binary (see the M104 fix round
/// report) that this failure mode is completely silent: exit 0, no
/// diagnostic, just quietly falling back to LLVM style. This test's own
/// process working directory (`CARGO_MANIFEST_DIR'-relative, i.e. the
/// repo, not the scratch dir below) stands in for "the editor's own
/// unrelated directory" without needing to chdir anything.
#[test]
fn manual_e2e_clang_format_style_file_finds_the_projects_own_clang_format() {
    if !have_on_path("clang-format") {
        eprintln!("skipping: clang-format not on PATH");
        return;
    }
    let (mut i, _ed) = setup();
    let dir = std::env::temp_dir().join(format!(
        "reticle_format_tests_clangfmt_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    // IndentWidth: 5 is deliberately an eyeball-distinctive value no
    // real style presets use, so there is no ambiguity about whether
    // this specific `.clang-format' was actually read.
    std::fs::write(
        dir.join(".clang-format"),
        "IndentWidth: 5
",
    )
    .unwrap();
    let file = dir.join("t.c");
    std::fs::write(
        &file,
        "int main() {
if (1) {
return 0;
}
}
",
    )
    .unwrap();

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", file.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'c-mode)");
    // c-mode's default style is already 'file (format-style-alist's own
    // default) -- no override needed.
    ok(&mut i, "(format-buffer)");

    assert_eq!(
        bs(&mut i),
        "int main() {
     if (1) {
          return 0;
     }
}
",
        "expected 5-space IndentWidth from the scratch dir's own .clang-format"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn manual_e2e_three_verible_styles_produce_pairwise_different_output() {
    if !have_on_path("verible-verilog-format") {
        eprintln!("skipping: verible-verilog-format not on PATH");
        return;
    }
    let (mut i, _ed) = setup();
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    ok(&mut i, &format!("(insert {:?})", MESSY_SV));

    let mut outputs = Vec::new();
    for style in ["verible", "aligned", "compact"] {
        ok(&mut i, &format!("(setq format-style-alist (format--alist-put format-style-alist 'verilog-mode '{}))", style));
        let args = ok(
            &mut i,
            "(format--verible-args (cdr (assq 'verilog-mode format-style-alist)))",
        );
        ok(&mut i, &format!("(setq test--r (call-process-string \"verible-verilog-format\" '{} (buffer-string)))", args));
        let code = ok(&mut i, "(nth 0 test--r)");
        assert_eq!(
            code, "0",
            "verible-verilog-format should succeed on well-formed input, style {}",
            style
        );
        let stdout = ok(&mut i, "(nth 1 test--r)");
        // Every style's output uses 2-space indentation for the module
        // body ("  always_comb begin"), confirming --indentation_spaces=2
        // took effect regardless of style.
        assert!(
            stdout.contains("\\n  always_comb"),
            "style {} output should indent the module body by 2 spaces, got {}",
            style,
            stdout
        );
        outputs.push((style, stdout));
    }
    assert_ne!(
        outputs[0].1, outputs[1].1,
        "verible vs aligned should differ"
    );
    assert_ne!(
        outputs[0].1, outputs[2].1,
        "verible vs compact should differ"
    );
    assert_ne!(
        outputs[1].1, outputs[2].1,
        "aligned vs compact should differ"
    );
}
