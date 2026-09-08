;;; set-opacity.el --- change the window opacity mid-run, with no keystroke  -*- lexical-binding: t -*-

;; A driver for `dev/gui-drive.sh'. Loaded as the init file of a throwaway
;; HOME, so it never touches your real ~/.reticle/init.el.
;;
;;     dev/gui-drive.sh dev/drivers/set-opacity.el demo/rtl/core/alu.sv /tmp/out 1 4 8 12
;;
;; What it proves: `gui-opacity' is re-read every frame (M111), so changing
;; it at runtime must retint the window's fill and every per-cell
;; background on the very next frame, with foreground text staying fully
;; opaque throughout.

;; Same trick as `switch-font.el' (read that file's header first): the
;; editor calls a handful of elisp functions on every idle tick
;; (`idle_tick', crates/core/src/lib.rs). Redefining one of them gives you a
;; timer built entirely out of the editor's own public elisp surface.
;; `eshell-process-pending-all' is chosen for the same reason switch-font.el
;; picked it: its only production call site is that tick, and nothing in a
;; driven session starts an eshell, so replacing it changes nothing else
;; about the run.
;;
;; **Fire on elapsed time, not on a tick count** -- same reasoning as
;; `switch-font.el': the idle tick's own interval is not fixed (33ms while
;; focused and animating, 100ms with async work outstanding, 500ms
;; otherwise), and a window driven by this script may never take OS focus,
;; so a count tuned against the fast cadence can silently need far longer at
;; the slow one. `float-time' does not care which cadence the tick is
;; running at.

(setq gui-opacity 100)

(defvar drive--start (float-time)
  "When this driver was loaded, in seconds.")

(defvar drive--switch-after 5.0
  "Drop the opacity this many seconds after startup. Must sit strictly
between two entries of the capture schedule passed to `dev/gui-drive.sh',
so that one capture lands before the change and one after.")

(defvar drive--switched nil
  "Set once the drop has fired, so it happens exactly once.")

(fset 'eshell-process-pending-all
      (lambda ()
        (unless drive--switched
          (when (> (- (float-time) drive--start) drive--switch-after)
            (setq drive--switched t)
            ;; Clearly translucent -- well below 100, well above the
            ;; `clamp_opacity' floor of 20 -- so the drop is unambiguous in
            ;; a pixel comparison between the bracketing captures.
            (setq gui-opacity 40)))))
