use elisp::builtins::{defun, need_int, need_str, opt};
use elisp::{Interp, Value};

use super::{cur, ed_handle, get_pos, int_pos};
use crate::editor::{adjust_windows_for_edit, edit_delete, edit_insert};

pub fn register(interp: &mut Interp) {
    // Point and basic positions.
    defun(interp, "point", 0, Some(0), |i, _| {
        Ok(int_pos(cur(i).borrow().point))
    });
    defun(interp, "point-min", 0, Some(0), |_, _| Ok(Value::Int(1)));
    defun(interp, "point-max", 0, Some(0), |i, _| {
        Ok(int_pos(cur(i).borrow().text.len()))
    });
    defun(interp, "goto-char", 1, Some(1), |i, a| {
        let b = cur(i);
        let pos = {
            let borrowed = b.borrow();
            get_pos(i, &borrowed, &a[0])?
        };
        b.borrow_mut().point = pos;
        b.borrow_mut().goal_column = None;
        Ok(int_pos(pos))
    });
    defun(interp, "forward-char", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        let mut bb = b.borrow_mut();
        bb.point = bb.clamp(bb.point as i64 + n);
        bb.goal_column = None;
        Ok(Value::Nil)
    });
    defun(interp, "backward-char", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        let mut bb = b.borrow_mut();
        bb.point = bb.clamp(bb.point as i64 - n);
        bb.goal_column = None;
        Ok(Value::Nil)
    });
    defun(interp, "beginning-of-line", 0, Some(1), |i, _| {
        let b = cur(i);
        let mut bb = b.borrow_mut();
        bb.point = bb.text.line_start(bb.point);
        bb.goal_column = None;
        Ok(Value::Nil)
    });
    defun(interp, "end-of-line", 0, Some(1), |i, _| {
        let b = cur(i);
        let mut bb = b.borrow_mut();
        bb.point = bb.text.line_end(bb.point);
        bb.goal_column = None;
        Ok(Value::Nil)
    });
    defun(interp, "line-beginning-position", 0, Some(1), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(int_pos(bb.text.line_start(bb.point)))
    });
    defun(interp, "line-end-position", 0, Some(1), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(int_pos(bb.text.line_end(bb.point)))
    });
    defun(interp, "forward-line", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        let mut bb = b.borrow_mut();
        let mut shortfall = 0i64;
        if n >= 0 {
            for _ in 0..n {
                let le = bb.text.line_end(bb.point);
                if le >= bb.text.len() {
                    // Can't cross a newline: report the shortfall (like
                    // Emacs, this must be non-zero at eob or loops that
                    // walk the buffer line by line never terminate).
                    bb.point = bb.text.len();
                    shortfall += 1;
                } else {
                    bb.point = le + 1;
                }
            }
        } else {
            bb.point = bb.text.line_start(bb.point);
            for _ in 0..(-n) {
                if bb.point == 0 {
                    shortfall += 1;
                } else {
                    let prev_end = bb.point - 1;
                    bb.point = bb.text.line_start(prev_end);
                }
            }
        }
        bb.goal_column = None;
        Ok(Value::Int(shortfall.max(0)))
    });
    defun(interp, "next-line", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        move_lines(i, n);
        Ok(Value::Nil)
    });
    defun(interp, "previous-line", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        move_lines(i, -n);
        Ok(Value::Nil)
    });
    defun(interp, "bolp", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(Value::bool(
            bb.point == bb.text.line_start(bb.point),
            i.syms.t,
        ))
    });
    defun(interp, "eolp", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(Value::bool(
            bb.point == bb.text.line_end(bb.point),
            i.syms.t,
        ))
    });
    defun(interp, "bobp", 0, Some(0), |i, _| {
        Ok(Value::bool(cur(i).borrow().point == 0, i.syms.t))
    });
    defun(interp, "eobp", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(Value::bool(bb.point == bb.text.len(), i.syms.t))
    });
    defun(interp, "char-after", 0, Some(1), |i, a| {
        let b = cur(i);
        let bb = b.borrow();
        let pos = get_pos(i, &bb, &opt(a, 0))?;
        Ok(bb
            .text
            .char_at(pos)
            .map(|c| Value::Int(c as i64))
            .unwrap_or(Value::Nil))
    });
    defun(interp, "char-before", 0, Some(1), |i, a| {
        let b = cur(i);
        let bb = b.borrow();
        let pos = get_pos(i, &bb, &opt(a, 0))?;
        if pos == 0 {
            return Ok(Value::Nil);
        }
        Ok(bb
            .text
            .char_at(pos - 1)
            .map(|c| Value::Int(c as i64))
            .unwrap_or(Value::Nil))
    });
    defun(interp, "line-number-at-pos", 0, Some(1), |i, a| {
        let b = cur(i);
        let bb = b.borrow();
        let pos = get_pos(i, &bb, &opt(a, 0))?;
        Ok(Value::Int(bb.text.line_number(pos) as i64))
    });
    defun(interp, "current-column", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        let start = bb.text.line_start(bb.point);
        Ok(Value::Int((bb.point - start) as i64))
    });

    // Insertion and deletion.
    defun(interp, "insert", 0, None, |i, a| {
        let mut text = String::new();
        for v in a.iter() {
            match v {
                Value::Str(s) => text.push_str(s),
                Value::Int(c) => {
                    let ch = u32::try_from(*c)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| i.wrong_type("characterp", v))?;
                    text.push(ch);
                }
                other => return Err(i.wrong_type("char-or-string-p", other)),
            }
        }
        let b = cur(i);
        super::check_writable(i, &b)?;
        let point = b.borrow().point;
        let ed = ed_handle(i);
        edit_insert(&ed, &b, point, &text);
        Ok(Value::Nil)
    });
    defun(interp, "delete-region", 2, Some(2), |i, a| {
        let b = cur(i);
        super::check_writable(i, &b)?;
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let ed = ed_handle(i);
        edit_delete(&ed, &b, s, e);
        Ok(Value::Nil)
    });
    // M46: replace [BEG, END) with NEWTEXT via the minimal set of hunks
    // that turn the old text into NEWTEXT, instead of deleting the whole
    // region and reinserting it. Named to match GNU Emacs 27+'s
    // `replace-region-contents` (this project already has precedent for
    // aligning with GNU-added names — see `buffer-modified-tick` in
    // `crates/core/src/builtins/buffers.rs:145`); the GNU version's third
    // argument is a function, ours is a plain string, which is simpler
    // and sufficient for our one caller (`lsp--apply-text-edits`).
    //
    // Why not `(delete-region BEG END) (insert NEWTEXT)`: that collapses
    // every marker/point/overlay inside the region to a single point,
    // and worse, silently *reverses* overlay start/end (measured:
    // overlay 15..18 became 40..1 after a full-buffer replace-and-insert
    // — 0-based end-of-buffer..start-of-buffer). That in turn makes
    // `overlays-in` blind to the surviving overlay, so
    // `lsp--decorate-buffer`'s "delete my old diagnostic overlays" loop
    // (`crates/core/lisp/lsp.el:184-186`) leaks a zombie overlay on every
    // format-on-save. Diffing and touching only the hunks that actually
    // changed avoids all of this: unaffected markers/point/overlays ride
    // out the edit untouched via the buffer's existing
    // `adjust_positions_insert`/`_delete` (`crates/core/src/buffer.rs:467`
    // /`:506`), no special-casing needed here.
    //
    // Point/marker/overlay behavior when a hunk *does* cover them:
    // deleting first collapses the position to the hunk's start, then
    // inserting the replacement text carries it forward to the
    // replacement's end. That's "falls at the end of the replacement
    // text" — the covered region was rewritten, so there's no principled
    // original position to preserve; landing at the end (rather than,
    // say, the start) matches Emacs's own insert-after-delete semantics.
    defun(interp, "replace-region-contents", 3, Some(4), |i, a| {
        let b = cur(i);
        super::check_writable(i, &b)?;
        let (beg, end) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let newtext = need_str(i, &a[2])?;
        // MAX-COST is exposed as an optional argument purely for
        // testability: to deterministically exercise the "give up, do a
        // full replace" path we set it to 0 rather than constructing a
        // pathologically large input.
        let max_cost = match opt(a, 3) {
            Value::Nil => crate::textdiff::DEFAULT_BUDGET,
            v => {
                let n = need_int(i, &v)?;
                if n < 0 {
                    0
                } else {
                    n as usize
                }
            }
        };
        let old = b.borrow().text.slice(beg, end);
        let ed = ed_handle(i);
        match crate::textdiff::diff_hunks(&old, &newtext, max_cost) {
            None => {
                edit_delete(&ed, &b, beg, end);
                edit_insert(&ed, &b, beg, &newtext);
                Ok(Value::Sym(i.intern("full")))
            }
            Some(hunks) => {
                if hunks.is_empty() {
                    return Ok(Value::Nil);
                }
                // Apply back-to-front: earlier hunks' offsets stay valid
                // since nothing after them has shifted yet.
                for h in hunks.iter().rev() {
                    let s = beg + h.start;
                    let e = beg + h.end;
                    edit_delete(&ed, &b, s, e);
                    edit_insert(&ed, &b, s, &h.text);
                }
                Ok(Value::Sym(i.syms.t))
            }
        }
    });
    defun(interp, "delete-char", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        super::check_writable(i, &b)?;
        let (start, end) = {
            let bb = b.borrow();
            let point = bb.point;
            if n >= 0 {
                (point, bb.clamp(point as i64 + n))
            } else {
                (bb.clamp(point as i64 + n), point)
            }
        };
        let ed = ed_handle(i);
        edit_delete(&ed, &b, start, end);
        Ok(Value::Nil)
    });

    // M20: sexp motion (elisp syntax only, no syntax tables — v1).
    defun(interp, "forward-sexp", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        let chars: Vec<char> = {
            let bb = b.borrow();
            bb.text.slice(0, bb.text.len()).chars().collect()
        };
        let mut pos = b.borrow().point;
        for _ in 0..n.unsigned_abs() {
            let step = if n >= 0 {
                crate::sexp::forward_one(&chars, pos)
            } else {
                crate::sexp::backward_one(&chars, pos)
            };
            match step {
                Ok(p) => pos = p,
                Err(msg) => return Err(i.error(msg)),
            }
        }
        b.borrow_mut().point = pos;
        Ok(Value::Nil)
    });

    // Mark and region.
    defun(interp, "mark", 0, Some(1), |i, _| {
        let b = cur(i);
        let m = b.borrow().mark;
        Ok(m.map(int_pos).unwrap_or(Value::Nil))
    });
    defun(interp, "set-mark", 1, Some(1), |i, a| {
        let b = cur(i);
        let pos = {
            let bb = b.borrow();
            get_pos(i, &bb, &a[0])?
        };
        let mut bb = b.borrow_mut();
        bb.mark = Some(pos);
        bb.mark_active = true;
        Ok(int_pos(pos))
    });
    defun(interp, "push-mark", 0, Some(3), |i, a| {
        let b = cur(i);
        let pos = {
            let bb = b.borrow();
            get_pos(i, &bb, &opt(a, 0))?
        };
        let mut bb = b.borrow_mut();
        bb.mark = Some(pos);
        Ok(Value::Nil)
    });
    defun(interp, "deactivate-mark", 0, Some(1), |i, _| {
        cur(i).borrow_mut().mark_active = false;
        Ok(Value::Nil)
    });
    defun(interp, "region-beginning", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        match bb.mark {
            Some(m) => Ok(int_pos(m.min(bb.point))),
            None => Err(i.error("The mark is not set now, so there is no region")),
        }
    });
    defun(interp, "region-end", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        match bb.mark {
            Some(m) => Ok(int_pos(m.max(bb.point))),
            None => Err(i.error("The mark is not set now, so there is no region")),
        }
    });
    // M101: t only when a mark exists AND it's marked active -- `push-mark'
    // sets the former without the latter, so callers that just want "is
    // there a usable region right now" (expand-region.el) need this rather
    // than testing `mark' directly.
    defun(interp, "region-active-p", 0, Some(0), |i, _| {
        let b = cur(i);
        let bb = b.borrow();
        Ok(Value::bool(bb.mark.is_some() && bb.mark_active, i.syms.t))
    });

    // Kill ring.
    defun(interp, "kill-region-internal", 2, Some(2), |i, a| {
        let b = cur(i);
        super::check_writable(i, &b)?;
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let ed = ed_handle(i);
        let text = edit_delete(&ed, &b, s, e);
        ed.borrow_mut().kill_new(text);
        b.borrow_mut().mark_active = false;
        Ok(Value::Nil)
    });
    defun(interp, "kill-ring-save-internal", 2, Some(2), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let text = b.borrow().text.slice(s, e);
        let ed = ed_handle(i);
        ed.borrow_mut().kill_new(text);
        b.borrow_mut().mark_active = false;
        Ok(Value::Nil)
    });
    defun(interp, "kill-line", 0, Some(1), |i, _| {
        let b = cur(i);
        super::check_writable(i, &b)?;
        let ed = ed_handle(i);
        let (start, end) = {
            let bb = b.borrow();
            let le = bb.text.line_end(bb.point);
            if bb.point == le && le < bb.text.len() {
                (bb.point, le + 1) // at eol: kill the newline
            } else {
                (bb.point, le)
            }
        };
        if start == end {
            return Ok(Value::Nil);
        }
        let text = edit_delete(&ed, &b, start, end);
        // Consecutive kill-lines append to the same kill-ring entry.
        let append = {
            let editor = ed.borrow();
            matches!(&editor.last_command, Value::Sym(id) if i.sym_name(*id) == "kill-line")
        };
        if append {
            ed.borrow_mut().kill_append(text, false);
        } else {
            ed.borrow_mut().kill_new(text);
        }
        Ok(Value::Nil)
    });
    // Kill the whole line(s) point is on. Mirrors `kill-line` above (same
    // append-to-kill-ring-on-repeat logic). Behavior verified against real
    // GNU Emacs 30.2 (`emacs -Q --batch`, plus reading `kill-whole-line` in
    // its `lisp/simple.el`) rather than assumed, after an earlier version
    // of this comment asserted a GNU rule ("back up over the preceding
    // newline when the last line has none") that turned out not to exist
    // and corrupted the buffer by eating the *previous* line's newline
    // whenever point sat on the buffer's implicit trailing empty line.
    //
    // - n >= 1: kill n whole lines forward, starting at the beginning of
    //   point's line, newlines included. If fewer than n real lines
    //   remain, kill only up to the buffer's end (no error) — GNU does the
    //   same via `forward-visible-line`. If point is already sitting on
    //   the buffer's implicit trailing empty line (an eob with no content
    //   after it, e.g. right after a final newline, or a wholly empty
    //   buffer), the loop below naturally comes out with `start == end`
    //   there — GNU signals `end-of-buffer` instead, but `kill-line` right
    //   above us handles its own equivalent case (`start == end` at eob)
    //   by silently doing nothing rather than signaling, so we follow that
    //   local convention here too instead of adding a new error signal.
    // - n == 0: kill the current line's content only, excluding its
    //   trailing newline (if any). A third divergence lives here too: on a
    //   line with no content (`start == end`), real GNU unconditionally
    //   pre-seeds the kill ring with `(kill-new "")` before it ever
    //   computes the range (`simple.el`, around line 6705), so it pushes
    //   an empty string onto the kill ring even though nothing changes in
    //   the buffer. Verified against real GNU Emacs 30.2: `(kill-whole-line
    //   0)` on a blank line leaves `kill-ring` holding `("")`. We instead
    //   fall straight into the shared `start == end` early return below
    //   (the same one the n>=1 and n<0 boundary cases use) and push
    //   nothing. This is the same shared short-circuit as the other two
    //   divergences, not a separate decision — behavior unchanged, noted
    //   here only because it was missing from this list.
    // - n < 0: kill backward. Kill |n| lines counting the current one,
    //   *excluding* the current line's own trailing newline but
    //   *including* the newline that precedes the first killed line (GNU:
    //   "Also kill the preceding newline. This is meant to make `repeat`
    //   work well with negative arguments.") If the backward count runs
    //   past the buffer's start, stop there (no error, matching the n >= 1
    //   overshoot rule); if point is already on an empty first line with
    //   nothing before it, the loop below likewise comes out with `start
    //   == end` — GNU signals `beginning-of-buffer`, we again follow
    //   `kill-line`'s silent no-op-at-the-boundary convention instead.
    defun(interp, "kill-whole-line", 0, Some(1), |i, a| {
        let n = match opt(a, 0) {
            Value::Nil => 1i64,
            v => need_int(i, &v)?,
        };
        let b = cur(i);
        super::check_writable(i, &b)?;
        let ed = ed_handle(i);
        let (start, end) = {
            let bb = b.borrow();
            let len = bb.text.len();
            let bol = bb.text.line_start(bb.point);
            if n >= 1 {
                let mut pos = bol;
                for _ in 0..n {
                    let le = bb.text.line_end(pos);
                    if le < len {
                        pos = le + 1;
                    } else {
                        pos = len;
                        break;
                    }
                }
                (bol, pos)
            } else if n == 0 {
                (bol, bb.text.line_end(bol))
            } else {
                let k = n.unsigned_abs() as usize;
                let mut pos = bol;
                for _ in 0..k.saturating_sub(1) {
                    if pos == 0 {
                        break;
                    }
                    pos = bb.text.line_start(pos - 1);
                }
                pos = pos.saturating_sub(1);
                (pos, bb.text.line_end(bol))
            }
        };
        if start == end {
            return Ok(Value::Nil);
        }
        let text = edit_delete(&ed, &b, start, end);
        // Consecutive kill-whole-lines append to the same kill-ring entry.
        let append = {
            let editor = ed.borrow();
            matches!(&editor.last_command, Value::Sym(id) if i.sym_name(*id) == "kill-whole-line")
        };
        if append {
            ed.borrow_mut().kill_append(text, false);
        } else {
            ed.borrow_mut().kill_new(text);
        }
        Ok(Value::Nil)
    });
    defun(interp, "current-kill", 0, Some(1), |i, _| {
        let ed = ed_handle(i);
        let editor = ed.borrow();
        Ok(editor
            .kill_ring
            .get(editor.kill_ring_yank)
            .map(Value::string)
            .unwrap_or(Value::Nil))
    });
    defun(interp, "yank", 0, Some(1), |i, _| {
        let ed = ed_handle(i);
        let text = {
            let editor = ed.borrow();
            match editor.kill_ring.get(editor.kill_ring_yank) {
                Some(t) => t.clone(),
                None => return Err(i.error("Kill ring is empty")),
            }
        };
        let b = cur(i);
        super::check_writable(i, &b)?;
        let start = b.borrow().point;
        edit_insert(&ed, &b, start, &text);
        ed.borrow_mut().last_yank = Some((start, text.chars().count()));
        Ok(Value::Nil)
    });
    defun(interp, "yank-pop", 0, Some(1), |i, _| {
        let ed = ed_handle(i);
        let was_yank = {
            let editor = ed.borrow();
            matches!(&editor.last_command, Value::Sym(id) if {
                let n = i.sym_name(*id);
                n == "yank" || n == "yank-pop"
            }) && editor.last_yank.is_some()
        };
        if !was_yank {
            return Err(i.error("Previous command was not a yank"));
        }
        let (start, len) = ed.borrow().last_yank.unwrap();
        {
            let mut editor = ed.borrow_mut();
            if editor.kill_ring_yank == 0 {
                editor.kill_ring_yank = editor.kill_ring.len().saturating_sub(1);
            } else {
                editor.kill_ring_yank -= 1;
            }
        }
        let text = {
            let editor = ed.borrow();
            editor.kill_ring[editor.kill_ring_yank].clone()
        };
        let b = cur(i);
        super::check_writable(i, &b)?;
        edit_delete(&ed, &b, start, start + len);
        edit_insert(&ed, &b, start, &text);
        ed.borrow_mut().last_yank = Some((start, text.chars().count()));
        Ok(Value::Nil)
    });

    // Undo.
    defun(interp, "undo-boundary", 0, Some(0), |i, _| {
        cur(i).borrow_mut().undo_boundary();
        Ok(Value::Nil)
    });
    // M30: suppresses the boundary `self_insert` (commands.rs) would
    // otherwise insert before the NEXT self-inserted character, so it
    // joins the CURRENT undo group instead of starting a new one — the
    // mechanism evil.el's `evil--operator-apply' (the `change' operator)
    // and `evil-open-below'/`evil-open-above' call right after their own
    // delete/newline to merge it with the text about to be typed. A
    // one-shot: consumed (cleared) by whichever comes first, the next
    // `self_insert' or the next `execute_command' (see both in
    // commands.rs), so it can never survive to suppress some LATER,
    // unrelated insert if nothing gets typed in between (e.g. `cw'
    // immediately followed by ESC with no text typed at all).
    defun(interp, "undo-amalgamate-boundary", 0, Some(0), |i, _| {
        ed_handle(i).borrow_mut().suppress_next_undo_boundary = true;
        Ok(Value::Nil)
    });
    defun(interp, "undo-internal", 0, Some(0), |i, _| {
        let ed = ed_handle(i);
        let b = cur(i);
        super::check_writable(i, &b)?;
        // Consecutive undos keep consuming older groups; any other
        // intervening command resets to the top (which then redoes).
        let continuing = {
            let editor = ed.borrow();
            matches!(&editor.last_command, Value::Sym(id) if i.sym_name(*id) == "undo")
        };
        let from = {
            let mut bb = b.borrow_mut();
            if !continuing {
                bb.pending_undo = None;
            }
            let len = bb.undo.len();
            bb.pending_undo.unwrap_or(len)
        };
        // `undo_step_from`'s `Buffer::borrow_mut` ends with this
        // statement (the whole call is a temporary) -- `adjust_windows_
        // for_edit` below borrows `ed`, not `b`, but keeping the two
        // borrows non-overlapping regardless is the established rule for
        // this boundary (see PLAN.md's borrow-discipline notes).
        let result = b.borrow_mut().undo_step_from(from);
        match result {
            Some((start, edits)) => {
                b.borrow_mut().pending_undo = Some(start);
                // M72 period 2: mirror every edit undo just applied onto
                // every OTHER window showing this buffer, in the same
                // order `undo_step_from` applied them -- see that
                // function's doc for why order (not just the set of
                // edits) matters here.
                for edit in edits {
                    adjust_windows_for_edit(&ed, &b, edit);
                }
                Ok(Value::Sym(i.syms.t))
            }
            None => Ok(Value::Nil),
        }
    });

    // Files.
    defun(interp, "insert-file-contents", 1, Some(2), |i, a| {
        let path = need_str(i, &a[0])?.to_string();
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| i.error(format!("Cannot read file {}: {}", path, e)))?;
        let b = cur(i);
        super::check_writable(i, &b)?;
        let point = b.borrow().point;
        let ed = ed_handle(i);
        let n = edit_insert(&ed, &b, point, &contents);
        Ok(Value::list(vec![Value::string(path), Value::Int(n as i64)]))
    });
    defun(interp, "write-region", 3, Some(3), |i, a| {
        let b = cur(i);
        let (s, e) = {
            let bb = b.borrow();
            let s = get_pos(i, &bb, &a[0])?;
            let e = get_pos(i, &bb, &a[1])?;
            (s.min(e), s.max(e))
        };
        let path = need_str(i, &a[2])?.to_string();
        let text = b.borrow().text.slice(s, e);
        if let Some(rp) = crate::remote::parse(&path) {
            crate::remote::write_file(&rp, &text).map_err(|er| i.error(er))?;
        } else {
            std::fs::write(&path, text)
                .map_err(|er| i.error(format!("Cannot write file {}: {}", path, er)))?;
        }
        Ok(Value::Nil)
    });
    // M62: FORCE (optional, default nil) skips the on-disk-conflict guard
    // below outright -- the non-interactive way to overwrite a file that
    // changed on disk since it was read. Neither a keybinding nor `M-x` can
    // ever pass an argument here (`interactive_spec()` returns `None` for
    // Rust builtins, so `execute_command` always calls with an empty arg
    // vector), but evil's `:w!` (a plain elisp call, not routed through the
    // command loop) calls `(save-buffer t)` directly. The interactive path
    // instead relies on `save_conflict_ack` below (per-buffer, per-path --
    // see its field doc for why NOT `Editor::last_command`).
    defun(interp, "save-buffer", 0, Some(1), |i, a| {
        let force = opt(a, 0).truthy();
        let b = cur(i);
        let (path, name) = {
            let bb = b.borrow();
            (bb.file.clone(), bb.name.clone())
        };
        let Some(path) = path else {
            return Err(i.error(format!("Buffer {} is not visiting a file", name)));
        };
        // Run before-save-hook before the snapshot below: it may edit
        // the buffer (M40's verilog-auto-on-save relies on this), and
        // the write must see its result.
        //
        // Known edge case (M40-4 review, informational): unlike GNU
        // Emacs's `basic-save-buffer` (which pins the buffer being
        // saved as current for both hooks via `save-current-buffer`),
        // this doesn't re-pin `b` as current before running
        // after-save-hook below. If before-save-hook switched the
        // current buffer, after-save-hook would run in that OTHER
        // buffer's context instead of `b`'s -- the write to disk above
        // is unaffected either way (it already has `b` and `path`
        // captured), only the two hooks' buffer context could disagree.
        // GNU Emacs has the same trap in the sense that a hook doing
        // this is unusual and easy to get wrong; neither of M40's own
        // hook consumers (verilog-auto-on-save, lsp--on-after-save)
        // switches buffers, so this doesn't fire today.
        crate::commands::run_hook_by_name(i, "before-save-hook");
        // M62 (local)/M75 (`/ssh:` remote): refuse to silently clobber an
        // external change. `metadata` (local) or `remote::read_file`
        // (remote) failing right here (e.g. the file was deleted since,
        // or -- remote only -- the connection is down) is handled
        // per-branch below: local treats a failed `metadata` as "not a
        // conflict, let `fs::write` sort it out"; remote treats a failed
        // `read_file` as a hard error (see the remote branch's own
        // comment for why the two can't share that policy).
        //
        // Known gaps (not fixed here, see PLAN.md M62):
        // - `write-region` bypasses this entirely: it can write to any
        //   path, including this buffer's own file, without touching
        //   `modified` or `disk_state` -- a `(write-region ... (buffer-file-name))`
        //   call will make the NEXT `save-buffer` here falsely report a
        //   conflict against a write the user made themselves. Symmetric
        //   between local and remote -- `write-region`'s remote branch
        //   doesn't touch `disk_state` either, so this gap isn't widened
        //   by M75.
        // - Deliberate local/remote ASYMMETRY on "file deleted since
        //   open": local's `if let Ok(m) = std::fs::metadata(&path)`
        //   silently skips the whole conflict check when `metadata`
        //   fails (e.g. the file was deleted), so a local save after an
        //   external delete just silently recreates the file --
        //   `fs::write` handles it on its own, no error. The remote
        //   branch below does the opposite on purpose: `read_file`
        //   returning `Ok(None)` against a `RemoteContent` baseline IS
        //   treated as a conflict and refused (test 7 in
        //   `ssh_tests.rs`). Not made consistent with local here: remote
        //   is strictly the safer of the two policies (refuse-by-default
        //   on an ambiguous "did someone delete this on purpose or is
        //   this transient" situation), and hardening local to match
        //   would be its own decision with its own test fallout,
        //   out of scope for a milestone about closing the "/ssh: has NO
        //   guard at all" gap.
        // - There's no `revert-buffer` in this editor, so on a real
        //   conflict the only options are "give up" or FORCE-overwrite;
        //   there's no way to reload the on-disk version instead.
        // - Local: the check is mtime+size, not a content hash: an
        //   external edit that lands within the same second AND produces
        //   the same byte length is invisible to it. (Remote doesn't have
        //   this gap -- it compares a content digest -- but see the
        //   remote branch's own comment for why mtime+size isn't what's
        //   used there either way.)
        // - The check-then-write is NOT atomic: an external write landing
        //   in the window between the read-back check below and the
        //   write further down would still be silently clobbered.
        //   Deliberately left unaddressed -- closing it needs a
        //   filesystem-level lock or atomic rename-based write, out of
        //   scope here. For remote this window is wider than local (an
        //   extra `read_file` round trip's worth), not narrower.
        // - `save_conflict_ack` binds BOTH the path AND the on-disk state
        //   observed at refusal time (not just the path): a save-buffer
        //   that "just retries" only force-overwrites if disk is still in
        //   exactly the state the user was warned about. This replaces an
        //   earlier design where the ack stayed valid indefinitely until
        //   the next successful write -- that let a SECOND, unrelated
        //   external edit landing between the refusal and the retry get
        //   silently clobbered too, with the user never having seen a
        //   warning about THAT change. Mirrored exactly in the remote
        //   branch below.
        // - M77: `remote::run`'s blocking ssh invocation now has a
        //   wall-clock timeout (`RETICLE_REMOTE_TIMEOUT`, default
        //   30s, see `remote.rs`), not just `ConnectTimeout=5` -- so an
        //   unbounded hang from this milestone's extra `read_file` per
        //   remote save is bounded, not open-ended. Two things it does
        //   NOT fix: the timeout is per-`run()` call, not per user-
        //   visible operation, so one `save-buffer` on a hung remote can
        //   still take up to 3x the configured timeout -- this guard's
        //   `read_file` is itself TWO `run()` calls (`test -e` then
        //   `cat`, see `remote::read_file`), plus one for the write (M77
        //   tail review, finding 6: "probe plus write" read like two);
        //   and the wait is
        //   still not interruptible -- `C-g` does nothing while a
        //   `run()` call is in flight, timeout or not, because breaking
        //   out of it early needs the event loop itself to notice
        //   keypresses mid-call, which is a different fix than this one.
        if let Some(rp) = crate::remote::parse(&path) {
            // M75: no single-file stat primitive exists for `/ssh:` (see
            // `DiskState::RemoteContent`'s doc for the three designs
            // that were tried and rejected before this one), so the
            // baseline is a content digest instead of mtime/size, and
            // checking it means actually rereading the file -- one more
            // `read_file` call per remote save, which is exactly 2 more
            // ssh round trips (its `test -e` probe, plus one of
            // `cat`/a `true` liveness check).
            let disk_state = b.borrow().disk_state;
            match crate::remote::read_file(&rp).map_err(|e| i.error(e))? {
                None => {
                    // The remote file doesn't exist right now. Only a
                    // conflict if we last saw content there -- if we
                    // already knew it was Absent (new file, nobody's
                    // raced us) this is the expected first save.
                    if matches!(disk_state, crate::buffer::DiskState::RemoteContent { .. })
                        && !force
                    {
                        refuse_save_conflict(i, &b, &path, crate::buffer::DiskState::Absent)?;
                    }
                }
                Some(remote_text) => {
                    let observed = crate::buffer::DiskState::RemoteContent {
                        digest: crate::buffer::content_digest(remote_text.as_bytes()),
                        len: remote_text.len() as u64,
                    };
                    let conflict = match disk_state {
                        crate::buffer::DiskState::RemoteContent { .. } => disk_state != observed,
                        crate::buffer::DiskState::Absent => true,
                        // `Known`/`Unknown` are local-only in practice --
                        // `find_file_remote` only ever sets a remote
                        // buffer's `disk_state` to `RemoteContent` or
                        // `Absent` -- so these two arms shouldn't be
                        // reachable today. Not a conflict either way, per
                        // M62's "no trustworthy baseline never
                        // false-positives" policy (see `DiskState::
                        // Unknown`'s doc): a baseline-less save that
                        // sometimes false-positives just trains the user
                        // to reflexively force past the warning.
                        // Listed explicitly instead of `_` on purpose: a
                        // wildcard here would silently treat any FUTURE
                        // new `DiskState` variant as "safe to overwrite"
                        // too, with nobody forced to make that call. This
                        // match staying exhaustive against real variant
                        // names means adding one breaks the build until
                        // someone decides on purpose, the same argument
                        // M52 made for `ExtRef`'s tracer.
                        crate::buffer::DiskState::Known(..) | crate::buffer::DiskState::Unknown => {
                            false
                        }
                    };
                    if conflict && !force {
                        refuse_save_conflict(i, &b, &path, observed)?;
                    }
                }
            }
            // Unlike the local branch, a failed reread here is NOT
            // treated as "no conflict, proceed" -- this is the guard's
            // single most important failure mode: if the check itself
            // can't run (dead connection, remote command failing), the
            // save must be refused, not silently allowed through as if
            // nothing changed. `?` above already returns on `Err`.
        } else {
            let disk_state = b.borrow().disk_state;
            if let Ok(m) = std::fs::metadata(&path) {
                let observed = m
                    .modified()
                    .ok()
                    .map(|t| crate::buffer::DiskState::Known(t, m.len()))
                    .unwrap_or(crate::buffer::DiskState::Unknown);
                let conflict = match disk_state {
                    crate::buffer::DiskState::Known(t, n) => {
                        m.modified().ok() != Some(t) || m.len() != n
                    }
                    crate::buffer::DiskState::Absent => true,
                    crate::buffer::DiskState::Unknown => false,
                    // `RemoteContent` is remote-only in practice --
                    // `find-file-internal`'s local path only ever sets
                    // `Known`/`Absent`/`Unknown` -- so this arm shouldn't
                    // be reachable today. Listed explicitly instead of
                    // `_` for the same reason as the remote match's
                    // trailing arm below: a wildcard would silently
                    // absorb any FUTURE new `DiskState` variant as "not a
                    // conflict, overwrite freely" without forcing anyone
                    // to decide that on purpose (M52 made the same
                    // argument for `ExtRef`'s tracer).
                    crate::buffer::DiskState::RemoteContent { .. } => false,
                };
                if conflict && !force {
                    refuse_save_conflict(i, &b, &path, observed)?;
                }
            }
            // metadata failing here is NOT treated as a conflict; fs::write
            // below handles a since-deleted file on its own.
        }
        let text = b.borrow().text.to_string();
        if let Some(rp) = crate::remote::parse(&path) {
            crate::remote::write_file(&rp, &text).map_err(|e| i.error(e))?;
        } else {
            std::fs::write(&path, &text)
                .map_err(|e| i.error(format!("Cannot write file {}: {}", path, e)))?;
        }
        {
            let mut bb = b.borrow_mut();
            bb.modified = false;
            bb.save_conflict_ack = None;
            bb.disk_state = if let Some(_rp) = crate::remote::parse(&path) {
                // M75: refresh the baseline from what was just written,
                // not another reread -- we already know exactly what
                // landed on disk. Skipping this would make the very next
                // save falsely report a conflict against our own write.
                crate::buffer::DiskState::RemoteContent {
                    digest: crate::buffer::content_digest(text.as_bytes()),
                    len: text.len() as u64,
                }
            } else {
                std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| {
                        m.modified()
                            .ok()
                            .map(|t| crate::buffer::DiskState::Known(t, m.len()))
                    })
                    .unwrap_or(crate::buffer::DiskState::Unknown)
            };
        }
        let msg = i.intern("message");
        let _ = elisp::eval::apply_function(
            i,
            &Value::Sym(msg),
            vec![Value::string(format!("Wrote {}", path))].into(),
        );
        crate::commands::run_hook_by_name(i, "after-save-hook");
        Ok(Value::Nil)
    });
    defun(interp, "find-file-internal", 1, Some(1), |i, a| {
        let raw = need_str(i, &a[0])?.to_string();
        let path = expand_path(&raw);
        // M22: /ssh: paths go through the remote transport.
        if let Some(rp) = crate::remote::parse(&path) {
            return find_file_remote(i, &path, &rp);
        }
        // A directory opens in dired instead (M19), like C-x C-f RET
        // on a directory in GNU Emacs.
        if std::fs::metadata(&path)
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            let dired = i.intern("dired");
            return elisp::eval::apply_function(
                i,
                &Value::Sym(dired),
                vec![Value::string(path)].into(),
            );
        }
        let ed = ed_handle(i);
        // Reuse an existing buffer visiting the same file.
        let existing = ed
            .borrow()
            .buffers
            .iter()
            .find(|b| b.borrow().file.as_deref() == Some(path.as_str()))
            .cloned();
        if let Some(b) = existing {
            crate::editor::show_buffer_in_selected_window(i, &ed, b.clone());
            return Ok(crate::editor::Editor::buffer_value(&b));
        }
        let mut disk_state = crate::buffer::DiskState::Absent;
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => {
                disk_state = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| {
                        m.modified()
                            .ok()
                            .map(|t| crate::buffer::DiskState::Known(t, m.len()))
                    })
                    .unwrap_or(crate::buffer::DiskState::Unknown);
                c
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(i.error(format!("Cannot read file {}: {}", path, e))),
        };
        let base = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let name = {
            let editor = ed.borrow();
            if editor.find_buffer(&base).is_none() {
                base.clone()
            } else {
                let mut n = 2;
                loop {
                    let candidate = format!("{}<{}>", base, n);
                    if editor.find_buffer(&candidate).is_none() {
                        break candidate;
                    }
                    n += 1;
                }
            }
        };
        let buf = std::rc::Rc::new(std::cell::RefCell::new(crate::buffer::Buffer::new(
            &name, &contents,
        )));
        // The buffer's default directory: where the visited file lives.
        buf.borrow_mut().default_directory = std::path::Path::new(&path)
            .parent()
            .map(|p| format!("{}/", p.to_string_lossy()));
        buf.borrow_mut().disk_state = disk_state;
        buf.borrow_mut().file = Some(path);
        ed.borrow_mut().buffers.push(buf.clone());
        crate::editor::show_buffer_in_selected_window(i, &ed, buf.clone());
        // Modes hook in here (M24: auto-mode-alist dispatch via
        // normal-mode, e.g. org-mode for .org files) — before
        // find-file-hook, like GNU's set-auto-mode/find-file-hook order.
        crate::commands::run_normal_mode(i);
        crate::commands::run_hook_by_name(i, "find-file-hook");
        Ok(crate::editor::Editor::buffer_value(&buf))
    });
    // M30 review fix (issue 1): FORCE (optional, default nil) skips the
    // unsaved-changes guard outright — the HONEST way for a caller that
    // isn't the interactive C-x C-c binding (evil's `:q!'; see
    // `evil-ex-quit-force' in evil.el) to force a quit, replacing an
    // earlier "invoke this twice in a row" hack that piggybacked on the
    // `again' check below via `command-execute'. That hack corrupted
    // `last-command' as a SIDE EFFECT of the nested `execute_command'
    // call, which then poisoned the `again' check for ANY unrelated
    // following invocation (a second, un-banged `:q' after the first
    // one warned, or even the real interactive C-x C-c binding) into
    // silently force-quitting — see the evil_tests.rs tests guarding
    // against exactly that. FORCE has no such side effect: it's read
    // once, right here, and never touches `last-command' at all.
    defun(interp, "save-buffers-kill-terminal", 0, Some(1), |i, a| {
        let ed = ed_handle(i);
        let force = opt(a, 0).truthy();
        let modified: Vec<String> = ed
            .borrow()
            .buffers
            .iter()
            .filter(|b| {
                let bb = b.borrow();
                bb.modified && bb.file.is_some()
            })
            .map(|b| b.borrow().name.clone())
            .collect();
        let again = {
            let editor = ed.borrow();
            matches!(&editor.last_command, Value::Sym(id)
                if i.sym_name(*id) == "save-buffers-kill-terminal")
        };
        if !modified.is_empty() && !again && !force {
            let mut editor = ed.borrow_mut();
            editor.echo(format!(
                "Unsaved: {} — C-x C-c again to quit anyway",
                modified.join(", ")
            ));
            return Ok(Value::Nil);
        }
        ed.borrow_mut().quit = true;
        Ok(Value::Nil)
    });

    // Plain-text search.
    // Regexp search over the current buffer. Positions in match data
    // are stored 0-based with offset 1, so match-beginning/end report
    // 1-based buffer positions consistent with (point).
    //
    // M43 period 2: `elisp::regex::Regex` now works over `&str` + byte
    // positions (was `&[char]`). `bb.search_text()` is now that `&str`
    // snapshot directly (`Rc<str>`, was `Rc<Vec<char>>`); every byte
    // position the engine hands back gets converted to a char position
    // via `GapBuffer::byte_to_char` (`caps_bytes_to_buffer_chars` below)
    // before it's stored in `point`/`MatchData.caps` or returned to
    // elisp — `byte_to_char` is anchor-amortized (M43 design §3.4), so a
    // monotonic access pattern like repeated `re-search-forward` calls
    // stays O(delta) per call, not O(buffer size).
    defun(interp, "re-search-forward", 1, Some(3), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let noerror = opt(a, 2).truthy();
        let re = elisp::regex::compile(i, &pat)?;
        let b = cur(i);
        let (hay, start_byte) = {
            let bb = b.borrow();
            let start_byte = bb.text.char_to_byte(bb.point);
            (bb.search_text(), start_byte)
        };
        // M81: `?` propagates a `regexp-too-complex` limit-hit BEFORE
        // the `noerror` check below — `noerror` means "not found isn't
        // an error", not "a budget blow-up isn't an error" (see
        // `regex.rs`'s module doc / `Interp::regexp_too_complex`).
        let found = re
            .search(&hay, start_byte)
            .map_err(|_| i.regexp_too_complex())?;
        match found {
            Some(caps_bytes) => {
                let caps = {
                    let bb = b.borrow();
                    caps_bytes_to_buffer_chars(&bb.text, &caps_bytes)
                };
                let (_, me) = caps[0].unwrap();
                i.match_data = elisp::regex::MatchData {
                    caps,
                    caps_bytes,
                    target: Some(hay),
                    offset: 1,
                };
                b.borrow_mut().point = me;
                Ok(int_pos(me))
            }
            None => {
                if noerror {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Search failed: {:?}", pat)))
                }
            }
        }
    });
    defun(interp, "re-search-backward", 1, Some(3), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let noerror = opt(a, 2).truthy();
        let re = elisp::regex::compile(i, &pat)?;
        let b = cur(i);
        let (hay, point_byte) = {
            let bb = b.borrow();
            let point_byte = bb.text.char_to_byte(bb.point);
            (bb.search_text(), point_byte)
        };
        // Last match that STARTS at or before point (Emacs semantics).
        let mut best: Option<Vec<Option<(usize, usize)>>> = None;
        let mut from = 0usize;
        // M81 R5: ONE `Scratch` shared across this WHOLE loop, via
        // `search_with` (not the public `search`, which would allocate
        // a fresh `Scratch` — and so a fresh budget — on every
        // iteration). `re-search-backward` retries `search` from
        // scratch at every earlier position to find the LAST match
        // before point; without a shared budget, an adversarial pattern
        // that's individually-cheap-but-repeated-many-times across this
        // outer loop would never be caught, only ever one retry's worth
        // at a time — the exact gap reviewer found (`replace_all`'s
        // `:s///g` shape has the same bug, fixed the same way).
        let mut scratch = elisp::regex::Scratch::new(hay.len());
        while from <= point_byte {
            // M81: same `?`-before-`noerror` ordering as
            // `re-search-forward` above — a limit hit is not "not
            // found".
            let found = re
                .search_with(&hay, from, &mut scratch)
                .map_err(|_| i.regexp_too_complex())?;
            match found {
                Some(caps_bytes) => {
                    let (ms, _) = caps_bytes[0].unwrap();
                    if ms > point_byte {
                        break;
                    }
                    // Advance past exactly one CHAR (not byte) of this
                    // match's start, so the next search can find an
                    // overlapping match starting one character later —
                    // the byte-cursor analogue of the pre-M43 char
                    // engine's `from = ms + 1`.
                    from = ms + hay[ms..].chars().next().map_or(1, char::len_utf8);
                    best = Some(caps_bytes);
                }
                None => break,
            }
        }
        match best {
            Some(caps_bytes) => {
                let caps = {
                    let bb = b.borrow();
                    caps_bytes_to_buffer_chars(&bb.text, &caps_bytes)
                };
                let (ms, _) = caps[0].unwrap();
                i.match_data = elisp::regex::MatchData {
                    caps,
                    caps_bytes,
                    target: Some(hay),
                    offset: 1,
                };
                b.borrow_mut().point = ms;
                Ok(int_pos(ms))
            }
            None => {
                if noerror {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Search failed: {:?}", pat)))
                }
            }
        }
    });
    defun(interp, "looking-at", 1, Some(1), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let re = elisp::regex::compile(i, &pat)?;
        let b = cur(i);
        let (hay, point_byte) = {
            let bb = b.borrow();
            let point_byte = bb.text.char_to_byte(bb.point);
            (bb.search_text(), point_byte)
        };
        let found = re
            .match_at(&hay, point_byte)
            .map_err(|_| i.regexp_too_complex())?;
        match found {
            Some(caps_bytes) => {
                let caps = {
                    let bb = b.borrow();
                    caps_bytes_to_buffer_chars(&bb.text, &caps_bytes)
                };
                i.match_data = elisp::regex::MatchData {
                    caps,
                    caps_bytes,
                    target: Some(hay),
                    offset: 1,
                };
                Ok(Value::Sym(i.syms.t))
            }
            None => Ok(Value::Nil),
        }
    });
    defun(interp, "search-forward", 1, Some(3), |i, a| {
        let needle = need_str(i, &a[0])?.to_string();
        let noerror = opt(a, 2).truthy();
        let b = cur(i);
        let found = {
            let bb = b.borrow();
            let hay = bb.search_text();
            let start_byte = bb.text.char_to_byte(bb.point);
            crate::gapbuffer::find_forward(&hay, &needle, start_byte)
                .map(|start_byte| bb.text.byte_to_char(start_byte + needle.len()))
        };
        match found {
            Some(end) => {
                b.borrow_mut().point = end;
                Ok(int_pos(end))
            }
            None => {
                if noerror {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Search failed: {:?}", needle)))
                }
            }
        }
    });
    defun(interp, "search-backward", 1, Some(3), |i, a| {
        let needle = need_str(i, &a[0])?.to_string();
        let noerror = opt(a, 2).truthy();
        let b = cur(i);
        let found = {
            let bb = b.borrow();
            let hay = bb.search_text();
            let point_byte = bb.text.char_to_byte(bb.point);
            crate::gapbuffer::find_backward(&hay, &needle, point_byte)
                .map(|start_byte| bb.text.byte_to_char(start_byte))
        };
        match found {
            Some(start) => {
                b.borrow_mut().point = start;
                Ok(int_pos(start))
            }
            None => {
                if noerror {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Search failed: {:?}", needle)))
                }
            }
        }
    });
}

