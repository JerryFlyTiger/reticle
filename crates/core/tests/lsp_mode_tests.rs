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
    // M104 fix round: several tests here save real .rs files while
    // attached to a fake `cat' LSP connection whose capabilities never
    // populate -- the asymmetric `lsp--capability-supported-p' policy
    // treats that as "supported", so `format-on-save' (M104, default
    // `t') could route saves through `lsp-format-buffer' (or, for a
    // buffer with no LSP client at all, through the `rustfmt' fallback
    // if that happens to be installed), inserting extra frames or
    // silently rewriting buffer content depending on what's on THIS
    // machine's PATH. This file tests LSP wiring, not formatting, so it
    // opts out unconditionally.
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

/// M93 third fix round (W3): `$HOME` is process-global, not
/// thread-local, and `cargo test` runs this file's tests on multiple
/// threads by default. `HomeGuard' below overrides it; separately,
/// ANY `project_root_*' test that resolves a Verilog-suffixed file
/// (`.v'/`.vh'/`.sv'/`.svh') reads it too, transitively, through
/// `lsp--home-directory' (called from `lsp--nearest-filelist-root',
/// which only Verilog buffers reach). Both are real races against a
/// `HomeGuard' override running on another thread at the same moment
/// -- this project has twice shipped a race that only surfaced under
/// default thread counts (M65, M84; see this repo's own delegation
/// notes), so this is not treated as acceptable on "no path collision"
/// grounds alone the way the previous round's docstring argued.
///
/// This mutex is the fix: every test that either installs a
/// `HomeGuard' or reads `$HOME' via a Verilog-suffixed
/// `lsp--project-root' call takes `home_env_lock' for its own
/// duration, so an override window and a read window can never
/// overlap, regardless of thread count or scheduling.
static HOME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire the shared lock guarding `$HOME` for the life of the
/// returned guard. Poison-tolerant (`unwrap_or_else(PoisonError::
/// into_inner)`): one test panicking while holding this must not
/// permanently wedge every later test that also needs the lock.
fn home_env_lock() -> std::sync::MutexGuard<'static, ()> {
    HOME_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Same message-capturing shape used by `lsp_action_tests.rs`,
