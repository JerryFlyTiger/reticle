//! M108: regression guard for the `Buffer::ts_tree` parse cache
//! (`crates/core/src/buffer.rs`, `crates/core/src/treesit.rs`). Before
//! this milestone, `treesit::parse` re-materialized the whole buffer
//! text AND re-ran `tree_sitter::Parser::parse` on EVERY call, even
//! when nothing had changed since the previous call for the same
//! buffer/language -- `indent-for-tab-command` on a real Verilog file
//! measured at 0.34ms/TAB for 61 lines up to 37.61ms/TAB for 8011 lines
//! (release build, mean of 20 runs each), scaling linearly with file
//! size and landing squarely in the keystroke path.
//!
//! A timing assertion here would be exactly the kind of thing this
//! project's own perf-test convention (see `search_perf_tests.rs`,
//! `dabbrev_perf_tests.rs`, etc.) deliberately keeps OUT of the normal
//! `cargo test` run -- every one of those is `#[ignore]`d precisely
//! because wall-clock thresholds are flaky across machines/load. This
//! file instead asserts the thing that actually makes the speedup real:
//! that a repeated, unedited query returns the SAME cached tree
//! (`Rc::ptr_eq`, exposed to Rust here directly rather than going
//! through `treesit-node-eq`'s elisp-level check) rather than a fresh
//! reparse -- deterministic, and it fails immediately (not just
//! "eventually slow") if the cache is ever removed or its invalidation
//! is loosened incorrectly. `measure_indent_for_tab_scaling_by_file_size`
//! at the bottom is the manual, `#[ignore]`d timing probe used to
//! reproduce the table above; run it explicitly:
//!
//!   cargo test --release -p core --test indent_perf_tests -- --ignored --nocapture
//!
//! M109 replaces the from-scratch reparse that ran on every actual edit
//! (the cache above only helps when nothing changed) with real
//! incremental reuse (see `treesit::derive_edit`'s module doc). Same
//! probe, same command, release build, mean of 20 runs each, this
//! machine, with the M108 column re-measured in a `git worktree` at the
//! previous commit using this same file:
//!
//!         before (M108)   after (M109)
//!   62 lines:   0.223 ms       0.018 ms
//!  512 lines:   1.614 ms       0.023 ms
//! 2012 lines:   6.172 ms       0.055 ms
//! 8012 lines:  27.408 ms       0.167 ms
//!
//! An earlier version of this table read 0.060 / 0.073 / 0.129 / 0.348.
//! Those came from a broken probe: `indent_one_tab_at_eof` removed its
//! scratch line with `(kill-whole-line)`, **a command this editor does
//! not have**, the error was swallowed, and the buffer's last line grew
//! by 17 characters on every one of the 20 timed rounds. See that
//! function's own comment.
//!
//! **Read that table for what it is: TAB at end of file.** Each round
//! inserts a line at `point-max`, presses TAB, and removes it, so the
//! edit tree-sitter has to absorb is always at the very end, where the
//! preceding thousands of lines are reused untouched. That is a real
//! incremental reparse -- it was checked, not assumed: TAB on that line
//! genuinely re-indents it from two spaces to zero (the line sits after
//! the closing `endmodule`, so it parses under an ERROR node and
//! `indent--block-depth` falls back to copying the previous line's
//! indentation). It is not the no-op-TAB path, and an earlier version of
//! this paragraph claimed it was.
//!
//! What the table does not show is how much the shape of the buffer
//! matters. Measured against a `git worktree` baseline at the previous
//! commit, release, per keystroke, single-character edits scattered
//! through real code: 21.420 ms -> 0.425 ms on 570 small SystemVerilog
//! modules, but only 38.527 ms -> 24.043 ms on one 8000-arm `case`, because
//! a single enormous node leaves tree-sitter almost nothing to reuse.
//! `measure_incremental_vs_full_on_two_buffer_shapes` below reproduces
//! both. The full table is in `PLAN.md`'s M109 record.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use core::treesit::{self, Lang};
use elisp::Interp;

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

