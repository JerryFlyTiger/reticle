// M52: mark must be able to walk into `Value`s held *inside* a
// `Value::Ext` payload (keymap bindings, overlay props, buffer locals),
// not just the Ext handle itself. Before this milestone `Value::Ext(_)`
// was a mark-phase no-op, so anything reachable only through one of
// those payloads (e.g. a closure captured by a keymap binding) could be
// swept as a false cycle the moment the *editor's* explicit root walk
// (editor.buffers / global_keymap / ...) stopped covering it -- see
// PLAN.md M52 for the repro this file is built from.
//
// Every test here runs its body on a thread with an explicit large
// stack, matching this codebase's other elisp-evaluating tests (see
// crates/elisp/tests/gc_tests.rs and
// crates/core/tests/evil_ex_macro_tests.rs): the tree-walking evaluator
// can overflow a default debug-build stack.
//
// KNOWN GAP, NOT COVERED HERE: each `ExtTracer` (`trace_keymap`,
// `trace_overlay`, `trace_buffer`) returns `false` when the payload's
// `RefCell` is already borrowed, which `gc::mark` propagates to abort
// the whole collection (same convention as `RootProvider` returning
// `false`). No test here exercises that abort path: reaching it needs a
// `RefCell` to be borrowed at the exact moment `(garbage-collect)` runs
// mark, and `(garbage-collect)` only executes at `interp.depth == 1`
// (a bare top-level form, see gc.rs's own note) -- there is no elisp-
// reachable way to hold one of these borrows open across that call
// without also being mid-eval, which the depth check already forbids.
// Constructing that interleaving would need a Rust-level unit test that
// borrows the `RefCell` directly and calls `gc::collect` while holding
// it, which is out of reach from this integration-test file. This path's
// correctness is therefore unverified by any automated test right now;
// it rests on code review and structural analogy to the `RootProvider`
// abort path, which is exercised (see the editor's own root-provider
// `try_borrow` calls in `lib.rs`, covered indirectly by every test in
// this file that completes a collection at all).

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::feed_keys;
use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn run_big_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(f)
        .expect("spawn failed")
        .join()
        .expect("test thread panicked");
}

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

fn feed(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, desc: &str) {
    feed_keys(interp, ed, desc).unwrap_or_else(|e| panic!("feed_keys({:?}) failed: {}", desc, e));
}

// 1. Nested prefix keymap ("C-c a") reached via global-set-key: the
// prefix sub-keymap is itself a Value::Ext stored in the top keymap's
// entries, and the leaf binding is a closure capturing a mutable `acc`.
// GC between the two key presses must not clear the closure's LexEnv.
#[test]
fn nested_prefix_keymap_closure_survives_gc() {
    run_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(setq log nil)");
        run(
            &mut i,
            r#"(let ((acc nil))
                 (global-set-key (kbd "C-c a")
                   (lambda () (interactive)
                     (setq acc (cons 1 acc))
                     (setq log (cons (length acc) log)))))"#,
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(run(&mut i, "log"), "(1)");
        // Displace last_command so it no longer roots the lambda.
        feed(&mut i, &ed, "C-f");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(
            run(&mut i, "log"),
            "(2 1)",
            "closure's captured acc lost state across GC"
        );
    });
}

// 2. Same shape, but a single (non-prefix) key binding, to cover the
// path that doesn't go through a nested keymap Ext at all.
#[test]
fn single_key_binding_closure_survives_gc() {
    run_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(setq log nil)");
        run(
            &mut i,
            r#"(let ((acc nil))
                 (global-set-key (kbd "C-c")
                   (lambda () (interactive)
                     (setq acc (cons 1 acc))
                     (setq log (cons (length acc) log)))))"#,
        );
        feed(&mut i, &ed, "C-c");
        assert_eq!(run(&mut i, "log"), "(1)");
        feed(&mut i, &ed, "C-f");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        feed(&mut i, &ed, "C-c");
        assert_eq!(
            run(&mut i, "log"),
            "(2 1)",
            "closure's captured acc lost state across GC"
        );
    });
}

// 3. local-set-key -- binding lives in Buffer.keymap rather than
// editor.global_keymap.
#[test]
fn local_set_key_closure_survives_gc() {
    run_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(setq log nil)");
        run(
            &mut i,
            r#"(let ((acc nil))
                 (local-set-key (kbd "C-c a")
                   (lambda () (interactive)
                     (setq acc (cons 1 acc))
                     (setq log (cons (length acc) log)))))"#,
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(run(&mut i, "log"), "(1)");
        feed(&mut i, &ed, "C-f");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(
            run(&mut i, "log"),
            "(2 1)",
            "closure's captured acc lost state across GC"
        );
    });
}

