//! P1.5 correctness cross-check: `OverlayStyleScan` (the scanline
//! replacement for `redisplay.rs`'s per-character `style_at`) must
//! produce exactly the same result as `style_at_naive` (the original
//! implementation, kept around for this purpose) for every position a
//! render pass could ever query.
//!
//! `style_at_naive` folds every overlay covering `pos`, in creation
//! order (`OverlayData::seq` — since P1.3, `buffer.overlays` itself is
//! kept sorted by `start` for `make-overlay`/`delete-overlay`/
//! `overlays-in`'s sake, so creation order is no longer its iteration
//! order and has to be recovered via `seq`), via `merge_styles` — later
//! overlays win ties on fg/bg/underline, while bold/italic/reverse just
//! OR together (so their relative order never matters). `OverlayStyleScan`
//! keeps that same fold order for its *active* subset as `pos` sweeps
//! forward with a two-pointer scan. The risk this test targets: the sweep
//! state (which overlays are "active") getting out of sync with the naive
//! per-position scan, especially around zero-length overlays, several
//! overlays sharing the same start, nested/overlapping overlays, and
//! `pos` jumping forward by more than one position between queries (the
//! render loop does this when it walks over an invisible region — see
//! `redisplay.rs`'s `invisible_end`).
//!
//! Random overlay layouts are generated with a small deterministic PRNG
//! (seeded, so failures reproduce) rather than pulling in a `rand`
//! dependency for one test file. Overlays are pushed directly to
//! `buffer.overlays` (bypassing `Buffer::insert_overlay`) so this file's
//! index-based same-start tie-forcing (see below) still means exactly
//! "first and second created" — harmless here since neither function
//! under test requires `overlays` itself to be sorted (each recovers
//! creation order from `seq` explicitly).

use std::cell::RefCell;
use std::rc::Rc;

use core::buffer::OverlayData;
use core::editor::Editor;
use core::redisplay::{style_at_naive, OverlayStyleScan};
use elisp::{Interp, Value};

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

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) {
    if let Err(flow) = interp.eval_source(src) {
        panic!("eval {src:?} failed: {}", interp.describe_flow(&flow));
    }
}

fn eval(interp: &mut Interp, src: &str) -> Value {
    match interp.eval_source(src) {
        Ok(v) => v,
        Err(flow) => panic!("eval {src:?} failed: {}", interp.describe_flow(&flow)),
    }
}

/// Random overlapping/nested/zero-length overlay layouts, checked
/// position-by-position (in increasing order, with random gaps to
/// simulate the render loop's invisible-region jumps) against the
/// naive reference implementation.
#[test]
fn scanline_matches_naive_style_at_random_overlays() {
    let (mut interp, ed) = setup();
    run(&mut interp, "(set-face 'named-a :foreground \"#123456\")");
    run(
        &mut interp,
        "(set-face 'named-b :background \"#654321\" :weight 'bold)",
    );

    // A mix of face forms `style_at`/`merge_face` all handle
    // differently: nil, a plist, a plist with the wave-underline
    // sub-form, a symbol naming a registered face, a symbol naming
    // *nothing* registered (real tree-sitter capture names without a
    // matching face resolve this way — contributes nothing), and a
    // list-of-faces.
    let face_srcs = [
        "nil",
        "'(:foreground \"#ff0000\")",
        "'(:background \"#00ff00\" :weight bold)",
        "'(:foreground \"#0000ff\" :slant italic :underline t)",
        "'(:underline (:style wave :color \"#ffcc00\"))",
        "'(:inverse-video t)",
        "'unregistered-face-xyz",
        "'named-a",
        "'named-b",
        "'(named-a named-b)",
    ];
    let face_vals: Vec<Value> = face_srcs.iter().map(|s| eval(&mut interp, s)).collect();
    let face_prop = interp.intern("face");

    let buf = ed.borrow().current.clone();
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);

    for trial in 0..60 {
        let span_len = 200 + rng.range(400);
        let weak = Rc::downgrade(&buf);
        {
            let mut b = buf.borrow_mut();
            b.overlays.clear();
            let overlay_count = 20 + rng.range(60);
            for k in 0..overlay_count {
                let s = rng.range(span_len + 1);
                let width = match k % 7 {
                    0 => 0,                  // zero-length: never active
                    1 => 30 + rng.range(80), // wide "nesting" span
                    _ => 1 + rng.range(5),   // typical token-sized span
                };
                let e = (s + width).min(span_len + 1);
                let face = face_vals[rng.range(face_vals.len())].clone();
                b.overlays.push(Rc::new(RefCell::new(OverlayData {
                    buffer: weak.clone(),
                    start: s,
                    end: e,
                    props: vec![(face_prop, face)],
                    seq: k as u64,
                })));
            }
            // Force an explicit same-start tie beyond whatever the
            // random draw already produced.
            if b.overlays.len() >= 2 {
                let s0 = b.overlays[0].borrow().start;
                let mut ov1 = b.overlays[1].borrow_mut();
                ov1.start = s0;
                if ov1.end <= s0 {
                    ov1.end = s0 + 3;
                }
            }
        }

        let editor = ed.borrow();
        let b = buf.borrow();
        let mut scan = OverlayStyleScan::new(&interp, &editor, &b);

        // Monotonic query sequence with random gaps of 1..=4 — a stand-in
        // for the render loop's forward-only, occasionally-jumping walk
        // over an invisible region.
        let mut pos = 0usize;
        loop {
            let naive = style_at_naive(&interp, &editor, &b, pos);
            let scanned = scan.at(pos);
            assert_eq!(
                naive, scanned,
                "trial {trial} pos {pos} (span_len {span_len}): \
                 naive style {naive:?} != scanline style {scanned:?}"
            );
            if pos >= span_len {
                break;
            }
            pos = (pos + 1 + rng.range(4)).min(span_len);
        }
    }
}

