//! M133: telling slang-server where a project's `+incdir+' headers
//! live -- `initializationOptions' and `workspace/
//! didChangeConfiguration' are both measured IGNORED by slang-server,
//! and its own `.slang/server.json' is only read from the server's OWN
//! rootUri, which M132 can compute to be a directory the user never
//! authored a config file in. This file covers:
//!
//! - `lsp--filelist-incdirs' (`+incdir+' parsing, a second reader over
//!   `verible.filelist' that leaves `lsp--filelist-entries' untouched).
//! - `lsp--verilog-include-directories' (the explicit-variable /
//!   discovery / cap / degenerate-root / cache algorithm).
//! - `lsp--verilog-write-build-file' (the generated `.f' file's exact
//!   content).
//! - `lsp--verilog-maybe-push-include-directories' (the wire, on BOTH
//!   `lsp-connect' and `lsp--autostart-begin', and its four gates).
//!
//! Deterministic by construction, same discipline as `lsp_mode_tests.rs':
//! the wire tests stub `lsp--await'/`lsp-request-async' via `fset' rather
//! than talking to a real slang-server.

use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> Interp {
    let mut interp = elisp::new_interp();
    core::init_editor(&mut interp);
    let r = interp.eval_source("(setq format-on-save nil)");
    assert!(r.is_ok(), "setq format-on-save nil failed");
    interp
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

/// A scratch directory that deletes itself on drop -- same shape as
/// `lsp_mode_tests.rs''s own `Scratch' (each test file brings its own
/// helpers; there is no shared fixture in this codebase).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_lsp_verilog_include_{}_{}_{}",
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

fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// Same process-global `$HOME` race guard as `lsp_mode_tests.rs' -- see
/// that file's own doc comment for why a mutex, not just distinct scratch
/// paths, is required (`cargo test` runs this file's tests on multiple
/// threads by default, and `lsp--home-directory' reads `$HOME' fresh on
/// every call).
static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn home_env_lock() -> std::sync::MutexGuard<'static, ()> {
    HOME_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct HomeGuard {
    prev: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl HomeGuard {
    fn set(dir: &std::path::Path) -> HomeGuard {
        let lock = home_env_lock();
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", dir);
        HomeGuard { prev, _lock: lock }
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// Same message-capturing shape `lsp_mode_tests.rs' and friends use.
fn capture_messages(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

// ============================================================
// 1. `+incdir+' parsing (`lsp--filelist-incdirs')
// ============================================================

#[test]
fn filelist_incdirs_parses_a_single_entry() {
    let mut i = setup();
    let dir = scratch_dir("incdirs_single");
    std::fs::create_dir_all(dir.join("inc")).unwrap();
    let fl = dir.join("verible.filelist");
    std::fs::write(&fl, "+incdir+inc\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-incdirs {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(r, format!("({:?})", dir.join("inc").to_str().unwrap()));
}

#[test]
fn filelist_incdirs_splits_a_packed_plus_separated_entry_into_two() {
    let mut i = setup();
    let dir = scratch_dir("incdirs_packed");
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let fl = dir.join("verible.filelist");
    std::fs::write(&fl, "+incdir+a+b\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-incdirs {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(
        r,
        format!(
            "({:?} {:?})",
            dir.join("a").to_str().unwrap(),
            dir.join("b").to_str().unwrap()
        )
    );
}

#[test]
fn filelist_incdirs_resolves_relative_entries_against_the_filelists_own_directory() {
    let mut i = setup();
    let dir = scratch_dir("incdirs_relative");
    std::fs::create_dir_all(dir.join("sub/inc")).unwrap();
    let fl = dir.join("sub/verible.filelist");
    std::fs::write(&fl, "+incdir+inc\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-incdirs {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(r, format!("({:?})", dir.join("sub/inc").to_str().unwrap()));
}

#[test]
fn filelist_incdirs_ignores_comment_and_blank_lines() {
    let mut i = setup();
    let dir = scratch_dir("incdirs_comments");
    std::fs::create_dir_all(dir.join("inc")).unwrap();
    let fl = dir.join("verible.filelist");
    std::fs::write(&fl, "# a comment\n\n// another comment\n+incdir+inc\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-incdirs {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(r, format!("({:?})", dir.join("inc").to_str().unwrap()));
}

#[test]
fn filelist_incdirs_drops_an_entry_pointing_at_a_nonexistent_directory() {
    let mut i = setup();
    let dir = scratch_dir("incdirs_nonexistent");
    std::fs::create_dir_all(&*dir).unwrap();
    let fl = dir.join("verible.filelist");
    std::fs::write(&fl, "+incdir+does_not_exist\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-incdirs {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(r, "nil");
}

#[test]
fn filelist_entries_still_drops_incdir_lines() {
    // Pins that section 3.1 (`lsp--filelist-incdirs') did NOT change
    // `lsp--filelist-entries''s own, deliberately-preserved behavior.
    let mut i = setup();
    let dir = scratch_dir("entries_still_drops_incdir");
    std::fs::create_dir_all(dir.join("inc")).unwrap();
    std::fs::write(dir.join("a.sv"), "module a; endmodule\n").unwrap();
    let fl = dir.join("verible.filelist");
    std::fs::write(&fl, "+incdir+inc\na.sv\n").unwrap();
    let r = run(
        &mut i,
        &format!("(lsp--filelist-entries {:?})", fl.to_str().unwrap()),
    );
    assert_eq!(r, format!("({:?})", dir.join("a.sv").to_str().unwrap()));
}

// ============================================================
// 2. `lsp--verilog-include-directories'
// ============================================================

#[test]
fn explicit_variable_wins_outright_and_discovery_does_not_run() {
    let mut i = setup();
    let dir = scratch_dir("explicit_wins");
    std::fs::create_dir_all(dir.join("myinc")).unwrap();
    // A `verible.filelist' with a DIFFERENT `+incdir+' sitting right
    // there -- if discovery ran at all, this would show up too.
    std::fs::create_dir_all(dir.join("other")).unwrap();
    std::fs::write(dir.join("verible.filelist"), "+incdir+other\n").unwrap();

    ok(&mut i, "(setq test--rg-calls 0)");
    ok(
        &mut i,
        "(fset 'call-process-string
               (lambda (&rest _args) (setq test--rg-calls (1+ test--rg-calls)) (list 1 \"\" \"\")))",
    );
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            dir.join("myinc").to_str().unwrap()
        ),
    );
    let r = run(
        &mut i,
        &format!(
            "(lsp--verilog-include-directories {:?})",
            dir.to_str().unwrap()
        ),
    );
    assert_eq!(r, format!("({:?})", dir.join("myinc").to_str().unwrap()));
    assert_eq!(
        run(&mut i, "test--rg-calls"),
        "0",
        "discovery must not shell out to rg when the explicit variable is set"
    );
}

#[test]
fn discovery_unions_a_header_directory_and_a_filelist_incdir_deduplicated() {
    let mut i = setup();
    let dir = scratch_dir("discovery_union");
    // Discovered via holding a `.svh'.
    std::fs::create_dir_all(dir.join("include")).unwrap();
    std::fs::write(dir.join("include/defs.svh"), "`define X 1\n").unwrap();
    // Discovered via a `verible.filelist''s own `+incdir+', naming a
    // DIFFERENT directory than the header scan already found.
    std::fs::create_dir_all(dir.join("sub/other_inc")).unwrap();
    std::fs::write(dir.join("sub/verible.filelist"), "+incdir+other_inc\n").unwrap();

    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
    let r = run(
        &mut i,
        &format!(
            "(lsp--verilog-include-directories {:?})",
            dir.to_str().unwrap()
        ),
    );
    let expected_a = dir.join("include").to_str().unwrap().to_string();
    let expected_b = dir.join("sub/other_inc").to_str().unwrap().to_string();
    assert!(r.contains(&expected_a), "{} missing from {}", expected_a, r);
    assert!(r.contains(&expected_b), "{} missing from {}", expected_b, r);
    // Dedup: exactly two entries, not more (each source contributing
    // its own directory exactly once, even though both `rg' calls run).
    let count = run(
        &mut i,
        &format!(
            "(length (lsp--verilog-include-directories {:?}))",
            dir.to_str().unwrap()
        ),
    );
    assert_eq!(count, "2");
}

#[test]
fn discovery_cap_truncates_and_announces_the_truncation() {
    let mut i = setup();
    let dir = scratch_dir("discovery_cap");
    for n in 0..5 {
        let sub = dir.join(format!("h{}", n));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("x.svh"), "").unwrap();
    }
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
    ok(&mut i, "(setq lsp-verilog-include-directories-max 2)");
    capture_messages(&mut i);
    let r = run(
        &mut i,
        &format!(
            "(length (lsp--verilog-include-directories {:?}))",
            dir.to_str().unwrap()
        ),
    );
    assert_eq!(r, "2");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("truncated"),
        "expected a truncation message, got: {}",
        messages
    );
    assert!(
        messages.contains('5'),
        "truncation message should name the true count (5): {}",
        messages
    );
    ok(&mut i, "(setq lsp-verilog-include-directories-max 64)");
}

#[test]
fn discovery_refuses_a_degenerate_root() {
    let mut i = setup();
    let home = scratch_dir("discovery_degenerate_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
    capture_messages(&mut i);

    assert_eq!(run(&mut i, "(lsp--verilog-include-directories nil)"), "nil");
    assert_eq!(
        run(&mut i, "(lsp--verilog-include-directories \"/\")"),
        "nil"
    );
    let r = run(
        &mut i,
        &format!(
            "(lsp--verilog-include-directories {:?})",
            home.to_str().unwrap()
        ),
    );
    assert_eq!(r, "nil");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("skipped"),
        "expected a skip message for the degenerate root(s): {}",
        messages
    );
}

#[test]
fn discovery_result_is_cached_and_does_not_reshell_out_on_a_second_call() {
    let mut i = setup();
    let dir = scratch_dir("discovery_cache");
    std::fs::create_dir_all(dir.join("include")).unwrap();
    std::fs::write(dir.join("include/x.svh"), "").unwrap();
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");

    ok(&mut i, "(setq test--rg-calls 0)");
    ok(
        &mut i,
        "(let ((real (symbol-function 'call-process-string)))
           (fset 'call-process-string
                 (lambda (&rest args)
                   (setq test--rg-calls (1+ test--rg-calls))
                   (apply real args))))",
    );

    let call = format!(
        "(lsp--verilog-include-directories {:?})",
        dir.to_str().unwrap()
    );
    let first = run(&mut i, &call);
    let calls_after_first = run(&mut i, "test--rg-calls");
    let second = run(&mut i, &call);
    let calls_after_second = run(&mut i, "test--rg-calls");

    assert_eq!(first, second);
    assert_eq!(
        calls_after_first, calls_after_second,
        "a second call for the SAME root must reuse the cached answer, \
         not shell out to rg again"
    );
}

// ============================================================
// 3. `lsp--verilog-write-build-file'
// ============================================================

#[test]
fn write_build_file_content_is_exactly_the_dash_i_lines_absolute_quoted_one_per_line() {
    // Fix-round finding, measured against a real `slang-server': `+incdir+'
    // (this function's ORIGINAL directive) splits its argument on both
    // whitespace and `+' -- a directory containing a `+' comes back as
    // TWO separate, both-wrong include directories, and double-quoting
    // fixes the space case but NOT the `+' case. `-I "<dir>"' (quoted)
    // has neither problem -- see `lsp--verilog-write-build-file''s own
    // docstring for the full measurement. This test pins the new
    // directive; the two tests below pin it specifically against a
    // space-containing and a `+'-containing directory name.
    let mut i = setup();
    let dir = scratch_dir("write_build_file");
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let out = dir.join("gen/incdirs.f");
    ok(
        &mut i,
        &format!(
            "(lsp--verilog-write-build-file {:?} {:?} (list {:?} {:?}))",
            out.to_str().unwrap(),
            dir.to_str().unwrap(),
            dir.join("a").to_str().unwrap(),
            dir.join("b").to_str().unwrap(),
        ),
    );
    let content = std::fs::read_to_string(&out).unwrap();
    let dash_i_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("-I ")).collect();
    assert_eq!(
        dash_i_lines,
        vec![
            format!("-I \"{}\"", dir.join("a").to_str().unwrap()),
            format!("-I \"{}\"", dir.join("b").to_str().unwrap()),
        ]
    );
    // No other non-comment, non-blank content -- and any comment line
    // must be `//'-led, NOT `;'-led (see the dedicated fix-round test
    // below for why `;;' specifically is wrong here, not just
    // "a style choice").
    for line in content.lines() {
        assert!(
            line.is_empty() || line.starts_with("//") || line.starts_with("-I \""),
            "unexpected line in generated build file: {:?}",
            line
        );
        assert!(
            !line.starts_with(';'),
            "generated build file has a `;'-led line, which slang's `.f' \
             format does not recognise as a comment at all: {:?}",
            line
        );
        assert!(
            !line.starts_with("+incdir+"),
            "generated build file still emits `+incdir+', the directive \
             measured to split on `+' and (unquoted) on whitespace: {:?}",
            line
        );
    }
    // The throwaway buffer must not linger in the buffer list.
    let buffers = run(&mut i, "(mapcar #'buffer-name (buffer-list))");
    assert!(
        !buffers.contains("lsp-verilog-build-file"),
        "throwaway build-file buffer leaked: {}",
        buffers
    );
}

#[test]
fn write_build_file_quotes_a_directory_name_containing_a_space() {
    let mut i = setup();
    let dir = scratch_dir("write_build_file_space");
    let inc = dir.join("inc dir");
    std::fs::create_dir_all(&inc).unwrap();
    let out = dir.join("gen/incdirs.f");
    ok(
        &mut i,
        &format!(
            "(lsp--verilog-write-build-file {:?} {:?} (list {:?}))",
            out.to_str().unwrap(),
            dir.to_str().unwrap(),
            inc.to_str().unwrap(),
        ),
    );
    let content = std::fs::read_to_string(&out).unwrap();
    let dash_i_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("-I ")).collect();
    assert_eq!(
        dash_i_lines,
        vec![format!("-I \"{}\"", inc.to_str().unwrap())]
    );
}

#[test]
fn write_build_file_quotes_a_directory_name_containing_a_plus() {
    // The measurement that ruled out `+incdir+' even double-quoted:
    // `+incdir+"/abs/inc+plus"' still splits into `inc' and `plus' on
    // the real server, because slang splits `+incdir+''s own argument
    // on `+' regardless of quoting. `-I "<dir>"' has no such problem.
    let mut i = setup();
    let dir = scratch_dir("write_build_file_plus");
    let inc = dir.join("inc+plus");
    std::fs::create_dir_all(&inc).unwrap();
    let out = dir.join("gen/incdirs.f");
    ok(
        &mut i,
        &format!(
            "(lsp--verilog-write-build-file {:?} {:?} (list {:?}))",
            out.to_str().unwrap(),
            dir.to_str().unwrap(),
            inc.to_str().unwrap(),
        ),
    );
    let content = std::fs::read_to_string(&out).unwrap();
    let dash_i_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("-I ")).collect();
    assert_eq!(
        dash_i_lines,
        vec![format!("-I \"{}\"", inc.to_str().unwrap())]
    );
}

#[test]
fn write_build_file_drops_a_directory_name_containing_a_quote_and_announces_it() {
    // Fix-round finding: a literal `"' (also true of a backslash or a
    // newline, none tested separately here since the mechanism is one
    // shared regex) cannot be safely represented on a quoted `-I
    // "<dir>"' line -- this format defines no escape convention for it.
    // Rather than emit a silently-wrong line, the directory is dropped
    // and the drop is announced via `message'.
    let mut i = setup();
    let dir = scratch_dir("write_build_file_quote");
    let unsafe_inc = dir.join("inc\"quote");
    let safe_inc = dir.join("inc_plain");
    std::fs::create_dir_all(&unsafe_inc).unwrap();
    std::fs::create_dir_all(&safe_inc).unwrap();
    let out = dir.join("gen/incdirs.f");
    capture_messages(&mut i);
    ok(
        &mut i,
        &format!(
            "(lsp--verilog-write-build-file {:?} {:?} (list {:?} {:?}))",
            out.to_str().unwrap(),
            dir.to_str().unwrap(),
            unsafe_inc.to_str().unwrap(),
            safe_inc.to_str().unwrap(),
        ),
    );
    let content = std::fs::read_to_string(&out).unwrap();
    let dash_i_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("-I ")).collect();
    assert_eq!(
        dash_i_lines,
        vec![format!("-I \"{}\"", safe_inc.to_str().unwrap())],
        "the quote-containing directory must not appear in the generated file at all"
    );
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("dropped"),
        "expected a message announcing the drop: {}",
        messages
    );
    // `messages' is `prin1'-escaped (the raw `"' inside the dropped
    // path becomes `\"' in the printed string), so check the two
    // halves either side of the quote rather than the raw path whole.
    assert!(
        messages.contains("write_build_file_quote")
            && messages.contains("inc")
            && messages.contains("quote"),
        "expected the message to name the dropped directory: {}",
        messages
    );
}

#[test]
fn write_build_file_comment_header_uses_slash_slash_not_semicolon_semicolon() {
    // Fix-round regression test. `.f' command-file format (the shape
    // this function writes) has NO comment syntax that recognises `;;'
    // at all -- measured against a real `slang-server 0.2.9+4f33c99'
    // with a `;;'-commented file: every whitespace-separated token on
    // the `;;' lines is read as a bare FILE NAME (`Error Notif:
    // ';;': No such file or directory`, `'Generated': ...`, `'by':
    // ...`, one `'/Users/.../demo': Is a directory` for a directory
    // token, and so on), and the `+incdir+' line's effect is lost right
    // along with it -- `dev/tui-drive.py` against
    // `demo/rtl/top/soc_top.sv` showed the mode-line stuck at `!10` and
    // the inline `'soc_defs.svh': No such file or directory` row still
    // present with a `;;'-commented generated file, and `!9` with no
    // inline row once switched to `//`.
    //
    // `;;' is elisp's OWN comment syntax -- the obvious thing to reach
    // for writing this function in this codebase -- and it is wrong for
    // the format this function writes into. The failure is also SILENT
    // as far as this client can tell: `workspace/executeCommand`
    // `slang.setBuildFile` replies `{"result": null}` whether the file
    // was well-formed or not (see `lsp--verilog-maybe-push-include-
    // directories''s own docstring), so nothing short of watching the
    // server's own diagnostics change (or its stderr `Error Notif:`
    // lines) can catch a wrong comment syntax here -- every OTHER test
    // in this file that asserts build-file content was checking that
    // the content matched what THIS project asked for, never that
    // slang itself would accept it.
    let mut i = setup();
    let dir = scratch_dir("write_build_file_comment_syntax");
    std::fs::create_dir_all(dir.join("a")).unwrap();
    let out = dir.join("gen/incdirs.f");
    ok(
        &mut i,
        &format!(
            "(lsp--verilog-write-build-file {:?} {:?} (list {:?}))",
            out.to_str().unwrap(),
            dir.to_str().unwrap(),
            dir.join("a").to_str().unwrap(),
        ),
    );
    let content = std::fs::read_to_string(&out).unwrap();
    let comment_lines: Vec<&str> = content
        .lines()
        .filter(|l| !l.starts_with("-I ") && !l.is_empty())
        .collect();
    assert!(
        !comment_lines.is_empty(),
        "expected at least one comment header line in the generated file"
    );
    for line in &comment_lines {
        assert!(
            line.starts_with("//"),
            "comment line {:?} does not start with `//' -- a `;;'-led \
             line here is silently rejected by slang's `.f' format (see \
             this test's own doc comment)",
            line
        );
        assert!(
            !line.starts_with(';'),
            "comment line {:?} starts with `;', elisp's own comment \
             syntax and NOT valid in slang's `.f' format",
            line
        );
    }
}

// ============================================================
// 4. The wire: `lsp--verilog-maybe-push-include-directories', on BOTH
//    `lsp-connect' and `lsp--autostart-begin'.
// ============================================================

/// Stubs `lsp--await' so `lsp-connect''s synchronous `initialize' round
/// trip returns canned capabilities declaring `slang.setBuildFile',
/// without needing a real slang-server -- `lsp-connect' still spawns a
/// real (but otherwise untouched) `cat' process for CONN.
fn stub_await_declares_set_build_file(i: &mut Interp) {
    ok(
        i,
        "(fset 'lsp--await
               (lambda (client id &optional timeout)
                 (let ((caps (make-hash-table))
                       (ecp (make-hash-table))
                       (result (make-hash-table)))
                   (puthash \"commands\" (vector \"slang.setBuildFile\") ecp)
                   (puthash \"executeCommandProvider\" ecp caps)
                   (puthash \"capabilities\" caps result)
                   result)))",
    );
}

fn stub_capture_request_async(i: &mut Interp) {
    ok(i, "(setq test--captured nil)");
    ok(
        i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
}

#[test]
fn wire_sends_set_build_file_on_the_lsp_connect_path() {
    let mut i = setup();
    let home = scratch_dir("wire_connect_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("wire_connect_root");
    std::fs::create_dir_all(root.join("include")).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            root.join("include").to_str().unwrap()
        ),
    );

    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);

    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );

    assert_eq!(run(&mut i, "(eq (car test--captured) test--client)"), "t");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"workspace/executeCommand\""
    );
    assert_eq!(
        run(&mut i, "(gethash \"command\" (nth 2 test--captured))"),
        "\"slang.setBuildFile\""
    );
    let expected_path = run(
        &mut i,
        &format!(
            "(lsp--verilog-build-file-path {:?})",
            root.to_str().unwrap()
        ),
    );
    assert_eq!(
        run(
            &mut i,
            "(car (gethash \"arguments\" (nth 2 test--captured)))"
        ),
        expected_path
    );
    // The build file was actually written to disk. `expected_path` is
    // a `prin1'-escaped Lisp string (`"\"...\""'); strip the surrounding
    // quotes rather than pull in a JSON parser just for this.
    let path = expected_path
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&expected_path);
    assert!(
        std::path::Path::new(path).exists(),
        "generated build file does not exist at {}",
        path
    );

    // Client-side bookkeeping.
    assert_eq!(
        run(
            &mut i,
            "(cdr (assq :sent (lsp--client-verilog-include-info test--client)))"
        ),
        "t"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

#[test]
fn wire_sends_set_build_file_on_the_autostart_path() {
    // Without this call site, deleting the `lsp--autostart-begin' push
    // call would leave the suite green while the actual default
    // file-open path stays broken -- this is the "second, async" path
    // the M133 spec calls out by name.
    let mut i = setup();
    let home = scratch_dir("wire_autostart_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("wire_autostart_root");
    std::fs::create_dir_all(root.join("include")).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            root.join("include").to_str().unwrap()
        ),
    );

    // The `initialize' reply must NOT be delivered synchronously inside
    // `lsp-request-async' itself -- the real async path always pushes
    // its `lsp--autostart-pending' bookkeeping AFTER the
    // `lsp-request-async' call returns, and the completion callback's
    // own `(not entry)' guard (see `lsp--autostart-begin''s own
    // docstring) treats a too-early reply as a race and bails out doing
    // nothing. So this stub defers: it stashes the `initialize'
    // callback in `test--init-callback' and this test fires it by hand
    // once `lsp--autostart-begin' has returned, matching the real
    // ordering.
    ok(&mut i, "(setq test--captured nil)");
    ok(&mut i, "(setq test--init-callback nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (cond
                  ((string= method \"initialize\")
                   (setq test--init-callback callback)
                   99)
                  (t
                   (setq test--captured (list client method params callback))
                   100))))",
    );

    ok(
        &mut i,
        &format!(
            "(lsp--autostart-begin \"cat\" nil {:?} 'verilog-mode)",
            root.to_str().unwrap()
        ),
    );
    ok(
        &mut i,
        "(let ((caps (make-hash-table))
               (ecp (make-hash-table))
               (result (make-hash-table)))
           (puthash \"commands\" (vector \"slang.setBuildFile\") ecp)
           (puthash \"executeCommandProvider\" ecp caps)
           (puthash \"capabilities\" caps result)
           (funcall test--init-callback result))",
    );

    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"workspace/executeCommand\"",
        "autostart's own completion callback must have pushed the build \
         file, not just lsp-connect's"
    );
    assert_eq!(
        run(&mut i, "(gethash \"command\" (nth 2 test--captured))"),
        "\"slang.setBuildFile\""
    );

    ok(
        &mut i,
        &format!(
            "(let ((c (cdr (assoc (cons \"cat\" {:?}) lsp--connections))))
               (when c (lsp-kill (lsp--client-conn c))))",
            root.to_str().unwrap()
        ),
    );
}