/// Convert regex byte-position captures to char positions via the
/// buffer's anchor-amortized `GapBuffer::byte_to_char` (M43 design
/// §3.4): buffer searches use the live `GapBuffer` anchor rather than a
/// fresh full-text scan (unlike `elisp::regex::string_match`'s plain-
/// string path, which has no persistent anchor to amortize against and
/// does a single batch pass instead — see `caps_bytes_to_chars` there),
/// so a monotonic access pattern (e.g. `re-search-forward` walking
/// forward through a fontify-style loop) stays O(delta) per call rather
/// than O(buffer size).
fn caps_bytes_to_buffer_chars(
    text: &crate::gapbuffer::GapBuffer,
    caps_bytes: &[Option<(usize, usize)>],
) -> Vec<Option<(usize, usize)>> {
    caps_bytes
        .iter()
        .map(|c| c.map(|(s, e)| (text.byte_to_char(s), text.byte_to_char(e))))
        .collect()
}

/// next-line/previous-line with a sticky goal column.
fn move_lines(interp: &mut Interp, n: i64) {
    let b = cur(interp);
    let mut bb = b.borrow_mut();
    let start = bb.text.line_start(bb.point);
    let col = bb.goal_column.unwrap_or(bb.point - start);
    // Move to target line.
    let mut line_start = start;
    if n >= 0 {
        for _ in 0..n {
            let le = bb.text.line_end(line_start);
            if le >= bb.text.len() {
                break;
            }
            line_start = le + 1;
        }
    } else {
        for _ in 0..(-n) {
            if line_start == 0 {
                break;
            }
            line_start = bb.text.line_start(line_start - 1);
        }
    }
    let le = bb.text.line_end(line_start);
    bb.point = (line_start + col).min(le);
    bb.goal_column = Some(col);
}

