use std::cell::RefCell;
use std::rc::Rc;

use elisp::eval::{apply_function, resolve_function};
use elisp::value::Function;
use elisp::{Interp, Value};

use crate::editor::{Editor, Minibuffer};
use crate::keymap::{self, as_keymap, key_sequence_description, CTRL, META};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// Character with the reader's C-/M- bit encoding.
    Char(i64),
    /// Named function key: "up", "down", "home", ...
    Sym(String),
}

#[derive(Default)]
pub struct ArgSpec {
    pub code: char,
    pub prompt: String,
    /// Caller-provided candidate set (`completing-read`). `None` means
    /// the source is derived from `code` instead (M47).
    pub collection: Option<std::rc::Rc<Vec<String>>>,
    /// Submission must name a member of `collection` (M47).
    pub require_match: bool,
    /// Prefill for the minibuffer when it opens (M47); cursor lands at
    /// the end, same as the existing file/default-directory prefill.
    pub initial: Option<String>,
    /// M-p/M-n history ring key (M47 Part D).
    pub history_key: String,
}

pub struct PendingArgs {
    pub command: Value,
    pub specs: Vec<ArgSpec>,
    pub collected: Vec<Value>,
    pub index: usize,
    /// M50 follow-up: whether the command cycle that OPENED this minibuffer
    /// has already been closed out by `finish_command` (i.e. whether
    /// `post-command-hook` already fired for it once). The two
    /// construction sites differ here:
    ///  - `execute_command`'s `InteractiveSpec::Codes` branch (spec
    ///    collection, e.g. `M-g M-g`'s `"n"`): `false` -- it builds this
    ///    and calls `process_pending` directly, WITHOUT ever reaching
    ///    `call_command`, so the cycle is still open when the minibuffer
    ///    shows up. A cancel here must run `finish_command` itself, or
    ///    the cycle never gets closed at all.
    ///  - `read_from_minibuffer_impl`/`completing_read_impl`
    ///    (`builtins/ui.rs`): `true` -- these run from INSIDE a command
    ///    body that `call_command` already wrapped, so `finish_command`
    ///    already fired once for this cycle by the time the minibuffer
    ///    opens (the CALLBACK, not the surrounding command, is the
    ///    actual continuation). A cancel here must NOT run
    ///    `finish_command` again -- the callback never runs on cancel
    ///    (M47, `completing_read_tests.rs`'s
    ///    `esc_and_c_g_cancel_without_callback`), so there is no second
    ///    cycle to close; doing so anyway is a false extra firing of
    ///    `post-command-hook`.
    pub opener_cycle_closed: bool,
}

const KEY_CTRL_G: i64 = 7;

/// Main entry point: frontends feed every key event through this.
pub fn handle_key(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, key: Key) {
    {
        let mut editor = ed.borrow_mut();
        editor.echo = None;
        editor.hover_popup = None;
    }
    // Match Emacs: the command loop keeps the current buffer in sync with
    // whatever the selected window displays before handling each key, so
    // a stray (set-buffer ...) that wasn't restored can't hijack typing.
    crate::editor::sync_current_to_selected(interp, ed);
    // M42-II: keyboard-macro recording tap -- deliberately sits exactly
    // here: after the buffer-sync above (so a recorded macro replays
    // against whichever buffer was actually current when each key was
    // typed, matching every other key's own ordering) and before
    // `key_capture` is taken below (so a capture callback's OWN key --
    // e.g. the register character of `qa`, or a `f`/`t`/`r`/`m`/`"`
    // capture that happens to fire while a macro is recording around
    // it -- is captured verbatim as raw keystrokes, not silently
    // skipped just because something downstream is about to consume
    // it). Never records while replaying (`macro_replay_depth > 0`):
    // vim's own semantics record the `@x` invocation itself, not
    // whatever keys `@x` expands to -- see `Editor::kbd_macro_recording`
    // and `Editor::macro_replay_depth`'s own doc comments.
    {
        let mut editor = ed.borrow_mut();
        if editor.macro_replay_depth == 0 {
            if let Some((_, keys)) = editor.kbd_macro_recording.as_mut() {
                keys.push(key.clone());
            }
        }
    }
    // M28 `capture-next-key`: a pending capture wins over everything else
    // — isearch, the minibuffer, even the ordinary C-g handling below —
    // since elisp asked for the very next key event, unconditionally.
    // Taking the callback out before invoking it (rather than clearing it
    // after) lets the callback re-arm capture for the following key.
    let captured = ed.borrow_mut().key_capture.take();
    if let Some(f) = captured {
        if key == Key::Char(KEY_CTRL_G) {
            {
                let mut editor = ed.borrow_mut();
                editor.echo("Quit");
                // M30 review fix (issue 3): a capture-time C-g (e.g.
                // cancelling `r' mid-capture) never reaches
                // `execute_command' either, so it must ALSO clear a
                // pending `undo-amalgamate-boundary' suppression itself
                // — see the identical comment on the branch below.
                editor.suppress_next_undo_boundary = false;
            }
            // M29: give an emulation layer mid-capture (e.g. evil's
            // operator-pending state, captured via `f`/`t`/`r`) a chance
            // to reset its own state — see the identical call below.
            run_hook_by_name(interp, "keyboard-quit-hook");
            return;
        }
        let arg = key_to_value(interp, &key);
        if let Err(flow) = apply_function(interp, &f, vec![arg].into()) {
            let msg = interp.describe_flow(&flow);
            ed.borrow_mut().echo(msg);
        }
        return;
    }
    // M53: an isearch session's `start`/`origin`/`origin_byte` are offsets
    // into whatever buffer was current when the session started
    // (`isearch_start`). If elisp swaps the current buffer out from under
    // a live session (`switch-to-buffer`, `kill-buffer`, ...) those
    // offsets become meaningless for the NEW current buffer, and
    // `GapBuffer::char_to_byte`/`byte_to_char` clamp out-of-range offsets
    // instead of erroring, so a stale session would silently move point
    // to a semantically unrelated position (or search text ranges based
    // on the wrong offsets) here.
    //
    // Checked right here, at the single point every isearch keystroke
    // passes through, rather than in each of `set_current_buffer` /
    // `kill-buffer` / any future buffer-switching path: enumerating every
    // caller that can change `editor.current` is exactly the kind of
    // thing that's easy to leave one out of (and silently regress this
    // fix later when a new one is added), whereas this check can't be
    // bypassed no matter how the buffer got swapped.
    //
    // Note this deliberately does NOT invalidate a session just because
    // the user switched away and back to the SAME buffer — the check
    // only runs (and only fires) when the CURRENT buffer at keystroke
    // time differs from the session's buffer, so switching away and back
    // leaves the session alive when the next isearch key arrives. That
    // matches how the rest of isearch already tolerates the buffer being
    // edited out from under a live session; not a new hole opened here.
    //
    // `Editor::isearch_live` is the shared read-only test for "is there a
    // session AND is it still valid for the current buffer" — other
    // readers (`isearch-active-p`, the completion-popup modal-session
    // guard) use the same method so this check has exactly one
    // implementation; this call site is the only one that also clears
    // the stale session out, since it's the one place on the isearch-key
    // path where doing so is safe.
    {
        let stale = ed.borrow().isearch.is_some() && !ed.borrow().isearch_live();
        if stale {
            ed.borrow_mut().isearch = None;
        }
    }
    if ed.borrow().isearch.is_some() {
        if key == Key::Char(KEY_CTRL_G) {
            crate::editor::isearch_cancel(ed);
            ed.borrow_mut().echo("Quit");
            return;
        }
        if isearch_key(ed, &key) {
            return;
        }
        // Any other key ends the search (keeping the match) and is then
        // handled normally, matching Emacs isearch-exit semantics.
    }
    if key == Key::Char(KEY_CTRL_G) {
        // M50: only a C-g that actually cancels an open minibuffer whose
        // OPENING command cycle is still unclosed marks that cycle as
        // ended (`finish_command` below). Two cases must NOT re-fire it:
        //  - A "bare" C-g with no minibuffer open isn't cancelling
        //    anything — there's no command mid-flight collecting args —
        //    so running `post-command-hook` for it would be a FALSE
        //    firing, and `evil--post-command' (evil.el) relies on exactly
        //    one firing per real command cycle to decide whether
        //    `evil--count' / `evil--pending-register' have gone stale.
        //  - M50 follow-up: a minibuffer opened by `read-string`/
        //    `completing-read` FROM INSIDE an already-running command
        //    body (`opener_cycle_closed`, see `PendingArgs`'s doc
        //    comment) — that command's cycle was already closed by
        //    `call_command` before the minibuffer ever opened; the
        //    minibuffer's callback (never invoked on cancel) would have
        //    been its own, separate cycle that simply never happened.
        // Both booleans are recorded before the borrow below clears
        // `minibuffer` out from under us.
        let had_minibuffer = ed.borrow().minibuffer.is_some();
        let opener_cycle_open = ed
            .borrow()
            .minibuffer
            .as_ref()
            .is_some_and(|mb| !mb.pending.opener_cycle_closed);
        {
            let mut editor = ed.borrow_mut();
            editor.pending_keys.clear();
            editor.minibuffer = None;
            editor.current.borrow_mut().mark_active = false;
            editor.echo("Quit");
            // M30 review fix (issue 3): a C-g that reaches here (e.g.
            // insert state, via `keyboard-quit-hook' -> evil's
            // `evil--on-keyboard-quit' -> `evil-insert-exit') never
            // goes through `execute_command' (that hook runs via a
            // plain `apply_function', see `run_hook_by_name'), so
            // `execute_command''s own clearing of a pending
            // `undo-amalgamate-boundary' suppression (see its comment)
            // never fires either. Left uncleared, the flag would
            // survive to wrongly suppress the undo boundary before
            // some LATER, unrelated self-insert, merging it into the
            // undo group `c'/`o'/`O' had just abandoned via this C-g.
            editor.suppress_next_undo_boundary = false;
            // M40-4: C-g never reaches `completion_popup_key` below (it's
            // handled here, ahead of that branch, same as every other
            // C-g special case) so it must clear the popup itself.
            editor.completion_popup = None;
        }
        // M29: C-g is intercepted here ahead of all keymap dispatch (see
        // the module doc), so an emulation layer's own keymap never gets
        // a chance to bind it directly — e.g. evil-mode needs C-g to
        // cancel a pending operator (`d` left hanging) back to normal
        // state. `keyboard-quit-hook` (mirroring GNU Emacs 29's hook of
        // the same name) is the escape hatch: unbound by default (every
        // existing buffer that never adds to it sees no change at all —
        // `run_hook_by_name` on a void symbol is a no-op), so this is
        // purely additive.
        run_hook_by_name(interp, "keyboard-quit-hook");
        // M50: `finish_command` runs `post-command-hook` and updates
        // `last_command`, so it runs AFTER `keyboard-quit-hook` — the
        // hook is "the user cancelled", `finish_command`'s hook is "this
        // command cycle is over"; cancel-then-close-out is the right
        // order. Only when a minibuffer was actually open AND its
        // opener's cycle is still unclosed (`had_minibuffer` /
        // `opener_cycle_open`, both above) — see that binding's comment
        // for why a bare C-g, or one cancelling a CPS `read-string`/
        // `completing-read` prompt, must NOT reach this.
        if had_minibuffer && opener_cycle_open {
            finish_command(interp, ed);
        }
        return;
    }
    // M40-4: the LSP completion popup gets first crack at the key, same
    // "modal session intercepts ahead of everything else" shape as the
    // isearch branch above (and, like isearch, past the C-g special case,
    // which always needs its own handling regardless of what else is
    // active). `completion_popup_key` returns false either when there's
    // no popup open (nothing to intercept) or when this key closed it and
    // should still reach normal dispatch (any key besides the ones the
    // popup itself binds) — see its own doc comment.
    if completion_popup_key(interp, ed, &key) {
        return;
    }
    // M44-3: captured before the key actually does anything, so the
    // tail below can tell "point moved during this key event" (a pure
    // motion command reached dispatch, e.g. some future non-edit
    // binding on a self-insert/DEL key) apart from "point is wherever
    // the edit that just happened left it" — see
    // `refilter_completion_popup`'s doc comment.
    let point_before = ed.borrow().current.borrow().point;
    let in_minibuffer = ed.borrow().minibuffer.is_some();
    if in_minibuffer {
        minibuffer_key(interp, ed, key);
    } else {
        dispatch_key(interp, ed, key);
    }
    refilter_completion_popup(interp, ed, point_before);
}

