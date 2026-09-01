//! P1.2 probes for the "every search call re-materializes the whole
//! buffer, and every regex call recompiles the pattern" cost (see
//! `Buffer::search_text` in `crates/core/src/buffer.rs`, the compiled
//! pattern cache in `crates/elisp/src/regex.rs`'s `compile`, and the
//! `re-search-forward`/`re-search-backward`/`looking-at`/`search-forward`
//! builtins plus isearch in `crates/core/src/editor.rs`, all of which
//! used to call `bb.text.slice(...)` — an O(buffer size) gap-buffer walk
//! plus UTF-8 re-encode — on every single call).
//!
//! Deliberately `#[ignore]`d — run explicitly:
//!
//!   cargo test --release -p core --test search_perf_tests -- --ignored --nocapture
//!
//! Baseline (before the snapshot cache + regex cache) and post-fix
//! numbers are recorded in the P1.2 report.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::{isearch_push_char, isearch_start, Editor};
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

/// ~10MB of plain text: 200k lines, mostly ASCII with a periodic
/// Chinese-mixed line — same generator shape as
/// `line_number_perf_tests.rs`'s, so the two probes are comparable.
/// The target string "needle-target-phrase" is planted once near the
/// very end so every incremental isearch keystroke below has to walk
/// (almost) the whole buffer before matching — the worst case the
/// snapshot cache is meant to fix.
fn make_large_plain_text_with_needle(lines: usize, needle: &str) -> String {
    let mut s = String::with_capacity(lines * 55);
    for n in 0..lines {
        if n + 5 == lines {
            s.push_str(needle);
            s.push('\n');
        } else if n % 41 == 0 {
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

/// Probe: per-keystroke cost of incremental search (`C-s` + typing) over
/// a ~10MB buffer. Before the search-text snapshot cache, `isearch_step`
/// called `bb.text.slice(origin, len)` on every keystroke — an O(N)
/// gap-buffer materialization independent of how short the query is —
/// making each keystroke of a search O(buffer size). Measures the mean
/// cost of the searches triggered while typing a 7-char needle one
/// character at a time (7 incremental searches, each re-run from the
/// fixed isearch origin per Emacs semantics).
#[test]
#[ignore]
fn measure_isearch_per_keystroke_cost_10mb_buffer() {
    let needle = "zqfetch";
    let src = make_large_plain_text_with_needle(200_000, needle);
    let byte_len = src.len();
    let char_len = src.chars().count();

    let (_interp, ed) = setup();
    {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
        b.point = 0;
    }

    isearch_start(&ed, true);
    let query_chars: Vec<char> = needle.chars().collect();
    assert!(
        query_chars.len() >= 5 && query_chars.len() <= 8,
        "probe expects a 5-8 char needle, got {}",
        query_chars.len()
    );

    let mut durations = Vec::with_capacity(query_chars.len());
    for &c in &query_chars {
        let start = std::time::Instant::now();
        isearch_push_char(&ed, c);
        durations.push(start.elapsed());
    }

    // Confirm the search actually found the planted needle (correctness
    // guard, not just a speed probe): after typing the full query, point
    // should sit right after the match and isearch should not be in the
    // "failing" state.
    assert!(
        !ed.borrow().isearch.as_ref().unwrap().failed,
        "isearch never matched the planted needle"
    );

    durations.sort();
    let n = durations.len();
    let total: std::time::Duration = durations.iter().sum();
    let mean = total / n as u32;
    let median = durations[n / 2];
    eprintln!(
        "10MB plain text ({byte_len} bytes, {char_len} chars): {n}-keystroke isearch for \
         {needle:?} — mean {mean:?}, median {median:?}, min {:?}, max {:?}",
        durations[0],
        durations[n - 1]
    );

    // Sanity assertion so a regression fails the run outright when
    // executed, not just when someone reads the eprintln output.
    assert!(
        mean < std::time::Duration::from_millis(5),
        "expected <5ms mean per-keystroke isearch cost on a 10MB buffer, got {mean:?}"
    );
}
