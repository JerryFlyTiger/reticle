//! M15 item 4: background syntax highlighting. The parse happens on a
//! real worker thread; tests drive the main-thread side by pumping
//! `core::idle_tick` until results land (bounded, ~ms in practice).

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(&mut interp, &format!("(insert {:?})", src));
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// Pump ticks until `pred` returns "t", or a 30s deadline passes.
///
/// The 30s ceiling is a hang guard, not a latency budget: under
/// contended load (measured by the main conversation at load average
/// ~14, same binary, same tree) a fixed ~2s budget went red — 17
/// pre-M142 tests were 4/4 green, but 31 tests with M142's additions
/// hit a round with 20 failures (19 of them `enable_and_wait`'s "no
/// treesit-hl overlays ever arrived"). Each test spins up its own
/// highlight worker thread (`crates/core/src/highlight.rs:117-126`),
/// and M142 roughly doubled how many run concurrently. Follows M135
/// Part B's pattern: poll the condition, bound only by a generous
/// ceiling.
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

const COUNT_HL: &str = "(let ((n 0))
   (dolist (ov (overlays-in (point-min) (point-max)) nil)
     (when (overlay-get ov 'treesit-hl) (setq n (1+ n))))
   n)";

#[test]
fn background_highlight_arrives_without_blocking() {
    let (mut i, _ed) = setup("fn add(a: i32, b: i32) -> i32 {\n    a + b // sum\n}\n");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    // Enabling costs nothing on the spot: no overlays yet, the parse is
    // on the worker thread.
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // The `fn` keyword (chars 0..2, elisp positions 1..3) must carry the
    // keyword face via a treesit-hl overlay.
    let has_kw = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 3) hit)
             (when (and (overlay-get ov 'treesit-hl)
                        (eq (overlay-get ov 'face) 'font-lock-keyword-face))
               (setq hit t))))",
    );
    assert_eq!(has_kw, "t");
    // And the comment got the comment face somewhere.
    let has_comment = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in (point-min) (point-max)) hit)
             (when (eq (overlay-get ov 'face) 'font-lock-comment-face)
               (setq hit t))))",
    );
    assert_eq!(has_comment, "t");
}

#[test]
fn edits_are_rehighlighted_after_the_debounce() {
    let (mut i, _ed) = setup("fn a() {}\n");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Insert a string literal at the head; its face must appear.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(insert \"const S: &str = \\\"hello\\\";\\n\")");
    assert!(tick_until(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 30) hit)
             (when (eq (overlay-get ov 'face) 'font-lock-string-face)
               (setq hit t))))"
    ));
}

#[test]
fn foreign_overlays_survive_rehighlighting() {
    let (mut i, _ed) = setup("fn a() {}\n");
    run(&mut i, "(setq mine (make-overlay 1 5))");
    run(&mut i, "(overlay-put mine 'my-marker t)");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // Force a re-apply cycle via an edit, then confirm ours is intact.
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn b() {}\\n\")");
    assert!(tick_until(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 10) hit)
             (when (overlay-get ov 'my-marker) (setq hit t))))"
    ));
}

#[test]
fn overlay_volume_is_bounded_to_the_visible_region() {
    // 1000 functions, far more than a 24-row window + margin can show.
    let mut src = String::new();
    for n in 0..1000 {
        src.push_str(&format!("fn func_{:04}() {{ let x = \"s\"; }}\n", n));
    }
    let total_chars = src.chars().count();
    let (mut i, _ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // Nothing materialized anywhere near the end of the buffer: the far
    // tail is outside visible + margin.
    let far_tail = run(
        &mut i,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {} (point-max)) hit)
                 (when (overlay-get ov 'treesit-hl) (setq hit t))))",
            total_chars - 5000
        ),
    );
    assert_eq!(far_tail, "nil", "far tail should not be materialized");
    // But the visible head is.
    let head = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 100) hit)
             (when (overlay-get ov 'treesit-hl) (setq hit t))))",
    );
    assert_eq!(head, "t");
}

#[test]
fn typing_latency_stays_low_while_highlighting_a_large_file() {
    // The headline guarantee, measured: with background highlighting
    // enabled on a substantial buffer, a keystroke (which triggers
    // reparse scheduling, overlay adjustment, redisplay bookkeeping)
    // stays fast — the parse itself is on the other thread.
    let mut src = String::new();
    for n in 0..2000 {
        src.push_str(&format!(
            "fn func_{:04}(x: i32) -> i32 {{ x + {} }}\n",
            n, n
        ));
    }
    let (mut i, ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let start = std::time::Instant::now();
    for _ in 0..30 {
        core::commands::feed_keys(&mut i, &ed, "x").unwrap();
        core::idle_tick(&mut i, std::time::Duration::ZERO);
    }
    let elapsed = start.elapsed();
    let per_key = elapsed / 30;
    eprintln!(
        "30 keystrokes over a {}-char highlighted buffer: {:?} total, {:?}/key",
        src.chars().count(),
        elapsed,
        per_key
    );
    assert!(
        per_key < std::time::Duration::from_millis(50),
        "keystroke cost {:?} — background highlighting is leaking onto the keystroke path",
        per_key
    );
}

/// M15 item 5 probe (run deliberately: `cargo test --release -p core
/// --test highlight_tests -- --ignored --nocapture`): full-grid render()
/// cost at a large window size over a highlighted buffer, to decide
/// whether incremental redisplay is worth building. Recorded in PLAN.md.
#[test]
#[ignore]
fn measure_full_redisplay_cost() {
    let mut src = String::new();
    for n in 0..2000 {
        src.push_str(&format!(
            "fn func_{:04}(x: i32) -> i32 {{ x + {} }}\n",
            n, n
        ));
    }
    let (mut i, ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    for &(cols, rows) in &[(80usize, 24usize), (200, 60), (300, 100)] {
        ed.borrow_mut().frame = (cols, rows);
        core::idle_tick(&mut i, std::time::Duration::ZERO);
        let n = 200;
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = core::redisplay::render(&i, &ed);
        }
        let per = start.elapsed() / n;
        eprintln!("render() at {}x{}: {:?} per frame", cols, rows, per);
    }
}

// -----------------------------------------------------------------------
// M121: `verilog-highlights.scm` faced no SVA keyword (`assert`/
// `property`/`cover`/`assume`/`disable`), no `interface`/`endinterface`/
// `modport`/`class`/`endclass`/`covergroup`/`endgroup`/`coverpoint`/
// `program`/`endprogram`/`new`, no modport's own declared name, and no
// coverpoint label. Helpers below mirror
// `font_lock_philosophy_tests.rs`'s `enable_and_wait`/`nth_token`/
// `exact_hl`/`face_at` (duplicated per file, this test suite's existing
// convention, rather than shared).
//
// **False-positive guard**: none of the fixtures below carry a comment
// that repeats a construct's name, so occurrence 0 of every needle is
// the real token, never comment text (the trap `font_lock_philosophy_
// tests.rs`'s own history records: a fixture's doc comment repeating a
// construct name made "the first occurrence" land on a comment).
// -----------------------------------------------------------------------

/// Enable highlighting for LANG and block until at least one
/// `treesit-hl` overlay has landed. Mirrors `font_lock_philosophy_
/// tests.rs`'s helper of the same name.
fn enable_and_wait(interp: &mut Interp, lang: &str) {
    run(interp, &format!("(treesit-highlight-mode '{lang})"));
    assert!(
        tick_until(interp, &format!("(> {COUNT_HL} 0)")),
        "{lang}: no treesit-hl overlays ever arrived"
    );
}

/// Byte range of NEEDLE's `occurrence`-th (0-based) appearance in SRC as
/// a *whole token*. See `font_lock_philosophy_tests.rs`'s copy of this
/// helper for the full rationale.
fn nth_token(src: &str, needle: &str, occurrence: usize) -> (usize, usize) {
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '-'
    }
    let mut seen = 0;
    let mut search_from = 0;
    loop {
        let rel = src[search_from..].find(needle).unwrap_or_else(|| {
            panic!(
                "{needle:?} occurrence {occurrence} not found as a whole token \
                 (matched {seen} time(s) before giving up, searching from byte {search_from})"
            )
        });
        let start = search_from + rel;
        let end = start + needle.len();
        let before_ok = src[..start]
            .chars()
            .next_back()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
        let after_ok = src[end..]
            .chars()
            .next()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
        search_from = start + 1;
        if before_ok && after_ok {
            if seen == occurrence {
                let start_ch = src[..start].chars().count() + 1; // 1-based, like point-min
                let end_ch = start_ch + needle.chars().count();
                return (start_ch, end_ch);
            }
            seen += 1;
        }
    }
}

