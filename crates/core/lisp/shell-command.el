;;; shell-command.el --- M-!/M-&/M-| (M79 second half) -*- lexical-binding: t -*-

;; All three commands here are ASYNCHRONOUS, unlike GNU Emacs's `M-!'
;; (which blocks the whole editor until the child exits). batch/tui both
;; run on a single event-loop thread with no way to pump keys mid-call
;; (same constraint `read-from-minibuffer' documents in builtins/ui.rs)
;; -- a synchronous wait here would reproduce exactly the "editor frozen
;; solid, not even C-g gets through" failure M77 fixed for remote
;; commands. So every command below spawns via `start-shell-process' and
;; returns immediately; the running job is drained by the idle pump
;; (`shell-command-process-pending-all', wired into `idle-tick' in
;; lib.rs) the same way eshell's own commands are.
;;
;; This does NOT reuse eshell's pump (`eshell-process-pending-all',
;; eshell.el). That function hardcodes `eshell--input-start' -- a
;; GLOBAL variable, per eshell.el's own "v1 supports one *eshell*
;; buffer" comment -- as the landing point for streamed output, and its
;; exit branch inserts a fresh shell prompt, both REPL-only behaviors
;; that make no sense for one-shot commands. `shell-command--procs'
;; below is its own list, `shell-command-process-pending-all' its own
;; pump; neither touches any eshell global.
;;
;; No `C-u' variant of any of these three: there is no prefix-argument
;; mechanism in this editor at all (`P'/`p' interactive-spec codes are
;; hardcoded to nil/1, see builtins/ui.rs) so a single key can only ever
;; carry one meaning. `M-|' below picks the "replace the region with
;; the command's stdout" meaning (GNU's `C-u M-|') rather than GNU's
;; default "just show me the output", because the no-replacement case
;; already has a substitute one keystroke away (`M-!' on the whole
;; buffer/file), while nothing else lets you filter a chunk of text
;; through an external script in one step -- exactly the RTL workflow
;; ("pipe this always-block through a formatter") this editor's
;; priorities (see CLAUDE.md) call out.
;;
;; Known gaps (v1, not attempted here):
;; - All three commands share ONE output buffer and track at most ONE
;;   running job; starting a second one kills whatever the first was
;;   still doing (see `shell-command--reset'). Deliberate simplification
;;   -- GNU Emacs lets several `M-&' jobs run concurrently into distinct
;;   buffers, but that needs per-invocation buffer naming/bookkeeping
;;   this milestone does not attempt.
;; - `M-|' under evil's Visual Line selection (`V') does NOT expand to
;;   whole lines -- `region-beginning'/`region-end' (editing.rs) only
;;   ever know about the buffer's raw mark/point positions, not
;;   `evil--visual-type' (evil.el), so a `V'-selected region is treated
;;   exactly like a `v'-selected one (the raw character span).
;; - No `C-u' variant of any of the three commands -- see above.
;; - `M-|' refuses to replace the region if its CONTENT changed while
;;   the command was running (see `shell-command--finish-filter''s
;;   content-comparison guard) -- not just when the region collapsed or
;;   inverted. This editor's markers have no insertion-type distinction
;;   (`buffer.rs''s `adjust_positions_insert' shifts every marker at or
;;   after the insertion point forward the same way, with no way to
;;   make the END marker of a region hang back while the BEGIN marker
;;   advances), so an edit landing exactly at the region's end boundary,
;;   or anywhere inside it, gets silently absorbed INTO the tracked span
;;   rather than pushed past it. Replacing in that case would delete the
;;   user's own concurrent edit together with the stale region and never
;;   record it as its own undo step. Refusing and dumping the command's
;;   stdout into the output buffer instead is the least-bad option
;;   available without a marker type this editor doesn't have (fix
;;   round, R7 -- caught by tail review's mechanical repro after R1's
;;   first pass only tested editing strictly BEFORE the region).
;; - `shell-command-max-output-chars' truncation is bounded, not exact,
;;   in BOTH directions:
;;   - Undershoot: `shell-command--drain-before-kill' can still lose a
;;     process's very last bytes if they land after the drain pass's own
;;     bounds (`shell-command--drain-poll-limit' /
;;     `shell-command--drain-overshoot-multiplier') give up.
;;   - Overshoot: that same drain pass can fold in several times
;;     `shell-command-max-output-chars' worth of extra output before
;;     giving up and killing (fix round, R8) -- neither the iteration
;;     count nor a single `shell-process-poll' call is bounded by real
;;     output volume, only by however much the reader threads had
;;     already buffered, so a fast enough producer can make one drain
;;     pass balloon well past the nominal cap. The cap is a safety net
;;     against unbounded runaway output, not a byte-exact accounting of
;;     it either way.
;; - Filter-mode ('M-|') liveness-checks its ORIGINAL buffer by NAME
;;   (`(get-buffer (aref entry 3))'), not by object identity. If the
;;   user kills that buffer and then creates a brand new, unrelated
;;   buffer that happens to get the SAME name before the job finishes,
;;   `shell-command--finish-filter' would find a buffer under that name
;;   again and could substitute into the wrong (new) buffer at whatever
;;   raw positions the now-detached markers still hold. Pre-existing
;;   since before this fix round (R1 first introduced markers, but this
;;   name-vs-identity gap was already latent in the ORIG-BUFFER-NAME
;;   slot design); extremely unlikely in practice (requires killing and
;;   immediately re-creating a same-named buffer inside one job's short
;;   lifetime) and not fixed here -- would need a buffer OBJECT slot
;;   plus a liveness check via `buffer-list' membership, the same
;;   pattern `quit-source' itself already uses (see simple.el) and
;;   deliberately avoided here for its own name-based reasons -- out of
;;   scope for this fix round.

(defvar shell-command-output-buffer-name "*Shell Command Output*"
  "Shared output buffer for `shell-command'/`async-shell-command'/
`shell-command-on-region''s failure/stderr display. All three commands
track at most ONE running job at a time (see `shell-command--reset');
this is a deliberate simplification, not an oversight -- GNU Emacs
lets several `M-&' jobs run concurrently into distinct buffers, but
that needs per-invocation buffer naming/bookkeeping this milestone
does not attempt.")

(defvar shell-command-max-output-chars (* 1024 1024)
  "Cap on how many characters of combined stdout+stderr a single
`shell-command'/`async-shell-command'/`shell-command-on-region' job may
produce before it gets killed. `start-shell-process' itself has no
limit (by design -- see shell.rs's module doc comment: only the elisp
caller knows which buffer output lands in and how to report a
truncation), so runaway output (`M-! yes') would otherwise grow a
buffer -- and this list's accumulated strings -- without bound.")

(defvar shell-command--drain-poll-limit 512
  "Bound on how many `shell-process-poll' calls
`shell-command--drain-before-kill' makes while flushing a process's
already-buffered-but-undelivered output before killing it (fix round,
R3). Needs a cap because a still-running process producing output
faster than we can drain it would otherwise spin here forever instead
of ever reaching the `shell-process-kill' call -- same
infinite-work-needs-a-ceiling reasoning `enumerate_bindings' (keymap.rs)
documents for its own unrelated depth cap. See also
`shell-command--drain-overshoot-multiplier' -- iteration count ALONE
does not bound how much output a fast producer can hand over per call.")

(defvar shell-command--drain-overshoot-multiplier 4
  "How many multiples of `shell-command-max-output-chars' the drain pass
(`shell-command--drain-before-kill') is allowed to fold in past the cap
before giving up and killing anyway (fix round, R8). Needed because
NEITHER `shell-command--drain-poll-limit' (an iteration count) NOR a
single `shell-process-poll' call's own size is bounded by real time or
volume -- each call hands back everything the reader threads had
already buffered, so a fast enough producer can make the drain pass
itself balloon well past the cap it exists to enforce (`filter' mode
makes this doubly expensive: its `concat' accumulation copies the whole
string so far on every call, O(n^2) over the drain). The multiplier is
intentionally greater than 1x, not tight to the cap: a little slack
means the eventual truncation message still carries some real context
around the cutoff instead of stopping with zero margin the instant the
nominal cap is crossed.")

;; Each entry: a vector #[PROC MODE TARGET-BUFFER-NAME ORIG-BUFFER-NAME
;;             STDOUT STDERR BEG-MARKER END-MARKER CHARS QUIT-SOURCE
;;             REGION-SNAPSHOT]
;;   PROC              -- the `start-shell-process' handle.
;;   MODE              -- 'view (M-!/M-&, streams into the shared
;;                         output buffer) or 'filter (M-|, accumulates
;;                         silently and only touches a buffer at exit).
;;   TARGET-BUFFER-NAME -- 'view: `shell-command-output-buffer-name'.
;;                         'filter: the buffer the region came from.
;;   ORIG-BUFFER-NAME   -- 'filter only: same as TARGET-BUFFER-NAME,
;;                         kept as its own slot so a rename doesn't
;;                         conflate the two independent purposes. Looked
;;                         up by NAME, not object identity -- see this
;;                         file's header "Known gaps" for the same-name
;;                         reuse edge case that leaves open.
;;   STDOUT / STDERR    -- accumulated strings ('filter only; 'view
;;                         streams straight into the buffer instead of
;;                         buffering here).
;;   BEG-MARKER/        -- 'filter only: markers into the region to
;;   END-MARKER            replace on success (nil for 'view entries).
;;                         MUST be markers, not raw integers -- M-| is
;;                         asynchronous (see file header), so the user
;;                         can keep editing the SAME buffer while the
;;                         job runs; `buffer.rs''s
;;                         `adjust_positions_insert'/`_delete' keep
;;                         markers tracking the right text through any
;;                         such edit, which two bare offsets captured at
;;                         invocation time cannot (fix round, R1 --
;;                         concurrent-edit data corruption, caught by
;;                         reviewer's mechanical repro). But markers
;;                         alone are NOT sufficient -- see
;;                         REGION-SNAPSHOT below and the header's R7
;;                         "Known gaps" entry.
;;   CHARS              -- running total of output characters seen so
;;                         far, checked against
;;                         `shell-command-max-output-chars'.
;;   QUIT-SOURCE        -- the value `quit-source-of-current' returned
;;                         at the moment the command was INVOKED (i.e.
;;                         while the user's own buffer was still
;;                         current), not whenever the output buffer
;;                         eventually gets shown. `shell-command''s
;;                         has-output path and `shell-command-on-region'
;;                         's failure path are only ever decided from
;;                         the idle pump, arbitrarily long after
;;                         invocation -- recomputing
;;                         `quit-source-of-current' at THAT point would
;;                         capture whatever buffer the user had since
;;                         switched to instead (fix round, R2).
;;   REGION-SNAPSHOT    -- 'filter only: the exact text originally piped
;;                         to the command as stdin (nil for 'view
;;                         entries). `shell-command--finish-filter'
;;                         compares this against whatever text currently
;;                         sits between the (marker-tracked, possibly
;;                         moved) BEG/END markers at exit time -- a
;;                         mismatch means the region's CONTENT changed
;;                         during the run even though the markers never
;;                         collapsed (fix round, R7; see this file's
;;                         header for why markers alone can't detect
;;                         this).
(defvar shell-command--procs nil
  "Active shell-command jobs (at most one in practice, see
`shell-command--reset'); see the comment above this defvar for the
per-entry vector layout.")

(defun shell-command--release-markers (entry)
  "Drop ENTRY's own references to its region markers (a no-op for
'view entries, whose marker slots are always nil). A marker stays
adjusted for every insert/delete in its buffer for as long as
something holds a live reference to it (`buffer.rs' keeps a Weak list,
see `adjust_positions_insert'/`_delete'); nil-ing these two slots is
what lets the marker get garbage collected and its now-dead Weak entry
pruned out on the buffer's next edit, instead of it silently tracking
position changes in that buffer for the rest of the session."
  (aset entry 6 nil)
  (aset entry 7 nil))

(defun shell-command--reset ()
  "Kill any previous shell-command job and clear the shared output
buffer. Called at the start of all three commands below -- M-!/M-&/M-|
track only one running job at a time (see `shell-command--procs''s doc
comment), so starting a second one abandons the first rather than
letting both write into the same buffer."
  (dolist (entry shell-command--procs)
    (shell-process-kill (aref entry 0))
    (shell-command--release-markers entry))
  (setq shell-command--procs nil)
  (let ((buf (get-buffer shell-command-output-buffer-name)))
    (when buf
      (with-current-buffer buf
        (let ((inhibit-read-only t))
          (erase-buffer))
        (set-buffer-modified-p nil)))))

(defun shell-command--ensure-output-buffer ()
  "Get-or-create the shared output buffer, installing `shell-command-mode'
and its local `q' binding the first time. Does not switch to it -- callers
decide separately whether to display it (see `shell-command--maybe-show')."
  (let ((buf (get-buffer-create shell-command-output-buffer-name)))
    (with-current-buffer buf
      (unless (eq (major-mode-internal-get) 'shell-command-mode)
        (major-mode-internal-set 'shell-command-mode)
        (let ((map (make-sparse-keymap)))
          (define-key map "q" 'quit-source-return)
          (use-local-map map))))
    buf))

(defun shell-command--default-dir ()
  "The directory to run a command in: the current buffer's
`default-directory', or the process's own cwd if that's nil. A brand
new buffer's `default-directory' is always nil (`buffer.rs' never
inherits it from the buffer that created it -- see `Buffer::new'), so
without this fallback `start-shell-process' would get handed nil as
its DIR argument and fail `need_str' every time a shell-command runs
from a buffer that was never associated with a file (e.g. `*scratch*').
Same fallback `eshell' itself uses, see eshell.el's own definition."
  (or (default-directory) (file-name-as-directory (expand-file-name "."))))

(defun shell-command--insert-output (buf text)
  "Append TEXT to BUF (the shared output buffer), keeping it unmodified.
M71: streamed output is machine-generated, same reasoning as eshell's
own pump (see `eshell-process-pending-all''s comment) -- it arrives in
many small chunks while the process is still running, so the flag has
to be cleared after EVERY insertion, not just once at the end."
  (with-current-buffer buf
    (let ((inhibit-read-only t))
      (goto-char (point-max))
      (insert text))
    (set-buffer-modified-p nil)))

(defun shell-command--maybe-show (buf source)
  "Display BUF (the shared output buffer), recording SOURCE as its
`quit-source' so `q' returns to wherever the command was invoked from.
SOURCE must be a value `quit-source-of-current' already produced AT
INVOCATION TIME (see the QUIT-SOURCE slot doc comment above) -- this
function must NOT call `quit-source-of-current' itself here, because
both call sites that matter (`shell-command''s has-output path,
`shell-command-on-region''s failure path) only run from the idle pump,
potentially long after the user switched to some other buffer (fix
round, R2).

M103: `pop-to-buffer', not `switch-to-buffer-internal' -- this is the
single display choke point for THREE features (async shell output,
compile/recompile, search-project/search-again all call through here),
so switching it to split/reuse a window instead of overwriting fixes
all three at once. See window.el's header for the full design."
  (pop-to-buffer buf)
  (setq-local quit-source source))

;;; --- M-! -------------------------------------------------------------

(defun shell-command ()
  "Run a shell command asynchronously. If it produces no output and
exits 0, just report the exit status in the echo area without
switching windows -- the most common case (a lint/format/build command
that only speaks up on failure) would otherwise cost you whatever
buffer you were editing, every single time."
  (interactive)
  (let ((dir (shell-command--default-dir))
        (source (quit-source-of-current)))
    (read-string "Shell command: "
                 (lambda (cmd)
                   (shell-command--reset)
                   (let ((proc (condition-case nil
                                   (start-shell-process cmd dir)
                                 (error nil))))
                     (if (not proc)
                         (message "Cannot start process: %s" cmd)
                       (let ((buf (shell-command--ensure-output-buffer)))
                         (setq shell-command--procs
                               (list (vector proc 'view
                                             (buffer-name buf) nil
                                             nil nil nil nil 0 source nil)))))))
                 nil "shell-command")))

;;; --- M-& -------------------------------------------------------------

(defun async-shell-command ()
  "Like `shell-command', but shows the output buffer immediately instead
of waiting to see whether there was any output -- for jobs you expect
to watch run (a build, a long lint pass), not one-shot lint/format
calls where silence is the success case."
  (interactive)
  (let ((dir (shell-command--default-dir))
        (source (quit-source-of-current)))
    (read-string "Async shell command: "
                 (lambda (cmd)
                   (shell-command--reset)
                   (let ((proc (condition-case nil
                                   (start-shell-process cmd dir)
                                 (error nil))))
                     (if (not proc)
                         (message "Cannot start process: %s" cmd)
                       (let ((buf (shell-command--ensure-output-buffer)))
                         (shell-command--maybe-show buf source)
                         (setq shell-command--procs
                               (list (vector proc 'view
                                             (buffer-name buf) nil
                                             nil nil nil nil 0 source nil)))))))
                 nil "shell-command")))

;;; --- M-| -------------------------------------------------------------

(defun shell-command-on-region ()
  "Run a shell command with the current region piped in as stdin, and
replace the region with its stdout on success (GNU Emacs's default is
to just display the output and require `C-u' to replace -- see this
file's header comment for why that split collapses to one meaning
here). On a non-zero exit, OR if the region's own content changed while
the command was running (see this file's header \"Known gaps\" entry on
markers having no insertion-type), the region is left untouched
byte-for-byte and the command's stderr (or stdout, if stderr was empty)
is shown in the output buffer instead."
  (interactive)
  (condition-case nil
      (let ((beg-marker (copy-marker (region-beginning)))
            (end-marker (copy-marker (region-end)))
            (dir (shell-command--default-dir))
            (orig-buf (buffer-name))
            (source (quit-source-of-current)))
        (let ((input (buffer-substring (marker-position beg-marker)
                                        (marker-position end-marker))))
          (read-string "Shell command on region: "
                       (lambda (cmd)
                         (shell-command--reset)
                         (let ((proc (condition-case nil
                                         (start-shell-process cmd dir input 'separate)
                                       (error nil))))
                           (if (not proc)
                               (message "Cannot start process: %s" cmd)
                             (setq shell-command--procs
                                   (list (vector proc 'filter
                                                 orig-buf orig-buf
                                                 "" "" beg-marker end-marker
                                                 0 source input))))))
                       nil "shell-command")))
    (error (message "No region selected"))))

;;; --- pump --------------------------------------------------------------

(defun shell-command--count (entry n)
  "Add N to ENTRY's running output-character total; returns t once the
total exceeds `shell-command-max-output-chars'. Does NOT kill the
process itself -- see `shell-command--drain-before-kill', which the
pump calls once this returns t, so anything the process had already
produced but not yet delivered through `shell-process-poll' gets
folded in before the kill discards it (fix round, R3)."
  (let ((total (+ (aref entry 8) n)))
    (aset entry 8 total)
    (> total shell-command-max-output-chars)))

(defun shell-command--drain-before-kill (entry)
  "Poll ENTRY's process, folding any output it had already buffered
before the cap was noticed into ENTRY ('view mode: straight into its
target buffer, same as the normal chunk path; 'filter mode: into the
STDOUT/STDERR accumulator slots), then kill it. `shell-process-kill'
(shell.rs) drops whatever the reader threads had already buffered but
not yet handed out through `shell-process-poll' -- without this drain
pass first, that trailing output would just vanish instead of showing
up in the truncation message (fix round, R3).

Bounded TWO ways, not just by iteration count
(`shell-command--drain-poll-limit'): also by total characters drained,
capped at `shell-command-max-output-chars' times
`shell-command--drain-overshoot-multiplier' (fix round, R8) -- a single
`shell-process-poll' call can hand back an arbitrarily large chunk (it
merges everything the reader threads had already buffered, regardless
of how that got there), so the iteration count alone does not bound how
much output this pass can fold in past the nominal cap."
  (let ((proc (aref entry 0))
        (mode (aref entry 1))
        (n 0)
        (drained 0)
        (drain-cap (* shell-command-max-output-chars
                      shell-command--drain-overshoot-multiplier))
        (looping t))
    (while (and looping
                (< n shell-command--drain-poll-limit)
                (< drained drain-cap))
      (setq n (1+ n))
      (let ((ev (shell-process-poll proc)))
        (cond
         ((null ev) (setq looping nil))
         ((and (eq mode 'view) (stringp ev))
          (setq drained (+ drained (length ev)))
          (let ((buf (get-buffer (aref entry 2))))
            (when buf (shell-command--insert-output buf ev))))
         ((and (eq mode 'filter) (consp ev) (eq (car ev) 'stdout))
          (setq drained (+ drained (length (cdr ev))))
          (aset entry 4 (concat (aref entry 4) (cdr ev))))
         ((and (eq mode 'filter) (consp ev) (eq (car ev) 'stderr))
          (setq drained (+ drained (length (cdr ev))))
          (aset entry 5 (concat (aref entry 5) (cdr ev))))
         (t ; (exit . CODE) -- nothing further to drain.
          (setq looping nil)))))
    (shell-process-kill proc)))

(defun shell-command--finish-view (entry code)
  "Wrap up a 'view-mode (M-!/M-&) job: report exit status, and for
`shell-command' (silent unless there was output or a failure) show the
buffer only if it actually has something in it."
  (let ((buf (get-buffer (aref entry 2))))
    (when buf
      (let ((empty (with-current-buffer buf (= (buffer-size) 0))))
        (if (and empty (integerp code) (= code 0))
            (message "(Shell command succeeded with no output)")
          (progn
            (shell-command--maybe-show buf (aref entry 9))
            (message "Shell command %s"
                     (if (and (integerp code) (= code 0))
                         "finished"
                       (format "exited abnormally with code %s" code)))))))))

(defun shell-command--finish-filter (entry code)
  "Wrap up a 'filter-mode (M-|) job: substitute the region with stdout
on success, leave it untouched on failure -- OR on success if the
region's CONTENT no longer matches what was piped to the command (see
the content-comparison guard below and this file's header \"Known
gaps\" entry, fix round R7). STDERR never becomes part of the buffer's
content in any branch -- see this file's header and
`shell-command-on-region''s docstring for why `separate' streams exist
at all."
  (let* ((stdout (aref entry 4))
         (stderr (aref entry 5))
         (beg-marker (aref entry 6))
         (end-marker (aref entry 7))
         (snapshot (aref entry 10))
         (orig-buf (get-buffer (aref entry 3))))
    ;; Drop ENTRY's own references now; the local variables above still
    ;; keep the marker objects alive for the rest of this function via
    ;; ordinary lexical binding.
    (shell-command--release-markers entry)
    (cond
     ((not orig-buf)
      ;; The buffer the region came from is gone -- nothing to
      ;; substitute into, and no undo-able edit is possible. Same
      ;; "dead target, drop safely" handling eshell.el:172-173 gives a
      ;; dead buffer.
      nil)
     ((and (integerp code) (= code 0))
      (let ((beg (marker-position beg-marker))
            (end (marker-position end-marker)))
        (cond
         ((>= beg end)
          ;; The user's own concurrent edits collapsed or inverted the
          ;; region while the command was running (e.g. deleted all of
          ;; it) -- substituting would either do nothing meaningful or
          ;; insert stdout at the wrong place. Refuse rather than guess
          ;; (fix round, R1).
          (message "Shell command on region: the region vanished while \
the command was running; not replacing anything"))
         ((not (string= (with-current-buffer orig-buf
                          (buffer-substring beg end))
                        snapshot))
          ;; The tracked span survived (didn't collapse) but its CONTENT
          ;; no longer matches what was piped to the command as stdin --
          ;; someone edited exactly at the region's end boundary, or
          ;; inside it, while the command ran. This editor's markers
          ;; have no insertion-type distinction (see the
          ;; BEG-MARKER/END-MARKER slot comment and this file's header),
          ;; so that edit got silently absorbed INTO the tracked span
          ;; instead of pushed past it -- replacing now would delete the
          ;; user's own edit together with the stale region and never
          ;; record it as its own undo step (fix round, R7 -- caught by
          ;; tail review's mechanical repro). Refuse, but don't just
          ;; discard the command's own work either.
          (let ((buf (shell-command--ensure-output-buffer)))
            (shell-command--insert-output buf stdout))
          (message "Shell command on region: the region was edited while \
the command was running; not replacing anything (output kept in %s)"
                   shell-command-output-buffer-name))
         (t
          (with-current-buffer orig-buf
            (delete-region beg end)
            (goto-char beg)
            (insert stdout))
          (if (string-empty-p stderr)
              (message "Shell command on region: replaced with output")
            (progn
              (let ((buf (shell-command--ensure-output-buffer)))
                (shell-command--insert-output buf stderr))
              (message
               "Shell command on region: replaced with output (stderr in %s)"
               shell-command-output-buffer-name)))))))
     (t
      (let ((buf (shell-command--ensure-output-buffer)))
        (shell-command--insert-output
         buf (if (string-empty-p stderr) stdout stderr))
        (shell-command--maybe-show buf (aref entry 9)))
      (message "Shell command on region exited abnormally with code %s; region unchanged"
               code)))))

(defun shell-command-process-pending-all ()
  "Drain output from every live shell-command job (idle-tick pump). Own
list, own logic -- see this file's header for why it does not share
eshell's pump. A dead target buffer gets its process killed and the
entry dropped, checked TWO ways for filter-mode jobs (fix round, R9):
proactively at the top of every pump tick (covers a process that has
gone quiet -- e.g. mid `sleep' -- and so delivers no chunk event for the
per-chunk check below to ever run against), and reactively on every
accumulate-chunk event (added in the earlier fix round, R4, and kept
for a prompt reaction while output IS actively streaming, without
waiting for the next full pump tick). 'view mode only had the reactive
check as of R4; the proactive gap above is equally real there but
untouched here -- out of scope for what R9 asked for."
  (let ((procs shell-command--procs)
        (remaining nil))
    (while procs
      (let* ((entry (car procs))
             (proc (aref entry 0))
             (mode (aref entry 1))
             (keep t)
             (looping t)
             (truncated nil))
        ;; R9: proactive liveness check for filter mode, before polling
        ;; at all -- a process that produced its last chunk long ago and
        ;; is now just sitting there (e.g. `sleep') would otherwise never
        ;; get noticed as orphaned until it finally exits on its own.
        (when (and (eq mode 'filter) (not (get-buffer (aref entry 3))))
          (shell-process-kill proc)
          (shell-command--release-markers entry)
          (setq looping nil)
          (setq keep nil))
        (while looping
          (let ((ev (shell-process-poll proc)))
            (cond
             ((null ev) (setq looping nil))
             ;; 'view mode: `merged' streams -> plain strings.
             ((and (eq mode 'view) (stringp ev))
              (when (shell-command--count entry (length ev))
                (setq truncated t))
              (let ((buf (get-buffer (aref entry 2))))
                (if (not buf)
                    (progn (shell-process-kill proc) (setq looping nil) (setq keep nil))
                  (shell-command--insert-output buf ev))))
             ;; 'filter mode: `separate' streams -> (stdout . STR) /
             ;; (stderr . STR). R4: check the ORIGINAL buffer is still
             ;; alive on every chunk too, same aggressiveness 'view
             ;; mode's branch above already has -- without this, killing
             ;; the source buffer mid-command left the process running
             ;; for no reason until `shell-command--finish-filter'
             ;; finally noticed at exit. (The proactive check above
             ;; handles the complementary case where no chunk ever
             ;; arrives to trigger this one -- R9.)
             ((and (eq mode 'filter) (consp ev) (eq (car ev) 'stdout))
              (let ((buf (get-buffer (aref entry 3))))
                (if (not buf)
                    (progn (shell-process-kill proc)
                           (shell-command--release-markers entry)
                           (setq looping nil) (setq keep nil))
                  (when (shell-command--count entry (length (cdr ev)))
                    (setq truncated t))
                  (aset entry 4 (concat (aref entry 4) (cdr ev))))))
             ((and (eq mode 'filter) (consp ev) (eq (car ev) 'stderr))
              (let ((buf (get-buffer (aref entry 3))))
                (if (not buf)
                    (progn (shell-process-kill proc)
                           (shell-command--release-markers entry)
                           (setq looping nil) (setq keep nil))
                  (when (shell-command--count entry (length (cdr ev)))
                    (setq truncated t))
                  (aset entry 5 (concat (aref entry 5) (cdr ev))))))
             (t ; (exit . CODE)
              (let ((code (cdr ev)))
                (if (eq mode 'view)
                    (shell-command--finish-view entry code)
                  (shell-command--finish-filter entry code)))
              (setq looping nil)
              (setq keep nil))))
          (when truncated
            (setq looping nil)
            (setq keep nil)
            ;; R3: fold in whatever was already buffered before killing,
            ;; rather than killing first and losing it.
            (shell-command--drain-before-kill entry)
            (if (eq mode 'view)
                (let ((buf (get-buffer (aref entry 2))))
                  (when buf
                    (shell-command--insert-output
                     buf "\n*** Output truncated (too large) ***\n")
                    (shell-command--maybe-show buf (aref entry 9))))
              (progn
                (let ((buf (shell-command--ensure-output-buffer)))
                  (shell-command--insert-output
                   buf (concat (aref entry 4) (aref entry 5)
                               "\n*** Output truncated (too large); region left unchanged ***\n"))
                  (shell-command--maybe-show buf (aref entry 9)))
                (shell-command--release-markers entry)
                (message "Shell command on region: output truncated, region unchanged")))))
        (when keep
          (setq remaining (cons entry remaining))))
      (setq procs (cdr procs)))
    (setq shell-command--procs remaining))
  nil)

(provide 'shell-command)
