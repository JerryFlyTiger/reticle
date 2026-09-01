//! M24 probes for the "every keystroke rescans the whole buffer for a
//! line number" cost (see `crates/core/src/gapbuffer.rs`'s
//! `newline_count`/`line_cache`, and `redisplay.rs`'s gutter code).
//! Deliberately `#[ignore]`d — run explicitly:
//!
//!   cargo test --release -p core --test line_number_perf_tests -- --ignored --nocapture
//!
//! Baseline (before the cache) and post-fix numbers are recorded in the
//! M24 report.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
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

/// ~10MB of plain text: 200k lines, mostly ASCII with a periodic
/// Chinese-mixed line, no markup or highlighting — isolates the
/// gap-buffer/redisplay cost from font-lock overhead.
fn make_large_plain_text(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 55);
    for n in 0..lines {
        if n % 41 == 0 {
            s.push_str(&format!(
                "第 {:06} 行：中文測試文字，混合 ASCII words here too.\n",
                n
            ));
        } else {
            s.push_str(&format!(
                "line {:06}: filler filler filler filler filler pad.\n",
                n
            ));
        }
    }
    s
}

/// Probe 1: steady-state per-keystroke `render()` cost over a ~10MB
/// plain-text buffer with point in the middle. Before the line-number
/// cache, `render_window`'s unconditional `line_number(len)` (total
/// lines) plus the `point_line`/`window_start` lookups each rescanned
/// from position 0, making every keystroke's redisplay O(buffer size).
#[test]
#[ignore]
fn measure_per_keystroke_render_cost_10mb_plain_text() {
    let src = make_large_plain_text(200_000);
    let byte_len = src.len();
    let char_len = src.chars().count();

    // Insert directly through the buffer, not via the elisp reader —
    // feeding a 10MB string literal through `(insert ...)` would make
    // *test setup* slow enough to dominate, and find-file-internal
    // (the real large-file path) inserts this way too.
    let (mut i, ed) = setup("");
    {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
        b.point = char_len / 2;
    }
    ed.borrow_mut().frame = (120, 45);
    // Settle window_start onto point before the timed loop: the very
    // first render after jumping point to the buffer middle has to walk
    // window_start there (ensure_point_visible's row-by-row scan — a
    // separate, not-yet-fixed cost, see probe 3 below). We want this
    // probe to isolate the *steady-state per-keystroke* cost the
    // line-number cache targets, not that one-time settling cost.
    let _ = core::redisplay::render(&i, &ed);

    let n = 100usize;
    let mut durations = Vec::with_capacity(n);
    for _ in 0..n {
        feed_keys(&mut i, &ed, "x").unwrap();
        let start = std::time::Instant::now();
        let _ = core::redisplay::render(&i, &ed);
        durations.push(start.elapsed());
    }
    durations.sort();
    let total: std::time::Duration = durations.iter().sum();
    let mean = total / n as u32;
    let median = durations[n / 2];
    eprintln!(
        "10MB plain text ({byte_len} bytes, {char_len} chars): {n} keystrokes — \
         render() mean {mean:?}, median {median:?}, min {:?}, max {:?}",
        durations[0],
        durations[n - 1]
    );
}

/// Probe 3: a single `render()` right after point jumps from 0 to the
/// end of a ~10MB buffer (`M->` / end-of-buffer). This exercises
/// `ensure_point_visible`'s row-by-row forward scan from `window_start`,
/// which is a separate (not yet fixed) O(distance-in-rows) cost — not
/// held to the <2ms bar the keystroke probe is.
#[test]
#[ignore]
fn measure_render_cost_after_jump_to_buffer_end() {
    let src = make_large_plain_text(200_000);
    let byte_len = src.len();
    let char_len = src.chars().count();

    let (i, ed) = setup("");
    {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
        b.point = char_len; // M-> jump straight to the end
    }
    ed.borrow_mut().frame = (120, 45);

    let start = std::time::Instant::now();
    let _ = core::redisplay::render(&i, &ed);
    let elapsed = start.elapsed();
    eprintln!(
        "10MB plain text ({byte_len} bytes, {char_len} chars): render() right after \
         M-> jump to buffer end: {elapsed:?}"
    );
}

/// Probe 4 (P2 candidate #9): `line-number-at-pos` at five widely spread
/// positions (point-min, 25%, 50%, 75%, point-max), cycled and queried
/// 2000 times each, over the same ~8.4MB/200k-line buffer used in the
/// GNU Emacs comparison. This is the "no locality" case the line-cache
/// added by P1.1 can't help with — every query misses the cache and
/// falls back to a full scan between the last position and the new
/// one, so it isolates the *per-character constant factor* of that
/// fallback scan rather than the cache hit path (already covered by the
/// 39us steady-state probe above).
///
/// Before this fix, the fallback scanned with a per-char `char_at` call
/// (a function call plus an `index()` branch and bounds check per
/// character). This probe's counterpart run against GNU Emacs 30 (elisp
/// script issuing the same 5-position x 2000-repeat access pattern via
/// `line-number-at-pos`) took 5.4s; reticle took 16.6s (~3x
/// slower) before this fix, calling `GapBuffer::line_number` directly
/// (bypassing elisp/redisplay entirely, to isolate the gap-buffer cost
/// itself — the same reason probe 1 inserts directly through the
/// buffer rather than via `(insert ...)`).
#[test]
#[ignore]
fn measure_line_number_far_jump_access_pattern() {
    let src = make_large_plain_text(200_000);
    let byte_len = src.len();
    let char_len = src.chars().count();

    let (_i, ed) = setup("");
    let buf = ed.borrow().current.clone();
    {
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
    }

    let b = buf.borrow();
    let len = b.text.len();
    let positions = [0, len / 4, len / 2, (3 * len) / 4, len];
    let repeats = 2000;

    let start = std::time::Instant::now();
    let mut total: usize = 0; // prevent the loop from being optimized away
    for _ in 0..repeats {
        for &pos in &positions {
            total = total.wrapping_add(b.text.line_number(pos));
        }
    }
    let elapsed = start.elapsed();
    eprintln!(
        "{byte_len} bytes, {char_len} chars: {} line_number() calls over 5 far-apart \
         positions (point-min/25%/50%/75%/point-max) x {repeats} cycles: {elapsed:?} \
         (checksum {total}) — GNU Emacs 30 comparison for the same access pattern: ~5.4s",
        positions.len() * repeats
    );
}
