//! M15 item 1: the cooperative-interruption deadline. These tests pin
//! the exact semantics the hook watchdog (M15 item 2) depends on:
//! interruption fires in both the tree-walker and the VM, cleanups run,
//! and — critically — `ignore-errors` cannot swallow the timeout.

use elisp::printer::prin1_to_string;

fn run(src: &str) -> String {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut interp = elisp::new_interp();
            match interp.eval_source(&src) {
                Ok(v) => prin1_to_string(&interp, &v),
                Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
            }
        })
        .expect("spawn failed")
        .join()
        .expect("eval thread panicked")
}

/// Like `run` but also enforces a wall-clock bound: if interruption were
/// broken, the infinite loops in these tests would hang the whole suite,
/// so every call must come back well before `max` elapses.
fn run_bounded(src: &str, max: std::time::Duration) -> String {
    let start = std::time::Instant::now();
    let r = run(src);
    let elapsed = start.elapsed();
    assert!(
        elapsed < max,
        "took {:?}, budget-interruption is not working (result: {})",
        elapsed,
        r
    );
    r
}

#[test]
fn infinite_interpreted_loop_is_interrupted() {
    let r = run_bounded(
        "(run-with-deadline 50 (lambda () (while t)))",
        std::time::Duration::from_secs(5),
    );
    assert!(
        r.contains("time budget"),
        "expected elisp-timeout, got {}",
        r
    );
}

#[test]
fn infinite_bytecode_loop_is_interrupted() {
    // byte-compile first so the loop runs in the VM dispatch loop, not
    // the tree-walker — both interruption points must work.
    let r = run_bounded(
        "(defun spin-vm () (let ((n 0)) (while t (setq n (+ n 1)))))
         (byte-compile 'spin-vm)
         (run-with-deadline 50 (lambda () (spin-vm)))",
        std::time::Duration::from_secs(5),
    );
    assert!(
        r.contains("time budget"),
        "expected elisp-timeout, got {}",
        r
    );
}

#[test]
fn unwind_protect_cleanup_runs_on_timeout() {
    let r = run_bounded(
        "(setq cleaned nil)
         (condition-case nil
             (run-with-deadline 50 (lambda () (unwind-protect (while t) (setq cleaned t))))
           (elisp-timeout nil))
         cleaned",
        std::time::Duration::from_secs(5),
    );
    assert_eq!(r, "t");
}

#[test]
fn ignore_errors_cannot_swallow_the_timeout() {
    // The load-bearing semantic: elisp-timeout is NOT a child of `error`,
    // so a hook body wrapped in ignore-errors still gets interrupted and
    // the timeout still propagates to the watchdog.
    let r = run_bounded(
        "(condition-case nil
             (run-with-deadline 50 (lambda () (ignore-errors (while t)) 'swallowed))
           (elisp-timeout 'escaped))",
        std::time::Duration::from_secs(5),
    );
    assert_eq!(r, "escaped");
}

#[test]
fn explicit_handler_can_catch_it() {
    let r = run_bounded(
        "(condition-case nil
             (run-with-deadline 50 (lambda () (while t)))
           (elisp-timeout 'caught))",
        std::time::Duration::from_secs(5),
    );
    assert_eq!(r, "caught");
}

#[test]
fn normal_completion_passes_the_value_through() {
    assert_eq!(run("(run-with-deadline 5000 (lambda () (+ 40 2)))"), "42");
    // And the deadline is disarmed afterwards: follow-up work is unbudgeted.
    assert_eq!(
        run("(run-with-deadline 5000 (lambda () 1)) (let ((n 0)) (dotimes (_ 100000) (setq n (1+ n))) n)"),
        "100000"
    );
}

#[test]
fn nested_budget_can_only_shrink() {
    // The inner 60-second "budget" must not extend the outer 50ms one.
    let r = run_bounded(
        "(run-with-deadline 50 (lambda () (run-with-deadline 60000 (lambda () (while t)))))",
        std::time::Duration::from_secs(5),
    );
    assert!(
        r.contains("time budget"),
        "expected elisp-timeout, got {}",
        r
    );
}

#[test]
fn timeout_in_recursion_unwinds_cleanly() {
    // Deep recursion + timeout: the signal must unwind through many
    // frames without corrupting depth bookkeeping — a follow-up eval in
    // the same interpreter must work normally.
    let r = run_bounded(
        "(defun deep-spin (n) (if (> n 0) (deep-spin (- n 1)) (while t)))
         (condition-case nil
             (run-with-deadline 50 (lambda () (deep-spin 500)))
           (elisp-timeout nil))
         (+ 1 2)",
        std::time::Duration::from_secs(5),
    );
    assert_eq!(r, "3");
}
