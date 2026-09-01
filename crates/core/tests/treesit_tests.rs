use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

const RUST_SRC: &str =
    "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, 2);\n}\n";

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(&mut interp, &format!("(insert {:?})", src));
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

#[test]
fn language_availability() {
    let (mut i, _ed) = setup(RUST_SRC);
    assert_eq!(run(&mut i, "(treesit-language-available-p 'rust)"), "t");
    // M25: python is now bundled too (see lang_modes_tests.rs for the
    // other six); `ruby` stands in here as a language this v1 never
    // bundles.
    assert_eq!(run(&mut i, "(treesit-language-available-p 'python)"), "t");
    assert_eq!(run(&mut i, "(treesit-language-available-p 'ruby)"), "nil");
}

#[test]
fn unsupported_language_errors_cleanly() {
    let (mut i, _ed) = setup(RUST_SRC);
    let r = run(&mut i, "(treesit-parser-create 'ruby)");
    assert!(r.starts_with("ERROR"), "expected an error, got {}", r);
}

#[test]
fn parses_real_rust_source_into_a_syntax_tree() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    assert_eq!(run(&mut i, "(treesit-node-type root)"), "\"source_file\"");
    // Two top-level items: the two `fn`s.
    assert_eq!(run(&mut i, "(treesit-node-child-count root)"), "2");
    assert_eq!(
        run(&mut i, "(treesit-node-type (treesit-node-child root 0))"),
        "\"function_item\""
    );
    assert_eq!(
        run(&mut i, "(treesit-node-type (treesit-node-child root 1))"),
        "\"function_item\""
    );
}

#[test]
fn node_positions_and_text_match_the_buffer() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(&mut i, "(setq fn1 (treesit-node-child root 0))");
    // `fn add(...` starts at char 1 (the very start of the buffer).
    assert_eq!(run(&mut i, "(treesit-node-start fn1)"), "1");
    // The node's own text, sliced from the actual source, must round-trip
    // through our normal buffer-substring for the same range.
    let node_text = run(&mut i, "(treesit-node-text fn1)");
    let start = run(&mut i, "(treesit-node-start fn1)");
    let end = run(&mut i, "(treesit-node-end fn1)");
    let buf_text = run(&mut i, &format!("(buffer-substring {} {})", start, end));
    assert_eq!(node_text, buf_text);
    assert!(node_text.contains("fn add"), "node text: {}", node_text);
}

#[test]
fn child_parent_roundtrip_and_node_eq() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(&mut i, "(setq fn1 (treesit-node-child root 0))");
    run(&mut i, "(setq back (treesit-node-parent fn1))");
    assert_eq!(run(&mut i, "(treesit-node-eq root back)"), "t");
    assert_eq!(run(&mut i, "(treesit-node-parent root)"), "nil");
    assert_eq!(run(&mut i, "(treesit-node-child root 99)"), "nil");
}

#[test]
fn node_at_finds_the_smallest_enclosing_node() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    // Char 4 is the 'a' in "add" (1-based: f=1 n=2 SPC=3 a=4).
    run(&mut i, "(setq n (treesit-node-at 4 p))");
    assert_eq!(run(&mut i, "(treesit-node-type n)"), "\"identifier\"");
    assert_eq!(run(&mut i, "(treesit-node-text n)"), "\"add\"");
}

#[test]
fn query_capture_finds_both_function_names() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    let names = run(
        &mut i,
        "(mapcar (lambda (cap) (treesit-node-text (cdr cap)))
           (treesit-query-capture root \"(function_item name: (identifier) @fn.name)\"))",
    );
    assert_eq!(names, "(\"add\" \"main\")");
    let capture_names = run(
        &mut i,
        "(mapcar #'car
           (treesit-query-capture root \"(function_item name: (identifier) @fn.name)\"))",
    );
    assert_eq!(capture_names, "(fn.name fn.name)");
}

#[test]
fn reparse_reflects_buffer_edits() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-child-count (treesit-parser-root-node p))"
        ),
        "2"
    );
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn extra() {}\\n\")");
    // Same parser handle, fresh call: v1 always reparses from the current
    // buffer text (see PLAN.md M12), so this must see the new function.
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-child-count (treesit-parser-root-node p))"
        ),
        "3"
    );
}

#[test]
fn treesit_fontify_region_applies_faces_via_overlays() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(
        &mut i,
        "(treesit-fontify-region root '((\"(function_item name: (identifier) @fn.name)\" . font-lock-function-name-face)))",
    );
    let faces = run(
        &mut i,
        "(let (faces)
           (dolist (ov (overlays-in (point-min) (point-max)) faces)
             (when (overlay-get ov 'treesit-face)
               (push (overlay-get ov 'face) faces))))",
    );
    assert_eq!(
        faces,
        "(font-lock-function-name-face font-lock-function-name-face)"
    );
}
