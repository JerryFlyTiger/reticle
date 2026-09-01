//! M36: the indentation engine (indent.el) -- tree-sitter block-depth for
//! c/c++/java/rust/emacs-lisp, a textual heuristic for python, and
//! copy-previous-line for bash. Both the `feed_keys' path (real TAB/RET
//! keypresses through the normal command dispatch) and the direct-call
//! path (invoking a `*-indent-line' function straight from elisp) are
//! exercised, per the M36 plan. Every expected value below was derived
//! by hand from indent.el's documented algorithm (see its file header)
//! and cross-checked against a one-off dump of real tree-sitter parses
//! for the exact snippets used here (the dump tool itself was deleted
//! after use, per the M33/M34 convention -- see indent.el's header for
//! the node-kind-name findings it produced).

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::editor::Editor;
use elisp::{Interp, Value};

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (60, 20);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => elisp::printer::prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn feed(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, keys: &str) {
    feed_keys(interp, ed, keys).unwrap_or_else(|e| panic!("feed_keys {:?}: {}", keys, e));
}

/// Raw buffer content (unlike `run', not the `prin1' printed form --
/// needed to compare multi-line text without hand-escaping newlines).
fn bs(interp: &mut Interp) -> String {
    match interp.eval_source("(buffer-string)") {
        Ok(Value::Str(s)) => (*s).clone(),
        other => panic!(
            "(buffer-string) didn't return a string: {:?}",
            other.is_ok()
        ),
    }
}

fn pt(interp: &mut Interp) -> i64 {
    match interp.eval_source("(point)") {
        Ok(Value::Int(n)) => n,
        other => panic!("(point) didn't return an int: {:?}", other.is_ok()),
    }
}

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

// --- C: nested TAB, closing-brace dedent, RET after `{', RET after `;' ---

#[test]
fn c_nested_block_tab_indents_to_correct_depth() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int foo() {\n    if (x) {\nbar();\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bar\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "int foo() {\n    if (x) {\n        bar();\n    }\n}\n"
    );
}

#[test]
fn c_closing_brace_line_dedents() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int foo() {\n    if (x) {\n        bar();\n}\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "bar();\n"));
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "int foo() {\n    if (x) {\n        bar();\n    }\n}\n"
    );
}

#[test]
fn c_ret_after_open_brace_indents_one_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(&mut i, &format!("(insert {:?})", "int foo() {"));
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "int foo() {\n    ");
    assert_eq!(pt(&mut i), 17);
}

#[test]
fn c_ret_after_statement_stays_same_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n    int x = 1;"),
    );
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "int foo() {\n    int x = 1;\n    ");
}

// --- C++: one sanity check (shares c's block-node-type table) ------------

#[test]
fn cpp_nested_block_tab_indents_to_correct_depth() {
    let (mut i, ed) = setup();
    run(&mut i, "(c++-mode)");
    let src =
        "class Foo {\npublic:\n    void bar() {\n        if (x) {\nbaz();\n        }\n    }\n};\n";
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"baz\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    let want =
        "class Foo {\npublic:\n    void bar() {\n        if (x) {\n            baz();\n        }\n    }\n};\n";
    assert_eq!(bs(&mut i), want);
}

// --- Rust: nested TAB (direct call), closing-brace dedent, RET cases -----

#[test]
fn rust_nested_block_direct_call_computes_correct_depth() {
    let (mut i, _ed) = setup();
    run(&mut i, "(rust-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "fn foo() {\n    if x {\nbar();\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bar\")");
    run(&mut i, "(beginning-of-line)");
    // Direct call to the raw indent-line-function, bypassing command
    // dispatch entirely -- the other half of "feed_keys + direct call"
    // coverage the M36 plan asks for.
    assert_eq!(run(&mut i, "(rust-indent-line)"), "8");
}

#[test]
fn rust_closing_brace_line_dedents() {
    let (mut i, ed) = setup();
    run(&mut i, "(rust-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "fn foo() {\n    if x {\n        bar();\n}\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "bar();\n"));
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "fn foo() {\n    if x {\n        bar();\n    }\n}\n"
    );
}

// NOTE (found via this very test failing its first hand-derived
// expectation, then re-diagnosed with a throwaway debug binary):
// tree-sitter-rust's error recovery for an unclosed `fn' body is
// markedly worse than tree-sitter-c's -- C gracefully parses `int foo()
// {' as `(compound_statement (MISSING "}"))' (see indent.el's header),
// but rust ALWAYS wraps the entire still-open `fn' (signature and all)
// in one flat ERROR node, with no MISSING recovery at all, REGARDLESS
// of whether it's preceded by other, already-complete, well-formed
// functions. So "RET right after typing `{'" for rust-mode can never
// reach the smart depth engine at all -- it always exercises the
// ERROR-tree fallback (`indent--copy-previous-indentation') instead,
// same as deliberately broken code would. Not a bug in indent.el: the
// documented fallback handles it exactly as designed (no crash,
// "previous line's indentation" rather than the ideal "+1 level" -- see
// indent.el's header, the ERROR-tree section). Rather than assert a
// smart-engine result this language structurally can't reach, this
// test pins the (still correct, still non-crashing) fallback outcome.
#[test]
fn rust_ret_after_open_brace_falls_back_to_error_tree_recovery() {
    let (mut i, ed) = setup();
    run(&mut i, "(rust-mode)");
    run(&mut i, &format!("(insert {:?})", "fn foo() {"));
    feed(&mut i, &ed, "RET");
    // No previous non-blank line exists, so the fallback's own fallback
    // (`indent--prev-nonblank-line-start' finds nothing) is 0 -- not a
    // crash, not `nil', just plain no indentation added.
    assert_eq!(bs(&mut i), "fn foo() {\n");
}

#[test]
fn rust_ret_after_statement_stays_same_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(rust-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "fn foo() {\n    let x = 1;"),
    );
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "fn foo() {\n    let x = 1;\n    ");
}

