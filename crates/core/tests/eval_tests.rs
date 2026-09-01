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

#[test]
fn eval_last_sexp_at_point() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"(setq eval-test-var (+ 40 2))\")");
    feed_keys(&mut i, &ed, "C-x C-e").unwrap();
    assert_eq!(run(&mut i, "eval-test-var"), "42");
    let echo = ed.borrow().echo.clone().unwrap_or_default();
    assert_eq!(echo, "42");
}

#[test]
fn eval_defun_from_inside_body() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(insert \"(defun eval-test-fn (x)\\n  (* x 3))\\nother line\")",
    );
    // Put point inside the defun body (line 2).
    run(
        &mut i,
        "(progn (goto-char 1) (forward-line 1) (forward-char 3))",
    );
    feed_keys(&mut i, &ed, "C-M-x").unwrap();
    assert_eq!(run(&mut i, "(eval-test-fn 7)"), "21");
}

#[test]
fn eval_buffer_defines_everything() {
    let (mut i, ed) = setup();
    // Comments and a string containing "(" must not break the reader loop.
    run(
        &mut i,
        "(insert \"; a comment (with parens\\n\\
(defun ev-one () 1)\\n\\
(defvar ev-str \\\"has ( inside\\\")\\n\\
(defun ev-two () 2)\\n\")",
    );
    run(&mut i, "(eval-buffer)");
    assert_eq!(run(&mut i, "(ev-one)"), "1");
    assert_eq!(run(&mut i, "(ev-two)"), "2");
    assert_eq!(run(&mut i, "ev-str"), "\"has ( inside\"");
    let _ = ed;
}

#[test]
fn eval_region_only_covers_region() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"(defvar ev-a 1)\\n(defvar ev-b 2)\\n\")");
    // Region = first line only.
    run(&mut i, "(eval-region 1 16)");
    assert_eq!(run(&mut i, "ev-a"), "1");
    assert_eq!(
        run(&mut i, "ev-b"),
        "ERROR: Symbol's value as variable is void: ev-b"
    );
}

#[test]
fn read_from_string_positions() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(read-from-string \"(a b) c\")"), "((a b) . 5)");
    assert_eq!(run(&mut i, "(read-from-string \"(a b) c\" 5)"), "(c . 7)");
    assert!(run(&mut i, "(read-from-string \"  \")").starts_with("ERROR:"));
}

#[test]
fn forward_and_backward_sexp() {
    let (mut i, _ed) = setup();
    run(&mut i, "(insert \"(a \\\"x)y\\\" b) tail\")");
    run(&mut i, "(goto-char 1)");
    run(&mut i, "(forward-sexp)");
    // The group is 11 chars: point at 12 (1-based).
    assert_eq!(run(&mut i, "(point)"), "12");
    run(&mut i, "(backward-sexp)");
    assert_eq!(run(&mut i, "(point)"), "1");
    // Unbalanced: error, point unmoved.
    run(
        &mut i,
        "(progn (erase-buffer) (insert \"(a b\") (goto-char 1))",
    );
    assert!(run(&mut i, "(forward-sexp)").starts_with("ERROR:"));
    assert_eq!(run(&mut i, "(point)"), "1");
}
