;;; eshell.el --- shell buffer (M23), GNU eshell style -*- lexical-binding: t -*-

;; A *eshell* buffer: the prompt shows the cwd; RET runs the line.
;; Lines starting with "(" evaluate as elisp (ielm-style, multi-line
;; continuation included); cd/pwd/clear are built in; anything else is
;; spawned through `sh -c` asynchronously — output streams into the
;; buffer from the idle pump, typing never blocks. C-c C-c kills the
;; running process; M-p/M-n walk the history.

(defvar eshell-prompt-suffix " $ ")

;; Position right after the current prompt (input region start).
;; Global like ielm's — v1 supports one *eshell* buffer.
(defvar eshell--input-start 1)

;; Live processes: list of (BUFFER-NAME . PROC).
(defvar eshell--procs nil)

;; Command history, most recent first; idx 0 = editing a fresh line.
(defvar eshell--history nil)
(defvar eshell--hist-idx 0)

(defun eshell--abbrev-dir (dir)
  ;; M69: `string-prefix-p' alone has no separator boundary -- a sibling
  ;; directory like "~2" (i.e. HOME + "2") used to get mis-abbreviated to
  ;; "~2" -> "~" + "2/..." -- so require the match to be either exactly
  ;; `home' or `home' followed by "/".
  (let ((home (expand-file-name "~")))
    (cond
     ((string= dir home) "~")
     ((string-prefix-p (concat home "/") dir)
      (concat "~/" (substring dir (1+ (length home)))))
     (t dir))))

(defun eshell--prompt-string ()
  (concat (eshell--abbrev-dir (default-directory)) eshell-prompt-suffix))