#[test]
fn wire_not_sent_when_the_server_declares_no_set_build_file_command() {
    let mut i = setup();
    let home = scratch_dir("wire_no_command_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("wire_no_command_root");
    std::fs::create_dir_all(root.join("include")).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            root.join("include").to_str().unwrap()
        ),
    );

    // No `executeCommandProvider' at all -- e.g. verible.
    ok(
        &mut i,
        "(fset 'lsp--await (lambda (client id &optional timeout) (make-hash-table)))",
    );
    stub_capture_request_async(&mut i);

    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );

    assert_eq!(
        run(&mut i, "test--captured"),
        "nil",
        "a server with no executeCommandProvider must never receive \
         workspace/executeCommand slang.setBuildFile"
    );
    assert_eq!(
        run(
            &mut i,
            "(cdr (assq :sent (lsp--client-verilog-include-info test--client)))"
        ),
        "nil"
    );
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

#[test]
fn wire_not_sent_when_the_user_has_their_own_slang_config_under_auto() {
    let mut i = setup();
    let home = scratch_dir("wire_user_config_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("wire_user_config_root");
    std::fs::create_dir_all(root.join("include")).unwrap();
    std::fs::create_dir_all(root.join(".slang")).unwrap();
    std::fs::write(
        root.join(".slang/server.json"),
        "{\"build\": \"design.f\"}\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            root.join("include").to_str().unwrap()
        ),
    );
    ok(&mut i, "(setq lsp-verilog-push-include-directories 'auto)");

    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);
    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );
    assert_eq!(
        run(&mut i, "test--captured"),
        "nil",
        "the `auto' guard must skip when the user has their own \
         .slang/server.json"
    );
    let reason = run(
        &mut i,
        "(cdr (assq :reason (lsp--client-verilog-include-info test--client)))",
    );
    assert!(reason.contains("slang"), "unexpected reason: {}", reason);
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");

    // Setting the override to `t' must send anyway, over the user's config.
    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);
    ok(&mut i, "(setq lsp-verilog-push-include-directories t)");
    ok(
        &mut i,
        &format!(
            "(setq test--client2 (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"workspace/executeCommand\"",
        "lsp-verilog-push-include-directories = t must push even over a \
         user's own .slang/server.json"
    );
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client2))");
    ok(&mut i, "(setq lsp-verilog-push-include-directories 'auto)");
}

