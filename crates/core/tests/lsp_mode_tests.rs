//! M26: LSP <-> major-mode wiring -- `lsp-server-alist`, the `lsp`
//! connect command (project-root detection, connection reuse, and
//! clean degradation when a server command is missing or dies), and
//! the async hover/definition/diagnostic-navigation commands layered
//! on M15's `lsp-request-async`/`lsp-process-pending` plumbing.
//!
//! Deterministic by construction: real subprocesses appear only where
//! they can't misbehave (a command guaranteed not to exist, or `cat`
//! echoing frames straight back), and the hover/definition *logic*
//! (0-based position computation, callback wiring, marker ring) is
//! tested by stubbing `lsp-request-async` itself -- `fset` a capturing
//! lambda in place of the real one -- so it never depends on a real
//! server or even a real subprocess. That mirrors how
//! `lsp_async_tests.rs` drives `lsp--dispatch` directly by hand rather
//! than through a live connection. A real language server (rust-analyzer,
//! clangd) is exercised only manually / in `lsp_tests.rs`'s `#[ignore]`d
//! end-to-end test.

use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> Interp {
    let mut interp = elisp::new_interp();
    core::init_editor(&mut interp);
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
            "reticle_lsp_mode_{}_{}_{}",
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

/// A fresh scratch directory under the OS temp dir, unique per test run.
fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

// ============================================================
// lsp-server-alist
// ============================================================

#[test]
fn server_alist_has_default_entries_for_every_registered_mode() {
    let mut i = setup();
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'rust-mode)"),
        "(\"rust-analyzer\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'c-mode)"),
        "(\"clangd\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'c++-mode)"),
        "(\"clangd\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'python-mode)"),
        "(\"pyright-langserver\" \"--stdio\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'sh-mode)"),
        "(\"bash-language-server\" \"start\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'java-mode)"),
        "(\"jdtls\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'perl-mode)"),
        "(\"pls\")"
    );
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'verilog-mode)"),
        "(\"verible-verilog-ls\")"
    );
    // A mode with no entry: nil, not an error.
    assert_eq!(run(&mut i, "(lsp--server-for-mode 'text-mode)"), "nil");
}

#[test]
fn server_alist_entries_are_genuinely_command_dot_args_pairs() {
    let mut i = setup();
    // sh-mode carries a real ARGS list (not just nil) -- the clearest
    // check that the stored shape is (COMMAND . ARGS), not merely a
    // flat list that happens to print the same way when ARGS is empty.
    assert_eq!(
        run(&mut i, "(car (lsp--server-for-mode 'sh-mode))"),
        "\"bash-language-server\""
    );
    assert_eq!(
        run(&mut i, "(cdr (lsp--server-for-mode 'sh-mode))"),
        "(\"start\")"
    );
    assert_eq!(
        run(&mut i, "(car (lsp--server-for-mode 'rust-mode))"),
        "\"rust-analyzer\""
    );
    assert_eq!(
        run(&mut i, "(cdr (lsp--server-for-mode 'rust-mode))"),
        "nil"
    );
}

#[test]
fn server_alist_user_override_via_add_to_list_wins() {
    let mut i = setup();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"my-rust-analyzer\" \"--flag\")))",
    );
    // add-to-list prepends; assq's first-match-wins makes the new entry
    // shadow the built-in default without needing to remove it.
    assert_eq!(
        run(&mut i, "(lsp--server-for-mode 'rust-mode)"),
        "(\"my-rust-analyzer\" \"--flag\")"
    );
}

// ============================================================
// Project root detection
// ============================================================

