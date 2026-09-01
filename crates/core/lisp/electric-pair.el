;;; electric-pair.el --- M37: auto-close/skip matching brackets and quotes -*- lexical-binding: t -*-

;; GNU `electric-pair-mode' equivalent. Mechanism: a `post-self-insert-
;; hook' function, NOT a keybinding for `(' etc. -- and this is a safety
;; requirement, not a style preference. `self_insert' (commands.rs)
;; already runs `post-self-insert-hook' after every successful insertion
;; (pre-existing plumbing: `run_post_insert_hook' was already calling
;; `run_hook_by_name(interp, "post-self-insert-hook")' before this file
;; existed, added ahead of time for "modes react to typed text" -- see
;; its own doc comment -- so M37 needed ZERO Rust changes to wire this
;; up; confirmed by grepping `post-self-insert-hook' for consumers
;; before writing this file and finding none). A real key BINDING for
;; `(' would instead be a `Lookup::Command' hit in `dispatch_key'
;; (commands.rs), dispatched unconditionally by `execute_command' --
;; punching straight through M34's evil `inhibit-self-insert' guard,
;; which only ever gets consulted at the self-insert FALLBACK inside
;; `dispatch_key''s `Undefined' arm. Hooking `post-self-insert-hook'
;; instead means evil's normal state -- which never reaches
;; `self_insert' at all for a key that keymap dispatch treats as
;; Undefined -- never reaches this file's code either: inherently safe
;; by construction, not by a special case here. See
;; `evil_normal_state_open_paren_does_not_pair_or_insert' in
;; electric_pair_tests.rs, which pins exactly this reasoning.
;;
;; `char-before' stands in for GNU's `last-command-event' (the character
;; just self-inserted): this codebase has no `last-command-event'
;; equivalent, but since this hook only ever runs immediately after a
;; successful self-insert (never on a no-op, e.g. a read-only buffer --
;; see `self_insert''s own early return), the character it just inserted
;; is always exactly the one sitting right before point.
;;
;; No syntax awareness (M37 review disclosure, same standard as the
;; dot-repeat interaction gap documented in electric_pair_tests.rs --
;; search that file for "Evil dot-repeat x electric-pair interaction"):
;; every skip/pair decision below is a bare CHARACTER comparison
;; (`char-before'/`char-after'), never a `syntax-ppss'/in-string-or-
;; comment-p check. Concretely: with buffer text `"foo)"' and point
;; sitting right before that `)' (which is really just string TEXT, not
;; a structural closer), typing `)' still reads `char-after' as `)' and
;; SKIPS (deletes the existing one forward) instead of inserting a
;; second `)' -- exactly as if the `)' were a real bracket. The same
;; blindness applies to the quote arm: a `"' typed while already inside
;; a string literal is judged solely by whatever character happens to
;; follow point, not by whether point is actually inside a string. This
;; is not an oversight to be fixed opportunistically; it is the same
;; naive, syntax-free skip behavior GNU's own `electric-pair-mode' falls
;; back to whenever its optional `electric-pair-skip-syntax'/
;; `electric-pair-inhibit-predicate' layers are absent -- v1 here simply
;; has no such layer at all yet.

(defvar electric-pair-mode nil
  "Buffer-local (via `setq-local'/a mode hook, like `display-line-
numbers' -- see modes.el). When non-nil, `electric-pair-post-self-
insert' auto-closes an opening bracket/quote just typed and skips over
\(rather than duplicating\) a closing one already sitting at point. Off
by default; `prog-mode-hook' turns it on buffer-locally below --
`(remove-hook 'prog-mode-hook ...)' or a buffer-local `(setq-local
electric-pair-mode nil)' turns it back off.")

(defvar electric-pair-pairs
  '((?\( . ?\))
    (?\[ . ?\])
    (?\{ . ?\})
    (?\" . ?\"))
  "Alist of (OPEN-CHAR . CLOSE-CHAR) pairs `electric-pair-mode' auto-
closes/skips. v1: one shared table for every major mode (a mode or the
user can still rebind it, globally or buffer-locally) -- elisp's `''
reader-macro quote is deliberately absent, since it isn't a balanced
delimiter the way the other four are.")

(defun electric-pair-post-self-insert ()
  "`post-self-insert-hook' function (M37). A no-op unless
`electric-pair-mode' is non-nil in the current buffer. Three cases,
keyed off the character `char-before' reports (see this file's header):

 - An OPEN bracket (an `electric-pair-pairs' car): insert its matching
   close right after it, then `backward-char' back between the two.
 - A double quote: quotes are their own close, so this collapses to
   \"skip\" (below) when one already sits at point, else \"pair\",
   exactly like an open bracket.
 - A CLOSE bracket (an `electric-pair-pairs' cdr): when `char-after' is
   that SAME character, delete it -- the net effect is \"typed over\"
   the existing closer instead of duplicating it. Otherwise the literal
   character self-insert already placed stands as typed, unbalanced or
   not (matching real `electric-pair-mode')."
  (when electric-pair-mode
    (let ((c (char-before)))
      (when c
        (cond
          ((eq c ?\")
           (if (eq (char-after) ?\")
               (delete-char 1)
             (progn (insert ?\") (backward-char 1))))
          ((assq c electric-pair-pairs)
           (insert (cdr (assq c electric-pair-pairs)))
           (backward-char 1))
          ((rassq c electric-pair-pairs)
           (when (eq (char-after) c)
             (delete-char 1))))))))

(add-hook 'post-self-insert-hook 'electric-pair-post-self-insert)

(defun electric-pair-backward-delete ()
  "DEL command bound locally in prog buffers (see the `prog-mode-hook'
addition below): when point sits exactly between a matching empty pair
\(`electric-pair-pairs' -- `(|)', `\"|\"', ...\) deletes BOTH characters
as one edit (GNU's `electric-pair-delete-adjacent-pairs'); otherwise
falls back to plain `delete-backward-char'. Both branches land in the
SAME undo group as an ordinary DEL: `execute_command' (commands.rs)
inserts its one undo boundary before this function starts running, and
neither `delete-char' nor `delete-backward-char' ever inserts one of
its own (see buffer.rs's `Buffer::insert'/`delete', which only ever
record an Insert/Delete undo entry, never a Boundary) -- so a single DEL
press here is a single undo step regardless of which branch runs."
  (interactive)
  (if (and electric-pair-mode
           (char-before)
           (char-after)
           (eq (cdr (assq (char-before) electric-pair-pairs)) (char-after)))
      (progn (delete-char 1) (delete-backward-char 1))
    (delete-backward-char 1)))

;; Buffer-local, like `display-line-numbers' (modes.el) and RET
;; (indent.el) -- a user's `(remove-hook 'prog-mode-hook ...)' or a
;; later `(local-set-key "DEL" ...)' in a mode's own hook still wins.
(add-hook 'prog-mode-hook
          (lambda ()
            (setq-local electric-pair-mode t)
            (local-set-key "DEL" 'electric-pair-backward-delete)))

(provide 'electric-pair)