/// Whether a `treesit-hl` overlay with *exactly* `[start_ch, end_ch)`
/// bounds and FACE exists.
fn exact_hl(interp: &mut Interp, start_ch: usize, end_ch: usize, face: &str) -> bool {
    run(
        interp,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {start_ch} {end_ch}) hit)
                 (when (and (overlay-get ov 'treesit-hl)
                            (eq (overlay-get ov 'face) '{face})
                            (= (overlay-start ov) {start_ch})
                            (= (overlay-end ov) {end_ch}))
                   (setq hit t))))"
        ),
    ) == "t"
}

/// NEEDLE's `occurrence`-th whole-token appearance in SRC carries FACE,
/// with exactly NEEDLE's own span as the overlay bounds.
fn face_at(interp: &mut Interp, src: &str, needle: &str, occurrence: usize, face: &str) -> bool {
    let (s, e) = nth_token(src, needle, occurrence);
    exact_hl(interp, s, e, face)
}

const SVA_SRC: &str = "module sva_demo;
  logic clk;
  logic a;
  logic b;
  assert property (@(posedge clk) a |-> b);
  cover property (@(posedge clk) a);
  assume property (@(posedge clk) a);
  initial begin : blk
    disable blk;
  end
endmodule
";

#[test]
fn verilog_sva_keywords_get_keyword_face() {
    let (mut i, _ed) = setup(SVA_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, SVA_SRC, "assert", 0, "font-lock-keyword-face"),
        "verilog: `assert` in `assert property (...)` must get keyword face \
         -- delete the `\"assert\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, SVA_SRC, "property", 0, "font-lock-keyword-face"),
        "verilog: `property` in `assert property (...)` must get keyword face \
         -- delete the `\"property\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, SVA_SRC, "cover", 0, "font-lock-keyword-face"),
        "verilog: `cover` in `cover property (...)` must get keyword face \
         -- delete the `\"cover\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, SVA_SRC, "assume", 0, "font-lock-keyword-face"),
        "verilog: `assume` in `assume property (...)` must get keyword face \
         -- delete the `\"assume\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, SVA_SRC, "disable", 0, "font-lock-keyword-face"),
        "verilog: `disable blk;` must get keyword face \
         -- delete the `\"disable\" @keyword` rule and this goes red"
    );
}

const SVA_IFF_SRC: &str = "module sva_iff_demo;
  logic clk;
  logic rst;
  logic a;
  assert property (@(posedge clk) disable iff (rst) a);
endmodule
";

#[test]
fn verilog_sva_iff_gets_keyword_face() {
    let (mut i, _ed) = setup(SVA_IFF_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, SVA_IFF_SRC, "iff", 0, "font-lock-keyword-face"),
        "verilog: `disable iff (rst)`'s `iff` must get keyword face \
         -- delete the `\"iff\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, SVA_IFF_SRC, "disable", 0, "font-lock-keyword-face"),
        "verilog: `disable iff (rst)`'s `disable` must get keyword face"
    );
}

const INTERFACE_SRC: &str = "interface my_if;
  logic clk;
  modport mst(input clk);
endinterface
";

#[test]
fn verilog_interface_modport_keywords_and_modport_name() {
    let (mut i, _ed) = setup(INTERFACE_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(
            &mut i,
            INTERFACE_SRC,
            "interface",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `interface my_if;`'s `interface` keyword must get keyword face \
         -- delete the `\"interface\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            INTERFACE_SRC,
            "endinterface",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `endinterface` must get keyword face \
         -- delete the `\"endinterface\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            INTERFACE_SRC,
            "modport",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `modport mst(...)`'s `modport` keyword must get keyword face \
         -- delete the `\"modport\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, INTERFACE_SRC, "mst", 0, "font-lock-constant-face"),
        "verilog: `modport mst(...)`'s own declared name must get constant face \
         (matching the reference-site face M97 already chose) \
         -- delete the `(modport_item . (simple_identifier) @constant)` rule \
         and this goes red"
    );
}

const CLASS_SRC: &str = "class my_cls;
  int x;
  function new();
    x = 0;
  endfunction
endclass
";

#[test]
fn verilog_class_keywords_and_constructor_new() {
    let (mut i, _ed) = setup(CLASS_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, CLASS_SRC, "class", 0, "font-lock-keyword-face"),
        "verilog: `class my_cls;`'s `class` keyword must get keyword face \
         -- delete the `\"class\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, CLASS_SRC, "endclass", 0, "font-lock-keyword-face"),
        "verilog: `endclass` must get keyword face \
         -- delete the `\"endclass\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, CLASS_SRC, "new", 0, "font-lock-keyword-face"),
        "verilog: the constructor keyword `new` in `function new();` must get \
         keyword face -- delete the `\"new\" @keyword` rule and this goes red"
    );
}