#[test]
fn project_root_finds_nearest_ancestor_containing_dot_git() {
    let mut i = setup();
    let root = scratch_dir("git_root");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn project_root_finds_a_slang_config_directory_from_a_nested_source_file() {
    // slang-server keeps its configuration in a `.slang/' DIRECTORY, not
    // a file, so this also pins that `lsp--dir-has-marker-p' accepts a
    // directory marker -- `file-exists-p', which it uses, is true for
    // both, but a future switch to a file-only predicate would silently
    // break exactly this server.
    //
    // Why it matters more here than for most markers: slang-server
    // indexes everything under the root it is given and answers
    // cross-file requests from that index. Handed the nested file's own
    // directory instead, it indexes one directory, finds nothing, and
    // returns empty results while still advertising full capabilities
    // -- measured 2026-08-11, see `lsp--project-root-markers''s own doc
    // string for the probe output.
    let mut i = setup();
    let root = scratch_dir("slang_root");
    std::fs::create_dir_all(root.join(".slang")).unwrap();
    std::fs::write(root.join(".slang/config.json"), "{}\n").unwrap();
    std::fs::create_dir_all(root.join("rtl/core")).unwrap();
    let file = root.join("rtl/core/alu.sv");
    std::fs::write(&file, "module alu (input logic clk_i);\nendmodule\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn project_root_finds_nearest_ancestor_containing_cargo_toml() {
    let mut i = setup();
    let root = scratch_dir("cargo_root");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn project_root_prefers_the_nearer_marker_over_an_outer_one() {
    let mut i = setup();
    // root/.git (outer) and root/sub/Cargo.toml (inner, nearer the
    // file) both qualify; walking upward must stop at the inner one.
    let root = scratch_dir("nearest_wins");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("sub/src")).unwrap();
    std::fs::write(root.join("sub/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let file = root.join("sub/src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(
        run(&mut i, &src),
        format!("{:?}", root.join("sub").to_str().unwrap())
    );
}

#[test]
fn project_root_falls_back_to_the_files_own_directory_when_nothing_matches() {
    let mut i = setup();
    let root = scratch_dir("no_markers");
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("orphan.txt");
    std::fs::write(&file, "hi\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

// ============================================================
// lsp--get-connection (the (command . root) reuse table)
// ============================================================

#[test]
fn get_connection_returns_nil_when_nothing_is_registered() {
    let mut i = setup();
    assert_eq!(
        run(&mut i, "(lsp--get-connection \"cmd\" \"/root\")"),
        "nil"
    );
}

#[test]
fn get_connection_prunes_a_dead_entry_instead_of_returning_it() {
    let mut i = setup();
    ok(
        &mut i,
        "(setq lsp--connections
               (list (cons (cons \"cmd\" \"/root\") (make-lsp--client :conn nil))))",
    );
    assert_eq!(
        run(&mut i, "(lsp--get-connection \"cmd\" \"/root\")"),
        "nil"
    );
    assert_eq!(run(&mut i, "lsp--connections"), "nil");
}

#[test]
fn get_connection_reuses_a_live_entry_without_pruning_it() {
    let mut i = setup();
    ok(&mut i, "(setq test--client (lsp-connect \"cat\" nil nil))");
    ok(
        &mut i,
        "(setq lsp--connections (list (cons (cons \"cat-key\" \"/root\") test--client)))",
    );
    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--get-connection \"cat-key\" \"/root\") test--client)"
        ),
        "t"
    );
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

#[test]
fn lsp_connect_records_its_own_command_on_the_client() {
    // M59 tail-review round: `lsp-connect' is the ONLY real call site
    // that builds a live `lsp--client' (every M59 test builds one by
    // hand via `make-lsp--client' + `setf', bypassing this entirely) --
    // this is the one test standing guard on `(make-lsp--client :conn
    // conn :command command)' actually wiring COMMAND through, so
    // `lsp--client-command' can tell `lsp--references-empty-message'
    // which server it's talking to.
    let mut i = setup();
    ok(&mut i, "(setq test--client (lsp-connect \"cat\" nil nil))");
    assert_eq!(run(&mut i, "(lsp--client-command test--client)"), "\"cat\"");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

// ============================================================
// M-x lsp: clean degradation
// ============================================================

#[test]
fn lsp_command_reports_missing_registration_without_signaling() {
    let mut i = setup();
    let dir = scratch_dir("no_reg");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    // fundamental-mode (what a .txt file gets) has no lsp-server-alist entry.
    assert_eq!(
        run(&mut i, "(lsp)"),
        "\"No LSP server registered for fundamental-mode\""
    );
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

#[test]
fn lsp_command_reports_no_file_without_signaling() {
    let mut i = setup();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'fundamental-mode (list \"rust-analyzer\")))",
    );
    ok(&mut i, "(switch-to-buffer-internal \"*scratch-lsp-test*\")");
    // A freshly created buffer's major-mode is unset (nil) until
    // something dispatches one -- switch-to-buffer-internal doesn't,
    // unlike find-file's normal-mode call -- so set it explicitly to
    // reach the file-visiting check this test is actually about.
    ok(&mut i, "(fundamental-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"Buffer is not visiting a file\"");
}

// ============================================================
// M66: `M-x lsp' on a remote (`/ssh:') buffer. `lsp--auto-attach-
// client' has had this guard since M63; `lsp' itself did not, so it
// used to walk `lsp--project-root''s ancestor-directory marker scan
// on a remote path -- each `file-exists-p' there really shells out an
// `ssh ... test -e' (see `crates/core/src/builtins/files.rs' and
// `crates/core/src/remote.rs'). Real-TUI measurement with a fake-ssh
// hook: a 4-directory-deep `/ssh:' path cost 32 synchronous ssh calls
// and a 6.15s freeze at a 150ms RTT before this guard existed.
//
// A `/ssh:' buffer is faked via `fset'-ing `buffer-file-name' (M65-
// era technique, confirmed working against a builtin at
// crates/elisp/src/builtins/data.rs:325) rather than a real
// `find-file' on a `/ssh:' path, which would actually shell out.
// ============================================================

#[test]
fn lsp_command_reports_and_declines_a_remote_ssh_buffer() {
    // T1: the user-visible message, and that `(lsp)' never signals.
    let mut i = setup();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'lsp-test--ssh-mode-1 (list \"cat\")))",
    );
    ok(
        &mut i,
        "(switch-to-buffer-internal \"*scratch-lsp-ssh-1*\")",
    );
    ok(&mut i, "(major-mode-internal-set 'lsp-test--ssh-mode-1)");
    ok(
        &mut i,
        "(fset 'buffer-file-name (lambda (&optional _) \"/ssh:buildhost:/proj/rtl/core/alu.sv\"))",
    );
    // Stubbed for HERMETICITY, not for what this test asserts: if the
    // guard ever stops firing, the real `lsp--project-root' walks the
    // ancestors of a `/ssh:' path, and every `file-exists-p' on the way
    // up shells out to the system `ssh' (remote.rs) against a host that
    // does not exist -- so a broken guard would turn this test into a
    // network-dependent multi-second hang instead of a fast assertion
    // failure. Same reason the mutation run for this milestone is safe
    // to point at this file.
    ok(&mut i, "(fset 'lsp--project-root (lambda (f) f))");
    assert_eq!(
        run(&mut i, "(lsp)"),
        "\"LSP: remote (/ssh:) files are not supported\""
    );
}

#[test]
fn lsp_command_on_a_remote_buffer_never_calls_project_root() {
    // T2 (most important): the whole point of the guard is to avoid
    // the ssh storm `lsp--project-root' would otherwise trigger, so
    // this counts calls to it directly rather than trusting the
    // message text alone.
    let mut i = setup();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'lsp-test--ssh-mode-2 (list \"cat\")))",
    );
    ok(
        &mut i,
        "(switch-to-buffer-internal \"*scratch-lsp-ssh-2*\")",
    );
    ok(&mut i, "(major-mode-internal-set 'lsp-test--ssh-mode-2)");
    ok(
        &mut i,
        "(fset 'buffer-file-name (lambda (&optional _) \"/ssh:buildhost:/proj/rtl/core/alu.sv\"))",
    );
    ok(&mut i, "(setq test--project-root-calls 0)");
    ok(
        &mut i,
        "(fset 'lsp--project-root
               (lambda (f) (setq test--project-root-calls (1+ test--project-root-calls)) f))",
    );
    ok(&mut i, "(lsp)");
    assert_eq!(run(&mut i, "test--project-root-calls"), "0");
}

#[test]
fn lsp_command_on_a_remote_buffer_leaves_buffer_client_nil() {
    // T3: the guard fires before any connection attempt, so nothing
    // gets attached.
    let mut i = setup();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'lsp-test--ssh-mode-3 (list \"cat\")))",
    );
    ok(
        &mut i,
        "(switch-to-buffer-internal \"*scratch-lsp-ssh-3*\")",
    );
    ok(&mut i, "(major-mode-internal-set 'lsp-test--ssh-mode-3)");
    ok(
        &mut i,
        "(fset 'buffer-file-name (lambda (&optional _) \"/ssh:buildhost:/proj/rtl/core/alu.sv\"))",
    );
    // Hermeticity, same reason as T1: a broken guard must fail here as
    // an assertion, not as a real `ssh' to a host that does not exist.
    ok(&mut i, "(fset 'lsp--project-root (lambda (f) f))");
    ok(&mut i, "(lsp)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

#[test]
fn lsp_command_local_buffer_still_connects_after_remote_guard() {
    // T4: the new remote guard must not disturb the existing local
    // success path -- same `cat'-as-fake-server shape as
    // `lsp_command_connects_reuses_and_did_opens_via_a_real_cat_subprocess'.
    let mut i = setup();
    let dir = scratch_dir("remote_guard_local_still_ok");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    let r = run(&mut i, "(lsp)");
    assert_eq!(r, "\"LSP: connected to cat\"");
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn remote_path_p_unit() {
    // T5: the shared predicate's own truth table, independent of `lsp'
    // or `lsp--auto-attach-client'.
    let mut i = setup();
    assert_eq!(run(&mut i, "(lsp--remote-path-p \"/ssh:h:/x/a.sv\")"), "t");
    assert_eq!(
        run(&mut i, "(lsp--remote-path-p \"/proj/rtl/a.sv\")"),
        "nil"
    );
    assert_eq!(run(&mut i, "(lsp--remote-path-p nil)"), "nil");
    // Similar-looking but NOT the "/ssh:" prefix -- must not false-positive.
    assert_eq!(run(&mut i, "(lsp--remote-path-p \"/sshfoo/a.sv\")"), "nil");
}

#[test]
fn lsp_command_degrades_cleanly_when_the_server_binary_does_not_exist() {
    let mut i = setup();
    let dir = scratch_dir("missing_bin");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist
               (cons 'lsp-test--fake-mode (list \"definitely-not-a-real-lsp-binary-xyz-926\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'lsp-test--fake-mode)");

    // The command itself must not signal: its return value is the
    // friendly message text (see misc.rs: `message` returns what it
    // formats), so this is really asserting "no ERROR: prefix" *and*
    // pinning the friendly wording in one assertion.
    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "M-x lsp signaled: {}", r);
    assert!(r.contains("failed to start"), "message: {}", r);
    assert!(
        r.contains("definitely-not-a-real-lsp-binary-xyz-926"),
        "message: {}",
        r
    );

    // No half-registered state left behind.
    assert_eq!(run(&mut i, "lsp--connections"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Editor state is unharmed: buffer content intact, interpreter fine.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"fn main() {}\\n\"");
    assert_eq!(run(&mut i, "(+ 1 2)"), "3");
}

#[test]
fn lsp_command_degrades_cleanly_when_the_server_process_dies_mid_handshake() {
    // "true" exists on every Unix but exits immediately without ever
    // answering `initialize`, exercising the *other* documented lsp.el
    // failure mode: `lsp-start` succeeds (spawn worked) but
    // `lsp--await` then reports the server died, via `(error ...)`, not
    // a spawn failure. Same `condition-case` in `lsp`, different Rust
    // path (`LspEvent::Died` instead of `lsp-start`'s Err branch).
    let mut i = setup();
    let dir = scratch_dir("dies_fast");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'lsp-test--dies-mode (list \"true\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'lsp-test--dies-mode)");

    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "M-x lsp signaled: {}", r);
    assert!(r.contains("failed to start"), "message: {}", r);
    assert_eq!(run(&mut i, "lsp--connections"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(+ 1 2)"), "3");
}

#[test]
fn lsp_command_connects_reuses_and_did_opens_via_a_real_cat_subprocess() {
    // "cat" is a real, always-spawnable process, so this drives the
    // *success* path of `lsp` end to end through the real Rust
    // transport (unlike the fset-stubbed tests below): spawn, the
    // initialize handshake (see the module doc for why cat's byte-for-
    // byte echo satisfies it), didOpen, buffer-local client wiring, and
    // reusing the same connection for a second buffer in the same
    // fabricated "project".
    let mut i = setup();
    let dir = scratch_dir("cat_connect");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    let r = run(&mut i, "(lsp)");
    assert_eq!(r, "\"LSP: connected to cat\"");
    assert_eq!(
        run(
            &mut i,
            "(lsp-connection-p (lsp--client-conn lsp--buffer-client))"
        ),
        "t"
    );
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    let r = run(&mut i, "(lsp)");
    // M63: `find-file' on b.rs already auto-attached it (same project
    // root + command as a.rs's connection), so this `(lsp)' reports
    // "already connected" rather than doing a fresh connect -- same
    // reuse this test was always pinning, just reached one step earlier.
    assert_eq!(r, "\"LSP: already connected to cat\"");
    // Same project root (both files share the same .git ancestor) and
    // same command: the second `lsp` call must reuse, not duplicate.
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// M44: textDocument/didOpen's languageId -- `lsp` used to call
// `lsp-did-open` with no `language-id` argument at all, so every
// buffer, regardless of major mode, reported itself as "rust" (see the
// lsp.el file header). Undetected until now because the servers these
// tests previously exercised (clangd, pyright) infer the language from
// the URI's file extension rather than trusting this field -- so these
// two go straight at the field itself via the same cat-echo technique
// as the connect test above, plus `drain_frames` (defined further down,
// first used by the M35 didChange tests) to pull the didOpen frame back
// out and inspect it.
// ============================================================

#[test]
fn lsp_command_did_open_sends_verilog_language_id_for_verilog_mode() {
    let mut i = setup();
    let dir = scratch_dir("langid_verilog");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("t.v");
    std::fs::write(&file, "module m; endmodule\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    let r = run(&mut i, "(lsp)");
    assert_eq!(r, "\"LSP: connected to cat\"");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        1
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"languageId\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))"
        ),
        "\"verilog\""
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn lsp_command_did_open_sends_c_language_id_for_c_mode() {
    // Regression guard, not just new-feature coverage: before M44's
    // fix, `c-mode` -- exercised for years by the clangd manual e2e test
    // -- got the "rust" fallback too, just silently, since clangd
    // ignores languageId and infers C from the ".c" URI extension
    // instead. This pins the field itself so a future change to the
    // `lsp' call site can't regress it back to always-"rust" unnoticed.
    let mut i = setup();
    let dir = scratch_dir("langid_c");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("t.c");
    std::fs::write(&file, "int main(void) { return 0; }\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'c-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'c-mode)");
    let r = run(&mut i, "(lsp)");
    assert_eq!(r, "\"LSP: connected to cat\"");

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        1
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"languageId\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))"
        ),
        "\"c\""
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// Async hover-at-point (logic only: lsp-request-async is stubbed)
// ============================================================

#[test]
fn hover_at_point_sends_0_based_position_and_delivers_via_show_hover_popup() {
    let mut i = setup();
    let dir = scratch_dir("hover_spy");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "abc\ndef\nghij\n").unwrap();

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 2)"); // start of "ghij" -- 0-based line 2
    ok(&mut i, "(forward-char 2)"); // ...character 2 into it

    // Capture what lsp-hover-at-point asks for instead of sending
    // anything -- isolates "did it compute the right position and wire
    // the right callback" from the transport (covered separately by
    // the cat-subprocess test above and by lsp_async_tests.rs).
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-hover-at-point)");

    assert_eq!(run(&mut i, "(car test--captured)"), "fake-client");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/hover\""
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"line\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "2"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"character\" (gethash \"position\" (nth 2 test--captured)))"
        ),
        "2"
    );

    // Firing the captured callback must reach show-hover-popup with the
    // extracted hover text (stub it too, rather than touching the real
    // GUI/TUI echo-area plumbing that M16 already tests).
    ok(&mut i, "(setq test--popup nil)");
    ok(
        &mut i,
        "(fset 'show-hover-popup (lambda (text) (setq test--popup text)))",
    );
    ok(
        &mut i,
        "(funcall (nth 3 test--captured)
               (json-parse-string \"{\\\"contents\\\":\\\"fn add(a, b)\\\"}\"))",
    );
    assert_eq!(run(&mut i, "test--popup"), "\"fn add(a, b)\"");
}

#[test]
fn hover_at_point_reports_when_no_client_is_connected() {
    let mut i = setup();
    let dir = scratch_dir("hover_noclient");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hi\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    assert_eq!(
        run(&mut i, "(lsp-hover-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
}

// ============================================================
// Async definition-at-point + marker ring (logic only, then one real
// subprocess round trip)
// ============================================================

#[test]
fn definition_at_point_jumps_and_marker_ring_returns() {
    let mut i = setup();
    let dir = scratch_dir("def_spy");
    std::fs::create_dir_all(&dir).unwrap();
    let origin_file = dir.join("origin.txt");
    let target_file = dir.join("target.txt");
    std::fs::write(&origin_file, "call add(1, 2)\n").unwrap();
    std::fs::write(&target_file, "fn add(a, b) {\n  a + b\n}\n").unwrap();

    ok(
        &mut i,
        &format!("(find-file {:?})", origin_file.to_str().unwrap()),
    );
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-char 5)"); // sitting on "add"
    assert_eq!(run(&mut i, "(point)"), "6");

    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(nth 1 test--captured)"),
        "\"textDocument/definition\""
    );
    // Nothing pushed onto the ring until the callback actually finds a
    // location -- a "no definition" answer must not perturb it.
    assert_eq!(run(&mut i, "(length lsp--marker-stack)"), "0");

    // Fire the callback with a fabricated single-Location result
    // pointing at target_file line 0 (built directly as hash tables to
    // avoid double-escaping a path inside a JSON string inside elisp
    // inside Rust).
    let fire = format!(
        "(let ((range (make-hash-table)) (start (make-hash-table)) (loc (make-hash-table)))
           (puthash \"line\" 0 start)
           (puthash \"character\" 3 start)
           (puthash \"start\" start range)
           (puthash \"uri\" {:?} loc)
           (puthash \"range\" range loc)
           (funcall (nth 3 test--captured) loc))",
        format!("file://{}", target_file.to_str().unwrap())
    );
    ok(&mut i, &fire);

    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", target_file.to_str().unwrap())
    );
    assert_eq!(run(&mut i, "(point)"), "1"); // forward-line 0 from point-min
    assert_eq!(run(&mut i, "(length lsp--marker-stack)"), "1");

    // M-, must return to the exact origin buffer and position.
    ok(&mut i, "(lsp-pop-definition-stack)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", origin_file.to_str().unwrap())
    );
    assert_eq!(run(&mut i, "(point)"), "6");
    assert_eq!(run(&mut i, "lsp--marker-stack"), "nil");
}

#[test]
fn definition_at_point_reports_no_definition_found_and_does_not_touch_the_ring() {
    let mut i = setup();
    let dir = scratch_dir("def_none");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(funcall (nth 3 test--captured) nil)"),
        "\"No definition found\""
    );
    assert_eq!(run(&mut i, "(length lsp--marker-stack)"), "0");
}

// ============================================================
// M55: `local-definition-function' dispatch tier -- see that
// variable's own docstring in lsp.el and verilog-nav.el's header.
// ============================================================

#[test]
fn local_definition_function_handling_the_call_skips_the_lsp_request_entirely() {
    let mut i = setup();
    let dir = scratch_dir("local_def_skips_lsp");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client 'fake-client)");
    ok(
        &mut i,
        "(setq-local local-definition-function (lambda () t))",
    );
    ok(&mut i, "(setq test--captured nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async
               (lambda (client method params callback)
                 (setq test--captured (list client method params callback))
                 99))",
    );
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "test--captured"),
        "nil",
        "local-definition-function returning non-nil must stop `lsp-definition-at-point' before it ever sends a request"
    );
}

#[test]
fn local_definition_function_runs_and_can_jump_even_with_no_lsp_client_connected() {
    let mut i = setup();
    let dir = scratch_dir("local_def_no_client");
    std::fs::create_dir_all(&dir).unwrap();
    let origin_file = dir.join("origin.txt");
    let target_file = dir.join("target.txt");
    std::fs::write(&origin_file, "hello\n").unwrap();
    std::fs::write(&target_file, "world\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file {:?})", origin_file.to_str().unwrap()),
    );
    // No `lsp--buffer-client' set at all -- `(not live)' would normally
    // fire first; `local-definition-function' must run BEFORE that
    // check, not after it, for this to matter at all.
    ok(
        &mut i,
        &format!(
            "(setq-local local-definition-function
                   (lambda () (find-file {:?}) t))",
            target_file.to_str().unwrap()
        ),
    );
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", target_file.to_str().unwrap())
    );
}

// M55 review (A1, most severe finding): the two tests above stub
// `local-definition-function' with an inline lambda, and every one of
// `verilog_nav_tests.rs''s 20 tests calls `verilog-goto-module-at-point'
// DIRECTLY -- nothing anywhere called `(lsp-definition-at-point)' on a
// REAL `verilog-mode' buffer to prove `modes.el''s own
// `(setq-local local-definition-function 'verilog-goto-module-at-point)'
// line actually wires the two together. Deleting that one line
// (mutation M5) survived both test files' full suites. This test closes
// that gap: opens a real `.sv' file (major mode picked up via
// `auto-mode-alist' dispatch inside `find-file-internal', same as a real
// `C-x C-f' -- no manual `(verilog-mode)' call, no manual `setq-local'
// of anything), places point on an instantiated module's type name, and
// calls `lsp-definition-at-point' (the actual `M-.'/`g d' binding, not
// `verilog-goto-module-at-point' directly) end to end.
#[test]
fn verilog_mode_wires_local_definition_function_end_to_end() {
    let mut i = setup();
    let dir = scratch_dir("verilog_e2e_dispatch");
    std::fs::create_dir_all(&dir).unwrap();
    let origin_path = dir.join("origin.sv");
    let target_path = dir.join("fifo.sv");
    std::fs::write(
        &origin_path,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n",
    )
    .unwrap();
    std::fs::write(&target_path, "module fifo (input wr);\nendmodule\n").unwrap();

    // `.sv' extension -> `auto-mode-alist' -> `verilog-mode' ->
    // `verilog-mode-hook' -> `local-definition-function' set, all inside
    // this one call (see `editing.rs''s `find-file-internal', which runs
    // `run_normal_mode' before returning).
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", origin_path.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "local-definition-function"),
        "verilog-goto-module-at-point",
        "verilog-mode-hook must have set this without any manual setq-local"
    );
    // No `lsp--buffer-client' at all -- proves this path needs no LSP
    // server, matching the M55 repro this milestone is built against.
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(search-forward \"fifo\")"); // right after the type name's own tail
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", target_path.to_str().unwrap()),
        "M-./g d must have followed the type name to fifo.sv with no LSP client at all"
    );
}

