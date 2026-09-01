//! Sanity check: byte-compile every real function defined by our own
//! simple.el/org.el/prelude.el and make sure the compiled versions still
//! pass the normal core/org test scenarios. Not a substitute for the
//! automated test suites — a quick real-world stress test for the
//! compiler beyond its own synthetic unit tests.
use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;

fn run(interp: &mut elisp::Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => elisp::printer::prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn main() {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);

    // Since M10, init_editor already byte-compiles every shipped lisp
    // function at startup (like GNU Emacs's .elc files) — so by the
    // time we get here there are normally zero still-interpreted
    // Lambdas left to compile. Re-running byte-compile on everything
    // already compiled is still a meaningful check (idempotency), and
    // catches anything that somehow slipped through as a Lambda.
    let names: Vec<String> = interp
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            matches!(&s.function, Some(elisp::Value::Func(f))
                if matches!(f.as_ref(), elisp::value::Function::Lambda(l) if !l.is_macro)
                    || matches!(f.as_ref(), elisp::value::Function::Compiled(_)))
        })
        .map(|(_, s)| s.name.clone())
        .collect();
    println!(
        "re-compiling {} already-compiled-or-lambda functions (startup auto-compile already ran)...",
        names.len()
    );
    let mut failed = Vec::new();
    for name in &names {
        let src = format!("(byte-compile '{})", name);
        let r = run(&mut interp, &src);
        if r.starts_with("ERROR:") {
            failed.push(format!("{}: {}", name, r));
        }
    }
    if !failed.is_empty() {
        println!("COMPILE FAILURES:");
        for f in &failed {
            println!("  {}", f);
        }
        std::process::exit(1);
    }
    println!("all {} functions compiled cleanly", names.len());

    // Now re-run representative editing/org scenarios against the fully
    // compiled command layer.
    run_scenarios(&mut interp, &ed);
}

fn run_scenarios(interp: &mut elisp::Interp, ed: &Rc<RefCell<core::editor::Editor>>) {
    feed_keys(interp, ed, "h e l l o SPC w o r l d").unwrap();
    assert_eq!(run(interp, "(buffer-string)"), "\"hello world\"");
    feed_keys(interp, ed, "C-a").unwrap();
    feed_keys(interp, ed, "C-SPC").unwrap();
    for _ in 0..5 {
        feed_keys(interp, ed, "C-f").unwrap();
    }
    feed_keys(interp, ed, "C-w").unwrap();
    assert_eq!(run(interp, "(buffer-string)"), "\" world\"");
    feed_keys(interp, ed, "C-y").unwrap();
    assert_eq!(run(interp, "(buffer-string)"), "\"hello world\"");
    feed_keys(interp, ed, "C-/").unwrap();
    assert_eq!(run(interp, "(buffer-string)"), "\" world\"");

    run(interp, "(erase-buffer)");
    run(
        interp,
        "(insert \"* Heading\\nbody\\n** Child\\nchild body\\n* Next\\n\")",
    );
    run(interp, "(org-mode)");
    ed.borrow_mut().frame = (40, 10);
    run(interp, "(goto-char (point-min))");
    feed_keys(interp, ed, "TAB").unwrap();
    let grid = core::redisplay::render(interp, ed);
    let row0: String = grid.lines[0]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect();
    assert!(
        row0.trim_end().contains("..."),
        "expected folded heading, got: {}",
        row0
    );
    feed_keys(interp, ed, "C-c C-t").unwrap();
    assert!(run(interp, "(buffer-string)").starts_with("\"* TODO Heading"));

    println!("scenario checks passed with a fully byte-compiled lisp layer");
}