/// M44-3 filter-as-you-type collation point: called at the tail of every
/// `handle_key`, after `dispatch_key`/`minibuffer_key` has already run
/// for a key `completion_popup_key` chose NOT to consume (a self-insert
/// character, DEL/backspace, or — today unreachable but handled anyway,
/// defense in depth like `completion_popup_key`'s own stale-session
/// guard — any other key that somehow reached ordinary dispatch with the
/// popup still open). A single collation point rather than hooking every
/// individual edit command: self-insert, backspace, AND electric-pair's
/// extra auto-inserted close paren can all land in the same key event
/// (`post-self-insert-hook` runs synchronously inside `self_insert`), so
/// only the FINAL state after the whole event matters — a no-op if
/// `completion_popup` is `None` (the overwhelmingly common case: no
/// popup was open to begin with).
///
/// Compares the buffer's current `edit_ticks` against the tick the popup
/// recorded when it was last opened or refiltered:
///  - tick changed, popup not `incomplete`, POINT still `>=`
///    `CompletionPopup::prefix_start` — refilter every item against the
///    SAME popup-wide `prefix_start..point` typed span (M44 review fix
///    #1: not each item's own `start`, see `CompletionPopup::
///    prefix_start`'s doc comment for why a per-item anchor silently
///    drops candidates like postfix completions whose `filterText`
///    doesn't cover what their own `textEdit` replaces); empty afterward
///    closes the popup, otherwise `selected` resets to 0 and the popup's
///    tick is updated to match.
///  - tick changed, POINT backed up before `prefix_start` — nothing left
///    for the popup to be replacing (e.g. backspacing past where the
///    identifier run started); close it outright, same outcome the old
///    per-item check gave for its own candidate but now decided once for
///    the whole popup.
///  - tick changed, popup `incomplete` — the server said its answer was
///    partial, so narrowing it further isn't right; close the popup and
///    re-request via `lsp-completion-at-point` instead (that function
///    has its own alive-gate/staleness guards; a failure here just
///    echoes a message, same "never blocks typing" spirit as the rest
///    of the LSP client).
///  - tick unchanged, POINT moved during this key event — a pure motion
///    with no edit at all; close the popup (vim/VS Code convention).
///  - neither changed — leave the popup exactly as it is.
fn refilter_completion_popup(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, point_before: usize) {
    enum Action {
        Nothing,
        Close,
        Refilter {
            tick: u64,
            kept: Vec<crate::editor::PopupItem>,
        },
        Requery,
    }

    // Short-lived borrows only: the Requery action below calls into
    // elisp (`apply_function`), which must never run with an Editor or
    // Buffer borrow still active.
    let action = {
        let editor = ed.borrow();
        let Some(popup) = editor.completion_popup.as_ref() else {
            return;
        };
        let buf = editor.current.clone();
        let (tick, point) = {
            let b = buf.borrow();
            (b.edit_ticks, b.point)
        };
        if tick != popup.tick {
            if popup.incomplete {
                Action::Requery
            } else if point < popup.prefix_start {
                // M44 review fix #1: this used to be a per-item check
                // (`point < item.start`) that only dropped THAT
                // candidate; a popup-wide prefix_start means there's
                // nothing left ANY candidate could still be replacing,
                // so the whole popup closes instead of narrowing to
                // empty.
                Action::Close
            } else {
                let typed = buf.borrow().text.slice(popup.prefix_start, point);
                let mut kept = Vec::new();
                for item in &popup.items {
                    if item.filter.starts_with(&typed) {
                        kept.push(item.clone());
                    }
                }
                Action::Refilter { tick, kept }
            }
        } else if point != point_before {
            Action::Close
        } else {
            Action::Nothing
        }
    };

    match action {
        Action::Nothing => {}
        Action::Close => {
            ed.borrow_mut().completion_popup = None;
        }
        Action::Refilter { tick, kept } => {
            if kept.is_empty() {
                ed.borrow_mut().completion_popup = None;
            } else {
                let mut editor = ed.borrow_mut();
                if let Some(popup) = editor.completion_popup.as_mut() {
                    popup.items = kept;
                    popup.selected = 0;
                    popup.tick = tick;
                }
            }
        }
        Action::Requery => {
            ed.borrow_mut().completion_popup = None;
            if let Some(sym) = interp.intern_soft("lsp-completion-at-point") {
                if let Err(flow) = apply_function(interp, &Value::Sym(sym), vec![].into()) {
                    let msg = interp.describe_flow(&flow);
                    ed.borrow_mut().echo(msg);
                }
            }
        }
    }
}

pub(crate) enum Lookup {
    Command(Value),
    Prefix,
    Undefined,
}

pub(crate) fn lookup_in(map: &Value, keys: &[Key]) -> Lookup {
    let Some(mut current) = as_keymap(map) else {
        return Lookup::Undefined;
    };
    for (i, key) in keys.iter().enumerate() {
        match current.get(key) {
            None => return Lookup::Undefined,
            Some(v) => {
                if let Some(sub) = as_keymap(&v) {
                    current = sub;
                } else if i + 1 == keys.len() {
                    return Lookup::Command(v);
                } else {
                    return Lookup::Undefined;
                }
            }
        }
    }
    Lookup::Prefix
}

/// Which of the three layered keymaps (M28's `emulation-keymap`, a
/// buffer's own `local` keymap, or the `global` keymap) a `lookup_layered`
/// result came from — M67's `lookup-key`/`all-key-bindings` builtins
/// report this alongside the binding so elisp can tell them apart.
pub(crate) enum Layer {
    Emulation,
    Local,
    Global,
}

impl Layer {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Layer::Emulation => "emulation",
            Layer::Local => "local",
            Layer::Global => "global",
        }
    }
}

