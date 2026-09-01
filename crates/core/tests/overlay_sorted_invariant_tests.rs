//! P1.3 correctness: `buffer.overlays` must stay sorted by `start` at
//! all times — the invariant `Buffer::insert_overlay`/`delete_overlay`/
//! `overlays_in` (see `crates/core/src/buffer.rs`) depend on for their
//! O(log n) binary searches, and which `adjust_positions_insert`/
//! `adjust_positions_delete` rely on being preserved automatically
//! (via the monotonicity argument in their doc comments) rather than
//! re-establishing it themselves.
//!
//! Drives a few hundred random `make-overlay` / `delete-overlay` /
//! insert-text / delete-text steps (fixed seed, so a failure
//! reproduces) directly through `Buffer`'s public API — no Interp/
//! Editor needed — and after every step checks:
//!
//!   1. `overlays` is still sorted by `start`.
//!   2. `overlays_in` at several random windows returns exactly the
//!      same *set* of overlays as a naive full-scan oracle using the
//!      same `start < e && end > s` predicate `overlays_in` itself
//!      implements — i.e. this is a same-semantics cross-check, not a
//!      test of some independent spec.

use std::cell::RefCell;
use std::rc::Rc;

use core::buffer::{Buffer, OverlayData};

/// Deterministic xorshift64 PRNG, consistent with the style already
/// used in `style_scanline_perf_tests.rs`/`style_scanline_correctness_tests.rs`.
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

fn is_sorted_by_start(buf: &Buffer) -> bool {
    buf.overlays
        .windows(2)
        .all(|w| w[0].borrow().start <= w[1].borrow().start)
}

/// Same predicate `Buffer::overlays_in` implements, applied as a plain
/// linear scan — the oracle this test cross-checks the real
/// (binary-search-accelerated) implementation against.
fn naive_overlays_in(buf: &Buffer, s: usize, e: usize) -> Vec<Rc<RefCell<OverlayData>>> {
    buf.overlays
        .iter()
        .filter(|ov| {
            let o = ov.borrow();
            o.start < e && o.end > s
        })
        .cloned()
        .collect()
}

#[test]
fn overlays_stay_sorted_and_overlays_in_matches_naive_oracle_across_random_ops() {
    let mut rng = Xorshift(0x1234_5678_9abc_def0);
    let mut text = String::new();
    for i in 0..2000 {
        text.push(if i % 60 == 59 { '\n' } else { 'a' });
    }
    let buf = Rc::new(RefCell::new(Buffer::new("test", &text)));
    let mut live: Vec<Rc<RefCell<OverlayData>>> = Vec::new();

    for step in 0..500 {
        let len = buf.borrow().text.len();
        if len == 0 {
            continue;
        }
        let op = rng.range(4);
        match op {
            0 => {
                // make-overlay: random span, inserted via the real
                // sorted-insert path.
                let a = rng.range(len);
                let b = rng.range(len);
                let (s, e) = (a.min(b), (a.max(b) + rng.range(20)).min(len));
                let weak = Rc::downgrade(&buf);
                let mut b = buf.borrow_mut();
                let seq = b.alloc_overlay_seq();
                let ov = Rc::new(RefCell::new(OverlayData {
                    buffer: weak,
                    start: s,
                    end: e,
                    props: Vec::new(),
                    seq,
                }));
                b.insert_overlay(ov.clone());
                drop(b);
                live.push(ov);
            }
            1 => {
                // delete-overlay: remove a random live overlay.
                if !live.is_empty() {
                    let idx = rng.range(live.len());
                    let ov = live.remove(idx);
                    buf.borrow_mut().delete_overlay(&ov);
                }
            }
            2 => {
                // Insert text at a random position.
                let pos = rng.range(len + 1);
                let s: String = "b".repeat(1 + rng.range(5));
                buf.borrow_mut().insert(pos, &s);
            }
            _ => {
                // Delete a random small text range.
                let s = rng.range(len);
                let width = 1 + rng.range(5);
                let e = (s + width).min(len);
                if s < e {
                    buf.borrow_mut().delete(s, e);
                }
            }
        }

        {
            let b = buf.borrow();
            assert!(
                is_sorted_by_start(&b),
                "step {step} (op {op}): overlays no longer sorted by start"
            );
        }

        // Cross-check overlays_in against the naive oracle at a few
        // random windows each step.
        let len_now = buf.borrow().text.len();
        if len_now > 0 {
            for _ in 0..3 {
                let a = rng.range(len_now);
                let bnd = rng.range(len_now);
                let (s, e) = (a.min(bnd), (a.max(bnd) + rng.range(30)).min(len_now));
                let b = buf.borrow();
                let mut got: Vec<*const RefCell<OverlayData>> =
                    b.overlays_in(s, e).iter().map(Rc::as_ptr).collect();
                let mut want: Vec<*const RefCell<OverlayData>> =
                    naive_overlays_in(&b, s, e).iter().map(Rc::as_ptr).collect();
                got.sort_unstable();
                want.sort_unstable();
                assert_eq!(
                    got, want,
                    "step {step}: overlays_in({s}, {e}) mismatch vs naive oracle"
                );
            }
        }
    }
}
