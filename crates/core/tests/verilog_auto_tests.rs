//! M39: GNU verilog-mode's AUTO system core -- AUTOINST/AUTOWIRE/AUTOARG
//! expansion (`verilog-auto', C-c C-a) and cleanup (`verilog-delete-auto',
//! C-c C-k). See crates/core/lisp/verilog-auto.el's own header for the v1
//! scope cuts and policy choices these tests pin down.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
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

/// Like `run`, but asserts success -- for setup steps whose own success
/// isn't the thing under test (mirrors `verilog_complete_tests.rs'/
/// `verilog_nav_tests.rs' own `ok` helper).
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
            "se_va_{}_{}_{}",
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

/// 0-based position of PATH in `(verilog-auto--library-files)`'s own
/// return value, or the list's length if PATH isn't in it at all (a
/// deliberately out-of-range sentinel, never confusable with a real
/// index -- callers compare two of these against each other, so an
/// absent path failing loudly via a nonsensical ordering is preferable
/// to a silent false match). Implemented via `member` (`equal`-based)
/// rather than parsing `run`'s own `prin1-to-string` output, which
/// would double-escape a path string's own characters.
fn library_files_index_of(interp: &mut Interp, path: &str) -> i64 {
    let total = run(interp, "(length (verilog-auto--library-files))")
        .parse::<i64>()
        .unwrap();
    let tail = run(
        interp,
        &format!("(length (member {:?} (verilog-auto--library-files)))", path),
    )
    .parse::<i64>()
    .unwrap();
    total - tail
}

fn bs(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-string)") {
        Ok(Value::Str(s)) => (*s).clone(),
        other => panic!(
            "(buffer-string) didn't return a string: {:?}",
            other.is_ok()
        ),
    }
}

fn insert_src(interp: &mut Interp, src: &str) {
    let r = run(interp, &format!("(insert {:?})", src));
    assert!(!r.starts_with("ERROR"), "insert failed: {}", r);
}

fn verilog_auto(interp: &mut Interp) -> String {
    let r = run(interp, "(verilog-auto)");
    assert!(!r.starts_with("ERROR"), "verilog-auto failed: {}", r);
    r
}

fn delete_auto(interp: &mut Interp) -> String {
    let r = run(interp, "(verilog-delete-auto)");
    assert!(!r.starts_with("ERROR"), "verilog-delete-auto failed: {}", r);
    r
}

/// Mirrors `verilog-auto--pad-to-column`: S padded with trailing spaces
/// to column COL, measured with OFFSET (leading indentation not part of
/// S itself) already added, at least one space always kept.
fn pad(s: &str, col: usize, offset: usize) -> String {
    let n = (col as isize - (offset + s.len()) as isize).max(1) as usize;
    format!("{}{}", s, " ".repeat(n))
}

/// One AUTOINST connection line, INDENT included, matching
/// `verilog-auto--connection-text` + its caller's own INDENT prefix.
fn conn(indent: &str, name: &str, expr: &str) -> String {
    format!(
        "{}{}({})",
        indent,
        pad(&format!(".{}", name), 40, indent.len()),
        expr
    )
}

// ===================== AUTOINST =====================

const SUB_MOD: &str = "\
module sub_mod (
  input  logic clk,
  input  logic rst_n,
  inout  logic io_bus,
  output logic [WIDTH-1:0] count,
  output logic done
);
endmodule

";

#[test]
fn autoinst_full_expansion_groups_aligns_and_orders() {
    let (mut i, _ed) = setup();
    let inst_line = "  sub_mod u1 (/*AUTOINST*/);\n";
    let indent = " ".repeat("  sub_mod u1 (".len());
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire clk;\n  wire rst_n;\n  wire io_bus;\n  wire [WIDTH-1:0] count;\n  wire done;\n{}endmodule\n",
            SUB_MOD, inst_line
        ),
    );
    verilog_auto(&mut i);
    let expected_block = format!(
        "/*AUTOINST*/\n{indent}// Outputs\n{c1},\n{c2},\n{indent}// Inouts\n{c3},\n{indent}// Inputs\n{c4},\n{c5}",
        indent = indent,
        c1 = conn(&indent, "count", "count[WIDTH-1:0]"),
        c2 = conn(&indent, "done", "done"),
        c3 = conn(&indent, "io_bus", "io_bus"),
        c4 = conn(&indent, "clk", "clk"),
        c5 = conn(&indent, "rst_n", "rst_n"),
    );
    let text = bs(&mut i);
    assert!(
        text.contains(&expected_block),
        "expected block:\n{}\n\ngot buffer:\n{}",
        expected_block,
        text
    );
    // Closing paren glued directly onto the last connection line, no
    // trailing comma, immediately followed by the instantiation's `;'.
    assert!(
        text.contains(&format!("{});\n", conn(&indent, "rst_n", "rst_n"))),
        "closing paren must directly follow the last connection: {}",
        text
    );
}

#[test]
fn autoinst_skips_explicit_connections() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire done;\n  sub_mod u1 (.clk(clk), /*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // `clk' was already explicit -- it must appear only once (the
    // user's own hand-written connection), never regenerated.
    assert_eq!(
        text.matches(".clk(").count(),
        1,
        "clk must not be re-generated: {}",
        text
    );
    let indent = " ".repeat("  sub_mod u1 (".len());
    let expected = format!(
        "/*AUTOINST*/\n{}// Outputs\n{});",
        indent,
        conn(&indent, "done", "done")
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autoinst_all_explicit_expands_to_nothing() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire done;\n  sub_mod u1 (.clk(clk), .done(done), /*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOINST*/);"),
        "every port already connected -- nothing should be inserted: {}",
        text
    );
}

#[test]
fn autoinst_param_override_substitutes_range_word_boundary_and_no_override_keeps_symbol() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [WIDTH-1:0] count,\n  output logic [WIDTH2-1:0] count2\n);\nendmodule\n\nmodule top;\n  wire [WIDTH-1:0] count;\n  wire [WIDTH2-1:0] count2;\n  sub_mod #(.WIDTH(8)) u1 (/*AUTOINST*/);\n  sub_mod u2 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // With a #(.WIDTH(8)) override: WIDTH -> (8), whole-word only, so
    // WIDTH2's own range is untouched by a WIDTH substitution.
    assert!(
        text.contains("count[(8)-1:0]"),
        "WIDTH should substitute to (8): {}",
        text
    );
    assert!(
        text.contains("count2[WIDTH2-1:0]"),
        "WIDTH2 must not be touched by a WIDTH override: {}",
        text
    );
    // Without any override (u2): both ranges stay fully symbolic.
    let u2_start = text.find("sub_mod u2").expect("u2 instantiation present");
    let u2_text = &text[u2_start..];
    assert!(
        u2_text.contains("count[WIDTH-1:0]") && u2_text.contains("count2[WIDTH2-1:0]"),
        "no override -- ranges must stay symbolic verbatim: {}",
        u2_text
    );
}

