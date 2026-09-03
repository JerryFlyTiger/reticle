//! M54: Verilog port-name completion (`verilog-complete.el`) -- the
//! `local-completion-function' tier `completion-at-point' (lsp.el) tries
//! before LSP/dabbrev, plus the LSP-side `completionProvider' capability
//! gate (`lsp--capability-supported-p') that keeps a connected buffer
//! from silently sending a request `verible-verilog-ls' has no handler
//! for at all. See verilog-complete.el's own header for the M54 repro
//! this milestone is built against and the full scope-cut list.

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

fn insert_src(interp: &mut Interp, src: &str) {
    ok(interp, &format!("(insert {:?})", src));
}

/// A fresh scratch directory under the OS temp dir, unique per test run
/// (mirrors `verilog_auto_tests.rs`'s/`lsp_mode_tests.rs`'s own helper).
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
            "reticle_verilog_complete_{}_{}_{}",
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

/// Every popup item's own (label, insert, start, filter), read straight
/// off `Editor::completion_popup` -- same approach `completion_popup_
/// tests.rs` uses (no elisp-level accessor exposes item contents, only
/// `completion-popup-active-p`).
fn popup_items(ed: &Rc<RefCell<Editor>>) -> Option<Vec<(String, String, usize, String)>> {
    let e = ed.borrow();
    e.completion_popup.as_ref().map(|p| {
        p.items
            .iter()
            .map(|it| {
                (
                    it.label.clone(),
                    it.insert.clone(),
                    it.start,
                    it.filter.clone(),
                )
            })
            .collect()
    })
}

fn insert_names(items: &[(String, String, usize, String)]) -> Vec<String> {
    items.iter().map(|it| it.1.clone()).collect()
}

/// Moves point to right after the FIRST (from `point-min`) occurrence of
/// NEEDLE, via `search-forward` -- real Emacs point semantics, sidesteps
/// any manual byte-offset-vs-1-based-point arithmetic entirely.
fn goto_after(interp: &mut Interp, needle: &str) {
    ok(interp, "(goto-char (point-min))");
    ok(interp, &format!("(search-forward {:?})", needle));
}

// ============================================================
// 1. Port context, module defined in the SAME buffer
// ============================================================

#[test]
fn same_buffer_module_offers_its_own_port_names() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr );\nendmodule\n\nmodule fifo (input wr_en, input rd_en, output [7:0] dout);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".wr"); // right after "wr"
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "port context must be handled (t): {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(names, vec!["wr_en".to_string()]);
    assert!(
        items[0].0.contains("input"),
        "label should show direction: {}",
        items[0].0
    );
}

// ============================================================
// 1a. M97: `.'-port completion inside an INTERFACE instantiation's own
//     connection list -- before this milestone, `verilog-auto--find-
//     module-in-buffer' only ever matched `module_declaration', so
//     resolving "axi_if" here came back nil and completion silently fell
//     through to dabbrev instead of offering the interface's own ports.
// ============================================================

#[test]
fn same_buffer_interface_instantiation_offers_its_own_port_names() {
    let (mut i, ed) = setup();
    let src = "module top;\n  axi_if u_if ( .cl );\nendmodule\n\ninterface axi_if (input clk, input rst_n, output valid);\nendinterface\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".cl"); // right after "cl"
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "port context must be handled (t): {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(names, vec!["clk".to_string()]);
    assert!(
        items[0].0.contains("input"),
        "label should show direction: {}",
        items[0].0
    );
}

// ============================================================
// 2. Port context, module defined in a LIBRARY directory file
// ============================================================

#[test]
fn library_file_module_offers_its_own_port_names() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("lib_basic");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        // A bare "." (empty typed prefix): every port of `fifo' should
        // be offered, proving this isn't just an accidental single-name
        // prefix match.
        "module top;\n  fifo u_fifo ( . );\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module fifo (input wr, input rd, output [7:0] dout);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "( .");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "port context in a cross-file lookup: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["dout".to_string(), "rd".to_string(), "wr".to_string()],
        "dabbrev cannot do this -- sub.sv is a different buffer entirely"
    );
}

// ============================================================
// 2a. M56: module in a recursively-scanned library subdirectory
// ============================================================

#[test]
fn library_module_in_a_subdirectory_offers_its_own_port_names() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("m56_lib_recurse");
    std::fs::create_dir_all(dir.join("core")).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("core").join("sub.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_fifo ( . );\nendmodule\n").unwrap();
    std::fs::write(
        &sub_path,
        "module fifo (input wr, input rd, output [7:0] dout);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "( .");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "port context via a recursed-into subdirectory: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["dout".to_string(), "rd".to_string(), "wr".to_string()],
        "sub.sv lives in core/, only reachable by M56's recursive scan"
    );
}

// ============================================================
// 2b. M56: module only reachable via `verible.filelist'
// ============================================================

#[test]
fn library_module_reachable_only_via_verible_filelist_offers_its_own_port_names() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("m56_lib_filelist");
    let proj_dir = dir.join("proj");
    let other_dir = dir.join("other");
    std::fs::create_dir_all(&proj_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    let top_path = proj_dir.join("top.sv");
    let sub_path = other_dir.join("sub.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_fifo ( . );\nendmodule\n").unwrap();
    std::fs::write(
        &sub_path,
        "module fifo (input wr, input rd, output [7:0] dout);\nendmodule\n",
    )
    .unwrap();
    // `verible.filelist' is itself one of `lsp--project-root-markers', so
    // dropping it in `proj/' makes `proj/' the discovered project root;
    // `other/' is a sibling `verilog-library-directories' (default `(".")')
    // can never reach on its own.
    std::fs::write(proj_dir.join("verible.filelist"), "../other/sub.sv\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "( .");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "port context via verible.filelist: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["dout".to_string(), "rd".to_string(), "wr".to_string()],
        "sub.sv is only named by verible.filelist, not any library directory"
    );
}

// ============================================================
// 3. Prefix filtering
// ============================================================

#[test]
fn prefix_filters_to_matching_ports_only() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr_en );\nendmodule\n\nmodule fifo (input wr_en, input wr_data, input rd_en);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".wr_en");
    ok(&mut i, "(verilog-complete-at-point)");
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["wr_en".to_string()],
        "only wr_en itself starts with the full typed prefix \"wr_en\": {:?}",
        names
    );
}