// 4. A keymap that is reachable ONLY through a plain global variable
// (not editor.global_keymap, not any Buffer.keymap) -- this is what
// distinguishes a real Ext-tracer fix from a half-fix that special-cases
// mark() to walk editor.global_keymap/Buffer.keymap explicitly and
// happens to miss any keymap sitting in an arbitrary elisp variable.
// Routed to actual dispatch via buffer-local `emulation-keymap`
// (consulted by dispatch_key ahead of local/global -- commands.rs:621).
#[test]
fn keymap_reachable_only_via_global_variable_survives_gc() {
    run_big_stack(|| {
        let (mut i, ed) = setup();
        run(&mut i, "(setq log nil)");
        run(
            &mut i,
            r#"(setq my-map (make-sparse-keymap))
               (let ((acc nil))
                 (define-key my-map (kbd "C-c a")
                   (lambda () (interactive)
                     (setq acc (cons 1 acc))
                     (setq log (cons (length acc) log)))))
               (make-local-variable 'emulation-keymap)
               (setq emulation-keymap my-map)"#,
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(run(&mut i, "log"), "(1)");
        feed(&mut i, &ed, "C-f");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        feed(&mut i, &ed, "C-c a");
        assert_eq!(
            run(&mut i, "log"),
            "(2 1)",
            "closure bound only via a global-variable keymap lost state across GC"
        );
    });
}

// 5. Two keymaps bound directly into each other's entries (each is the
// raw `def` value of a key in the other -- `define-key` allows binding
// a keymap object itself, the same mechanism nested prefix keymaps use)
// form a cycle that passes through Ext -> Ext -> Ext with no Cons/Func/
// Env node in between to gate the revisit. This is the only observable
// point for the `visited.insert` guard in the Ext branch of `gc::mark`:
// a closure-mediated self-reference (keymap -> lambda -> LexEnv ->
// keymap) does NOT hit this code path, because `Value::Func`'s own
// visited-gate already stops that particular cycle -- only a cycle
// closing entirely through Ext nodes exercises the Ext-level guard.
// Without it, this test hangs instead of failing.
#[test]
fn mutually_referential_keymaps_do_not_hang_gc() {
    run_big_stack(|| {
        let (mut i, ed) = setup();
        run(
            &mut i,
            r#"(setq map-a (make-sparse-keymap))
               (setq map-b (make-sparse-keymap))
               (define-key map-a (kbd "C-c a") map-b)
               (define-key map-b (kbd "C-c b") map-a)"#,
        );
        feed(&mut i, &ed, "C-f"); // move last_command off anything relevant
        let gc = run(&mut i, "(garbage-collect)");
        assert!(
            !gc.starts_with("ERROR"),
            "collection errored instead of completing: {}",
            gc
        );
        // Both keymaps must still work post-GC.
        assert_eq!(run(&mut i, "(keymapp map-a)"), "t");
        assert_eq!(run(&mut i, "(keymapp map-b)"), "t");
    });
}

// 6. delete-overlay removes the overlay from Buffer.overlays, but an
// elisp variable can still hold the Value::Ext -- its props must
// survive a GC that happens after the delete.
#[test]
fn deleted_overlay_props_survive_gc() {
    run_big_stack(|| {
        let (mut i, _ed) = setup();
        run(&mut i, "(insert \"hello world\")");
        run(
            &mut i,
            r#"(setq ov (make-overlay 1 6))
               (overlay-put ov 'payload (list 'still 'alive))"#,
        );
        // Register the payload's *second* cons cell as a GC cycle
        // candidate via mutation -- setcdr on it is what actually calls
        // `register_value` (a fresh, never-mutated cons registers
        // nothing, see the `stored_can_cycle` pitfall) -- and make it
        // self-referential so "still reachable" is the only sound
        // outcome. The assertion below must inspect exactly this cell
        // (its `car`, 'alive): the head cons was never registered, so
        // asserting on the head alone would pass even with the cell
        // wrongly cleared.
        run(
            &mut i,
            "(setcdr (cdr (overlay-get ov 'payload)) (overlay-get ov 'payload))",
        );
        run(&mut i, "(delete-overlay ov)");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        assert_eq!(
            run(&mut i, "(cadr (overlay-get ov 'payload))"),
            "alive",
            "deleted overlay's mutated payload cell lost after GC"
        );
    });
}

// 7. A buffer killed by kill-buffer is removed from editor.buffers, so
// the pre-M52 root provider (which only walks editor.buffers) stops
// seeing its locals. If an elisp variable still holds the buffer's
// Value::Ext, a buffer-local value that got registered as a GC cycle
// candidate (via setcar creating a self-reference) must still survive.
#[test]
fn killed_but_referenced_buffer_locals_survive_gc() {
    run_big_stack(|| {
        let (mut i, _ed) = setup();
        run(&mut i, "(setq victim (generate-new-buffer \"victim\"))");
        run(
            &mut i,
            r#"(with-current-buffer victim
                 (make-local-variable 'my-local)
                 (setq my-local (list 1)))"#,
        );
        // Register the cons as a GC cycle candidate via mutation, and
        // make it self-referential so the only sound outcome is "still
        // reachable" (the alternative isn't "leaked" but "cleared").
        run(
            &mut i,
            "(with-current-buffer victim (setcar my-local my-local))",
        );
        run(&mut i, "(kill-buffer victim)");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        assert_eq!(
            run(
                &mut i,
                "(with-current-buffer victim (eq (car my-local) my-local))"
            ),
            "t",
            "killed buffer's buffer-local value was cleared as false garbage"
        );
    });
}

// 8. Negative control: adding Ext tracers must not turn Ext objects into
// conservative roots. A cons cycle that really is garbage (unreachable
// except through itself) must still be collected.
#[test]
fn genuinely_dead_cycle_still_collected_alongside_ext_tracers() {
    run_big_stack(|| {
        let (mut i, _ed) = setup();
        // An Ext (keymap) exists in this session (from init_editor's
        // global keymap) so tracer code paths are exercised, but the
        // dead cycle below has no path through any Ext at all.
        assert_eq!(
            run(
                &mut i,
                "(let ((a (list 1 2 3))) (setcdr (cdr (cdr a)) a) nil)
                 (garbage-collect)"
            ),
            "(1 0)",
            "a genuinely dead cycle was not collected once Ext tracers exist"
        );
    });
}

// 9. `overlays-in` builds a *second* `Value::Ext` around an existing
// overlay (ui.rs's other OVERLAY_TAG construction site). Tests 6 covers
// only the handle `make-overlay` returns, so that site's `trace` field
// had no observer -- flipping it to `None` left the whole suite green.
// Here the `make-overlay` handle is deliberately discarded and the only
// retained reference comes back out of `overlays-in`.
#[test]
fn overlays_in_result_props_survive_gc() {
    run_big_stack(|| {
        let (mut i, _ed) = setup();
        run(&mut i, "(setq buf (generate-new-buffer \"ovin\"))");
        run(&mut i, "(with-current-buffer buf (insert \"hello\"))");
        // Return value dropped on purpose: the Ext handle under test must
        // be the one `overlays-in` mints, not this one.
        run(&mut i, "(with-current-buffer buf (make-overlay 1 3))");
        run(
            &mut i,
            "(setq ov (car (with-current-buffer buf (overlays-in 1 5))))",
        );
        run(&mut i, "(overlay-put ov 'payload (list 'still 'alive))");
        // Same registration pitfall as test 6: only the *mutated* cell is
        // a sweep candidate, so the assertion must land on that cell.
        run(
            &mut i,
            "(setcdr (cdr (overlay-get ov 'payload)) (overlay-get ov 'payload))",
        );
        // Detach from the buffer so the editor's root provider stops
        // covering these props and the Ext handle is the only path left.
        run(&mut i, "(delete-overlay ov)");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        assert_eq!(
            run(&mut i, "(cadr (overlay-get ov 'payload))"),
            "alive",
            "props of an overlay obtained via overlays-in lost after GC"
        );
    });
}

// 10. `trace_buffer` walks the buffer's *overlays* as well as its
// locals. Test 7 only exercises the locals arm, so deleting the overlay
// loop entirely left the suite green. Here the overlay stays attached to
// a killed buffer (so the root provider no longer reaches it) and no
// elisp variable holds the overlay handle -- the only path to the props
// is buffer Ext -> b.overlays -> props.
#[test]
fn killed_buffer_overlay_props_survive_gc() {
    run_big_stack(|| {
        let (mut i, _ed) = setup();
        run(&mut i, "(setq buf (generate-new-buffer \"ovkill\"))");
        run(&mut i, "(with-current-buffer buf (insert \"hello\"))");
        run(
            &mut i,
            "(setq tmp (with-current-buffer buf (make-overlay 1 3)))",
        );
        run(&mut i, "(overlay-put tmp 'payload (list 'still 'alive))");
        run(
            &mut i,
            "(setcdr (cdr (overlay-get tmp 'payload)) (overlay-get tmp 'payload))",
        );
        // Drop the overlay handle: if elisp kept it, `trace_overlay`
        // would cover the props and this test could not observe the
        // buffer-side walk at all.
        run(&mut i, "(setq tmp nil)");
        run(&mut i, "(kill-buffer buf)");
        let gc = run(&mut i, "(garbage-collect)");
        assert_ne!(
            gc, "(0 0)",
            "GC did nothing -- test isn't exercising the bug"
        );
        assert_eq!(
            run(
                &mut i,
                "(cadr (overlay-get (car (with-current-buffer buf (overlays-in 1 5))) 'payload))"
            ),
            "alive",
            "killed buffer's still-attached overlay props cleared as false garbage"
        );
    });
}
