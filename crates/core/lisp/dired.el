;;; dired.el --- directory editor (M19), colored ls -al listing -*- lexical-binding: t -*-

;; The command layer over the Rust listing core: `dired-insert-listing'
;; inserts the formatted rows (with face overlays) and returns
;; ((NAME . DIRP) ...) in row order, so this file never parses the
;; listing text back.

;; M41: marking and file operations. Every row starts with a two-char
;; mark column (a space when unmarked) that `dired-mark'/`dired-unmark'/
;; `dired-flag-file-deletion' overwrite in place; `dired--marks' is the
;; buffer-local (NAME . CHAR) alist backing it, CHAR being ?* (marked)
;; or ?D (flagged for deletion). `dired--targets' is what every
;; operation command (`x'/`D'/`C'/`R') acts on: the `*'-marked set if
;; any, else the entry at point; "." and ".." never qualify.
;;
;; v1 boundaries (documented, not bugs):
;; - `C'/`R' destination prompts are static strings ("Copy to: " /
;;   "Rename to: ") -- GNU Emacs interpolates the source file's name in
;;   there, but our `interactive' spec strings are plain literals, not
;;   computed at call time.
;; - A remote (`/ssh:') destination for `C'/`R' is probed for "is this
;;   an existing directory" the same as a local one -- `file-directory-
;;   p' (files.rs) dispatches to a remote `test -d' round trip -- so an
;;   existing remote directory gets each target copied/renamed in as
;;   DEST/basename, matching what the remote `cp'/`mv' shell command
;;   itself does with that destination. See `dired--dest-is-directory-
;;   p'.
;; - SRC and DST must be on the same side (both local, or the same
;;   remote host); `copy-file'/`rename-file' (files.rs) error cleanly
;;   otherwise.
;; - M130: `j'/`k'/`G' (vim-style line motion) are bound alongside `n'/
;;   `p', but `gg' (jump to first line) is NOT, in any of the FIVE
;;   emacs-state modes this milestone actually bound them in (dired-mode,
;;   help-mode, shell-command-mode, compilation-mode, search-mode) -- not
;;   just here. The reason is mechanical, not a judgment call: `Keymap'
;;   stores one Value per key (keymap.rs), and `Keymap::define-sequence'
;;   silently OVERWRITES an existing single-key binding with a fresh
;;   empty sub-keymap when a longer sequence sharing that prefix is
;;   defined (no error, no warning). `g' here is already bound to
;;   `dired-revert' (a GNU-parity binding), so `(define-key map "g g"
;;   ...)' would silently delete `dired-revert' with no way to notice
;;   short of testing for its disappearance. The other four modes' `g'
;;   is unbound, so `gg' would "work" there -- but "`gg' functions in
;;   four modes and not in dired" is worse than "`gg' is absent
;;   everywhere," so it is left out uniformly instead.
;;   `search-edit-mode', `eshell-mode', and `ielm-mode' are ALSO on
;;   `evil-emacs-state-modes' (evil.el) and also writable, but M130
;;   deliberately did NOT bind `j'/`k'/`G' in any of the three (fix
;;   round FIX-1 removed `search-edit-mode' from the original five-plus-
;;   one after cold review: it had been bound there in the first draft,
;;   which was wrong). The test that decides this is "is this buffer's
;;   PURPOSE to be typed into," never "is it read-only" -- `*compilation*'
;;   and the shell-command output buffer are also writable in the Rust
;;   sense and still get the bindings, because nothing about their
;;   purpose involves typing. `search-edit-mode' is M83's wgrep-level
;;   editable search results: free typing that gets written back to the
;;   real source files verbatim (`search-edit-apply') is its entire
;;   purpose, exactly like `eshell-mode'/`ielm-mode' being REPLs where
;;   typing a command that happens to contain `j'/`k'/`G' at the prompt
;;   is normal, expected input. Binding those letters to motion in any
;;   of the three would silently eat them out of ordinary typing --
;;   Verilog identifiers routinely contain `j'/`k' (`clk', `jtag',
;;   `join_none').
;;
;; M68: buffer identity, quit target, and cursor landing.
;; - Buffer identity: `dired' used to hand a bare NAME (the directory's
;;   basename) to `switch-to-buffer-internal', which -- via `buffer_arg's
;;   name-only lookup (builtins/mod.rs) -- takes over ANY existing buffer
;;   of that name rather than creating a new one. Two different
;;   directories sharing a basename would silently alias to the same
;;   buffer, and worse, an ordinary FILE buffer of that name (e.g. a file
;;   literally called "core") could get clobbered into a dired buffer
;;   while its `.file' field still pointed at the original file -- `C-x
;;   C-s' from there overwrites the file's disk contents with the
;;   directory listing. Reproduced live in a real TUI session, not a
;;   hypothetical. Fixed by `dired--buffer-for': identity is now the
;;   normalized directory path (`dired--dir'), matching how
;;   `find-file-internal' already avoids the same class of bug (its
;;   `.file' field, not buffer name).
;; - Reusing an existing dired buffer for the same directory is treated
;;   as a REVERT, not a fresh open: `dired--marks' are preserved (and
;;   pruned against the new listing by `dired--fill', same as `g'). This
;;   is a deliberate behavior change from the old unconditional
;;   `(setq-local dired--marks nil)' on every call.
;; - `dired--marks' is (re)initialized via `(unless (local-variable-p
;;   'dired--marks) ...)', not "is this buffer new" -- `dired--marks-set'
;;   /`-remove' write through a bare `setq' (no `setq-local'), so any
;;   path that skips making it buffer-local first lets a mark leak into
;;   the GLOBAL value and bleed across every dired buffer, which shows up
;;   as "deleted the wrong file". `local-variable-p' checks the actual
;;   invariant directly instead of inferring it from buffer freshness.
;; - `q' (`dired-quit') and `*Help*''s `q' now share one mechanism,
;;   `quit-source'/`quit-source-return' (simple.el): the buffer to return
;;   to is captured BEFORE switching and stored AFTER, and a chain of
;;   `dired'/`describe-bindings' calls inherits the ORIGINAL source
;;   rather than the last hop, so e.g. `alu.sv' -> `C-x d' -> RET (into a
;;   subdirectory) -> `^' -> `q' lands back on `alu.sv' directly.
;; - Cursor landing: `dired--fill' no longer positions point (see its own
;;   doc comment) -- each caller does that itself. `dired' lands on the
;;   first real entry past "."/".." (`dired--goto-first-real-entry');
;;   `dired-up-directory' additionally re-positions onto the child
;;   directory row just left (`dired--goto-name'); `g' (`dired-revert')
;;   keeps its pre-M68 "restore the same line number" behavior unchanged.
;; - v1 does not kill dired buffers automatically (no `kill-buffer' call
;;   anywhere in this file): `kill-buffer' (buffers.rs) redirects EVERY
;;   window still showing the target buffer to a fallback, so an
;;   automatic kill on navigating away could silently swap out a
;;   DIFFERENT window's contents -- the same class of surprise M68 fixes
;;   for buffer identity, just from a different angle -- and `dired--
;;   marks' is the one piece of state a user builds up by hand that an
;;   automatic kill would discard. Cost: dired buffers accumulate for the
;;   life of the session, and `C-x b' completion can show ambiguous
;;   entries like "core" and "core<2>" with no way to tell them apart
;;   from the name alone (the mode line's directory path is the only
;;   disambiguator). Future direction: uniquify, or a
;;   `dired-kill-when-opening-new' opt-in toggle.
;; - `quit-source' being OVERWRITTEN on every reuse (unconditionally,
;;   unlike `dired--marks''s `local-variable-p' guard) is deliberate, not
;;   an inconsistency -- the two variables answer different questions.
;;   `dired--marks' is content the USER built up by hand in that buffer,
;;   which reuse must preserve (revert semantics). `quit-source' is "who
;;   summoned this buffer THIS time", which must always reflect the most
;;   recent caller: from B.txt, `C-x d' into a directory previously
;;   opened from A.txt reuses that same buffer (correctly, by identity)
;;   but `q' must return to B.txt, not the stale A.txt. Manually
;;   confirmed: `(dired D1)' from A -> buffer "D1"; `q' -> A.txt; switch
;;   to B.txt; `(dired D1)' again -> still buffer "D1" (reused), but its
;;   `quit-source' is now B.txt; `q' -> B.txt.
;; - `dired--goto-first-real-entry' assumes `.'/`..' sort first in
;;   `dired--files'. For a LOCAL directory that's a structural guarantee
;;   (`list_dir' in dired.rs pushes them before appending the sorted
;;   rest). A REMOTE directory (`/ssh:') instead parses `ls -al' output
;;   (`remote.rs'), whose ordering depends on the remote shell's locale
;;   -- not a structural guarantee. If some remote host's locale doesn't
;;   put `.'/`..' first, this function would skip past two real entries
;;   instead of `.'/`..' (a misplaced landing, not a crash). Not tested
;;   against a real non-default-locale remote host -- written down as an
;;   assumption, not a verified property.
;; - `dired--buffer-for' compares directories with `equal' on the string
;;   `expand-file-name' produces, which is pure string manipulation
;;   (files.rs) with no filesystem-level normalization. On a
;;   case-insensitive filesystem (e.g. macOS's default APFS), `/x/Foo'
;;   and `/x/foo' are the SAME directory on disk but compare unequal
;;   here, so each gets its own buffer instead of being reused -- extra
;;   buffer accumulation, same direction/cost as the "no auto-kill" cost
;;   above, not a data-loss risk (unlike the pre-M68 name-based bug this
;;   file fixes, which went the other way: WRONGLY reusing/aliasing).
;;   Symlinked directories are unresolved for the same reason and hit
;;   the same cost.
;; - `dired--fill' erases the buffer, inserts the "  DIR:\n" header line,
;;   THEN calls `dired-insert-listing' -- with no `condition-case' around
;;   any of it. If listing the directory fails partway (e.g. its
;;   permissions are revoked mid-session), the buffer is left holding
;;   just that one header line (not empty), and point is never
;;   repositioned -- `dired--goto-first-real-entry' never runs. Read-only
;;   status is UNCHANGED either way: `set-buffer-read-only' (files.rs)
;;   writes the `Buffer' struct's `read_only' FIELD directly, entirely
;;   separate from the `inhibit-read-only' elisp variable `dired--fill'
;;   binds around its own writes (that variable only ever affects
;;   `check_writable's read/write gate, never the field). So a buffer the
;;   user is already looking at -- which reached this state via some
;;   earlier SUCCESSFUL `(set-buffer-read-only t)' -- stays read-only
;;   through a later failed revert; only a buffer that has never
;;   completed even one successful fill could still be writable here,
;;   and reaching THIS gap (mid-listing failure, not "never listed") from
;;   there isn't possible. This gap predates M68 and isn't specific to
;;   buffer reuse either: even under the pre-M68 name-based lookup, a
;;   second visit to the same directory already aliased onto the same
;;   (already read-only) buffer, so a listing failure there already
;;   emptied a buffer the user was looking at. What M68 changes is only
;;   how RELIABLY that reused buffer is the CORRECT one (`dired--buffer-
;;   for's identity fix), not whether this failure mode exists at all.
;;   Not fixed here (a dired buffer has no `.file', so this can't corrupt
;;   anything on disk) -- v1 does not cover recovering from a listing
;;   failure on an already-open buffer.

(defvar inhibit-read-only nil)

;; Per dired buffer: the directory listed, and the row table.
;; (Stored via setq-local so several dired buffers coexist.)
(defvar dired--dir nil)
(defvar dired--files nil)
(defvar dired--header-lines 1)
;; Buffer-local (NAME . CHAR) alist -- see the M41 note above.
(defvar dired--marks nil)

(defun dired--buffer-for (dir)
  "The existing dired buffer already showing DIR (compared via the
buffer-local `dired--dir', not buffer NAME -- see this file's M68
header note), or nil if none. `buffer-local-value' on a buffer that
never made `dired--dir' local of its own (an ordinary file buffer, say)
just returns the (nil) global default rather than signaling
(editor.rs's fallback in `buffer_local_value'), so this can walk every
buffer in `(buffer-list)' unconditionally, without a `major-mode' guard
first."
  (let ((hit nil))
    (dolist (b (buffer-list))
      (when (and (not hit) (equal (buffer-local-value 'dired--dir b) dir))
        (setq hit b)))
    hit))

(defun dired (dir)
  "Show directory DIR as a colored ls -al style listing. Reuses the
existing dired buffer for DIR if one is already open (see
`dired--buffer-for' and this file's M68 header note) instead of always
creating or aliasing onto a buffer picked by name."
  (interactive "DDired (directory): ")
  (let* ((dir (file-name-as-directory (expand-file-name dir)))
         (name (file-name-nondirectory (directory-file-name dir)))
         (name (if (string-empty-p name) "/" name))
         (source (quit-source-of-current))
         (buf (or (dired--buffer-for dir) (generate-new-buffer name))))
    (switch-to-buffer-internal buf)
    (setq-local quit-source source)
    (set-default-directory dir)
    (major-mode-internal-set 'dired-mode)
    (let ((map (make-sparse-keymap)))
      (define-key map "n" 'next-line)
      (define-key map "p" 'previous-line)
      (define-key map "RET" 'dired-find-file)
      (define-key map "^" 'dired-up-directory)
      (define-key map "g" 'dired-revert)
      (define-key map "q" 'dired-quit)
      (define-key map "m" 'dired-mark)
      (define-key map "u" 'dired-unmark)
      (define-key map "U" 'dired-unmark-all-marks)
      (define-key map "d" 'dired-flag-file-deletion)
      (define-key map "x" 'dired-do-flagged-delete)
      (define-key map "D" 'dired-do-delete)
      (define-key map "C" 'dired-do-copy)
      (define-key map "R" 'dired-do-rename)
      (define-key map "j" 'next-line)
      (define-key map "k" 'previous-line)
      ;; M130 fix round FIX-2: `dired-goto-last-entry', not `end-of-
      ;; buffer' -- see that function's own doc comment for why a plain
      ;; end-of-buffer lands one line PAST the last real entry here.
      (define-key map "G" 'dired-goto-last-entry)
      (use-local-map map))
    (setq-local dired--dir dir)
    (setq-local dired--files nil)
    ;; M68: reusing an existing dired buffer is a revert, not a fresh
    ;; open -- its marks must survive (`dired--fill' below prunes any
    ;; that no longer apply). Checking `local-variable-p' instead of "is
    ;; this buffer new" tracks the actual invariant `dired--marks-set'/
    ;; `-remove' depend on (see this file's M68 header note).
    (unless (local-variable-p 'dired--marks)
      (setq-local dired--marks nil))
    (dired--fill)
    (dired--goto-first-real-entry)
    (set-buffer-read-only t)))

(defun dired--fill ()
  "(Re)insert the listing for the current dired buffer, then redraw
`dired--marks' over it -- pruning any name that's no longer listed
first, so a `g'/`dired-revert' after deleting a marked file drops its
now-stale mark instead of leaving it dangling. Leaves point at
`point-min' and does NOT otherwise position it (M68: that used to
always land on \".\", the one row no dired command ever does anything
useful to) -- callers position point themselves afterward."
  (let ((inhibit-read-only t))
    (erase-buffer)
    (remove-overlays)
    (goto-char (point-min))
    (insert "  " dired--dir ":\n")
    (setq dired--files (dired-insert-listing dired--dir))
    (dired--marks-prune-and-reapply)
    (goto-char (point-min))
    (set-buffer-modified-p nil)))

(defun dired--entry-at-point ()
  "The (NAME . DIRP) row under point, or nil on the header."
  (let ((row (- (line-number-at-pos) 1 dired--header-lines)))
    (if (>= row 0) (nth row dired--files) nil)))

;; --- M41: marking ----------------------------------------------------

(defun dired--set-mark-char (ch)
  "Overwrite the mark column of the line at point with CH."
  (let ((bol (line-beginning-position))
        (inhibit-read-only t))
    (goto-char bol)
    (delete-char 1)
    (insert (char-to-string ch))
    ;; M71: `dired--fill' (above) already clears this flag for the
    ;; open/revert path, but every interactive marking command
    ;; (`dired-mark'/`dired-unmark'/`dired-unmark-all-marks'/
    ;; `dired-flag-file-deletion') goes through THIS function instead,
    ;; and `insert' above unconditionally re-dirties the buffer. A
    ;; dired listing is entirely machine-generated -- there is nothing
    ;; in it a user could "lose" -- so `*' on this buffer is never a
    ;; meaningful signal; clear it again here.
    (set-buffer-modified-p nil)))

(defun dired--marks-remove (name)
  "Drop NAME's entry (if any) from `dired--marks'."
  (let ((kept nil))
    (dolist (pair dired--marks)
      (unless (equal (car pair) name)
        (push pair kept)))
    (setq dired--marks (nreverse kept))))

(defun dired--marks-set (name ch)
  (dired--marks-remove name)
  (push (cons name ch) dired--marks))

(defun dired--marks-prune-and-reapply ()
  "Drop marks for names `dired--files' no longer lists, then redraw the
mark column for every name that still carries one. Point is restored
afterward (called from `dired--fill', before the caller repositions
point itself)."
  (let ((kept nil))
    (dolist (pair dired--marks)
      (when (assoc (car pair) dired--files)
        (push pair kept)))
    (setq dired--marks (nreverse kept)))
  (save-excursion
    (goto-char (point-min))
    (forward-line dired--header-lines)
    (dolist (entry dired--files)
      (let ((ch (cdr (assoc (car entry) dired--marks))))
        (when ch
          (dired--set-mark-char ch)))
      (forward-line 1))))

(defun dired-mark ()
  "Mark the file on this line with `*' for a later `x'/`D'/`C'/`R'."
  (interactive)
  (let ((entry (dired--entry-at-point)))
    (cond
     ((not entry) (message "No file on this line"))
     ((member (car entry) '("." "..")) (message "Cannot mark %s" (car entry)))
     (t
      (dired--marks-set (car entry) ?*)
      (dired--set-mark-char ?*)
      (forward-line 1)))))

(defun dired-unmark ()
  "Clear any mark on the file on this line."
  (interactive)
  (let ((entry (dired--entry-at-point)))
    (if (not entry)
        (message "No file on this line")
      (dired--marks-remove (car entry))
      (dired--set-mark-char ?\s)
      (forward-line 1))))

(defun dired-unmark-all-marks ()
  "Clear every mark in this buffer."
  (interactive)
  (setq dired--marks nil)
  (save-excursion
    (goto-char (point-min))
    (forward-line dired--header-lines)
    (dolist (_entry dired--files)
      (dired--set-mark-char ?\s)
      (forward-line 1))))

(defun dired-flag-file-deletion ()
  "Flag the file on this line for deletion with `D' (see `dired-do-
flagged-delete', bound to `x')."
  (interactive)
  (let ((entry (dired--entry-at-point)))
    (cond
     ((not entry) (message "No file on this line"))
     ((member (car entry) '("." "..")) (message "Cannot flag %s" (car entry)))
     (t
      (dired--marks-set (car entry) ?D)
      (dired--set-mark-char ?D)
      (forward-line 1)))))

(defun dired-find-file ()
  "Visit the file or directory on the current line."
  (interactive)
  (let ((entry (dired--entry-at-point)))
    (if entry
        (find-file (expand-file-name (car entry) dired--dir))
      (message "No file on this line"))))

(defun dired--goto-first-real-entry ()
  "Move point to the first entry past \".\"/\"..\" (`dired--files' always
lists them first -- see `list_dir' in dired.rs) -- or to the line right
after the header when the directory is empty (only \".\"/\"..\" present),
so this never walks `forward-line' past the last real row into nothing."
  (goto-char (point-min))
  (if (> (length dired--files) 2)
      (forward-line (+ dired--header-lines 2))
    (forward-line dired--header-lines)))

(defun dired-goto-last-entry ()
  "Move point to the LAST real entry in the listing (M130's `G' binding).
Addresses the SAME mechanical problem `dired--goto-first-real-entry'
addresses at the other end of the listing, but the two do NOT land on
the same row when the directory is empty -- see below. Every row
`dired-insert-listing' emits, including the last one, ends in its own
\"\\n\" (files.rs's `list_dir'), so a plain `end-of-buffer' lands on
the empty line PAST the last real row, where `dired--entry-at-point'
computes a row index one past the end of `dired--files' and returns
nil (\"No file on this line\" from every command that reads it). This
goes to `(1- (length dired--files))' rows past the header instead --
the last row, not past it. `dired--files' always lists \".\" and \"..\"
even in an empty directory (see that function's own doc comment), so
on an empty directory (only \".\"/\"..\" present, length 2) this lands
on the SECOND of the two, \"..\" -- one row PAST where
`dired--goto-first-real-entry' lands in that same case (its own
\"only \".\"/\"..\" present\" branch stops at the FIRST of the two,
\".\"): the two functions land on different rows here by design, one
at each end of the listing, not on the same row."
  (interactive)
  (goto-char (point-min))
  (if dired--files
      (forward-line (+ dired--header-lines (1- (length dired--files))))
    (forward-line dired--header-lines)))

(defun dired--goto-name (name)
  "Move point to the row naming NAME (matching `dired--files' entries) and
return t, or leave point untouched and return nil if no such row
exists. Used by `dired-up-directory' to land back on the child
directory row just left."
  (let ((row 0) (found nil))
    (dolist (entry dired--files)
      (when (and (not found) (equal (car entry) name))
        (setq found row))
      (setq row (1+ row)))
    (when found
      (goto-char (point-min))
      (forward-line (+ dired--header-lines found))
      t)))

(defun dired-up-directory ()
  "Enter the parent directory, then re-position point on the child
directory row just left (`dired--goto-name'). At the filesystem root,
CHILD computes to the empty string and no such row is found, so point
stays at `dired''s own default landing (the first real entry) instead
-- this must not error."
  (interactive)
  (let* ((child (file-name-nondirectory (directory-file-name dired--dir)))
         (parent (file-name-directory (directory-file-name dired--dir))))
    (dired parent)
    (dired--goto-name child)))

(defun dired-revert ()
  "Re-read the directory, keeping point's line (clamped to the last line
still present -- revert can shrink the listing out from under a saved
line number)."
  (interactive)
  (let ((line (line-number-at-pos)))
    (dired--fill)
    (goto-char (point-min))
    (forward-line (1- line))
    (let ((max-line (+ dired--header-lines (length dired--files))))
      (when (> (line-number-at-pos) max-line)
        (goto-char (point-min))
        (forward-line (1- max-line))))))

;; --- M41: file operations ---------------------------------------------

(defun dired--marked-names ()
  "Names with a `*' mark, in `dired--files' row order."
  (let ((out nil))
    (dolist (entry dired--files)
      (when (eq (cdr (assoc (car entry) dired--marks)) ?*)
        (push (car entry) out)))
    (nreverse out)))

(defun dired--flagged-names ()
  "Names with a `D' flag, in `dired--files' row order."
  (let ((out nil))
    (dolist (entry dired--files)
      (when (eq (cdr (assoc (car entry) dired--marks)) ?D)
        (push (car entry) out)))
    (nreverse out)))

(defun dired--targets ()
  "The (NAME . DIRP) entries to operate on: the `*'-marked set if any,
else the entry at point (a one-element list), always excluding
\".\"/\"..\". nil when that leaves nothing (an empty buffer, the header
line, or point on \".\"/\"..\" with nothing marked) -- every caller must
check for that and echo rather than proceed."
  (let* ((marked (dired--marked-names))
         (names (if marked
                    marked
                  (let ((entry (dired--entry-at-point)))
                    (if entry (list (car entry)) nil))))
         (out nil))
    (dolist (name names)
      (unless (member name '("." ".."))
        (push (assoc name dired--files) out)))
    (nreverse out)))

(defun dired--error-first-line (err)
  "First line of the message string carried by ERR, a `condition-
case' ERR object from one of this file's disk operations. Our own
primitives always signal a single string via `error' (`(cdr err)' is
then a one-element list holding it, mirroring `lsp--error-string's
unwrap in lsp.el), so this just also guards against an embedded
newline -- a multi-line remote stderr, say -- making a skip summary
spill across more than one echo line."
  (let* ((data (cdr err))
         (msg (if (and (consp data) (stringp (car data)) (null (cdr data)))
                  (car data)
                (format "%S" err)))
         (nl (string-search "\n" msg)))
    (if nl (substring msg 0 nl) msg)))

(defun dired--summarize (verb count skipped)
  "Echo a batch-operation result: VERB (\"Deleted\"/\"Renamed\") COUNT
file(s), plus SKIPPED -- an ((NAME . REASON) ...) list -- appended as
a `skipped: NAME (REASON), ...' tail when SKIPPED is non-empty."
  (if skipped
      (message "%s %d file(s), skipped %s" verb count
               (string-join
                (mapcar (lambda (s) (format "%s (%s)" (car s) (cdr s))) skipped)
                ", "))
    (message "%s %d file(s)" verb count)))

(defun dired--delete-entries (targets)
  "Delete each (NAME . DIRP) in TARGETS from disk (recursively for a
directory), skipping -- not aborting on -- any that error: one
already-gone or permission-denied file in a batch must not stop the
rest from being deleted. Returns the skipped entries as ((NAME
. REASON) ...), REASON via `dired--error-first-line', in TARGETS
order; the caller (`dired--confirm-delete') folds that into its
summary message and always reverts the listing, whether or not
anything was skipped."
  (let ((skipped nil))
    (dolist (entry targets)
      (let ((path (expand-file-name (car entry) dired--dir)))
        (condition-case err
            (if (cdr entry)
                (delete-directory path t)
              (delete-file path))
          (error (push (cons (car entry) (dired--error-first-line err)) skipped)))))
    (nreverse skipped)))

(defun dired--confirm-delete (targets)
  "Ask to delete TARGETS (a `dired--targets'-shaped list) and act on the
next keystroke via `capture-next-key' -- there is no `y-or-n-p' in
this editor, so this is dired's own version of that idiom (mirroring
evil.el's `f'/`t'/`r' captures). One failed deletion does not abort
the rest -- see `dired--delete-entries' -- and the listing is always
reverted afterward, so it can't go stale relative to what actually
happened on disk."
  (if (not targets)
      (message "(No deletions requested)")
    (message "Delete %d file(s)? (y/n)" (length targets))
    (capture-next-key
     (lambda (k)
       (if (eq k ?y)
           (let ((skipped (dired--delete-entries targets)))
             (dired-revert)
             (dired--summarize "Deleted" (- (length targets) (length skipped)) skipped))
         (message "(No deletions performed)"))))))

(defun dired--flagged-targets ()
  "Like `dired--targets', but the `D'-flagged set instead of `*'/point."
  (let ((out nil))
    (dolist (name (dired--flagged-names))
      (unless (member name '("." ".."))
        (push (assoc name dired--files) out)))
    (nreverse out)))

(defun dired-do-flagged-delete ()
  "Delete every file flagged with `D' (see `dired-flag-file-deletion'),
after confirmation."
  (interactive)
  (dired--confirm-delete (dired--flagged-targets)))

(defun dired-do-delete ()
  "Delete the `*'-marked files, or the file at point with nothing
marked, after confirmation."
  (interactive)
  (dired--confirm-delete (dired--targets)))

(defun dired--dest-is-directory-p (path)
  "T if PATH names an existing directory, local or remote alike --
`file-directory-p' (files.rs) dispatches a `/ssh:' PATH to a remote
`test -d' itself, so this is a thin, name-only wrapper now (kept so
`dired-do-copy'/`dired-do-rename' read the same either way)."
  (file-directory-p path))

(defun dired-do-copy (dest)
  "Copy the `*'-marked files (or the file at point) to DEST. DEST an
existing local directory copies each target in as DEST/basename;
otherwise DEST is the literal destination name, valid only for a
single target."
  (interactive "FCopy to: ")
  (let ((targets (dired--targets)))
    (if (not targets)
        (message "No file to copy")
      (let* ((dest (expand-file-name dest))
             (into-dir (dired--dest-is-directory-p dest)))
        (if (and (not into-dir) (cdr targets))
            (message "Destination must be a directory for multiple files")
          (let ((count 0) (skipped nil) (single-src nil) (single-dst nil))
            (dolist (entry targets)
              (let* ((name (car entry))
                     (src (expand-file-name name dired--dir))
                     (dst (if into-dir (expand-file-name name dest) dest)))
                (condition-case nil
                    (progn
                      (copy-file src dst)
                      (setq count (1+ count))
                      (setq single-src src)
                      (setq single-dst dst))
                  (error (push name skipped)))))
            (dired-revert)
            (cond
             (skipped
              (message "Copied %d file(s), skipped %s" count
                       (string-join (nreverse skipped) ", ")))
             ((and (= count 1) (not into-dir))
              (message "Copied %s -> %s" single-src single-dst))
             (t
              (message "Copied %d file(s)" count)))))))))

(defun dired-do-rename (dest)
  "Rename the `*'-marked files (or the file at point) to DEST, same
DEST-resolution rules as `dired-do-copy'. Any buffer visiting a
renamed file is kept in sync (see `rename-file' in files.rs). Mirrors
`dired-do-copy's skip-and-continue discipline: one failure in a batch
does not abort the rest, and the listing is always reverted afterward
so it can't go stale relative to what actually happened on disk."
  (interactive "FRename to: ")
  (let ((targets (dired--targets)))
    (if (not targets)
        (message "No file to rename")
      (let* ((dest (expand-file-name dest))
             (into-dir (dired--dest-is-directory-p dest)))
        (if (and (not into-dir) (cdr targets))
            (message "Destination must be a directory for multiple files")
          (let ((count 0) (skipped nil) (single-src nil) (single-dst nil))
            (dolist (entry targets)
              (let* ((name (car entry))
                     (src (expand-file-name name dired--dir))
                     (dst (if into-dir (expand-file-name name dest) dest)))
                (condition-case err
                    (progn
                      (rename-file src dst)
                      (setq count (1+ count))
                      (setq single-src src)
                      (setq single-dst dst))
                  (error (push (cons name (dired--error-first-line err)) skipped)))))
            (dired-revert)
            (cond
             (skipped
              (dired--summarize "Renamed" count (nreverse skipped)))
             ((and (= count 1) (not into-dir))
              (message "Renamed %s -> %s" single-src single-dst))
             (t
              (message "Renamed %d file(s)" count)))))))))

(defun dired-quit ()
  "Leave the dired buffer, returning to wherever this dired/help chain
started (see `quit-source-return' in simple.el, M68) rather than always
landing on `*scratch*'."
  (interactive)
  (quit-source-return))

(global-set-key "C-x d" 'dired)

(provide 'dired)
