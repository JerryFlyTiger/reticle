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
    // this file's own header. Docstring claims v1 doesn't attempt this;
    // confirm it doesn't misfire either.
    let (mut i, ed) = setup();
    let src = "module top;\n  fifo #(.WIDTH(8)) u_fifo ( .wr );\nendmodule\n\nmodule fifo #(parameter WIDTH = 1) (input wr);\nendmodule\n";
    insert_src(&mut i, src);
    goto_after(&mut i, ".WIDTH");
    let r = run(&mut i, "(verilog-complete-at-point)");
    assert_eq!(
        r, "nil",
        "a parameter override's `.' must NOT be treated as a port connection: {}",
        r
    );
    assert!(popup_items(&ed).is_none());
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
    std::fs::write(&top_path, "module top;\n  fifo u_fifo ( . );\nendmodule\n").unwrap();
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

    // Now change the library file's content -- the cache must notice
    // (content-equality invalidation, see verilog-complete.el's header
    // for why this substitutes for mtime) and re-parse.
    std::fs::write(
        &sub_path,
        "module fifo (input wr, input rd, input clk);\nendmodule\n",
    )
    .unwrap();
    ok(&mut i, "(hide-completion-popup)");
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