/// A repeated verilog module body -- enough real block-nesting (module/
/// always/begin/end) to exercise the same code path
/// `indent--treesit-depth-column` walks for a real TAB press, not just
/// flat text.
fn make_verilog_module(lines_per_block: usize, blocks: usize) -> String {
    let mut s = String::from("module top;\n");
    for b in 0..blocks {
        s.push_str(&format!("  always @(posedge clk) begin : blk_{}\n", b));
        for l in 0..lines_per_block {
            s.push_str(&format!("    reg_{}_{} <= reg_{}_{} + 1;\n", b, l, b, l));
        }
        s.push_str("  end\n");
    }
    s.push_str("endmodule\n");
    s
}

/// Grab the current buffer's parser + parsed root node's `TsTreeData`
/// `Rc` pointer as a `usize` (its address), via the same public
/// `treesit::parse` entry point `treesit-parser-root-node` calls, so
/// this test observes exactly the same cache the real elisp-visible
/// path uses -- not a hand-rolled duplicate of it.
fn ts_tree_ptr(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, lang: Lang) -> usize {
    let buf = ed.borrow().current.clone();
    let parser = treesit::TsParserState {
        lang,
        buffer: Rc::downgrade(&buf),
    };
    let data = match treesit::parse(interp, &parser) {
        Ok(d) => d,
        Err(_) => panic!("parse must succeed"),
    };
    Rc::as_ptr(&data) as usize
}

/// The actual regression guard: two parses back-to-back with no edit in
/// between must be the literal same cached tree (same `Rc` allocation),
/// on a buffer large enough that "it happened to reparse into something
/// pointer-equal by coincidence" is not a concern (it never would be --
/// `tree_sitter::Parser::parse` allocates a fresh `Tree` every call --
/// but the size also makes this test double as a sanity check that the
/// cache path doesn't quietly early-return something wrong for big
/// buffers specifically).
#[test]
fn repeated_unedited_parse_hits_the_same_cached_tree() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    let src = make_verilog_module(20, 50);
    run(&mut i, &format!("(insert {:?})", src));

    let p1 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    let p2 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    let p3 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    assert_eq!(p1, p2, "second unedited parse must reuse the cached tree");
    assert_eq!(
        p2, p3,
        "third unedited parse must still reuse the same cached tree"
    );
}

/// The other half: an edit between two parses MUST produce a different
/// tree (an interior mutability slip that made the cache never
/// invalidate would pass the test above but silently hand back stale
/// trees forever after any edit -- this is the mutation target for
/// that failure mode).
#[test]
fn edit_between_parses_invalidates_the_cached_tree() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    let src = make_verilog_module(20, 50);
    run(&mut i, &format!("(insert {:?})", src));

    let p1 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"// a comment\\n\")");
    let p2 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    assert_ne!(
        p1, p2,
        "an edit between two parses must invalidate the cache"
    );
}