#[test]
fn shorter_prefix_still_narrows_but_keeps_multiple_matches() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr );\nendmodule\n\nmodule fifo (input wr_en, input wr_data, input rd_en);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".wr");
    ok(&mut i, "(verilog-complete-at-point)");
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["wr_data".to_string(), "wr_en".to_string()],
        "both wr_-prefixed ports, rd_en excluded: {:?}",
        names
    );
}

// ============================================================
// 4. Non-port context -> nil, falls through to dabbrev
// ============================================================

#[test]
fn non_port_identifier_returns_nil_and_falls_through_to_dabbrev() {
    let (mut i, ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  logic foobar;\n  logic foo;\nendmodule\n",
    );
    ok(&mut i, "(verilog-mode)");
    // Place point right after the second, shorter "foo" so dabbrev has
    // something in the SAME buffer to expand it to.
    goto_after(&mut i, "logic foo;");
    ok(&mut i, "(backward-char 1)"); // right after "foo", before ";"
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "nil", "not a port-connecting dot at all: {}", r);
    assert!(
        popup_items(&ed).is_none(),
        "no popup must open for an ordinary identifier"
    );
    // completion-at-point (the real C-M-i entry point) must fall
    // through to dabbrev-expand and actually expand "foo" -> "foobar".
    ok(&mut i, "(completion-at-point)");
    let bs = match i.eval_source("(buffer-string)") {
        Ok(elisp::Value::Str(s)) => (*s).clone(),
        other => panic!("buffer-string didn't return a string: {}", other.is_ok()),
    };
    assert!(
        bs.contains("logic foobar;\n  logic foobar;"),
        "dabbrev-expand should have completed \"foo\" to \"foobar\": {}",
        bs
    );
}

// ============================================================
// 5. Module definition not found -> no popup, never signals
// ============================================================

#[test]
fn module_not_found_returns_t_without_opening_a_popup_or_signaling() {
    let (mut i, ed) = setup();
    // No "nope" module anywhere (current buffer, nor any library file --
    // an empty scratch buffer has no visited file, so its library scan
    // resolves relative to whatever `default-directory` happens to be;
    // "nope" is not a plausible real module name in this repo's own
    // Verilog fixtures, so the resolution genuinely fails either way).
    let src = "module top;\n  nope u_x ( .a );\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".a");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "still a confirmed port-connection position -- must not fall through to dabbrev: {}",
        r
    );
    assert!(
        !r.starts_with("ERROR"),
        "must never signal on an unresolvable module: {}",
        r
    );
    assert!(
        popup_items(&ed).is_none(),
        "no popup when the instantiated module can't be found anywhere"
    );
    let echo = ed.borrow().echo.clone();
    assert!(
        echo.as_deref().is_some_and(|m| m.contains("not found")),
        "must message that the module couldn't be resolved at all: {:?}",
        echo
    );
}

#[test]
fn empty_port_list_returns_t_without_opening_a_popup() {
    let (mut i, ed) = setup();
    let src =
        "module top;\n  empty_mod u_e ( .a );\nendmodule\n\nmodule empty_mod ();\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".a");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert!(popup_items(&ed).is_none());
    let echo = ed.borrow().echo.clone();
    assert!(
        echo.as_deref().is_some_and(|m| m.contains("no ports")),
        "must message that the module WAS found but has no ports, distinguishing from \
         module-not-found: {:?}",
        echo
    );
}

#[test]
fn no_matching_prefix_returns_t_without_opening_a_popup_and_names_the_prefix() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .zzz );\nendmodule\n\nmodule fifo (input wr_en, input rd_en);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".zzz");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert!(popup_items(&ed).is_none());
    let echo = ed.borrow().echo.clone();
    assert!(
        echo.as_deref().is_some_and(|m| m.contains("zzz")),
        "must distinguish \"module found, ports exist, none match the typed prefix\" from \
         both module-not-found and empty-port-list: {:?}",
        echo
    );
}

#[test]
fn empty_port_list_in_a_library_file_exercises_the_module_found_p_library_branch() {
    // M54 review: the three "not found"/"no ports"/"no matching prefix"
    // message tests above all define the instantiated module in the
    // SAME buffer (`insert_src'), so `verilog-complete--module-found-p'
    // (called from `verilog-complete-at-point''s cond, see verilog-
    // complete.el) always finds its answer via `verilog-auto--find-
    // module-in-buffer' -- the library-directory loop inside `--module-
    // found-p' (the `(setq found t)' branch) never actually runs in any
    // of them. Here the module is defined ONLY in a library file, with
    // NO ports at all, so `verilog-complete--module-ports' returns nil
    // for a reason other than "not found", forcing `--module-found-p'
    // down its own library-scan branch to tell the two apart.
    let (mut i, ed) = setup();
    let dir = scratch_dir("lib_empty_ports");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;\n  empty_lib_mod u_e ( .a );\nendmodule\n",
    )
    .unwrap();
    std::fs::write(&sub_path, "module empty_lib_mod ();\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, ".a");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert!(popup_items(&ed).is_none());
    let echo = ed.borrow().echo.clone();
    assert!(
        echo.as_deref().is_some_and(|m| m.contains("no ports")),
        "module IS found (in a library file, not the current buffer), just has zero \
         ports -- must not be reported as \"not found\": {:?}",
        echo
    );
}

// ============================================================
// 6. Port-context detection edge cases (verilog-complete--port-context)
//
// M54 review: the exclusion logic here was only backed by static
// analysis against the tree-sitter grammar, never an actual test that
// puts point at these shapes. Each test below asserts the REAL
// behavior of `verilog-complete-at-point' at one specific `.' shape.
// ============================================================

#[test]
fn parameter_override_dot_does_not_trigger_port_completion() {
    // `#(.WIDTH(8))' is a parameter override list, not a port
    // connection list -- a different grammar shape entirely
    // (`parameter_value_assignment'/`named_parameter_assignment'), per
    // this file's own header. M54: this fell all the way through to
    // nil (not attempted at all). M91: it is now attempted, but via
    // the PARAMETER path, not the port path -- confirm the popup that
    // opens offers the parameter name (`WIDTH'), never the port name
    // (`wr') that a misrouted port-completion would have offered
    // instead. See `parameter_completion_does_not_fire_inside_port_
    // connection_list' below for the opposite direction.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo #(.WIDTH(8)) u_fifo ( .wr );\nendmodule\n\nmodule fifo #(parameter WIDTH = 1) (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".WIDTH");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "M91: a parameter override's `.' must now be handled (by the parameter path): {}",
        r
    );
    let items = popup_items(&ed).expect("parameter path must have opened a popup");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["WIDTH".to_string()],
        "must offer the PARAMETER name, never a port name: {:?}",
        names
    );
}

