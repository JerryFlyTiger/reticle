;;; format.el --- style-selectable, save-time code formatting -*- lexical-binding: t -*-

;; M104: "pick a style name like clang-format, then saving the file
;; formats it automatically -- no manual reformatting" (the owner's own
;; words). Two separate mechanisms stay deliberately apart:
;;
;;   - Typing only ever runs `indent-line-function' (indent.el, M73 and
;;     earlier) -- unchanged by this file.
;;   - Saving runs a FULL reformat, via `format--maybe-on-save' on
;;     `before-save-hook', ON BY DEFAULT (`format-on-save', see below).
;;
;; This is the VS Code / JetBrains split, not GNU Emacs's (which has no
;; save-time full-reformat hook at all): format-as-you-type is
;; deliberately NOT built, only format-on-save.
;;
;; Two kinds of formatting provider, keyed by major mode:
;;
;;   'lsp                      -- delegate to the existing
;;                                `lsp-format-buffer'/`lsp-format-region'
;;                                (lsp.el, M46/M57). Unmodified here.
;;   (PROGRAM . ARGS-FUNCTION) -- run PROGRAM synchronously via M104's
;;                                `call-process-string' (elisp/src/
;;                                shell.rs), feeding the whole buffer on
;;                                stdin and reading the formatted result
;;                                back off stdout.
;;
;; Why LSP can't be the only path: `verible-verilog-ls' silently ignores
;; `FormattingOptions' in the LSP request (measured directly against the
;; real server -- `tabSize: 2' and `tabSize: 8' produce byte-identical
;; output), but DOES honor `--indentation_spaces' as a command-line flag
;; to the separate `verible-verilog-format' binary. So Verilog style
;; selection is only possible through the external-process path; the
;; LSP path exists for languages whose server DOES take formatting
;; options seriously (rust-analyzer et al., not applicable here since
;; this project doesn't ship one, but the mechanism is generic).
;;
;; `aligned'/`compact' below (Verilog styles) are NOT upstream verible
;; concepts -- verible has no named styles, only individual flags. Those
;; two names, and which flags they bundle, are this project's own
;; invention, chosen to give a Verilog engineer two opinionated presets
;; beyond verible's own un-opinionated `infer' default.

;; --- Provider tables ---------------------------------------------------

(defvar format-provider-alist
  (list (cons 'verilog-mode (cons "verible-verilog-format" 'format--verible-args))
        (cons 'c-mode (cons "clang-format" 'format--clang-args))
        (cons 'c++-mode (cons "clang-format" 'format--clang-args))
        (cons 'rust-mode 'lsp)
        (cons 'python-mode 'lsp)
        (cons 'java-mode 'lsp)
        (cons 'sh-mode 'lsp)
        (cons 'perl-mode 'lsp))
  "Major mode -> formatting provider. Either the symbol `lsp' (dispatch
to `lsp-format-buffer'/`lsp-format-region', falling back to
`format-fallback-alist' when the buffer has no LSP formatting
capability -- see `format--lsp-buffer-capable-p'), or a cons
`(PROGRAM . ARGS-FUNCTION)' run via `call-process-string': PROGRAM is
an executable name/path, ARGS-FUNCTION is a function of one argument
(the mode's current style symbol, from `format-style-alist') returning
the argv list to pass (not including PROGRAM itself).

A mode absent from this alist has no formatting support at all --
`format-buffer'/`format-region' just `message' and leave the buffer
untouched.")

(defvar format-fallback-alist
  (list (cons 'rust-mode (cons "rustfmt" 'format--rustfmt-args))
        (cons 'python-mode (cons "black" 'format--black-args)))
  "Major mode -> `(PROGRAM . ARGS-FUNCTION)' fallback, used only when
that mode's `format-provider-alist' entry is `lsp' AND the current
buffer has no usable LSP formatting capability (no connected server, or
a connected server that doesn't advertise
`\"documentFormattingProvider\"'). Modes not listed here that fall into
that situation just get a `message' and no formatting -- there is no
external fallback wired up for them in this v1.")

(defvar format-style-alist
  (list (cons 'verilog-mode 'verible)
        (cons 'c-mode 'file)
        (cons 'c++-mode 'file))
  "Major mode -> the style symbol currently selected for that mode.
Deliberately keyed by MODE, not buffer-local -- style is a per-language
setting, not a per-buffer one, so picking a style in one Verilog buffer
via `format-set-style' affects every other Verilog buffer too. Modes
whose provider has only one possible style (the `lsp'-backed modes, and
the `rustfmt'/`black' fallbacks) have no entry here at all.")