// --- Java: nested TAB, closing-brace dedent, RET cases --------------------

#[test]
fn java_nested_block_tab_indents_to_correct_depth() {
    let (mut i, ed) = setup();
    run(&mut i, "(java-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "class Foo {\n    void bar() {\n        if (x) {\nbaz();\n        }\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"baz\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "class Foo {\n    void bar() {\n        if (x) {\n            baz();\n        }\n    }\n}\n"
    );
}

#[test]
fn java_closing_brace_line_dedents() {
    let (mut i, ed) = setup();
    run(&mut i, "(java-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "class Foo {\n    void bar() {\n        if (x) {\n            baz();\n}\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "baz();\n"));
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "class Foo {\n    void bar() {\n        if (x) {\n            baz();\n        }\n    }\n}\n"
    );
}

#[test]
fn java_ret_after_open_brace_indents_one_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(java-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "class Foo {\n    void bar() {"),
    );
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "class Foo {\n    void bar() {\n        ");
}

#[test]
fn java_ret_after_statement_stays_same_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(java-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "class Foo {\n    void bar() {\n        int x = 1;"
        ),
    );
    feed(&mut i, &ed, "RET");
    assert_eq!(
        bs(&mut i),
        "class Foo {\n    void bar() {\n        int x = 1;\n        "
    );
}

// --- Python: heuristic (colon/dedent-keyword/copy) ------------------------

#[test]
fn python_ret_after_colon_indents_one_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(python-mode)");
    run(&mut i, &format!("(insert {:?})", "if x:"));
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "if x:\n    ");
}

#[test]
fn python_else_tab_aligns_with_if() {
    let (mut i, ed) = setup();
    run(&mut i, "(python-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "if x:\n    pass\n        else:"),
    );
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "if x:\n    pass\nelse:");
}

#[test]
fn python_normal_line_tab_follows_previous_line() {
    let (mut i, ed) = setup();
    run(&mut i, "(python-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "if x:\n    pass\n  y = 1"),
    );
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "if x:\n    pass\n    y = 1");
}

// --- Elisp: bracket-depth x 2, `)' dedent ----------------------------------

#[test]
fn elisp_nested_form_tab_indents_two_times_depth() {
    let (mut i, ed) = setup();
    run(&mut i, "(emacs-lisp-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "(defun foo (x)\n  (bar\nbaz))"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"baz\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "(defun foo (x)\n  (bar\n    baz))");
}

#[test]
fn elisp_close_paren_line_dedents() {
    let (mut i, ed) = setup();
    run(&mut i, "(emacs-lisp-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "(defun foo (x)\n  (bar)\n    )"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "(bar)\n"));
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "(defun foo (x)\n  (bar)\n)");
}

// --- Verilog: module/seq_block/case_item/generate block-depth, word ------
// closers (`end'/`endmodule'/...) -- see indent.el's M38 header section for
// the full dump-verified rationale (word closers vs. c-like's single-
// character closers; why `case_statement'/`generate_region' are
// deliberately EXCLUDED from the block-type list while `case_item'/
// `loop_generate_construct'/`if_generate_construct' are included instead;
// the one documented module-header-line quirk).

#[test]
fn verilog_module_body_tab_indents_one_level() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "module m;\nwire w;\nendmodule\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "module m;\n    wire w;\nendmodule\n");
}

#[test]
fn verilog_nested_block_tab_indents_to_correct_depth() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module foo;\n  always @(*) begin\n    if (x) begin\nold_style = 1;\n    end\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"old_style\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "module foo;\n  always @(*) begin\n    if (x) begin\n            old_style = 1;\n    end\n  end\nendmodule\n"
    );
}

/// Three levels of `seq_block' nesting (always -> if -> if), confirming
/// each level is counted EXACTLY ONCE (dump-verified against a hand-walked
/// ancestor chain -- see indent.el's block-node-types comment) rather than
/// double-counted against the `always_construct'/`conditional_statement'
/// each one sits inside (neither is a block type itself).
#[test]
fn verilog_triple_nested_begin_counts_each_level_exactly_once() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module nested_demo;\n  reg a;\n  always @(*) begin\n    if (a) begin\n      if (a) begin\na = 0;\n      end\n    end\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"a = 0\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "module nested_demo;\n  reg a;\n  always @(*) begin\n    if (a) begin\n      if (a) begin\n                a = 0;\n      end\n    end\n  end\nendmodule\n"
    );
}

#[test]
fn verilog_closing_end_line_dedents() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module foo;\n  always @(*) begin\n    if (x) begin\n            old_style = 1;\nend\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(search-forward {:?})", "old_style = 1;\n"),
    );
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "module foo;\n  always @(*) begin\n    if (x) begin\n            old_style = 1;\n        end\n  end\nendmodule\n"
    );
}