const COVERGROUP_SRC: &str = "module cg_demo;
  logic x;
  covergroup my_cg;
    cp_data: coverpoint x;
  endgroup
endmodule
";

#[test]
fn verilog_covergroup_keywords_and_coverpoint_label() {
    let (mut i, _ed) = setup(COVERGROUP_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(
            &mut i,
            COVERGROUP_SRC,
            "covergroup",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `covergroup my_cg;`'s `covergroup` keyword must get keyword \
         face -- delete the `\"covergroup\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            COVERGROUP_SRC,
            "endgroup",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `endgroup` must get keyword face \
         -- delete the `\"endgroup\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            COVERGROUP_SRC,
            "coverpoint",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `coverpoint x;`'s `coverpoint` keyword must get keyword face \
         -- delete the `\"coverpoint\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            COVERGROUP_SRC,
            "cp_data",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `cp_data: coverpoint x;`'s own label must get variable face \
         -- delete the `(cover_point name: (simple_identifier) @variable)` \
         rule and this goes red"
    );
}

const PROGRAM_SRC: &str = "program my_prog;
  initial $display(\"hi\");
endprogram
";

#[test]
fn verilog_program_keywords_get_keyword_face() {
    let (mut i, _ed) = setup(PROGRAM_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, PROGRAM_SRC, "program", 0, "font-lock-keyword-face"),
        "verilog: `program my_prog;`'s `program` keyword must get keyword \
         face -- delete the `\"program\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            PROGRAM_SRC,
            "endprogram",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `endprogram` must get keyword face \
         -- delete the `\"endprogram\" @keyword` rule and this goes red"
    );
}

// -----------------------------------------------------------------
// M122: the constructs above were only ever tested against invented
// snippets because `demo/rtl/` had no real material for them. M122
// added that material; these tests re-run the same face assertions
// against the real files instead of a hand-typed fixture.
// -----------------------------------------------------------------

fn read_demo(rel: &str) -> String {
    let path =
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{:?}: {}", path, e))
}

#[test]
fn verilog_interface_modport_and_sva_in_real_axi4_lite_if() {
    let src = read_demo("rtl/bus/axi4_lite_if.sv");
    let (mut i, _ed) = setup(&src);
    enable_and_wait(&mut i, "verilog");
    // occurrence 2: the file's own two-line header comment says
    // "SystemVerilog interface" and "bundled interface" before the
    // real `interface axi4_lite_if #(` declaration.
    assert!(
        face_at(&mut i, &src, "interface", 2, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `interface axi4_lite_if #(`'s keyword must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "endinterface", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `endinterface : axi4_lite_if` must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "modport", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `modport master(...)`'s keyword must get keyword face"
    );
    // occurrence 1: occurrence 0 is the header comment's
    // "`master`/`slave`/`monitor` modports".
    assert!(
        face_at(&mut i, &src, "master", 1, "font-lock-constant-face"),
        "axi4_lite_if.sv: `modport master(...)`'s own declared name must get \
         constant face"
    );
    assert!(
        face_at(&mut i, &src, "slave", 2, "font-lock-constant-face"),
        "axi4_lite_if.sv: `modport slave(...)`'s own declared name must get \
         constant face"
    );
    assert!(
        face_at(&mut i, &src, "monitor", 1, "font-lock-constant-face"),
        "axi4_lite_if.sv: `modport monitor(...)`'s own declared name must get \
         constant face"
    );
    assert!(
        face_at(&mut i, &src, "assert", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `a_aw_stable: assert property (...)`'s `assert` must \
         get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "property", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `property p_aw_stable;`'s keyword must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "disable", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `disable iff (!rst_ni)`'s `disable` must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "iff", 0, "font-lock-keyword-face"),
        "axi4_lite_if.sv: `disable iff (!rst_ni)`'s `iff` must get keyword face"
    );
}

#[test]
fn verilog_always_latch_in_real_clk_gate() {
    let src = read_demo("rtl/core/clk_gate.sv");
    let (mut i, _ed) = setup(&src);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, &src, "always_latch", 0, "font-lock-keyword-face"),
        "clk_gate.sv: `always_latch begin` must get keyword face \
         -- delete the `always_keyword` handling and this goes red"
    );
}

#[test]
fn verilog_genvar_in_real_sram_bank() {
    let src = read_demo("rtl/mem/sram_bank.sv");
    let (mut i, _ed) = setup(&src);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, &src, "genvar", 0, "font-lock-keyword-face"),
        "sram_bank.sv: `for (genvar b = 0; ...)`'s `genvar` must get keyword face \
         -- delete the `\"genvar\" @keyword` rule and this goes red"
    );
}

#[test]
fn verilog_interface_port_header_in_real_axi4_lite_monitor() {
    let src = read_demo("verif/axi4_lite_monitor.sv");
    let (mut i, _ed) = setup(&src);
    enable_and_wait(&mut i, "verilog");
    // occurrence 2: occurrences 0 and 1 are the header comment's
    // "binding onto an axi4_lite_if instance" and
    // "interface-typed (`axi4_lite_if.monitor bus`)".
    assert!(
        face_at(&mut i, &src, "axi4_lite_if", 2, "font-lock-type-face"),
        "axi4_lite_monitor.sv: `axi4_lite_if.monitor bus`'s interface name \
         must get type face -- delete the \
         `(interface_port_header interface_name: ...)` rule and this goes red"
    );
    // occurrence 1: occurrence 0 is the header comment's own use of
    // the word inside the same sentence.
    assert!(
        face_at(&mut i, &src, "monitor", 1, "font-lock-constant-face"),
        "axi4_lite_monitor.sv: `axi4_lite_if.monitor bus`'s modport name must \
         get constant face -- delete the \
         `(interface_port_header modport_name: ...)` rule and this goes red"
    );
}

