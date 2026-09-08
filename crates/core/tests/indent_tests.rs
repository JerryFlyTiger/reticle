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
    assert_eq!(bs(&mut i), "module m;\n  wire w;\nendmodule\n");
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
        "module foo;\n  always @(*) begin\n    if (x) begin\n      old_style = 1;\n    end\n  end\nendmodule\n"
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
        "module nested_demo;\n  reg a;\n  always @(*) begin\n    if (a) begin\n      if (a) begin\n        a = 0;\n      end\n    end\n  end\nendmodule\n"
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
        "module foo;\n  always @(*) begin\n    if (x) begin\n            old_style = 1;\n    end\n  end\nendmodule\n"
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
        "4",
        "`case (x)` should align with an ordinary statement inside `initial begin`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"1: y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "case item content one level deeper than `case`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"default: y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "the `default:` case item is at the same depth as a numbered one"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endcase\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
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
        "4",
        "function body is one level deeper than the function's own header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endfunction\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
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
        "2",
        "`generate` aligns with the module body"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"for (\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "the for-generate's own line is one level inside `generate`"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire w\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "the generate_block's own content is one level inside the for-generate line"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endgenerate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
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
        "2",
        "`generate` aligns with the module body"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire plain_w\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "documented v1 gap: a plain generate-region item lands at the SAME column as \
         `generate` itself, not one level in -- see indent.el's header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (1)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "documented v1 gap: an if-generate construct sibling lands one level deeper than \
         the plain item right above it, even though both are direct children of the same \
         generate region -- see indent.el's header"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endgenerate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
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
        "2",
        "documented v1 gap: the module's own header line computes one level deep, not 0 -- \
         see indent.el's M38 header section"
    );
}

/// M97 fix round (FF2): `package'/`interface'/`class' header lines inherit
/// `module_declaration''s own documented one-level indent quirk, for the
/// exact same structural reason (the opening keyword is a descendant of
/// the very node whose BODY needs the extra level) -- see indent.el's
/// header, M97 fix round FF2 section, for the reasoning; this pins it as a
/// known, intentional v1 gap for the three new types too rather than
/// leaving it undocumented and unpinned.
#[test]
fn verilog_package_interface_and_class_header_lines_share_the_documented_one_level_indent_quirk() {
    for (open, close) in [
        ("package p;", "endpackage"),
        ("interface i;", "endinterface"),
        ("class c;", "endclass"),
    ] {
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(
            &mut i,
            &format!("(insert {:?})", format!("{}\n{}\n", open, close)),
        );
        run(&mut i, "(goto-char (point-min))");
        assert_eq!(
            run(&mut i, "(verilog-indent-line)"),
            "2",
            "documented v1 gap: `{}''s own header line computes one level deep, not 0",
            open
        );
    }
}

/// M97 fix round (FF2): the same quirk verified against the real showcase
/// file, `demo/rtl/pkg/soc_pkg.sv' -- line 7, `package soc_pkg;', is
/// on-disk at column 0; this pins that re-TABbing it computes column 2
/// instead (one level at this file's own 2-column style), NOT because
/// this milestone regressed it, but because it inherits the exact same
/// documented quirk as every other declaration header line in this
/// section.
#[test]
fn verilog_soc_pkg_sv_package_header_line_computes_one_level_deep_against_its_own_on_disk_column() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo/rtl/pkg/soc_pkg.sv");
    let src = std::fs::read_to_string(path).expect("demo/rtl/pkg/soc_pkg.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"package soc_pkg;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "documented v1 gap, not a regression: the package header line computes one level \
         deep (2, this file's own indent width) against its real on-disk column 0"
    );
}

/// M122: `program' header lines inherit the exact same documented
/// one-level indent quirk as `module'/`package'/`interface'/`class'
/// above, for the exact same structural reason -- dump-verified
/// (indent.el's own `program_declaration' comment) that `program_
/// declaration' has the IDENTICAL flat-container shape as
/// `module_declaration' (the `program' keyword is a descendant of
/// `program_ansi_header', itself a child of the very node whose body
/// needs the +1, with no separate header-vs-body split in the grammar).
#[test]
fn verilog_program_header_line_shares_the_documented_one_level_indent_quirk() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "program p;\nendprogram\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "documented v1 gap: `program p;''s own header line computes one level deep, not 0"
    );
}

/// M122: the same quirk verified against the real showcase file,
/// `demo/verif/sram_bank_tb.sv' -- line 14, `program automatic
/// axi_driver #(', is on-disk at column 0; this pins that re-TABbing it
/// computes column 2 instead, for the same reason
/// `verilog_soc_pkg_sv_package_header_line_computes_one_level_deep_
/// against_its_own_on_disk_column' does for `package'.
#[test]
fn verilog_sram_bank_tb_sv_program_header_line_computes_one_level_deep_against_its_own_on_disk_column(
) {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/verif/sram_bank_tb.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/verif/sram_bank_tb.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        "(search-forward \"program automatic axi_driver #(\")",
    );
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "documented v1 gap, not a regression: the program header line computes one level \
         deep (2, this file's own indent width) against its real on-disk column 0"
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

// --- M90: a SEPARATE wrap-step axis for wrapped port/parameter/argument ---
// lists (`indent-wrap-width'/`indent--wrap-node-types' in indent.el) --
// `verible-verilog-format' at its default flags (the tool that produced
// `demo/rtl''s committed formatting byte for byte) indents these
// continuation lines by a FIXED 4 columns regardless of the file's own
// block-indent width -- measured at `--indentation_spaces' 2/3/4/8, the
// wrap delta stayed 4 every time. Node names below (`list_of_port_
// connections'/`list_of_parameter_value_assignments'/`list_of_arguments')
// were confirmed against a real parse (M90, a throwaway `(treesit-node-
// string root)' dump of this same shape of snippet -- deleted after use,
// per the M33/M34/M38 convention), not just read off `grammar.js'.

#[test]
fn verilog_wrapped_port_list_continuation_lands_at_wrap_step_not_block_step() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  sub_module u_sub (\n    .clk(clk),\n    .rst(rst)\n  );\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \".clk\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "module body depth (1 * standard-indent-width 2) + one indent-wrap-width step (4) = \
         6, matching verible's --wrap_spaces default, not the module's own 2-column block width"
    );
}

#[test]
fn verilog_wrapped_port_list_continuation_wrap_step_is_decoupled_from_block_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 8)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  sub_module u_sub (\n    .clk(clk),\n    .rst(rst)\n  );\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \".clk\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "12",
        "same instantiation, standard-indent-width now 8: 1 * 8 + 4 = 12, NOT 1 * 8 + 8 = 16 -- \
         indent-wrap-width stays a fixed 4 even when the block width changes; this is the test \
         that would fail if the wrap step were ever folded back into the block multiply"
    );
}

#[test]
fn verilog_wrapped_parameter_value_assignment_list_continuation_gets_a_wrap_step() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  sub_module #(\n    .W(8),\n    .D(4)\n  ) u_sub (\n    .clk(clk)\n  );\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \".W(8)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "a `#(...)' parameter list continuation gets the same wrap step as a port list"
    );
}

#[test]
fn verilog_wrapped_call_argument_list_continuation_gets_a_wrap_step() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  initial begin\n    do_call(\n      a,\n      b\n    );\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"a,\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "ordinary call argument continuation: module body (1) + seq_block (1) = 2 block \
         levels at width 2 (= 4), plus one wrap step (4) = 8"
    );
}

#[test]
fn verilog_wrapped_port_list_closing_paren_line_stays_at_base_column() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  sub_module u_sub (\n    .clk(clk),\n    .rst(rst)\n  );\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "the closing `);' line is a SIBLING of list_of_port_connections, not a descendant, so \
         it gets no wrap step and stays at the instantiation's own base column"
    );
}

#[test]
fn verilog_wrapped_port_list_second_continuation_line_does_not_stack_wrap_steps() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  sub_module u_sub (\n    .clk(clk),\n    .rst(rst)\n  );\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \".rst\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "the second continuation line in a three-line list lands at the SAME column as the \
         first -- the wrap step applies once (per ancestor list node), not once per line"
    );
}