#[test]
fn struct_member_access_dot_does_not_trigger_port_completion() {
    // `s.field' outside any instantiation's connection list -- the
    // dot's immediate parent is never `named_port_connection' nor
    // `hierarchical_instance', so `verilog-complete--port-context'
    // must return nil here.
    let (mut i, ed) = setup();
    let src = "module top;\n  logic result;\n  assign result = s.fie;\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, "s.fie");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a struct/interface member access `.' must NOT be treated as a port connection: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn second_port_connection_in_same_instantiation_offers_ports() {
    // Docstring's own shape 1 variant: a SECOND `.' after an existing
    // comma-separated connection, with a non-empty typed prefix.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr(1), .rd );\nendmodule\n\nmodule fifo (input wr, input rd, input clk);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".rd");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "second port connection must be a confirmed port context: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["rd".to_string()],
        "prefix \"rd\" narrows to rd only: {:?}",
        names
    );
}

#[test]
fn bare_second_dot_after_comma_offers_all_ports() {
    // Docstring's own shape 2, verbatim: `.wr(x), .' -- a bare SECOND
    // dot right after a comma, port_name a zero-width MISSING node.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr(1), . );\nendmodule\n\nmodule fifo (input wr, input rd, input clk);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".wr(1), .");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "bare second dot (MISSING port_name) must still be a confirmed port context: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["clk".to_string(), "rd".to_string(), "wr".to_string()],
        "empty prefix -> every port of fifo offered: {:?}",
        names
    );
}

#[test]
fn multiline_instantiation_offers_ports() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo (\n    .wr\n  );\nendmodule\n\nmodule fifo (input wr, input rd);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".wr");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "a port connection split across lines must still be detected: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(names, vec!["wr".to_string()]);
}

#[test]
fn same_buffer_bare_dot_empty_prefix_offers_all_ports() {
    // A bare first dot (docstring shape 3, ERROR node whose parent is
    // `hierarchical_instance' directly) with a zero-length prefix, in
    // the SAME-buffer resolution path (existing test 2 only covers this
    // shape via a cross-file library lookup).
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( . );\nendmodule\n\nmodule fifo (input wr, input rd);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, "( .");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "bare first dot, empty prefix: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(names, vec!["rd".to_string(), "wr".to_string()]);
}

#[test]
fn port_connection_value_side_dot_does_not_trigger_port_completion() {
    // `.addr(cfg.|base)' -- a hierarchical reference INSIDE a
    // connection's own value expression, not the port-connecting dot.
    // Docstring claims this is excluded by the immediate-parent check,
    // not a bounded upward walk; pin it down with a real parse.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .addr(cfg.base) );\nendmodule\n\nmodule fifo (input addr);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, "cfg.");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a value-side `.' inside a connection's expression must NOT be treated as a port connection: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

// ============================================================
// 7. Capabilities gate (lsp.el)
// ============================================================

fn stub_client_with_capabilities(interp: &mut Interp, caps_expr: &str) {
    // A `lsp--client' whose `conn' passes `lsp--live-buffer-client's
    // alive-gate untouched (same `nil'-conn convention `lsp_mode_
    // tests.rs'/`completion_popup_tests.rs' use), with CAPS-EXPR as its
    // `capabilities' slot.
    ok(
        interp,
        &format!(
            "(setq-local lsp--buffer-client (make-lsp--client :conn nil :capabilities {}))",
            caps_expr
        ),
    );
}

#[test]
fn capability_key_absent_blocks_the_lsp_request_and_falls_to_dabbrev() {
    let (mut i, _ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  logic foobar;\n  logic foo;\nendmodule\n",
    );
    // capabilities present, but no "completionProvider" key at all.
    stub_client_with_capabilities(&mut i, "(make-hash-table)");
    ok(
        &mut i,
        "(setq test-lsp-called nil)\n(fset 'lsp-completion-at-point (lambda () (setq test-lsp-called t)))",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--capability-supported-p lsp--buffer-client \"completionProvider\")"
        ),
        "nil"
    );
    goto_after(&mut i, "logic foo;");
    ok(&mut i, "(backward-char 1)");
    ok(&mut i, "(completion-at-point)");
    assert_eq!(
        run(&mut i, "test-lsp-called"),
        "nil",
        "absent completionProvider key must not send the LSP request"
    );
}

#[test]
fn capability_key_present_but_false_still_sends_the_lsp_request() {
    // M46 regression guard: a capability VALUE of `:false' must stay
    // trusted-as-supported, same as `hoverProvider: false' already is
    // for hover -- only an ABSENT key blocks anything.
    let (mut i, _ed) = setup();
    insert_src(&mut i, "module top;\n  logic foobar;\nendmodule\n");
    ok(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"completionProvider\" :false h) (setq-local lsp--buffer-client (make-lsp--client :conn nil :capabilities h)))",
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--capability-supported-p lsp--buffer-client \"completionProvider\")"
        ),
        "t"
    );
    ok(
        &mut i,
        "(setq test-lsp-called nil)\n(fset 'lsp-completion-at-point (lambda () (setq test-lsp-called t)))",
    );
    ok(&mut i, "(goto-char 20)");
    ok(&mut i, "(completion-at-point)");
    assert_eq!(
        run(&mut i, "test-lsp-called"),
        "t",
        "a present-but-false completionProvider key must still send the request"
    );
}

#[test]
fn capabilities_nil_still_sends_the_lsp_request() {
    // Pre-M54 behavior preserved: a client whose capabilities were
    // never recorded (nil) is trusted, same as before this milestone.
    let (mut i, _ed) = setup();
    insert_src(&mut i, "module top;\n  logic foobar;\nendmodule\n");
    stub_client_with_capabilities(&mut i, "nil");
    assert_eq!(
        run(
            &mut i,
            "(lsp--capability-supported-p lsp--buffer-client \"completionProvider\")"
        ),
        "t"
    );
    ok(
        &mut i,
        "(setq test-lsp-called nil)\n(fset 'lsp-completion-at-point (lambda () (setq test-lsp-called t)))",
    );
    ok(&mut i, "(goto-char 20)");
    ok(&mut i, "(completion-at-point)");
    assert_eq!(run(&mut i, "test-lsp-called"), "t");
}

