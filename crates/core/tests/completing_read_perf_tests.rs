//! Scale probe for `completing-read`'s `Source::Custom` filter path
//! (`crates/core/src/complete.rs`'s `custom_filter`) at a candidate
//! count no existing test exercises: every other `completing-read`
//! picker in this codebase (M47's `lsp-goto-symbol-by-name', M48's
//! `lsp-code-action-at-point'/`lsp-rename', M59's own
//! `lsp-references-at-point') has only ever been tested against
//! single-digit candidate lists.
//!
//! Scope, precisely: this covers ONLY the picker's own candidate-
//! FILTERING data layer -- the Rust `custom_filter` function -- at 1000
//! candidates. It does NOT cover, and makes no claim about, the cost of
//! M59's own elisp-side candidate-BUILDING path that runs before this
//! ever gets called: `lsp--reference-alist''s per-element `sort'
//! comparator and its per-distinct-file `file-contents-as-string' read/
//! cache loop (`lsp--reference-line-text'). Those run once, in elisp,
//! against however many `Location' elements a real reply carries
//! (`lsp-references-at-point''s own real-server probe, see PLAN.md's
//! M59 pre-flight, found a single query returning 889 candidates spread
//! across 889 files) -- untested at that scale by anything in this
//! repo as of this file's writing; not measured here, and no fabricated
//! timing number is asserted about it.
//!
//! What IS asserted below is correctness (prefix matches sorted before
//! substring matches, both in the collection's own relative order,
//! exact candidate count) plus a single, deliberately generous timing
//! ceiling actually measured on this machine for `custom_filter' alone
//! (well under a millisecond in practice -- the ceiling asserted is
//! 200ms specifically so it can't flake on a slow, loaded CI box; it is
//! not a claim that 200ms is an expected or acceptable latency).
//!
//! Deliberately `#[ignore]`d, matching every other `_perf_tests.rs` file
//! in this crate -- run explicitly:
//!
//!   cargo test --release -p core --test completing_read_perf_tests -- --ignored --nocapture

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
use core::complete::custom_filter;
use core::editor::Editor;
use elisp::Interp;

/// 1000 candidates in a references-picker-like shape
/// (`"REL:LINE:COL  TEXT"'), built so `custom_filter''s two-tier
/// prefix-then-substring split has real work to do:
///   - 2 candidates start with the probe input ("ref_042_a.sv:1:1  x",
///     "ref_042_b.sv:2:3  y") -- prefix matches.
///   - 1 candidate contains the probe input but NOT as a prefix
///     ("other/xref_042_c.sv:5:9  z") -- substring-only match.
///   - The remaining 997 share no characters with the probe input at
///     all ("filler_NNNN.sv:1:1  nothing-here-at-all") -- true negatives,
///     just bulk for the scan to plow through.
fn build_candidates() -> Vec<String> {
    let mut out = Vec::with_capacity(1000);
    out.push("ref_042_a.sv:1:1  x".to_string());
    for n in 0..498 {
        out.push(format!("filler_{n:04}.sv:1:1  nothing-here-at-all"));
    }
    out.push("ref_042_b.sv:2:3  y".to_string());
    for n in 498..996 {
        out.push(format!("filler_{n:04}.sv:1:1  nothing-here-at-all"));
    }
    out.push("other/xref_042_c.sv:5:9  z".to_string());
    for n in 996..997 {
        out.push(format!("filler_{n:04}.sv:1:1  nothing-here-at-all"));
    }
    assert_eq!(out.len(), 1000, "fixture must be exactly 1000 candidates");
    out
}

#[test]
#[ignore]
fn custom_filter_over_1000_candidates_prefix_before_substring() {
    let cands = build_candidates();

    let start = std::time::Instant::now();
    let result = custom_filter(&cands, "ref_042");
    let elapsed = start.elapsed();

    assert_eq!(
        result,
        vec![
            "ref_042_a.sv:1:1  x".to_string(),
            "ref_042_b.sv:2:3  y".to_string(),
            "other/xref_042_c.sv:5:9  z".to_string(),
        ],
        "expected both prefix matches (collection order) then the one \
         substring-only match, got {result:?}"
    );

    eprintln!("custom_filter over {} candidates: {elapsed:?}", cands.len());
    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "expected a single-pass filter over 1000 short strings to stay \
         well under 200ms (deliberately generous, not a tight bound), \
         got {elapsed:?}"
    );
}

#[test]
#[ignore]
fn custom_filter_over_1000_candidates_no_match_returns_empty() {
    let cands = build_candidates();
    let result = custom_filter(&cands, "nonexistent-input-xyz");
    assert!(result.is_empty(), "expected no matches, got {result:?}");
}

