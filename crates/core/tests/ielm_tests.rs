use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, handle_key, Key};
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

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

#[test]
fn a_freshly_opened_ielm_buffer_is_not_marked_modified() {
    // M71: the welcome message + first prompt are inserted via
    // `insert', which sets the modified flag -- `*ielm*' has no file to
    // save, so a `*' on it would only ever mean "you evaluated
    // something", never "you have unsaved work". `ielm--insert-prompt'
    // clears the flag after every insertion; this covers the very
    // first one.
    let (mut i, _ed) = setup();
    run(&mut i, "(ielm)");
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "a freshly opened *ielm* buffer must not be modified"
    );
}

#[test]
fn ielm_stays_unmodified_after_evaluating() {
    let (mut i, ed) = setup();
    run(&mut i, "(ielm)");
    type_str(&mut i, &ed, "(+ 1 2)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "evaluating an expression and getting a fresh prompt must not leave *ielm* modified"
    );
}

#[test]
fn ielm_stays_unmodified_after_incomplete_input() {
    // Fix round: `ielm-return''s `incomplete' branch (unbalanced
    // parens, RET pressed) is a stable state distinct from "typing
    // mid-word" -- M71's fix round added `set-buffer-modified-p' to
    // it. Assert both the flag and that the buffer really did grow a
    // line, same discipline as the dired mark tests: a "does nothing"
    // fix would pass a flag-only assertion.
    let (mut i, ed) = setup();
    run(&mut i, "(ielm)");
    let before = run(&mut i, "(buffer-string)");
    type_str(&mut i, &ed, "(+ 1");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let after = run(&mut i, "(buffer-string)");
    assert_ne!(
        after, before,
        "RET on incomplete input should insert a newline"
    );
    assert!(
        after.contains("(+ 1\\n"),
        "expected the unbalanced input followed by a newline: {}",
        after
    );
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "RET on incomplete input is a stable state (not mid-typing) and must not leave *ielm* modified"
    );
}

#[test]
fn meta_x_ielm_opens_repl_and_evaluates() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "M-x").unwrap();
    type_str(&mut i, &ed, "ielm");
    feed_keys(&mut i, &ed, "RET").unwrap();
    assert_eq!(run(&mut i, "(buffer-name)"), "\"*ielm*\"");
    let text = run(&mut i, "(buffer-string)");
    assert!(text.contains("ELISP> "), "prompt missing: {}", text);

    type_str(&mut i, &ed, "(+ 1 2)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.contains("(+ 1 2)\\n3\\nELISP> "),
        "eval output wrong: {}",
        text
    );
}

#[test]
fn ielm_multiline_input_and_errors() {
    let (mut i, ed) = setup();
    run(&mut i, "(ielm)");

    // Incomplete input continues on the next line, then evaluates whole.
    type_str(&mut i, &ed, "(+ 1");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        !text.contains("Error"),
        "incomplete input must not error: {}",
        text
    );
    type_str(&mut i, &ed, " 2)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.contains("\\n3\\nELISP> "),
        "multiline eval wrong: {}",
        text
    );

    // Runtime errors are printed into the buffer, REPL keeps going.
    type_str(&mut i, &ed, "(no-such-function-xyz)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(text.contains("*** Error:"), "error not shown: {}", text);
    type_str(&mut i, &ed, "42");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.ends_with("42\\nELISP> \""),
        "REPL should keep going: {}",
        text
    );

    // Blank input: just a fresh prompt, no error output.
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.ends_with("ELISP> \\nELISP> \""),
        "blank input should just re-prompt: {}",
        text
    );
}

#[test]
fn ielm_state_persists_across_inputs() {
    let (mut i, ed) = setup();
    run(&mut i, "(ielm)");
    type_str(&mut i, &ed, "(defun ielm-test-double (x) (* 2 x))");
    feed_keys(&mut i, &ed, "RET").unwrap();
    type_str(&mut i, &ed, "(ielm-test-double 21)");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let text = run(&mut i, "(buffer-string)");
    assert!(
        text.contains("\\n42\\nELISP> "),
        "defun did not persist: {}",
        text
    );
}
