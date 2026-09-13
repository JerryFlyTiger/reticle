//! M58: guards `demo/`'s showcase claims against silent rot.
//!
//! `demo/` is real code shown to visitors as evidence this editor can
//! actually be used for RTL work, but until this file existed the
//! claims in `demo/README.md`'s "What was actually run" section were
//! only ever checked by hand, once, and had already drifted (the
//! README said "15 files"; the directory actually holds 22). Each test
//! below pins down one of those claims:
//!
//! - `every_file_under_demo_opens_in_its_expected_major_mode` -- "All
//!   NN files open in the correct major mode".
//! - `soc_top_m_dot_jumps_to_the_alu_module_definition` -- "`M-.` on
//!   `alu` lands in `rtl/core/alu.sv`".
//! - `m_comma_returns_from_alu_back_to_the_instantiation` -- `M-,`
//!   jumps back (not separately claimed in the README table, but the
//!   natural round trip of the row above).
//! - `regfile_instantiation_offers_all_nine_port_names` -- "port
//!   completion engages inside `u_regfile`'s port list".
//! - `arbiter_autoinst_expands_then_deletes` -- `C-c C-a` / `C-c C-k`
//!   on `u_arbiter`.
//!
//! Known NOT covered here, same as the README's own "Not verified"
//! section -- both need an external toolchain this test suite doesn't
//! assume is present:
//!
//! - `tools/TimingReport.java` is never compiled (no JDK dependency).
//! - `rtl-verilog2001/fifo_sync_tb.v` is never simulated (no Icarus
//!   Verilog dependency).
//! - `rtl-verilog2001/gray_ctr_tb.v` (M124) is never simulated either,
//!   for the same reason. `demo/tools/run_sim.sh` is the only thing that
//!   runs it, and it is the only thing that can see whether
//!   `gray_ctr.v` still encodes Gray at all -- M124's mutation V14
//!   (`dev/mutations/m124.py`) is declared a survivor precisely because
//!   no test in this suite simulates anything.
//!
//! Also pinned here as a known, deliberate non-round-trip: the M39
//! `verilog-delete-auto` does not restore the exact original
//! whitespace around `/*AUTOINST*/` (see test 5's own comment).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

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

/// Repo-root-relative `demo/` directory (mirrors `lsp_format_tests.rs`'s
/// own `demo_rtl_root` helper, one level up).
fn demo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo"))
}

/// Raw string value of `(buffer-file-name)`, read straight off the
/// interpreter `Value` (mirrors `verilog_auto_tests.rs`'s own `bs`
/// helper for `buffer-string`) rather than through `prin1_to_string`,
/// which would wrap/escape the path.
fn buffer_file_name(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-file-name)") {
        Ok(Value::Str(s)) => (*s).clone(),
        Ok(_) => panic!("(buffer-file-name) did not return a string"),
        Err(flow) => panic!(
            "(buffer-file-name) errored: {}",
            interp.describe_flow(&flow)
        ),
    }
}

/// Canonicalized path. M61 made `buffer-file-name` always absolute and
/// `.`/`..`-free (string-level normalization, not symlink resolution —
/// see `find-file-internal`'s `expand_path`), so it can no longer be
/// `../`-laden itself. What still needs canonicalizing is this test's
/// own `demo_root()`, built via `concat!` and therefore `../..`-laden
/// (`CARGO_MANIFEST_DIR` + `/../../demo`) — comparing it against
/// `buffer-file-name` requires running both sides through `canon` so
/// the `../..` on this side collapses to match the other.
fn canon(p: &str) -> std::path::PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|e| panic!("canonicalize({:?}) failed: {}", p, e))
}

fn goto_after(interp: &mut Interp, needle: &str) {
    ok(interp, "(goto-char (point-min))");
    ok(interp, &format!("(search-forward {:?})", needle));
}

// ============================================================
// 1. Every file under demo/ opens in its expected major mode
// ============================================================

/// Recursively collects every regular file under ROOT, relative to
/// ROOT with `/`-separated components, skipping only `.DS_Store`
/// (macOS-generated, not tracked in git -- everything else, including
/// other dotfiles like `.rules.verible_lint`, is included on purpose).
fn walk_files(root: &std::path::Path, rel: &std::path::Path, out: &mut Vec<String>) {
    let dir = root.join(rel);
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir({:?}) failed: {}", dir, e))
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let child_rel = rel.join(&name);
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            walk_files(root, &child_rel, out);
        } else if ty.is_file() {
            if name_str == ".DS_Store" {
                continue;
            }
            out.push(child_rel.to_string_lossy().replace('\\', "/"));
        } else {
            // Symlinks and anything else report neither is_dir nor
            // is_file here (file_type() does not follow links), so
            // without this arm they would be skipped silently and the
            // "every file is accounted for" property would quietly
            // stop holding. demo/ has none today; if one is ever
            // added, decide deliberately rather than by omission.
            panic!(
                "{:?} under demo/ is neither a regular file nor a directory ({:?}); \
                 this walker deliberately refuses to guess",
                child_rel, ty
            );
        }
    }
}

