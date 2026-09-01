use elisp::printer::prin1_to_string;

/// Eval `src` on a big-stack thread (recursion needs headroom), return
/// the printed result (or "ERROR: msg").
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

#[test]
fn arithmetic_and_control_flow() {
    let src = "(progn
        (defun f (n)
          (cond
            ((< n 0) 'neg)
            ((= n 0) 'zero)
            (t 'pos)))
        (byte-compile 'f)
        (list (f -5) (f 0) (f 5)))";
    assert_eq!(run(src), "(neg zero pos)");

    let src2 = "(progn
        (defun g (n)
          (let ((acc 0) (i 0))
            (while (< i n)
              (setq acc (+ acc i))
              (setq i (1+ i)))
            acc))
        (byte-compile 'g)
        (g 10))";
    assert_eq!(run(src2), "45");

    let src3 = "(progn
        (defun h (a b) (and (> a 0) (> b 0) (+ a b)))
        (byte-compile 'h)
        (list (h 1 2) (h -1 2) (h 1 -2)))";
    assert_eq!(run(src3), "(3 nil nil)");

    let src4 = "(progn
        (defun o (a b) (or a b 'fallback))
        (byte-compile 'o)
        (list (o nil 2) (o 1 2) (o nil nil)))";
    assert_eq!(run(src4), "(2 1 fallback)");

    let src5 = "(progn
        (defun p1 () (prog1 1 2 3))
        (defun p2 () (prog2 1 2 3))
        (byte-compile 'p1) (byte-compile 'p2)
        (list (p1) (p2)))";
    assert_eq!(run(src5), "(1 2)");
}

#[test]
fn cond_without_body_returns_test_value() {
    // 0 is truthy in elisp (only nil is false), so (f 0)'s test value 0
    // is itself the clause's result, not a fallthrough to nil.
    let src = "(progn
        (defun f (x) (cond ((* x 2))))
        (byte-compile 'f)
        (list (f 5) (f 0) (f -5)))";
    assert_eq!(run(src), "(10 0 -10)");

    let src2 = "(progn
        (defun f (x) (cond ((= x 99) 'matched) (nil 'unreached)))
        (byte-compile 'f)
        (list (f 99) (f 1)))";
    assert_eq!(run(src2), "(matched nil)");
}

#[test]
fn let_and_let_star() {
    let src = "(progn
        (defun f (x)
          (let ((a 1) (b (+ x 1)))
            (+ a b)))
        (byte-compile 'f)
        (f 10))";
    assert_eq!(run(src), "12");

    // let* sees earlier bindings; plain let doesn't.
    let src2 = "(progn
        (defun f ()
          (let* ((x 10) (y (+ x 1))) (+ x y)))
        (byte-compile 'f)
        (f))";
    assert_eq!(run(src2), "21");

    // Shadowing: inner let with the same name doesn't clobber the outer
    // binding's slot.
    let src3 = "(progn
        (defun f ()
          (let ((x 1))
            (let ((x 2)) (setq x 99))
            x))
        (byte-compile 'f)
        (f))";
    assert_eq!(run(src3), "1");
}

#[test]
fn recursion_and_arity_errors() {
    let src = "(progn
        (defun fib (n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
        (byte-compile 'fib)
        (fib 15))";
    assert_eq!(run(src), "610");

    let src2 = "(progn
        (defun f (a b) (+ a b))
        (byte-compile 'f)
        (f 1))";
    assert!(run(src2).starts_with("ERROR:"));

    let src3 = "(progn
        (defun loop-forever () (loop-forever))
        (byte-compile 'loop-forever)
        (loop-forever))";
    assert!(run(src3).starts_with("ERROR:"));
}

#[test]
fn optional_and_rest_params() {
    let src = "(progn
        (defun f (a &optional b &rest c) (list a b c))
        (byte-compile 'f)
        (list (f 1) (f 1 2) (f 1 2 3 4)))";
    assert_eq!(run(src), "((1 nil nil) (1 2 nil) (1 2 (3 4)))");
}

#[test]
fn closures_capture_mutably() {
    let src = "(progn
        (defun make-counter ()
          (let ((n 0))
            (lambda () (setq n (1+ n)) n)))
        (byte-compile 'make-counter)
        (let ((c (make-counter)))
          (list (funcall c) (funcall c) (funcall c))))";
    assert_eq!(run(src), "(1 2 3)");

    // Two closures from the same maker have independent state.
    let src2 = "(progn
        (defun make-counter ()
          (let ((n 0)) (lambda () (setq n (1+ n)) n)))
        (byte-compile 'make-counter)
        (let ((c1 (make-counter)) (c2 (make-counter)))
          (funcall c1) (funcall c1)
          (funcall c2)
          (list (funcall c1) (funcall c2))))";
    assert_eq!(run(src2), "(3 2)");
}

#[test]
fn nested_closure_three_deep() {
    // Nested closures two levels deep still resolve the grandparent var.
    let src = "(progn
        (defun outer (x)
          (lambda (y) (lambda (z) (+ x y z))))
        (byte-compile 'outer)
        (let ((f (outer 1)))
          (funcall (funcall f 2) 3)))";
    assert_eq!(run(src), "6");
}

#[test]
fn dynamic_binding_under_compilation() {
    let src = "(progn
        (defvar dv 'global)
        (defun reader () dv)
        (byte-compile 'reader)
        (defun caller () (let ((dv 'local)) (reader)))
        (byte-compile 'caller)
        (list (caller) dv))";
    assert_eq!(run(src), "(local global)");

    // A dynamic param (special variable used as a parameter name).
    let src2 = "(progn
        (defvar dp 'outer)
        (defun user () dp)
        (byte-compile 'user)
        (defun setter (dp) (user))
        (byte-compile 'setter)
        (setter 'inner))";
    assert_eq!(run(src2), "inner");

    // lexical-binding: nil makes even non-special params dynamic.
    let src3 = ";; -*- lexical-binding: nil -*-\n\
        (defun reader2 () x)\n\
        (defun caller2 (x) (reader2))\n\
        (byte-compile 'reader2)\n\
        (byte-compile 'caller2)\n\
        (caller2 42)";
    assert_eq!(run(src3), "42");
}

#[test]
fn condition_case_catch_unwind_protect_fallback() {
    let src = "(progn
        (defun f (n)
          (let ((acc n))
            (condition-case e
                (if (< acc 0) (error \"neg: %d\" acc) acc)
              (error (cadr e)))))
        (byte-compile 'f)
        (list (f 5) (f -5)))";
    assert_eq!(run(src), "(5 \"neg: -5\")");

    let src2 = "(progn
        (defun f (n)
          (catch 'done
            (let ((i 0))
              (while (< i 100)
                (when (= i n) (throw 'done i))
                (setq i (1+ i)))
              'never)))
        (byte-compile 'f)
        (f 7))";
    assert_eq!(run(src2), "7");

    let src3 = "(progn
        (setq log nil)
        (defun f ()
          (let ((x 1))
            (unwind-protect
                (setq x 2)
              (push x log))
            x))
        (byte-compile 'f)
        (list (f) log))";
    assert_eq!(run(src3), "(2 (2))");
}

#[test]
fn fallback_form_sees_and_writes_back_locals() {
    let src = "(progn
        (defun f ()
          (let ((acc 0))
            (condition-case nil (setq acc 99) (error nil))
            acc))
        (byte-compile 'f)
        (f))";
    assert_eq!(run(src), "99");
}

#[test]
fn macros_expand_at_compile_time() {
    let src = "(progn
        (defun f (n)
          (let ((acc nil))
            (dotimes (i n) (push (* i i) acc))
            (nreverse acc)))
        (byte-compile 'f)
        (f 5))";
    assert_eq!(run(src), "(0 1 4 9 16)");

    let src2 = "(progn
        (defun f (xs)
          (let ((sum 0))
            (dolist (x xs) (when (> x 0) (setq sum (+ sum x))))
            sum))
        (byte-compile 'f)
        (f '(1 -2 3 -4 5)))";
    assert_eq!(run(src2), "9");

    // A user-defined macro, defined before compilation, also expands.
    let src3 = "(progn
        (defmacro my-double (x) `(* 2 ,x))
        (defun f (n) (my-double n))
        (byte-compile 'f)
        (f 21))";
    assert_eq!(run(src3), "42");
}

#[test]
fn quote_and_function_forms() {
    let src = "(progn
        (defun f () '(a b c))
        (byte-compile 'f)
        (f))";
    assert_eq!(run(src), "(a b c)");

    let src2 = "(progn
        (defun sq (x) (* x x))
        (defun f () (mapcar #'sq '(1 2 3)))
        (byte-compile 'f)
        (f))";
    assert_eq!(run(src2), "(1 4 9)");
}

#[test]
fn mixed_compiled_and_interpreted_calls() {
    let src = "(progn
        (defun helper (x) (* x x))
        (byte-compile 'helper)
        (defun interpreted-caller (x) (+ 1 (helper x)))
        (defun compiled-caller (x) (+ 1 (interpreted-caller x)))
        (byte-compile 'compiled-caller)
        (list (interpreted-caller 5) (compiled-caller 5)
              (mapcar 'helper '(1 2 3))))";
    assert_eq!(run(src), "(26 27 (1 4 9))");
}

#[test]
fn closures_capture_variables_not_values() {
    // The M10 regression set: compiled closures must share BINDINGS
    // with their creator (and each other), exactly like the
    // tree-walker — not capture a snapshot of values at creation time.

    // Mutation after creation is visible to the closure.
    let src = "(progn
        (defun v () (let ((x 1)) (let ((g (lambda () x))) (setq x 2) (funcall g))))
        (byte-compile 'v)
        (v))";
    assert_eq!(run(src), "2");

    // A closure stored into its own captured variable can see itself.
    let src2 = "(progn
        (defun sr () (let ((f nil)) (setq f (lambda () f)) (eq (funcall f) f)))
        (byte-compile 'sr)
        (sr))";
    assert_eq!(run(src2), "t");

    // Two closures over the same let share one variable.
    let src3 = "(progn
        (defun pair ()
          (let ((n 0)) (cons (lambda () (setq n (1+ n)) n) (lambda () n))))
        (byte-compile 'pair)
        (let ((p (pair)))
          (funcall (car p))
          (funcall (car p))
          (funcall (cdr p))))";
    assert_eq!(run(src3), "2");

    // Mutation flows the other way too: creator sees closure's writes.
    let src4 = "(progn
        (defun w ()
          (let ((x 10))
            (let ((setter (lambda (v) (setq x v))))
              (funcall setter 42)
              x)))
        (byte-compile 'w)
        (w))";
    assert_eq!(run(src4), "42");
}

#[test]
fn tiered_auto_compilation() {
    // Tier 1: a named interpreted function byte-compiles itself after
    // AUTO_BYTE_THRESHOLD (64) calls — no byte-compile call anywhere.
    let src = "(progn
        (defun hot (n) (* n n))
        (let ((i 0)) (while (< i 100) (hot i) (setq i (1+ i))))
        (byte-code-function-p (symbol-function 'hot)))";
    assert_eq!(run(src), "t");

    // Tier 2: past AUTO_NATIVE_THRESHOLD (1024) bytecode calls, an
    // eligible pure-integer function is native-compiled automatically.
    let src2 = "(progn
        (defun hot2 (n) (* n n))
        (let ((i 0)) (while (< i 1300) (hot2 i) (setq i (1+ i))))
        (native-compiled-function-p (symbol-function 'hot2)))";
    assert_eq!(run(src2), "t");

    // An ineligible function stops at bytecode (one attempt, no error).
    let src3 = "(progn
        (defun hot3 (s) (concat s \"x\"))
        (let ((i 0)) (while (< i 1300) (hot3 \"a\") (setq i (1+ i))))
        (list (byte-code-function-p (symbol-function 'hot3))
              (native-compiled-function-p (symbol-function 'hot3))
              (hot3 \"b\")))";
    assert_eq!(run(src3), "(t nil \"bx\")");

    // Cold functions stay interpreted.
    let src4 = "(progn
        (defun cold (n) n)
        (cold 1)
        (byte-code-function-p (symbol-function 'cold)))";
    assert_eq!(run(src4), "nil");
}

#[test]
fn byte_compile_api() {
    // Compiling a symbol replaces its function cell in place.
    assert_eq!(
        run("(progn (defun f (x) (1+ x)) (byte-compile 'f) (byte-code-function-p (symbol-function 'f)))"),
        "t"
    );
    // Compiling a function value directly returns a compiled value
    // without touching any symbol.
    assert_eq!(
        run("(byte-code-function-p (byte-compile (lambda (x) (1+ x))))"),
        "t"
    );
    // Idempotent: compiling an already-compiled function is a no-op.
    // (byte-compile on a symbol always returns the symbol, matching
    // Emacs, so check the function cell rather than the return value.)
    assert_eq!(
        run("(progn (defun f (x) x) (byte-compile 'f) (byte-compile 'f) (byte-code-function-p (symbol-function 'f)))"),
        "t"
    );
    assert_eq!(
        run("(progn (defun f (x) x) (byte-compile 'f) (byte-compile 'f))"),
        "f"
    );
    // Compiling a macro is rejected.
    assert!(run("(progn (defmacro m (x) x) (byte-compile 'm))").starts_with("ERROR:"));
    // Compiling an undefined symbol signals void-function.
    assert!(run("(byte-compile 'totally-undefined-fn-xyz)").starts_with("ERROR:"));
    // Compiling a builtin is a no-op: byte-compile on a symbol returns
    // the symbol itself (matching Emacs), and the builtin still works.
    assert_eq!(run("(byte-compile 'car)"), "car");
    assert_eq!(run("(progn (byte-compile 'car) (car '(1 2)))"), "1");
}

#[test]
fn compiled_function_prints_reasonably() {
    let src = "(progn (defun f (x) x) (byte-compile 'f) (prin1-to-string (symbol-function 'f)))";
    let out = run(src);
    assert!(out.contains("compiled-function"), "got: {}", out);
    assert!(out.contains("f"), "got: {}", out);
}
