//! M67: `describe-key` / `describe-bindings` and their three supporting
//! Rust builtins (`key-description`, `lookup-key`, `all-key-bindings`).

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

fn echo(ed: &Rc<RefCell<Editor>>) -> Option<String> {
    ed.borrow().echo.clone()
}

fn type_str(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, s: &str) {
    for c in s.chars() {
        handle_key(interp, ed, Key::Char(c as i64));
    }
}

/// Render and return one row of the grid as text, continuation cells
/// dropped -- the same shape `core_tests.rs`'s own `grid_row` helper
/// uses (kept separate here since Rust integration test binaries can't
/// share private helpers across files).
fn grid_row(interp: &Interp, ed: &Rc<RefCell<Editor>>, row: usize) -> String {
    let grid = core::redisplay::render(interp, ed);
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

// --- key-description --------------------------------------------------

#[test]
fn key_description_printable_char() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(key-description (list ?a))"), "\"a\"");
}

#[test]
fn key_description_ctrl_and_meta() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(key-description (list ?\\C-x))"), "\"C-x\"");
    assert_eq!(run(&mut i, "(key-description (list ?\\M-x))"), "\"M-x\"");
}

#[test]
fn key_description_named_keys() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(key-description (list ?\\C-m))"), "\"RET\"");
    assert_eq!(run(&mut i, "(key-description (list ?\\C-i))"), "\"TAB\"");
    assert_eq!(run(&mut i, "(key-description (list ?\\ ))"), "\"SPC\"");
    assert_eq!(run(&mut i, "(key-description (list ?\\C-\\[))"), "\"ESC\"");
    assert_eq!(run(&mut i, "(key-description (list ?\\C-\\?))"), "\"DEL\"");
}

#[test]
fn key_description_c0_specials() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(key-description (list 0))"), "\"C-@\"");
    assert_eq!(run(&mut i, "(key-description (list 28))"), "\"C-\\\\\"");
    assert_eq!(run(&mut i, "(key-description (list 29))"), "\"C-]\"");
    assert_eq!(run(&mut i, "(key-description (list 30))"), "\"C-^\"");
    assert_eq!(run(&mut i, "(key-description (list 31))"), "\"C-_\"");
}

#[test]
fn key_description_symbol_key() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(key-description (list 'up))"), "\"<up>\"");
}

#[test]
fn key_description_sequence_joins_with_spaces() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(key-description (list ?\\C-x ?\\C-f))"),
        "\"C-x C-f\""
    );
}

// --- lookup-key ----------------------------------------------------------

#[test]
fn lookup_key_global_command_hit() {
    let (mut i, _ed) = setup();
    // C-n is bound globally to next-line.
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-n))"),
        "(command next-line global)"
    );
}

#[test]
fn lookup_key_prefix() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-h))"),
        "(prefix nil global)"
    );
}

#[test]
fn lookup_key_full_prefix_sequence_hits_command() {
    let (mut i, _ed) = setup();
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-h ?.))"),
        "(command lsp-hover-at-point global)"
    );
}

#[test]
fn lookup_key_unbound_is_nil() {
    let (mut i, _ed) = setup();
    assert_eq!(run(&mut i, "(lookup-key (list ?\\M-z))"), "nil");
}

#[test]
fn lookup_key_local_shadows_global() {
    let (mut i, _ed) = setup();
    // C-n is globally next-line; bind it locally to something else.
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-n\" 'previous-line) (use-local-map map))",
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-n))"),
        "(command previous-line local)"
    );
}

#[test]
fn lookup_key_emulation_shadows_local() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-n\" 'previous-line) (use-local-map map))",
    );
    run(
        &mut i,
        "(setq-local emulation-keymap (let ((m (make-sparse-keymap))) (define-key m \"C-n\" 'eval-buffer) m))",
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-n))"),
        "(command eval-buffer emulation)"
    );
}

#[test]
fn lookup_key_upper_layer_prefix_short_circuits() {
    let (mut i, _ed) = setup();
    // NOTE: despite the name, this only queries a SINGLE key (?\C-x) --
    // it does not exercise a multi-key sequence at all, and is really
    // just another local-shadows-global case (same shape as
    // `lookup_key_local_shadows_global` above). What it actually checks:
    // C-x is a prefix globally (C-x C-f etc.), but the LOCAL layer here
    // claims plain C-x as an ordinary command, and that Command result
    // must win outright rather than somehow still consulting global's
    // prefix table underneath it. Multi-key layered shadowing (the thing
    // this test's old name implied) is covered separately by
    // `lookup_key_local_shadows_global_for_multi_key_sequence` and
    // `lookup_key_emulation_shadows_local_for_multi_key_sequence` below.
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-x\" 'next-line) (use-local-map map))",
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-x))"),
        "(command next-line local)"
    );
}

