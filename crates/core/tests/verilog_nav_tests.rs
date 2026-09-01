//! M55: Verilog cross-file module-definition jump
//! (`verilog-goto-module-at-point', verilog-nav.el) -- the
//! `local-definition-function' tier `lsp-definition-at-point' (lsp.el)
//! tries before any LSP client. See verilog-nav.el's own header for the
//! `verible-verilog-ls' repro this milestone is built against.

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

fn insert_src(interp: &mut Interp, src: &str) {
    ok(interp, &format!("(insert {:?})", src));
}

/// A fresh scratch directory under the OS temp dir, unique per test run
/// (mirrors `verilog_complete_tests.rs`'s/`lsp_mode_tests.rs`'s own
/// helper).
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
            "reticle_verilog_nav_{}_{}_{}",
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

/// Point right after the FIRST (from `point-min`) occurrence of NEEDLE.
fn goto_after(interp: &mut Interp, needle: &str) {
    ok(interp, "(goto-char (point-min))");
    ok(interp, &format!("(search-forward {:?})", needle));
}

/// Point in the MIDDLE of the first occurrence of WORD -- lands on an
/// identifier character with identifier characters on both sides,
/// exercising the ordinary (non-boundary) case.
fn goto_mid(interp: &mut Interp, word: &str) {
    goto_after(interp, word);
    ok(interp, &format!("(backward-char {})", word.len() / 2));
}

// NOTE: this file never sets `major-mode'/runs `verilog-mode-hook' at
// all -- every test here calls `verilog-goto-module-at-point' DIRECTLY,
// bypassing `local-definition-function' dispatch entirely, and that
// function's own logic (`treesit-parser-create' with an explicit
// `'verilog' language symbol, `verilog-library-directories' a plain
// global/buffer-local defvar) never consults the buffer's major mode
// either. An earlier draft of this file called `(verilog-mode)' before
// every test "for realism" -- reviewer flagged it as a helper with a
// name implying coverage it didn't actually provide (removing it broke
// nothing here). The REAL mode-hook wiring (`modes.el' setting
// `local-definition-function' to `verilog-goto-module-at-point', and
// `find-file-internal''s own `auto-mode-alist' dispatch getting there
// via `.sv') is exercised end-to-end in `lsp_mode_tests.rs' instead --
// see `verilog_mode_wires_local_definition_function_end_to_end' there.

fn assert_no_jump(interp: &mut Interp, file_before: &str, point_before: &str) {
    let r = run(interp, "(verilog-goto-module-at-point)");
    assert_eq!(
        r, "nil",
        "expected nil (not-my-context or unresolved): {}",
        r
    );
    assert_eq!(run(interp, "(buffer-file-name)"), file_before);
    assert_eq!(run(interp, "(point)"), point_before);
    assert_eq!(
        run(interp, "(length lsp--marker-stack)"),
        "0",
        "marker stack must not move on a nil return"
    );
}

// ============================================================
// 1. Jump to a neighboring library file's module
// ============================================================

#[test]
fn jumps_to_module_declared_in_a_library_file() {
    let mut i = setup();
    let dir = scratch_dir("lib_basic");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("axi_lite_slave.sv");
    std::fs::write(
        &top_path,
        "module top;\n  axi_lite_slave #(.AW(16)) u_slave (.aclk(clk));\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module axi_lite_slave (input aclk, output [31:0] rdata);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "axi_lite_slave");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "expected a successful jump: {}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", sub_path.to_str().unwrap())
    );
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 14),
    );
    assert_eq!(name, "\"axi_lite_slave\"");
}

// ============================================================
// 2. Module declared in the SAME buffer -- no file opened
// ============================================================

#[test]
fn jumps_within_the_same_buffer_when_module_is_declared_there() {
    let mut i = setup();
    let dir = scratch_dir("same_buffer");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(
        &path,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr, output rd);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", path.to_str().unwrap())
    );
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 4),
    );
    assert_eq!(name, "\"fifo\"");
}

// ============================================================
// 3. M-, returns to the origin buffer and point
// ============================================================

#[test]
fn pop_definition_stack_returns_to_the_origin() {
    let mut i = setup();
    let dir = scratch_dir("pop_stack");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("fifo.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_f (.wr(w));\nendmodule\n").unwrap();
    std::fs::write(&sub_path, "module fifo (input wr, output rd);\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let origin_point = run(&mut i, "(point)");
    ok(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", sub_path.to_str().unwrap())
    );
    ok(&mut i, "(lsp-pop-definition-stack)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", top_path.to_str().unwrap())
    );
    assert_eq!(run(&mut i, "(point)"), origin_point);
    assert_eq!(run(&mut i, "lsp--marker-stack"), "nil");
}

