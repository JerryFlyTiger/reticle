;;; switch-theme.el --- walk every theme, one per capture  -*- lexical-binding: t -*-

;; A driver for `dev/gui-drive.sh'. Loaded as the init file of a throwaway
;; HOME, so it never touches your real ~/.reticle/init.el.
;;
;;     dev/gui-drive.sh dev/drivers/switch-theme.el demo/rtl/core/alu.sv /tmp/out 3 7 11 15 19
;;
;; What it proves: a theme is a set of face values, and this project settles
;; every claim about how those values LOOK with a screenshot, never by reading
;; the hex out of themes.el. M112 raised the chrome faces (current line,
;; indent guide, scrollbar, mode line) across all five themes to stated
;; contrast bands; the default theme was verified on screen, and this driver
;; is how the other four get the same treatment. Point it at a capture
;; schedule with one entry per theme and diff the frames.
;;
;; **Fire on elapsed time, not on a tick count** -- same reason as
;; `switch-font.el', whose header explains it at length: the idle tick's own
;; interval varies between 33ms and 500ms depending on focus and outstanding
;; async work, so a count tuned against the fast cadence can silently need
;; ten times as long at the slow one, and every capture would come back
;; looking identical with nothing to explain why.
;;
;; Note the offset between this file's clock and the capture schedule's:
;; `drive--start' is set when the init file loads, while `dev/gui-drive.sh'
;; starts counting only once it has FOUND the window, which is later. Leave a
;; comfortable gap between steps rather than assuming the two agree.

(defvar drive--themes '(dracula dark light xcode vscode)
  "Themes to walk, in order. The first one is applied immediately so the
first capture is not whatever `themes.el' loaded at startup.")

(defvar drive--step 4.0
  "Seconds between theme switches. Must be larger than the gap between
capture times passed to `dev/gui-drive.sh', so each capture lands on a
settled frame rather than mid-switch.")

(defvar drive--start (float-time)
  "When this driver was loaded, in seconds.")

(defvar drive--applied -1
  "Index of the theme applied most recently, so each one fires once.")

(fset 'eshell-process-pending-all
      (lambda ()
        (let ((want (min (1- (length drive--themes))
                         (floor (/ (- (float-time) drive--start)
                                   drive--step)))))
          (when (> want drive--applied)
            (setq drive--applied want)
            (load-theme (nth want drive--themes))))))