#[test]
fn lookup_key_local_shadows_global_for_multi_key_sequence() {
    let (mut i, _ed) = setup();
    // Every OTHER local-vs-global test here (`lookup_key_local_shadows_
    // global`, the one directly above) queries a single-key sequence, so
    // a bug that special-cases "len > 1 -> always resolve through the
    // global layer, skip local/emulation entirely" (single-key lookups
    // untouched) would pass every existing test. Both layers bind the
    // SAME three-key sequence to different commands here specifically to
    // catch that: the global one must never win once local claims it.
    run(&mut i, "(global-set-key \"C-c z a\" 'next-line)");
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-c z a\" 'previous-line) (use-local-map map))",
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?z ?a))"),
        "(command previous-line local)"
    );
}

#[test]
fn lookup_key_emulation_shadows_local_for_multi_key_sequence() {
    let (mut i, _ed) = setup();
    // Same rationale as `lookup_key_local_shadows_global_for_multi_key_
    // sequence` above, one layer up: emulation must still win over local
    // for a THREE-key sequence, not just the single-key case
    // `lookup_key_emulation_shadows_local` already covers. Deliberately
    // no global binding for this sequence at all, so a "collapse
    // multi-key lookups straight to global" bug would resolve to
    // Undefined here (mismatching the expected Command result) rather
    // than accidentally matching by coincidence.
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-c z a\" 'previous-line) (use-local-map map))",
    );
    run(
        &mut i,
        "(setq-local emulation-keymap (let ((m (make-sparse-keymap))) (define-key m \"C-c z a\" 'eval-buffer) m))",
    );
    assert_eq!(
        run(&mut i, "(lookup-key (list ?\\C-c ?z ?a))"),
        "(command eval-buffer emulation)"
    );
}

// --- all-key-bindings ------------------------------------------------

#[test]
fn all_key_bindings_flattens_nested_prefix() {
    let (mut i, _ed) = setup();
    let out = run(&mut i, "(all-key-bindings)");
    assert!(
        out.contains("(global \"C-h .\" lsp-hover-at-point)"),
        "expected full sequence entry, got: {}",
        out
    );
    assert!(
        !out.contains("(global \"C-h\" "),
        "must not emit the bare prefix key itself: {}",
        out
    );
}

#[test]
fn all_key_bindings_separates_layers() {
    let (mut i, _ed) = setup();
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-t\" 'next-line) (use-local-map map))",
    );
    let out = run(&mut i, "(all-key-bindings)");
    assert!(out.contains("(local \"C-t\" next-line)"), "got: {}", out);
    assert!(out.contains("(global \"C-n\" next-line)"), "got: {}", out);
}

#[test]
fn all_key_bindings_skips_absent_emulation_layer() {
    let (mut i, _ed) = setup();
    // emulation-keymap defaults to nil -- no emulation-layer entries at all.
    let out = run(&mut i, "(all-key-bindings)");
    assert!(!out.contains("(emulation "), "got: {}", out);
}

/// Coordinator mutation review (M67, gap 2): a self-referential keymap
/// -- one that binds a key back to ITSELF -- is legal at the elisp
/// level. `define-key` (`ui.rs`) never checks what kind of `Value`
/// BINDING is, so `(define-key map "C-x" map)` just stores the keymap
/// value in its own entry table; probed directly (outside this test
/// file, since it can't hang forever inside a normal test run) this
/// really does succeed and `(all-key-bindings)` on it really does
/// return promptly (~0.1s, 2225-char result) rather than hanging.
/// `enumerate_bindings`'s depth cap (`keymap.rs`) is the ONLY thing
/// standing between this and infinite recursion: a mutation that widens
/// it (e.g. `depth > 8` -> `depth > 100_000`) turns this call into one
/// that either hangs or blows the stack, which is exactly what makes
/// the cap observable here.
#[test]
fn all_key_bindings_terminates_on_self_referential_keymap() {
    let (mut i, _ed) = setup();
    // The cycle carries a real leaf (`a`) on purpose, so that the depth
    // cap's effect is OBSERVABLE: each level the cap allows contributes
    // exactly one more entry (`a`, `C-x a`, `C-x C-x a`, ...). A purely
    // self-referential map bottoms out at no leaf at all, so it yields
    // zero entries whether the cap is 8 or 8000 -- an assertion on that
    // shape cannot detect the guard being removed, which is what the
    // first version of this test got wrong.
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-x\" map) (define-key map \"a\" 'next-line) (use-local-map map))",
    );
    let out = run(&mut i, "(all-key-bindings)");
    let local_entries = out.matches("(local ").count();
    assert_eq!(
        local_entries, 9,
        "the guard is `depth > 8` with depth starting at 0, so the cycle \
         must bottom out at exactly 9 leaves; got {} in: {}",
        local_entries, out
    );
    assert!(
        out.contains("(global \"C-n\" next-line)"),
        "ordinary global entries must still come through unaffected, got: {}",
        out
    );
}