#[test]
fn autoinst_multiple_instances_same_module_and_a_different_module() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule mod_b (\n  output logic ready\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire done;\n  wire ready;\n  sub_mod u1 (/*AUTOINST*/);\n  sub_mod u2 (/*AUTOINST*/);\n  mod_b u3 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("3 inst"),
        "expected 3 instances processed: {}",
        msg
    );
    let text = bs(&mut i);
    // Every instance resolved its own module's ports correctly -- no
    // cross-contamination between sub_mod's cache entry and mod_b's.
    assert_eq!(
        text.matches(".clk").count(),
        2,
        "u1 and u2 each connect clk: {}",
        text
    );
    assert_eq!(
        text.matches(".done").count(),
        2,
        "u1 and u2 each connect done: {}",
        text
    );
    assert_eq!(
        text.matches(".ready").count(),
        1,
        "only u3 connects ready: {}",
        text
    );
}

// ===================== AUTOWIRE =====================

#[test]
fn autowire_declares_wire_for_undeclared_output() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic [WIDTH-1:0] count,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), .count(count), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(
            "/*AUTOWIRE*/\n  // Beginning of automatic wires (for undeclared instantiated-module outputs)\n  wire [WIDTH-1:0] count;\n  wire done;\n  // End of automatics\n"
        ),
        "buffer:\n{}",
        text
    );
}

#[test]
fn autowire_skips_already_declared_signal() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic [WIDTH-1:0] count,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire done;\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), .count(count), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("wire [WIDTH-1:0] count;"),
        "count still needed: {}",
        text
    );
    // `done' was already declared by the user -- it must appear only
    // that once, never regenerated by AUTOWIRE.
    assert_eq!(
        text.matches("wire done;").count(),
        1,
        "done already declared, must not be regenerated: {}",
        text
    );
}

#[test]
fn autowire_input_only_module_produces_no_wire() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module in_only (\n  input logic clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  /*AUTOWIRE*/\n  in_only u1 (.clk(clk));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOWIRE*/\n  in_only"),
        "an input-only connection set must insert nothing: {}",
        text
    );
}

#[test]
fn autowire_skips_non_bare_connection() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [3:0] count\n);\nendmodule\n\nmodule top;\n  wire [7:0] bus;\n  /*AUTOWIRE*/\n  sub_mod u1 (.count(bus[3:0]));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOWIRE*/\n  sub_mod"),
        "a non-bare connection target must not synthesize a wire: {}",
        text
    );
}

#[test]
fn autowire_dedups_same_name_first_range_wins() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module mod_a (\n  output logic [3:0] shared\n);\nendmodule\n\nmodule mod_b (\n  output logic [7:0] shared\n);\nendmodule\n\nmodule top;\n  /*AUTOWIRE*/\n  mod_a ua (.shared(shared));\n  mod_b ub (.shared(shared));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // "wire" alone would also match the "...automatic wires (for..."
    // marker comment text; "wire [" only ever appears in an actual
    // ranged wire declaration line.
    assert_eq!(
        text.matches("wire [").count(),
        1,
        "must dedup to exactly one wire: {}",
        text
    );
    assert!(
        text.contains("wire [3:0] shared;"),
        "first-seen (mod_a's) range must win: {}",
        text
    );
    // mod_b's OWN port declaration legitimately contains "[7:0] shared"
    // (that's its source text, not something AUTOWIRE generated) -- the
    // thing that must be ABSENT is a generated wire with that range.
    assert!(
        !text.contains("wire [7:0] shared"),
        "mod_b's range must not win a generated wire: {}",
        text
    );
}

#[test]
fn autowire_empty_candidate_set_inserts_nothing() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire done;\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOWIRE*/\n  sub_mod"),
        "nothing to add -- no Beginning/End markers at all: {}",
        text
    );
    assert!(!text.contains("Beginning of automatic"));
}

#[test]
fn autowire_reads_autoinst_generated_connections() {
    // A rangeless output ("done"): AUTOINST connects it as the bare
    // ".done(done)" (no `[range]` suffix, since the port itself has no
    // packed dimension -- see autoinst_full_expansion_groups_aligns_and_orders
    // for the ranged-port shape, `.count(count[WIDTH-1:0])', which is
    // deliberately NOT bare and so isn't the right shape to probe this
    // ordering requirement with).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), /*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("wire done;"), "buffer:\n{}", text);
    assert!(
        text.contains(".done"),
        "AUTOINST connections must also be present: {}",
        text
    );
    assert!(msg.contains("1 wires"), "echo: {}", msg);
}

// ===================== AUTOARG =====================

#[test]
fn autoarg_nonansi_groups_and_no_trailing_comma() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module foo (/*AUTOARG*/);\n  input clk;\n  output [7:0] q;\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    // Closing paren glued directly onto the last arg line -- no
    // trailing newline in generated text, same convention as AUTOINST/
    // AUTOWIRE (see verilog-auto.el's header).
    let expected = "/*AUTOARG*/\n    // Outputs\n    q,\n    // Inputs\n    clk);";
    assert!(
        text.contains(expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
    assert!(msg.contains("2 args"), "echo: {}", msg);
}

#[test]
fn autoarg_ansi_header_expands_empty_with_notice() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module baz (/*AUTOARG*/\n  input clk,\n  output [7:0] q\n);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOARG*/\n  input clk,"),
        "must expand to nothing: {}",
        text
    );
    assert!(msg.contains("AUTOARG in ANSI header"), "echo: {}", msg);
    assert!(msg.contains("baz"), "echo should name the module: {}", msg);
}

// ===================== Idempotence, delete-auto, undo ====================

const COMBO_SRC: &str = "\
module sub_mod (
  input  logic clk,
  output logic [WIDTH-1:0] count,
  output logic done
);
endmodule

module foo (/*AUTOARG*/);
  input clk;
  output [7:0] q;
  wire clk_int;
  /*AUTOWIRE*/
  sub_mod u1 (.clk(clk), /*AUTOINST*/);
endmodule
";

