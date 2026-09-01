;;; init.el --- reticle config file example -*- lexical-binding: t -*-

;; Put this file at ~/.reticle/init.el and it will be loaded on startup.
;; If loading it errors out, the error message shows in the echo area at the
;; bottom of the screen, without interrupting startup.

;; --- General variable settings ---

(defvar my-name "Ada Lovelace")
(setq my-name "Ada Lovelace")

;; --- Custom commands and key bindings ---

(defun insert-signature ()
  "Insert a signature at point."
  (interactive)
  (insert (format "-- %s" my-name)))

(global-set-key "C-c s" 'insert-signature)

(defun duplicate-line ()
  "Duplicate the current line below itself."
  (interactive)
  (let ((line (buffer-substring (line-beginning-position)
                                (line-end-position))))
    (end-of-line)
    (insert "\n" line)))

(global-set-key "C-c d" 'duplicate-line)

;; --- Hook example ---

(add-hook 'org-mode-hook
          (lambda ()
            (message "org buffer ready")))

;; --- Loading your own extension file ---
;; ~/.reticle/ is already on load-path, so ~/.reticle/my-ext.el
;; can be loaded like this as long as it ends with (provide 'my-ext):
;;
;; (require 'my-ext)

;; --- Appearance ---

(set-face 'org-level-1 :foreground "#61afef" :weight 'bold)
