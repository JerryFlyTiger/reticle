;;; simple.el --- basic editing commands and default keymap -*- lexical-binding: t -*-

;; Editing commands are defined here in elisp on top of the Rust
;; primitives, exactly the layering real Emacs uses.

(defmacro save-excursion (&rest body)
  `(save-excursion-internal (lambda () ,@body)))

(defmacro with-current-buffer (buffer &rest body)
  `(with-current-buffer-internal ,buffer (lambda () ,@body)))

(defmacro setq-local (var val)
  `(set (make-local-variable ',var) ,val))

;; M47: sugar over the callback-style (CPS) minibuffer readers
;; (`read-string'/`completing-read'/`y-or-n-p', all defined on the Rust
;; side in builtins/ui.rs -- see that file's doc comments for why they
;; take a CALLBACK instead of GNU Emacs's blocking return value). Each
;; macro just expands to the builtin call plus `(lambda (VAR) BODY...)',
;; relying on `lexical-binding: t' (this file's own `-*-' header) so the
;; lambda closes over whatever BODY references from its surrounding
;; scope -- confirmed against this editor's own interpreter default
;; (`Interp::lexical_binding' is `true'), not assumed.
(defmacro with-read-string (spec &rest body)
  (let ((var (nth 0 spec))
        (prompt (nth 1 spec))
        (initial (nth 2 spec)))
    `(read-string ,prompt (lambda (,var) ,@body) ,initial)))

(defmacro with-completing-read (spec &rest body)
  (let ((var (nth 0 spec))
        (prompt (nth 1 spec))
        (collection (nth 2 spec))
        (require-match (nth 3 spec))
        (initial (nth 4 spec)))
    `(completing-read ,prompt ,collection (lambda (,var) ,@body)
                       ,require-match ,initial)))

(defmacro with-y-or-n (spec &rest body)
  (let ((var (nth 0 spec))
        (prompt (nth 1 spec)))
    `(y-or-n-p ,prompt (lambda (,var) ,@body))))

(defun y-or-n-p (prompt callback)
  "Ask PROMPT, answer y/n via one keystroke, and call CALLBACK with t
or nil. There is no blocking `y-or-n-p' in this editor (see
`read-from-minibuffer's own note in builtins/ui.rs) -- this is the
callback-style equivalent, built the same way `dired--confirm-delete'
already built its own one-off version: on top of `capture-next-key'.
`capture-next-key' itself cancels silently on C-g without calling back
in here at all, so CALLBACK only ever sees t or nil, never an aborted
read. Any key other than y/n is ignored and re-captured -- the prompt
stays up until the user actually answers."
  (message "%s(y or n) " prompt)
  (capture-next-key
   (lambda (k)
     (cond
      ((eq k ?y) (funcall callback t))
      ((eq k ?n) (funcall callback nil))
      (t (y-or-n-p prompt callback))))))

(defun switch-to-buffer (name)
  (interactive "BSwitch to buffer: ")
  (switch-to-buffer-internal name))

(defun find-file (filename)
  (interactive "FFind file: ")
  (find-file-internal filename))

(defun kill-buffer-command (name)
  (interactive "BKill buffer: ")
  (kill-buffer name))

(defun newline ()
  (interactive)
  (insert "\n"))

(defun delete-backward-char (&optional n)
  (interactive "p")
  (delete-char (- (or n 1))))

(defun set-mark-command ()
  (interactive)
  (set-mark (point))
  (message "Mark set"))

(defun exchange-point-and-mark ()
  (interactive)
  (let ((m (mark)))
    (unless m (error "No mark set in this buffer"))
    (set-mark (point))
    (goto-char m)))

(defun beginning-of-buffer ()
  (interactive)
  (goto-char (point-min)))

(defun end-of-buffer ()
  (interactive)
  (goto-char (point-max)))

(defun kill-region (start end)
  (interactive "r")
  (kill-region-internal start end))

(defun kill-ring-save (start end)
  (interactive "r")
  (kill-ring-save-internal start end)
  (message "Copied"))

(defun undo ()
  (interactive)
  (if (undo-internal)
      (message "Undo!")
    (message "No further undo information")))

(defun execute-extended-command (command)
  (interactive "CM-x ")
  (command-execute command))

(defun eval-expression (form)
  (interactive "xEval: ")
  (message "%S" (eval form t)))

(defun goto-line (n)
  (interactive "nGoto line: ")
  (goto-char (point-min))
  (forward-line (1- n)))

(defun split-window--report (result)
  "Shared tail for `split-window-below'/`split-window-right': echo the
message matching RESULT, the raw return of `split-window-internal'
(builtins/ui.rs), which distinguishes two different failure causes
that both used to collapse into one misleading \"Window too small to
split\" message (M102 fix round, cold-review): `nil' means the window
itself has no room for any split; the symbol `bad-size' means the
window is big enough but the given SIZE argument doesn't fit
`[WINDOW_MIN_HEIGHT/WIDTH, avail - WINDOW_MIN_HEIGHT/WIDTH]'. `t'
means success -- nothing to echo."
  (cond
   ((eq result 'bad-size) (message "Invalid window size"))
   ((not result) (message "Window too small to split"))))

(defun split-window-below (&optional size)
  "Split the selected window into two, one above the other. SIZE, if
given, is the number of lines given to the upper (original) window;
otherwise the split is even. Does nothing but echo a message if the
split can't happen (M102): either the selected window itself is too
small, or SIZE doesn't leave room for both windows to keep the
minimum size.

This editor has no `universal-argument'/`C-u' yet (see commands.rs:
the `p'/`P' interactive codes always evaluate to 1/nil regardless of
any prefix keys typed before a command), so SIZE can only be supplied
by calling this function from elisp directly, e.g. `(split-window-below
10)' -- `C-x 2' always splits evenly."
  (interactive "P")
  (split-window--report (split-window-internal nil size)))

(defun split-window-right (&optional size)
  "Split the selected window into two, side by side. SIZE, if given, is
the number of columns given to the left (original) window; otherwise
the split is even. Does nothing but echo a message if the split can't
happen (M102): either the selected window itself is too small, or
SIZE doesn't leave room for both windows to keep the minimum size.

This editor has no `universal-argument'/`C-u' yet (see commands.rs:
the `p'/`P' interactive codes always evaluate to 1/nil regardless of
any prefix keys typed before a command), so SIZE can only be supplied
by calling this function from elisp directly, e.g. `(split-window-right
10)' -- `C-x 3' always splits evenly."
  (interactive "P")
  (split-window--report (split-window-internal t size)))

;; --- M102: window resizing ---

(defun enlarge-window (&optional n)
  "Make the selected window N lines taller (shrinking a neighboring
window by the same amount). N defaults to 1; a negative N shrinks.

This editor has no `universal-argument'/`C-u' yet (see commands.rs:
the `p'/`P' interactive codes always evaluate to 1/nil, regardless of
any prefix keys typed before a command), so a single `C-x ^' keypress
always moves by exactly one line -- a larger N can only be supplied by
calling this function from elisp directly, e.g. `(enlarge-window 5)'."
  (interactive "p")
  (unless (window-resize-selected (or n 1) nil)
    (message "Cannot resize a single window")))

(defun shrink-window (&optional n)
  "Make the selected window N lines shorter (growing a neighboring
window by the same amount). N defaults to 1."
  (interactive "p")
  (unless (window-resize-selected (- (or n 1)) nil)
    (message "Cannot resize a single window")))