#[test]
fn verilog_auto_is_idempotent() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, COMBO_SRC);
    verilog_auto(&mut i);
    let once = bs(&mut i);
    verilog_auto(&mut i);
    let twice = bs(&mut i);
    assert_eq!(once, twice, "a second verilog-auto must change nothing");
}

#[test]
fn delete_auto_restores_original_text_exactly() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, COMBO_SRC);
    verilog_auto(&mut i);
    assert_ne!(
        bs(&mut i),
        COMBO_SRC,
        "sanity: expansion actually changed the buffer"
    );
    delete_auto(&mut i);
    assert_eq!(
        bs(&mut i),
        COMBO_SRC,
        "delete-auto must restore the original byte-for-byte"
    );
}

#[test]
fn verilog_auto_is_a_single_undo_group() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, COMBO_SRC);
    run(&mut i, "(undo-boundary)");
    verilog_auto(&mut i);
    assert_ne!(bs(&mut i), COMBO_SRC);
    let r = run(&mut i, "(undo)");
    assert!(!r.starts_with("ERROR"), "undo failed: {}", r);
    assert_eq!(
        bs(&mut i),
        COMBO_SRC,
        "one undo must revert the entire expansion"
    );
}

// ===================== Library directories =====================

#[test]
fn library_directory_resolves_submodule_definition() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("lib");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sub_mod.v"),
        "module sub_mod (\n  input  clk,\n  output [WIDTH-1:0] count\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  wire [WIDTH-1:0] count;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".count"),
        "submodule found in library dir -- buffer:\n{}",
        text
    );
    assert!(text.contains(".clk"), "buffer:\n{}", text);
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

// ===================== M56: recursive library scan + verible.filelist =====

/// A fresh scratch directory under the OS temp dir, unique per test run
/// (mirrors `verilog_complete_tests.rs`'s/`verilog_nav_tests.rs`'s own
/// `scratch_dir` helper -- this file historically used a bare
/// `se_va_<tag>_<pid>` name per test instead, kept as-is above; the M56
/// tests below use this one so distinct M56 tests can never collide even
/// if this file's pid-only convention would).
fn m56_scratch_dir(tag: &str) -> Scratch {
    let p = std::env::temp_dir().join(format!(
        "reticle_verilog_auto_m56_{}_{}_{}",
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

#[test]
fn recursive_scan_finds_module_in_a_subdirectory() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("recurse");
    std::fs::create_dir_all(dir.join("core")).unwrap();
    std::fs::write(
        dir.join("core").join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".clk") && text.contains("(clk)"),
        "submodule found by recursing into core/ -- buffer:\n{}",
        text
    );
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

#[test]
fn max_depth_zero_restores_the_pre_m56_non_recursive_behavior() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("depth0");
    std::fs::create_dir_all(dir.join("core")).unwrap();
    std::fs::write(
        dir.join("core").join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(&mut i, "(setq verilog-library-max-depth 0)");
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        msg.contains("module sub_mod not found"),
        "max-depth 0 must not recurse into core/: {}",
        msg
    );
    assert!(
        text.contains("sub_mod u1 (/*AUTOINST*/);"),
        "buffer:\n{}",
        text
    );
}

#[test]
fn depth_beyond_the_configured_max_is_not_reached() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("depth_cap");
    std::fs::create_dir_all(dir.join("a").join("b")).unwrap();
    std::fs::write(
        dir.join("a").join("b").join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    // depth 1 reaches `a/' (depth 1) but not `a/b/' (depth 2), where
    // sub_mod.v actually lives.
    ok(&mut i, "(setq verilog-library-max-depth 1)");
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("module sub_mod not found"),
        "depth 2 must be out of reach with max-depth 1: {}",
        msg
    );
}

#[test]
fn dot_prefixed_subdirectories_are_never_descended_into() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("dotdir");
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    std::fs::write(
        dir.join(".hidden").join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("module sub_mod not found"),
        "`.hidden/' must never be descended into: {}",
        msg
    );
}

#[test]
fn max_files_truncates_the_candidate_list_and_messages_once() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("max_files");
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..5 {
        std::fs::write(
            dir.join(format!("m{}.v", n)),
            format!("module m{} (input clk{});\nendmodule\n", n, n),
        )
        .unwrap();
    }
    let top_path = dir.join("top.v");
    std::fs::write(&top_path, "module top;\nendmodule\n").unwrap();
    let captured: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_write = captured.clone();
    i.output = Some(Box::new(move |s| {
        captured_write.borrow_mut().push(s.to_string())
    }));
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(&mut i, "(setq verilog-library-max-files 2)");
    let count = run(&mut i, "(length (verilog-auto--library-files))");
    assert_eq!(count, "2", "collection must stop at the configured cap");
    let out = captured.borrow().join("");
    assert!(
        out.contains("verilog-library-max-files") && out.contains('2'),
        "truncation must be messaged, naming the variable and the cap: {:?}",
        out
    );
}

#[test]
fn own_file_exclusion_still_applies_to_a_file_reached_by_recursion() {
    // Same shape as `library_scan_excludes_the_buffers_own_file_ignoring_
    // unsaved_deletions' above, but the buffer's own file is reached via
    // the M56 RECURSIVE branch (library dir set to the parent, `dup_mod'
    // lives one level down in `lib/'), not the depth-0 scan.
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("own_file_recurse");
    let lib_dir = dir.join("lib");
    std::fs::create_dir_all(&lib_dir).unwrap();
    let path = lib_dir.join("dup.sv");
    std::fs::write(
        &path,
        "module dup_mod (\n  input clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  dup_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    // Buffer's own directory is `lib/'; `..' resolves to `dir', from
    // which recursion re-discovers `lib/dup.sv' on disk -- own-file
    // exclusion must still keep it out.
    ok(&mut i, "(setq-local verilog-library-directories '(\"..\"))");
    run(&mut i, "(erase-buffer)");
    insert_src(
        &mut i,
        "module top;\n  wire clk;\n  dup_mod u1 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        msg.contains("module dup_mod not found"),
        "own file, reached via recursion, must still be excluded: {}",
        msg
    );
    assert!(
        text.contains("dup_mod u1 (/*AUTOINST*/);"),
        "buffer:\n{}",
        text
    );
}

#[test]
fn same_module_name_at_two_depths_the_shallower_one_wins() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("shallow_wins");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(
        dir.join("fifo.v"),
        "module fifo (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("sub").join("fifo.v"),
        "module fifo (\n  input rst\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  fifo u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".clk") && text.contains("(clk)"),
        "depth-0 fifo.v (its own `clk' port) must win over sub/fifo.v: {}",
        text
    );
    assert!(!text.contains(".rst"), "buffer:\n{}", text);
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

#[test]
fn verible_filelist_finds_a_module_outside_library_directories() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_reach");
    let proj_dir = dir.join("proj");
    let other_dir = dir.join("other");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(
        other_dir.join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    // `verible.filelist' is itself one of `lsp--project-root-markers', so
    // dropping it in `proj/' makes `proj/' the discovered project root.
    std::fs::write(proj_dir.join("verible.filelist"), "../other/sub_mod.v\n").unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".clk") && text.contains("(clk)"),
        "sub_mod.v, unreachable by `verilog-library-directories' alone, \
         must be found via verible.filelist -- buffer:\n{}",
        text
    );
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