/// `lsp_autostart_tests.rs`, `lsp_format_tests.rs`, `lsp_highlight_tests.rs`,
/// `lsp_references_tests.rs`: `fset`s (via `defun`) a capturing lambda in
/// place of the real `message` builtin, since `Editor::echo`/`message` is
/// not observable through `Interp::out` (M62 precedent).
fn capture_messages(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

/// Overrides `$HOME` for the duration of one test and restores it on
/// drop (even on panic/assertion failure), the same discipline as
/// `ssh_tests.rs`'s `TmpdirGuard`. `lsp--home-directory' (lsp.el) reads
/// `$HOME' fresh via `expand-file-name' on every call rather than
/// caching it, specifically so this override takes effect immediately
/// -- see that function's own docstring.
///
/// Holds `home_env_lock' for its entire lifetime (acquired in `set',
/// released when the guard drops) -- see that function's own doc for
/// why a lock is needed at all, on top of the fact that the directory
/// this override points AT is always a scratch path no other test's
/// OWN file layout can ever collide with.
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
    // M93 third fix round (W3): this test resolves a `.sv`/`.vh`/
    // `.v`/`.svh` file, so `lsp--project-root` reads `$HOME` via
    // `lsp--home-directory` even though this test never overrides it
    // -- see `home_env_lock`'s own doc for why that still needs the
    // shared lock.
    let _lock = home_env_lock();
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
fn project_root_verilog_filelist_outranks_a_nearer_dot_git() {
    // M93: proj/verible.filelist plus proj/sub/.git/ -- a Verilog buffer
    // at proj/sub/buf.sv must resolve to `proj', not `proj/sub', because
    // the nearer `.git' would otherwise shadow the filelist that
    // actually defines the project (and this value becomes the LSP
    // server's own rootUri, so a wrong answer here misroots the server
    // too, not just this editor's local navigation).
    let mut i = setup();
    // M93 third fix round (W3): this test resolves a `.sv`/`.vh`/
    // `.v`/`.svh` file, so `lsp--project-root` reads `$HOME` via
    // `lsp--home-directory` even though this test never overrides it
    // -- see `home_env_lock`'s own doc for why that still needs the
    // shared lock.
    let _lock = home_env_lock();
    let root = scratch_dir("sv_filelist_outranks_git");
    std::fs::create_dir_all(root.join("sub/.git")).unwrap();
    std::fs::write(root.join("verible.filelist"), "sub/buf.sv\n").unwrap();
    let file = root.join("sub/buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn project_root_verilog_filelist_walk_is_bounded_at_home_and_does_not_shadow_a_nearer_dot_git() {
    // M93 fix round (R1): a stray `verible.filelist' living ABOVE
    // `$HOME' (a NAS mount point, a directory shared by several
    // unrelated checkouts, ...) must not outrank the buffer's own
    // much nearer `.git' -- reviewer traced this statically (not
    // reproduced) as "the filelist walk is unbounded and can leave the
    // project entirely."
    //
    // Layout: fake_home/proj/.git (the buffer's own project marker)
    // and fake_root/verible.filelist, where fake_root is fake_home's
    // OWN PARENT -- i.e. strictly above `$HOME' once `$HOME' is
    // overridden to fake_home. Before the R1 bound, the filelist walk
    // ignores `.git' entirely and climbs straight past `fake_home' to
    // `fake_root', finding the stray filelist and returning it --
    // reproduced by hand before this fix (see the M93 fix-round
    // report). After the bound, the walk stops climbing at
    // `fake_home' itself (never returning `fake_home', and never
    // looking above it), finds no filelist, and falls back to the
    // ordinary marker walk, landing on `proj' (the `.git' directory).
    let mut i = setup();
    let fake_root = scratch_dir("home_bound");
    let fake_home = fake_root.join("home");
    let proj = fake_home.join("proj");
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    std::fs::write(fake_root.join("verible.filelist"), "home/proj/buf.sv\n").unwrap();
    let file = proj.join("buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let _home_guard = HomeGuard::set(&fake_home);

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", proj.to_str().unwrap()));
}

#[test]
fn project_root_verilog_filelist_at_home_itself_is_not_honoured() {
    // M93 third fix round (W1): pins DELIBERATE behaviour, not a bug --
    // a `verible.filelist' sitting EXACTLY AT `$HOME', with the
    // buffer's own nearer `.git' below it, must resolve to the `.git'
    // directory, never to `$HOME'. Accepting `$HOME' as an answer here
    // would hand `lsp-connect' the user's entire home directory as a
    // `rootUri' -- the same class of danger R1 bounded the walk to
    // prevent for a filelist ABOVE `$HOME'; a filelist AT `$HOME' is
    // excluded by the very same check
    // (`lsp--nearest-filelist-root''s own docstring covers the
    // mechanism; this test is the behavioral pin so a future reader
    // does not "fix" it back to honouring a home-level filelist).
    //
    // Layout: fake_home/verible.filelist (at home itself) and
    // fake_home/proj/.git (nearer, below home). Expected: `proj', via
    // the ordinary marker walk -- the filelist search finds nothing
    // usable (home itself is excluded from ever being accepted) and
    // falls through.
    let mut i = setup();
    let fake_root = scratch_dir("home_filelist_not_honoured");
    let fake_home = fake_root.join("home");
    let proj = fake_home.join("proj");
    std::fs::create_dir_all(proj.join(".git")).unwrap();
    std::fs::create_dir_all(&fake_home).unwrap();
    std::fs::write(
        fake_home.join("verible.filelist"),
        "proj/buf.sv
",
    )
    .unwrap();
    let file = proj.join("buf.sv");
    std::fs::write(
        &file,
        "module buf; endmodule
",
    )
    .unwrap();

    let _home_guard = HomeGuard::set(&fake_home);

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", proj.to_str().unwrap()));
}

#[test]
fn project_root_non_verilog_buffer_still_prefers_the_nearer_marker() {
    // Same layout as the test above, but the buffer is a `.rs' file --
    // this milestone must not change resolution for any non-Verilog
    // language. `.git' at `sub' wins, exactly as
    // `project_root_prefers_the_nearer_marker_over_an_outer_one' pins
    // for the general case.
    let mut i = setup();
    let root = scratch_dir("non_sv_prefers_nearer");
    std::fs::create_dir_all(root.join("sub/.git")).unwrap();
    std::fs::write(root.join("verible.filelist"), "sub/main.rs\n").unwrap();
    let file = root.join("sub/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(
        run(&mut i, &src),
        format!("{:?}", root.join("sub").to_str().unwrap())
    );
}

#[test]
fn project_root_verilog_buffer_with_no_filelist_anywhere_behaves_as_today() {
    // No `verible.filelist' at all above the buffer -- the new filelist
    // search finds nothing, and this must fall back to the ordinary
    // nearest-marker walk exactly as before M93.
    let mut i = setup();
    // M93 third fix round (W3): this test resolves a `.sv`/`.vh`/
    // `.v`/`.svh` file, so `lsp--project-root` reads `$HOME` via
    // `lsp--home-directory` even though this test never overrides it
    // -- see `home_env_lock`'s own doc for why that still needs the
    // shared lock.
    let _lock = home_env_lock();
    let root = scratch_dir("sv_no_filelist_unaffected");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    let file = root.join("sub/buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

// M93 fix round (R3): `project_root_verilog_filelist_at_nearest_marker_
// directory_is_unaffected' was deleted here. Its layout put the
// `verible.filelist' in the SAME directory as the nearest generic
// marker, but `verible.filelist' is itself one of the eight entries in
// `lsp--project-root-markers' -- so the OLD, unmodified nearest-marker
// walk already stopped at that exact directory too, for the same
// reason (it's the nearest ancestor holding ANY marker, filelist
// included). Reverting this entire milestone leaves that test green,
// so it was not discriminating between old and new behavior; no
// layout with "filelist co-located with the nearest marker" can ever
// discriminate them, since that's precisely the case where both
// algorithms trivially agree. Deleted per the reviewer's instruction
// rather than kept as something that only looks like coverage.

#[test]
fn project_root_nested_filelists_resolve_to_the_nearest_one() {
    // M93 fix round (R3): the original version of this test put
    // `verible.filelist' at both `proj' and `proj/sub', with no other
    // marker anywhere -- but `verible.filelist' is itself a member of
    // `lsp--project-root-markers', so the OLD nearest-marker walk
    // already stopped at `proj/sub' too (its own filelist counts as
    // "any marker"), making that layout non-discriminating between old
    // and new behavior (same failure family as the deleted test
    // above). This version adds `proj/mid/sub/.git' as the nearest
    // GENERIC marker, nearer than either filelist: the OLD algorithm
    // stops there and returns `sub', ignoring both filelists entirely.
    // The NEW, Verilog-specific filelist walk ignores `.git' and finds
    // `proj/mid/verible.filelist' (nearer than `proj/verible.filelist')
    // -- so the correct M93 answer is `mid', not `sub' (what the old
    // algorithm gives) and not `proj' (the outermost filelist).
    let mut i = setup();
    // M93 third fix round (W3): this test resolves a `.sv`/`.vh`/
    // `.v`/`.svh` file, so `lsp--project-root` reads `$HOME` via
    // `lsp--home-directory` even though this test never overrides it
    // -- see `home_env_lock`'s own doc for why that still needs the
    // shared lock.
    let _lock = home_env_lock();
    let root = scratch_dir("sv_nested_filelists");
    std::fs::create_dir_all(root.join("mid/sub/.git")).unwrap();
    std::fs::write(root.join("verible.filelist"), "mid/sub/buf.sv\n").unwrap();
    std::fs::write(root.join("mid/verible.filelist"), "sub/buf.sv\n").unwrap();
    let file = root.join("mid/sub/buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let src = format!("(lsp--project-root {:?})", file.to_str().unwrap());
    assert_eq!(
        run(&mut i, &src),
        format!("{:?}", root.join("mid").to_str().unwrap())
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
// M132: per-server rootUri -- `lsp--server-root-style',
// `lsp--project-root-for-command', `lsp--filelist-entries',
// `lsp--filelist-component-root', and the duplicate-declaration
// detector (`lsp--workspace-duplicate-declarations'/`-warning').
// ============================================================

#[test]
fn project_root_for_command_unlisted_command_uses_project_root() {
    // A command with no entry in `lsp-server-root-style-alist' (the
    // default `verible-verilog-ls', or any other name entirely) must
    // fall back to plain `lsp--project-root' -- the exact same answer,
    // for a non-Verilog file so no filelist logic can be in play at
    // all.
    let mut i = setup();
    let root = scratch_dir("unlisted_command");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let plain = run(
        &mut i,
        &format!("(lsp--project-root {:?})", file.to_str().unwrap()),
    );
    let for_command = run(
        &mut i,
        &format!(
            "(lsp--project-root-for-command \"clangd\" {:?})",
            file.to_str().unwrap()
        ),
    );
    assert_eq!(for_command, plain);
}

#[test]
fn project_root_for_command_workspace_style_widens_to_the_filelist_component() {
    // proj/.git, proj/rtl/verible.filelist and proj/verif/verible.filelist
    // share `core/alu.sv' -- one connected component, smallest covering
    // directory is `proj' itself.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("workspace_widens");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("rtl/core")).unwrap();
    std::fs::create_dir_all(root.join("rtl/top")).unwrap();
    std::fs::create_dir_all(root.join("verif")).unwrap();
    std::fs::write(
        root.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        root.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = root.join("rtl/core/alu.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        root.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(root.join("verif/tb.sv"), "module tb; endmodule\n").unwrap();

    let src = format!(
        "(lsp--project-root-for-command \"slang-server\" {:?})",
        alu.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn project_root_for_command_filelist_style_stays_at_the_nearest_filelist() {
    // Same file, same fixture as the widening test above -- the two
    // styles must disagree: `filelist' stays at `proj/rtl' (the
    // nearest `verible.filelist' ancestor, exactly what plain
    // `lsp--project-root' already returns), `workspace' widens all the
    // way to `proj'. This is the per-server claim itself.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("filelist_style_stays_narrow");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("rtl/core")).unwrap();
    std::fs::create_dir_all(root.join("rtl/top")).unwrap();
    std::fs::create_dir_all(root.join("verif")).unwrap();
    std::fs::write(
        root.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        root.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = root.join("rtl/core/alu.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        root.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(root.join("verif/tb.sv"), "module tb; endmodule\n").unwrap();

    let filelist_style = run(
        &mut i,
        &format!(
            "(lsp--project-root-for-command \"verible-verilog-ls\" {:?})",
            alu.to_str().unwrap()
        ),
    );
    let workspace_style = run(
        &mut i,
        &format!(
            "(lsp--project-root-for-command \"slang-server\" {:?})",
            alu.to_str().unwrap()
        ),
    );
    assert_eq!(
        filelist_style,
        format!("{:?}", root.join("rtl").to_str().unwrap())
    );
    assert_eq!(workspace_style, format!("{:?}", root.to_str().unwrap()));
    assert_ne!(
        filelist_style, workspace_style,
        "the two root styles must disagree for the same file"
    );
}

#[test]
fn filelist_component_root_disjoint_lists_do_not_merge() {
    // chipA and chipB each have their own `verible.filelist' with no
    // entry in common, one `.git' above both -- each buffer must
    // resolve to its OWN chip directory, never to the shared `.git'
    // directory.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("disjoint_no_merge");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("chipA")).unwrap();
    std::fs::create_dir_all(root.join("chipB")).unwrap();
    std::fs::write(root.join("chipA/verible.filelist"), "alu.sv\ntop_a.sv\n").unwrap();
    std::fs::write(root.join("chipB/verible.filelist"), "alu.sv\ntop_b.sv\n").unwrap();
    std::fs::write(
        root.join("chipA/alu.sv"),
        "module alu(a,b,sum); endmodule\n",
    )
    .unwrap();
    std::fs::write(root.join("chipA/top_a.sv"), "module top_a; endmodule\n").unwrap();
    std::fs::write(
        root.join("chipB/alu.sv"),
        "module alu(a,b,diff); endmodule\n",
    )
    .unwrap();
    std::fs::write(root.join("chipB/top_b.sv"), "module top_b; endmodule\n").unwrap();

    let a_file = root.join("chipA/alu.sv");
    let b_file = root.join("chipB/alu.sv");
    let a_root = run(
        &mut i,
        &format!(
            "(lsp--filelist-component-root {:?})",
            a_file.to_str().unwrap()
        ),
    );
    let b_root = run(
        &mut i,
        &format!(
            "(lsp--filelist-component-root {:?})",
            b_file.to_str().unwrap()
        ),
    );
    assert_eq!(
        a_root,
        format!("{:?}", root.join("chipA").to_str().unwrap())
    );
    assert_eq!(
        b_root,
        format!("{:?}", root.join("chipB").to_str().unwrap())
    );
}

#[test]
fn filelist_component_root_is_nil_without_a_filelist_ancestor() {
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("no_filelist_ancestor");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let file = root.join("buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let src = format!(
        "(lsp--filelist-component-root {:?})",
        file.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "nil");
}

#[test]
fn filelist_component_root_is_capped_at_git() {
    // `verible.filelist' sits ABOVE the nearest `.git' -- no widening
    // (and no honouring of the filelist at all): nil.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("capped_at_git");
    std::fs::create_dir_all(root.join("sub/.git")).unwrap();
    std::fs::write(root.join("verible.filelist"), "sub/buf.sv\n").unwrap();
    let file = root.join("sub/buf.sv");
    std::fs::write(&file, "module buf; endmodule\n").unwrap();

    let src = format!(
        "(lsp--filelist-component-root {:?})",
        file.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "nil");
}

#[test]
fn filelist_component_root_clamps_an_entry_that_escapes_the_git_root() {
    // `rg --files -g verible.filelist CAP' only guarantees the filelist
    // FILE ITSELF is under `.git' -- an ENTRY inside that filelist can
    // still name a path outside the repo via `../..'. Without the clamp
    // at step 10 of `lsp--filelist-component-root', the covering
    // directory over that entry's directory would land ABOVE `.git',
    // and this function would hand a workspace root outside the repo to
    // the LSP server. `filelist_component_root_is_capped_at_git' above
    // does NOT exercise this: there the filelist itself sits above
    // `.git', which is rejected by an earlier, different guard (step 5)
    // before the finishing function's own clamp is ever reached.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("clamp_escaping_entry");
    std::fs::create_dir_all(root.join("outside")).unwrap();
    std::fs::create_dir_all(root.join("repo/.git")).unwrap();
    std::fs::create_dir_all(root.join("repo/rtl")).unwrap();
    std::fs::write(root.join("outside/far.sv"), "module far; endmodule\n").unwrap();
    std::fs::write(root.join("repo/rtl/alu.sv"), "module alu; endmodule\n").unwrap();
    std::fs::write(
        root.join("repo/rtl/verible.filelist"),
        "alu.sv\n../../outside/far.sv\n",
    )
    .unwrap();

    let file = root.join("repo/rtl/alu.sv");
    let src = format!(
        "(lsp--filelist-component-root {:?})",
        file.to_str().unwrap()
    );
    assert_eq!(
        run(&mut i, &src),
        format!("{:?}", root.join("repo").to_str().unwrap()),
        "an entry escaping `.git' via `../..' must be clamped back to the \
         repo root, never widen the workspace root outside the repo"
    );
}

#[test]
fn filelist_component_root_merges_transitively_across_three_lists() {
    // A shares with B, B shares with C, A and C share nothing directly
    // -- all three must still merge into one component.
    let mut i = setup();
    let _lock = home_env_lock();
    let root = scratch_dir("transitive_merge");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::create_dir_all(root.join("c")).unwrap();
    std::fs::write(root.join("a/verible.filelist"), "a.sv\nshared_ab.sv\n").unwrap();
    std::fs::write(
        root.join("b/verible.filelist"),
        "../a/shared_ab.sv\nshared_bc.sv\n",
    )
    .unwrap();
    std::fs::write(root.join("c/verible.filelist"), "../b/shared_bc.sv\nc.sv\n").unwrap();
    std::fs::write(root.join("a/a.sv"), "module a; endmodule\n").unwrap();
    std::fs::write(root.join("a/shared_ab.sv"), "module shared_ab; endmodule\n").unwrap();
    std::fs::write(root.join("b/shared_bc.sv"), "module shared_bc; endmodule\n").unwrap();
    std::fs::write(root.join("c/c.sv"), "module c; endmodule\n").unwrap();

    let file = root.join("a/a.sv");
    let src = format!(
        "(lsp--filelist-component-root {:?})",
        file.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), format!("{:?}", root.to_str().unwrap()));
}

#[test]
fn filelist_entries_skips_comments_blanks_and_plusargs() {
    let mut i = setup();
    let root = scratch_dir("entries_skip");
    std::fs::create_dir_all(&root).unwrap();
    let list = root.join("x.f");
    std::fs::write(
        &list,
        "# a comment\n// another comment\n+incdir+foo\n-f other.f\n\n   \nfoo.sv\n  bar.sv  \n",
    )
    .unwrap();

    let src = format!("(lsp--filelist-entries {:?})", list.to_str().unwrap());
    assert_eq!(
        run(&mut i, &src),
        format!(
            "({:?} {:?})",
            root.join("foo.sv").to_str().unwrap(),
            root.join("bar.sv").to_str().unwrap()
        )
    );
}

#[test]
fn filelist_entries_resolve_relative_to_the_filelist_directory() {
    let mut i = setup();
    let root = scratch_dir("entries_relative");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(root.join("rtl")).unwrap();
    let list = root.join("sub/list.f");
    std::fs::write(&list, "../rtl/x.sv\n").unwrap();

    let src = format!("(lsp--filelist-entries {:?})", list.to_str().unwrap());
    assert_eq!(
        run(&mut i, &src),
        format!("({:?})", root.join("rtl/x.sv").to_str().unwrap())
    );
}

#[test]
fn project_root_for_command_workspace_style_leaves_a_non_verilog_file_alone() {
    // A `workspace'-style command on a non-Verilog file: `lsp--filelist-
    // component-root' is structurally nil (guard 1), so this must fall
    // back to, and exactly equal, plain `lsp--project-root'.
    let mut i = setup();
    let root = scratch_dir("workspace_style_non_verilog");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let file = root.join("main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();

    let plain = run(
        &mut i,
        &format!("(lsp--project-root {:?})", file.to_str().unwrap()),
    );
    let for_command = run(
        &mut i,
        &format!(
            "(lsp--project-root-for-command \"slang-server\" {:?})",
            file.to_str().unwrap()
        ),
    );
    assert_eq!(for_command, plain);
}

#[test]
fn filelist_component_root_of_demo_verif_is_the_demo_directory() {
    // Real repo material (CLAUDE.md: use what's here, don't fabricate).
    // `demo/rtl/verible.filelist' (11 paths) and `demo/verif/verible.
    // filelist' (8 paths, 5 shared with `demo/rtl') share files ->
    // one component -> smallest covering directory is `demo' itself.
    let mut i = setup();
    let _lock = home_env_lock();
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/core has two ancestors up to the repo root");
    let demo = repo_root.join("demo");
    let file = demo.join("verif/sram_bank_tb.sv");
    assert!(file.is_file(), "expected {:?} to exist", file);

    let src = format!(
        "(lsp--filelist-component-root {:?})",
        file.to_str().unwrap()
    );
    assert_eq!(
        run(&mut i, &src),
        format!("{:?}", demo.to_str().unwrap()),
        "expected the widened root to land on demo/, not demo/verif/ or the repo root"
    );
}

#[test]
fn workspace_duplicate_declarations_finds_a_name_declared_in_two_files() {
    let mut i = setup();
    let root = scratch_dir("dup_two_files");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "module foo(a,b,sum); endmodule\n").unwrap();
    std::fs::write(root.join("b.sv"), "module foo(a,b,diff); endmodule\n").unwrap();

    let src = format!(
        "(lsp--workspace-duplicate-declarations {:?})",
        root.to_str().unwrap()
    );
    let result = run(&mut i, &src);
    assert!(
        result.contains("\"foo\""),
        "expected the duplicated name in the result: {}",
        result
    );
    assert!(result.contains("a.sv"), "expected a.sv named: {}", result);
    assert!(result.contains("b.sv"), "expected b.sv named: {}", result);
}

#[test]
fn workspace_duplicate_declarations_is_empty_when_every_name_is_unique() {
    let mut i = setup();
    let root = scratch_dir("dup_none");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "module foo; endmodule\n").unwrap();
    std::fs::write(root.join("b.sv"), "module bar; endmodule\n").unwrap();

    let src = format!(
        "(lsp--workspace-duplicate-declarations {:?})",
        root.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "nil");
}

#[test]
fn workspace_duplicate_declarations_covers_package_interface_and_program() {
    let mut i = setup();
    let root = scratch_dir("dup_all_kinds");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("m1.sv"), "module dup_a; endmodule\n").unwrap();
    std::fs::write(root.join("m2.sv"), "module dup_a; endmodule\n").unwrap();
    std::fs::write(root.join("p1.sv"), "package dup_b; endpackage\n").unwrap();
    std::fs::write(root.join("p2.sv"), "package dup_b; endpackage\n").unwrap();
    std::fs::write(root.join("i1.sv"), "interface dup_c; endinterface\n").unwrap();
    std::fs::write(root.join("i2.sv"), "interface dup_c; endinterface\n").unwrap();
    std::fs::write(root.join("g1.sv"), "program dup_d; endprogram\n").unwrap();
    std::fs::write(root.join("g2.sv"), "program dup_d; endprogram\n").unwrap();

    let src = format!(
        "(mapcar #'car (lsp--workspace-duplicate-declarations {:?}))",
        root.to_str().unwrap()
    );
    assert_eq!(
        run(&mut i, &src),
        "(\"dup_a\" \"dup_b\" \"dup_c\" \"dup_d\")"
    );
}

#[test]
fn workspace_duplicate_declarations_reads_the_name_of_an_interface_class() {
    // `interface class Foo;' -- the first identifier after `interface'
    // is the keyword `class', not the declared name. Before the fix,
    // this reported the name "class" for both directions: two
    // different `interface class'es falsely flagged as a duplicate of
    // a thing called "class", and a real collision reported under a
    // useless name.
    let mut i = setup();
    let root = scratch_dir("dup_interface_class");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("a.sv"),
        "interface class comparable_if;\nendclass\n",
    )
    .unwrap();
    std::fs::write(
        root.join("b.sv"),
        "interface class comparable_if;\nendclass\n",
    )
    .unwrap();

    let src = format!(
        "(mapcar #'car (lsp--workspace-duplicate-declarations {:?}))",
        root.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "(\"comparable_if\")");
}

#[test]
fn workspace_duplicate_declarations_covers_macromodule() {
    // `macromodule alu (a,b);' is a legacy synonym for `module' and was
    // not in the keyword alternation at all -- two files both
    // declaring the same `macromodule' name were silently reported
    // clean.
    let mut i = setup();
    let root = scratch_dir("dup_macromodule");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "macromodule alu (a,b,sum);\nendmodule\n").unwrap();
    std::fs::write(
        root.join("b.sv"),
        "macromodule alu (a,b,diff);\nendmodule\n",
    )
    .unwrap();

    let src = format!(
        "(mapcar #'car (lsp--workspace-duplicate-declarations {:?}))",
        root.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "(\"alu\")");
}

#[test]
fn workspace_duplicate_warning_names_the_symbol_and_both_files() {
    let mut i = setup();
    let root = scratch_dir("dup_warning_names");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "module foo(a,b,sum); endmodule\n").unwrap();
    std::fs::write(root.join("b.sv"), "module foo(a,b,diff); endmodule\n").unwrap();

    let src = format!(
        "(lsp--workspace-duplicate-warning {:?})",
        root.to_str().unwrap()
    );
    let result = run(&mut i, &src);
    assert!(
        result.contains("foo"),
        "expected the symbol name: {}",
        result
    );
    assert!(result.contains("a.sv"), "expected a.sv named: {}", result);
    assert!(result.contains("b.sv"), "expected b.sv named: {}", result);
    assert!(
        result.to_lowercase().contains("slang"),
        "expected the consequence (slang binds silently) spelled out: {}",
        result
    );
}

#[test]
fn workspace_duplicate_warning_is_nil_for_a_clean_root() {
    let mut i = setup();
    let root = scratch_dir("dup_warning_clean");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "module foo; endmodule\n").unwrap();
    std::fs::write(root.join("b.sv"), "module bar; endmodule\n").unwrap();

    let src = format!(
        "(lsp--workspace-duplicate-warning {:?})",
        root.to_str().unwrap()
    );
    assert_eq!(run(&mut i, &src), "nil");
}

#[test]
fn workspace_duplicate_warning_is_emitted_once_per_root() {
    // Deletion test for F2's wire: calling `lsp--workspace-maybe-warn-
    // duplicates' twice for the SAME (command . root) must record ROOT
    // as warned exactly once -- deleting the "already warned" guard
    // would make the second call push a second entry.
    let mut i = setup();
    let root = scratch_dir("dup_warning_once");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.sv"), "module foo(a,b,sum); endmodule\n").unwrap();
    std::fs::write(root.join("b.sv"), "module foo(a,b,diff); endmodule\n").unwrap();

    let call = format!(
        "(lsp--workspace-maybe-warn-duplicates \"slang-server\" {:?})",
        root.to_str().unwrap()
    );
    ok(&mut i, &call);
    assert_eq!(
        run(&mut i, "(length lsp--workspace-duplicate-warned-roots)"),
        "1"
    );
    ok(&mut i, &call);
    assert_eq!(
        run(&mut i, "(length lsp--workspace-duplicate-warned-roots)"),
        "1",
        "a second call for the same root must not record it a second time"
    );
}

#[test]
fn lsp_connect_warns_about_duplicate_declarations_under_a_widened_root() {
    // FIX 3 (M132 fix round, review #4): every other test in this file
    // calls `lsp--workspace-maybe-warn-duplicates' directly, bypassing
    // BOTH real call sites (`lsp-connect' and `lsp--autostart-begin').
    // Deleting `(when root-path (lsp--workspace-maybe-warn-duplicates
    // command root-path))' from `lsp-connect' (lsp.el:1215) must make
    // THIS test fail -- it goes through `(lsp)' -> `lsp-connect' for
    // real, with a fixture whose widened root actually contains a
    // duplicate module declaration.
    //
    // Fixture: same shape as `auto_attach_uses_the_per_command_root_for_
    // a_workspace_style_server' (rtl/verible.filelist and
    // verif/verible.filelist share `core/alu.sv', so a `workspace'-style
    // command widens all the way to the top `dir', capped at `.git') --
    // plus one duplicate module name (`dup_widened') declared in a file
    // under `rtl/top' and another under `verif`, which the widened root
    // covers but neither single filelist does on its own.
    let mut i = setup();
    let _lock = home_env_lock();
    let dir = scratch_dir("m132_fix3_lsp_connect_warns");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/core")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/top")).unwrap();
    std::fs::create_dir_all(dir.join("verif")).unwrap();
    std::fs::write(
        dir.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = dir.join("rtl/core/alu.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        dir.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(dir.join("verif/tb.sv"), "module tb; endmodule\n").unwrap();
    // The actual duplicate the widened root (but neither individual
    // filelist) covers.
    std::fs::write(
        dir.join("rtl/top/extra_a.sv"),
        "module dup_widened; endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/extra_b.sv"),
        "module dup_widened; endmodule\n",
    )
    .unwrap();

    // A real, executable "slang-server" (basename match for
    // `lsp-server-root-style-alist''s `workspace' entry) that just
    // `exec's `cat' -- the same fake-server shape this file already
    // uses successfully for every other `(lsp)' test, only renamed so
    // `lsp--server-root-style' resolves it as `workspace'-style.
    let script = dir.join("slang-server");
    std::fs::write(&script, "#!/bin/sh\nexec cat\n").unwrap();
    let mut perm = std::fs::metadata(&script).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&script, perm).unwrap();

    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list {:?})))",
            script.to_str().unwrap()
        ),
    );
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", alu.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    ok(&mut i, "(lsp)");

    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("dup_widened"),
        "expected the duplicate-declaration warning to have been \
         messaged via the real lsp-connect call site: {}",
        messages
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
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

#[test]
fn lsp_connect_records_its_own_root_on_the_client() {
    // M131 Part A: `lsp-connect' is the real call site that builds a
    // live `lsp--client' with a non-nil ROOT-PATH -- this is the
    // struct-field-level guard standing on `(make-lsp--client :conn
    // conn :command command :root root-path)' actually wiring
    // ROOT-PATH through, independent of any `lsp-references-at-point'
    // wiring (covered separately, at the message-text level, in
    // `lsp_references_tests.rs').
    let mut i = setup();
    let dir = scratch_dir("lsp_connect_records_root");
    std::fs::create_dir_all(&dir).unwrap();
    ok(
        &mut i,
        &format!(
            "(setq test--client (lsp-connect \"cat\" nil {:?}))",
            dir.to_str().unwrap()
        ),
    );
    assert_eq!(
        run(&mut i, "(lsp--client-root test--client)"),
        format!("{:?}", dir.to_str().unwrap())
    );
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

/// M145: decides whether a test needing PROGRAM should run. Split out
/// from its thin per-tool wrapper below so the decision itself --
/// present/absent x opt-out-env-value -- is testable without touching
/// real process environment or PATH (see `require_tool_tests` below).
/// Returns true if the caller should proceed; false if the caller
/// should `return` early because a deliberate, visible opt-out was set
/// (ENV_VALUE is exactly "1"/"true"/"yes" -- anything else, including
/// "0", means "no, don't skip", so a leftover boolean-style "false" or
/// an accidental "0" cannot silently disable the check). Panics -- does
/// not return -- when PROGRAM is absent and no opt-out was given. Only
/// used by the default-run `manual_e2e_verible_verilog_ls_*` test below
/// -- the `#[ignore]`d tests above this point keep their own silent
/// `have_on_path` skip since cargo already reports them as ignored, not
/// as passed.
fn require_tool(program: &str, present: bool, env_name: &str, env_value: Option<&str>) -> bool {
    if present {
        return true;
    }
    let opted_out = matches!(env_value, Some("1") | Some("true") | Some("yes"));
    if opted_out {
        // Opt-out convention for this project: RETICLE_ALLOW_MISSING_*
        // / RETICLE_SKIP_* env vars (see `test_source_hygiene_tests.rs`).
        eprintln!(
            "skipping (opted out via {}): {} is not on PATH",
            env_name, program
        );
        return false;
    }
    panic!(
        "{} is not on PATH -- this e2e test was not run. Failing by default so a \
         missing dependency cannot silently pass as a green gate. Install {}, or set \
         {}=1 to deliberately skip on a machine that genuinely lacks it.",
        program, program, env_name
    );
}

/// Same env var `lsp_format_tests.rs`/`verilog_complete_tests.rs` use
/// for the same tool -- one variable per tool, not per call site.
const VERIBLE_LS_SKIP_ENV: &str = "RETICLE_ALLOW_MISSING_VERIBLE_LS";

fn require_verible_verilog_ls() -> bool {
    let env_value = std::env::var(VERIBLE_LS_SKIP_ENV).ok();
    require_tool(
        "verible-verilog-ls",
        have_on_path("verible-verilog-ls"),
        VERIBLE_LS_SKIP_ENV,
        env_value.as_deref(),
    )
}

#[cfg(test)]
mod require_tool_tests {
    use super::require_tool;

    /// M145 deletion question: if `require_tool` silently returned
    /// `false` on an absent tool with no opt-out set (instead of
    /// panicking), this is the test that would have to go red to catch
    /// it -- so it must actually observe the panic, not just call the
    /// function.
    #[test]
    fn absent_and_no_opt_out_panics_naming_the_env_var() {
        let result = std::panic::catch_unwind(|| {
            require_tool("fake-tool", false, "RETICLE_ALLOW_MISSING_FAKE", None)
        });
        let payload = result.expect_err("expected require_tool to panic when absent, no opt-out");
        let msg = payload
            .downcast_ref::<String>()
            .map(|s| s.as_str())
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("");
        assert!(
            msg.contains("RETICLE_ALLOW_MISSING_FAKE"),
            "panic message should name the env var: {}",
            msg
        );
    }

    #[test]
    fn absent_and_opted_out_with_1_returns_false() {
        assert!(!require_tool(
            "fake-tool",
            false,
            "RETICLE_ALLOW_MISSING_FAKE",
            Some("1")
        ));
    }

    /// `FOO=0` must NOT mean "yes, skip" -- an environment left over
    /// from some other boolean convention must not silently disable
    /// this check.
    #[test]
    fn absent_and_env_set_to_0_still_panics() {
        let result = std::panic::catch_unwind(|| {
            require_tool("fake-tool", false, "RETICLE_ALLOW_MISSING_FAKE", Some("0"))
        });
        assert!(result.is_err(), "FOO=0 must not opt out of the check");
    }

    #[test]
    fn present_returns_true_regardless_of_env() {
        assert!(require_tool(
            "fake-tool",
            true,
            "RETICLE_ALLOW_MISSING_FAKE",
            None
        ));
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
// editor targets) exercise it every run. **M145: everyone else now
// FAILS this test by default instead of silently skipping** -- see
// `require_tool`'s own doc comment above; set
// `RETICLE_ALLOW_MISSING_VERIBLE_LS=1` to opt out deliberately.
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
    if !require_verible_verilog_ls() {
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

// ============================================================
// M94: a second attached client per buffer, routed by capability.
// Attach/sync/save/close use two real "cat" subprocesses (same
// technique the rest of this file already uses) so the actual
// `textDocument/*' framing on each connection can be inspected;
// capability ROUTING itself uses lightweight `:conn nil' stub clients
// (same convention `verilog_complete_tests.rs`'s
// `stub_client_with_capabilities' already established), since routing
// only reads `lsp--client-capabilities'/`lsp--client-command', not a
// real transport.
// ============================================================

/// `H' with KEY present (an empty nested hash-table value -- the exact
/// value never matters to `lsp--capability-supported-p', only presence
/// does) -- an elisp expression string for `make-lsp--client'
/// `:capabilities'.
fn caps_with(key: &str) -> String {
    format!(
        "(let ((h (make-hash-table))) (puthash {:?} (make-hash-table) h) h)",
        key
    )
}

#[test]
fn attaching_a_second_client_adds_it_without_disturbing_the_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_attach_two");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    // Z2 (M94 review): the primary slot is only for a client whose own
    // `command' matches the PRIMARY table for the buffer's mode -- so
    // "cat" must actually be registered as rust-mode's primary here for
    // test--a to be eligible to occupy it.
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );

    ok(&mut i, "(setq test--a (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--a 'rust-mode)");
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--a)"), "t");
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "1");

    ok(&mut i, "(setq test--b (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--b 'rust-mode)");

    // Primary is unchanged by the second attach.
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--a)"), "t");
    // Both are attached.
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");
    assert_eq!(
        run(&mut i, "(and (memq test--a lsp--buffer-clients) t)"),
        "t"
    );
    assert_eq!(
        run(&mut i, "(and (memq test--b lsp--buffer-clients) t)"),
        "t"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--a))");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--b))");
}

