use elisp::printer::prin1_to_string;

/// Eval `src`, return printed result (or "ERROR: msg").
/// Runs on a big-stack thread, matching how the real binary evaluates.
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
fn arithmetic() {
    assert_eq!(run("(+ 1 2 3)"), "6");
    assert_eq!(run("(- 10 3 2)"), "5");
    assert_eq!(run("(- 5)"), "-5");
    assert_eq!(run("(* 2 3 4)"), "24");
    assert_eq!(run("(/ 10 3)"), "3");
    assert_eq!(run("(/ 10.0 4)"), "2.5");
    assert_eq!(run("(% 10 3)"), "1");
    assert_eq!(run("(mod -1 5)"), "4");
    assert_eq!(run("(+ 1 2.5)"), "3.5");
    assert_eq!(run("(max 3 1 4 1 5)"), "5");
    assert_eq!(run("(min 3 1 4)"), "1");
    assert_eq!(run("(abs -7)"), "7");
    assert!(run("(/ 1 0)").starts_with("ERROR:"));
}

#[test]
fn comparisons() {
    assert_eq!(run("(< 1 2 3)"), "t");
    assert_eq!(run("(< 1 3 2)"), "nil");
    assert_eq!(run("(= 2 2.0)"), "t");
    assert_eq!(run("(>= 3 3 2)"), "t");
    assert_eq!(run("(/= 1 2)"), "t");
}

#[test]
fn lists() {
    assert_eq!(run("(car '(1 2 3))"), "1");
    assert_eq!(run("(cdr '(1 2 3))"), "(2 3)");
    assert_eq!(run("(cons 1 2)"), "(1 . 2)");
    assert_eq!(run("(list 1 2 3)"), "(1 2 3)");
    assert_eq!(run("(length '(a b c))"), "3");
    assert_eq!(run("(nth 1 '(a b c))"), "b");
    assert_eq!(run("(append '(1 2) '(3 4))"), "(1 2 3 4)");
    assert_eq!(run("(append '(1) 2)"), "(1 . 2)");
    assert_eq!(run("(reverse '(1 2 3))"), "(3 2 1)");
    assert_eq!(run("(assq 'b '((a . 1) (b . 2)))"), "(b . 2)");
    assert_eq!(
        run("(assoc \"b\" '((\"a\" . 1) (\"b\" . 2)))"),
        "(\"b\" . 2)"
    );
    assert_eq!(run("(memq 'c '(a b c d))"), "(c d)");
    assert_eq!(run("(member \"b\" '(\"a\" \"b\"))"), "(\"b\")");
    assert_eq!(run("(last '(1 2 3))"), "(3)");
    assert_eq!(run("(delete 2 '(1 2 3 2))"), "(1 3)");
    assert_eq!(run("(sort '(3 1 2) #'<)"), "(1 2 3)");
    assert_eq!(run("(setcar (list 1 2) 9)"), "9");
    assert_eq!(run("(let ((l (list 1 2))) (setcar l 9) l)"), "(9 2)");
}

#[test]
fn strings() {
    assert_eq!(run("(concat \"foo\" \"bar\")"), "\"foobar\"");
    assert_eq!(run("(substring \"hello\" 1 3)"), "\"el\"");
    assert_eq!(run("(substring \"hello\" -2)"), "\"lo\"");
    assert_eq!(run("(upcase \"abc\")"), "\"ABC\"");
    assert_eq!(run("(string= \"a\" \"a\")"), "t");
    assert_eq!(run("(split-string \"a b  c\")"), "(\"a\" \"b\" \"c\")");
    assert_eq!(run("(split-string \"a,b\" \",\")"), "(\"a\" \"b\")");
    assert_eq!(run("(string-join '(\"a\" \"b\") \"-\")"), "\"a-b\"");
    assert_eq!(run("(format \"%s=%d\" 'x 42)"), "\"x=42\"");
    assert_eq!(run("(format \"%S\" \"q\")"), "\"\\\"q\\\"\"");
    assert_eq!(run("(length \"中文字\")"), "3");
    assert_eq!(run("(substring \"中文字\" 0 2)"), "\"中文\"");
    assert_eq!(run("(string-to-number \"42\")"), "42");
    assert_eq!(run("(number-to-string 3.5)"), "\"3.5\"");
}