#[test]
fn local_completion_function_tier_wins_over_a_capable_lsp_client() {
    // Task 2 dispatch order: `local-completion-function' is tried
    // FIRST, ahead of even a client that DOES support completion.
    let (mut i, ed) = setup();
    let src =
        "module top;\n  fifo u_fifo ( .wr );\nendmodule\n\nmodule fifo (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    ok(
        &mut i,
        "(let ((h (make-hash-table))) (puthash \"completionProvider\" (make-hash-table) h) (setq-local lsp--buffer-client (make-lsp--client :conn nil :capabilities h)))",
    );
    ok(
        &mut i,
        "(setq test-lsp-called nil)\n(fset 'lsp-completion-at-point (lambda () (setq test-lsp-called t)))",
    );
    goto_after(&mut i, ".wr");
    ok(&mut i, "(completion-at-point)");
    assert_eq!(
        run(&mut i, "test-lsp-called"),
        "nil",
        "verilog-complete-at-point must have handled it before LSP was ever tried"
    );
    assert!(popup_items(&ed).is_some());
}

// ============================================================
// 8. Cache: same library file queried twice
// ============================================================

#[test]
fn same_library_file_queried_twice_does_not_reparse_when_unchanged() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("cache_reuse");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    // The trailing "  fif" line (M91) is a SEPARATE, still-being-typed
    // instantiation type-name -- exercised below by the module-name
    // completion query, on the SAME file this test's port-completion
    // queries already visit, to prove the cache is shared, not
    // rebuilt, across the two completion paths.
    std::fs::write(
        &top_path,
        "module top;\n  fifo u_fifo ( . );\n  fif\nendmodule\n",
    )
    .unwrap();
    std::fs::write(&sub_path, "module fifo (input wr, input rd);\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    // Count real parses via a counting wrapper around
    // `verilog-auto--parse-string' -- the one step this cache exists to
    // skip on an unchanged file (see verilog-complete.el's own header).
    ok(
        &mut i,
        "(fset 'verilog-complete--test-orig-parse-string (symbol-function 'verilog-auto--parse-string))\n(setq test-parse-count 0)\n(fset 'verilog-auto--parse-string (lambda (text) (setq test-parse-count (1+ test-parse-count)) (funcall 'verilog-complete--test-orig-parse-string text)))",
    );
    goto_after(&mut i, "( .");
    ok(&mut i, "(verilog-complete-at-point)");
    assert!(popup_items(&ed).is_some(), "first query must find wr/rd");
    let count_after_first = run(&mut i, "test-parse-count");
    assert_eq!(count_after_first, "1", "one parse for the one library file");

    ok(&mut i, "(hide-completion-popup)");
    ok(&mut i, "(verilog-complete-at-point)");
    assert!(
        popup_items(&ed).is_some(),
        "second query on the same unchanged file must still find candidates"
    );
    assert_eq!(
        run(&mut i, "test-parse-count"),
        "1",
        "unchanged file content: no re-parse on the second query"
    );

    // M91: a module-name completion query, at a DIFFERENT position in
    // the SAME buffer, must reuse the identical cached sub.sv entry --
    // not the port machinery being tested above, a wholly separate
    // detector/handler pair (`verilog-complete--instantiation-type-
    // context'/`--handle-instantiation-type-context'), sharing only
    // `verilog-complete--library-file-modules''s own cache.
    ok(&mut i, "(hide-completion-popup)");
    goto_after(&mut i, "  fif");
    ok(&mut i, "(verilog-complete-at-point)");
    let module_name_items =
        popup_items(&ed).expect("module-name query must find the library module");
    assert!(
        insert_names(&module_name_items).contains(&"fifo".to_string()),
        "must offer the library module by name: {:?}",
        insert_names(&module_name_items)
    );
    assert_eq!(
        run(&mut i, "test-parse-count"),
        "1",
        "M91 module-name completion must reuse the SAME cached parse, not add a new one"
    );

    // Now change the library file's content -- the cache must notice
    // (content-equality invalidation, see verilog-complete.el's header
    // for why this substitutes for mtime) and re-parse.
    std::fs::write(
        &sub_path,
        "module fifo (input wr, input rd, input clk);\nendmodule\n",
    )
    .unwrap();
    ok(&mut i, "(hide-completion-popup)");
    goto_after(&mut i, "( .");
    ok(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        run(&mut i, "test-parse-count"),
        "2",
        "changed file content must trigger exactly one more parse"
    );
    let items = popup_items(&ed).expect("third query must still find candidates");
    let mut names = insert_names(&items);
    names.sort();
    assert_eq!(
        names,
        vec!["clk".to_string(), "rd".to_string(), "wr".to_string()],
        "must reflect the NEW port list, not a stale cached one: {:?}",
        names
    );
}

#[test]
fn clear_library_cache_command_forces_a_reparse() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("cache_clear");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;\n  fifo u_fifo ( .wr );\nendmodule\n",
    )
    .unwrap();
    std::fs::write(&sub_path, "module fifo (input wr);\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    ok(
        &mut i,
        "(fset 'verilog-complete--test-orig-parse-string (symbol-function 'verilog-auto--parse-string))\n(setq test-parse-count 0)\n(fset 'verilog-auto--parse-string (lambda (text) (setq test-parse-count (1+ test-parse-count)) (funcall 'verilog-complete--test-orig-parse-string text)))",
    );
    goto_after(&mut i, ".wr");
    ok(&mut i, "(verilog-complete-at-point)");
    assert_eq!(run(&mut i, "test-parse-count"), "1");
    ok(&mut i, "(hide-completion-popup)");
    ok(&mut i, "(verilog-complete-clear-library-cache)");
    ok(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        run(&mut i, "test-parse-count"),
        "2",
        "manual cache clear must force a re-read/re-parse even with unchanged content"
    );
    let _ = &ed;
}

// ============================================================
// M91: module-name completion at an instantiation's own type-name
// ============================================================

#[test]
fn module_name_completes_from_library_file() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_module_name_lib");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(&top_path, "module top;\n  fif\nendmodule\n").unwrap();
    std::fs::write(&sub_path, "module fifo (input wr);\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "  fif");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "instantiation type-name position: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["fifo".to_string()],
        "must offer the library module, matched by the typed prefix \"fif\": {:?}",
        names
    );
    assert!(
        items[0].0.contains("sub.sv"),
        "label should name the SOURCE file, since the module isn't in this buffer: {}",
        items[0].0
    );
}