// --- describe-key end to end -------------------------------------------

#[test]
fn describe_key_reports_documented_command() {
    let (mut i, ed) = setup();
    // C-h f is bound to describe-function, which has a docstring.
    feed_keys(&mut i, &ed, "C-h k").unwrap();
    feed_keys(&mut i, &ed, "C-h f").unwrap();
    let msg = echo(&ed).unwrap();
    assert!(
        msg.starts_with("C-h f runs describe-function: "),
        "got: {:?}",
        msg
    );
    assert!(!msg.contains('\n'), "echo must be single-line: {:?}", msg);
}

#[test]
fn describe_key_reports_undocumented_command() {
    let (mut i, ed) = setup();
    // C-n is bound to next-line, a Rust builtin with no docstring.
    feed_keys(&mut i, &ed, "C-h k").unwrap();
    feed_keys(&mut i, &ed, "C-n").unwrap();
    let msg = echo(&ed).unwrap();
    assert_eq!(msg, "C-n runs next-line (not documented)");
}

#[test]
fn describe_key_reports_prefix_and_waits_for_more() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-h k").unwrap();
    feed_keys(&mut i, &ed, "C-h").unwrap();
    let msg = echo(&ed).unwrap();
    assert_eq!(msg, "C-h-");
    // Still capturing: finish the sequence and check it resolves.
    feed_keys(&mut i, &ed, ".").unwrap();
    let msg2 = echo(&ed).unwrap();
    assert!(
        msg2.starts_with("C-h . runs lsp-hover-at-point"),
        "got: {:?}",
        msg2
    );
}

#[test]
fn describe_key_reports_undefined() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-h k").unwrap();
    feed_keys(&mut i, &ed, "M-z").unwrap();
    let msg = echo(&ed).unwrap();
    assert_eq!(msg, "M-z is undefined");
}

// --- describe-bindings ------------------------------------------------

#[test]
fn describe_bindings_builds_readonly_help_buffer() {
    let (mut i, ed) = setup();
    let source_name = ed.borrow().current.borrow().name.clone();
    run(&mut i, "(describe-bindings)");
    let cur_name = ed.borrow().current.borrow().name.clone();
    assert_eq!(cur_name, "*Help*");
    assert!(ed.borrow().current.borrow().read_only);
    let text = ed.borrow().current.borrow().search_text().to_string();
    assert!(
        text.contains("C-n") && text.contains("next-line"),
        "expected a known binding in *Help* content, got: {}",
        text
    );
    // `q` has a local binding in *Help*'s own keymap.
    assert_eq!(
        run(&mut i, "(lookup-key (list ?q))"),
        "(command help-quit local)"
    );
    // Pressing q returns to the buffer describe-bindings was called from.
    feed_keys(&mut i, &ed, "q").unwrap();
    let back_name = ed.borrow().current.borrow().name.clone();
    assert_eq!(back_name, source_name);
}

#[test]
fn describe_bindings_help_buffer_is_not_marked_modified() {
    // M71: `describe-bindings' builds `*Help*' with `erase-buffer' +
    // `insert', which sets the modified flag. `*Help*' is entirely
    // regenerated from `all-key-bindings' every time -- there is
    // nothing a user could edit or lose here -- so a `*' on it would
    // only ever be noise. This test doesn't dispatch any keys, so it
    // doesn't need evil-mode enabled (unlike the `q'-reachability test
    // above, which does).
    let (mut i, _ed) = setup();
    run(&mut i, "(describe-bindings)");
    assert_eq!(
        run(&mut i, "(buffer-modified-p)"),
        "nil",
        "a freshly built *Help* buffer must not be modified"
    );
}

