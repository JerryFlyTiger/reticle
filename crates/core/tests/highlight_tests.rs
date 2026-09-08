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

/// Pump ticks (10ms apart, max 2s) until `pred` returns "t".
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    for _ in 0..200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
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