#[test]
fn module_name_completes_from_same_buffer() {
    let (mut i, ed) = setup();
    let src = "module top;\n  fif\nendmodule\n\nmodule fifo (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "  fif");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "instantiation type-name position: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["fifo".to_string()],
        "must offer the SAME-buffer module too, not just library files: {:?}",
        names
    );
    assert!(
        !items[0].0.contains('('),
        "a same-buffer module's label carries no source annotation: {}",
        items[0].0
    );
}

#[test]
fn module_name_no_matching_prefix_falls_through_to_dabbrev() {
    // M91 fix round (V1, measure 2): unlike the port/parameter paths,
    // a structurally-plausible instantiation type-name position with
    // NO matching module must return nil, not t+message -- claiming
    // the position (and blocking dabbrev) is only justified once a
    // real candidate exists. "zzzzz" is not a keyword prefix (measure
    // 1 doesn't apply here) and matches no module (measure 2 applies).
    let (mut i, ed) = setup();
    let src = "module top;\n  zzzzz\nendmodule\n\nmodule fifo (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "  zzzzz");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "no module named \"zzzzz\" exists -- must fall through to dabbrev, not claim the position: {}",
        r
    );
    assert!(
        popup_items(&ed).is_none(),
        "no module name starts with \"zzzzz\""
    );
}

#[test]
fn instantiation_type_context_returns_nil_for_a_plain_expression_identifier() {
    // `assign y = fo' -- probed (see verilog-complete.el's own M91
    // docstring) to collapse into an ERROR with MANY children, not the
    // lone-identifier ERROR shape the type-name detector requires.
    let (mut i, ed) = setup();
    insert_src(
        &mut i,
        "module top;\n  logic y;\n  logic foobar;\n  assign y = foo\nendmodule\n",
    );
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "assign y = foo");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a plain expression identifier is not an instantiation type name: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn instantiation_type_context_returns_nil_for_a_declaration_name() {
    // `module fif' -- the module's OWN declaration name, being typed.
    // Probed to collapse into an ERROR with TWO children (`module_
    // keyword' and the identifier), not the type-name detector's own
    // lone-identifier shape.
    let (mut i, ed) = setup();
    insert_src(&mut i, "module fif");
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "module fif");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a module's own declaration name is not an instantiation type name: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn instantiation_type_context_does_not_bleed_into_a_port_connection_dot() {
    // A `.' position must still take the PORT path even when a module
    // happens to be named exactly like the typed port prefix -- proves
    // the port-context detector (tried first) wins and the module-name
    // detector never gets a chance to misfire here.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo u_fifo ( .wr );\nendmodule\n\nmodule fifo (input wr_en);\nendmodule\n\nmodule wr (input x);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, ".wr");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "{}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["wr_en".to_string()],
        "must offer the PORT name (wr_en), never the unrelated module named literally \"wr\": {:?}",
        names
    );
}

// ============================================================
// M91: parameter-name completion inside an instantiation's `#(...)'
// ============================================================

#[test]
fn parameter_completes_from_library_file() {
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_param_lib");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(&top_path, "module top;\n  fifo #(.W\nendmodule\n").unwrap();
    std::fs::write(
        &sub_path,
        "module fifo #(parameter WIDTH = 8, parameter DEPTH = 16) (input wr);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "#(.W");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "parameter-override position: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["WIDTH".to_string()],
        "only WIDTH starts with the typed prefix \"W\" (DEPTH doesn't): {:?}",
        names
    );
    assert!(
        items[0].0.contains('8'),
        "label should show the declared default value, mirroring the port label's own \
         direction/range convention: {}",
        items[0].0
    );
}

#[test]
fn parameter_completion_multiline_instantiation_offers_the_second_parameter() {
    // Same shape the port tests found mattered (`multiline_
    // instantiation_offers_ports') -- an override list spanning
    // several lines, point placed mid-identifier inside the SECOND,
    // already-fully-typed override (`.DE|PTH(16)') so the typed
    // prefix (\"DE\") narrows to just that one parameter. A fully
    // CLOSED multi-line override list, unlike the single-line ERROR-
    // collapse shapes the other parameter tests exercise -- probed
    // (M91) to parse as the well-formed `named_parameter_assignment'
    // branch, not the ERROR-collapse one; both branches of
    // `verilog-complete--param-context' need their own coverage.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo #(.WIDTH(8),\n          .DEPTH(16)\n  ) u_fifo (\n    .wr(x)\n  );\nendmodule\n\nmodule fifo #(parameter WIDTH = 8, parameter DEPTH = 16) (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, ".DE");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "parameter-override position: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["DEPTH".to_string()],
        "only DEPTH starts with the typed prefix \"DE\": {:?}",
        names
    );
}

#[test]
fn parameter_completion_does_not_fire_inside_port_connection_list() {
    // The reverse direction of `parameter_override_dot_does_not_
    // trigger_port_completion' above: a `.' inside the PORT connection
    // list must offer the port name, never a parameter name, even when
    // the instantiated module declares both.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo #(.WIDTH(8)) u_fifo ( .wr );\nendmodule\n\nmodule fifo #(parameter WIDTH = 1) (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "( .wr");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "{}", r);
    let items = popup_items(&ed).expect("popup must open");
    let names = insert_names(&items);
    assert_eq!(
        names,
        vec!["wr".to_string()],
        "must offer the PORT name, never WIDTH: {:?}",
        names
    );
}

// ============================================================
// M91 fix round: V1 -- ordinary statement-start typing must not
// misfire as module-name completion
// ============================================================

#[test]
fn keyword_prefixed_statement_starts_with_no_matching_module_fall_through() {
    // Ordinary statement-start typing whose prefix happens to also be
    // a prefix of a Verilog reserved word, but where NO real module
    // matches either -- must still fall through to dabbrev. This is
    // the "nothing matches" half; see `keyword_prefixed_typing_still_
    // completes_a_matching_module_name' below for the half that Y1's
    // fix round exists for (a REAL match must win even when the typed
    // prefix also looks like a keyword being typed).
    let cases = [
        (
            "module top;
  wi
endmodule
",
            "  wi",
        ), // wire, no "wi*" module
        (
            "module top;
  lo
endmodule
",
            "  lo",
        ), // logic, no "lo*" module
        (
            "module top;
  para
endmodule
",
            "  para",
        ), // parameter, no "para*" module
    ];
    for (src, needle) in cases {
        let (mut i, ed) = setup();
        insert_src(&mut i, src);
        ok(&mut i, "(verilog-mode)");
        goto_after(&mut i, needle);
        let r = run(&mut i, "(verilog-complete-at-point)");
        assert_eq!(
            r, "nil",
            "no module matches -- must fall through to dabbrev, src={:?} needle={:?}: {}",
            src, needle, r
        );
        assert!(
            popup_items(&ed).is_none(),
            "no popup when nothing matches, src={:?}",
            src
        );
    }
}