/// Coordinator TUI repro (M67 tail fix): with evil-mode ON, evil's
/// normal-state keymap lives in `emulation-keymap`, which outranks the
/// LOCAL `q' binding `describe-bindings' installs (`lookup_layered' in
/// commands.rs: emulation > local > global). Every OTHER `q' test in
/// this file (`describe_bindings_builds_readonly_help_buffer' above,
/// `describe_bindings_called_again_inside_help_still_returns_to_
/// original_source' below) runs with evil-mode OFF, so `emulation-
/// keymap' is nil there and can never shadow anything -- none of them
/// would have caught this. `*Help*' only reaches evil's `emacs' state
/// (bypassing the normal-state map entirely) if `help-mode' is listed
/// in `evil-emacs-state-modes' (evil.el) -- confirmed missing by
/// `describe-bindings' actually calling `(major-mode-internal-set
/// 'help-mode)' (simple.el) and evil.el's own `evil--initial-state-for-
/// mode' consulting exactly that variable.
#[test]
fn describe_bindings_q_reachable_under_evil_normal_state() {
    let (mut i, ed) = setup();
    let source_name = ed.borrow().current.borrow().name.clone();
    run(&mut i, "(evil-mode 1)");
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(ed.borrow().current.borrow().name, "*Help*");
    assert_eq!(
        run(&mut i, "evil--state"),
        "emacs",
        "*Help* must start in evil's `emacs' state (evil-emacs-state-modes) so its \
         own local `q' binding isn't shadowed by the normal-state emulation keymap"
    );
    feed_keys(&mut i, &ed, "q").unwrap();
    let back_name = ed.borrow().current.borrow().name.clone();
    assert_eq!(
        back_name, source_name,
        "q under evil's normal state must still reach help-quit, not get intercepted \
         by evil's normal-state `q' (record-macro) sitting in emulation-keymap"
    );
}

// --- M67 review fix #1: echo area control-char rendering ---------------

/// Repro for review finding #1: a raw `\n` (or any C0 control char) fed
/// to `message` used to reach `Cell.ch` verbatim in the echo row instead
/// of the `^X` caret notation the buffer-text path already draws for the
/// same characters (`redisplay.rs`'s buffer-text loop vs. its echo-area
/// loop, previously out of sync). Frame is 80x24 by default
/// (`Editor::frame`, see `editor.rs`), so the echo row is row 23.
#[test]
fn echo_control_chars_render_as_caret_notation() {
    let (mut i, ed) = setup();
    run(&mut i, "(message \"a\\nb\")");
    let row = grid_row(&i, &ed, 23);
    assert!(
        !row.contains('\n'),
        "echo row must never contain a raw newline: {:?}",
        row
    );
    assert_eq!(row, "a^Jb");
}

#[test]
fn echo_del_also_renders_as_the_buffer_path_does() {
    let (mut i, ed) = setup();
    // The elisp string reader has no `\x` hex escape (reader.rs
    // `read_string`), so the DEL byte is spliced in directly here rather
    // than through an elisp escape sequence.
    let src = format!("(message \"x{}y\")", '\u{7f}');
    run(&mut i, &src);
    assert_eq!(grid_row(&i, &ed, 23), "x^?y");
}

// --- M67 review fix: describe-function first-line + multiline docstring ---

#[test]
fn describe_function_shows_only_first_docstring_line() {
    let (mut i, ed) = setup();
    // y-or-n-p (simple.el) has a genuinely multi-line docstring.
    feed_keys(&mut i, &ed, "C-h f").unwrap();
    type_str(&mut i, &ed, "y-or-n-p");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let msg = echo(&ed).unwrap();
    assert!(!msg.contains('\n'), "echo must be single-line: {:?}", msg);
    assert_eq!(
        msg,
        "y-or-n-p: Ask PROMPT, answer y/n via one keystroke, and call CALLBACK with t"
    );
}

// --- M67 review fix #2: shadow marking must catch prefix chains too ----

#[test]
fn describe_bindings_marks_exact_duplicate_as_shadowed() {
    let (mut i, ed) = setup();
    // Bind C-t identically in both local (higher priority) and global
    // (lower priority) layers; the GLOBAL entry -- the one that would
    // never actually be reached -- must be marked shadowed. The local
    // entry, which wins, must NOT be.
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-t\" 'next-line) (use-local-map map))",
    );
    run(&mut i, "(global-set-key \"C-t\" 'next-line)");
    run(&mut i, "(describe-bindings)");
    let text = ed.borrow().current.borrow().search_text().to_string();
    let mut ct_lines = text.lines().filter(|l| l.trim_start().starts_with("C-t"));
    let local_line = ct_lines.next().unwrap_or_default();
    let global_line = ct_lines.next().unwrap_or_default();
    assert!(
        !local_line.contains("(shadowed)"),
        "the winning local C-t entry must not be marked shadowed, got: {:?}\nfull text:\n{}",
        local_line,
        text
    );
    assert!(
        global_line.contains("(shadowed)"),
        "expected the unreachable global C-t entry marked shadowed, got: {:?}\nfull text:\n{}",
        global_line,
        text
    );
}