/// M109's only non-timing guard that incremental parsing is actually
/// running. If the dispatch in `treesit::parse` ever regressed to
/// taking the `parse_text` arm unconditionally, **every correctness
/// test in this workspace would stay green** -- a full parse produces a
/// correct tree, just a slow one. Before this test existed the
/// milestone's entire point had no guard at all; the mutation that
/// forces that arm is U7 in `dev/mutations/m109.py`, and it is this
/// test that kills it. (Do not confuse U7 with U6, which inverts
/// `incremental_parse`'s span-fallback check: that line is never even
/// reached for this test's input, because `derive_edit` returns `None`
/// and `incremental_parse` returns before it. U6 has no test that can
/// catch it, by construction -- see the note at that check.)
///
/// This is the observable that distinguishes the two without a clock.
/// An edit that cancels itself out bumps `edit_ticks` -- so the
/// generation-keyed cache from M108 MUST miss -- while leaving the text
/// byte-for-byte identical. On the incremental path `derive_edit`
/// returns `None` for identical text and `incremental_parse` hands back
/// `Rc::clone(old_data)`, so the pointer is unchanged. On the
/// full-parse path the same input produces a freshly allocated tree and
/// a different pointer. Confirmed against a `git worktree` checkout of
/// the previous commit (M108, before incremental parsing existed):
/// this test FAILS there and passes here.
///
/// The `edit_ticks` assertion below is load-bearing, not decoration. If
/// some future change stopped a cancelling edit pair from bumping the
/// generation, M108's cache would hit outright, the pointers would
/// match for a reason that has nothing to do with incremental parsing,
/// and this test would pass while guarding nothing.
///
/// This is also the mechanism behind the largest measured win in the
/// milestone: a TAB press re-indents by deleting and reinserting the
/// leading whitespace, so on an already-correctly-indented line it
/// bumps the generation without changing a byte. Measured on this
/// machine, 7980 lines of SystemVerilog, release, mean of 20:
/// **21.005 ms/TAB before, 0.076 ms/TAB after.** Nobody designed that;
/// it falls out of `derive_edit` reporting "no change". Reproducible:
/// the `no-op TAB` row printed by
/// `measure_incremental_vs_full_on_two_buffer_shapes`.
#[test]
fn an_edit_that_cancels_out_reuses_the_tree_without_reparsing() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    let src = make_verilog_module(20, 50);
    run(&mut i, &format!("(insert {:?})", src));

    let p1 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);

    let buf = ed.borrow().current.clone();
    let gen_before = buf.borrow().edit_ticks;
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"x\")");
    run(&mut i, "(delete-char -1)");
    let gen_after = buf.borrow().edit_ticks;
    assert!(
        gen_after > gen_before,
        "the edit pair must bump the generation, or this test proves \
         nothing: {} -> {}",
        gen_before,
        gen_after
    );

    let p2 = ts_tree_ptr(&mut i, &ed, Lang::Verilog);
    assert_eq!(
        p1, p2,
        "text is byte-identical, so the incremental path must reuse the \
         existing tree; a different pointer means the parse went through \
         `parse_text` and incremental parsing is no longer running"
    );
}

/// A real `indent-for-tab-command` keypress on an already-correctly-
/// indented line does a full `indent-line-to` regardless of whether the
/// column actually changes (see `indent-for-tab-command`'s own body in
/// indent.el) -- that always bumps `edit_ticks` even when the visible
/// text doesn't change (deleting and reinserting the same leading
/// whitespace still goes through `Buffer::delete`/`insert`).
///
/// **M109 changed what follows from that, and this comment used to say
/// the opposite.** Under M108 the generation bump alone forced a full
/// reparse, so two consecutive TAB presses reparsed twice; that is what
/// the original version of this comment documented. Now the bump still
/// misses the generation-keyed cache, but `derive_edit` sees identical
/// text, returns `None`, and the existing tree is shared -- so those two
/// presses reparse zero times. The behaviour this test's prose described
/// is gone; the test itself never asserted it (it only checks that
/// `edit_ticks` is monotonic, which both designs satisfy), which is
/// exactly why the stale prose survived the change. The shared-tree
/// property is asserted by
/// `an_edit_that_cancels_out_reuses_the_tree_without_reparsing` above.
#[test]
fn consecutive_tab_presses_on_different_lines_each_bump_the_generation() {
    let (mut i, ed) = setup();
    run(&mut i, "(verilog-mode)");
    let src = make_verilog_module(3, 3);
    run(&mut i, &format!("(insert {:?})", src));
    run(&mut i, "(goto-char (point-min))");

    let buf = ed.borrow().current.clone();
    let gen0 = buf.borrow().edit_ticks;
    feed(&mut i, &ed, "TAB");
    let gen1 = buf.borrow().edit_ticks;
    run(&mut i, "(forward-line 1)");
    feed(&mut i, &ed, "TAB");
    let gen2 = buf.borrow().edit_ticks;
    assert!(
        gen1 >= gen0,
        "a TAB press must not decrease edit_ticks: {} -> {}",
        gen0,
        gen1
    );
    assert!(
        gen2 >= gen1,
        "a second TAB press must not decrease edit_ticks: {} -> {}",
        gen1,
        gen2
    );
}