/// M61: the single normalization choke point for `find-file-internal`.
/// Guarantees an absolute, `.`/`..`-free path (via `expand_file_name`,
/// which internally does the M18 `//`/`/~` shadowing and `~` expansion
/// first) so that two different spellings of the same file — relative
/// vs. absolute, or `a/b/../c` vs `a/c` — normalize to the same `.file`
/// string and share one buffer instead of silently diverging into two.
fn expand_path(p: &str) -> String {
    crate::builtins::files::expand_file_name(p, None)
}

/// M62 (local)/M75 (remote): shared "have we already warned about
/// exactly this on-disk conflict" check for `save-buffer`'s guard.
/// `observed` is the on-disk state just read back (mtime+size locally,
/// a content digest remotely -- see `Buffer::disk_state`'s field doc for
/// why `save_conflict_ack` binds the OBSERVED state, not just the path).
/// Returns `Ok(())` if this exact `(path, observed)` pair was already
/// acked (the user retrying right after a refusal); otherwise records
/// the ack and returns the refusal error, so callers can just do
/// `if conflict && !force { refuse_save_conflict(i, &b, &path, observed)?; }`.
fn refuse_save_conflict(
    i: &mut Interp,
    b: &std::rc::Rc<std::cell::RefCell<crate::buffer::Buffer>>,
    path: &str,
    observed: crate::buffer::DiskState,
) -> Result<(), elisp::error::Flow> {
    let already_acked = b.borrow().save_conflict_ack == Some((path.to_string(), observed));
    if already_acked {
        return Ok(());
    }
    b.borrow_mut().save_conflict_ack = Some((path.to_string(), observed));
    Err(i.error(format!(
        "{} has changed on disk since it was read — save again to overwrite",
        path
    )))
}