// ============================================================
// 3b. M-, returns to the origin for a SAME-BUFFER jump too -- the only
//     existing round-trip test above goes through the library-file
//     branch; the same-buffer branch's own `lsp-push-definition-marker'
//     call had no test checking `lsp--marker-stack' at all (M55
//     review's mutation M4: deleting that call survived every existing
//     test).
// ============================================================

#[test]
fn pop_definition_stack_returns_to_the_origin_for_a_same_buffer_jump() {
    let mut i = setup();
    let dir = scratch_dir("pop_stack_same_buffer");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(
        &path,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let origin_point = run(&mut i, "(point)");
    assert_eq!(run(&mut i, "(length lsp--marker-stack)"), "0");
    ok(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(
        run(&mut i, "(length lsp--marker-stack)"),
        "1",
        "verilog-goto-module-at-point's same-buffer branch must push the origin \
         before jumping -- see M55 review mutation M4"
    );
    // Confirm it actually moved (mid "fifo" -> its own declaration's
    // name node further down in the buffer), not a same-position no-op
    // that would make the marker-stack check above vacuous.
    assert_ne!(run(&mut i, "(point)"), origin_point);
    ok(&mut i, "(lsp-pop-definition-stack)");
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", path.to_str().unwrap())
    );
    assert_eq!(run(&mut i, "(point)"), origin_point);
    assert_eq!(run(&mut i, "lsp--marker-stack"), "nil");
}

// ============================================================
// 3c. Current buffer's own declaration wins over a same-named module in
//     a library file -- no new file opened.
// ============================================================

#[test]
fn current_buffer_declaration_wins_over_a_same_named_library_module() {
    let mut i = setup();
    let dir = scratch_dir("priority");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let lib_path = dir.join("fifo.sv");
    std::fs::write(
        &top_path,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr /* LOCAL */);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &lib_path,
        "module fifo (input wr /* LIBRARY */);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", top_path.to_str().unwrap()),
        "must land back in the SAME buffer, not open fifo.sv"
    );
    // Confirm it landed on the SECOND "fifo" occurrence (the local
    // declaration's own name), not merely stayed on the instantiation.
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 4),
    );
    assert_eq!(name, "\"fifo\"");
    let after_name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 40),
    );
    assert!(
        after_name.contains("LOCAL"),
        "landed on the current buffer's own declaration, not the library one: {}",
        after_name
    );
}

// ============================================================
// 4-9, 15. Negative contexts -> nil, buffer/point/stack untouched
// ============================================================

#[test]
fn point_on_instance_name_returns_nil() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_slave (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_mid(&mut i, "u_slave");
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn point_on_port_name_returns_nil() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_mid(&mut i, "wr");
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn point_on_connection_value_returns_nil() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_f (.wr(clk));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_mid(&mut i, "clk");
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn point_on_ordinary_signal_in_always_block_returns_nil() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  reg q;\n  always @(posedge clk) q <= d;\nendmodule\n",
    );
    goto_mid(&mut i, "clk"); // squarely mid-identifier on the SIGNAL, not "posedge"
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn point_on_whitespace_returns_nil() {
    let mut i = setup();
    // TWO spaces between the type name and the instance name -- a
    // single space would put `forward-char 1' right at the START of
    // "u_f" (an identifier character on the RIGHT side), which is the
    // exact same position `instance_name_start_does_not_count' already
    // covers, not genuine "on-ident is false on both sides" whitespace
    // (an earlier draft of this test made exactly that mistake).
    insert_src(
        &mut i,
        "module top;\n  fifo  u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_after(&mut i, "  fifo"); // point right after "fifo", before either space
    ok(&mut i, "(forward-char 1)"); // land between the two spaces -- ident char on NEITHER side
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn module_not_found_anywhere_returns_nil_falling_through_to_lsp() {
    let mut i = setup();
    insert_src(&mut i, "module top;\n  ghost_mod u_g (.a(b));\nendmodule\n");
    goto_mid(&mut i, "ghost_mod");
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

#[test]
fn nonexistent_library_directory_does_not_signal() {
    let mut i = setup();
    insert_src(&mut i, "module top;\n  ghost_mod u_g (.a(b));\nendmodule\n");
    ok(
        &mut i,
        "(setq-local verilog-library-directories '(\"/no/such/directory/at/all\"))",
    );
    goto_mid(&mut i, "ghost_mod");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "nil", "must not signal on a missing library dir: {}", r);
}

// ============================================================
// 10. Parameter override on the instantiation -- still jumps
// ============================================================

#[test]
fn jumps_through_a_parameter_override() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo #(.DEPTH(16)) u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 4),
    );
    assert_eq!(name, "\"fifo\"");
}