/// The three keymap `Value`s `dispatch_key` consults, for the CURRENT
/// buffer, in priority order — factored out (M67) so `lookup-key`/
/// `all-key-bindings` can build the exact same layer set `dispatch_key`
/// itself would see for a real keystroke, without duplicating the
/// buffer-local `emulation-keymap` lookup dance.
pub(crate) fn keymap_layers(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
) -> (Value, Value, Value) {
    let editor = ed.borrow();
    let buf = editor.current.clone();
    let emulation = match interp.intern_soft("emulation-keymap") {
        Some(sym) => {
            crate::editor::buffer_local_value(interp, &editor, sym, &buf).unwrap_or(Value::Nil)
        }
        None => Value::Nil,
    };
    let local = buf.borrow().keymap.clone();
    let global = editor.global_keymap.clone();
    (emulation, local, global)
}

/// The layered 3-tier lookup itself (M67 extraction from `dispatch_key`):
/// a `Prefix` or `Command` hit at a given layer wins outright — like
/// Emacs keymap shadowing, a longer or different binding in a
/// lower-priority keymap is never consulted once a higher one claims the
/// prefix — only `Undefined` falls through to the next layer. Shared by
/// `dispatch_key` (the real keystroke path) and the `lookup-key` elisp
/// builtin (M67's `describe-key`), so the two can never drift apart —
/// `describe-key` reports the SAME decision the real dispatcher would
/// make for that key sequence.
pub(crate) fn lookup_layered(
    emulation: &Value,
    local: &Value,
    global: &Value,
    keys: &[Key],
) -> (Lookup, Layer) {
    match lookup_in(emulation, keys) {
        Lookup::Undefined => match lookup_in(local, keys) {
            Lookup::Undefined => (lookup_in(global, keys), Layer::Global),
            other => (other, Layer::Local),
        },
        other => (other, Layer::Emulation),
    }
}

/// Convert an elisp key value (as produced by `key_to_value` for
/// `capture-next-key`, and by extension whatever `describe-key` collects
/// via it) back into a `Key`. `None` for anything else — callers turn
/// that into `wrong-type-argument`.
pub(crate) fn value_to_key(interp: &Interp, v: &Value) -> Option<Key> {
    match v {
        Value::Int(n) => Some(Key::Char(*n)),
        Value::Sym(id) => Some(Key::Sym(interp.sym_name(*id).to_string())),
        _ => None,
    }
}

/// Handle one key while an incremental search is active. Returns true if
/// the key was consumed (search stays open), false if the search ended
/// and this key should fall through to normal dispatch.
fn isearch_key(ed: &Rc<RefCell<Editor>>, key: &Key) -> bool {
    match key {
        Key::Char(19) => {
            // C-s: start (if just opened) or repeat forward.
            let forward = ed
                .borrow()
                .isearch
                .as_ref()
                .map(|s| s.forward)
                .unwrap_or(true);
            if !forward {
                ed.borrow_mut().isearch = None;
                crate::editor::isearch_start(ed, true);
            } else {
                crate::editor::isearch_repeat(ed);
            }
            true
        }
        Key::Char(18) => {
            let forward = ed
                .borrow()
                .isearch
                .as_ref()
                .map(|s| s.forward)
                .unwrap_or(true);
            if forward {
                ed.borrow_mut().isearch = None;
                crate::editor::isearch_start(ed, false);
            } else {
                crate::editor::isearch_repeat(ed);
            }
            true
        }
        Key::Char(127) | Key::Char(8) => {
            crate::editor::isearch_pop_char(ed);
            true
        }
        Key::Char(13) | Key::Char(27) => {
            crate::editor::isearch_exit(ed);
            true
        }
        Key::Char(c) if is_self_insert_char(*c) && *c != 9 => {
            crate::editor::isearch_push_char(ed, char::from_u32(*c as u32).unwrap());
            true
        }
        _ => {
            crate::editor::isearch_exit(ed);
            false
        }
    }
}

/// Handle one key while the LSP completion popup (M40-4) is open. Returns
/// true if the key was consumed (dispatch is done for this event), false
/// if there was no popup to begin with, or this key fell through to
/// normal dispatch — either because it closed the popup on its way out
/// (M18's "any editing key dismisses the popup" convention, still true
/// for keys this function doesn't otherwise recognize) or because it's
/// self-insert/DEL and the popup is meant to stay open through it
/// (M44-3, see below) — same consumed/fall-through contract as
/// `isearch_key`.
///
/// C-n/down and C-p/up cycle the selection (wrapping); this shadows M31's
/// dabbrev C-n/C-p in insert state, but only while the popup is open —
/// closed, this function is a no-op and dabbrev gets the keys exactly as
/// before, so there's no real conflict, just a temporary priority
/// override matching what the open popup is showing on screen. RET/TAB
/// accept the highlighted candidate; ESC closes the popup without
/// exiting insert state (a second ESC, now reaching evil's own binding,
/// does that).
///
/// M44-3 filter-as-you-type: a self-insert character (TAB excepted —
/// `Key::Char(9)` is caught by the RET/TAB arm above, still an accept)
/// or DEL/backspace (`Key::Char(127)`, both the plain
/// `delete-backward-char` global binding and prog-mode's
/// `electric-pair-backward-delete` local override arrive as this same
/// `Key` — see `frontend-tui`/`frontend-gui`'s `KeyCode::Backspace` ->
/// `Key::Char(127)` mapping and `keymap.rs`'s `"DEL" => 127`) leaves the
/// popup open and falls through unconsumed instead of closing it, so
/// the character actually gets typed / the char actually gets deleted;
/// `handle_key`'s tail (`refilter_completion_popup`, run after
/// `dispatch_key`/`minibuffer_key` return) is what decides afterward
/// whether the now-edited buffer should narrow the candidate list,
/// re-request (`isIncomplete`), or close the popup outright. Any OTHER
/// key still closes the popup and falls through immediately, unchanged
/// from M40-4.
fn completion_popup_key(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, key: &Key) -> bool {
    if ed.borrow().completion_popup.is_none() {
        return false;
    }
    // M40-4 review fix (#3): a popup that's somehow still open once a
    // minibuffer/isearch/key-capture session is active must never
    // intercept that session's keys -- `show-completion-popup`'s own
    // guard (builtins/ui.rs) is meant to keep this from happening in
    // the first place, but `handle_key` dispatches THIS function ahead
    // of its `in_minibuffer` check, so the minibuffer has no protection
    // of its own besides this one. (isearch and key_capture each return
    // out of `handle_key` in their own branch above this one, so they
    // can't actually reach here today -- this covers them anyway as
    // defense in depth against that ordering ever changing.) Clear the
    // popup rather than leave it for the NEXT key to trip over, and
    // fall through unconsumed so this key reaches its real destination.
    //
    // M53: `isearch_live()` rather than the raw `isearch.is_some()`, so
    // that if the ordering above ever does change, a session already
    // made stale by a `switch-to-buffer`/`kill-buffer` isn't mistaken
    // for a live one. Unlike this file's other two readers, this one is
    // unreachable with `isearch` still `Some` today, so no test can tell
    // the two spellings apart -- see `dev/mutations/m53.py`'s header.
    {
        let stale = {
            let editor = ed.borrow();
            editor.minibuffer.is_some() || editor.isearch_live() || editor.key_capture.is_some()
        };
        if stale {
            ed.borrow_mut().completion_popup = None;
            return false;
        }
    }
    match key {
        Key::Char(14) => {
            move_completion_selection(ed, 1);
            true
        }
        Key::Sym(s) if s == "down" => {
            move_completion_selection(ed, 1);
            true
        }
        Key::Char(16) => {
            move_completion_selection(ed, -1);
            true
        }
        Key::Sym(s) if s == "up" => {
            move_completion_selection(ed, -1);
            true
        }
        Key::Char(13) | Key::Char(9) => {
            accept_completion(interp, ed);
            true
        }
        Key::Char(27) => {
            ed.borrow_mut().completion_popup = None;
            true
        }
        // M44-3: self-insert (TAB already claimed by the accept arm
        // above) — keep the popup open, let the character land via the
        // ordinary self-insert fallback, `refilter_completion_popup`
        // (handle_key's tail) narrows/closes it afterward.
        Key::Char(c) if is_self_insert_char(*c) && *c != 9 => false,
        // M44-3: DEL/backspace — same "leave it to the tail" treatment,
        // whether it lands on the global `delete-backward-char` or a
        // local `electric-pair-backward-delete` override.
        Key::Char(127) => false,
        _ => {
            ed.borrow_mut().completion_popup = None;
            false
        }
    }
}

fn move_completion_selection(ed: &Rc<RefCell<Editor>>, delta: i64) {
    let mut editor = ed.borrow_mut();
    if let Some(popup) = editor.completion_popup.as_mut() {
        let n = popup.items.len() as i64;
        if n > 0 {
            popup.selected = (popup.selected as i64 + delta).rem_euclid(n) as usize;
        }
    }
}

