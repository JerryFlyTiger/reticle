//! M61: `find-file-internal` normalizes the path it stores in `.file`
//! (relative -> absolute, `.`/`..` components collapsed) so that two
//! different spellings of the same file share one buffer instead of
//! silently diverging into two — the second of which would shadow saves
//! made through the first. See `crates/core/src/builtins/editing.rs`'s
//! `expand_path` and `crates/core/src/builtins/files.rs`'s
//! `expand_file_name`.
//!
//! Known gap (documented, not a regression target here): this is purely
//! string-level normalization, not `fs::canonicalize` — symlinks are not
//! resolved, so two different symlinked names for the same underlying
//! file still open two buffers. GNU Emacs has the same default gap
//! (`find-file-visit-truename` is nil there too).
//!
//! cwd trap: `std::env::set_current_dir` is process-global, and tests in
//! this binary run in parallel. Only T7 (`relative_path_from_shell_opens_one_buffer`)
//! touches cwd, guarded by a `Drop` guard that restores it even on panic.
//! Do not add a second test that touches cwd here.

use std::cell::RefCell;
use std::rc::Rc;

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
            "reticle_file_path_{}_{}_{}",
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

/// Builds `<tmp>/rtl/top/soc_top.sv` and returns (clean_path, dotted_path)
/// where `dotted_path` is `<tmp>/rtl/core/../top/soc_top.sv` — a second
/// spelling of the same absolute file, with no cwd involved.
fn two_spellings(dir: &std::path::Path) -> (String, String) {
    let top = dir.join("rtl").join("top");
    let core_dir = dir.join("rtl").join("core");
    std::fs::create_dir_all(&top).unwrap();
    std::fs::create_dir_all(&core_dir).unwrap();
    let clean = top.join("soc_top.sv");
    std::fs::write(&clean, "module soc_top; endmodule\n").unwrap();
    let clean = clean.to_str().unwrap().to_string();
    let dotted = format!("{}/../top/soc_top.sv", core_dir.to_str().unwrap());
    (clean, dotted)
}

#[test]
fn two_spellings_of_one_file_share_one_buffer() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("two_spellings");
    let (clean, dotted) = two_spellings(&dir);

    ok(&mut i, &format!("(find-file-internal {:?})", clean));
    let n_after_first = ed.borrow().buffers.len();
    ok(&mut i, &format!("(find-file-internal {:?})", dotted));
    let n_after_second = ed.borrow().buffers.len();

    assert_eq!(
        n_after_first, n_after_second,
        "opening the dotted spelling created a new buffer"
    );
    assert_eq!(run(&mut i, "(get-buffer \"soc_top.sv<2>\")"), "nil");
}

#[test]
fn unsaved_edits_survive_the_second_spelling() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("unsaved_edits");
    let (clean, dotted) = two_spellings(&dir);

    ok(&mut i, &format!("(find-file-internal {:?})", clean));
    let n_after_first = ed.borrow().buffers.len();
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"// unsaved marker\")");
    assert_eq!(run(&mut i, "(buffer-modified-p)"), "t");

    ok(&mut i, &format!("(find-file-internal {:?})", dotted));
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "t",
        "the second spelling re-read from disk, losing the unsaved edit"
    );
    let text = ok(&mut i, "(buffer-string)");
    assert!(
        text.contains("// unsaved marker"),
        "unsaved edit missing after opening the second spelling: {}",
        text
    );
    assert_eq!(
        ed.borrow().buffers.len(),
        n_after_first,
        "opening the second spelling created a new buffer instead of reusing the first"
    );
}

#[test]
fn buffer_file_name_is_absolute_and_dot_free() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("bfn_abs");
    let (clean, dotted) = two_spellings(&dir);

    ok(&mut i, &format!("(find-file-internal {:?})", dotted));
    let bfn = ok(&mut i, "(buffer-file-name)");
    // prin1 wraps strings in quotes; strip them for the raw comparison.
    let bfn_raw = bfn.trim_matches('"');
    assert!(bfn_raw.starts_with('/'), "not absolute: {}", bfn_raw);
    assert!(!bfn_raw.contains("/../"), "still has /../: {}", bfn_raw);
    assert!(!bfn_raw.contains("/./"), "still has /./: {}", bfn_raw);
    assert_eq!(bfn_raw, clean);
}

#[test]
fn default_directory_is_absolute() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("dd_abs");
    let (_clean, dotted) = two_spellings(&dir);

    ok(&mut i, &format!("(find-file-internal {:?})", dotted));
    let dd = ok(&mut i, "(default-directory)");
    let dd_raw = dd.trim_matches('"');
    assert!(dd_raw.starts_with('/'), "not absolute: {}", dd_raw);
    assert!(!dd_raw.contains("/../"), "still has /../: {}", dd_raw);
    assert!(!dd_raw.contains("/./"), "still has /./: {}", dd_raw);
    assert!(dd_raw.ends_with('/'), "doesn't end with /: {}", dd_raw);
}

#[test]
fn get_file_buffer_matches_across_spellings() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("gfb_spell");
    let (clean, dotted) = two_spellings(&dir);

    ok(&mut i, &format!("(find-file-internal {:?})", clean));
    let hit = ok(&mut i, &format!("(get-file-buffer {:?})", dotted));
    assert_ne!(
        hit, "nil",
        "get-file-buffer didn't match the dotted spelling"
    );
}