#[test]
fn wire_not_sent_when_no_include_directories_were_found() {
    let mut i = setup();
    let home = scratch_dir("wire_no_dirs_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("wire_no_dirs_root");
    std::fs::create_dir_all(&*root).unwrap();
    // Explicit variable pointing at a directory that doesn't exist --
    // resolves to an empty (nil) directory list.
    ok(
        &mut i,
        "(setq lsp-verilog-include-directories (list \"does-not-exist\"))",
    );

    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);
    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );
    assert_eq!(run(&mut i, "test--captured"), "nil");
    let reason = run(
        &mut i,
        "(cdr (assq :reason (lsp--client-verilog-include-info test--client)))",
    );
    assert!(
        reason.contains("directories"),
        "unexpected reason: {}",
        reason
    );
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
}

// ============================================================
// 5. Fix round: a failure inside the push must never escape and kill
//    the connection (item 1), the show-command's own observable
//    surface (item 3), the remaining `auto'-guard disjuncts (item 4),
//    build-file path injectivity (item 5), the explicit-variable
//    branch's degenerate-root guard (item 6), and the nil-root reason
//    (item 7).
// ============================================================

#[test]
fn maybe_push_a_build_file_write_failure_does_not_kill_the_connection() {
    // Fix-round fault injection. `~/.reticle` exists as a PLAIN FILE,
    // so `make-directory "~/.reticle/lsp" t` cannot succeed
    // (`create_dir_all` errors when an existing PATH COMPONENT is a
    // regular file, not a directory) -- the exact shape of "no
    // permission" / "disk full" the coordinator's report named. Before
    // the fix, this error escaped `lsp--verilog-write-build-file'
    // uncaught, propagated into `lsp-connect''s own handshake
    // `condition-case', and killed the brand-new (otherwise perfectly
    // healthy) connection. After the fix, it is caught and recorded as
    // an ordinary `(:sent . nil)' reason, and the connection this test
    // asserts on is untouched.
    let mut i = setup();
    let home = scratch_dir("push_fault_home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join(".reticle"), "not a directory").unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("push_fault_root");
    std::fs::create_dir_all(root.join("include")).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            root.join("include").to_str().unwrap()
        ),
    );

    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);

    // `ok' itself asserts this does not signal an ERROR result --
    // before the fix, the write failure re-signals here and this call
    // fails outright.
    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            root.to_str().unwrap()
        ),
    );

    assert_eq!(
        run(&mut i, "(lsp--client-p test--client)"),
        "t",
        "lsp-connect must still have returned a real client"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p (lsp--client-conn test--client))"),
        "t",
        "the connection must still be alive despite the build-file write failing"
    );
    assert_eq!(
        run(
            &mut i,
            "(cdr (assq :sent (lsp--client-verilog-include-info test--client)))"
        ),
        "nil"
    );
    let reason = run(
        &mut i,
        "(cdr (assq :reason (lsp--client-verilog-include-info test--client)))",
    );
    assert!(
        reason.contains("could not push include directories"),
        "unexpected reason: {}",
        reason
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
}