#[test]
fn verible_filelist_ignores_comments_blank_lines_flags_and_missing_paths() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_edge_cases");
    let proj_dir = dir.join("proj");
    let other_dir = dir.join("other");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(
        other_dir.join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        proj_dir.join("verible.filelist"),
        "# a comment\n// another comment\n+incdir+/some/dir\n-f other.f\n\n../other/missing.sv\n../other/sub_mod.v\n",
    )
    .unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !msg.starts_with("ERROR"),
        "malformed filelist lines must never signal: {}",
        msg
    );
    assert!(
        text.contains(".clk") && text.contains("(clk)"),
        "the one real, existing entry must still be found -- buffer:\n{}",
        text
    );
}

#[test]
fn use_filelist_nil_disables_filelist_based_resolution() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_disabled");
    let proj_dir = dir.join("proj");
    let other_dir = dir.join("other");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    std::fs::write(
        other_dir.join("sub_mod.v"),
        "module sub_mod (\n  input clk\n);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(proj_dir.join("verible.filelist"), "../other/sub_mod.v\n").unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(&mut i, "(setq verilog-library-use-filelist nil)");
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("module sub_mod not found"),
        "with the feature off, the same project must NOT resolve sub_mod: {}",
        msg
    );
}

// M56 review follow-up: the tests above pin BEHAVIOR (which module wins)
// but every tree they use is a single straight line of subdirectories --
// a shape where true level-order BFS and "own files first, then recurse
// into each subdir immediately" DFS produce byte-identical file lists.
// The tests below use an ASYMMETRIC, multi-branch tree specifically so
// BFS and DFS disagree, and assert the ORDER `verilog-auto--library-
// files' itself returns things in (not just which module wins an
// AUTOINST) via `library_files_index_of'.