/// Accept the highlighted candidate: `delete-region(start, point)` then
/// insert its text, via the same internal `edit_delete`/`edit_insert`
/// path `self_insert` and the `delete-region`/`insert` builtins use
/// (read-only buffers refused the same way, through `check_writable`).
/// The two edits land in the same key event with no `undo_boundary`
/// between them, so they merge into one undo group for free — the same
/// electric-pair precedent noted on `PopupItem::start`'s use here.
///
/// M40-4 review fix (#5): this bypasses `execute_command` entirely (it's
/// reached from `completion_popup_key`, not keymap dispatch), so it must
/// reproduce that function's undo-grouping preamble itself -- an
/// `undo_boundary()` plus resetting `consec_inserts` to 0 -- or the
/// delete+insert below merges into whichever undo group the typing that
/// triggered completion left open, AND the typing that follows
/// acceptance merges into THAT (since `self_insert` only cuts a fresh
/// boundary when `consec_inserts` is 0 or has hit its cap, and nothing
/// else would reset it here). Accepting a candidate must be exactly one
/// undo step, no more, no less -- matching what `execute_command` gives
/// every other command for free. The preamble also clears
/// `suppress_next_undo_boundary` like `execute_command` does: no reachable
/// path leaves it pending here (arming it takes a command dispatch, which
/// clears it on entry, and the popup can't open in evil's non-insert
/// states), but acceptance is command-like, so it invalidates a pending
/// suppression under the same M30 rule rather than trusting that argument.
fn accept_completion(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    let extracted = {
        let editor = ed.borrow();
        editor.completion_popup.as_ref().map(|p| {
            let item = &p.items[p.selected];
            (item.start, item.insert.clone())
        })
    };
    let Some((start, text)) = extracted else {
        return;
    };
    let buf = ed.borrow().current.clone();
    if crate::builtins::check_writable(interp, &buf).is_ok() {
        buf.borrow_mut().undo_boundary();
        {
            let mut editor = ed.borrow_mut();
            editor.consec_inserts = 0;
            editor.suppress_next_undo_boundary = false;
        }
        let point = buf.borrow().point;
        let (s, e) = (start.min(point), start.max(point));
        crate::editor::edit_delete(ed, &buf, s, e);
        crate::editor::edit_insert(ed, &buf, s, &text);
    } else {
        let name = buf.borrow().name.clone();
        ed.borrow_mut()
            .echo(format!("Buffer is read-only: {}", name));
    }
    ed.borrow_mut().completion_popup = None;
}

fn dispatch_key(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, key: Key) {
    let (keys, inhibit_self_insert) = {
        let mut editor = ed.borrow_mut();
        editor.pending_keys.push(key);
        let buf = editor.current.clone();
        // M34: buffer-local `inhibit-self-insert`, consulted ONLY at the
        // self-insert fallback in the `Undefined` arm below — never for
        // the three keymap lookups themselves (a real binding in any of
        // them, e.g. `C-x b`/`M-x` under evil's normal state, dispatches
        // exactly as before). This is what lets an emulation layer's
        // modal state (evil's normal/visual/operator-pending) swallow an
        // unbound printable character — most importantly one outside the
        // small ASCII vim keymap it can practically enumerate, e.g. a CJK
        // character — instead of it silently falling through to plain
        // self-insert.
        let inhibit_self_insert = match interp.intern_soft("inhibit-self-insert") {
            Some(sym) => {
                crate::editor::buffer_local_value(interp, &editor, sym, &buf).unwrap_or(Value::Nil)
            }
            None => Value::Nil,
        }
        .truthy();
        let keys = editor.pending_keys.clone();
        (keys, inhibit_self_insert)
    };
    // M28: an emulation layer (evil-mode and friends) binds keys via a
    // buffer-local `emulation-keymap` variable consulted ahead of the
    // ordinary local/global keymaps, mirroring Emacs's
    // `emulation-mode-map-alists`. Never set (or explicitly nil, the
    // default) means this layer is invisible: `lookup_in` treats a
    // non-keymap Value as Undefined, so dispatch below falls straight
    // through to local/global exactly as before M28.
    //
    // M67: the three-layer keymap fetch and the layered lookup itself are
    // shared with the `lookup-key` elisp builtin via `keymap_layers`/
    // `lookup_layered` — see their doc comments — so `describe-key`
    // reports exactly what a real keystroke would do here.
    let (emulation, local, global) = keymap_layers(interp, ed);
    let (result, _layer) = lookup_layered(&emulation, &local, &global, &keys);
    match result {
        Lookup::Command(cmd) => {
            ed.borrow_mut().pending_keys.clear();
            execute_command(interp, ed, cmd);
        }
        Lookup::Prefix => {
            let desc = key_sequence_description(&keys);
            ed.borrow_mut().echo(format!("{}-", desc));
        }
        Lookup::Undefined => {
            ed.borrow_mut().pending_keys.clear();
            if keys.len() == 1 && !inhibit_self_insert {
                if let Key::Char(c) = keys[0] {
                    if is_self_insert_char(c) {
                        self_insert(interp, ed, char::from_u32(c as u32).unwrap());
                        return;
                    }
                }
            }
            let desc = key_sequence_description(&keys);
            ed.borrow_mut().echo(format!("{} is undefined", desc));
        }
    }
}

fn is_self_insert_char(c: i64) -> bool {
    if c & (CTRL | META) != 0 {
        return false;
    }
    if c == 9 {
        return true; // TAB inserts itself unless bound
    }
    (32..=0x10FFFF).contains(&c) && c != 127
}

/// The elisp representation of a key event, for M28 `capture-next-key`:
/// a `Char` becomes the very same integer the reader produces for that
/// character (control/meta bits included — the same encoding `Key::Char`
/// already carries internally, e.g. `?\C-x` => 24), and a named key
/// becomes the symbol keymaps are indexed by (e.g. `up`, `C-M-up`).
fn key_to_value(interp: &mut Interp, key: &Key) -> Value {
    match key {
        Key::Char(c) => Value::Int(*c),
        Key::Sym(s) => Value::Sym(interp.intern(s)),
    }
}

fn self_insert(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, ch: char) {
    // Typing into a read-only buffer (dired etc.) echoes instead of
    // inserting (M19).
    {
        let buf = ed.borrow().current.clone();
        if crate::builtins::check_writable(interp, &buf).is_err() {
            let name = buf.borrow().name.clone();
            ed.borrow_mut()
                .echo(format!("Buffer is read-only: {}", name));
            return;
        }
    }
    let buf = {
        let mut editor = ed.borrow_mut();
        let consec = editor.consec_inserts;
        let buf = editor.current.clone();
        // Group ~20 consecutive self-inserts per undo boundary, like
        // Emacs -- unless M30's `undo-amalgamate-boundary' suppressed
        // this one (one-shot: always consumed here, whether or not it
        // actually skipped a boundary this time) so a change-operator's
        // delete or `o'/`O''s newline joins the group about to start
        // instead of the self-inserts starting a fresh one after it.
        let suppress = editor.suppress_next_undo_boundary;
        editor.suppress_next_undo_boundary = false;
        if (consec == 0 || consec >= 20) && !suppress {
            buf.borrow_mut().undo_boundary();
            editor.consec_inserts = 0;
        }
        editor.consec_inserts += 1;
        editor.last_command = Value::Nil;
        editor.this_command = Value::Nil;
        buf
    };
    let point = buf.borrow().point;
    crate::editor::edit_insert(ed, &buf, point, &ch.to_string());
    run_post_insert_hook(interp, ed, ch);
}

/// Hooks that sit on the keystroke path get a time budget (M15). Hooks
/// that don't (find-file-hook, mode hooks run from elisp) stay
/// unbudgeted: the guarantee this watchdog enforces is "typing never
/// blocks", not "no elisp is ever slow" — a large file's fontification
/// on open may legitimately take longer than a keystroke may.
const KEYSTROKE_HOOKS: [&str; 2] = ["post-command-hook", "post-self-insert-hook"];

/// Timeout strikes before a hook function is evicted from its hook.
const HOOK_STRIKES: u32 = 3;

/// Run a hook variable by name if it is bound and non-nil.
///
/// M15 watchdog semantics for keystroke-path hooks: each hook function
/// runs under `hook-time-budget` milliseconds (default 50; nil disables).
/// One that exceeds it is interrupted via `elisp-timeout` — which
/// `ignore-errors` inside the hook cannot swallow — named in the echo
/// area, and evicted after three strikes. The user's own commands are
/// never budgeted (they asked for them; C-g is their tool), only the
/// code that rides along on every keystroke. This is the line VS Code
/// draws: an extension's command may be slow, the typing pipeline can't.
pub fn run_hook_by_name(interp: &mut Interp, name: &str) {
    let hook = interp.intern(name);
    let Some(val) = interp.sym_value(hook) else {
        return;
    };
    if !val.truthy() {
        return;
    }
    // A hook value may be a single function or a list of functions.
    let fns: Vec<Value> = match val.list_to_vec() {
        Some(items) => items,
        None => vec![val.clone()],
    };
    let budget_ms = if KEYSTROKE_HOOKS.contains(&name) {
        let budget_sym = interp.intern("hook-time-budget");
        match interp.sym_value(budget_sym) {
            Some(Value::Int(ms)) if ms > 0 => Some(ms as u64),
            _ => None,
        }
    } else {
        None
    };
    for f in fns {
        let result = match budget_ms {
            None => apply_function(interp, &f, vec![].into()),
            Some(ms) => {
                let target = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                let saved = interp.deadline;
                interp.deadline = Some(match saved {
                    Some(outer) => outer.min(target),
                    None => target,
                });
                let r = apply_function(interp, &f, vec![].into());
                interp.deadline = saved;
                r
            }
        };
        match result {
            Ok(_) => {}
            Err(flow) if flow_is_timeout(interp, &flow) => {
                note_hook_offense(interp, name, hook, &f, budget_ms.unwrap_or(0));
            }
            Err(flow) => {
                // Errors in hooks must not kill the keystroke, but
                // staying silent hides real bugs — echo them, like Emacs.
                let msg = interp.describe_flow(&flow);
                let fname = elisp::printer::prin1_to_string(interp, &f);
                interp.out(&format!("Error in {} ({}): {}\n", name, fname, msg));
            }
        }
    }
}

