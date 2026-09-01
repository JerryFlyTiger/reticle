//! M28: the evil-mode integration foundation.
//!
//! - ESC no longer doubles as a terminal Meta prefix (`esc_pending` is
//!   gone): isearch, the minibuffer, and an ordinary buffer each give a
//!   bare ESC its own real meaning instead of it silently swallowing the
//!   next key. See `commands::handle_key` / `commands::minibuffer_key`.
//! - The buffer-local `emulation-keymap` variable is consulted ahead of
//!   the local/global keymaps on every key. See `commands::dispatch_key`.
//! - `capture-next-key` hands the very next key event straight to an
//!   elisp callback, bypassing dispatch entirely. See
//!   `commands::handle_key` / `builtins::ui::register`.
//! - `mode-line-prefix` prepends a buffer-local tag to the modeline. See
//!   `redisplay::buffer_var_str`.

use std::cell::RefCell;
use std::rc::Rc;

use core::commands::{feed_keys, Key};
use core::editor::Editor;
use core::redisplay::render;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    ed.borrow_mut().frame = (50, 8);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn row_text(grid: &core::redisplay::Grid, row: usize) -> String {
    grid.lines[row]
        .iter()
        .filter(|c| !c.continuation)
        .map(|c| c.ch)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The (first) row whose rendered text contains `needle` (see
/// modes_tests.rs's helper of the same name/purpose).
fn find_row(grid: &core::redisplay::Grid, needle: &str) -> usize {
    (0..grid.lines.len())
        .find(|&r| row_text(grid, r).contains(needle))
        .unwrap_or_else(|| {
            let dump: Vec<String> = (0..grid.lines.len()).map(|r| row_text(grid, r)).collect();
            panic!("no row contains {:?}; grid:\n{}", needle, dump.join("\n"))
        })
}

// --- 1. ESC is no longer a terminal Meta prefix ----------------------

#[test]
fn esc_in_an_ordinary_buffer_is_undefined_and_does_not_eat_the_next_key() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "ESC").unwrap();
    {
        let echo = ed.borrow().echo.clone();
        assert_eq!(echo.as_deref(), Some("ESC is undefined"));
    }
    // The old esc_pending behavior would have folded this "x" into a
    // Meta-x prefix (giving M-x, the command palette) instead of
    // inserting it. That must no longer happen.
    feed_keys(&mut i, &ed, "x").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"x\"");
}

#[test]
fn meta_bindings_still_work_directly_not_through_an_esc_prefix() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    // M-< / M-> arrive as a single Char with the META bit already set
    // (parse_kbd's "M-" handling), never touching the ESC path at all.
    feed_keys(&mut i, &ed, "M-<").unwrap();
    assert_eq!(run(&mut i, "(point)"), "1");
    feed_keys(&mut i, &ed, "M->").unwrap();
    assert_eq!(run(&mut i, "(point)"), "12");
}

#[test]
fn esc_exits_isearch_keeping_point_at_the_match() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"hello world\")");
    run(&mut i, "(goto-char 1)");
    feed_keys(&mut i, &ed, "C-s w o r").unwrap();
    assert!(ed.borrow().isearch.is_some(), "search should still be open");
    let point_during_search = run(&mut i, "(point)");
    assert_ne!(
        point_during_search, "1",
        "search should have moved point to the match"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert!(ed.borrow().isearch.is_none(), "ESC must end the search");
    assert_eq!(
        run(&mut i, "(point)"),
        point_during_search,
        "ESC keeps point at the match (unlike C-g, which reverts it)"
    );
}

#[test]
fn esc_cancels_the_minibuffer_like_c_g() {
    let (mut i, ed) = setup();
    feed_keys(&mut i, &ed, "C-x C-f").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "C-x C-f should open the minibuffer"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "ESC should cancel the minibuffer"
    );
    let echo = ed.borrow().echo.clone();
    assert_eq!(echo.as_deref(), Some("Quit"));
}

// M50: minibuffer cancel paths (ESC and C-g) used to skip
// `finish_command` entirely -- `last_command` would keep whatever value
// it had BEFORE the minibuffer-opening command ran, as if that whole
// command cycle (open minibuffer, then cancel it) had never happened.
// `kill-line`'s own append check (`crates/core/src/builtins/editing.rs`)
// reads `last_command` to decide whether to merge into the previous
// kill-ring entry, so this is directly observable: `C-k`, open a
// prompting command, cancel it, `C-k` again -- the two kills must land
// as separate kill-ring entries, not merged as if they were back to
// back. `M-g M-g` (bound to `n`, `crates/core/lisp/simple.el`, `"n"`
// interactive spec) is the prompting command; no evil state involved.