#[test]
fn library_files_pins_true_level_order_not_dfs() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("level_order");
    // `a/x/deep_mod.v' is three directories down `a's own branch;
    // `b/shallow_mod.v' is one directory down `b's own branch. `a' sorts
    // before `b' in `directory-files' order, so a DFS implementation
    // (recurse into `a', fully exhaust its own subtree, THEN move on to
    // `b') would emit deep_mod.v BEFORE shallow_mod.v -- the opposite of
    // what true breadth-first (every depth-1 file/dir before ANY
    // depth-2 one) must produce.
    std::fs::create_dir_all(dir.join("a").join("x")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let deep_path = dir.join("a").join("x").join("deep_mod.v");
    let shallow_path = dir.join("b").join("shallow_mod.v");
    std::fs::write(&deep_path, "module deep_mod (input clk);\nendmodule\n").unwrap();
    std::fs::write(
        &shallow_path,
        "module shallow_mod (input clk);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(&top_path, "module top;\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let deep_idx = library_files_index_of(&mut i, deep_path.to_str().unwrap());
    let shallow_idx = library_files_index_of(&mut i, shallow_path.to_str().unwrap());
    assert!(
        shallow_idx < deep_idx,
        "shallow_mod.v (depth 1, under b/) must come before deep_mod.v \
         (depth 2, under a/x/) in true BFS order -- got shallow_idx={}, \
         deep_idx={}",
        shallow_idx,
        deep_idx
    );
}

#[test]
fn multiple_library_directories_first_directorys_whole_tree_wins() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("multi_dir_order");
    let top_dir = dir.join("top_buf");
    let dir1 = dir.join("dir1");
    let dir2 = dir.join("dir2");
    std::fs::create_dir_all(&top_dir).unwrap();
    std::fs::create_dir_all(dir1.join("sub")).unwrap();
    std::fs::create_dir_all(&dir2).unwrap();
    // Same module name in both: dir1's copy lives DEEP (depth 1 inside
    // dir1), dir2's copy lives SHALLOW (depth 0 inside dir2). Directory-
    // list order (dir1 listed first) must still win over depth: dir1's
    // own whole subtree is scanned to exhaustion before dir2's tree
    // starts at all.
    std::fs::write(
        dir1.join("sub").join("target_mod.v"),
        "module target_mod (input from_dir1);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir2.join("target_mod.v"),
        "module target_mod (input from_dir2);\nendmodule\n",
    )
    .unwrap();
    let top_path = top_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire from_dir1;\n  wire from_dir2;\n  target_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(
        &mut i,
        &format!(
            "(setq-local verilog-library-directories (list {:?} {:?}))",
            dir1.to_str().unwrap(),
            dir2.to_str().unwrap()
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".from_dir1") && text.contains("(from_dir1)"),
        "dir1's own (deeper) target_mod.v must win -- directory-list \
         order is the outermost sort key: {}",
        text
    );
    assert!(!text.contains(".from_dir2"), "buffer:\n{}", text);
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

#[test]
fn directory_scan_wins_over_verible_filelist_for_the_same_module_name() {
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("dirscan_beats_filelist");
    let proj_dir = dir.join("proj");
    let other_dir = dir.join("other");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    // Same module name reachable BOTH by the ordinary directory scan
    // (proj/shared_mod.v, depth 0 under the buffer's own directory) AND
    // by verible.filelist (pointing at other/shared_mod.v). The
    // directory-scan copy must win -- filelist only ever supplements
    // what the scan can't already reach.
    std::fs::write(
        proj_dir.join("shared_mod.v"),
        "module shared_mod (input from_dirscan);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        other_dir.join("shared_mod.v"),
        "module shared_mod (input from_filelist);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(proj_dir.join("verible.filelist"), "../other/shared_mod.v\n").unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire from_dirscan;\n  wire from_filelist;\n  shared_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".from_dirscan") && text.contains("(from_dirscan)"),
        "the directory-scan copy must win over the filelist's own: {}",
        text
    );
    assert!(!text.contains(".from_filelist"), "buffer:\n{}", text);
    assert!(msg.contains("1 inst"), "echo: {}", msg);
}

#[test]
fn verible_filelist_minus_and_plus_prefixed_lines_are_skipped_whole() {
    // Unlike the existing `verible_filelist_ignores_comments_blank_
    // lines_flags_and_missing_paths' test (whose skipped lines happen
    // to point nowhere real, so removing the skip guard wouldn't change
    // its outcome), the two paths below are constructed so that
    // removing the `+'/`-' guard WOULD make each one resolve to a real,
    // existing, module-defining file: the leading `-'/`+' character is
    // itself part of the real on-disk directory name, not a separate
    // flag token, so the guard is the only thing standing between the
    // unmodified line text and a real `file-exists-p' hit.
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_flag_prefixes");
    let proj_dir = dir.join("proj");
    let dash_dir = proj_dir.join("-dashdir");
    let plus_dir = proj_dir.join("+plusdir");
    std::fs::create_dir_all(&dash_dir).unwrap();
    std::fs::create_dir_all(&plus_dir).unwrap();
    std::fs::write(
        dash_dir.join("real_mod.v"),
        "module real_mod (input clk);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        plus_dir.join("plus_mod.v"),
        "module plus_mod (input clk);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        proj_dir.join("verible.filelist"),
        "-dashdir/real_mod.v\n+plusdir/plus_mod.v\n",
    )
    .unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  real_mod u1 (/*AUTOINST*/);\n  plus_mod u2 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    // `verilog-library-directories' defaults to `("."), which would
    // recurse into `-dashdir'/`+plusdir' on its OWN (neither name starts
    // with `.', so the M56 dot-skip rule doesn't touch them) and find
    // both files regardless of the filelist guard under test. Disabling
    // the directory scan entirely isolates the filelist parsing path.
    ok(&mut i, "(setq-local verilog-library-directories nil)");
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    // `verilog-auto''s own echo policy (see its header) folds multiple
    // distinct missing modules into ONE message -- first name plus a
    // total count, not every name -- so check the total count and, more
    // directly, that BOTH instantiation sites are still bare (neither
    // module resolved) rather than relying on both names appearing in
    // the text.
    assert!(
        msg.contains("not found (2 total)"),
        "both real_mod and plus_mod must be missing -- neither line's \
         `-'/`+' prefix may be treated as part of a resolvable path: {}",
        msg
    );
    assert!(
        text.contains("real_mod u1 (/*AUTOINST*/);"),
        "u1's site (real_mod, named on a `-'-prefixed line) must stay \
         unexpanded: {}",
        text
    );
    assert!(
        text.contains("plus_mod u2 (/*AUTOINST*/);"),
        "u2's site (plus_mod, named on a `+'-prefixed line) must stay \
         unexpanded: {}",
        text
    );
}

#[test]
fn verible_filelist_hash_and_slash_comment_lines_are_skipped_whole() {
    // Companion to the `-'/`+' test above, added after a mutation run
    // showed the `#'/`//' guards had NO coverage at all: every fixture
    // that exercised them used comment text pointing nowhere real, so
    // deleting the guards changed nothing observable.
    //
    // The `//' case looked unobservable BY CONSTRUCTION at first glance
    // ("a `//'-prefixed line is an absolute path, so it can never name
    // a file under the fixture's own temp root") -- that reasoning is
    // wrong. `expand-file-name' routes through
    // `crate::complete::expand_file_input' FIRST
    // (`crates/core/src/complete.rs:251-254'), which implements GNU's
    // `//' shadowing rule: everything up to and including the last
    // `//' is discarded, so `//var/folders/.../slash_mod.v' becomes the
    // real, existing `/var/folders/.../slash_mod.v'. Doubling the
    // leading slash of a genuine absolute path is therefore a
    // resolvable line, and the guard is the only thing rejecting it.
    // (The later empty-component filter at `files.rs:384-394' would
    // produce the same collapse on its own, which is what an earlier
    // draft of this comment credited -- but the shadowing rule gets
    // there first, so that attribution was wrong.)
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_comment_prefixes");
    let proj_dir = dir.join("proj");
    let hash_dir = proj_dir.join("#hashdir");
    let slash_dir = proj_dir.join("slashdir");
    std::fs::create_dir_all(&hash_dir).unwrap();
    std::fs::create_dir_all(&slash_dir).unwrap();
    std::fs::write(
        hash_dir.join("hash_mod.v"),
        "module hash_mod (input clk);\nendmodule\n",
    )
    .unwrap();
    let slash_mod_path = slash_dir.join("slash_mod.v");
    std::fs::write(
        &slash_mod_path,
        "module slash_mod (input clk);\nendmodule\n",
    )
    .unwrap();
    // `#hashdir/hash_mod.v' is root-relative and real; the `//' line is
    // the real absolute path with its leading slash doubled.
    std::fs::write(
        proj_dir.join("verible.filelist"),
        format!(
            "#hashdir/hash_mod.v\n/{}\n",
            slash_mod_path.to_str().unwrap()
        ),
    )
    .unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top;\n  wire clk;\n  hash_mod u1 (/*AUTOINST*/);\n  slash_mod u2 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    // Same isolation reason as the `-'/`+' test: the default `(".")'
    // directory scan would recurse into both subdirectories on its own
    // and find each module regardless of the filelist guard under test.
    ok(&mut i, "(setq-local verilog-library-directories nil)");
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        msg.contains("not found (2 total)"),
        "both hash_mod and slash_mod must be missing -- neither line's \
         `#'/`//' prefix may be treated as part of a resolvable path: {}",
        msg
    );
    assert!(
        text.contains("hash_mod u1 (/*AUTOINST*/);"),
        "u1's site (hash_mod, named on a `#'-prefixed line) must stay \
         unexpanded: {}",
        text
    );
    assert!(
        text.contains("slash_mod u2 (/*AUTOINST*/);"),
        "u2's site (slash_mod, named on a `//'-prefixed line) must stay \
         unexpanded: {}",
        text
    );
}

