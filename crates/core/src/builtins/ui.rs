use std::cell::RefCell;
use std::rc::Rc;

use elisp::builtins::{defun, need_int, need_list, need_str, need_sym, opt};
use elisp::error::Flow;
use elisp::value::ExtRef;
use elisp::{Interp, Value};

use super::{buffer_arg, cur, ed_handle, get_pos, int_pos, MARKER_TAG, OVERLAY_TAG};
use crate::buffer::{trace_overlay, MarkerData, OverlayData};
use crate::commands::{ArgSpec, Key, PendingArgs};
use crate::keymap::{as_keymap, make_keymap, parse_kbd, Keymap};

pub fn register(interp: &mut Interp) {
    // Keymaps.
    defun(interp, "make-sparse-keymap", 0, Some(1), |_, _| {
        Ok(make_keymap())
    });
    defun(interp, "keymapp", 1, Some(1), |i, a| {
        Ok(Value::bool(as_keymap(&a[0]).is_some(), i.syms.t))
    });
    defun(interp, "kbd", 1, Some(1), |_, a| Ok(a[0].clone()));
    defun(interp, "define-key", 3, Some(3), |i, a| {
        let map = as_keymap(&a[0]).ok_or_else(|| i.wrong_type("keymapp", &a[0]))?;
        let desc = need_str(i, &a[1])?;
        let keys = parse_kbd(&desc).map_err(|e| i.error(e))?;
        Keymap::define_sequence(&map, &keys, a[2].clone());
        Ok(a[2].clone())
    });
    defun(interp, "global-set-key", 2, Some(2), |i, a| {
        let ed = ed_handle(i);
        let gmap = ed.borrow().global_keymap.clone();
        let map = as_keymap(&gmap).ok_or_else(|| i.error("no global keymap"))?;
        let desc = need_str(i, &a[0])?;
        let keys = parse_kbd(&desc).map_err(|e| i.error(e))?;
        Keymap::define_sequence(&map, &keys, a[1].clone());
        Ok(Value::Nil)
    });
    defun(interp, "local-set-key", 2, Some(2), |i, a| {
        let b = cur(i);
        let existing = b.borrow().keymap.clone();
        let map_val = if as_keymap(&existing).is_some() {
            existing
        } else {
            let m = make_keymap();
            b.borrow_mut().keymap = m.clone();
            m
        };
        let map = as_keymap(&map_val).unwrap();
        let desc = need_str(i, &a[0])?;
        let keys = parse_kbd(&desc).map_err(|e| i.error(e))?;
        Keymap::define_sequence(&map, &keys, a[1].clone());
        Ok(Value::Nil)
    });
    defun(interp, "use-local-map", 1, Some(1), |i, a| {
        if !a[0].is_nil() && as_keymap(&a[0]).is_none() {
            return Err(i.wrong_type("keymapp", &a[0]));
        }
        cur(i).borrow_mut().keymap = a[0].clone();
        Ok(Value::Nil)
    });
    defun(interp, "current-global-map", 0, Some(0), |i, _| {
        Ok(ed_handle(i).borrow().global_keymap.clone())
    });
    defun(interp, "current-local-map", 0, Some(0), |i, _| {
        Ok(cur(i).borrow().keymap.clone())
    });
    // (key-description SEQ) — M67: SEQ is a list of elisp key values (the
    // same shape `capture-next-key`'s callback receives, and so what a
    // `describe-key` collector accumulates one keystroke at a time — see
    // `commands::key_to_value`). Converts back to `Key` and defers to the
    // SAME `keymap::key_sequence_description` the "... is undefined" echo
    // (commands.rs) and prefix-echo already use, so a stringified key
    // sequence can never drift from what those paths print.
    defun(interp, "key-description", 1, Some(1), |i, a| {
        let items = need_list(i, &a[0])?;
        let mut keys = Vec::with_capacity(items.len());
        for v in &items {
            keys.push(
                crate::commands::value_to_key(i, v)
                    .ok_or_else(|| i.wrong_type("integer-or-symbol-p", v))?,
            );
        }
        Ok(Value::string(crate::keymap::key_sequence_description(
            &keys,
        )))
    });
    // (lookup-key SEQ) — M67: resolve SEQ (same shape as `key-description`
    // above) through the exact three-layer decision `dispatch_key` itself
    // makes for a real keystroke (`commands::keymap_layers` +
    // `lookup_layered`, both shared with that function — see their doc
    // comments), so `describe-key` reports what pressing SEQ would
    // actually run, never a parallel guess. Returns nil when SEQ is
    // unbound, otherwise `(STATE BINDING LAYER)`: STATE is the symbol
    // `prefix` (BINDING nil) or `command`; LAYER is `emulation`/`local`/
    // `global`, naming which of the three keymaps the hit came from.
    defun(interp, "lookup-key", 1, Some(1), |i, a| {
        let items = need_list(i, &a[0])?;
        let mut keys = Vec::with_capacity(items.len());
        for v in &items {
            keys.push(
                crate::commands::value_to_key(i, v)
                    .ok_or_else(|| i.wrong_type("integer-or-symbol-p", v))?,
            );
        }
        let ed = ed_handle(i);
        let (emulation, local, global) = crate::commands::keymap_layers(i, &ed);
        let (result, layer) = crate::commands::lookup_layered(&emulation, &local, &global, &keys);
        match result {
            crate::commands::Lookup::Undefined => Ok(Value::Nil),
            crate::commands::Lookup::Prefix => {
                let state = Value::Sym(i.intern("prefix"));
                let layer = Value::Sym(i.intern(layer.name()));
                Ok(Value::list(vec![state, Value::Nil, layer]))
            }
            crate::commands::Lookup::Command(binding) => {
                let state = Value::Sym(i.intern("command"));
                let layer = Value::Sym(i.intern(layer.name()));
                Ok(Value::list(vec![state, binding, layer]))
            }
        }
    });
    // (all-key-bindings) — M67: every binding currently reachable through
    // the three layered keymaps (`emulation-keymap`/local/global, same
    // set `keymap_layers` builds for `lookup-key` above), flattened to
    // full key-sequence strings via `keymap::enumerate_bindings` — a
    // nested prefix map like the existing `C-h .` appears as one entry
    // keyed `"C-h ."`, never just `"C-h"`. A layer that isn't a keymap
    // right now (most commonly `emulation-keymap` still nil) contributes
    // no entries at all rather than an error. Returns a list of
    // `(LAYER KEYDESC BINDING)`, LAYER as in `lookup-key`.
    defun(interp, "all-key-bindings", 0, Some(0), |i, _| {
        let ed = ed_handle(i);
        let (emulation, local, global) = crate::commands::keymap_layers(i, &ed);
        let mut out = Vec::new();
        for (name, val) in [
            ("emulation", &emulation),
            ("local", &local),
            ("global", &global),
        ] {
            if let Some(map) = as_keymap(val) {
                for (keys, binding) in crate::keymap::enumerate_bindings(&map, 0) {
                    let layer = Value::Sym(i.intern(name));
                    let desc = Value::string(crate::keymap::key_sequence_description(&keys));
                    out.push(Value::list(vec![layer, desc, binding]));
                }
            }
        }
        Ok(Value::list(out))
    });
    // (capture-next-key FUNCTION) — M28: steal the very next key event
    // away from all keymap dispatch (emulation/local/global) and hand it
    // to FUNCTION as one argument, the key's elisp representation (an
    // integer for a character, a symbol for a named key — see
    // `commands::key_to_value`). One-shot: FUNCTION must call
    // capture-next-key again itself to keep capturing, which is exactly
    // how evil's `f`/`t`-then-`;` character search is meant to build on
    // this. Only C-g cancels a pending capture without calling
    // FUNCTION; ESC is delivered as an ordinary key (integer 27) — a
    // deliberate API boundary, keeping the primitive general. A
    // vim-style "ESC aborts the pending f/t/r" belongs in the
    // consumer: evil.el's capture callbacks check for 27 themselves.
    //
    // Known gap (v1, documented not fixed): no reentrancy guard.
    // `ed.key_capture` is a single slot, so calling `capture-next-key`
    // again while a capture is already pending (e.g. from inside
    // another callback that itself runs before the first captured key
    // arrives) silently overwrites it -- the first pending callback is
    // dropped on the floor with no error, same failure shape
    // `read_from_minibuffer_impl`'s explicit re-entry check exists to
    // prevent for the minibuffer, but no equivalent check exists here.
    defun(interp, "capture-next-key", 1, Some(1), |i, a| {
        let ed = ed_handle(i);
        ed.borrow_mut().key_capture = Some(a[0].clone());
        Ok(Value::Nil)
    });

    // M42-II: keyboard macros (evil's `q`/`@`). Recording itself is a
    // Rust-side tap in `commands::handle_key` (see its own doc comment);
    // these five builtins are the whole elisp-facing surface: arm/
    // disarm recording, replay, and query — see `Editor::
    // kbd_macro_recording`/`kbd_macros`/`macro_replay_depth` for the
    // storage model.
    //
    // (start-kbd-macro REG) — begin recording into register REG (any
    // integer; evil.el itself restricts callers to a-z before this ever
    // runs, matching `evil--set-local-mark`/`evil--set-register`'s own
    // division of labor). Starting a NEW recording silently replaces
    // whatever was already stored there — no confirmation, no append
    // mode (v1, matching every other M42 register: see evil.el's own
    // doc notes).
    defun(interp, "start-kbd-macro", 1, Some(1), |i, a| {
        let reg = need_int(i, &a[0])?;
        ed_handle(i).borrow_mut().kbd_macro_recording = Some((reg, Vec::new()));
        Ok(Value::Nil)
    });
    // (end-kbd-macro TRIM) — stop recording (a harmless no-op if
    // nothing was recording), dropping the TRIM most-recently-recorded
    // keys before storing the rest under the armed register. TRIM
    // exists because the recording tap runs unconditionally, ahead of
    // dispatch, so the very keystroke that STOPS recording (evil's `q`)
    // has already been appended by the time this builtin runs — the
    // caller trims it back off explicitly rather than this builtin
    // guessing which trailing key(s) meant "stop".
    defun(interp, "end-kbd-macro", 1, Some(1), |i, a| {
        let trim = need_int(i, &a[0])?.max(0) as usize;
        let recording = ed_handle(i).borrow_mut().kbd_macro_recording.take();
        if let Some((reg, mut keys)) = recording {
            let kept = keys.len().saturating_sub(trim);
            keys.truncate(kept);
            ed_handle(i).borrow_mut().kbd_macros.insert(reg, keys);
        }
        Ok(Value::Nil)
    });
    // (execute-kbd-macro REG &optional COUNT) — replay register REG's
    // recorded macro COUNT times (default 1) by feeding its keys back
    // through `handle_key`, one at a time, exactly as if the user had
    // retyped them (this is what lets a replayed `:` minibuffer command
    // work unmodified, and what makes nested `@x` replay -- including
    // `@@` self-recursion -- fall naturally out of the SAME mechanism).
    // A no-op (no error) when REG has no recorded macro. Depth-capped
    // at 32 with a drop-guard restore, so a runaway self-recursive
    // macro aborts cleanly instead of overflowing the stack, and an
    // elisp error partway through a replay (a capture callback
    // signaling, a command erroring) can't leave `macro_replay_depth`
    // stuck above 0. Never holds an `Editor` borrow across the
    // `handle_key` calls below (mirrors `execute_command`'s own
    // borrow-scope discipline) — `handle_key` reborrows internally on
    // every call, and a still-live borrow here would panic the first
    // time it tried to.
    defun(interp, "execute-kbd-macro", 1, Some(2), |i, a| {
        let reg = need_int(i, &a[0])?;
        let count = match opt(a, 1) {
            Value::Nil => 1,
            v => need_int(i, &v)?.max(0),
        };
        let ed = ed_handle(i);
        let keys: Option<Vec<Key>> = {
            let editor = ed.borrow();
            editor.kbd_macros.get(&reg).cloned()
        };
        let Some(keys) = keys else {
            return Ok(Value::Nil);
        };
        let depth = ed.borrow().macro_replay_depth;
        if depth >= 32 {
            ed.borrow_mut().echo("Macro recursion too deep");
            return Ok(Value::Nil);
        }
        ed.borrow_mut().macro_replay_depth = depth + 1;
        struct DepthGuard(Rc<RefCell<crate::editor::Editor>>);
        impl Drop for DepthGuard {
            fn drop(&mut self) {
                let mut editor = self.0.borrow_mut();
                editor.macro_replay_depth = editor.macro_replay_depth.saturating_sub(1);
            }
        }
        let _guard = DepthGuard(ed.clone());
        for _ in 0..count {
            for k in &keys {
                crate::commands::handle_key(i, &ed, k.clone());
            }
        }
        Ok(Value::Nil)
    });
    // (defining-kbd-macro-p) — non-nil while a recording is armed.
    defun(interp, "defining-kbd-macro-p", 0, Some(0), |i, _| {
        Ok(Value::bool(
            ed_handle(i).borrow().kbd_macro_recording.is_some(),
            i.syms.t,
        ))
    });
    // (kbd-macro-p REG) — non-nil iff REG has a recorded macro.
    defun(interp, "kbd-macro-p", 1, Some(1), |i, a| {
        let reg = need_int(i, &a[0])?;
        Ok(Value::bool(
            ed_handle(i).borrow().kbd_macros.contains_key(&reg),
            i.syms.t,
        ))
    });

    // Faces.
    defun(interp, "set-face", 1, None, |i, a| {
        let name = need_sym(i, &a[0])?;
        let style = crate::redisplay::parse_face_plist(i, &a[1..]);
        let ed = ed_handle(i);
        ed.borrow_mut().faces.insert(name, style);
        Ok(Value::Sym(name))
    });

    // (show-hover-popup TEXT) — M16: floating popup at the cursor in
    // the GUI, echo-area text in the TUI. Cleared on the next key.
    defun(interp, "show-hover-popup", 1, Some(1), |i, a| {
        let text = elisp::builtins::need_str(i, &a[0])?.to_string();
        let ed = ed_handle(i);
        {
            let mut editor = ed.borrow_mut();
            editor.hover_popup = Some(text.clone());
            // TUI fallback: first line in the echo area.
            let first = text.lines().next().unwrap_or("").to_string();
            editor.echo = Some(first);
        }
        Ok(Value::Nil)
    });

    // (show-completion-popup ITEMS PREFIX-START &optional INCOMPLETE) —
    // M40-4, extended M44-3, PREFIX-START added by the M44 review fix
    // #1: cursor-anchored LSP completion candidate popup, drawn on the
    // shared character grid so TUI and GUI render it identically (unlike
    // `hover_popup`'s free text, which each frontend presents its own
    // way). ITEMS is a list of (LABEL INSERT START FILTER) proper lists,
    // server order: LABEL/INSERT/FILTER are strings, START is a buffer
    // position (elisp, so 1-based like every other position argument —
    // see `get_pos`) THIS candidate's own replacement start, used ONLY
    // by `accept_completion`'s deletion range — a `textEdit`-bearing
    // candidate can start earlier than another in the same list, so this
    // is per-item, not a single popup-wide value (see `PopupItem`).
    // PREFIX-START (also a buffer position) is the identifier-run prefix
    // start `commands::refilter_completion_popup` filters EVERY item
    // against as one shared span (`CompletionPopup::prefix_start`, see
    // its own doc comment for why this must be popup-wide rather than
    // each item's own START: `filterText` usually excludes whatever a
    // `textEdit` replaces ahead of the ordinary prefix, e.g. postfix
    // completions replacing "obj." while filtering on just "if").
    // INCOMPLETE mirrors the server's `CompletionList.isIncomplete` —
    // `commands::refilter_completion_popup` re-requests instead of
    // narrowing this list further once the buffer changes under an
    // incomplete popup.
    //
    // Silently a no-op when buffer-local `inhibit-self-insert' (M34) is
    // non-nil or ITEMS parses to nothing (an empty list, or every
    // element failing to parse as a well-shaped 4-element list): the
    // former means this buffer has already left insert state (evil
    // normal/visual/operator-pending) by the time this runs — most
    // likely a stale async LSP completion callback racing an ESC — so
    // nothing would ever accept the popup this would open; opening it
    // anyway would just be a dead widget sitting over normal-state
    // text. `lsp-completion-at-point`'s own identifier-run staleness
    // check in lsp.el catches a related race one layer up, but this
    // guard is what closes it for good — any OTHER caller of this
    // builtin gets the same protection for free.
    defun(interp, "show-completion-popup", 2, Some(3), |i, a| {
        let ed = ed_handle(i);
        // M40-4 review fix (#3): an async LSP completion reply can land
        // after the user has already moved on to some other modal
        // session -- M-x's minibuffer, C-s's isearch, an evil `f`/`t`
        // capture -- while the request was in flight. Buffer/tick/point
        // staleness alone can't catch that, since none of them
        // necessarily changed (M-x in particular never touches the
        // buffer). Opening the popup on top of an active session would
        // let `completion_popup_key` (commands.rs) intercept that
        // session's own keys ahead of its own dispatch (see its doc
        // comment -- most concretely, `in_minibuffer` is checked AFTER
        // `completion_popup_key` in `handle_key`, so the minibuffer has
        // no protection of its own). Silently drop the reply instead:
        // this is a callback, not a user action, so the user's current
        // activity wins.
        {
            let editor = ed.borrow();
            // M53: `isearch_live()`, not the raw `isearch.is_some()`,
            // so a session already made stale by a `switch-to-buffer`/
            // `kill-buffer` that hasn't yet been cleared by the next
            // keystroke (`handle_key`'s lazy invalidation) doesn't get
            // mistaken for a real modal session and silently swallow
            // this reply.
            if editor.minibuffer.is_some() || editor.isearch_live() || editor.key_capture.is_some()
            {
                return Ok(Value::Nil);
            }
        }
        let buf = ed.borrow().current.clone();
        let inhibit = match i.intern_soft("inhibit-self-insert") {
            Some(sym) => {
                let editor = ed.borrow();
                crate::editor::buffer_local_value(i, &editor, sym, &buf).unwrap_or(Value::Nil)
            }
            None => Value::Nil,
        }
        .truthy();
        if inhibit {
            return Ok(Value::Nil);
        }
        let prefix_start = {
            let bb = buf.borrow();
            get_pos(i, &bb, &a[1])?
        };
        let mut items = Vec::new();
        for item in a[0].list_to_vec().unwrap_or_default() {
            let Some(fields) = item.list_to_vec() else {
                continue;
            };
            if fields.len() != 4 {
                continue;
            }
            if let (Value::Str(label), Value::Str(insert), Value::Str(filter)) =
                (&fields[0], &fields[1], &fields[3])
            {
                let start = {
                    let bb = buf.borrow();
                    get_pos(i, &bb, &fields[2])?
                };
                items.push(crate::editor::PopupItem {
                    label: label.to_string(),
                    insert: insert.to_string(),
                    start,
                    filter: filter.to_string(),
                });
            }
        }
        if items.is_empty() {
            return Ok(Value::Nil);
        }
        let incomplete = opt(a, 2).truthy();
        let tick = buf.borrow().edit_ticks;
        ed.borrow_mut().completion_popup = Some(crate::editor::CompletionPopup {
            items,
            prefix_start,
            selected: 0,
            incomplete,
            tick,
        });
        Ok(Value::Nil)
    });
    // (hide-completion-popup) — M40-4: dismiss the popup without
    // accepting a candidate. The key-driven paths (ESC, C-g, any other
    // key) clear `Editor::completion_popup` directly from commands.rs;
    // this is the elisp-callable equivalent for callers that want to
    // close it programmatically.
    defun(interp, "hide-completion-popup", 0, Some(0), |i, _| {
        ed_handle(i).borrow_mut().completion_popup = None;
        Ok(Value::Nil)
    });
    // (completion-popup-active-p) — M40-4: for tests/elisp to query
    // whether the popup is currently open.
    defun(interp, "completion-popup-active-p", 0, Some(0), |i, _| {
        Ok(Value::bool(
            ed_handle(i).borrow().completion_popup.is_some(),
            i.syms.t,
        ))
    });

    // Markers.
    defun(interp, "make-marker", 0, Some(0), |i, _| {
        let b = cur(i);
        let m = Rc::new(RefCell::new(MarkerData {
            buffer: Rc::downgrade(&b),
            pos: 0,
        }));
        Ok(Value::Ext(ExtRef {
            tag: MARKER_TAG,
            obj: m,
            // MarkerData: buffer position + a Weak<RefCell<Buffer>>, no Value.
            trace: None,
        }))
    });
    defun(interp, "copy-marker", 0, Some(2), |i, a| {
        let b = cur(i);
        let pos = {
            let bb = b.borrow();
            get_pos(i, &bb, &opt(a, 0))?
        };
        let m = Rc::new(RefCell::new(MarkerData {
            buffer: Rc::downgrade(&b),
            pos,
        }));
        b.borrow_mut().markers.push(Rc::downgrade(&m));
        Ok(Value::Ext(ExtRef {
            tag: MARKER_TAG,
            obj: m,
            // MarkerData: buffer position + a Weak<RefCell<Buffer>>, no Value.
            trace: None,
        }))
    });
    defun(interp, "point-marker", 0, Some(0), |i, _| {
        let b = cur(i);
        let pos = b.borrow().point;
        let m = Rc::new(RefCell::new(MarkerData {
            buffer: Rc::downgrade(&b),
            pos,
        }));
        b.borrow_mut().markers.push(Rc::downgrade(&m));
        Ok(Value::Ext(ExtRef {
            tag: MARKER_TAG,
            obj: m,
            // MarkerData: buffer position + a Weak<RefCell<Buffer>>, no Value.
            trace: None,
        }))
    });
    defun(interp, "marker-position", 1, Some(1), |i, a| {
        let m = a[0]
            .as_ext::<RefCell<MarkerData>>(MARKER_TAG)
            .ok_or_else(|| i.wrong_type("markerp", &a[0]))?;
        let pos = m.borrow().pos;
        Ok(int_pos(pos))
    });
    defun(interp, "set-marker", 2, Some(3), |i, a| {
        let m = a[0]
            .as_ext::<RefCell<MarkerData>>(MARKER_TAG)
            .ok_or_else(|| i.wrong_type("markerp", &a[0]))?;
        let b = cur(i);
        let pos = {
            let bb = b.borrow();
            get_pos(i, &bb, &a[1])?
        };
        m.borrow_mut().pos = pos;
        m.borrow_mut().buffer = Rc::downgrade(&b);
        let already = b
            .borrow()
            .markers
            .iter()
            .any(|w| w.upgrade().map(|x| Rc::ptr_eq(&x, &m)).unwrap_or(false));
        if !already {
            b.borrow_mut().markers.push(Rc::downgrade(&m));
        }
        Ok(a[0].clone())
    });
    defun(interp, "markerp", 1, Some(1), |i, a| {
        let is = a[0].as_ext::<RefCell<MarkerData>>(MARKER_TAG).is_some();
        Ok(Value::bool(is, i.syms.t))
    });

    // Overlays.
    // Text properties (M11 item 5, buffer side). Backed by the same
    // interval store as overlays — at this editor's scale the two
    // mechanisms are one; the API compatibility is the point (elisp
    // written against put-text-property/get-text-property just works).
    // Deliberately NOT covered (documented): string-attached properties
    // (propertize) — those need a string-representation change — and
    // property copying through kill/yank.
    defun(interp, "put-text-property", 4, Some(5), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let prop = elisp::builtins::need_sym(i, &a[2])?;
        let mut bb = b.borrow_mut();
        let seq = bb.alloc_overlay_seq();
        let ov = Rc::new(RefCell::new(OverlayData {
            buffer: Rc::downgrade(&b),
            start: s,
            end: e,
            props: vec![(prop, a[3].clone())],
            seq,
        }));
        bb.insert_overlay(ov);
        Ok(Value::Nil)
    });
    defun(interp, "get-text-property", 2, Some(3), |i, a| {
        let b = cur(i);
        let pos = {
            let bb = b.borrow();
            get_pos(i, &bb, &a[0])?
        };
        let prop = elisp::builtins::need_sym(i, &a[1])?;
        // Later-added intervals win, mirroring overlay stacking. `seq`
        // (not iteration order) is what tracks creation order now that
        // `overlays` is kept sorted by `start` (P1.3), so this compares
        // `seq` directly instead of scanning in reverse.
        let bb = b.borrow();
        let mut best: Option<(u64, Value)> = None;
        for ov in bb.overlays.iter() {
            let o = ov.borrow();
            if pos >= o.start && pos < o.end {
                if let Some((_, v)) = o.props.iter().find(|(p, _)| *p == prop) {
                    if best.as_ref().map(|(s, _)| o.seq > *s).unwrap_or(true) {
                        best = Some((o.seq, v.clone()));
                    }
                }
            }
        }
        Ok(best.map(|(_, v)| v).unwrap_or(Value::Nil))
    });
    defun(interp, "remove-text-properties", 3, Some(4), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        // PROPS is a plist whose keys name the properties to remove
        // (values are ignored, per Emacs).
        let mut to_remove: Vec<elisp::value::SymId> = Vec::new();
        let mut cur_p = a[2].clone();
        while let Value::Cons(c) = &cur_p {
            let (head, rest) = {
                let cb = c.borrow();
                (cb.car.clone(), cb.cdr.cdr())
            };
            if let Value::Sym(id) = head {
                to_remove.push(id);
            }
            cur_p = rest;
        }
        let mut bb = b.borrow_mut();
        for ov in &bb.overlays {
            let mut o = ov.borrow_mut();
            // Only intervals fully inside the range lose their props
            // (partial-overlap splitting is out of scope at our scale).
            if o.start >= s && o.end <= e {
                o.props.retain(|(p, _)| !to_remove.contains(p));
            }
        }
        bb.retain_overlays(|ov| !ov.borrow().props.is_empty());
        Ok(Value::Nil)
    });

    defun(interp, "make-overlay", 2, Some(3), |i, a| {
        let b = buffer_arg(i, &opt(a, 2))?;
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let mut bb = b.borrow_mut();
        let seq = bb.alloc_overlay_seq();
        let ov = Rc::new(RefCell::new(OverlayData {
            buffer: Rc::downgrade(&b),
            start: s,
            end: e,
            props: Vec::new(),
            seq,
        }));
        bb.insert_overlay(ov.clone());
        Ok(Value::Ext(ExtRef {
            tag: OVERLAY_TAG,
            obj: ov,
            trace: Some(trace_overlay),
        }))
    });
    defun(interp, "overlay-put", 3, Some(3), |i, a| {
        let ov = a[0]
            .as_ext::<RefCell<OverlayData>>(OVERLAY_TAG)
            .ok_or_else(|| i.wrong_type("overlayp", &a[0]))?;
        let prop = need_sym(i, &a[1])?;
        ov.borrow_mut().put(prop, a[2].clone());
        Ok(a[2].clone())
    });
    defun(interp, "overlay-get", 2, Some(2), |i, a| {
        let ov = a[0]
            .as_ext::<RefCell<OverlayData>>(OVERLAY_TAG)
            .ok_or_else(|| i.wrong_type("overlayp", &a[0]))?;
        let prop = need_sym(i, &a[1])?;
        let v = ov.borrow().get(prop);
        Ok(v)
    });
    defun(interp, "overlay-start", 1, Some(1), |i, a| {
        let ov = a[0]
            .as_ext::<RefCell<OverlayData>>(OVERLAY_TAG)
            .ok_or_else(|| i.wrong_type("overlayp", &a[0]))?;
        let s = ov.borrow().start;
        Ok(int_pos(s))
    });
    defun(interp, "overlay-end", 1, Some(1), |i, a| {
        let ov = a[0]
            .as_ext::<RefCell<OverlayData>>(OVERLAY_TAG)
            .ok_or_else(|| i.wrong_type("overlayp", &a[0]))?;
        let e = ov.borrow().end;
        Ok(int_pos(e))
    });
    defun(interp, "delete-overlay", 1, Some(1), |i, a| {
        let ov = a[0]
            .as_ext::<RefCell<OverlayData>>(OVERLAY_TAG)
            .ok_or_else(|| i.wrong_type("overlayp", &a[0]))?;
        if let Some(b) = ov.borrow().buffer.upgrade() {
            b.borrow_mut().delete_overlay(&ov);
        }
        Ok(Value::Nil)
    });
    defun(interp, "overlays-in", 2, Some(2), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let list: Vec<Value> = b
            .borrow()
            .overlays_in(s, e)
            .into_iter()
            .map(|ov| {
                Value::Ext(ExtRef {
                    tag: OVERLAY_TAG,
                    obj: ov,
                    trace: Some(trace_overlay),
                })
            })
            .collect();
        Ok(Value::list(list))
    });
    defun(interp, "remove-overlays", 0, Some(4), |i, _| {
        cur(i).borrow_mut().clear_overlays();
        Ok(Value::Nil)
    });

    // Command execution.
    defun(interp, "command-execute", 1, Some(1), |i, a| {
        let ed = ed_handle(i);
        let cmd = a[0].clone();
        crate::commands::execute_command(i, &ed, cmd);
        Ok(Value::Nil)
    });
    defun(interp, "this-command-keys", 0, Some(0), |i, _| {
        let ed = ed_handle(i);
        let desc = crate::keymap::key_sequence_description(&ed.borrow().pending_keys);
        Ok(Value::string(desc))
    });
    defun(interp, "major-mode-internal-set", 1, Some(1), |i, a| {
        cur(i).borrow_mut().major_mode = a[0].clone();
        Ok(a[0].clone())
    });
    defun(interp, "major-mode-internal-get", 0, Some(0), |i, _| {
        Ok(cur(i).borrow().major_mode.clone())
    });
    // Windows.
    defun(interp, "split-window-internal", 1, Some(1), |i, a| {
        let horizontal = a[0].truthy();
        let ed = ed_handle(i);
        crate::editor::split_selected(&ed, horizontal);
        Ok(Value::Nil)
    });
    defun(interp, "other-window", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => elisp::builtins::need_int(i, &v)?,
        };
        let ed = ed_handle(i);
        let ids: Vec<usize> = {
            let mut ids: Vec<usize> = ed.borrow().windows.keys().copied().collect();
            ids.sort_unstable();
            ids
        };
        if ids.len() <= 1 {
            return Ok(Value::Nil);
        }
        let cur = ed.borrow().selected_window;
        let idx = ids.iter().position(|id| *id == cur).unwrap_or(0) as i64;
        let len = ids.len() as i64;
        let next = ((idx + n) % len + len) % len;
        crate::editor::select_window(i, &ed, ids[next as usize]);
        Ok(Value::Nil)
    });
    defun(interp, "delete-other-windows", 0, Some(0), |i, _| {
        let ed = ed_handle(i);
        let (sel, buf) = {
            let editor = ed.borrow();
            (
                editor.selected_window,
                editor.windows[&editor.selected_window].buffer.clone(),
            )
        };
        {
            let mut editor = ed.borrow_mut();
            editor.windows.retain(|id, _| *id == sel);
            editor.layout = crate::editor::Layout::Leaf(sel);
        }
        let _ = buf;
        Ok(Value::Nil)
    });
    defun(interp, "delete-window", 0, Some(0), |i, _| {
        let ed = ed_handle(i);
        let sel = ed.borrow().selected_window;
        if ed.borrow().windows.len() <= 1 {
            return Err(i.error("Attempt to delete the sole ordinary window"));
        }
        {
            let mut editor = ed.borrow_mut();
            editor.layout.remove_leaf(sel);
            editor.windows.remove(&sel);
        }
        let first = {
            let mut ids: Vec<usize> = ed.borrow().windows.keys().copied().collect();
            ids.sort_unstable();
            ids[0]
        };
        crate::editor::select_window(i, &ed, first);
        Ok(Value::Nil)
    });
    defun(interp, "window-count", 0, Some(0), |i, _| {
        Ok(Value::Int(ed_handle(i).borrow().windows.len() as i64))
    });
    defun(interp, "selected-window", 0, Some(0), |i, _| {
        Ok(Value::Int(ed_handle(i).borrow().selected_window as i64))
    });
    // (select-window-in-direction DIR) — M45, evil's `C-w h/j/k/l`: DIR is
    // one of the symbols `left'/`right'/`up'/`down'. Reads the SAME
    // geometry `render' paints from (`redisplay::window_rects', the M45
    // single-source-of-truth extraction — never a second copy of the
    // split math), picks the best candidate window in that direction (see
    // `window_in_direction`'s own doc comment for the tie-break rules),
    // and switches to it through `select_window' -- the one legal window-
    // switch primitive (point save/restore + `set_current_buffer'), same
    // as `other-window'/`delete-window' above. Returns the new selected
    // window's id (truthy) on success; nil with NO side effect if there
    // is no window in that direction (evil.el turns that into a
    // "No window in that direction" message).
    defun(interp, "select-window-in-direction", 1, Some(1), |i, a| {
        let dir_id = need_sym(i, &a[0])?;
        let dir = i.sym_name(dir_id).to_string();
        let ed = ed_handle(i);
        let found = {
            let editor = ed.borrow();
            let rects = crate::redisplay::window_rects(&editor);
            let sel = editor.selected_window;
            rects
                .iter()
                .find(|(id, _)| *id == sel)
                .and_then(|(_, sel_rect)| window_in_direction(&rects, sel, *sel_rect, &dir))
        };
        match found {
            Some(id) => {
                crate::editor::select_window(i, &ed, id);
                Ok(Value::Int(id as i64))
            }
            None => Ok(Value::Nil),
        }
    });

    // Incremental search: the modal key handling lives in commands.rs
    // (isearch_key) since it needs to intercept keys before normal
    // dispatch; these builtins let elisp start/query it too.
    defun(interp, "isearch-start", 1, Some(1), |i, a| {
        let ed = ed_handle(i);
        crate::editor::isearch_start(&ed, a[0].truthy());
        Ok(Value::Nil)
    });
    defun(interp, "isearch-active-p", 0, Some(0), |i, _| {
        // M53: `isearch_live()`, not the raw `isearch.is_some()` -- see
        // its doc comment. Otherwise this would report `t` for a session
        // already made stale by a buffer swap that hasn't yet reached
        // the next keystroke.
        Ok(Value::bool(ed_handle(i).borrow().isearch_live(), i.syms.t))
    });
    // M30: minimal readers for the last COMMITTED isearch (see
    // `Editor::last_search`'s doc) -- evil's `n`/`N` bridge onto plain
    // `search-forward`/`search-backward` (already elisp-callable) using
    // these two to learn what/which-direction to repeat, since neither
    // the query string nor its direction had any elisp accessor before.
    defun(
        interp,
        "isearch-last-string",
        0,
        Some(0),
        |i, _| match &ed_handle(i).borrow().last_search {
            Some((s, _)) => Ok(Value::string(s.clone())),
            None => Ok(Value::Nil),
        },
    );
    defun(interp, "isearch-last-forward-p", 0, Some(0), |i, _| {
        let forward = ed_handle(i)
            .borrow()
            .last_search
            .as_ref()
            .map(|(_, f)| *f)
            .unwrap_or(true);
        Ok(Value::bool(forward, i.syms.t))
    });
    // (isearch-set-last STRING FORWARD) — M42-II: the write side of
    // `Editor::last_search`, added for evil's `:s' substitute command
    // (evil.el) to hand `n'/`N' a pattern to repeat afterward — until
    // now the only writer was `isearch_exit' (editor.rs), reachable
    // solely by actually running an interactive isearch session.
    defun(interp, "isearch-set-last", 2, Some(2), |i, a| {
        let s = need_str(i, &a[0])?.to_string();
        let forward = a[1].truthy();
        ed_handle(i).borrow_mut().last_search = Some((s, forward));
        Ok(Value::Nil)
    });

    defun(interp, "frame-width", 0, Some(1), |i, _| {
        Ok(Value::Int(ed_handle(i).borrow().frame.0 as i64))
    });
    defun(interp, "frame-height", 0, Some(1), |i, _| {
        Ok(Value::Int(ed_handle(i).borrow().frame.1 as i64))
    });
    defun(interp, "string-width", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        // See `redisplay::display_width::string_width_elisp` for why
        // this deliberately does NOT expand tabs the way the buffer
        // grid does.
        let w = crate::redisplay::display_width::string_width_elisp(&s);
        Ok(Value::Int(w as i64))
    });
    defun(interp, "format-time-string", 1, Some(2), |i, a| {
        let fmt = need_str(i, &a[0])?;
        let out = std::process::Command::new("/bin/date")
            .arg(format!("+{}", fmt))
            .output()
            .map_err(|e| i.error(format!("cannot run date: {}", e)))?;
        Ok(Value::string(
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
        ))
    });
    defun(interp, "browse-url", 1, Some(1), |i, a| {
        let url = need_str(i, &a[0])?;
        // M60: all three stdio explicitly Stdio::null() -- `open`'s own
        // stdout/stderr used to be inherited (Command's default), which
        // writes straight past the TUI's diff-based redisplay and can
        // never be overwritten (see `elisp::bglog`'s module doc). Note
        // this does NOT fix the pre-existing gap that the spawned child
        // is never reaped (no `.wait()`/watcher) -- that's out of scope
        // for M60, left as a documented, not hidden, debt.
        std::process::Command::new("open")
            .arg(url.as_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| i.error(format!("cannot open url: {}", e)))?;
        Ok(Value::Nil)
    });

    // M47: minibuffer reading, callback-style (CPS) rather than blocking
    // GNU Emacs's `read-from-minibuffer`/`completing-read` — see this
    // module's own doc comment on `read_from_minibuffer_impl` for why:
    // `crates/core` reads no keyboard events of its own (crossterm/
    // eframe both live one layer up, in the frontends) and the GUI's
    // event loop is a `winit` callback that cannot be paused mid-call to
    // pump for more keys, so a truly blocking read is not implementable
    // here at all. CALLBACK receives the result once the user submits;
    // `crates/core/lisp/simple.el`'s `with-read-string`/
    // `with-completing-read`/`with-y-or-n` macros hide the CPS shape for
    // the common case of "read one thing, then keep going".
    defun(
        interp,
        "read-from-minibuffer",
        2,
        Some(4),
        read_from_minibuffer_impl,
    );
    // (read-string PROMPT CALLBACK &optional INITIAL HISTORY-KEY) — a
    // plain synonym for `read-from-minibuffer` (same signature, same
    // CPS deviation from GNU Emacs noted there); kept as a separate name
    // only because elisp callers reach for whichever spelling GNU Emacs
    // code would have used.
    defun(interp, "read-string", 2, Some(4), read_from_minibuffer_impl);
    // (completing-read PROMPT COLLECTION CALLBACK &optional
    //  REQUIRE-MATCH INITIAL HISTORY-KEY) — COLLECTION is a list of
    // strings or symbols (interned symbols are read by name); anything
    // else in the list signals `wrong-type-argument`. See
    // `read_from_minibuffer_impl`'s doc comment for the CALLBACK
    // deviation from GNU Emacs's blocking signature.
    defun(interp, "completing-read", 3, Some(6), completing_read_impl);
    // (orderless-rank CANDIDATE INPUT) (M85) — the SAME orderless
    // matcher `completing-read`/`M-x`/`C-x b` already use internally
    // (`crate::complete::orderless_rank`, M84), exposed to elisp so
    // callers that maintain their own candidate list outside the
    // minibuffer (e.g. `*search*`'s live result filter, search.el) get
    // identical matching semantics instead of a hand-rolled second
    // implementation that would silently drift from this one over
    // time. Faithfully passes the Rust return value through rather
    // than collapsing it to a boolean — nil (no match), 0 (the FIRST
    // whitespace-separated token of INPUT is a prefix of CANDIDATE), or
    // 1 (matches, but not by that rule) — because a caller sorting
    // multiple matches needs the rank, not just match/no-match.
    // INPUT's tokens are matched in ANY order, all must appear
    // SOMEWHERE in CANDIDATE (`alu clk` matches "clk_alu_top"); an
    // empty INPUT, or one containing only whitespace, has zero tokens
    // and so matches every CANDIDATE at rank 0. Each token is matched
    // "smart-case" (`rg --smart-case`'s own convention, PER TOKEN, not
    // per whole INPUT): a token containing an uppercase letter is
    // matched case-sensitively, an all-lowercase token case-
    // insensitively. See `orderless_rank`'s own doc comment
    // (complete.rs) for the full semantics this wraps unchanged.
    defun(interp, "orderless-rank", 2, Some(2), |i, a| {
        let cand = need_str(i, &a[0])?;
        let input = need_str(i, &a[1])?;
        Ok(match crate::complete::orderless_rank(&cand, &input) {
            Some(rank) => Value::Int(rank as i64),
            None => Value::Nil,
        })
    });
    // (minibuffer-prompt) (M85 fix round F3) — the CURRENT minibuffer's
    // own prompt string (`Editor::minibuffer`'s `prompt` field,
    // verbatim, exactly as passed to `read-string`/`completing-read`/
    // `read-from-minibuffer`), or nil when no minibuffer is open right
    // now. Exists because opening a minibuffer does NOT change the
    // CURRENT buffer at all (there is no separate "*Minibuffer*" buffer
    // in this editor's model — see the minibuffer-open code paths in
    // this file, none of which ever call `set_current_buffer`), so
    // `(buffer-name)` alone can never tell an elisp caller whether the
    // minibuffer presently open on top of its buffer is the SAME one it
    // itself opened earlier, or some entirely unrelated later prompt
    // (`M-x`, `find-file`, ...) that merely happens to be running while
    // that same buffer is still current underneath it. A caller that
    // needs to tell those apart (e.g. `*search*`'s live filter,
    // search.el's `search--filter-input-changed') compares this
    // against its own known prompt string instead of trusting
    // `(buffer-name)` alone. A general-purpose primitive, not a
    // search-specific one — any future minibuffer-aware elisp code
    // gets the same disambiguation for free.
    defun(interp, "minibuffer-prompt", 0, Some(0), |i, _a| {
        let ed = ed_handle(i);
        let prompt = ed.borrow().minibuffer.as_ref().map(|mb| mb.prompt.clone());
        Ok(match prompt {
            Some(p) => Value::string(p),
            None => Value::Nil,
        })
    });
}