#[test]
fn keyword_prefixed_typing_still_completes_a_matching_module_name() {
    // Y1 (final round): the coordinator's own repro, reproduced
    // against the real `verilog-complete-at-point' before this fix --
    // with `module reset_ctrl' declared, typing "re" used to return
    // nil and offer nothing (likewise "gen"/`gen_ctrl' and
    // "as"/`as_ctrl'), because the OLD keyword-prefix veto rejected
    // the position unconditionally, before ever checking for a real
    // module match. A real match must now win regardless: this test
    // DEPENDS on that fix -- if the veto (or an equivalent one) were
    // reintroduced ahead of the match check, all three flip from t
    // back to nil, which is exactly the regression this test exists
    // to catch.
    let cases = [
        (
            "module top;
  re
endmodule

module reset_ctrl (input x);
endmodule
",
            "  re",
            "reset_ctrl",
        ),
        (
            "module top;
  gen
endmodule

module gen_ctrl (input x);
endmodule
",
            "  gen",
            "gen_ctrl",
        ),
        (
            "module top;
  as
endmodule

module as_ctrl (input x);
endmodule
",
            "  as",
            "as_ctrl",
        ),
    ];
    for (src, needle, expected) in cases {
        let (mut i, ed) = setup();
        insert_src(&mut i, src);
        ok(&mut i, "(verilog-mode)");
        goto_after(&mut i, needle);
        let r = run(&mut i, "(verilog-complete-at-point)");
        assert_eq!(
            r, "t",
            "a real module DOES match, even though the prefix also looks like a keyword              being typed, src={:?} needle={:?}: {}",
            src, needle, r
        );
        let items = popup_items(&ed).expect("popup must open");
        assert_eq!(
            insert_names(&items),
            vec![expected.to_string()],
            "src={:?}",
            src
        );
    }
}

#[test]
fn non_keyword_task_call_identifier_with_no_matching_module_falls_through() {
    // "my_ta" is NOT a prefix of any keyword in `verilog-complete--
    // statement-keywords' -- measure 1 does not apply here at all.
    // It is a task-call identifier (`my_task(...)') that happens to
    // match no real module name either -- measure 2 (`verilog-
    // complete--handle-instantiation-type-context' only claiming the
    // position when a module actually matches) is what excludes it.
    let (mut i, ed) = setup();
    let src = "module top;
  my_ta
endmodule

module fifo (input wr);
endmodule
";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "  my_ta");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "no module named \"my_ta...\" exists -- must fall through: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn residual_ambiguity_a_matching_module_name_still_completes_at_a_statement_start() {
    // Documented, ACCEPTED residual ambiguity (see `verilog-complete--
    // instantiation-type-context's own docstring): "cnt" is not a
    // keyword prefix, and a real module named "cnt_fifo" exists --
    // even though a real engineer typing "cnt" here might well mean
    // an ordinary signal, this file offers the module, same as every
    // other prefix-based completion source would.
    let (mut i, ed) = setup();
    let src = "module top;
  cnt
endmodule

module cnt_fifo (input wr);
endmodule
";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "  cnt");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "a real module DOES match \"cnt\": {}", r);
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["cnt_fifo".to_string()]);
}

// ============================================================
// M91 fix round: V3 -- guards made mutation-observable
// ============================================================

#[test]
fn instantiation_type_context_child_count_guard_excludes_a_second_bare_identifier() {
    // The exact shape the coordinator's review used to prove the
    // child-count-1 guard (`verilog-complete--instantiation-type-
    // context', ERROR branch) is load-bearing, not redundant with the
    // grandparent check: "bar"'s own immediate parent is an ERROR
    // DIRECTLY under `module_declaration' (the very grandparent this
    // function otherwise accepts), so only the child-count check
    // excludes it -- removing that check flips the DETECTOR's own
    // return from nil to t.
    //
    // Y2 (final round): a real module named EXACTLY "bar" is declared
    // in a separate library file, reachable from this buffer -- with
    // no matching module anywhere, `verilog-complete--any-module-
    // name-matches-p' would return nil regardless of what the
    // detector itself says, masking a mutation of the child-count
    // guard entirely (measure 2 alone already answers nil). With a
    // REAL "bar" module present, the TOP-LEVEL result genuinely
    // depends on the guard: present, nil (this test); removed, t with
    // "bar" offered.
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_y2_child_count_guard");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;
  wire w;
  assign y = foo bar
endmodule
",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module bar (input x);
endmodule
",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, "foo bar");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a second bare identifier after a broken assign statement is not an instantiation type name, \
         even though a real module named \"bar\" exists: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn instantiation_type_context_fires_inside_an_unlabeled_generate_block() {
    // The `generate_block' entry in the grandparent list -- an
    // UNLABELED `if (...) begin ... end' nested inside `generate
    // ... endgenerate' (a LABELED `begin : name ... end' instead
    // wraps in `seq_block', excluded, see the next test). "fif" is not
    // a keyword prefix.
    let (mut i, ed) = setup();
    let src = "module top;
  generate
    if (1) begin
      fif
    end
  endgenerate
endmodule

module fifo (input wr);
endmodule
";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "      fif");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "an unlabeled generate block is a valid instantiation site: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["fifo".to_string()]);
}

