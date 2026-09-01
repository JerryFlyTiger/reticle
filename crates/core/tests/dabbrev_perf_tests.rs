//! M31 perf probe for dabbrev completion (`C-n' / `evil-complete-next')
//! over a ~10MB buffer. Candidate collection (`dabbrev--collect' in
//! dabbrev.el) is a single `re-search-forward' sweep of the WHOLE
//! buffer for `"\\bPREFIX[A-Za-z0-9_-]+"' -- exactly the shape the P2.1
//! literal-prefix fast-skip (`crates/elisp/src/regex.rs', see
//! `regex_prefix_skip_perf_tests.rs' in the elisp crate) targets. This
//! buffer's filler text shares NOT EVEN ONE CHARACTER with the chosen
//! prefix ("quux"), so the fast-skip should let the sweep blow through
//! essentially all ~10MB of it for free, leaving only the handful of
//! planted, genuinely-matching identifiers to actually check.
//!
//! Deliberately `#[ignore]`d -- run explicitly:
//!
//!   cargo test --release -p core --test dabbrev_perf_tests -- --ignored --nocapture

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::{Interp, Value};

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// ~10MB / 200k lines of filler that shares no characters with `"quux"`
/// at all, plus a `quuxCandidateNN` identifier planted every 10,000
/// lines (20 of them, scattered through the whole buffer) -- the
/// realistic shape of "complete this identifier in a big source file":
/// huge buffer, but genuine matches are sparse.
fn make_haystack(lines: usize) -> String {
    let mut s = String::with_capacity(lines * 55);
    for n in 0..lines {
        if n % 10_000 == 0 {
            s.push_str(&format!("let quuxCandidate{:02} = 0;\n", n / 10_000));
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
fn measure_c_n_completion_over_10mb_buffer() {
    let src = make_haystack(200_000);
    let byte_len = src.len();
    let char_len = src.chars().count();
    assert!(
        byte_len > 8 * 1024 * 1024,
        "expected an 8MB+ buffer, got {byte_len} bytes"
    );

    // Insert directly through the buffer, not via the elisp reader --
    // feeding a 10MB string literal through `(insert ...)` would make
    // *test setup* slow enough to dominate (see line_number_perf_tests.rs).
    let (mut i, ed) = setup();
    {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.insert(0, &src);
        b.point = char_len; // end of buffer
    }
    // The short prefix itself is fine through the elisp reader.
    run(&mut i, "(insert \"quux\")");
    let on = run(&mut i, "(evil-mode 1)");
    assert!(!on.starts_with("ERROR"), "evil-mode 1 failed: {}", on);
    let ins = run(&mut i, "(evil-insert)"); // enter insert state so C-n is bound
    assert!(!ins.starts_with("ERROR"), "evil-insert failed: {}", ins);

    let point_before = match i.eval_source("(point)") {
        Ok(Value::Int(n)) => n,
        other => panic!("(point) didn't return an int: {:?}", other.is_ok()),
    };

    let start = std::time::Instant::now();
    feed_keys(&mut i, &ed, "C-n").unwrap();
    let elapsed = start.elapsed();

    let point_after = match i.eval_source("(point)") {
        Ok(Value::Int(n)) => n,
        other => panic!("(point) didn't return an int: {:?}", other.is_ok()),
    };
    let echoed = ed.borrow().echo.clone().unwrap_or_default();
    // Sanity check: a real candidate was found and applied (point moved
    // past just the 4-char "quux" that was there before), not a no-op
    // "No dynamic expansion..." message -- otherwise this would be
    // timing an early-exit path instead of a genuine full-buffer scan.
    assert!(
        point_after > point_before,
        "expected a real expansion (point should advance past \"quux\"), \
         got point_before={point_before} point_after={point_after} echo={echoed:?}"
    );

    eprintln!(
        "dabbrev C-n completion over {byte_len}-byte / {char_len}-char buffer: {elapsed:?} \
         (point {point_before} -> {point_after})"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "expected the P2.1 fast-skip to keep a single full-buffer dabbrev \
         collection well under 50ms, got {elapsed:?}"
    );
}
