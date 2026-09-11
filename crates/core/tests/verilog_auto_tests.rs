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
    // M104 fix round: this file saves real .v files, and M104's
    // `format-on-save' defaults to `t'. Whether that then actually
    // reformats anything depends on whether THIS MACHINE happens to
    // have `verible-verilog-format' on PATH -- exactly the kind of
    // "green or red depending on what tools are installed" a test must
    // never be. This file tests AUTOINST/AUTOWIRE/AUTOARG expansion, not
    // formatting, so it opts out unconditionally.
    let r = interp.eval_source("(setq format-on-save nil)");
    assert!(r.is_ok(), "setq format-on-save nil failed");
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

/// One AUTOOUTPUT/AUTOINPUT/AUTOINOUT declaration line, INDENT included,
/// mirroring `verilog-auto--port-decl-line`.
#[allow(clippy::too_many_arguments)]
fn port_decl(
    indent: &str,
    decl_kw: &str,
    ty: Option<&str>,
    range: Option<&str>,
    name: &str,
    verb: &str,
    inst: &str,
    module: &str,
    ellipsis: bool,
) -> String {
    let mut body = format!("{} ", decl_kw);
    if let Some(t) = ty {
        body.push_str(t);
        body.push(' ');
    }
    if let Some(r) = range {
        body.push_str(r);
        body.push(' ');
    }
    body.push_str(name);
    body.push(';');
    let comment = format!(
        "// {} {} of {}{}",
        verb,
        inst,
        module,
        if ellipsis { ", ..." } else { "" }
    );
    format!("{}{}{}", indent, pad(&body, 40, indent.len()), comment)
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

/// M97 Part 2: AUTOINST against an INTERFACE (not a module) instantiation
/// target, exercising the ANSI-header path specifically -- before this
/// milestone, `verilog-auto--ports-of-module''s literal `(string= ...
/// "module_ansi_header")' check failed on an `interface_ansi_header' and
/// silently took the NON-ANSI branch instead, which would have produced
/// wrong (here, empty) results rather than signaling an error. Real-parse
/// dump-verified (M97 recon): an interface's own ANSI header port list
/// (`interface axi_if (input logic clk, ...);') has the identical
/// `list_of_port_declarations'/`ansi_port_declaration' shape a module's
/// does.
#[test]
fn autoinst_against_an_interface_ansi_header_target() {
    let (mut i, _ed) = setup();
    let inst_line = "  axi_if u_if (/*AUTOINST*/);\n";
    let indent = " ".repeat("  axi_if u_if (".len());
    insert_src(
        &mut i,
        &format!(
            "interface axi_if (\n  input  logic clk,\n  input  logic rst_n,\n  output logic valid\n);\nendinterface\n\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire valid;\n{}endmodule\n",
            inst_line
        ),
    );
    verilog_auto(&mut i);
    let expected_block = format!(
        "/*AUTOINST*/\n{indent}// Outputs\n{c1},\n{indent}// Inputs\n{c2},\n{c3}",
        indent = indent,
        c1 = conn(&indent, "valid", "valid"),
        c2 = conn(&indent, "clk", "clk"),
        c3 = conn(&indent, "rst_n", "rst_n"),
    );
    let text = bs(&mut i);
    assert!(
        text.contains(&expected_block),
        "expected block:\n{}\n\ngot buffer:\n{}",
        expected_block,
        text
    );
}

/// M97 fix round (FF3): `verilog-auto--top-level-modules''s own docstring
/// claims document order is preserved across BOTH node kinds by a single
/// tree walk (`verilog-auto--find-all-of-types'), not two separate
/// single-type walks concatenated. A buffer of only-modules or only-
/// interfaces can't distinguish the two implementations (their outputs
/// are identical either way) -- only an INTERLEAVED mix (module,
/// interface, module, interface) tells them apart: a single walk yields
/// them in that same interleaved order, while two concatenated walks
/// would group all modules first, then all interfaces
/// (`m_a', `m_b', `i_a', `i_b'), a different order entirely.
#[test]
fn top_level_modules_preserves_document_order_across_interleaved_modules_and_interfaces() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module m_a;\nendmodule\n\ninterface i_a;\nendinterface\n\nmodule m_b;\nendmodule\n\ninterface i_b;\nendinterface\n",
    );
    let names = ok(
        &mut i,
        "(mapcar #'verilog-auto--module-name (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer)))",
    );
    assert_eq!(
        names, "(\"m_a\" \"i_a\" \"m_b\" \"i_b\")",
        "single-walk document order, not module-kind-then-interface-kind: {}",
        names
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

/// M97 fix round (FF1): `/*AUTOWIRE*/' directly inside an INTERFACE body
/// used to crash `verilog-auto' outright (`Wrong type argument:
/// treesit-node-p, nil') -- `verilog-auto--enclosing-of-type' only ever
/// matched `module_declaration', so the enclosing-declaration lookup came
/// back nil for an interface and every downstream helper
/// (`verilog-auto--declared-names', `--find-all-of-type') called
/// `treesit-node-child-count' on that nil unchecked. An interface with no
/// instantiations of its own has no output-port candidates for AUTOWIRE
/// to find, so the correct behavior is a clean no-op, not a crash.
#[test]
fn autowire_inside_an_interface_is_a_clean_no_op_not_a_crash() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, "interface foo;\n  /*AUTOWIRE*/\nendinterface\n");
    let msg = verilog_auto(&mut i);
    assert!(msg.contains("0 wires"), "echo: {}", msg);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOWIRE*/\nendinterface"),
        "nothing to add -- no Beginning/End markers at all: {}",
        text
    );
    assert!(!text.contains("Beginning of automatic"));
}

/// M97 fix round (FF1): blast-radius check for the crash above, in a
/// buffer that mixes ordinary modules with a crashing interface.
/// `verilog-auto--expand-all-autowire' processes AUTOWIRE sites in
/// REVERSE document order (`sort' with `>' on each site's own start
/// position -- see that function's own docstring for why: one module,
/// one AUTOWIRE, GNU convention), so before this fix, a crash on the
/// INTERFACE's comment (`foo', positioned between `top' and `bottom')
/// stopped the `dolist' partway through: `bottom' (processed first,
/// since it is the LAST site in the buffer) got its AUTOWIRE fully
/// expanded, while `top' (processed after the crash point) was left with
/// its own AUTOWIRE comment bare and unexpanded -- a partial expansion,
/// not a clean all-or-nothing failure. AUTOINST is a separate, EARLIER
/// pass (unaffected by the AUTOWIRE crash) and had already fully expanded
/// both instantiations by the time AUTOWIRE ran. Pinned here as the fixed
/// behavior: every real module's AUTOWIRE now expands, and the interface
/// site stays a clean no-op, with no partial-expansion asymmetry.
#[test]
fn autowire_crash_in_one_interface_does_not_leave_other_sites_partially_expanded() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire clk;\n  sub_mod u1 (/*AUTOINST*/);\n  /*AUTOWIRE*/\nendmodule\n\ninterface foo;\n  /*AUTOWIRE*/\nendinterface\n\nmodule bottom;\n  wire clk;\n  sub_mod u2 (/*AUTOINST*/);\n  /*AUTOWIRE*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // Both real modules' AUTOWIRE sites expanded (each declares its own
    // "wire done;" for the AUTOINST-connected but undeclared output).
    assert_eq!(
        text.matches("wire done;").count(),
        2,
        "both `top' and `bottom' must get their own AUTOWIRE expansion, \
         with no partial-expansion gap left by the interface's crash: {}",
        text
    );
    // The interface's own site stays untouched -- correct no-op, not a
    // second, differently-broken failure mode.
    assert!(
        text.contains("/*AUTOWIRE*/\nendinterface"),
        "interface AUTOWIRE site must be a clean no-op: {}",
        text
    );
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

// ===================== AUTO_TEMPLATE (M92) =====================
//
// GNU verilog-mode semantics (verilog-mode.el 30.2, read directly rather
// than guessed -- see crates/core/lisp/verilog-auto.el's own AUTO_TEMPLATE
// section header): an exact `.NAME (EXPR)' rule always wins over a
// wildcard rule for the same port; a wildcard's LHS is `^...$'-anchored
// against the WHOLE port name, and its EXPR's own `\N' backreferences
// come from that match. Template lookup searches backward from the
// instantiation first, falling back forward.

const SUB_MOD_TPL: &str = "\
module sub_mod (
  input  logic clk,
  input  logic rst_n,
  output logic done
);
endmodule

";

#[test]
fn autotemplate_exact_rule_replaces_identity_connection() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".done"),
        "done port must still be connected: {}",
        text
    );
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "exact rule must replace done's identity connection with `finished': {}",
        text
    );
    assert!(
        !text.contains(&conn(&indent, "done", "done")),
        "identity connection for done must not appear once a template rule applies: {}",
        text
    );
}

#[test]
fn autotemplate_wildcard_rule_with_backreference() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1),\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    for (name, expr) in [
        ("clk", "my_clk"),
        ("rst_n", "my_rst_n"),
        ("done", "my_done"),
    ] {
        assert!(
            text.contains(&conn(&indent, name, expr)),
            "wildcard rule must connect {} to {}: {}",
            name,
            expr,
            text
        );
    }
}

#[test]
fn autotemplate_exact_wins_over_wildcard_for_same_port() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  .\\(.*\\) (my_\\1),\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "exact rule must win over the wildcard for the same port `done': {}",
        text
    );
    assert!(
        !text.contains(&conn(&indent, "done", "my_done")),
        "wildcard's own expansion for done must never appear once the exact rule wins: {}",
        text
    );
    assert!(
        text.contains(&conn(&indent, "clk", "my_clk")),
        "wildcard rule still applies to ports the exact rule doesn't name: {}",
        text
    );
}

#[test]
fn autotemplate_no_matching_template_falls_back_to_identity() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* some_other_module AUTO_TEMPLATE (\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "a template for a DIFFERENT module must not apply -- done falls back to identity: {}",
        text
    );
}

#[test]
fn autotemplate_rule_naming_an_already_hand_connected_port_stays_excluded() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire done_by_hand;\n  sub_mod u1 (.done(done_by_hand), /*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches(".done(").count(),
        1,
        "done must not be regenerated by the template once hand-connected: {}",
        text
    );
    assert!(
        text.contains(".done(done_by_hand)"),
        "the user's own hand connection must survive untouched: {}",
        text
    );
    // "(finished)" legitimately appears once, inside the AUTO_TEMPLATE
    // comment itself (`.done (finished),') -- what must never appear is
    // a GENERATED, padded connection line using it.
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        !text.contains(&conn(&indent, "done", "finished")),
        "the template's own EXPR for done must never appear as a generated connection -- the port is already excluded: {}",
        text
    );
}

#[test]
fn autotemplate_running_verilog_auto_twice_is_byte_identical() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1),\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let after_first = bs(&mut i);
    verilog_auto(&mut i);
    let after_second = bs(&mut i);
    assert_eq!(
        after_first, after_second,
        "verilog-auto must be idempotent with a template in play"
    );
}

#[test]
fn autotemplate_comment_survives_delete_auto() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    assert!(bs(&mut i).contains("AUTO_TEMPLATE"));
    delete_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("AUTO_TEMPLATE"),
        "verilog-delete-auto must never delete the template comment itself: {}",
        text
    );
    // "(finished)" legitimately survives once inside the AUTO_TEMPLATE
    // comment itself -- what must be gone is the GENERATED, padded
    // connection line that used it.
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        !text.contains(&conn(&indent, "done", "finished")),
        "delete-auto must still strip the generated connection text: {}",
        text
    );
}

#[test]
fn autotemplate_bare_identifier_becomes_autowire_candidate() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (my_done_wire),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  sub_mod u1 (/*AUTOINST*/);\n  /*AUTOWIRE*/\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("wire my_done_wire;"),
        "AUTOWIRE must rescan AUTOINST's own templated output as a bare-identifier candidate: {}",
        text
    );
}

#[test]
fn autotemplate_backward_lookup_wins_when_templates_appear_both_sides() {
    let (mut i, _ed) = setup();
    // A template BEFORE the instantiation applies `finished'; a template
    // AFTER it (same module name) applies a different EXPR -- the
    // backward (preceding) one must win.
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  wire done_after;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n\n/* sub_mod AUTO_TEMPLATE (\n  .done (done_after),\n  ); */\n",
            SUB_MOD_TPL
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "the PRECEDING template must win when both a preceding and a following one match: {}",
        text
    );
    assert!(
        !text.contains(&conn(&indent, "done", "done_after")),
        "the following template's own EXPR must not be used when a preceding one exists: {}",
        text
    );
}

#[test]
fn autowire_multi_instance_comma_form_sees_every_instance() {
    // `sometype u1(...), u2(...);' -- one module_instantiation, two
    // hierarchical_instance children. AUTOWIRE must consider every
    // instance's own output connections, not just the first.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module mod_b (\n  output logic ready\n);\nendmodule\n\nmodule top;\n  mod_b u1 (.ready(ready1)), u2 (.ready(ready2));\n  /*AUTOWIRE*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("wire ready1;"),
        "the FIRST instance's own output must become an AUTOWIRE candidate: {}",
        text
    );
    assert!(
        text.contains("wire ready2;"),
        "the SECOND instance's own output must also become an AUTOWIRE candidate -- this is the M92 fix itself: {}",
        text
    );
}

#[test]
fn autotemplate_demo_shape_axi4_lite_arbiter_naming_convention() {
    // Mirrors demo/rtl/top/soc_top.sv's own unexpanded
    // `axi4_lite_arbiter' instance (see this milestone's own spec): the
    // submodule's `req_*'/`gnt_*_o' ports need renaming through a
    // wildcard template to connect to a differently-named parent scope,
    // exactly the shape identity AUTOINST cannot handle. Built as a
    // fixture here -- demo/ itself is left untouched per this
    // milestone's own scope.
    let (mut i, _ed) = setup();
    let arbiter = "\
module axi4_lite_arbiter (
  input  logic req_valid_i,
  output logic req_ready_o,
  input  logic [3:0] req_i,
  output logic gnt_valid_o,
  input  logic gnt_ready_i,
  output logic [3:0] gnt_req_o,
  output logic [1:0] gnt_idx_o
);
endmodule

";
    let top = "\
/* axi4_lite_arbiter AUTO_TEMPLATE (
  .req_\\(.*\\)  (host_req_\\1),
  .gnt_\\(.*\\)_o (gnt_\\1),
  ); */
module soc_top;
  wire host_req_valid_i;
  wire host_req_ready_o;
  wire [3:0] host_req_i;
  wire gnt_valid;
  wire gnt_ready_i;
  wire [3:0] gnt_req;
  wire [1:0] gnt_idx;
  axi4_lite_arbiter u_arb (/*AUTOINST*/);
endmodule
";
    insert_src(&mut i, &format!("{}{}", arbiter, top));
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  axi4_lite_arbiter u_arb (".len());
    for (port, expr) in [
        ("req_valid_i", "host_req_valid_i"),
        ("req_ready_o", "host_req_ready_o"),
        ("req_i", "host_req_i"),
        ("gnt_valid_o", "gnt_valid"),
        ("gnt_ready_i", "gnt_ready_i"),
        ("gnt_req_o", "gnt_req"),
        ("gnt_idx_o", "gnt_idx"),
    ] {
        assert!(
            text.contains(&conn(&indent, port, expr)),
            "port {} must connect to {}: {}",
            port,
            expr,
            text
        );
    }
}

// ===================== AUTO_TEMPLATE fix round (M92 S1/S2) =====================

#[test]
fn autotemplate_exact_rule_with_trailing_line_comment_still_applies() {
    // GNU's own point-based scanner has an explicit clause for a `//'
    // end-of-line comment following a rule (`verilog-mode.el' :10184-
    // :10267) -- a whole-line-anchored regex without the same tolerance
    // would silently drop the rule instead.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished), // primary output\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "an exact rule followed by a trailing `//' comment must still apply: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a well-formed rule with its own trailing comment must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_wildcard_rule_with_trailing_line_comment_still_applies() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1), // rename every port\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "my_done")),
        "a wildcard rule followed by a trailing `//' comment must still apply: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a well-formed wildcard rule with its own trailing comment must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_malformed_rule_line_is_reported_not_silently_dropped() {
    // A line that matches neither the exact nor the wildcard shape must
    // be recorded and surfaced in `verilog-auto''s own final message --
    // never silently discarded (this milestone's whole point is that a
    // dropped rule must never be invisible).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  totally not a rule\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    // The one well-formed rule alongside the malformed line must still
    // apply.
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "a well-formed rule elsewhere in the same template must still apply despite a malformed sibling line: {}",
        text
    );
    assert!(
        msg.contains("not recognized"),
        "the malformed line must be reported in verilog-auto's own final message: {}",
        msg
    );
}

#[test]
fn autotemplate_standalone_comment_line_inside_body_is_skipped_without_warning() {
    // A `//'-prefixed line entirely on its own (not trailing a rule) is
    // tolerated -- must not be reported as a malformed/dropped rule, and
    // must not interfere with the rules around it.
    //
    // M92 fix round X3 (recorded, not chased): this test's own fixture
    // never reaches `verilog-auto--template-rule-head-re' or
    // `verilog-auto--template-rule-at' at all -- the `//'-prefix check
    // in `verilog-auto--parse-template-body' filters the line out
    // BEFORE either is consulted. It is a real regression guard for
    // that one skip, but provides no coverage of the X1 balance-scan
    // parser or the U2/X1 warning path despite living in this same
    // block of tests.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  // legacy mapping, keep for reference\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "finished")),
        "a rule following a standalone comment line must still apply: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a standalone `//' comment line inside the template body must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_wildcard_anchoring_prevents_substring_match() {
    // The wildcard LHS is anchored `^...$' against the WHOLE port name
    // (GNU's own behavior, `verilog-mode.el' :10232). Without that
    // anchoring, a pattern that's semantically just the literal `clk'
    // (written with a capture group to force the wildcard branch, since
    // a bare identifier alone is always parsed as an EXACT rule) would
    // match `clk' as a SUBSTRING inside an unrelated port like
    // `sysclk_i' too.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input  logic clk,\n  input  logic sysclk_i\n);\nendmodule\n\n/* sub_mod AUTO_TEMPLATE (\n  .\\(clk\\) (buf_\\1),\n  ); */\nmodule top;\n  wire buf_clk;\n  wire sysclk_i;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&conn(&indent, "clk", "buf_clk")),
        "the port literally named `clk' must match the anchored pattern: {}",
        text
    );
    assert!(
        text.contains(&conn(&indent, "sysclk_i", "sysclk_i")),
        "`sysclk_i' merely CONTAINS `clk' as a substring -- with `^...$' anchoring it must stay an identity connection, not `buf_clk': {}",
        text
    );
}

// ===================== AUTO_TEMPLATE fix round 2 (M92 X1/X2/X3) =====================

#[test]
fn autotemplate_exact_trailing_comment_containing_a_paren_does_not_corrupt_expr() {
    // M92 fix round X1: `.done (finished), // see (note)' used to
    // backtrack the greedy `(.*)' capture all the way to the `)' INSIDE
    // the comment, corrupting EXPR into `finished), // see (note'. A
    // real parenthetical remark in an RTL comment (`// width = WIDTH
    // (see spec)') is ordinary, not contrived.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished), // see (note)\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "finished"))),
        "EXPR must be exactly `finished', not corrupted by the comment's own paren: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a well-formed rule must not be reported as malformed just because its comment has a paren: {}",
        msg
    );
}