#[test]
fn instantiation_type_context_does_not_fire_inside_a_labeled_generate_begin_block() {
    // A LABELED `begin : g ... end' parses as `seq_block' under an
    // `always_construct', same shape as an ordinary procedural block
    // -- correctly excluded (not one of the three accepted
    // grandparent types). "fif" is not a keyword prefix, so this is a
    // genuine structural exclusion, not measure 1.
    let (mut i, ed) = setup();
    let src = "module top;
  generate
    begin : g
      fif
    end
  endgenerate
endmodule

module fifo (input wr);
endmodule
";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "      fif");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a LABELED begin:end block is not an instantiation site: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn instantiation_type_context_returns_nil_for_an_empty_prefix() {
    // Symmetric to the port path's own `same_buffer_bare_dot_empty_
    // prefix_offers_all_ports' -- but the OPPOSITE answer: the module
    // path has no anchoring token analogous to the port path's `.',
    // so an empty typed prefix must never claim the position (see
    // this file's own header's scope-cut note).
    //
    // Y2 (final round): confirmed load-bearing by manual backup +
    // targeted edit (temporarily replacing `(when (> point
    // prefix-start) ...)' with `(when t ...)', reverted after) --
    // WITHOUT this guard, positioning point at the very START of an
    // EXISTING identifier (nothing newly typed, just moved there) is
    // structurally indistinguishable from freshly typing that same
    // identifier, except the typed prefix comes out empty -- and an
    // empty prefix matches EVERY module name (`string-prefix-p ""'
    // is always true), so the fix round's own cheap existence check
    // would then always say yes, opening a popup listing every
    // reachable module. This fixture reproduces exactly that: point
    // sits right before the EXISTING "fifo" text, a real "fifo"
    // module is declared. With the guard: nil. Without it (verified
    // manually): t, offering BOTH "top" and "fifo" -- the guard is
    // what excludes this position, not the `simple_identifier'-at-
    // `prefix-start' check (that check still matches: point IS at the
    // exact start of a real `simple_identifier' node).
    let (mut i, ed) = setup();
    let src = "module top;
  fifo
endmodule

module fifo (input wr);
endmodule
";
    insert_src(&mut i, src);
    ok(&mut i, "(verilog-mode)");
    goto_after(&mut i, "  ");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "empty prefix, point at the start of an existing identifier: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
}

#[test]
fn parameter_completion_skips_a_comma_separated_not_yet_closed_second_override() {
    // Exercises the `("," "named_parameter_assignment")' skip list in
    // `verilog-complete--param-type-name-from-error' -- a first
    // override is already CLOSED (`.WIDTH(8)'), a comma follows, and
    // a second override is only partially typed and NOT yet closed
    // (`.DE'), so the whole thing collapses into one top-level ERROR
    // (unlike the multiline test above, which uses a fully CLOSED
    // list and takes the well-formed `named_parameter_assignment'
    // branch instead). Kept in a SEPARATE library file, module
    // definition in another -- probed (M91 fix round) that adding
    // trailing content to the SAME buffer (e.g. the module
    // definition right after) changes tree-sitter's own recovery
    // shape entirely (it stops being a top-level ERROR at all), so
    // this needs the current buffer to end right after the broken
    // text, same as `parameter_completes_from_library_file' above.
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_v3_comma_skip");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;\n  fifo #(.WIDTH(8), .DE\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module fifo #(parameter WIDTH = 8, parameter DEPTH = 16) (input wr);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, ".DE");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(r, "t", "parameter-override position: {}", r);
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["DEPTH".to_string()]);
}

// ============================================================
// M91 fix round: V2 -- parameter ERROR-recovery gaps
// ============================================================

#[test]
fn parameter_completion_recovers_the_type_name_when_two_broken_overrides_stack() {
    // The coordinator's own repro: TWO separate instantiations, both
    // simultaneously broken, on consecutive lines -- tree-sitter
    // nests the FIRST one's leftover tokens (PLUS the second
    // statement's own type-name identifier, "fifo2") inside a nested
    // ERROR rather than leaving "fifo2" as a flat sibling. See
    // `verilog-complete--param-type-name-unwrap's own docstring.
    // Module definitions live in a separate library file -- probed
    // (M91 fix round) that adding trailing content to the SAME buffer
    // changes tree-sitter's own recovery shape entirely (it stops
    // being an ERROR at all), so the current buffer must end right
    // after the broken text, same as `parameter_completes_from_
    // library_file' above.
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_v2_stacked_error");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;\n  fifo #(.W\n  fifo2 #(.X\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module fifo (input wr);\nendmodule\n\nmodule fifo2 #(parameter XLEN = 8) (input wr);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, ".X");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "parameter-override position, second stacked instantiation: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["XLEN".to_string()]);
}

#[test]
fn parameter_completion_recovers_the_type_name_through_four_stacked_broken_overrides() {
    // Y3 (final round): the two-stack test above only verifies ONE
    // level of nested-ERROR unwrap; this verifies the recursive
    // unwrap keeps working through FOUR consecutive, simultaneously
    // broken instantiations, confirming `verilog-complete--param-
    // type-name-unwrap' has no hidden depth limit of its own.
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_v3_four_stack");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(
        &top_path,
        "module top;
  a #(.W
  b #(.X
  c #(.Y
  d #(.Z
endmodule
",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module a (input p);
endmodule
module b (input p);
endmodule
module c (input p);
endmodule
module d #(parameter ZP = 1) (input p);
endmodule
",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, ".Z");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "fourth stacked instantiation's own type name must resolve: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["ZP".to_string()]);
}

#[test]
fn parameter_completion_recovers_the_type_name_across_a_comment_between_hash_and_paren() {
    // `fifo #/*c*/(.W' -- the comment sits between `#' and `(' as its
    // own flat sibling in the ERROR's child list. Module definition
    // in a separate library file -- see the stacked-override test
    // above for why the current buffer must end right after the
    // broken text.
    let (mut i, ed) = setup();
    let dir = scratch_dir("m91_v2_comment");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("sub.sv");
    std::fs::write(&top_path, "module top;\n  fifo #/*c*/(.W\nendmodule\n").unwrap();
    std::fs::write(
        &sub_path,
        "module fifo #(parameter WIDTH = 8) (input wr);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_after(&mut i, ".W");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "t",
        "parameter-override position, comment between # and (: {}",
        r
    );
    let items = popup_items(&ed).expect("popup must open");
    assert_eq!(insert_names(&items), vec!["WIDTH".to_string()]);
}

// ============================================================
// 9. verible-verilog-ls e2e: local source wins over a connected LSP
// ============================================================