#[test]
fn a_secondary_attach_that_fails_did_open_does_not_clobber_the_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_secondary_fails");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );

    ok(&mut i, "(setq test--a (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--a 'rust-mode)");
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--a)"), "t");

    ok(
        &mut i,
        "(setq test--orig-did-open (symbol-function 'lsp-did-open))",
    );
    ok(
        &mut i,
        "(fset 'lsp-did-open (lambda (&rest _) (error \"boom\")))",
    );
    ok(
        &mut i,
        "(setq test--b (make-lsp--client :conn nil :command \"other\"))",
    );
    let r = run(&mut i, "(lsp--attach-current-buffer test--b 'rust-mode)");
    assert!(r.starts_with("ERROR"), "expected a signal: {}", r);
    ok(&mut i, "(fset 'lsp-did-open test--orig-did-open)");

    // The primary must be untouched, and B must never have joined
    // `lsp--buffer-clients'.
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--a)"), "t");
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "1");
    assert_eq!(
        run(&mut i, "(and (memq test--b lsp--buffer-clients) t)"),
        "nil"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--a))");
}

#[test]
fn idle_pump_syncs_every_attached_client_with_independent_watermarks() {
    let mut i = setup();
    let dir = scratch_dir("m94_multi_sync");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    ok(&mut i, "(setq test--a (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--a 'rust-mode)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(&mut i, "(lsp--client-conn test--a)", "textDocument/didOpen");

    // Edit BEFORE B ever attaches.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"fn b() {}\\n\")");

    // B attaches AFTER the edit -- its own didOpen already carries the
    // post-edit text, so no didChange is owed to it for this edit.
    ok(&mut i, "(setq test--b (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--b 'rust-mode)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(&mut i, "(lsp--client-conn test--b)", "textDocument/didOpen");

    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));

    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn test--a)",
            "textDocument/didChange"
        ),
        1,
        "A must get the didChange for the edit it missed while B hadn't attached yet"
    );
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn test--b)",
            "textDocument/didChange"
        ),
        0,
        "B attached AFTER the edit -- its own didOpen already covered it, so it must \
         not be treated as needing a didChange for text it never actually missed"
    );

    // A second edit, made after both are attached: both must now get
    // exactly one didChange each.
    ok(&mut i, "(insert \"fn c() {}\\n\")");
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn test--a)",
            "textDocument/didChange"
        ),
        1
    );
    assert_eq!(
        drain_frames(
            &mut i,
            "(lsp--client-conn test--b)",
            "textDocument/didChange"
        ),
        1
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--a))");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--b))");
}