#[test]
fn show_include_directories_success_message_names_the_build_file_and_directories() {
    let mut i = setup();
    ok(&mut i, "(get-buffer-create \"m133-show-success\")");
    ok(&mut i, "(set-buffer \"m133-show-success\")");
    ok(
        &mut i,
        "(setq-local lsp--buffer-client
               (make-lsp--client
                :command \"slang-server\"
                :root \"/proj\"
                :verilog-include-info
                (list (cons :sent t)
                      (cons :build-file \"/home/x/.reticle/lsp/incdirs-proj.f\")
                      (cons :directories (list \"/proj/include\" \"/proj/other\")))))",
    );
    capture_messages(&mut i);
    ok(&mut i, "(lsp-verilog-show-include-directories)");
    let messages = run(&mut i, "test--messages");
    assert!(messages.contains("incdirs-proj.f"), "{}", messages);
    assert!(messages.contains("/proj/include"), "{}", messages);
    assert!(messages.contains("/proj/other"), "{}", messages);
    assert!(
        messages.contains('2'),
        "expected the directory count: {}",
        messages
    );
}

#[test]
fn show_include_directories_reports_the_reason_when_nothing_was_sent() {
    let mut i = setup();
    ok(&mut i, "(get-buffer-create \"m133-show-reason\")");
    ok(&mut i, "(set-buffer \"m133-show-reason\")");
    ok(
        &mut i,
        "(setq-local lsp--buffer-client
               (make-lsp--client
                :command \"slang-server\"
                :verilog-include-info
                (list (cons :sent nil)
                      (cons :reason \"no include directories found\"))))",
    );
    capture_messages(&mut i);
    ok(&mut i, "(lsp-verilog-show-include-directories)");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("no include directories found"),
        "{}",
        messages
    );
}

#[test]
fn show_include_directories_reports_when_no_slang_client_is_attached() {
    let mut i = setup();
    ok(&mut i, "(get-buffer-create \"m133-show-none\")");
    ok(&mut i, "(set-buffer \"m133-show-none\")");
    // A non-slang client attached -- must not be mistaken for one.
    ok(
        &mut i,
        "(setq-local lsp--buffer-client (make-lsp--client :command \"verible-verilog-ls\"))",
    );
    capture_messages(&mut i);
    ok(&mut i, "(lsp-verilog-show-include-directories)");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("no slang-server client attached"),
        "{}",
        messages
    );
}

#[test]
fn show_include_directories_finds_the_slang_client_among_several_attached() {
    // Pins the client-selection `dolist' itself: a NON-slang primary is
    // attached, and the slang client only shows up in
    // `lsp--buffer-clients' (the secondary-clients list) -- deleting or
    // breaking the dolist's scan would either find the wrong client or
    // none at all.
    let mut i = setup();
    ok(&mut i, "(get-buffer-create \"m133-show-multi\")");
    ok(&mut i, "(set-buffer \"m133-show-multi\")");
    ok(
        &mut i,
        "(setq-local lsp--buffer-client (make-lsp--client :command \"verible-verilog-ls\"))",
    );
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients
               (list (make-lsp--client
                      :command \"slang-server\"
                      :verilog-include-info
                      (list (cons :sent nil)
                            (cons :reason \"server does not declare slang.setBuildFile\")))))",
    );
    capture_messages(&mut i);
    ok(&mut i, "(lsp-verilog-show-include-directories)");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("server does not declare slang.setBuildFile"),
        "expected the SLANG client's own reason, not the verible \
         client's (which has no verilog-include-info at all): {}",
        messages
    );
}

#[test]
fn show_include_directories_prefix_arg_clears_the_cache() {
    let mut i = setup();
    ok(&mut i, "(get-buffer-create \"m133-show-force\")");
    ok(&mut i, "(set-buffer \"m133-show-force\")");
    ok(
        &mut i,
        "(setq-local lsp--buffer-client (make-lsp--client :command \"slang-server\" :root \"/proj-force\"))",
    );
    ok(
        &mut i,
        "(puthash \"/proj-force\" (list \"/proj-force/include\") lsp--verilog-include-directories-cache)",
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"/proj-force\" lsp--verilog-include-directories-cache 'missing)"
        ),
        "(\"/proj-force/include\")"
    );
    ok(&mut i, "(lsp-verilog-show-include-directories t)");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"/proj-force\" lsp--verilog-include-directories-cache 'missing)"
        ),
        "missing",
        "a prefix argument must have cleared this root's cache entry"
    );
}

#[test]
fn slang_command_p_matches_a_substring_of_the_basename_only() {
    let mut i = setup();
    assert_eq!(
        run(
            &mut i,
            "(lsp--verilog-slang-command-p \"/usr/local/bin/slang-server\")"
        ),
        "t"
    );
    assert_eq!(
        run(&mut i, "(lsp--verilog-slang-command-p \"slang-server\")"),
        "t"
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--verilog-slang-command-p \"verible-verilog-ls\")"
        ),
        "nil"
    );
    assert_eq!(run(&mut i, "(lsp--verilog-slang-command-p nil)"), "nil");
}

#[test]
fn user_configured_slang_p_detects_root_slash_dot_slang_slash_local_slash_server_json() {
    // Fix-round hermeticity fix: `lsp--verilog-user-configured-slang-p'
    // ORs three disjuncts, and the third reads the REAL `$HOME' -- this
    // test must pin `$HOME' to an empty scratch directory the same way
    // its `~/.slang/server.json' sibling already does, or it only
    // proves the ROOT/.slang/local leg works on a machine that happens
    // to have no `~/.slang/server.json' of its own (exactly the
    // machine of the user this feature is built around, who may well
    // have one).
    let mut i = setup();
    let home = scratch_dir("user_cfg_local_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("user_cfg_local");
    std::fs::create_dir_all(root.join(".slang/local")).unwrap();
    std::fs::write(root.join(".slang/local/server.json"), "{}").unwrap();
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(lsp--verilog-user-configured-slang-p {:?})",
                root.to_str().unwrap()
            )
        ),
        "t"
    );
}

#[test]
fn user_configured_slang_p_detects_home_slash_dot_slang_slash_server_json() {
    let mut i = setup();
    let home = scratch_dir("user_cfg_home");
    std::fs::create_dir_all(home.join(".slang")).unwrap();
    std::fs::write(home.join(".slang/server.json"), "{}").unwrap();
    let _guard = HomeGuard::set(&home);
    let root = scratch_dir("user_cfg_home_root");
    std::fs::create_dir_all(&*root).unwrap();
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(lsp--verilog-user-configured-slang-p {:?})",
                root.to_str().unwrap()
            )
        ),
        "t"
    );
}

