;;; ielm.el --- interactive elisp REPL buffer (M-x ielm) -*- lexical-binding: t -*-

;; A GNU-Emacs-style IELM: a *ielm* buffer with a prompt; RET reads the
;; input after the last prompt, evaluates it, and inserts the printed
;; result and a fresh prompt. Incomplete input (unbalanced parens)
;; continues on the next line, like the standalone REPL.

(defvar ielm-prompt "ELISP> ")

;; Position right after the most recent prompt: the input region is
;; everything from here to the end of the buffer. A plain position (not
;; a marker) is fine because only ielm itself inserts before it.
(defvar ielm--input-start 1)

(defun ielm ()
  "Open the *ielm* interactive elisp REPL buffer."
  (interactive)
  (switch-to-buffer-internal "*ielm*")
  ;; M29: without this, `ielm-mode' never actually names this buffer's
  ;; major-mode (it stayed nil/fundamental), so evil-mode's default
  ;; `evil-emacs-state-modes' entry for it was inert. Same pattern
  ;; dired.el/eshell.el already use.
  (major-mode-internal-set 'ielm-mode)
  (let ((map (make-sparse-keymap)))
    (define-key map "RET" 'ielm-return)
    (use-local-map map))
  (when (= (buffer-size) 0)
    (insert "*** Welcome to IELM ***  Type an elisp expression and press RET.\n")
    (ielm--insert-prompt))
  (goto-char (point-max)))

(defun ielm--insert-prompt ()
  (goto-char (point-max))
  (insert ielm-prompt)
  (setq ielm--input-start (point-max))
  ;; M71: the buffer construction (`ielm', above) and three of
  ;; `ielm-return''s four outcomes (blank input, read error, evaluated
  ;; result) end by calling this; the fourth (incomplete input) does
  ;; NOT call this function and clears the flag itself right after its
  ;; own `insert' instead (see that branch's own comment). *ielm* has
  ;; no file to save -- its `*' would only ever mean "you evaluated
  ;; something", never "you have unsaved work".
  ;;
  ;; This is NOT a blanket guarantee the buffer never shows `*':
  ;; `insert' is not a no-op through `check_writable' (*ielm* is
  ;; writable, unlike dired/*Help*), so every character the user types
  ;; between one prompt and the next (before RET) sets the flag back to
  ;; true via `Buffer::insert' (buffer.rs:311) -- deliberately
  ;; unaddressed here; catching it would mean intercepting every
  ;; self-insert. Also, `C-x u'/`C-/'/`C-_' (undo) reach
  ;; `Buffer::undo_step_from' (buffer.rs:670), which sets the flag
  ;; unconditionally once entered. Getting there IS gated by
  ;; `check_writable' (editing.rs:534 -- the same gate inserts and
  ;; deletes go through), and that gate is exactly why dired and *Help*
  ;; are safe while *ielm* is not: those two end their fill with
  ;; `set-buffer-read-only t', so undo is refused there; *ielm* stays
  ;; writable, so undo goes through and re-dirties it.
  ;; elisp has no primitive to suppress undo recording; fixing that
  ;; would require a Rust-level change, out of scope here.
  (set-buffer-modified-p nil))

(defun ielm-return ()
  "Evaluate the current IELM input, or continue the line if incomplete."
  (interactive)
  (let* ((input (buffer-substring ielm--input-start (point-max)))
         (parse (condition-case nil
                    (cons 'ok (read input))
                  (end-of-file 'incomplete)
                  (error 'bad))))
    (goto-char (point-max))
    (cond
      ;; Blank input: just start a fresh prompt line.
      ((string-empty-p (string-trim input))
       (insert "\n")
       (ielm--insert-prompt))
      ;; Unbalanced parens/string: keep typing on the next line. This is
      ;; a stable state (the user already pressed RET, not "typing
      ;; mid-word") so M71 clears the flag here too, same as every
      ;; other branch.
      ((eq parse 'incomplete)
       (insert "\n")
       (set-buffer-modified-p nil))
      ((eq parse 'bad)
       (insert "\n*** Read error\n")
       (ielm--insert-prompt))
      (t
       (insert "\n")
       (let ((result (condition-case err
                         (prin1-to-string (eval (cdr parse) t))
                       (error (format "*** Error: %S" err)))))
         (insert result "\n"))
       (ielm--insert-prompt)))))

(provide 'ielm)