// M61 fixup: `get-file-buffer` normalizing its argument (D4) must skip
// `/ssh:` paths. Reason: `expand_file_name` delegates to
// `expand_file_input`, whose `//`-shadowing runs *before* its own
// `/ssh:` prefix check (a pre-existing, out-of-scope quirk of
// `expand_file_input` itself) — so a `/ssh:` argument containing a
// literal `//` would get silently rewritten into a *local* absolute
// path. If `get-file-buffer` normalized it anyway, a query the caller
// clearly meant as remote could wrongly match an unrelated local
// buffer. No network needed: this never dispatches to the remote
// transport at all, it only exercises `get-file-buffer`'s string
// comparison against a real local buffer's `.file`.
#[test]
fn get_file_buffer_does_not_let_a_mangled_ssh_query_match_a_local_buffer() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("gfb_ssh_no_collide");
    std::fs::create_dir_all(&dir).unwrap();
    let local = dir.join("local_target.txt");
    std::fs::write(&local, "local contents").unwrap();
    let local_str = local.to_str().unwrap().to_string();

    ok(&mut i, &format!("(find-file-internal {:?})", local_str));

    // Crafted so that if `/ssh:` args were (wrongly) run through local
    // normalization, `expand_file_input`'s `//`-shadowing would strip
    // everything up through the last `//` here and leave exactly
    // `local_str` — matching the local buffer above by accident.
    let query = format!("/ssh:fake@host:/{}", local_str);
    assert!(
        query.contains("//"),
        "test construction bug: no // in the crafted query"
    );

    let hit = ok(&mut i, &format!("(get-file-buffer {:?})", query));
    assert_eq!(
        hit, "nil",
        "a /ssh: query wrongly matched an unrelated local buffer"
    );
}

#[test]
fn rename_file_finds_visiting_buffer_across_spellings() {
    let (mut i, _ed) = setup();
    let dir = scratch_dir("rename_spell");
    let top = dir.join("rtl").join("top");
    let core_dir = dir.join("rtl").join("core");
    std::fs::create_dir_all(&top).unwrap();
    std::fs::create_dir_all(&core_dir).unwrap();
    let src_clean = top.join("soc_top.sv");
    std::fs::write(&src_clean, "module soc_top; endmodule\n").unwrap();
    let src_clean_str = src_clean.to_str().unwrap().to_string();

    ok(&mut i, &format!("(find-file-internal {:?})", src_clean_str));

    // Dotted spellings of the src (same file, `../` roundtrip) and the
    // dst (a rename target under a `../`-laden path).
    let src_dotted = format!("{}/../top/soc_top.sv", core_dir.to_str().unwrap());
    let dst_dotted = format!("{}/../top/soc_top_renamed.sv", core_dir.to_str().unwrap());
    let dst_clean = top.join("soc_top_renamed.sv");
    let dst_clean_str = dst_clean.to_str().unwrap().to_string();

    ok(
        &mut i,
        &format!("(rename-file {:?} {:?})", src_dotted, dst_dotted),
    );

    let bfn = ok(&mut i, "(buffer-file-name)");
    let bfn_raw = bfn.trim_matches('"');
    assert_eq!(bfn_raw, dst_clean_str);

    let bname = ok(&mut i, "(buffer-name)");
    assert_eq!(bname, "\"soc_top_renamed.sv\"");
}

/// Restores the process cwd on drop, even if the test panics partway
/// through — this is the only test in this file that touches cwd.
struct CwdGuard {
    original: std::path::PathBuf,
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

#[test]
fn relative_path_from_shell_opens_one_buffer() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("shell_relative");
    let top = dir.join("rtl").join("top");
    std::fs::create_dir_all(&top).unwrap();
    let clean = top.join("soc_top.sv");
    std::fs::write(&clean, "module soc_top; endmodule\n").unwrap();
    // On darwin, `std::env::temp_dir()` lives under `/var`, which is
    // itself a symlink to `/private/var`. `getcwd(3)` (what
    // `std::env::current_dir` calls after a `chdir`) resolves that
    // symlink, so the production normalizer (which joins relative paths
    // onto `std::env::current_dir()`) would produce a `/private/var/...`
    // path even though `dir` here is spelled `/var/...`. Canonicalize
    // once here so this test's own "absolute" spelling agrees with what
    // `getcwd` will report after the `chdir` below — this is test-only
    // bookkeeping, not part of the D2 string-only normalization contract.
    let dir = std::fs::canonicalize(&dir).unwrap();
    let clean = dir.join("rtl").join("top").join("soc_top.sv");
    let clean_str = clean.to_str().unwrap().to_string();

    let original = std::env::current_dir().unwrap();
    let _guard = CwdGuard {
        original: original.clone(),
    };
    std::env::set_current_dir(&dir).unwrap();

    ok(&mut i, "(find-file-internal \"rtl/top/soc_top.sv\")");
    let n_after_relative = ed.borrow().buffers.len();
    ok(&mut i, &format!("(find-file-internal {:?})", clean_str));
    let n_after_absolute = ed.borrow().buffers.len();

    assert_eq!(
        n_after_relative, n_after_absolute,
        "opening the same file's absolute path after its relative path created a second buffer"
    );

    drop(_guard);
}
