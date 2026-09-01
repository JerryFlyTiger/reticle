//! Integration tests for the multi-process worker layer (Plan D).
//!
//! These run the actual built `reticle` binary and drive worker
//! forms through `--eval`. `CARGO_BIN_EXE_reticle` points at the
//! freshly built binary, and because a worker spawns `current_exe()
//! --worker`, that binary is exactly what the worker forms launch.

use std::process::Command;

/// Evaluate `src` in a fresh `reticle --eval` process, returning its
/// trimmed stdout.
fn eval(src: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .arg("--eval")
        .arg(src)
        .output()
        .expect("failed to run reticle");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn roundtrip_returns_ok_result() {
    assert_eq!(
        eval("(let ((w (worker-start))) (worker-eval w '(+ 1 2)) (worker-wait w))"),
        "(ok . 3)"
    );
}

#[test]
fn error_in_worker_comes_back_as_error_cons() {
    let r = eval("(let ((w (worker-start))) (worker-eval w '(/ 1 0)) (worker-wait w))");
    assert!(r.starts_with("(error . "), "got: {}", r);
    assert!(r.contains("Arithmetic"), "got: {}", r);
}

#[test]
fn worker_state_persists_across_requests() {
    // defun in one request, call it in the next — same interpreter.
    let r = eval(
        "(let ((w (worker-start)))
           (worker-eval w '(defun sq (n) (* n n)))
           (worker-wait w)
           (worker-eval w '(sq 9))
           (worker-wait w))",
    );
    assert_eq!(r, "(ok . 81)");
}

#[test]
fn multiple_sequential_results_come_back_in_order() {
    let r = eval(
        "(let ((w (worker-start)) (out nil))
           (worker-eval w '(+ 1 0))
           (worker-eval w '(+ 1 1))
           (worker-eval w '(+ 1 2))
           (push (cdr (worker-wait w)) out)
           (push (cdr (worker-wait w)) out)
           (push (cdr (worker-wait w)) out)
           (nreverse out))",
    );
    assert_eq!(r, "(1 2 3)");
}

#[test]
fn strings_with_newlines_survive_framing() {
    // Length-prefixed framing (not line-delimited) must carry embedded
    // newlines intact.
    let r = eval(
        "(let ((w (worker-start)))
           (worker-eval w '(concat \"a\\nb\\nc\"))
           (cdr (worker-wait w)))",
    );
    assert_eq!(r, "\"a\\nb\\nc\"");
}

#[test]
fn large_payload_roundtrips() {
    let r = eval(
        "(let ((w (worker-start)))
           (worker-eval w '(length (make-string 200000 ?x)))
           (worker-wait w))",
    );
    assert_eq!(r, "(ok . 200000)");
}

#[test]
fn non_serializable_result_downgrades_to_error() {
    // A closure prints as #<...>, which can't round-trip; the worker
    // must detect this and return an error rather than a broken frame.
    let r = eval("(let ((w (worker-start))) (worker-eval w '(lambda (x) x)) (worker-wait w))");
    assert!(r.starts_with("(error . "), "got: {}", r);
    assert!(r.contains("not serializable"), "got: {}", r);
}

#[test]
fn kill_makes_worker_not_alive() {
    let r = eval(
        "(let ((w (worker-start)))
           (list (worker-live-p w) (progn (worker-kill w) (worker-live-p w))))",
    );
    assert_eq!(r, "(t nil)");
}

#[test]
fn poll_is_nil_until_a_result_arrives() {
    // Immediately after start (no job sent), poll returns nil.
    let r = eval("(let ((w (worker-start))) (worker-poll w))");
    assert_eq!(r, "nil");
}

#[test]
fn worker_predicate_and_pending_count() {
    let r = eval(
        "(let ((w (worker-start)))
           (list (worker-p w)
                 (worker-p 42)
                 (progn (worker-eval w '(+ 1 1)) (worker-pending w))
                 (progn (worker-wait w) (worker-pending w))))",
    );
    assert_eq!(r, "(t nil 1 0)");
}

#[test]
fn worker_can_native_compile_internally() {
    // Each worker is a full interpreter — it can byte-compile and even
    // native-compile inside its own process.
    let r = eval(
        "(let ((w (worker-start)))
           (worker-eval w '(progn
             (defun loop-sum (n)
               (let ((acc 0) (i 0))
                 (while (< i n) (setq acc (+ acc i)) (setq i (1+ i))) acc))
             (native-compile 'loop-sum)
             (list (native-compiled-function-p (symbol-function 'loop-sum))
                   (loop-sum 1000))))
           (worker-wait w))",
    );
    assert_eq!(r, "(ok t 499500)");
}

#[test]
fn genuine_parallelism_beats_sequential() {
    // The core claim: N workers run N jobs concurrently across CPU
    // cores, so wall-clock time for the parallel dispatch is well under
    // the sum of the individual jobs. We only assert a conservative
    // bound (parallel < 70% of sequential) to stay robust on busy or
    // few-core CI machines while still failing if work were secretly
    // serialized.
    let r = eval(
        "(let ((job '(let ((acc 0) (i 0))
                       (while (< i 1000000) (setq acc (+ acc i)) (setq i (1+ i))) acc)))
           ;; sequential: one worker, two jobs back to back
           (let ((w (worker-start)) (t0 (float-time)))
             (worker-eval w job) (worker-wait w)
             (worker-eval w job) (worker-wait w)
             (worker-kill w)
             (let ((seq (- (float-time) t0)))
               ;; parallel: two workers, both jobs at once
               (let ((a (worker-start)) (b (worker-start)) (t1 (float-time)))
                 (worker-eval a job) (worker-eval b job)
                 (worker-wait a) (worker-wait b)
                 (worker-kill a) (worker-kill b)
                 (let ((par (- (float-time) t1)))
                   (if (< par (* seq 0.7)) 'parallel-confirmed
                     (list 'too-slow seq par)))))))",
    );
    assert_eq!(r, "parallel-confirmed", "parallelism not observed");
}
