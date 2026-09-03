use std::cell::RefCell;
use std::rc::Rc;

use elisp::builtins::{defun, need_str, need_sym, opt};
use elisp::eval::apply_function;
use elisp::{Interp, Value};

use super::{buffer_arg, cur, ed_handle};
use crate::buffer::Buffer;
use crate::editor::{make_buffer_local, set_current_buffer, Editor};

pub fn register(interp: &mut Interp) {
    defun(interp, "current-buffer", 0, Some(0), |i, _| {
        Ok(Editor::buffer_value(&cur(i)))
    });
    defun(interp, "bufferp", 1, Some(1), |i, a| {
        let is = a[0]
            .as_ext::<RefCell<Buffer>>(crate::editor::BUFFER_TAG)
            .is_some();
        Ok(Value::bool(is, i.syms.t))
    });
    defun(interp, "get-buffer", 1, Some(1), |i, a| {
        match buffer_arg(i, &a[0]) {
            Ok(b) => Ok(Editor::buffer_value(&b)),
            Err(_) => Ok(Value::Nil),
        }
    });
    defun(interp, "get-buffer-create", 1, Some(1), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        let ed = ed_handle(i);
        if let Some(b) = ed.borrow().find_buffer(&name) {
            return Ok(Editor::buffer_value(&b));
        }
        let b = Rc::new(RefCell::new(Buffer::new(&name, "")));
        ed.borrow_mut().buffers.push(b.clone());
        Ok(Editor::buffer_value(&b))
    });
    defun(interp, "generate-new-buffer", 1, Some(1), |i, a| {
        let base = need_str(i, &a[0])?.to_string();
        let ed = ed_handle(i);
        let name = unique_name(&ed, &base);
        let b = Rc::new(RefCell::new(Buffer::new(&name, "")));
        ed.borrow_mut().buffers.push(b.clone());
        Ok(Editor::buffer_value(&b))
    });
    defun(interp, "set-buffer", 1, Some(1), |i, a| {
        let b = buffer_arg(i, &a[0])?;
        let ed = ed_handle(i);
        set_current_buffer(i, &ed, b.clone());
        Ok(Editor::buffer_value(&b))
    });
    defun(interp, "switch-to-buffer-internal", 1, Some(1), |i, a| {
        // Creates the buffer if a name doesn't exist yet, like C-x b.
        let b = match buffer_arg(i, &a[0]) {
            Ok(b) => b,
            Err(_) => {
                let name = need_str(i, &a[0])?.to_string();
                let b = Rc::new(RefCell::new(Buffer::new(&name, "")));
                ed_handle(i).borrow_mut().buffers.push(b.clone());
                b
            }
        };
        let ed = ed_handle(i);
        crate::editor::show_buffer_in_selected_window(i, &ed, b.clone());
        Ok(Editor::buffer_value(&b))
    });
    defun(interp, "buffer-name", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let name = b.borrow().name.clone();
        Ok(Value::string(name))
    });
    defun(interp, "rename-buffer", 1, Some(2), |i, a| {
        let name = need_str(i, &a[0])?.to_string();
        cur(i).borrow_mut().name = name.clone();
        Ok(Value::string(name))
    });
    defun(interp, "buffer-file-name", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let file = b.borrow().file.clone();
        Ok(file.map(Value::string).unwrap_or(Value::Nil))
    });
    defun(interp, "get-file-buffer", 1, Some(1), |i, a| {
        let path = elisp::builtins::need_str(i, &a[0])?.to_string();
        // M61: normalize the argument the same way `.file` is normalized
        // (find-file-internal's expand_path), matching GNU's
        // `get-file-buffer` which also does `expand-file-name` on its
        // argument — otherwise a caller (e.g. lsp.el) passing a
        // differently-spelled but equivalent path would wrongly get nil.
        //
        // `/ssh:` paths are excluded from this: `expand_file_name`'s
        // `//`/`/~` shadowing (in `expand_file_input`) runs *before* its
        // own `/ssh:` prefix check, so it can mangle a remote path (e.g.
        // eating the host on a `//` inside it, or rewriting a literal
        // `~` in the remote path to this machine's local $HOME) — a
        // pre-existing quirk of `expand_file_input` itself, out of scope
        // to fix here. Per D4, the remote side of this comparison (the
        // visiting buffer's `.file`) is stored verbatim, un-normalized —
        // so normalizing only this side for `/ssh:` paths would just
        // reintroduce the same kind of mismatch M61 is fixing elsewhere.
        let path = if crate::remote::parse(&path).is_none() {
            crate::builtins::files::expand_file_name(&path, None)
        } else {
            path
        };
        let ed = ed_handle(i);
        let hit = ed
            .borrow()
            .buffers
            .iter()
            .find(|b| b.borrow().file.as_deref() == Some(path.as_str()))
            .cloned();
        Ok(hit
            .map(|b| crate::editor::Editor::buffer_value(&b))
            .unwrap_or(Value::Nil))
    });
    // (lsp--set-buffer-diagnostics BUFFER ((LINE . (SEVERITY . MESSAGE)) ...))
    // — M16, extended by M87 stage 3 to carry the message text: feeds the
    // gutter dots, modeline count, AND (as of stage 3) the inline
    // diagnostic block rows the renderer draws under the offending line;
    // the squiggle overlays are separate, made by lsp.el directly.
    //
    // Accepts the pre-stage-3 `(LINE . SEVERITY)` shape too (`cdr` is a
    // bare int, not a cons) -- several tests call this builtin directly
    // as a gutter-data injection shortcut, bypassing `lsp--decorate-
    // buffer` (the one real caller, which always sends the new shape).
    // Those old calls keep working, just with no message (no block rows
    // for them, which is exactly what they're testing anyway).
    defun(interp, "lsp--set-buffer-diagnostics", 2, Some(2), |i, a| {
        let b = buffer_arg(i, &a[0])?;
        let items = a[1].list_to_vec().unwrap_or_default();
        let mut lines = Vec::new();
        for item in items {
            if let Value::Cons(c) = &item {
                let (line, rest) = {
                    let cell = c.borrow();
                    (cell.car.clone(), cell.cdr.clone())
                };
                let (sev, message) = match &rest {
                    Value::Cons(c2) => {
                        let (sev, msg) = {
                            let cell2 = c2.borrow();
                            (cell2.car.clone(), cell2.cdr.clone())
                        };
                        let message = match msg {
                            Value::Str(s) => (*s).clone(),
                            _ => String::new(),
                        };
                        (sev, message)
                    }
                    // Pre-stage-3 shape: `cdr` IS the severity.
                    _ => (rest.clone(), String::new()),
                };
                if let (Value::Int(l), Value::Int(s)) = (line, sev) {
                    lines.push((l.max(0) as usize, s.clamp(1, 4) as u8, message));
                }
            }
        }
        let ed = ed_handle(i);
        ed.borrow_mut()
            .diagnostics
            .insert(Rc::as_ptr(&b) as usize, lines);
        Ok(Value::Nil)
    });
    defun(interp, "buffer-list", 0, Some(1), |i, _| {
        let ed = ed_handle(i);
        let list = ed
            .borrow()
            .buffers
            .iter()
            .map(Editor::buffer_value)
            .collect();
        Ok(Value::list(list))
    });
    defun(interp, "buffer-size", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let n = b.borrow().text.len();
        Ok(Value::Int(n as i64))
    });
    defun(interp, "buffer-string", 0, Some(0), |i, _| {
        let b = cur(i);
        let s = b.borrow().text.to_string();
        Ok(Value::string(s))
    });
    defun(interp, "buffer-modified-p", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let m = b.borrow().modified;
        Ok(Value::bool(m, i.syms.t))
    });
    // GNU-named accessor for `Buffer::edit_ticks` (M31): bumped by every
    // insert/delete/undo entry point (see the field doc in buffer.rs and
    // highlight.rs's use of the same counter for parse staleness) — a
    // monotonic per-buffer generation number, not just a "modified?"
    // bool, so a caller can tell "unchanged since I last looked" apart
    // from "changed, but happens to be modified either way". dabbrev.el
    // uses this (together with `eq'-comparing `current-buffer') to tell
    // a genuine repeat completion apart from a false-positive session
    // continuation across an intervening edit or buffer switch.
    defun(interp, "buffer-modified-tick", 0, Some(1), |i, a| {
        let b = buffer_arg(i, &opt(a, 0))?;
        let tick = b.borrow().edit_ticks;
        Ok(Value::Int(tick as i64))
    });
    defun(interp, "set-buffer-modified-p", 1, Some(1), |i, a| {
        cur(i).borrow_mut().modified = a[0].truthy();
        Ok(a[0].clone())
    });
    defun(interp, "kill-buffer", 0, Some(1), |i, a| {
        let target = buffer_arg(i, &opt(a, 0))?;
        let ed = ed_handle(i);
        let is_current = Rc::ptr_eq(&ed.borrow().current, &target);
        // kill-buffer-hook runs with the dying buffer as current (GNU
        // semantics), so a non-current target is switched in for the
        // call and switched back out afterward.
        if is_current {
            crate::commands::run_hook_by_name(i, "kill-buffer-hook");
        } else {
            let original = ed.borrow().current.clone();
            set_current_buffer(i, &ed, target.clone());
            crate::commands::run_hook_by_name(i, "kill-buffer-hook");
            // The hook may itself have killed ORIGINAL (e.g. a "kill
            // this buffer's companions together" hook) -- switching
            // back to a buffer no longer in `ed.buffers` would leave
            // `Editor::current` pointing at a zombie the rest of the
            // editor can't see (absent from `buffer-list`, unfindable
            // by name), and the window-redirect fallback below would
            // then redirect windows onto that zombie too. Fall back to
            // picking a live buffer the same way the is-current branch
            // does when TARGET itself was the last one standing.
            let original_alive = ed.borrow().buffers.iter().any(|b| Rc::ptr_eq(b, &original));
            if original_alive {
                set_current_buffer(i, &ed, original);
            } else {
                switch_to_a_live_buffer(i, &ed, &target);
            }
        }
        // The hook may have already killed TARGET itself (or otherwise
        // removed it) -- don't redo the removal below.
        if !ed.borrow().buffers.iter().any(|b| Rc::ptr_eq(b, &target)) {
            return Ok(Value::Sym(i.syms.t));
        }
        if is_current {
            // Switch away first so buffer-local swapping stays consistent.
            switch_to_a_live_buffer(i, &ed, &target);
        }
        // Any window still showing the killed buffer must switch away too.
        let fallback = ed.borrow().current.clone();
        for win in ed.borrow_mut().windows.values_mut() {
            if Rc::ptr_eq(&win.buffer, &target) {
                win.buffer = fallback.clone();
                win.point = fallback.borrow().point;
                win.window_start = 0;
            }
        }
        ed.borrow_mut().buffers.retain(|b| !Rc::ptr_eq(b, &target));
        // M87 stage 3 (D12): `diagnostics` is keyed by `Rc::as_ptr` and was
        // never invalidated anywhere -- after TARGET is freed, an unrelated
        // later `Rc` allocation can land on the same address and inherit
        // its dead diagnostics (wrong gutter dot today, a wrong inline
        // message once stage 3 lands). This is the buffer's one true death
        // point (the hook may have already removed it above, in which case
        // this is a harmless no-op).
        ed.borrow_mut()
            .diagnostics
            .remove(&(Rc::as_ptr(&target) as usize));
        Ok(Value::Sym(i.syms.t))
    });
    defun(interp, "buffer-substring", 2, Some(2), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let borrowed = b.borrow();
            let s = super::get_pos(i, &borrowed, &a[0])?;
            let e = super::get_pos(i, &borrowed, &a[1])?;
            (s.min(e), s.max(e))
        };
        let text = b.borrow().text.slice(s, e);
        Ok(Value::string(text))
    });
    defun(
        interp,
        "buffer-substring-no-properties",
        2,
        Some(2),
        |i, a| {
            let b = cur(i);
            let (s, e) = {
                let borrowed = b.borrow();
                let s = super::get_pos(i, &borrowed, &a[0])?;
                let e = super::get_pos(i, &borrowed, &a[1])?;
                (s.min(e), s.max(e))
            };
            let text = b.borrow().text.slice(s, e);
            Ok(Value::string(text))
        },
    );
    defun(interp, "erase-buffer", 0, Some(0), |i, _| {
        let b = cur(i);
        super::check_writable(i, &b)?;
        let len = b.borrow().text.len();
        // Must go through `edit_delete`, not `Buffer::delete` directly --
        // `Buffer::delete` only shifts positions `Buffer` itself owns
        // (point/mark/markers/overlays); `Window.point`/`window_start`
        // live on `Editor` (see `editor::adjust_windows_for_edit`'s doc),
        // so a caller that bypasses `edit_delete` leaves every OTHER
        // window showing this buffer with a stale point/window_start
        // (M72 period 2 -- this was the one other direct-`delete` call
        // site left in the crate after the `edit_insert`/`edit_delete`
        // consolidation).
        let ed = ed_handle(i);
        crate::editor::edit_delete(&ed, &b, 0, len);
        Ok(Value::Nil)
    });

    // Buffer-local variables.
    defun(interp, "make-local-variable", 1, Some(1), |i, a| {
        let sym = need_sym(i, &a[0])?;
        let ed = ed_handle(i);
        let current_val = i.sym_value(sym).unwrap_or(Value::Nil);
        let already = ed.borrow().current.borrow().locals.contains_key(&sym);
        if !already {
            make_buffer_local(i, &ed, sym, current_val);
        }
        Ok(Value::Sym(sym))
    });
    defun(interp, "local-variable-p", 1, Some(2), |i, a| {
        let sym = need_sym(i, &a[0])?;
        let b = buffer_arg(i, &opt(a, 1))?;
        let is = b.borrow().locals.contains_key(&sym);
        Ok(Value::bool(is, i.syms.t))
    });
    defun(interp, "buffer-local-value", 2, Some(2), |i, a| {
        let sym = need_sym(i, &a[0])?;
        let b = buffer_arg(i, &a[1])?;
        let ed = ed_handle(i);
        let found = crate::editor::buffer_local_value(i, &ed.borrow(), sym, &b);
        found.ok_or_else(|| {
            let e = i.syms.void_variable;
            i.signal(e, vec![Value::Sym(sym)])
        })
    });

    // Excursion / current-buffer helpers used by simple.el macros.
    defun(interp, "save-excursion-internal", 1, Some(1), |i, a| {
        let ed = ed_handle(i);
        let buf = ed.borrow().current.clone();
        // Save point as a marker so edits before it shift it correctly.
        let point_marker = Rc::new(RefCell::new(crate::buffer::MarkerData {
            buffer: Rc::downgrade(&buf),
            pos: buf.borrow().point,
        }));
        buf.borrow_mut().markers.push(Rc::downgrade(&point_marker));
        let saved_mark = buf.borrow().mark;
        let f = a[0].clone();
        let result = apply_function(i, &f, vec![].into());
        let ed = ed_handle(i);
        set_current_buffer(i, &ed, buf.clone());
        {
            let mut b = buf.borrow_mut();
            let pos = point_marker.borrow().pos;
            b.point = b.clamp(pos as i64);
            b.mark = saved_mark;
        }
        result
    });
    defun(
        interp,
        "with-current-buffer-internal",
        2,
        Some(2),
        |i, a| {
            let target = buffer_arg(i, &a[0])?;
            let ed = ed_handle(i);
            let prev = ed.borrow().current.clone();
            set_current_buffer(i, &ed, target);
            let f = a[1].clone();
            let result = apply_function(i, &f, vec![].into());
            let ed = ed_handle(i);
            let still_alive = ed.borrow().buffers.iter().any(|b| Rc::ptr_eq(b, &prev));
            if still_alive {
                set_current_buffer(i, &ed, prev);
            }
            result
        },
    );
}