#[test]
fn verilog_endmodule_dedents_to_column_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "module m;\n  wire w;\nendmodule\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endmodule\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "0");
}

/// `case_item' (one per arm, INCLUDING `default:') supplies the one extra
/// level case contents need; `case_statement' itself is deliberately NOT a
/// block type, which is exactly why `case'/`endcase' land at the SAME
/// depth instead of `endcase' dedenting one level below `case' -- see
/// indent.el's header for why counting `case_statement' would have gotten
/// this wrong (a case_item's contents would live at the same distance
/// from `initial begin' whichever way this is decided, but `case' and
/// `endcase' would end up at DIFFERENT depths, not aligned).
#[test]
fn verilog_case_and_endcase_align_case_items_indent_one_level_more() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  initial begin\ncase (x)\n1: y = 1;\ndefault: y = 2;\nendcase\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"case (x)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "`case (x)` should align with an ordinary statement inside `initial begin`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"1: y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "12",
        "case item content one level deeper than `case`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"default: y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "12",
        "the `default:` case item is at the same depth as a numbered one"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endcase\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "`endcase` must align with `case`, not dedent below it"
    );
}

/// `function_body_declaration'/`task_body_declaration' wrap params+body+
/// the closing `endfunction'/`endtask' directly (no begin/end at all --
/// dump-verified), so the function/task's own header line and its body
/// must land one level apart, and `endfunction' must dedent back to the
/// header's own depth.
#[test]
fn verilog_function_body_indents_one_level_and_endfunction_dedents() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  function automatic int add_one(int x);\nreturn x + 1;\nendfunction\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"return\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "function body is one level deeper than the function's own header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endfunction\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "`endfunction` dedents back to the function header's own depth"
    );
}

/// `generate'/`endgenerate' are, like `case'/`endcase', a FLAT container
/// (dump-verified: the `generate' keyword is itself a descendant of
/// `generate_region', the very node whose content needs the extra level)
/// -- `loop_generate_construct'/`generate_block' supply the two levels a
/// for-generate body needs instead, and `endgenerate' is excluded from the
/// closer list to match, aligning it with `generate' rather than dedenting
/// past it. See indent.el's header for the full reasoning.
#[test]
fn verilog_generate_for_loop_body_indents_and_endgenerate_aligns_with_generate() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  genvar i;\ngenerate\nfor (i = 0; i < 4; i = i + 1) begin : g\nwire w;\nend\nendgenerate\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    // Bare "generate" (not "generate\n"): `search-forward` leaves point
    // AFTER the match, so a needle including the trailing newline would
    // land point at the START of the FOLLOWING ("for (...)") line instead
    // of on the "generate" line itself -- caught by this test's own first
    // run, which silently computed the "for" line's depth (8) instead of
    // "generate"'s (4) until this was fixed.
    run(&mut i, "(search-forward \"generate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "`generate` aligns with the module body"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"for (\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "the for-generate's own line is one level inside `generate`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire w\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "12",
        "the generate_block's own content is one level inside the for-generate line"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endgenerate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "`endgenerate` aligns with `generate`, not the for-generate line"
    );
}

/// M38 review-round documented v1 gap (see indent.el's M38 header section,
/// the paragraph right after the case/generate alignment one): a
/// `generate'...`endgenerate' region whose direct children MIX a plain
/// item (no enclosing construct of its own) with an `if_generate_construct'/
/// `loop_generate_construct' lands the two sibling kinds at DIFFERENT
/// columns -- the plain item has no block-type node of its own to supply
/// the extra level `generate_region' is deliberately excluded from
/// providing (see above), while the construct gets one from
/// `if_generate_construct' itself. Pinned here as a known, intentional
/// outcome -- reversing which side owns the count would fix this rare
/// mixed case but break the far more common pure-generate-construct case
/// (see indent.el's own paragraph for why), so this is left as a
/// documented imprecision, the same policy as
/// `verilog_module_header_line_has_a_documented_one_level_indent_quirk'
/// just below.
#[test]
fn verilog_mixed_generate_region_plain_item_and_if_generate_construct_indent_at_different_columns()
{
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module gm;\ngenerate\nwire plain_w;\nif (1) begin : blk\nwire w2;\nend\nendgenerate\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"generate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "`generate` aligns with the module body"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire plain_w\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "documented v1 gap: a plain generate-region item lands at the SAME column as \
         `generate` itself, not one level in -- see indent.el's header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (1)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "documented v1 gap: an if-generate construct sibling lands one level deeper than \
         the plain item right above it, even though both are direct children of the same \
         generate region -- see indent.el's header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endgenerate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "`endgenerate` still aligns with `generate`, unaffected by the mixed content above it"
    );
}

/// M38 documented v1 gap (see indent.el's M38 header section): unlike
/// function/task, a module has no per-item wrapper separating its OWN
/// header line from its body, so `module_declaration' must directly be a
/// block type for the body to indent at all -- the accepted side effect is
/// that re-TABbing the module's own single-line header computes one level
/// too deep instead of column 0. Pinned here as a known, intentional
/// outcome, not a silent accident -- mirrors how
/// `rust_ret_after_open_brace_falls_back_to_error_tree_recovery' pins
/// rust's own documented ERROR-tree gap.
#[test]
fn verilog_module_header_line_has_a_documented_one_level_indent_quirk() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "module m;\n  wire w;\nendmodule\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "documented v1 gap: the module's own header line computes one level deep, not 0 -- \
         see indent.el's M38 header section"
    );
}

