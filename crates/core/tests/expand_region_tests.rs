//! M101: `expand-region`/`contract-region` (crates/core/lisp/expand-region.el).
//! See that file's header for the algorithm and known gaps.

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

/// Evaluate SRC, panicking (with the interpreter's own error text) if it
/// signals -- used where a test needs the raw `Value`, not its printed
/// form, and an error here is a setup bug, not something under test.
fn eval(interp: &mut Interp, src: &str) -> Value {
    match interp.eval_source(src) {
        Ok(v) => v,
        Err(flow) => panic!("eval {:?} failed: {}", src, interp.describe_flow(&flow)),
    }
}

/// Current `(buffer-substring (region-beginning) (region-end))`, as a
/// plain Rust `String` (not a printed/quoted Lisp form).
fn selected(interp: &mut Interp) -> String {
    match eval(interp, "(buffer-substring (region-beginning) (region-end))") {
        Value::Str(s) => s.as_str().to_string(),
        v => panic!("expected a string, got {}", prin1_to_string(interp, &v)),
    }
}

fn region_active(interp: &mut Interp) -> bool {
    matches!(eval(interp, "(region-active-p)"), Value::Sym(s) if interp.sym_name(s) == "t")
}

/// 1-based char position of BYTE_OFFSET into SRC -- SRC may contain
/// multi-byte (e.g. CJK) characters, so this is not just `byte_offset + 1`.
fn char_pos(src: &str, byte_offset: usize) -> i64 {
    src[..byte_offset].chars().count() as i64 + 1
}

/// Insert SRC into a fresh buffer and place point at the 1-based char
/// position corresponding to BYTE_OFFSET into SRC.
fn setup_at(src: &str, byte_offset: usize) -> (Interp, Rc<RefCell<Editor>>) {
    let (mut i, ed) = setup();
    run(&mut i, &format!("(insert {:?})", src));
    let pos = char_pos(src, byte_offset);
    run(&mut i, &format!("(goto-char {})", pos));
    (i, ed)
}

/// Call `(expand-region)` repeatedly (up to MAX times) until the
/// selected text equals TARGET, returning true if it did. Leaves
/// whatever state the last call produced (whether it found TARGET or
/// exhausted MAX attempts) -- callers that chase a second target after
/// this one rely on that.
fn expand_until(interp: &mut Interp, target: &str, max: usize) -> bool {
    for _ in 0..max {
        run(interp, "(expand-region)");
        if selected(interp) == target {
            return true;
        }
    }
    false
}

const ALU_SRC: &str = "module alu (\n  input logic [7:0] operand_a_i,\n  input logic [7:0] operand_b_i,\n  output logic [7:0] result_d\n);\n  // add inputs together\n  always_comb begin\n    case (1'b1)\n      1'b1: begin\n        result_d = operand_a_i + operand_b_i;\n      end\n    endcase\n  end\nendmodule\n";

#[test]
fn word_start_selects_the_identifier() {
    let byte = ALU_SRC.find("operand_a_i").unwrap() + 3; // inside the word
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "operand_a_i");
}

#[test]
fn region_state_after_expand_matches_mark_and_point() {
    let byte = ALU_SRC.find("operand_a_i").unwrap() + 3;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(expand-region)");
    assert!(region_active(&mut i));
    assert_eq!(run(&mut i, "(mark)"), run(&mut i, "(region-beginning)"));
    assert_eq!(run(&mut i, "(point)"), run(&mut i, "(region-end)"));
}

#[test]
fn ladder_never_repeats_and_strictly_grows() {
    let byte = ALU_SRC.find("operand_a_i").unwrap() + 3;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    let mut seen = Vec::new();
    for _ in 0..4 {
        run(&mut i, "(expand-region)");
        seen.push(selected(&mut i));
    }
    for a in 0..seen.len() {
        for b in (a + 1)..seen.len() {
            assert_ne!(
                seen[a], seen[b],
                "steps {} and {} repeated: {:?}",
                a, b, seen
            );
        }
    }
    for w in seen.windows(2) {
        assert!(
            w[1].len() > w[0].len(),
            "expected strictly growing lengths, got {:?}",
            seen
        );
    }
}