#[test]
fn special_forms() {
    assert_eq!(run("(if t 1 2)"), "1");
    assert_eq!(run("(if nil 1 2 3)"), "3");
    assert_eq!(run("(cond (nil 1) (t 2))"), "2");
    assert_eq!(run("(cond ((= 1 2) 'a))"), "nil");
    assert_eq!(run("(and 1 2 3)"), "3");
    assert_eq!(run("(and 1 nil 3)"), "nil");
    assert_eq!(run("(or nil nil 2)"), "2");
    assert_eq!(run("(or)"), "nil");
    assert_eq!(run("(and)"), "t");
    assert_eq!(run("(progn 1 2 3)"), "3");
    assert_eq!(run("(prog1 1 2 3)"), "1");
    assert_eq!(run("(prog2 1 2 3)"), "2");
    assert_eq!(run("(quote (a b))"), "(a b)");
    assert_eq!(run("(let ((x 5)) (while (> x 0) (setq x (- x 1))) x)"), "0");
}

#[test]
fn let_and_scoping() {
    assert_eq!(run("(let ((x 1) (y 2)) (+ x y))"), "3");
    // let evaluates values in the outer scope; let* sees earlier bindings.
    assert_eq!(run("(let ((x 1)) (let ((x 10) (y x)) (+ x y)))"), "11");
    assert_eq!(run("(let* ((x 10) (y x)) (+ x y))"), "20");
    // Lexical closures capture variables.
    assert_eq!(
        run("(progn (defun make-adder (n) (lambda (x) (+ x n))) (funcall (make-adder 5) 10))"),
        "15"
    );
    // Closures share mutable state.
    assert_eq!(
        run(
            "(let* ((counter (let ((n 0)) (lambda () (setq n (1+ n)) n))))
               (funcall counter) (funcall counter) (funcall counter))"
        ),
        "3"
    );
}

