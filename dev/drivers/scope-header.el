;;; scope-header.el --- scroll into a module, so the sticky header appears  -*- lexical-binding: t -*-

;; A driver for `dev/gui-drive.sh' (M118).
;;
;;     dev/gui-drive.sh dev/drivers/scope-header.el demo/rtl/top/soc_top.sv /tmp/out 2 8
;;
;; What it proves: the sticky scope header is not a startup decoration. At
;; capture 1 the buffer is at its top, every enclosing construct's own line is
;; still on screen, and NO header row is drawn. Between the captures the driver
;; walks point deep into the module, so the `module' line (and whatever else
;; encloses point) scrolls off the top -- and capture 2 must show those lines
;; pinned at the top of the window, in the `scope-header' face, over the text
;; that would otherwise be there.
;;
;; Two frames are the point. A single screenshot of a header cannot distinguish
;; "pinned because it scrolled off" from "a banner that is always drawn"; the
;; before/after pair can, and `dev/gui-drive.sh' prints the pixel difference
;; between consecutive frames for exactly this kind of claim.
;;
;; The timer trick is `dev/drivers/switch-font.el''s -- see its header for why
;; a redefined `idle_tick' callee is used instead of keystrokes, and why the
;; firing condition is elapsed time rather than a tick count. The same caveat
;; applies here: the tick's cadence is not fixed, so `float-time' decides.

(defvar drive--start (float-time)
  "When this driver was loaded, in seconds.")

(defvar drive--scroll-after 5.0
  "Walk point into the module this many seconds after startup. Must sit
strictly between two entries of the capture schedule passed to
`dev/gui-drive.sh', so one capture lands before the scroll and one after.")

(defvar drive--scrolled nil
  "Set once the scroll has fired, so it happens exactly once.")

(fset 'eshell-process-pending-all
      (lambda ()
        (unless drive--scrolled
          (when (> (- (float-time) drive--start) drive--scroll-after)
            (setq drive--scrolled t)
            ;; Deep enough that the module header line is well above the
            ;; window, shallow enough to stay inside the module in a
            ;; 139-line file.
            (goto-char (point-min))
            (forward-line 110)
            (end-of-line)))))