#[test]
fn verilog_ladder_reaches_the_port_connection_then_the_instantiation() {
    let src = "module top;\n\n  sub_mod u_sub(.port_name(signal));\n\nendmodule\n";
    let byte = src.find("signal").unwrap() + 1;
    let (mut i, _ed) = setup_at(src, byte);
    run(&mut i, "(verilog-mode)");
    assert!(
        expand_until(&mut i, ".port_name(signal)", 10),
        "never reached the port connection; last selection: {}",
        selected(&mut i)
    );
    assert!(
        expand_until(&mut i, "sub_mod u_sub(.port_name(signal));", 10),
        "never reached the full instantiation; last selection: {}",
        selected(&mut i)
    );
}

#[test]
fn leading_whitespace_selects_the_line_not_the_whole_module() {
    // Point sits in the leading whitespace of the `always_comb` line.
    let byte = ALU_SRC.find("  always_comb").unwrap() + 1;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    run(&mut i, "(expand-region)");
    let sel = selected(&mut i);
    // The candidate is widened to include the anchor itself (see
    // `expand-region--line-content-candidate`'s header), so a click one
    // space short of the trimmed content pulls in that one space too --
    // trimming the result is what matters here, not the exact leading
    // whitespace, and that it is nowhere near the size of the whole module.
    assert_eq!(sel.trim(), "always_comb begin");
    assert_ne!(sel, ALU_SRC);
}

#[test]
fn anchor_uses_region_beginning_not_point_when_a_region_is_already_active() {
    // R3 mutation-testing gap: `(anchor (if active (region-beginning)
    // (point)))' -- mutating this to always use `(point)' makes all 14
    // other tests pass unchanged (tree-sitter's ancestor-node candidates
    // can't tell the two apart: walking up from either end of a region
    // reaches the same lowest common ancestor first). The one place they
    // DO diverge is `expand-region--line-content-candidate''s widening
    // fix-up (see its docstring): it only gets a chance to widen past
    // the trimmed start/end at all if it's handed the WHITESPACE end of
    // the region as the anchor.
    //
    // Build a region by hand: mark at the very first character of an
    // indented line (inside its leading whitespace), point moved into
    // the word on that line. `region-beginning' is the mark (in the
    // whitespace); `(point)' is the word position instead.
    let line_beg_byte = ALU_SRC.find("  always_comb").unwrap();
    let word_byte = ALU_SRC.find("always_comb").unwrap() + 3;
    let (mut i, _ed) = setup_at(ALU_SRC, word_byte);
    run(&mut i, "(verilog-mode)");
    run(
        &mut i,
        &format!("(set-mark {})", char_pos(ALU_SRC, line_beg_byte)),
    );
    run(
        &mut i,
        &format!("(goto-char {})", char_pos(ALU_SRC, word_byte)),
    );
    assert!(region_active(&mut i));

    run(&mut i, "(expand-region)");
    let sel = selected(&mut i);
    // Correct (anchor = region-beginning, in the whitespace): the
    // widened line-content candidate starts at the mark itself, so the
    // leading whitespace is part of the selection.
    assert_eq!(sel, "  always_comb begin");
    // Under the `anchor = (point)' mutation, the trimmed candidate no
    // longer reaches back to the mark (in the whitespace), gets
    // filtered out by `expand-region--pick', and the ladder jumps
    // straight to something far bigger -- pin that this stays small.
    assert_ne!(sel, ALU_SRC);
    assert_ne!(sel, ALU_SRC.trim_end());
}

#[test]
fn comment_text_expands_without_error() {
    let byte = ALU_SRC.find("add inputs").unwrap() + 1;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    // Word inside the comment, then the whole comment line, then the
    // enclosing module (no tree-sitter node sits strictly between a
    // `comment' node and the module here), then the whole buffer
    // (the buffer candidate includes ALU_SRC's trailing newline, one
    // char past the module-only candidate).
    let expected = ["add", "// add inputs together", ALU_SRC.trim_end(), ALU_SRC];
    for want in expected {
        let r = run(&mut i, "(expand-region)");
        assert!(!r.starts_with("ERROR"), "expand-region errored: {}", r);
        assert_eq!(selected(&mut i), want);
    }
}

