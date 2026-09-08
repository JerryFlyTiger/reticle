;;; show-paren.el --- move point onto a bracket, mid-run  -*- lexical-binding: t -*-

;; A driver for `dev/gui-drive.sh'. Loaded as the init file of a throwaway
;; HOME, so it never touches your real ~/.reticle/init.el.
;;
;;     dev/gui-drive.sh dev/drivers/show-paren.el demo/rtl/core/alu.sv /tmp/out 3 9
;;
;; Why a driver is needed at all: `show-paren-mode' only draws when point is
;; adjacent to a bracket, and a file opens with point at position 1. A plain
;; `dev/gui-shot.sh' capture can therefore never show this feature -- the
;; screenshot would be evidence of nothing. The first capture here is the
;; control (point at the top of the file, no highlight) and the second is the
;; test (point on a bracket, the pair lit).
;;
;; The target is `module alu #(' -- the opener at end of line 6, whose match is
;; the `)' that closes the parameter list further down, so a single frame shows
;; both members of the pair and the distance between them.
;;
;; **Fire on elapsed time, not on a tick count**, for the reason
;; `dev/drivers/switch-font.el' explains at length: the idle tick's interval
;; varies from 33ms to 500ms with focus and outstanding async work, so a count
;; tuned against the fast cadence silently needs ten times as long at the slow
;; one, and every capture comes back looking the same with nothing to explain
;; why.
;;
;; Note also that highlighting depends on the tree-sitter parse having landed,
;; which happens on a worker thread -- leave enough time before the second
;; capture that a slow first parse is not mistaken for a missing feature.

(defvar drive--start (float-time)
  "When this driver was loaded, in seconds.")

(defvar drive--move-after 5.0
  "Move point onto the bracket this many seconds after startup. Must sit
strictly between two entries of the capture schedule, so one capture
lands before the move and one after.")

(defvar drive--moved nil
  "Set once the move has fired, so it happens exactly once.")

(fset 'eshell-process-pending-all
      (lambda ()
        (unless drive--moved
          (when (> (- (float-time) drive--start) drive--move-after)
            (setq drive--moved t)
            (goto-char (point-min))
            (forward-line 5)
            (end-of-line)
            ;; `end-of-line' leaves point AFTER the trailing `(' ; step back so
            ;; point sits immediately before the opener, which is one of the
            ;; two positions GNU highlights from.
            (backward-char 1)))))
