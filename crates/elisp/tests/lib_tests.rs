//! Tests for the M11 elisp library layer written in prelude.el itself:
//! plist helpers, setf, cl-defstruct, advice, pcase. These double as a
//! workout for the macro system, closures, and error machinery.

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

// --- plists ---

#[test]
fn plist_helpers() {
    assert_eq!(run("(plist-get '(:a 1 :b 2) :b)"), "2");
    assert_eq!(run("(plist-get '(:a 1) :missing)"), "nil");
    // plist-member distinguishes explicit nil from absent.
    assert_eq!(run("(plist-member '(:a nil :b 2) :a)"), "(:a nil :b 2)");
    assert_eq!(run("(plist-member '(:a nil) :c)"), "nil");
    assert_eq!(
        run("(let ((l (list :a 1))) (setq l (plist-put l :b 2)) (plist-get l :b))"),
        "2"
    );
    assert_eq!(
        run("(let ((l (list :a 1))) (setq l (plist-put l :a 9)) l)"),
        "(:a 9)"
    );
    assert_eq!(run("(plist-put nil :x 5)"), "(:x 5)");
}

// --- setf ---

#[test]
fn setf_places() {
    assert_eq!(run("(let ((x 1)) (setf x 42) x)"), "42");
    assert_eq!(
        run("(let ((l (list 1 2 3))) (setf (car l) 9) l)"),
        "(9 2 3)"
    );
    assert_eq!(
        run("(let ((l (list 1 2 3))) (setf (cdr l) '(8)) l)"),
        "(1 8)"
    );
    assert_eq!(
        run("(let ((l (list 1 2 3))) (setf (nth 2 l) 99) l)"),
        "(1 2 99)"
    );
    assert_eq!(
        run("(let ((v (make-vector 3 0))) (setf (aref v 1) 'hi) v)"),
        "[0 hi 0]"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (setf (gethash 'k h) 7) (gethash 'k h))"),
        "7"
    );
    // Multiple pairs in one setf.
    assert_eq!(
        run("(let ((a 0) (b 0)) (setf a 1 b 2) (list a b))"),
        "(1 2)"
    );
    // Unknown place errors cleanly.
    assert!(run("(setf (no-such-place 1) 2)").starts_with("ERROR:"));
}

// --- cl-defstruct ---

#[test]
fn defstruct_basics() {
    let src = "(progn
        (cl-defstruct point x y)
        (let ((p (make-point :x 3 :y 4)))
          (list (point-p p) (point-x p) (point-y p) (point-p 42) (point-p [1 2 3]))))";
    assert_eq!(run(src), "(t 3 4 nil nil)");
}

#[test]
fn defstruct_defaults_and_explicit_nil() {
    let src = "(progn
        (cl-defstruct job (state 'pending) owner)
        (list (job-state (make-job))
              (job-owner (make-job))
              (job-state (make-job :state 'running))
              ;; explicit nil must override a non-nil default
              (job-state (make-job :state nil))))";
    assert_eq!(run(src), "(pending nil running nil)");
}

#[test]
fn defstruct_setf_and_copy() {
    let src = "(progn
        (cl-defstruct point x y)
        (let* ((p (make-point :x 1 :y 2))
               (q (copy-point p)))
          (setf (point-x p) 10)
          (list (point-x p) (point-x q) (point-y p))))";
    assert_eq!(run(src), "(10 1 2)");
}

#[test]
fn defstruct_accessor_type_check() {
    let src = "(progn
        (cl-defstruct point x y)
        (point-x 42))";
    assert!(run(src).starts_with("ERROR:"));
    // Structs of different types don't cross-match.
    let src2 = "(progn
        (cl-defstruct point x y)
        (cl-defstruct size w h)
        (point-p (make-size :w 1 :h 2)))";
    assert_eq!(run(src2), "nil");
}

// --- advice ---

#[test]
fn advice_before_after_around_override() {
    let src = "(progn
        (setq log nil)
        (defun base (n) (push (list 'base n) log) (* n 10))
        (advice-add 'base :before (lambda (n) (push (list 'before n) log)))
        (let ((r (base 2)))
          (list r (nreverse log))))";
    assert_eq!(run(src), "(20 ((before 2) (base 2)))");

    let src2 = "(progn
        (defun base (n) (* n 10))
        (advice-add 'base :around (lambda (orig n) (+ 1 (funcall orig n))))
        (base 5))";
    assert_eq!(run(src2), "51");

    let src3 = "(progn
        (defun base (n) (* n 10))
        (advice-add 'base :override (lambda (n) 'overridden))
        (base 5))";
    assert_eq!(run(src3), "overridden");

    let src4 = "(progn
        (setq order nil)
        (defun base (n) (push 'main order) n)
        (advice-add 'base :after (lambda (n) (push 'after order)))
        (base 1)
        (nreverse order))";
    assert_eq!(run(src4), "(main after)");
}