/// Blank-line MISSING-token handling (see indent.el's header) applies
/// unchanged to verilog: the nearest real character before a blank line
/// between two top-level `endmodule's is that PREVIOUS `endmodule' itself,
/// a closer, so the blank line correctly computes 0, not 1.
#[test]
fn verilog_blank_line_between_top_level_endmodules_computes_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module foo;\nendmodule\n\nmodule bar;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(search-forward {:?})", "module foo;\nendmodule\n"),
    );
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "0");
}

/// Verilog's error recovery for an unclosed `begin' is even less
/// forgiving than rust's own documented gap (see indent.el's header): the
/// ENTIRE enclosing module, not just the still-open construct, flattens
/// into one ERROR node, so RET right after typing `begin' always falls
/// back to `indent--copy-previous-indentation' -- pinned here the same
/// way `rust_ret_after_open_brace_falls_back_to_error_tree_recovery' pins
/// rust's.
#[test]
fn verilog_ret_after_open_begin_falls_back_to_error_tree_recovery() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "module m;\n  always @(*) begin"),
    );
    feed(&mut i, &ed, "RET");
    // Fallback copies the nearest non-blank line's own indentation --
    // "  always @(*) begin" has 2 leading spaces, so the new line gets 2,
    // not the ideal (unreachable here) "one level deeper".
    assert_eq!(bs(&mut i), "module m;\n  always @(*) begin\n  ");
}

// --- Bash: v1 copy-previous-line's indentation -----------------------------

#[test]
fn sh_tab_copies_previous_line_indentation() {
    let (mut i, ed) = setup();
    run(&mut i, "(sh-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "if [ \"$x\" = 1 ]; then\n    echo hi\necho bye"
        ),
    );
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "if [ \"$x\" = 1 ]; then\n    echo hi\n    echo bye"
    );
}

// --- ERROR-tree fallback ----------------------------------------------------

#[test]
fn broken_c_code_tab_does_not_crash_and_falls_back_to_previous_indentation() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    // A stray, unmatched trailing `}' -- dump-verified to parse as its
    // own ERROR node while the two REAL closing braces above it stay
    // completely normal (see indent.el's header). Deliberately given
    // wrong (8-space) leading whitespace so a successful fallback is
    // visibly different from a no-op.
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int main() {\n    if (x) {\n        foo();\n    }\n}\n        }\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "}\n}\n"));
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "int main() {\n    if (x) {\n        foo();\n    }\n}\n}\n",
        "ERROR-tree fallback should copy the previous line's indentation (0), not crash"
    );
}

// --- The MISSING-token / query-position fix (see indent.el's header) -----

#[test]
fn c_blank_line_between_top_level_closing_braces_computes_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n}\n\nint bar() {\n}\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(search-forward {:?})", "int foo() {\n}\n"),
    );
    // Point is now on the blank line between the two top-level
    // functions -- direct call, no keypress (the other half of
    // "feed_keys + direct call" coverage). The nearest real character
    // before this blank line is itself a CLOSER (the first `}'), which
    // must dedent the query back OUT of the block it closed, not leave
    // it counted as still "inside" -- the specific bug the dump
    // investigation caught (see indent.el's header).
    assert_eq!(run(&mut i, "(c-indent-line)"), "0");
}

// --- Spaces only -------------------------------------------------------------

#[test]
fn indentation_is_always_spaces_never_tab_characters() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int foo() {\n    if (x) {\nbar();\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bar\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    let text = bs(&mut i);
    assert!(
        !text.contains('\t'),
        "indentation must use spaces only, got: {:?}",
        text
    );
    assert!(
        text.contains("        bar();"),
        "expected 8-space indentation: {:?}",
        text
    );
}

// --- Point semantics (indent-for-tab-command, GNU convention) -------------

#[test]
fn point_within_old_indentation_moves_to_new_first_char() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n  int x = 1;"),
    );
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "C-f");
    assert_eq!(
        pt(&mut i),
        14,
        "sanity: point is inside the old 2-space indentation"
    );
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "int foo() {\n    int x = 1;");
    assert_eq!(
        pt(&mut i),
        17,
        "point must land on the new indentation's first char"
    );
}

#[test]
fn point_within_text_keeps_relative_offset_after_reindent() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n  int x = 1;"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"int x\")");
    let before = pt(&mut i);
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "int foo() {\n    int x = 1;");
    // Old indentation was 2 spaces, new is 4 -- point was WITHIN the
    // text (past the old indentation), so it must shift by exactly the
    // same +2 delta, staying on the same character (`x'), not snap to
    // the new indentation's start.
    assert_eq!(pt(&mut i), before + 2);
}

// --- Regression pins: buffers/paths this milestone must NOT touch --------

#[test]
fn org_buffer_tab_is_still_org_cycle() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "* Heading\nbody\n"));
    run(&mut i, "(org-mode)");
    run(&mut i, "(goto-char (point-min))");
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        "* Heading\nbody\n",
        "org-cycle must not edit the buffer text"
    );
    let grid = core::redisplay::render(&i, &ed);
    let row0: String = grid.lines[0]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    assert!(
        row0.trim_end().contains("..."),
        "expected org-cycle to fold the heading: {:?}",
        row0
    );
}