/// M119 D1: real `verible-verilog-format --indentation_spaces=2', run on
/// this exact fixture, puts `parameter W = 8'/`input logic clk'/`input
/// logic rst' at column 4 and `wire w;' at column 2 -- confirmed against
/// real material too: `demo/rtl/core/alu.sv' lines 7-24 sit at column 4
/// on disk and `verible-verilog-format --verify demo/rtl/core/alu.sv'
/// exits 0. This test previously asserted column 2 for the two
/// continuation lines, read off this fixture's OWN hand-typed
/// indentation rather than off verible -- the exact trap M110 (see
/// CLAUDE.md) warns about; M119 fixes the expectation and renames the
/// test (the old name, `..._unaffected_by_wrap_step', asserted the
/// opposite of what real verible does).
#[test]
fn verilog_ansi_header_wrapped_port_and_parameter_lists_get_a_wrap_step_to_four() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top #(\n  parameter W = 8\n) (\n  input logic clk,\n  input logic rst\n);\n  wire w;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"parameter W\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: `verible-verilog-format --indentation_spaces=2' puts this line at column 4 \
         -- `parameter_port_list' is now in `indent--wrap-node-types', getting the same fixed \
         wrap step (4) every other wrapped list gets, with `indent--verilog-header-wrap-depth-\
         adjust' cancelling `module_declaration''s own block-depth contribution so the two \
         axes combine to 0 + 4 = 4, not 1*2 + 4 = 6"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"input logic clk\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: same column 4 for the ANSI port list continuation -- `list_of_port_\
         declarations' gets the identical treatment as `parameter_port_list' above"
    );
    // FIX-4 (M119 fix round): the closing-paren lines of the wrapped
    // header, not just its continuation CONTENT lines. Real
    // `verible-verilog-format --indentation_spaces=2' puts BOTH `) ('
    // (closing `parameter_port_list', opening `list_of_port_declarations')
    // and the final `);' at column 0, the header's own on-disk column --
    // measured against this exact fixture (`demo/rtl/core/alu.sv' lines 11
    // and 25 are the same shape). Before this fix round, only the
    // continuation-content assertions above existed, so reverting any of
    // the four `indent--verilog-closers' contextual closer entries
    // (`")" . "parameter_port_list"'/`")" . "list_of_port_declarations"'/
    // `"#" . "parameter_port_list"'/`"(" . "list_of_port_declarations"')
    // left this whole test green -- only the full-file sweep
    // (`verilog_every_demo_rtl_file_reindents_to_its_own_on_disk_columns_
    // except_named_divergences') caught it, because it, not this test,
    // happens to walk a real closing-paren line.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \") (\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: the `) (' line between the wrapped parameter list and the wrapped port list \
         stays at the header's own column 0, not the wrap step's column 4 -- this is the direct \
         guard for the `\")\" . \"parameter_port_list\"' and `\"(\" . \"list_of_port_declarations\"' \
         contextual closer entries"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: the final `);' closing the wrapped port list also stays at column 0 -- the \
         direct guard for the `\")\" . \"list_of_port_declarations\"' contextual closer entry"
    );
}

/// M119 D1 (interface variant): real `verible-verilog-format
/// --indentation_spaces=2', run on this exact fixture, puts `parameter W
/// = 8'/`input logic clk' at column 4 and leaves `endinterface' at column
/// 0 -- dump-verified (M119) that `interface_ansi_header' uses the exact
/// same two node kind NAMES (`parameter_port_list'/`list_of_port_
/// declarations') as `module_ansi_header', so one shared table entry
/// covers both.
#[test]
fn verilog_interface_ansi_header_wrapped_parameter_and_port_lists_get_a_wrap_step_to_four() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "interface top_if #(\n  parameter W = 8\n) (\n  input logic clk\n);\nendinterface\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"parameter W\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: `verible-verilog-format --indentation_spaces=2' puts this line at column 4, \
         same as the module case"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"input logic clk\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: same for the interface's ANSI port list continuation"
    );
    // FIX-4 (M119 fix round): closing-paren lines for the interface
    // variant too -- see the module-variant test above for the full
    // rationale.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \") (\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: the `) (' line stays at the header's own column 0, not the wrap step's \
         column 4"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: the final `);' closing the wrapped port list also stays at column 0"
    );
}

/// M119 D2: real `verible-verilog-format --indentation_spaces=2', run on
/// this exact fixture, puts the unwrapped (module-scope, no `generate'/
/// `endgenerate') `if' line at column 2 and its body at column 4 --
/// confirmed against real material too: `demo/rtl/core/alu.sv' is fully
/// verible-canonical (`verible-verilog-format --indentation_spaces=2
/// demo/rtl/core/alu.sv' diffs empty against the file on disk), and its
/// lines 57-75 use exactly this shape (`if (Pipelined) begin :
/// gen_pipelined' at column 2). Before this fix, `if_generate_construct'
/// was unconditionally counted, landing this shape one level too deep
/// throughout (`if' at column 4, body at column 6) -- the same column
/// the WRAPPED form (`generate'/`endgenerate' around the same body,
/// pinned correct already by
/// `verilog_generate_for_loop_body_indents_and_endgenerate_aligns_with_generate')
/// legitimately gets.
#[test]
fn verilog_unwrapped_module_scope_if_generate_is_one_level_shallower_than_the_wrapped_form() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  if (P) begin : g_yes\n    wire x;\n  end else begin : g_no\n    wire y;\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (P)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: real verible puts an unwrapped generate-if's own header line at column 2, \
         module-depth alone -- not column 4, which is what the WRAPPED form (inside `generate'/\
         `endgenerate') legitimately gets"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire x\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the body of an unwrapped generate-if sits at column 4, one level less than \
         the wrapped form's column 6"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"end else\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: `end else begin : g_no' dedents back to the `if' line's own column, same as \
         any other `end'"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the else-branch body is the same shape as the if-branch body"
    );
}

/// M119 D3: real `verible-verilog-format --indentation_spaces=2', run on
/// this exact fixture, puts a class CONSTRUCTOR's body at column 4 and
/// its own `endfunction' at column 2 -- the SAME shape an ordinary
/// method in the same class already gets. Before this fix,
/// `class_constructor_declaration' (a node kind DISTINCT from
/// `function_body_declaration', dump-verified) was absent from
/// `indent--block-node-types', so only the constructor under-indented
/// (body at column 2, `endfunction' at column 0) while an ordinary
/// method right below it in the same class was already correct.
#[test]
fn verilog_class_constructor_body_indents_the_same_as_an_ordinary_method_in_the_same_class() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "class c;\n  int x;\n  function new();\n    x = 0;\n  endfunction\n  function void bump();\n    x = x + 1;\n  endfunction\nendclass\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"x = 0\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: real verible puts the constructor body at column 4"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endfunction\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: the constructor's own `endfunction' dedents to column 2, matching the \
         module-level `endfunction' shape, not column 0"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"x = x + 1\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "control: the ordinary method `bump()' right below the constructor was already correct \
         before this fix, and must stay that way"
    );
}

/// M119 D4: real `verible-verilog-format --indentation_spaces=2' puts
/// the `coverpoint ... {' header line at column 4, each `bins ...;'
/// line at column 6, and the closing `}' back at column 4 --
/// `endgroup' dedents to column 2. Before this fix, neither
/// `covergroup_declaration' nor `bins_or_empty' appeared in
/// `indent--block-node-types' at all, so a bin line computed column 4
/// (module/class-depth-driven fallback), two columns shallower than
/// real verible.
///
/// FIX-5 correction (M119 fix round): the embedded fixture below is
/// NOT actually wide enough to force a wrap -- real verible collapses
/// its own two `bins' lines onto ONE line (measured: `verible-verilog-
/// format --indentation_spaces=2' on this exact fixture text produces
/// `bins lo = {0}; bins hi = {1};' as a single physical line, not two).
/// The pinned columns below are correct regardless, confirmed instead
/// against a fixture verible really does keep split. Note the bin name
/// has to be long enough to pass verible's default 100-column limit at
/// this indentation -- a merely "long-looking" name is not enough, and
/// the first version of THIS correction quoted one that is not (95
/// columns, which verible still collapses). Measured, `verible-verilog-
/// format --indentation_spaces=2':
///
///     class c;
///       covergroup cg @(posedge clk);
///         coverpoint sig {
///           bins lo_bin_with_a_very_long_descriptive_name_that_exceeds_verible_default_column_limit = {0};
///           bins hi = {1};
///         }
///       endgroup
///     endclass
///
/// -- columns 4/6/6/4/2, identical to what this test pins.
/// This engine's own indentation logic does not depend on verible's
/// own line-splitting decision either way (it always treats each
/// source line independently), so the embedded fixture still exercises
/// the same code path even though it does not, on its own, reproduce a
/// verible-forced wrap the way the original claim asserted.
#[test]
fn verilog_covergroup_coverpoint_wrapped_bins_indent_one_level_deeper_than_coverpoint_header() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "class c;\n  covergroup cg @(posedge clk);\n    coverpoint some_extremely_long_signal_name_used_to_force_wrap_of_bins_list_here {\n      bins lo = {0};\n      bins hi = {1};\n    }\n  endgroup\nendclass\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"coverpoint \")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: real verible puts the `coverpoint ... {{' header line at column 4"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bins lo\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "measured: a wrapped bin line sits one level deeper than the `coverpoint' header, \
         column 6"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"bins hi\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "measured: the second bin line lands at the same column as the first"
    );
    run(&mut i, "(goto-char (point-min))");
    // The needle is the bin list's own CLOSING `}' -- not the first `}'
    // in the buffer, which belongs to `bins lo = {0};''s value set, so
    // search for the LAST bin line and step one line further instead.
    run(&mut i, "(search-forward \"bins hi\")");
    run(&mut i, "(forward-line 1)");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the bin list's own closing `}}' dedents back to the `coverpoint' header's \
         column, via the CONTEXTUAL `(\"}}\" . \"bins_or_empty\")' closer -- a blanket `}}' \
         closer would be wrong (see the enum/struct closing-brace test above)"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endgroup\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: `endgroup' dedents to column 2, matching `covergroup''s own column"
    );
}

