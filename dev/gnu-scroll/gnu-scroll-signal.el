;;; gnu-scroll-signal.el --- does a signalled scroll command move anything?
;; Added after a cold read of the M138 record noticed that the earlier probes'
;; `try' helper printed only the SIGNAL line on error, so "signals and moves
;; nothing" had no evidence for the "moves nothing" half.
;;
;; Run in BATCH mode (`OUT=out.txt emacs -Q --batch -l gnu-scroll-signal.el`),
;; and read only the "-> after signal" lines: whether a signalled command left
;; window-start and point alone does not depend on redisplay. Everything else
;; batch prints is subject to the warning in README.md (the "C-v from 82" line
;; scrolls instead of signalling, exactly the kind of number not to trust).
;; A real-frame run of this file on 2026-09-14 wrote nothing at all and the
;; cause was not chased; the batch answer is enough for this question.
(setq out-file (getenv "OUT"))
(defun out (s) (append-to-file s nil out-file))
(defun st (tag) (redisplay t) (out (format "%-34s ws=%3d pt=%3d wend=%3d h=%d\n" tag (line-number-at-pos (window-start)) (line-number-at-pos (point)) (line-number-at-pos (window-end nil t)) (window-text-height))))
(defun try (tag form) (condition-case e (progn (eval form) (st tag)) (error (out (format "%-34s SIGNAL: %S\n" tag e)) (st (concat tag " -> after signal")))))
(defun setws (n) (goto-char (point-min)) (forward-line (1- n)) (set-window-start nil (point)) (redisplay t))
(set-frame-size (selected-frame) 80 24)
(switch-to-buffer "*t*")
(dotimes (i 100) (insert (format "line %d\n" (1+ i))))
(goto-char (point-min)) (redisplay t)
(setws 70) (goto-char (point-min)) (forward-line 70) (st "ws 70 pt 71")
(try "C-v arg 50 from ws70" '(scroll-up-command 50))
(setws 82) (st "ws 82")
(try "C-v from 82" '(scroll-up-command))
(setws 1) (goto-char (point-min)) (forward-line 5) (st "ws 1 pt 6")
(try "M-v at top" '(scroll-down-command))
(kill-emacs 0)