(defun enlarge-window-horizontally (&optional n)
  "Make the selected window N columns wider (shrinking a neighboring
window by the same amount). N defaults to 1; a negative N shrinks."
  (interactive "p")
  (unless (window-resize-selected (or n 1) t)
    (message "Cannot resize a single window")))

(defun shrink-window-horizontally (&optional n)
  "Make the selected window N columns narrower (growing a neighboring
window by the same amount). N defaults to 1."
  (interactive "p")
  (unless (window-resize-selected (- (or n 1)) t)
    (message "Cannot resize a single window")))

(defun balance-windows ()
  "Make all windows in the current layout the same size (M102)."
  (interactive)
  (window-balance))

(defun isearch-forward ()
  (interactive)
  (isearch-start t))

(defun isearch-backward ()
  (interactive)
  (isearch-start nil))

;; --- M20: evaluating elisp from a buffer ---

(defun backward-sexp (&optional n)
  (interactive "p")
  (forward-sexp (- (or n 1))))

(defun eval-last-sexp ()
  "Evaluate the sexp before point; echo the result."
  (interactive)
  (let* ((end (point))
         (start (save-excursion (backward-sexp) (point)))
         (form (car (read-from-string (buffer-substring start end)))))
    (message "%s" (prin1-to-string (eval form t)))))

(defun eval-defun ()
  "Evaluate the top-level form around or before point.
Uses the GNU beginning-of-defun heuristic: the nearest previous
line that starts with an open parenthesis."
  (interactive)
  (save-excursion
    (end-of-line)
    (if (re-search-backward "^(" nil t)
        (let ((start (point)))
          (forward-sexp)
          (let ((form (car (read-from-string
                            (buffer-substring start (point))))))
            (message "%s" (prin1-to-string (eval form t)))))
      (error "No defun found before point"))))

(defun eval-buffer ()
  "Evaluate every form in the current buffer."
  (interactive)
  (let ((src (buffer-string))
        (idx 0)
        (done nil))
    (while (not done)
      (condition-case nil
          (let ((r (read-from-string src idx)))
            (eval (car r) t)
            (setq idx (cdr r)))
        (end-of-file (setq done t))))
    (message "eval-buffer done")))

(defun eval-region (start end)
  "Evaluate every form between START and END."
  (interactive "r")
  (let ((src (buffer-substring start end))
        (idx 0)
        (done nil))
    (while (not done)
      (condition-case nil
          (let ((r (read-from-string src idx)))
            (eval (car r) t)
            (setq idx (cdr r)))
        (end-of-file (setq done t))))
    (message "eval-region done")))

(defun read-only-mode ()
  "Toggle whether the current buffer refuses edits."
  (interactive)
  (if (buffer-read-only-p)
      (progn (set-buffer-read-only nil) (message "Buffer is now writable"))
    (set-buffer-read-only t)
    (message "Buffer is now read-only")))

(defun describe-function (name)
  "Show the docstring of the function called NAME in the echo area."
  (interactive "sDescribe function: ")
  (let* ((sym (intern name))
         (doc (documentation sym)))
    (cond
      ((not (fboundp sym)) (message "%s is not a function" name))
      (doc (message "%s: %s" name (help--first-line doc)))
      (t (message "%s is a function (not documented)" name)))))