#[test]
fn build_file_path_sanitisation_is_injective_for_roots_that_collided_under_either_naive_scheme() {
    // Fix-round finding (round 2): the ORIGINAL naive `/' -> `-' scheme
    // was not injective (`/tmp/x/proj-a/sub' vs `/tmp/x/proj/a-sub'),
    // and the FIRST attempted fix -- escape `-' to `--', then map `/'
    // to a single `-' -- was ALSO not injective: both land in the same
    // one-character alphabet with no way to tell an escaped original
    // `-' apart from two mapped `/'s once adjacent, so
    // `/foo/-bar' and `/foo-/bar' BOTH became `-foo---bar' under that
    // scheme. This test would have failed against either broken
    // scheme; the current one (two DISTINCT two-character tags, `-d'
    // for `-' and `-s' for `/', in that order) survives every pair.
    let mut i = setup();
    let pairs = [
        ("/tmp/x/proj-a/sub", "/tmp/x/proj/a-sub"),
        ("/foo/-bar", "/foo-/bar"),
        ("/a-db", "/a/db"),
        ("/home/user/proj-/sub", "/home/user/proj/-sub"),
    ];
    for (p1, p2) in pairs {
        let a = run(&mut i, &format!("(lsp--verilog-build-file-path {:?})", p1));
        let b = run(&mut i, &format!("(lsp--verilog-build-file-path {:?})", p2));
        assert_ne!(
            a, b,
            "{:?} and {:?} collided on the generated build-file path: {} vs {}",
            p1, p2, a, b
        );
    }
}

