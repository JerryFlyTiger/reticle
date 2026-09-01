;;; treesit.el --- lisp-layer convenience over the tree-sitter primitives

;; The Rust side (crate::treesit, crate::builtins::treesit) provides the
;; low-level treesit-parser-*/treesit-node-* primitives. This file adds
;; the one thing worth writing in elisp rather than Rust: turning query
;; captures into buffer faces, the same "font-lock-lite via overlays"
;; pattern org.el already uses (see org--face-region).

(defun treesit--face-node (node face)
  (let ((ov (make-overlay (treesit-node-start node) (treesit-node-end node))))
    (overlay-put ov 'treesit-face t)
    (overlay-put ov 'face face)))

(defun treesit-remove-fontification (start end)
  (dolist (ov (overlays-in start end))
    (when (overlay-get ov 'treesit-face)
      (delete-overlay ov))))

;; QUERY-ALIST is ((QUERY-STRING . FACE) ...); each query runs against
;; NODE (typically a parser's root node) and every capture gets FACE.
(defun treesit-fontify-region (node query-alist)
  (dolist (pair query-alist)
    (dolist (cap (treesit-query-capture node (car pair)))
      (treesit--face-node (cdr cap) (cdr pair)))))

;; --- M15: background highlighting -----------------------------------
;; The engine (Rust, crate::highlight) maps tree-sitter captures onto
;; the classic font-lock faces; their colors come from the theme system
;; (themes.el, loaded after this file).
;;
;; Turning it on for Rust files used to happen via an ad hoc
;; find-file-hook lambda here; M24's rust-mode (modes.el, registered in
;; auto-mode-alist for "\\.rs\\'") calls `treesit-highlight-mode' itself
;; now, so this file no longer needs to know about file extensions.