#[test]
fn verilog_class_constructor_in_real_soc_verif_pkg() {
    let src = read_demo("verif/soc_verif_pkg.sv");
    let (mut i, _ed) = setup(&src);
    enable_and_wait(&mut i, "verilog");
    // occurrence 3: occurrences 0-2 are the header comment's three
    // uses of the word "class".
    assert!(
        face_at(&mut i, &src, "class", 3, "font-lock-keyword-face"),
        "soc_verif_pkg.sv: `class rw_checker;`'s keyword must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "endclass", 0, "font-lock-keyword-face"),
        "soc_verif_pkg.sv: `endclass : rw_checker` must get keyword face"
    );
    assert!(
        face_at(&mut i, &src, "new", 0, "font-lock-keyword-face"),
        "soc_verif_pkg.sv: the constructor keyword `new` in `function new();` \
         must get keyword face"
    );
}

/// This file keeps growing across M122 review rounds, so relying on
/// the default `window_start' of 0 to bring an early-in-the-file
/// construct into the visible+margin window (see
/// `overlay_volume_is_bounded_to_visible_region' above) is fragile --
/// it already broke once when the file crossed a length threshold.
/// Move `window_start' explicitly to POS and wait for overlays to
/// materialize there instead of assuming any particular line is
/// "close enough" to the top.
fn scroll_to_and_wait(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, pos: usize) {
    {
        let mut e = ed.borrow_mut();
        let sel = e.selected_window;
        e.windows.get_mut(&sel).unwrap().window_start = pos;
    }
    assert!(
        tick_until(
            interp,
            &format!(
                "(let (hit)
               (dolist (ov (overlays-in {pos} (+ {pos} 400)) hit)
                 (when (overlay-get ov 'treesit-hl) (setq hit t))))"
            )
        ),
        "no treesit-hl overlays materialized near byte {pos} after moving window_start there"
    );
}

#[test]
fn verilog_program_and_covergroup_in_real_sram_bank_tb() {
    let src = read_demo("verif/sram_bank_tb.sv");
    let (mut i, ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    enable_and_wait(&mut i, "verilog");

    let program_pos = src
        .find("program automatic axi_driver")
        .expect("program not found");
    scroll_to_and_wait(&mut i, &ed, program_pos);
    assert!(
        face_at(&mut i, &src, "program", 1, "font-lock-keyword-face"),
        "sram_bank_tb.sv: `program automatic axi_driver #(...)`'s keyword \
         must get keyword face"
    );

    let endprogram_pos = src.find("endprogram").expect("endprogram not found");
    scroll_to_and_wait(&mut i, &ed, endprogram_pos);
    assert!(
        face_at(&mut i, &src, "endprogram", 0, "font-lock-keyword-face"),
        "sram_bank_tb.sv: `endprogram : axi_driver` must get keyword face"
    );

    let covergroup_pos = src
        .find("covergroup cg_axi_bank")
        .expect("covergroup not found");
    scroll_to_and_wait(&mut i, &ed, covergroup_pos);
    assert!(
        face_at(&mut i, &src, "covergroup", 0, "font-lock-keyword-face"),
        "sram_bank_tb.sv: `covergroup cg_axi_bank @(...)`'s keyword must get \
         keyword face"
    );
    assert!(
        face_at(&mut i, &src, "endgroup", 0, "font-lock-keyword-face"),
        "sram_bank_tb.sv: `endgroup : cg_axi_bank` must get keyword face"
    );
    assert!(
        // occurrence 1: occurrence 0 is a nearby comment's own use of
        // the word ("the coverpoint below never needs to wrap").
        face_at(&mut i, &src, "coverpoint", 1, "font-lock-keyword-face"),
        "sram_bank_tb.sv: `cp_wr_bank: coverpoint ...`'s keyword must get \
         keyword face"
    );
}

// -----------------------------------------------------------------------
// M142: complete keyword highlighting for Verilog/SystemVerilog.
//
// A census of the grammar's Annex B reserved-word list found 171 unfaced
// words, plus a real defect: non-ANSI `input`/`output`/`inout` (the
// pre-M142 header wrongly claimed these always went through the
// `port_direction` wrapper -- see the corrected header in
// `verilog-highlights.scm`). This block adds named tests for each new
// rule class, plus a structural sweep guard over real corpus files that
// makes the milestone complete rather than a longer ad hoc list.
// -----------------------------------------------------------------------

/// NEEDLE's `occurrence`-th whole-token appearance in SRC carries no
/// `treesit-hl` overlay at all (checked as "no exact-bounds overlay",
/// which still allows a *wider* enclosing overlay to exist -- mirrors
/// `font_lock_philosophy_tests.rs`'s helper of the same name, redefined
/// here rather than shared because this file is the only one allowed to
/// change for M142).
fn not_colored_at(interp: &mut Interp, src: &str, needle: &str, occurrence: usize) -> bool {
    let (s, e) = nth_token(src, needle, occurrence);
    !exact_hl(interp, s, e, "font-lock-keyword-face")
        && !exact_hl(interp, s, e, "font-lock-type-face")
}

const NONANSI_PORTS_SRC: &str = "module m142_ports(a, b, c);
  input a;
  output b;
  inout c;
  defparam u1.WIDTH = 4;
  task t(ref int rv);
    rv = 0;
  endtask
endmodule
";

#[test]
fn verilog_nonansi_ports_defparam_ref_get_keyword_face() {
    let (mut i, _ed) = setup(NONANSI_PORTS_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(
            &mut i,
            NONANSI_PORTS_SRC,
            "input",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: non-ANSI `input a;`'s bare `input` token must get keyword \
         face -- delete the bare `\"input\" @keyword` rule and this goes red \
         (the ANSI-only `(port_direction) @keyword` rule cannot reach a \
         non-ANSI `input_declaration`, which holds no `port_direction` node)"
    );
    assert!(
        face_at(
            &mut i,
            NONANSI_PORTS_SRC,
            "output",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: non-ANSI `output b;`'s bare `output` token must get keyword \
         face -- delete the bare `\"output\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            NONANSI_PORTS_SRC,
            "inout",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: non-ANSI `inout c;`'s bare `inout` token must get keyword \
         face -- delete the bare `\"inout\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            NONANSI_PORTS_SRC,
            "defparam",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `defparam u1.WIDTH = 4;` must get keyword face \
         -- delete the `\"defparam\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            NONANSI_PORTS_SRC,
            "ref",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `task t(ref int rv);`'s `ref` must get keyword face \
         -- delete the `\"ref\" @keyword` rule and this goes red"
    );
}

const CONST_REF_SRC: &str = "module m142_const_ref;
  task t(const ref int rv);
    $display(rv);
  endtask
endmodule
";

#[test]
fn verilog_const_ref_gets_exact_span_keyword_faces() {
    let (mut i, _ed) = setup(CONST_REF_SRC);
    enable_and_wait(&mut i, "verilog");
    // Exact spans: a whole-"const ref" overlay (the old
    // `(tf_port_direction) @keyword` wrapper's span) would fail both of
    // these, since neither `const`'s nor `ref`'s own token bounds would
    // then carry a *matching* overlay.
    assert!(
        face_at(&mut i, CONST_REF_SRC, "const", 0, "font-lock-keyword-face"),
        "verilog: `task t(const ref int rv);`'s `const` must get keyword \
         face, spanning only `const` -- delete the `\"const\" @keyword` \
         rule and this goes red"
    );
    assert!(
        face_at(&mut i, CONST_REF_SRC, "ref", 0, "font-lock-keyword-face"),
        "verilog: `task t(const ref int rv);`'s `ref` must get keyword \
         face, spanning only `ref` -- delete the `\"ref\" @keyword` rule \
         and this goes red"
    );
    // No overlay may span the whole `const ref` -- that would be the old
    // `(tf_port_direction) @keyword` wrapper back, which both assertions
    // above pass right alongside (since neither checks for the ABSENCE
    // of a wider overlay).
    let (const_start, _) = nth_token(CONST_REF_SRC, "const", 0);
    let (_, ref_end) = nth_token(CONST_REF_SRC, "ref", 0);
    let no_wrapper = run(
        &mut i,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {const_start} {ref_end}) hit)
                 (when (and (overlay-get ov 'treesit-hl)
                            (<= (overlay-start ov) {const_start})
                            (>= (overlay-end ov) {ref_end}))
                   (setq hit t))))"
        ),
    );
    assert_eq!(
        no_wrapper, "nil",
        "verilog: no `treesit-hl` overlay may span both `const` and `ref` \
         together -- re-adding `(tf_port_direction) @keyword` would put \
         this back and should turn this test red"
    );
}

const TYPEDEF_SRC: &str = "module m142_types;
  typedef enum logic [1:0] {S_A, S_B} state_e;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  typedef union packed { logic [7:0] u8; logic [7:0] u8b; } byte_u;
  logic signed [7:0] sdata;
  parameter int unsigned WIDTH = 8;
endmodule
";

#[test]
fn verilog_typedef_enum_struct_union_signed_unsigned_get_type_face() {
    let (mut i, _ed) = setup(TYPEDEF_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "enum", 0, "font-lock-type-face"),
        "verilog: `typedef enum ...` must get type face \
         -- delete the `\"enum\" @type` rule and this goes red"
    );
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "struct", 0, "font-lock-type-face"),
        "verilog: `typedef struct packed ...` must get type face \
         -- delete the `\"struct\" @type` rule and this goes red"
    );
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "packed", 0, "font-lock-type-face"),
        "verilog: `struct packed` must get type face \
         -- delete the `\"packed\" @type` rule and this goes red"
    );
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "union", 0, "font-lock-type-face"),
        "verilog: `typedef union packed ...` must get type face \
         -- delete the `\"union\" @type` rule and this goes red"
    );
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "signed", 0, "font-lock-type-face"),
        "verilog: `logic signed [7:0] sdata;` must get type face \
         -- delete the `\"signed\" @type` rule and this goes red"
    );
    assert!(
        face_at(&mut i, TYPEDEF_SRC, "unsigned", 0, "font-lock-type-face"),
        "verilog: `parameter int unsigned WIDTH` must get type face \
         -- delete the `\"unsigned\" @type` rule and this goes red"
    );
}

