;;; window.el --- display-buffer / pop-to-buffer -*- lexical-binding: t -*-

;; M103: before this file, every "show a buffer" path in this editor
;; (`switch-to-buffer-internal' -> `show_buffer_in_selected_window',
;; editor.rs) unconditionally OVERWRITES the selected window. That is
;; exactly right for `find-file'/`switch-to-buffer' themselves (the user
;; asked to go look at that buffer), but every helper buffer this editor
;; pops up on its own initiative -- `*Help*' (`C-h b'), `*eshell*',
;; `*ielm*', and the shared `*Shell Command Output*'/`*compilation*'/
;; `*search*' buffer (`shell-command--maybe-show') -- used the SAME
;; primitive, which means each one blows away whatever file the user was
;; actually editing. This file adds GNU's `display-buffer'/`pop-to-
;; buffer' (split off a window when there's room, reuse one already
;; showing the buffer, reuse the next window over when there are
;; already several) and repoints those four call sites at them.
;;
;; Deliberate departure from GNU, spelled out here because it is a
;; judgment call, not an oversight (see CLAUDE.md: "keys can be copied
;; verbatim, behavior must be evaluated"): GNU's own callers for these
;; same four kinds of buffer mostly use `display-buffer' itself, which
;; shows the buffer WITHOUT moving selection away from the window the
;; user was in. Every call site converted below uses `pop-to-buffer'
;; instead (display AND select) for two reasons: (1) every helper buffer
;; here carries interactive keys of its own -- `q' to leave
;; (`quit-source-return'), compile's next-error, search's result
;; navigation -- and if selection doesn't move, the user has to `C-x o'
;; into it before any of that works; (2) GNU users who dislike the
;; default can reach for `display-buffer-alist'/`help-window-select';
;; this editor has no such per-buffer configuration surface to defer to.
;;
;; M103 (third fix round, cold-review): the "was this window created
;; for display" flag started life as a buffer-local elisp variable
;; (`window--created-for-display'), matching GNU's naming but not its
;; DATA MODEL -- GNU's own equivalent (`quit-restore') is a WINDOW
;; parameter, not a buffer-local variable, and it turns out that
;; distinction is load-bearing, not cosmetic. A buffer-local flag can't
;; tell two windows showing the SAME buffer apart (a plain `C-x 2'/`C-x
;; 3' split, or a `C-x b' switch into an already-displayed buffer, both
;; produce exactly that), so it was structurally unable to satisfy both
;; "delete the window `display-buffer' created" and "never delete a
;; window I didn't create" in the same case -- two real, reproduced
;; bugs, one per property, and fixing either one by adjusting the
;; condition just moved the failure onto the other property (see
;; PLAN.md's M103 record for both repros). Moving the flag onto
;; `Window' itself (`created_for_display', editor.rs;
;; `window-created-for-display-p'/`set-window-created-for-display',
;; ui.rs) makes the two properties the same statement: a window nobody
;; asked `display-buffer' to create simply never has the flag, no
;; matter what its buffer is doing in some OTHER window.
;;
;; Known gaps (v1, honestly incomplete, not hidden):
;; - No `display-buffer-alist' or any other user-configurable display
;;   rule -- the policy in `display-buffer''s own doc comment (reuse a
;;   window already showing the buffer; else reuse the selected window
;;   itself if it is already a helper window; else split; else reuse
;;   another helper window or the window after selected; else give up
;;   and overwrite) is hard-coded.
;; - No `window-configuration' save/restore. `quit-source-return''s
;;   window-aware branch (simple.el) can only delete the ONE window it
;;   itself created via a split -- it cannot restore an arbitrary prior
;;   layout beyond that.
;; - `set-window-buffer' (ui.rs) keeps `switch-to-buffer-internal''s own
;;   `window_start = 0' behavior, so a buffer's scroll position is not
;;   preserved across being displayed in a different window than before.
;; - `*background-output*' (lib.rs) is still never displayed
;;   automatically; this milestone did not touch it.
;; - `core::idle_tick' (lib.rs) runs each of the eshell/shell-command/
;;   compile/search idle-pump forms through `let _ = interp.eval_source
;;   (...)', silently discarding any elisp error. All four of those
;;   pump paths now indirectly call `pop-to-buffer' (via `shell-
;;   command--maybe-show' or `eshell''s own display call), so a type
;;   error anywhere in THIS file reachable from one of them surfaces as
;;   the feature silently hanging forever (the process never gets
;;   marked finished) rather than a visible error -- exactly what
;;   happened during this milestone's own fix round with a
;;   `get-buffer-create' call given a buffer object instead of a
;;   string. Not fixed here (pre-existing gap, out of scope for M103).
;; - Step 4's `window--next-after-selected' fallback (no other window
;;   is already flagged as a helper) can still overwrite a SECOND real
;;   file window: two windows each showing a different file, a helper
;;   command invoked from the SECOND one picks the first (`next after
;;   selected' wraps to it), overwriting that file's window rather than
;;   the one the command was invoked from. Matches GNU's own behavior
;;   in the equivalent situation (it also picks some other window to
;;   reuse) -- accepted deliberately, not an oversight, and a direct
;;   consequence of having no `display-buffer-alist' to consult
;;   instead.

(defun window--set-difference (a b)
  "Elements of list A not in list B, in A's order. Used to find the
window id a split just created: `split-window-internal' keeps the
ORIGINAL window's id on the side that stays selected (see its own doc
comment in ui.rs), so the new id can't be guessed from a numbering
rule -- it has to be read off a before/after snapshot of `window-list'."
  (let (out)
    (dolist (x a)
      (unless (memq x b) (push x out)))
    (nreverse out)))

(defun window--position (x lst)
  "0-based index of X (compared with `eq') in LST, or nil if absent."
  (let ((i 0) (found nil))
    (dolist (y lst)
      (when (and (not found) (eq x y))
        (setq found i))
      (setq i (1+ i)))
    found))

(defun window--resolve-buffer (buffer-or-name)
  "BUFFER-OR-NAME as a buffer object, creating it if it was a NAME that
doesn't exist yet (`get-buffer-create' only accepts a string --
`display-buffer''s callers pass either a buffer object already in hand
(`shell-command--maybe-show') or a literal name (`describe-bindings',
`eshell', `ielm'), so this has to handle both)."
  (if (bufferp buffer-or-name)
      buffer-or-name
    (get-buffer-create buffer-or-name)))

(defun window--next-after-selected ()
  "The window in `(window-list)' that comes right after the selected
one, wrapping around -- same cyclic order `other-window' walks."
  (let* ((wins (window-list))
         (pos (window--position (selected-window) wins)))
    (nth (mod (1+ (or pos 0)) (length wins)) wins)))

(defun window--is-display-window (win)
  "Non-nil if WIN is currently flagged (`window-created-for-display-p',
ui.rs -- a per-WINDOW flag, see `Window::created_for_display''s own doc
comment in editor.rs for why) as a window `display-buffer' created via
a split. Used by step 4 of `display-buffer' (below) to prefer evicting
ANOTHER helper window over evicting whatever the user is actually
editing."
  (window-created-for-display-p win))

(defun window--other-display-window (exclude)
  "A window other than EXCLUDE that is flagged `window--is-display-
window', or nil if none exist."
  (let (found)
    (dolist (w (window-list))
      (unless (or found (eq w exclude))
        (when (window--is-display-window w)
          (setq found w))))
    found))

(defun display-buffer (buffer-or-name)
  "Show BUFFER-OR-NAME in some window, WITHOUT changing which window is
selected, and return that window's id (or nil if even the last-resort
overwrite somehow didn't happen -- in practice this only returns nil
never; the overwrite fallback below always succeeds).

Order of preference (see this file's header for why there is no
user-configurable alternative):
1. A window already showing this buffer -- reuse it verbatim, no
   other side effect at all.
2. The SELECTED window is itself flagged `window-created-for-display-p'
   (a helper window `display-buffer' created earlier) -- reuse IT. The
   flag is a per-WINDOW property (`Window::created_for_display',
   editor.rs), so reusing the window just works: nothing needs to be
   moved or re-set, unlike the buffer-local design this replaced (M103
   third fix round) which had to transplant a buffer-local flag from
   the outgoing buffer to the incoming one by hand. This is what makes
   `C-h b' followed by `M-!' swap `*Help*' for the shell-output buffer
   IN THE SAME window instead of evicting the file window (M103 fix
   round, cold-review defect: step 4 below picks \"the next window
   after selected\" with no regard for what it holds, and right after
   a split the NEXT window is exactly the user's file window).
3. Exactly one window -- try to split it (top/bottom first, then
   left/right if the window is too short but wide enough), and show
   the buffer in the NEW window. The new window is flagged via
   `set-window-created-for-display' so `quit-source-return' (simple.el)
   knows to delete it again on `q' rather than just switching its
   buffer back.
4. More than one window exists and the selected one is NOT a helper
   window -- prefer reusing ANOTHER helper window if one exists
   (`window--other-display-window'), so two helper buffers displayed
   back to back settle into swapping the same one or two windows
   rather than spreading across the user's other windows one at a
   time; only if no helper window exists elsewhere does this fall back
   to \"the window after selected\" (`window--next-after-selected').
   This step never sets the flag -- the window it picks (whichever
   branch) is not one THIS call created.
5. The window is too small to split either way (falls out of step 3)
   -- fall back to overwriting the selected window
   (`switch-to-buffer-internal'), same as this editor's behavior
   before this file existed. This fallback matters: a display action
   must never silently do nothing just because the frame is small."
  ;; `get-buffer-create', not `get-buffer': every current call site
  ;; (describe-bindings's "*Help*", eshell's "*eshell*", ielm's
  ;; "*ielm*", shell-command--maybe-show's shared output buffer) used to
  ;; go through `switch-to-buffer-internal', which creates the buffer on
  ;; first use like `C-x b' does -- `display-buffer' must keep that or
  ;; every one of them breaks on its very first call. Also normalizes
  ;; BUFFER-OR-NAME to a buffer object up front so `get-buffer-window'/
  ;; `set-window-buffer' below always compare/act on the same object.
  (let* ((buf (window--resolve-buffer buffer-or-name))
         (existing (get-buffer-window buf)))
    (cond
     (existing existing)
     ((window-created-for-display-p (selected-window))
      (let ((sel (selected-window)))
        (set-window-buffer sel buf)
        sel))
     ((> (window-count) 1)
      (let ((target (or (window--other-display-window (selected-window))
                         (window--next-after-selected))))
        (set-window-buffer target buf)
        target))
     (t
      (let ((before (window-list)))
        (cond
         ((or (split-window-internal nil) (split-window-internal t))
          (let ((new-win (car (window--set-difference (window-list) before))))
            (set-window-buffer new-win buf)
            (set-window-created-for-display new-win t)
            new-win))
         (t
          (switch-to-buffer-internal buf)
          (selected-window))))))))

(defun pop-to-buffer (buffer-or-name)
  "Like `display-buffer', but also select the window it ends up in
(see this file's header for why this editor always selects, unlike
GNU's own `pop-to-buffer' callers for the same kinds of buffer)."
  (let ((win (display-buffer buffer-or-name)))
    (when win
      (select-window win))
    win))

;;; --- C-x 4: other-window variants --------------------------------------

(defun window--other-window-for-display ()
  "Make sure there are at least two windows and select one that is NOT
the one the caller started in, splitting if necessary -- the shared
first half of `find-file-other-window'/`switch-to-buffer-other-window'.
Falls back to the CURRENT window (no-op split) if the frame is too
small to split, same last-resort reasoning as `display-buffer''s own
fallback."
  (if (> (window-count) 1)
      (select-window (window--next-after-selected))
    (let ((before (window-list)))
      (cond
       ((split-window-internal nil)
        (select-window (car (window--set-difference (window-list) before))))
       ((split-window-internal t)
        (select-window (car (window--set-difference (window-list) before))))
       (t nil)))))

(defun find-file-other-window (filename)
  "Like `find-file', but in another window (`C-x 4 f')."
  (interactive "FFind file: ")
  (window--other-window-for-display)
  (find-file filename))

(defun switch-to-buffer-other-window (buffer-or-name)
  "Like `switch-to-buffer', but in another window (`C-x 4 b')."
  (interactive "BSwitch to buffer: ")
  (window--other-window-for-display)
  (switch-to-buffer buffer-or-name))

(global-set-key "C-x 4 f" 'find-file-other-window)
(global-set-key "C-x 4 b" 'switch-to-buffer-other-window)