(defvar format-style-choices
  (list (cons 'verilog-mode (list 'verible 'aligned 'compact))
        (cons 'c-mode (list 'llvm 'gnu 'google 'mozilla 'webkit 'chromium 'microsoft 'file))
        (cons 'c++-mode (list 'llvm 'gnu 'google 'mozilla 'webkit 'chromium 'microsoft 'file)))
  "Major mode -> list of style symbols `format-set-style' offers via
`completing-read'. Modes with no entry here have nothing to pick from
(`format-set-style' just `message's and stops).")

;; --- Small alist helper --------------------------------------------------

(defun format--alist-put (alist key val)
  "Return a copy of ALIST with KEY's entry set to VAL, replacing an
existing entry for KEY if present or adding one if not. This project's
elisp has no `assq-delete-all'/`seq-filter' to build this out of, so it
is written directly with `dolist'."
  (let (out (replaced nil))
    (dolist (entry alist)
      (if (eq (car entry) key)
          (progn
            (push (cons key val) out)
            (setq replaced t))
        (push entry out)))
    (unless replaced (push (cons key val) out))
    (nreverse out)))

;; --- Verilog: verible-verilog-format --------------------------------------

(defun format--verible-args (style)
  "ARGS-FUNCTION for `verible-verilog-format'. Every style pins
`--indentation_spaces=2' explicitly (matching modes.el's verilog-mode
default -- see that file's M104 comment) rather than relying on
verible's own default of 2, so an upstream default change can't
silently change ours. Ends with `-' so verible reads the buffer text
from stdin (`verible-verilog-format --helpfull': \"To pipe from stdin,
use '-' as <file>\").

`verible' -- upstream defaults for everything besides indentation.
`aligned' -- port declarations, named port connections, named
parameter assignments, parameter/localparam declarations, assignment
statements, case items, and net/variable declarations are all aligned
into columns (each flag confirmed against `--helpfull': accepts
`{align,flush-left,preserve,infer}').
`compact' -- the same set of constructs flush-left (no column
alignment), plus a wider `--column_limit' (120 instead of verible's
default 100) since flush-left text tends to run a little longer per
line than aligned text would have padded it to anyway."
  (append
   (list "--indentation_spaces=2")
   (cond
    ((eq style 'aligned)
     (list "--port_declarations_alignment=align"
           "--named_port_alignment=align"
           "--named_parameter_alignment=align"
           "--parameter_declaration_alignment=align"
           "--assignment_statement_alignment=align"
           "--case_items_alignment=align"
           "--module_net_variable_alignment=align"))
    ((eq style 'compact)
     (list "--column_limit=120"
           "--port_declarations_alignment=flush-left"
           "--named_port_alignment=flush-left"
           "--named_parameter_alignment=flush-left"
           "--parameter_declaration_alignment=flush-left"
           "--assignment_statement_alignment=flush-left"
           "--case_items_alignment=flush-left"
           "--module_net_variable_alignment=flush-left"))
    (t nil))
   (list "-")))

;; --- C/C++: clang-format ---------------------------------------------------

(defun format--clang-style-name (style)
  "Map a `format-style-choices' symbol to clang-format's own `-style='
spelling (it's case-sensitive: `LLVM', `Google', etc., not lowercase)."
  (cond
   ((eq style 'gnu) "GNU")
   ((eq style 'google) "Google")
   ((eq style 'mozilla) "Mozilla")
   ((eq style 'webkit) "WebKit")
   ((eq style 'chromium) "Chromium")
   ((eq style 'microsoft) "Microsoft")
   ((eq style 'llvm) "LLVM")
   (t "file")))

(defun format--clang-args (style)
  "ARGS-FUNCTION for `clang-format'. `-style=file' makes clang-format
search for a project `.clang-format' file and, if none is found, fall
back to LLVM style on its own -- that fallback is clang-format's
existing behavior, not special-cased here. clang-format reads from
stdin and writes to stdout by default when given no positional file
argument, so no `-' is needed (unlike verible).

M104 fix round: also appends `-assume-filename=PATH' when the current
buffer is visiting a file. Two independent reasons a real path matters
to clang-format, not one: (1) with no positional file argument and no
`-assume-filename', clang-format has no extension to infer the
language from at all; (2) `-style=file' searches for a `.clang-format'
starting from the DIRECTORY of whatever path it's given, walking
upward -- silently falling back to LLVM style if it finds none, with
no diagnostic (measured against the real binary). That second point is
also why `format--run-external' independently sets `call-process-
string''s DIR argument to the buffer's own directory (M104 fix round)
rather than relying on `-assume-filename' alone -- belt and suspenders,
since clang-format's own docs don't commit to which one wins if a
future version changes how the search starting point is chosen.
Omitted (both the flag and the DIR override) for a buffer with no file
name, e.g. an unsaved scratch buffer."
  (append
   (list (concat "-style=" (format--clang-style-name style)))
   (when (buffer-file-name)
     (list (concat "-assume-filename=" (buffer-file-name))))))

;; --- LSP fallbacks: rustfmt / black ----------------------------------------

(defun format--rustfmt-args (_style)
  "ARGS-FUNCTION for the `rust-mode' LSP fallback. `rustfmt' with no
file argument reads the source from stdin and writes the formatted
result to stdout. No style choices exist for this fallback (see
`format-style-choices')."
  nil)

(defun format--black-args (_style)
  "ARGS-FUNCTION for the `python-mode' LSP fallback. `-' tells `black'
to read from stdin and write to stdout; `-q' suppresses its normal
\"reformatted -\" status line on stderr, which would otherwise look
like an error to `format--run-external''s failure path (it isn't one:
`black' still exits 0)."
  (list "-q" "-"))

;; --- Dispatch --------------------------------------------------------------

(defun format--lsp-buffer-capable-p ()
  "Non-nil if the current buffer has a live LSP client that advertises
`\"documentFormattingProvider\"'. Both \"no client at all\" and \"client
connected but doesn't advertise the capability\" count as NOT capable
here -- either way `format-buffer' should fall through to
`format-fallback-alist' rather than let `lsp-format-buffer' print its
own \"No LSP server connected\" message and stop."
  (and (fboundp 'lsp--live-buffer-client)
       (let ((client (lsp--live-buffer-client)))
         (and client (lsp--capability-supported-p client "documentFormattingProvider")))))

(defun format--lsp-region-capable-p ()
  "Region-formatting counterpart of `format--lsp-buffer-capable-p', for
`\"documentRangeFormattingProvider\"'."
  (and (fboundp 'lsp--live-buffer-client)
       (let ((client (lsp--live-buffer-client)))
         (and client (lsp--capability-supported-p client "documentRangeFormattingProvider")))))

(defun format--first-line (s)
  "The text of S up to (not including) its first newline, or all of S
if it has none -- used to keep a `message' to one line even when a
formatter's stderr is a multi-line diagnostic dump."
  (car (split-string s "\n" nil)))

(defun format--run-external (mode provider)
  "PROVIDER is `(PROGRAM . ARGS-FUNCTION)'. Runs PROGRAM synchronously
against the whole current buffer via `call-process-string' (default
5-second timeout), feeding the buffer's text on stdin, and IN the
directory of the buffer's own file (M104 fix round -- `call-process-
string''s DIR argument, nil for a buffer with no file name, which
leaves `call-process-string' at its own default of inheriting the
editor's current directory). This matters for `clang-format -style=
file', which searches for a project `.clang-format' starting from ITS
OWN working directory -- without this, that search started from
wherever the EDITOR itself was launched from, not from anywhere near
the file actually being formatted (see `format--clang-args' for the
other half of this fix, `-assume-filename'). On a non-zero exit, or if
PROGRAM can't be run at all (`call-process-string' reports that the
same way, see its own doc comment), the buffer is left completely
untouched and the first line of stderr is `message'd. On a zero exit
whose stdout is identical to the input, `message's \"Already
formatted\" without touching the buffer. Otherwise applies the result
via `replace-region-contents' (point.min)..(point-max) rather than
delete+insert, so point/markers/overlays outside the changed hunks
survive -- see that builtin's own docstring for why."
  (let* ((program (car provider))
         (args-fn (cdr provider))
         (style (cdr (assq mode format-style-alist)))
         (args (funcall args-fn style))
         (dir (and (buffer-file-name) (file-name-directory (buffer-file-name))))
         (old (buffer-string))
         (result (call-process-string program args old 5000 dir))
         (code (nth 0 result))
         (stdout (nth 1 result))
         (stderr (nth 2 result)))
    (cond
     ((/= code 0)
      (message "format-buffer: %s failed (exit %s): %s"
               program code (format--first-line stderr)))
     ((equal stdout old)
      (message "Already formatted"))
     (t
      (replace-region-contents (point-min) (point-max) stdout)))))

(defun format-buffer ()
  "Format the whole current buffer in place, using the provider and
style configured for its major mode in `format-provider-alist'/
`format-style-alist'. See this file's header for the overall design,
and `format--run-external'/`format--lsp-buffer-capable-p' for the
per-provider behavior. A mode with no entry in `format-provider-alist'
just gets a `message'; nothing about this command can signal an error
into the caller (a failed external formatter is reported the same way,
via `message', not `error' -- see `format--run-external')."
  (interactive)
  (let* ((mode (major-mode-internal-get))
         (entry (assq mode format-provider-alist)))
    (cond
     ((not entry)
      (message "format-buffer: no formatter configured for %s" mode))
     ((eq (cdr entry) 'lsp)
      (if (format--lsp-buffer-capable-p)
          (lsp-format-buffer)
        (let ((fallback (cdr (assq mode format-fallback-alist))))
          (if fallback
              (format--run-external mode fallback)
            (message "format-buffer: no LSP formatting capability and no fallback for %s" mode)))))
     (t
      (format--run-external mode (cdr entry))))))

(defun format-region (start end)
  "Format the active region. The `lsp' provider dispatches to
`lsp-format-region' (which reads `region-beginning'/`region-end' itself
-- START/END here only exist to satisfy the `\"r\"' interactive spec,
matching `kill-region''s own shape). External-process providers do NOT
support region formatting in this v1 -- most command-line formatters
only know how to format an entire file/stdin stream, and gluing
together a region-only external format would mean re-parenting the
formatted fragment back into a syntax tree it wasn't seen in, e.g. an
unbalanced-brace region. So the external path always just `message's
and leaves the buffer untouched; it never falls back to
`lsp-format-region' either, on the same \"no LSP capability -> external
fallback\" logic as `format-buffer' would suggest, because the two
external fallbacks that DO exist (`rustfmt'/`black') are whole-file
tools with the identical limitation."
  (interactive "r")
  (ignore start end)
  (let* ((mode (major-mode-internal-get))
         (entry (assq mode format-provider-alist)))
    (cond
     ((not entry)
      (message "format-region: no formatter configured for %s" mode))
     ((eq (cdr entry) 'lsp)
      (if (format--lsp-region-capable-p)
          (lsp-format-region)
        (message "format-region: no LSP region-formatting capability for %s" mode)))
     (t
      (message "format-region: %s does not support region formatting, only whole-buffer"
               (car (cdr entry)))))))

(defun format-set-style ()
  "Pick a new style for the current major mode's formatter, from
`format-style-choices', via `completing-read'. The choice is written
into `format-style-alist' -- GLOBALLY (keyed by mode), not
buffer-local, since style is a language-level setting (see
`format-style-alist''s own docstring). A mode with no entry in
`format-style-choices' (either because its provider has only one style,
or because it has no provider at all) just gets a `message' and stops."
  (interactive)
  (let* ((mode (major-mode-internal-get))
         (choices (cdr (assq mode format-style-choices))))
    (if (not choices)
        (message "format-set-style: no style choices for %s" mode)
      (with-completing-read
       (choice "Format style: " (mapcar 'symbol-name choices) t)
       (let ((sym (intern choice)))
         (setq format-style-alist (format--alist-put format-style-alist mode sym))
         (message "Format style for %s: %s" mode sym))))))

;; --- Format on save ---------------------------------------------------------

(defvar format-on-save t
  "When non-nil (the DEFAULT -- the owner's own call, see this file's
header), `format-buffer' runs automatically on every save of a buffer
whose major mode has an entry in `format-provider-alist'. Set to nil
(e.g. in init.el) to go back to purely manual formatting via
`format-buffer'/`C-c f f'.")

(defun format--maybe-on-save ()
  "`before-save-hook' function: runs `format-buffer' when
`format-on-save' is non-nil and the current buffer's major mode has an
entry in `format-provider-alist'. This hook runs on EVERY save of
EVERY buffer in this whole editor's test suite, Verilog or not, C or
not -- so a mode with no formatter configured, or the variable being
nil, MUST be a true no-op, checked before anything else runs. Wrapped
in `condition-case' so a formatter crash (or a defect in this file)
can never block a save -- same discipline as `verilog-auto--maybe-on-
save' (verilog-auto.el, M40), which this is deliberately modeled on.

M104 fix round: this `condition-case' is a SECOND line of defense, not
the only one -- `before-save-hook''s own runner
\(`run_hook_by_name_with_arg', commands.rs\) already catches any error a
hook function raises and just echoes it, never letting it escape to
`save-buffer' itself. That means calling this function via an actual
`(save-buffer)' can never observe whether this `condition-case' is even
still here (removing it entirely does not turn any `save-buffer'-based
test red) -- the only way to observe it is to call
`format--maybe-on-save' directly and check that IT ALSO returns
normally, without relying on the hook runner's own safety net."
  (when (and format-on-save (assq (major-mode-internal-get) format-provider-alist))
    (condition-case err
        (format-buffer)
      (error (message "format-on-save: formatting failed: %S" err)))))

(add-hook 'before-save-hook 'format--maybe-on-save)

(provide 'format)