/// M85: like `run_hook_by_name`, but calls each hook function with ARG
/// as its single argument rather than zero arguments. Introduced for
/// `minibuffer-input-changed-hook` — a listener there (e.g. `*search*`'s
/// live filter, search.el) needs the new input text itself, and there
/// is no other way for elisp to read it: `mb.input` (`Editor`'s own
/// minibuffer state) has no elisp-visible accessor today, so the hook's
/// own argument IS the only way a listener ever sees it. Deliberately
/// does not carry `run_hook_by_name`'s keystroke time-budget machinery
/// (`KEYSTROKE_HOOKS`/`hook-time-budget`) — this hook is not in that
/// list, and every existing hook function registered on it is expected
/// to be cheap (a buffer redraw from already-in-memory data, not I/O).
pub fn run_hook_by_name_with_arg(interp: &mut Interp, name: &str, arg: Value) {
    let hook = interp.intern(name);
    let Some(val) = interp.sym_value(hook) else {
        return;
    };
    if !val.truthy() {
        return;
    }
    let fns: Vec<Value> = match val.list_to_vec() {
        Some(items) => items,
        None => vec![val.clone()],
    };
    for f in fns {
        if let Err(flow) = apply_function(interp, &f, vec![arg.clone()].into()) {
            let msg = interp.describe_flow(&flow);
            let fname = elisp::printer::prin1_to_string(interp, &f);
            interp.out(&format!("Error in {} ({}): {}\n", name, fname, msg));
        }
    }
}

/// Dispatch the current buffer's major mode via `normal-mode` (M24:
/// auto-mode-alist), called from the find-file paths just before
/// find-file-hook runs (GNU orders `set-auto-mode` ahead of
/// find-file-hook; `normal-mode` is our equivalent). Safety mirrors
/// `run_hook_by_name`: a fresh interpreter that hasn't loaded modes.el
/// yet silently does nothing rather than erroring, and an error raised
/// by a (possibly user-defined) major-mode function is echoed rather
/// than propagated — one broken mode must not stop the file from
/// opening.
pub fn run_normal_mode(interp: &mut Interp) {
    let Some(sym) = interp.intern_soft("normal-mode") else {
        return;
    };
    if interp.symbols[sym as usize].function.is_none() {
        return;
    }
    if let Err(flow) = apply_function(interp, &Value::Sym(sym), vec![].into()) {
        let msg = interp.describe_flow(&flow);
        interp.out(&format!("Error in normal-mode: {}\n", msg));
    }
}

fn flow_is_timeout(interp: &Interp, flow: &elisp::error::Flow) -> bool {
    matches!(flow,
        elisp::error::Flow::Signal { error_symbol: Value::Sym(id), .. }
            if *id == interp.syms.elisp_timeout)
}

/// One timeout strike: name the offender in the echo area; on the third
/// strike remove it from the hook so it cannot keep degrading typing.
fn note_hook_offense(
    interp: &mut Interp,
    hook_name: &str,
    hook: elisp::value::SymId,
    f: &Value,
    budget_ms: u64,
) {
    let fname = elisp::printer::prin1_to_string(interp, f);
    let key = format!("{}:{}", hook_name, fname);
    let ed = crate::editor::editor(interp);
    let strikes = {
        let mut e = ed.borrow_mut();
        let n = e.hook_offenses.entry(key).or_insert(0);
        *n += 1;
        *n
    };
    if strikes >= HOOK_STRIKES {
        // Evict: rebuild the hook list without this function (identity eq).
        if let Some(val) = interp.sym_value(hook) {
            if let Some(items) = val.list_to_vec() {
                let kept: Vec<Value> = items.into_iter().filter(|x| !x.eq(f)).collect();
                interp.symbols[hook as usize].value = Some(Value::list(kept));
            } else if val.eq(f) {
                interp.symbols[hook as usize].value = Some(Value::Nil);
            }
        }
        interp.out(&format!(
            "{}: {} removed after {} timeouts (exceeded {}ms budget each run)\n",
            hook_name, fname, HOOK_STRIKES, budget_ms
        ));
    } else {
        interp.out(&format!(
            "{}: {} exceeded {}ms and was interrupted ({}/{})\n",
            hook_name, fname, budget_ms, strikes, HOOK_STRIKES
        ));
    }
}

/// Lets modes react to typed text (org table alignment etc. later).
fn run_post_insert_hook(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, _ch: char) {
    run_hook_by_name(interp, "post-self-insert-hook");
    run_hook_by_name(interp, "post-command-hook");
    let _ = ed;
}

pub fn execute_command(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: Value) {
    {
        let mut editor = ed.borrow_mut();
        editor.this_command = cmd.clone();
        editor.consec_inserts = 0;
        // M30: any dispatched command -- not just a self-insert --
        // invalidates a pending `undo-amalgamate-boundary' suppression
        // (e.g. a change-operator immediately followed by ESC with
        // nothing typed in between: nothing ever consumes it via
        // `self_insert', so it must not silently survive to suppress
        // some unrelated LATER insert). This command's own boundary
        // below is never itself suppressed by the flag -- only
        // `self_insert' consults it.
        editor.suppress_next_undo_boundary = false;
        let buf = editor.current.clone();
        buf.borrow_mut().undo_boundary();
    }
    let spec = interactive_spec(interp, &cmd);
    match spec {
        InteractiveSpec::None | InteractiveSpec::Empty => {
            call_command(interp, ed, &cmd, Vec::new());
        }
        InteractiveSpec::Form(form) => match elisp::eval::eval(interp, &form, &None) {
            Ok(v) => {
                let args = v.list_to_vec().unwrap_or_default();
                call_command(interp, ed, &cmd, args);
            }
            Err(flow) => {
                let msg = interp.describe_flow(&flow);
                ed.borrow_mut().echo(msg);
                finish_command(interp, ed);
            }
        },
        InteractiveSpec::Codes(specs) => {
            let pending = PendingArgs {
                command: cmd,
                specs,
                collected: Vec::new(),
                index: 0,
                opener_cycle_closed: false,
            };
            process_pending(interp, ed, pending);
        }
    }
}

enum InteractiveSpec {
    None,
    Empty,
    Codes(Vec<ArgSpec>),
    Form(Value),
}

fn interactive_spec(interp: &mut Interp, cmd: &Value) -> InteractiveSpec {
    let Ok(f) = resolve_function(interp, cmd) else {
        return InteractiveSpec::None;
    };
    let Value::Func(func) = &f else {
        return InteractiveSpec::None;
    };
    // A byte-compiled function carries its own `.interactive` cell too
    // (byte-compile copies it over), so this must handle both — a
    // command doesn't stop being interactive just because it got
    // compiled for speed.
    let interactive = match func.as_ref() {
        Function::Lambda(l) => l.interactive.borrow().clone(),
        Function::Compiled(c) => c.interactive.borrow().clone(),
        // native-compiled functions carry their bytecode twin's
        // .interactive too (native-compile only replaces the fast
        // path; the interactive metadata still lives on `fallback`).
        Function::Native(n) => n.fallback.interactive.borrow().clone(),
        // No M-x-able module commands in v1 (see crate::module doc comment).
        Function::Builtin { .. } | Function::Module(_) => None,
    };
    let Some(spec_list) = interactive else {
        return InteractiveSpec::None;
    };
    match spec_list.car() {
        Value::Nil => InteractiveSpec::Empty,
        Value::Str(s) => InteractiveSpec::Codes(parse_spec_string(&s)),
        form => InteractiveSpec::Form(form),
    }
}

fn parse_spec_string(s: &str) -> Vec<ArgSpec> {
    let mut out = Vec::new();
    for part in s.split('\n') {
        let mut chars = part.chars();
        match chars.next() {
            Some('*') | Some('@') | Some('^') => {
                // Modifier prefixes we don't implement; the rest of this
                // part is still a spec.
                let rest: String = chars.collect();
                if !rest.is_empty() {
                    out.extend(parse_spec_string(&rest));
                }
            }
            Some(code) => out.push(ArgSpec {
                code,
                prompt: chars.collect(),
                history_key: code.to_string(),
                ..Default::default()
            }),
            None => {}
        }
    }
    out
}