/// M84 T12: `Source::Symbol` (the `S` interactive code) scans and sorts
/// the WHOLE symbol table on every `complete::candidates` call
/// (`filter_sorted`, run from the panel's per-keystroke refresh). M84's
/// D6 session cache means that scan only happens once per minibuffer
/// session (see `completing_read_tests.rs`'s
/// `symbol_source_candidate_list_is_computed_once_per_session` for the
/// direct assertion on that); this is the OTHER half -- that even with
/// the raw scan cached, filtering+sorting the cached ~2000+-entry vector
/// on every single keystroke still stays fast, since the panel now
/// recomputes on every key rather than only on TAB.
///
/// Like every test in this file, this is `#[ignore]`d -- it does NOT
/// run under a plain `cargo test`, only under the `--ignored` release
/// invocation in this file's module doc comment. Nothing in the default
/// test run guards this cost; a regression here would only be caught by
/// someone remembering to run this explicitly. Measured baselines on
/// the machine this was written on (release build): `custom_filter`
/// over 1000 candidates ~343.8µs (the OTHER test in this file); 13
/// keystrokes filtering this test's ~2000+-entry cached symbol list
/// ~6.7ms total. Both comfortably under the generous ceilings asserted
/// below -- these numbers are a reference point for "did something get
/// much slower", not a tight bound.
#[test]
#[ignore]
fn symbol_source_keystroke_filtering_stays_bounded_at_scale() {
    let (mut i, ed) = setup_perf();
    // 2000 synthetic symbols sharing a common, filterable prefix, on top
    // of whatever the interpreter already interns for its builtins --
    // real bulk for `filter_sorted`'s per-key sort+dedup to plow through.
    i.eval_source("(dotimes (n 2000) (intern (format \"se-perf-sym-%d\" n)))")
        .map_err(|e| i.describe_flow(&e))
        .unwrap();
    i.eval_source("(defun se-perf-symbol-cmd (s) (interactive \"S\") (setq result s))")
        .map_err(|e| i.describe_flow(&e))
        .unwrap();
    feed_keys(&mut i, &ed, "M-x").unwrap();
    for c in "se-perf-symbol-cmd".chars() {
        handle_key(&mut i, &ed, Key::Char(c as i64));
    }
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "the command's own \"S\" prompt should have opened"
    );

    let start = std::time::Instant::now();
    for c in "se-perf-sym-1".chars() {
        handle_key(&mut i, &ed, Key::Char(c as i64));
    }
    let elapsed = start.elapsed();

    eprintln!("13 keystrokes filtering ~2000+ cached symbols: {elapsed:?}");
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "expected 13 keystrokes' worth of filtering over the cached \
         symbol list to stay well under 500ms total (deliberately \
         generous), got {elapsed:?}"
    );
}

/// M85 fix round F8: `*search*''s live filter (search.el,
/// `search--filter-input-changed'/`search--render') redraws the ENTIRE
/// buffer from `search--results' on EVERY keystroke while a filter
/// session is active -- O(result count) per keystroke, and (unlike
/// `KEYSTROKE_HOOKS'-listed hooks, commands.rs) `minibuffer-input-
/// changed-hook' carries no per-call time budget at all. This measures
/// that cost at `search-max-results''s own default scale (10000) --
/// see search.el's own "M85 v1 does not include" section (F8) for why
/// this is recorded as a known gap rather than fixed: the acceptable
/// interactive-latency threshold is a product decision for whichever
/// future milestone addresses it, not a number this fix round can pick
/// in isolation. Like every test in this file, `#[ignore]`d -- run
/// explicitly via `cargo test --release -p core --test
/// completing_read_perf_tests -- --ignored --nocapture`. Deliberately
/// does NOT assert a pass/fail ceiling (same reasoning) -- it only
/// prints the measured cost, so a future reader has a real number
/// instead of having to reconstruct this scenario from scratch.
///
/// Bypasses the real `rg' subprocess/streaming pipeline entirely --
/// `search--results' is populated directly with 10000 synthetic
/// entries (`search--render' never touches disk or the process list,
/// only the in-memory vector and `*search*''s own buffer, see D2), and
/// `search--render' is invoked directly rather than through `handle_
/// key'/`search--filter-input-changed' (see the test body's own
/// comment for why going through that wrapper here would measure a
/// no-op instead), since the cost under measurement is entirely inside
/// `search--render' itself, not the key-dispatch path leading to it.
#[test]
#[ignore]
fn search_filter_render_cost_at_max_results_scale() {
    let (mut i, _ed) = setup_perf();
    i.eval_source("(search--ensure-output-buffer)")
        .map_err(|e| i.describe_flow(&e))
        .unwrap();
    // 10000 entries (`search-max-results''s own default), 2 of which
    // ("NEEDLE") match the filter probed below -- real work for
    // `search--matching-p''s `orderless-rank' call on every entry, not
    // a trivially-short-circuited all-nil or all-match scan.
    let build_src = "(setq search--results \
         (let ((acc nil) (n 0)) \
           (while (< n 10000) \
             (setq acc (cons (vector (format \"/tmp/f%d.v\" n) 1 1 nil \
                                      (if (< n 2) \
                                          (format \"/tmp/f%d.v:1:1:NEEDLE\" n) \
                                        (format \"/tmp/f%d.v:1:1:filler text here\" n))) \
                              acc)) \
             (setq n (1+ n))) \
           acc))";
    i.eval_source(build_src)
        .map_err(|e| i.describe_flow(&e))
        .unwrap();
    i.eval_source("(setq search--filter \"NEEDLE\")")
        .map_err(|e| i.describe_flow(&e))
        .unwrap();

    // `search--render' directly, not `search--filter-input-changed' --
    // that wrapper's own THIRD guard (F3 fix, this same round) checks
    // `(minibuffer-prompt)' against `search--filter-prompt', which is
    // nil/false here (no real minibuffer is open in this standalone
    // scale probe) and would make the wrapper a no-op, measuring
    // nothing. The cost under test is entirely inside `search--render'
    // itself; the real per-keystroke path just reaches it through that
    // wrapper.
    let start = std::time::Instant::now();
    i.eval_source("(search--render)")
        .map_err(|e| i.describe_flow(&e))
        .unwrap();
    let elapsed = start.elapsed();

    eprintln!(
        "single search--render call over 10000 search--results entries \
         (filter narrowing to 2 matches): {elapsed:?}"
    );
}

fn setup_perf() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}