#[test]
fn verible_filelist_own_file_exclusion_applies_even_when_only_reachable_via_filelist() {
    // Buffer visits proj/top.v; `verilog-library-directories' is set to
    // nil so the ordinary directory scan can never reach top.v itself --
    // the ONLY route by which top.v could end up as a library candidate
    // is via verible.filelist naming it directly. On disk, top.v still
    // defines `self_mod'; the BUFFER's own (unsaved) content has had
    // that definition deleted, mirroring `library_scan_excludes_the_
    // buffers_own_file_ignoring_unsaved_deletions''s own shape for the
    // filelist source specifically.
    let (mut i, _ed) = setup();
    let dir = m56_scratch_dir("filelist_own_file");
    let proj_dir = dir.join("proj");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let top_path = proj_dir.join("top.v");
    std::fs::write(
        &top_path,
        "module self_mod (\n  input clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  self_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(proj_dir.join("verible.filelist"), "top.v\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(&mut i, "(setq-local verilog-library-directories nil)");
    run(&mut i, "(erase-buffer)");
    insert_src(
        &mut i,
        "module top;\n  wire clk;\n  self_mod u1 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        msg.contains("module self_mod not found"),
        "own file must be excluded even when filelist is the ONLY route \
         to it: {}",
        msg
    );
    assert!(
        text.contains("self_mod u1 (/*AUTOINST*/);"),
        "buffer:\n{}",
        text
    );
}

// ===================== Missing module: warn and continue =====================

#[test]
fn missing_module_warns_skips_that_instance_other_autos_proceed() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  missing_mod u1 (/*AUTOINST*/);\n  /*AUTOWIRE*/\n  sub_mod u2 (.clk(clk), .done(done));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        msg.contains("module missing_mod not found"),
        "echo: {}",
        msg
    );
    // u1's site: unresolved, nothing inserted.
    assert!(
        text.contains("missing_mod u1 (/*AUTOINST*/);"),
        "buffer:\n{}",
        text
    );
    // u2's AUTOWIRE still ran normally.
    assert!(text.contains("Beginning of automatic"), "buffer:\n{}", text);
}

// ===================== M39 review fixes =====================
//
// Four tests below pin down the reviewer's findings on the initial
// M39 implementation: a data-corruption-severity AUTOWIRE delete bug
// (two sub-fixes plus a policy change, all three exercised together),
// an order-dependent parameter substitution bug, and a stale-disk-
// content-over-unsaved-edits bug. See verilog-auto.el's own header
// ("Policy choices worth documenting explicitly") for the full
// reasoning behind each fix.

const BUG1_STALE_END_SRC: &str = "\
module top;
  /*AUTOWIRE*/
  // Beginning of automatic wires (for undeclared instantiated-module outputs)
  wire a;
  sub_mod u1 (.clk(clk));
  /*AUTOWIRE*/
  // Beginning of automatic wires (for undeclared instantiated-module outputs)
  wire b;
  // End of automatics
  sub_mod u2 (.clk(clk));
endmodule

module canary;
  wire untouched;
endmodule
";

#[test]
fn delete_auto_stale_end_boundary_prevents_cross_site_corruption() {
    // Hand-constructed (never produced by verilog-auto itself post-fix,
    // since only the first AUTOWIRE per module ever expands -- but a
    // buffer can still reach this shape by hand-editing or from an
    // older file): two AUTOWIRE sites in one module, both once
    // expanded, with the FIRST site's own "// End of automatics" line
    // deleted by hand. Without the stale-end boundary check, the first
    // site's forward scan would misattribute the SECOND site's End
    // marker to itself, producing two overlapping delete ranges and
    // (per the position-clamping every edit already goes through)
    // deleting everything from the first comment onward, including the
    // unrelated `canary' module below -- silently, with no warning.
    let (mut i, _ed) = setup();
    insert_src(&mut i, BUG1_STALE_END_SRC);
    delete_auto(&mut i);
    let text = bs(&mut i);
    // First site: conservative -- its own missing End means nothing is
    // deleted there at all, stale Beginning/wire lines included.
    assert!(
        text.contains(
            "/*AUTOWIRE*/\n  // Beginning of automatic wires (for undeclared instantiated-module outputs)\n  wire a;\n  sub_mod u1"
        ),
        "first site (missing its own End) must be left untouched: {}",
        text
    );
    // Second site: cleanly bounded by its own End marker, correctly removed.
    assert!(
        text.contains("/*AUTOWIRE*/\n  sub_mod u2 (.clk(clk));"),
        "second site's own complete block must be removed: {}",
        text
    );
    // The actual data corruption this bug produced deleted straight
    // through to the buffer's end -- the canary module must survive
    // completely intact.
    assert!(
        text.contains("module canary;\n  wire untouched;\nendmodule\n"),
        "canary module must be completely untouched: {}",
        text
    );
}

#[test]
fn autowire_only_first_site_per_module_expands() {
    // GNU convention (and this file's own policy, post-fix): one
    // module, one AUTOWIRE. A second comment in the same module must
    // stay bare -- expanding both used to duplicate every wire
    // declaration once per comment (a real Verilog error). The single
    // block that DOES expand (at the first comment) must still cover
    // the WHOLE module's instantiations, not just whatever precedes
    // it -- `done1' (from u1) and `done2' (from u2) both need a wire.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), .done(done1));\n  /*AUTOWIRE*/\n  sub_mod u2 (.clk(clk), .done(done2));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("Beginning of automatic").count(),
        1,
        "only one Beginning/End block, however many AUTOWIRE comments: {}",
        text
    );
    assert!(text.contains("wire done1;"), "buffer:\n{}", text);
    assert!(text.contains("wire done2;"), "buffer:\n{}", text);
    assert!(
        text.contains("/*AUTOWIRE*/\n  sub_mod u2"),
        "second comment must stay bare, no competing block: {}",
        text
    );
    assert!(
        msg.contains("multiple /*AUTOWIRE*/"),
        "echo must mention the policy: {}",
        msg
    );
    assert!(msg.contains("top"), "echo should name the module: {}", msg);
}

#[test]
fn autoinst_param_override_chaining_is_order_independent() {
    // `#(.WIDTH(DEPTH), .DEPTH(4))': WIDTH's own override value is
    // itself another parameter's bare name. A naive per-override,
    // sequential pass over the ACCUMULATING result would apply WIDTH's
    // rule first (producing "(DEPTH)"), then rescan that OUTPUT for
    // DEPTH's own rule and wrongly re-substitute it to "((4))" -- with
    // the result depending on override list order. The fix (a single
    // pass over the ORIGINAL text) must produce the same, correct
    // result regardless of that order.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [WIDTH-1:0] count,\n  output logic [DEPTH-1:0] count2\n);\nendmodule\n\nmodule top;\n  wire [WIDTH-1:0] count;\n  wire [DEPTH-1:0] count2;\n  sub_mod #(.WIDTH(DEPTH), .DEPTH(4)) u1 (/*AUTOINST*/);\n  sub_mod #(.DEPTH(4), .WIDTH(DEPTH)) u2 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("count[(DEPTH)-1:0]").count(),
        2,
        "both instances (either override order) substitute WIDTH -> (DEPTH), unchained: {}",
        text
    );
    assert_eq!(
        text.matches("count2[(4)-1:0]").count(),
        2,
        "both instances substitute count2's own DEPTH -> (4): {}",
        text
    );
    assert!(
        !text.contains("((4))"),
        "WIDTH's own substitution must never be rescanned by DEPTH's rule: {}",
        text
    );
}

