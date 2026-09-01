//! P2 #8 perf probe for the literal-prefix fast-skip in `Regex::search`
//! (`crates/elisp/src/regex.rs`).
//!
//! Real-world numbers that motivated this optimization: an elisp script
//! running `(re-search-forward "line 199999")` 50 times over an 8.4MB /
//! 200k-line buffer (each line `"line N some filler text here
//! abcdefgh\n"`), searching for a line near the very end. GNU Emacs: 50
//! calls in 106ms. reticle before this fix: 7716ms — ~73x slower.
//! Root cause: every line shares the literal prefix `"line "` with the
//! search pattern, so the old position-by-position `search` loop called
//! `match_at` (heap-allocating a fresh `saves` Vec, then running the
//! backtracking VM) at every one of ~200,000 line starts before finally
//! reaching the one true match near the end.
//!
//! M43 period 2 rewrote the engine to work over `&str` + byte positions
//! (was `&[char]`), with the prefix-skip itself now riding `str::find`
//! (libstd's Two-Way/memchr substring search) instead of a hand-rolled
//! per-char scan — see the module doc on `regex.rs` and the M43 design
//! doc §3.4/§1.4 for the ~7.3x measured win that motivated it.
//!
//! Deliberately `#[ignore]`d — run explicitly:
//!
//!   cargo test --release -p elisp --test regex_prefix_skip_perf_tests -- --ignored --nocapture

use elisp::regex::Regex;

/// ~8.4MB, 200k lines: `"line N some filler text here abcdefgh\n"` for
/// every line, matching the shape of the real script that exposed the
/// 73x slowdown. Every line starts with `"line "`, so any pattern
/// beginning with that literal shares a prefix with all 200k lines —
/// the exact adversarial shape the fast-skip targets.
fn make_haystack(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 42);
    for n in 0..lines {
        s.push_str(&format!("line {n} some filler text here abcdefgh\n"));
    }
    s
}

#[test]
#[ignore]
fn measure_search_near_end_of_8mb_buffer_with_shared_line_prefix() {
    let hay = make_haystack(200_000);
    let byte_len = hay.len();
    assert!(
        byte_len > 8 * 1024 * 1024,
        "expected an 8MB+ buffer, got {byte_len} bytes"
    );

    // "line 199999" is near the tail: the last full line is "line
    // 199999 ...", planted at index 199_999 (0-based), i.e. the very
    // last line written by make_haystack(200_000).
    let re = Regex::new("line 199999").expect("pattern should compile");

    let n = 50;
    let start = std::time::Instant::now();
    for _ in 0..n {
        let caps = re
            .search(&hay, 0)
            .expect("must not hit the M81 step/frame budget")
            .expect("planted needle must be found");
        let (ms, _me) = caps[0].expect("match without group 0");
        assert!(ms > 0, "sanity: match should not be at position 0");
    }
    let elapsed = start.elapsed();
    let mean = elapsed / n;
    eprintln!(
        "prefixed pattern \"line 199999\" over {byte_len}-byte buffer: \
         {n} calls in {elapsed:?} (mean {mean:?})",
    );

    // Before the fix this probe measured ~7.7s total (~154ms/call) on
    // this shape of input; the fix should land solidly in the
    // 100-300ms *total* ballpark for all 50 calls, an order of
    // magnitude improvement. Generous margin (500ms total) so the
    // assertion isn't flaky on a loaded CI box, while still failing
    // outright if the fast-skip path regresses back toward the old
    // per-position behavior.
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "expected order-of-magnitude speedup (target 100-300ms total for {n} calls), got {elapsed:?}"
    );
}

#[test]
#[ignore]
fn measure_no_prefix_pattern_is_unaffected() {
    // Sanity check: a pattern with no usable literal prefix (starts
    // with `.`) takes the untouched fallback path in `search` and
    // should show no change in behavior or asymptotic cost from this
    // optimization — it was already trying every position before, and
    // still does.
    let hay = make_haystack(200_000);
    let re = Regex::new(".*199999").expect("pattern should compile");

    let n = 5; // fewer iterations: this path is still O(n) per call by design
    let start = std::time::Instant::now();
    for _ in 0..n {
        let caps = re
            .search(&hay, 0)
            .expect("must not hit the M81 step/frame budget")
            .expect("planted needle must be found");
        assert!(caps[0].is_some());
    }
    let elapsed = start.elapsed();
    eprintln!(
        "no-prefix pattern \".*199999\" over {}-byte buffer: {n} calls in {elapsed:?} \
         (mean {:?}) — unaffected path, informational only",
        hay.len(),
        elapsed / n
    );
}

/// M81 R6: third benchmark, run explicitly (not part of the assertion
/// suite — see the module doc for why `#[ignore]`). 100,000 EXTERNAL
/// `Regex::search()` calls (each one allocates and drops its own
/// `Scratch` — `search` does NOT share state across separate calls,
/// unlike `search_with`) against a fixed 46-byte, single-line haystack
/// with a literal (prefix-skip-eligible) pattern.
#[test]
#[ignore]
fn measure_repeated_external_search_calls_on_a_short_line() {
    let line = "the quick brown fox jumps over the lazy dog!!\n"; // 46 bytes
    assert_eq!(line.len(), 46);
    let re = Regex::new("brown").expect("pattern should compile");
    let n = 100_000;
    let start = std::time::Instant::now();
    for _ in 0..n {
        let caps = re
            .search(line, 0)
            .expect("must not hit the M81 step/frame budget")
            .expect("needle must be found");
        assert!(caps[0].is_some());
    }
    let elapsed = start.elapsed();
    eprintln!(
        "46-byte line, {n} EXTERNAL search() calls in {elapsed:?} (mean {:?})",
        elapsed / n
    );
}