#[test]
fn every_file_under_demo_opens_in_its_expected_major_mode() {
    const TABLE: [(&str, &str); 38] = [
        ("README.md", "fundamental-mode"),
        ("docs/design-notes.org", "org-mode"),
        ("editor/init-example.el", "emacs-lisp-mode"),
        ("rtl-verilog2001/.rules.verible_lint", "fundamental-mode"),
        ("rtl-verilog2001/fifo_gray_top.v", "verilog-mode"),
        ("rtl-verilog2001/fifo_gray_top_tb.v", "verilog-mode"),
        ("rtl-verilog2001/fifo_sync.v", "verilog-mode"),
        ("rtl-verilog2001/fifo_sync_tb.v", "verilog-mode"),
        ("rtl-verilog2001/gray_ctr.v", "verilog-mode"),
        ("rtl-verilog2001/gray_ctr_tb.v", "verilog-mode"),
        ("rtl/.slang/server.json", "fundamental-mode"),
        ("rtl/bus/axi4_lite_arbiter.sv", "verilog-mode"),
        ("rtl/bus/axi4_lite_if.sv", "verilog-mode"),
        ("rtl/core/alu.sv", "verilog-mode"),
        ("rtl/core/clk_gate.sv", "verilog-mode"),
        ("rtl/core/regfile.sv", "verilog-mode"),
        ("rtl/core/status_regs_stub.sv", "verilog-mode"),
        ("rtl/include/soc_defs.svh", "verilog-mode"),
        ("rtl/mem/sram_bank.sv", "verilog-mode"),
        ("rtl/mem/sram_dual_channel.sv", "verilog-mode"),
        ("rtl/mem/sram_wrapper.sv", "verilog-mode"),
        ("rtl/pkg/soc_pkg.sv", "verilog-mode"),
        ("rtl/top/soc_top.sv", "verilog-mode"),
        ("rtl/verible.filelist", "fundamental-mode"),
        ("tools/TimingReport.java", "java-mode"),
        ("run_editor.sh", "sh-mode"),
        ("tools/bitvec.rs", "rust-mode"),
        ("tools/crc32.c", "c-mode"),
        ("tools/lint_rtl.sh", "sh-mode"),
        ("tools/run_sim.sh", "sh-mode"),
        ("tools/sample_sim.log", "fundamental-mode"),
        ("tools/simlog_report.pl", "perl-mode"),
        ("tools/vcd_summary.py", "python-mode"),
        ("tools/vcd_writer.cpp", "c++-mode"),
        ("verif/axi4_lite_monitor.sv", "verilog-mode"),
        ("verif/soc_verif_pkg.sv", "verilog-mode"),
        ("verif/sram_bank_tb.sv", "verilog-mode"),
        ("verif/verible.filelist", "fundamental-mode"),
    ];
    let expected: BTreeMap<&str, &str> = TABLE.into_iter().collect();
    // Collecting into a map would silently drop a duplicated path,
    // taking its check with it.
    assert_eq!(
        expected.len(),
        TABLE.len(),
        "the expected-mode table lists the same path twice"
    );

    let root = demo_root();
    let mut on_disk = Vec::new();
    walk_files(&root, std::path::Path::new(""), &mut on_disk);
    on_disk.sort();

    let (mut i, _ed) = setup();
    let mut actual: BTreeMap<String, String> = BTreeMap::new();
    for rel in &on_disk {
        let abs = root.join(rel);
        ok(
            &mut i,
            &format!("(find-file-internal {:?})", abs.to_str().unwrap()),
        );
        let mode = ok(&mut i, "(major-mode-internal-get)");
        actual.insert(rel.clone(), mode);
    }

    let const_msg = "demo/ smoke test: adding/removing files under demo/ requires \
        updating both this test's expected-mode table and demo/README.md's file count";

    for (rel, mode) in &actual {
        match expected.get(rel.as_str()) {
            Some(expected_mode) => {
                assert_eq!(
                    mode, expected_mode,
                    "{:?} opened in major-mode {:?}, expected {:?}. {}",
                    rel, mode, expected_mode, const_msg
                );
            }
            None => panic!(
                "{:?} exists under demo/ but is not in this test's expected table. {}",
                rel, const_msg
            ),
        }
    }
    for rel in expected.keys() {
        assert!(
            actual.contains_key(*rel),
            "{:?} is in this test's expected table but no longer exists under demo/. {}",
            rel,
            const_msg
        );
    }
}