#[test]
fn pop_definition_stack_reports_when_empty_or_when_the_target_buffer_was_killed() {
    let mut i = setup();
    assert_eq!(
        run(&mut i, "(lsp-pop-definition-stack)"),
        "\"No previous position to return to\""
    );

    ok(
        &mut i,
        "(setq test--b (generate-new-buffer \"lsp-pop-test\"))",
    );
    ok(
        &mut i,
        "(setq lsp--marker-stack
               (list (cons test--b (with-current-buffer test--b (point-marker)))))",
    );
    ok(&mut i, "(kill-buffer test--b)");
    assert_eq!(
        run(&mut i, "(lsp-pop-definition-stack)"),
        "\"Buffer for previous position no longer exists\""
    );
}

#[test]
fn definition_at_point_round_trips_through_a_real_cat_subprocess() {
    // Unlike the fset-stubbed tests above, this drives the real Rust
    // transport: spawn -> send -> background reader thread -> lsp-poll
    // -> lsp--dispatch -> the registered callback, with `cat` standing
    // in for a server (see the module doc). Deterministic and fast
    // (a local pipe echo), unlike rust-analyzer's real indexing delay.
    let mut i = setup();
    let dir = scratch_dir("cat_def_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello world\n").unwrap();

    ok(&mut i, "(setq test--client (lsp-connect \"cat\" nil nil))");
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq-local lsp--buffer-client test--client)");
    ok(&mut i, "(goto-char (point-min))");

    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(length (lsp--client-callbacks test--client))"),
        "1"
    );

    let mut delivered = false;
    for _ in 0..200 {
        let r = run(&mut i, "(lsp-process-pending test--client)");
        assert!(
            !r.starts_with("ERROR"),
            "lsp-process-pending signaled: {}",
            r
        );
        if run(&mut i, "(length (lsp--client-callbacks test--client))") == "0" {
            delivered = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        delivered,
        "callback was never delivered through the cat round trip"
    );
    // cat's echo has no "result" key, so lsp--definition-location parses
    // it as "no location" and the real (non-stubbed) callback takes the
    // "No definition found" branch, which never touches the ring.
    assert_eq!(run(&mut i, "(length lsp--marker-stack)"), "0");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--client))");
}