#[test]
fn dynamic_binding() {
    // defvar'd variables bind dynamically even under lexical-binding.
    assert_eq!(
        run("(progn
               (defvar my-var 'global)
               (defun get-it () my-var)
               (let ((my-var 'local)) (get-it)))"),
        "local"
    );
    // ...and the binding is undone after let exits.
    assert_eq!(
        run("(progn
               (defvar my-var2 'global)
               (let ((my-var2 'local)) nil)
               my-var2)"),
        "global"
    );
    // defvar does not override an existing value.
    assert_eq!(run("(progn (setq foo 1) (defvar foo 2) foo)"), "1");
    // Non-special vars are invisible to called functions under lexical binding.
    assert_eq!(
        run("(progn
               (defun try-read () (if (boundp 'lex-x) lex-x 'unbound))
               (let ((lex-x 42)) (try-read)))"),
        "unbound"
    );
}

#[test]
fn functions_and_macros() {
    assert_eq!(run("(progn (defun sq (x) (* x x)) (sq 7))"), "49");
    assert_eq!(run("(funcall #'+ 1 2)"), "3");
    assert_eq!(run("(apply #'+ 1 '(2 3))"), "6");
    assert_eq!(run("(apply #'+ '())"), "0");
    assert_eq!(run("(mapcar #'1+ '(1 2 3))"), "(2 3 4)");
    assert_eq!(run("(mapcar (lambda (x) (* x 2)) '(1 2 3))"), "(2 4 6)");
    assert_eq!(run("(mapconcat #'symbol-name '(a b) \"-\")"), "\"a-b\"");
    // &optional and &rest.
    assert_eq!(
        run("(progn (defun f (a &optional b) (list a b)) (f 1))"),
        "(1 nil)"
    );
    assert_eq!(
        run("(progn (defun g (a &rest r) (cons a r)) (g 1 2 3))"),
        "(1 2 3)"
    );
    // Macros.
    assert_eq!(
        run("(progn (defmacro my-twice (form) `(progn ,form ,form))
                    (setq n 0) (my-twice (setq n (1+ n))) n)"),
        "2"
    );
    assert_eq!(run("(macroexpand-1 '(when t 1))"), "(if t (progn 1))");
    // ((lambda ...) ...) direct call.
    assert_eq!(run("((lambda (x) (* x 3)) 4)"), "12");
    // Docstrings are stripped from the body.
    assert_eq!(run("(progn (defun d () \"doc\" 42) (d))"), "42");
    assert_eq!(run("(progn (defun e () \"just-doc\") (e))"), "\"just-doc\"");
}

#[test]
fn backquote() {
    assert_eq!(run("(let ((x 2)) `(1 ,x 3))"), "(1 2 3)");
    assert_eq!(run("(let ((xs '(2 3))) `(1 ,@xs 4))"), "(1 2 3 4)");
    assert_eq!(run("`(a b c)"), "(a b c)");
    assert_eq!(run("(let ((x 1)) `(a . ,x))"), "(a . 1)");
    assert_eq!(run("(let ((x 5)) `[1 ,x])"), "[1 5]");
    assert_eq!(run("(let ((x 1)) `(a (b ,x)))"), "(a (b 1))");
}

#[test]
fn errors_and_control() {
    assert_eq!(
        run("(condition-case nil (error \"boom\") (error 'caught))"),
        "caught"
    );
    assert_eq!(
        run("(condition-case e (error \"boom %d\" 7) (error (cadr e)))"),
        "\"boom 7\""
    );
    // Error hierarchy: arith-error is caught by `error`.
    assert_eq!(
        run("(condition-case nil (/ 1 0) (error 'caught))"),
        "caught"
    );
    assert_eq!(
        run("(condition-case nil (/ 1 0) (arith-error 'specific))"),
        "specific"
    );
    // Uncaught by non-matching handler.
    assert!(run("(condition-case nil (error \"x\") (arith-error 'no))").starts_with("ERROR:"));
    // catch/throw.
    assert_eq!(run("(catch 'tag (throw 'tag 42) 99)"), "42");
    assert_eq!(run("(catch 'tag 1 2 3)"), "3");
    // unwind-protect runs cleanup on both paths.
    assert_eq!(
        run("(progn (setq log nil)
               (ignore-errors (unwind-protect (error \"x\") (push 'cleanup log)))
               log)"),
        "(cleanup)"
    );
    assert_eq!(run("(unwind-protect 'val (list 1))"), "val");
    // void-variable / void-function.
    assert!(run("undefined-var-xyz").starts_with("ERROR:"));
    assert!(run("(undefined-fn-xyz)").starts_with("ERROR:"));
    // user-defined errors via define-error.
    assert_eq!(
        run("(progn (define-error 'my-err \"My error\")
               (condition-case nil (signal 'my-err nil) (my-err 'got-it)))"),
        "got-it"
    );
}

#[test]
fn prelude_macros() {
    assert_eq!(run("(when t 1 2)"), "2");
    assert_eq!(run("(when nil 1 2)"), "nil");
    assert_eq!(run("(unless nil 'yes)"), "yes");
    assert_eq!(
        run("(let ((acc nil)) (dolist (x '(1 2 3) acc) (push (* x x) acc)))"),
        "(9 4 1)"
    );
    assert_eq!(
        run("(let ((n 0)) (dotimes (i 5) (setq n (+ n i))) n)"),
        "10"
    );
    assert_eq!(run("(let ((l '(1 2))) (pop l))"), "1");
    assert_eq!(run("(let ((l '(2))) (push 1 l) l)"), "(1 2)");
    assert_eq!(run("(ignore-errors (error \"x\"))"), "nil");
    assert_eq!(run("(zerop 0)"), "t");
    assert_eq!(run("(alist-get 'b '((a . 1) (b . 2)))"), "2");
}

#[test]
fn hooks() {
    assert_eq!(
        run("(progn
               (setq test-log nil)
               (add-hook 'my-hook (lambda () (push 'a test-log)))
               (add-hook 'my-hook (lambda () (push 'b test-log)))
               (run-hooks 'my-hook)
               test-log)"),
        "(a b)"
    );
}

#[test]
fn symbols_and_plists() {
    assert_eq!(run("(symbol-name 'foo)"), "\"foo\"");
    assert_eq!(run("(intern \"bar\")"), "bar");
    assert_eq!(run("(eq (intern \"x\") 'x)"), "t");
    assert_eq!(run("(keywordp :key)"), "t");
    assert_eq!(run(":self-eval"), ":self-eval");
    assert_eq!(run("(progn (put 's 'prop 42) (get 's 'prop))"), "42");
    assert_eq!(run("(progn (fset 'my-alias #'car) (my-alias '(1 2)))"), "1");
    assert_eq!(run("(progn (defalias 'al #'cdr) (al '(1 2)))"), "(2)");
    assert_eq!(run("(boundp 'never-bound-xyz)"), "nil");
    assert_eq!(run("(progn (defun ff ()) (fboundp 'ff))"), "t");
}

#[test]
fn vectors_and_hash() {
    assert_eq!(run("(aref [1 2 3] 1)"), "2");
    assert_eq!(
        run("(let ((v (make-vector 3 'x))) (aset v 0 'y) v)"),
        "[y x x]"
    );
    assert_eq!(run("(length [1 2 3])"), "3");
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'k 1 h) (gethash 'k h))"),
        "1"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash \"a\" 1 h) (gethash \"a\" h))"),
        "1"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (gethash 'missing h 'default))"),
        "default"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'k 1 h) (remhash 'k h) (hash-table-count h))"),
        "0"
    );
}

#[test]
fn equality() {
    assert_eq!(run("(eq 'a 'a)"), "t");
    assert_eq!(run("(eq \"a\" \"a\")"), "nil");
    assert_eq!(run("(equal \"a\" \"a\")"), "t");
    assert_eq!(run("(equal '(1 (2 3)) '(1 (2 3)))"), "t");
    assert_eq!(run("(eql 1.5 1.5)"), "t");
    assert_eq!(run("(equal [1 2] [1 2])"), "t");
}

#[test]
fn reader_syntax() {
    assert_eq!(run("?a"), "97");
    assert_eq!(run("?\\n"), "10");
    assert_eq!(run("?\\C-a"), "1");
    assert_eq!(run("#x10"), "16");
    assert_eq!(run("#b101"), "5");
    assert_eq!(run("'(a . b)"), "(a . b)");
    assert_eq!(run("-5"), "-5");
    assert_eq!(run("1.5e2"), "150.0");
    assert_eq!(run("'sym-with-dash"), "sym-with-dash");
    assert_eq!(run("(car '(1 . 2))"), "1");
    assert_eq!(run("'中文符號"), "中文符號");
    assert_eq!(run("\"字串\""), "\"字串\"");
}

#[test]
fn read_and_eval() {
    assert_eq!(run("(eval (read \"(+ 1 2)\"))"), "3");
    assert_eq!(run("(eval '(+ 1 2))"), "3");
    assert_eq!(run("(prin1-to-string '(a \"b\" 3))"), "\"(a \\\"b\\\" 3)\"");
}

#[test]
fn lexical_binding_cookie() {
    // With lexical-binding: nil, lambdas see dynamic bindings of plain vars.
    assert_eq!(
        run(";; -*- lexical-binding: nil -*-\n\
             (defun dyn-read () x)\n\
             (let ((x 42)) (dyn-read))"),
        "42"
    );
}

#[test]
fn depth_limit() {
    assert!(
        run("(progn (defun loop-forever () (loop-forever)) (loop-forever))").starts_with("ERROR:")
    );
}