// ============================================================
// 2 & 3. M-. / M-, round trip: soc_top.sv <-> alu.sv
// ============================================================

#[test]
fn soc_top_m_dot_jumps_to_the_alu_module_definition() {
    let (mut i, _ed) = setup();
    let soc_top = demo_root().join("rtl/top/soc_top.sv");
    let alu = demo_root().join("rtl/core/alu.sv");
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", soc_top.to_str().unwrap()),
    );
    goto_after(&mut i, "alu #(");
    ok(&mut i, "(backward-char 5)");

    let r = ok(&mut i, "(lsp-definition-at-point)");
    assert_ne!(r, "nil", "M-. on `alu` must jump: {}", r);

    let landed = buffer_file_name(&mut i);
    assert_eq!(
        canon(&landed),
        canon(alu.to_str().unwrap()),
        "M-. must land in rtl/core/alu.sv, landed in {:?} instead",
        landed
    );

    // The module NAME `alu` (not `module`) is what point should sit on
    // -- read the text at point rather than asserting a byte offset,
    // which would go stale the moment the file is edited.
    let point = ok(&mut i, "(point)");
    let p: i64 = point.parse().unwrap();
    let text = ok(&mut i, &format!("(buffer-substring {} {})", p, p + 3));
    assert_eq!(
        text, "\"alu\"",
        "point must sit on the module name: {}",
        text
    );
}

#[test]
fn m_comma_returns_from_alu_back_to_the_instantiation() {
    let (mut i, _ed) = setup();
    let soc_top = demo_root().join("rtl/top/soc_top.sv");
    let alu = demo_root().join("rtl/core/alu.sv");
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", soc_top.to_str().unwrap()),
    );
    goto_after(&mut i, "alu #(");
    ok(&mut i, "(backward-char 5)");
    let point_before = ok(&mut i, "(point)");

    let r = ok(&mut i, "(lsp-definition-at-point)");
    assert_ne!(r, "nil", "M-. on `alu` must jump: {}", r);

    // Without this the whole test passes vacuously when M-. is broken:
    // a failed jump leaves point in soc_top.sv and pushes nothing onto
    // `lsp--marker-stack`, and popping an empty stack is a no-op, so
    // both assertions below would hold for the wrong reason.
    let mid = buffer_file_name(&mut i);
    assert_eq!(
        canon(&mid),
        canon(alu.to_str().unwrap()),
        "M-. must actually reach rtl/core/alu.sv before M-, is meaningful, reached {:?}",
        mid
    );

    ok(&mut i, "(lsp-pop-definition-stack)");

    let landed = buffer_file_name(&mut i);
    assert_eq!(
        canon(&landed),
        canon(soc_top.to_str().unwrap()),
        "M-, must jump back to soc_top.sv, landed in {:?} instead",
        landed
    );
    assert_eq!(
        ok(&mut i, "(point)"),
        point_before,
        "M-, must restore the exact point it jumped from"
    );
}

// ============================================================
// 4. C-M-i inside u_regfile's port list
// ============================================================

/// Every popup item's `insert` string, read straight off
/// `Editor::completion_popup` (mirrors `verilog_complete_tests.rs`'s
/// own `popup_items`/`insert_names` -- no elisp-level accessor exposes
/// item contents).
fn insert_names(ed: &Rc<RefCell<Editor>>) -> Option<Vec<String>> {
    let e = ed.borrow();
    e.completion_popup
        .as_ref()
        .map(|p| p.items.iter().map(|it| it.insert.clone()).collect())
}

#[test]
fn regfile_instantiation_offers_all_nine_port_names() {
    let (mut i, ed) = setup();
    let soc_top = demo_root().join("rtl/top/soc_top.sv");
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", soc_top.to_str().unwrap()),
    );
    goto_after(&mut i, ".raddr_a_i");
    ok(&mut i, "(backward-char 9)");

    let r = run(&mut i, "(completion-at-point)");
    assert_eq!(r, "t", "port context must be handled (t): {}", r);

    let mut names = insert_names(&ed).expect("popup must open");
    names.sort();

    // The complete port list of demo/rtl/core/regfile.sv -- completion
    // sees it from a DIFFERENT file (rtl/top/soc_top.sv) because it
    // resolves modules across rtl/verible.filelist and recursive
    // library search (M56), not just the current buffer.
    let mut expected = vec![
        "clk_i",
        "rst_ni",
        "raddr_a_i",
        "rdata_a_o",
        "raddr_b_i",
        "rdata_b_o",
        "we_i",
        "waddr_i",
        "wdata_i",
    ];
    expected.sort();
    assert_eq!(names, expected);
}

// ============================================================
// 5. C-c C-a / C-c C-k on u_arbiter's /*AUTOINST*/
// ============================================================