#[test]
fn autotemplate_exact_trailing_comment_shaped_like_another_rule_does_not_corrupt_expr() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished), // .other (val)\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "finished"))),
        "EXPR must be exactly `finished', not corrupted by a comment that merely LOOKS like another rule: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a comment shaped like a rule must not itself be treated as one, nor make the real rule look malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_exact_comment_with_no_separator_before_it_does_not_corrupt_expr() {
    // No comma/semicolon between the closing `)' and the `//' -- the
    // balance-scanner's own tail consumption must not require one.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished)// see (note)\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "finished"))),
        "EXPR must be exactly `finished' even with no separator before the trailing comment: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a rule with no separator before its own trailing comment must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_wildcard_trailing_comment_containing_a_paren_does_not_corrupt_expr() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1), // see (note)\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "my_done"))),
        "wildcard EXPR must be exactly `my_done', not corrupted by the comment's own paren: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a well-formed wildcard rule must not be reported as malformed just because its comment has a paren: {}",
        msg
    );
}

#[test]
fn autotemplate_wildcard_trailing_comment_shaped_like_another_rule_does_not_corrupt_expr() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1), // .other (val)\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "my_done"))),
        "wildcard EXPR must be exactly `my_done', not corrupted by a comment that merely LOOKS like another rule: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a wildcard rule followed by a rule-shaped comment must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_wildcard_comment_with_no_separator_before_it_does_not_corrupt_expr() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .\\(.*\\) (my_\\1)// see (note)\n  ); */\nmodule top;\n  wire my_clk;\n  wire my_rst_n;\n  wire my_done;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "my_done"))),
        "wildcard EXPR must be exactly `my_done' even with no separator before its trailing comment: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a wildcard rule with no separator before its own trailing comment must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_expr_with_legitimately_balanced_parens_parses_correctly() {
    // `.a (foo(bar))' -- EXPR itself legitimately contains a balanced
    // paren pair. A non-greedy `.*?' fix (rejected -- see this
    // milestone's own docstring) would get this case wrong the same way
    // the original greedy `.*' got the trailing-comment case wrong.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (foo(bar)),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "foo(bar)"))),
        "EXPR must be exactly `foo(bar)', its own balanced parens intact: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "a rule whose EXPR legitimately contains balanced parens must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_two_rules_on_one_line_both_parse() {
    // M92 fix round X1: this is the reviewer's "free" second finding --
    // `.a (x), .b (y)' on one physical line used to match as a SINGLE
    // rule with EXPR captured as `x), .b (y'. Chosen behavior (see
    // `verilog-auto--template-rule-at's own doc string): both rules
    // parse correctly, rather than reporting the remainder as
    // malformed.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished), .clk (my_clk)\n  ); */\nmodule top;\n  wire my_clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    assert!(
        text.contains(&format!("{},", conn(&indent, "done", "finished"))),
        "the FIRST rule on the shared line must parse correctly: {}",
        text
    );
    assert!(
        text.contains(&format!("{},", conn(&indent, "clk", "my_clk"))),
        "the SECOND rule on the shared line must also parse correctly: {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "two well-formed rules sharing one line must not be reported as malformed: {}",
        msg
    );
}

#[test]
fn autotemplate_nested_block_comment_truncation_is_reported_not_silent() {
    // M92 fix round X2: an embedded `/* */' inside the AUTO_TEMPLATE
    // parens truncates the outer `block_comment' node at the FIRST
    // `*/' (confirmed by a real parse during the prior fix round), so
    // `verilog-auto--template-body-text' can never find a balanced
    // close paren and returns nil. This used to bypass the warning
    // channel entirely (indistinguishable from "no template at all");
    // `verilog-auto--find-template' now pushes its own warning in that
    // case.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  /* inner */\n  .done (finished),\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  sub_mod u1 (".len());
    // The whole template silently vanishes -- `done' falls back to
    // identity, exactly the pre-existing (still not fixed) behavior.
    assert!(
        text.contains(&format!("{},\n", conn(&indent, "done", "done"))),
        "the truncated template must fall back to identity, same as before this fix round: {}",
        text
    );
    // What's NEW is that this is no longer silent.
    assert!(
        msg.contains("could not be extracted"),
        "a template comment that was found but whose body couldn't be extracted must now be reported: {}",
        msg
    );
}

#[test]
fn autotemplate_parse_warnings_reset_across_two_verilog_auto_runs() {
    // M92 fix round X3: the fresh `(verilog-auto--template-parse-warnings
    // nil)' binding at the top of `verilog-auto' must not leak a
    // warning from an earlier run into a LATER, clean run's own
    // message.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* sub_mod AUTO_TEMPLATE (\n  .done (finished),\n  totally not a rule\n  ); */\nmodule top;\n  wire clk;\n  wire rst_n;\n  wire finished;\n  sub_mod u1 (/*AUTOINST*/);\nendmodule\n",
            SUB_MOD_TPL
        ),
    );
    let first_msg = verilog_auto(&mut i);
    assert!(
        first_msg.contains("not recognized"),
        "premise: the first run must see the malformed line: {}",
        first_msg
    );
    // Second run over the SAME (now already-expanded) buffer must not
    // carry the first run's warning forward.
    let second_msg = verilog_auto(&mut i);
    assert!(
        second_msg.contains("not recognized"),
        "the malformed template comment is still there on the second run too, so it must still be reported: {}",
        second_msg
    );
    // The count must not have grown across the two runs (would indicate
    // the warning list carried over instead of resetting).
    assert!(
        first_msg.contains("1 AUTO_TEMPLATE line(s) not recognized"),
        "first run must report exactly 1: {}",
        first_msg
    );
    assert!(
        second_msg.contains("1 AUTO_TEMPLATE line(s) not recognized"),
        "second run must ALSO report exactly 1, not 2 -- proving the warning list reset between runs: {}",
        second_msg
    );
}

// --- M124 Part C: AUTOINST files an interface-typed port under its own --
// `// Interfaces' category (real GNU `verilog-mode.el' 30.2 parity,
// `verilog-auto-inst', :12852-12862 -- `// Interfaces' emitted BEFORE
// `// Outputs'/`// Inouts'/`// Inputs'). Reproduced (M124) against the
// real `demo/verif/axi4_lite_monitor.sv' port shape
// (`axi4_lite_if.monitor bus'): before this fix, AUTOINST filed that
// port under `// Inputs' -- `verilog-auto--port-direction-of' fell back
// to 'input for any port with no `port_direction' descendant, which an
// `interface_port_header' structurally never has.

#[test]
fn autoinst_interface_typed_port_gets_its_own_interfaces_category() {
    let (mut i, _ed) = setup();
    let inst_line = "  axi4_lite_monitor u_mon (/*AUTOINST*/);\n";
    let indent = " ".repeat("  axi4_lite_monitor u_mon (".len());
    insert_src(
        &mut i,
        &format!(
            "module axi4_lite_monitor (\n  axi4_lite_if.monitor bus\n);\nendmodule\n\nmodule top;\n  axi4_lite_if bus();\n{}endmodule\n",
            inst_line
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected_block = format!(
        "/*AUTOINST*/\n{indent}// Interfaces\n{c1});",
        indent = indent,
        c1 = conn(&indent, "bus", "bus"),
    );
    assert!(
        text.contains(&expected_block),
        "expected block:\n{}\n\ngot buffer:\n{}",
        expected_block,
        text
    );
}

#[test]
fn autoinst_interface_plus_input_plus_output_gets_all_groups_in_gnu_order() {
    let (mut i, _ed) = setup();
    let inst_line = "  sub_mod u1 (/*AUTOINST*/);\n";
    let indent = " ".repeat("  sub_mod u1 (".len());
    insert_src(
        &mut i,
        &format!(
            "module sub_mod (\n  axi4_lite_if.monitor bus,\n  input  logic clk,\n  output logic done\n);\nendmodule\n\nmodule top;\n  axi4_lite_if bus();\n  wire clk;\n  wire done;\n{}endmodule\n",
            inst_line
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected_block = format!(
        "/*AUTOINST*/\n{indent}// Interfaces\n{c1},\n{indent}// Outputs\n{c2},\n{indent}// Inputs\n{c3});",
        indent = indent,
        c1 = conn(&indent, "bus", "bus"),
        c2 = conn(&indent, "done", "done"),
        c3 = conn(&indent, "clk", "clk"),
    );
    assert!(
        text.contains(&expected_block),
        "expected block (GNU order: Interfaces, Outputs, Inouts, Inputs):\n{}\n\ngot buffer:\n{}",
        expected_block,
        text
    );
}

/// AUTOARG must keep exactly its pre-M124 three-category output --
/// catches a shared-helper change (`verilog-auto--group-by-direction'/
/// `verilog-auto--grouped-lines') leaking an `// Interfaces' category
/// into AUTOARG, which real GNU's own `verilog-auto-arg' never has
/// (`verilog-mode.el' :12124-12143). AUTOARG can't actually be handed an
/// interface-typed port at all (its own port source,
/// `verilog-auto--nonansi-port-info', only ever reads body
/// input/output/inout declarations), so this is a regression pin, not a
/// new behavior test.
#[test]
fn autoarg_output_unchanged_by_the_interfaces_bucket() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top (/*AUTOARG*/);\n  input a;\n  input b;\n  output c;\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = "/*AUTOARG*/\n    // Outputs\n    c,\n    // Inputs\n    a,\n    b);";
    assert!(
        text.contains(expected),
        "AUTOARG's own three-category output must be byte-for-byte unchanged: {}",
        text
    );
    assert!(
        !text.contains("// Interfaces"),
        "AUTOARG must never emit an Interfaces category: {}",
        text
    );
}

// ===================== M125: AUTOOUTPUT/AUTOINPUT/AUTOINOUT =====================

#[test]
fn autooutput_declares_output_for_undriven_submodule_output() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), None, "done", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_skips_signal_consumed_by_another_instance() {
    // Spec section 1.1's own pinned case: `mid` is an output of u_p AND
    // an input of u_c -- internal, AUTOWIRE's job (out of THIS test's
    // scope), never AUTOOUTPUT's or AUTOINPUT's.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module prod (\n  output wire [7:0] mid,\n  output wire spare_o,\n  input wire clk\n);\nendmodule\n\nmodule cons (\n  input wire clk,\n  input wire [7:0] mid,\n  input wire spare_i\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  prod u_p (/*AUTOINST*/);\n  cons u_c (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected_output = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", None, None, "spare_o", "From", "u_p", "prod", false)
    );
    let expected_input = format!(
        "/*AUTOINPUT*/\n  // Beginning of automatic inputs (from unused autoinst inputs)\n{}\n{}\n  // End of automatics",
        port_decl("  ", "input", None, None, "clk", "To", "u_p", "prod", true),
        port_decl("  ", "input", None, None, "spare_i", "To", "u_c", "cons", false)
    );
    assert!(
        text.contains(&expected_output),
        "expected output block:\n{}\ngot:\n{}",
        expected_output,
        text
    );
    assert!(
        text.contains(&expected_input),
        "expected input block:\n{}\ngot:\n{}",
        expected_input,
        text
    );
    assert!(
        !text.contains("output wire [7:0] mid;") && !text.contains("output [7:0] mid;"),
        "mid must never become an AUTOOUTPUT: {}",
        text
    );
    assert!(
        !text.contains("input [7:0] mid;"),
        "mid must never become an AUTOINPUT: {}",
        text
    );
}

#[test]
fn autoinput_declares_input_for_unconnected_submodule_input() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOINPUT*/\n  sub_mod u1 (.clk(clk));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOINPUT*/\n  // Beginning of automatic inputs (from unused autoinst inputs)\n{}\n  // End of automatics",
        port_decl("  ", "input", Some("logic"), None, "clk", "To", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autoinput_skips_signal_driven_by_another_instance() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module prod (\n  output wire [7:0] mid\n);\nendmodule\n\nmodule cons (\n  input wire [7:0] mid\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOINPUT*/\n  prod u_p (/*AUTOINST*/);\n  cons u_c (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("input wire [7:0] mid;") && !text.contains("input [7:0] mid;"),
        "mid is driven by u_p -- must not become an AUTOINPUT: {}",
        text
    );
    assert!(
        text.contains("/*AUTOINPUT*/\n  prod"),
        "nothing else to declare -- no Beginning/End markers at all: {}",
        text
    );
}

#[test]
fn autoinout_declares_inout_for_submodule_inout() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  inout logic io_bus\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOINOUT*/\n  sub_mod u1 (.io_bus(io_bus));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOINOUT*/\n  // Beginning of automatic inouts (from unused autoinst inouts)\n{}\n  // End of automatics",
        port_decl("  ", "inout", Some("logic"), None, "io_bus", "To/From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_copies_type_and_range_from_submodule_port() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [7:0] dout\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.dout(dout));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), Some("[7:0]"), "dout", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_omits_plain_wire_type() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output wire spare_o\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.spare_o(spare_o));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", None, None, "spare_o", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

/// M125 fix round, spec section 1: GNU (measured 2026-09-09, real GNU
/// Emacs 30.2, against a real `output reg [WIDTH-1:0] bin_count;'
/// submodule port) drops `reg' from a propagated port declaration the
/// same way it drops `wire' -- `logic' is kept. The M125 spec's own
/// section 2.1 ("omit the type when it is exactly `wire'") was
/// under-specified; this pins the corrected rule.
#[test]
fn autooutput_omits_reg_and_net_type_keywords() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output reg [3:0] cnt,\n  output wire spare_o,\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.cnt(cnt), .spare_o(spare_o), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // Sorted alphabetically by name: cnt, done, spare_o.
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n{}\n{}\n  // End of automatics",
        port_decl("  ", "output", None, Some("[3:0]"), "cnt", "From", "u1", "sub_mod", false),
        port_decl("  ", "output", Some("logic"), None, "done", "From", "u1", "sub_mod", false),
        port_decl("  ", "output", None, None, "spare_o", "From", "u1", "sub_mod", false),
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_substitutes_instance_parameters_in_range() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [WIDTH-1:0] dout\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod #(.WIDTH(8)) u1 (.dout(dout));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), Some("[(8)-1:0]"), "dout", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_reports_conflicting_widths_across_instances() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module mod_a (\n  output logic [3:0] shared\n);\nendmodule\n\nmodule mod_b (\n  output logic [7:0] shared\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  mod_a ua (.shared(shared));\n  mod_b ub (.shared(shared));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("output logic [3:0] shared;"),
        "first-seen (mod_a's) range must win: {}",
        text
    );
    assert!(
        !text.contains("[7:0] shared;"),
        "mod_b's range must not win: {}",
        text
    );
    assert!(
        msg.contains("conflicting widths") && msg.contains("shared"),
        "echo must report the conflict: {}",
        msg
    );
}

/// Measured GNU output for this exact shape (spec section 1.5, case 21):
/// GNU emits `output logic\t\trvalid_o;' a second time, directly BELOW
/// the user's own `wire rvalid_o;' -- a genuine duplicate declaration
/// that does not compile. Reticle diverges (spec section 2.3): skip and
/// record a notice instead of generating code that can't compile.
#[test]
fn autooutput_skips_name_already_declared_in_module() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic rvalid_o\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  wire rvalid_o;\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.rvalid_o(rvalid_o));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("output logic rvalid_o").count(),
        1,
        "the submodule's own port declaration must be the ONLY `output logic rvalid_o' text -- no re-declared output in `top': {}",
        text
    );
    assert!(
        text.contains("/*AUTOOUTPUT*/\n  sub_mod"),
        "nothing to declare -- no Beginning/End markers: {}",
        text
    );
    assert!(
        msg.contains("already declared, skipped") && msg.contains("rvalid_o"),
        "echo must report the skip: {}",
        msg
    );
}

#[test]
fn autooutput_excludes_the_modules_own_ports() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic gnt_o\n);\nendmodule\n\nmodule top (gnt_o);\n  output logic gnt_o;\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.gnt_o(gnt_o));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOOUTPUT*/\n  sub_mod"),
        "the module's own port must not be re-declared -- no Beginning/End markers: {}",
        text
    );
    // The module's own port is excluded silently (not the same code path
    // as the "already declared elsewhere" notice) -- no port-count in the
    // echo either.
    assert!(msg.contains("0 ports"), "echo: {}", msg);
}

#[test]
fn auto_port_declarations_are_sorted_by_name() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic zed,\n  output logic alpha,\n  output logic mid\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.zed(zed), .alpha(alpha), .mid(mid));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n{}\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), None, "alpha", "From", "u1", "sub_mod", false),
        port_decl("  ", "output", Some("logic"), None, "mid", "From", "u1", "sub_mod", false),
        port_decl("  ", "output", Some("logic"), None, "zed", "From", "u1", "sub_mod", false),
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_regexp_argument_filters_signals() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic rdata_o,\n  output logic gnt_o\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT(\"^r\")*/\n  sub_mod u1 (.rdata_o(rdata_o), .gnt_o(gnt_o));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT(\"^r\")*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), None, "rdata_o", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_inverse_regexp_argument_filters_signals() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic rdata_o,\n  output logic gnt_o\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT(\"?!^r\")*/\n  sub_mod u1 (.rdata_o(rdata_o), .gnt_o(gnt_o));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = format!(
        "/*AUTOOUTPUT(\"?!^r\")*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", Some("logic"), None, "gnt_o", "From", "u1", "sub_mod", false)
    );
    assert!(
        text.contains(&expected),
        "expected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

#[test]
fn autooutput_malformed_regexp_argument_is_reported() {
    let (mut i, _ed) = setup();
    // The measured GNU trap (spec section 1.7): `?!' OUTSIDE the quotes.
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic rdata_o\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT(?!\"^r\")*/\n  sub_mod u1 (.rdata_o(rdata_o));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("output logic rdata_o;"),
        "malformed argument is treated as no filter -- everything still emits: {}",
        text
    );
    assert!(
        msg.contains("malformed"),
        "unlike GNU, the malformed argument must be reported, not silent: {}",
        msg
    );
}

#[test]
fn autooutput_provenance_comment_names_instance_and_module() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("// From u1 of sub_mod"), "buffer: {}", text);
}

#[test]
fn autooutput_block_layout_is_pinned_literally() {
    // Every other assertion in this file builds its expectation with
    // `port_decl', which recomputes the same padding rule the
    // implementation uses -- so a change to the layout on both sides at
    // once would keep every one of them green. This test spells the block
    // out character for character instead. The column is derived from the
    // rule (`verilog-auto-inst-column' is 40, so the trailing comment
    // begins at column 40: two spaces of indent plus the 18 characters of
    // "output logic done;" is 20, then 20 spaces), NOT read back off what
    // the code happened to print.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.clk(clk), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = concat!(
        "  /*AUTOOUTPUT*/\n",
        "  // Beginning of automatic outputs (from unused autoinst outputs)\n",
        "  output logic done;                    // From u1 of sub_mod\n",
        "  // End of automatics\n",
    );
    assert!(text.contains(expected), "buffer:\n{}", text);
}

#[test]
fn autoinput_provenance_comment_ends_with_ellipsis_for_several_instances() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOINPUT*/\n  sub_mod u_a (.clk(clk));\n  sub_mod u_b (.clk(clk));\n  sub_mod u_c (.clk(clk));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// To u_a of sub_mod, ..."),
        "GNU prints only the FIRST instance plus an ellipsis: {}",
        text
    );
}

#[test]
fn autooutput_in_ansi_header_module_expands_nothing_and_warns() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top (\n  input clk\n);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOOUTPUT*/\n  sub_mod"),
        "must expand to nothing on an ANSI header: {}",
        text
    );
    assert!(
        msg.contains("AUTOOUTPUT/AUTOINPUT/AUTOINOUT in ANSI header"),
        "echo: {}",
        msg
    );
    assert!(msg.contains("top"), "echo should name the module: {}", msg);
}