const PKG_SRC: &str = "package m142_pkg2;
endpackage
package m142_pkg;
  import m142_pkg2::*;
  export m142_pkg2::*;
endpackage
";

#[test]
fn verilog_package_import_export_endpackage_get_keyword_face() {
    let (mut i, _ed) = setup(PKG_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, PKG_SRC, "package", 0, "font-lock-keyword-face"),
        "verilog: `package m142_pkg2;` must get keyword face \
         -- delete the `\"package\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, PKG_SRC, "endpackage", 0, "font-lock-keyword-face"),
        "verilog: `endpackage` must get keyword face \
         -- delete the `\"endpackage\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, PKG_SRC, "import", 0, "font-lock-keyword-face"),
        "verilog: `import m142_pkg2::*;` must get keyword face \
         -- delete the `\"import\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, PKG_SRC, "export", 0, "font-lock-keyword-face"),
        "verilog: `export m142_pkg2::*;` must get keyword face \
         -- delete the `\"export\" @keyword` rule and this goes red"
    );
}

const PROP_SEQ_SRC: &str = "module m142_prop_seq;
  logic clk, a, b;
  sequence sq1;
    a ##1 b;
  endsequence
  property p1;
    @(posedge clk) a |-> b;
  endproperty
endmodule
";

#[test]
fn verilog_property_sequence_closers_get_keyword_face() {
    let (mut i, _ed) = setup(PROP_SEQ_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(
            &mut i,
            PROP_SEQ_SRC,
            "sequence",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `sequence sq1;` must get keyword face \
         -- delete the `\"sequence\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            PROP_SEQ_SRC,
            "endsequence",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `endsequence` must get keyword face \
         -- delete the `\"endsequence\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            PROP_SEQ_SRC,
            "property",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `property p1;` must get keyword face (pre-existing M121 \
         rule, not touched by M142)"
    );
    assert!(
        face_at(
            &mut i,
            PROP_SEQ_SRC,
            "endproperty",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `endproperty` must get keyword face \
         -- delete the `\"endproperty\" @keyword` rule and this goes red"
    );
}

const CASE_SRC: &str = "module m142_case;
  logic [1:0] i;
  logic x;
  always_comb begin
    unique case (i)
      2'd0: x = 1;
      default: x = 0;
    endcase
    priority if (i == 0)
      x = 1;
    else
      x = 0;
    unique0 case (i)
      2'd0: x = 1;
      default: x = 0;
    endcase
  end
endmodule
";

#[test]
fn verilog_unique_priority_unique0_get_keyword_face() {
    let (mut i, _ed) = setup(CASE_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, CASE_SRC, "unique", 0, "font-lock-keyword-face"),
        "verilog: `unique case (...)` must get keyword face \
         -- delete the `(unique_priority) @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, CASE_SRC, "priority", 0, "font-lock-keyword-face"),
        "verilog: `priority if (...)` must get keyword face \
         -- delete the `(unique_priority) @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, CASE_SRC, "unique0", 0, "font-lock-keyword-face"),
        "verilog: `unique0 case (...)` must get keyword face \
         -- delete the `(unique_priority) @keyword` rule and this goes red"
    );
}

const CLASS_QUAL_SRC: &str = "class m142_base;
endclass
class m142_cls extends m142_base;
  protected int z;
  local int w;
  static int s;
  virtual function void vf();
  endfunction
endclass
";

#[test]
fn verilog_class_qualifiers_get_keyword_face() {
    let (mut i, _ed) = setup(CLASS_QUAL_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(
            &mut i,
            CLASS_QUAL_SRC,
            "extends",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `class m142_cls extends m142_base;` must get keyword face \
         -- delete the `\"extends\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            CLASS_QUAL_SRC,
            "protected",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `protected int z;` must get keyword face \
         -- delete the `\"protected\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(&mut i, CLASS_QUAL_SRC, "local", 0, "font-lock-keyword-face"),
        "verilog: `local int w;` must get keyword face \
         -- delete the `\"local\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            CLASS_QUAL_SRC,
            "static",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `static int s;` (class property) must get keyword face \
         -- delete the `\"static\" @keyword` rule and this goes red"
    );
    assert!(
        face_at(
            &mut i,
            CLASS_QUAL_SRC,
            "virtual",
            0,
            "font-lock-keyword-face"
        ),
        "verilog: `virtual function void vf();` must get keyword face \
         -- delete the `\"virtual\" @keyword` rule and this goes red"
    );
}

const BINS_SRC: &str = "module m142_bins;
  logic [1:0] x;
  covergroup m142_cg;
    coverpoint x {
      bins b0 = {0};
    }
  endgroup
endmodule
";

#[test]
fn verilog_bins_gets_keyword_face() {
    let (mut i, _ed) = setup(BINS_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, BINS_SRC, "bins", 0, "font-lock-keyword-face"),
        "verilog: `bins b0 = {{0}};` must get keyword face \
         -- delete the `\"bins\" @keyword` rule and this goes red"
    );
}

const GATE_SRC: &str = "module m142_gate;
  wire y, a, b;
  and g1(y, a, b);
endmodule
";

#[test]
fn verilog_gate_instantiation_type_gets_type_face() {
    let (mut i, _ed) = setup(GATE_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, GATE_SRC, "and", 0, "font-lock-type-face"),
        "verilog: `and g1(y, a, b);`'s gate-type keyword must get type face \
         -- delete the `(n_input_gatetype) @type` rule and this goes red"
    );
}