pub fn process_pending(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, mut pending: PendingArgs) {
    while pending.index < pending.specs.len() {
        let code = pending.specs[pending.index].code;
        match code {
            'p' => {
                pending.collected.push(Value::Int(1));
                pending.index += 1;
            }
            'P' | 'i' => {
                pending.collected.push(Value::Nil);
                pending.index += 1;
            }
            'r' => {
                let (start, end) = {
                    let editor = ed.borrow();
                    let b = editor.current.borrow();
                    match b.mark {
                        Some(m) => (m.min(b.point), m.max(b.point)),
                        None => {
                            drop(b);
                            drop(editor);
                            ed.borrow_mut().echo("The mark is not set now");
                            // M48 Part E fix: this aborts before the command
                            // itself ever runs, so `call_command`'s own
                            // post-command-hook/`last_command` tail never
                            // fires either -- left undone, an emulation
                            // layer's pending-operator state (evil's
                            // `evil--pending-operator`/`evil--state`, reset
                            // by a `post-command-hook` function) stays
                            // stuck until some LATER, unrelated command
                            // happens to reach a real `call_command` and
                            // drags it down as collateral. Run just the
                            // hook/bookkeeping tail here, without invoking
                            // the command (there's nothing to invoke: arg
                            // collection failed).
                            finish_command(interp, ed);
                            return;
                        }
                    }
                };
                pending.collected.push(Value::Int(start as i64 + 1));
                pending.collected.push(Value::Int(end as i64 + 1));
                pending.index += 1;
            }
            _ => {
                let spec = &pending.specs[pending.index];
                let prompt = spec.prompt.clone();
                let source = crate::complete::source_for_spec(spec);
                // M47: an explicit INITIAL wins over the file-prompt
                // default-directory prefill; both still start with the
                // cursor at the end (below).
                let input = if let Some(initial) = spec.initial.clone() {
                    initial
                } else if matches!(code, 'f' | 'F' | 'D') {
                    // File prompts start from the default directory, like
                    // GNU Emacs (M18).
                    crate::complete::default_directory(ed)
                } else {
                    String::new()
                };
                let cursor = input.chars().count();
                // M84 D6: a fresh minibuffer session starts with a clean
                // Command/Function/Symbol candidate cache — see
                // `complete::reset_session_cache`'s doc comment for why
                // this must run before the panel-opening refresh below
                // (which is what actually populates the cache).
                crate::complete::reset_session_cache();
                ed.borrow_mut().minibuffer = Some(Minibuffer {
                    prompt,
                    input,
                    cursor,
                    pending,
                    note: None,
                    completion: None,
                    panel: None,
                    hist_pos: None,
                    hist_stash: String::new(),
                });
                // M21: file and buffer prompts open the selector panel
                // immediately, listing before any key is typed. M47
                // extends this to caller-provided (`completing-read`)
                // collections. M84 D4 extends it again to Command/
                // Function/Symbol (M-x and friends), which used to rely
                // on TAB opening a separate popup instead.
                if matches!(
                    source,
                    crate::complete::Source::File
                        | crate::complete::Source::Buffer
                        | crate::complete::Source::Custom(_)
                        | crate::complete::Source::Command
                        | crate::complete::Source::Function
                        | crate::complete::Source::Symbol
                ) {
                    crate::panel::refresh(interp, ed);
                }
                return;
            }
        }
    }
    let cmd = pending.command.clone();
    call_command(interp, ed, &cmd, pending.collected);
}

fn call_command(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, cmd: &Value, args: Vec<Value>) {
    match apply_function(interp, cmd, args.into()) {
        Ok(_) => {}
        Err(flow) => {
            let msg = interp.describe_flow(&flow);
            ed.borrow_mut().echo(msg);
        }
    }
    finish_command(interp, ed);
}

/// M48 Part E: the post-invocation tail every command run must get,
/// pulled out of `call_command` so a path that ABORTS before the command
/// itself is ever invoked (interactive-spec collection failing, e.g. `r`
/// with no mark set) can still run it. Runs `post-command-hook` and
/// updates `last_command` -- see `call_command`'s own former inline
/// version for why both matter (an emulation layer like evil-mode resets
/// pending state from `post-command-hook`).
fn finish_command(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    run_hook_by_name(interp, "post-command-hook");
    let mut editor = ed.borrow_mut();
    editor.last_command = editor.this_command.clone();
}