;; --- M67: describe-key / describe-bindings ---
;;
;; Both read what a key ACTUALLY does through `lookup-key' (Rust side,
;; commands.rs `lookup_layered'/`keymap_layers') -- the exact three-layer
;; decision `dispatch_key' itself makes for a real keystroke -- rather
;; than re-deriving that decision here in elisp, so this report can never
;; drift from real dispatch.
;;
;; v1 does not include (documented, not bugs):
;; - `where-is' (the reverse query: "what key(s) run this command").
;; - Docstrings for Rust-side builtins: `function-documentation' is only
;;   ever populated by elisp's own `defun' (see eval.rs), so any binding
;;   to a pure Rust command (there is no such binding in this editor's
;;   default keymaps, but a user's init.el could make one) reports
;;   "(not documented)" even though the command exists and works.
;; - Diagnosing minibuffer-local keys: keystrokes typed while the
;;   minibuffer is active go through `minibuffer_key' (commands.rs),
;;   never `dispatch_key', so `describe-key' run from inside a minibuffer
;;   read would report on the buffer BEHIND it, not the minibuffer's own
;;   bindings -- there is no minibuffer-local keymap to report on either.
;; - (M103, was here, now fixed) `*Help*' used to REPLACE the current
;;   window rather than splitting to show both. `describe-bindings'
;;   below now goes through `pop-to-buffer' (window.el), which splits
;;   (or reuses an existing window) instead of overwriting -- see
;;   window.el's own header for the full design and its one remaining
;;   gap (no user-configurable display policy).
;; - A single "Major mode / Minor mode" split in `describe-bindings':
;;   every minor-mode-style keymap installed via `local-set-key' is
;;   flattened into the SAME buffer-local `local' keymap as the major
;;   mode's own bindings (`use-local-map' only ever holds one keymap
;;   value) -- the provenance needed to separate them again doesn't exist
;;   at the data-structure level, so `describe-bindings' only ever
;;   reports the three dispatch layers (emulation/local/global), not a
;;   finer split within `local'.
;;
;; `*Help*''s `q' binding depends on `help-mode' being listed in
;; evil.el's `evil-emacs-state-modes': with evil-mode on, its
;; normal-state keymap lives in `emulation-keymap', which outranks the
;; local `q' -> `help-quit' binding this file installs outright (no
;; fallthrough once a higher layer claims a key -- see
;; `lookup_layered' in commands.rs). Without that entry, evil's own
;; normal-state `q' (`evil-record-macro') intercepts the key first and
;; `help-quit' is never reached at all (a real bug caught via
;; `dev/tui-drive.py' against a release build, not a hypothetical).

;; Not redundant with dired.el's own `(defvar inhibit-read-only nil)`:
;; this file loads BEFORE dired.el (see `init_editor` in lib.rs), and
;; `describe-bindings' below must not depend on load order to make its
;; own `let'-bound `inhibit-read-only' dynamically visible to the
;; buffer-write-check builtins (`builtins/mod.rs`) that consult it by
;; name. `defvar' is idempotent (eval.rs `eval_defvar`: re-running it
;; never resets an already-bound value), so declaring it again here is
;; harmless regardless of which file runs first.
(defvar inhibit-read-only nil)

(defun help--first-line (s)
  "The first line of string S, up to (not including) the first
newline -- GNU Emacs's own convention for summarizing a possibly
multi-line docstring in a one-line context (here: the echo area, which
`redisplay.rs' truncates hard at the frame width and does NOT wrap or
treat an embedded newline specially -- see this file's header note)."
  (let ((nl (string-match "\n" s)))
    (if nl (substring s 0 nl) s)))