const SENS_OR_SRC: &str = "module m142_sens_or;
  logic clk, rst;
  always @(posedge clk or negedge rst) begin
  end
endmodule
";

#[test]
fn verilog_sensitivity_or_stays_plain() {
    let (mut i, _ed) = setup(SENS_OR_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        not_colored_at(&mut i, SENS_OR_SRC, "or", 0),
        "verilog: the sensitivity-list `or` in `@(posedge clk or negedge \
         rst)` must stay plain (an operator/connective, not a structural \
         keyword) -- if a rule starts capturing bare `\"or\"`, this goes red"
    );
}

const PROP_AND_SRC: &str = "module m142_prop_and;
  logic a, b;
  property p1;
    a and b s_until_with b;
  endproperty
endmodule
";

#[test]
fn verilog_property_and_stays_plain() {
    let (mut i, _ed) = setup(PROP_AND_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        not_colored_at(&mut i, PROP_AND_SRC, "and", 0),
        "verilog: the `and` connecting two `property_expr`s in `a and b;` \
         must stay plain (an operator, not the gate-type keyword) -- if a \
         rule starts capturing bare `\"and\"`, this goes red"
    );
    assert!(
        not_colored_at(&mut i, PROP_AND_SRC, "s_until_with", 0),
        "verilog: `s_until_with` connecting two `property_expr`s must stay \
         plain, matching its siblings `until`/`s_until`/`until_with` -- if a \
         rule starts capturing bare `\"s_until_with\"`, this goes red"
    );
}

const SUPER_SRC: &str = "class m142_super_base;
endclass
class m142_super_cls extends m142_super_base;
  function new();
    super.new();
  endfunction
endclass
";

#[test]
fn verilog_super_gets_keyword_face() {
    let (mut i, _ed) = setup(SUPER_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, SUPER_SRC, "super", 0, "font-lock-keyword-face"),
        "verilog: `super.new();` must get keyword face \
         -- delete the `\"super\" @keyword` rule and this goes red"
    );
}