#[test]
fn autooutput_then_autowire_do_not_double_declare() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  wire clk;\n  /*AUTOOUTPUT*/\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), /*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("output logic done;"),
        "AUTOOUTPUT must declare done: {}",
        text
    );
    assert!(
        !text.contains("wire done;"),
        "AUTOWIRE must NOT also declare a wire for a name AUTOOUTPUT just turned into a port: {}",
        text
    );
}

#[test]
fn auto_port_blocks_are_idempotent() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done,\n  inout logic io_bus\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  /*AUTOINOUT*/\n  sub_mod u1 (.clk(clk), .done(done), .io_bus(io_bus));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let once = bs(&mut i);
    verilog_auto(&mut i);
    let twice = bs(&mut i);
    assert_eq!(once, twice, "a second verilog-auto must change nothing");
}

#[test]
fn delete_auto_removes_port_blocks_and_restores_original_text() {
    let (mut i, _ed) = setup();
    let src = "module sub_mod (\n  input logic clk,\n  output logic done,\n  inout logic io_bus\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  /*AUTOINOUT*/\n  sub_mod u1 (.clk(clk), .done(done), .io_bus(io_bus));\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    assert_ne!(
        bs(&mut i),
        src,
        "sanity: expansion actually changed the buffer"
    );
    delete_auto(&mut i);
    assert_eq!(
        bs(&mut i),
        src,
        "delete-auto must restore the original byte-for-byte"
    );
}

#[test]
fn delete_auto_recovers_from_hand_deleted_end_marker_in_port_block() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.contains("// End of automatics"),
        "sanity: {}",
        expanded
    );
    // Hand-delete the "// End of automatics" line, same shape as the
    // pre-existing AUTOWIRE test for this recovery path.
    let corrupted = expanded.replacen("  // End of automatics\n", "", 1);
    ok(&mut i, "(erase-buffer)");
    insert_src(&mut i, &corrupted);
    let r = delete_auto(&mut i);
    assert!(!r.starts_with("ERROR"), "delete-auto must not crash: {}", r);
    let text = bs(&mut i);
    assert!(
        text.contains("// Beginning of automatic outputs"),
        "the corrupted block must be left alone (staleness detected, not guessed at): {}",
        text
    );
}

#[test]
fn delete_auto_handles_adjacent_port_blocks() {
    let (mut i, _ed) = setup();
    let src = "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  sub_mod u1 (.clk(clk), .done(done));\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.contains("output logic done;") && expanded.contains("input logic clk;"),
        "sanity -- both adjacent blocks expanded: {}",
        expanded
    );
    delete_auto(&mut i);
    assert_eq!(
        bs(&mut i),
        src,
        "two adjacent, DIFFERENT-kind blocks must each delete their own range exactly, with no cross-attribution: {}",
        bs(&mut i)
    );
}

#[test]
fn auto_port_declarations_resolve_module_from_library_directory() {
    let (mut i, _ed) = setup();
    let dir = Scratch::new("m125lib");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("sub_mod.v"),
        "module sub_mod (\n  output logic done\n);\nendmodule\n",
    )
    .unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done));\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// From u1 of sub_mod.v"),
        "a module resolved via a library file must carry its REAL basename, `.v' and all: {}",
        text
    );
}

#[test]
fn autooutput_on_module_without_port_list_does_not_error() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, "module top;\n  /*AUTOOUTPUT*/\nendmodule\n");
    let r = run(&mut i, "(verilog-auto)");
    assert!(!r.starts_with("ERROR"), "must not crash: {}", r);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOOUTPUT*/\nendmodule"),
        "nothing to declare, nothing to crash on: {}",
        text
    );
}

#[test]
fn autooutput_with_no_instances_leaves_buffer_unchanged() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOOUTPUT*/\nendmodule"),
        "no instances -- no candidates -- no Beginning/End markers: {}",
        text
    );
    assert!(!text.contains("Beginning of automatic"));
}

#[test]
fn verilog_auto_echo_reports_port_count() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done,\n  inout logic io_bus\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  /*AUTOINOUT*/\n  sub_mod u1 (.clk(clk), .done(done), .io_bus(io_bus));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    assert!(msg.contains("3 ports"), "echo: {}", msg);
}

#[test]
fn auto_port_declarations_against_demo_rtl_sram_wrapper() {
    let (mut i, _ed) = setup();
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mem_dir = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("demo")
        .join("rtl")
        .join("mem");
    assert!(
        mem_dir.join("sram_wrapper.sv").exists(),
        "expected real demo material at {:?}",
        mem_dir
    );
    let dir = Scratch::new("m125sram");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.v");
    std::fs::write(
        &top_path,
        "module top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  sram_wrapper u_sw (/*AUTOINST*/);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(
        &mut i,
        &format!(
            "(setq verilog-library-directories (list {:?}))",
            mem_dir.to_str().unwrap()
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let outputs = [
        ("gnt_o", None),
        ("rdata_o", Some("[DataWidth-1:0]")),
        ("rvalid_o", None),
    ];
    for (name, range) in outputs {
        let expected = port_decl(
            "  ",
            "output",
            Some("logic"),
            range,
            name,
            "From",
            "u_sw",
            "sram_wrapper.sv",
            false,
        );
        assert!(
            text.contains(&expected),
            "expected output line:\n{}\ngot buffer:\n{}",
            expected,
            text
        );
    }
    let inputs = [
        ("addr_i", Some("[AddrWidth-1:0]")),
        ("clk_i", None),
        ("req_i", None),
        ("rst_ni", None),
        ("wdata_i", Some("[DataWidth-1:0]")),
        ("we_i", None),
        ("wstrb_i", Some("[soc_pkg::StrbWidth-1:0]")),
    ];
    for (name, range) in inputs {
        let expected = port_decl(
            "  ",
            "input",
            Some("logic"),
            range,
            name,
            "To",
            "u_sw",
            "sram_wrapper.sv",
            false,
        );
        assert!(
            text.contains(&expected),
            "expected input line:\n{}\ngot buffer:\n{}",
            expected,
            text
        );
    }
}

#[test]
fn autoinout_partitions_signals_away_from_output_and_input() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_a (\n  output logic sig\n);\nendmodule\n\nmodule sub_b (\n  inout logic sig\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  /*AUTOINOUT*/\n  sub_a ua (.sig(sig));\n  sub_b ub (.sig(sig));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("inout logic sig;"),
        "sig touches an inout port somewhere -- AUTOINOUT declares it unconditionally: {}",
        text
    );
    assert!(
        !text.contains("output logic sig;"),
        "sig must not ALSO be declared as an output: {}",
        text
    );
    assert!(
        !text.contains("input logic sig;"),
        "sig must not ALSO be declared as an input: {}",
        text
    );
}

/// M125 bugfix, beyond the spec's own 29-test list: a TYPED non-ANSI
/// body port declaration (`output reg [WIDTH-1:0] bin_count;', the real
/// shape `demo/rtl-verilog2001/gray_ctr.v' uses) used to come back from
/// `verilog-auto--module-ports' with direction defaulted to `input' and
/// no range at all -- `verilog-auto--nonansi-port-info' only ever
/// searched for `list_of_port_identifiers', not the
/// `list_of_variable_port_identifiers' a TYPED body declaration actually
/// uses (confirmed by a real tree dump, M125 recon). This is the same
/// gap `verilog-auto--declared-names' had, fixed together (see this
/// file's own M125 header).
#[test]
fn nonansi_typed_output_reg_port_direction_and_range_detected_via_module_ports() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module gray_ctr (\n  clk,\n  bin_count\n);\n  input clk;\n  output reg [WIDTH-1:0] bin_count;\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  gray_ctr u1 (.clk(clk), .bin_count(bin_count));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // The EXACT generated block, not a loose substring -- the
    // submodule's own SOURCE declaration line
    // ("  output reg [WIDTH-1:0] bin_count;") legitimately contains
    // "output reg [WIDTH-1:0] bin_count;" as a substring too, so a
    // substring assertion here would pass against the source text and
    // never actually look at what AUTOOUTPUT generated (M92's own Q1
    // lesson, dev/mutations/m92-auto-template.py). `reg' is dropped per
    // the M125 fix round (spec section 1) -- bin_count must still be
    // classified as an OUTPUT with its own range, just without `reg'.
    let expected = format!(
        "/*AUTOOUTPUT*/\n  // Beginning of automatic outputs (from unused autoinst outputs)\n{}\n  // End of automatics",
        port_decl("  ", "output", None, Some("[WIDTH-1:0]"), "bin_count", "From", "u1", "gray_ctr", false)
    );
    assert!(
        text.contains(&expected),
        "bin_count must be classified as an OUTPUT with its own range \
        (not the wrong-default input/no-range) and without `reg':\nexpected:\n{}\ngot:\n{}",
        expected,
        text
    );
}

/// Beyond the spec's own 29-test list: exercises the M125 generalization
/// of `verilog-auto--autowire-stale-end''s own "another marker proves my
/// End is missing" check to ALL FOUR block-style markers, not just
/// `/*AUTOWIRE*/'. AUTOOUTPUT's own End line is hand-deleted while an
/// intact AUTOINPUT block sits immediately after it -- if the scan only
/// recognized `/*AUTOWIRE*/' as a stop condition (the pre-M125 shape),
/// it would walk straight past AUTOINPUT's own marker into AUTOINPUT's
/// declarations and misattribute AUTOINPUT's own End line to AUTOOUTPUT.
#[test]
fn delete_auto_adjacent_port_blocks_with_hand_deleted_end_still_detects_staleness() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  /*AUTOINPUT*/\n  sub_mod u1 (.clk(clk), .done(done));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.matches("// End of automatics").count() == 2,
        "sanity -- both blocks expanded: {}",
        expanded
    );
    // Remove the FIRST "// End of automatics" -- AUTOOUTPUT's own.
    let corrupted = expanded.replacen("  // End of automatics\n", "", 1);
    ok(&mut i, "(erase-buffer)");
    insert_src(&mut i, &corrupted);
    let r = delete_auto(&mut i);
    assert!(!r.starts_with("ERROR"), "delete-auto must not crash: {}", r);
    let text = bs(&mut i);
    assert!(
        text.contains("// Beginning of automatic outputs"),
        "AUTOOUTPUT's own corrupted block must be left alone, not swallowed into AUTOINPUT's range: {}",
        text
    );
    assert!(
        text.contains("/*AUTOINPUT*/\n  sub_mod"),
        "AUTOINPUT's own INTACT block must still be deleted correctly, back to a bare marker: {}",
        text
    );
}

/// M125 fix round, spec section 3.2: `verilog-auto--predeclared-port-
/// names' (and its two siblings, `--port-range-conflicts' and
/// `--port-marker-arg-warnings') are filled during a RIGHTMOST-first
/// site walk, so `push' alone already leaves the list in document
/// order -- an extra `nreverse' at the end of `verilog-auto' used to
/// invert it, naming the LATER of two offending modules as "first" in
/// the echo. Two modules, `top_a' (earlier) and `top_b' (later), each
/// contribute one already-declared signal under a DIFFERENT name so the
/// echo's own choice of "first" is unambiguous.
#[test]
fn auto_port_notices_name_the_earlier_modules_signal_not_the_later() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top_a (/*AUTOARG*/);\n  wire done_a;\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done_a));\nendmodule\n\nmodule top_b (/*AUTOARG*/);\n  wire done_b;\n  /*AUTOOUTPUT*/\n  sub_mod u2 (.done(done_b));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("first: done_a"),
        "top_a comes FIRST in the buffer -- its own signal must be named, not top_b's: {}",
        msg
    );
    assert!(
        !msg.contains("first: done_b"),
        "must not name the LATER module's signal: {}",
        msg
    );
}

/// M125 fix round, spec section 3.4: the comma-separated multi-instance
/// form (`sub_mod u_a (...), u_b (...);' -- one `module_instantiation'
/// with TWO `hierarchical_instance' children) is a shape the candidate
/// loop already handles (`verilog-auto--port-propagation-candidates'
/// walks every `hierarchical_instance' under each `module_instantiation'
/// individually) but no M125 test exercised it -- AUTOWIRE's own
/// `autowire_multi_instance_comma_form_sees_every_instance' is the
/// precedent this mirrors.
#[test]
fn auto_port_propagation_multi_instance_comma_form_sees_every_instance() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic done\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  sub_mod u_a (.done(done_a)), u_b (.done(done_b));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("output logic done_a;") && text.contains("// From u_a of sub_mod"),
        "the FIRST comma-separated instance must be seen: {}",
        text
    );
    assert!(
        text.contains("output logic done_b;") && text.contains("// From u_b of sub_mod"),
        "the SECOND comma-separated instance must be seen too, not just the first: {}",
        text
    );
}

/// M125 fix round, spec section 3.4: an unresolvable submodule feeding
/// AUTOOUTPUT/AUTOINPUT/AUTOINOUT must be a silent no-op for THAT
/// instance (consistent with the pre-existing missing-module notice
/// AUTOINST already produces), not a crash and not a spurious
/// declaration.
#[test]
fn auto_port_propagation_unresolvable_submodule_is_a_silent_no_op() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top (/*AUTOARG*/);\n  /*AUTOOUTPUT*/\n  nonexistent_mod u1 (.done(done));\nendmodule\n",
    );
    let r = run(&mut i, "(verilog-auto)");
    assert!(!r.starts_with("ERROR"), "must not crash: {}", r);
    let text = bs(&mut i);
    assert!(
        text.contains("/*AUTOOUTPUT*/\n  nonexistent_mod"),
        "an unresolvable submodule contributes no candidates -- no Beginning/End markers: {}",
        text
    );
}

/// M125 fix round, spec section 3.4: the AUTOOUTPUT/AUTOWIRE ordering
/// interaction (`autooutput_then_autowire_do_not_double_declare') has no
/// AUTOINOUT counterpart -- an inout signal AUTOINOUT just turned into a
/// port must not ALSO get an AUTOWIRE wire declaration.
#[test]
fn autoinout_then_autowire_do_not_double_declare() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  inout logic io_bus\n);\nendmodule\n\nmodule top (/*AUTOARG*/);\n  wire clk;\n  /*AUTOINOUT*/\n  /*AUTOWIRE*/\n  sub_mod u1 (.clk(clk), /*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("inout logic io_bus;"),
        "AUTOINOUT must declare io_bus: {}",
        text
    );
    assert!(
        !text.contains("wire io_bus;"),
        "AUTOWIRE must NOT also declare a wire for a name AUTOINOUT just turned into a port: {}",
        text
    );
}

/// Cold-review finding (trailing re-review): `verilog-auto--expand-all-
/// port-propagation' is called once per KIND ('output/'input/'inout),
/// each its OWN independent rightmost-first walk -- the
/// `auto_port_notices_name_the_earlier_modules_signal_not_the_later'
/// test above only ever used ONE kind (AUTOOUTPUT for both modules), so
/// it never exercised the ACROSS-kind case: the fixed 'output ->
/// 'input -> 'inout call order in `verilog-auto' decides which kind's
/// own contribution ends up last in the list, independent of buffer
/// position -- `top_a' (earlier in the buffer, AUTOOUTPUT) versus
/// `top_b' (later, AUTOINPUT) used to report `top_b''s own signal as
/// "first" regardless of the fact that `top_a' comes first on screen.
#[test]
fn auto_port_notices_name_the_earlier_modules_signal_across_different_kinds() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  input logic clk,\n  output logic done\n);\nendmodule\n\nmodule top_a (/*AUTOARG*/);\n  wire done_a;\n  /*AUTOOUTPUT*/\n  sub_mod u1 (.done(done_a));\nendmodule\n\nmodule top_b (/*AUTOARG*/);\n  wire clk_b;\n  /*AUTOINPUT*/\n  sub_mod u2 (.clk(clk_b));\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    assert!(
        msg.contains("first: done_a"),
        "top_a comes FIRST in the buffer (an AUTOOUTPUT conflict) -- must be named over top_b's AUTOINPUT conflict, which is a later call in a fixed kind order but a LATER module on screen: {}",
        msg
    );
    assert!(
        !msg.contains("first: clk_b"),
        "must not name the LATER module's signal just because its KIND (AUTOINPUT) is processed after AUTOOUTPUT's own pass: {}",
        msg
    );
}

// ===================== M126: AUTOREG / AUTOTIEOFF =====================
//
// Ground truth measured against real GNU Emacs 30.2 (M126 spec section 1);
// every divergence from GNU is cross-referenced to that spec's own section
// 2 and to `crates/core/lisp/verilog-auto.el''s own M126 header.

/// One AUTOTIEOFF declaration line, INDENT included, mirroring
/// `verilog-auto--tieoff-decl-line`: DECL_KW "wire" carries SIGNED/RANGE,
/// DECL_KW "assign" carries neither. NOT used by the one "layout pinned
/// literally" test below, which spells the block out with `concat!`
/// instead so a coordinated change to both this helper and the
/// implementation can't stay green together.
fn tieoff_line(
    indent: &str,
    decl_kw: &str,
    signed: bool,
    range: &str,
    name: &str,
    konst: &str,
) -> String {
    let mut body = decl_kw.to_string();
    if decl_kw == "wire" {
        if signed {
            body.push_str(" signed");
        }
        if !range.is_empty() {
            body.push(' ');
            body.push_str(range);
        }
    }
    body.push(' ');
    body.push_str(name);
    format!("{}{}= {};", indent, pad(&body, 40, indent.len()), konst)
}

// --- AUTOREG ---

#[test]
fn autoreg_basic_nonansi_emission_with_and_without_range() {
    // R1: `output [3:0] a; output b;' -> `reg [3:0] a;'/`reg b;'.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("reg [3:0] a;"), "buffer: {}", text);
    assert!(text.contains("reg b;"), "buffer: {}", text);
    assert!(
        text.contains("// Beginning of automatic regs (for this module's undeclared outputs)"),
        "buffer: {}",
        text
    );
}

#[test]
fn autoreg_ansi_header_emits_nothing_and_records_module() {
    // R2: an ANSI header -- file byte-identical, no frame at all.
    let (mut i, _ed) = setup();
    let src = "module dut (\n  output logic [3:0] a\n);\n  /*AUTOREG*/\nendmodule\n";
    insert_src(&mut i, src);
    let msg = verilog_auto(&mut i);
    assert_eq!(bs(&mut i), src, "ANSI header: AUTOREG must expand nothing");
    assert!(
        msg.contains("AUTOREG in ANSI header (module dut"),
        "echo: {}",
        msg
    );
}

#[test]
fn autoreg_body_reg_skipped() {
    // R3: `a' already declared `reg' in the body -> skipped, `b' emitted.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  reg [3:0] a;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("reg [3:0] a;").count(),
        1,
        "a must not be regenerated: {}",
        text
    );
    assert!(text.contains("reg b;"), "buffer: {}", text);
}

#[test]
fn autoreg_body_wire_skipped() {
    // R4: `a' already declared `wire' in the body -> skipped, `b' emitted.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  wire [3:0] a;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;"),
        "a already a wire, AUTOREG must not also declare it a reg: {}",
        text
    );
    assert!(text.contains("reg b;"), "buffer: {}", text);
}

#[test]
fn autoreg_instance_driven_output_skipped() {
    // R5: `a' driven by a submodule instance -> skipped, `b' emitted.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [3:0] a\n);\nendmodule\n\nmodule dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOREG*/\n  sub_mod u1 (.a(a));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;"),
        "a is instance-driven, must not be declared reg: {}",
        text
    );
    assert!(text.contains("reg b;"), "buffer: {}", text);
}