#[test]
fn minibuffer_tab_is_still_completion() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "hello world"));
    feed(&mut i, &ed, "M-x");
    type_str(&mut i, &ed, "beginning-of-b");
    // Raw TAB key: the minibuffer branch in `handle_key' must intercept
    // this before `dispatch_key' (and so before M36's new global TAB
    // binding) ever sees it.
    handle_key(&mut i, &ed, Key::Char(9));
    feed(&mut i, &ed, "RET");
    assert!(
        ed.borrow().minibuffer.is_none(),
        "M-x should have submitted"
    );
    assert_eq!(
        pt(&mut i),
        1,
        "TAB must have completed to beginning-of-buffer (which moves point to 1), not indented"
    );
}

#[test]
fn ielm_ret_is_unaffected_by_prog_mode_ret_binding() {
    let (mut i, ed) = setup();
    run(&mut i, "(ielm)");
    type_str(&mut i, &ed, "(+ 1 2)");
    feed(&mut i, &ed, "RET");
    let text = bs(&mut i);
    assert!(
        text.contains("(+ 1 2)\n3\n"),
        "ielm's own local RET binding must evaluate, not `newline-and-indent': {:?}",
        text
    );
}

// --- evil.el integration (o/O autoindent) ---------------------------------

#[test]
fn evil_o_in_block_opens_indented_line() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n    int x = 1;\n}"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "int x = 1;"));
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "o");
    assert_eq!(bs(&mut i), "int foo() {\n    int x = 1;\n    \n}");
}

// --- Review fix #1: normal/visual/op-pending TAB no longer bypasses --------
// evil's "normal state edits nothing" guarantee (M36's global TAB binding
// is a REAL keymap command, not the self-insert fallback M34's
// `inhibit-self-insert' guards -- see evil.el's keymap-population section).

#[test]
fn evil_normal_state_tab_does_not_touch_buffer_and_echoes_undefined() {
    let (mut i, ed) = setup();
    // c-mode has an indent-line-function -- proves TAB doesn't silently reindent either.
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n    bar();\n}"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    let before = bs(&mut i);
    feed(&mut i, &ed, "TAB");
    assert_eq!(
        bs(&mut i),
        before,
        "TAB in normal state must not reindent or self-insert"
    );
    assert_eq!(
        ed.borrow().echo.clone().as_deref(),
        Some("TAB is undefined")
    );
}

#[test]
fn evil_visual_state_tab_does_not_touch_buffer_and_echoes_undefined() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "hello world"));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "v");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(
        ed.borrow().echo.clone().as_deref(),
        Some("TAB is undefined")
    );
}

#[test]
fn evil_op_pending_tab_cancels_back_to_normal() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "hello world"));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "d");
    assert_eq!(run(&mut i, "evil--state"), "operator-pending");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

#[test]
fn evil_insert_state_tab_in_prog_buffer_still_indents() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "int foo() {\n  int x = 1;"),
    );
    run(&mut i, "(beginning-of-line)");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "i");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "int foo() {\n    int x = 1;");
}

// --- Review fix #2: evil's normal/visual RET is a real vim motion --------
// (`+'-equivalent), not a silent `newline'/`newline-and-indent' side
// effect; op-pending's RET cancels (`d<CR>' as a linewise operator target
// is out of v1 scope); insert-state RET is unaffected (falls through to
// `newline-and-indent' exactly as before).

#[test]
fn evil_normal_state_ret_moves_to_next_line_first_non_blank() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "first\n    second\nthird"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "RET");
    assert_eq!(
        bs(&mut i),
        "first\n    second\nthird",
        "RET in normal state must not edit buffer"
    );
    assert_eq!(
        pt(&mut i),
        11,
        "point must land on next line's first non-blank char"
    );
}

#[test]
fn evil_normal_state_ret_with_count_moves_n_lines() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "one\ntwo\nthree\nfour"));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "3 RET");
    assert_eq!(
        pt(&mut i),
        15,
        "3 RET from line 1 must land on line 4 (\"four\")"
    );
}

#[test]
fn evil_visual_state_ret_extends_selection() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "first\n    second\nthird"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "v");
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "first\n    second\nthird");
    assert_eq!(pt(&mut i), 11);
    // Mark must stay put -- a genuine extend, not a fresh selection.
    assert_eq!(run(&mut i, "(mark)"), "1");
}

#[test]
fn evil_op_pending_ret_cancels_back_to_normal() {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", "hello world"));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "d");
    feed(&mut i, &ed, "RET");
    // RET must not delete/insert anything as an operator target.
    assert_eq!(bs(&mut i), "hello world");
    assert_eq!(run(&mut i, "evil--state"), "normal");
}

#[test]
fn evil_insert_state_ret_in_prog_buffer_still_newline_and_indents() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(&mut i, &format!("(insert {:?})", "int foo() {"));
    run(&mut i, "(evil-mode 1)");
    feed(&mut i, &ed, "a");
    feed(&mut i, &ed, "RET");
    assert_eq!(bs(&mut i), "int foo() {\n    ");
}

// --- Review fix #3: a size cap on the tree-sitter reparse ------------------

#[test]
fn oversized_buffer_skips_treesit_and_falls_back_to_copy_previous() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(&mut i, "(setq-local indent-treesit-max-chars 5)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int foo() {\n    if (x) {\nbar();\n    }\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bar\")");
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    // The smart depth engine would give 8 (2 levels x 4); with the size
    // cap tripped, it must instead copy the previous line's own
    // indentation ("    if (x) {" = 4).
    assert_eq!(
        bs(&mut i),
        "int foo() {\n    if (x) {\n    bar();\n    }\n}\n"
    );
}