/// M90 fix-round pin, NOT a target: real `verible-verilog-format' does not
/// always use the fixed wrap step -- when a wrapped call's own argument is
/// itself a call whose opening paren is followed by MORE TEXT on the same
/// line (not a hanging list), verible switches to paren-column alignment
/// for that continuation line (measured: column 29, aligned under
/// `other_call_with_a_long_name('s own opening paren). This engine keeps
/// computing the fixed-step answer instead -- block depth 1 * 2 (M104:
/// verilog-mode's own standard-indent-width default, 2, not 4) + wrap
/// depth 2 * 4 (indent-wrap-width, its own fixed constant) = 10 --
/// because reproducing verible's conditional switch needs line-length
/// lookahead this engine has no mechanism for (see indent.el's M90 header
/// comment for the full reasoning, and why this is a documented scope
/// decision rather than a bug). This test pins what this engine ACTUALLY
/// computes, 10, not verible's 29 -- if this test ever needs to change to
/// 29, that is a real feature (line-length-aware paren alignment), not a
/// refactor, and belongs in its own milestone.
#[test]
fn verilog_nested_call_inside_wrapped_call_diverges_from_verible_paren_alignment_a_documented_scope_decision(
) {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top;\n  initial do_call_with_a_long_name(other_call_with_a_long_name(\n    arg1, arg2, arg3\n  ));\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"arg1\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "10",
        "block depth 1 (module body, standard-indent-width default 2, M104) + wrap depth 2 (two \
         nested list_of_arguments ancestors, indent-wrap-width 4 each -- indent-wrap-width is its \
         own fixed constant, independent of standard-indent-width) = 2 + 8 = 10 -- this is what \
         the fixed-step model computes, NOT real verible's paren-aligned 29, and that gap is a \
         documented, deliberate scope decision (see indent.el's M90 header), not a bug"
    );
}

/// M97 Part 1: `package'/`endpackage' body, using `demo/rtl/pkg/soc_pkg.sv''s
/// real shape (a `parameter int unsigned ...;' line one level inside
/// `package soc_pkg;'). Reproduced BEFORE the fix landed a `package_declaration'
/// entry in `indent--block-node-types': with that entry absent, a body line
/// gets zero block-depth increment and computes column 0, not 2 -- see
/// indent.el's M97 header for the SUSPECTED-entry reconnaissance this
/// confirmed.
#[test]
fn verilog_package_body_indents_one_level_and_endpackage_dedents() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package soc_pkg;\nparameter int unsigned AddrWidth = 32;\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"parameter\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "package body is one level deeper than the package header, standard-indent-width default 2 (M104) \
         (the on-disk file's own 2-column formatting is a separate, `indent-detect-width' concern -- \
         not exercised here, since `verilog-mode' is turned on before any content is inserted)"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endpackage\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "`endpackage` dedents back to the package header's own depth"
    );
}

/// M97 Part 1: `interface'/`endinterface' body -- `interface_declaration' was
/// as absent from `indent--block-node-types' as `package_declaration', and
/// `endinterface' as absent from `indent--verilog-closers' as `endpackage'.
#[test]
fn verilog_interface_body_indents_one_level_and_endinterface_dedents() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "interface axi_if;\nlogic valid;\nendinterface\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "interface body is one level deeper than the interface header, standard-indent-width 2 (M104)"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endinterface\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "`endinterface` dedents back to the interface header's own depth"
    );
}

/// M97 Part 1: `class'/`endclass' body, the third of the three
/// SUSPECTED-entry-adjacent block types.
#[test]
fn verilog_class_body_indents_one_level_and_endclass_dedents() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(insert {:?})", "class foo;\nint x;\nendclass\n"),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"int\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "class body is one level deeper than the class header, standard-indent-width 2 (M104)"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"endclass\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "`endclass` dedents back to the class header's own depth"
    );
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

// NOTE (M73 review; M104 update below): these two tests' `standard-
// indent-width' assertion alone WAS a false test at the time this note
// was written -- verilog-mode's OWN default was also 4 back then, so
// the assertion passed identically whether `indent--maybe-detect-width'
// ran or was never wired up at all (confirmed by physically commenting
// out the `(indent--maybe-detect-width)' call in modes.el: both tests
// stayed green). Each was given a second, direct assertion on what
// `(indent--detect-width)' itself returned on the same buffer, so the
// test could tell "detected 4" apart from "fell back to the mode
// default 4" regardless of what the mode default happened to be.
//
// M104 dropped verilog-mode's own default from 4 to 2 (to match
// `verible-verilog-format''s own default -- see modes.el). That
// actually UNDOES the vacuousness for the test right below
// (`find_file_four_space_sv_sets_width_to_four'): its file is genuinely
// 4-space-indented, so detection finds 4, which is now DIFFERENT from
// the mode default (2) -- the plain `standard-indent-width' assertion
// alone would now correctly fail if detection silently stopped running
// (it would read 2, the fallen-back-to default, not 4). It stays a
// false-test risk in general, though, since a future default change
// could put it right back at 4 -- which is exactly why the direct
// `(indent--detect-width)' assertion on each test remains the actual,
// default-independent line of defense, not the `standard-indent-width'
// one; nothing here should be read as license to remove either
// assertion. `find_file_tab_indented_sv_keeps_mode_default' (the next
// test after this one) still needs its own direct assertion for the
// same reason -- see its own M104 comment.
//
// These two pin down "detection result == mode default" / "detection
// fell back to the mode default" as states, NOT the find-file
// integration point -- that point is covered by the four OTHER
// end-to-end tests (`find_file_two_space_sv_opens_new_lines_at_
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
    // M104: verilog-mode's own default dropped from 4 to 2 (matching
    // verible-verilog-format's default), so the mode-default fallback
    // this test checks is now 2, not 4.
    assert_eq!(run(&mut i, "standard-indent-width"), "2");
    assert_eq!(
        run(&mut i, "(indent--detect-width)"),
        "nil",
        "detection itself must have declined (nil) here, not \"detected\" 4"
    );
}

// M104 fix round (cold-review defect): this test's whole point is
// proving that `(setq indent-detect-width nil)' really disables
// detection. That requires a fixture whose DETECTED width differs from
// verilog-mode's own MODE DEFAULT -- otherwise "detection is off, mode
// default (2) stands" and "detection is on, and happens to detect 2
// anyway" produce the exact same `standard-indent-width' reading, and
// disabling the flag becomes unobservable. Before M104 the mode
// default was 4, so this test's old 2-space fixture already had that
// property by coincidence; M104 dropped the default to 2 (to match
// `verible-verilog-format''s own default), which collided with this
// fixture's own 2-space content and silently voided the test -- the
// comment that used to sit here noticed the coincidence but concluded
// "detection is off here anyway so it doesn't matter," which is
// backwards: that collision is exactly the one thing this test needs
// to rule out. Fixed by switching the fixture to 4-space indentation
// (with `indent--detect-min-samples' == 5 indented lines, so detection
// would confidently find 4 if it ran) -- now genuinely different from
// the mode default (2), and paired with a second assertion on the same
// file that detection, when left ON, actually finds 4 on it.
#[test]
fn find_file_four_space_sv_with_detection_disabled_keeps_mode_default_of_two() {
    let (mut i, _ed) = setup();
    run(&mut i, "(setq indent-detect-width nil)");
    let dir = temp_dir("b13");
    write_and_open(
        &mut i,
        &dir,
        "m.sv",
        "module m;\n    logic a;\n    logic b;\n    logic c;\n    logic d;\n    logic e;\nendmodule\n",
    );
    assert_eq!(
        run(&mut i, "standard-indent-width"),
        "2",
        "with detection disabled, verilog-mode's own default (2) must stand, \
         NOT the 4 this file's own content would otherwise detect"
    );

    // Same content, in a fresh interp/buffer with detection left ON
    // this time: it must actually find 4 -- the fact that the FIRST
    // assertion above reads 2 depends entirely on this second one being
    // true; otherwise 2 could just as easily mean "detection ran and
    // detected 2 anyway."
    let (mut i2, _ed2) = setup();
    let dir2 = temp_dir("b13-control");
    write_and_open(
        &mut i2,
        &dir2,
        "m.sv",
        "module m;\n    logic a;\n    logic b;\n    logic c;\n    logic d;\n    logic e;\nendmodule\n",
    );
    assert_eq!(
        run(&mut i2, "(indent--detect-width)"),
        "4",
        "detection left on must actually find 4 on this same 4-space content"
    );
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

// --- M100: enum member / struct field lines were dedenting by one level ---
// (see indent.el's `indent--block-node-types' comment and its M100 note
// alongside the M97 discussion for the fix and the node shapes it relies
// on). `enum_name_declaration'/`struct_union_member' are each SELF-
// REFERENTIAL nodes -- exactly like `case_item', already accepted above --
// so adding them adds exactly one level to the one line each starts on,
// without touching ordinary (non-enum/struct) declarations, whose leading
// token is also a `data_type' descendant but NOT under either of these two
// new node types.

/// M100: a package-level enum member line is now package-depth (1) plus
/// its own `enum_name_declaration' (1) = 2 levels, matching the real
/// showcase file's on-disk formatting at its own 2-column width.
#[test]
fn verilog_package_enum_member_line_indents_to_four_at_two_column_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package p;\ntypedef enum logic [3:0] {\nAluAdd = 4'h0,\nAluSub = 4'h1\n} e;\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"AluAdd\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "M100: enum member line is package-depth (1) plus its own \
         enum_name_declaration (1) = 2 levels at this file's 2-column width"
    );
}

/// M100: same shape, a packed struct's field line.
#[test]
fn verilog_package_struct_field_line_indents_to_four_at_two_column_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package p;\ntypedef struct packed {\nlogic a;\nlogic b;\n} req_t;\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic a;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "M100: struct field line is package-depth (1) plus its own \
         struct_union_member (1) = 2 levels at this file's 2-column width"
    );
}

