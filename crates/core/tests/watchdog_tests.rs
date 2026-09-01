//! M15 item 2: the hook watchdog. The flagship scenario: a hook that
//! loops forever — the exact GNU Emacs failure mode where one bad
//! package makes every keystroke hang — must cost at most the budget,
//! get named in the echo area, and be evicted after three strikes.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

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

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_wd_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn infinite_hook_cannot_hang_typing_and_is_evicted_after_three_strikes() {
    let (mut i, ed) = setup();
    run(&mut i, "(defun slow-hook () (while t))");
    run(&mut i, "(add-hook 'post-command-hook 'slow-hook)");

    // Three keystrokes. Before M15 this test would never return (the
    // first keystroke would spin forever); with the watchdog each key
    // costs at most ~budget, the offender is named, and the third
    // strike evicts it.
    let start = std::time::Instant::now();
    feed_keys(&mut i, &ed, "a").unwrap();
    let after_first = start.elapsed();
    assert!(
        after_first < std::time::Duration::from_secs(2),
        "first keystroke took {:?} — watchdog not working",
        after_first
    );
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("slow-hook") && echo.contains("exceeded"),
        "offender not named in echo area: {:?}",
        echo
    );

    feed_keys(&mut i, &ed, "b").unwrap();
    feed_keys(&mut i, &ed, "c").unwrap();
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(
        echo.contains("removed"),
        "third strike should evict the hook: {:?}",
        echo
    );
    assert_eq!(run(&mut i, "(memq 'slow-hook post-command-hook)"), "nil");

    // All three keystrokes actually landed in the buffer, and the total
    // stayed near 3 × budget, nowhere near "hangs forever".
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abc\"");
    let total = start.elapsed();
    assert!(
        total < std::time::Duration::from_secs(3),
        "three keystrokes took {:?}",
        total
    );

    // Fourth key: hook gone, typing is instant and quiet.
    let t4 = std::time::Instant::now();
    feed_keys(&mut i, &ed, "d").unwrap();
    assert!(t4.elapsed() < std::time::Duration::from_millis(200));
    assert_eq!(run(&mut i, "(buffer-string)"), "\"abcd\"");
}

#[test]
fn fast_hooks_run_normally_and_accumulate_no_strikes() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq fast-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq fast-count (1+ fast-count))))",
    );
    feed_keys(&mut i, &ed, "x y z").unwrap();
    // Ran on every keystroke, never interrupted, never evicted.
    assert_eq!(run(&mut i, "(> fast-count 0)"), "t");
    assert_eq!(run(&mut i, "(length post-command-hook)"), "1");
    assert!(ed.borrow().hook_offenses.is_empty());
}

#[test]
fn budget_nil_disables_the_watchdog() {
    let (mut i, ed) = setup();
    // A hook that busy-spins ~120ms then sets a flag. Under the default
    // 50ms budget it is interrupted before the flag; with the budget
    // disabled it runs to completion.
    run(
        &mut i,
        "(defun spin-120 ()
           (let ((start (float-time)))
             (while (< (- (float-time) start) 0.12))
             (setq spin-completed t)))",
    );
    run(&mut i, "(setq spin-completed nil)");
    run(&mut i, "(add-hook 'post-command-hook 'spin-120)");

    feed_keys(&mut i, &ed, "a").unwrap();
    assert_eq!(
        run(&mut i, "spin-completed"),
        "nil",
        "should have been interrupted at 50ms"
    );

    run(&mut i, "(setq hook-time-budget nil)");
    feed_keys(&mut i, &ed, "b").unwrap();
    assert_eq!(
        run(&mut i, "spin-completed"),
        "t",
        "budget nil must disable interruption"
    );
}

#[test]
fn non_keystroke_hooks_are_not_budgeted() {
    let (mut i, _ed) = setup();
    // find-file-hook is off the keystroke path: a ~120ms hook must run
    // to completion there even under the default 50ms budget.
    run(
        &mut i,
        "(defun slow-open-hook ()
           (let ((start (float-time)))
             (while (< (- (float-time) start) 0.12))
             (setq open-hook-completed t)))",
    );
    run(&mut i, "(setq open-hook-completed nil)");
    run(&mut i, "(add-hook 'find-file-hook 'slow-open-hook)");
    let dir = Scratch::new("non_keystroke_hooks");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.txt");
    std::fs::write(&path, "hello").unwrap();
    run(
        &mut i,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "open-hook-completed"), "t");
}

#[test]
fn hook_errors_are_echoed_not_fatal() {
    let (mut i, ed) = setup();
    run(&mut i, "(defun broken-hook () (error \"boom\"))");
    run(&mut i, "(add-hook 'post-command-hook 'broken-hook)");
    feed_keys(&mut i, &ed, "a").unwrap();
    // The keystroke landed despite the error, and the error was surfaced.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"a\"");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert!(echo.contains("broken-hook"), "echo: {:?}", echo);
}