// --- Review fix #4: closing-bracket dedent is node-type-checked, not a ---
// bare character comparison (a `}' inside a comment/string must not
// falsely trigger it).

#[test]
fn closing_brace_inside_comment_does_not_falsely_trigger_dedent() {
    let (mut i, _ed) = setup();
    run(&mut i, "(c-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "int foo() {\n    bar(); // returns {1, 2, 3}\n\n}\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        &format!("(search-forward {:?})", "returns {1, 2, 3}\n"),
    );
    // Direct call, point on the blank line right after the comment -- the
    // `}' that ends the COMMENT TEXT must not be mistaken for a real
    // closing brace and dedent this back to 0.
    assert_eq!(run(&mut i, "(c-indent-line)"), "4");
}

// --- Review fix #5: current-indentation is tab-stop aware -----------------

#[test]
fn current_indentation_is_tab_stop_aware_for_copy_previous_fallback() {
    let (mut i, ed) = setup();
    run(&mut i, "(sh-mode)");
    run(&mut i, &format!("(insert {:?})", "\tfoo\nbar"));
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    // A single leading tab advances from column 0 to the next multiple of
    // 8, i.e. column 8 -- not column 1.
    assert_eq!(bs(&mut i), "\tfoo\n        bar");
}

// --- Review fix #6: treesit--prog-mode-setup tolerates an old-style ------
// 3-arg call (WIDTH/INDENT-FN now &optional, defaulting to 4/nil).

#[test]
fn treesit_prog_mode_setup_tolerates_old_three_arg_call() {
    let (mut i, _ed) = setup();
    let r = run(&mut i, "(treesit--prog-mode-setup 'c-mode 'c 'c-mode-hook)");
    assert!(
        !r.starts_with("ERROR"),
        "old 3-arg call must not error: {}",
        r
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
    assert_eq!(run(&mut i, "indent-line-function"), "nil");
}

// --- General: TAB with no indent-line-function still self-inserts --------

#[test]
fn tab_in_fundamental_mode_still_inserts_literal_tab() {
    let (mut i, ed) = setup();
    run(&mut i, "(fundamental-mode)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "\t");
}

// --- M73: `standard-indent-width' detection from the file's own content ---
// See indent.el's header (M73 section) for the algorithm. Part A exercises
// `indent--detect-width' directly against inserted content; part B walks
// the real `find-file-internal' open path end to end.

/// A scratch directory that deletes itself on drop -- same shape as
/// lang_modes_tests.rs's `Scratch' (this file's own copy, per this repo's
/// "no shared test helper module" convention).
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn unique_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn temp_dir(tag: &str) -> Scratch {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64;
    let dir = std::env::temp_dir().join(format!(
        "reticle_indentdetect_{}_{}_{}_{}",
        tag,
        std::process::id(),
        nanos,
        unique_seq()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn write_and_open(
    interp: &mut Interp,
    dir: &std::path::Path,
    name: &str,
    contents: &str,
) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    run(
        interp,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    )
}

// --- A. `indent--detect-width' directly -----------------------------------

#[test]
fn detect_width_two_space_content() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "x\n  a\n  b\n  c\n  d\n  e\n"),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "2");
}

#[test]
fn detect_width_three_space_content() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "x\n   a\n   b\n   c\n   d\n   e\n"),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "3");
}

#[test]
fn detect_width_four_space_content() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "x\n    a\n    b\n    c\n    d\n    e\n"),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "4");
}

#[test]
fn detect_width_eight_space_content() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "x\n        a\n        b\n        c\n        d\n        e\n"
        ),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "8");
}

#[test]
fn detect_width_tab_indented_returns_nil() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!("(insert {:?})", "x\n\ta\n\tb\n\tc\n\td\n\te\n"),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "nil");
}

#[test]
fn detect_width_fewer_than_min_samples_returns_nil() {
    let (mut i, _ed) = setup();
    // Only 4 indented lines -- below `indent--detect-min-samples' (5).
    run(&mut i, &format!("(insert {:?})", "x\n  a\n  b\n  c\n  d\n"));
    assert_eq!(run(&mut i, "(indent--detect-width)"), "nil");
}

#[test]
fn detect_width_alignment_style_odd_columns_returns_nil() {
    let (mut i, _ed) = setup();
    // 1/5/7/9/11-space indents: no candidate width (8 4 3 2) evenly
    // divides >= 90% of these -- an alignment style, not a fixed step.
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "x\n a\n     b\n       c\n         d\n           e\n"
        ),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "nil");
}

/// The 90% tolerance, pinned from ABOVE: nine 2-divisible samples plus one
/// stray odd column is exactly 9/10, so detection still answers 2. Without
/// a tolerance (i.e. requiring every sample to divide) this would be nil --
/// one reformatted line in an otherwise uniform file would disable
/// detection for the whole buffer. Added after M73's mutation run showed
/// the threshold constant had NO coverage: raising it from 90% to 100% left
/// every existing test green, because the `soc_top' fixture below happens
/// to be 100% 2-divisible and never exercises the slack.
#[test]
fn detect_width_one_odd_line_among_nine_still_detects_two() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "x\n  a\n  b\n    c\n    d\n  e\n  f\n    g\n  h\n  i\n   odd\n"
        ),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "2");
}