// ============================================================
// Manual timing probe -- reproduces the before/after table from the
// M108 spec. Deliberately `#[ignore]`d; not part of the normal gate.
// ============================================================

fn make_verilog_of_roughly(lines: usize) -> String {
    // ~4 lines of module scaffolding + 1 line per register per block +
    // 2 lines of always/end per block; pick blocks so total lines lands
    // close to the requested count.
    let blocks = (lines / 5).max(1);
    make_verilog_module(3, blocks)
}

/// Insert a line at end of buffer, TAB it, then remove it again so the
/// next call starts from the same buffer.
///
/// The removal used to be `(kill-whole-line)`, **which does not exist in
/// this editor** -- `run` turns an elisp error into a string and the
/// probe discarded it, so every call silently left its line behind and
/// the last line grew by 17 characters per iteration for all 20 timed
/// rounds. Measured 2026-09-05, three rounds printed side by side: the
/// line went `"  reg_extra <= 1;"` -> `"reg_extra <= 1;  reg_extra <= 1;"`
/// -> three copies. `kill-whole-line` being absent is a real gap in this
/// editor's command set, recorded as a lead in `PLAN.md`; it is not
/// fixed here, so this helper uses `delete-region` instead of depending
/// on it.
///
/// `expect_ok` exists for the same reason: a probe that quietly does
/// nothing produces numbers that look exactly like real ones.
fn expect_ok(interp: &mut Interp, form: &str) {
    let out = run(interp, form);
    assert!(
        !out.starts_with("ERROR:"),
        "{} failed inside the timing probe: {}",
        form,
        out
    );
}

fn indent_one_tab_at_eof(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    expect_ok(interp, "(goto-char (point-max))");
    expect_ok(interp, "(insert \"  reg_extra <= 1;\")");
    expect_ok(interp, "(beginning-of-line)");
    feed(interp, ed, "TAB");
    // The inserted text is the whole last line (the generated source ends
    // with "endmodule\n"), so this deletes exactly what was inserted --
    // whatever TAB did to its indentation.
    expect_ok(
        interp,
        "(delete-region (line-beginning-position) (point-max))",
    );
}