// ============================================================
// Diagnostic navigation: M-g n / M-g p
// ============================================================

#[test]
fn diagnostic_navigation_jumps_forward_backward_and_wraps() {
    let mut i = setup();
    let dir = scratch_dir("diag_nav");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "line0\nline1\nline2\nline3\n").unwrap();

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(setq test--client (make-lsp--client :conn nil))");
    ok(&mut i, "(setq-local lsp--buffer-client test--client)");

    // Two diagnostics at (0-based) lines 1 and 3, in the same shape
    // lsp--dispatch stores from a real publishDiagnostics notification.
    ok(
        &mut i,
        r#"(setf (lsp--client-diagnostics test--client)
               (list (cons (lsp--path-to-uri (buffer-file-name))
                           (json-parse-string "[{\"range\":{\"start\":{\"line\":1,\"character\":0}},\"message\":\"first\"},{\"range\":{\"start\":{\"line\":3,\"character\":0}},\"message\":\"second\"}]"))))"#,
    );

    ok(&mut i, "(goto-char (point-min))");
    assert_eq!(run(&mut i, "(next-diagnostic)"), "\"first\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");

    assert_eq!(run(&mut i, "(next-diagnostic)"), "\"second\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "4");

    // Past the last diagnostic: wraps to the first.
    assert_eq!(run(&mut i, "(next-diagnostic)"), "\"first\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");

    // previous-diagnostic from the first wraps to the last.
    assert_eq!(run(&mut i, "(previous-diagnostic)"), "\"second\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "4");

    assert_eq!(run(&mut i, "(previous-diagnostic)"), "\"first\"");
    assert_eq!(run(&mut i, "(line-number-at-pos)"), "2");
}

#[test]
fn diagnostic_navigation_reports_no_diagnostics_when_none_published() {
    let mut i = setup();
    let dir = scratch_dir("diag_none");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.txt");
    std::fs::write(&file, "hello\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    // No client connected at all: M63 changed this from "No diagnostics"
    // (which reads as "your code is clean", a false claim with no server
    // even attached) to the same "no connection" message every other LSP
    // command already gives.
    assert_eq!(
        run(&mut i, "(next-diagnostic)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
    assert_eq!(
        run(&mut i, "(previous-diagnostic)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );

    // A connected client that simply hasn't published anything yet: this
    // is the one case that still says "No diagnostics".
    ok(
        &mut i,
        "(setq-local lsp--buffer-client (make-lsp--client :conn nil))",
    );
    assert_eq!(run(&mut i, "(next-diagnostic)"), "\"No diagnostics\"");
    assert_eq!(run(&mut i, "(previous-diagnostic)"), "\"No diagnostics\"");
}

// ============================================================
// Manual real-server end-to-end verification (M-x lsp -> hover ->
// definition -> M-, back). Mirrors lsp_tests.rs's #[ignore]d
// real-rust-analyzer test: skipped (not failed) when the tool isn't on
// PATH, and excluded from the default run since it depends on a real
// external process and can be flaky under parallel test-binary
// contention. Run deliberately with e.g.
//   cargo test -p core --test lsp_mode_tests -- --ignored --test-threads=1
// ============================================================

/// Whether CMD resolves to a real executable on PATH -- checked by
/// spawning it with no meaningful arguments and killing it immediately
/// rather than running `--version` and checking success, since not
/// every language server implements that flag the same way
/// (`pyright-langserver --version` exits 1: it only recognizes
/// `--stdio`/`--node-ipc`/`--socket`, not `--version`, and would make
/// this check falsely report "not installed"). A successful spawn
/// proves the OS found and started the binary; what it does with the
/// argument doesn't matter since the process is killed before it can
/// do anything -- notably before an LSP server would sit forever
/// waiting on stdin.
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

#[test]
#[ignore]
fn manual_e2e_rust_analyzer_hover_definition_pop_on_this_repo() {
    if !have_on_path("rust-analyzer") {
        eprintln!("skipping: rust-analyzer not on PATH");
        return;
    }
    let mut i = setup();
    // A real file in this repo (not a scratch project): hovering/
    // jumping from `Interp` in `use elisp::{Interp, Value};` exercises
    // real cross-crate navigation (commands.rs is in `core`, `Interp`
    // is defined over in the `elisp` crate) and, since
    // `lsp--project-root` resolves to `crates/core` (the nearest
    // Cargo.toml, not the workspace root -- see `lsp--project-root-markers`),
    // also verifies rust-analyzer still finds the whole workspace when
    // started from a member crate's directory.
    let commands_rs = concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands.rs");
    let text = std::fs::read_to_string(commands_rs).unwrap();
    let line_idx = text
        .lines()
        .position(|l| l.trim() == "use elisp::{Interp, Value};")
        .expect("marker line not found in commands.rs -- did it move?");
    let char_idx = text.lines().nth(line_idx).unwrap().find("Interp").unwrap() + 2;
    println!(
        "target: {}:{}:{} (0-based)",
        commands_rs, line_idx, char_idx
    );

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"rust-analyzer\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", commands_rs));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to rust-analyzer\"");
    println!(
        "project root resolved to => {}",
        run(&mut i, &format!("(lsp--project-root {:?})", commands_rs))
    );

    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, &format!("(forward-line {})", line_idx));
    ok(&mut i, &format!("(forward-char {})", char_idx));
    let origin_point = run(&mut i, "(point)");
    println!("origin point => {}", origin_point);

    // --- C-h . : hover ---
    ok(&mut i, "(setq test--popup nil)");
    ok(
        &mut i,
        "(fset 'show-hover-popup (lambda (text) (setq test--popup text)))",
    );
    ok(&mut i, "(lsp-hover-at-point)");
    let mut hover_seen: Option<String> = None;
    // This is a real multi-crate workspace (unlike lsp_tests.rs's tiny
    // two-file scratch project), so a cold rust-analyzer index can
    // genuinely take minutes, not seconds -- generous bound, still
    // finite so a real failure doesn't hang the run forever.
    //
    // While indexing, rust-analyzer answers hover immediately with
    // null -- a real, delivered answer that leaves the popup empty.
    // A human just presses C-h . again, so the loop re-issues the
    // request periodically instead of waiting forever on a request
    // that was already consumed.
    for attempt in 0..600 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        let popup = run(&mut i, "test--popup");
        if popup != "nil" {
            hover_seen = Some(popup);
            break;
        }
        if attempt % 15 == 14 {
            ok(&mut i, "(lsp-hover-at-point)");
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if attempt % 10 == 0 {
            eprintln!("waiting for hover... attempt {attempt}");
        }
    }
    println!("hover result => {:?}", hover_seen);
    assert!(hover_seen.is_some(), "rust-analyzer never answered hover");
    assert!(
        hover_seen.as_ref().unwrap().contains("Interp"),
        "hover text doesn't look like it resolved `Interp`: {:?}",
        hover_seen
    );

    // --- M-. : go to definition ---
    ok(&mut i, "(lsp-definition-at-point)");
    let mut jumped = false;
    for attempt in 0..120 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        let file = run(&mut i, "(buffer-file-name)");
        if file.contains("interp.rs") {
            jumped = true;
            break;
        }
        // Same re-issue logic as the hover loop: an empty locations
        // answer during indexing is consumed silently.
        if attempt % 15 == 14 {
            ok(&mut i, "(lsp-definition-at-point)");
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if attempt % 10 == 0 {
            eprintln!("waiting for definition... attempt {attempt}, current buffer file: {file}");
        }
    }
    println!("jumped to => {}", run(&mut i, "(buffer-file-name)"));
    println!("point after jump => {}", run(&mut i, "(point)"));
    assert!(
        jumped,
        "rust-analyzer never answered definition, or it didn't land in interp.rs"
    );

    // --- M-, : pop back to the origin ---
    ok(&mut i, "(lsp-pop-definition-stack)");
    println!(
        "after M-, => file={} point={}",
        run(&mut i, "(buffer-file-name)"),
        run(&mut i, "(point)")
    );
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", commands_rs)
    );
    assert_eq!(run(&mut i, "(point)"), origin_point);

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!("PASS: rust-analyzer end-to-end (M-x lsp -> hover -> definition -> M-,) on this repo");
}

#[test]
#[ignore]
fn manual_e2e_clangd_hover_definition_pop_on_a_small_c_file() {
    if !have_on_path("clangd") {
        eprintln!("skipping: clangd not on PATH");
        return;
    }
    let mut i = setup();
    let dir = scratch_dir("clangd_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.c");
    std::fs::write(
        &file,
        "int add(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    int result = add(1, 2);\n    return result;\n}\n",
    )
    .unwrap();
    println!("target: {}", file.to_str().unwrap());

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'c-mode (list \"clangd\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'c-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to clangd\"");

    // Land on the `add` call inside main (0-based line 5, "    int result = add(1, 2);").
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 5)");
    ok(&mut i, "(forward-char 17)"); // inside "add"
    let origin_point = run(&mut i, "(point)");
    println!("origin point => {}", origin_point);

    ok(&mut i, "(setq test--popup nil)");
    ok(
        &mut i,
        "(fset 'show-hover-popup (lambda (text) (setq test--popup text)))",
    );
    ok(&mut i, "(lsp-hover-at-point)");
    let mut hover_seen: Option<String> = None;
    for attempt in 0..60 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        let popup = run(&mut i, "test--popup");
        if popup != "nil" {
            hover_seen = Some(popup);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if attempt % 10 == 0 {
            eprintln!("waiting for hover... attempt {attempt}");
        }
    }
    println!("hover result => {:?}", hover_seen);
    assert!(hover_seen.is_some(), "clangd never answered hover");

    ok(&mut i, "(lsp-definition-at-point)");
    let mut jumped = false;
    for attempt in 0..60 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        if run(&mut i, "(point)") == "1" {
            jumped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if attempt % 10 == 0 {
            eprintln!("waiting for definition... attempt {attempt}");
        }
    }
    println!(
        "jumped to => file={} point={}",
        run(&mut i, "(buffer-file-name)"),
        run(&mut i, "(point)")
    );
    assert!(
        jumped,
        "clangd never answered definition (or it didn't land at line 0)"
    );

    ok(&mut i, "(lsp-pop-definition-stack)");
    println!(
        "after M-, => file={} point={}",
        run(&mut i, "(buffer-file-name)"),
        run(&mut i, "(point)")
    );
    assert_eq!(run(&mut i, "(point)"), origin_point);

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!("PASS: clangd end-to-end (M-x lsp -> hover -> definition -> M-,) on a small C file");
}