#[test]
fn advice_remove_restores_original() {
    let src = "(progn
        (defun base (n) (* n 10))
        (setq adv (lambda (orig n) (+ 1 (funcall orig n))))
        (advice-add 'base :around adv)
        (let ((advised (base 1)))
          (advice-remove 'base adv)
          (list advised (base 1))))";
    assert_eq!(run(src), "(11 10)");
}

#[test]
fn advice_stacks_in_order() {
    // Most recently added advice runs outermost, like GNU Emacs.
    let src = "(progn
        (defun base (n) n)
        (advice-add 'base :around (lambda (orig n) (cons 'inner (funcall orig n))))
        (advice-add 'base :around (lambda (orig n) (cons 'outer (funcall orig n))))
        (base 7))";
    assert_eq!(run(src), "(outer inner . 7)");
}

// --- pcase ---

#[test]
fn pcase_literals_and_fallthrough() {
    assert_eq!(run("(pcase 5 (1 'one) (5 'five) (_ 'other))"), "five");
    assert_eq!(run("(pcase 9 (1 'one) (5 'five) (_ 'other))"), "other");
    assert_eq!(run("(pcase \"hi\" (\"no\" 1) (\"hi\" 2))"), "2");
    assert_eq!(run("(pcase :kw (:other 1) (:kw 2))"), "2");
    assert_eq!(run("(pcase nil (nil 'was-nil) (_ 'no))"), "was-nil");
    assert_eq!(run("(pcase 42 ('42 'quoted-num))"), "quoted-num");
    // No case matches -> nil.
    assert_eq!(run("(pcase 3 (1 'a) (2 'b))"), "nil");
}

#[test]
fn pcase_binding_and_guard() {
    assert_eq!(run("(pcase 7 (n (* n 2)))"), "14");
    assert_eq!(
        run("(pcase 10 ((and n (guard (> n 5))) (list 'big n)) (n (list 'small n)))"),
        "(big 10)"
    );
    assert_eq!(
        run("(pcase 3 ((and n (guard (> n 5))) (list 'big n)) (n (list 'small n)))"),
        "(small 3)"
    );
    assert_eq!(run("(pcase 4 ((pred integerp) 'int) (_ 'not-int))"), "int");
    assert_eq!(
        run("(pcase \"s\" ((pred integerp) 'int) (_ 'not-int))"),
        "not-int"
    );
}

#[test]
fn pcase_backquote_structures() {
    // Fixed symbols match with eq; ,PAT unquotes to a subpattern.
    assert_eq!(
        run("(pcase '(add 2 3) (`(add ,a ,b) (+ a b)) (_ 'no))"),
        "5"
    );
    assert_eq!(
        run("(pcase '(mul 2 3) (`(add ,a ,b) (+ a b)) (`(mul ,a ,b) (* a b)))"),
        "6"
    );
    // Nested and dotted patterns.
    assert_eq!(
        run("(pcase '(outer (inner 42)) (`(outer (inner ,x)) x))"),
        "42"
    );
    assert_eq!(run("(pcase (cons 1 2) (`(,a . ,b) (list a b)))"), "(1 2)");
    // Structure mismatch falls through.
    assert_eq!(
        run("(pcase '(add 1) (`(add ,a ,b) 'two-args) (_ 'wrong-shape))"),
        "wrong-shape"
    );
    // Literal integers inside backquote compare with equal.
    assert_eq!(run("(pcase '(1 2) (`(1 ,x) x))"), "2");
}

#[test]
fn pcase_or_and_patterns() {
    assert_eq!(run("(pcase 2 ((or 1 2 3) 'small) (_ 'big))"), "small");
    assert_eq!(run("(pcase 9 ((or 1 2 3) 'small) (_ 'big))"), "big");
    assert_eq!(
        run("(pcase 6 ((and (pred integerp) (guard (> 10 6)) n) (list 'ok n)))"),
        "(ok 6)"
    );
}

#[test]
fn pcase_evaluates_subject_once() {
    let src = "(progn
        (setq hits 0)
        (defun subject () (setq hits (+ hits 1)) 5)
        (pcase (subject) (1 'one) (5 'five))
        hits)";
    assert_eq!(run(src), "1");
}