#[test]
fn explicit_variable_branch_refuses_a_relative_entry_under_a_degenerate_root() {
    let mut i = setup();
    let _guard = HomeGuard::set(std::path::Path::new("/"));
    // With HOME == "/", a nil root is degenerate; a RELATIVE entry
    // must be refused rather than silently resolved against the
    // editor's own working directory.
    ok(
        &mut i,
        "(setq lsp-verilog-include-directories (list \"relative-inc\"))",
    );
    capture_messages(&mut i);
    let r = run(&mut i, "(lsp--verilog-include-directories nil)");
    assert_eq!(r, "nil");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("skipped"),
        "expected a skip message naming the dropped relative entry: {}",
        messages
    );
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
}

#[test]
fn explicit_variable_branch_still_honours_an_absolute_entry_under_a_degenerate_root() {
    let mut i = setup();
    let _guard = HomeGuard::set(std::path::Path::new("/"));
    let dir = scratch_dir("explicit_absolute_degenerate");
    std::fs::create_dir_all(dir.join("inc")).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq lsp-verilog-include-directories (list {:?}))",
            dir.join("inc").to_str().unwrap()
        ),
    );
    let r = run(&mut i, "(lsp--verilog-include-directories nil)");
    assert_eq!(r, format!("({:?})", dir.join("inc").to_str().unwrap()));
    ok(&mut i, "(setq lsp-verilog-include-directories nil)");
}

#[test]
fn maybe_push_gives_a_nil_root_its_own_reason_distinct_from_empty_directories() {
    let mut i = setup();
    let home = scratch_dir("nil_root_reason_home");
    std::fs::create_dir_all(&home).unwrap();
    let _guard = HomeGuard::set(&home);
    stub_await_declares_set_build_file(&mut i);
    stub_capture_request_async(&mut i);
    // lsp-connect with ROOT-PATH nil.
    ok(&mut i, "(setq test--client (lsp-connect \"cat\" nil nil))");
    assert_eq!(run(&mut i, "test--captured"), "nil");
    let reason = run(
        &mut i,
        "(cdr (assq :reason (lsp--client-verilog-include-info test--client)))",
    );
    assert_ne!(
        reason, "\"no include directories found\"",
        "a nil root must not be reported with the SAME reason as a \
         searched, empty root"
    );
    assert!(
        reason.contains("no root"),
        "expected the reason to name the actual cause (no root): {}",
        reason
    );
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}