#[test]
fn esc_cancelling_a_minibuffer_prompt_ends_the_command_cycle_kill_line_repro() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-k").unwrap(); // kills "aaa"
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-g M-g should have opened the minibuffer prompt"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "ESC should cancel the minibuffer"
    );
    feed_keys(&mut i, &ed, "C-k").unwrap(); // kills the newline left after "aaa"
    assert_eq!(
        ed.borrow().kill_ring.len(),
        2,
        "the two C-k's must land as separate kill-ring entries -- an \
         intervening cancelled command must not look like a no-op to \
         kill-line's own-command-repeat check"
    );
}

#[test]
fn c_g_cancelling_a_minibuffer_prompt_ends_the_command_cycle_kill_line_repro() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"aaa\\nbbb\\nccc\")");
    run(&mut i, "(goto-char (point-min))");
    feed_keys(&mut i, &ed, "C-k").unwrap(); // kills "aaa"
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "M-g M-g should have opened the minibuffer prompt"
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert!(
        ed.borrow().minibuffer.is_none(),
        "C-g should cancel the minibuffer"
    );
    feed_keys(&mut i, &ed, "C-k").unwrap(); // kills the newline left after "aaa"
    assert_eq!(
        ed.borrow().kill_ring.len(),
        2,
        "the two C-k's must land as separate kill-ring entries -- an \
         intervening cancelled command must not look like a no-op to \
         kill-line's own-command-repeat check"
    );
}

#[test]
fn esc_cancelling_the_minibuffer_still_runs_post_command_hook_once() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    // `M-g M-g' (`n', `"n"' interactive spec) opens the minibuffer via
    // `execute_command''s `InteractiveSpec::Codes' branch, which builds
    // `PendingArgs' and calls `process_pending' directly WITHOUT ever
    // reaching `call_command' -- so the hook must not have fired yet.
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "0",
        "opening the minibuffer via spec collection must not itself fire \
         post-command-hook -- the command cycle isn't over yet"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "ESC's cancel must fire post-command-hook exactly once"
    );
}

#[test]
fn c_g_cancelling_the_minibuffer_still_runs_post_command_hook_once() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "0",
        "opening the minibuffer via spec collection must not itself fire \
         post-command-hook -- the command cycle isn't over yet"
    );
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "C-g's cancel of an open minibuffer must fire post-command-hook \
         exactly once"
    );
}

#[test]
fn esc_cancelling_a_cps_read_string_prompt_does_not_re_run_post_command_hook() {
    // M50 follow-up review fix: unlike the spec-collection case above,
    // `read-string'/`completing-read' (`builtins/ui.rs') open their
    // minibuffer from INSIDE a command body that `call_command' already
    // wrapped -- `finish_command' (and thus post-command-hook) already
    // fired once for that command's cycle by the time the prompt shows
    // up. The prompt's callback (never invoked on cancel, see
    // `completing_read_tests.rs`'s `esc_and_c_g_cancel_without_callback`)
    // would have been its own, separate cycle that simply never
    // happened, so cancelling it must NOT fire the hook a second time.
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    run(
        &mut i,
        "(defun se--cps-cmd () (interactive) (read-string \"Say: \" (lambda (s) s)))",
    );
    run(&mut i, "(global-set-key \"C-c Q\" 'se--cps-cmd)");
    feed_keys(&mut i, &ed, "C-c Q").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "se--cps-cmd's read-string should have opened the minibuffer"
    );
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "se--cps-cmd's own command cycle must already have closed out \
         (fired the hook once) by the time read-string's prompt opens"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "ESC cancelling a CPS read-string prompt must NOT fire \
         post-command-hook again -- the callback's cycle never happened"
    );
}

#[test]
fn c_g_cancelling_a_cps_read_string_prompt_does_not_re_run_post_command_hook() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    run(
        &mut i,
        "(defun se--cps-cmd () (interactive) (read-string \"Say: \" (lambda (s) s)))",
    );
    run(&mut i, "(global-set-key \"C-c Q\" 'se--cps-cmd)");
    feed_keys(&mut i, &ed, "C-c Q").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "se--cps-cmd's read-string should have opened the minibuffer"
    );
    assert_eq!(run(&mut i, "se--hook-count"), "1");
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "C-g cancelling a CPS read-string prompt must NOT fire \
         post-command-hook again -- the callback's cycle never happened"
    );
}

