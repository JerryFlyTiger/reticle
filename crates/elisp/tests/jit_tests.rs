use elisp::printer::prin1_to_string;

/// Eval `src` on a big-stack thread, return the printed result (or
/// "ERROR: msg").
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

/// Native-compile `name`, assert it actually became native code (not a
/// silent bytecode-only fallback), and return the printed result of
/// evaluating `body` afterward.
fn run_native(defuns: &str, name: &str, body: &str) -> String {
    let src = format!(
        "(progn {defuns} (native-compile '{name}) (if (native-compiled-function-p (symbol-function '{name})) ({body}) 'NOT-NATIVE))"
    );
    run(&src)
}

#[test]
fn tight_loop_is_eligible_and_correct() {
    let defuns = "(defun loop-sum (n)
        (let ((acc 0) (i 0))
          (while (< i n)
            (setq acc (+ acc i))
            (setq i (1+ i)))
          acc))";
    assert_eq!(run_native(defuns, "loop-sum", "loop-sum 10"), "45");
    assert_eq!(run_native(defuns, "loop-sum", "loop-sum 100"), "4950");
    assert_eq!(run_native(defuns, "loop-sum", "loop-sum 0"), "0");
}

#[test]
fn recursive_function_is_not_eligible_but_stays_correct() {
    // v1 scope: no calls except the arithmetic whitelist, so
    // self-recursion never natively compiles — but must still be 100%
    // correct on the bytecode fallback, with no behavior change.
    let src = "(progn
        (defun fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
        (native-compile 'fib)
        (list (native-compiled-function-p (symbol-function 'fib)) (fib 15)))";
    assert_eq!(run(src), "(nil 610)");
}

#[test]
fn nested_while_loops() {
    let defuns = "(defun nested (n m)
        (let ((total 0) (i 0))
          (while (< i n)
            (let ((j 0))
              (while (< j m)
                (setq total (+ total 1))
                (setq j (1+ j))))
            (setq i (1+ i)))
          total))";
    assert_eq!(run_native(defuns, "nested", "nested 3 4"), "12");
    assert_eq!(run_native(defuns, "nested", "nested 0 5"), "0");
}

#[test]
fn cond_multi_clause() {
    // The catch-all clause uses a literal 1000 rather than `t` — `t` is
    // a free variable reference (LoadFree), which is out of scope for
    // v1 (see free_variable_reference_not_eligible) and would make this
    // whole function fall back instead.
    let defuns = "(defun classify (n)
        (cond
          ((< n 0) -1)
          ((= n 0) 0)
          ((< n 10) 1)
          (1000 2)))";
    assert_eq!(run_native(defuns, "classify", "classify -5"), "-1");
    assert_eq!(run_native(defuns, "classify", "classify 0"), "0");
    assert_eq!(run_native(defuns, "classify", "classify 5"), "1");
    assert_eq!(run_native(defuns, "classify", "classify 500"), "2");
}

#[test]
fn cond_empty_body_returns_test_value() {
    let defuns = "(defun f (x) (cond ((* x 2))))";
    assert_eq!(run_native(defuns, "f", "f 5"), "10");
    assert_eq!(run_native(defuns, "f", "f 0"), "0");
    assert_eq!(run_native(defuns, "f", "f -3"), "-6");
}

#[test]
fn comparison_used_only_for_branching() {
    // The classic case a naive "carry raw SSA values across blocks"
    // translation gets wrong: `and`'s short-circuit jumps straight into
    // `if`'s own JumpIfNil at the same bytecode position, so that one
    // position is reached with a DIFFERENT underlying comparison result
    // depending on path taken — needs a real Cranelift block parameter
    // (phi node), not just a carried-forward value.
    let defuns = "(defun in-range (x lo hi)
        (if (and (>= x lo) (<= x hi)) 1 0))";
    assert_eq!(run_native(defuns, "in-range", "in-range 5 1 10"), "1");
    assert_eq!(run_native(defuns, "in-range", "in-range 50 1 10"), "0");
    assert_eq!(run_native(defuns, "in-range", "in-range 1 1 10"), "1");
    assert_eq!(run_native(defuns, "in-range", "in-range 10 1 10"), "1");
    assert_eq!(run_native(defuns, "in-range", "in-range 0 1 10"), "0");
}

#[test]
fn and_or_chains_used_for_branching() {
    let defuns = "(defun sum-if-both-positive (a b)
        (if (and (> a 0) (> b 0)) (+ a b) 0))";
    assert_eq!(
        run_native(defuns, "sum-if-both-positive", "sum-if-both-positive 3 4"),
        "7"
    );
    assert_eq!(
        run_native(defuns, "sum-if-both-positive", "sum-if-both-positive -3 4"),
        "0"
    );

    let defuns2 = "(defun neither-negative (a b)
        (if (or (< a 0) (< b 0)) 0 1))";
    assert_eq!(
        run_native(defuns2, "neither-negative", "neither-negative 1 2"),
        "1"
    );
    assert_eq!(
        run_native(defuns2, "neither-negative", "neither-negative -1 2"),
        "0"
    );
}

