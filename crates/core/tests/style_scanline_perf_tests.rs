//! P1.5 probe: `style_at` scanline-ization. Before this fix,
//! `redisplay.rs`'s `style_at` re-scanned *every* overlay in the buffer
//! (a `RefCell::borrow` + linear prop lookup each) for *every* visible
//! character. Tree-sitter highlighting (`highlight.rs`, `MARGIN_CHARS =
//! 4000`) materializes overlays across the visible region plus a
//! 4000-char margin on each side — hundreds to low thousands of them on
//! a typical source file — so an 80x50 window's render() cost was
//! O(cells * overlays), tens of milliseconds per frame. Deliberately
//! `#[ignore]`d — run explicitly:
//!
//!   cargo test --release -p core --test style_scanline_perf_tests -- --ignored --nocapture
//!
//! Baseline (pre-scanline) and post-fix numbers are recorded in the
//! P1.5 report.

use std::cell::RefCell;
use std::rc::Rc;

use core::buffer::OverlayData;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::{Interp, Value};

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

/// Tiny deterministic PRNG (xorshift64) — no external `rand` dependency,
/// but still varied enough to avoid an artificially regular overlay
/// layout that the scanline (or the naive scan) could special-case.
struct Xorshift(u64);

impl Xorshift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() as usize) % n
        }
    }
}

/// Materializes `count` style-bearing overlays spread across `span`
/// chars starting at `origin`, at a density similar to token-by-token
/// tree-sitter highlighting: mostly short (1..6 char) spans, with a
/// wider "containing" span every 50th overlay (a function/block-level
/// capture nesting the token-level ones, the way real tree-sitter
/// captures do). Faces cycle through a small pool of named faces
/// registered via `set-face`, so every overlay actually resolves to a
/// non-default style — the realistic case this fix targets.
fn materialize_overlays(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    origin: usize,
    span: usize,
    count: usize,
) {
    let face_names = ["kw", "ty", "str", "num", "cmt"];
    for (i, name) in face_names.iter().enumerate() {
        run(
            interp,
            &format!(
                "(set-face '{name} :foreground \"#{:02x}{:02x}{:02x}\")",
                (i * 37) % 255,
                (i * 53) % 255,
                (i * 91) % 255
            ),
        );
    }
    let face_syms: Vec<elisp::value::SymId> = face_names.iter().map(|n| interp.intern(n)).collect();
    let face_prop = interp.intern("face");

    let buf = ed.borrow().current.clone();
    let len = buf.borrow().text.len();
    let weak = Rc::downgrade(&buf);
    let mut rng = Xorshift(0x2545_f491_4f6c_dd1d);
    let mut b = buf.borrow_mut();
    for i in 0..count {
        let width = if i % 50 == 0 { 40 } else { 1 + rng.range(6) };
        let start_off = rng.range(span.max(1));
        let s = (origin + start_off).min(len.saturating_sub(1));
        let e = (s + width).min(len);
        if s >= e {
            continue;
        }
        let face = Value::Sym(face_syms[i % face_syms.len()]);
        b.overlays.push(Rc::new(RefCell::new(OverlayData {
            buffer: weak.clone(),
            start: s,
            end: e,
            props: vec![(face_prop, face)],
            seq: i as u64,
        })));
    }
}

#[test]
#[ignore]
fn measure_render_cost_with_2000_style_overlays() {
    // ~15k chars of plain text — big enough that a 4000-char margin on
    // each side of the window fits comfortably inside it.
    let mut src = String::new();
    for n in 0..500 {
        src.push_str(&format!(
            "line {n:04}: filler filler filler filler filler pad.\n"
        ));
    }
    let (mut i, ed) = setup(&src);
    let len = ed.borrow().current.borrow().text.len();
    {
        let buf = ed.borrow().current.clone();
        buf.borrow_mut().point = len / 2;
    }
    ed.borrow_mut().frame = (80, 50);
    // Settle window_start onto point before materializing overlays or
    // timing — isolates steady-state render() cost from the one-time
    // ensure_point_visible settling cost (see line_number_perf_tests.rs).
    let _ = core::redisplay::render(&i, &ed);

    let margin = 4000usize;
    let origin = (len / 2).saturating_sub(margin);
    let span = margin * 2;
    materialize_overlays(&mut i, &ed, origin, span, 2000);

    let n = 200usize;
    let mut durations = Vec::with_capacity(n);
    for _ in 0..n {
        let start = std::time::Instant::now();
        let _ = core::redisplay::render(&i, &ed);
        durations.push(start.elapsed());
    }
    durations.sort();
    let total: std::time::Duration = durations.iter().sum();
    let mean = total / n as u32;
    let median = durations[n / 2];
    eprintln!(
        "80x50 window, {len}-char buffer, 2000 style overlays across {span} chars: \
         render() mean {mean:?}, median {median:?}, min {:?}, max {:?}",
        durations[0],
        durations[n - 1]
    );
}