#[test]
fn autoreg_inout_not_emitted() {
    // R6: `inout' is never considered by AUTOREG.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, io);\n  output [3:0] a;\n  inout [1:0] io;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("reg [3:0] a;"), "buffer: {}", text);
    assert!(
        !text.contains("reg [1:0] io;") && !text.contains("reg io;"),
        "an inout must never be declared a reg: {}",
        text
    );
}

#[test]
fn autoreg_typed_port_skipped_bare_port_still_emitted() {
    // R7/R12: a per-port type keyword skips ONLY that port -- a bare
    // port in the SAME module is still emitted (this is a PER-PORT
    // rule, not a whole-module bail-out).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output logic [3:0] a;\n  output b;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;") && !text.contains("reg a;"),
        "a already typed logic on the port, must not be re-declared reg: {}",
        text
    );
    assert!(text.contains("reg b;"), "buffer: {}", text);
}

#[test]
fn autoreg_signed_preserved_and_emitted() {
    // R15: `signed' is NOT a type keyword and does not suppress AUTOREG;
    // it is preserved in the emitted declaration.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output signed [3:0] a;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("reg signed [3:0] a;"), "buffer: {}", text);
}

#[test]
fn autoreg_continuous_assign_driven_skipped() {
    // R17: `a' driven by a continuous assign -> skipped.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  assign a = 4'h1;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;"),
        "a is continuous-assign-driven, must not be declared reg: {}",
        text
    );
    assert!(text.contains("reg b;"), "buffer: {}", text);
}

#[test]
fn autoreg_always_driven_still_emitted() {
    // W4: AUTOREG does not look at procedural (`always'-block) drivers
    // at all -- `a' is STILL emitted (an always-driven output must
    // legally be a reg).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOREG*/\n  always @(posedge clk) a <= 4'h2;\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("reg [3:0] a;"), "buffer: {}", text);
}

#[test]
fn autoreg_symbolic_range_copied_verbatim() {
    // R8: a symbolic range is copied verbatim, no parameter substitution.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [WIDTH-1:0] a;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("reg [WIDTH-1:0] a;"), "buffer: {}", text);
}

#[test]
fn autoreg_no_outputs_no_frame() {
    // R9: no outputs at all -> file byte-identical.
    let (mut i, _ed) = setup();
    let src = "module dut (clk);\n  input clk;\n  /*AUTOREG*/\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    assert_eq!(bs(&mut i), src);
}

#[test]
fn autoreg_regexp_argument_nothing_and_warning() {
    // Divergence 4: a regexp argument makes the command a no-op AND
    // records a warning (GNU is silent here, R10).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOREG(\"^a\")*/\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg"),
        "an argument makes AUTOREG a no-op, unlike GNU's own filter semantics: {}",
        text
    );
    assert!(
        msg.contains("malformed/unsupported AUTO marker argument"),
        "unlike GNU, this must be reported: {}",
        msg
    );
}

#[test]
fn autoreg_alphabetical_order() {
    // R11: emitted alphabetically, not document order.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (zeta, alpha, mid);\n  output zeta;\n  output alpha;\n  output mid;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let pa = text.find("reg alpha;").expect("alpha present");
    let pm = text.find("reg mid;").expect("mid present");
    let pz = text.find("reg zeta;").expect("zeta present");
    assert!(pa < pm && pm < pz, "must be alphabetical: {}", text);
}

#[test]
fn autoreg_idempotent_under_second_run() {
    // R16.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let first = bs(&mut i);
    verilog_auto(&mut i);
    assert_eq!(bs(&mut i), first, "second run must be a no-op");
}

#[test]
fn autoreg_block_layout_is_pinned_literally() {
    // AUTOREG's own declaration line is plain, single-space-separated,
    // with NO column padding -- it has nothing trailing to align to
    // (unlike AUTOTIEOFF's own `= CONST;'), matching this file's
    // existing AUTOWIRE convention.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = concat!(
        "  /*AUTOREG*/\n",
        "  // Beginning of automatic regs (for this module's undeclared outputs)\n",
        "  reg [3:0] a;\n",
        "  // End of automatics\n",
    );
    assert!(text.contains(expected), "buffer:\n{}", text);
}

// --- AUTOTIEOFF ---

#[test]
fn autotieoff_numeric_constant_table() {
    // The numeric constant table, several ports in one module.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b, c, d, e);\n  output [3:0] a;\n  output b;\n  output [31:0] c;\n  output [15:8] d;\n  output [0:3] e;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[3:0]", "a", "4'h0")),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[31:0]", "c", "32'h0")),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[15:8]", "d", "8'h0")),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[0:3]", "e", "4'h0")),
        "ascending range [0:3] must still compute width 4: {}",
        text
    );
}

#[test]
fn autotieoff_symbolic_forms_including_special_case_boundary() {
    // Every symbolic form in spec section 1.3, with the `[WIDTH-1:0]'
    // special case and `[2*W-1:0]' NOT taking it side by side so the
    // boundary can't rot in half. Also two whitespace-tolerant spellings
    // of the special case (`g'/`h', fix round -- GNU's own regex
    // tolerates whitespace at every position and matches the whole
    // range text, measured against real GNU Emacs 30.2; this file's own
    // original regex required no whitespace at all and matched MSB
    // only). Of those two, only `g' (`[WIDTH - 1:0]') actually depends
    // on the widened regex: `h' (`[ WIDTH-1 : 0 ]') has all of its
    // whitespace stripped by `verilog-auto--normalize-range-whitespace'
    // and `verilog-auto--range-bounds's `string-trim' before any regex
    // runs, so its MSB arrives as "WIDTH-1" and the ORIGINAL regex
    // matched it too. `h' is kept because it pins that normalisation
    // step, not the regex. And `[WIDTH-1:1]' (`ii'), which must still take the GENERAL
    // form despite sharing `WIDTH-1' with the special case -- only LSB
    // `0' triggers it, and this pins that boundary alongside the
    // whitespace tolerance so neither can rot without the other.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b, c, d, e, f, g, h, ii);\n  output [WIDTH-1:0] a;\n  output [2*W-1:0] b;\n  output [WIDTH:0] c;\n  output [N:1] d;\n  output [A+B:0] e;\n  output [7:LSB] f;\n  output [WIDTH - 1:0] g;\n  output [ WIDTH-1 : 0 ] h;\n  output [WIDTH-1:1] ii;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[WIDTH-1:0]",
            "a",
            "{WIDTH{1'b0}}"
        )),
        "WIDTH-1:0 special case: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[2*W-1:0]",
            "b",
            "{(1+(2*W-1)){1'b0}}"
        )),
        "2*W-1:0 must NOT take the special case: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[WIDTH:0]",
            "c",
            "{(1+(WIDTH)){1'b0}}"
        )),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[N:1]",
            "d",
            "{(1+(N)-(1)){1'b0}}"
        )),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[A+B:0]",
            "e",
            "{(1+(A+B)){1'b0}}"
        )),
        "buffer: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[7:LSB]",
            "f",
            "{(1+(7)-(LSB)){1'b0}}"
        )),
        "numeric msb + symbolic lsb still takes the general form: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[WIDTH - 1:0]",
            "g",
            "{WIDTH{1'b0}}"
        )),
        "whitespace around the '-' must still take the special case, and the \
        user's own range spelling is kept verbatim (not GNU's own rewritten \
        [WIDTH-1:0]): {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[WIDTH-1 : 0]",
            "h",
            "{WIDTH{1'b0}}"
        )),
        "whitespace around the ':' must still take the special case: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[WIDTH-1:1]",
            "ii",
            "{(1+(WIDTH-1)-(1)){1'b0}}"
        )),
        "WIDTH-1:1 shares 'WIDTH-1' with the special case but has LSB 1, not \
        0 -- must still take the general form: {}",
        text
    );
}

#[test]
fn autotieoff_signed_emits_sh_constant() {
    // T19: signed switches the numeric constant's suffix to 'sh.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output signed [3:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "wire", true, "[3:0]", "a", "4'sh0")),
        "buffer: {}",
        text
    );
}

#[test]
fn autotieoff_body_reg_skipped() {
    // T4.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  reg [3:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(!text.contains("a = "), "a already a body reg: {}", text);
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "buffer: {}",
        text
    );
}

#[test]
fn autotieoff_port_reg_skipped_with_name_recorded() {
    // Divergence 3: GNU ties off a port already declared `reg' on the
    // port itself (T10), duplicating the declaration -- Reticle
    // deliberately differs here and skips it, recording the name.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output reg [1:0] a;\n  output b;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("a = "),
        "a already reg on the port: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "buffer: {}",
        text
    );
    assert!(
        msg.contains("already `reg' on the port") && msg.contains("first: a"),
        "echo: {}",
        msg
    );
}

#[test]
fn autotieoff_typed_ports_other_than_reg_are_still_tied_off() {
    // Fix round coverage gap: AUTOTIEOFF is deliberately ASYMMETRIC with
    // AUTOREG -- a port-level type keyword (`logic'/`wire') suppresses
    // AUTOREG per-port (R7/R12-R15), but does NOT suppress AUTOTIEOFF at
    // all, except for `reg' specifically (divergence 3, the test above).
    // T10 measured GNU tying off ALL THREE of `output logic [3:0] a;',
    // `output wire b;' and `output reg [1:0] c;' -- Reticle's own
    // divergence is narrower, refusing only the `reg' one. The test
    // above only ever exercises the ONE type that IS skipped; this one
    // exercises the two that are NOT, so the asymmetry has coverage on
    // both sides.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output logic [3:0] a;\n  output wire b;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[3:0]", "a", "4'h0")),
        "a is `logic'-typed on the port, unlike `reg' this must NOT suppress AUTOTIEOFF: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "b is `wire'-typed on the port, must also still be tied off: {}",
        text
    );
}

#[test]
fn autotieoff_instance_driven_skipped() {
    // T5.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [3:0] a\n);\nendmodule\n\nmodule dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOTIEOFF*/\n  sub_mod u1 (.a(a));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(!text.contains("a = "), "a is instance-driven: {}", text);
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "buffer: {}",
        text
    );
}

#[test]
fn autotieoff_continuous_assign_driven_skipped() {
    // T12.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  assign a = 4'h1;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("wire [3:0] a"),
        "a is continuous-assign-driven: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "", "b", "1'h0")),
        "buffer: {}",
        text
    );
}

// --- Fix round coverage gap: `verilog-auto--net-lvalue-driven-names' own
// concatenation/part-select shapes -- every other continuous-assign test
// above only ever uses a bare identifier LHS (`assign a = ...;'). ---

#[test]
fn autoreg_concatenation_lvalue_drives_every_element() {
    // `assign {a, b} = ...;' -- both `a' and `b' count as driven.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b, c);\n  output [3:0] a;\n  output [3:0] b;\n  output [3:0] c;\n  assign {a, b} = 8'h0;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;"),
        "a is concat-driven: {}",
        text
    );
    assert!(
        !text.contains("reg [3:0] b;"),
        "b is concat-driven: {}",
        text
    );
    assert!(
        text.contains("reg [3:0] c;"),
        "c is untouched by the assign, must still be emitted: {}",
        text
    );
}

#[test]
fn autotieoff_concatenation_lvalue_drives_every_element() {
    // Same shape, AUTOTIEOFF side (T12's own sibling).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b, c);\n  output [3:0] a;\n  output [3:0] b;\n  output [3:0] c;\n  assign {a, b} = 8'h0;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("wire [3:0] a"),
        "a is concat-driven: {}",
        text
    );
    assert!(
        !text.contains("wire [3:0] b"),
        "b is concat-driven: {}",
        text
    );
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[3:0]", "c", "4'h0")),
        "c is untouched by the assign, must still be tied off: {}",
        text
    );
}

#[test]
fn autoreg_part_select_lvalue_drives_the_base_name() {
    // `assign a[3:0] = ...;' -- a partial drive is treated as a full
    // drive (the conservative direction: suppresses a candidate
    // declaration rather than risking a real duplicate).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [7:0] a;\n  assign a[3:0] = 4'h0;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(!text.contains("reg"), "a's base name is driven: {}", text);
}

#[test]
fn autotieoff_part_select_lvalue_drives_the_base_name() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [7:0] a;\n  assign a[3:0] = 4'h0;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(!text.contains("wire"), "a's base name is driven: {}", text);
}

#[test]
fn autoreg_mixed_concatenation_and_part_select_does_not_mistake_the_index_for_a_driven_name() {
    // `assign {a, b[IDX]} = ...;' -- `a' and `b' both count as driven
    // (concatenation element / part-select base name respectively), but
    // `IDX' -- the index expression INSIDE `b's own part-select, which
    // sits as a SIBLING `constant_select' node next to `b's own
    // `simple_identifier', not nested inside another `net_lvalue' --
    // must NOT be mistaken for a driven signal, even when a port
    // happens to share its exact name. This is the shape this file's
    // own `verilog-auto--net-lvalue-driven-names' header calls out as
    // the easiest place for the sibling-node structure to be gotten
    // wrong.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b, IDX);\n  output [3:0] a;\n  output [3:0] b;\n  output [3:0] IDX;\n  assign {a, b[IDX]} = 8'h0;\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("reg [3:0] a;"),
        "a is concat-driven: {}",
        text
    );
    assert!(
        !text.contains("reg [3:0] b;"),
        "b is part-select-driven: {}",
        text
    );
    assert!(
        text.contains("reg [3:0] IDX;"),
        "IDX is a port in its own right, only used as an INDEX expression \
        elsewhere -- it must not be mistaken for a driven signal just \
        because its name appears inside a constant_select: {}",
        text
    );
}

#[test]
fn autotieoff_inout_not_tied() {
    // T11.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, io);\n  output [3:0] a;\n  inout [1:0] io;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "wire", false, "[3:0]", "a", "4'h0")),
        "buffer: {}",
        text
    );
    assert!(
        !text.contains("io ="),
        "an inout must never be tied off: {}",
        text
    );
}

#[test]
fn autotieoff_assign_knob_on_nonansi_header() {
    // T6: `verilog-auto-tieoff-declaration' set to "assign" -- no `wire'
    // keyword, no range.
    let (mut i, _ed) = setup();
    ok(&mut i, "(setq verilog-auto-tieoff-declaration \"assign\")");
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "assign", false, "", "a", "4'h0")),
        "buffer: {}",
        text
    );
    assert!(!text.contains("wire"), "buffer: {}", text);
}

#[test]
fn autotieoff_ansi_header_emits_assign_form_and_records_switch() {
    // Divergence 2: ANSI header -> `assign' form regardless of the
    // knob's default, plus a notice.
    let (mut i, _ed) = setup();
    let src = "module dut (\n  output logic [3:0] a\n);\n  /*AUTOTIEOFF*/\nendmodule\n";
    insert_src(&mut i, src);
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line("  ", "assign", false, "", "a", "4'h0")),
        "buffer: {}",
        text
    );
    assert!(
        msg.contains("AUTOTIEOFF switched to `assign' form in ANSI header (module dut"),
        "echo: {}",
        msg
    );
}

#[test]
fn autotieoff_multidim_all_numeric_uses_product() {
    // Divergence 5, numeric case: width is the PRODUCT of every
    // dimension (GNU would silently use only the LAST one, width 2).
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0][1:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(&tieoff_line(
            "  ",
            "wire",
            false,
            "[3:0] [1:0]",
            "a",
            "8'h0"
        )),
        "width must be the PRODUCT 4*2=8, not GNU's own under-reported 2: {}",
        text
    );
}

#[test]
fn autotieoff_multidim_symbolic_dimension_skipped_with_notice() {
    // Divergence 5, symbolic case: skipped entirely rather than emitting
    // a width Reticle cannot justify.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [WIDTH:0][1:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        !text.contains("// Beginning of automatic tieoffs"),
        "a's only candidate got skipped, so no frame at all: {}",
        text
    );
    assert!(
        msg.contains("symbolic multi-dimensional range") && msg.contains("first: a"),
        "echo: {}",
        msg
    );
}

#[test]
fn autotieoff_regexp_argument_nothing_and_warning() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOTIEOFF(\"^a\")*/\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(!text.contains("= 4'h0"), "buffer: {}", text);
    assert!(
        msg.contains("malformed/unsupported AUTO marker argument"),
        "echo: {}",
        msg
    );
}

#[test]
fn autotieoff_no_outputs_no_frame() {
    // T13.
    let (mut i, _ed) = setup();
    let src = "module dut (clk);\n  input clk;\n  /*AUTOTIEOFF*/\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    assert_eq!(bs(&mut i), src);
}

#[test]
fn autotieoff_block_layout_is_pinned_literally() {
    // The column is derived from the rule (`verilog-auto-inst-column' is
    // 40, so `= 4'h0;' begins at column 40: two spaces of indent plus
    // the 12 characters of "wire [3:0] a" is 14, then 26 spaces), NOT
    // read back off what the code happened to print.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a);\n  output [3:0] a;\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = concat!(
        "  /*AUTOTIEOFF*/\n",
        "  // Beginning of automatic tieoffs (for this module's unterminated outputs)\n",
        "  wire [3:0] a                          = 4'h0;\n",
        "  // End of automatics\n",
    );
    assert!(text.contains(expected), "buffer:\n{}", text);
}

// --- Interaction: AUTOTIEOFF suppresses AUTOREG (execution order) ---

#[test]
fn autotieoff_suppresses_autoreg_in_same_module() {
    // O2/T9: with both markers present, AUTOTIEOFF's own tie-off
    // declaration becomes visible to AUTOREG's own fresh reparse, so
    // AUTOREG emits NOTHING AT ALL -- not even the frame comments.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOTIEOFF*/\n  /*AUTOREG*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// Beginning of automatic tieoffs"),
        "AUTOTIEOFF must still expand: {}",
        text
    );
    assert!(
        !text.contains("// Beginning of automatic regs"),
        "AUTOREG must emit nothing at all, not even the frame: {}",
        text
    );
    assert!(
        text.contains("/*AUTOREG*/\nendmodule"),
        "the AUTOREG marker itself must be left bare: {}",
        text
    );
}

#[test]
fn autotieoff_suppresses_autoreg_regardless_of_marker_document_order() {
    // T18: document order of the two markers is irrelevant -- execution
    // order (not buffer position) decides that AUTOTIEOFF runs first.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module dut (a, b);\n  output [3:0] a;\n  output b;\n  /*AUTOREG*/\n  /*AUTOTIEOFF*/\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// Beginning of automatic tieoffs"),
        "buffer: {}",
        text
    );
    assert!(
        !text.contains("// Beginning of automatic regs"),
        "AUTOREG must still emit nothing even though it comes FIRST in the buffer: {}",
        text
    );
}

// --- delete-auto ---

#[test]
fn delete_auto_removes_autoreg_and_autotieoff_blocks_and_restores_original_text() {
    let (mut i, _ed) = setup();
    let src = "module dut (a);\n  output [3:0] a;\n  /*AUTOREG*/\nendmodule\n\nmodule dut2 (b);\n  output b;\n  /*AUTOTIEOFF*/\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    assert_ne!(bs(&mut i), src, "sanity: expansion changed the buffer");
    delete_auto(&mut i);
    assert_eq!(
        bs(&mut i),
        src,
        "delete-auto must restore the original byte-for-byte"
    );
}