#[test]
fn esc_cancelling_a_cps_completing_read_prompt_does_not_re_run_post_command_hook() {
    // Same class as the two `read-string` tests above, but exercising the
    // OTHER CPS construction site (`completing_read_impl`,
    // `builtins/ui.rs`) -- a separate `PendingArgs { opener_cycle_closed:
    // true, .. }` literal from `read_from_minibuffer_impl`'s, so it needs
    // its own coverage.
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    run(
        &mut i,
        "(defun se--cps-cmd () (interactive) \
           (completing-read \"Pick: \" '(\"a\" \"b\") (lambda (s) s)))",
    );
    run(&mut i, "(global-set-key \"C-c Q\" 'se--cps-cmd)");
    feed_keys(&mut i, &ed, "C-c Q").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "se--cps-cmd's completing-read should have opened the minibuffer"
    );
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "se--cps-cmd's own command cycle must already have closed out \
         (fired the hook once) by the time completing-read's prompt opens"
    );
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "ESC cancelling a CPS completing-read prompt must NOT fire \
         post-command-hook again -- the callback's cycle never happened"
    );
}

#[test]
fn c_g_cancelling_a_cps_completing_read_prompt_does_not_re_run_post_command_hook() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    run(
        &mut i,
        "(defun se--cps-cmd () (interactive) \
           (completing-read \"Pick: \" '(\"a\" \"b\") (lambda (s) s)))",
    );
    run(&mut i, "(global-set-key \"C-c Q\" 'se--cps-cmd)");
    feed_keys(&mut i, &ed, "C-c Q").unwrap();
    assert!(
        ed.borrow().minibuffer.is_some(),
        "se--cps-cmd's completing-read should have opened the minibuffer"
    );
    assert_eq!(run(&mut i, "se--hook-count"), "1");
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        "1",
        "C-g cancelling a CPS completing-read prompt must NOT fire \
         post-command-hook again -- the callback's cycle never happened"
    );
}

#[test]
fn bare_c_g_with_no_minibuffer_open_does_not_run_post_command_hook() {
    // The counterpart of the two tests above: a C-g that isn't cancelling
    // anything (no minibuffer open) must NOT fire post-command-hook --
    // that would be a false firing of a hook meant to run exactly once
    // per real command cycle (see the M50 comment on the C-g branch in
    // `commands::handle_key`).
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--hook-count 0)");
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--hook-count (1+ se--hook-count))))",
    );
    feed_keys(&mut i, &ed, "x").unwrap(); // self-insert: a real command cycle
    let after_insert = run(&mut i, "se--hook-count");
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "se--hook-count"),
        after_insert,
        "a bare C-g with nothing open must not fire post-command-hook again"
    );
}

#[test]
fn esc_cancelling_the_minibuffer_does_not_run_keyboard_quit_hook() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--quit-count 0)");
    run(
        &mut i,
        "(add-hook 'keyboard-quit-hook (lambda () (setq se--quit-count (1+ se--quit-count))))",
    );
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    feed_keys(&mut i, &ed, "ESC").unwrap();
    assert_eq!(
        run(&mut i, "se--quit-count"),
        "0",
        "ESC cancelling the minibuffer must not run keyboard-quit-hook -- \
         see the M50 comment on the ESC branch in \
         `commands::minibuffer_key` for why"
    );
}

#[test]
fn c_g_cancelling_the_minibuffer_still_runs_keyboard_quit_hook() {
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--quit-count 0)");
    run(
        &mut i,
        "(add-hook 'keyboard-quit-hook (lambda () (setq se--quit-count (1+ se--quit-count))))",
    );
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "se--quit-count"),
        "1",
        "C-g cancelling the minibuffer must still run keyboard-quit-hook, \
         unlike ESC -- this existing behavior must not regress"
    );
}