#[test]
fn incomplete_syntax_still_expands_at_least_once() {
    let src = "module top;\n  assign a = b +\nendmodule\n";
    let byte = src.find('b').unwrap() + 1; // the `b` in `= b +`
    let (mut i, _ed) = setup_at(src, byte);
    run(&mut i, "(verilog-mode)");
    // The dangling `+' with nothing after it puts an `ERROR' node
    // somewhere in the parse tree, but `expand-region--treesit-candidates'
    // collects those like any other node (see the file header) -- the
    // ladder still runs cleanly: word, then the (broken) statement line,
    // then the module, then the whole buffer.
    let expected = ["b", "assign a = b +", src.trim_end(), src];
    for want in expected {
        let r = run(&mut i, "(expand-region)");
        assert!(!r.starts_with("ERROR"), "expand-region errored: {}", r);
        assert_eq!(selected(&mut i), want);
    }
}

#[test]
fn contract_reverses_the_expand_sequence_then_deactivates() {
    let byte = ALU_SRC.find("operand_a_i").unwrap() + 3;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    let mut expanded = Vec::new();
    for _ in 0..3 {
        run(&mut i, "(expand-region)");
        expanded.push(selected(&mut i));
    }
    // Contract once: back to the state right after the 2nd expand.
    run(&mut i, "(contract-region)");
    assert_eq!(selected(&mut i), expanded[1]);
    // Contract again: back to right after the 1st expand.
    run(&mut i, "(contract-region)");
    assert_eq!(selected(&mut i), expanded[0]);
    // Contract a third time: back to no region at all.
    run(&mut i, "(contract-region)");
    assert!(!region_active(&mut i));
}

#[test]
fn contract_with_no_history_does_not_error() {
    let (mut i, _ed) = setup_at("hello world\n", 0);
    let before_active = region_active(&mut i);
    let r = run(&mut i, "(contract-region)");
    assert!(!r.starts_with("ERROR"), "contract-region errored: {}", r);
    assert_eq!(region_active(&mut i), before_active);
}

#[test]
fn moving_point_breaks_the_sequence_and_restarts_at_the_new_anchor() {
    let src = "first_ident second_line_here\n";
    let second_byte = src.find("second_line_here").unwrap() + 3;
    let (mut i, _ed) = setup_at(src, second_byte);
    // Two expansions: word "second_line_here", then the whole line
    // (mark ends up at position 1, the start of "first_ident").
    run(&mut i, "(expand-region)");
    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), src.trim_end());

    // Move point into "first_ident" -- a different identifier than the
    // one this sequence grew from.
    let first_byte = src.find("first_ident").unwrap() + 3;
    run(
        &mut i,
        &format!("(goto-char {})", char_pos(src, first_byte)),
    );
    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "first_ident");
    assert_ne!(selected(&mut i), src.trim_end());
}

#[test]
fn non_treesit_buffer_ladder_is_word_line_paragraph_buffer() {
    let src = "hello world\nsecond line here\n\nthird paragraph\n";
    let byte = src.find("world").unwrap() + 1;
    // Deliberately never calls a major-mode function, so
    // `treesit--buffer-language' stays nil and only the text ladder runs.
    let (mut i, _ed) = setup_at(src, byte);

    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "world");

    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "hello world");

    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "hello world\nsecond line here");

    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), src);
}

#[test]
fn multibyte_text_before_the_identifier_does_not_shift_positions() {
    let src = "// \u{6ce8}\u{91ca}\u{6d4b}\u{8bd5} comment\nsome_identifier_after\n";
    let byte = src.find("some_identifier_after").unwrap() + 4;
    let (mut i, _ed) = setup_at(src, byte);
    run(&mut i, "(expand-region)");
    assert_eq!(selected(&mut i), "some_identifier_after");
}