/// Mirrors `delete_auto_adjacent_port_blocks_with_hand_deleted_end_still_
/// detects_staleness`: AUTOWIRE's own End line is hand-deleted while an
/// intact AUTOREG block sits immediately after it.
///
/// AUTOTIEOFF and AUTOREG themselves are NOT used as the pair here: by
/// construction (this file's M126 header), any untyped output port
/// AUTOREG would ever emit is EXACTLY the same set of ports a live
/// AUTOTIEOFF marker in the SAME module would already have tied off on
/// the immediately preceding phase -- the O2/T9 interaction this
/// milestone's own spec measures is not a corner case, it is what
/// happens EVERY time both markers are live in one module. So there is
/// no module shape where both produce a real, non-empty block to test
/// adjacency with in the first place; AUTOWIRE (a disjoint candidate
/// set -- undeclared INSTANCE outputs, never a port name) pairs with
/// AUTOREG without that interaction.
#[test]
fn delete_auto_adjacent_autowire_autoreg_blocks_with_hand_deleted_end_still_detects_staleness() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module sub_mod (\n  output logic [3:0] internal_sig\n);\nendmodule\n\nmodule dut (a);\n  output [3:0] a;\n  /*AUTOWIRE*/\n  /*AUTOREG*/\n  sub_mod u1 (.internal_sig(internal_sig));\nendmodule\n",
    );
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.matches("// End of automatics").count() == 2,
        "sanity -- both blocks expanded: {}",
        expanded
    );
    // Remove the FIRST "// End of automatics" -- AUTOWIRE's own.
    let corrupted = expanded.replacen("  // End of automatics\n", "", 1);
    ok(&mut i, "(erase-buffer)");
    insert_src(&mut i, &corrupted);
    let r = delete_auto(&mut i);
    assert!(!r.starts_with("ERROR"), "delete-auto must not crash: {}", r);
    let text = bs(&mut i);
    assert!(
        text.contains("// Beginning of automatic wires"),
        "AUTOWIRE's own corrupted block must be left alone, not swallowed into AUTOREG's range: {}",
        text
    );
    assert!(
        text.contains("/*AUTOREG*/\n  sub_mod"),
        "AUTOREG's own INTACT block must still be deleted correctly, back to a bare marker: {}",
        text
    );
}

// ===================== M127: AUTO_TEMPLATE's substitution language =========
// `@' instance-number substitution, `[]'/`[][]' bit-range tokens, the
// `// Templated' annotation, the quoted-regexp template-head parse fix, and
// the instance-array non-gap regression pins. See the M127 spec and
// crates/core/lisp/verilog-auto.el's own M127 header for the full design.

fn autotemplate_at_case(inst: &str, expected: &str) -> String {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_{e};\n  leaf {inst} (/*AUTOINST*/);\nendmodule\n",
            e = expected,
            inst = inst
        ),
    );
    verilog_auto(&mut i);
    bs(&mut i)
}

#[test]
fn autotemplate_at_first_digit_run_u0() {
    let text = autotemplate_at_case("u0", "0");
    assert!(text.contains("(sig_0)"), "{}", text);
}

#[test]
fn autotemplate_at_first_digit_run_u_ch0() {
    let text = autotemplate_at_case("u_ch0", "0");
    assert!(text.contains("(sig_0)"), "{}", text);
}

#[test]
fn autotemplate_at_first_digit_run_u_ch12() {
    let text = autotemplate_at_case("u_ch12", "12");
    assert!(text.contains("(sig_12)"), "{}", text);
}

#[test]
fn autotemplate_at_first_digit_run_u2_ch3_is_the_first_run_not_the_last() {
    let text = autotemplate_at_case("u2_ch3", "2");
    assert!(
        text.contains("(sig_2)"),
        "must be 2 (the FIRST digit run), not 3: {}",
        text
    );
}

#[test]
fn autotemplate_at_first_digit_run_u_1_2_is_the_first_run_not_the_last() {
    let text = autotemplate_at_case("u_1_2", "1");
    assert!(
        text.contains("(sig_1)"),
        "must be 1 (the FIRST digit run), not 2: {}",
        text
    );
}

#[test]
fn autotemplate_at_first_digit_run_bank10x() {
    let text = autotemplate_at_case("bank10x", "10");
    assert!(text.contains("(sig_10)"), "{}", text);
}

#[test]
fn autotemplate_at_no_digits_in_instance_name_expands_empty_and_warns() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_;\n  leaf u_chan (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(sig_)"),
        "no-digit instance name must expand `@' to the empty string: {}",
        text
    );
    assert!(
        msg.contains("resolved to empty"),
        "must record the divergence-3 notice: {}",
        msg
    );
}

#[test]
fn autotemplate_at_substituted_globally_not_first_only() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic [7:0] data\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .data (a@[]b@[]c),\n  ); */\nmodule top;\n  wire [7:0] a3b3c;\n  leaf u_ch3 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(a3[7:0]b3[7:0]c)"),
        "`@' must substitute GLOBALLY, not just the first occurrence: {}",
        text
    );
}

#[test]
fn autotemplate_at_is_inert_without_a_matching_auto_template() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\nmodule other_leaf (\n  output logic done\n);\nendmodule\n\n/* other_leaf AUTO_TEMPLATE (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire done;\n  leaf u_ch3 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(done)"),
        "no template applies to THIS module -- plain identity connection: {}",
        text
    );
    assert!(
        !msg.contains("resolved to empty"),
        "`@' machinery must not even run when no template applies: {}",
        msg
    );
}

#[test]
fn autotemplate_at_lhs_selects_ports_but_rhs_always_uses_the_instance_number() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic data_0,\n  input logic data_1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .data_@ (in[@]),\n  ); */\nmodule top;\n  wire [7:0] in;\n  leaf u_ch3 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains(".data_0"), "{}", text);
    assert!(text.contains(".data_1"), "{}", text);
    assert_eq!(
        text.matches("(in[3])").count(),
        2,
        "BOTH ports must connect to the INSTANCE's own `@' (3), never their own matched digit (0/1): {}",
        text
    );
}

#[test]
fn autotemplate_explicit_group_carries_ports_digit_at_carries_instance_number() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic data_5,\n  input logic data_9\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .data_\\(.*\\) (in_\\1_@),\n  ); */\nmodule top;\n  wire in_5_3;\n  wire in_9_3;\n  leaf u_ch3 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(in_5_3)"),
        "explicit `\\1' must carry the PORT's own digit: {}",
        text
    );
    assert!(text.contains("(in_9_3)"), "{}", text);
}

#[test]
fn autotemplate_lhs_at_group_positional_case_a_at_after_user_group() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic data_0,\n  input logic data_1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .\\(.*\\)_@ (out_\\1_@),\n  ); */\nmodule top;\n  wire out_data_7;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("(out_data_7)").count(),
        2,
        "both data_0/data_1 must emit out_data_7 -- `\\1' is the user's own group, the trailing `@' is the INSTANCE number, never the port's matched digit: {}",
        text
    );
}

#[test]
fn autotemplate_lhs_at_group_positional_case_b_at_before_user_group() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic data_0_x,\n  input logic data_1_y\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .data_@_\\(.*\\) (out_\\1_\\2_@),\n  ); */\nmodule top;\n  wire out_0_x_7;\n  wire out_1_y_7;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(out_0_x_7)"),
        "the `@'-generated group comes FIRST in the pattern text, so it is `\\1' here: {}",
        text
    );
    assert!(text.contains("(out_1_y_7)"), "{}", text);
}

#[test]
fn autotemplate_lhs_starting_with_at_is_structurally_unreachable() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic sig\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .@_\\(.*\\) (bogus_\\1),\n  ); */\nmodule top;\n  wire sig;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // The AUTO_TEMPLATE comment's OWN source text (`.@_\(.*\) (bogus_\1),')
    // stays verbatim in the buffer, so restrict the check to the
    // generated region after `module top;' -- otherwise "bogus" would
    // trivially "appear" via the rule's own definition, not via any
    // actual substitution.
    let generated = text.split("module top;").nth(1).unwrap();
    assert!(
        generated.contains("(sig)"),
        "a real port name can never begin with a digit, so this rule must match nothing and fall back to identity: {}",
        generated
    );
    assert!(!generated.contains("bogus"), "{}", generated);
}

#[test]
fn autotemplate_custom_instance_number_regexp_overrides_default_digit_run() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE \"_ch\\([0-9]+\\)$\" (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_3;\n  leaf u2_ch3 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(sig_3)"),
        "custom regexp's own group 1 (3) must win over the default rule's answer (2): {}",
        text
    );
    assert!(
        !msg.contains("not recognized"),
        "regression pin for the section 3.1 parse defect -- the rule must be APPLIED, not dropped: {}",
        msg
    );
}

#[test]
fn autotemplate_custom_instance_number_regexp_not_matching_expands_empty_and_warns() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE \"_ch\\([0-9]+\\)$\" (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_;\n  leaf u_bank (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("(sig_)"), "{}", text);
    assert!(msg.contains("resolved to empty"), "{}", msg);
}

#[test]
fn autotemplate_custom_instance_number_regexp_without_capture_group_never_crashes() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE \"ch[0-9]+\" (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(sig_)"),
        "no capture group -> empty substitution, never a crash, never an aborted run: {}",
        text
    );
    assert!(
        msg.contains("malformed/unsupported AUTO marker argument"),
        "must record the divergence-4 notice, reusing M125's marker-argument list: {}",
        msg
    );
}

#[test]
fn autotemplate_custom_instance_number_regexp_two_groups_always_uses_group_one() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE \"\\(u\\)_ch\\([0-9]+\\)\" (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_u;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(sig_u)"),
        "must use group 1 (`u'), not group 2 (`7'), deterministically -- GNU parity: {}",
        text
    );
    assert!(
        !msg.contains("malformed/unsupported AUTO marker argument"),
        "two-or-more groups is silent, no notice: {}",
        msg
    );
    assert!(!msg.contains("resolved to empty"), "{}", msg);
}

#[test]
fn autotemplate_unterminated_quoted_regexp_is_inert_and_warned_not_a_hard_error() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE \"ch[0-9]+ (\n  .done (finished),\n  ); */\nmodule top;\n  wire finished;\n  leaf u_ch7 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains(".done"),
        "malformed template head must be inert, falling back to identity, same posture as any other malformed template: {}",
        text
    );
    assert!(
        msg.contains("not recognized") || msg.contains("could not be extracted"),
        "must be warned, never silently dropped: {}",
        msg
    );
}

#[test]
fn autotemplate_bracket_matrix_single_bracket_takes_the_last_packed_dimension() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic p1,\n  input logic [7:0] p2,\n  input logic [WIDTH-1:0] p3,\n  input logic [1:0][7:0] p4,\n  input logic [7:0] p5 [0:3]\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (w1[]),\n  .p2 (w2[]),\n  .p3 (w3[]),\n  .p4 (w4[]),\n  .p5 (w5[]),\n  ); */\nmodule top;\n  wire w1;\n  wire [7:0] w2;\n  wire [WIDTH-1:0] w3;\n  wire [7:0] w4;\n  wire [7:0] w5;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(text.contains("(w1)"), "1-bit port -> empty `[]': {}", text);
    assert!(text.contains("(w2[7:0])"), "{}", text);
    assert!(text.contains("(w3[WIDTH-1:0])"), "{}", text);
    assert!(
        text.contains("(w4[7:0])"),
        "2-D packed port -> `[]' takes the LAST dimension, not the first: {}",
        text
    );
    assert!(
        text.contains("(w5[7:0])"),
        "unpacked-array port -> `[]' still only sees the packed dimension: {}",
        text
    );
}

#[test]
fn autotemplate_bracket_matrix_double_bracket_is_a_block_comment_only_when_multidimensional() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic p1,\n  input logic [7:0] p2,\n  input logic [WIDTH-1:0] p3,\n  input logic [1:0][7:0] p4,\n  input logic [7:0] p5 [0:3]\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (v1[][]),\n  .p2 (v2[][]),\n  .p3 (v3[][]),\n  .p4 (v4[][]),\n  .p5 (v5[][]),\n  ); */\nmodule top;\n  wire v1;\n  wire [7:0] v2;\n  wire [WIDTH-1:0] v3;\n  wire v4;\n  wire v5;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(v1)"),
        "1-bit port -> `[][]' behaves like `[]': {}",
        text
    );
    assert!(text.contains("(v2[7:0])"), "{}", text);
    assert!(text.contains("(v3[WIDTH-1:0])"), "{}", text);
    assert!(
        text.contains("(v4/*[1:0][7:0]*/)"),
        "2-D packed port -> `[][]' emits the full spec wrapped in a block comment: {}",
        text
    );
    assert!(
        text.contains("(v5/*[7:0].[0:3]*/)"),
        "unpacked-array port -> packed dims then `.' then unpacked dims: {}",
        text
    );
}

#[test]
fn autotemplate_bracket_zero_to_zero_range_is_preserved_not_collapsed() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic [0:0] p1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (w1[]),\n  ); */\nmodule top;\n  wire [0:0] w1;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(w1[0:0])"),
        "an explicit `[0:0]' port must keep `[0:0]', not collapse to empty like a true scalar: {}",
        text
    );
}

#[test]
fn autotemplate_bracket_ordering_double_bracket_not_eaten_as_two_empty_brackets() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic [1:0][7:0] p4\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p4 (w4[][]),\n  ); */\nmodule top;\n  wire w4;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(w4/*[1:0][7:0]*/)"),
        "if `[]' ran first it would eat `[][]' as two independent empty brackets, giving `w4[7:0][7:0]' instead: {}",
        text
    );
    assert!(!text.contains("(w4[7:0][7:0])"), "{}", text);
}

#[test]
fn autotemplate_templated_connection_carries_the_annotation_identity_connection_does_not() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic p1,\n  output logic p2\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (renamed),\n  ); */\nmodule top;\n  wire renamed;\n  wire p2;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert_eq!(
        text.matches("// Templated").count(),
        1,
        "exactly one connection (p1) came from a rule; the identity connection (p2) must carry no annotation at all: {}",
        text
    );
    // The AUTO_TEMPLATE comment's own source text also contains
    // "renamed"; restrict the line search to the generated region.
    let generated = text.split("module top;").nth(1).unwrap();
    let p1_line = generated.lines().find(|l| l.contains("(renamed)")).unwrap();
    assert!(p1_line.contains("// Templated"), "{}", p1_line);
}

#[test]
fn autotemplate_templated_annotation_placement_middle_comma_vs_last_close_paren() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic p1,\n  output logic p2\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (r1),\n  .p2 (r2),\n  ); */\nmodule top;\n  wire r1;\n  wire r2;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    // The AUTO_TEMPLATE comment's own source text also contains
    // "(r1)"/"(r2)"; restrict the line search to the generated region.
    let generated = text.split("module top;").nth(1).unwrap();
    let p1_line = generated
        .lines()
        .find(|l| l.contains("(r1)"))
        .expect("p1 line");
    assert!(
        p1_line.contains("(r1),"),
        "middle line keeps its own comma before the annotation: {}",
        p1_line
    );
    assert!(
        p1_line.trim_end().ends_with("// Templated"),
        "middle line: annotation sits after the `,': {}",
        p1_line
    );
    let p2_line = generated
        .lines()
        .find(|l| l.contains("(r2)"))
        .expect("p2 line");
    assert!(
        p2_line.contains(");"),
        "last line is glued directly onto the pre-existing closing paren/`;': {}",
        p2_line
    );
    assert!(
        p2_line.trim_end().ends_with("// Templated"),
        "last line: annotation sits after `));'/`;', not before it: {}",
        p2_line
    );
}

#[test]
fn autotemplate_templated_annotation_alignment_uses_the_longest_connection_line_even_when_last() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic a,\n  output logic b\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .a (ra),\n  .b (rb_much_longer_expression),\n  ); */\nmodule top;\n  wire ra;\n  wire rb_much_longer_expression;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    let line_a = format!("{},", conn(&indent, "a", "ra"));
    let line_b_no_comma = conn(&indent, "b", "rb_much_longer_expression");
    assert!(
        line_b_no_comma.len() > line_a.len(),
        "test setup sanity: b's own line must be the longer one"
    );
    let col = 1 + line_b_no_comma.len();
    let expected_a = format!("{}{}// Templated", line_a, " ".repeat(col - line_a.len()));
    assert!(
        text.contains(&expected_a),
        "the SHORTER line (a) must be padded to align with the LONGER, LAST line (b), not its own length:\nexpected: {:?}\ngot: {}",
        expected_a,
        text
    );
    let generated = text.split("module top;").nth(1).unwrap();
    let b_line = generated
        .lines()
        .find(|l| l.contains("(rb_much_longer_expression)"))
        .unwrap();
    assert!(
        b_line.contains(");") && b_line.trim_end().ends_with("// Templated"),
        "last line still gets its own annotation, past the real closing text, with at least one space: {}",
        b_line
    );
}

#[test]
fn autotemplate_group_headers_never_carry_the_templated_annotation() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic p1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (r1),\n  ); */\nmodule top;\n  wire r1;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let header_line = text
        .lines()
        .find(|l| l.trim() == "// Outputs")
        .expect("header line");
    assert!(!header_line.contains("Templated"), "{}", header_line);
}

#[test]
fn autotemplate_lhs_mode_prints_anchored_pattern_for_wildcard_and_bare_name_for_exact() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(setq verilog-auto-inst-template-numbers 'lhs)");
    insert_src(
        &mut i,
        "module leaf (\n  output logic exact_port,\n  output logic wild_en\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .exact_port (rp),\n  .\\(.*\\)_en (r_\\1),\n  ); */\nmodule top;\n  wire rp;\n  wire r_wild;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// Templated LHS: exact_port"),
        "exact rule -> bare port name, no anchors: {}",
        text
    );
    assert!(
        text.contains("// Templated LHS: ^\\(.*\\)_en$"),
        "wildcard rule -> the compiled, anchored pattern text: {}",
        text
    );
}

#[test]
fn autotemplate_template_numbers_t_behaves_as_nil_and_records_notice() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(setq verilog-auto-inst-template-numbers t)");
    insert_src(
        &mut i,
        "module leaf (\n  output logic p1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (r1),\n  ); */\nmodule top;\n  wire r1;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("// Templated") && !text.contains("// Templated LHS") && !text.contains("// Templated 1"),
        "`t' must behave EXACTLY like nil -- a bare `// Templated', never the absolute-line-number form: {}",
        text
    );
    assert!(
        msg.contains("verilog-auto-inst-template-numbers"),
        "must record a notice naming the variable, never silently ignored: {}",
        msg
    );
}

#[test]
fn autoarg_output_byte_identical_before_m127() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module foo (/*AUTOARG*/);\n  input clk;\n  output [7:0] q;\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let expected = "/*AUTOARG*/\n    // Outputs\n    q,\n    // Inputs\n    clk);";
    assert!(
        text.contains(expected),
        "the shared `verilog-auto--grouped-lines' change must leave AUTOARG byte-for-byte unchanged:\nexpected: {}\ngot: {}",
        expected,
        text
    );
    assert!(!msg.contains("Templated"), "{}", msg);
}

#[test]
fn autotemplate_forward_fallback_notice_when_a_template_precedes_a_different_instance() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\nmodule top;\n  wire done_a;\n  wire done_b;\n  leaf u0 (/*AUTOINST*/);\n  /* leaf AUTO_TEMPLATE (\n     .done (done_a),\n     ); */\n  leaf u1 (/*AUTOINST*/);\n  /* leaf AUTO_TEMPLATE (\n     .done (done_b),\n     ); */\nendmodule\n",
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    // Count only actual GENERATED connections (`(done_a));`, the LAST --
    // and here, only -- connection in each single-port instantiation),
    // never the AUTO_TEMPLATE comments' own rule-definition source text
    // (`.done (done_a),` -- a comma, not `));`), which also contains the
    // literal substrings "(done_a)"/"(done_b)".
    let done_a_conns = text.matches("(done_a));").count();
    let done_b_conns = text.matches("(done_b));").count();
    assert_eq!(
        done_a_conns, 2,
        "both u0 (via the forward fallback) and u1 (backward search) must resolve to the FIRST template: {}",
        text
    );
    assert_eq!(
        done_b_conns, 0,
        "the second template must never be used by anything: {}",
        text
    );
    assert!(
        msg.contains("forward fallback"),
        "the divergence-5 notice must fire (for u0's own lookup): {}",
        msg
    );
}

