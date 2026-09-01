//! P1.3 probes: overlay bookkeeping de-quadratic-ization. Before this
//! fix, `buffer.overlays` (`crates/core/src/buffer.rs`) was an unsorted
//! `Vec`, so:
//!
//!   * `make-overlay` / `put-text-property` appended in O(1), but
//!   * `delete-overlay` (`Vec::retain` over every overlay) and
//!     `overlays-in` (a `Vec::iter().filter()` over every overlay) were
//!     both O(overlays-in-buffer) — a `RefCell::borrow` plus a range
//!     comparison per overlay, every call.
//!
//! `org-mode`'s fontify (`org--fontify-line` in `lisp/org.el`) calls
//! `org--remove-face-overlays`, which is `overlays-in` followed by
//! `delete-overlay` on each match, once per line to clear that line's
//! old face overlays before re-painting it. So a whole-buffer fontify
//! pass did O(lines) of those calls, each O(overlays-in-buffer) — and
//! the overlay count itself grows with line count, making the total
//! cost O(lines²).
//!
//! Deliberately `#[ignore]`d — run explicitly:
//!
//!   cargo test --release -p core --test overlay_bookkeeping_perf_tests -- --ignored --nocapture
//!
//! These probes deliberately go through the real elisp builtins
//! (`make-overlay`/`delete-overlay`/`overlays-in`/`insert`/`org-mode`)
//! rather than calling internal Rust methods directly, so the exact
//! same test file measures both the pre-fix and post-fix
//! implementation unmodified — a fair before/after comparison. Baseline
//! (pre-fix) and post-fix numbers are recorded in the P1.3 report.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::Interp;

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    {
        let buf = ed.borrow().current.clone();
        buf.borrow_mut().insert(0, src);
    }
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) {
    if let Err(flow) = interp.eval_source(src) {
        panic!("eval {src:?} failed: {}", interp.describe_flow(&flow));
    }
}

fn summarize(label: &str, durations: &mut [std::time::Duration]) {
    durations.sort();
    let n = durations.len();
    let total: std::time::Duration = durations.iter().sum();
    let mean = total / n as u32;
    let median = durations[n / 2];
    eprintln!(
        "{label}: mean {mean:?}, median {median:?}, min {:?}, max {:?}",
        durations[0],
        durations[n - 1]
    );
}

/// Org content mixing headings, TODO/tag lines, a linked list item, a
/// table, and plain prose — same shape as `org_tests.rs`'s existing
/// `measure_org_fontify_cost_20k_lines`, parameterized by line count so
/// this probe can compare several sizes in one run.
fn make_org_content(lines: usize) -> String {
    let mut src = String::new();
    for n in 0..lines {
        match n % 20 {
            0 => src.push_str(&format!("* Heading {n}\n")),
            5 => src.push_str(&format!("** TODO Sub {n} :tag:\n")),
            10 => src.push_str(&format!(
                "- item {n} with [[https://example.com/{n}][a link]]\n"
            )),
            15 => src.push_str(&format!("| col{n} | col{} | col{} |\n", n + 1, n + 2)),
            _ => src.push_str(&format!(
                "body text line {n} — plain prose, nothing special, some 中文 mixed in.\n"
            )),
        }
    }
    src
}

/// Probe (a): does a whole-buffer org fontify pass scale linearly or
/// quadratically with line count? `org--fontify-line` calls
/// `overlays-in` + `delete-overlay` once per line, and the overlay
/// count grows with the line count too, so O(overlays-in-buffer) per
/// call makes the whole pass O(lines²): going from 20k to 60k lines
/// (3x the lines) should cost ~9x if quadratic, ~3x (allow up to 3.5x)
/// if linear. 5k -> 20k is a 4x line increase (~16x if quadratic, ~4x
/// if linear) — a second, independent data point.
#[test]
#[ignore]
fn measure_org_fontify_scaling_5k_20k_60k_lines() {
    let sizes = [5_000usize, 20_000, 60_000];
    let mut elapsed = Vec::with_capacity(sizes.len());
    for &lines in &sizes {
        let src = make_org_content(lines);
        let char_len = src.chars().count();
        let (mut interp, _ed) = setup(&src);
        let start = std::time::Instant::now();
        run(&mut interp, "(org-mode)");
        let d = start.elapsed();
        eprintln!("org-mode fontify over {lines} lines ({char_len} chars): {d:?}");
        elapsed.push(d);
    }
    let ratio_20_5 = elapsed[1].as_secs_f64() / elapsed[0].as_secs_f64().max(1e-9);
    let ratio_60_20 = elapsed[2].as_secs_f64() / elapsed[1].as_secs_f64().max(1e-9);
    eprintln!(
        "scaling ratios: 20k/5k lines (4x lines) = {ratio_20_5:.2}x time (linear ~4x, \
         quadratic ~16x); 60k/20k lines (3x lines) = {ratio_60_20:.2}x time (linear ~3x, \
         quadratic ~9x)"
    );
}

