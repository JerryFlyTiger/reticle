;;; modes.el --- auto-mode-alist, normal-mode, prog-mode conventions -*- lexical-binding: t -*-

;; M24: file-extension major-mode dispatch, GNU-compatible. Loaded right
;; after simple.el (needs setq-local/add-hook, both from prelude.el/
;; simple.el) and before org.el/treesit.el, whose major modes register
;; themselves in `auto-mode-alist' here instead of the ad hoc
;; find-file-hook lambdas they used before this file existed.
;;
;; M25 adds seven more tree-sitter-backed major modes (c/c++/python/sh/
;; java/perl/emacs-lisp, alongside M12-M15's rust-mode) and, since several
;; of their languages are conventionally extensionless scripts,
;; `interpreter-mode-alist' + a #! shebang fallback in `normal-mode'.

;; --- auto-mode-alist / normal-mode ------------------------------------

(defvar auto-mode-alist nil
  "Alist of (REGEXP . MODE-FUNCTION) consulted by `normal-mode'.
REGEXP is matched against the full visited file name with
`string-match'; the first entry that matches wins. Extend it the
GNU way: (add-to-list 'auto-mode-alist '(\"\\\\.foo\\\\'\" . foo-mode)).")

;; Generic list helper (GNU subr.el) — nothing here needed it before
;; auto-mode-alist did.
(defun add-to-list (list-var element &optional append)
  "Add ELEMENT to the value of LIST-VAR if not already `member' of it.
Prepends by default; appends when APPEND is non-nil. Returns the new
value of LIST-VAR."
  (let ((current (symbol-value list-var)))
    (if (member element current)
        current
      (set list-var
           (if append
               (append current (list element))
             (cons element current))))))

(defvar interpreter-mode-alist
  '(("^python[0-9.]*$" . python-mode)
    ("^bash$" . sh-mode)
    ("^sh$" . sh-mode)
    ("^zsh$" . sh-mode)
    ("^perl[0-9.]*$" . perl-mode))
  "Alist of (INTERPRETER-REGEXP . MODE-FUNCTION) consulted by