fn unique_name(ed: &Rc<RefCell<Editor>>, base: &str) -> String {
    let editor = ed.borrow();
    if editor.find_buffer(base).is_none() {
        return base.to_string();
    }
    for n in 2.. {
        let candidate = format!("{}<{}>", base, n);
        if editor.find_buffer(&candidate).is_none() {
            return candidate;
        }
    }
    unreachable!()
}

/// Make some OTHER live buffer (never `exclude`) current -- any buffer
/// still in `ed.buffers` will do, falling back to a freshly created
/// `*scratch*` if `exclude` was the last one standing. Shared by
/// `kill-buffer`'s two "the buffer we were about to switch to/from is
/// gone" cases: killing the current buffer itself, and a
/// `kill-buffer-hook` that killed the caller's own (non-current) buffer
/// out from under it.
fn switch_to_a_live_buffer(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    exclude: &Rc<RefCell<Buffer>>,
) {
    let other = ed
        .borrow()
        .buffers
        .iter()
        .find(|b| !Rc::ptr_eq(b, exclude))
        .cloned();
    match other {
        Some(b) => set_current_buffer(interp, ed, b),
        None => {
            let scratch = Rc::new(RefCell::new(Buffer::new("*scratch*", "")));
            ed.borrow_mut().buffers.push(scratch.clone());
            set_current_buffer(interp, ed, scratch);
        }
    }
}
