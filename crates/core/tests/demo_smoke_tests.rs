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
    const TABLE: [(&str, &str); 22] = [
        ("README.md", "fundamental-mode"),
        ("docs/design-notes.org", "org-mode"),
        ("editor/init-example.el", "emacs-lisp-mode"),
        ("rtl-verilog2001/.rules.verible_lint", "fundamental-mode"),
        ("rtl-verilog2001/fifo_sync.v", "verilog-mode"),
        ("rtl-verilog2001/fifo_sync_tb.v", "verilog-mode"),
        ("rtl/bus/axi4_lite_arbiter.sv", "verilog-mode"),
        ("rtl/core/alu.sv", "verilog-mode"),
        ("rtl/core/regfile.sv", "verilog-mode"),
        ("rtl/include/soc_defs.svh", "verilog-mode"),
        ("rtl/mem/sram_wrapper.sv", "verilog-mode"),
        ("rtl/pkg/soc_pkg.sv", "verilog-mode"),
        ("rtl/top/soc_top.sv", "verilog-mode"),
        ("rtl/verible.filelist", "fundamental-mode"),
        ("tools/TimingReport.java", "java-mode"),
        ("tools/bitvec.rs", "rust-mode"),
        ("tools/crc32.c", "c-mode"),
        ("tools/lint_rtl.sh", "sh-mode"),
        ("tools/sample_sim.log", "fundamental-mode"),
        ("tools/simlog_report.pl", "perl-mode"),
        ("tools/vcd_summary.py", "python-mode"),
        ("tools/vcd_writer.cpp", "c++-mode"),
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
            expanded.contains(port),
            "expanded u_arbiter must connect {}: buffer does not contain it",
            port
        );
    }
    assert!(
        expanded.contains("// Outputs"),
        "expanded AUTOINST must have an // Outputs header"
    );
    assert!(
        expanded.contains("// Inputs"),
        "expanded AUTOINST must have an // Inputs header"
    );

    ok(&mut i, "(verilog-delete-auto)");
    let deleted = ok(&mut i, "(buffer-string)");
    assert!(
        !deleted.contains(".req_ready_o"),
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

// ============================================================
// 6. M73: every demo/ Verilog file's `standard-indent-width' matches
//    its own 2-space style, and editing alu.sv produces a 2-column
//    indent (not the verilog-mode default of 4) -- see demo/README.md's
//    "three editor claims" section and indent.el's M73 header.
// ============================================================

#[test]
fn demo_rtl_verilog_files_detect_two_space_width_except_the_undersampled_svh() {
    let root = demo_root();
    let verilog_files: [(&str, i64); 9] = [
        ("rtl-verilog2001/fifo_sync.v", 2),
        ("rtl-verilog2001/fifo_sync_tb.v", 2),
        ("rtl/bus/axi4_lite_arbiter.sv", 2),
        ("rtl/core/alu.sv", 2),
        ("rtl/core/regfile.sv", 2),
        // Only 2 lines of this 28-line file are indented at all (a
        // `SOC_ASSERT' macro continuation) -- below
        // `indent--detect-min-samples' (5), so detection returns nil
        // ("not enough evidence to guess") and the mode default (4)
        // stands. Not a bug: "don't guess without evidence" is the
        // documented algorithm, and this file is the real-world case
        // that exercises it, not a synthetic one.
        ("rtl/include/soc_defs.svh", 4),
        ("rtl/mem/sram_wrapper.sv", 2),
        ("rtl/pkg/soc_pkg.sv", 2),
        ("rtl/top/soc_top.sv", 2),
    ];
    let (mut i, _ed) = setup();
    for (rel, expected_width) in verilog_files {
        let abs = root.join(rel);
        ok(
            &mut i,
            &format!("(find-file-internal {:?})", abs.to_str().unwrap()),
        );
        let width = ok(&mut i, "standard-indent-width");
        assert_eq!(
            width,
            expected_width.to_string(),
            "{:?}: standard-indent-width was {}, expected {}",
            rel,
            width,
            expected_width
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