(defun describe-key--report (desc binding)
  "Echo what running key-description DESC would do, given its BINDING
(as `lookup-key' returned it)."
  (if (not (symbolp binding))
      (message "%s runs an anonymous command" desc)
    (let ((doc (documentation binding)))
      (if doc
          (message "%s runs %s: %s" desc binding (help--first-line doc))
        (message "%s runs %s (not documented)" desc binding)))))

(defun describe-key--read (keys)
  "Capture one more key, append it to KEYS, and either report (command/
undefined) or re-prompt for the next key of a still-open prefix. Mirrors
`y-or-n-p''s own capture-next-key recursion (see its doc comment) --
this is the same CPS shape, one key at a time. C-g needs no special
case here: `capture-next-key' already swallows it without ever calling
back into this lambda."
  (capture-next-key
   (lambda (k)
     (let* ((keys (append keys (list k)))
            (desc (key-description keys))
            (result (lookup-key keys)))
       (cond
        ((null result) (message "%s is undefined" desc))
        ((eq (car result) 'prefix)
         (message "%s-" desc)
         (describe-key--read keys))
        (t (describe-key--report desc (nth 1 result))))))))

(defun describe-key ()
  "Read a key sequence and report, in the echo area, what running it
would actually do (command name + first docstring line, or an
undefined/anonymous/undocumented note) -- resolved through the same
three-layer keymap decision a real keystroke uses (`lookup-key')."
  (interactive)
  (message "Describe key: ")
  (describe-key--read nil))

;; M68: shared "return to where I came from" mechanism, generalized from
;; M67's `*Help*'-only `help--source-buffer'/`help-quit' so `dired' (and
;; anything else with a `q'-style leave command) doesn't grow its own
;; near-duplicate copy with subtly different behavior.
(defvar quit-source nil
  "Buffer-local: the buffer a `q'-style leave-this-buffer command should
return to. Judged alive via `memq' against `(buffer-list)', NOT by
name/`get-buffer' -- `rename-buffer' (buffers.rs) only ever changes
`.name' and notifies no one, so a name-based check would silently
follow the buffer to whatever else later takes that name (same
argument as `lsp--buffer-live-p's own doc comment in lsp.el, which
already does the same `memq' check for the same reason). Set via
`quit-source-of-current' (called BEFORE switching into the new buffer)
and `setq-local' (called AFTER).")

(defun quit-source-of-current ()
  "The value a buffer about to be switched into (a fresh `dired' buffer,
`*Help*', ...) should record as its OWN `quit-source': the CURRENT
buffer's `quit-source' if it already has one, else the current buffer
itself. Chaining through this -- rather than always capturing the buffer
being switched away from -- is what makes a `dired' -> `dired' -> ...
or `describe-bindings' -> `describe-bindings' chain return to the
ORIGINAL starting point instead of just the last hop."
  (or quit-source (current-buffer)))

(defun quit-source-return ()
  "Leave the current buffer, restoring whatever was on screen before it
was shown.

M103: window-aware. If the SELECTED window itself was created by
`display-buffer' splitting a window to make room for it
(`window-created-for-display-p', ui.rs -- a per-WINDOW flag, see
`Window::created_for_display''s own doc comment in editor.rs), `q'
deletes that window (`delete-window') instead of switching a buffer,
restoring the pre-split layout. Otherwise (the buffer was shown by
reusing an already-existing window, or there is only one window left
so there is nothing to delete): the pre-M103 behavior -- switch to the
current buffer's `quit-source', or `*scratch*' if that buffer no
longer exists (killed, or never set).

Two real bugs, and a model correction, are folded into this simple
shape (M103's third fix round; see PLAN.md's M103 record for the full
history): this used to check a BUFFER-LOCAL flag
(`window--created-for-display') instead of a per-window one, which
could not tell apart two windows showing the same buffer -- exactly
the situation a plain `C-x 2'/`C-x 3' split, or a `C-x b' switch into
an already-displayed buffer, produces. That version had to choose
between two failure modes depending on how its condition was written:
either it deleted whichever window the flag happened to name even when
the user was looking at a DIFFERENT window showing the same buffer
(first repro: `C-h b' splits off B for `*Help*'; `C-x 2' from B clones
it into C; `C-x o' to C; `q' must close C, not jump to B and delete
that), or it deleted a window the mechanism never created at all
(second repro: `C-h b' splits off B for `*Help*'; `C-x o' back to the
ORIGINAL window A; `C-x b' switches A's buffer to `*Help*' directly,
bypassing `display-buffer' entirely; `q' with A selected must not
delete A). No condition on a single buffer-local flag can satisfy both
repros at once, because the flag cannot distinguish which of two
windows sharing a buffer is the one THIS mechanism is responsible for.
Moving the flag onto `Window' itself removes the ambiguity: neither
C (a plain split) nor A (a plain buffer switch) is EVER flagged,
regardless of what buffer they show, so \"delete the window I created\"
and \"delete the window being looked at\" collapse into the same
window whenever there is one to delete, and into NO deletion
(fall through to the plain buffer-switch below) whenever there isn't
-- which is also what GNU does for a window with no `quit-restore'
parameter."
  (interactive)
  (if (and (window-created-for-display-p (selected-window))
           (> (window-count) 1))
      (delete-window)
    (let ((target (if (and quit-source (memq quit-source (buffer-list)))
                       quit-source
                     "*scratch*")))
      (switch-to-buffer-internal target))))

(defun help-quit ()
  "Leave `*Help*', returning to wherever `describe-bindings' was called
from (see `quit-source-return'). Kept as its own command name (rather
than binding `q' straight to `quit-source-return') so `*Help*''s local
keymap keeps reporting a `*Help*'-specific command."
  (interactive)
  (quit-source-return))

(defun help--shadowed-p (desc seen)
  "Non-nil if key-description DESC is unreachable because a
higher-priority layer already claims it (SEEN: the list of KEYDESCs
collected from those layers so far) -- either DESC itself, or a
PROPER PREFIX of DESC. The prefix case matters because `all-key-
bindings' only ever lists LEAF commands: if a higher layer binds
\"C-c\" straight to a command, `lookup_in' resolves the single key
\"C-c\" to `Lookup::Command' and dispatch stops right there (commands.rs)
-- a lower layer's longer \"C-c a\" is never reached even though its own
KEYDESC never equals \"C-c\". The prefix check is bounded by a trailing
space so \"C-c\" only shadows \"C-c ...\", never an unrelated key that
merely starts with the same characters, like \"C-cx\" (not a real key
description this editor ever produces, but the boundary check is cheap
insurance regardless)."
  (or (member desc seen)
      (let ((shadowed nil))
        (dolist (s seen)
          (when (string-prefix-p (concat s " ") desc)
            (setq shadowed t)))
        shadowed)))

(defun help--pad (s width)
  "S followed by enough spaces to reach WIDTH columns (no truncation
when S is already at or past WIDTH) -- `format''s own %s has no width
specifier in this editor (see `format_impl' in misc.rs), so alignment
is built by hand here instead."
  (let ((n (- width (length s))))
    (if (> n 0) (concat s (make-string n ?\s)) s)))