#[test]
fn empty_buffer_and_point_max_do_not_error() {
    let (mut i, _ed) = setup();
    let r = run(&mut i, "(expand-region)");
    assert!(!r.starts_with("ERROR"), "expand-region errored: {}", r);

    run(&mut i, "(insert \"abc\")");
    run(&mut i, "(goto-char (point-max))");
    let r = run(&mut i, "(expand-region)");
    assert!(!r.starts_with("ERROR"), "expand-region errored: {}", r);
    // Pins `expand-region--word-candidate''s `(< end (point-max))' bound:
    // point sits AT point-max (right after the `c'), so the rightward
    // scan must still pick up all of "abc" rather than stopping one
    // character short (or reading past the end of the buffer).
    assert_eq!(selected(&mut i), "abc");
}

#[test]
fn moving_point_mid_sequence_resets_history_so_a_second_contract_is_a_noop() {
    // R2 mutation-testing gap: `(unless (and active expand-region--last
    // (equal expand-region--last cur)) (setq-local expand-region--history
    // nil))' resets the growth history whenever the region has drifted
    // away from what the last expand/contract produced -- `goto-char'
    // does exactly that (it moves point but never touches mark or
    // `mark-active', see the file header's "History / freshness"
    // section), so three expansions, a `goto-char' elsewhere, then one
    // more expansion must leave EXACTLY one entry in the history, not
    // four. Mutating that `unless' condition to `t' (never reset) makes
    // all 14 other tests in this file pass unchanged; only comparing the
    // result of a SECOND contract catches it (see below).
    let byte = ALU_SRC.find("operand_a_i").unwrap() + 3;
    let (mut i, _ed) = setup_at(ALU_SRC, byte);
    run(&mut i, "(verilog-mode)");
    for _ in 0..3 {
        run(&mut i, "(expand-region)");
    }
    // Move point to a distant identifier. `region-active-p' stays t
    // (mark is untouched), but the region now spans from the old mark
    // to this new point -- nothing like `expand-region--last'.
    let far_byte = ALU_SRC.find("result_d").unwrap() + 1;
    run(
        &mut i,
        &format!("(goto-char {})", char_pos(ALU_SRC, far_byte)),
    );
    assert!(region_active(&mut i));

    // One more expansion: correct behavior resets the history to a
    // single entry (the odd drifted region above) before pushing it.
    run(&mut i, "(expand-region)");

    // First contract: pops that single entry -- identical in both the
    // correct implementation and the `(unless t)' mutant, since it's the
    // top of the stack either way. Not itself the distinguishing step.
    run(&mut i, "(contract-region)");
    let after_first_contract = selected(&mut i);

    // Second contract: the correct implementation's history is now
    // empty, so this must be a no-op (unchanged selection, or the
    // region no longer active if the stack bottomed out at a bare
    // point). The `(unless t)' mutant instead still has the three
    // pre-goto-char entries left over and pops one of them, changing
    // the selection.
    run(&mut i, "(contract-region)");
    if region_active(&mut i) {
        assert_eq!(
            selected(&mut i),
            after_first_contract,
            "second contract should have been a no-op"
        );
    }
}

#[test]
fn key_bindings_reach_expand_and_contract_region() {
    // C-= / M-= : expand.
    for key in ["C-=", "M-="] {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"abc def\")");
        run(&mut i, "(goto-char 2)"); // inside "abc"
        feed_keys(&mut i, &ed, key).unwrap();
        assert!(region_active(&mut i), "{} did not activate a region", key);
        assert_eq!(selected(&mut i), "abc", "{} selected the wrong text", key);
    }
    // C-- / M-- : contract. Build the region directly via `expand-region`
    // first (bypassing the keymap), then feed only the contract key.
    for key in ["C--", "M--"] {
        let (mut i, ed) = setup();
        run(&mut i, "(insert \"abc def\")");
        run(&mut i, "(goto-char 2)");
        run(&mut i, "(expand-region)");
        assert!(region_active(&mut i));
        feed_keys(&mut i, &ed, key).unwrap();
        assert!(
            !region_active(&mut i),
            "{} did not contract back to no region",
            key
        );
    }
}