#[test]
fn save_and_kill_reach_every_attached_client() {
    let mut i = setup();
    let dir = scratch_dir("m94_save_close");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    ok(&mut i, "(setq test--a (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--a 'rust-mode)");
    ok(&mut i, "(setq test--b (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--b 'rust-mode)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    drain_frames(&mut i, "(lsp--client-conn test--a)", "textDocument/didOpen");
    drain_frames(&mut i, "(lsp--client-conn test--b)", "textDocument/didOpen");

    ok(&mut i, "(insert \"more\")");
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer signaled: {}", r);
    std::thread::sleep(std::time::Duration::from_millis(300));

    drain_methods(&mut i, "(lsp--client-conn test--a)");
    assert_eq!(
        run(&mut i, "test--methods"),
        "(\"textDocument/didChange\" \"textDocument/didSave\")",
        "A must get both the sync didChange and the didSave"
    );
    drain_methods(&mut i, "(lsp--client-conn test--b)");
    assert_eq!(
        run(&mut i, "test--methods"),
        "(\"textDocument/didChange\" \"textDocument/didSave\")",
        "B must get both the sync didChange and the didSave too"
    );

    ok(&mut i, "(kill-buffer)");
    std::thread::sleep(std::time::Duration::from_millis(300));
    // test--a/test--b are plain globals (not buffer-local), so they
    // still reach both connections after the buffer that held them
    // buffer-locally is gone.
    drain_methods(&mut i, "(lsp--client-conn test--a)");
    assert_eq!(run(&mut i, "test--methods"), "(\"textDocument/didClose\")");
    drain_methods(&mut i, "(lsp--client-conn test--b)");
    assert_eq!(run(&mut i, "test--methods"), "(\"textDocument/didClose\")");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--a))");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--b))");
}