#[test]
fn autotemplate_round_trip_with_at_brackets_and_templated_annotation_is_byte_identical() {
    let (mut i, _ed) = setup();
    let src = "module leaf (\n  output logic done,\n  output logic [7:0] data\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .done (sig_@),\n  .data (bus_@[]),\n  ); */\nmodule top;\n  wire sig_1;\n  wire [7:0] bus_1;\n  leaf u_ch1 (/*AUTOINST*/);\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.contains("// Templated"),
        "sanity -- something actually got templated: {}",
        expanded
    );
    delete_auto(&mut i);
    let reverted = bs(&mut i);
    assert_eq!(
        reverted, src,
        "verilog-delete-auto must return EXACTLY the original source, including no orphan `// Templated' text left past the closing paren"
    );
    verilog_auto(&mut i);
    let second = bs(&mut i);
    assert_eq!(
        expanded, second,
        "verilog-auto must stay idempotent with `@'/`[]'/`// Templated' all in play at once"
    );
}

#[test]
fn autoinst_template_at_instance_array_uses_name_not_array_range() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  output logic done\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .done (sig_@),\n  ); */\nmodule top;\n  wire sig_7;\n  leaf u_ch7[3:0] (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(sig_7)"),
        "`@' must read 7 from the instance NAME field, ignoring the `[3:0]' array range entirely: {}",
        text
    );
}

#[test]
fn autoinst_instance_array_and_scalar_produce_identical_connection_lists() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf (\n  input logic clk,\n  output logic [7:0] q\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire [7:0] q;\n  leaf u_arr[3:0] (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let arr_text = bs(&mut i);

    let (mut j, _ed2) = setup();
    insert_src(
        &mut j,
        "module leaf (\n  input logic clk,\n  output logic [7:0] q\n);\nendmodule\n\nmodule top;\n  wire clk;\n  wire [7:0] q;\n  leaf u_scalar (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut j);
    let scalar_text = bs(&mut j);

    // `verilog-auto-inst-column' pads each `.NAME' to the SAME absolute
    // column, measured from each instance's own indent -- and
    // `u_arr[3:0] (' is 5 characters longer than `u_scalar (', so the
    // amount of INTERNAL padding differs even though both land on the
    // same absolute column. Collapse whitespace runs before comparing,
    // so this incidental column-alignment difference (a consequence of
    // instance-NAME length, section 0's own point) doesn't masquerade
    // as a real difference in the connection list itself.
    let normalize = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| l.trim_start().starts_with('.'))
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    };
    let arr_conns = normalize(&arr_text);
    let scalar_conns = normalize(&scalar_text);
    assert_eq!(
        arr_conns, scalar_conns,
        "an array instance and a scalar instance must produce IDENTICAL connection lists (M127 spec section 0's own measured GNU parity):\narr: {}\nscalar: {}",
        arr_text, scalar_text
    );
}

#[test]
fn autotemplate_nonansi_single_port_unpacked_dimension_bracket_single() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf(p5);\n  input [7:0] p5 [0:3];\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p5 (w5[]),\n  ); */\nmodule top;\n  wire [7:0] w5;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(w5[7:0])"),
        "a NON-ANSI-declared unpacked-array port: `[]' must still resolve to its own packed dimension only: {}",
        text
    );
}

#[test]
fn autotemplate_nonansi_single_port_unpacked_dimension_bracket_double() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf(p5);\n  input [7:0] p5 [0:3];\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p5 (v5[][]),\n  ); */\nmodule top;\n  wire v5;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(v5/*[7:0].[0:3]*/)"),
        "a NON-ANSI-declared unpacked-array port: `[][]' must see the unpacked dimension and wrap packed+unpacked in a block comment, exactly like the ANSI shape already tested: {}",
        text
    );
}

#[test]
fn autotemplate_nonansi_multiname_unpacked_dimension_second_name_only() {
    // `input [7:0] a, b [0:3];' -- ONE packed dimension shared by both
    // names, but the unpacked dimension belongs to `b' ALONE. This is
    // the exact multi-name shape `verilog-auto--nonansi-port-dims' must
    // get right (M127 fix round: a cold review found no test anywhere
    // in this milestone's own diff exercised a non-ANSI unpacked-array
    // port at all -- a mutation narrowing that function's own
    // `find-all-of-type' to only the FIRST name in each declaration
    // went undetected by every other test). `a' must NOT inherit `b's
    // own unpacked dimension, and `b' must not lose it (or its own
    // packed dimension) to the mutation either.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf(a, b);\n  input [7:0] a, b [0:3];\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .a (wa[][]),\n  .b (wb[][]),\n  ); */\nmodule top;\n  wire wa;\n  wire wb;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(wa[7:0])"),
        "`a' has no unpacked dimension of its own -- `[][]' must behave like `[]' (packed only, no block comment): {}",
        text
    );
    assert!(
        text.contains("(wb/*[7:0].[0:3]*/)"),
        "`b' has its OWN unpacked dimension -- `[][]' must see it (packed AND unpacked, wrapped in a block comment), a dims lookup that finds only the first name in the declaration would leave `b' with no dims at all and this would come back empty: {}",
        text
    );
}

#[test]
fn autotemplate_lhs_mode_templated_last_line_round_trips_through_delete_auto() {
    // M127 fix round: `verilog-auto--trailing-templated-annotation-
    // range's own `'lhs' alternative (`"// Templated LHS: ..."') had no
    // test at all -- the existing round-trip test only ever ran in the
    // default `nil' mode, so removing that regex alternative broke
    // nothing observable. This is the LAST (and only) connection in a
    // single-port module, so its own annotation lands PAST the
    // pre-existing closing paren -- exactly the code path
    // `verilog-auto--trailing-templated-annotation-range' exists for.
    let (mut i, _ed) = setup();
    ok(&mut i, "(setq verilog-auto-inst-template-numbers 'lhs)");
    let src = "module leaf (\n  output logic p1\n);\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .p1 (r1),\n  ); */\nmodule top;\n  wire r1;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n";
    insert_src(&mut i, src);
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    assert!(
        expanded.contains("// Templated LHS: p1"),
        "sanity -- `'lhs' mode's own annotation text must actually appear: {}",
        expanded
    );
    delete_auto(&mut i);
    let reverted = bs(&mut i);
    assert_eq!(
        reverted, src,
        "verilog-delete-auto must strip the `'lhs'-mode annotation back out too, exactly like the default `// Templated' form, leaving no orphan text past the closing paren"
    );
}

#[test]
fn autotemplate_nonansi_typed_multiname_unpacked_dimension_second_name_only() {
    // Same shape as `autotemplate_nonansi_multiname_unpacked_dimension_
    // second_name_only', but with a TYPED non-ANSI declaration (`output
    // logic ...') instead of a bare untyped one -- a trailing cold
    // review found every existing non-ANSI test used the untyped
    // `input [7:0] ...' form, so `verilog-auto--nonansi-port-dims''s own
    // `(or (find-first-of-type decl "list_of_port_identifiers")
    // (find-first-of-type decl "list_of_variable_port_identifiers"))'
    // never actually took its SECOND branch anywhere in this
    // milestone's own test suite -- a wrong node-type string there
    // would have gone undetected.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf(a, b);\n  output logic [7:0] a, b [0:3];\nendmodule\n\n/* leaf AUTO_TEMPLATE (\n  .a (wa[][]),\n  .b (wb[][]),\n  ); */\nmodule top;\n  wire wa;\n  wire wb;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    assert!(
        text.contains("(wa[7:0])"),
        "`a' has no unpacked dimension of its own -- `[][]' must behave like `[]': {}",
        text
    );
    assert!(
        text.contains("(wb/*[7:0].[0:3]*/)"),
        "`b' has its OWN unpacked dimension, declared via the TYPED (`logic') non-ANSI shape -- \
         `list_of_variable_port_identifiers', never exercised by any earlier test: {}",
        text
    );
}

// ===================== M128: `@"(lisp-expr)"' evaluated AUTO_TEMPLATE ======
// tokens. See the M128 spec and crates/core/lisp/verilog-auto.el's own M128
// header for the design (measured against real GNU Emacs 30.2).

const M128_LEAF: &str = "\
module leaf (
  input  logic              clk,
  input  logic [7:0]        p2,
  input  logic [WIDTH-1:0]  p3,
  input  logic [1:0][7:0]   p4,
  input  logic [7:0]        mem [0:3],
  input  logic [0:0]        z1,
  output logic              done
);
endmodule

";

#[test]
fn lisp_token_minimal_exact_rule_end_to_end() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat vl-name \\\"_x\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done_x;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done_x")),
        "@\"(lisp-expr)\"' must evaluate and splice its result, not leak literal text: {}",
        text
    );
}

#[test]
fn lisp_token_embedded_in_surrounding_text() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (pre@\"(concat \\\"X\\\")\"post),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire preXpost;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "preXpost")),
        "a lisp token embedded in surrounding text must splice in place: {}",
        text
    );
}

#[test]
fn lisp_token_two_tokens_in_one_expression() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat \\\"A\\\")\"_@\"(concat \\\"B\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire A_B;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "A_B")),
        "each `@\"...\"' token in one EXPR must be evaluated in turn, left to right: {}",
        text
    );
}

#[test]
fn lisp_token_at_substituted_into_source_before_read_including_nested_string() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat \\\"AT@HERE\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire AT9HERE;\n  leaf u_ch9 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u_ch9 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "AT9HERE")),
        "`@' must be substituted into the token's own SOURCE text before it is read, even inside a nested string literal: {}",
        text
    );
}

#[test]
fn lisp_token_result_rescanned_for_at() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat \\\"sig\\\" \\\"@\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire sig2;\n  leaf u_ch2 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u_ch2 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "sig2")),
        "a lisp-evaluated result containing a bare `@' must still be re-scanned by the ordinary `@' substitution pass afterward: {}",
        text
    );
}

#[test]
fn lisp_token_result_rescanned_for_brackets() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .p2 (@\"(concat \\\"sig\\\" \\\"[]\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  wire sig;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p2", "sig[7:0]")),
        "a lisp-evaluated result containing a bare `[]' must still be re-scanned by the ordinary `[]' substitution pass afterward: {}",
        text
    );
}

#[test]
fn lisp_token_combined_with_bracket_token_elsewhere_in_same_expr() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .p2 (@\"(concat \\\"W\\\")\"[]),\n  ); */\nmodule top;\n  wire clk;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  wire W;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p2", "W[7:0]")),
        "a lisp token and an ordinary `[]' token in the SAME expression must both apply: {}",
        text
    );
}

// --- Escaping (section 2.2): two mutually exclusive conventions ----------

#[test]
fn lisp_token_escaping_exact_single_backslash_quote_works() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat vl-name \\\"_a\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done_a;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done_a")),
        "an EXACT rule's `\\\"' escaping convention must work: {}",
        text
    );
}

#[test]
fn lisp_token_escaping_exact_double_backslash_quote_is_fatal_falls_back() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat vl-name \\\\\"_a\\\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "an EXACT rule's `\\\\\"' (double-backslash) convention is fatal in GNU's exact-rule context -- reticle falls back to an identity connection rather than aborting: {}",
        text
    );
    assert!(
        msg.contains("AUTO_TEMPLATE lisp expression"),
        "the fallback must be reported: {}",
        msg
    );
}

#[test]
fn lisp_token_escaping_wildcard_single_backslash_quote_is_fatal_falls_back() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .\\(done\\) (@\"(concat \\\"X\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "a WILDCARD rule's `\\\"' (single-backslash) convention is fatal in GNU's wildcard context (its own EXPR already went through `replace-regexp-in-string') -- reticle falls back rather than aborting: {}",
        text
    );
    assert!(
        msg.contains("AUTO_TEMPLATE lisp expression") || msg.contains("failed"),
        "the fallback must be reported: {}",
        msg
    );
}

#[test]
fn lisp_token_escaping_wildcard_double_backslash_quote_works() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .\\(done\\) (@\"(concat \\\\\"X\\\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire X;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "X")),
        "a WILDCARD rule's `\\\\\"' (double-backslash) convention must work: {}",
        text
    );
}

#[test]
fn lisp_token_escaping_wildcard_no_quotes_works() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .\\(done\\) (@\"(number-to-string 42)\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire 42;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "42")),
        "a wildcard rule expression with no quotes at all must work unchanged: {}",
        text
    );
}

// --- The nine `vl-*' variables (section 2.3) ------------------------------

fn m128_vl_probe(port: &str, expr_lisp: &str) -> String {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .{port} (probe_@\"{expr}\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF,
            port = port,
            expr = expr_lisp
        ),
    );
    verilog_auto(&mut i);
    bs(&mut i)
}

#[test]
fn vl_vars_name_is_the_declared_port_name() {
    let text = m128_vl_probe("done", "(concat vl-name)");
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "probe_done")),
        "vl-name must be the port's own declared name: {}",
        text
    );
}

#[test]
fn vl_vars_name_is_declared_not_connected_signal() {
    // A wildcard rule connects `done' to a DIFFERENT signal name
    // (`finished') -- vl-name must still read as `done', never
    // `finished'.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .\\(done\\) (@\"(concat vl-name)\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire finished;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "vl-name must be the DECLARED port name (\"done\"), never the connected signal: {}",
        text
    );
}

#[test]
fn vl_vars_width_bits_mbits_memory_scalar() {
    let text = m128_vl_probe("done", "(concat vl-width \\\"|\\\" vl-bits \\\"|\\\" vl-mbits \\\"|\\\" (if vl-memory vl-memory \\\"NIL\\\"))");
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "probe_1|||NIL")),
        "scalar port: vl-width=1, vl-bits/vl-mbits empty, vl-memory nil: {}",
        text
    );
}

#[test]
fn vl_vars_width_bits_numeric_range() {
    let text = m128_vl_probe("p2", "(concat vl-width \\\"|\\\" vl-bits)");
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p2", "probe_8|[7:0]")),
        "[7:0] port: vl-width=8, vl-bits=[7:0]: {}",
        text
    );
}

#[test]
fn vl_vars_width_symbolic_range_is_unevaluated_text() {
    let text = m128_vl_probe("p3", "(concat vl-width \\\"|\\\" vl-bits)");
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p3", "probe_WIDTH|[WIDTH-1:0]")),
        "[WIDTH-1:0] port: vl-width is the unevaluated \"WIDTH\" text, vl-bits the full range: {}",
        text
    );
}

#[test]
fn vl_vars_2d_packed_bits_is_last_dim_mbits_is_the_rest() {
    let text = m128_vl_probe(
        "p4",
        "(concat vl-width \\\"|\\\" vl-bits \\\"|\\\" vl-mbits)",
    );
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p4", "probe_8|[7:0]|[1:0]")),
        "[1:0][7:0] port: vl-bits is the LAST (innermost) dim, vl-mbits the rest: {}",
        text
    );
}

#[test]
fn vl_vars_unpacked_memory_dimension() {
    let text = m128_vl_probe(
        "mem",
        "(concat vl-width \\\"|\\\" vl-bits \\\"|\\\" vl-memory)",
    );
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "mem", "probe_8|[7:0]|[0:3]")),
        "`input [7:0] mem [0:3]': vl-width/vl-bits from the packed part, vl-memory the unpacked one: {}",
        text
    );
}

#[test]
fn vl_vars_zero_to_zero_range_not_collapsed() {
    let text = m128_vl_probe("z1", "(concat vl-width \\\"|\\\" vl-bits)");
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "z1", "probe_1|[0:0]")),
        "[0:0] port: vl-bits stays \"[0:0]\", not collapsed to empty like a true scalar: {}",
        text
    );
}

#[test]
fn vl_vars_dir_input_output_inout() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf2 (\n  input  logic a,\n  output logic b,\n  inout  logic c\n);\nendmodule\n\n/* leaf2 AUTO_TEMPLATE (\n  .\\(.*\\) (probe_@\"(concat vl-dir)\"),\n  ); */\nmodule top;\n  wire probe_input;\n  wire probe_output;\n  wire probe_inout;\n  leaf2 u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf2 u1 (".len());
    for (name, dir) in [("a", "input"), ("b", "output"), ("c", "inout")] {
        assert!(
            text.contains(&conn(&indent, name, &format!("probe_{}", dir))),
            "vl-dir for {} must be \"{}\": {}",
            name,
            dir,
            text
        );
    }
}

#[test]
fn vl_vars_modport_and_dir_on_interface_port() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf3 (\n  some_if.mst bus_i\n);\nendmodule\n\n/* leaf3 AUTO_TEMPLATE (\n  .bus_i (probe_@\"(concat vl-dir \\\"_\\\" vl-modport)\"),\n  ); */\nmodule top;\n  some_if bus_i();\n  wire probe_interface_mst;\n  leaf3 u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf3 u1 (".len());
    assert!(
        text.contains(&conn(&indent, "bus_i", "probe_interface_mst")),
        "an interface-typed port: vl-dir=\"interface\", vl-modport=\"mst\": {}",
        text
    );
}

#[test]
fn vl_vars_cell_name_excludes_array_range_cell_type_is_module_name() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf4 (\n  output logic done\n);\nendmodule\n\n/* leaf4 AUTO_TEMPLATE (\n  .done (probe_@\"(concat vl-cell-name \\\"_\\\" vl-cell-type)\"),\n  ); */\nmodule top;\n  wire [3:0] probe_u_bank_leaf4;\n  leaf4 u_bank[3:0] (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf4 u_bank[3:0] (".len());
    assert!(
        text.contains(&conn(&indent, "done", "probe_u_bank_leaf4")),
        "vl-cell-name must exclude the array range (\"u_bank\", not \"u_bank[3:0]\"); vl-cell-type is the module name: {}",
        text
    );
}

#[test]
fn vl_vars_the_five_unbound_names_stay_unbound() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(if (boundp (quote vl-signed)) \\\"BOUND\\\" \\\"OK\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire OK;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "OK")),
        "vl-signed (and by the same construction vl-decl/vl-type/vl-array/vl-bounds) must stay unbound: {}",
        text
    );
}

// --- Evaluation semantics --------------------------------------------------

#[test]
fn lisp_token_evaluated_once_per_port_monotonic_across_two_instances() {
    let (mut i, _ed) = setup();
    ok(&mut i, "(defvar m128-counter 0)");
    insert_src(
        &mut i,
        "module leaf5 (\n  input  logic a,\n  input  logic b,\n  output logic done\n);\nendmodule\n\n/* leaf5 AUTO_TEMPLATE (\n  .\\(.*\\) (cnt_@\"(progn (setq m128-counter (1+ m128-counter)) (number-to-string m128-counter))\"),\n  ); */\nmodule top;\n  wire cnt_1;\n  wire cnt_2;\n  wire cnt_3;\n  wire cnt_4;\n  wire cnt_5;\n  wire cnt_6;\n  leaf5 u1 (/*AUTOINST*/);\n  leaf5 u2 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    for n in 1..=6 {
        assert!(
            text.contains(&format!("(cnt_{})", n)),
            "the counter must advance monotonically 1..6 across every port of both instances, never reset or memoised: {}",
            text
        );
    }
}

// --- Error handling (section 2.5) -----------------------------------------