/// Slices out just the u_arbiter *instantiation* -- from `u_arbiter (`
/// to the first `);` that follows it -- and nothing else.
///
/// This is required, not cosmetic: `soc_top.sv` carries an
/// `AUTO_TEMPLATE` comment directly above `u_arbiter` (added so
/// AUTOINST connects arbiter's oddly-named ports to this module's own
/// signal names instead of leaving them unconnected). That comment's
/// text deliberately contains the same port-name substrings the
/// expansion produces (`.req_ready_o(host_req_ready_o),` and friends).
/// If the assertions below searched the *whole buffer* instead of just
/// this instantiation, every one of them would pass even if AUTOINST
/// expanded nothing at all -- the template comment alone would satisfy
/// them. Slicing to the instantiation keeps the template out of the
/// region under test, so these assertions can actually fail.
fn arbiter_instance_segment(buffer: &str) -> &str {
    let start_marker = "u_arbiter (";
    let start = buffer
        .find(start_marker)
        .expect("u_arbiter instantiation must be present in soc_top.sv")
        + start_marker.len();
    let rest = &buffer[start..];
    let end = rest
        .find(");")
        .expect("u_arbiter instantiation must be closed with ');'");
    &rest[..end]
}

#[test]
fn arbiter_autoinst_expands_then_deletes() {
    let (mut i, _ed) = setup();
    let soc_top = demo_root().join("rtl/top/soc_top.sv");
    let soc_top_str = soc_top.to_str().unwrap();

    let bytes_before = std::fs::read(&soc_top).unwrap();

    ok(&mut i, &format!("(find-file-internal {:?})", soc_top_str));

    let auto_result = ok(&mut i, "(verilog-auto)");
    assert!(
        // Anchored on the message prefix, not a bare "1 inst": the echo
        // is "verilog-auto: %d inst, ...", so an unanchored substring
        // would also match a run that expanded 11 or 21 instances.
        auto_result.contains("verilog-auto: 1 inst,"),
        "verilog-auto's echo must report exactly 1 instance expanded: {}",
        auto_result
    );

    let expanded = ok(&mut i, "(buffer-string)");
    let expanded_instance = arbiter_instance_segment(&expanded);
    for port in [
        ".req_ready_o",
        ".gnt_valid_o",
        ".gnt_req_o",
        ".gnt_idx_o",
        ".clk_i",
        ".rst_ni",
        ".req_valid_i",
        ".req_i",
        ".gnt_ready_i",
    ] {
        assert!(
            expanded_instance.contains(port),
            "expanded u_arbiter instantiation must connect {}: instance text does not contain it",
            port
        );
    }
    assert!(
        expanded_instance.contains("// Outputs"),
        "expanded AUTOINST must have an // Outputs header"
    );
    assert!(
        expanded_instance.contains("// Inputs"),
        "expanded AUTOINST must have an // Inputs header"
    );

    ok(&mut i, "(verilog-delete-auto)");
    let deleted = ok(&mut i, "(buffer-string)");
    let deleted_instance = arbiter_instance_segment(&deleted);
    assert!(
        !deleted_instance.contains(".req_ready_o"),
        "verilog-delete-auto must remove the expanded connections"
    );
    assert!(
        deleted.contains("/*AUTOINST*/);"),
        "verilog-delete-auto is a known NON-round-trip: it collapses the \
        original two-line `/*AUTOINST*/\\n  );` into one line `/*AUTOINST*/);` \
        rather than restoring the original whitespace -- this asserts that \
        actual (documented) behavior, not a restored original"
    );

    let bytes_after = std::fs::read(&soc_top).unwrap();
    assert_eq!(
        bytes_before, bytes_after,
        "this test must never write back to demo/rtl/top/soc_top.sv on disk"
    );
}

/// Raw string value of `(buffer-string)`, read straight off the
/// interpreter `Value` (mirrors `buffer_file_name` above and
/// `verilog_auto_tests.rs`'s own `bs` helper) rather than through
/// `prin1_to_string`, which would escape/quote the text and defeat a
/// byte-identical comparison.
fn buffer_string(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-string)") {
        Ok(Value::Str(s)) => (*s).clone(),
        Ok(_) => panic!("(buffer-string) did not return a string"),
        Err(flow) => panic!("(buffer-string) errored: {}", interp.describe_flow(&flow)),
    }
}

/// Set this to `1`/`true`/`yes` to turn a missing `verible-verilog-format`
/// on `PATH` into a deliberate, visible skip instead of a failure --
/// mirrors `dev_tools_tests.rs`'s own `RETICLE_ALLOW_MISSING_PILLOW`. Any
/// other value, including `0`, means "not allowed to skip" (matching
/// `demo/tools/run_sim.sh`'s own `DEMO_SIM_ALLOW_MISSING` convention).
const SKIP_ENV_VERIBLE_FORMAT: &str = "RETICLE_ALLOW_MISSING_VERIBLE_FORMAT";