#[test]
fn completion_routes_to_a_capable_secondary_even_though_it_is_not_the_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_completion_route");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    // PRIMARY (first in the list -- proves this is genuinely a
    // capability filter, not just "whichever client is first") does
    // NOT advertise completion; the SECONDARY does.
    ok(
        &mut i,
        "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
           :capabilities (make-hash-table)))",
    );
    ok(
        &mut i,
        &format!(
            "(setq test--secondary (make-lsp--client :conn nil :command \"slang\" \
               :capabilities {}))",
            caps_with("completionProvider")
        ),
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );

    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--capable-client \"completionProvider\") test--secondary)"
        ),
        "t"
    );

    ok(&mut i, "(setq test--captured-client nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async (lambda (client method params callback) \
           (setq test--captured-client client) 1))",
    );
    ok(&mut i, "(completion-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "the LSP tier must have sent the request to the SECONDARY, not the primary"
    );

    // With no attached client advertising completion at all, behavior
    // is exactly today's: the LSP tier is never reached.
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary))",
    );
    ok(&mut i, "(setq test--captured-client nil)");
    ok(&mut i, "(completion-at-point)");
    assert_eq!(
        run(&mut i, "test--captured-client"),
        "nil",
        "no capable client -- lsp-completion-at-point must never have been invoked"
    );
}

#[test]
fn hover_routes_to_a_capable_secondary_even_though_it_is_not_the_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_hover_route");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    // PRIMARY (first in the list) has capabilities recorded but NO
    // "hoverProvider" key at all; the SECONDARY does.
    ok(
        &mut i,
        "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
           :capabilities (make-hash-table)))",
    );
    ok(
        &mut i,
        &format!(
            "(setq test--secondary (make-lsp--client :conn nil :command \"slang\" \
               :capabilities {}))",
            caps_with("hoverProvider")
        ),
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );

    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--capable-client \"hoverProvider\") test--secondary)"
        ),
        "t"
    );

    ok(&mut i, "(setq test--captured-client nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async (lambda (client method params callback) \
           (setq test--captured-client client) 1))",
    );
    ok(&mut i, "(lsp-hover-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "hover must have gone to the SECONDARY, not the primary"
    );
}

#[test]
fn formatting_still_goes_to_the_primary_when_a_secondary_is_also_attached() {
    let mut i = setup();
    let dir = scratch_dir("m94_format_primary");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a(){}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    // Both clients advertise formatting, on purpose: this proves
    // routing is UNCHANGED (still "always the primary"), not merely
    // "the primary happens to be the only one that could answer".
    ok(
        &mut i,
        &format!(
            "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
               :capabilities {}))",
            caps_with("documentFormattingProvider")
        ),
    );
    ok(
        &mut i,
        &format!(
            "(setq test--secondary (make-lsp--client :conn nil :command \"slang\" \
               :capabilities {}))",
            caps_with("documentFormattingProvider")
        ),
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--secondary test--primary))",
    );
    ok(&mut i, "(fset 'lsp--sync-buffer-now (lambda () nil))");

    ok(&mut i, "(setq test--captured-client nil)");
    ok(
        &mut i,
        "(fset 'lsp-request-async (lambda (client method params callback) \
           (setq test--captured-client client) 1))",
    );
    ok(&mut i, "(lsp-format-buffer)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--primary)"),
        "t",
        "lsp-format-buffer must still route to lsp--buffer-client, the primary"
    );
}

/// Small helper installed once via `ok', not a Rust `format!' string
/// with nested elisp string literals -- building a `publishDiagnostics'
/// notification and dispatching it in one call keeps the Rust side of
/// the next test free of doubled braces/escapes.
fn install_test_publish_helper(i: &mut Interp) {
    ok(
        i,
        r#"(defun test--publish (client uri msg line)
             (let ((h (make-hash-table)) (p (make-hash-table)))
               (puthash "uri" uri p)
               (puthash "diagnostics"
                        (json-parse-string
                         (format "[{\"range\":{\"start\":{\"line\":%d,\"character\":0},\"end\":{\"line\":%d,\"character\":1}},\"message\":\"%s\"}]"
                                 line line msg))
                        p)
               (puthash "method" "textDocument/publishDiagnostics" h)
               (puthash "params" p h)
               (lsp--dispatch client h)))"#,
    );
}

/// M99: pre-M99 this test pinned `lsp--decorate-buffer''s DEFAULT
/// behavior (only the primary decorates). M99 flipped the default to
/// merge every attached client's diagnostics
/// (`lsp-merge-diagnostics-from-all-clients', `crates/core/lisp/lsp.el')
/// -- the default-behavior claim this test's old name made is no longer
/// true, so it's renamed to say what it actually pins down now: the
/// M94 exclusive-decoration path, which M99 kept as an explicit opt-out
/// (`(setq lsp-merge-diagnostics-from-all-clients nil)`), not the
/// default. The DEFAULT (merged) behavior is covered by
/// `lsp_highlight_tests.rs`'s M99 tests
/// (`merge_diagnostics_default_paints_the_union_of_two_attached_clients'
/// and friends), not here.
#[test]
fn with_merge_diagnostics_nil_only_the_primarys_diagnostics_decorate_but_a_secondarys_are_still_stored(
) {
    let mut i = setup();
    let dir = scratch_dir("m94_diag_decorate");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "line0\nline1\nline2\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);

    ok(&mut i, "(setq test--primary (make-lsp--client :conn nil))");
    ok(
        &mut i,
        "(setq test--secondary (make-lsp--client :conn nil))",
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );
    // `lsp-merge-diagnostics-from-all-clients' is a plain (non-buffer-
    // local) `defvar' -- `setq' here is unconditional global state, not
    // scoped to this buffer, but it's placed after the buffer/client
    // setup above to read top-to-bottom as "opt out of the M99 default,
    // then exercise the M94 exclusive path" alongside the rest of this
    // test's setup.
    ok(&mut i, "(setq lsp-merge-diagnostics-from-all-clients nil)");

    ok(
        &mut i,
        "(test--publish test--primary (lsp--path-to-uri (buffer-file-name)) \"from primary\" 0)",
    );
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1",
        "the primary's own publish must decorate the buffer"
    );

    ok(
        &mut i,
        "(test--publish test--secondary (lsp--path-to-uri (buffer-file-name)) \"from secondary\" 1)",
    );
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1",
        "a non-primary publish must not repaint the buffer's decoration at all"
    );
    assert_ne!(
        run(
            &mut i,
            "(assoc (lsp--path-to-uri (buffer-file-name)) (lsp--client-diagnostics test--secondary))"
        ),
        "nil",
        "the secondary's own publish must still be STORED on its own client"
    );
}