/// Filler text with a newline every 60 chars — realistic enough line
/// shape for overlay density without org-mode's own parsing cost
/// muddying these three probes (they exercise the overlay store
/// directly through `make-overlay`/`delete-overlay`/`overlays-in`, not
/// through org's fontify).
fn make_filler_text(chars: usize) -> String {
    let mut s = String::with_capacity(chars);
    for i in 0..chars {
        s.push(if i % 60 == 59 { '\n' } else { 'x' });
    }
    s
}

/// Materializes `count` overlays via the real `make-overlay` builtin,
/// evenly spaced across a `chars`-long buffer (not timed; only the
/// operations in each probe below are).
fn make_overlays_via_builtin(interp: &mut Interp, chars: usize, count: usize) {
    let step = (chars / count.max(1)).max(1);
    for i in 0..count {
        let s = (i * step).min(chars.saturating_sub(2));
        let e = s + 1;
        run(interp, &format!("(make-overlay {s} {e})"));
    }
}

/// Probe (b)(i): 1000 `overlays-in` calls with small (40-char),
/// strictly increasing-position windows over a buffer that already
/// holds 5000 overlays — the exact access pattern
/// `org--remove-face-overlays` drives once per line during a fontify
/// pass. This is a steady-state-N=5000 measurement (the buffer's
/// overlay population is static across all 1000 calls) rather than
/// the incrementally-growing population a real fontify pass sees —
/// probe (a) above is what captures the true end-to-end scaling.
#[test]
#[ignore]
fn measure_overlays_in_1000_increasing_small_ranges_with_5000_overlays() {
    let chars = 200_000;
    let src = make_filler_text(chars);
    let (mut interp, _ed) = setup(&src);
    make_overlays_via_builtin(&mut interp, chars, 5000);

    let n = 1000usize;
    let span = chars / n;
    let mut durations = Vec::with_capacity(n);
    for i in 0..n {
        let s = i * span;
        let e = (s + 40).min(chars);
        let src = format!("(overlays-in {s} {e})");
        let start = std::time::Instant::now();
        run(&mut interp, &src);
        durations.push(start.elapsed());
    }
    summarize(
        "overlays-in x1000, small increasing ranges, 5000 overlays live",
        &mut durations,
    );
}

/// Probe (b)(ii): 1000 make-overlay/delete-overlay round trips at
/// increasing positions over a buffer that already holds 5000
/// overlays — the `org--face-region` / `org--remove-face-overlays`
/// cycle. Note this buffer's 5000 pre-existing overlays already span
/// its *entire* range (unlike a real fontify pass, where only the
/// lines already scanned have contributed overlays), so each new
/// make-overlay here binary-searches into the middle of the sorted
/// vec rather than landing at the tail — a harder case than org's own
/// amortized-O(1)-at-the-tail access pattern, deliberately kept this
/// way so this probe doesn't hide worst-case cost within a realistic
/// overlay count.
#[test]
#[ignore]
fn measure_make_delete_overlay_1000_round_trips_with_5000_overlays() {
    let chars = 200_000;
    let src = make_filler_text(chars);
    let (mut interp, _ed) = setup(&src);
    make_overlays_via_builtin(&mut interp, chars, 5000);

    let n = 1000usize;
    let step = chars / n;
    let mut durations = Vec::with_capacity(n);
    for i in 0..n {
        let s = (i * step).min(chars.saturating_sub(2));
        let e = s + 1;
        let start = std::time::Instant::now();
        run(
            &mut interp,
            &format!("(delete-overlay (make-overlay {s} {e}))"),
        );
        durations.push(start.elapsed());
    }
    summarize(
        "make+delete-overlay x1000 round trip, increasing positions, 5000 overlays live",
        &mut durations,
    );
}

/// Probe (b)(iii): 1000 single-character inserts at a fixed point with
/// 5000 overlays live — measures `adjust_positions_insert`'s
/// full-scan cost (every overlay's `start`/`end` gets touched on every
/// text edit, regardless of where in the buffer the edit happens).
/// P1.3's design does NOT target this cost (see the P1.3 report) — the
/// fix is about `make-overlay`/`delete-overlay`/`overlays-in`'s own
/// O(overlays) scans, not the per-edit position-adjustment pass — so
/// this probe's post-fix number is expected to be roughly unchanged
/// from its baseline.
#[test]
#[ignore]
fn measure_single_char_insert_1000_with_5000_overlays() {
    let chars = 200_000;
    let src = make_filler_text(chars);
    let (mut interp, ed) = setup(&src);
    make_overlays_via_builtin(&mut interp, chars, 5000);

    let pos = chars / 2;
    {
        let buf = ed.borrow().current.clone();
        buf.borrow_mut().point = pos;
    }
    let n = 1000usize;
    let mut durations = Vec::with_capacity(n);
    for _ in 0..n {
        let start = std::time::Instant::now();
        run(&mut interp, "(insert \"x\")");
        durations.push(start.elapsed());
    }
    summarize(
        "single-char insert x1000, 5000 overlays live",
        &mut durations,
    );
}