/// Shared entry point for `read-from-minibuffer`/`read-string` (M47):
/// opens a plain (non-completing) minibuffer prompt whose submitted
/// string is handed to CALLBACK. See the `register`-site doc comment
/// for why this is callback-style (CPS) rather than the blocking read
/// GNU Emacs offers -- `crates/core` has no way to pump keyboard events
/// itself, so the minibuffer's own continuation-passing machinery
/// (`PendingArgs`/`process_pending`, already used by every other
/// interactive-spec argument) is reused here directly instead of trying
/// to fake a blocking call on top of it.
///
/// Signals an error instead of opening a SECOND minibuffer when one is
/// already open: `process_pending` unconditionally overwrites
/// `Editor::minibuffer`, so calling straight through here would drop
/// the first prompt's `PendingArgs` (its callback) on the floor with no
/// way to ever run it. Calling from INSIDE a callback is fine and
/// intentional -- `submit_minibuffer` (commands.rs) already took the
/// previous `Minibuffer` out of `Editor` before invoking the callback,
/// so by the time this runs there is nothing left to clobber. This is
/// the supported way to chain/nest reads.
fn read_from_minibuffer_impl(i: &mut Interp, a: &mut [Value]) -> Result<Value, Flow> {
    let ed = ed_handle(i);
    if ed.borrow().minibuffer.is_some() {
        return Err(i.error("Command attempted to use minibuffer while in minibuffer"));
    }
    let prompt = need_str(i, &a[0])?.to_string();
    let callback = a[1].clone();
    let initial = match opt(a, 2) {
        Value::Nil => None,
        v => Some(need_str(i, &v)?.to_string()),
    };
    let history_key = history_key_arg(i, &opt(a, 3))?;
    let pending = PendingArgs {
        command: callback,
        specs: vec![ArgSpec {
            code: 's',
            prompt,
            collection: None,
            require_match: false,
            initial,
            history_key,
        }],
        collected: Vec::new(),
        index: 0,
        // M50 follow-up: this runs from INSIDE `read-string`'s own command
        // body, which `call_command` already wrapped -- `finish_command`
        // already fired once for that surrounding command's cycle by the
        // time we get here. See `PendingArgs::opener_cycle_closed`'s doc
        // comment.
        opener_cycle_closed: true,
    };
    crate::commands::process_pending(i, &ed, pending);
    Ok(Value::Nil)
}