#[test]
fn c_g_cancelling_the_minibuffer_runs_keyboard_quit_hook_before_post_command_hook() {
    // The order isn't observable from either hook's own counter alone
    // (both existing tests above pass even if the two calls in
    // `commands::handle_key`'s C-g branch were swapped) -- both hooks
    // must append to the SAME list to pin the order down.
    let (mut i, ed) = setup();
    run(&mut i, "(setq se--order nil)");
    run(
        &mut i,
        "(add-hook 'keyboard-quit-hook (lambda () (setq se--order (cons 'quit se--order))))",
    );
    run(
        &mut i,
        "(add-hook 'post-command-hook (lambda () (setq se--order (cons 'post se--order))))",
    );
    feed_keys(&mut i, &ed, "M-g M-g").unwrap();
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(
        run(&mut i, "(reverse se--order)"),
        "(quit post)",
        "keyboard-quit-hook (\"the user cancelled\") must run before \
         finish_command's post-command-hook (\"this command cycle is \
         over\")"
    );
}

#[test]
fn parse_kbd_bare_esc_and_bracket_escape_both_mean_char_27() {
    assert_eq!(core::keymap::parse_kbd("ESC").unwrap(), vec![Key::Char(27)]);
    assert_eq!(
        core::keymap::parse_kbd("<escape>").unwrap(),
        vec![Key::Char(27)]
    );
}

// --- 2. The emulation-keymap dispatch layer ---------------------------

#[test]
fn emulation_keymap_overrides_global_for_a_bound_key() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun evil--marker () (interactive) (insert \"E\"))",
    );
    run(
        &mut i,
        "(let ((m (make-sparse-keymap))) \
           (define-key m \"C-f\" 'evil--marker) \
           (setq-local emulation-keymap m))",
    );
    // C-f is globally forward-char; the emulation layer must win.
    feed_keys(&mut i, &ed, "C-f").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"E\"");
}

#[test]
fn emulation_keymap_multi_key_sequence_walks_to_completion() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun evil--diw () (interactive) (insert \"DIW\"))",
    );
    run(
        &mut i,
        "(let ((m (make-sparse-keymap))) \
           (define-key m \"d i w\" 'evil--diw) \
           (setq-local emulation-keymap m))",
    );
    // First key: a prefix, not yet a command -- echoed, buffer untouched.
    feed_keys(&mut i, &ed, "d").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    {
        let echo = ed.borrow().echo.clone();
        assert!(
            echo.as_deref().unwrap_or("").contains('d'),
            "expected a prefix echo, got {:?}",
            echo
        );
    }
    // Second key: still a prefix.
    feed_keys(&mut i, &ed, "i").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    // Third key completes the sequence.
    feed_keys(&mut i, &ed, "w").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"DIW\"");
}

#[test]
fn emulation_keymap_falls_through_to_global_and_self_insert_when_unbound() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun evil--marker () (interactive) (insert \"E\"))",
    );
    run(
        &mut i,
        "(let ((m (make-sparse-keymap))) \
           (define-key m \"C-f\" 'evil--marker) \
           (setq-local emulation-keymap m))",
    );
    // "z" isn't bound in the emulation map at all: falls through local
    // (also unbound) to self-insert.
    feed_keys(&mut i, &ed, "z").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"z\"");
    // M-> isn't bound in the emulation map either: falls all the way
    // through to the ordinary global binding (end-of-buffer).
    run(&mut i, "(goto-char 1)");
    feed_keys(&mut i, &ed, "M->").unwrap();
    assert_eq!(run(&mut i, "(point)"), "2");
}

#[test]
fn emulation_keymap_is_buffer_local_independent_per_buffer() {
    let (mut i, ed) = setup();
    run(
        &mut i,
        "(defun evil--marker () (interactive) (insert \"E\"))",
    );
    run(&mut i, "(switch-to-buffer \"has-emulation\")");
    run(
        &mut i,
        "(let ((m (make-sparse-keymap))) \
           (define-key m \"C-f\" 'evil--marker) \
           (setq-local emulation-keymap m))",
    );
    run(&mut i, "(switch-to-buffer \"no-emulation\")");
    assert_eq!(run(&mut i, "(local-variable-p 'emulation-keymap)"), "nil");
    feed_keys(&mut i, &ed, "C-f").unwrap();
    // Plain forward-char on an empty buffer: nothing is inserted.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
    run(&mut i, "(switch-to-buffer \"has-emulation\")");
    feed_keys(&mut i, &ed, "C-f").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"E\"");
}

// --- 3. capture-next-key ----------------------------------------------