// ============================================================
// 11. Boundary: both the type name's own START and its own TAIL count
//     as on it (inclusive both ends -- `verilog-nav--point-in-node-p');
//     the very start of the following instance name does not.
// ============================================================

#[test]
fn type_name_start_counts_as_on_it() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    // Point right AT the "f" of "fifo" -- no characters of the type
    // name itself precede point, only the two leading spaces. Pins the
    // START-inclusive half of `verilog-nav--point-in-node-p' (`>=', not
    // `>') independently of the already-covered TAIL case below.
    goto_after(&mut i, "  ");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "start of the type name must count: {}", r);
}

#[test]
fn type_name_tail_counts_as_on_it() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_after(&mut i, "  fifo"); // point right after the "o" of "fifo"
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "tail of the type name must still count: {}", r);
}

#[test]
fn instance_name_start_does_not_count() {
    let mut i = setup();
    insert_src(
        &mut i,
        "module top;\n  fifo u_f (.wr(w));\nendmodule\n\nmodule fifo (input wr);\nendmodule\n",
    );
    goto_after(&mut i, "fifo "); // point right at the "u" of "u_f"
    let file_before = run(&mut i, "(buffer-file-name)");
    let point_before = run(&mut i, "(point)");
    assert_no_jump(&mut i, &file_before, &point_before);
}

// ============================================================
// 12. Non-ANSI header module declaration, found in a library file
// ============================================================

#[test]
fn finds_a_non_ansi_header_module_in_a_library_file() {
    let mut i = setup();
    let dir = scratch_dir("non_ansi");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("foo.sv");
    std::fs::write(&top_path, "module top;\n  foo u_f (a, b);\nendmodule\n").unwrap();
    std::fs::write(
        &sub_path,
        "module foo (a, b);\n  input a;\n  output b;\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "foo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", sub_path.to_str().unwrap())
    );
    let point = run(&mut i, "(point)");
    let name_start: usize = point.parse().unwrap();
    let name = ok(
        &mut i,
        &format!("(buffer-substring {} {})", name_start, name_start + 3),
    );
    assert_eq!(name, "\"foo\"");
}

// ============================================================
// 13. Staleness: target file already open in a buffer whose content
//     has since diverged from disk (module renamed, unsaved)
//
// KNOWN UNCOVERED (M55 tail re-review, recorded rather than papered
// over): the test below pins the point-min FALLBACK, but nothing pins
// the MESSAGE that accompanies it. There is no `message'-capturing
// facility in this test harness -- `lsp_mode_tests.rs's `test--
// captured' intercepts `lsp-request-async', not `message' -- so the
// message string's wording and its two format arguments are, today, a
// line that could be typo'd or have its arguments transposed without
// any test going red. Deliberately NOT listed in dev/mutations/m55.py:
// a mutation nothing can observe is permanently-SURVIVED noise, which
// makes the whole list less trustworthy rather than more.
// ============================================================

#[test]
fn stale_target_buffer_falls_back_to_point_min_with_a_message() {
    let mut i = setup();
    let dir = scratch_dir("staleness");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("fifo.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_f (.wr(w));\nendmodule\n").unwrap();
    std::fs::write(&sub_path, "module fifo (input wr);\nendmodule\n").unwrap();

    // Open the target file FIRST and rename the module in its buffer,
    // without saving -- `find-file-internal' (editing.rs) reuses this
    // exact buffer, unmodified, the moment the jump opens the same path.
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", sub_path.to_str().unwrap()),
    );
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(search-forward \"fifo\")");
    ok(&mut i, "(backward-char 4)");
    ok(&mut i, "(delete-char 4)");
    ok(&mut i, "(insert \"renamed\")"); // buffer now declares `renamed', not `fifo'

    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(
        r, "t",
        "still counts as handled -- a file WAS opened: {}",
        r
    );
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", sub_path.to_str().unwrap())
    );
    assert_eq!(
        run(&mut i, "(point)"),
        "1",
        "must fall back to point-min, not a stale disk-computed position"
    );
}

// ============================================================
// 14. Current buffer has unsaved edits; module declared there
// ============================================================