/// `completing-read` (M47): like `read_from_minibuffer_impl`, but the
/// spec's `collection` field is set from COLLECTION, which both opens
/// the M21 selector panel (`process_pending`'s panel-opening condition
/// already covers `Source::Custom`) and, with REQUIRE-MATCH non-nil,
/// blocks submission of anything not in COLLECTION
/// (`commands::require_match_blocks`).
fn completing_read_impl(i: &mut Interp, a: &mut [Value]) -> Result<Value, Flow> {
    let ed = ed_handle(i);
    if ed.borrow().minibuffer.is_some() {
        return Err(i.error("Command attempted to use minibuffer while in minibuffer"));
    }
    let prompt = need_str(i, &a[0])?.to_string();
    let collection = collection_arg(i, &a[1])?;
    let callback = a[2].clone();
    let require_match = opt(a, 3).truthy();
    let initial = match opt(a, 4) {
        Value::Nil => None,
        v => Some(need_str(i, &v)?.to_string()),
    };
    let history_key = history_key_arg(i, &opt(a, 5))?;
    let pending = PendingArgs {
        command: callback,
        specs: vec![ArgSpec {
            code: 's',
            prompt,
            collection: Some(Rc::new(collection)),
            require_match,
            initial,
            history_key,
        }],
        collected: Vec::new(),
        index: 0,
        // M50 follow-up: same reasoning as `read_from_minibuffer_impl` above
        // -- this also runs from inside an already-`call_command`-wrapped
        // command body.
        opener_cycle_closed: true,
    };
    crate::commands::process_pending(i, &ed, pending);
    Ok(Value::Nil)
}

