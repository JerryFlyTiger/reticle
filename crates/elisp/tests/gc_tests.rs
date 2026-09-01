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

// NOTE: (garbage-collect) only runs as a bare top-level form (the
// evaluator must be otherwise quiescent for the root set to be
// complete), so these tests use multiple top-level forms per source
// string; eval_source evaluates them in sequence and returns the last.

#[test]
fn dead_circular_list_is_collected() {
    assert_eq!(
        run("(let ((a (list 1 2 3))) (setcdr (cdr (cdr a)) a) nil)
             (garbage-collect)"),
        "(1 0)"
    );
}

#[test]
fn dead_self_referential_closure_is_collected() {
    assert_eq!(
        run("(let ((f nil)) (setq f (lambda () f)) nil)
             (garbage-collect)"),
        "(1 0)"
    );
}

#[test]
fn dead_vector_cycle_is_collected() {
    assert_eq!(
        run("(let ((v (make-vector 1 nil))) (aset v 0 v) nil)
             (garbage-collect)"),
        "(1 0)"
    );
}

#[test]
fn dead_hash_table_cycle_is_collected() {
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'self h h) nil)
             (garbage-collect)"),
        "(1 0)"
    );
}

#[test]
fn mutual_cycle_between_two_cells_is_collected() {
    // a -> b -> a: both cells registered (both setcdr'd); breaking
    // either collapses the pair. freed counts doomed registry entries.
    assert_eq!(
        run(
            "(let ((a (list 'a)) (b (list 'b))) (setcdr a b) (setcdr b a) nil)
             (garbage-collect)"
        ),
        "(2 0)"
    );
}

#[test]
fn live_cycle_rooted_in_global_survives_and_works() {
    assert_eq!(
        run(
            "(progn (defvar keep (list 1 2 3)) (setcdr (cdr (cdr keep)) keep) nil)
             (garbage-collect)
             (list (car keep) (nth 4 keep) (nth 100 keep) (gc-registered-count))"
        ),
        // circular list: nth wraps around; registry keeps the live cell
        "(1 2 2 1)"
    );
}

#[test]
fn live_closure_cycle_in_function_cell_survives() {
    // Self-referential closure stored via defvar: reachable, callable.
    assert_eq!(
        run(
            "(progn (defvar self-f nil) (setq self-f (lambda () self-f)) nil)
             (garbage-collect)
             (eq (funcall self-f) self-f)"
        ),
        "t"
    );
}

#[test]
fn mutated_constants_inside_compiled_functions_survive() {
    // A quoted list literal lives in the compiled function's constant
    // pool; setcar registers it. The mark phase must traverse chunk
    // constants or the sweep would corrupt the function.
    assert_eq!(
        run(
            "(progn (defun give () '(a b)) (byte-compile 'give) (setcar (give) 'x) nil)
             (garbage-collect)
             (give)"
        ),
        "(x b)"
    );
}

#[test]
fn second_collection_finds_nothing() {
    assert_eq!(
        run("(let ((a (list 1))) (setcdr a a) nil)
             (garbage-collect)
             (garbage-collect)"),
        "(0 0)"
    );
}

#[test]
fn dry_run_reports_without_freeing() {
    // Both dry runs see the same 1 doomed object (nothing was freed in
    // between). Only the freed count is asserted: the registry also
    // carries dead-but-unpruned entries (e.g. frames from macro
    // expansion during startup auto-compilation), and dry runs prune
    // nothing, so `remaining` is noisy by design.
    let two_dry = run("(let ((a (list 1))) (setcdr a a) nil)
                       (garbage-collect t)
                       (garbage-collect t)");
    assert!(two_dry.starts_with("(1 "), "got: {}", two_dry);
    // A real collection after a dry run still frees it...
    let dry_then_real = run("(let ((a (list 1))) (setcdr a a) nil)
                             (garbage-collect t)
                             (garbage-collect)");
    assert!(dry_then_real.starts_with("(1 "), "got: {}", dry_then_real);
    // ...and afterwards there is nothing left to free.
    let real_then_real = run("(let ((a (list 1))) (setcdr a a) nil)
                              (garbage-collect)
                              (garbage-collect)");
    assert!(real_then_real.starts_with("(0 "), "got: {}", real_then_real);
}

#[test]
fn nested_garbage_collect_refuses() {
    // Anywhere but a bare top-level form, the collector declines (nil)
    // rather than run with a possibly incomplete root set.
    assert_eq!(run("(list (garbage-collect))"), "(nil)");
    assert_eq!(
        run("(progn (defun try-gc () (garbage-collect)) (try-gc))"),
        "nil"
    );
}

#[test]
fn storing_a_non_container_registers_nothing() {
    // (setcar l 9): an integer store can never create a cycle, so the
    // hot path skips registration entirely — the common integer-mutation
    // case stays zero-overhead for the GC.
    assert_eq!(
        run("(progn (defvar l (list 1 2)) (setcar l 9) nil)
             (garbage-collect)
             (list l (gc-registered-count))"),
        "((9 2) 0)"
    );
}

#[test]
fn acyclic_container_mutation_is_not_reported_as_garbage() {
    // Storing a cons registers the cell; live + acyclic means the
    // collection must free nothing and leave the data intact.
    assert_eq!(
        run("(progn (defvar l (list 1 2)) (setcar l (list 9)) nil)
             (garbage-collect)
             (list l (gc-registered-count))"),
        "(((9) 2) 1)"
    );
}

#[test]
fn cycles_through_deep_structures_collect_without_stack_overflow() {
    // A long chain ending in a back-edge: mark must be iterative.
    assert_eq!(
        run("(let ((head (list 0)))
               (let ((tail head) (i 0))
                 (while (< i 50000)
                   (setcdr tail (list i))
                   (setq tail (cdr tail))
                   (setq i (1+ i)))
                 (setcdr tail head))
               nil)
             (garbage-collect)
             (gc-registered-count)"),
        "0"
    );
}

#[test]
fn threshold_variable_exists_with_default() {
    assert_eq!(run("gc-cons-threshold"), "100000");
}