`normal-mode' when a buffer's file name didn't match anything in
`auto-mode-alist' but its first line is a #! shebang. Matched against
the basename of the interpreter binary named on that line with
`string-match' -- the first entry that matches wins, GNU style (see
`auto-mode-alist') -- so version-suffixed interpreters like
\"python3.11\" or \"perl5.36\" still match. A leading `env' is unwrapped
first, skipping env's own dash-prefixed flags (e.g. the \"-S\" in
\"#!/usr/bin/env -S python3 -u\"), so both \"#!/usr/bin/python3\" and
\"#!/usr/bin/env -S python3 -u\" resolve to \"python3\". Extend it the
GNU way: (add-to-list 'interpreter-mode-alist '(\"^ruby[0-9.]*$\" . ruby-mode)).")

(defun normal-mode--shebang-interpreter ()
  "The basename of the interpreter named on the buffer's #! line, or nil
if the buffer has none. A leading `env' is unwrapped: env's own
dash-prefixed flags (e.g. -S, -i) are skipped and the first remaining
word is taken as the real interpreter."
  (let ((first-line (save-excursion
                       (goto-char (point-min))
                       (buffer-substring (point-min) (line-end-position)))))
    (when (string-prefix-p "#!" first-line)
      (let* ((words (split-string (substring first-line 2)))
             (prog (and (car words) (file-name-nondirectory (car words)))))
        (when (and prog (string= prog "env"))
          (setq words (cdr words))
          (while (and words (string-prefix-p "-" (car words)))
            (setq words (cdr words)))
          (setq prog (and (car words) (file-name-nondirectory (car words)))))
        prog))))

(defun normal-mode--shebang-mode ()
  "Return the mode function named by the buffer's #! line, or nil if
the buffer has none, or names an interpreter no `interpreter-mode-alist'
regexp matches."
  (let ((prog (normal-mode--shebang-interpreter))
        (alist interpreter-mode-alist)
        (mode nil))
    (while (and alist (not mode))
      (let ((entry (car alist)))
        (when (and prog (string-match (car entry) prog))
          (setq mode (cdr entry))))
      (setq alist (cdr alist)))
    mode))

(defun fundamental-mode ()
  "Major mode that does nothing in particular."
  (interactive)
  (major-mode-internal-set 'fundamental-mode))

(defun normal-mode ()
  "Choose a major mode for the current buffer from `auto-mode-alist'.
Matches `buffer-file-name' against each entry's REGEXP in order and
calls the MODE-FUNCTION of the first match. When nothing matches (or
the buffer visits no file), falls back to the interpreter named on a
#! first line via `interpreter-mode-alist' (see
`normal-mode--shebang-mode'), and only then to `fundamental-mode'."
  (interactive)
  (let ((file (buffer-file-name))
        (alist auto-mode-alist)
        (mode nil))
    (when file
      (while (and alist (not mode))
        (let ((entry (car alist)))
          (when (string-match (car entry) file)
            (setq mode (cdr entry))))
        (setq alist (cdr alist))))
    (unless mode
      (setq mode (normal-mode--shebang-mode)))
    (if mode (funcall mode) (fundamental-mode))))

;; --- prog-mode: shared conventions for programming-language modes ----

(defvar prog-mode-hook nil
  "Hook run by every programming-language major mode, after its own
setup and before its own mode-specific hook.")

;; Programming modes show line numbers by default; text/fundamental
;; buffers stay off unless the user turns display-line-numbers on
;; globally. A mode can override in its own hook, and the user can
;; always (remove-hook 'prog-mode-hook ...) or setq-local it back off.
(add-hook 'prog-mode-hook
          (lambda () (setq-local display-line-numbers t)))

;; M37: rainbow-delimiters -- matching brackets colored by nesting depth
;; (see highlight.rs's `collect_rainbow_spans'/`apply_visible'). Same
;; buffer-local-default-on-for-prog-buffers convention as display-line-
;; numbers just above (own `add-hook' call, kept independent of it since
;; the two toggle unrelated features that merely happen to share
;; `prog-mode-hook'); `rainbow-delimiters-depth-1..9-face' are defined in
;; themes.el.
(defvar rainbow-delimiters-mode nil
  "Buffer-local. Non-nil shows matching brackets colored by nesting
depth (`rainbow-delimiters-depth-N-face', N cycling 1..9). Off by
default; `prog-mode-hook' turns it on buffer-locally.")

(add-hook 'prog-mode-hook
          (lambda () (setq-local rainbow-delimiters-mode t)))

;; Shared body for every tree-sitter-backed programming major mode below:
;; set MODE as the buffer's major mode, turn on tree-sitter highlighting
;; for the `treesit-highlight-mode' language LANG, set the buffer-local
;; M36 indentation variables (WIDTH -> `standard-indent-width', INDENT-FN
;; -> `indent-line-function' -- see indent.el, loaded right after this
;; file), then run `prog-mode-hook' followed by HOOK (the calling mode's
;; own hook variable, passed as a symbol -- `run-hooks' just needs its
;; value to *be* a symbol, not a literal quoted one, so this works the
;; same as each mode running `(run-hooks 'foo-mode-hook)' itself).
;; Callers are thin wrapper defuns; see `rust-mode' immediately below for
;; the shape every major mode in this file follows. (indent.el's own
;; defvars for `standard-indent-width'/`indent-line-function' load AFTER
;; this function is DEFINED, but not before it's ever CALLED -- a mode
;; function only runs when a matching file is actually opened, always
;; well after startup has loaded every built-in lisp file; `setq-local'
;; never does a compile-time-resolved variable reference either way, see
;; indent.el's header.) WIDTH/INDENT-FN are `&optional' (M36 review fix,
;; low severity): defaulting to 4/nil means an old-style 3-arg call --
;; none left in this file, but a user's own `init.el' could plausibly
;; have copied this shape before M36 -- degrades gracefully (no
;; indentation engine for that mode, TAB just self-inserts, matching
;; every buffer's pre-M36 behavior) instead of an arity error.
;; M73: `(indent--maybe-detect-width)' sits here -- after the mode
;; default is set, before `prog-mode-hook'/HOOK run -- for three
;; reasons (see indent.el's own header for the detection algorithm
;; itself): (1) this single point covers all 9 prog modes and every
;; open-file entry point (`find-file', dired RET, LSP `M-.'/`M-?', M55's
;; Verilog jump, command-line args, `/ssh:' remote) -- the M73 survey
;; confirmed all of them collapse into `find_file_internal'
;; (editing.rs:735) or `find_file_remote' (editing.rs:1108), and both
;; are "content lands in the buffer first (editing.rs:802 / :1151),
;; THEN `run_normal_mode' (:816 / :1162)" -- so no Rust change is needed
;; here at all. (2) running it before `prog-mode-hook'/HOOK means a
;; user's own `(setq-local standard-indent-width ...)' in e.g.
;; `verilog-mode-hook' still wins over whatever this guesses -- explicit
;; beats guessed. (3) this doesn't rely on `add-hook' prepending vs.
;; appending (the M73 survey did not verify that ordering either way).
(defun treesit--prog-mode-setup (mode lang hook &optional width indent-fn)
  (major-mode-internal-set mode)
  (treesit-highlight-mode lang)
  (setq-local standard-indent-width (or width 4))
  (setq-local indent-line-function indent-fn)
  (indent--maybe-detect-width)
  (run-hooks 'prog-mode-hook)
  (run-hooks hook))

;; --- rust-mode ---------------------------------------------------------

(defvar rust-mode-hook nil)

(defun rust-mode ()
  "Major mode for editing Rust code."
  (interactive)
  (treesit--prog-mode-setup 'rust-mode 'rust 'rust-mode-hook 4 'rust-indent-line))

(add-to-list 'auto-mode-alist '("\\.rs\\'" . rust-mode))

;; --- c-mode --------------------------------------------------------------

(defvar c-mode-hook nil)

(defun c-mode ()
  "Major mode for editing C code."
  (interactive)
  (treesit--prog-mode-setup 'c-mode 'c 'c-mode-hook 4 'c-indent-line))

(add-to-list 'auto-mode-alist '("\\.c\\'" . c-mode))
(add-to-list 'auto-mode-alist '("\\.h\\'" . c-mode))

;; --- c++-mode --------------------------------------------------------------

(defvar c++-mode-hook nil)

(defun c++-mode ()
  "Major mode for editing C++ code."
  (interactive)
  (treesit--prog-mode-setup 'c++-mode 'cpp 'c++-mode-hook 4 'c++-indent-line))

(add-to-list 'auto-mode-alist '("\\.cc\\'" . c++-mode))
(add-to-list 'auto-mode-alist '("\\.cpp\\'" . c++-mode))
(add-to-list 'auto-mode-alist '("\\.cxx\\'" . c++-mode))
(add-to-list 'auto-mode-alist '("\\.hpp\\'" . c++-mode))
(add-to-list 'auto-mode-alist '("\\.hh\\'" . c++-mode))

;; --- python-mode -----------------------------------------------------------

(defvar python-mode-hook nil)

(defun python-mode ()
  "Major mode for editing Python code."
  (interactive)
  (treesit--prog-mode-setup 'python-mode 'python 'python-mode-hook 4 'python-indent-line))

(add-to-list 'auto-mode-alist '("\\.py\\'" . python-mode))

;; --- sh-mode ---------------------------------------------------------------

(defvar sh-mode-hook nil)

(defun sh-mode ()
  "Major mode for editing shell scripts."
  (interactive)
  (treesit--prog-mode-setup 'sh-mode 'bash 'sh-mode-hook 2 'sh-indent-line))

(add-to-list 'auto-mode-alist '("\\.sh\\'" . sh-mode))
(add-to-list 'auto-mode-alist '("\\.bash\\'" . sh-mode))

;; --- java-mode ---------------------------------------------------------

(defvar java-mode-hook nil)

(defun java-mode ()
  "Major mode for editing Java code."
  (interactive)
  (treesit--prog-mode-setup 'java-mode 'java 'java-mode-hook 4 'java-indent-line))

(add-to-list 'auto-mode-alist '("\\.java\\'" . java-mode))

;; --- perl-mode ---------------------------------------------------------

(defvar perl-mode-hook nil)

(defun perl-mode ()
  "Major mode for editing Perl code."
  (interactive)
  (treesit--prog-mode-setup 'perl-mode 'perl 'perl-mode-hook 4 'perl-indent-line))

(add-to-list 'auto-mode-alist '("\\.pl\\'" . perl-mode))
(add-to-list 'auto-mode-alist '("\\.pm\\'" . perl-mode))

;; --- emacs-lisp-mode -----------------------------------------------------

(defvar emacs-lisp-mode-hook nil)

(defun emacs-lisp-mode ()
  "Major mode for editing Emacs Lisp code."
  (interactive)
  (treesit--prog-mode-setup 'emacs-lisp-mode 'elisp 'emacs-lisp-mode-hook 2 'emacs-lisp-indent-line))

(add-to-list 'auto-mode-alist '("\\.el\\'" . emacs-lisp-mode))

;; --- verilog-mode --------------------------------------------------------

;; M38: Verilog/SystemVerilog, via tree-sitter-systemverilog (an IEEE
;; 1800-2023 grammar -- SystemVerilog is a strict superset of Verilog, and
;; the go/no-go survey found no separate plain-Verilog grammar worth using
;; instead, so .v/.vh share this exact mode/grammar/query with .sv/.svh,
;; matching real verilog-ts-mode's own choice).

(defvar verilog-mode-hook nil)

(defun verilog-mode ()
  "Major mode for editing Verilog/SystemVerilog code."
  (interactive)
  (treesit--prog-mode-setup 'verilog-mode 'verilog 'verilog-mode-hook 4 'verilog-indent-line))

(add-to-list 'auto-mode-alist '("\\.v\\'" . verilog-mode))
(add-to-list 'auto-mode-alist '("\\.vh\\'" . verilog-mode))
(add-to-list 'auto-mode-alist '("\\.sv\\'" . verilog-mode))
(add-to-list 'auto-mode-alist '("\\.svh\\'" . verilog-mode))

;; M54: `C-M-i' in a Verilog buffer tries `verilog-complete-at-point'
;; (verilog-complete.el) before LSP/dabbrev -- see `local-completion-
;; function''s own docstring in lsp.el for why. `local-completion-
;; function' is a plain `defvar' in lsp.el, loaded AFTER this file (see
;; lib.rs's load order comments), but `setq-local' inside a hook
;; function that only runs once a `verilog-mode' buffer is actually set
;; up -- long after every file here has finished loading -- needs no
;; load-order relationship with lsp.el at all; `verilog-complete-at-
;; point' itself (verilog-complete.el) is loaded later still, for the
;; same reason.
;;
;; M55: `M-.'/`g d' (`lsp-definition-at-point') try
;; `verilog-goto-module-at-point' (verilog-nav.el) first, same
;; no-load-order-dependency reasoning as `local-completion-function'
;; just above -- see `local-definition-function''s own docstring in
;; lsp.el.
(add-hook 'verilog-mode-hook
          (lambda ()
            (setq-local local-completion-function 'verilog-complete-at-point)
            (setq-local local-definition-function 'verilog-goto-module-at-point)))

(provide 'modes)