const WEAK_SRC: &str = "module m142_weak;
  logic clk, a, b;
  property p1;
    weak(a ##1 b);
  endproperty
endmodule
";

#[test]
fn verilog_weak_gets_keyword_face() {
    let (mut i, _ed) = setup(WEAK_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        face_at(&mut i, WEAK_SRC, "weak", 0, "font-lock-keyword-face"),
        "verilog: `weak(a ##1 b);` must get keyword face \
         -- delete the `\"weak\" @keyword` rule and this goes red"
    );
}

const WITH_DIST_SRC: &str = "module m142_with_dist;
  logic [7:0] x;
  class m142_with_dist_cls;
    rand int r;
    constraint c { x dist {0 := 1, 1 := 1}; }
    function void go();
      void'(this.randomize() with { r > 0; });
    endfunction
  endclass
endmodule
";

#[test]
fn verilog_with_and_dist_stay_plain() {
    let (mut i, _ed) = setup(WITH_DIST_SRC);
    enable_and_wait(&mut i, "verilog");
    assert!(
        not_colored_at(&mut i, WITH_DIST_SRC, "dist", 0),
        "verilog: `x dist {{...}};`'s `dist` must stay plain (a constraint \
         connective, not a structural keyword) -- if a rule starts \
         capturing bare `\"dist\"`, this goes red"
    );
    assert!(
        not_colored_at(&mut i, WITH_DIST_SRC, "with", 0),
        "verilog: `randomize() with {{...}}`'s `with` must stay plain (an \
         inline-constraint connective, not a structural keyword) -- if a \
         rule starts capturing bare `\"with\"`, this goes red"
    );
}
const M142_FIXTURE_SRC: &str = r#"module m142_fixture(a, b, c);
  input a;
  output b;
  inout c;
  wire y, z;
  logic clk, rst_n, req, gnt;
  logic [7:0] data;
  logic signed [7:0] sdata;
  parameter int unsigned WIDTH = 8;
  defparam u1.WIDTH = 4;

  typedef enum logic [1:0] {S_A, S_B} state_e;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;
  typedef union packed { logic [7:0] u8; logic [7:0] u8b; } byte_u;

  and g1(y, a, b);
  or  g2(z, a, b);
  not g3(y, a);
  buf g4(y, a);
  nand g5(y, a, b);
  nor g6(y, a, b);
  xor g7(y, a, b);
  xnor g8(y, a, b);
  bufif0 g9(y, a, req);
  notif1 g10(y, a, req);
  cmos g11(y, a, req, gnt);
  nmos g12(y, a, req);
  tran g13(y, z);
  tranif0 g14(y, z, req);
  pulldown g15(y);
  pullup g16(z);

  task t(ref int rv, input int iv, output int ov);
    rv = iv;
    ov = iv;
  endtask

  function void f();
    int i;
    unique case (i)
      0: i = 1;
      default: i = 0;
    endcase
    priority if (i == 0)
      i = 1;
    else
      i = 2;
    unique0 case (i)
      0: i = 1;
      default: i = 0;
    endcase
  endfunction

  initial begin : blk142
    repeat (3) begin
      break;
    end
    forever begin
      disable blk142;
    end
    while (1) begin
      break;
    end
    do begin
      continue;
    end while (0);
    fork
      begin end
      begin end
    join_any
  end

  specify
    (a => b) = 1;
    $setup(posedge clk, posedge rst_n, 0);
    $hold(negedge clk, rst_n, 0);
  endspecify

  sequence sq1;
    a ##1 b;
  endsequence

  property p1;
    @(posedge clk) a |-> b;
  endproperty

  property p2;
    a and b or a intersect b throughout b within b implies b until b s_until b until_with b s_until_with b;
  endproperty

  property p3;
    weak(a ##1 b);
  endproperty

  clocking cb @(posedge clk);
  endclocking

endmodule

package m142_pkg2;
endpackage

package m142_pkg;
  import m142_pkg2::*;
  export m142_pkg2::*;
endpackage

interface m142_iface;
endinterface

class m142_base;
endclass

class m142_cls extends m142_base implements m142_iface;
  rand int x;
  randc int y;
  protected int z;
  local int w;
  static int s;
  function new();
    super.new();
  endfunction
  virtual function void vf();
  endfunction
  constraint c1 { x inside {[0:10]}; solve x before y; }
  constraint c2 { x dist {0 := 1, 1 := 1}; }
  function void go();
    void'(this.randomize() with { x > 0; });
  endfunction
endclass

checker m142_chk;
endchecker

covergroup m142_cg;
  coverpoint m142_cg_var {
    bins b0 = {0};
  }
endgroup
"#;

/// Every `(reserved-word-kind, immediate-parent-kind)` pair this sweep is
/// allowed to find with NO face on it, each with a one-line reason. This
/// is the only escape hatch: any anonymous reserved-word-shaped leaf not
/// listed here must carry `font-lock-keyword-face` or
/// `font-lock-type-face`, exactly spanning the token.
const M142_SWEEP_ALLOWLIST: &[(&str, &str)] = &[
    // Sensitivity-list connective (M38's own pre-existing cut, header
    // lines 89-92) and SVA/property operators/connectives (M142 spec):
    // structurally an operator joining two expressions, not a keyword
    // introducing a construct.
    ("or", "event_expression"),
    ("and", "sequence_expr"),
    ("or", "sequence_expr"),
    ("intersect", "sequence_expr"),
    ("throughout", "sequence_expr"),
    ("within", "sequence_expr"),
    ("implies", "property_expr"),
    ("until", "property_expr"),
    ("until_with", "property_expr"),
    ("s_until", "property_expr"),
    ("s_until_with", "property_expr"),
    ("inside", "inside_expression"),
    ("dist", "expression_or_dist"),
    ("with", "randomize_call"),
    // Real SV builtin array/queue method names (`grammar.js:3765-3774`,
    // `queue_method_name`/`array_or_queue_method_name`) -- a USE of a
    // built-in method, the same "a call is a use, not a definition" cut
    // this file already draws for every other method/function call name.
    ("pop_front", "queue_method_name"),
    ("push_back", "queue_method_name"),
    ("size", "array_or_queue_method_name"),
    // `randomize` (`randomize_call`, `grammar.js:3747-3751`) is a real SV
    // builtin method call name too, but unlike the others it is not even
    // in the grammar's own Annex B reserved-word list (checked: absent
    // from both `grammar.js:4591`/`4786` blocks) -- it is a plain builtin
    // call, the same "a call is a use, not a definition" cut.
    ("randomize", "randomize_call"),
    // `timeunit`/`timeprecision` value suffixes (`1ns`, `10ps`) -- a time
    // LITERAL's unit suffix, not a keyword at all.
    ("ns", "time_unit"),
    ("ps", "time_unit"),
];

/// Recursively collect every anonymous (unnamed) leaf node whose `kind()`
/// looks like a lowercase reserved word (`^[a-z_][a-z0-9_]*$` -- this
/// excludes punctuation/operator tokens like `;`/`+`/`::`, which are not
/// reserved words, and excludes every NAMED node, including declared/used
/// identifiers). Returns `(kind, parent_kind, start_byte, end_byte)`.
fn collect_keyword_leaves<'a>(
    node: tree_sitter::Node<'a>,
    out: &mut Vec<(String, String, usize, usize)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.child_count() == 0 && !child.is_named() {
            let kind = child.kind();
            let looks_like_word = kind
                .chars()
                .next()
                .map(|c| c.is_ascii_lowercase() || c == '_')
                .unwrap_or(false)
                && kind
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if looks_like_word {
                out.push((
                    kind.to_string(),
                    node.kind().to_string(),
                    child.start_byte(),
                    child.end_byte(),
                ));
            }
        }
        collect_keyword_leaves(child, out);
    }
}