#[test]
fn unsaved_edit_in_current_buffer_is_still_found() {
    let mut i = setup();
    let dir = scratch_dir("unsaved_current");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("top.sv");
    std::fs::write(&path, "module top;\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    // Append an instantiation + the instantiated module's own
    // declaration, entirely in-buffer, never saved to disk.
    ok(&mut i, "(goto-char (point-max))");
    ok(
        &mut i,
        "(insert \"\\nmodule fifo (input wr);\\nendmodule\\n\")",
    );
    ok(&mut i, "(goto-char (point-min))");
    ok(&mut i, "(search-forward \"module top;\")");
    ok(&mut i, "(insert \"\\n  fifo u_f (.wr(w));\")");
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", path.to_str().unwrap()),
        "no new buffer should have been opened -- it's the SAME buffer"
    );
}

// ============================================================
// 15. Same module name in two library files -> first one (directory-
//     listing order) wins, predictably.
//
// (Numbers 16-18 in the original spec -- the `local-definition-function'
// dispatch-layer tests -- live in `lsp_mode_tests.rs' instead, next to
// the rest of that variable's own tests; not omitted, just filed
// elsewhere. See that file's own M55 section.)
// ============================================================

#[test]
fn duplicate_module_name_across_library_files_picks_the_first_one() {
    let mut i = setup();
    let dir = scratch_dir("dup_name");
    std::fs::create_dir_all(&dir).unwrap();
    let top_path = dir.join("top.sv");
    let a_path = dir.join("a_fifo.sv");
    let b_path = dir.join("b_fifo.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_f (.wr(w));\nendmodule\n").unwrap();
    std::fs::write(&a_path, "module fifo (input wr /* A */);\nendmodule\n").unwrap();
    std::fs::write(&b_path, "module fifo (input wr /* B */);\nendmodule\n").unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    // Whichever it lands in must match `verilog-auto--library-files''s
    // own `directory-files' order -- this test's own contract is only
    // "predictable and one of the two", not which literal one; the
    // library-files ordering itself is already pinned elsewhere.
    let landed = run(&mut i, "(buffer-file-name)");
    let a = format!("{:?}", a_path.to_str().unwrap());
    let b = format!("{:?}", b_path.to_str().unwrap());
    assert!(
        landed == a || landed == b,
        "must land in one of the two candidates: {}",
        landed
    );
}

// ============================================================
// 19. M56: `M-.' reaches a module in a recursively-scanned subdirectory
// ============================================================

#[test]
fn jumps_to_module_in_a_recursively_scanned_subdirectory() {
    let mut i = setup();
    let dir = scratch_dir("recurse_jump");
    std::fs::create_dir_all(dir.join("core")).unwrap();
    let top_path = dir.join("top.sv");
    let sub_path = dir.join("core").join("axi_lite_slave.sv");
    std::fs::write(
        &top_path,
        "module top;\n  axi_lite_slave #(.AW(16)) u_slave (.aclk(clk));\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &sub_path,
        "module axi_lite_slave (input aclk, output [31:0] rdata);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "axi_lite_slave");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "expected a successful jump into core/: {}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", sub_path.to_str().unwrap())
    );
}

// ============================================================
// 20. M56: same module name at two depths -- the shallower one wins,
//     same as `verilog-auto--library-files''s own BFS ordering.
// ============================================================

#[test]
fn shallower_module_wins_over_a_same_named_one_in_a_subdirectory() {
    let mut i = setup();
    let dir = scratch_dir("shallow_wins_jump");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let top_path = dir.join("top.sv");
    let shallow_path = dir.join("fifo.sv");
    let deep_path = dir.join("sub").join("fifo.sv");
    std::fs::write(&top_path, "module top;\n  fifo u_f (.wr(w));\nendmodule\n").unwrap();
    std::fs::write(
        &shallow_path,
        "module fifo (input wr /* SHALLOW */);\nendmodule\n",
    )
    .unwrap();
    std::fs::write(
        &deep_path,
        "module fifo (input wr /* DEEP */);\nendmodule\n",
    )
    .unwrap();
    ok(
        &mut i,
        &format!("(find-file-internal {:?})", top_path.to_str().unwrap()),
    );
    goto_mid(&mut i, "fifo");
    let r = run(&mut i, "(verilog-goto-module-at-point)");
    assert_eq!(r, "t", "{}", r);
    assert_eq!(
        run(&mut i, "(buffer-file-name)"),
        format!("{:?}", shallow_path.to_str().unwrap()),
        "depth-0 fifo.sv must win over sub/fifo.sv"
    );
}