/// Whether `verible-verilog-format` is on `PATH` at all -- `format-buffer'
/// (`crates/core/lisp/format.el:284`) never signals an ELISP error when
/// its external formatter binary is missing (a failed `call-process`
/// is reported via `message', not `error' -- see `format--run-external'),
/// so this Rust-level check is the only thing that can turn a missing
/// binary into a loud test failure rather than a silent no-op that
/// happens to leave the buffer already matching the (unformatted)
/// on-disk file by coincidence.
fn verible_verilog_format_available() -> bool {
    match std::process::Command::new("verible-verilog-format")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

/// M125 Part C, fix round (spec section 2): pins "the checked-in
/// `demo/rtl-verilog2001/fifo_gray_top.v` is exactly what a real user
/// ends up with on disk" -- and a real user's own pipeline is *generate,
/// then save*, because `format-on-save' defaults to `t'
/// (`crates/core/lisp/format.el:360`, `before-save-hook' at `:393') and
/// `format-buffer' is a public command (`:284'). So this test runs
/// `(verilog-delete-auto)', `(verilog-auto)', THEN `(format-buffer)' --
/// generator layout followed by verible -- not generator layout alone,
/// which is what the pre-fix-round version of this test checked and is
/// why the checked-in file used to fail `demo/tools/lint_rtl.sh's own
/// `format' step (the AUTO generator aligns trailing comments to a
/// shared column and indents connection lists to the marker's own
/// column, GNU verilog-mode's own house style; `verible-verilog-format
/// --indentation_spaces=2' collapses all of that).
///
/// Two instances (`u_fifo_sync`, `u_gray_ctr`) share `clk`/`rst_n`, which
/// is what makes this real material rather than a single-instance
/// fixture: the checked-in AUTOINPUT block collapses both contributions
/// into one declaration per name.
#[test]
fn demo_verilog2001_auto_top_matches_editor_output() {
    if !verible_verilog_format_available() {
        let opted_out = matches!(
            std::env::var(SKIP_ENV_VERIBLE_FORMAT).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        if opted_out {
            eprintln!(
                "skipping (opted out via {}): verible-verilog-format is not \
                 available, demo_verilog2001_auto_top_matches_editor_output was not run",
                SKIP_ENV_VERIBLE_FORMAT
            );
            return;
        }
        panic!(
            "verible-verilog-format is not available -- \
             demo_verilog2001_auto_top_matches_editor_output was not run. \
             Failing by default so a missing dependency cannot silently \
             pass as a green gate. Install Verible \
             (https://github.com/chipsalliance/verible), or set {}=1 to \
             deliberately skip on a machine that genuinely lacks it.",
            SKIP_ENV_VERIBLE_FORMAT
        );
    }

    let (mut i, _ed) = setup();
    let top = demo_root().join("rtl-verilog2001/fifo_gray_top.v");
    let top_str = top.to_str().unwrap();

    let on_disk = std::fs::read_to_string(&top).unwrap();

    ok(&mut i, &format!("(find-file-internal {:?})", top_str));
    ok(&mut i, "(verilog-delete-auto)");
    ok(&mut i, "(verilog-auto)");
    ok(&mut i, "(format-buffer)");

    let regenerated = buffer_string(&mut i);
    assert_eq!(
        regenerated, on_disk,
        "demo/rtl-verilog2001/fifo_gray_top.v on disk must be byte-identical \
        to what (verilog-delete-auto), (verilog-auto), then (format-buffer) \
        regenerate -- i.e. exactly the generate-then-save pipeline a real user gets"
    );

    let bytes_after = std::fs::read(&top).unwrap();
    assert_eq!(
        bytes_after,
        on_disk.as_bytes(),
        "this test must never write back to demo/rtl-verilog2001/fifo_gray_top.v on disk"
    );
}

/// M126 Part C1: pins "the checked-in `demo/rtl-verilog2001/gray_ctr.v`
/// is exactly what a real user ends up with on disk" for `/*AUTOREG*/',
/// the same generate-then-save pipeline
/// `demo_verilog2001_auto_top_matches_editor_output' above pins for
/// `fifo_gray_top.v''s own AUTOOUTPUT/AUTOINPUT/AUTOINOUT. `gray_ctr.v'
/// is the only non-ANSI module under `demo/' with an `output reg' before
/// M126 -- its own header now records why `bin_count' was changed to a
/// bare, untyped `output' so AUTOREG has something real to fill in.
#[test]
fn demo_verilog2001_gray_ctr_autoreg_matches_editor_output() {
    if !verible_verilog_format_available() {
        let opted_out = matches!(
            std::env::var(SKIP_ENV_VERIBLE_FORMAT).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        if opted_out {
            eprintln!(
                "skipping (opted out via {}): verible-verilog-format is not \
                 available, demo_verilog2001_gray_ctr_autoreg_matches_editor_output was not run",
                SKIP_ENV_VERIBLE_FORMAT
            );
            return;
        }
        panic!(
            "verible-verilog-format is not available -- \
             demo_verilog2001_gray_ctr_autoreg_matches_editor_output was not run. \
             Failing by default so a missing dependency cannot silently \
             pass as a green gate. Install Verible \
             (https://github.com/chipsalliance/verible), or set {}=1 to \
             deliberately skip on a machine that genuinely lacks it.",
            SKIP_ENV_VERIBLE_FORMAT
        );
    }

    let (mut i, _ed) = setup();
    let path = demo_root().join("rtl-verilog2001/gray_ctr.v");
    let path_str = path.to_str().unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();

    ok(&mut i, &format!("(find-file-internal {:?})", path_str));
    ok(&mut i, "(verilog-delete-auto)");
    ok(&mut i, "(verilog-auto)");
    ok(&mut i, "(format-buffer)");

    let regenerated = buffer_string(&mut i);
    assert_eq!(
        regenerated, on_disk,
        "demo/rtl-verilog2001/gray_ctr.v on disk must be byte-identical \
        to what (verilog-delete-auto), (verilog-auto), then (format-buffer) \
        regenerate -- i.e. exactly the generate-then-save pipeline a real user gets"
    );

    let bytes_after = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes_after,
        on_disk.as_bytes(),
        "this test must never write back to demo/rtl-verilog2001/gray_ctr.v on disk"
    );
}

/// M126 Part C2: pins "the checked-in `demo/rtl/core/status_regs_stub.sv`
/// is exactly what a real user ends up with on disk" for `/*AUTOTIEOFF*/'
/// on a real ANSI SystemVerilog module -- the same generate-then-save
/// pipeline the two tests above pin for their own AUTO commands. This is
/// the ONE test in this file proving divergence 2 (the ANSI `assign'
/// switch) actually matters on `demo/rtl/', which is entirely ANSI --
/// not just on a synthetic fixture in verilog_auto_tests.rs.
#[test]
fn demo_rtl_status_regs_stub_autotieoff_matches_editor_output() {
    if !verible_verilog_format_available() {
        let opted_out = matches!(
            std::env::var(SKIP_ENV_VERIBLE_FORMAT).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        if opted_out {
            eprintln!(
                "skipping (opted out via {}): verible-verilog-format is not \
                 available, demo_rtl_status_regs_stub_autotieoff_matches_editor_output was not run",
                SKIP_ENV_VERIBLE_FORMAT
            );
            return;
        }
        panic!(
            "verible-verilog-format is not available -- \
             demo_rtl_status_regs_stub_autotieoff_matches_editor_output was not run. \
             Failing by default so a missing dependency cannot silently \
             pass as a green gate. Install Verible \
             (https://github.com/chipsalliance/verible), or set {}=1 to \
             deliberately skip on a machine that genuinely lacks it.",
            SKIP_ENV_VERIBLE_FORMAT
        );
    }

    let (mut i, _ed) = setup();
    let path = demo_root().join("rtl/core/status_regs_stub.sv");
    let path_str = path.to_str().unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();

    ok(&mut i, &format!("(find-file-internal {:?})", path_str));
    ok(&mut i, "(verilog-delete-auto)");
    ok(&mut i, "(verilog-auto)");
    ok(&mut i, "(format-buffer)");

    let regenerated = buffer_string(&mut i);
    assert_eq!(
        regenerated, on_disk,
        "demo/rtl/core/status_regs_stub.sv on disk must be byte-identical \
        to what (verilog-delete-auto), (verilog-auto), then (format-buffer) \
        regenerate -- i.e. exactly the generate-then-save pipeline a real user gets"
    );

    let bytes_after = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes_after,
        on_disk.as_bytes(),
        "this test must never write back to demo/rtl/core/status_regs_stub.sv on disk"
    );
}

/// M127: `rtl/mem/sram_dual_channel.sv` is the showcase file for
/// AUTO_TEMPLATE's `@' instance-number substitution and `[]' bit-range
/// tokens -- neither had any exercise anywhere under `demo/' before this
/// milestone. Same pinning discipline as the two tests above: this is the
/// ONE test in this file proving `@'/`[]' actually work on real,
/// parameterised RTL, not just on a synthetic fixture in
/// verilog_auto_tests.rs.
#[test]
fn demo_rtl_sram_dual_channel_autoinst_matches_editor_output() {
    if !verible_verilog_format_available() {
        let opted_out = matches!(
            std::env::var(SKIP_ENV_VERIBLE_FORMAT).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        if opted_out {
            eprintln!(
                "skipping (opted out via {}): verible-verilog-format is not \
                 available, demo_rtl_sram_dual_channel_autoinst_matches_editor_output was not run",
                SKIP_ENV_VERIBLE_FORMAT
            );
            return;
        }
        panic!(
            "verible-verilog-format is not available -- \
             demo_rtl_sram_dual_channel_autoinst_matches_editor_output was not run. \
             Failing by default so a missing dependency cannot silently \
             pass as a green gate. Install Verible \
             (https://github.com/chipsalliance/verible), or set {}=1 to \
             deliberately skip on a machine that genuinely lacks it.",
            SKIP_ENV_VERIBLE_FORMAT
        );
    }

    let (mut i, _ed) = setup();
    let path = demo_root().join("rtl/mem/sram_dual_channel.sv");
    let path_str = path.to_str().unwrap();

    let on_disk = std::fs::read_to_string(&path).unwrap();

    ok(&mut i, &format!("(find-file-internal {:?})", path_str));
    ok(&mut i, "(verilog-delete-auto)");
    ok(&mut i, "(verilog-auto)");
    ok(&mut i, "(format-buffer)");

    let regenerated = buffer_string(&mut i);
    assert_eq!(
        regenerated, on_disk,
        "demo/rtl/mem/sram_dual_channel.sv on disk must be byte-identical \
        to what (verilog-delete-auto), (verilog-auto), then (format-buffer) \
        regenerate -- i.e. exactly the generate-then-save pipeline a real user gets"
    );
    assert!(
        regenerated.contains("// Templated"),
        "sanity -- this file's whole point is exercising the `// Templated' annotation: {}",
        regenerated
    );

    let bytes_after = std::fs::read(&path).unwrap();
    assert_eq!(
        bytes_after,
        on_disk.as_bytes(),
        "this test must never write back to demo/rtl/mem/sram_dual_channel.sv on disk"
    );
}

// ============================================================
// 6. M73: every demo/ Verilog file's own on-disk indent width is what
//    `indent--detect-width' (indent.el:650, a pure scanner -- read-only,
//    never applies anything) actually detects, and `standard-indent-width'
//    ends up 2 for all nine -- see demo/README.md's "three editor claims"
//    section and indent.el's M73 header.
//
//    M104 note: before M104, verilog-mode's own mode DEFAULT was 4, so
//    asserting `standard-indent-width' alone was enough to tell "detection
//    found 2" apart from "detection declined, mode default stood" for
//    `rtl/include/soc_defs.svh' (4 vs. 2). M104 dropped that default to 2
//    (to match `verible-verilog-format''s own default -- see modes.el),
//    which made the old assertion vacuous: with the default now ALSO 2,
//    every file in this array reads `standard-indent-width' == 2 whether
//    detection ran successfully or silently declined and fell back to the
//    default -- the array stopped being able to tell those two cases
//    apart, exactly the "test doesn't reach what it claims to check"
//    failure mode this project has hit seven times in one session before.
//    So this test now asserts the DETECTOR's own return value directly
//    (`indent--detect-width', not `standard-indent-width') -- that
//    function's behavior has nothing to do with any mode's default, so
//    this stays a real detection-mechanism test regardless of what the
//    default is set to in the future. `standard-indent-width' is still
//    checked below, but only as a secondary confirmation that the
//    detected/defaulted value actually got applied to the buffer.
// ============================================================

#[test]
fn demo_rtl_verilog_files_detect_two_space_width_except_the_undersampled_svh() {
    let root = demo_root();
    let verilog_files: [(&str, Option<i64>); 21] = [
        ("rtl-verilog2001/fifo_gray_top.v", Some(2)),
        ("rtl-verilog2001/fifo_gray_top_tb.v", Some(2)),
        ("rtl-verilog2001/fifo_sync.v", Some(2)),
        ("rtl-verilog2001/fifo_sync_tb.v", Some(2)),
        ("rtl-verilog2001/gray_ctr.v", Some(2)),
        ("rtl-verilog2001/gray_ctr_tb.v", Some(2)),
        ("rtl/bus/axi4_lite_arbiter.sv", Some(2)),
        ("rtl/bus/axi4_lite_if.sv", Some(2)),
        ("rtl/core/alu.sv", Some(2)),
        ("rtl/core/clk_gate.sv", Some(2)),
        ("rtl/core/regfile.sv", Some(2)),
        ("rtl/core/status_regs_stub.sv", Some(2)),
        // Only 2 lines of this 28-line file are indented at all (a
        // `SOC_ASSERT' macro continuation) -- below
        // `indent--detect-min-samples' (5), so detection returns nil
        // ("not enough evidence to guess") and the mode default (2,
        // as of M104) stands. Not a bug: "don't guess without evidence"
        // is the documented algorithm, and this file is the real-world
        // case that exercises it, not a synthetic one.
        ("rtl/include/soc_defs.svh", None),
        ("rtl/mem/sram_bank.sv", Some(2)),
        ("rtl/mem/sram_dual_channel.sv", Some(2)),
        ("rtl/mem/sram_wrapper.sv", Some(2)),
        ("rtl/pkg/soc_pkg.sv", Some(2)),
        ("rtl/top/soc_top.sv", Some(2)),
        ("verif/axi4_lite_monitor.sv", Some(2)),
        ("verif/soc_verif_pkg.sv", Some(2)),
        ("verif/sram_bank_tb.sv", Some(2)),
    ];
    // M122: a mechanism that decides what gets checked must fail loudly
    // when it decides nothing does (CLAUDE.md, M114) -- this array is
    // hand-maintained, so a new file added under demo/rtl or demo/verif
    // without a matching entry here would previously be silently never
    // checked at all. Cross-check against real filesystem discovery
    // (mirroring `m119_collect_sv_files' in indent_tests.rs, but for
    // `.v'/`.vh' too, since demo/rtl-verilog2001 is `.v').
    fn collect_verilog_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{} must exist and be readable: {}", dir.display(), e));
        for entry in entries {
            let entry = entry.expect("readable dir entry");
            let path = entry.path();
            if path.is_dir() {
                collect_verilog_files(&path, out);
            } else if path
                .extension()
                .is_some_and(|e| e == "sv" || e == "svh" || e == "v" || e == "vh")
            {
                out.push(path);
            }
        }
    }
    let mut on_disk = Vec::new();
    collect_verilog_files(&root.join("rtl-verilog2001"), &mut on_disk);
    collect_verilog_files(&root.join("rtl"), &mut on_disk);
    collect_verilog_files(&root.join("verif"), &mut on_disk);
    let checked: std::collections::BTreeSet<&str> =
        verilog_files.iter().map(|(rel, _)| *rel).collect();
    let mut missing: Vec<String> = Vec::new();
    for path in &on_disk {
        let rel = path
            .strip_prefix(&root)
            .expect("path is under demo_root")
            .to_string_lossy()
            .replace('\\', "/");
        if !checked.contains(rel.as_str()) {
            missing.push(rel);
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "demo smoke test: these Verilog/SystemVerilog file(s) exist on disk under demo/rtl-\
         verilog2001, demo/rtl or demo/verif but are NOT in this test's hand-maintained \
         `verilog_files' array, so indent-width detection is silently never checked for them: \
         {missing:?}"
    );
    let (mut i, _ed) = setup();
    for (rel, expected_detection) in verilog_files {
        let abs = root.join(rel);
        ok(
            &mut i,
            &format!("(find-file-internal {:?})", abs.to_str().unwrap()),
        );
        let detected = ok(&mut i, "(indent--detect-width)");
        let expected_detected_str = match expected_detection {
            Some(w) => w.to_string(),
            None => "nil".to_string(),
        };
        assert_eq!(
            detected, expected_detected_str,
            "{:?}: indent--detect-width returned {}, expected {}",
            rel, detected, expected_detected_str
        );
        // Whether detection succeeded or declined, the applied width
        // (detected value, or the mode default of 2 when it declined)
        // must be 2 for every file here.
        let width = ok(&mut i, "standard-indent-width");
        assert_eq!(
            width, "2",
            "{:?}: standard-indent-width was {}, expected 2 (detected or mode default)",
            rel, width
        );
    }
}

#[test]
fn opening_a_new_line_in_alu_sv_body_lands_at_column_two_matching_verible() {
    let (mut i, ed) = setup();
    let abs = demo_root().join("rtl/core/alu.sv");
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", abs.to_str().unwrap()),
    );
    assert_eq!(ok(&mut i, "standard-indent-width"), "2");
    ok(&mut i, "(evil-mode 1)");
    // "logic valid_d, valid_q;" sits directly in the module body (depth
    // 1) -- verilog-mode's own default width (4) would land the new
    // line's cursor at column 4, one column verible-verilog-format
    // would then diff back down to 2 (this milestone's repro).
    goto_after(&mut i, "logic valid_d, valid_q;");
    core::commands::feed_keys(&mut i, &ed, "o")
        .unwrap_or_else(|e| panic!("feed_keys \"o\": {}", e));
    let line = match i.eval_source("(buffer-substring (line-beginning-position) (point))") {
        Ok(Value::Str(s)) => (*s).clone(),
        other => panic!(
            "(buffer-substring (line-beginning-position) (point)) didn't return a string: {:?}",
            other.is_ok()
        ),
    };
    assert_eq!(
        line, "  ",
        "new line opened in alu.sv's module body must be indented to column 2, not 4"
    );
}