(defun eshell--insert-prompt ()
  (goto-char (point-max))
  (insert (eshell--prompt-string))
  (setq eshell--input-start (point-max))
  ;; M71: this runs after the welcome message and after every command
  ;; (including the `eshell--eval-elisp' `incomplete' branch, which
  ;; clears the flag itself right after its own `insert' since it does
  ;; NOT call this function -- see that branch's own comment), so it
  ;; clears the flag left by THIS function's own `insert' above.
  ;; *eshell* has no file to save -- its `*' would only ever mean "you
  ;; typed a command", never "you have unsaved work".
  ;;
  ;; This is NOT a blanket guarantee the buffer never shows `*':
  ;; `insert' is not a no-op through `check_writable' (*eshell* is
  ;; writable, unlike dired/*Help*), so every character the user types
  ;; between one prompt and the next (before RET) sets the flag back to
  ;; true via `Buffer::insert' (buffer.rs:311) -- deliberately
  ;; unaddressed here; catching it would mean intercepting every
  ;; self-insert. Also, `C-x u'/`C-/'/`C-_' (undo) reach
  ;; `Buffer::undo_step_from' (buffer.rs:670), which sets the flag
  ;; unconditionally once entered. Getting there IS gated by
  ;; `check_writable' (editing.rs:534 -- the same gate inserts and
  ;; deletes go through), and that gate is exactly why dired and *Help*
  ;; are safe while *eshell* is not: those two end their fill with
  ;; `set-buffer-read-only t', so undo is refused there; *eshell* stays
  ;; writable, so undo goes through and re-dirties it.
  ;; elisp has no primitive to suppress undo recording; fixing that
  ;; would require a Rust-level change, out of scope here.
  (set-buffer-modified-p nil))

(defun eshell ()
  "Open the *eshell* shell buffer."
  (interactive)
  (let ((dir (or (default-directory)
                 (file-name-as-directory (expand-file-name ".")))))
    ;; M103: `pop-to-buffer', not `switch-to-buffer-internal' -- splits a
    ;; window for `*eshell*' instead of overwriting whatever the user
    ;; was editing. See window.el's header for the full design.
    (pop-to-buffer "*eshell*")
    (major-mode-internal-set 'eshell-mode)
    (unless (default-directory)
      (set-default-directory dir)))
  (let ((map (make-sparse-keymap)))
    (define-key map "RET" 'eshell-return)
    (define-key map "C-c C-c" 'eshell-interrupt)
    (define-key map "M-p" 'eshell-history-prev)
    (define-key map "M-n" 'eshell-history-next)
    (use-local-map map))
  (when (= (buffer-size) 0)
    (insert "*** Welcome to eshell ***\n")
    (eshell--insert-prompt))
  (goto-char (point-max)))

(defun eshell--record-history (input)
  (setq eshell--history (cons input eshell--history))
  (setq eshell--hist-idx 0))

(defun eshell-return ()
  "Run the current input line."
  (interactive)
  (let ((input (string-trim (buffer-substring eshell--input-start (point-max)))))
    (goto-char (point-max))
    (cond
      ((string-empty-p input)
       (insert "\n")
       (eshell--insert-prompt))
      ((string-prefix-p "(" input)
       (eshell--eval-elisp input))
      (t
       (insert "\n")
       (eshell--record-history input)
       (eshell--run input)))))

(defun eshell--eval-elisp (input)
  "IELM-style: evaluate INPUT as an elisp form."
  (let ((parse (condition-case nil
                   (cons 'ok (read input))
                 (end-of-file 'incomplete)
                 (error 'bad))))
    (cond
      ;; Unbalanced parens: keep typing on the next line. This is a
      ;; stable state (the user already pressed RET, not "typing
      ;; mid-word") so M71 clears the flag here too, same as every
      ;; other branch.
      ((eq parse 'incomplete)
       (insert "\n")
       (set-buffer-modified-p nil))
      ((eq parse 'bad)
       (insert "\n*** Read error\n")
       (eshell--insert-prompt))
      (t
       (insert "\n")
       (eshell--record-history input)
       (let ((result (condition-case err
                         (prin1-to-string (eval (cdr parse) t))
                       (error (format "*** Error: %S" err)))))
         (insert result "\n"))
       (eshell--insert-prompt)))))

(defun eshell--run (input)
  "Dispatch INPUT: builtins, or an async external command."
  (let* ((words (split-string input " "))
         (cmd (car words)))
    (cond
      ((equal cmd "cd")
       (let* ((arg (or (nth 1 words) "~"))
              (dir (expand-file-name arg (default-directory))))
         (if (file-directory-p dir)
             (set-default-directory (file-name-as-directory dir))
           (insert (format "cd: no such directory: %s\n" arg))))
       (eshell--insert-prompt))
      ((equal cmd "pwd")
       (insert (default-directory) "\n")
       (eshell--insert-prompt))
      ((equal cmd "clear")
       (erase-buffer)
       (eshell--insert-prompt))
      (t
       (let ((proc (condition-case nil
                       (start-shell-process input (default-directory))
                     (error nil))))
         (if proc
             ;; The prompt returns when the process exits (pump below).
             (setq eshell--procs
                   (cons (cons (buffer-name) proc) eshell--procs))
           (insert "*** Cannot start process\n")
           (eshell--insert-prompt)))))))

(defun eshell-process-pending-all ()
  "Drain output from every live eshell process (idle-tick pump).
Dead buffers get their processes killed and dropped."
  (let ((procs eshell--procs)
        (remaining nil))
    (while procs
      (let* ((entry (car procs))
             (buf (get-buffer (car entry)))
             (proc (cdr entry)))
        (if (not buf)
            (shell-process-kill proc)
          (let ((keep t)
                (looping t))
            (while looping
              (let ((ev (shell-process-poll proc)))
                (cond
                  ((null ev) (setq looping nil))
                  ((stringp ev)
                   (with-current-buffer buf
                     (goto-char (point-max))
                     (insert ev)
                     (setq eshell--input-start (point-max))
                     ;; M71: streamed stdout is machine-generated, same
                     ;; as the prompt -- and it arrives while the
                     ;; process is still running, so without this the
                     ;; buffer would sit there showing `*' for the whole
                     ;; length of a long command. That case is neither
                     ;; "the user typed something" nor "undo", the two
                     ;; exceptions `eshell--insert-prompt' documents, so
                     ;; it gets cleared here rather than being added to
                     ;; that list (M71 tail review, finding 2).
                     (set-buffer-modified-p nil)))
                  (t ; (exit . CODE)
                   (with-current-buffer buf
                     (goto-char (point-max))
                     (let ((code (cdr ev)))
                       (when (and (integerp code) (/= code 0))
                         (insert (format "[exit %d]\n" code))))
                     (eshell--insert-prompt))
                   (setq looping nil)
                   (setq keep nil)))))
            (when keep
              (setq remaining (cons entry remaining)))))
        (setq procs (cdr procs))))
    (setq eshell--procs remaining))
  nil)

(defun eshell-interrupt ()
  "Kill the current buffer's running process."
  (interactive)
  (let ((entry (assoc (buffer-name) eshell--procs)))
    (if (and entry (shell-process-live-p (cdr entry)))
        (progn
          (shell-process-kill (cdr entry))
          (setq eshell--procs (delq entry eshell--procs))
          (goto-char (point-max))
          (insert "\n[killed]\n")
          (eshell--insert-prompt))
      (message "No running process"))))

(defun eshell--set-input (s)
  (delete-region eshell--input-start (point-max))
  (goto-char (point-max))
  (insert s))

(defun eshell-history-prev ()
  "Recall the previous history entry into the input."
  (interactive)
  (when (and eshell--history
             (< eshell--hist-idx (length eshell--history)))
    (setq eshell--hist-idx (1+ eshell--hist-idx))
    (eshell--set-input (nth (1- eshell--hist-idx) eshell--history))))

(defun eshell-history-next ()
  "Walk forward in history; past the newest entry clears the input."
  (interactive)
  (cond
    ((> eshell--hist-idx 1)
     (setq eshell--hist-idx (1- eshell--hist-idx))
     (eshell--set-input (nth (1- eshell--hist-idx) eshell--history)))
    ((= eshell--hist-idx 1)
     (setq eshell--hist-idx 0)
     (eshell--set-input ""))))

(provide 'eshell)