#[test]
fn chained_comparisons() {
    let defuns = "(defun in-open-range (a b c) (if (< a b c) 1 0))";
    assert_eq!(
        run_native(defuns, "in-open-range", "in-open-range 1 2 3"),
        "1"
    );
    assert_eq!(
        run_native(defuns, "in-open-range", "in-open-range 1 3 2"),
        "0"
    );
    assert_eq!(
        run_native(defuns, "in-open-range", "in-open-range 5 5 6"),
        "0"
    );
}

#[test]
fn overflow_bails_out_to_correct_bytecode_result() {
    let defuns = "(defun mul (a b) (* a b))";
    // Small values: native path handles it directly.
    assert_eq!(run_native(defuns, "mul", "mul 3 4"), "12");
    // Overflow: native detects it, bails, and the bytecode VM re-runs
    // the (pure) call. Since M11's bignums, the re-run PROMOTES to
    // arbitrary precision — a native-compiled function returns a
    // bignum, never a silently wrapped number and no longer an error.
    let src = "(progn
        (defun mul (a b) (* a b))
        (native-compile 'mul)
        (list (native-compiled-function-p (symbol-function 'mul))
              (mul 4611686018427387904 4)))";
    assert_eq!(run(src), "(t 18446744073709551616)");
}

#[test]
fn division_by_zero_bails_out_correctly() {
    let src = "(progn
        (defun divide (a b) (/ a b))
        (native-compile 'divide)
        (list (native-compiled-function-p (symbol-function 'divide))
              (divide 10 2)
              (condition-case e (divide 10 0) (arith-error 'caught))))";
    assert_eq!(run(src), "(t 5 caught)");
}

#[test]
fn modulo_division_by_zero_bails_out_correctly() {
    let src = "(progn
        (defun modu (a b) (% a b))
        (native-compile 'modu)
        (list (native-compiled-function-p (symbol-function 'modu))
              (modu 10 3)
              (condition-case e (modu 10 0) (arith-error 'caught))))";
    assert_eq!(run(src), "(t 1 caught)");
}

#[test]
fn negative_and_unary_minus() {
    let defuns = "(defun neg (a) (- a))";
    assert_eq!(run_native(defuns, "neg", "neg 5"), "-5");
    assert_eq!(run_native(defuns, "neg", "neg -5"), "5");
}

#[test]
fn boolean_return_value_not_eligible_but_correct() {
    // Comparisons return elisp t/nil, which this i64-only representation
    // has no room for — so a function whose OWN return value could be a
    // raw comparison result must not be natively compiled (it would
    // otherwise silently hand back the integer 0/1 instead of nil/t).
    let src = "(progn
        (defun less (a b) (< a b))
        (native-compile 'less)
        (list (native-compiled-function-p (symbol-function 'less))
              (less 1 2) (less 2 1)))";
    assert_eq!(run(src), "(nil t nil)");
}

#[test]
fn free_variable_reference_not_eligible() {
    let src = "(progn
        (defvar dyn-x 10)
        (defun f (a) (+ a dyn-x))
        (native-compile 'f)
        (list (native-compiled-function-p (symbol-function 'f)) (f 5)))";
    assert_eq!(run(src), "(nil 15)");
}

#[test]
fn optional_and_rest_params_not_eligible() {
    let src1 = "(progn
        (defun f (a &optional b) (+ a (or b 0)))
        (native-compile 'f)
        (list (native-compiled-function-p (symbol-function 'f)) (f 5) (f 5 3)))";
    assert_eq!(run(src1), "(nil 5 8)");

    let src2 = "(progn
        (defun g (a &rest rest) (+ a (length rest)))
        (native-compile 'g)
        (list (native-compiled-function-p (symbol-function 'g)) (g 5 1 2 3)))";
    assert_eq!(run(src2), "(nil 8)");
}

#[test]
fn macro_expanded_body_still_eligible() {
    // dotimes/dolist are prelude macros expanding to while+let — since
    // native compilation works from the already-macro-expanded bytecode
    // chunk, this is transparent.
    let defuns = "(defun sum-of-squares (n)
        (let ((acc 0))
          (dotimes (i n) (setq acc (+ acc (* i i))))
          acc))";
    assert_eq!(
        run_native(defuns, "sum-of-squares", "sum-of-squares 5"),
        "30"
    );
}

#[test]
fn native_compile_is_idempotent_and_correct_after_recompile() {
    let src = "(progn
        (defun f (n) (* n 2))
        (native-compile 'f)
        (native-compile 'f)
        (list (native-compiled-function-p (symbol-function 'f)) (f 21)))";
    assert_eq!(run(src), "(t 42)");
}

#[test]
fn byte_compile_then_native_compile() {
    let src = "(progn
        (defun f (n) (+ n 1))
        (byte-compile 'f)
        (native-compile 'f)
        (list (native-compiled-function-p (symbol-function 'f)) (f 41)))";
    assert_eq!(run(src), "(t 42)");
}

#[test]
fn deep_recursion_depth_limit_still_enforced_on_fallback() {
    // Not native-eligible (self-recursive), but must still hit the same
    // stack-overflow guard as everything else, not spin forever.
    let src = "(progn
        (defun loop-forever () (loop-forever))
        (native-compile 'loop-forever)
        (loop-forever))";
    assert!(run(src).starts_with("ERROR:"));
}