// ============================================================
// M94 review fix round (Z2/Z4/Z5).
// ============================================================

#[test]
fn primary_slot_self_heals_after_dying_while_a_secondary_stays_attached() {
    // Z2: verible attaches (primary); verible dies; a slang autostart
    // completes and must NOT walk into the now-empty primary slot; a
    // fresh verible reconnect afterwards must reclaim it.
    let mut i = setup();
    let dir = scratch_dir("m94_z2_self_heal");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );

    // Primary attaches.
    ok(&mut i, "(setq test--primary1 (lsp-connect \"cat\"))");
    ok(
        &mut i,
        "(lsp--attach-current-buffer test--primary1 'rust-mode)",
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--primary1)"), "t");

    // Primary dies.
    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary1))");

    // A secondary (its command does NOT match rust-mode's primary
    // table entry) attaches while the slot is stale-but-non-nil.
    ok(
        &mut i,
        "(setq test--secondary (lsp-connect \"sh\" (list \"-c\" \"cat\")))",
    );
    ok(
        &mut i,
        "(lsp--attach-current-buffer test--secondary 'rust-mode)",
    );
    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the dead primary must have been cleared, and the secondary must NOT \
         have taken the now-empty primary slot"
    );
    assert_eq!(
        run(&mut i, "(and (memq test--secondary lsp--buffer-clients) t)"),
        "t",
        "the secondary must still be attached, just not as primary"
    );

    // A fresh primary reconnects and must reclaim the slot.
    ok(&mut i, "(setq test--primary2 (lsp-connect \"cat\"))");
    ok(
        &mut i,
        "(lsp--attach-current-buffer test--primary2 'rust-mode)",
    );
    assert_eq!(
        run(&mut i, "(eq lsp--buffer-client test--primary2)"),
        "t",
        "the real primary must have reclaimed the slot"
    );
    assert_eq!(
        run(
            &mut i,
            "(equal (lsp--client-command lsp--buffer-client) \"cat\")"
        ),
        "t"
    );
    assert_eq!(
        run(&mut i, "(and (memq test--secondary lsp--buffer-clients) t)"),
        "t",
        "the secondary must still be attached throughout"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary2))");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--secondary))");
}

/// M99: same rename rationale as `with_merge_diagnostics_nil_only_the_
/// primarys_diagnostics_decorate_but_a_secondarys_are_still_stored'
/// just above -- this test's exact-overlay-count assertions ("1", not
/// "2") only hold under the M94 exclusive-decoration path, which M99
/// demoted from the default to an explicit opt-out
/// (`lsp-merge-diagnostics-from-all-clients' set to nil). Under the new
/// DEFAULT (merged) behavior the secondary's publish would ADD an
/// overlay on top of the (still-stored, even though the primary is
/// dead) primary diagnostic, making the "1" assertions below false --
/// that is exactly the M99 behavior change, not a regression in it, so
/// this test now pins the nil-opt-out path by name instead of silently
/// assuming it.
#[test]
fn with_merge_diagnostics_nil_a_dead_primary_reopens_decoration_to_a_live_secondary() {
    // Z4: `lsp--diagnostics-authoritative-p' must not stay permanently
    // closed once the ONE-TIME authoritative primary has died -- a
    // live secondary's own publish must decorate again.
    let mut i = setup();
    let dir = scratch_dir("m94_z4_dead_primary_decorate");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "line0\nline1\nline2\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    install_test_publish_helper(&mut i);

    ok(&mut i, "(setq test--primary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        "(setq test--secondary (make-lsp--client :conn nil))",
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );
    // Global (non-buffer-local) `defvar' -- see the sibling test's own
    // comment on this same line for why placement here (after buffer/
    // client setup, before any publish) is only about readability, not
    // buffer-local scoping.
    ok(&mut i, "(setq lsp-merge-diagnostics-from-all-clients nil)");

    ok(
        &mut i,
        "(test--publish test--primary (lsp--path-to-uri (buffer-file-name)) \"from primary\" 0)",
    );
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1"
    );
    let primary_pos: i64 = run(
        &mut i,
        "(overlay-start (car (overlays-in (point-min) (point-max))))",
    )
    .parse()
    .expect("overlay start should print as an integer");

    // The primary dies.
    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary))");

    // The secondary's own publish must now decorate -- authority is
    // "unestablished" (the primary is dead), which is default-OPEN,
    // not default-closed.
    ok(
        &mut i,
        "(test--publish test--secondary (lsp--path-to-uri (buffer-file-name)) \"from secondary\" 2)",
    );
    assert_eq!(
        run(&mut i, "(length (overlays-in (point-min) (point-max)))"),
        "1",
        "the secondary's publish must have repainted the buffer"
    );
    let secondary_pos: i64 = run(
        &mut i,
        "(overlay-start (car (overlays-in (point-min) (point-max))))",
    )
    .parse()
    .expect("overlay start should print as an integer");
    assert_ne!(
        primary_pos, secondary_pos,
        "the overlay must have actually moved to the secondary's own diagnostic \
         location, proving a real repaint happened rather than the old \
         primary overlay simply being left alone"
    );
}

#[test]
fn hover_prefers_a_capable_primary_over_a_capable_secondary_in_real_attach_order() {
    // Z5: `lsp--buffer-clients' is most-recently-attached-first, so a
    // secondary (attached after the primary, the ordinary sequence)
    // sits AHEAD of the primary in that list. `lsp--capable-client'
    // must still prefer the primary when it is ALSO capable, rather
    // than mechanically returning the list's first match. Unlike the
    // earlier routing tests, this one drives the list through the real
    // `lsp--attach-current-buffer' sequence instead of hand-building it.
    let mut i = setup();
    let dir = scratch_dir("m94_z5_real_order");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'rust-mode (list \"cat\")))",
    );

    ok(&mut i, "(setq test--primary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        &format!(
            "(setf (lsp--client-capabilities test--primary) {})",
            caps_with("hoverProvider")
        ),
    );
    ok(
        &mut i,
        "(lsp--attach-current-buffer test--primary 'rust-mode)",
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--primary)"), "t");

    // A second "cat" connection attaches AFTER the primary -- same
    // command, but arrives too late to take the (already-live) primary
    // slot, so it becomes a plain member of `lsp--buffer-clients',
    // consed onto the front (most-recent-first).
    ok(&mut i, "(setq test--secondary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        &format!(
            "(setf (lsp--client-capabilities test--secondary) {})",
            caps_with("hoverProvider")
        ),
    );
    ok(
        &mut i,
        "(lsp--attach-current-buffer test--secondary 'rust-mode)",
    );
    assert_eq!(
        run(&mut i, "(eq (car lsp--buffer-clients) test--secondary)"),
        "t",
        "sanity: the secondary really is first in list order"
    );

    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--capable-client \"hoverProvider\") test--primary)"
        ),
        "t",
        "the PRIMARY must win even though it is not first in lsp--buffer-clients"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary))");
    ok(&mut i, "(lsp-kill (lsp--client-conn test--secondary))");
}

#[test]
fn idle_pump_prunes_a_dead_client_out_of_buffer_clients_and_its_watermark() {
    // Z6/AA4: `lsp--sync-buffer-now' opportunistically drops a now-dead
    // client out of `lsp--buffer-clients' (and its own
    // `lsp--last-synced-tick' entry) while it's already walking every
    // attached client for liveness -- no correctness bug without this
    // (every consumer re-checks liveness itself), but nothing observed
    // the list actually shrinking until this test.
    let mut i = setup();
    let dir = scratch_dir("m94_z6_prune");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    ok(&mut i, "(setq test--a (lsp-connect \"cat\"))");
    ok(&mut i, "(lsp--attach-current-buffer test--a 'rust-mode)");
    ok(
        &mut i,
        "(setq test--b (lsp-connect \"sh\" (list \"-c\" \"cat\")))",
    );
    ok(&mut i, "(lsp--attach-current-buffer test--b 'rust-mode)");
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");
    assert_ne!(
        run(&mut i, "(lsp--client-synced-tick test--b)"),
        "nil",
        "sanity: B has a watermark entry before it dies"
    );

    // B dies out from under the buffer.
    ok(&mut i, "(lsp-kill (lsp--client-conn test--b))");

    // Drive a sync (the idle pump), which is where the pruning happens.
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "idle pump signaled: {}", r);

    assert_eq!(
        run(&mut i, "(length lsp--buffer-clients)"),
        "1",
        "the dead client must have been pruned out of the list"
    );
    assert_eq!(
        run(&mut i, "(and (memq test--b lsp--buffer-clients) t)"),
        "nil"
    );
    assert_eq!(
        run(&mut i, "(and (memq test--a lsp--buffer-clients) t)"),
        "t",
        "the still-live client must be untouched"
    );
    assert_eq!(
        run(&mut i, "(lsp--client-synced-tick test--b)"),
        "nil",
        "the dead client's own watermark entry must be gone too"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--a))");
}

// ============================================================
// M95: five more methods (definition, references, documentSymbol,
// rename, documentHighlight) route via `lsp--preferred-role-client',
// which prefers a capable SECONDARY over the primary -- the opposite
// tie-break from `lsp--capable-client''s own default (still exactly
// right for completion/hover, and everything else not listed in
// `lsp-request-preferred-role-alist'). Same stub-client technique as
// the M94 completion/hover routing tests above: `:conn nil' clients,
// since routing only reads `lsp--client-capabilities'/
// `lsp--client-command', never a real transport.
// ============================================================

/// (METHOD . CAPABILITY-KEY) for each of the five M95 methods, and the
/// elisp expression that triggers it -- shared by every test below so
/// the five don't have to be typed out five times each.
fn m95_methods() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "textDocument/definition",
            "definitionProvider",
            "(lsp-definition-at-point)",
        ),
        (
            "textDocument/references",
            "referencesProvider",
            "(lsp-references-at-point)",
        ),
        (
            "textDocument/documentSymbol",
            "documentSymbolProvider",
            "(lsp-next-symbol)",
        ),
        (
            "textDocument/documentHighlight",
            "documentHighlightProvider",
            "(lsp-highlight-at-point)",
        ),
    ]
}

