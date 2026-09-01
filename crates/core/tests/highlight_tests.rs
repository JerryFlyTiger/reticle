//! M15 item 4: background syntax highlighting. The parse happens on a
//! real worker thread; tests drive the main-thread side by pumping
//! `core::idle_tick` until results land (bounded, ~ms in practice).

use std::cell::RefCell;
use std::rc::Rc;

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

/// Pump ticks (10ms apart, max 2s) until `pred` returns "t".
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    for _ in 0..200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

const COUNT_HL: &str = "(let ((n 0))
   (dolist (ov (overlays-in (point-min) (point-max)) nil)
     (when (overlay-get ov 'treesit-hl) (setq n (1+ n))))
   n)";

#[test]
fn background_highlight_arrives_without_blocking() {
    let (mut i, _ed) = setup("fn add(a: i32, b: i32) -> i32 {\n    a + b // sum\n}\n");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    // Enabling costs nothing on the spot: no overlays yet, the parse is
    // on the worker thread.
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // The `fn` keyword (chars 0..2, elisp positions 1..3) must carry the
    // keyword face via a treesit-hl overlay.
    let has_kw = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 3) hit)
             (when (and (overlay-get ov 'treesit-hl)
                        (eq (overlay-get ov 'face) 'font-lock-keyword-face))
               (setq hit t))))",
    );
    assert_eq!(has_kw, "t");
    // And the comment got the comment face somewhere.
    let has_comment = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in (point-min) (point-max)) hit)
             (when (eq (overlay-get ov 'face) 'font-lock-comment-face)
               (setq hit t))))",
    );
    assert_eq!(has_comment, "t");
}

#[test]
fn edits_are_rehighlighted_after_the_debounce() {
    let (mut i, _ed) = setup("fn a() {}\n");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    // Insert a string literal at the head; its face must appear.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(insert \"const S: &str = \\\"hello\\\";\\n\")");
    assert!(tick_until(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 30) hit)
             (when (eq (overlay-get ov 'face) 'font-lock-string-face)
               (setq hit t))))"
    ));
}

#[test]
fn foreign_overlays_survive_rehighlighting() {
    let (mut i, _ed) = setup("fn a() {}\n");
    run(&mut i, "(setq mine (make-overlay 1 5))");
    run(&mut i, "(overlay-put mine 'my-marker t)");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // Force a re-apply cycle via an edit, then confirm ours is intact.
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn b() {}\\n\")");
    assert!(tick_until(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 10) hit)
             (when (overlay-get ov 'my-marker) (setq hit t))))"
    ));
}

#[test]
fn overlay_volume_is_bounded_to_the_visible_region() {
    // 1000 functions, far more than a 24-row window + margin can show.
    let mut src = String::new();
    for n in 0..1000 {
        src.push_str(&format!("fn func_{:04}() {{ let x = \"s\"; }}\n", n));
    }
    let total_chars = src.chars().count();
    let (mut i, _ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // Nothing materialized anywhere near the end of the buffer: the far
    // tail is outside visible + margin.
    let far_tail = run(
        &mut i,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {} (point-max)) hit)
                 (when (overlay-get ov 'treesit-hl) (setq hit t))))",
            total_chars - 5000
        ),
    );
    assert_eq!(far_tail, "nil", "far tail should not be materialized");
    // But the visible head is.
    let head = run(
        &mut i,
        "(let (hit)
           (dolist (ov (overlays-in 1 100) hit)
             (when (overlay-get ov 'treesit-hl) (setq hit t))))",
    );
    assert_eq!(head, "t");
}

#[test]
fn typing_latency_stays_low_while_highlighting_a_large_file() {
    // The headline guarantee, measured: with background highlighting
    // enabled on a substantial buffer, a keystroke (which triggers
    // reparse scheduling, overlay adjustment, redisplay bookkeeping)
    // stays fast — the parse itself is on the other thread.
    let mut src = String::new();
    for n in 0..2000 {
        src.push_str(&format!(
            "fn func_{:04}(x: i32) -> i32 {{ x + {} }}\n",
            n, n
        ));
    }
    let (mut i, ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));

    let start = std::time::Instant::now();
    for _ in 0..30 {
        core::commands::feed_keys(&mut i, &ed, "x").unwrap();
        core::idle_tick(&mut i, std::time::Duration::ZERO);
    }
    let elapsed = start.elapsed();
    let per_key = elapsed / 30;
    eprintln!(
        "30 keystrokes over a {}-char highlighted buffer: {:?} total, {:?}/key",
        src.chars().count(),
        elapsed,
        per_key
    );
    assert!(
        per_key < std::time::Duration::from_millis(50),
        "keystroke cost {:?} — background highlighting is leaking onto the keystroke path",
        per_key
    );
}

/// M15 item 5 probe (run deliberately: `cargo test --release -p core
/// --test highlight_tests -- --ignored --nocapture`): full-grid render()
/// cost at a large window size over a highlighted buffer, to decide
/// whether incremental redisplay is worth building. Recorded in PLAN.md.
#[test]
#[ignore]
fn measure_full_redisplay_cost() {
    let mut src = String::new();
    for n in 0..2000 {
        src.push_str(&format!(
            "fn func_{:04}(x: i32) -> i32 {{ x + {} }}\n",
            n, n
        ));
    }
    let (mut i, ed) = setup(&src);
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(treesit-highlight-mode 'rust)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    for &(cols, rows) in &[(80usize, 24usize), (200, 60), (300, 100)] {
        ed.borrow_mut().frame = (cols, rows);
        core::idle_tick(&mut i, std::time::Duration::ZERO);
        let n = 200;
        let start = std::time::Instant::now();
        for _ in 0..n {
            let _ = core::redisplay::render(&i, &ed);
        }
        let per = start.elapsed() / n;
        eprintln!("render() at {}x{}: {:?} per frame", cols, rows, per);
    }
}