/// M100: the enum/struct closing `}' line is NOT itself an
/// `enum_name_declaration'/`struct_union_member' (those are its
/// SIBLINGS under `data_type', not its ancestors), so it must stay
/// unaffected by this milestone's addition -- still one level (the
/// enclosing `package_declaration' alone), matching real-file column 2.
#[test]
fn verilog_enum_and_struct_closing_brace_lines_unaffected_still_two() {
    for body in [
        "package p;\ntypedef enum logic [3:0] {\nAluAdd = 4'h0,\nAluSub = 4'h1\n} e;\nendpackage\n",
        "package p;\ntypedef struct packed {\nlogic a;\nlogic b;\n} req_t;\nendpackage\n",
    ] {
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, "(set-indent-width 2)");
        run(&mut i, &format!("(insert {:?})", body));
        run(&mut i, "(goto-char (point-min))");
        run(&mut i, "(search-forward \"}\")");
        run(&mut i, "(beginning-of-line)");
        assert_eq!(
            run(&mut i, "(verilog-indent-line)"),
            "2",
            "closing `}}' line of {:?} must stay at one level, unaffected by M100",
            body
        );
    }
}

/// M100: the `typedef enum ... {'/`typedef struct packed {' OPENING line
/// itself is also not one of the two new node types (it is the sibling
/// `data_type' node's own start, not an item under it) -- still one
/// level, matching real-file column 2.
#[test]
fn verilog_enum_and_struct_opening_line_unaffected_still_two() {
    for (needle, body) in [
        (
            "typedef enum",
            "package p;\ntypedef enum logic [3:0] {\nAluAdd = 4'h0,\nAluSub = 4'h1\n} e;\nendpackage\n",
        ),
        (
            "typedef struct",
            "package p;\ntypedef struct packed {\nlogic a;\nlogic b;\n} req_t;\nendpackage\n",
        ),
    ] {
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, "(set-indent-width 2)");
        run(&mut i, &format!("(insert {:?})", body));
        run(&mut i, "(goto-char (point-min))");
        run(&mut i, &format!("(search-forward {:?})", needle));
        run(&mut i, "(beginning-of-line)");
        assert_eq!(
            run(&mut i, "(verilog-indent-line)"),
            "2",
            "`{}' opening line must stay at one level, unaffected by M100",
            needle
        );
    }
}

/// M100 regression guard: this is what the DO-NOT-add-`data_type' comment
/// in indent.el warns about. Ordinary (non-enum/non-struct) declarations
/// share the same `data_type' ancestor as enum members/struct fields, but
/// must NOT gain an extra level from this milestone -- neither a plain
/// signal declaration inside a package function body (package_declaration
/// + function_body_declaration = 2 levels, unchanged) nor an ordinary
///   module-level signal declaration (module_declaration alone = 1 level,
///   unchanged).
#[test]
fn verilog_ordinary_declarations_unaffected_by_the_new_enum_struct_node_types() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package p;\nfunction automatic void f();\nlogic [7:0] mask;\nendfunction\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic [7:0] mask;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "an ordinary declaration inside a package function body stays at 2 levels (4 \
         columns), not 3, even though its leading token is also a `data_type' descendant"
    );

    let (mut i2, _ed2) = setup();
    run(&mut i2, "(verilog-mode)");
    run(&mut i2, "(set-indent-width 2)");
    run(
        &mut i2,
        &format!("(insert {:?})", "module m;\nlogic [3:0] foo;\nendmodule\n"),
    );
    run(&mut i2, "(goto-char (point-min))");
    run(&mut i2, "(search-forward \"logic [3:0] foo;\")");
    run(&mut i2, "(beginning-of-line)");
    assert_eq!(
        run(&mut i2, "(verilog-indent-line)"),
        "2",
        "an ordinary module-level signal declaration stays at 1 level (2 columns), \
         unaffected by the enum/struct-only node types added at M100"
    );
}

/// M100's strongest guard: every non-blank line of the real showcase file,
/// `demo/rtl/pkg/soc_pkg.sv', reindented at this file's own 2-column
/// width, must compute the SAME column it already has on disk -- except
/// line 7, `package soc_pkg;', which keeps the separate, already-pinned
/// M97 header-line quirk (computes 2 against on-disk 0; see
/// `verilog_soc_pkg_sv_package_header_line_computes_one_level_deep_
/// against_its_own_on_disk_column', above). Before M100, this failed on
/// 16 lines -- the enum's 10 member lines and the two structs' 6 field
/// lines (each of which had been computing 2 against on-disk 4).
#[test]
fn verilog_soc_pkg_sv_full_file_reindents_to_its_own_on_disk_columns_except_the_documented_package_header_quirk(
) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo/rtl/pkg/soc_pkg.sv");
    let src = std::fs::read_to_string(path).expect("demo/rtl/pkg/soc_pkg.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");

    let mut mismatches: Vec<(usize, usize, String, String)> = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if !trimmed.is_empty() && line != "package soc_pkg;" {
            let expected = line.len() - trimmed.len();
            run(&mut i, "(beginning-of-line)");
            let actual_s = run(&mut i, "(verilog-indent-line)");
            let actual: usize = actual_s.parse().unwrap_or_else(|_| {
                panic!(
                    "line {}: (verilog-indent-line) returned non-numeric {:?} for {:?}",
                    lineno, actual_s, line
                )
            });
            if actual != expected {
                mismatches.push((lineno, expected, actual_s.clone(), line.to_string()));
            }
        }
        run(&mut i, "(forward-line 1)");
    }

    assert!(
        mismatches.is_empty(),
        "soc_pkg.sv lines that don't reindent to their own on-disk column \
         (line, expected, actual, text): {:#?}\n\
         NOTE: this test hardcodes the string \"package soc_pkg;\" to skip the \
         one known, separately-pinned header-line quirk (see \
         verilog_soc_pkg_sv_package_header_line_computes_one_level_deep_against_\
         its_own_on_disk_column, above). If the mismatch above is on the package \
         header line and the file's package was renamed, this comparison and \
         that string constant are both stale, not a real regression -- update \
         both to the new package name.",
        mismatches
    );
}

/// M119: recursively collect every `.sv'/`.svh' file under `demo/rtl'
/// (M122: and, via the second call site below, `demo/verif'). Real
/// filesystem discovery, not a hand-maintained list -- CLAUDE.md's
/// M114 note: "a mechanism that decides what gets run must fail loudly
/// when it decides nothing does." Panics (rather than returning an empty
/// Vec silently) if DIR itself doesn't exist, so a moved/renamed
/// `demo/rtl' can't quietly turn this into a 0-file no-op.
fn m119_collect_sv_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!(
            "demo/rtl must exist and be readable: {}: {}",
            dir.display(),
            e
        )
    });
    for entry in entries {
        let entry = entry.expect("readable dir entry");
        let path = entry.path();
        if path.is_dir() {
            m119_collect_sv_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "sv" || e == "svh") {
            out.push(path);
        }
    }
}