/// The same tolerance pinned from BELOW: eight 2-divisible samples and two
/// odd ones is 8/10 = 80%, under the threshold, so detection declines
/// rather than guessing. Together with the test above this fixes the
/// constant from both sides; neither alone does.
#[test]
fn detect_width_two_odd_lines_among_ten_declines() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "x\n  a\n  b\n    c\n    d\n  e\n  f\n    g\n  h\n   odd\n     odder\n"
        ),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "nil");
}

/// The `soc_top.sv' shape: a 2-space module body plus 4-space-aligned
/// port-connection continuation lines inside an instantiation. The delta-
/// between-adjacent-lines method would answer 4 (the outlier continuation
/// style); the divisibility method answers 2 (the body, since 2 also
/// evenly divides every 4-space sample) -- this is the case the spec calls
/// out as the reason divisibility was chosen over deltas.
#[test]
fn detect_width_soc_top_shape_body_two_continuation_four_returns_two() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module soc_top;\n  wire a;\n  wire b;\n  wire c;\n  wire d;\n  wire e;\n  inst u_inst (\n    .clk    (clk),\n    .rst_n  (rst_n),\n    .data   (data)\n  );\nendmodule\n"
        ),
    );
    assert_eq!(run(&mut i, "(indent--detect-width)"), "2");
}

#[test]
fn detect_width_scan_cap_ignores_lines_past_max_lines() {
    let (mut i, _ed) = setup();
    // First 500 lines 2-space (well past `indent--detect-min-samples'),
    // then 4-space after -- only the first 500 lines are ever scanned, so
    // the trailing 4-space lines must not sway the answer.
    let mut content = String::from("x\n");
    content.push_str(&"  a;\n".repeat(500));
    content.push_str(&"    b;\n".repeat(20));
    run(&mut i, &format!("(insert {:?})", content));
    assert_eq!(run(&mut i, "(indent--detect-width)"), "2");
}

/// M73 review fix: `indent--first-non-blank-pos' only skips `?\s'/`?\t',
/// so on a CRLF file it stops AT the line's trailing `\r', not at EOL --
/// before this fix a blank line that's really "N spaces + CR" got its N
/// spaces pushed into `samples' as if it were real leading indentation.
/// Repro (real TUI, confirmed before this fix): convert `demo/rtl/core/
/// alu.sv' to CRLF with a 3-space-then-CR blank line, `o' in the module
/// body landed at column 4 instead of 2 -- the exact M73 defect, back.
/// This test pins the same shape without a TUI: the CRLF content below
/// must detect identically to its LF twin.
#[test]
fn detect_width_crlf_blank_line_with_trailing_spaces_matches_lf_twin() {
    let (mut i, _ed) = setup();
    let lf =
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n   \n  logic e;\nendmodule\n";
    let crlf = lf.replace('\n', "\r\n");
    let (mut j, _ed2) = setup();
    run(&mut i, &format!("(insert {:?})", lf));
    run(&mut j, &format!("(insert {:?})", crlf));
    let lf_result = run(&mut i, "(indent--detect-width)");
    let crlf_result = run(&mut j, "(indent--detect-width)");
    assert_eq!(lf_result, "2", "LF fixture itself must detect 2");
    assert_eq!(
        crlf_result, lf_result,
        "CRLF twin (blank line is 3 spaces + CR) must detect the same as its LF original"
    );
}

/// M73 review fix: before this milestone's `(> tab-lines total)' branch
/// had a test hitting it, every tab fixture had zero space samples, so
/// `(> tab-lines total)' and `(< total indent--detect-min-samples)'
/// were always simultaneously true -- the tab-lines branch itself was
/// never independently exercised. This fixture has 5 real 2-space
/// samples (clears `indent--detect-min-samples' on its own) AND 6
/// tab-indented lines outnumbering them, so only the `(> tab-lines
/// total)' branch, not the min-samples one, can be what returns nil.
#[test]
fn detect_width_tab_lines_outnumbering_space_samples_returns_nil() {
    let (mut i, _ed) = setup();
    let content = "x\n\ta\n\tb\n\tc\n\td\n\te\n\tf\n  g\n  h\n  i\n  j\n  k\n";
    run(&mut i, &format!("(insert {:?})", content));
    assert_eq!(run(&mut i, "(indent--detect-width)"), "nil");
}

// --- B. End to end via `find-file-internal' --------------------------------

#[test]
fn find_file_two_space_sv_opens_new_lines_at_detected_width() {
    let (mut i, ed) = setup();
    let dir = temp_dir("b10");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\n  always_comb begin\n    a = b;\n  end\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
    run(&mut i, "(evil-mode 1)");
    // Open a new line right after "logic e;" -- module-body depth 1 -> 2 columns.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "logic e;"));
    feed(&mut i, &ed, "o");
    assert_eq!(
        bs(&mut i),
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\n  \n  always_comb begin\n    a = b;\n  end\nendmodule\n"
    );
    // Open a new line right after "a = b;" -- module + always_comb begin, depth 2 -> 4 columns.
    feed(&mut i, &ed, "ESC");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "a = b;"));
    feed(&mut i, &ed, "o");
    assert_eq!(
        bs(&mut i),
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\n  \n  always_comb begin\n    a = b;\n    \n  end\nendmodule\n"
    );
}