#[test]
fn capture_next_key_receives_plain_upper_ctrl_and_named_keys() {
    let (mut i, ed) = setup();
    run(&mut i, "(defvar captured nil)");

    run(&mut i, "(capture-next-key (lambda (k) (setq captured k)))");
    feed_keys(&mut i, &ed, "x").unwrap();
    assert_eq!(run(&mut i, "captured"), "120"); // ?x

    run(&mut i, "(capture-next-key (lambda (k) (setq captured k)))");
    feed_keys(&mut i, &ed, "X").unwrap();
    assert_eq!(run(&mut i, "captured"), "88"); // ?X

    run(&mut i, "(capture-next-key (lambda (k) (setq captured k)))");
    feed_keys(&mut i, &ed, "C-x").unwrap();
    assert_eq!(run(&mut i, "captured"), "24"); // ?\C-x, not the C-x prefix map

    run(&mut i, "(capture-next-key (lambda (k) (setq captured k)))");
    feed_keys(&mut i, &ed, "<up>").unwrap();
    assert_eq!(run(&mut i, "captured"), "up");

    // None of the captured keys touched the buffer or dispatched normally.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
}

#[test]
fn capture_next_key_c_g_cancels_without_calling_the_callback() {
    let (mut i, ed) = setup();
    run(&mut i, "(defvar captured 'untouched)");
    run(&mut i, "(capture-next-key (lambda (k) (setq captured k)))");
    feed_keys(&mut i, &ed, "C-g").unwrap();
    assert_eq!(run(&mut i, "captured"), "untouched");
    {
        let echo = ed.borrow().echo.clone();
        assert_eq!(echo.as_deref(), Some("Quit"));
    }
    // The capture is gone: an ordinary key afterward is not swallowed.
    feed_keys(&mut i, &ed, "x").unwrap();
    assert_eq!(run(&mut i, "(buffer-string)"), "\"x\"");
}

#[test]
fn capture_next_key_can_rearm_itself_from_the_callback() {
    let (mut i, ed) = setup();
    run(&mut i, "(defvar seen nil)");
    run(
        &mut i,
        "(defun collect-and-rearm (k) \
           (setq seen (cons k seen)) \
           (capture-next-key 'collect-and-rearm))",
    );
    run(&mut i, "(capture-next-key 'collect-and-rearm)");
    feed_keys(&mut i, &ed, "a").unwrap();
    feed_keys(&mut i, &ed, "b").unwrap();
    feed_keys(&mut i, &ed, "c").unwrap();
    assert_eq!(run(&mut i, "(reverse seen)"), "(97 98 99)");
    // All three keys were captured -- nothing self-inserted.
    assert_eq!(run(&mut i, "(buffer-string)"), "\"\"");
}

// --- 4. mode-line-prefix ------------------------------------------------

#[test]
fn mode_line_prefix_absent_by_default() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"x\")");
    let grid = render(&i, &ed);
    let mode = row_text(&grid, 6);
    assert!(mode.contains("*scratch*"), "modeline: {:?}", mode);
    assert!(!mode.contains('<'), "no prefix marker expected: {:?}", mode);
}

#[test]
fn mode_line_prefix_shown_when_buffer_local_is_set() {
    let (mut i, ed) = setup();
    run(&mut i, "(insert \"x\")");
    run(&mut i, "(setq-local mode-line-prefix \"<N> \")");
    let grid = render(&i, &ed);
    let mode = row_text(&grid, 6);
    assert!(
        mode.contains("<N> *scratch*"),
        "modeline should show the prefix immediately ahead of the name: {:?}",
        mode
    );
}

#[test]
fn mode_line_prefix_is_independent_per_window_buffer() {
    let (mut i, ed) = setup();
    ed.borrow_mut().frame = (60, 14); // headroom for two stacked windows

    run(&mut i, "(switch-to-buffer \"alpha\")");
    run(&mut i, "(insert \"alpha text\")");
    run(&mut i, "(setq-local mode-line-prefix \"<1> \")");

    run(&mut i, "(split-window-below)");
    run(&mut i, "(other-window 1)");

    run(&mut i, "(switch-to-buffer \"beta\")");
    run(&mut i, "(insert \"beta text\")");
    assert_eq!(run(&mut i, "(local-variable-p 'mode-line-prefix)"), "nil");

    let grid = render(&i, &ed);
    let prefixed_row = find_row(&grid, "<1> alpha");
    assert!(row_text(&grid, prefixed_row).contains("<1> alpha"));
    // The prefix must not leak onto beta's modeline (or anywhere else).
    let occurrences = (0..grid.lines.len())
        .filter(|&r| row_text(&grid, r).contains("<1>"))
        .count();
    assert_eq!(
        occurrences, 1,
        "prefix should appear on exactly one modeline"
    );
}