/// COLLECTION list -> Vec<String> for `completing-read`: elements are
/// either strings (used verbatim) or symbols (read by their print name,
/// so callers can pass an alist's keys straight through without mapping
/// `symbol-name` over them first). Anything else signals
/// `wrong-type-argument`.
fn collection_arg(i: &mut Interp, v: &Value) -> Result<Vec<String>, Flow> {
    let items = need_list(i, v)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::Str(s) => out.push(s.to_string()),
            Value::Sym(id) => out.push(i.sym_name(id).to_string()),
            other => return Err(i.wrong_type("stringp", &other)),
        }
    }
    Ok(out)
}

/// HISTORY-KEY argument shared by `read-from-minibuffer`/`read-string`/
/// `completing-read` (M47): a string or symbol names the history ring
/// (`ArgSpec::history_key`, Part D); nil defaults to `"minibuffer"`, GNU
/// Emacs's own default history variable name.
fn history_key_arg(i: &mut Interp, v: &Value) -> Result<String, Flow> {
    match v {
        Value::Nil => Ok("minibuffer".to_string()),
        Value::Str(s) => Ok(s.to_string()),
        Value::Sym(id) => Ok(i.sym_name(*id).to_string()),
        other => Err(i.wrong_type("stringp", other)),
    }
}