/// Common buffer + primary/secondary setup for the M95 routing tests:
/// a real file visited, PRIMARY (command "verible") first in
/// `lsp--buffer-clients', SECONDARY (command "slang") second -- same
/// shape as `completion_routes_to_a_capable_secondary...' above.
/// PRIMARY_CAPS/SECONDARY_CAPS are elisp expressions for each client's
/// `:capabilities' (typically built with `caps_with').
fn m95_setup(i: &mut Interp, tag: &str, primary_caps: &str, secondary_caps: &str) {
    let dir = scratch_dir(tag);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        i,
        &format!(
            "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
               :capabilities {}))",
            primary_caps
        ),
    );
    ok(
        i,
        &format!(
            "(setq test--secondary (make-lsp--client :conn nil :command \"slang\" \
               :capabilities {}))",
            secondary_caps
        ),
    );
    ok(i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );
}

fn m95_stub_request_capture(i: &mut Interp) {
    ok(i, "(setq test--captured-client nil)");
    ok(i, "(setq test--captured-method nil)");
    ok(
        i,
        "(fset 'lsp-request-async (lambda (client method params callback) \
           (setq test--captured-client client) \
           (setq test--captured-method method) 1))",
    );
}

#[test]
fn four_of_the_five_route_to_a_capable_secondary_over_the_primary() {
    // Test 1: primary + capable secondary attached -> secondary wins,
    // for every one of the five methods.
    for (method, key, trigger) in m95_methods() {
        let mut i = setup();
        m95_setup(
            &mut i,
            &format!("m95_secondary_wins_{}", key),
            "(make-hash-table)",
            &caps_with(key),
        );
        m95_stub_request_capture(&mut i);
        // rename reads a new name via `read-string' before sending.
        if method == "textDocument/rename" {
            ok(
                &mut i,
                "(fset 'read-string (lambda (prompt callback &optional initial) \
                   (funcall callback \"bar\")))",
            );
        }
        ok(&mut i, trigger);
        assert_eq!(
            run(&mut i, "(eq test--captured-client test--secondary)"),
            "t",
            "{method}: must have routed to the capable secondary, not the primary"
        );
        assert_eq!(
            run(&mut i, "test--captured-method"),
            format!("{:?}", method),
            "{method}: sanity check that this call site really sends the method \
             it is paired with in `m95_methods'"
        );
    }
}

#[test]
fn rename_routes_to_a_capable_secondary_over_the_primary() {
    // `lsp-rename' isn't in `m95_methods' (it needs `read-string'
    // stubbed before the request even goes out) -- covered on its own
    // here instead, same shape as the loop above.
    let mut i = setup();
    m95_setup(
        &mut i,
        "m95_secondary_wins_rename",
        "(make-hash-table)",
        &caps_with("renameProvider"),
    );
    m95_stub_request_capture(&mut i);
    ok(
        &mut i,
        "(fset 'read-string (lambda (prompt callback &optional initial) \
           (funcall callback \"bar\")))",
    );
    ok(&mut i, "(lsp-rename)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t"
    );
    assert_eq!(
        run(&mut i, "test--captured-method"),
        "\"textDocument/rename\""
    );
}

#[test]
fn four_of_the_five_still_go_to_a_single_primary_exactly_as_before() {
    // Test 2: the single-server case every OTHER language is in (a
    // Rust buffer only ever has rust-analyzer, and it is the primary)
    // -- must be indistinguishable from before this milestone. No
    // secondary attached at all.
    for (method, _key, trigger) in m95_methods() {
        let mut i = setup();
        let dir = scratch_dir(&format!("m95_single_primary_{}", method.replace('/', "_")));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
        // No `:capabilities' at all -- the M46 asymmetric-trust default
        // (unknown capabilities => trusted supported), same as a real
        // client before its `initialize' reply has been recorded. An
        // EMPTY hash-table, by contrast, is capabilities KNOWN with the
        // key ABSENT -- UNSUPPORTED (see `lsp--capability-supported-p'
        // and the `four_of_the_five_route_to_a_capable_secondary...'
        // test above, which relies on exactly that to make its primary
        // incapable on purpose).
        ok(
            &mut i,
            "(setq test--primary (make-lsp--client :conn nil :command \"verible\"))",
        );
        ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
        m95_stub_request_capture(&mut i);
        if method == "textDocument/rename" {
            ok(
                &mut i,
                "(fset 'read-string (lambda (prompt callback &optional initial) \
                   (funcall callback \"bar\")))",
            );
        }
        ok(&mut i, trigger);
        assert_eq!(
            run(&mut i, "(eq test--captured-client test--primary)"),
            "t",
            "{method}: a single attached client (no secondary at all) must still \
             be routed to, exactly as before this milestone"
        );
    }
}

#[test]
fn rename_still_goes_to_a_single_primary_exactly_as_before() {
    let mut i = setup();
    let dir = scratch_dir("m95_single_primary_rename");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    // No `:capabilities' -- trusted supported, see the analogous note in
    // `four_of_the_five_still_go_to_a_single_primary_exactly_as_before'.
    ok(
        &mut i,
        "(setq test--primary (make-lsp--client :conn nil :command \"verible\"))",
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    m95_stub_request_capture(&mut i);
    ok(
        &mut i,
        "(fset 'read-string (lambda (prompt callback &optional initial) \
           (funcall callback \"bar\")))",
    );
    ok(&mut i, "(lsp-rename)");
    assert_eq!(run(&mut i, "(eq test--captured-client test--primary)"), "t");
}

#[test]
fn a_secondary_not_declaring_one_method_falls_back_to_primary_for_that_method_only() {
    // Test 3: the secondary declares "referencesProvider" but NOT
    // "definitionProvider" -- definition must fall back to the
    // primary, while references still routes to the secondary, in the
    // SAME buffer with the SAME two clients attached.
    let mut i = setup();
    // Primary's own `:capabilities' is `nil' (unknown -> trusted
    // supported, see the note in
    // `four_of_the_five_still_go_to_a_single_primary_exactly_as_before')
    // so this test genuinely exercises "the secondary lacks the key,
    // fall back to a CAPABLE primary" rather than "neither is capable".
    m95_setup(
        &mut i,
        "m95_partial_capability",
        "nil",
        &caps_with("referencesProvider"),
    );
    m95_stub_request_capture(&mut i);

    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--primary)"),
        "t",
        "definition: secondary doesn't declare definitionProvider, must fall back \
         to the primary"
    );

    m95_stub_request_capture(&mut i);
    ok(&mut i, "(lsp-references-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "references: secondary DOES declare referencesProvider, must still win"
    );
}

#[test]
fn a_dead_secondary_falls_back_to_the_primary() {
    // Test 4: both clients are real (`lsp-connect'-produced) so
    // `lsp--client-conn-live-p' can actually observe the secondary as
    // dead, rather than trusting a stub `:conn nil' client the way the
    // other tests here do.
    let mut i = setup();
    let dir = scratch_dir("m95_dead_secondary");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    ok(&mut i, "(setq test--primary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        &format!(
            "(setf (lsp--client-capabilities test--primary) {})",
            caps_with("definitionProvider")
        ),
    );
    ok(&mut i, "(setq test--secondary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        &format!(
            "(setf (lsp--client-capabilities test--secondary) {})",
            caps_with("definitionProvider")
        ),
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );

    // Sanity: while both are alive, the secondary wins.
    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--preferred-role-client \"textDocument/definition\" \
             \"definitionProvider\") test--secondary)"
        ),
        "t",
        "sanity: with both alive, the secondary must win"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--secondary))");

    assert_eq!(
        run(
            &mut i,
            "(eq (lsp--preferred-role-client \"textDocument/definition\" \
             \"definitionProvider\") test--primary)"
        ),
        "t",
        "the now-dead secondary must not be picked -- falls back to the primary"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary))");
}

#[test]
fn formatting_still_goes_to_the_primary_even_though_it_is_not_in_the_preference_table() {
    // Test 5: the guard that M95 did not quietly widen the routing
    // table beyond the five named methods -- formatting keeps reading
    // `lsp--buffer-client' directly (via `lsp--live-buffer-client'),
    // unaffected by `lsp-request-preferred-role-alist' having no entry
    // for it, exactly as `formatting_still_goes_to_the_primary_when_a_
    // secondary_is_also_attached' (M94, above) already covers for
    // `lsp--capable-client'. This test instead asserts directly against
    // the new M95 table and selector, so a future edit that
    // accidentally adds "documentFormattingProvider" to the preference
    // alist is caught here rather than only by the older M94 test.
    assert_eq!(
        {
            let mut i = setup();
            run(
                &mut i,
                "(assoc \"textDocument/formatting\" lsp-request-preferred-role-alist)",
            )
        },
        "nil",
        "formatting must have no entry in the M95 preference table at all"
    );

    let mut i = setup();
    m95_setup(
        &mut i,
        "m95_formatting_guard",
        &caps_with("documentFormattingProvider"),
        &caps_with("documentFormattingProvider"),
    );
    m95_stub_request_capture(&mut i);
    ok(&mut i, "(lsp-format-buffer)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--primary)"),
        "t",
        "formatting must still go to the primary even with a capable secondary \
         attached"
    );
}