/// M22: open a `/ssh:host:path` — remote directories land in dired
/// (the listing core dispatches), remote files are fetched over ssh;
/// a missing remote file opens an empty buffer (new file), matching
/// the local behavior.
fn find_file_remote(
    i: &mut Interp,
    full: &str,
    rp: &crate::remote::RemotePath,
) -> Result<Value, elisp::error::Flow> {
    if crate::remote::is_dir(rp).map_err(|e| i.error(e))? {
        let dired = i.intern("dired");
        return elisp::eval::apply_function(
            i,
            &Value::Sym(dired),
            vec![Value::string(full)].into(),
        );
    }
    let ed = ed_handle(i);
    let existing = ed
        .borrow()
        .buffers
        .iter()
        .find(|b| b.borrow().file.as_deref() == Some(full))
        .cloned();
    if let Some(b) = existing {
        crate::editor::show_buffer_in_selected_window(i, &ed, b.clone());
        return Ok(crate::editor::Editor::buffer_value(&b));
    }
    let remote_read = crate::remote::read_file(rp).map_err(|e| i.error(e))?;
    // M75: record a baseline the same way find-file-internal's local
    // path does (see `Buffer::disk_state`), so save-buffer can detect an
    // external change to the remote file since it was opened here.
    // `Ok(None)` mirrors the local NotFound branch's `Absent` -- this is
    // a new file, and someone else creating it before our first save is
    // a conflict, same as locally.
    let disk_state = match &remote_read {
        Some(text) => crate::buffer::DiskState::RemoteContent {
            digest: crate::buffer::content_digest(text.as_bytes()),
            len: text.len() as u64,
        },
        None => crate::buffer::DiskState::Absent,
    };
    let contents = remote_read.unwrap_or_default();
    let base = rp.path.rsplit('/').next().unwrap_or(&rp.path).to_string();
    let name = {
        let editor = ed.borrow();
        if editor.find_buffer(&base).is_none() {
            base.clone()
        } else {
            let mut n = 2;
            loop {
                let candidate = format!("{}<{}>", base, n);
                if editor.find_buffer(&candidate).is_none() {
                    break candidate;
                }
                n += 1;
            }
        }
    };
    let buf = std::rc::Rc::new(std::cell::RefCell::new(crate::buffer::Buffer::new(
        &name, &contents,
    )));
    let parent = match rp.path.rfind('/') {
        Some(idx) => &rp.path[..idx + 1],
        None => "",
    };
    buf.borrow_mut().default_directory = Some(crate::remote::format_path(&rp.host, parent));
    buf.borrow_mut().disk_state = disk_state;
    buf.borrow_mut().file = Some(full.to_string());
    ed.borrow_mut().buffers.push(buf.clone());
    crate::editor::show_buffer_in_selected_window(i, &ed, buf.clone());
    crate::commands::run_normal_mode(i);
    crate::commands::run_hook_by_name(i, "find-file-hook");
    Ok(crate::editor::Editor::buffer_value(&buf))
}