fn find_verilog_corpus_files() -> Vec<std::path::PathBuf> {
    fn walk_dir(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_dir(&path, out);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ["sv", "svh", "v", "vh"].contains(&ext) {
                    out.push(path);
                }
            }
        }
    }
    let demo_root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo"));
    let mut out = vec![];
    for sub in ["rtl", "rtl-verilog2001", "verif"] {
        walk_dir(&demo_root.join(sub), &mut out);
    }
    out
}

/// Byte offset -> 1-based elisp char position (same convention as
/// `nth_token`/`exact_hl`: `point-min` is 1, and a multi-byte UTF-8
/// source is measured in Unicode scalar values, not bytes).
fn byte_to_char_pos(src: &str, byte_off: usize) -> usize {
    src[..byte_off].chars().count() + 1
}

fn line_of(src: &str, byte_off: usize) -> usize {
    src[..byte_off].matches('\n').count() + 1
}

/// M142's structural sweep guard: the part that makes "complete keyword
/// coverage" a checked fact rather than a longer hand-written list. It
/// parses each corpus file with the REAL grammar crate to find every
/// anonymous reserved-word-shaped leaf, drives the REAL highlighting
/// pipeline on the same source through `setup`/`enable_and_wait` (the
/// same path every other test in this file uses -- so this checks the
/// real overlay output, not a second, independent run of the query), and
/// asserts every one of those leaves carries a face exactly covering its
/// span, unless it is on `M142_SWEEP_ALLOWLIST`.
///
/// This test doubles as the query's compile-check: `highlight.rs`
/// isolates a `.scm` compile failure by logging and skipping (M33) rather
/// than panicking, so a broken query silently produces ZERO overlays --
/// every non-allowlisted keyword leaf in every file would then fail this
/// sweep's face check, in a way `cargo build`/`cargo test`'s own compile
/// step cannot see (the query is loaded and compiled at runtime, on the
/// worker thread, not at Rust-compile time).
///
/// Failure floors (M114 rule: a mechanism that decides what gets checked
/// must fail loudly if it decides to check nothing): at least 10 corpus
/// files must be found, and at least 1200 keyword-leaf occurrences must
/// be checked in total (the real count the day this was written was
/// 1451, corpus + the hand-written fixture below, printed on failure and
/// on a floor breach).
#[test]
fn verilog_keyword_sweep_covers_every_reserved_word_leaf() {
    let corpus = find_verilog_corpus_files();
    assert!(
        corpus.len() >= 10,
        "verilog keyword sweep: only found {} corpus file(s) under demo/rtl, \
         demo/rtl-verilog2001, demo/verif -- the discovery glob is broken, \
         not the corpus (M114: a sweep with nothing to check must fail, not \
         silently pass)",
        corpus.len()
    );

    let mut sources: Vec<(String, String)> = corpus
        .iter()
        .map(|p| {
            (
                p.display().to_string(),
                std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{:?}: {}", p, e)),
            )
        })
        .collect();
    sources.push((
        "<M142 hand-written fixture>".to_string(),
        M142_FIXTURE_SRC.to_string(),
    ));

    let mut ts_parser = tree_sitter::Parser::new();
    ts_parser
        .set_language(&tree_sitter_systemverilog::LANGUAGE.into())
        .expect("verilog keyword sweep: failed to load tree-sitter-systemverilog");

    let mut total_leaves = 0usize;
    let mut failures: Vec<String> = vec![];

    for (name, src) in &sources {
        let tree = ts_parser
            .parse(src, None)
            .unwrap_or_else(|| panic!("{name}: tree-sitter failed to parse"));
        let mut leaves = vec![];
        collect_keyword_leaves(tree.root_node(), &mut leaves);
        // Sorted by position so the window-scrolling loop below only ever
        // moves forward through the buffer.
        leaves.sort_by_key(|l| l.2);

        let (mut interp, ed) = setup(src);
        enable_and_wait(&mut interp, "verilog");
        // The highlighter only materializes `treesit-hl` overlays near the
        // visible window (`overlay_volume_is_bounded_to_the_visible_region`,
        // this same file) -- exactly the mechanism the pre-existing
        // `verilog_program_and_covergroup_in_real_sram_bank_tb` test above
        // already works around with `scroll_to_and_wait`. A real corpus
        // file is far bigger than one window, so this sweep must scroll
        // there too, once per ~300-byte span rather than once per leaf (most
        // leaves on nearby lines share one already-materialized window).
        let mut last_scroll: Option<usize> = None;

        for (kind, parent_kind, start_byte, end_byte) in leaves {
            total_leaves += 1;
            if M142_SWEEP_ALLOWLIST
                .iter()
                .any(|(k, p)| *k == kind && *p == parent_kind)
            {
                continue;
            }
            if last_scroll.is_none_or(|ls| start_byte < ls || start_byte > ls + 300) {
                scroll_to_and_wait(&mut interp, &ed, start_byte);
                last_scroll = Some(start_byte);
            }
            let s = byte_to_char_pos(src, start_byte);
            let e = byte_to_char_pos(src, end_byte);
            // Accepts EITHER face as "faced": this sweep only checks that
            // some structural face landed on the span, so a keyword/type
            // face SWAP (e.g. `"enum" @keyword` instead of `@type`) is
            // invisible here and is caught only by the named tests above,
            // which pin the specific face each construct must get.
            let faced = exact_hl(&mut interp, s, e, "font-lock-keyword-face")
                || exact_hl(&mut interp, s, e, "font-lock-type-face");
            if !faced {
                failures.push(format!(
                    "{name}:{} keyword={kind:?} parent={parent_kind:?}",
                    line_of(src, start_byte)
                ));
            }
        }
    }

    assert!(
        total_leaves >= 1200,
        "verilog keyword sweep: only checked {total_leaves} keyword-leaf \
         occurrences across {} source(s) -- expected at least 1200 (M114: \
         a sweep that checks almost nothing must fail, not silently pass)",
        sources.len()
    );

    assert!(
        failures.is_empty(),
        "verilog keyword sweep: {} unfaced reserved-word leaf(ves) found \
         (checked {total_leaves} total leaves across {} source(s)):\n{}",
        failures.len(),
        sources.len(),
        failures.join("\n")
    );
}
