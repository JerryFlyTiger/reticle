//! M43 period-4 probe for the buffer+snapshot resident-memory claim —
//! the design doc's *one* item promising a straight 4x win (§6/§7 #5:
//! "be honest about where the real gains come from" singles this out as
//! the only 4x-scale item; the others are 1.5-7x depending on path).
//! Before M43, `GapBuffer` stored `Vec<char>` (4 bytes/char) and
//! `Buffer::search_snapshot` cached an
//! `Rc<Vec<char>>` copy of the same data — two 4-bytes-per-char copies
//! resident at once. After M43, `GapBuffer` stores `Vec<u8>` (UTF-8,
//! ~1 byte/char for mostly-ASCII text) and the snapshot is an `Rc<str>`
//! — two ~1-byte-per-char copies. Deliberately `#[ignore]`d, single-
//! threaded (RSS sampling only makes sense in isolation):
//!
//!   cargo test --release -p core --test memory_footprint_perf_tests \
//!     -- --ignored --nocapture --test-threads=1
//!
//! Two independent measurements, both reported (M43 report records
//! both rather than picking the flattering one):
//!
//! 1. Empirical process RSS via `ps -o rss=` before/after building the
//!    buffer and forcing a snapshot rebuild. Real, but noisy: page-
//!    granularity, and the allocator's own growth/rounding behavior
//!    (a single big `insert` growing a gap from near-empty via
//!    `Vec::splice` doesn't land on an exact-fit allocation, and macOS
//!    malloc zones round up too) doesn't scale identically between the
//!    4-bytes/char and 1-byte/char representations, so the *measured*
//!    ratio undershoots the structural one somewhat.
//! 2. Structural estimate from the actually-measured byte/char counts
//!    of this probe's own text (`char_len`/`byte_len`) and the known
//!    per-element size of each representation's storage — deterministic,
//!    and what the design doc's §6 predicted-table numbers are computed
//!    from.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn rss_kb() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("ps should run");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

/// Same generator as `line_number_perf_tests.rs`/`search_perf_tests.rs`
/// — ~10MB, 200k lines, mostly ASCII with a periodic Chinese-mixed line
/// — so this probe's numbers are directly comparable to theirs.
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

#[test]
#[ignore]
fn measure_10mb_buffer_plus_snapshot_resident_memory() {
    let (_i, ed) = setup();
    let rss_baseline = rss_kb();

    let src = make_large_plain_text(200_000);
    let byte_len = src.len();
    let char_len = src.chars().count();

    {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
    }
    drop(src);
    let rss_after_buffer = rss_kb();

    let rss_after_snapshot;
    {
        let buf = ed.borrow().current.clone();
        let b = buf.borrow();
        let snap = b.search_text();
        rss_after_snapshot = rss_kb();
        std::hint::black_box(&snap);
    }

    let buffer_delta_kb = rss_after_buffer.saturating_sub(rss_baseline);
    let snapshot_delta_kb = rss_after_snapshot.saturating_sub(rss_after_buffer);
    let total_delta_kb = rss_after_snapshot.saturating_sub(rss_baseline);

    // Structural estimate: this build stores `char_len` chars as
    // `byte_len` UTF-8 bytes (buffer) plus one `Rc<str>` snapshot copy
    // of the same `byte_len` bytes -- 2 * byte_len total. The pre-M43
    // shape (see module doc) was 2 * 4 * char_len (two `Vec<char>`-sized
    // copies). Both numbers are printed so a from-source reader can
    // recompute the "4x" claim without re-running the pre-M43 binary.
    let structural_after_bytes = 2 * byte_len;
    let structural_before_bytes = 2 * 4 * char_len;

    eprintln!(
        "10MB probe ({byte_len} bytes, {char_len} chars): \
         rss_baseline={rss_baseline}KB rss_after_buffer={rss_after_buffer}KB \
         (delta {buffer_delta_kb}KB) rss_after_snapshot={rss_after_snapshot}KB \
         (delta {snapshot_delta_kb}KB) total_delta={total_delta_kb}KB -- structural estimate: \
         post-M43 2*byte_len={structural_after_bytes}B (~{:.1}MB), \
         pre-M43 2*4*char_len={structural_before_bytes}B (~{:.1}MB), \
         structural ratio {:.2}x",
        structural_after_bytes as f64 / 1_000_000.0,
        structural_before_bytes as f64 / 1_000_000.0,
        structural_before_bytes as f64 / structural_after_bytes as f64,
    );
}