/// M119: the structural fix `CLAUDE.md's M119 spec calls for -- through
/// M118, the only full-file reindent check ran against
/// `demo/rtl/pkg/soc_pkg.sv', the ONE file in the showcase with no port
/// list, so D1 (the biggest of the four divergences: a wrapped ANSI
/// port/parameter list two columns too shallow) was invisible to it.
/// This check instead walks EVERY `.sv'/`.svh' file under `demo/rtl'
/// (real filesystem discovery, `m119_collect_sv_files' -- not a
/// hand-maintained list, so a new showcase file is covered automatically)
/// and compares the editor's computed indentation against each file's
/// own on-disk column, which is verible ground truth: `demo/tools/
/// lint_rtl.sh' enforces `verible-verilog-format' cleanliness over
/// `demo/rtl' with NO waivers at all (unlike `demo/rtl-verilog2001',
/// which has a `.rules.verible_lint' -- see CLAUDE.md's `demo/` section).
///
/// Fails loudly (asserts a minimum file count, panics rather than
/// silently checking zero files) per CLAUDE.md's M114 note.
///
/// Known, NAMED divergences are excluded by exact (file, line) pair
/// below, each with the reason -- not by a blanket tolerance, per this
/// milestone's own spec:
///   - the module/package HEADER LINE one-level quirk (already pinned by
///     `verilog_module_header_line_has_a_documented_one_level_indent_
///     quirk' and `verilog_package_interface_and_class_header_lines_
///     share_the_documented_one_level_indent_quirk' -- NOT this
///     milestone's D1, which is about the wrapped list's CONTINUATION
///     lines, not the header line the list starts on);
///   - `demo/rtl/top/soc_top.sv' lines 106-113, hand-aligned commented-out
///     code inside a `/* axi4_lite_arbiter AUTO_TEMPLATE (...) */' block
///     comment -- tree-sitter parses a block comment as ONE leaf node
///     spanning every one of its lines, so the block-depth engine has no
///     way to see structure inside it at all (same gap as a C block
///     comment's internal alignment never being smart-indented);
///   - `demo/rtl/include/soc_defs.svh' lines 22-23, inside a
///     backslash-continued `` `define `` macro body -- preprocessor text,
///     not modeled by the verilog grammar's ordinary statement/expression
///     productions this engine's block-depth walk relies on.
///
/// Anything else is a real mismatch this test must catch.
#[test]
fn verilog_every_demo_rtl_file_reindents_to_its_own_on_disk_columns_except_named_divergences() {
    let demo_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo"));
    let demo_rtl = demo_root.join("rtl");
    let demo_verif = demo_root.join("verif");
    let mut files = Vec::new();
    m119_collect_sv_files(&demo_rtl, &mut files);
    // M122: demo/verif/ is the second real-file source this sweep walks
    // -- axi4_lite_monitor.sv, soc_verif_pkg.sv and sram_bank_tb.sv all
    // need the same on-disk-column check demo/rtl/'s files get, and a
    // sweep that only covers demo/rtl/ would silently never catch a
    // divergence in any of the three.
    m119_collect_sv_files(&demo_verif, &mut files);
    files.sort();
    assert!(
        files.len() >= 13,
        "found only {} .sv/.svh file(s) under demo/rtl and demo/verif -- expected at least 13 \
         (there were 13 at M122 authoring time: 10 under demo/rtl, 3 under demo/verif); a \
         broken discovery mechanism that silently found 0 files would report success here, so \
         this floor turns that into a hard failure instead (CLAUDE.md, M114)",
        files.len()
    );

    // (file path suffix, 1-based line number, reason) -- see this test's
    // own doc comment above for the full explanation of each.
    // M122: paths are relative to `demo/` itself (not `demo/rtl/`
    // alone, as before) now that this sweep also walks `demo/verif/`.
    let known_divergences: &[(&str, usize, &str)] = &[
        (
            "rtl/bus/axi4_lite_arbiter.sv",
            7,
            "module header-line quirk",
        ),
        ("rtl/core/alu.sv", 6, "module header-line quirk"),
        ("rtl/core/regfile.sv", 7, "module header-line quirk"),
        ("rtl/mem/sram_wrapper.sv", 7, "module header-line quirk"),
        ("rtl/pkg/soc_pkg.sv", 7, "package header-line quirk"),
        (
            "rtl/top/soc_top.sv",
            38,
            "module header-line quirk (multi-line header, `import' clause)",
        ),
        (
            "rtl/top/soc_top.sv",
            106,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            107,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            108,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            109,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            110,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            111,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            112,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/top/soc_top.sv",
            113,
            "hand-aligned commented-out code inside a block comment",
        ),
        (
            "rtl/include/soc_defs.svh",
            22,
            "inside a backslash-continued `define macro body",
        ),
        (
            "rtl/include/soc_defs.svh",
            23,
            "inside a backslash-continued `define macro body",
        ),
        // M122: `interface_declaration'/`class_constructor_declaration'/
        // `program_declaration'/`covergroup_declaration' all inherit the
        // exact same documented header-line quirk `module_declaration'/
        // `interface_declaration'/`package_declaration'/`class_declaration'
        // already have (this file's own module doc, and indent.el's
        // header) -- each one's own opening keyword is a descendant of
        // the very node whose BODY needs the extra level, so re-TABbing
        // its single-line header computes one level deeper than ideal.
        // `program_declaration' additionally dump-verified (M122, see
        // indent.el's own comment on the `program_declaration' block-type
        // entry) to have the IDENTICAL flat-container shape as
        // `module_declaration', so it shares the quirk for the same
        // structural reason, not by assumption.
        ("rtl/bus/axi4_lite_if.sv", 9, "interface header-line quirk"),
        ("rtl/core/clk_gate.sv", 11, "module header-line quirk"),
        ("rtl/mem/sram_bank.sv", 20, "module header-line quirk"),
        ("verif/axi4_lite_monitor.sv", 19, "module header-line quirk"),
        ("verif/soc_verif_pkg.sv", 12, "package header-line quirk"),
        ("verif/sram_bank_tb.sv", 14, "program header-line quirk"),
        ("verif/sram_bank_tb.sv", 183, "module header-line quirk"),
        // M122: a SECOND, larger-magnitude instance of the SAME quirk
        // above, visible for the first time because no demo file before
        // M122 nested one self-referential header-line-quirk block
        // directly inside another (a class inside a package, a class
        // constructor inside that class, a covergroup inside a module).
        // Dump/engine-verified (M122): real `verible-verilog-format
        // --indentation_spaces=2' puts a NESTED block's own header line
        // at its PARENT's real (already-quirk-adjusted) body depth --
        // e.g. `class rw_checker;' at column 2, matching `soc_verif_pkg'
        // 's own body depth, NOT at column 4 (which is what the class's
        // OWN body -- `protected int unsigned hits_q;' etc, confirmed
        // correct on disk -- actually sits at). The editor instead
        // double-counts: `package_declaration' contributes one level,
        // then `class_declaration' ALSO contributes its own self-
        // referential level to its own header line (the same mechanism
        // that produces the single-level quirk above), so the header
        // ends up AT the class's own body depth instead of one level
        // shallower. This is the identical root cause as the top-level
        // quirk above (a block-type node's own opening keyword is a
        // descendant of itself, so it can't be distinguished from the
        // node's body for depth-counting purposes) -- nesting just makes
        // the SAME single always-off-by-one-level defect compound,
        // rather than being a second, independent bug. Fixing it
        // generally would mean reworking how EVERY already-accepted
        // top-level header-line quirk is computed (each of the seven
        // entries just above pins the CURRENT, quirky value as
        // intentional v1 behaviour), which is a broader redesign than
        // this milestone's scope of "wire the missing program/modport/
        // property constructs into the indent engine" -- recorded here,
        // not chased, the same policy CLAUDE.md documents for the
        // mixed-generate-region gap in this file's own header.
        (
            "verif/soc_verif_pkg.sv",
            18,
            "class header-line quirk, compounded by nesting inside a package (see comment above)",
        ),
        (
            "verif/soc_verif_pkg.sv",
            23,
            "class constructor header-line quirk, compounded by nesting inside a class inside a \
             package (see comment above)",
        ),
        (
            "verif/sram_bank_tb.sv",
            265,
            "covergroup header-line quirk, compounded by nesting inside a module (see comment \
             above)",
        ),
    ];

    let mut mismatches: Vec<(String, usize, usize, String, String)> = Vec::new();
    for path in &files {
        let src =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e));
        let suffix = path
            .strip_prefix(demo_root)
            .expect("path is under demo_root")
            .to_string_lossy()
            .into_owned();
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, "(set-indent-width 2)");
        run(&mut i, &format!("(insert {:?})", src));
        run(&mut i, "(goto-char (point-min))");

        for (idx, line) in src.lines().enumerate() {
            let lineno = idx + 1;
            let trimmed = line.trim_start();
            let excluded = known_divergences
                .iter()
                .any(|(f, l, _)| *f == suffix && *l == lineno);
            if !trimmed.is_empty() && !excluded {
                let expected = line.len() - trimmed.len();
                run(&mut i, "(beginning-of-line)");
                let actual_s = run(&mut i, "(verilog-indent-line)");
                let actual: usize = actual_s.parse().unwrap_or_else(|_| {
                    panic!(
                        "{} line {}: (verilog-indent-line) returned non-numeric {:?} for {:?}",
                        suffix, lineno, actual_s, line
                    )
                });
                if actual != expected {
                    mismatches.push((
                        suffix.clone(),
                        lineno,
                        expected,
                        actual_s.clone(),
                        line.to_string(),
                    ));
                }
            }
            run(&mut i, "(forward-line 1)");
        }
    }

    assert!(
        mismatches.is_empty(),
        "demo/rtl lines that don't reindent to their own on-disk column (file, line, expected, \
         actual, text): {:#?}\n\
         If a mismatch here is genuinely a new, documented, deliberate divergence (not a bug), \
         add it BY NAME to `known_divergences' above with a reason -- do not loosen this into a \
         blanket tolerance.",
        mismatches
    );
}

/// F4 (reviewer follow-up): `struct_union_member' is the SAME production
/// used for `union' fields, not just `struct' fields -- so a packed
/// union's field lines must indent identically to a packed struct's.
#[test]
fn verilog_package_union_field_line_indents_to_four_at_two_column_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package p;\ntypedef union packed {\nlogic [31:0] w;\nlogic [3:0][7:0] b;\n} u_t;\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic [31:0] w;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "union field line is package-depth (1) plus its own struct_union_member \
         (1) = 2 levels at this file's 2-column width, same production as struct"
    );
}

/// F4 (reviewer follow-up): a struct nested inside another struct. The
/// outer field (an anonymous struct type named `inner') is itself a
/// `struct_union_member' whose SPAN covers the whole nested block, so an
/// inner field's own `struct_union_member' stacks on top of it. Computed
/// by hand before writing the assertion (see the F4 spec): three ancestor
/// nodes match the block-type list on the way up from an inner field
/// (package_declaration, the outer struct_union_member for the `inner'
/// field, and the inner struct_union_member for `a'/`b' itself), so 3
/// levels, column 6 at this file's 2-column width; the outer struct's own
/// sibling field `c' (not inside the nested block) stays at 2 levels,
/// column 4, same as any ordinary flat struct field.
#[test]
fn verilog_nested_struct_inner_field_stacks_two_extra_levels() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "package p;\ntypedef struct packed {\nstruct packed {\nlogic a;\nlogic b;\n} inner;\nlogic c;\n} outer_t;\nendpackage\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic a;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "inner struct field line: package_declaration (1) + outer struct_union_member \
         for the `inner' field (1) + inner struct_union_member for `a' (1) = 3 levels"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic c;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "outer struct's own sibling field `c' (not inside the nested block) stays at \
         2 levels (package_declaration + its own struct_union_member), same as any \
         ordinary flat struct field"
    );
}

/// F4 (reviewer follow-up): the same depth arithmetic holds under a
/// different outer wrapper -- a `typedef struct' inside a `module', not a
/// `package'. module_declaration (1) + struct_union_member (1) = 2
/// levels = column 4, same shape as the package case.
#[test]
fn verilog_module_level_typedef_struct_field_line_indents_to_four_at_two_column_width() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\ntypedef struct packed {\nlogic a;\nlogic b;\n} req_t;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic a;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "module-level typedef struct field line is module-depth (1) plus its own \
         struct_union_member (1) = 2 levels at this file's 2-column width"
    );
}