fn minibuffer_key(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, key: Key) {
    // The note is transient; any key that edits the input also
    // invalidates the completion popup (it's recomputed on TAB). The
    // M21 panel instead refreshes after the key (see the tail of this
    // function).
    {
        let keeps_popup = matches!(key, Key::Char(9) | Key::Char(13))
            || matches!(&key, Key::Sym(s) if s == "up" || s == "down");
        if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
            mb.note = None;
            if !keeps_popup {
                mb.completion = None;
            }
        }
    }
    let input_before = ed
        .borrow()
        .minibuffer
        .as_ref()
        .map(|mb| mb.input.clone())
        .unwrap_or_default();
    match key {
        Key::Char(13) => {
            // A highlighted panel row (M21) or popup candidate (M18) is
            // accepted into the input first; accepting a directory
            // keeps completing instead of submitting.
            let panel_action = {
                let mut editor = ed.borrow_mut();
                let mb = editor.minibuffer.as_mut().unwrap();
                match mb.panel.as_ref() {
                    Some(panel) => {
                        // File prompt with an untouched selection: RET
                        // submits the literal input when the stem is
                        // empty (C-x C-f RET on the prefilled directory
                        // = dired) or already names a directory exactly
                        // (typing a full dir path + RET = dired too) —
                        // not the highlighted row.
                        let stem: String = mb.input.chars().skip(panel.stem_start).collect();
                        let exact_dir = panel
                            .rows
                            .get(panel.selected)
                            .map(|r| r.accept == format!("{}/", stem))
                            .unwrap_or(false);
                        let literal = if matches!(panel.source, crate::complete::Source::File) {
                            !panel.chosen && (stem.is_empty() || exact_dir)
                        } else {
                            // User-typed exact match beats the highlighted
                            // row: `custom_filter` deliberately does NOT
                            // sort (keeps the caller's own order, e.g. LSP
                            // symbols in document order), so an exact match
                            // isn't guaranteed to land on row 0 the way
                            // `filter_sorted`'s dictionary order guarantees
                            // for File/Buffer -- collection ("alphabet"
                            // "alpha") + input "alpha" highlights row 0
                            // ("alphabet") while the user actually typed
                            // "alpha". `stem_start` is 0 for both Custom
                            // and Buffer, so `stem` equals `mb.input` here;
                            // for Buffer this is a no-op (already sorted,
                            // so an exact match is already row 0).
                            !panel.chosen && panel.rows.iter().any(|r| r.accept == stem)
                        };
                        match panel.rows.get(panel.selected) {
                            Some(row) if !literal => {
                                let accept = row.accept.clone();
                                let stem_start = panel.stem_start;
                                let idx = char_index(&mb.input, stem_start);
                                mb.input.truncate(idx);
                                mb.input.push_str(&accept);
                                mb.cursor = mb.input.chars().count();
                                Some(accept.ends_with('/'))
                            }
                            // Empty panel (or literal submit): the raw
                            // input goes through — new files get
                            // created this way.
                            _ => Some(false),
                        }
                    }
                    None => None,
                }
            };
            match panel_action {
                Some(true) => {
                    // Descended into a directory: stay in the
                    // minibuffer; the tail refresh relists.
                }
                Some(false) => submit_minibuffer(interp, ed),
                None => {
                    // F3 correction (M84 fix round): this used to be
                    // called "the M-x popup path", but M-x (`Source::
                    // Command`) has had its own panel since M84's D4/D5
                    // -- `panel_action` is `Some(...)` for M-x now, so
                    // this `None` arm only runs for sources with NO
                    // panel at all (`Source::None`, e.g. plain
                    // `read-string`). `mb.completion.take()` below is
                    // therefore always `None` in practice too: nothing
                    // populates `Minibuffer::completion` anymore (see
                    // its own doc comment in `editor.rs`) — this arm's
                    // real job now is just "fall through to the RET
                    // submit below", the popup-acceptance branch having
                    // gone permanently dead alongside it.
                    let accepted_dir = {
                        let mut editor = ed.borrow_mut();
                        let mb = editor.minibuffer.as_mut().unwrap();
                        match mb.completion.take() {
                            Some(cs) => match cs.candidates.get(cs.selected) {
                                Some(cand) => {
                                    let idx = char_index(&mb.input, cs.stem_start);
                                    mb.input.truncate(idx);
                                    mb.input.push_str(cand);
                                    mb.cursor = mb.input.chars().count();
                                    cand.ends_with('/')
                                }
                                None => false,
                            },
                            None => false,
                        }
                    };
                    if !accepted_dir {
                        submit_minibuffer(interp, ed);
                    }
                }
            }
        }
        // C-j: always submit the literal input, bypassing the panel
        // selection — the escape hatch when the new name is a prefix of
        // an existing one.
        Key::Char(10) => {
            submit_minibuffer(interp, ed);
        }
        // ESC cancels the minibuffer (M28: with esc_pending gone, ESC
        // no longer doubles as a terminal Meta prefix, so it's free to
        // mean "cancel" here — matching both modern editors and
        // evil-mode's Insert-state ESC). Deliberately narrower than the
        // top-level C-g branch: only the minibuffer closes; the current
        // buffer's mark/region is untouched, the same ESC-vs-C-g scope
        // split isearch already makes (ESC exits keeping point, C-g
        // aborts restoring it). M50: for the same reason ESC does NOT
        // run `keyboard-quit-hook` either, unlike C-g below — evil's
        // `evil--on-keyboard-quit' (bound on that hook) calls
        // `evil-insert-exit' in Insert state, which exits Insert state
        // and nudges point left. A user in Insert state who opens a
        // minibuffer (e.g. `M-x`) and presses ESC means "close the
        // minibuffer", not "also kick me out of Insert state" — that
        // would be the hook's C-g-flavored wide semantics leaking into
        // ESC's deliberately narrow one. `finish_command` (which runs
        // `post-command-hook` and updates `last_command`, but not
        // `keyboard-quit-hook`) still runs when the minibuffer's OPENING
        // command's cycle is still unclosed (`opener_cycle_closed` is
        // `false` — the `execute_command` spec-collection case): this
        // cancel ends that command's cycle, so `last_command` must
        // reflect it happened here, same as M48's other silent-abort
        // paths. M50 follow-up: when `opener_cycle_closed` is already `true`
        // (the CPS `read-string`/`completing-read` case,
        // `PendingArgs::opener_cycle_closed`'s doc comment), the
        // surrounding command's cycle was already closed by
        // `call_command` before this minibuffer ever opened — running
        // `finish_command` again here would be a second, false firing of
        // `post-command-hook` for a callback that (on cancel) never even
        // runs.
        Key::Char(27) => {
            let opener_cycle_open = ed
                .borrow()
                .minibuffer
                .as_ref()
                .is_some_and(|mb| !mb.pending.opener_cycle_closed);
            ed.borrow_mut().minibuffer = None;
            ed.borrow_mut().echo("Quit");
            if opener_cycle_open {
                finish_command(interp, ed);
            }
        }
        Key::Char(9) => {
            minibuffer_tab(interp, ed);
        }
        Key::Sym(ref s) if s == "down" => {
            if !crate::panel::move_selection(ed, 1) {
                if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                    if let Some(cs) = mb.completion.as_mut() {
                        if !cs.candidates.is_empty() {
                            cs.selected = (cs.selected + 1) % cs.candidates.len();
                        }
                    }
                }
            }
        }
        Key::Sym(ref s) if s == "up" => {
            if !crate::panel::move_selection(ed, -1) {
                if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                    if let Some(cs) = mb.completion.as_mut() {
                        if !cs.candidates.is_empty() {
                            cs.selected =
                                (cs.selected + cs.candidates.len() - 1) % cs.candidates.len();
                        }
                    }
                }
            }
        }
        Key::Char(127) | Key::Char(8) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                if mb.cursor > 0 {
                    let idx = char_index(&mb.input, mb.cursor - 1);
                    mb.input.remove(idx);
                    mb.cursor -= 1;
                }
            }
        }
        Key::Char(1) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                mb.cursor = 0;
            }
        }
        Key::Char(5) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                mb.cursor = mb.input.chars().count();
            }
        }
        Key::Char(2) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                if mb.cursor > 0 {
                    mb.cursor -= 1;
                }
            }
        }
        Key::Char(6) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                if mb.cursor < mb.input.chars().count() {
                    mb.cursor += 1;
                }
            }
        }
        Key::Sym(ref s) if s == "left" => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                if mb.cursor > 0 {
                    mb.cursor -= 1;
                }
            }
        }
        Key::Sym(ref s) if s == "right" => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                if mb.cursor < mb.input.chars().count() {
                    mb.cursor += 1;
                }
            }
        }
        Key::Char(11) => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                let idx = char_index(&mb.input, mb.cursor);
                mb.input.truncate(idx);
            }
        }
        // M47 Part D: M-p/M-n walk the current spec's history ring
        // (`Editor::minibuffer_history`, keyed by `ArgSpec::history_key`).
        Key::Char(c) if c == META | 'p' as i64 => {
            minibuffer_history_prev(ed);
        }
        Key::Char(c) if c == META | 'n' as i64 => {
            minibuffer_history_next(ed);
        }
        Key::Char(c) if is_self_insert_char(c) && c != 9 => {
            if let Some(mb) = ed.borrow_mut().minibuffer.as_mut() {
                let idx = char_index(&mb.input, mb.cursor);
                mb.input.insert(idx, char::from_u32(c as u32).unwrap());
                mb.cursor += 1;
            }
        }
        _ => {}
    }
    // M21 panel: any key that changed the input relists (and resets
    // the selection); a submit above already tore the minibuffer down,
    // making this a no-op.
    let input_changed = ed
        .borrow()
        .minibuffer
        .as_ref()
        .map(|mb| mb.panel.is_some() && mb.input != input_before)
        .unwrap_or(false);
    if input_changed {
        crate::panel::refresh(interp, ed);
    }
    // M85: `minibuffer-input-changed-hook` — a general-purpose
    // notification (NOT panel-specific, unlike `input_changed` above)
    // that the minibuffer's own input text just changed, passing the
    // new input string as the hook function's sole argument. Before
    // this, no elisp code could ever learn a minibuffer's input changed
    // at all except by way of the M21 panel machinery, which only
    // exists for `completing-read`-family prompts with candidates — a
    // plain `read-string` prompt (e.g. `*search*`'s live filter, M85)
    // has no panel and so never tripped `input_changed` above. Gated on
    // the same `mb.input != input_before` comparison, just without the
    // `mb.panel.is_some()` half of that condition.
    let changed_input = ed.borrow().minibuffer.as_ref().and_then(|mb| {
        if mb.input != input_before {
            Some(mb.input.clone())
        } else {
            None
        }
    });
    if let Some(input) = changed_input {
        run_hook_by_name_with_arg(
            interp,
            "minibuffer-input-changed-hook",
            Value::string(input),
        );
    }
}

/// M47: whether the current spec's REQUIRE-MATCH gate blocks submitting
/// `mb.input` as-is — sets the " [No match]" note and returns true when
/// it does, without touching `mb.pending`. Called from BOTH submit paths
/// (RET and C-j, via `submit_minibuffer` below) so C-j — normally the
/// escape hatch that bypasses the panel selection and submits the
/// literal input — can't be used to smuggle non-matching text past
/// `completing-read`'s REQUIRE-MATCH; a single check point means there's
/// only one place this can be gotten wrong. An empty collection with
/// REQUIRE-MATCH set makes submission permanently impossible — that's
/// correct, not a bug to special-case around.
fn require_match_blocks(mb: &mut Minibuffer) -> bool {
    let spec = &mb.pending.specs[mb.pending.index];
    if !spec.require_match {
        return false;
    }
    let ok = spec
        .collection
        .as_ref()
        .map(|c| c.iter().any(|s| s == &mb.input))
        .unwrap_or(false);
    if !ok {
        mb.note = Some(" [No match]".to_string());
    }
    !ok
}

/// Submit the minibuffer's literal input to the pending command.
fn submit_minibuffer(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    {
        let mut editor = ed.borrow_mut();
        let mb = editor.minibuffer.as_mut().unwrap();
        if require_match_blocks(mb) {
            return;
        }
    }
    let mb = ed.borrow_mut().minibuffer.take().unwrap();
    let mut pending = mb.pending;
    let code = pending.specs[pending.index].code;
    let history_key = pending.specs[pending.index].history_key.clone();
    let arg = convert_minibuffer_arg(interp, code, &mb.input);
    match arg {
        Ok(v) => {
            push_minibuffer_history(ed, &history_key, &mb.input);
            pending.collected.push(v);
            pending.index += 1;
            process_pending(interp, ed, pending);
        }
        Err(msg) => {
            ed.borrow_mut().echo(msg);
            finish_command(interp, ed);
        }
    }
}

/// M47 Part D: record INPUT into `history_key`'s ring on successful
/// submission. Empty input is never recorded (nothing worth recalling),
/// and a repeat of the ring's own most recent entry isn't pushed again
/// (so answering the same prompt with the same text repeatedly doesn't
/// pad the ring with duplicates) — capped at 100 entries per key, oldest
/// dropped first.
fn push_minibuffer_history(ed: &Rc<RefCell<Editor>>, history_key: &str, input: &str) {
    if input.is_empty() {
        return;
    }
    let mut editor = ed.borrow_mut();
    let ring = editor
        .minibuffer_history
        .entry(history_key.to_string())
        .or_default();
    if ring.last().map(|s| s.as_str()) == Some(input) {
        return;
    }
    ring.push(input.to_string());
    if ring.len() > 100 {
        ring.remove(0);
    }
}

/// M47 Part D: M-p — walk one entry further back (toward index 0, the
/// oldest) into the current spec's history ring, stashing the
/// live-edge input first so M-n can restore it later. A no-op with no
/// minibuffer open, an empty ring, or already at the oldest entry.
fn minibuffer_history_prev(ed: &Rc<RefCell<Editor>>) {
    let mut editor = ed.borrow_mut();
    let key = match editor.minibuffer.as_ref() {
        Some(mb) => mb.pending.specs[mb.pending.index].history_key.clone(),
        None => return,
    };
    let ring: Vec<String> = editor
        .minibuffer_history
        .get(&key)
        .cloned()
        .unwrap_or_default();
    if ring.is_empty() {
        return;
    }
    let Some(mb) = editor.minibuffer.as_mut() else {
        return;
    };
    let new_pos = match mb.hist_pos {
        None => {
            mb.hist_stash = mb.input.clone();
            ring.len() - 1
        }
        Some(0) => 0,
        Some(p) => p - 1,
    };
    mb.hist_pos = Some(new_pos);
    mb.input = ring[new_pos].clone();
    mb.cursor = mb.input.chars().count();
}