#[test]
fn error_non_string_result_falls_back_with_notice() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(quote foo)\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "a symbol result (not string/number/nil) must fall back to an identity connection: {}",
        text
    );
    assert!(
        text.contains("// Templated (expression failed)"),
        "the failed connection must carry the failure-flavored annotation: {}",
        text
    );
    assert!(
        msg.contains("AUTO_TEMPLATE lisp expression"),
        "the failure must be reported in the echo message: {}",
        msg
    );
}

#[test]
fn error_signal_falls_back_with_notice() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(error \\\"boom\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "an `error'-signaling expression must fall back to an identity connection, never abort the run: {}",
        text
    );
    assert!(
        msg.contains("AUTO_TEMPLATE lisp expression"),
        "the failure must be reported: {}",
        msg
    );
}

#[test]
fn error_unreadable_token_falls_back_with_notice() {
    // A raw `\' immediately before the closing quote (section 2.2's own
    // "also fatal" case) -- the extracted+unescaped text is an
    // incomplete form, `read-from-string' signals `end-of-file'.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat vl-name \\\"_a\\\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "an unreadable token must fall back to an identity connection, never abort the run: {}",
        text
    );
    assert!(
        msg.contains("AUTO_TEMPLATE lisp expression"),
        "the failure must be reported: {}",
        msg
    );
}

#[test]
fn error_lhs_token_dropped_other_rules_still_apply() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .@\"(concat \\\"p\\\" \\\"1\\\")\" (sig_x),\n  .p2 (real_p2),\n  ); */\nmodule top;\n  wire clk;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  wire real_p2;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    let msg = verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p2", "real_p2")),
        "a malformed LHS rule (`@\"...\"' on the port-name side) must be dropped, but every OTHER rule in the same template still applies: {}",
        text
    );
    assert!(
        !text.contains(".p1"),
        "the malformed rule must never resolve as a `.p1' connection (its own RHS `sig_x' spliced in) -- only the ORIGINAL template comment's own source text may contain \"sig_x\": {}",
        text
    );
    assert!(
        msg.contains("not recognized"),
        "the dropped rule must be reported via the existing AUTO_TEMPLATE parse-warning channel: {}",
        msg
    );
}

#[test]
fn error_one_bad_expression_does_not_abort_rest_of_file() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(error \\\"boom\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  wire other_done;\n  leaf u1 (/*AUTOINST*/);\n  leaf u2 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent1 = " ".repeat("  leaf u1 (".len());
    let indent2 = " ".repeat("  leaf u2 (".len());
    assert!(
        text.contains(&conn(&indent1, "clk", "clk")) && text.contains(&conn(&indent2, "clk", "clk")),
        "every OTHER port of BOTH instances must still expand normally when one expression fails on each: {}",
        text
    );
    assert!(
        text.contains(&conn(&indent1, "p2", "p2[7:0]")) && text.contains(&conn(&indent2, "p2", "p2[7:0]")),
        "AUTOINST for every OTHER instance in the file must not be aborted by one bad expression: {}",
        text
    );
}

// --- Annotation (section 2.4/2.5) ------------------------------------------

#[test]
fn annotation_failed_expression_vs_success() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module leaf6 (\n  input  logic a,\n  input  logic b,\n  output logic done\n);\nendmodule\n\n/* leaf6 AUTO_TEMPLATE (\n  .a (ok_@\"(concat \\\"x\\\")\"),\n  .b (@\"(error \\\"boom\\\")\"),\n  ); */\nmodule top;\n  wire ok_x;\n  wire b;\n  wire done;\n  leaf6 u1 (/*AUTOINST*/);\nendmodule\n",
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf6 u1 (".len());
    // Scoped to each connection's own generated line and checked with
    // `ends_with', not a fixed-spacing substring match against the
    // padding-dependent gap before the annotation -- the padding column
    // depends on the longest connection line in the SAME AUTOINST block
    // (`verilog-auto--pad-to-column'), which is incidental to what this
    // test pins and must not be baked into the expected string (fix
    // round: the original assertion only worked because `done'/`ok_x'
    // happen to tie for that maximum at four characters each).
    let a_line = text
        .lines()
        .find(|l| l.trim_start().starts_with(".a") && l.contains("Templated"))
        .unwrap_or_else(|| panic!("no generated `.a' connection line found: {}", text));
    assert!(
        a_line.contains(&conn(&indent, "a", "ok_x")) && a_line.ends_with("// Templated"),
        "the SUCCESSFUL `.a' connection's own line must carry the plain `// Templated' annotation, not the failure-flavored one: {:?}",
        a_line
    );
    let b_line = text
        .lines()
        .find(|l| l.trim_start().starts_with(".b") && l.contains("Templated"))
        .unwrap_or_else(|| panic!("no generated `.b' connection line found: {}", text));
    assert!(
        b_line.contains(&conn(&indent, "b", "b")) && b_line.ends_with("// Templated (expression failed)"),
        "the FAILED `.b' connection's own line must be the identity fallback `b' AND carry the ` (expression failed)' annotation: {:?}",
        b_line
    );
}

// --- Round trip -------------------------------------------------------------

#[test]
fn round_trip_successful_expression_is_byte_identical() {
    let (mut i, _ed) = setup();
    let src = format!(
        "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(concat \\\"finished\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire finished;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
        M128_LEAF
    );
    insert_src(&mut i, &src);
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    delete_auto(&mut i);
    let reverted = bs(&mut i);
    assert_eq!(
        reverted, src,
        "verilog-delete-auto must strip a successful lisp-expression connection back out byte-identically"
    );
    verilog_auto(&mut i);
    let expanded_again = bs(&mut i);
    assert_eq!(
        expanded, expanded_again,
        "re-running verilog-auto over the reverted buffer must reproduce the identical expansion"
    );
}

#[test]
fn round_trip_failed_expression_is_byte_identical() {
    let (mut i, _ed) = setup();
    let src = format!(
        "{}/* leaf AUTO_TEMPLATE (\n  .done (@\"(error \\\"boom\\\")\"),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
        M128_LEAF
    );
    insert_src(&mut i, &src);
    verilog_auto(&mut i);
    let expanded = bs(&mut i);
    delete_auto(&mut i);
    let reverted = bs(&mut i);
    assert_eq!(
        reverted, src,
        "verilog-delete-auto must strip a FAILED lisp-expression connection's identity fallback back out byte-identically too"
    );
    verilog_auto(&mut i);
    let expanded_again = bs(&mut i);
    assert_eq!(
        expanded, expanded_again,
        "re-running verilog-auto over the reverted buffer must reproduce the identical (still-failing) expansion"
    );
}

#[test]
fn result_conversion_nil_becomes_empty_string() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .done (pre_@\"nil\"_post),\n  ); */\nmodule top;\n  wire clk;\n  wire p2;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire pre__post;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "pre__post")),
        "`nil' must convert to the empty string, matching GNU's own `@\"nil\"' -> empty behavior (a bare `nil' atom evaluates to itself): {}",
        text
    );
}

// ===================== M128 Part B: unreachable AUTOINST sites =============
// `verilog-delete-auto' failing LOUDLY instead of silently returning
// `(0 . 0)' when a generated connection's own text made tree-sitter
// reclassify the whole enclosing statement, losing every `module_
// instantiation'/`hierarchical_instance' node for that site. See
// crates/core/lisp/verilog-auto.el's own M128 header correction and
// `verilog-auto--unreachable-autoinst-markers'.
//
// This predates M127 -- it is reproduced here with LITERAL, hand-typed
// connection text (no template, no `[]' token at all), not generated
// through `verilog-auto', matching the spec's own "not an M127
// regression" measurement.

const M128_PARTB_LEAF: &str = "\
module leaf (
  output logic done
);
endmodule

";

// M129 fix round: renamed from `unreachable_autoinst_string_adjacent_
// to_bracket_is_reported_not_silent' -- that name (and its own body)
// asserted the site stayed permanently stuck, untouched. M129's
// text-based fallback scanner now rescues exactly this shape, so the
// site deletes instead; see `m129_text_fallback_deletes_a_string_
// adjacent_to_bracket_site' below for the full rescue-shape coverage
// (including the return-value / element-3 assertions M128 could not
// have had). This test now only pins that the OLD M128 return-value
// slot narrowed correctly to 0 once M129 recovers the site.
#[test]
fn unreachable_autoinst_string_adjacent_to_bracket_is_now_rescued_by_text_fallback() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "the site deletes via the text fallback, zero left unrecovered: {}",
        msg
    );
    assert_eq!(
        bs(&mut i),
        format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_PARTB_LEAF
        ),
        "the rescued site must collapse to a bare marker, same as a tree-reachable site would"
    );
}

#[test]
fn unreachable_autoinst_verilog_delete_auto_echoes_its_own_message() {
    // `verilog-delete-auto's OWN `(message ...)' call (as opposed to
    // `verilog-auto's own final-message suffix, pinned separately by
    // `unreachable_autoinst_mixed_file_good_site_still_deletes_and_
    // reexpands') is a non-tail side effect -- its text never reaches
    // `verilog-delete-auto's own RETURN VALUE, so no test asserting that
    // return value (every other test in this section) can ever pin its
    // wording. Captured via `i.output', the same mechanism
    // `max_files_truncates_the_candidate_list_and_messages_once' already
    // uses in this crate for exactly this reason.
    //
    // M129 fix round: switched the input from the `.done ("W"[7:0])'
    // shape (M129's text fallback now rescues that one, so it no longer
    // exercises the "left untouched" message this test pins) to an
    // unterminated string, which the text fallback still cannot recover
    // (a genuine lexical error) -- see `m129_verilog_delete_auto_echoes_
    // the_text_recovered_count' below for the NEW message this test's
    // old input would now trigger instead.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"unterminated));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let captured: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_write = captured.clone();
    i.output = Some(Box::new(move |s| {
        captured_write.borrow_mut().push(s.to_string())
    }));
    delete_auto(&mut i);
    let out = captured.borrow().join("");
    assert!(
        out.contains("/*AUTOINST*/ marker(s) have no enclosing instantiation")
            && out.contains("left untouched"),
        "verilog-delete-auto's own echoed message must name the unreachable-site observation, not just the return value: {:?}",
        out
    );
    // Fix round (cold review): this assertion previously only checked
    // substrings that already existed in the pre-M129 wording, so
    // reverting M129's own reworded clause ("a text-based fallback scan
    // was tried and could not recover them either...") went unnoticed --
    // confirmed by experiment (reverting the wording still left this
    // test green). Pin the new phrase explicitly, same gap M128's own
    // trailing review found once already.
    assert!(
        out.contains("a text-based fallback scan was tried and could not recover them either"),
        "verilog-delete-auto's own echoed message must say the text fallback was ATTEMPTED and failed, not just that the site was left untouched: {:?}",
        out
    );
}

#[test]
fn unreachable_autoinst_unterminated_string_is_reported_not_silent() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"unterminated));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "an unterminated string literal must trip the SAME unreachable-site accounting, and the M129 text fallback must ALSO refuse it (lexical error): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "left completely untouched");
}

#[test]
fn unreachable_autoinst_negative_control_unbalanced_paren_recovers_locally() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done ((unbal));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 0)",
        "an unbalanced paren recovers LOCALLY (module_instantiation still found) -- must NOT be counted as unreachable: {}",
        msg
    );
}

#[test]
fn unreachable_autoinst_negative_control_unbalanced_bracket_recovers_locally() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (unbal[));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 0)",
        "an unbalanced bracket recovers LOCALLY -- must NOT be counted as unreachable: {}",
        msg
    );
}

#[test]
fn unreachable_autoinst_negative_control_backtick_recovers_locally() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (`unbal));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 0)",
        "a stray backtick recovers LOCALLY -- must NOT be counted as unreachable: {}",
        msg
    );
}

#[test]
fn unreachable_autoinst_negative_control_hash_recovers_locally() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (#unbal));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 0)",
        "a lone `#' recovers LOCALLY -- must NOT be counted as unreachable: {}",
        msg
    );
}

#[test]
fn unreachable_autoinst_negative_control_bare_keyword_recovers_locally() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (endmodule));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 0)",
        "a bare keyword recovers LOCALLY -- must NOT be counted as unreachable: {}",
        msg
    );
}

// M129 fix round: `.done ("W"[7:0])' -- the exact shape this test used
// to demonstrate a PERMANENTLY stuck site -- is now rescued by the
// text-based fallback scanner (see `m129_text_fallback_deletes_a_
// string_adjacent_to_bracket_site' below), so both sites in this file
// now delete. Renamed from `unreachable_autoinst_mixed_file_good_site_
// still_deletes_and_reexpands' (that name asserted the u1 site stayed
// untouched, which is no longer true) to describe what actually
// happens now. `m129_mixed_file_good_and_stuck_sites_both_delete'
// below covers the genuinely-unrecoverable-stuck-site case this test
// used to (with an unterminated string, which text fallback still
// cannot rescue).
#[test]
fn unreachable_autoinst_mixed_file_good_and_fallback_rescued_sites_both_delete_and_reexpand() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\n  leaf u2 (/*AUTOINST*/);\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(2 0 0 1)",
        "the good (tree-reachable) site plus the fallback-rescued site both deleted, none left unrecovered: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u2 (/*AUTOINST*/)"),
        "the GOOD site's own generated connection must have been deleted back to a bare marker: {}",
        text
    );
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/)"),
        "the fallback-RESCUED site's own generated connection must also have been deleted back to a bare marker: {}",
        text
    );
    assert!(
        !text.contains(".done (\"W\"[7:0])"),
        "the fallback-rescued site's own stuck connection text must be gone: {}",
        text
    );
    // Both sites must still re-expand normally afterward.
    verilog_auto(&mut i);
    let text2 = bs(&mut i);
    let indent = " ".repeat("  leaf u2 (".len());
    assert!(
        text2.contains(&conn(&indent, "done", "done")),
        "both sites must still re-expand normally via verilog-auto: {}",
        text2
    );
}

// `verilog-auto's own final message can only fold in the text-recovered
// notice on a call whose OWN internal `verilog-delete-auto' invocation
// still finds the site unreachable -- the test above already deleted
// (and rescued) the site via its own explicit `delete_auto' call before
// ever calling `verilog_auto', so by the time `verilog_auto' runs, the
// buffer's marker is already a bare, tree-reachable `/*AUTOINST*/' and
// there is nothing left to rescue. This is a fresh buffer instead, so
// `verilog_auto's own FIRST internal delete pass is the one that hits
// the stuck site.
#[test]
fn unreachable_autoinst_verilog_auto_final_message_folds_in_text_recovered_notice() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let reexpanded = verilog_auto(&mut i);
    assert!(
        reexpanded.contains("recovered by a text-based fallback scan"),
        "verilog-auto's own final message must fold in the text-recovered notice: {}",
        reexpanded
    );
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "done", "done")),
        "the rescued site must still re-expand normally via verilog-auto: {}",
        text
    );
}

// --- Part A / Part B interaction (fix round, cold review finding) ---------
// Part A can *manufacture* a new instance of the exact defect Part B exists
// to surface: an ordinarily-SUCCESSFUL lisp expression whose result string
// contains a literal `"' immediately adjacent to a `[]'/`[][]' token in the
// same rule produces generated text shaped exactly like Part B's own known
// trigger (`.p2 ("W"[7:0])'), and nothing in Part A validates or rejects
// this -- by design (adding such a check would invent a restriction GNU
// itself does not have). M128 pinned that the resulting site was reported
// through Part B's unreachable-count mechanism, not silently lost, and
// left it PERMANENTLY stuck expanded. M129 fix round: renamed from
// `lisp_result_embedding_a_quote_adjacent_to_bracket_becomes_a_part_b_
// unreachable_site' (that name asserted the site stayed unreachable/
// stuck; M129's text-based fallback scanner now rescues this shape too,
// same as the direct M128 case) -- this test now pins that a
// Part-A-MANUFACTURED instance of the shape gets the SAME rescue as a
// hand-typed one.

#[test]
fn lisp_result_embedding_a_quote_adjacent_to_bracket_is_rescued_by_text_fallback() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}/* leaf AUTO_TEMPLATE (\n  .p2 (@\"(concat (char-to-string 34) (char-to-string 87) (char-to-string 34))\"[]),\n  ); */\nmodule top;\n  wire clk;\n  wire p3;\n  wire p4;\n  wire mem;\n  wire z1;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_LEAF
        ),
    );
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent = " ".repeat("  leaf u1 (".len());
    assert!(
        text.contains(&conn(&indent, "p2", "\"W\"[7:0]")),
        "the lisp expression succeeds (result \"W\" spliced next to the `[]' token's own [7:0]), producing exactly Part B's own known-rescuable shape: {}",
        text
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "the Part-A-manufactured site must be rescued and deleted by the text fallback, zero left unrecovered: {}",
        msg
    );
    assert!(
        !bs(&mut i).contains("\"W\"[7:0]"),
        "the rescued site's own stuck connection text must be gone: {}",
        bs(&mut i)
    );
}

#[test]
fn unreachable_autoinst_stray_marker_with_no_enclosing_instantiation() {
    // A stray, hand-written `/*AUTOINST*/' comment that simply is not
    // inside any instantiation at all (never wrapped in a `module_
    // instantiation'/`hierarchical_instance' by anything -- no parse
    // error involved whatsoever) produces the IDENTICAL "no enclosing
    // instantiation" observation as the parse-error shapes above. Fix
    // round (cold review): the message must state the observation, not
    // assert an unverified "parse error" cause -- this is the shape that
    // makes the difference visible.
    let (mut i, _ed) = setup();
    insert_src(&mut i, "module top;\n  /*AUTOINST*/\nendmodule\n");
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "a stray marker with no enclosing instantiation must still be counted via the SAME unreachable-site mechanism, and the M129 text fallback cannot rescue it either (no enclosing paren pair at all): {}",
        msg
    );
    assert_eq!(
        bs(&mut i),
        src_before,
        "a stray marker must be left completely untouched, not crash or delete anything"
    );
}

// === M129: a text-based fallback scanner for `verilog-delete-auto' ========
//
// Every test below exercises `verilog-auto--lex-paren-pairs' /
// `verilog-auto--instantiation-shaped-p' / `verilog-auto--text-fallback-
// range' through `verilog-delete-auto's own public surface -- none of
// those helpers is called directly. `M128_PARTB_LEAF' (a single-port
// `leaf' module, declared above) is reused throughout: the fallback
// scanner is purely textual and never validates connection text against
// a module's own real port list, so an arbitrary `.NAME (...)' fragment
// inside the parens is enough to exercise its lexical rules.
//
// The `.dat/.done ("W"[7:0])' shape is this whole milestone's own
// established unreachable-trigger (M128 recon): it reclassifies the
// WHOLE enclosing statement, so `verilog-delete-auto's tree-based path
// never reaches these sites at all, and every test below that wants its
// site to go through the FALLBACK path (as opposed to the ordinary tree
// path) includes it somewhere in the connection list.

#[test]
fn m129_text_fallback_deletes_a_string_adjacent_to_bracket_site() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "deleted via the text fallback (element 3), zero left unrecovered (element 2): {}",
        msg
    );
    assert_eq!(
        bs(&mut i),
        format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/);\nendmodule\n",
            M128_PARTB_LEAF
        ),
        "the whole feature deleted: this is the deletion-question test -- delete `verilog-delete-auto's M129 wiring entirely and this goes red"
    );
}

#[test]
fn m129_text_fallback_site_round_trips_through_verilog_auto() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\n  leaf u2 (/*AUTOINST*/);\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    delete_auto(&mut i);
    verilog_auto(&mut i);
    let text = bs(&mut i);
    let indent1 = " ".repeat("  leaf u1 (".len());
    let indent2 = " ".repeat("  leaf u2 (".len());
    assert!(
        text.contains(&conn(&indent1, "done", "done")),
        "the rescued site (u1) must re-expand to the SAME connection text as a healthy site: {}",
        text
    );
    assert!(
        text.contains(&conn(&indent2, "done", "done")),
        "the always-healthy site (u2) re-expands identically, confirming the two are indistinguishable after the round trip: {}",
        text
    );
}