/// M119 D2 fix round (FIX-1): real `verible-verilog-format
/// --indentation_spaces=2', run on this exact fixture (two unwrapped,
/// stacked `if_generate_construct's, neither inside `generate'/
/// `endgenerate'), puts the nested `if (Q)' at column 4 and `wire x;'
/// at column 6:
///
/// ```
/// module m;
///   if (P) begin : g_outer
///     if (Q) begin : g_inner
///       wire x;
///     end
///   end
/// endmodule
/// ```
///
/// (command: `verible-verilog-format --indentation_spaces=2
/// nested_if.sv', reproduced verbatim above). Before this fix,
/// `indent--verilog-generate-depth-adjust' found only the NEAREST
/// enclosing generate-construct and applied a flat -1 regardless of
/// nesting depth, so `if (Q)' computed column 6 and `wire x;' computed
/// column 8 -- one level (2 columns) too deep, confirmed red against
/// this exact fixture before the fix (`if_Q: 6', `wire_x: 8', measured
/// with a throwaway probe test run via `cargo test -p core --test
/// indent_tests m119_probe_nested_unwrapped_generate -- --nocapture').
#[test]
fn verilog_nested_unwrapped_if_generate_indents_one_level_per_nesting_level() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  if (P) begin : g_outer\n    if (Q) begin : g_inner\n      wire x;\n    end\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (P)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: the outer unwrapped generate-if's own header line is unaffected by the fix, \
         module-depth alone"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (Q)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: real verible puts the nested generate-if's header ONE level deeper than the \
         outer one, not two"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire x\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "measured: the innermost body is one level deeper again"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"    end\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the inner `end' dedents back to the inner `if''s own column"
    );
}

/// M119 D2 fix round (FIX-1, `for'-inside-`if' variant): same nesting
/// shape as `verilog_nested_unwrapped_if_generate_indents_one_level_
/// per_nesting_level' above, but the inner construct is a
/// `loop_generate_construct' (`for') rather than another
/// `if_generate_construct', confirming the ticket's explicit "cover
/// `for'-inside-`if' as well as `if'-inside-`if'" requirement. Real
/// `verible-verilog-format --indentation_spaces=2':
///
/// ```
/// module m;
///   if (P) begin : g_outer
///     for (genvar i = 0; i < 4; i = i + 1) begin : g_inner
///       wire x;
///     end
///   end
/// endmodule
/// ```
#[test]
fn verilog_nested_unwrapped_for_generate_inside_if_generate_indents_one_level_per_nesting_level() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  if (P) begin : g_outer\n    for (genvar i = 0; i < 4; i = i + 1) begin : g_inner\n      wire x;\n    end\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"for (genvar\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the nested `for'-generate header is one level deeper than the enclosing `if', \
         same as the if-inside-if case"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire x\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "measured: the innermost body is one level deeper again"
    );
}

/// M119 D2 fix round (FIX-3): the same over-count also reproduces when
/// BOTH generate-if levels sit inside `generate'/`endgenerate' --
/// reported by the reviewer as pre-existing (reproduces with D2's
/// unwrap correction forced to 0) rather than introduced by FIX-1. Real
/// `verible-verilog-format --indentation_spaces=2':
///
/// ```
/// module m;
///   generate
///     if (P) begin : g_outer
///       if (Q) begin : g_inner
///         wire x;
///       end
///     end
///   endgenerate
/// endmodule
/// ```
///
/// Before this fix, the nested `if (Q)' computed column 8 and `wire x;'
/// computed column 10 -- one level too deep, same mechanism as the
/// unwrapped case (measured via a throwaway probe test run against this
/// exact fixture before the fix: `if_Q: 8', `wire_x: 10').
/// M119 trailing round: the two tests above stop at TWO levels of
/// nesting, and the fix-round formula
/// `-((count - 1) + (0 if wrapped else 1))' is linear in `count' -- so
/// a future regression that happens to cancel out at exactly depth 2
/// would pass both of them. Three levels is the cheapest shape that
/// pins the linearity itself rather than one point on the line.
///
/// Measured, `verible-verilog-format --indentation_spaces=2':
///
///     module m;                        module m;
///       if (A) begin : g1     col 2      generate                 col 2
///         if (B) begin : g2   col 4        if (A) begin : g1      col 4
///           if (C) begin : g3 col 6          if (B) begin : g2    col 6
///             wire w;         col 8            if (C) begin : g3  col 8
///           end               col 6              wire w;          col 10
///         end                 col 4
///       end                   col 2
///     endmodule
///
/// Both forms are checked below; the wrapped one is the same shape
/// shifted down one level by `generate' itself.
#[test]
fn verilog_three_level_nested_generate_stays_linear_in_the_nesting_count() {
    for (label, src, cols) in [
        (
            "unwrapped",
            "module m;\n  if (A) begin : g1\n    if (B) begin : g2\n      if (C) begin : g3\n        wire w;\n      end\n    end\n  end\nendmodule\n",
            ["2", "4", "6", "8"],
        ),
        (
            "wrapped",
            "module m;\n  generate\n    if (A) begin : g1\n      if (B) begin : g2\n        if (C) begin : g3\n          wire w;\n        end\n      end\n    end\n  endgenerate\nendmodule\n",
            ["4", "6", "8", "10"],
        ),
    ] {
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, "(set-indent-width 2)");
        run(&mut i, &format!("(insert {:?})", src));
        for (needle, want) in [
            ("if (A)", cols[0]),
            ("if (B)", cols[1]),
            ("if (C)", cols[2]),
            ("wire w", cols[3]),
        ] {
            run(&mut i, "(goto-char (point-min))");
            run(&mut i, &format!("(search-forward {:?})", needle));
            run(&mut i, "(beginning-of-line)");
            assert_eq!(
                run(&mut i, "(verilog-indent-line)"),
                want,
                "{} three-level nesting: `{}' must land at column {} per real verible; a \
                 formula that is not linear in the generate-ancestor count breaks here while \
                 still passing the two-level tests above",
                label,
                needle,
                want
            );
        }
    }
}

#[test]
fn verilog_nested_wrapped_if_generate_indents_one_level_per_nesting_level() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  generate\n    if (P) begin : g_outer\n      if (Q) begin : g_inner\n        wire x;\n      end\n    end\n  endgenerate\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"generate\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "measured: `generate' itself is unaffected by this fix, module-depth alone"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (P)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: the outer wrapped generate-if's own header is unaffected by this fix, same as \
         the pre-existing single-level wrapped case"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"if (Q)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "measured: the nested wrapped generate-if's header is ONE level deeper than the outer \
         one, not two -- the same double-count mechanism as the unwrapped case, independent of \
         wrap/unwrap"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"wire x\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "measured: the innermost body is one level deeper again"
    );
}

/// M119 FIX-B: real `verible-verilog-format --indentation_spaces=2', run on
/// this exact fixture (module with an `import' clause and NO parameter
/// list, so the port list's own opening `(' is pushed onto its own line --
/// the same forcing mechanism as `soc_top.sv''s standalone `#(', just with
/// no parameter list ahead of it):
///
///   module top
///     import soc_pkg::*;
///   (
///       input logic clk
///   );
///   endmodule
///
/// puts the standalone `(' line at column 0, the header's own on-disk
/// column -- confirmed directly (`echo` fixture piped through real
/// `verible-verilog-format --indentation_spaces=2`, command and output
/// recorded in the M119 fix-round PR notes). This is the shape
/// `("(" . "list_of_port_declarations")' in `indent--verilog-closers'
/// exists for: deleting that entry makes this line compute column 4
/// (wrap step) instead of 0.
#[test]
fn verilog_import_clause_with_no_parameter_list_pushes_port_list_open_paren_to_its_own_line_at_column_zero(
) {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top\n  import soc_pkg::*;\n(\n  input logic clk\n);\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"import soc_pkg::*;\\n\")");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: `verible-verilog-format --indentation_spaces=2' puts the standalone `(' line \
         (port list opener, no parameter list ahead of it) at column 0, the header's own \
         on-disk column -- the direct guard for the `(\"(\" . \"list_of_port_declarations\")' \
         contextual closer entry, which was otherwise unreachable from every file under \
         `demo/rtl' (none currently splits the port-list opener onto its own line this way)"
    );
}

/// M122: `property_spec' pinning -- deleting the `"property_spec"' entry
/// from `indent--block-node-types' must make this go red (the deletion
/// question for this fix). Real material:
/// `demo/rtl/bus/axi4_lite_if.sv''s `p_aw_stable' property.
#[test]
fn verilog_property_spec_body_indents_one_level_against_real_axi4_lite_if_sv() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl/bus/axi4_lite_if.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/rtl/bus/axi4_lite_if.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        "(search-forward \"@(posedge clk_i) disable iff (!rst_ni) (aw_valid\")",
    );
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "property_spec's own body line must sit one level inside interface_declaration (column \
         4), matching demo/rtl/bus/axi4_lite_if.sv's own on-disk column -- deleting \
         \"property_spec\" from indent--block-node-types makes this go red (computes 2)"
    );
}