#[test]
fn describe_bindings_marks_prefix_chain_as_shadowed() {
    let (mut i, ed) = setup();
    // emulation binds plain "C-c" to a command -- dispatch stops there,
    // so local's longer "C-c a" chain is unreachable even though its
    // KEYDESC string never equals "C-c" exactly (review finding #2).
    run(
        &mut i,
        "(let ((map (make-sparse-keymap))) (define-key map \"C-c a\" 'next-line) (use-local-map map))",
    );
    run(
        &mut i,
        "(setq-local emulation-keymap (let ((m (make-sparse-keymap))) (define-key m \"C-c\" 'previous-line) m))",
    );
    run(&mut i, "(describe-bindings)");
    let text = ed.borrow().current.borrow().search_text().to_string();
    let chain_line = text
        .lines()
        .find(|l| l.trim_start().starts_with("C-c a"))
        .unwrap_or_default();
    assert!(
        chain_line.contains("(shadowed)"),
        "expected \"C-c a\" marked shadowed by the higher-priority plain \"C-c\" binding, got line: {:?}\nfull text:\n{}",
        chain_line,
        text
    );
}

// --- M67 review fix #3: q in a second *Help* buffer must not get stuck -

/// Repro for review finding #3: calling `describe-bindings` a second
/// time while ALREADY inside `*Help*` used to overwrite the buffer-local
/// source-buffer variable (M68: renamed `help--source-buffer` ->
/// `quit-source`, shared with `dired`) with `"*Help*"` itself, so `q`
/// (`help-quit`) switched to `*Help*` and went nowhere.
#[test]
fn describe_bindings_called_again_inside_help_still_returns_to_original_source() {
    let (mut i, ed) = setup();
    let source_name = ed.borrow().current.borrow().name.clone();
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(ed.borrow().current.borrow().name, "*Help*");
    // Call it again from inside *Help* itself.
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(ed.borrow().current.borrow().name, "*Help*");
    feed_keys(&mut i, &ed, "q").unwrap();
    let back_name = ed.borrow().current.borrow().name.clone();
    assert_eq!(
        back_name, source_name,
        "q after a second describe-bindings call must return to the ORIGINAL source buffer, not get stuck in *Help*"
    );
}

// --- M67 review fix: the C-h b / C-h f bindings themselves, via feed_keys -

#[test]
fn c_h_b_binding_opens_help_buffer_via_real_dispatch() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(ed.borrow().current.borrow().name, "*Help*");
}

#[test]
fn c_h_f_binding_prompts_for_a_function_via_real_dispatch() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-h f").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "C-h f should open the describe-function minibuffer prompt"
    );
    type_str(&mut i, &ed, "next-line");
    feed_keys(&mut i, &ed, "RET").unwrap();
    let msg = echo(&ed).unwrap();
    assert_eq!(msg, "next-line is a function (not documented)");
}

#[test]
fn help_j_and_k_move_by_line() {
    let (mut i, ed) = setup();
    run(&mut i, "(evil-mode 1)");
    feed_keys(&mut i, &ed, "C-h b").unwrap();
    assert_eq!(ed.borrow().current.borrow().name, "*Help*");
    assert_eq!(
        run(&mut i, "evil--state"),
        "emacs",
        "*Help* must start in evil's `emacs' state"
    );
    let before = run(&mut i, "(buffer-string)");
    run(&mut i, "(goto-char (point-min))");
    let line0 = run(&mut i, "(line-number-at-pos)");
    feed_keys(&mut i, &ed, "j").unwrap();
    let line1 = run(&mut i, "(line-number-at-pos)");
    assert_ne!(line1, line0, "j must move down a line");
    feed_keys(&mut i, &ed, "k").unwrap();
    assert_eq!(
        run(&mut i, "(line-number-at-pos)"),
        line0,
        "k must move back up to the original line"
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before,
        "j/k must never alter *Help*'s text"
    );
    // M130 fix round FIX-4: `*Help*' is a plain text buffer (no
    // trailing row past the last line of content the way dired/
    // search-mode have), so plain `end-of-buffer' semantics are
    // correct and need no special-case landing function -- still
    // asserted here since nothing else in this test presses `G'.
    feed_keys(&mut i, &ed, "G").unwrap();
    assert_eq!(
        run(&mut i, "(point)"),
        run(&mut i, "(point-max)"),
        "G must move point to the end of *Help*"
    );
    assert_eq!(
        run(&mut i, "(buffer-string)"),
        before,
        "G must never alter *Help*'s text"
    );
}
