;;; expand-region.el --- semantic selection growing/shrinking -*- lexical-binding: t -*-

;; M101: `expand-region' grows the active region (or starts one at
;; point) outward one semantic step at a time -- word, then line
;; content, then paragraph, then whole buffer, interleaved with however
;; many tree-sitter ancestor nodes fall strictly between those, from
;; smallest to largest. `contract-region' walks the same steps back in.
;; Named without the `er/' prefix the reference implementation (Magnar
;; Sveen's expand-region.el for GNU Emacs) uses -- this project's own
;; commands don't carry a namespace prefix (compare `dabbrev-expand',
;; not `dabbrev/expand').
;;
;; --- Algorithm -------------------------------------------------------
;;
;; One "expansion" = build a set of candidate ranges around the anchor
;; (point, or the region's start if a region is active) -> keep only
;; the candidates that strictly contain the current region (superset,
;; not equal) -> take the smallest survivor. That single filter step is
;; simultaneously how duplicate-range tree-sitter ancestors get skipped:
;; a `simple_identifier' and the `hierarchical_identifier'/`primary'/
;; `expression' wrapping it at the exact same byte range are common in
;; the SystemVerilog grammar (confirmed by hand against
;; verible-verilog's grammar on real RTL: four same-range layers in a
;; row above a plain identifier is unremarkable), and "strictly larger"
;; throws all but the outermost of them away without needing a
;; type-name table.
;;
;; The text-ladder candidates (word/line/paragraph/buffer) exist
;; independently of tree-sitter and always run, so a buffer with no
;; grammar (`treesit--buffer-language' nil, see modes.el) still gets a
;; four-step ladder. They also fix a rough edge tree-sitter has on its
;; own: `treesit-node-at' on a position inside a line's leading
;; whitespace returns the smallest node whose byte range happens to
;; cover it, which in a block-structured language is often the entire
;; enclosing construct (verified: landing on the indentation before a
;; statement inside a SystemVerilog `case' arm handed back the whole
;; `module_declaration'). The line-content candidate is always
;; available and almost always smaller, so it wins that comparison and
;; the first expansion from indentation feels like "select this line",
;; not "select everything".
;;
;; --- History / freshness, without `last-command' ----------------------
;;
;; Real Emacs's `expand-region' tells "the user is still repeating the
;; same expand/contract sequence" from "this is a fresh call" by
;; comparing against `last-command'. This editor's elisp layer has no
;; way to read `last-command'/`this-command' (the Rust dispatcher tracks
;; them internally but never exposes them to elisp) -- deliberately not
;; plumbed through for this feature; see the project CLAUDE.md for why
;; that boundary is being left alone. Instead, freshness is judged
;; structurally: `expand-region--last' remembers the exact range this
;; buffer's most recent expand/contract produced, and a new call only
;; trusts `expand-region--history' when the region right now still
;; equals that remembered range. Anything else -- the user moved point,
;; typed, ran a different command that touched the region -- looks
;; exactly like "the sequence was broken" and starts over with an empty
;; history. This is slightly more conservative than `last-command'
;; (a command that leaves the region untouched but isn't itself
;; expand/contract will also look "unbroken" here, where real Emacs
;; would reset), which is the safe direction for a heuristic to be wrong
;; in: at worst one contract goes one step further back than expected,
;; never never a corrupted or nonsensical history.
;;
;; --- Known gaps --------------------------------------------------------
;;
;; - No subword step: on `operand_a_i' the ladder never stops at just
;;   `operand' the way `er/expand-region''s subword mode does.
;; - No per-language "which node types matter" table -- every
;;   tree-sitter ancestor is a candidate, indistinguishable by type;
;;   de-duplication is purely by range, not by skipping "boring" wrapper
;;   node types on purpose.
;; - `C-=' / `C--' depend on the terminal actually sending a
;;   distinguishable byte sequence for `Ctrl' held with `=' / `-' -- this
;;   project does not negotiate the kitty keyboard protocol or any other
;;   disambiguation extension, so on a plain terminal those two may not
;;   reach the TUI at all. `M-=' / `M--' are bound as the same commands
;;   for exactly this reason: a bare ESC prefix (`ESC =' / `ESC -') is
;;   what every terminal sends for `Alt-=' / `Alt--', unconditionally.
;; - Every expansion re-parses the whole buffer from scratch (tree-sitter
;;   has no incremental cache here, see treesit.rs's own header) -- O(file
;;   size) per keypress, unnoticeable on realistic source files, not
;;   validated on anything huge.

(defvar expand-region--history nil
  "Buffer-local. List of (START . END) ranges, most recent expansion
first -- the ranges `contract-region' pops back through. Reset to nil
whenever a fresh expansion sequence starts (see `expand-region--last').")

(defvar expand-region--last nil
  "Buffer-local. (START . END), the range the most recent
`expand-region'/`contract-region' call in this buffer produced, or nil.
Comparing the current region against this is how this file tells
\"still the same sequence\" from \"fresh start\" without `last-command'
-- see the file header.")

;; --- Candidate builders ------------------------------------------------

(defun expand-region--ident-char-p (c)
  "Non-nil if C is a plain-ASCII identifier constituent: letters,
digits, `_'. Deliberately not `verilog-complete--ident-char-p' (which
also allows `$', a SystemVerilog-only rule) -- this file has no
language of its own to assume."
  (and c (or (and (>= c ?a) (<= c ?z))
             (and (>= c ?A) (<= c ?Z))
             (and (>= c ?0) (<= c ?9))
             (= c ?_))))

(defun expand-region--whitespace-char-p (c)
  "Non-nil if C is a plain space or tab (the \"blank line\"/\"line
content\" definition this file uses -- not general whitespace, no
newlines involved since these helpers are always applied within a
single line's text)."
  (or (= c ?\s) (= c ?\t)))

(defun expand-region--word-candidate (anchor)
  "(START . END) of the run of `expand-region--ident-char-p' characters
touching ANCHOR, or nil if ANCHOR isn't adjacent to any identifier
character at all."
  (when (or (expand-region--ident-char-p (char-before anchor))
            (expand-region--ident-char-p (char-after anchor)))
    (let ((beg anchor) (end anchor))
      (while (and (> beg (point-min))
                  (expand-region--ident-char-p (char-before beg)))
        (setq beg (1- beg)))
      (while (and (< end (point-max))
                  (expand-region--ident-char-p (char-after end)))
        (setq end (1+ end)))
      (cons beg end))))

(defun expand-region--line-bounds (pos)
  "(BEG . END) of the line containing POS -- `line-beginning-position'/
`line-end-position' only look at the CURRENT point (this editor's
implementation, unlike GNU Emacs's, ignores their optional N argument
entirely), so this temporarily moves point to POS via `save-excursion'
rather than duplicating the position arithmetic."
  (save-excursion
    (goto-char pos)
    (cons (line-beginning-position) (line-end-position))))

(defun expand-region--blank-line-p (pos)
  "Non-nil if the line containing POS has nothing on it but spaces/tabs
(or is empty)."
  (let ((bounds (expand-region--line-bounds pos)))
    (string= (string-trim (buffer-substring (car bounds) (cdr bounds))) "")))

(defun expand-region--line-content-candidate (anchor)
  "(START . END) of ANCHOR's line, trimmed to its first/last non-blank
character; if the whole line is blank, the untrimmed line bounds
instead (there is nothing to trim to).

Widened to include ANCHOR itself if trimming pushed START past it --
this is the fix for the case documented in the file header (point
sitting in a line's leading whitespace): trimming alone can produce a
range that excludes the very position this whole file is expanding
FROM, which would make this candidate fail `expand-region--pick''s
\"must contain the current region\" filter and silently drop out,
leaving nothing but a giant tree-sitter node (or the whole buffer) to
win instead -- exactly the bad outcome this candidate exists to avoid.
Every OTHER candidate in this file is built by growing outward from
ANCHOR and so contains it by construction; this is the one place that
starts from trimmed text instead and needed an explicit fix-up."
  (let* ((bounds (expand-region--line-bounds anchor))
         (beg (car bounds))
         (end (cdr bounds))
         (text (buffer-substring beg end))
         (len (length text)))
    (if (= len 0)
        (cons beg end)
      (let ((first 0) (last (1- len)))
        (while (and (<= first last)
                    (expand-region--whitespace-char-p (aref text first)))
          (setq first (1+ first)))
        (while (and (<= first last)
                    (expand-region--whitespace-char-p (aref text last)))
          (setq last (1- last)))
        (if (> first last)
            (cons beg end) ; all blank
          (cons (min (+ beg first) anchor) (max (+ beg last 1) anchor)))))))

(defun expand-region--paragraph-candidate (anchor)
  "(START . END) of the maximal run of consecutive lines, including
ANCHOR's own line, that are all blank or all non-blank the same way
ANCHOR's line is."
  (let* ((blank (expand-region--blank-line-p anchor))
         (bounds (expand-region--line-bounds anchor))
         (beg (car bounds))
         (end (cdr bounds)))
    (while (and (> beg (point-min))
                (equal (expand-region--blank-line-p (1- beg)) blank))
      (setq beg (car (expand-region--line-bounds (1- beg)))))
    (while (and (< end (point-max))
                (equal (expand-region--blank-line-p (1+ end)) blank))
      (setq end (cdr (expand-region--line-bounds (1+ end)))))
    (cons beg end)))

;; Walk-to-root cap: a pathological/very deep parse tree shouldn't be
;; able to hang this on a bad file. 200 is far beyond any real grammar's
;; nesting depth for a single statement.
(defconst expand-region--treesit-max-depth 200)

(defun expand-region--treesit-candidates (anchor)
  "List of (START . END) for every tree-sitter node from the one
covering ANCHOR up through the parse root, inclusive -- or nil if this
buffer has no `treesit--buffer-language' (modes.el) or that language
isn't actually available. `ERROR' nodes are collected like any other:
a syntax tree covering a temporarily-broken buffer (mid-edit,
unbalanced) is still useful for this feature, unlike, say,
indentation, which gives up on them."
  (when (and treesit--buffer-language
             (treesit-language-available-p treesit--buffer-language))
    (let* ((parser (treesit-parser-create treesit--buffer-language))
           (node (treesit-node-at anchor parser))
           (result nil)
           (depth 0))
      (while (and node (< depth expand-region--treesit-max-depth))
        (push (cons (treesit-node-start node) (treesit-node-end node)) result)
        (setq node (treesit-node-parent node))
        (setq depth (1+ depth)))
      result)))

(defun expand-region--candidates (anchor)
  "Every candidate range for an expansion anchored at ANCHOR."
  (let ((word (expand-region--word-candidate anchor)))
    (append (if word (list word) nil)
            (list (expand-region--line-content-candidate anchor))
            (list (expand-region--paragraph-candidate anchor))
            (list (cons (point-min) (point-max)))
            (expand-region--treesit-candidates anchor))))

(defun expand-region--pick (cur candidates)
  "Smallest member of CANDIDATES that strictly contains CUR (both
(START . END) conses), or nil if none does."
  (let ((cur-beg (car cur)) (cur-end (cdr cur)) (best nil))
    (dolist (c candidates)
      (let ((beg (car c)) (end (cdr c)))
        (when (and (<= beg cur-beg) (>= end cur-end)
                   (or (< beg cur-beg) (> end cur-end)))
          (when (or (null best) (< (- end beg) (- (cdr best) (car best))))
            (setq best c)))))
    best))

;; --- Commands ------------------------------------------------------------

(defun expand-region ()
  "Grow the active region (or start one at point) to the next larger
semantic unit: identifier word, then line content, then paragraph,
then the whole buffer, with any tree-sitter ancestor nodes that fall
strictly in between mixed in by size. Repeating grows further; see the
file header for how a repeated call is told apart from a fresh one,
and `contract-region' for undoing a step."
  (interactive)
  (let* ((active (region-active-p))
         (cur (if active (cons (region-beginning) (region-end))
                (cons (point) (point))))
         (anchor (if active (region-beginning) (point))))
    (unless (and active expand-region--last (equal expand-region--last cur))
      (setq-local expand-region--history nil))
    (let ((next (expand-region--pick cur (expand-region--candidates anchor))))
      (if (null next)
          (message "expand-region: no further expansion")
        (setq-local expand-region--history (cons cur expand-region--history))
        (setq-local expand-region--last next)
        (set-mark (car next))
        (goto-char (cdr next))))))

(defun contract-region ()
  "Undo the most recent `expand-region' step in this buffer, provided
the region hasn't changed since (see the file header). What happens on
the last step is decided by the range at the bottom of the history,
i.e. whatever the sequence grew out of: if that range is zero-width --
a bare point, which is the usual case, and also the case where a mark
was set but never moved away from -- the region is deactivated
entirely, back to no selection at all. If it has width, meaning the
user had selected something themselves before ever calling
`expand-region', the last step lands back on that original selection:
there is no \"further back\" than the range the sequence grew from."
  (interactive)
  (let ((cur (if (region-active-p) (cons (region-beginning) (region-end)) nil)))
    (if (not (and cur expand-region--last (equal expand-region--last cur)
                  expand-region--history))
        (message "contract-region: nothing to contract")
      (let ((prev (pop expand-region--history)))
        ;; `pop' already did `(setq expand-region--history ...)'; safe as
        ;; a plain `setq' here because this branch only runs when
        ;; `expand-region--history' is non-nil, which only happens once
        ;; `expand-region' has already made it buffer-local in this
        ;; buffer via `setq-local'.
        (if (= (car prev) (cdr prev))
            (progn
              (deactivate-mark)
              (goto-char (car prev))
              (setq-local expand-region--last nil))
          (set-mark (car prev))
          (goto-char (cdr prev))
          (setq-local expand-region--last prev))))))

;; --- Key bindings --------------------------------------------------------
;;
;; `C-=' / `C--' match the reference implementation's own bindings (GNU
;; Emacs, and reachable from a GUI frontend here too). `M-=' / `M--' are
;; the terminal-safe equivalents: a plain tty generally can't tell
;; `Ctrl-=' apart from a bare `=' keypress (no distinguishing byte
;; sequence without a protocol extension this project doesn't
;; negotiate -- see treesit gaps above), but `Alt-=' / `Alt--' always
;; arrive as `ESC =' / `ESC -', which every terminal sends unconditionally.
;; All four were unbound before this file (grepped `global-set-key' across
;; every `.el' file here).
(global-set-key "C-=" 'expand-region)
(global-set-key "M-=" 'expand-region)
(global-set-key "C--" 'contract-region)
(global-set-key "M--" 'contract-region)

(provide 'expand-region)