/// End-to-end sanity check on the actual `render()` path: a style
/// overlay that starts *inside* an invisible region and extends past
/// it must still paint once the visible text resumes — i.e. the
/// position jump `invisible_end` causes in the main render loop must
/// not desync the scanline's active set.
#[test]
fn render_applies_overlay_style_across_invisible_jump() {
    let (mut interp, ed) = setup();
    run(&mut interp, "(insert \"abcdefghijklmnopqrstuvwxyz\")");
    run(
        &mut interp,
        "(set-face 'hidden-through :foreground \"#00ff00\")",
    );

    let buf = ed.borrow().current.clone();
    let face_prop = interp.intern("face");
    let invisible_prop = interp.intern("invisible");
    let hidden_through_sym = interp.intern("hidden-through");
    let t = interp.syms.t;
    {
        let weak = Rc::downgrade(&buf);
        let mut b = buf.borrow_mut();
        // Invisible region [5, 10); the style overlay [8, 15) starts
        // inside it and extends 5 chars past its end.
        b.overlays.push(Rc::new(RefCell::new(OverlayData {
            buffer: weak.clone(),
            start: 5,
            end: 10,
            props: vec![(invisible_prop, Value::bool(true, t))],
            seq: 0,
        })));
        b.overlays.push(Rc::new(RefCell::new(OverlayData {
            buffer: weak,
            start: 8,
            end: 15,
            props: vec![(face_prop, Value::Sym(hidden_through_sym))],
            seq: 1,
        })));
        b.point = 0;
    }
    ed.borrow_mut().frame = (40, 5);

    let grid = core::redisplay::render(&interp, &ed);
    // Row 0: "abcde" (5 visible chars) + "..." (3-char invisible
    // indicator) + "klmno" (chars 10..15, still inside the style
    // overlay's [8, 15) range) + "pqr..." beyond it, unstyled.
    let row = &grid.lines[0];
    let k_col = 5 + 3; // after "abcde..."
    assert_eq!(row[k_col].ch, 'k');
    assert_eq!(
        row[k_col].style.fg,
        Some((0, 255, 0)),
        "char right after the invisible jump should still carry the \
         overlay's style — got {:?}",
        row[k_col].style
    );
    // "p" (char 15) is past the overlay's end and must be unstyled.
    let p_col = k_col + 5;
    assert_eq!(row[p_col].ch, 'p');
    assert_eq!(
        row[p_col].style.fg, None,
        "char past the overlay's end must not carry its style"
    );
}