// NOTE (M73 review): these two tests' `standard-indent-width' assertion
// alone is a false test -- verilog-mode's OWN default is also 4, so the
// assertion passes identically whether `indent--maybe-detect-width' ran
// or was never wired up at all (confirmed by physically commenting out
// the `(indent--maybe-detect-width)' call in modes.el: both tests stay
// green). They are kept, but each now ALSO asserts what
// `(indent--detect-width)' itself returned on the same buffer, so the
// test can tell "detected 4" apart from "fell back to the mode default
// 4". These two pin down "detection result == mode default" /
// "detection fell back to the mode default" as states, NOT the
// find-file integration point -- that point is covered by the four
// OTHER end-to-end tests (`find_file_two_space_sv_opens_new_lines_at_
// detected_width', `find_file_two_buffers_keep_independent_widths',
// `manually_switching_to_verilog_mode_reruns_detection',
// `user_hook_setq_local_wins_over_detection'), which all use a width
// (2, or a hook override) that provably differs from the mode default.
#[test]
fn find_file_four_space_sv_sets_width_to_four() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("b11");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n    logic a;\n    logic b;\n    logic c;\n    logic d;\n    logic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
    assert_eq!(
        run(&mut i, "(indent--detect-width)"),
        "4",
        "detection itself must have found 4 here, not merely fallen back to it"
    );
}

#[test]
fn find_file_tab_indented_sv_keeps_mode_default() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("b12");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n\tlogic a;\n\tlogic b;\n\tlogic c;\n\tlogic d;\n\tlogic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
    assert_eq!(
        run(&mut i, "(indent--detect-width)"),
        "nil",
        "detection itself must have declined (nil) here, not \"detected\" 4"
    );
}

#[test]
fn find_file_two_space_sv_with_detection_disabled_keeps_mode_default() {
    let (mut i, _ed) = setup();
    run(&mut i, "(setq indent-detect-width nil)");
    let dir = temp_dir("b13");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
}

#[test]
fn find_file_two_buffers_keep_independent_widths() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("b14");
    write_and_open(
        &mut i,
        &dir,
        "two.sv",
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
    write_and_open(
        &mut i,
        &dir,
        "four.sv",
        "module m;\n    logic a;\n    logic b;\n    logic c;\n    logic d;\n    logic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
    run(&mut i, "(switch-to-buffer \"two.sv\")");
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
    run(&mut i, "(switch-to-buffer \"four.sv\")");
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
}

#[test]
fn find_file_org_buffer_never_runs_detection() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("b15");
    write_and_open(
        &mut i,
        &dir,
        "notes.org",
        "* heading\n  a\n  b\n  c\n  d\n  e\n",
    );
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "org-mode");
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
}

#[test]
fn manually_switching_to_verilog_mode_reruns_detection() {
    let (mut i, _ed) = setup();
    run(&mut i, "(fundamental-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\nendmodule\n"
        ),
    );
    run(&mut i, "(verilog-mode)");
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
}

#[test]
fn set_indent_width_changes_value_and_rejects_out_of_range() {
    let (mut i, ed) = setup();
    run(&mut i, "(c-mode)");
    run(&mut i, &format!("(insert {:?})", "int foo() {\nbar();"));
    run(&mut i, "(set-indent-width 3)");
    assert_eq!(run(&mut i, "standard-indent-width"), "3");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, &format!("(search-forward {:?})", "bar();"));
    run(&mut i, "(beginning-of-line)");
    feed(&mut i, &ed, "TAB");
    assert_eq!(bs(&mut i), "int foo() {\n   bar();");
    assert!(run(&mut i, "(set-indent-width 0)").starts_with("ERROR"));
    assert!(run(&mut i, "(set-indent-width 17)").starts_with("ERROR"));
}

/// M73 review fix: `(interactive "n")' (`commands.rs') tries `i64' first
/// but falls back to `f64' when the minibuffer input parses as a float
/// -- typing `3.5' at the `Indent width: ' prompt used to sail through
/// the old range-only check, get `setq-local'd into
/// `standard-indent-width' as a `Value::Float', and then break EVERY
/// later indent in that buffer with `wrong-type-argument integerp'
/// (`need_int' in `elisp/builtins/mod.rs' rejects any Float, even 4.0)
/// -- confirmed live before this fix: `(set-indent-width 3.5)' reported
/// success ("Indent width set to 3"), and the next `TAB' then errored
/// instead of indenting. This test asserts the loud, immediate `error'
/// this fix adds, instead of that silent-until-the-next-TAB failure.
#[test]
fn set_indent_width_rejects_a_float() {
    let (mut i, _ed) = setup();
    run(&mut i, "(c-mode)");
    run(&mut i, &format!("(insert {:?})", "int foo() {\nbar();"));
    assert!(run(&mut i, "(set-indent-width 3.5)").starts_with("ERROR"));
    // And the pre-existing width (the mode default, untouched) must
    // still work fine afterwards -- the rejected call must not have
    // partially applied.
    assert_eq!(run(&mut i, "standard-indent-width"), "4");
}

#[test]
fn user_hook_setq_local_wins_over_detection() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        "(add-hook 'verilog-mode-hook (lambda () (setq-local standard-indent-width 9)))",
    );
    let dir = temp_dir("b18");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n  logic a;\n  logic b;\n  logic c;\n  logic d;\n  logic e;\nendmodule\n",
    );
    assert_eq!(run(&mut i, "standard-indent-width"), "9");
}