#[test]
fn library_scan_excludes_the_buffers_own_file_ignoring_unsaved_deletions() {
    // The buffer visits a file that, ON DISK, still defines sub_mod;
    // the user deletes that definition IN THE BUFFER (unsaved).
    // `verilog-library-directories' defaults to the buffer's own
    // directory, so AUTOINST must not silently fall back to
    // re-reading the STALE on-disk copy and report success -- the
    // buffer's own in-memory content is the sole authority for what it
    // defines.
    let (mut i, _ed) = setup();
    let dir = Scratch::new("stale");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.v");
    std::fs::write(
        &path,
        "module sub_mod (\n  input clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);
    // Simulate an unsaved edit: delete sub_mod's own definition from
    // the BUFFER only -- the file on disk still has it verbatim.
    run(&mut i, "(erase-buffer)");
    insert_src(
        &mut i,
        "module top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(msg.contains("module sub_mod not found"), "echo: {}", msg);
    assert!(
        text.contains("sub_mod u1 (/*AUTOINST*/);"),
        "must not expand using the stale on-disk definition: {}",
        text
    );
}

// ===================== Keybindings under evil =====================

#[test]
fn evil_normal_state_c_c_a_triggers_verilog_auto() {
    let (mut i, ed) = setup();
    insert_src(&mut i, "module sub_mod (\n  input logic clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n");
    run(&mut i, "(verilog-mode)");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    assert_eq!(run(&mut i, "evil--state"), "normal");
    feed_keys(&mut i, &ed, "C-c C-a").expect("feed C-c C-a");
    let text = bs(&mut i);
    assert!(
        text.contains(".clk"),
        "C-c C-a in normal state must run verilog-auto: {}",
        text
    );
}

#[test]
fn evil_insert_state_c_c_a_triggers_verilog_auto() {
    let (mut i, ed) = setup();
    insert_src(&mut i, "module sub_mod (\n  input logic clk\n);\nendmodule\n\nmodule top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n");
    run(&mut i, "(verilog-mode)");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    feed_keys(&mut i, &ed, "i").expect("enter insert state");
    assert_eq!(run(&mut i, "evil--state"), "insert");
    feed_keys(&mut i, &ed, "C-c C-a").expect("feed C-c C-a");
    let text = bs(&mut i);
    assert!(
        text.contains(".clk"),
        "C-c C-a in insert state must run verilog-auto: {}",
        text
    );
}

#[test]
fn evil_normal_state_c_c_k_triggers_verilog_delete_auto() {
    let (mut i, ed) = setup();
    insert_src(&mut i, COMBO_SRC);
    verilog_auto(&mut i);
    assert_ne!(bs(&mut i), COMBO_SRC);
    run(&mut i, "(verilog-mode)");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    feed_keys(&mut i, &ed, "C-c C-k").expect("feed C-c C-k");
    assert_eq!(
        bs(&mut i),
        COMBO_SRC,
        "C-c C-k must run verilog-delete-auto"
    );
}

// ===================== New Rust builtins (M39) =====================

#[test]
fn treesit_parse_string_parses_without_any_buffer() {
    let (mut i, _ed) = setup();
    let r = run(
        &mut i,
        "(treesit-node-type (treesit-parse-string 'verilog \"module m; endmodule\"))",
    );
    assert_eq!(r, "\"source_file\"");
    // The buffer the test started in is untouched -- confirms this
    // really took no buffer at all, unlike treesit-parser-create.
    assert_eq!(bs(&mut i), "");
}

#[test]
fn treesit_node_child_by_field_name_builtin() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        "(setq root (treesit-parse-string 'verilog \"module foo; endmodule\"))",
    );
    // root (source_file) -> module_declaration -> module_ansi_header,
    // the node that actually carries the "name" field.
    run(&mut i, "(setq mod (treesit-node-child root 0))");
    run(&mut i, "(setq hdr (treesit-node-child mod 0))");
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-text (treesit-node-child-by-field-name hdr \"name\"))"
        ),
        "\"foo\""
    );
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-child-by-field-name hdr \"no-such-field\")"
        ),
        "nil"
    );
}

#[test]
fn directory_files_builtin_lists_plain_names() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("dirfiles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.v"), "").unwrap();
    std::fs::write(dir.join("b.txt"), "").unwrap();
    let r = run(
        &mut i,
        &format!("(directory-files {:?})", dir.to_str().unwrap()),
    );
    assert!(r.contains("a.v"), "listing: {}", r);
    assert!(r.contains("b.txt"), "listing: {}", r);
    assert!(r.contains("\".\""), "GNU default includes \".\": {}", r);
}

#[test]
fn file_contents_as_string_builtin_reads_without_a_buffer() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("readstr");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.v");
    std::fs::write(&path, "module x; endmodule\n").unwrap();
    let before = run(&mut i, "(length (buffer-list))");
    let r = run(
        &mut i,
        &format!("(file-contents-as-string {:?})", path.to_str().unwrap()),
    );
    assert_eq!(r, "\"module x; endmodule\\n\"");
    let after = run(&mut i, "(length (buffer-list))");
    assert_eq!(before, after, "must not create any buffer");
    let missing = dir.join("nope.v");
    let err = run(
        &mut i,
        &format!("(file-contents-as-string {:?})", missing.to_str().unwrap()),
    );
    assert!(
        err.starts_with("ERROR"),
        "missing file must signal, not panic: {}",
        err
    );
}

// ===================== Expand on save: opt-in (M40) =====================

#[test]
fn verilog_auto_on_save_defaults_off_disk_stays_unexpanded() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("onsave_off");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.v");
    std::fs::write(&path, COMBO_SRC).unwrap();

    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer failed: {}", r);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk, COMBO_SRC,
        "verilog-auto-on-save defaults to nil: save must not expand"
    );
}

