;;; switch-font.el --- change the font mid-run, with no keystroke  -*- lexical-binding: t -*-

;; A driver for `dev/gui-drive.sh'. Loaded as the init file of a throwaway
;; HOME, so it never touches your real ~/.reticle/init.el.
;;
;;     dev/gui-drive.sh dev/drivers/switch-font.el demo/rtl/core/alu.sv /tmp/out 1 4 8 12
;;
;; What it proves: `gui-font' is re-read every frame, so changing it at
;; runtime must rebuild the font atlas, re-measure the character cell, redo
;; the grid's columns and rows, and drop the shaping caches (glyph ids belong
;; to the face that produced them -- a stale cache draws the OLD font's
;; glyphs). Selecting the same font at startup exercises none of that.
;;
;; Measured 2026-09-04 with this driver: the frame captured after the switch is
;; **pixel-for-pixel identical** to a startup-selected Fira Code frame (0
;; differing pixels), and both differ from JetBrains Mono by 107,376 pixels.
;; That is the strongest form of the claim -- not "the switch mostly works" but
;; "it leaves nothing of the old font behind".

;; The trick, and the reason this file exists rather than a keystroke script:
;; the editor calls a handful of elisp functions on every idle tick
;; (`idle_tick', crates/core/src/lib.rs). Redefining one of them gives you a
;; timer built entirely out of the editor's own public elisp surface. Any of
;; them would do; `eshell-process-pending-all' is chosen because its only
;; production call site is that tick and nothing in a driven session starts an
;; eshell, so replacing it changes nothing else about the run.
;;
;; The sandbox covers the configuration, not the file you open: see
;; `dev/gui-drive.sh`'s header. A driver that edits and saves a buffer writes
;; to the real file.
;;
;; **Fire on elapsed time, not on a tick count.** The idle tick's own interval
;; is not fixed -- `crates/frontend-gui/src/lib.rs' asks for a repaint at 33ms
;; while focused and animating, 100ms while there is async work outstanding,
;; and 500ms otherwise. A window driven by this script may never take OS focus,
;; so a count tuned against the fast cadence can silently need 50 seconds at
;; the slow one, and every capture in the schedule would come back looking the
;; same with no error to explain why. `float-time' does not care which cadence
;; the tick is running at.
;;
;; If a future milestone gives this editor a real timer (`run-with-timer'), use
;; that instead and delete this comment.

(setq gui-font 'jetbrains-mono)

(defvar drive--start (float-time)
  "When this driver was loaded, in seconds.")

(defvar drive--switch-after 5.0
  "Switch the font this many seconds after startup. Must sit strictly
between two entries of the capture schedule passed to `dev/gui-drive.sh',
so that one capture lands before the switch and one after.")

(defvar drive--switched nil
  "Set once the switch has fired, so it happens exactly once.")

(fset 'eshell-process-pending-all
      (lambda ()
        (unless drive--switched
          (when (> (- (float-time) drive--start) drive--switch-after)
            (setq drive--switched t)
            (setq gui-font 'fira-code)))))