/// M122: `modport_item' wrap-step pinning, paired with the
/// `(\")\" . \"modport_item\")' closer and
/// `indent--verilog-modport-item-closer-adjust'. Deleting any one of the
/// three must make this go red. Real material:
/// `demo/rtl/bus/axi4_lite_if.sv''s `modport master(...)'.
#[test]
fn verilog_modport_wrapped_port_list_gets_a_wrap_step_against_real_axi4_lite_if_sv() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl/bus/axi4_lite_if.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/rtl/bus/axi4_lite_if.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"output aw_addr, aw_valid,\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "a wrapped modport port line must sit at interface_declaration's own depth (2) plus \
         the fixed wrap step (4) = 6, matching demo/rtl/bus/axi4_lite_if.sv's own on-disk \
         column -- deleting \"modport_item\" from indent--wrap-node-types makes this go red \
         (computes 2, no wrap step at all)"
    );
    // The modport's own closing `)' must land back at the header's own
    // column (2), not the wrapped column (6) nor over-dedent to 0.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"output r_ready\\n  );\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "modport_item's own closing ')' must dedent back to the interface body's own column \
         (2), matching `modport master('s own header line -- deleting \
         indent--verilog-modport-item-closer-adjust's +1 correction makes this go red \
         (computes 0, over-dedented)"
    );
}

/// M122: the nested header-line-quirk compounding, named in
/// `known_divergences' above -- pinned directly rather than only via the
/// whole-file sweep, so deleting the divergence entries (or "fixing" the
/// underlying quirk without updating this test) is caught by name. Real
/// material: `demo/verif/soc_verif_pkg.sv''s `class rw_checker;', nested
/// inside `package soc_verif_pkg;'.
#[test]
fn verilog_nested_class_header_line_compounds_the_documented_quirk_against_real_soc_verif_pkg_sv() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/verif/soc_verif_pkg.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/verif/soc_verif_pkg.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"class rw_checker;\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "documented v1 gap, not a regression: a class nested inside a package computes its \
         own header line at column 4 (double-counting package_declaration + \
         class_declaration's own self-reference) against its real on-disk column 2 -- see \
         known_divergences's M122 comment in \
         verilog_every_demo_rtl_file_reindents_to_its_own_on_disk_columns_except_named_divergences"
    );
}

/// M122 review fix: `conditional_compilation_directive' always computes
/// column 0, however deeply nested -- deleting the special case in
/// `indent--query-pos-and-depth' makes this go red (both assertions:
/// the nested case would compute a normal block-depth column instead
/// of 0). Real material: `demo/rtl/bus/axi4_lite_if.sv''s `` `ifndef
/// SOC_SVA_OFF `` (nested one level inside the interface body) and
/// `demo/rtl/mem/sram_bank.sv''s equivalent guard.
#[test]
fn verilog_conditional_compilation_directive_is_always_column_zero_against_real_files() {
    for (rel, needle) in [
        ("rtl/bus/axi4_lite_if.sv", "`ifndef SOC_SVA_OFF"),
        ("rtl/bus/axi4_lite_if.sv", "`endif"),
        ("rtl/mem/sram_bank.sv", "`ifndef SOC_SVA_OFF"),
        ("rtl/mem/sram_bank.sv", "`endif"),
        ("verif/sram_bank_tb.sv", "`ifndef SOC_COVERAGE_OFF"),
        ("verif/sram_bank_tb.sv", "`endif"),
    ] {
        let path =
            std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo")).join(rel);
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let (mut i, _ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, "(set-indent-width 2)");
        run(&mut i, &format!("(insert {:?})", src));
        run(&mut i, "(goto-char (point-min))");
        run(&mut i, &format!("(search-forward {needle:?})"));
        run(&mut i, "(beginning-of-line)");
        assert_eq!(
            run(&mut i, "(verilog-indent-line)"),
            "0",
            "{rel}: {needle:?} must compute column 0, matching its real on-disk column, however \
             deeply nested the surrounding block is"
        );
    }
}

/// M122 review fix: a directive nested TWO levels deep (inside a
/// `generate_block' inside an `if_generate_construct') still computes
/// column 0, matching real `verible-verilog-format' exactly (dump-
/// verified separately) -- not one level per nesting depth like an
/// ordinary block-type node.
#[test]
fn verilog_conditional_compilation_directive_nested_two_levels_deep_is_still_column_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m2;\n  if (1) begin : g\n`ifndef Y\n    wire w;\n`endif\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"`ifndef Y\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "0");
    run(&mut i, "(search-forward \"`endif\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "0");
}

/// M122 review fix (second round): a coverpoint's own multi-line
/// `{...}' concatenation gets a normal one-level block-depth bump, and
/// its closing `}' DEDENTS back out to the coverpoint's own column --
/// the FIRST review round's pinned expectation here ("6", staying at
/// the wrapped-content column) was wrong, read off what this engine
/// computed rather than off verible, exactly the mistake this
/// project's rules warn against. Re-derived from a real run:
///
///   $ verible-verilog-format --indentation_spaces=2 <<'EOF'
///   module m;
///     logic a, b;
///     covergroup cg @(posedge a);
///       cp: coverpoint {a, b} {
///         bins x = {2'b00};
///       }
///     endgroup
///   endmodule
///   EOF
///   module m;
///     logic a, b;
///     covergroup cg @(posedge a);
///       cp: coverpoint {
///         a, b
///       } {
///         bins x = {2'b00};
///       }
///     endgroup
///   endmodule
///
/// `cp: coverpoint {' at column 4, members at column 6, `} {' back at
/// column 4 -- IDENTICAL treatment to the `case' selector in the test
/// below. Deleting the `"concatenation"' entry in
/// `indent--block-node-types', or the `("}" . "concatenation")' closer
/// entry in `indent--verilog-closers', each make one of the two
/// assertions here go red. Real material: `demo/verif/sram_bank_tb.sv'
/// (this exact shape existed only briefly during M122's own review
/// rounds, worked around first one way then another -- restored to its
/// natural shape, with verible's own on-disk columns, once this fix
/// actually landed).
#[test]
fn verilog_coverpoint_concatenation_expression_gets_a_block_level_and_its_closer_dedents() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/verif/sram_bank_tb.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/verif/sram_bank_tb.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        "(search-forward \"cp_channel_activity: coverpoint {\\n\")",
    );
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "a wrapped coverpoint concatenation's own members must sit one block level inside the \
         coverpoint (column 6), matching real verible (command and output quoted above this \
         test)"
    );
    run(&mut i, "(search-forward \"} {\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "the concatenation's own closing '}}' must dedent back to the coverpoint's own column \
         (4), not stay at the wrapped-content column (6) -- corrected in the second M122 review \
         round; see this test's own doc comment for the verible command that proves it"
    );
}

/// M122 review fix: a `case' statement's own `{...}' selector gets the
/// SAME one-level block bump AND THE SAME closer-dedent treatment as a
/// coverpoint's (see the test above -- both shapes are handled by the
/// exact same generic `("}" . "concatenation")' closer entry, with no
/// per-context special case needed at all). Real material:
/// `demo/verif/axi4_lite_monitor.sv''s `case ({...})' inside its
/// outstanding-transaction counters. Re-derived from a real run:
///
///   $ verible-verilog-format --indentation_spaces=2 <<'EOF'
///   module m;
///     logic a, b;
///     always_comb case ({a, b})
///       2'b10: ;
///     endcase
///   endmodule
///   EOF
///   (wraps identically: `case ({' / members one level in / `})' back
///   at `case''s own column -- see `demo/verif/axi4_lite_monitor.sv'
///   itself for the exact real-file columns asserted below: `case ({'
///   at column 6, members at column 8, `})' back at column 6.)
#[test]
fn verilog_case_selector_concatenation_closer_dedents_the_same_way_as_a_coverpoint_expression() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/verif/axi4_lite_monitor.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/verif/axi4_lite_monitor.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(
        &mut i,
        "(search-forward \"bus.aw_valid && bus.aw_ready, bus.b_valid && bus.b_ready\")",
    );
    run(&mut i, "(beginning-of-line)");
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "8");
    run(&mut i, "(search-forward \"})\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "a case selector's own concatenation closer must dedent back to the case statement's \
         own column (6), not stay at the wrapped-content column (8) -- the SAME treatment a \
         coverpoint's closer gets, see the test above"
    );
}

/// M122 review fix (second round): the reviewer's own counterexample to
/// the (now corrected, see `indent--verilog-depth-adjust''s docstring)
/// false "mutually exclusive" claim -- a `case' whose selector is a
/// wrapped concatenation, itself inside an (unwrapped, module-scope)
/// generate-if: `indent--verilog-generate-depth-adjust' (D2) and the
/// `("}" . "concatenation")' closer both act on the SAME closing `})'
/// line, D2 via the DEPTH-ADJUST-FN sum and the closer via its own
/// separate -1 step, and the combination is correct. Re-derived from a
/// real run:
///
///   $ verible-verilog-format --indentation_spaces=2 <<'EOF'
///   module m;
///     logic a, b;
///     logic [7:0] result;
///     if (1) begin : g
///       always_comb begin
///         case ({a, b})
///           2'b10: result = 8'h1;
///           default: result = 8'h0;
///         endcase
///       end
///     end
///   endmodule
///   EOF
///   module m;
///     logic a, b;
///     logic [7:0] result;
///     if (1) begin : g
///       always_comb begin
///         case ({
///           a, b
///         })
///           2'b10:   result = 8'h1;
///           default: result = 8'h0;
///         endcase
///       end
///     end
///   endmodule
///
/// members at column 8, `})' back at column 6.
#[test]
fn verilog_case_selector_concatenation_inside_a_generate_if_composes_correctly() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module m;\n  logic a, b;\n  logic [7:0] result;\n  if (1) begin : g\n    always_comb begin\n      case ({\n        a, b\n      })\n        2'b10:   result = 8'h1;\n        default: result = 8'h0;\n      endcase\n    end\n  end\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"case ({\\n\")");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "a case selector's wrapped concatenation, inside a generate-if, must still land one \
         block level in from the case (column 8) -- D2's generate correction and the raw block \
         depth (including the concatenation's own +1) must compose correctly"
    );
    run(&mut i, "(search-forward \"})\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "the closer must dedent back to the case statement's own column (6) even with D2's \
         generate correction also active on the same line -- proves the two mechanisms compose \
         via addition, not by picking one over the other"
    );
}

