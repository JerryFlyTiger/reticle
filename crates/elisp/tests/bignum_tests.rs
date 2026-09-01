//! Bignum (M11 item 2): integer overflow promotes to arbitrary
//! precision (Emacs 27+ semantics) instead of signaling arith-error.

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

#[test]
fn overflow_promotes_instead_of_erroring() {
    assert_eq!(run("(* 4611686018427387904 4)"), "18446744073709551616");
    assert_eq!(run("(1+ 9223372036854775807)"), "9223372036854775808");
    assert_eq!(run("(1- -9223372036854775808)"), "-9223372036854775809");
    assert_eq!(
        run("(+ 9223372036854775807 9223372036854775807)"),
        "18446744073709551614"
    );
    assert_eq!(run("(- -9223372036854775808)"), "9223372036854775808");
    assert_eq!(run("(abs -9223372036854775808)"), "9223372036854775808");
}

#[test]
fn factorial_30_interpreted_compiled_and_tiered() {
    let expected = "265252859812191058636308480000000";
    let defun = "(defun fact (n) (if (< n 2) 1 (* n (fact (- n 1)))))";
    assert_eq!(run(&format!("(progn {} (fact 30))", defun)), expected);
    assert_eq!(
        run(&format!("(progn {} (byte-compile 'fact) (fact 30))", defun)),
        expected
    );
}

#[test]
fn native_compiled_overflow_bails_to_bignum() {
    // The whole tier chain: native machine code hits the overflow flag,
    // bails out, the bytecode VM re-runs the (pure) call, and the
    // arithmetic promotes — a native-compiled function returns a bignum.
    let src = "(progn
        (defun mul (a b) (* a b))
        (native-compile 'mul)
        (list (native-compiled-function-p (symbol-function 'mul))
              (mul 3 4)
              (mul 4611686018427387904 4)))";
    assert_eq!(run(src), "(t 12 18446744073709551616)");
}

#[test]
fn reader_and_printer_roundtrip() {
    assert_eq!(
        run("123456789012345678901234567890"),
        "123456789012345678901234567890"
    );
    assert_eq!(
        run("-123456789012345678901234567890"),
        "-123456789012345678901234567890"
    );
    // Symbols that merely contain digits stay symbols.
    assert_eq!(
        run("(progn (setq x1234567890123456789012345 'sym) x1234567890123456789012345)"),
        "sym"
    );
}

#[test]
fn canonicalization_back_to_fixnum() {
    // A bignum result that fits in i64 must come back as a fixnum —
    // observable via eq (fixnums are eq by value).
    assert_eq!(
        run("(eq (- (1+ 9223372036854775807) 1) 9223372036854775807)"),
        "t"
    );
    assert_eq!(run("(- 18446744073709551616 18446744073709551616)"), "0");
}

#[test]
fn comparisons_are_exact_not_float_lossy() {
    // These two differ only below f64 precision: exact compare required.
    assert_eq!(run("(< 18446744073709551616 18446744073709551617)"), "t");
    assert_eq!(run("(= 18446744073709551616 18446744073709551617)"), "nil");
    assert_eq!(run("(< 5 18446744073709551616)"), "t");
    assert_eq!(run("(> -18446744073709551616 -18446744073709551617)"), "t");
    assert_eq!(
        run("(max 5 18446744073709551616 7)"),
        "18446744073709551616"
    );
    assert_eq!(run("(min 5 18446744073709551616 7)"), "5");
}

#[test]
fn equality_predicates_and_types() {
    assert_eq!(run("(let ((b (* 99999999999 99999999999))) (list (integerp b) (numberp b) (floatp b) (equal b (* 99999999999 99999999999)) (eq b b)))"),
        "(t t nil t t)");
}

#[test]
fn modulo_and_division() {
    assert_eq!(run("(% (* 99999999999 99999999999) 7)"), "2");
    assert_eq!(run("(mod (- (* 99999999999 99999999999)) 7)"), "5");
    assert_eq!(run("(/ 18446744073709551616 2)"), "9223372036854775808");
    assert_eq!(run("(/ 18446744073709551616 18446744073709551616)"), "1");
    // Division by exact zero still signals.
    assert!(run("(/ 18446744073709551616 0)").starts_with("ERROR:"));
}

#[test]
fn float_mixing() {
    assert_eq!(run("(floatp (+ 18446744073709551616 1.5))"), "t");
    assert_eq!(run("(floatp (float 18446744073709551616))"), "t");
    assert_eq!(
        run("(truncate 18446744073709551616)"),
        "18446744073709551616"
    );
}

#[test]
fn string_conversions() {
    assert_eq!(
        run("(number-to-string (* 4611686018427387904 4))"),
        "\"18446744073709551616\""
    );
    assert_eq!(
        run("(string-to-number \"18446744073709551616\")"),
        "18446744073709551616"
    );
    // Must not go through f64 (which would round).
    assert_eq!(
        run("(string-to-number \"18446744073709551617\")"),
        "18446744073709551617"
    );
    assert_eq!(
        run("(format \"%s\" (* 4611686018427387904 4))"),
        "\"18446744073709551616\""
    );
}

#[test]
fn iterative_bignum_fib_100() {
    // fib(100) = 354224848179261915075 — loops + bignums together.
    let src = "(progn
        (defun fib-iter (n)
          (let ((a 0) (b 1) (i 0))
            (while (< i n)
              (let ((tmp (+ a b)))
                (setq a b)
                (setq b tmp))
              (setq i (1+ i)))
            a))
        (fib-iter 100))";
    assert_eq!(run(src), "354224848179261915075");
}

#[test]
fn print_read_roundtrip() {
    // The worker protocol serializes with prin1 and parses with the
    // reader; verify the text roundtrip for bignums at that level.
    assert_eq!(
        run("(string-to-number (prin1-to-string (* 99999999999 99999999999)))"),
        "9999999999800000000001"
    );
}