// M46: unlike its `#[ignore]`d neighbours this one runs by default. It
// had both gates -- `#[ignore]` *and* the PATH check below -- which meant
// the Verilog round trip never ran unless someone remembered
// `-- --ignored`. The PATH check alone is the right gate: machines with
// verible-verilog-ls installed (the ones whose primary language this
// editor targets) exercise it every run, everyone else still skips.
#[test]
fn manual_e2e_verible_verilog_ls_connects_and_publishes_diagnostics_on_a_small_sv_file() {
    // Unlike the rust-analyzer/clangd/pyright tests above, this doesn't
    // chase hover/definition: verible-verilog-ls is primarily a linter,
    // and its hover/definition support is limited enough that it isn't
    // a reliable connectivity signal. `textDocument/publishDiagnostics`
    // arriving at all is: it only happens after a real spawn, a real
    // `initialize` handshake, and a real didOpen the server parsed, so
    // `lsp--client-diagnostics' going non-nil (via `lsp--merge-
    // diagnostics', which the idle pump already wires up for every
    // client -- see `lsp-process-pending-all') is proof the whole round
    // trip, languageId included, worked. Diagnostic content/count is
    // deliberately not asserted -- verible's message wording isn't a
    // contract this test should pin.
    if !have_on_path("verible-verilog-ls") {
        eprintln!("skipping: verible-verilog-ls not on PATH");
        return;
    }
    let mut i = setup();
    let dir = scratch_dir("verible_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    // An unused signal and a missing semicolon after $display -- both
    // syntactically tolerated by verible's lenient parser, both the
    // kind of thing its lint rules flag.
    std::fs::write(
        &file,
        "module t;\n  logic unused_signal;\n  initial begin\n    $display(\"hi\")\n  end\nendmodule\n",
    )
    .unwrap();
    println!("target: {}", file.to_str().unwrap());

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"verible-verilog-ls\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to verible-verilog-ls\"");

    let mut diagnostics_seen = false;
    for attempt in 0..40 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        if run(&mut i, "(lsp--client-diagnostics lsp--buffer-client)") != "nil" {
            diagnostics_seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if attempt % 10 == 0 {
            eprintln!("waiting for diagnostics... attempt {attempt}");
        }
    }
    println!(
        "diagnostics => {}",
        run(&mut i, "(lsp--client-diagnostics lsp--buffer-client)")
    );
    assert!(
        diagnostics_seen,
        "verible-verilog-ls never published diagnostics"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!(
        "PASS: verible-verilog-ls end-to-end (M-x lsp -> publishDiagnostics) on a small SystemVerilog file"
    );
}

// ============================================================
// Review findings (M26): dead-client degradation + didOpen dedup
// ============================================================

/// A server that dies after connecting must degrade to the friendly
/// "M-x lsp first" message on the next hover/definition -- not leak a
/// raw "lsp connection stdin closed" signal from lsp-send -- and the
/// stale buffer-local client reference must be cleared on the way.
#[test]
fn hover_and_definition_degrade_cleanly_when_the_server_died_after_connecting() {
    let mut i = setup();
    let dir = scratch_dir("dead_client");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    // Kill the server out from under the buffer.
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");

    // Both commands must return the readable message, not signal.
    assert_eq!(
        ok(&mut i, "(lsp-hover-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
    // The stale reference was cleared by the first command.
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(
        ok(&mut i, "(lsp-definition-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
}

/// Repeating M-x lsp in an already-connected buffer must not re-send
/// textDocument/didOpen (the spec forbids opening an open document);
/// it reports "already connected" instead. Verified by counting the
/// didOpen frames cat echoes back.
#[test]
fn repeated_lsp_command_in_the_same_buffer_does_not_resend_did_open() {
    let mut i = setup();
    let dir = scratch_dir("didopen_dedup");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: already connected to cat\"");

    // Drain everything cat echoed back; exactly one didOpen may appear.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let opens = ok(
        &mut i,
        "(let ((n 0) (msg t) (conn (lsp--client-conn lsp--buffer-client)))
           (while msg
             (setq msg (lsp-poll conn))
             (when (and msg
                        (equal (gethash \"method\" msg nil)
                               \"textDocument/didOpen\"))
               (setq n (1+ n))))
           n)",
    );
    assert_eq!(
        opens, "1",
        "didOpen echoed back {} times, want exactly 1",
        opens
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// Manual e2e: real pyright-langserver (python-mode's default since
// the pylsp -> pyright swap)
// ============================================================

#[test]
#[ignore]
fn manual_e2e_pyright_hover_definition_pop_on_a_small_py_file() {
    if !have_on_path("pyright-langserver") {
        eprintln!("skipping: pyright-langserver not on PATH");
        return;
    }
    let mut i = setup();
    let dir = scratch_dir("pyright_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.py");
    std::fs::write(
        &file,
        "def add(a: int, b: int) -> int:\n    return a + b\n\n\ndef main() -> None:\n    result = add(1, 2)\n    print(result)\n",
    )
    .unwrap();
    println!("target: {}", file.to_str().unwrap());

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'python-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to pyright-langserver\"");

    // Land on the `add` call inside main (0-based line 5, "    result = add(1, 2)").
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 5)");
    ok(&mut i, "(forward-char 13)"); // inside "add"
    let origin_point = run(&mut i, "(point)");
    println!("origin point => {}", origin_point);

    ok(&mut i, "(setq test--popup nil)");
    ok(
        &mut i,
        "(fset 'show-hover-popup (lambda (text) (setq test--popup text)))",
    );
    ok(&mut i, "(lsp-hover-at-point)");
    let mut hover_seen: Option<String> = None;
    for attempt in 0..60 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        let popup = run(&mut i, "test--popup");
        if popup != "nil" {
            hover_seen = Some(popup);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if attempt % 10 == 0 {
            eprintln!("waiting for hover... attempt {attempt}");
            if attempt > 0 {
                ok(&mut i, "(lsp-hover-at-point)");
            }
        }
    }
    println!("hover result => {:?}", hover_seen);
    assert!(hover_seen.is_some(), "pyright never answered hover");
    assert!(
        hover_seen.as_ref().unwrap().contains("add"),
        "hover text doesn't look like it resolved `add`: {:?}",
        hover_seen
    );

    ok(&mut i, "(lsp-definition-at-point)");
    let mut jumped = false;
    for attempt in 0..60 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        if run(&mut i, "(point)") == "1" {
            jumped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if attempt % 10 == 0 {
            eprintln!("waiting for definition... attempt {attempt}");
        }
    }
    println!(
        "jumped to => file={} point={}",
        run(&mut i, "(buffer-file-name)"),
        run(&mut i, "(point)")
    );
    assert!(
        jumped,
        "pyright never answered definition (or it didn't land at line 0)"
    );

    ok(&mut i, "(lsp-pop-definition-stack)");
    println!(
        "after M-, => file={} point={}",
        run(&mut i, "(buffer-file-name)"),
        run(&mut i, "(point)")
    );
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", file.to_str().unwrap())
    );
    assert_eq!(run(&mut i, "(point)"), origin_point);

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!(
        "PASS: pyright end-to-end (M-x lsp -> hover -> definition -> M-,) on a small Python file"
    );
}

// ============================================================
// M35: edit sync (textDocument/didChange) -- full-text sync on every
// change, version = buffer-modified-tick (see the lsp.el file header
// for the design rationale). Same cat-echo technique as the M26 tests
// above; the two helpers below generalize that section's inline poll
// loop (which only ever counted didOpen frames) since these tests also
// need each frame's fields (version/text/uri) or their arrival order.
// ============================================================

/// Drain every frame `conn` has echoed back whose "method" is METHOD
/// into `test--frames` (oldest first) -- same one-shot-`lsp-poll`
/// draining the M26 didOpen-dedup test above uses, just keeping the
/// matches instead of only counting them (anything not matching
/// METHOD is still consumed off the queue, same as that test). Returns
/// the count as a convenience: most callers below only need that, a
/// few also inspect `test--frames` afterward.
fn drain_frames(i: &mut Interp, conn_expr: &str, method: &str) -> usize {
    let src = format!(
        "(let ((frames nil) (msg t))
           (while msg
             (setq msg (lsp-poll {conn}))
             (when (and msg (equal (gethash \"method\" msg nil) {method:?}))
               (setq frames (cons msg frames))))
           (setq test--frames (nreverse frames))
           (length test--frames))",
        conn = conn_expr,
        method = method,
    );
    ok(i, &src)
        .parse()
        .expect("frame count should print as an integer")
}

/// Drain every frame `conn` has echoed back into `test--methods`, in
/// arrival order, as a list of "method" strings -- for checking arrival
/// order (e.g. a didChange landing before the request it precedes)
/// rather than counting/inspecting one method's frames.
fn drain_methods(i: &mut Interp, conn_expr: &str) {
    let src = format!(
        "(let ((methods nil) (msg t))
           (while msg
             (setq msg (lsp-poll {conn}))
             (when msg (setq methods (cons (gethash \"method\" msg nil) methods))))
           (setq test--methods (nreverse methods)))",
        conn = conn_expr,
    );
    ok(i, &src);
}

#[test]
fn idle_pump_sends_did_change_once_after_an_edit() {
    let mut i = setup();
    let dir = scratch_dir("didchange_basic");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    // Drain the didOpen frame cat echoed back and capture its version.
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        1
    );
    let open_version: i64 = ok(
        &mut i,
        "(gethash \"version\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))",
    )
    .parse()
    .expect("didOpen version should be an integer");

    // Edit, then run the idle pump -- the mechanism under test.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);

    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        1,
        "want exactly one didChange after one edit + one pump"
    );
    assert_eq!(
        ok(&mut i, "(buffer-string)"),
        ok(
            &mut i,
            "(gethash \"text\" (aref (gethash \"contentChanges\" (gethash \"params\" (nth 0 test--frames))) 0))"
        ),
        "didChange's text should be the post-edit full buffer text"
    );
    let change_version: i64 = ok(
        &mut i,
        "(gethash \"version\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))",
    )
    .parse()
    .expect("didChange version should be an integer");
    assert!(
        change_version > open_version,
        "didChange version {} should be greater than didOpen version {}",
        change_version,
        open_version
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn idle_pump_coalesces_multiple_edits_into_one_did_change() {
    let mut i = setup();
    let dir = scratch_dir("didchange_coalesce");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(
        &mut i,
        "(lsp--client-conn lsp--buffer-client)",
        "textDocument/didOpen",
    );

    // Three edits, no pump run in between.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    ok(&mut i, "(insert \"fn c() {}\\n\")");
    ok(&mut i, "(insert \"fn d() {}\\n\")");

    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));

    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        1,
        "three edits with no pump in between should still coalesce into one didChange"
    );
    assert_eq!(
        ok(&mut i, "(buffer-string)"),
        ok(
            &mut i,
            "(gethash \"text\" (aref (gethash \"contentChanges\" (gethash \"params\" (nth 0 test--frames))) 0))"
        ),
        "the one didChange should carry the latest full text, not an intermediate edit"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn idle_pump_sends_no_did_change_when_nothing_was_edited() {
    let mut i = setup();
    let dir = scratch_dir("didchange_none");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(
        &mut i,
        "(lsp--client-conn lsp--buffer-client)",
        "textDocument/didOpen",
    );

    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));

    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        0,
        "no edit happened, so the pump must send no didChange"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn hover_at_point_syncs_the_buffer_before_sending_its_request() {
    let mut i = setup();
    let dir = scratch_dir("didchange_hover_order");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(
        &mut i,
        "(lsp--client-conn lsp--buffer-client)",
        "textDocument/didOpen",
    );

    // Edit, then go straight to hover -- deliberately WITHOUT running
    // the idle pump first, so the only sync that can happen is the
    // pre-request one inside `lsp-hover-at-point` itself.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    ok(&mut i, "(lsp-hover-at-point)");

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_methods(&mut i, "(lsp--client-conn lsp--buffer-client)");
    assert_eq!(
        run(&mut i, "test--methods"),
        "(\"textDocument/didChange\" \"textDocument/hover\")",
        "didChange must be sent before the hover request it precedes"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn idle_pump_syncs_each_buffer_independently_under_the_same_client() {
    let mut i = setup();
    let dir = scratch_dir("didchange_two_bufs");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    // M63: `find-file' on b.rs already auto-attached it to a's connection
    // (same project root + command), so this second `(lsp)' just reports
    // -- it no longer does a fresh "connected to" here. The shared-client
    // assertion right below is what this test actually cares about.
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: already connected to cat\"");
    // One shared client for both buffers (same project root + command).
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");

    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(
        &mut i,
        "(lsp--client-conn lsp--buffer-client)",
        "textDocument/didOpen",
    );

    // Edit b (the current buffer) and a (via with-current-buffer).
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b2() {}\\n\")");
    ok(
        &mut i,
        &format!(
            "(with-current-buffer (get-file-buffer {:?})
               (goto-char (point-max))
               (insert \"fn a2() {{}}\\n\"))",
            file_a.to_str().unwrap()
        ),
    );

    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));

    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        2,
        "each of the two edited buffers should produce its own didChange"
    );
    let uris = ok(
        &mut i,
        "(mapcar (lambda (f) (gethash \"uri\" (gethash \"textDocument\" (gethash \"params\" f)))) test--frames)",
    );
    assert!(
        uris.contains(&format!("file://{}", file_a.to_str().unwrap())),
        "missing a.rs's didChange: {}",
        uris
    );
    assert!(
        uris.contains(&format!("file://{}", file_b.to_str().unwrap())),
        "missing b.rs's didChange: {}",
        uris
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

#[test]
fn idle_pump_skips_a_buffer_whose_client_died_and_does_not_signal() {
    let mut i = setup();
    let dir = scratch_dir("didchange_dead");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");

    // Kill the server out from under the buffer, keeping the (now-dead)
    // connection handle so we can also confirm nothing arrived on it.
    ok(
        &mut i,
        "(setq test--dead-conn (lsp--client-conn lsp--buffer-client))",
    );
    ok(&mut i, "(lsp-kill test--dead-conn)");

    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(
        !r.starts_with("ERROR"),
        "idle pump signaled on a dead client: {}",
        r
    );

    // M35 review hardening changed WHO clears the stale reference: the
    // pump's first phase prunes the dead client from lsp--clients, and
    // the (when lsp--clients ...) short-circuit then skips the buffer
    // walk entirely — nothing to sync, nothing to pay for. The stale
    // buffer-local is therefore cleared lazily by the next interactive
    // command's liveness gate (M26), not by the pump:
    assert_eq!(
        ok(&mut i, "(lsp-hover-at-point)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\""
    );
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    let polled = run(&mut i, "(lsp-poll test--dead-conn)");
    assert!(
        !polled.contains("didChange"),
        "a didChange must never reach a dead connection: {}",
        polled
    );
}

#[test]
fn idle_pump_sends_strictly_increasing_versions_across_rounds() {
    let mut i = setup();
    let dir = scratch_dir("didchange_monotonic");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(
        &mut i,
        "(lsp--client-conn lsp--buffer-client)",
        "textDocument/didOpen",
    );

    // Round 1.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        1
    );
    let v1: i64 = ok(
        &mut i,
        "(gethash \"version\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))",
    )
    .parse()
    .expect("version should be an integer");

    // Round 2.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn c() {}\\n\")");
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        1
    );
    let v2: i64 = ok(
        &mut i,
        "(gethash \"version\" (gethash \"textDocument\" (gethash \"params\" (nth 0 test--frames))))",
    )
    .parse()
    .expect("version should be an integer");

    assert!(
        v2 > v1,
        "second didChange version {} should exceed the first's {}",
        v2,
        v1
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

/// Manual e2e (like the rust-analyzer test earlier in this file):
/// proves a real server sees an edit through `textDocument/didChange`
/// without ever reconnecting, by adding a brand new function after
/// `M-x lsp` and getting a real hover answer about it. Run deliberately
/// with e.g.
///   cargo test -p core --test lsp_mode_tests -- --ignored --test-threads=1
#[test]
#[ignore]
fn manual_e2e_rust_analyzer_sees_an_edit_via_did_change_without_reconnecting() {
    if !have_on_path("rust-analyzer") {
        eprintln!("skipping: rust-analyzer not on PATH");
        return;
    }
    let mut i = setup();
    let dir = scratch_dir("didchange_e2e");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"didchange_e2e\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_rs = dir.join("src/main.rs");
    std::fs::write(&main_rs, "fn main() {\n    println!(\"hi\");\n}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"rust-analyzer\")))",
    );
    ok(
        &mut i,
        &format!("(find-file {:?})", main_rs.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to rust-analyzer\"");

    // Edit: append a function that was never part of what didOpen sent.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"\\nfn newly_added() -> i32 { 42 }\\n\")");

    // Pump-sync -- the M35 mechanism under test -- before ever asking
    // anything about the new function.
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);

    // Land inside "newly_added" (0-based line 4, char 3: right after
    // main's closing brace, a blank line, then this line).
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(forward-line 4)");
    ok(&mut i, "(forward-char 3)");

    ok(&mut i, "(setq test--popup nil)");
    ok(
        &mut i,
        "(fset 'show-hover-popup (lambda (text) (setq test--popup text)))",
    );
    ok(&mut i, "(lsp-hover-at-point)");

    let mut hover_seen: Option<String> = None;
    // Same resend-polling technique as the rust-analyzer e2e earlier in
    // this file: a cold index can take a while, and a null hover while
    // indexing is a real, delivered answer -- a human just asks again,
    // so this does too.
    for attempt in 0..600 {
        let r = run(&mut i, "(lsp-process-pending-all)");
        assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
        let popup = run(&mut i, "test--popup");
        if popup != "nil" {
            hover_seen = Some(popup);
            break;
        }
        if attempt % 15 == 14 {
            ok(&mut i, "(lsp-hover-at-point)");
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
        if attempt % 10 == 0 {
            eprintln!("waiting for hover on newly_added... attempt {attempt}");
        }
    }
    println!("hover result => {:?}", hover_seen);
    assert!(
        hover_seen.is_some(),
        "rust-analyzer never answered hover on newly_added"
    );
    assert!(
        hover_seen.as_ref().unwrap().contains("newly_added"),
        "hover text doesn't look like it resolved `newly_added` (server may not have seen the edit): {:?}",
        hover_seen
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!(
        "PASS: rust-analyzer saw an edit via didChange without reconnecting (hover resolved newly_added)"
    );
}

/// M35 review's requested pin: connect, edit, run M-x lsp AGAIN in the
/// same buffer (the already-connected branch — which deliberately
/// touches neither lsp--buffer-client nor lsp--last-synced-tick), then
/// pump. The dirty tick must survive the re-run untouched: exactly one
/// didChange, no second didOpen.
#[test]
fn repeated_lsp_command_does_not_disturb_the_pending_did_change() {
    let mut i = setup();
    let dir = scratch_dir("didchange_reconnect_cmd");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: connected to cat\"");
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        1
    );

    // Edit, then re-run M-x lsp: already-connected, reports and stops.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");
    assert_eq!(run(&mut i, "(lsp)"), "\"LSP: already connected to cat\"");

    // Pump: the edit made before the re-run must still sync — once.
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didChange"
        ),
        1,
        "exactly one didChange for the pre-re-run edit"
    );
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn lsp--buffer-client)",
            "textDocument/didOpen"
        ),
        0,
        "the already-connected re-run must not have re-sent didOpen"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}