/// M47 Part D: M-n — walk one entry forward (toward the live edge) in
/// history; past the newest entry, restores `hist_stash` (the input the
/// user had typed before M-p was first pressed) and clears `hist_pos`.
/// A no-op with no minibuffer open or already at the live edge.
fn minibuffer_history_next(ed: &Rc<RefCell<Editor>>) {
    let mut editor = ed.borrow_mut();
    let key = match editor.minibuffer.as_ref() {
        Some(mb) => mb.pending.specs[mb.pending.index].history_key.clone(),
        None => return,
    };
    let ring: Vec<String> = editor
        .minibuffer_history
        .get(&key)
        .cloned()
        .unwrap_or_default();
    let Some(mb) = editor.minibuffer.as_mut() else {
        return;
    };
    match mb.hist_pos {
        None => {}
        Some(p) if p + 1 < ring.len() => {
            mb.hist_pos = Some(p + 1);
            mb.input = ring[p + 1].clone();
            mb.cursor = mb.input.chars().count();
        }
        Some(_) => {
            mb.hist_pos = None;
            mb.input = mb.hist_stash.clone();
            mb.cursor = mb.input.chars().count();
        }
    }
}

/// TAB in the minibuffer (M18): extend to the longest common prefix,
/// then open the candidate popup; further TABs cycle the selection.
/// With an M21 panel open, the panel plays the popup's role: extend
/// the prefix if possible, else cycle the panel selection.
///
/// F3 correction (M84 fix round): "then open the candidate popup" above
/// is now aspirational, not actual, for every `Source` this function
/// can reach. Since M84's D4/D5 gave Command/Function/Symbol their own
/// panels (joining File/Buffer/Custom, which already had one since
/// M21/M47), EVERY non-`Source::None` source opens a panel; the early
/// `if let Some((cands, stem_start)) = panel_cands { ... return; }`
/// branch below (the "M21 panel path") therefore always returns first
/// for them. `Source::None` sources return even earlier, at the
/// `if source == Source::None { return; }` check further down. So the
/// ENTIRE tail of this function from that point on — starting at
/// `let (stem_start, cands) = complete::candidates(interp, ed, source,
/// &input);` (fetching candidates for a source that, by construction,
/// can only be `Source::None` at that point) through to the final
/// `mb.completion = Some(CompletionState { ... })` assignment, roughly
/// 38 lines — is unreachable code as of this fix round, not just that
/// last assignment (H3 correction: an earlier pass at this comment
/// understated the dead range to just the last line). Kept rather than
/// deleted since removing it is a bigger cleanup than this fix round's
/// scope.
fn minibuffer_tab(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    use crate::complete::{self, Source};
    // M21 panel path.
    let panel_cands = {
        let editor = ed.borrow();
        editor.minibuffer.as_ref().and_then(|mb| {
            mb.panel.as_ref().map(|p| {
                (
                    p.rows.iter().map(|r| r.accept.clone()).collect::<Vec<_>>(),
                    p.stem_start,
                )
            })
        })
    };
    if let Some((cands, stem_start)) = panel_cands {
        let mut editor = ed.borrow_mut();
        let Some(mb) = editor.minibuffer.as_mut() else {
            return;
        };
        if cands.is_empty() {
            mb.note = Some(" [No match]".to_string());
            return;
        }
        let stem: String = mb.input.chars().skip(stem_start).collect();
        let lcp = complete::longest_common_prefix(&cands);
        // H1 (M84 fix round): expanding to `lcp` only makes sense when
        // `lcp` actually EXTENDS what the user typed -- comparing
        // lengths alone (the pre-fix check) doesn't guarantee that.
        // Under orderless matching `lcp` can share zero characters with
        // `stem` (e.g. stem "clkr rst" against candidates
        // "se-review-clkrstz1"/"se-review-clkrstz2", both only
        // substring matches on each token: their lcp is
        // "se-review-clkrstz", 17 chars, longer than the 8-char stem
        // but sharing no prefix with it at all) -- without this guard
        // TAB would silently replace the user's typed query with an
        // unrelated string, no `[No match]` or any other signal.
        if lcp.chars().count() > stem.chars().count() && lcp.starts_with(&stem) {
            let idx = char_index(&mb.input, stem_start);
            mb.input.truncate(idx);
            mb.input.push_str(&lcp);
            mb.cursor = mb.input.chars().count();
            if cands.len() == 1 && !lcp.ends_with('/') {
                mb.note = Some(" [Sole completion]".to_string());
            }
            // The input changed: minibuffer_key's tail refresh relists.
        } else if cands.len() == 1 {
            mb.note = Some(" [Sole completion]".to_string());
        } else if let Some(panel) = mb.panel.as_mut() {
            panel.selected = (panel.selected + 1) % panel.rows.len();
            panel.chosen = true;
        }
        return;
    }
    // An open popup: TAB just cycles.
    {
        let mut editor = ed.borrow_mut();
        let Some(mb) = editor.minibuffer.as_mut() else {
            return;
        };
        if let Some(cs) = mb.completion.as_mut() {
            if !cs.candidates.is_empty() {
                cs.selected = (cs.selected + 1) % cs.candidates.len();
            }
            return;
        }
    }
    let (source, input) = {
        let editor = ed.borrow();
        let mb = editor.minibuffer.as_ref().unwrap();
        let spec = &mb.pending.specs[mb.pending.index];
        (complete::source_for_spec(spec), mb.input.clone())
    };
    if source == Source::None {
        return;
    }
    let (stem_start, cands) = complete::candidates(interp, ed, source, &input);
    let mut editor = ed.borrow_mut();
    let Some(mb) = editor.minibuffer.as_mut() else {
        return;
    };
    if cands.is_empty() {
        mb.note = Some(" [No match]".to_string());
        return;
    }
    let stem: String = input.chars().skip(stem_start).collect();
    let lcp = complete::longest_common_prefix(&cands);
    // H1 (M84 fix round): same guard as the panel path above -- `lcp`
    // must actually extend `stem`, not just be longer than it. This
    // branch is currently unreachable (every non-`Source::None` source
    // has a panel now, and `Source::None` returned earlier above), so
    // nothing exercises this guard today; it's still fixed so that if
    // this path is ever revived, it doesn't come back carrying the same
    // silent-replacement bug H1 fixed in the reachable copy.
    if lcp.chars().count() > stem.chars().count() && lcp.starts_with(&stem) {
        // Grow the input to the common prefix. If that settles it, say
        // so; otherwise the next TAB opens the popup.
        let idx = char_index(&mb.input, stem_start);
        mb.input.truncate(idx);
        mb.input.push_str(&lcp);
        mb.cursor = mb.input.chars().count();
        if cands.len() == 1 && !lcp.ends_with('/') {
            mb.note = Some(" [Sole completion]".to_string());
        }
        return;
    }
    if cands.len() == 1 {
        mb.note = Some(" [Sole completion]".to_string());
        return;
    }
    mb.completion = Some(crate::editor::CompletionState {
        candidates: cands,
        selected: 0,
        stem_start,
    });
}

/// Whether `sym`'s function binding is an interactive command — the
/// M-x completion predicate (M18).
pub(crate) fn is_command(interp: &mut Interp, sym: elisp::value::SymId) -> bool {
    !matches!(
        interactive_spec(interp, &Value::Sym(sym)),
        InteractiveSpec::None
    )
}

fn char_index(s: &str, char_pos: usize) -> usize {
    s.char_indices()
        .nth(char_pos)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn convert_minibuffer_arg(interp: &mut Interp, code: char, input: &str) -> Result<Value, String> {
    match code {
        's' | 'M' | 'f' | 'F' | 'D' | 'b' | 'B' => Ok(Value::string(input)),
        'S' | 'C' | 'a' => {
            if input.is_empty() {
                return Err("No input".to_string());
            }
            let id = interp.intern(input);
            Ok(Value::Sym(id))
        }
        'n' | 'N' => {
            let t = input.trim();
            if let Ok(i) = t.parse::<i64>() {
                Ok(Value::Int(i))
            } else if let Ok(f) = t.parse::<f64>() {
                Ok(Value::Float(f))
            } else {
                Err(format!("Not a number: {}", input))
            }
        }
        'x' => {
            let mut r = elisp::reader::Reader::new(input);
            match r.read(interp) {
                Ok(Some(v)) => Ok(v),
                _ => Err("Invalid expression".to_string()),
            }
        }
        other => Err(format!("Unsupported interactive code: {}", other)),
    }
}

/// Feed a whole kbd string through handle_key (used by tests and scripts).
pub fn feed_keys(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, desc: &str) -> Result<(), String> {
    for key in keymap::parse_kbd(desc)? {
        handle_key(interp, ed, key);
    }
    Ok(())
}