(defun help--format-layer (title bindings seen)
  "Format one `describe-bindings' section: TITLE line, then one
\"KEYDESC  COMMAND\" line per (KEYDESC . COMMAND) pair in BINDINGS
(already sorted by KEYDESC), marking entries whose KEYDESC is in SEEN
as `(shadowed)'. Returns the section's text as one string."
  (concat
   title "\n"
   (mapconcat
    (lambda (pair)
      (let ((desc (car pair))
            (cmd (cdr pair)))
        (format "  %s%s%s" (help--pad desc 16) cmd
                (if (help--shadowed-p desc seen) "  (shadowed)" ""))))
    bindings "\n")
   "\n\n"))

(defun describe-bindings ()
  "Show every key binding currently reachable in the current buffer, in
a new `*Help*' buffer -- one section per dispatch layer (emulation,
local, global, matching `lookup-key''s own priority order), each noting
which layer(s) below it that layer shadows, and each entry marked
`(shadowed)' when a higher-priority layer already claims the same key.
See this file's M67 header note for what this does NOT cover."
  (interactive)
  (let* (;; M68: `quit-source-of-current' subsumes the M67 review fix
         ;; this used to hand-roll here -- already inside `*Help*' (e.g.
         ;; a second `C-h b'), it returns the ORIGINAL source instead of
         ;; `*Help*' itself, which is exactly what kept `q' from
         ;; switching to `*Help*' and going nowhere.
         (source (quit-source-of-current))
         (all (all-key-bindings))
         (emulation nil)
         (local nil)
         (global nil))
    (dolist (entry all)
      (let ((layer (nth 0 entry))
            (desc (nth 1 entry))
            (binding (nth 2 entry)))
        (cond
         ((eq layer 'emulation) (push (cons desc binding) emulation))
         ((eq layer 'local) (push (cons desc binding) local))
         (t (push (cons desc binding) global)))))
    (let* ((by-desc (lambda (a b) (string< (car a) (car b))))
           (emulation (sort emulation by-desc))
           (local (sort local by-desc))
           (global (sort global by-desc))
           (emulation-descs (mapcar 'car emulation))
           (local-descs (mapcar 'car local))
           (text
            (concat
             (help--format-layer
              "Emulation keymap (shadows local and global):"
              emulation nil)
             (help--format-layer
              "Local keymap (shadows global):"
              local emulation-descs)
             (help--format-layer
              "Global keymap:"
              global (append emulation-descs local-descs)))))
      ;; M103: `pop-to-buffer', not `switch-to-buffer-internal' -- splits
      ;; a window for `*Help*' (or reuses one already showing it/the
      ;; window after selected) instead of overwriting whatever the user
      ;; was editing. See window.el's header for why this editor selects
      ;; the new window (unlike GNU's own `display-buffer'-based callers
      ;; for `*Help*').
      (pop-to-buffer "*Help*")
      (setq-local quit-source source)
      (major-mode-internal-set 'help-mode)
      (let ((map (make-sparse-keymap)))
        (define-key map "q" 'help-quit)
        ;; M130: `gg' is deliberately not bound anywhere in this
        ;; milestone -- see dired.el's header note for the mechanical
        ;; reason (`g' collision risk in `Keymap::define-sequence').
        (define-key map "j" 'next-line)
        (define-key map "k" 'previous-line)
        (define-key map "G" 'end-of-buffer)
        (use-local-map map))
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert text)
        (goto-char (point-min)))
      ;; M71: `insert' above unconditionally dirties the buffer, but
      ;; *Help* is entirely regenerated from `all-key-bindings' every
      ;; time -- there is nothing a user could edit or lose here -- so
      ;; `*' is never a meaningful signal on it. Clear it before making
      ;; the buffer read-only (matches the order the two calls appear
      ;; in above: content settles, then both "don't dirty" and "don't
      ;; edit" are locked in together).
      (set-buffer-modified-p nil)
      (set-buffer-read-only t))))

(global-set-key "C-h k" 'describe-key)
(global-set-key "C-h b" 'describe-bindings)
(global-set-key "C-h f" 'describe-function)

;; --- M16/M32: current-line highlighting ---

(defvar hl-line-mode t
  "Non-nil tints the selected window's current line — wherever nothing
else already set a background (region/overlay colors still win; see
redisplay.rs's current-line-highlight block, keyed off the `hl-line'
face). A plain global toggle, not buffer-local. On by default since
M32 (M16 originally shipped it off); add `(setq hl-line-mode nil)' to
init.el to turn it back off.")

;; --- M113: matching-bracket highlighting (GNU's show-paren-mode) ---

(defvar show-paren-mode t
  "Non-nil highlights the bracket pair adjacent to point (`show-paren-
match' face), matching GNU Emacs's default adjacency rule: triggers
when the character immediately AFTER point is an opener, or the one
immediately BEFORE point is a closer — never when point sits just
inside a bracket instead of facing it (verified against real
`emacs -Q --batch`, see the M113 report). An unmatched bracket
highlights nothing (GNU instead shows it alone in a mismatch face;
this project has no such face, so this is a deliberate divergence, not
an oversight — see redisplay.rs's paren-highlight block).

A plain global toggle, not buffer-local, same convention as `hl-line-
mode' just above — this depends on the background highlight engine's
cached parse (highlight.rs's `Engine::matching_pair'), so it only ever
does anything in a buffer that engine is running for, same as
`rainbow-delimiters-mode', but the TOGGLE itself is global because
matching brackets is exactly as useful in every language as
current-line highlighting is, not a per-major-mode decorative choice.
On by default: the editor comparisons this milestone exists to answer
(VS Code, JetBrains) both ship it on. Add `(setq show-paren-mode nil)'
to init.el to turn it back off.")

;; --- M116: trailing whitespace, fill-column ruler (GNU's own names) ---

(defvar show-trailing-whitespace nil
  "Buffer-local (GNU's own name/semantics). Non-nil highlights whitespace
at the end of a line (space/tab immediately before the newline or
buffer end) with the `trailing-whitespace' face — see redisplay.rs's
trailing-whitespace block. Lives in the grid rather than the GUI paint
pass so the TUI gets it too: trailing whitespace is exactly as invisible
in a terminal as in a graphical window, and the grid is the one thing
both frontends already read.

GNU defaults this off. This project defaults it ON for `prog-mode'
buffers instead (`prog-mode-hook', modes.el) — same buffer-local-
default-on-for-prog-buffers convention as `rainbow-delimiters-mode' and
`display-line-numbers'. The divergence: this project's target user is
an RTL engineer whose linter (verible) rejects trailing whitespace
outright, so showing it by default is the useful behavior here, not a
cosmetic default GNU just happens to ship differently.

Exclusion (verified against the real `emacs -Q --batch'/info manual
installed at `/opt/homebrew/bin/emacs', not assumed): GNU's manual
states this feature \"does not apply when point is at the end of the
line containing the whitespace\" — a narrower rule than \"point is
anywhere on that line\". This project follows the exact rule observed:
only the specific line where point sits AT its end (`point' equals that
line's end position, not merely equal to that line's number) is
exempted, so trailing whitespace elsewhere on point's own line (e.g.
point moved to column 0 with `C-a') still highlights. The reason GNU
gives, and this project's behavior shares: skipping this narrow case
avoids the highlight flashing on and off while typing new text at the
end of a line.")

(defvar fill-column 100
  "Buffer-local (GNU's own name/semantics). The column
`display-fill-column-indicator-mode' draws its ruler at — read via the
same buffer-local-aware path as every other per-buffer redisplay
toggle (`fill-column-indicator' block, frontend-gui/src/lib.rs).

GNU defaults this to 70, a prose line-length convention going back to
`auto-fill-mode'. This project defaults it to 100 instead: the target
user is an RTL engineer, and 100 is `verible''s own default
`--line_length' limit (see `demo/tools/lint_rtl.sh') — the number that
actually means something for Verilog/SystemVerilog as this project's
users write it, not GNU's prose-oriented number.")

(defvar display-fill-column-indicator-mode nil
  "Buffer-local (GNU's own name). Non-nil draws a 1-device-px vertical
rule at column `fill-column' (`fill-column-indicator' face). GUI only —
follows the same column-to-pixel code as the indent guides
(`indent_guide_columns' in frontend-gui/src/lib.rs, itself GUI-only for
the same reason: the TUI has no graphical-rule primitive to draw one
with, only character cells).

Same buffer-local-default-on-for-prog-buffers convention as
`rainbow-delimiters-mode', turned on by `prog-mode-hook' (modes.el).
GNU defaults this off; this project defaults it ON for `prog-mode'
buffers instead — a ruler nobody discovers by reading `M-x' menus is a
ruler nobody uses, and the whole point of adding this alongside
trailing-whitespace and the ligature toggle in the same milestone is
that a feature which exists but cannot be seen might as well not
exist.")

;; --- M118: sticky scope header + scope breadcrumb ---

(defvar scope-header t
  "Global. Non-nil pins the source line of each construct enclosing
point at the top of its window, once that construct's own opening line
has scrolled off screen — VS Code's \"sticky scroll\". Up to
`scope-header-max-lines' rows, outermost first, innermost kept when the
enclosing chain is deeper than that. Fed by `highlight.rs''s
`Engine::scope_chain', itself built from `scope.rs''s per-language
node-kind tables (Verilog first — see that file's header for exactly
which construct kinds count as a scope per language).

Painted as an overlay on top of the window's own top rows, after the
normal text loop — it HIDES the buffer lines underneath rather than
reserving space for them, and carries no syntax colouring of its own
(a single uniform `scope-header' face). Both are deliberate v1 limits,
not oversights: reserving real row budget for a non-buffer row would
require teaching the scroll/recenter machinery a concept it does not
have anywhere else in this codebase (see PLAN.md's M87 stage 3 record
for why that was rejected as a permanent version of an already-
occasional gap).")

(defvar scope-header-max-lines 3
  "Global. Cap on how many rows `scope-header' pins at once — see that
variable's doc. When the chain enclosing point is deeper than this,
the INNERMOST constructs are kept (the immediately enclosing one is
the useful one), not the outermost.")

(defvar scope-breadcrumb t
  "Global. Non-nil shows the full chain of constructs enclosing point
as a mode-line segment, `[outer > ... > inner]' — e.g.
`[soc_top > always_ff > case]'. Lowest priority of every mode-line
segment (`compose_mode_line', redisplay.rs): dropped before the
diagnostic count, the mode name, the LSP indicator, and the directory
segment, all of which are checked (and kept) ahead of it once the
window gets narrow.

Deliberate divergence from GNU's `which-function-mode' (verified
against real `emacs -Q --batch', not assumed): GNU shows only the
INNERMOST name as `[name]', and shows the literal string \"n/a\" when
it cannot tell. This project shows the FULL path instead, and shows
NOTHING AT ALL rather than \"n/a\" when the chain is empty — the
segment is already dropped under width pressure, so an \"unknown\"
placeholder would be noise in exactly the buffers (plain text, no
grammar) where it is permanently unknown, not an occasional state
worth flagging.")

;; --- M28: evil-mode integration foundation ---

;; A keymap (or nil) consulted ahead of the local and global keymaps on
;; every key, buffer-local like Emacs's `emulation-mode-map-alists` — the
;; hook a package like evil-mode binds its states' keys through without
;; having to fight the major mode for priority. nil (the default) means
;; no emulation layer is active and dispatch is unchanged.
(defvar emulation-keymap nil)

;; M34: buffer-local; when non-nil, a single unbound printable character
;; that would otherwise fall through all three keymaps (emulation/local/
;; global) to the ordinary self-insert fallback is instead left
;; undefined — echoing same as any other undefined key, inserting
;; nothing. Consulted by `dispatch_key` (commands.rs) at that one
;; fallback site only; a real binding in any of the three keymaps (`C-x
;; b`, `M-x`, evil's own `h`/`j`/`k`/`l`, ...) is completely unaffected
;; either way. nil (the default) means every existing buffer that never
;; sets this keeps today's plain-self-insert behavior.
;;
;; This is what lets evil-mode's normal/visual/operator-pending states
;; swallow a key that isn't one of their own ASCII vim bindings — most
;; importantly a CJK (or any other non-ASCII, non-enumerable) character,
;; which no fixed-size vim keymap could ever list one-by-one — instead of
;; silently self-inserting it into the buffer. See evil.el's
;; `evil--set-state`, the only place this is set for evil's own states.
(defvar inhibit-self-insert nil)

;; M85: run whenever the minibuffer's own input text changes (any key
;; that edits it -- self-insert, DEL, C-k, ... -- not just RET/submit),
;; each hook function called with the NEW input string as its single
;; argument (`run_hook_by_name_with_arg', commands.rs). This is a
;; GENERAL-PURPOSE notification, not tied to any one prompt or command:
;; before it existed, no elisp code could ever learn a minibuffer's
;; input changed at all except by way of the M21 completion-panel
;; machinery (`crate::panel::refresh'), which only fires for
;; `completing-read'-family prompts that HAVE a candidate panel -- a
;; plain `read-string' prompt with no candidates (e.g. `*search*''s
;; live result filter, search.el) never tripped anything. First
;; consumer: `search--filter-input-changed' (search.el).
(defvar minibuffer-input-changed-hook nil)

;; A buffer-local string shown ahead of the buffer name in the modeline,
;; e.g. evil-mode's state tag ("<N> "). nil/unset shows nothing.
(defvar mode-line-prefix nil)

;; A buffer-local string shown in the echo area whenever it would
;; otherwise be empty (the LOWEST of redisplay.rs's three echo-row
;; layers: the minibuffer, then `editor.echo' — a one-shot message —,
;; then this), e.g. evil-mode's persistent "-- INSERT --"/"-- VISUAL
;; --" indicator (M32). nil/unset shows nothing, matching
;; `mode-line-prefix''s own convention.
(defvar echo-area-fallback nil)

;; The cursor's shape -- GNU Emacs's own variable name and value
;; spellings ('box / 'bar / 'hbar; nil leaves the terminal's own cursor
;; alone, same meaning as the self-made `cursor-shape' this replaces,
;; M28-M31). Both frontends read this now (M32 — before, only the TUI
;; did, while evil.el kept setting the differently-named `cursor-shape',
;; so the GUI's cursor never actually changed): the TUI turns it into a
;; DECSCUSR escape after each draw (frontend-tui's `cursor_type_escape'),
;; the GUI draws 'box as reverse video and 'bar/'hbar as an overlaid
;; line (frontend-gui's own `cursor_type' local, already GNU-named
;; before this migration).
(defvar cursor-type nil)

;; --- Default global key bindings ---

(global-set-key "C-f" 'forward-char)
(global-set-key "C-b" 'backward-char)
(global-set-key "C-n" 'next-line)
(global-set-key "C-p" 'previous-line)
(global-set-key "<right>" 'forward-char)
(global-set-key "<left>" 'backward-char)
(global-set-key "<down>" 'next-line)
(global-set-key "<up>" 'previous-line)
(global-set-key "C-a" 'beginning-of-line)
(global-set-key "C-e" 'end-of-line)
(global-set-key "M-<" 'beginning-of-buffer)
(global-set-key "M->" 'end-of-buffer)
(global-set-key "<home>" 'beginning-of-line)
(global-set-key "<end>" 'end-of-line)

(global-set-key "RET" 'newline)
(global-set-key "DEL" 'delete-backward-char)
(global-set-key "C-d" 'delete-char)
(global-set-key "C-k" 'kill-line)
(global-set-key "C-w" 'kill-region)
(global-set-key "M-w" 'kill-ring-save)
(global-set-key "C-y" 'yank)
(global-set-key "M-y" 'yank-pop)
(global-set-key "C-SPC" 'set-mark-command)
(global-set-key "C-x C-x" 'exchange-point-and-mark)

(global-set-key "C-x C-s" 'save-buffer)
(global-set-key "C-x C-f" 'find-file)
(global-set-key "C-x C-c" 'save-buffers-kill-terminal)
(global-set-key "C-x b" 'switch-to-buffer)
(global-set-key "C-x k" 'kill-buffer-command)
(global-set-key "M-x" 'execute-extended-command)
(global-set-key "M-:" 'eval-expression)
(global-set-key "C-x C-e" 'eval-last-sexp)
(global-set-key "C-M-x" 'eval-defun)
(global-set-key "M-g M-g" 'goto-line)
;; M80: bound to the `next-error'/`previous-error' dispatcher
;; (compile.el) rather than `next-diagnostic'/`previous-diagnostic'
;; (lsp.el) directly -- one dispatcher picks compile-errors-vs-LSP-
;; diagnostics per call, so `M-g n' keeps meaning exactly one thing
;; instead of forcing the user to remember which of two key sequences
;; (GNU splits this into `M-g M-n' vs `M-g n') has the source they want
;; right now. See compile.el's own header for the dispatch rule.
(global-set-key "M-g n" 'next-error)
(global-set-key "M-g p" 'previous-error)
(global-set-key "C-h ." 'lsp-hover-at-point)
(global-set-key "M-." 'lsp-definition-at-point)
(global-set-key "M-," 'lsp-pop-definition-stack)
(global-set-key "M-?" 'lsp-references-at-point)

;; M46/M47/M48: the rest of the LSP interactive commands, under a `C-c
;; l' prefix -- `C-c l' itself is unclaimed (verilog-auto.el only takes
;; `C-c C-a'/`C-c C-k'; org.el/eshell.el's `C-c' bindings are all
;; `C-c C-*'), and evil-mode never binds `C-c'-prefixed keys in any
;; state (normal/insert/visual all fall through to this global map), so
;; this works unconditionally regardless of which mode/state is active.
;; A multi-key combo under a plain (non-control) prefix character has a
;; precedent already: `M-g M-g' above.
(global-set-key "C-c l a" 'lsp-code-action-at-point)
(global-set-key "C-c l r" 'lsp-rename)
(global-set-key "C-c l f" 'lsp-format-buffer)
(global-set-key "C-c l F" 'lsp-format-region)
;; M104: style-selectable formatting, dispatching per-mode to either LSP
;; or an external formatter (format.el) -- distinct from "C-c l f/F"
;; above, which always go straight to LSP with no style choice and no
;; external-process fallback.
(global-set-key "C-c f f" 'format-buffer)
(global-set-key "C-c f r" 'format-region)
(global-set-key "C-c f s" 'format-set-style)
(global-set-key "C-c l s" 'lsp-goto-symbol-by-name)
(global-set-key "C-c l n" 'lsp-next-symbol)
(global-set-key "C-c l p" 'lsp-previous-symbol)

;; M49: documentHighlight. Lowercase `C-c l n'/`C-c l p' are already
;; `lsp-next-symbol'/`lsp-previous-symbol' (above), so highlight
;; navigation gets the shifted `C-c l N'/`C-c l P' instead.
(global-set-key "C-c l h" 'lsp-highlight-at-point)
(global-set-key "C-c l H" 'lsp-highlight-clear)
(global-set-key "C-c l N" 'lsp-next-highlight)
(global-set-key "C-c l P" 'lsp-previous-highlight)

(global-set-key "C-x 2" 'split-window-below)
(global-set-key "C-x 3" 'split-window-right)
(global-set-key "C-x o" 'other-window)
(global-set-key "C-x 0" 'delete-window)
(global-set-key "C-x 1" 'delete-other-windows)

;; M102: window resizing. GNU has no default binding for
;; `shrink-window'; it's still reachable via M-x here.
(global-set-key "C-x ^" 'enlarge-window)
(global-set-key "C-x }" 'enlarge-window-horizontally)
(global-set-key "C-x {" 'shrink-window-horizontally)
(global-set-key "C-x +" 'balance-windows)

(global-set-key "C-s" 'isearch-forward)
(global-set-key "C-r" 'isearch-backward)

;; In a terminal both physical keys send byte 0x1F, so "C-_" is the one
;; that actually fires there; "C-/" only triggers in the GUI.
(global-set-key "C-/" 'undo)
(global-set-key "C-_" 'undo)
(global-set-key "C-x u" 'undo)

;; M79: evil-mode never claims any `M-'-prefixed key in any state (its
;; keymaps are entirely letter/control-key based -- same reasoning
;; `C-c l ...' above already relies on), so these three fall straight
;; through to this global map unconditionally.
(global-set-key "M-!" 'shell-command)
(global-set-key "M-|" 'shell-command-on-region)
(global-set-key "M-&" 'async-shell-command)

;; M82 (fix round R2, coordinator decision): the search family lives
;; under a `C-c s' PREFIX, structurally mirroring `C-c l ...' (the LSP
;; family) above, rather than the two flat keys (`C-c s'/`C-c S') this
;; file originally bound. Motivation (coordinator's own): M83 (editable
;; results), M85 (live-as-you-type search), and M86 (export) are all
;; expected to want their own key under this same family -- establishing
;; the prefix now avoids each of those instead claiming its own
;; unrelated flat key (`C-c r', `C-c g', ...) with no shared structure.
;; Accepted cost: the everyday case (`search-project') goes from two
;; keys to three.
;;
;; LOAD-BEARING ORDERING NOTE, not just a style preference: `C-c s'
;; itself must NEVER be bound directly to a command (see `help--
;; shadowed-p''s doc comment above for the exact mechanism) --
;; `lookup_in' (commands.rs) resolves a key sequence one key at a time,
;; and the moment it resolves `C-c s' to `Lookup::Command', dispatch
;; stops right there; a longer sequence like `C-c s s' can then NEVER be
;; reached no matter what it's bound to, because the lookup for it never
;; gets past its own `C-c s' prefix. This is exactly the failure mode
;; `search_c_c_s_r_key_reaches_search_project_regexp'
;; (search_tests.rs) exists to catch: binding `C-c s' straight to a command
;; again would make that test fail, not silently degrade.
(global-set-key "C-c s s" 'search-project)
(global-set-key "C-c s r" 'search-project-regexp)
(global-set-key "C-c s a" 'search-again)

(provide 'simple)