#[test]
fn single_client_missing_one_capability_key_still_routes_there_ungated() {
    // Reviewer BB1: before M95, all five of these call sites read
    // `lsp--live-buffer-client' directly, UNGATED -- a single attached
    // client answered every one of these five requests regardless of
    // what its own `initialize' reply declared. The fallback branch of
    // `lsp--preferred-role-client' must reproduce that exactly, so a
    // real capabilities hash that happens to omit KEY must not turn
    // into "No LSP server connected in this buffer" -- a server IS
    // connected, it just didn't declare that one key. Only the
    // SECONDARY's preference has to be earned via
    // `lsp--capability-supported-p'; the primary fallback must not be.
    for (method, key, trigger) in m95_methods() {
        let mut i = setup();
        let dir = scratch_dir(&format!("m95_bb1_ungated_{}", method.replace('/', "_")));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
        // A REAL capabilities hash-table that simply does not mention
        // KEY -- capabilities-known-but-key-absent, `lsp--capability-
        // supported-p''s UNSUPPORTED case, deliberately (not `nil',
        // which every other single-client test in this file uses, and
        // which lands on the trusted-supported branch instead -- that
        // is exactly why none of them could see this regression).
        ok(
            &mut i,
            "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
               :capabilities (let ((h (make-hash-table))) \
                                (puthash \"someOtherProvider\" t h) h)))",
        );
        ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
        m95_stub_request_capture(&mut i);
        if method == "textDocument/rename" {
            ok(
                &mut i,
                "(fset 'read-string (lambda (prompt callback &optional initial) \
                   (funcall callback \"bar\")))",
            );
        }
        ok(&mut i, trigger);
        assert_eq!(
            run(&mut i, "(eq test--captured-client test--primary)"),
            "t",
            "{method} ({key}): a single attached client whose capabilities hash \
             simply omits this key must still be routed to -- the pre-M95 \
             call sites never gated on capability at all"
        );
    }
}

#[test]
fn rename_single_client_missing_capability_key_still_routes_there_ungated() {
    // Same as `single_client_missing_one_capability_key_still_routes_
    // there_ungated' above, for `lsp-rename' (not in `m95_methods' --
    // needs `read-string' stubbed).
    let mut i = setup();
    let dir = scratch_dir("m95_bb1_ungated_rename");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        &mut i,
        "(setq test--primary (make-lsp--client :conn nil :command \"verible\" \
           :capabilities (let ((h (make-hash-table))) \
                            (puthash \"someOtherProvider\" t h) h)))",
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    m95_stub_request_capture(&mut i);
    ok(
        &mut i,
        "(fset 'read-string (lambda (prompt callback &optional initial) \
           (funcall callback \"bar\")))",
    );
    ok(&mut i, "(lsp-rename)");
    assert_eq!(run(&mut i, "(eq test--captured-client test--primary)"), "t");
}

#[test]
fn the_non_primary_exclusion_is_what_picks_the_secondary_when_both_are_capable() {
    // Reviewer BB3: with the primary ALSO capable of `key', the
    // `(not (eq client primary))' exclusion inside `lsp--preferred-
    // role-client''s candidate scan is the ONLY thing standing between
    // "the secondary wins, per its M95 preference" and "the dolist
    // walks onto the primary first and stops there instead" -- every
    // earlier routing test made the primary incapable, so this
    // exclusion could be deleted without failing any of them.
    //
    // `m95_setup' puts the primary FIRST in `lsp--buffer-clients' --
    // NOT the ordinary attach order (`lsp--attach-current-buffer'
    // conses onto the front, most-recently-attached first, so in the
    // real Verilog flow the secondary, attached after the primary,
    // ends up first and the primary second). That is deliberate here,
    // not an oversight: in the REAL order, list position alone would
    // already pick the secondary, and the exclusion's own effect would
    // never be exercised -- only with the primary ahead of the
    // secondary in the scan does reaching the secondary anyway prove
    // the exclusion (rather than mere list order) is what did it.
    let mut i = setup();
    m95_setup(
        &mut i,
        "m95_bb3_exclusion",
        &caps_with("definitionProvider"),
        &caps_with("definitionProvider"),
    );
    m95_stub_request_capture(&mut i);
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "both primary and secondary declare definitionProvider -- the secondary \
         must still win, per its M95 preference, not the primary the dolist \
         would reach first without the non-primary exclusion"
    );
}

#[test]
fn goto_symbol_by_name_and_previous_symbol_also_route_to_a_capable_secondary() {
    // Reviewer BB2: `lsp-goto-symbol-by-name' is a genuinely SEPARATE
    // call site from the M95 routing tests above (which only ever
    // drive documentSymbol through `lsp-next-symbol') -- it is the one
    // piece of new coverage here. `lsp-previous-symbol' is included
    // too, for its own sake, but it is NOT a second separate call
    // site: it shares `lsp--goto-symbol' with `lsp-next-symbol', which
    // the main loop test above already exercises, so this only re-runs
    // that same shared body under a different entry point rather than
    // covering new code.
    for trigger in ["(lsp-previous-symbol)", "(lsp-goto-symbol-by-name)"] {
        let mut i = setup();
        m95_setup(
            &mut i,
            &format!(
                "m95_bb2_docsym_{}",
                trigger.trim_matches(|c| c == '(' || c == ')')
            ),
            "(make-hash-table)",
            &caps_with("documentSymbolProvider"),
        );
        m95_stub_request_capture(&mut i);
        ok(&mut i, trigger);
        assert_eq!(
            run(&mut i, "(eq test--captured-client test--secondary)"),
            "t",
            "{trigger} must route documentSymbol to the capable secondary too"
        );
        assert_eq!(
            run(&mut i, "test--captured-method"),
            "\"textDocument/documentSymbol\""
        );
    }
}

#[test]
fn highlight_clear_and_idle_tick_treat_a_live_capable_secondary_as_connected() {
    // Reviewer BB2: `lsp-highlight-clear' and `lsp--idle-highlight-tick'
    // don't send a request themselves, so they're untested by the
    // request-capture technique the other M95 tests use -- but M95
    // routed their own liveness checks through `lsp--preferred-role-
    // client' too (see their docstrings' M95 notes), so a DEAD primary
    // with a LIVE, capable secondary attached must still count as
    // "connected" for both. Both real `lsp-connect'-produced clients
    // (same technique as `a_dead_secondary_falls_back_to_the_primary'),
    // so the primary's death is genuinely observable.
    let mut i = setup();
    let dir = scratch_dir("m95_bb2_highlight_dead_primary");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));

    ok(&mut i, "(setq test--primary (lsp-connect \"cat\"))");
    ok(&mut i, "(setq test--secondary (lsp-connect \"cat\"))");
    ok(
        &mut i,
        &format!(
            "(setf (lsp--client-capabilities test--secondary) {})",
            caps_with("documentHighlightProvider")
        ),
    );
    ok(&mut i, "(setq-local lsp--buffer-client test--primary)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--primary test--secondary))",
    );
    ok(&mut i, "(lsp-kill (lsp--client-conn test--primary))");

    // `lsp-highlight-clear': records `lsp--idle-highlight-last-point'
    // only when "connected" -- must fire with the dead primary but a
    // live, capable secondary.
    ok(&mut i, "(goto-char 3)");
    ok(&mut i, "(setq-local lsp--idle-highlight-last-point nil)");
    ok(&mut i, "(lsp-highlight-clear)");
    assert_eq!(
        run(&mut i, "lsp--idle-highlight-last-point"),
        "3",
        "lsp-highlight-clear must treat a live, capable secondary as connected \
         even though the primary is dead"
    );

    // `lsp--idle-highlight-tick': must still fire a request (routed to
    // the live secondary) under the same dead-primary condition.
    ok(&mut i, "(setq-local lsp--idle-highlight-last-point nil)");
    m95_stub_request_capture(&mut i);
    ok(&mut i, "(lsp--idle-highlight-tick 300)"); // default delay is 300
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "lsp--idle-highlight-tick must have fired, routed to the live secondary, \
         even though the primary is dead"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--secondary))");
}

#[test]
fn a_lone_secondary_in_an_empty_primary_slot_answers_the_five_but_not_formatting() {
    // Reviewer CC1: the "a single attached client behaves identically
    // to before M95" claim repeated across the M95 docstrings only
    // holds when that one client occupies the PRIMARY slot. It does
    // NOT hold when the sole attached client is a SECONDARY sitting in
    // an EMPTY primary slot -- exactly the state M94's own Z2 self-heal
    // produces (see `primary_slot_self_heals_after_dying_while_a_
    // secondary_stays_attached' above): the primary died, and a client
    // whose `command' doesn't match the mode's primary table entry
    // joined `lsp--buffer-clients' without ever taking the now-empty
    // slot. There `lsp--buffer-client' is nil, so `(not (eq client
    // primary))' in `lsp--preferred-role-client''s candidate scan is
    // vacuously true for every candidate, and a live, capable lone
    // secondary answers -- where, before M95, these five methods read
    // `lsp--buffer-client' raw, got nil, and refused. This is an
    // IMPROVEMENT, not a regression, and it is kept rather than
    // special-cased away (see that function's own M95 docstring note)
    // -- but nothing before this test pinned it in either direction.
    //
    // `lsp-format-buffer' still refuses in this exact state: formatting
    // is untouched by M95 and still reads `lsp--buffer-client' directly
    // -- this is the other half of the claim, that M95 did not quietly
    // extend the improvement to the methods it left alone.
    let mut i = setup();
    let dir = scratch_dir("m95_cc1_lone_secondary_empty_primary");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(
        &mut i,
        &format!(
            "(setq test--secondary (make-lsp--client :conn nil :command \"slang\" \
               :capabilities {}))",
            caps_with("definitionProvider")
        ),
    );
    // No `test--primary' at all -- the primary slot is genuinely empty
    // (`lsp--buffer-client' nil), not merely occupied by an incapable
    // client, matching the Z2 self-heal state exactly.
    ok(&mut i, "(setq-local lsp--buffer-client nil)");
    ok(
        &mut i,
        "(setq-local lsp--buffer-clients (list test--secondary))",
    );

    m95_stub_request_capture(&mut i);
    ok(&mut i, "(lsp-definition-at-point)");
    assert_eq!(
        run(&mut i, "(eq test--captured-client test--secondary)"),
        "t",
        "a lone secondary in an empty primary slot must answer definition -- an \
         improvement over the pre-M95 refusal, not a regression"
    );

    // Formatting is unaffected -- still reads `lsp--buffer-client'
    // directly, still nil, still refuses exactly as before M95.
    assert_eq!(
        run(&mut i, "(lsp-format-buffer)"),
        "\"No LSP server connected in this buffer (M-x lsp first)\"",
        "formatting must still refuse in this exact state -- M95 did not widen \
         its own improvement onto a method it left alone"
    );
}
