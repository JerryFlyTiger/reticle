//! Hash table tests (M10 item 5): real O(1)-average IndexMap storage
//! keyed by elisp `equal` semantics, replacing the old O(n) assoc-vector
//! scan. Correctness (equal-based lookup, insertion-order iteration,
//! GC integration) matters more here than the performance number
//! (verified separately, empirically, at the shell level) since a
//! wrong Hash/Eq pairing would silently corrupt lookups rather than
//! just being slow.

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
fn basic_put_get_default() {
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'a 1 h) (gethash 'a h))"),
        "1"
    );
    assert_eq!(
        run("(gethash 'missing (make-hash-table) 'default)"),
        "default"
    );
    assert_eq!(run("(gethash 'missing (make-hash-table))"), "nil");
}

#[test]
fn equal_semantics_not_identity() {
    // Two DISTINCT string/list objects with the same contents must
    // hash and compare equal as keys — this is the whole point of
    // keying by `equal` rather than `eq`.
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash (list 1 2 3) 'found h) (gethash (list 1 2 3) h))"),
        "found"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash (concat \"a\" \"b\") 'found h) (gethash \"ab\" h))"),
        "found"
    );
    // Nested structures compare structurally too.
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash '(1 (2 3) \"x\") 'deep h) (gethash (list 1 (list 2 3) \"x\") h))"),
        "deep"
    );
    // Distinct vectors with equal contents.
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash [1 2 3] 'vec h) (gethash (vector 1 2 3) h))"),
        "vec"
    );
    // Floats compare by eql (bit pattern, not ==), matching equal's fallthrough.
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 1.5 'f h) (gethash 1.5 h))"),
        "f"
    );
}

#[test]
fn distinct_keys_do_not_collide() {
    // Same printed form, different types (int vs string) — must hash
    // and compare distinctly, not collide because they "look similar".
    assert_eq!(
        run("(let ((h (make-hash-table)))
               (puthash 1 'int h) (puthash \"1\" 'str h)
               (list (gethash 1 h) (gethash \"1\" h)))"),
        "(int str)"
    );
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash '(1 2) 'a h) (puthash '(1 3) 'b h) (list (gethash '(1 2) h) (gethash '(1 3) h)))"),
        "(a b)"
    );
}

#[test]
fn update_existing_key_preserves_position_and_count() {
    let src = "(let ((h (make-hash-table)) (out nil))
        (puthash 'a 1 h) (puthash 'b 2 h) (puthash 'a 99 h)
        (maphash (lambda (k v) (push (cons k v) out)) h)
        (list (nreverse out) (hash-table-count h)))";
    // 'a keeps its original (first) position despite being updated.
    assert_eq!(run(src), "(((a . 99) (b . 2)) 2)");
}

#[test]
fn maphash_preserves_insertion_order() {
    let src = "(let ((h (make-hash-table)) (out nil))
        (puthash 'z 1 h) (puthash 'a 2 h) (puthash 'm 3 h)
        (maphash (lambda (k v) (push (cons k v) out)) h)
        (nreverse out))";
    assert_eq!(run(src), "((z . 1) (a . 2) (m . 3))");
}

#[test]
fn remhash_removes_and_preserves_remaining_order() {
    let src = "(let ((h (make-hash-table)) (out nil))
        (puthash 'a 1 h) (puthash 'b 2 h) (puthash 'c 3 h)
        (remhash 'b h)
        (maphash (lambda (k v) (push k out)) h)
        (list (nreverse out) (hash-table-count h) (gethash 'b h 'gone)))";
    assert_eq!(run(src), "((a c) 2 gone)");
}

#[test]
fn clrhash_empties_and_resets_count() {
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'a 1 h) (puthash 'b 2 h) (clrhash h) (list (hash-table-count h) (gethash 'a h 'gone)))"),
        "(0 gone)"
    );
}

#[test]
fn hash_table_p_and_printer() {
    assert_eq!(run("(hash-table-p (make-hash-table))"), "t");
    assert_eq!(run("(hash-table-p 5)"), "nil");
    let printed = run("(let ((h (make-hash-table))) (puthash 'a 1 h) (prin1-to-string h))");
    assert!(printed.contains("hash-table"), "got: {}", printed);
    assert!(
        printed.contains("count 1") || printed.contains(":count 1"),
        "got: {}",
        printed
    );
}

#[test]
fn maphash_function_can_mutate_table_during_iteration() {
    // maphash snapshots entries before iterating, so a callback that
    // itself calls puthash/remhash on the same table must not corrupt
    // or skip the current iteration.
    let src = "(let ((h (make-hash-table)) (seen nil))
        (puthash 'a 1 h) (puthash 'b 2 h)
        (maphash (lambda (k v) (push k seen) (puthash 'c 99 h)) h)
        (list (nreverse seen) (hash-table-count h) (gethash 'c h)))";
    assert_eq!(run(src), "((a b) 3 99)");
}

#[test]
fn gc_reclaims_dead_hash_table_cycle() {
    assert_eq!(
        run("(let ((h (make-hash-table))) (puthash 'self h h) nil)
             (garbage-collect)"),
        "(1 0)"
    );
}

#[test]
fn gc_preserves_live_hash_table_cycle() {
    assert_eq!(
        run(
            "(progn (defvar keep (make-hash-table)) (puthash 'self keep keep) nil)
             (garbage-collect)
             (hash-table-count keep)"
        ),
        "1"
    );
}

#[test]
fn large_scale_correctness() {
    // Not a timing test (that's verified at the shell level separately)
    // — this pins down correctness at a scale where a hashing bug
    // (e.g. inconsistent Hash/Eq, or truncated hash) would show up as
    // wrong values rather than just being slow.
    let src = "(let ((h (make-hash-table)) (i 0) (ok t))
        (while (< i 5000) (puthash i (* i i) h) (setq i (1+ i)))
        (setq i 0)
        (while (< i 5000)
          (unless (= (gethash i h) (* i i)) (setq ok nil))
          (setq i (1+ i)))
        (list ok (hash-table-count h)))";
    assert_eq!(run(src), "(t 5000)");
}

#[test]
fn byte_compiled_hash_operations_match_interpreted() {
    let defuns = "(defun build ()
        (let ((h (make-hash-table)))
          (puthash 'x 1 h) (puthash 'y 2 h) (puthash 'x 10 h)
          (list (gethash 'x h) (gethash 'y h) (hash-table-count h))))";
    let interpreted = run(&format!("(progn {} (build))", defuns));
    let compiled = run(&format!("(progn {} (byte-compile 'build) (build))", defuns));
    assert_eq!(interpreted, compiled);
    assert_eq!(compiled, "(10 2 2)");
}