/// Whether CMD resolves to a real executable on PATH -- copied from
/// `lsp_mode_tests.rs`'s own `have_on_path` verbatim (deliberately not
/// shared across test files, matching that file's own note on why).
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
fn manual_e2e_verible_connected_port_completion_still_opens_the_local_popup() {
    // The whole point of this milestone: `verible-verilog-ls' cannot
    // answer `textDocument/completion' at all (M54 repro), but a
    // CONNECTED buffer's `C-M-i' at a port-connection position must
    // still open a real popup -- proving `local-completion-function'
    // (verilog-complete.el) wins ahead of, and entirely independently
    // of, whatever the connected server can or can't do.
    if !have_on_path("verible-verilog-ls") {
        eprintln!("skipping: verible-verilog-ls not on PATH");
        return;
    }
    let (mut i, ed) = setup();
    let dir = scratch_dir("verible_completion_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    let src = "module fifo (input wr, input rd);\nendmodule\n\nmodule top;\n  fifo u_fifo ( . );\nendmodule\n";
    std::fs::write(&file, src).unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"verible-verilog-ls\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    // `(verilog-mode)', not `major-mode-internal-set' -- this test needs
    // `verilog-mode-hook' to actually run and set `local-completion-
    // function' (see `modes.el's own M54 addition), same as real
    // `find-file' via `auto-mode-alist' would.
    ok(&mut i, "(verilog-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to verible-verilog-ls\"");

    let _ = src;
    goto_after(&mut i, "( .");
    ok(&mut i, "(completion-at-point)");
    let items =
        popup_items(&ed).expect("local port-completion popup must open even while connected");
    let names = insert_names(&items);
    assert!(
        names.contains(&"rd".to_string()),
        "must offer fifo's own ports, sourced locally: {:?}",
        names
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!("PASS: verible-verilog-ls e2e -- local port completion wins while connected");
}

#[test]
fn manual_e2e_verible_capability_gate_falls_to_dabbrev_at_a_non_port_position() {
    // M54 review: `lsp-connect''s `(setf (lsp--client-capabilities
    // client) ...)' (lsp.el) -- the line that actually STORES a real
    // handshake's capabilities on the client -- had zero e2e coverage.
    // Every existing capabilities test hand-builds a `lsp--client'
    // struct directly (`make-lsp--client'), bypassing `lsp-connect'
    // entirely, and the one test that DOES call `(lsp)' for real only
    // ever probes a port-connection position, which tier 1
    // (`verilog-complete-at-point') always intercepts before the
    // capability gate is ever consulted.
    //
    // This test connects to the REAL `verible-verilog-ls' and checks
    // two things a stubbed client can't:
    //   1. the capabilities hash stored on the client after a real
    //      handshake has no "completionProvider" key (confirmed via
    //      `dev/lsp-probe.py' against this exact binary: verible
    //      declares codeAction/definition/diagnostic/documentFormatting/
    //      documentHighlight/documentRangeFormatting/documentSymbol/
    //      hover(false)/references/rename/textDocumentSync, but no
    //      completionProvider at all) -- and DOES have some key verible
    //      genuinely declares (`definitionProvider'), so this isn't
    //      just checking for an empty hash table.
    //   2. `C-M-i' at a position that ISN'T a port connection (so tier 1
    //      returns nil) falls all the way through to `dabbrev-expand',
    //      not a doomed `textDocument/completion' request -- i.e. the
    //      gate this milestone exists for actually blocks something.
    if !have_on_path("verible-verilog-ls") {
        eprintln!("skipping: verible-verilog-ls not on PATH");
        return;
    }
    let (mut i, ed) = setup();
    let dir = scratch_dir("verible_capability_gate_e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("t.sv");
    // `foobar'/`foo' for the dabbrev-fallback assertion, plus a port
    // connection elsewhere in the same file so the buffer looks like
    // ordinary Verilog verible can parse without complaint.
    let src = "module fifo (input wr, input rd);\nendmodule\n\nmodule top;\n  logic foobar;\n  logic foo;\n  fifo u_fifo ( . );\nendmodule\n";
    std::fs::write(&file, src).unwrap();

    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list \"verible-verilog-ls\")))",
    );
    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(verilog-mode)");

    let r = run(&mut i, "(lsp)");
    println!("M-x lsp => {}", r);
    assert_eq!(r, "\"LSP: connected to verible-verilog-ls\"");

    // --- Assertion 1: the real handshake's capabilities landed on the
    // client, and have the expected shape.
    let caps_is_hash = run(
        &mut i,
        "(hash-table-p (lsp--client-capabilities lsp--buffer-client))",
    );
    assert_eq!(
        caps_is_hash, "t",
        "lsp-connect's real `initialize' handshake must store a hash table of capabilities: {}",
        caps_is_hash
    );
    let has_completion = run(
        &mut i,
        "(gethash \"completionProvider\" (lsp--client-capabilities lsp--buffer-client) 'test-missing)",
    );
    assert_eq!(
        has_completion, "test-missing",
        "verible-verilog-ls declares NO completionProvider key at all (see dev/lsp-probe.py \
         output this test's own comment cites) -- if this ever starts failing, verible added \
         completion support and the M54 repro note needs revisiting: {}",
        has_completion
    );
    let has_definition = run(
        &mut i,
        "(gethash \"definitionProvider\" (lsp--client-capabilities lsp--buffer-client) 'test-missing)",
    );
    assert_ne!(
        has_definition, "test-missing",
        "sanity: verible DOES declare definitionProvider, so an empty/malformed capabilities \
         hash wouldn't have passed the completionProvider check above vacuously: {}",
        has_definition
    );

    // --- Assertion 2: the gate actually blocks the request and falls
    // through to dabbrev at a non-port-connection position.
    goto_after(&mut i, "logic foo;");
    ok(&mut i, "(backward-char 1)"); // right after "foo", before ";"
    let before = ed.borrow().echo.clone();
    let _ = before;
    ok(&mut i, "(completion-at-point)");
    assert!(
        popup_items(&ed).is_none(),
        "no LSP popup must open -- verible has no completionProvider, the gate must block it"
    );
    let bs = match i.eval_source("(buffer-string)") {
        Ok(elisp::Value::Str(s)) => (*s).clone(),
        other => panic!("buffer-string didn't return a string: {}", other.is_ok()),
    };
    assert!(
        bs.contains("logic foobar;\n  logic foobar;"),
        "completion-at-point must have fallen through all the way to dabbrev-expand, which \
         expands \"foo\" -> \"foobar\" using the current buffer's own word list: {}",
        bs
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
    println!("PASS: verible-verilog-ls e2e -- capability gate blocks LSP, falls to dabbrev");
}