/// M122 review fix: an `assert (...) else STATEMENT;' with no
/// `begin'/`end' (braceless), whose STATEMENT itself wraps across
/// multiple lines, gets a one-level bump for the statement's own
/// opening line -- deleting `indent--verilog-bare-action-block-adjust'
/// makes this go red. Real material: `demo/verif/sram_bank_tb.sv''s
/// own `assert (ok) else $error(...)' (this exact shape existed briefly
/// during M122's own review round, worked around by wrapping it in
/// `begin'/`end' instead of fixing the engine -- restored to its
/// natural braceless shape once this fix landed).
#[test]
fn verilog_bare_else_wrapped_statement_gets_a_block_level_against_real_sram_bank_tb_sv() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/verif/sram_bank_tb.sv"
    );
    let src = std::fs::read_to_string(path).expect("demo/verif/sram_bank_tb.sv must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"assert (ok)\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "6",
        "sanity: 'assert (ok)' itself unaffected by this fix"
    );
    run(&mut i, "(search-forward \"$error(\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "the braceless else-statement's own opening line must sit one level in from 'assert'/\
         'else' (column 8, matching real verible)"
    );
    run(&mut i, "(search-forward \"observed\\n\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "8",
        "the closing ');' must dedent back to the else-statement's own opening column (8)"
    );
}

// --- M124 Part A: non-ANSI (`list_of_ports') wrapped port lists --------
// -------------------------------------------------------------------------
// Ground truth, run today with `verible-verilog-format
// --indentation_spaces=2' on a non-ANSI `module top (\n  clk,\n  rst\n);'
// header: continuation lines column 4, closing `);' column 0, an
// ordinary body line stays at column 2. Dump-verified (M124) that
// `list_of_ports' sits as a SIBLING of `parameter_port_list' inside
// `module_nonansi_header'/`interface_nonansi_header'/
// `program_nonansi_header' alike -- see `indent--wrap-node-types''s own
// M124 comment in indent.el for the exact dump output. These tests are
// deliberately named `..._non_ansi_...' so they cannot be confused with
// the M90 `list_of_port_connections' tests above (an INSTANTIATION's own
// port list, a different grammar node entirely) -- reconnaissance nearly
// mistook those four for coverage of this gap.

#[test]
fn verilog_non_ansi_module_header_wrapped_port_list_continuation_lands_at_wrap_step() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top (\n  clk,\n  rst\n);\n  input clk;\n  input rst;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"clk,\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "measured: real verible-verilog-format --indentation_spaces=2 puts a non-ANSI header's \
         wrapped port-list continuation at column 4, not the module body's own block depth (2)"
    );
}

#[test]
fn verilog_non_ansi_module_header_wrapped_port_list_second_continuation_does_not_stack() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top (\n  clk,\n  rst\n);\n  input clk;\n  input rst;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"rst\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "second continuation line stays at 4 too, not 8 -- wrap steps do not stack per line"
    );
}

#[test]
fn verilog_non_ansi_module_header_closing_paren_line_is_column_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top (\n  clk,\n  rst\n);\n  input clk;\n  input rst;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: real verible puts the closing `);' at column 0, not the wrap step's 4"
    );
}

#[test]
fn verilog_non_ansi_module_header_ordinary_body_line_stays_at_two() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top (\n  clk,\n  rst\n);\n  input clk;\n  input rst;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"input rst\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "2",
        "an ordinary body line (outside the port-list header) is unaffected by this fix"
    );
}

/// The `#(...)' parameter-list-plus-non-ANSI-port-list combination:
/// dump-verified (M124) that `parameter_port_list' and `list_of_ports'
/// sit side by side inside the same `module_nonansi_header', so all
/// four columns below were already measured against real
/// `verible-verilog-format --indentation_spaces=2' output quoted in the
/// M124 spec. The parameter lines and the `) (' transition line were
/// already correct before this fix (via the pre-existing
/// `parameter_port_list' entries); only the port continuations and the
/// final closer were wrong -- this test pins all four so a future
/// change cannot silently break the ones that already worked.
#[test]
fn verilog_non_ansi_module_header_with_parameter_port_list_gets_all_four_columns_right() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top #(\n  parameter W = 8,\n  parameter DEPTH = 16\n) (\n  clk,\n  \
             rst\n);\n  input clk;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"parameter W\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(run(&mut i, "(verilog-indent-line)"), "4", "parameter line");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \") (\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "the `) (' transition line"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"clk,\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "the non-ANSI port-list continuation, the column this fix corrects"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "the final closing `);', the other column this fix corrects"
    );
}

/// Dump-verified (M124) that `interface_nonansi_header' uses the same
/// `list_of_ports' node kind name as the module case, so the shared
/// table entry covers it too -- proven with a real test rather than
/// asserted.
#[test]
fn verilog_non_ansi_interface_header_wrapped_port_list_gets_the_same_treatment() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "interface simple_bus (\n  clk,\n  rst_n\n);\n  input clk;\nendinterface\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"clk,\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "4",
        "non-ANSI interface header port-list continuation, column 4"
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \");\")");
    run(&mut i, "(beginning-of-line)");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "non-ANSI interface header closing `);', column 0"
    );
}

/// M124 fix round: the non-ANSI counterpart of
/// `verilog_import_clause_with_no_parameter_list_pushes_port_list_open_
/// paren_to_its_own_line_at_column_zero' above -- named differently so
/// the two cannot be confused. Reproduced (2026-09-08) before this fix
/// landed: `verilog-indent-line' returned "4" here (wrap-depth stayed 1
/// because `("(" . "list_of_ports")' was missing from
/// `indent--verilog-closers', so the wrap step never got cancelled),
/// where real `verible-verilog-format --indentation_spaces=2' puts this
/// line at column 0:
/// ```
/// module top
///   import soc_pkg::*;
/// (
///     clk,
///     rst
/// );
///   input clk;
/// ```
#[test]
fn verilog_non_ansi_import_clause_pushes_port_list_open_paren_to_its_own_line_at_column_zero() {
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(
        &mut i,
        &format!(
            "(insert {:?})",
            "module top\n  import soc_pkg::*;\n(\n  clk,\n  rst\n);\n  input clk;\nendmodule\n"
        ),
    );
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"import soc_pkg::*;\\n\")");
    assert_eq!(
        run(&mut i, "(verilog-indent-line)"),
        "0",
        "measured: `verible-verilog-format --indentation_spaces=2' puts the standalone `(' \
         line (non-ANSI port list opener, forced onto its own line by the preceding `import' \
         clause) at column 0, the header's own on-disk column -- the direct guard for the \
         `(\"(\" . \"list_of_ports\")' contextual closer entry"
    );
}

/// M124 Part A/E: real, non-fixture material for the non-ANSI wrapped
/// port list fix -- every OTHER file under `demo/rtl'/`demo/rtl-
/// verilog2001' is ANSI-style, so this sweep is the only place in the
/// repo that exercises the non-ANSI shape against real on-disk,
/// verible-canonical columns rather than a hand-typed fixture (the exact
/// trap M110/M119 warn about). `demo/rtl-verilog2001/gray_ctr.v` is
/// `verible-verilog-format --indentation_spaces=2 --verify`-clean on
/// disk (checked by `demo/tools/lint_rtl.sh`), so its own columns ARE
/// ground truth. Exactly one line is skipped: `module gray_ctr (` itself
/// hits the SAME pre-existing, already-documented module-header-line
/// quirk `demo/rtl/core/alu.sv`/`demo/rtl/pkg/soc_pkg.sv` are skipped
/// for above (a header line and the module's first real body line both
/// compute block-depth 1, even though their real on-disk columns
/// differ) -- not something this fix introduces or changes, since it
/// only affects the WRAPPED port-list lines below the header, not the
/// header line itself.
#[test]
fn verilog_demo_rtl_verilog2001_gray_ctr_v_full_file_reindents_to_its_own_on_disk_columns() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../demo/rtl-verilog2001/gray_ctr.v"
    );
    let src = std::fs::read_to_string(path).expect("demo/rtl-verilog2001/gray_ctr.v must exist");
    let (mut i, _ed) = setup();
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(set-indent-width 2)");
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");

    let mut mismatches: Vec<(usize, usize, String, String)> = Vec::new();
    for (idx, line) in src.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim_start();
        if !trimmed.is_empty() && line != "module gray_ctr (" {
            let expected = line.len() - trimmed.len();
            run(&mut i, "(beginning-of-line)");
            let actual_s = run(&mut i, "(verilog-indent-line)");
            let actual: usize = actual_s.parse().unwrap_or_else(|_| {
                panic!(
                    "line {}: (verilog-indent-line) returned non-numeric {:?} for {:?}",
                    lineno, actual_s, line
                )
            });
            if actual != expected {
                mismatches.push((lineno, expected, actual_s.clone(), line.to_string()));
            }
        }
        run(&mut i, "(forward-line 1)");
    }

    assert!(
        mismatches.is_empty(),
        "gray_ctr.v lines that don't reindent to their own on-disk column, which is \
         verible-produced ground truth (line, expected, actual, text): {:#?}\n\
         NOTE: this test hardcodes the string \"module gray_ctr (\" to skip the one \
         known, already-documented module-header-line quirk (see the soc_pkg.sv/alu.sv \
         tests above). If the mismatch is on the header line and the module was \
         renamed, this comparison and that string constant are both stale, not a real \
         regression.",
        mismatches
    );
}