#[test]
fn m129_text_fallback_leaves_text_before_the_marker_untouched() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire my_done;\n  leaf u1 (.done (my_done),\n           /*AUTOINST*/\n           .dat (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "one site, rescued by the text fallback: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (.done (my_done),\n           /*AUTOINST*/);"),
        "the hand-written connection BEFORE the marker must survive untouched, and deletion must start exactly at the marker's own end: {}",
        text
    );
    assert!(
        !text.contains(".dat"),
        "the generated connection AFTER the marker must be gone: {}",
        text
    );
}

#[test]
fn m129_text_fallback_preserves_trailing_text_after_the_close_paren() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/ .dat (\"W\"[7:0])); // tail\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "one site, rescued by the text fallback: {}",
        msg
    );
    assert!(
        bs(&mut i).contains("leaf u1 (/*AUTOINST*/); // tail"),
        "text past the close paren on the same line must survive untouched: {}",
        bs(&mut i)
    );
}

#[test]
fn m129_text_fallback_strips_the_trailing_templated_annotation() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (\"W\"[7:0]));  // Templated\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    // Element 0 (DELETED-COUNT) counts RANGES, not sites -- a templated
    // site pushes TWO ranges (the ports region plus the separate
    // annotation range, same as the tree path already does), so this is
    // 2 here even though only ONE site was rescued (element 3).
    assert_eq!(
        msg, "(2 0 0 1)",
        "one site, rescued by the text fallback, contributing two ranges (ports region + annotation): {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);\n"),
        "the rescued site must collapse to a bare marker: {}",
        text
    );
    assert!(
        !text.contains("Templated"),
        "the trailing `// Templated' annotation must be stripped, same as the tree path already does: {}",
        text
    );
}

#[test]
fn m129_text_fallback_handles_several_stuck_sites_in_one_file() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\n  leaf u2 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(2 0 0 2)",
        "both stuck sites rescued and deleted by the text fallback: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "u1 collapsed: {}",
        text
    );
    assert!(
        text.contains("leaf u2 (/*AUTOINST*/);"),
        "u2 collapsed: {}",
        text
    );
}

#[test]
fn m129_mixed_file_good_and_stuck_sites_both_delete() {
    // Divergence 1 (spec section 2): GNU aborts its ENTIRE command when
    // one site's own text scan fails -- the good site is left untouched
    // too (measured, spec section 1d). reticle must lose neither.
    //
    // An UNTERMINATED string (tried first) turned out to be a bad probe
    // for this specific test: with no closing `"' anywhere for the rest
    // of the buffer either, tree-sitter's own recovery swallows u2's
    // statement into the same broken parse too, so u2 stops being
    // tree-reachable in THAT construction -- not what this test wants to
    // exercise. A missing close paren on u1's own instantiation is
    // lexically broken in the SAME sense (the text-based fallback's own
    // step 1 refuses it, no CLOSE) but measured to stay LOCAL, leaving
    // u2's own statement intact and tree-reachable.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]);\n  leaf u2 (/*AUTOINST*/);\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 1 0)",
        "u2 (good, tree path) deletes; u1 (no matching close paren) stays unrecovered -- neither blocks the other: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains(".done (\"W\"[7:0])"),
        "u1's own stuck text must be left completely untouched: {}",
        text
    );
}

#[test]
fn m129_text_fallback_ignores_a_close_paren_inside_a_string() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (\"close ) paren in string\"),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite the `)' inside a string literal: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_ignores_parens_inside_a_line_comment() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (a), // trailing ) paren in comment\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite the `)' inside a line comment: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_ignores_parens_inside_a_block_comment() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (a), /* block ) comment */\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite the `)' inside a block comment: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_honours_a_backslash_escape_inside_a_string() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (\"a\\\") b\"),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite an escaped quote (`\\\"') inside a string not ending it early: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_balances_nested_parens_in_a_connection() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat ((a & b) | c),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite nested, balanced parens in a connection: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_ignores_parens_inside_an_escaped_identifier() {
    // The escaped identifier here is `\sig)x', whose embedded paren is
    // deliberately UNBALANCED. The first version of this test used
    // `\u1(0) ' -- the shape a real gate-level netlist actually contains
    // -- and that made the test WORTHLESS: with the escaped-identifier
    // branch of `verilog-auto--lex-paren-pairs' disabled entirely, the
    // `(' and `)' of `\u1(0) ' still balance each other out, the depth
    // accounting lands in exactly the same place, and the test stayed
    // GREEN. Caught by `dev/mutations/m129.py' D5 coming back SURVIVED
    // and then failing the two-part check: the mutation landed, it was
    // just semantically inert against this fixture. An unbalanced paren
    // is what actually distinguishes "the identifier's contents are not
    // lexed" from "they happen to cancel out" -- without the feature the
    // `)' inside the name closes `.dat (' early, `leaf u1 (' then closes
    // at the following `)', and the deleted region ends in the wrong
    // place. Escaped identifiers run to the next whitespace and may
    // legally contain any printable character, this one included.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (\\sig)x ),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `(' and `)' embedded in an escaped identifier's own name: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

// Fix round (cold review): a carriage return did not used to end an
// escaped identifier's own run (only `?\s'/`?\t'/`?\n' did), so a `\r'
// right after one (a CRLF-terminated line, or a bare CR) would be
// consumed as PART of the identifier's own name, along with everything
// up to the next LF -- including, here, the `)' that closes the
// connection's own paren. Mutation-tested (reverted, not left in the
// tree): dropping `?\r' from `verilog-auto--fallback-skip-escaped-
// identifier's own whitespace set turns this test's own result from
// `(1 0 0 1)' into a refusal.
#[test]
fn m129_text_fallback_escaped_identifier_ends_at_a_carriage_return() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (\\u1(0)\r),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `\\r' right after an escaped identifier, before the connection's own close paren: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

// Fix round (cold review): the bracket-group sub-scan used to count `['/
// `]' over raw text only, unlike the top-level scanner -- a `]' inside a
// string (or a comment) INSIDE a bracket group ended the group early,
// corrupting everything lexed afterward. Mutation-tested (reverted, not
// left in the tree): reverting the bracket sub-scan to the old raw-text
// version turns this test's own result from `(1 0 0 1)' into a refusal
// (the corrupted lexing desynchronizes the rest of the connection list,
// and the outer instantiation's own pair no longer closes cleanly).
#[test]
fn m129_text_fallback_ignores_a_close_bracket_inside_a_string_within_a_bracket_group() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (mem[\"]\"]),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `]' inside a string nested inside a bracket group: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

// Second fix round (trailing re-review): the string branch above was the
// only one of the bracket sub-scan's four sub-branches (escaped
// identifier / string / line comment / block comment) with a fixture --
// deleting any of the OTHER three and letting it fall through to the
// catch-all `(t (setq j (1+ j)))' made no named test go red. These three
// close that gap, one per remaining branch. Each embedded `]' is
// load-bearing BY CONSTRUCTION (not a balanced pair that happens to
// cancel out, the M83/Mutation-D5 trap this project already has a rule
// about): without the corresponding branch, that specific `]' is the
// character that closes the bracket group EARLY, at the wrong position,
// leaving real content stranded outside it and desynchronizing the rest
// of the lex -- each was hand-verified by literally reverting the
// branch and confirming the test goes red (see the coordinator's own
// report for the raw output).

// The fixture is `mem[\a]b(c d]' and TWO characters are load-bearing
// together: the FIRST `]' (right after `\a') is what a broken scan
// would wrongly treat as the bracket's own close -- an escaped
// identifier inside a bracket group still ends at WHITESPACE, not at
// `]', so `\a]b(c' is legally ONE token, and the space before `d' is
// what actually ends it. The `(' inside that same token
// (`\a]b(c') is what makes a wrong early close OBSERVABLE rather than
// inert: without the escaped-identifier branch, the bracket sub-scan's
// catch-all stops the group right after that first `]', stranding
// `b(c d]),' outside it -- and THAT `(' then gets read by the OUTER
// loop as a genuine paren-open, pushing a bogus pair that steals the
// NEXT real `)' (the one that should have closed `.dat('s own paren),
// which cascades into `leaf u1 ('s own outer pair never finding ITS
// matching close either. Two earlier, simpler fixtures were tried and
// rejected before this one: `mem[\a]b 0]' (digits right after the
// space) never even reaches the fallback -- with a digit there,
// tree-sitter's own GLR recovery finds a `hierarchical_instance' for u1
// regardless of the `"W"[7:0]' trigger elsewhere in the connection
// list, so the site stays tree-reachable and the fallback lexer is
// never invoked (msg was `(1 0 0 0)', deleted via the ORDINARY tree
// path). `mem[\a]b xxx]' (a trailing word, no embedded paren) DOES
// reach the fallback and DOES get rescued either way, but the mutation
// SURVIVED against it -- the misplaced bracket boundary changes which
// TOKENS get recorded, but with no real paren character anywhere in the
// stranded leftover text, the outer paren-matching (the only thing
// `verilog-delete-auto's own return value depends on) comes out
// identical regardless, exactly the inert-mutation shape this project
// already has a rule about (see mutation D5 elsewhere in this
// milestone). Confirmed by hand for THIS fixture: reverting the
// escaped-identifier branch turns the result from `(1 0 0 1)' into
// `(0 0 1 0)' (refused).
#[test]
fn m129_text_fallback_ignores_a_close_bracket_inside_an_escaped_identifier_within_a_bracket_group()
{
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (mem[\\a]b(c d]),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `]' embedded in an escaped identifier nested inside a bracket group: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

// The fixture is `mem[7:0 // stray ] in (comment\n]' and, as with the
// escaped-identifier fixture above, TWO characters matter together: the
// `]' right after `stray ' is what a broken scan wrongly treats as the
// bracket's own close (without the line-comment branch, the bracket
// sub-scan's catch-all treats `//' as two ordinary characters and hits
// this `]' first); the UNBALANCED `(' in ` in (comment' right after it
// is what makes that wrong close OBSERVABLE. A first version of this
// fixture used a plain `// stray ] in comment' with no paren at all --
// the mutation LANDED (it does change which characters get treated as
// a comment) but SURVIVED (no test went red), because with no real
// paren character anywhere in the stranded leftover text, the outer
// paren-matching `verilog-delete-auto's own return value actually
// depends on came out identical either way -- the same inert-mutation
// shape mutation D5 elsewhere in this milestone already hit. Here the
// stray `(' has no matching `)' before the buffer's own next real one,
// so once stranded outside the (wrongly-shortened) bracket it steals
// that real `)' -- the one that should have closed `.dat('s own paren
// -- leaving `.dat('s pair (and, cascading from there, `leaf u1 ('s own
// outer pair) without a matching close. Confirmed by hand: reverting
// the line-comment branch turns the result from `(1 0 0 1)' into
// `(0 0 1 0)' (refused).
#[test]
fn m129_text_fallback_ignores_a_close_bracket_inside_a_line_comment_within_a_bracket_group() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (mem[7:0 // stray ] in (comment\n]),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `]' inside a line comment nested inside a bracket group: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

// The fixture is `mem[7:0 /* stray ] in (comment */]' -- same two-
// character shape as the line-comment fixture above, for the same
// reason (a first version with a plain `/* stray ] in comment */' and
// no paren also LANDED but SURVIVED: the misplaced bracket boundary
// changed which characters got treated as a comment, but with no real
// paren character anywhere in the stranded leftover text, the outer
// paren-matching came out identical either way). The `]' right after
// `stray ' is what a broken scan wrongly treats as the bracket's own
// close (without the block-comment branch, the bracket sub-scan's
// catch-all treats `/*' as two ordinary characters and hits this `]'
// first); the UNBALANCED `(' in ` in (comment' right after it is what
// makes that wrong close OBSERVABLE -- once stranded outside the
// (wrongly-shortened) bracket, it steals the real `)' that should have
// closed `.dat('s own paren, cascading into `leaf u1 ('s own outer pair
// losing its own matching close too. Confirmed by hand: reverting the
// block-comment branch turns the result from `(1 0 0 1)' into
// `(0 0 1 0)' (refused).
#[test]
fn m129_text_fallback_ignores_a_close_bracket_inside_a_block_comment_within_a_bracket_group() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .dat (mem[7:0 /* stray ] in (comment */]),\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "rescued despite a `]' inside a block comment nested inside a bracket group: {}",
        msg
    );
    let text = bs(&mut i);
    assert!(
        text.contains("leaf u1 (/*AUTOINST*/);"),
        "collapsed to a bare marker: {}",
        text
    );
    assert!(!text.contains(".dat"), "the whole region deleted: {}", text);
}

#[test]
fn m129_text_fallback_refuses_a_marker_inside_a_module_header() {
    // The data-corruption test: falsely accepting here would delete a
    // module's own hand-written port list.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top (\n  /*AUTOINST*/\n  input logic clk,\n  input logic rst\n);\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the guard must refuse a module header's own port-list parens (`module'/`top' fail the whitelist): {}",
        msg
    );
    assert_eq!(
        bs(&mut i),
        src_before,
        "the header's own port declarations must be byte-identical afterward"
    );
}

#[test]
fn m129_text_fallback_refuses_a_marker_inside_a_for_loop_paren() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  integer i;\n  initial begin\n    for (/*AUTOINST*/ i = 0; i < 8; i = i + 1) begin\n    end\n  end\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the guard must refuse a `for' loop's own parens (`for' fails the whitelist): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_text_fallback_refuses_a_marker_inside_an_always_sensitivity_list() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  reg q;\n  wire clk;\n  always @(/*AUTOINST*/ posedge clk) begin\n    q <= 1'b0;\n  end\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the guard must refuse an `always' sensitivity list's own parens (`@' is not a plain identifier): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_text_fallback_refuses_a_marker_inside_a_function_argument_list() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  function foo(/*AUTOINST*/ input a, input b);\n    foo = a;\n  endfunction\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the guard must refuse a function argument list's own parens (`function' fails the whitelist): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

// Fix round (cold review): `let NAME(args) = expr;' tokenizes as
// `"let" "NAME" "("' -- `let' was absent from `verilog-auto--fallback-
// keywords', so before this fix round the guard accepted it as shape 1
// (`MODULE INSTANCE ('), which would delete a `let' declaration's own
// argument list mistaking it for a stuck AUTOINST site's connections.
// Mutation-tested (reverted, not left in the tree): removing `let' from
// the keyword list turns this test's own result from `(0 0 1 0)' to
// `(1 0 0 1)' (wrongly rescued and deleted).
#[test]
fn m129_text_fallback_refuses_a_marker_inside_a_let_declaration() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  let my_let(/*AUTOINST*/ a, b) = a + b;\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the guard must refuse a `let' declaration's own argument list (`let' fails the whitelist): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_text_fallback_accepts_a_parameterised_instantiation() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf #(.W(8)) u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "shape 2 (`MODULE #(...) INSTANCE (') accepted: {}",
        msg
    );
    assert!(
        bs(&mut i).contains("leaf #(.W(8)) u1 (/*AUTOINST*/);"),
        "rescued and collapsed: {}",
        bs(&mut i)
    );
}

#[test]
fn m129_text_fallback_accepts_an_instance_array() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1[3:0] (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(1 0 0 1)",
        "an instance-array bracket group accepted: {}",
        msg
    );
    assert!(
        bs(&mut i).contains("leaf u1[3:0] (/*AUTOINST*/);"),
        "rescued and collapsed: {}",
        bs(&mut i)
    );
}

#[test]
fn m129_text_fallback_refuses_an_unterminated_string_site() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"unterminated));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "an unterminated string is a genuine lexical error -- GNU also fails here (spec 1d) -- stays unrecovered: {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_text_fallback_refuses_a_site_with_no_matching_close_paren() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]);\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "the instantiation's own `(' is never closed anywhere in the buffer -- no CLOSE, so step 1 of the fallback's own range search returns nil: {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

// Fix round (cold review): every unterminated-string fixture elsewhere
// in this milestone (`m129_text_fallback_refuses_an_unterminated_
// string_site', `m129_text_fallback_refuses_a_site_with_no_matching_
// close_paren', `unreachable_autoinst_unterminated_string_is_reported_
// not_silent') puts the broken string INSIDE the instantiation's own
// connection list, so BOTH the connection's own paren and the
// instantiation's own outer paren end up with no CLOSE at all (the `)'
// characters that would have closed them are consumed as part of the
// broken string) -- `verilog-auto--text-fallback-range' returns from
// its OWN FIRST cond clause, `((not best) nil)', in every one of those
// cases, and the SEPARATE `:lex-error' clause is never reached. Mutation-
// tested (reverted, not left in the tree): deleting the `:lex-error'
// clause left all three of those tests green.
//
// This fixture is different: the lexical anomaly (an unterminated
// `string s = "oops;' statement) sits BEFORE and has nothing to do with
// u1's own instantiation, which is individually well-formed and
// otherwise exactly the known-rescuable `"W"[7:0]' shape -- its own
// pair gets a real, correctly-matched CLOSE. The site must still be
// refused, because the lexer's own `:lex-error' index (set by the
// EARLIER anomaly) sits before this pair's own CLOSE -- the anomaly
// poisons trust in everything the lexer saw afterward, even a pair that
// looks individually fine. Mutation-tested the same way: deleting the
// `:lex-error' clause turns this fixture's own result from `(0 0 1 0)'
// to `(1 0 0 1)' (wrongly rescued).
#[test]
fn m129_text_fallback_refuses_a_well_formed_site_after_an_earlier_lexical_anomaly() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  string s = \"oops;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
    );
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "u1's own pair is individually well-formed, but an EARLIER, unrelated lexical anomaly must still refuse it (the anomaly's own index precedes u1's own CLOSE): {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_stray_marker_with_no_enclosing_paren_is_still_reported_not_deleted() {
    let (mut i, _ed) = setup();
    insert_src(&mut i, "module top;\n  /*AUTOINST*/\nendmodule\n");
    let src_before = bs(&mut i);
    let msg = delete_auto(&mut i);
    assert_eq!(
        msg, "(0 0 1 0)",
        "no enclosing pair exists at all -- step 1 of the fallback's own range search returns nil immediately: {}",
        msg
    );
    assert_eq!(bs(&mut i), src_before, "must be left completely untouched");
}

#[test]
fn m129_verilog_delete_auto_echoes_the_text_recovered_count() {
    // M128's own trailing review found this exact gap once already (a
    // message asserted by no test) -- this test exists so it is not
    // repeated for M129's own new message.
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let captured: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_write = captured.clone();
    i.output = Some(Box::new(move |s| {
        captured_write.borrow_mut().push(s.to_string())
    }));
    delete_auto(&mut i);
    let out = captured.borrow().join("");
    assert!(
        out.contains("recovered and deleted by a text-based fallback scan"),
        "verilog-delete-auto's own echoed message must name the text-recovered count: {:?}",
        out
    );
}

#[test]
fn m129_verilog_auto_final_message_folds_in_the_text_recovered_count() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        &format!(
            "{}module top;\n  wire done;\n  leaf u1 (/*AUTOINST*/\n           .done (\"W\"[7:0]));\nendmodule\n",
            M128_PARTB_LEAF
        ),
    );
    let reexpanded = verilog_auto(&mut i);
    assert!(
        reexpanded.contains("recovered by a text-based fallback scan"),
        "verilog-auto's own final message must fold in the text-recovered count: {}",
        reexpanded
    );
}