#[test]
fn verilog_auto_on_save_enabled_expands_on_disk_and_stays_idempotent() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("onsave_on");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.v");
    std::fs::write(&path, COMBO_SRC).unwrap();

    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);
    run(&mut i, "(setq verilog-auto-on-save t)");

    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer failed: {}", r);
    let once = std::fs::read_to_string(&path).unwrap();
    assert_ne!(
        once, COMBO_SRC,
        "verilog-auto-on-save t: save must expand AUTOINST/AUTOWIRE/AUTOARG"
    );
    assert!(once.contains(".clk"), "expanded on disk:\n{}", once);

    // Idempotent across saves: verilog-auto itself is idempotent (see
    // verilog_auto_is_idempotent above), so a second save with nothing
    // else changed must write byte-identical content.
    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "second save-buffer failed: {}", r);
    let twice = std::fs::read_to_string(&path).unwrap();
    assert_eq!(once, twice, "a second save must change nothing on disk");
}

#[test]
fn verilog_auto_on_save_enabled_leaves_non_verilog_saves_untouched() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("onsave_other");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notes.txt");
    let src = "just some /*AUTOINST*/ text that is not verilog\n";
    std::fs::write(&path, src).unwrap();

    let r = run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    assert!(!r.starts_with("ERROR"), "find-file-internal failed: {}", r);
    run(&mut i, "(setq verilog-auto-on-save t)");

    let r = run(&mut i, "(save-buffer)");
    assert!(!r.starts_with("ERROR"), "save-buffer failed: {}", r);

    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk, src,
        "a non-verilog-mode buffer must never be run through verilog-auto"
    );
}

// ===================== M74: AUTOARG wrap width vs block step ==============
//
// M73 made `standard-indent-width' track a file's own detected indent
// style. `verilog-auto--expand-autoarg-site' used to read that same
// variable for the wrapped port-list continuation's width, so a
// 2-space-style file regressed AUTOARG's own output to a 2-column wrap --
// wrong, per verible's independent `--wrap_spaces' (default 4) vs
// `--indentation_spaces' flags (see verilog-auto.el's
// `verilog-auto-wrap-width' docstring for the full story). These tests
// use `find-file-internal' (not `insert' into a scratch buffer) because
// the regression only exists once M73's detection has actually run.

fn m74_write_and_open(interp: &mut Interp, dir: &std::path::Path, name: &str, contents: &str) {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    ok(
        interp,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
}

/// A non-ANSI Verilog-2001 module, consistently indented at INDENT
/// spaces per line (five body declarations -- `indent--detect-min-
/// samples' in indent.el is 5, so four would leave detection
/// inconclusive), with `/*AUTOARG*/' sitting in the otherwise-empty
/// port list.
fn m74_nonansi_module(indent: &str) -> String {
    format!(
        "module foo (/*AUTOARG*/);\n{i}input a;\n{i}input b;\n{i}input c;\n{i}output d;\n{i}output e;\nendmodule\n",
        i = indent
    )
}

#[test]
fn autoarg_wrap_width_independent_of_detected_2_space_block_step() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("wrap2");
    std::fs::create_dir_all(&dir).unwrap();
    m74_write_and_open(&mut i, &dir, "top.v", &m74_nonansi_module("  "));

    // Assert the premise FIRST: if detection silently didn't fire, this
    // whole test would fall back to the mode default (4) and pass for
    // the wrong reason -- exactly the false-positive shape this repo has
    // hit five milestones running (see CLAUDE.md).
    assert_eq!(
        run(&mut i, "standard-indent-width"),
        "2",
        "premise: this 2-space file must be detected as width 2 before \
         verilog-auto even runs"
    );

    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected =
        "/*AUTOARG*/\n    // Outputs\n    d,\n    e,\n    // Inputs\n    a,\n    b,\n    c);";
    assert!(
        text.contains(expected),
        "AUTOARG wrap must stay at column 4 even though block step is 2:\nexpected substring:\n{}\ngot:\n{}",
        expected,
        text
    );
    assert!(msg.contains("5 args"), "echo: {}", msg);
}

#[test]
fn autoarg_wrap_width_independent_of_detected_3_space_block_step() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("wrap3");
    std::fs::create_dir_all(&dir).unwrap();
    m74_write_and_open(&mut i, &dir, "top.v", &m74_nonansi_module("   "));

    assert_eq!(
        run(&mut i, "standard-indent-width"),
        "3",
        "premise: this 3-space file must be detected as width 3 before \
         verilog-auto even runs"
    );

    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected =
        "/*AUTOARG*/\n    // Outputs\n    d,\n    e,\n    // Inputs\n    a,\n    b,\n    c);";
    assert!(
        text.contains(expected),
        "wrap must stay at column 4, not follow the detected 3-space block step:\nexpected substring:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autoarg_wrap_width_reads_verilog_auto_wrap_width_variable() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("wrap8");
    std::fs::create_dir_all(&dir).unwrap();
    m74_write_and_open(&mut i, &dir, "top.v", &m74_nonansi_module("  "));
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
    ok(&mut i, "(setq verilog-auto-wrap-width 8)");

    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected =
        "/*AUTOARG*/\n        // Outputs\n        d,\n        e,\n        // Inputs\n        a,\n        b,\n        c);";
    assert!(
        text.contains(expected),
        "expected substring:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autoarg_wrap_width_adds_to_module_line_own_indent() {
    // The module header itself is indented two spaces (a synthetic
    // shape -- nesting is not semantically required, only that the
    // module line's OWN leading whitespace differs from the block
    // step used for its body) -- `verilog-auto--line-indent' must still
    // be honored, with `verilog-auto-wrap-width' (default 4) added on
    // top of it rather than replacing it.
    let (mut i, _ed) = setup();
    let dir = Scratch::new("wrap_moduleindent");
    std::fs::create_dir_all(&dir).unwrap();
    let src = "  module foo (/*AUTOARG*/);\n    input a;\n    input b;\n    input c;\n    output d;\n    output e;\n  endmodule\n";
    m74_write_and_open(&mut i, &dir, "top.v", src);

    assert_eq!(
        run(&mut i, "standard-indent-width"),
        "2",
        "premise: detection must still land on 2 for this file"
    );

    verilog_auto(&mut i);
    let text = bs(&mut i);
    // module's own line indent ("  ") + wrap width (4 spaces) = 6.
    let expected = "/*AUTOARG*/\n      // Outputs\n      d,\n      e,\n      // Inputs\n      a,\n      b,\n      c);";
    assert!(
        text.contains(expected),
        "expected substring:\n{}\ngot:\n{}",
        expected,
        text
    );
}