/// The corpora and the methodology behind the two-shape table in the
/// module doc, checked in so the numbers can be reproduced or refuted.
/// They were originally measured with a throwaway probe that lived only
/// in a scratch directory, which meant nobody else could check them --
/// this project is otherwise strict about performance claims, and a
/// number no reader can reproduce is not much better than no number.
///
/// This runs inside one binary, so it shows the *shape* dependence, not
/// the before/after. For the before/after, check out the commit before
/// M109 into a `git worktree`, copy this file in, and run the same
/// command in both trees -- confirming first, with
/// `nm target/release/deps/... | grep -c derive_edit`, that the two
/// binaries actually differ. A screenshot of that mistake (timing a
/// binary older than the change under test) cost five wrong hypotheses
/// elsewhere in this project.
///
///   cargo test --release -p core --test indent_perf_tests -- --ignored --nocapture
#[test]
#[ignore]
fn measure_incremental_vs_full_on_two_buffer_shapes() {
    // Realistic RTL: many small, self-contained modules.
    fn many_small_modules(n: usize) -> String {
        let mut s = String::new();
        for k in 0..n {
            s.push_str(&format!(
                "module blk_{k} #(parameter int W = 8) (\n    \
                 input  logic         clk_i,\n    \
                 input  logic         rst_ni,\n    \
                 input  logic [W-1:0] a_i,\n    \
                 output logic [W-1:0] y_o\n);\n  \
                 logic [W-1:0] r_q;\n  \
                 always_ff @(posedge clk_i or negedge rst_ni) begin\n    \
                 if (!rst_ni) r_q <= '0;\n    \
                 else         r_q <= a_i;\n  \
                 end\n  assign y_o = r_q;\nendmodule\n\n"
            ));
        }
        s
    }
    // Adversarial: one enormous construct, so an edit inside it leaves
    // tree-sitter almost nothing to reuse.
    fn one_huge_case(arms: usize) -> String {
        let mut s = String::from(
            "module big #(parameter int W = 32) (\n    input logic clk_i,\n    \
             input logic [W-1:0] a_i,\n    output logic [W-1:0] y_o\n);\n  \
             always_comb begin\n    unique case (a_i)\n",
        );
        for k in 0..arms {
            s.push_str(&format!("      {k}: y_o = a_i + {k};\n"));
        }
        s.push_str("      default: y_o = '0;\n    endcase\n  end\nendmodule\n");
        s
    }

    // The no-op TAB case, quoted in PLAN.md and in
    // `an_edit_that_cancels_out_reuses_the_tree_without_reparsing`'s
    // comment as the single largest number in the milestone. Pressing
    // TAB on a line that is already correctly indented rewrites the same
    // leading whitespace, so `edit_ticks` moves but not one byte does.
    {
        let src = many_small_modules(570);
        let (mut i, ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, &format!("(insert {:?})", src));
        run(&mut i, "(goto-char (point-min))");
        run(&mut i, "(forward-line 1400)");
        run(&mut i, "(end-of-line)");
        run(&mut i, "(indent-for-tab-command)");
        let n = 20;
        let start = std::time::Instant::now();
        for _ in 0..n {
            run(&mut i, "(indent-for-tab-command)");
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let _ = &ed;
        println!(
            "no-op TAB: 570 small modules   {:>5} lines {:>7} B  repeated TAB, text unchanged     {per:8.3} ms",
            src.lines().count(),
            src.len()
        );
    }

    for (label, src) in [
        ("realistic: 570 small modules ", many_small_modules(570)),
        ("adversarial: one 8000-arm case", one_huge_case(8000)),
    ] {
        let (mut i, ed) = setup();
        run(&mut i, "(verilog-mode)");
        run(&mut i, &format!("(insert {:?})", src));
        run(&mut i, "(goto-char (point-min))");
        run(&mut i, "(forward-line 100)");
        run(&mut i, "(end-of-line)");
        // Warm the cache: the first parse is always a full one.
        run(&mut i, "(indent-for-tab-command)");

        let n = 20;
        let start = std::time::Instant::now();
        for _ in 0..n {
            // A fresh line of real code every round, so the edit never
            // lands inside a run of whitespace that gets reused whole --
            // that pattern is what produced this milestone's first,
            // over-optimistic headline number.
            run(&mut i, "(forward-line 14)");
            run(&mut i, "(end-of-line)");
            run(&mut i, "(insert \"x\")");
            run(&mut i, "(indent-for-tab-command)");
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        let _ = &ed;
        println!(
            "{label}  {:>5} lines {:>7} B  scattered 1-char edit + reparse  {per:8.3} ms",
            src.lines().count(),
            src.len()
        );
    }
}

#[test]
#[ignore]
fn measure_indent_for_tab_scaling_by_file_size() {
    for target_lines in [61usize, 511, 2011, 8011] {
        let (mut i, ed) = setup();
        run(&mut i, "(verilog-mode)");
        let src = make_verilog_of_roughly(target_lines);
        let actual_lines = src.lines().count();
        let bytes = src.len();
        run(&mut i, &format!("(insert {:?})", src));

        // Warm-up (first parse always reparses -- exclude it from the
        // timed mean, matching the file's own doc comment claim of
        // "per-TAB" cost on an otherwise-unedited buffer).
        indent_one_tab_at_eof(&mut i, &ed);

        let n = 20;
        let start = std::time::Instant::now();
        for _ in 0..n {
            indent_one_tab_at_eof(&mut i, &ed);
        }
        let elapsed = start.elapsed();
        let per_tab = elapsed / n as u32;
        println!(
            "{:>6} lines ({:>8} bytes): {:>8.3} ms / TAB",
            actual_lines,
            bytes,
            per_tab.as_secs_f64() * 1000.0
        );
    }
}