/// The length of the overlap between two half-open ranges `[a0, a1)` and
/// `[b0, b1)`, or 0 when they don't overlap at all -- `select-window-in-
/// direction`'s tie-break metric (M45): among every candidate window that
/// sits in the requested direction, prefer whichever one's cross-axis
/// span overlaps the selected window's own span the most (so, e.g., a
/// `C-w j` from the LEFT pane of a `C-x 3` split lands in whichever pane
/// below spans the most of that same horizontal range, not just whichever
/// happens to sort first).
fn range_overlap(a0: usize, a1: usize, b0: usize, b1: usize) -> usize {
    let lo = a0.max(b0);
    let hi = a1.min(b1);
    hi.saturating_sub(lo)
}

/// Best candidate window id in direction DIR (`"left"`/`"right"`/
/// `"up"`/`"down"`) from SEL_RECT (the currently selected window's own
/// rect, at id SEL) among RECTS (`redisplay::window_rects`' output). A
/// window counts as a candidate when it sits ENTIRELY past the selected
/// window's edge in that direction (`left`: candidate's right edge at or
/// left of the selected window's left edge; `right`/`up`/`down`
/// symmetric) -- never a window that merely straddles the boundary.
/// Ranked by, in order: (1) `range_overlap` on the cross axis (row range
/// for left/right, column range for up/down) -- highest wins; (2) on a
/// tie, the candidate edge closest to the selected window's own edge in
/// the travel direction; (3) on a further tie, the smallest window id.
/// An unrecognized DIR (should never happen — evil.el only ever passes
/// one of the four symbols above) matches no candidates, same as a
/// direction with genuinely nothing there: `None`.
fn window_in_direction(
    rects: &[(usize, crate::redisplay::Rect)],
    sel: usize,
    sel_rect: crate::redisplay::Rect,
    dir: &str,
) -> Option<usize> {
    let vertical = dir == "up" || dir == "down";
    // (overlap, edge_closeness) both "higher is better"; edge_closeness
    // is the raw edge coordinate for `left'/`up' (larger = nearer the
    // selected window, since candidates lie strictly below it) and its
    // negation for `right'/`down' (smaller raw coordinate = nearer, so
    // negating keeps the same "higher is better" comparison uniform).
    let mut best: Option<(usize, usize, i64)> = None;
    for &(id, r) in rects {
        if id == sel {
            continue;
        }
        let is_candidate = match dir {
            "left" => r.col + r.width <= sel_rect.col,
            "right" => r.col >= sel_rect.col + sel_rect.width,
            "up" => r.row + r.height <= sel_rect.row,
            "down" => r.row >= sel_rect.row + sel_rect.height,
            _ => false,
        };
        if !is_candidate {
            continue;
        }
        let overlap = if vertical {
            range_overlap(
                r.col,
                r.col + r.width,
                sel_rect.col,
                sel_rect.col + sel_rect.width,
            )
        } else {
            range_overlap(
                r.row,
                r.row + r.height,
                sel_rect.row,
                sel_rect.row + sel_rect.height,
            )
        };
        let edge = match dir {
            "left" => (r.col + r.width) as i64,
            "right" => -(r.col as i64),
            "up" => (r.row + r.height) as i64,
            "down" => -(r.row as i64),
            _ => 0,
        };
        let better = match best {
            None => true,
            Some((best_id, best_overlap, best_edge)) => {
                (overlap, edge, std::cmp::Reverse(id))
                    > (best_overlap, best_edge, std::cmp::Reverse(best_id))
            }
        };
        if better {
            best = Some((id, overlap, edge));
        }
    }
    best.map(|(id, _, _)| id)
}
