//! Behavioral tests for the peephole optimizer (M10 item 4). The
//! optimizer runs on EVERY byte-compile now, so most of the existing
//! test suite already exercises it implicitly (if it broke something,
//! those would fail); these tests specifically pin down (a) that the
//! optimizations actually fire and shrink the instruction count for
//! the patterns they target, and (b) a battery of correctness checks
//! across control flow shapes, since a wrong transformation here would
//! corrupt otherwise-correct compiled functions.

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

fn compiled_instr_count(defun_src: &str, name: &str) -> usize {
    let mut interp = elisp::new_interp();
    interp.eval_source(defun_src).map_err(|_| ()).unwrap();
    let id = interp.intern(name);
    let elisp::Value::Func(f) = interp.symbols[id as usize].function.clone().unwrap() else {
        panic!("not a function")
    };
    let elisp::value::Function::Lambda(l) = f.as_ref() else {
        panic!("not interpreted")
    };
    let compiled = elisp::compiler::compile_parsed(
        &mut interp,
        l.params.clone(),
        l.body.clone(),
        l.interactive.borrow().clone(),
        l.lexical,
        l.env.clone(),
    )
    .map_err(|_| ())
    .unwrap();
    compiled.chunk.code.len()
}

#[test]
fn constant_arithmetic_folds_to_one_instruction() {
    // (+ 1 2) must collapse to a single Const, not Const;Const;Add2.
    let n = compiled_instr_count("(defun f () (+ 1 2))", "f");
    // Const(folded) + Return = 2 instructions, vs 4 unfolded.
    assert_eq!(
        n, 2,
        "expected constant folding to shrink to 2 instructions"
    );
    assert_eq!(run("(defun f () (+ 1 2)) (byte-compile 'f) (f)"), "3");
}

#[test]
fn folded_overflow_matches_runtime_promotion() {
    // Constant-fold path must promote to bignum exactly like the
    // runtime path, not silently wrap or diverge.
    assert_eq!(
        run("(defun f () (* 4611686018427387904 4)) (byte-compile 'f) (f)"),
        "18446744073709551616"
    );
}

#[test]
fn non_final_setq_drops_dead_dup_and_pop() {
    let defuns = "(defun f (n)
        (let ((acc 0))
          (setq acc (+ acc n))
          (setq acc (+ acc 1))
          acc))";
    let with_dup_pop = 3 /* Const 0 + StoreLocal + LoadLocal+LoadLocal+Add2+Dup+StoreLocal+Pop per setq */;
    let _ = with_dup_pop;
    let n = compiled_instr_count(defuns, "f");
    // Two setqs, each would cost 2 extra instructions (Dup + Pop) if
    // unoptimized. Just assert the optimized count is small enough
    // that both eliminations definitely fired (exact count pinned
    // below for a regression signal).
    assert_eq!(n, 12, "unexpected instruction count: {}", n);
    assert_eq!(
        run(&format!("(progn {} (byte-compile 'f) (f 10))", defuns)),
        "11"
    );
}

#[test]
fn optimizations_never_change_results_across_control_flow_shapes() {
    // A representative sweep: if/cond/while/and/or, nested, nothing
    // constant-only so the compiler routes through the full range of
    // opcodes the optimizer touches — must produce identical results
    // whether interpreted or byte-compiled (which now always runs
    // through peephole).
    let cases: &[(&str, &str)] = &[
        ("(defun f (n) (if (< n 0) -1 (if (= n 0) 0 1)))", "(list (f -5) (f 0) (f 5))"),
        (
            "(defun f (n) (cond ((< n 0) 'neg) ((= n 0) 'zero) (100 'pos)))",
            "(list (f -1) (f 0) (f 1))",
        ),
        (
            "(defun f (n) (let ((acc 0) (i 0)) (while (< i n) (setq acc (+ acc i)) (setq i (1+ i))) acc))",
            "(list (f 0) (f 1) (f 10))",
        ),
        (
            "(defun f (a b) (if (and (> a 0) (> b 0)) (+ a b) 0))",
            "(list (f 3 4) (f -1 4) (f 3 -1))",
        ),
        (
            "(defun f (a b) (if (or (< a 0) (< b 0)) 'neg 'ok))",
            "(list (f 1 1) (f -1 1) (f 1 -1))",
        ),
        (
            "(defun f (x lo hi) (if (and (>= x lo) (<= x hi)) 1 0))",
            "(list (f 5 1 10) (f 50 1 10) (f 1 1 10) (f 10 1 10))",
        ),
        (
            "(defun f (n) (let ((total 0) (i 0)) (while (< i n) (let ((j 0)) (while (< j n) (setq total (+ total 1)) (setq j (1+ j)))) (setq i (1+ i))) total))",
            "(f 4)",
        ),
    ];
    for (defun, call) in cases {
        let interpreted = run(&format!("(progn {} {})", defun, call));
        let compiled = run(&format!("(progn {} (byte-compile 'f) {})", defun, call));
        assert_eq!(interpreted, compiled, "mismatch for: {}", defun);
    }
}

#[test]
fn nested_closures_still_share_bindings_after_optimization() {
    // The M10 closure-capture fix depends on PushFrame/EnvDefine
    // sequences the optimizer must never disturb (it doesn't touch
    // them at all, but this pins the interaction down explicitly).
    let src = "(defun make-counter ()
        (let ((n 0)) (lambda () (setq n (1+ n)) n)))";
    assert_eq!(
        run(&format!(
            "(progn {} (byte-compile 'make-counter) (let ((c (make-counter))) (funcall c) (funcall c) (funcall c)))",
            src
        )),
        "3"
    );
}

#[test]
fn condition_case_and_fallback_forms_unaffected() {
    // Interpret-fallback instructions must survive optimization intact
    // (the optimizer only ever removes Dup/Pop/Const/Op windows, never
    // touches Interpret).
    let src = "(defun f (n)
        (let ((acc n))
          (condition-case e
              (if (< acc 0) (error \"neg\") acc)
            (error (cadr e)))))";
    assert_eq!(
        run(&format!(
            "(progn {} (byte-compile 'f) (list (f 5) (f -5)))",
            src
        )),
        "(5 \"neg\")"
    );
}

#[test]
fn deeply_nested_cond_with_many_clauses() {
    // Exercises many end_jumps all patched to the same target — a
    // scenario worth checking explicitly since jump-threading rewrites
    // targets in place.
    let src = "(defun f (n)
        (cond ((= n 1) 'a) ((= n 2) 'b) ((= n 3) 'c) ((= n 4) 'd)
              ((= n 5) 'e) ((= n 6) 'f) (100 'other)))";
    let interpreted = run(&format!("(progn {} (mapcar 'f '(1 2 3 4 5 6 7)))", src));
    let compiled = run(&format!(
        "(progn {} (byte-compile 'f) (mapcar 'f '(1 2 3 4 5 6 7)))",
        src
    ));
    assert_eq!(interpreted, compiled);
    assert_eq!(compiled, "(a b c d e f other)");
}

#[test]
fn recursive_function_with_constant_arithmetic_folds_correctly() {
    // fib itself isn't constant-foldable (n is a variable), but the
    // base-case comparisons and +/- against literals go through the
    // fold path repeatedly across many calls — a good end-to-end
    // regression check that folding composes correctly with recursion
    // and the auto-tiering call counters.
    let src = "(defun fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))";
    assert_eq!(
        run(&format!("(progn {} (byte-compile 'fib) (fib 15))", src)),
        "610"
    );
}
