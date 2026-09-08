;;; org.el --- minimal org major mode, in our own elisp -*- lexical-binding: t -*-

;; File-format compatible with GNU Emacs org files. Implements headline
;; folding, TODO states, heading navigation/insertion, table alignment,
;; timestamps, links, and highlighting — all on top of the editor
;; primitives, without loading any GNU org-mode code.

(defvar org-mode-hook nil)

;; M112: org's nine faces used to be set here, hard-coded to one dark
;; palette regardless of the active theme, by a now-deleted
;; `org--set-faces' that `org-mode' used to call below. They are now
;; set per-theme in themes.el alongside every other face -- see that
;; file's top-of-file comment.

(defun org-mode ()
  "Major mode for editing org files."
  (interactive)
  (major-mode-internal-set 'org-mode)
  (let ((map (make-sparse-keymap)))
    (define-key map "TAB" 'org-cycle)
    (define-key map "M-RET" 'org-insert-heading)
    (define-key map "C-c C-t" 'org-todo)
    (define-key map "C-c C-n" 'org-next-heading)
    (define-key map "C-c C-p" 'org-previous-heading)
    (define-key map "C-c C-c" 'org-ctrl-c-ctrl-c)
    (define-key map "C-c ." 'org-time-stamp)
    (define-key map "C-c C-o" 'org-open-at-point)
    (use-local-map map))
  (org--fontify-buffer)
  (setq-local post-command-hook (list 'org--post-command))
  (run-hooks 'org-mode-hook))

;; Turn on org-mode automatically when visiting .org files (M24:
;; auto-mode-alist, defined in modes.el, loaded before this file).
(add-to-list 'auto-mode-alist '("\\.org\\'" . org-mode))

;; --- Headings ---

(defun org--line-string ()
  (buffer-substring (line-beginning-position) (line-end-position)))

(defun org--heading-level ()
  "Level of the heading on the current line, or 0 if none."
  (let* ((line (org--line-string))
         (len (length line))
         (n 0))
    (while (and (< n len) (= (aref line n) ?*))
      (setq n (1+ n)))
    (if (and (> n 0) (< n len) (= (aref line n) ?\s)) n 0)))

(defun org-cycle ()
  "Fold/unfold the subtree at point, or insert a tab elsewhere."
  (interactive)
  (if (> (org--heading-level) 0)
      (org--toggle-fold)
    (insert "\t")))

(defun org--subtree-end ()
  "End position of the current heading's subtree."
  (let ((level (org--heading-level))
        (end (point-max))
        (scanning t))
    (save-excursion
      (while scanning
        (if (> (forward-line 1) 0)
            (setq scanning nil)
          (let ((l (org--heading-level)))
            (when (and (> l 0) (<= l level))
              (setq end (1- (point)))
              (setq scanning nil))))))
    end))

(defun org--fold-overlay-at-eol ()
  (let ((eol (line-end-position))
        (found nil))
    (dolist (ov (overlays-in eol (1+ eol)))
      (when (overlay-get ov 'org-fold)
        (setq found ov)))
    found))

(defun org--toggle-fold ()
  (save-excursion
    (beginning-of-line)
    (let ((existing (org--fold-overlay-at-eol)))
      (if existing
          (delete-overlay existing)
        (let ((eol (line-end-position))
              (end (org--subtree-end)))
          (when (> end eol)
            (let ((ov (make-overlay eol end)))
              (overlay-put ov 'invisible t)
              (overlay-put ov 'org-fold t))))))))

;; --- TODO states ---

(defun org--word-at-p (word)
  "Non-nil if WORD followed by a space starts at point."
  (let* ((start (point))
         (end (+ start (length word))))
    (and (<= end (point-max))
         (string= (buffer-substring start end) word)
         (eq (char-after end) ?\s))))

(defun org-todo ()
  "Cycle the heading through no keyword → TODO → DONE."
  (interactive)
  (save-excursion
    (beginning-of-line)
    (let ((level (org--heading-level)))
      (when (> level 0)
        (let ((after (+ (line-beginning-position) level 1)))
          (goto-char after)
          (cond
           ((org--word-at-p "TODO")
            (delete-region after (+ after 5))
            (goto-char after)
            (insert "DONE "))
           ((org--word-at-p "DONE")
            (delete-region after (+ after 5)))
           (t (insert "TODO ")))))))
  (org--fontify-line))

;; --- Heading insertion and navigation ---

(defun org-insert-heading ()
  "Insert a new heading below, at the same level."
  (interactive)
  (let ((level (save-excursion (beginning-of-line) (org--heading-level))))
    (when (= level 0) (setq level 1))
    (end-of-line)
    (insert "\n" (make-string level ?*) " ")))

(defun org--find-heading (dir)
  (save-excursion
    (let ((found nil) (moving t))
      (while moving
        (if (> (forward-line dir) 0)
            (setq moving nil)
          (when (> (org--heading-level) 0)
            (setq found (point))
            (setq moving nil))))
      found)))

(defun org-next-heading ()
  (interactive)
  (let ((pos (org--find-heading 1)))
    (if pos (goto-char pos) (message "No next heading"))))

(defun org-previous-heading ()
  (interactive)
  (let ((pos (org--find-heading -1)))
    (if pos (goto-char pos) (message "No previous heading"))))

;; --- Timestamps ---

(defun org-time-stamp ()
  "Insert an active timestamp for today."
  (interactive)
  (insert (format "<%s>" (format-time-string "%Y-%m-%d %a"))))

;; --- Links ---

(defun org--link-at (line col)
  "Link target of the [[...]] containing COL in LINE, or nil."
  (let ((result nil) (from 0) (scanning t))
    (while scanning
      (let ((open (string-search "[[" line from)))
        (if (null open)
            (setq scanning nil)
          (let ((close (string-search "]]" line (+ open 2))))
            (if (null close)
                (setq scanning nil)
              (if (and (>= col open) (<= col (1+ close)))
                  (progn
                    (let* ((inner (substring line (+ open 2) close))
                           (bar (string-search "][" inner)))
                      (setq result (if bar (substring inner 0 bar) inner)))
                    (setq scanning nil))
                (setq from (+ close 2))))))))
    result))

(defun org-open-at-point ()
  "Open the link at point (file links and web links)."
  (interactive)
  (let* ((bol (line-beginning-position))
         (line (org--line-string))
         (col (- (point) bol))
         (target (org--link-at line col)))
    (cond
     ((null target)
      (message "No link at point"))
     ((or (string-prefix-p "http://" target)
          (string-prefix-p "https://" target))
      (browse-url target)
      (message "Opening %s" target))
     (t
      (find-file-internal
       (if (string-prefix-p "file:" target) (substring target 5) target))))))

;; --- Tables ---

(defun org--table-line-p ()
  (string-prefix-p "|" (org--line-string)))

(defun org--table-bounds ()
  "Start/end of the contiguous table around point: (START . END)."
  (save-excursion
    (beginning-of-line)
    (while (and (not (bobp))
                (save-excursion (forward-line -1) (org--table-line-p)))
      (forward-line -1))
    (let ((start (line-beginning-position)))
      (while (and (save-excursion (= (forward-line 1) 0))
                  (save-excursion (forward-line 1) (org--table-line-p)))
        (forward-line 1))
      (cons start (line-end-position)))))

(defun org--split-cells (tline)
  (let ((inner tline))
    (when (string-prefix-p "|" inner)
      (setq inner (substring inner 1)))
    (when (string-suffix-p "|" inner)
      (setq inner (substring inner 0 (1- (length inner)))))
    (mapcar #'string-trim (split-string inner "|"))))

(defun org--pad (s width)
  (concat s (make-string (- width (string-width s)) ?\s)))

(defun org--format-row (cells widths)
  (let ((parts nil) (idx 0) (n (length widths)))
    (while (< idx n)
      (let ((cell (or (nth idx cells) "")))
        (push (org--pad cell (aref widths idx)) parts))
      (setq idx (1+ idx)))
    (concat "| " (string-join (nreverse parts) " | ") " |")))

(defun org--separator-line (widths)
  (let ((parts nil) (idx 0) (n (length widths)))
    (while (< idx n)
      (push (make-string (+ (aref widths idx) 2) ?-) parts)
      (setq idx (1+ idx)))
    (concat "|" (string-join (nreverse parts) "+") "|")))

(defun org-table-align ()
  "Align the table around point, using display widths (CJK-aware)."
  (interactive)
  (let* ((bounds (org--table-bounds))
         (start (car bounds))
         (end (cdr bounds))
         (text (buffer-substring start end))
         (lines (split-string text "\n"))
         (rows nil)
         (ncols 0))
    (dolist (line lines)
      (let ((tline (string-trim line)))
        (if (string-prefix-p "|-" tline)
            (push (cons t nil) rows)
          (let ((cells (org--split-cells tline)))
            (when (> (length cells) ncols)
              (setq ncols (length cells)))
            (push (cons nil cells) rows)))))
    (setq rows (nreverse rows))
    (when (> ncols 0)
      (let ((widths (make-vector ncols 0)))
        (dolist (row rows)
          (unless (car row)
            (let ((idx 0))
              (dolist (cell (cdr row))
                (when (> (string-width cell) (aref widths idx))
                  (aset widths idx (string-width cell)))
                (setq idx (1+ idx))))))
        (let ((out nil))
          (dolist (row rows)
            (if (car row)
                (push (org--separator-line widths) out)
              (push (org--format-row (cdr row) widths) out)))
          (delete-region start end)
          (goto-char start)
          (insert (string-join (nreverse out) "\n"))
          (goto-char start)))))
  (org--fontify-buffer))

(defun org-ctrl-c-ctrl-c ()
  "Realign the table at point, or refontify the buffer."
  (interactive)
  (if (org--table-line-p)
      (org-table-align)
    (org--fontify-buffer)))

;; --- Highlighting (overlay-based font-lock lite) ---

(defun org--face-region (start end face)
  (let ((ov (make-overlay start end)))
    (overlay-put ov 'org-face t)
    (overlay-put ov 'face face)))

(defun org--remove-face-overlays (start end)
  (dolist (ov (overlays-in start end))
    (when (overlay-get ov 'org-face)
      (delete-overlay ov))))

(defun org--level-face (level)
  (nth (mod (1- level) 4)
       '(org-level-1 org-level-2 org-level-3 org-level-4)))

(defun org--scan-delimited (line bol open close face)
  "Fontify OPEN...CLOSE spans of LINE (BOL = its buffer position)."
  (let ((from 0) (scanning t))
    (while scanning
      (let ((o (string-search open line from)))
        (if (null o)
            (setq scanning nil)
          (let ((c (string-search close line (+ o (length open)))))
            (if (null c)
                (setq scanning nil)
              (org--face-region (+ bol o) (+ bol c (length close)) face)
              (setq from (+ c (length close))))))))))

(defun org--fontify-line ()
  (save-excursion
    (let* ((bol (line-beginning-position))
           (eol (line-end-position))
           (line (buffer-substring bol eol)))
      (org--remove-face-overlays bol eol)
      (let ((len (length line)) (n 0))
        (while (and (< n len) (= (aref line n) ?*))
          (setq n (1+ n)))
        (when (and (> n 0) (< n len) (= (aref line n) ?\s))
          (org--face-region bol eol (org--level-face n))
          (let ((kw (+ bol n 1)))
            (save-excursion
              (goto-char kw)
              (cond
               ((org--word-at-p "TODO")
                (org--face-region kw (+ kw 4) 'org-todo))
               ((org--word-at-p "DONE")
                (org--face-region kw (+ kw 4) 'org-done)))))))
      (when (string-prefix-p "|" line)
        (org--face-region bol eol 'org-table))
      (org--scan-delimited line bol "<" ">" 'org-date)
      (org--scan-delimited line bol "[[" "]]" 'org-link))))

(defun org--fontify-buffer ()
  (save-excursion
    (goto-char (point-min))
    (org--fontify-line)
    (while (= (forward-line 1) 0)
      (org--fontify-line))))

(defun org--post-command ()
  (when (eq (major-mode-internal-get) 'org-mode)
    (org--fontify-line)))

(provide 'org)
