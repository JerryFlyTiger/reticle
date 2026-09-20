;;; compile.el --- M-x compile/recompile, next-error/previous-error (M80) -*- lexical-binding: t -*-

;; --- Why this milestone targets elaboration, not lint ------------------
;; The `M-g n'/`M-g p' path already reaches verible's per-file lint
;; diagnostics through the LSP client (`lsp.el', M46/M54) -- mode line
;; shows `!1', jumping to the flagged line already works. What that path
;; CANNOT see is a cross-file elaboration error, because
;; `verible-verilog-ls' advertises `interFileDependencies: false' -- it
;; is a single-file checker by design, not a bug in this editor's LSP
;; client. Measured directly against a two-file repro (a `top.sv' that
;; instantiates a module `nonexistent_module' with a typo'd port name)
;; before writing a line of this file:
;;   verible-verilog-lint top.sv   -> exit 0, zero output
;;   verible-verilog-ls (M-g n)    -> 0 diagnostics published
;;   iverilog -t null -g2012 top.sv child.sv
;;                                 -> "top.sv:11: error: Unknown module
;;                                     type: nonexistent_module"
;; So the value this milestone adds is a way to run an EXTERNAL
;; elaboration-capable tool (iverilog, a lint driver script, whatever
;; the project's real build is) and navigate whatever cross-file errors
;; IT finds -- lint coverage was already solved before this file existed.
;;
;; Four real error-line shapes were captured from real tools before
;; designing the parser below (see `compile--error-prefix-regexp'):
;;   styled.sv:2:15-19: Explicitly define a storage type ... [Style: ...]
;;   top.sv:11: error: Unknown module type: nonexistent_module
;;   bad.v:3: syntax error
;;   bus/axi4_lite_arbiter.sv:35: sorry: Overriding the default variable
;;     lifetime is not yet supported.
;; Column-optional, "error:"-prefix-optional, and a `sorry:' line that is
;; neither an error nor a warning by GNU's usual vocabulary -- all four
;; drove the design below.
;;
;; --- Why `merged', not `separate' ---------------------------------------
;; `start-shell-process' defaults to merging stdout+stderr into one
;; interleaved stream; `shell-command-on-region' (shell-command.el) opts
;; INTO `separate' instead, because it needs to keep stdout (the thing it
;; may splice into a buffer) apart from stderr (the thing it never does).
;; This file has no such need -- everything lands in `*compilation*' as
;; plain text -- and `separate' mode does NOT reconstruct the true
;; interleaving of the two streams from two independently-polled queues,
;; so a build's `Entering directory X' (often stderr, e.g. `make -w')
;; immediately followed by a real error line (often stderr too, but
;; occasionally stdout, tool-dependent) could come back reordered under
;; `separate' in a way that never actually happened on the terminal.
;; GNU Emacs's own `compile.el' displays one combined, in-order log for
;; exactly this reason. Using `merged' here keeps that property -- but
;; only since M141. Before it, `merged' gave the child two separate pipes
;; drained by two reader threads, which does NOT preserve write order
;; either: measured 2026-09-14 with 300 alternating stdout/stderr lines,
;; 94, 538 and 266 of 600 positions came back out of order on an idle
;; machine, and a real `make -w -C sub' had its stdout `Entering
;; directory' land after the compiler's stderr error (2 of 20 runs under
;; load), so the error was resolved against the wrong directory and
;; dropped. `merged' now shares ONE pipe between stdout and stderr
;; (crates/elisp/src/shell.rs), so the order is the order of the child's
;; writes. The child's own stdio buffering (e.g. stdout fully buffered
;; when it is not a tty) is still outside this editor's control.
;;
;; --- Why a separate `compile--procs' / pump, not shell-command.el's ----
;; `shell-command--procs' (shell-command.el) is a single-job list by
;; design -- starting a second `M-!' abandons whatever the first was
;; still doing (see that file's own header). Folding `compile'/
;; `recompile' into that same list would mean a stray `M-!' typed while a
;; build is running SILENTLY KILLS THE BUILD, and vice versa -- two
;; independent workflows sharing one slot. `compile--procs' below is its
;; own list with its own pump (`compile-process-pending-all', wired into
;; `idle-tick' in lib.rs next to `shell-command-process-pending-all');
;; the two never see each other. Only `shell-command--insert-output' /
;; `shell-command--maybe-show' (both already parametrized on BUFFER, not
;; hardcoded to the shared-output-buffer name) are reused as-is.
;;
;; --- Why "does the file exist on disk" replaces GNU's regexp table -----
;; GNU's `compilation-error-regexp-alist' is a table of one-regexp-per-tool
;; entries (`gnu', `gcc-include', `lcc', ... dozens), because a single
;; generic FILE:LINE:COL: pattern hits a wall on ambiguity: plenty of
;; ordinary non-error output lines just happen to start with something
;; that LOOKS like "word:number:" (a timestamp, a ratio, a labeled
;; number in a summary line). This file uses one generic pattern
;; (`compile--error-prefix-regexp') but resolves that same ambiguity a
;; different way: after a candidate match, check whether FILE actually
;; exists on disk (relative to the compile's own working directory).
;; A line that only accidentally LOOKS like "file:line:" almost never
;; names a real, existing file at that exact relative path, so this one
;; guard kills the overwhelming majority of false positives GNU's table
;; exists to prevent -- without a single per-tool entry, and so new
;; tools (a custom lint wrapper script, whatever) need zero code changes
;; here to be recognized. The trade a table doesn't have to make: a
;; genuine error whose FILE has since been deleted or renamed will be
;; silently dropped. Accepted -- a stale reference to a file that no
;; longer exists isn't something `compile-next-error' could usefully
;; jump to anyway.
;;
;; --- Why the match is bounded to a fixed-length PREFIX of each line ----
;; (fix round R1, tail review -- FATAL bug, whole-process abort, not a
;; catchable elisp error): `crates/elisp/src/regex.rs''s backtracking is
;; REAL RUST FUNCTION RECURSION (`Inst::Split'/`Inst::Save' call back
;; into `self.run(...)'), not an explicit heap-allocated stack -- an
;; unbounded quantifier (`[^:\n]+' for FILE, or the `.*' this file used
;; to have at the end for MESSAGE) adds one Rust call frame per extra
;; character it backtracks through. Measured directly against
;; `(string-match compile--error-prefix-regexp (concat "top.sv:11: 
;;   error: " (make-string N ?x)))' before this fix: N = 2800
;;   overflowed a `cargo test' thread's 2MB default stack;
;;   N = 2500-2800 overflowed an interactive probe's (larger) stack:
;;   thread has overflowed its stack
;;   fatal runtime error: stack overflow, aborting
;;   (signal: 6, SIGABRT: process abort signal)
;; This is NOT something `condition-case' can catch -- a stack overflow
;; aborts the whole OS process, not just the current elisp call. And it
;; was NOT a contrived edge case: `compile-command''s own default is
;; `"cargo build"' (below) run against this 6-crate workspace's own
;; rustc/linker output, whose single lines routinely exceed 2500
;; characters, and EDA tool log lines are frequently longer still. So
;; the fix is structural, not a length check added on top of the same
;; unbounded pattern:
;;   1. `compile--error-prefix-regexp' matches ONLY the
;;      "FILE:LINE[:COL[-END]]:" header -- no trailing `.*' at all. BY
;;      ITSELF this is NOT independent protection (corrected here, fix
;;      round R7 -- tail review disproved the earlier claim that it
;;      was): `[^:\n]+' (FILE) is still its own greedy, unbounded
;;      quantifier, and against a string with NO colon anywhere to stop
;;      it, its backtracking depth is O(string length) regardless of
;;      whether a trailing `.*' exists anywhere else in the pattern.
;;      Verified directly: `(string-match compile--error-prefix-regexp
;;      (make-string N ?x))' -- i.e. this bounded-header pattern alone,
;;      with item 2's limit NOT applied -- survives N=2500 and aborts
;;      with the same SIGABRT at N=2800 and N=5000, matching the
;;      pre-fix crash threshold almost exactly. Dropping the trailing
;;      `.*' only helps on input that DOES have an early colon (all
;;      four real samples this milestone was built against do, which is
;;      why the earlier, wrong claim never showed up against them) --
;;      it provides no protection at all against a colon-free string.
;;      Item 2 below is therefore the ONLY defense that holds in
;;      general; item 1 is a real but narrower optimization on top of
;;      it, not a second independent line of defense.
;;   2. It is matched against `(substring line 0 (min
;;      compile--line-prefix-limit (length line)))', never the full
;;      LINE -- even the header-matching quantifiers above (`[^:\n]+'
;;      for FILE, `[0-9]+' for LINE/COL) now have their absolute
;;      worst-case backtracking depth capped at
;;      `compile--line-prefix-limit' characters, comfortably under the
;;      ~2500-2800 measured crash threshold.
;;   3. MESSAGE is no longer captured by the regexp at all -- it is
;;      `substring'-sliced (plain, non-recursive string copy) out of
;;      the ORIGINAL, un-truncated LINE, from `(match-end 0)' onward.
;;      An arbitrarily long message (a real, legitimate compiler
;;      output) is therefore still captured whole; only the REGEXP
;;      MATCHING COST is bounded, not what gets recorded.
;;   A line whose "FILE:LINE:COL:" header itself doesn't fit inside the
;;   first `compile--line-prefix-limit' characters (no real header
;;   sampled for this milestone comes anywhere close) fails to match
;;   within the truncated prefix and is treated as an ordinary output
;;   line, same as any other non-matching line -- see
;;   `compile--line-prefix-limit''s own doc comment for why this file
;;   bounds the MATCHED PREFIX rather than gating on the line's total
;;   length (which would also discard genuinely long MESSAGES that a
;;   short, cheap-to-match header happens to precede).
;;
;; --- Parsing timing: once, at process exit, not incrementally ----------
;; Errors are extracted from the WHOLE `*compilation*' buffer content in
;; one pass (`compile--parse-buffer-errors') when the job's exit event
;; arrives, not line-by-line as chunks stream in. Streamed chunks from
;; `shell-process-poll' do not respect line boundaries -- a
;; "top.sv:11: error: ..." line can arrive split across two separate
;; poll events, and incremental per-chunk matching would have to buffer
;; and reassemble partial lines to avoid misparsing (or missing) a match
;; straddling a chunk boundary. A full-buffer reparse at exit sidesteps
;; that entirely, and there is no workflow cost: nobody expects to
;; navigate a build's errors before the build has finished.
;;
;; --- Entry vector layouts ------------------------------------------------
;; `compile--procs' entries: #[PROC DIR CHARS]
;;   PROC  -- the `start-shell-process' handle.
;;   DIR   -- the working directory the job was started in, carried
;;            through to exit time so `compile--parse-buffer-errors' can
;;            resolve a relative FILE the same way the compiler itself
;;            would have (relative to its own cwd, not to whatever buffer
;;            happens to be current when the process exits).
;;   CHARS -- running total of output characters seen so far (fix round
;;            R3, tail review), checked against `compile-max-output-chars'
;;            the same way `shell-command--procs''s CHARS slot
;;            (shell-command.el) is -- see that defvar's own doc comment
;;            for why an unbounded compile job needs the same cap a
;;            one-shot `M-!' does (a misconfigured `compile-command' that
;;            busy-loops is just as real a risk here, and R1's crash is
;;            easier to reach the more text `*compilation*' accumulates).
;;
;; `compile--errors' entries: #[FILE LINE COL SEVERITY MESSAGE BUFFER-POS]
;;   FILE       -- absolute path (`expand-file-name'-resolved against the
;;                 job's DIR), already confirmed to exist on disk.
;;   LINE       -- 1-based line number, integer.
;;   COL        -- 1-based character offset into that line, or nil if
;;                 the matched line had none. A verible "15-19" range
;;                 keeps only the START value -- GNU's own compile.el
;;                 output has no notion of an end column to jump to
;;                 either, and neither does any command in this file.
;;   SEVERITY   -- 'error / 'warning / 'sorry / 'note (`compile--severity').
;;                 'sorry covers iverilog's "not yet supported" messages,
;;                 which are neither an error nor a warning in the usual
;;                 sense; 'note is the catch-all for everything else
;;                 (e.g. verible's un-prefixed lint-style messages) so
;;                 every parsed entry always lands in a defined bucket,
;;                 never falls through unclassified.
;;   MESSAGE    -- the full text after the "FILE:LINE[:COL]: " prefix.
;;   BUFFER-POS -- this entry's line's starting character position
;;                 inside `*compilation*' at PARSE time. Used only by
;;                 `compile-goto-error-at-point' (RET) to map "which
;;                 line is point on" back to an entry -- see that
;;                 function's own doc comment for what happens if the
;;                 buffer gets hand-edited afterward and this drifts.
;;
;; --- Known gaps (v1, not attempted here) --------------------------------
;; - (M141) A tool that ANNOUNCES its own directory changes (`make -C
;;   sub', `make[N]: Entering/Leaving directory ...') is now tracked --
;;   `compile--parse-buffer-errors' keeps a directory stack
;;   (`compile--match-directory-line', `compile--directory-regexp',
;;   mirroring GNU Emacs 30.2's own `compilation-directory-matcher',
;;   measured directly) and resolves each line against the stack's top
;;   instead of the single `compile--dir' captured at job start. What
;;   remains unhandled is a tool that changes directory WITHOUT
;;   announcing it (plain `cd sub && tool', or `make' invoked without
;;   `-w' on some platforms -- macOS make 3.81 was measured to print no
;;   directory line at all in that case): such a line still resolves
;;   against the wrong directory, `file-exists-p' fails, and the line is
;;   dropped -- but no longer SILENTLY for every case: `compile--parse-
;;   buffer-errors' counts a dropped line whose header matched the
;;   FILE:LINE:COL regexp, whose severity classified as 'error/'warning/
;;   'sorry (fix round F7 -- 'note stays uncounted), and whose FILE text
;;   is NOT all-digits (fix round F2 -- excludes a timestamp column like
;;   \"14:23:01: warning: ...\" from inflating the count); `compile--
;;   finish'/`compile-process-pending-all''s truncation message both
;;   report that count (`compile--dropped-suffix'). This is exactly the
;;   evidence that exists, not \"always visible evidence a line went
;;   missing\": a 'note-severity line, an all-digits-FILE line, and a
;;   line that fails the header regexp entirely (so it was never a
;;   counting candidate at all -- e.g. a tool that reports an error with
;;   no FILE:LINE:COL shape whatsoever) can all still go missing with no
;;   count and no message, and which line and where it should have
;;   resolved are never recoverable from output text alone even when the
;;   count IS reported.
;; - (M141) Because the FILE:LINE header is tried before the directory
;;   regexp (so a real error whose message happens to say "Entering
;;   directory 'x'" stays an error), a directory line that ITSELF matches
;;   the header is read as an error and never pushed. That needs no
;;   `make: '-style prefix (the prefix's colon is followed by a space,
;;   which the header rejects) and a path with a `:<digits>:' segment,
;;   e.g. "Entering directory '/tmp/run:2024:5/build'": FILE becomes
;;   "Entering directory '/tmp/run", the message has no error keyword, so
;;   the line is neither tracked nor counted. Real make always prints the
;;   prefix; only a tool imitating the line without one can hit this.
;; - (M141) Deliberate divergence from GNU: GNU resolves a RELATIVE
;;   "Entering directory" path against the compilation buffer's own
;;   directory (`compilation-directory', effectively the job's start
;;   dir), not against whatever directory is currently on top of the
;;   stack -- measured directly: a synthetic `make[2]: Entering
;;   directory 'deep'' three levels into a `make -C sim' run resolves,
;;   under GNU, to `<job-dir>/deep', NOT `<job-dir>/sim/deep', even
;;   though the tool that printed it is actually running IN
;;   `<job-dir>/sim/deep' (that's what "Entering directory 'deep'",
;;   printed by a process whose cwd is already `<job-dir>/sim', means).
;;   This file resolves a relative "Entering directory" path against the
;;   CURRENT stack top instead, because that is where the build really
;;   is; GNU's own rule produces a path (`<job-dir>/deep') that need not
;;   exist on disk at all in this scenario, which is a worse answer than
;;   this file's, not a compatibility feature worth preserving.
;; - (M141, F9, NOT fixed) A line whose real content is truncated by
;;   `compile--line-prefix-limit' (see that variable's own doc comment)
;;   could, by byte-exact coincidence, happen to end in something
;;   shaped like \"...directory 'x'\" right at the truncation boundary
;;   and get misread as a directory announcement. Needs the truncated
;;   bytes to line up exactly with that shape; not attempted here.
;; - LINE (an entry's line number) is a plain integer captured once at
;;   PARSE time, not a marker -- if the user edits the TARGET SOURCE
;;   FILE (adds/removes lines above the error) between a compile
;;   finishing and calling `compile-next-error'/`M-g n', the stored
;;   LINE no longer points at the error's actual current line, and
;;   `compile--goto-error' jumps to whatever now happens to be at that
;;   line number with no indication anything drifted. This is a
;;   DIFFERENT risk from BUFFER-POS drift (documented above, which is
;;   about hand-editing `*compilation*' itself, not the file the error
;;   is IN) -- fixing it would need the same marker machinery
;;   `shell-command-on-region' (shell-command.el) uses for its region,
;;   applied to every open target buffer an error might point into, not
;;   attempted here.
;; - The `[lsp]' prefix on `next-error'/`previous-error''s LSP fallback
;;   (see the dispatcher section below) depends on `next-diagnostic'/
;;   `previous-diagnostic' (lsp.el) always returning their own
;;   `message' call's formatted string as their function's return
;;   value. Verified true by reading every branch of both functions'
;;   bodies, but that is NOT a contract either function's doc comment
;;   declares -- a future edit to lsp.el that adds a branch not ending
;;   in `message', or reorders forms so `message' isn't last, would
;;   silently drop the `[lsp]' prefix (not crash; `next-error' just
;;   does nothing further when the return value isn't a string) with no
;;   compile-time signal anywhere. Also: this file's own tests only
;;   drive the "no live LSP client" branch of that fallback (the only
;;   one reachable without standing up a real LSP connection, out of
;;   scope for this milestone's test suite) -- the "client live, has
;;   diagnostics" and "client live, no diagnostics" branches of the
;;   fallback are exercised by lsp.el's own tests, not this file's, and
;;   were never observed going through THIS dispatcher.
;; - TOCTOU on the existence check: `compile--parse-error-line' checks
;;   `file-exists-p' exactly once, at PARSE time (compile job exit). If
;;   the file is deleted between then and a later
;;   `compile-next-error'/RET navigating to it, `compile--goto-error''s
;;   `find-file' call doesn't re-check -- it silently opens a fresh,
;;   empty buffer for that now-nonexistent path (ordinary `find-file'
;;   behavior for a path that doesn't exist), with nothing distinguishing
;;   that from genuinely visiting an empty file.
;; - The cap-crossing/process-exit overlap tail review originally found
;;   (fix round R6's Finding 2 -- a double message and a double
;;   `compile--errors' reparse when both events landed in the same pump
;;   tick) is now structurally impossible: `compile-process-pending-all'
;;   checks `truncated' every loop iteration and stops on the SAME
;;   iteration the cap is crossed, so the primary loop can never reach a
;;   separately-queued exit event afterward (see that function's own
;;   doc comment for the full mechanism). What remains, and is NOT
;;   exercised by any test here: if the process HAD already exited right
;;   around the moment the cap was crossed, that real exit code is now
;;   simply discarded rather than ever inspected -- `compile--drain-
;;   before-kill''s own poll loop consumes it (its `t' branch is a
;;   no-op past stopping the drain) without calling `compile--finish'.
;;   Not a correctness bug (the output was truncated either way, so
;;   "success vs. failure" is already moot for a run that got killed
;;   mid-stream), but a real, deliberate behavior change worth naming:
;;   the truncation message is now unconditionally the final word on
;;   that job, even in the rare case where the real command had, in
;;   fact, already finished.
;; - `compile-process-pending-all' only checks whether `*compilation*'
;;   still exists ONCE per entry, at the top of each pump tick --unlike
;;   `shell-command-process-pending-all' (shell-command.el), whose
;;   'view-mode branch re-checks on EVERY streamed chunk (its own R4/R9
;;   comments document exactly this reactive-vs-proactive distinction).
;;   If the user kills `*compilation*' mid-job, this file's pump keeps
;;   polling and feeding `compile--count' (silently dropping each chunk
;;   once `get-buffer' comes back nil inside the loop) for the REST of
;;   that same tick, and the process itself isn't killed until the
;;   NEXT idle tick notices the buffer is still gone. Not a correctness
;;   problem (nothing is written anywhere invalid; the job is still
;;   reaped, just one tick later than `shell-command.el' would manage
;;   it) but this file's own header claims its pump works "the same
;;   way `shell-command-process-pending-all' enforces its own cap" --
;;   this one specific difference is real and not covered by that claim.

(defvar compile-output-buffer-name "*compilation*"
  "Output buffer for `compile'/`recompile'. Like
`shell-command-output-buffer-name' (shell-command.el), this editor
tracks at most ONE compile job at a time -- see `compile--procs''s doc
comment above.")

(defvar compile-max-output-chars (* 4 1024 1024)
  "Cap on how many characters of combined stdout+stderr a single compile
job may produce before it gets killed (fix round R3, tail review --
`compile-process-pending-all' had NO cap at all before this, unlike
`shell-command-max-output-chars' (shell-command.el, M79), which this
mirrors: same reasoning -- `start-shell-process' itself enforces no
limit (see that variable's own doc comment), so a misconfigured
`compile-command' that busy-loops (or a real build that just never
stops emitting warnings) would otherwise grow `*compilation*' without
bound. Set 4x `shell-command-max-output-chars' (1 MiB), not the same
value -- a full build's legitimate output (this editor's own `cargo
build', or a multi-file elaboration run) is routinely far larger than
a one-shot `M-!' command's, and a cap so tight it routinely truncates
genuine, wanted build output would defeat the point of watching a
build run at all.")

(defvar compile--drain-poll-limit 512
  "Bound on how many `shell-process-poll' calls
`compile--drain-before-kill' makes while flushing a process's
already-buffered-but-undelivered output before killing it. Same
reasoning as `shell-command--drain-poll-limit''s own doc comment
(shell-command.el) -- a still-running process producing output faster
than this can drain it would otherwise spin here forever.")

(defvar compile--drain-overshoot-multiplier 4
  "How many multiples of `compile-max-output-chars' `compile--drain-
before-kill' is allowed to fold in past the cap before giving up and
killing anyway. Same reasoning as `shell-command--drain-overshoot-
multiplier''s own doc comment (shell-command.el) -- a single
`shell-process-poll' call can hand back an arbitrarily large chunk
regardless of iteration count, so this bounds the drain pass by total
characters too, not just call count.")

(defvar compile-command "cargo build"
  "The command `compile' runs, and `recompile' reruns unchanged.
`compile' always pre-fills its prompt with the CURRENT value of this
variable (so accepting the default just reruns the same thing) and
`setq's it to whatever the user actually submits, remembering it across
invocations for the lifetime of the session -- same shape as GNU
Emacs's own `compile-command'. The default here has nothing
Verilog-specific about it -- this editor's own build is Rust -- so
change it (or just type over the prompt) the first time you actually
run this against a Verilog project.")

(defvar compile--dir nil
  "Working directory `recompile' reuses -- the directory `compile' was
last invoked from (by project root, see `compile''s doc comment), kept
as its OWN variable rather than folded into `compile-command' because a
compile command and the directory it should run in are two independent
pieces of state: editing the command in the minibuffer must not lose
track of where to run it, and vice versa.")

(defvar compile--procs nil
  "Active compile job (at most one; see `compile-output-buffer-name''s
doc comment); see the header comment at the top of this file for the
per-entry vector layout.")

(defvar compile--errors nil
  "Ascending-by-appearance-in-the-log list of parsed error entries from
the most recent finished compile job (see the header comment for the
per-entry vector layout), or nil before any job has finished. Walked in
the order errors appear in the build's own output, like GNU Emacs's
`next-error' -- NOT sorted by file/line, since a tool's own error order
often already reflects a meaningful priority (e.g. the first
elaboration failure that made everything after it unreliable).")

(defvar compile--current-index nil
  "0-based index into `compile--errors' of the entry
`compile-next-error'/`compile-previous-error' last jumped to, or nil
before either has been called since the last `compile--reset'. Point
alone cannot serve this role the way `next-diagnostic' (lsp.el) uses it
for single-buffer diagnostics -- compile errors span MULTIPLE files, so
there is no single buffer whose point could encode \"which one is
current\". `compile--reset' (called at the start of every fresh
`compile'/`recompile') always clears this alongside `compile--errors',
which is what keeps it from ever pointing past the end of a list that
shrank out from under it -- there is no scenario in this file where
`compile--errors' shrinks WITHOUT `compile--current-index' being reset
in the same step.")

(defun compile--severity (msg)
  "Classify MSG (an error entry's full message text) into one of
'error/'warning/'sorry/'note. Checked in that order so a message that
happens to mention more than one keyword (unlikely, but not
impossible) still gets the most actionable label. 'note is the
catch-all -- e.g. verible's un-prefixed lint-style messages
(\"Explicitly define a storage type ...\") carry none of the other three
keywords at all, and every parsed entry must land in SOME defined
bucket rather than falling through unclassified."
  (cond
   ((string-match-p "\\berror\\b" msg) 'error)
   ((string-match-p "\\bwarning\\b" msg) 'warning)
   ((string-match-p "\\bsorry\\b" msg) 'sorry)
   (t 'note)))

(defvar compile--line-prefix-limit 300
  "Max number of characters, counted from the START of an output line,
that `compile--parse-error-line' will ever hand to
`compile--error-prefix-regexp'. Exists ONLY to bound regexp backtracking
cost -- see this file's header (\"Why the match is bounded to a
fixed-length PREFIX of each line\", fix round R1) for the exact crash
this guards against: `crates/elisp/src/regex.rs' backtracks via REAL
Rust call recursion, so an unbounded quantifier matched against an
unbounded-length string can blow the OS thread's stack and SIGABRT the
whole process -- not something `condition-case' can catch. Measured
crash threshold was ~2500-2800 characters (a `cargo test' thread's 2MB
stack overflowed at 2800, an interactive session's larger stack a bit
higher); 300 leaves a wide margin above every real FILE:LINE:COL header
sampled for this milestone (the longest, `bus/axi4_lite_arbiter.sv:35:
sorry: ...', has an 18-character header) while staying nowhere near the
crash threshold. Only the HEADER match is bounded by this -- the
MESSAGE that follows is `substring'-sliced out of the original,
un-truncated line (plain string copy, no backtracking), so a message
longer than this limit is still captured in full; see
`compile--parse-error-line'.")

(defvar compile--error-prefix-regexp
  "^\\([^:\n]+\\):\\([0-9]+\\)\\(:\\([0-9]+\\)\\(-[0-9]+\\)?\\)?:[ \t]*"
  "FILE:LINE[:COL[-END]]: header pattern -- deliberately has NO trailing
`.*' for MESSAGE (fix round R1; see this file's header for why). This
pattern must ALWAYS be matched against a `compile--line-prefix-limit'-
bounded PREFIX of a line (see `compile--parse-error-line'), never the
whole line -- dropping the trailing `.*' does NOT make this pattern
safe to run unbounded on its own (corrected here, fix round R7 -- an
earlier version of this comment claimed it did, disproved by tail
review). `[^:\n]+' (FILE) is still a greedy, unbounded quantifier, and
against a colon-free string its backtracking depth is O(string length)
regardless of the trailing `.*''s absence -- verified directly:
`(string-match compile--error-prefix-regexp (make-string N ?x))' run
WITHOUT the prefix bound survives N=2500 and SIGABRTs at N=2800 and
N=5000, matching this file's pre-R1 crash threshold. The
`compile--line-prefix-limit' bound applied by every actual call site is
what makes this safe, not this pattern's own shape. Group 1 = FILE,
group 2 = LINE, group 4 = COL (nil if absent). Verified against all
four real-tool shapes in this file's header comment before being
written.")

(defvar compile--directory-regexp
  "\\(Entering\\|Leaving\\) directory [`']\\(.+\\)'$"
  "Matches a build tool's directory-change announcement line (M141),
mirroring GNU Emacs 30.2's own `compilation-directory-matcher'
(measured directly against real gmake/make output, not written from
memory -- see this file's header for the GNU-parity rule that requires
that). Unanchored at the START, same as GNU's own pattern, so any
\"make[N]: \"/\"gmake[N]: \" prefix a real tool prepends still matches;
anchored at the END with `$' so a directory path that itself happens to
contain a quote character doesn't stop the greedy group early. The
open-quote character class `[`']' matches either quote style measured:
gmake 4.4.1 opens and closes with a plain apostrophe; macOS make 3.81
run with `-w' opens with a backquote and closes with an apostrophe.
Group 1 = \"Entering\" or \"Leaving\" (decides push vs. pop in
`compile--match-directory-line'); group 2 = the raw, possibly-relative
directory text.")

(defun compile--strip-trailing-cr (line)
  "Return LINE with exactly one trailing carriage return (\\r) stripped,
or LINE unchanged if it doesn't end in one (fix round F8: a build
running under CRLF line endings, e.g. some `printf'/tool output piped
through a CRLF-preserving path). `compile--directory-regexp''s trailing
`$' anchors to end-of-STRING in this regex engine, not \"before a
trailing \\r\" the way some other engines' `$' does, so an unstripped
\\r would make a genuine \"Entering directory '...'\\r\" line fail to
match at all. Only used before matching the directory regexp -- the
error header regexp (`compile--error-prefix-regexp') has no end-of-line
anchor, so a trailing \\r there already just becomes the tail of MESSAGE
harmlessly, same as before this fix."
  (let ((n (length line)))
    (if (and (> n 0) (eq (aref line (1- n)) ?\r))
        (substring line 0 (1- n))
      line)))

(defun compile--match-directory-line (line)
  "Return (KIND . DIR-RAW) if LINE is a directory-change announcement
matching `compile--directory-regexp' (KIND is the string \"Entering\"
or \"Leaving\", DIR-RAW the raw path text between the quotes), or nil
otherwise. LINE is stripped of one trailing \\r first (fix round F8,
`compile--strip-trailing-cr') and then matched against a
`compile--line-prefix-limit'-bounded PREFIX, same defense
`compile--parse-error-line-raw' applies to the FILE:LINE header regexp
(see this file's header, \"Why the match is bounded to a fixed-length
PREFIX\") -- `compile--directory-regexp''s `.+' is just as unbounded a
quantifier as that pattern's FILE group, and this function is called on
every line `compile--parse-buffer-errors' didn't already recognize as an
error header, so it needs the same crash guard. No real
directory-announcement line sampled for this milestone comes anywhere
close to the limit; one that does is silently treated as an ordinary
output line, the same accepted trade `compile--line-prefix-limit''s own
doc comment already makes."
  (let* ((stripped (compile--strip-trailing-cr line))
         (bound (min compile--line-prefix-limit (length stripped)))
         (prefix (substring stripped 0 bound)))
    (when (string-match compile--directory-regexp prefix)
      (cons (match-string 1 prefix) (match-string 2 prefix)))))

(defun compile--all-digits-p (s)
  "Non-nil if S is non-empty and every character in it is a decimal
digit (fix round F2: a timestamp line like \"14:23:01: warning: disk
usage high\" matches `compile--error-prefix-regexp' with FILE = \"14\",
which will almost never exist on disk but would otherwise still get
counted as a dropped error/warning line -- this predicate is how
`compile--parse-buffer-errors' recognizes and excludes that shape of
false positive from the count. Does NOT affect entry-parsing itself
(`compile--parse-error-line' still runs the ordinary `file-exists-p'
check regardless) -- only the COUNT a nonexistent-FILE header
contributes to `compile--dropped-suffix'."
  (and (> (length s) 0)
       (let ((i 0) (n (length s)) (ok t))
         (while (and ok (< i n))
           (unless (and (>= (aref s i) ?0) (<= (aref s i) ?9))
             (setq ok nil))
           (setq i (1+ i)))
         ok)))

(defun compile--parse-error-line-raw (line dir)
  "Match LINE's FILE:LINE[:COL[-END]]: header against
`compile--error-prefix-regexp' and resolve FILE against DIR, WITHOUT
checking whether FILE exists on disk. Returns a vector [FILE LINE-NUM
COL SEVERITY MSG FILE-RAW] (FILE-RAW is the UNRESOLVED text matched for
FILE, before `expand-file-name' -- fix round F2 needs it to recognize an
all-digit timestamp column before it's turned into an absolute path),
or nil if the (`compile--line-prefix-limit'-bounded) header doesn't
match at all. Split out of `compile--parse-error-line' by M141, and
(fix round F5/F6) called EXACTLY ONCE per line by
`compile--parse-buffer-errors' -- see that function's own doc comment
for why the earlier version of this docstring's claim (\"without running
the regexp twice\") was false: `compile--parse-buffer-errors' used to
call `compile--parse-error-line' (which runs this) and then, on a nil
result, call this again itself."
  (let* ((bound (min compile--line-prefix-limit (length line)))
         (prefix (substring line 0 bound)))
    (when (string-match compile--error-prefix-regexp prefix)
      (let* ((file-raw (match-string 1 prefix))
             (file (expand-file-name file-raw dir))
             (line-num (string-to-number (match-string 2 prefix)))
             (col-str (match-string 4 prefix))
             (col (and col-str (string-to-number col-str)))
             ;; `(match-end 0)' is an offset into PREFIX, but PREFIX is
             ;; LINE's own characters 0..BOUND unchanged -- the same
             ;; offset is valid directly against LINE, no translation
             ;; needed. MESSAGE is sliced from the UNTRUNCATED LINE (not
             ;; PREFIX), so an arbitrarily long message survives whole;
             ;; see `compile--line-prefix-limit''s doc comment.
             (msg (substring line (match-end 0) (length line))))
        (vector file line-num col (compile--severity msg) msg file-raw)))))

(defun compile--parse-error-line (line offset dir)
  "Parse one LINE of compile output at character OFFSET within
`*compilation*' (see the header's BUFFER-POS doc), resolving a relative
FILE against DIR (the directory this LINE's own header should be
resolved against -- the job's own working directory, or, since M141,
whatever directory a preceding \"Entering directory\" announcement put
on top of `compile--parse-buffer-errors''s stack). Returns an error
entry vector, or nil if `compile--parse-error-line-raw' doesn't match
at all, OR if it matches but the named FILE doesn't exist on disk (see
this file's header for why that's the ambiguity guard here instead of a
regexp table). Kept as its own function, with this exact signature and
behavior, because two tests (`compile_parse_error_line_survives_long_
colonless_prefix', `compile_colonless_5000_chars_survives_under_
default_prefix_limit') call it directly -- `compile--parse-buffer-
errors' itself (fix round F5/F6) no longer calls this function at all,
to avoid running the header regexp twice per line; it calls
`compile--parse-error-line-raw' once and does its own `file-exists-p'
check inline."
  (let ((raw (compile--parse-error-line-raw line dir)))
    (when (and raw (file-exists-p (aref raw 0)))
      (vector (aref raw 0) (aref raw 1) (aref raw 2) (aref raw 3) (aref raw 4) offset))))

(defun compile--dropped-suffix (dropped)
  "Return a \" (...)\" message suffix reporting DROPPED error/warning
lines whose FILE:LINE header matched but whose named file doesn't exist
on disk (M141), or \"\" if DROPPED is 0. Shared by `compile--finish' and
`compile-process-pending-all''s truncation path so the two messages
report this consistently without duplicating the wording."
  (cond
   ((= dropped 0) "")
   ((= dropped 1) " (1 error line names a file that does not exist)")
   (t (format " (%d error lines name files that do not exist)" dropped))))

(defun compile--parse-buffer-errors (dir)
  "Full single-pass parse of `*compilation*''s current content into
(ENTRIES . DROPPED-COUNT) (see this file's header for why this runs
once at exit rather than incrementally per streamed chunk). DIR is the
job's own working directory, used as the resolution directory whenever
the directory stack described below is empty.

M141: walks a directory stack alongside the lines. For each line, the
FILE:LINE:COL header regexp is tried FIRST via
`compile--parse-error-line-raw' (fix round F1) -- ONLY a line that does
NOT match it is even considered as a directory-change announcement via
`compile--match-directory-line'. This order matters: a genuine error
whose MESSAGE happens to contain text shaped like \"Entering directory
'x'\" (e.g. a compiler literally reporting \"error: while Entering
directory 'x'\") still matches the header regexp first and is parsed as
an ordinary error line, never mistaken for a directory announcement and
never allowed to push/pop the stack. `compile--match-directory-line'
strips one trailing \\r first (fix round F8, `compile--strip-trailing-
cr') so CRLF build output still matches.

\"Entering\" pushes its (possibly relative, see this file's header for
the GNU-divergence rationale) directory, resolved against the CURRENT
stack top (or DIR if the stack is empty); \"Leaving\" pops. Popping an
empty stack is a no-op -- and, fix round F4, not merely \"harmless\" but
UNOBSERVABLE in this interpreter: `(cdr nil)' is `nil'
(`crates/elisp/src/value.rs:241-246'), so `(setq stack (cdr stack))'
already does nothing to an empty STACK on its own; the surrounding
`(when stack ...)' guard changes no behavior and exists only for
readability (naming the case), not correctness.

DROPPED-COUNT is how many non-directory lines matched
`compile--parse-error-line-raw''s header, classified as 'error/'warning/
'sorry severity (fix round F7 added 'sorry -- iverilog's \"sorry:\" is a
build-stopping unsupported-construct error, not merely a note), and had
a FILE-RAW that is NOT all-digits (fix round F2, `compile--all-digits-
p' -- excludes a timestamp column like \"14:23:01: warning: ...\" from
inflating the count), but were rejected only because their resolved
FILE doesn't exist -- `compile--finish'/`compile-process-pending-all'
report this count (`compile--dropped-suffix') instead of silently
losing the line. This is NOT exhaustive evidence of every dropped line:
a 'note-severity line, an all-digits-FILE line, and a line that fails
the header regexp entirely (so it was never a candidate at all) are all
invisible to this count -- it counts exactly the error/warning/sorry,
non-timestamp, header-matched-but-file-missing case, covering the
tool-changes-directory-without-announcing-it gap this file's header
names as still unhandled, and nothing broader.

Calls `compile--parse-error-line-raw' EXACTLY ONCE per line, directory
lines included: the FILE:LINE header is tried first and only a line it
rejects is offered to `compile--match-directory-line' (fix rounds F1,
F5/F6) -- it does its own `file-exists-p' check and
builds the final entry vector inline, rather than calling
`compile--parse-error-line' (which would run the same regexp a second
time)."
  (let ((buf (get-buffer compile-output-buffer-name)))
    (when buf
      (with-current-buffer buf
        (let ((lines (split-string (buffer-string) "\n" nil))
              (offset (point-min))
              (out nil)
              (dropped 0)
              (stack nil))
          (dolist (line lines)
            (let* ((cur-dir (if stack (car stack) dir))
                   (raw (compile--parse-error-line-raw line cur-dir)))
              (cond
               (raw
                (if (file-exists-p (aref raw 0))
                    (push (vector (aref raw 0) (aref raw 1) (aref raw 2)
                                  (aref raw 3) (aref raw 4) offset)
                          out)
                  (when (and (memq (aref raw 3) '(error warning sorry))
                             (not (compile--all-digits-p (aref raw 5))))
                    (setq dropped (1+ dropped)))))
               (t
                (let ((dirmatch (compile--match-directory-line line)))
                  (when dirmatch
                    (if (string= (car dirmatch) "Entering")
                        (push (expand-file-name (cdr dirmatch) cur-dir) stack)
                      (when stack (setq stack (cdr stack)))))))))
            (setq offset (+ offset (length line) 1)))
          (cons (nreverse out) dropped))))))

;; --- Column jump: the first place in this codebase that jumps to a
;; specific COLUMN, not just a line -----------------------------------
;; `lsp-definition-at-point' and every other existing cross-file jump
;; (lsp.el) only ever `forward-line' and stop -- there is no precedent
;; anywhere else in this codebase for "column" meaning anything. The
;; convention adopted here: COL is 1-based (every tool sampled for this
;; milestone reports columns starting at 1, matching GNU's own
;; convention) and counts CHARACTERS, not bytes -- this editor's buffer
;; positions are character counts end-to-end (`buffer.rs':
;; `adjust_positions_delete' counts via `.chars().count()', not byte
;; length), so treating a tool's reported column as a character offset
;; is the only convention consistent with how every OTHER position in
;; this editor already works, even though it will be silently wrong for
;; a tool that (unusually) reports byte columns against a non-ASCII
;; line -- no such tool showed up in any of the four real samples this
;; milestone was built against.
(defun compile--goto-column (col)
  "Advance point from the current line's beginning by COL-1 characters
(COL 1-based, or nil for \"no column, stay at the line's start\"),
clamped to BOTH ends of the current line -- a stale or simply
out-of-range column (the file was edited since the tool ran; the tool
miscounted) degrades to \"somewhere on the right line\" rather than
silently overshooting onto an ADJACENT line, which would be far more
confusing than just being a few characters off within the correct one.

Clamped on the LOW end too (fix round R2, tail review): the regexp
group feeding COL is `[0-9]+', which accepts \"0\" -- a tool that
(unusually) reports a 0 column would make `(1- col)' evaluate to -1,
and without a lower clamp `(goto-char (+ (point) -1))' lands on the
LAST character of the PREVIOUS line for any error not on line 1,
exactly the \"confusingly overshoots onto a different line\" failure
this function's own doc comment above already promises not to have."
  (when col
    (let ((bol (point)))
      (goto-char (max bol (min (+ bol (1- col)) (line-end-position)))))))

(defun compile--goto-error (entry)
  "Jump to ENTRY (an error entry vector, see this file's header),
switching buffers via `find-file' if needed -- compile errors routinely
span multiple files, the same cross-file jump `lsp-definition-at-point'
(lsp.el) already does."
  (find-file (aref entry 0))
  (goto-char (point-min))
  (forward-line (1- (aref entry 1)))
  (compile--goto-column (aref entry 2))
  (message "[compile] (%d/%d) %s"
           (1+ compile--current-index)
           (length compile--errors)
           (aref entry 4)))

(defun compile-next-error ()
  "Jump to the next entry in `compile--errors', wrapping to the first
after the last (same wrap convention `next-diagnostic', lsp.el, already
uses)."
  (interactive)
  (if (not compile--errors)
      (message "No compilation errors")
    (setq compile--current-index
          (if compile--current-index
              (mod (1+ compile--current-index) (length compile--errors))
            0))
    (compile--goto-error (nth compile--current-index compile--errors))))

(defun compile-previous-error ()
  "Jump to the previous entry in `compile--errors', wrapping to the last
before the first."
  (interactive)
  (if (not compile--errors)
      (message "No compilation errors")
    (setq compile--current-index
          (if compile--current-index
              (mod (1- compile--current-index) (length compile--errors))
            (1- (length compile--errors))))
    (compile--goto-error (nth compile--current-index compile--errors))))

(defun compile--index-of (entry)
  "0-based position of ENTRY (compared by object identity, `eq') within
`compile--errors', or nil if not found."
  (let ((entries compile--errors) (idx 0) (found nil))
    (dolist (e entries)
      (when (and (not found) (eq e entry))
        (setq found idx))
      (setq idx (1+ idx)))
    found))

(defun compile--error-at-buffer-pos (pos)
  "The entry in `compile--errors' whose recorded BUFFER-POS is exactly
POS, or nil. Matched by EXACT equality against the position captured at
parse time, not by searching for whichever entry's line currently
contains POS -- `*compilation*' is an ordinary, editable buffer (nothing
here makes it read-only), so if you hand-edit it after a job finishes,
the recorded BUFFER-POS offsets can drift out of sync with what's
actually on each line. Falling back to nil (\"no error on this line\")
in that case is the least-surprising failure mode available: jumping to
a nearby-but-wrong entry because of stale bookkeeping would be worse
than just not recognizing the (edited) line at all."
  (let ((entries compile--errors) (found nil))
    (dolist (e entries)
      (when (and (not found) (= (aref e 5) pos))
        (setq found e)))
    found))

(defun compile-goto-error-at-point ()
  "RET in `*compilation*': jump to whichever error entry started on the
line point is currently on, or report \"No error on this line\" if none
did (either because this line was never an error line, or because the
buffer was edited afterward -- see `compile--error-at-buffer-pos')."
  (interactive)
  (let* ((bol (line-beginning-position))
         (entry (compile--error-at-buffer-pos bol)))
    (if (not entry)
        (message "No error on this line")
      (setq compile--current-index (compile--index-of entry))
      (compile--goto-error entry))))

(defun compile--ensure-output-buffer ()
  "Get-or-create `*compilation*', installing `compilation-mode' and its
local keymap the first time. Parallel to `shell-command--ensure-output-
buffer' (shell-command.el), but not a call to it -- that function hard-
codes its OWN buffer name and mode symbol rather than taking them as
parameters (see this file's header for why that one function alone
needed a from-scratch rewrite instead of reuse)."
  (let ((buf (get-buffer-create compile-output-buffer-name)))
    (with-current-buffer buf
      (unless (eq (major-mode-internal-get) 'compilation-mode)
        (major-mode-internal-set 'compilation-mode)
        (let ((map (make-sparse-keymap)))
          (define-key map "q" 'quit-source-return)
          (define-key map "n" 'compile-next-error)
          (define-key map "p" 'compile-previous-error)
          (define-key map "RET" 'compile-goto-error-at-point)
          ;; M130: `j'/`k' are plain cursor motion (`next-line'/
          ;; `previous-line'), deliberately NOT the same as `n'/`p' --
          ;; `n'/`p' here jump to the NEXT/PREVIOUS parsed error's
          ;; location, while vim's `j'/`k' just move the cursor one
          ;; line. `gg' is deliberately not bound anywhere in this
          ;; milestone -- see dired.el's header note for the mechanical
          ;; reason (`g' collision risk in `Keymap::define-sequence').
          (define-key map "j" 'next-line)
          (define-key map "k" 'previous-line)
          (define-key map "G" 'end-of-buffer)
          (use-local-map map))))
    buf))

(defun compile--reset ()
  "Kill any previous compile job, clear `*compilation*', and drop the
previously parsed error list -- called at the start of every
`compile'/`recompile' invocation. Mirrors `shell-command--reset'
(shell-command.el) on `compile--procs' instead, its own list -- see this
file's header for why the two never share state. Clearing
`compile--errors'/`compile--current-index' together here is what keeps
the index from ever pointing past the end of a since-shrunk list (see
`compile--current-index''s own doc comment)."
  (dolist (entry compile--procs)
    (shell-process-kill (aref entry 0)))
  (setq compile--procs nil)
  (setq compile--errors nil)
  (setq compile--current-index nil)
  (let ((buf (get-buffer compile-output-buffer-name)))
    (when buf
      (with-current-buffer buf
        (let ((inhibit-read-only t))
          (erase-buffer))
        (set-buffer-modified-p nil)))))

(defun compile--finish (code dir)
  "Wrap up a finished compile job: parse `*compilation*''s full content
for error entries (DIR is the job's own working directory, needed to
resolve relative FILEs the same way the compiler itself would, and as
the base of `compile--parse-buffer-errors''s directory stack) and
report the exit status, how many were found, and (M141) how many more
were dropped because their resolved FILE doesn't exist
(`compile--dropped-suffix')."
  (let* ((result (compile--parse-buffer-errors dir))
         (dropped (cdr result)))
    (setq compile--errors (car result))
    (setq compile--current-index nil)
    (message "Compile %s%s%s"
             (if (and (integerp code) (= code 0))
                 "finished"
               (format "exited abnormally with code %s" code))
             (if compile--errors
                 (format " (%d error%s found)"
                         (length compile--errors)
                         (if (= (length compile--errors) 1) "" "s"))
               "")
             (compile--dropped-suffix dropped))))

(defun compile--start (cmd dir)
  "Shared body of `compile'/`recompile': reset any previous job, spawn
CMD in DIR (falling back to `shell-command--default-dir' if DIR is nil
-- covers `recompile' called before any `compile' has ever run, so
`compile--dir' is still nil), and show `*compilation*' immediately.
Unlike `shell-command' (M-!, silent unless there was output), this
always shows the buffer right away, same as `async-shell-command' (M-&)
-- a build is something you expect to watch run, not a one-shot command
where silence is the success case."
  (let ((dir (or dir (shell-command--default-dir)))
        (source (quit-source-of-current)))
    (compile--reset)
    (setq compile--dir dir)
    (let ((proc (condition-case nil
                    (start-shell-process cmd dir)
                  (error nil))))
      (if (not proc)
          (message "Cannot start process: %s" cmd)
        (let ((buf (compile--ensure-output-buffer)))
          (shell-command--maybe-show buf source)
          (setq compile--procs (list (vector proc dir 0))))))))

(defun compile ()
  "Prompt for a shell command (pre-filled with the current
`compile-command') and run it asynchronously in the current buffer's
project root (`lsp--project-root', lsp.el -- a build is normally
invoked from the project's top, not wherever the file being edited
happens to live; falls back to `shell-command--default-dir' for a
buffer with no file at all, same fallback `shell-command' itself uses).
Remembers both the command (`compile-command') and the directory
(`compile--dir') for `recompile'."
  (interactive)
  (let* ((file (buffer-file-name))
         (dir (if file (lsp--project-root file) (shell-command--default-dir))))
    (read-string "Compile command: "
                 (lambda (cmd)
                   (setq compile-command cmd)
                   (compile--start cmd dir))
                 compile-command "compile")))

(defun recompile ()
  "Rerun `compile-command' in `compile--dir' without prompting -- the
\"just do it again\" companion to `compile'."
  (interactive)
  (compile--start compile-command compile--dir))

(defun compile--count (entry n)
  "Add N to ENTRY's running output-character total (CHARS, slot 2);
returns t once the total exceeds `compile-max-output-chars'. Mirrors
`shell-command--count' (shell-command.el) -- does NOT kill the process
itself, see `compile--drain-before-kill'."
  (let ((total (+ (aref entry 2) n)))
    (aset entry 2 total)
    (> total compile-max-output-chars)))

(defun compile--drain-before-kill (entry)
  "Poll ENTRY's process, folding any output it had already buffered
before the cap was noticed straight into `*compilation*', then kill it.
Mirrors `shell-command--drain-before-kill' (shell-command.el) -- see
its doc comment for why a drain pass is needed at all (`shell-process-
kill' drops undelivered buffered output) and why it is bounded BOTH by
iteration count (`compile--drain-poll-limit') and by total characters
drained (`compile--drain-overshoot-multiplier' x `compile-max-output-
chars'), not iteration count alone."
  (let ((proc (aref entry 0))
        (n 0)
        (drained 0)
        (drain-cap (* compile-max-output-chars compile--drain-overshoot-multiplier))
        (looping t))
    (while (and looping
                (< n compile--drain-poll-limit)
                (< drained drain-cap))
      (setq n (1+ n))
      (let ((ev (shell-process-poll proc)))
        (cond
         ((null ev) (setq looping nil))
         ((stringp ev)
          (setq drained (+ drained (length ev)))
          (let ((buf (get-buffer compile-output-buffer-name)))
            (when buf (shell-command--insert-output buf ev))))
         (t (setq looping nil))))) ; (exit . CODE) -- nothing further to drain.
    (shell-process-kill proc)))

(defun compile-process-pending-all ()
  "Drain output from the active compile job (idle-tick pump). Own list,
own logic -- see this file's header for why it does not share
`shell-command-process-pending-all''s (shell-command.el). Unlike that
function, there is no 'view/'filter mode split here -- every compile
job behaves like 'view mode (stream straight into `*compilation*'), so
this is considerably shorter. Enforces `compile-max-output-chars' (fix
round R3, tail review -- this pump had no cap at all before) the same
way `shell-command-process-pending-all' enforces its own cap: kill the
process, drain what it had already buffered
(`compile--drain-before-kill'), and leave a truncation notice in the
buffer rather than the job just running forever.

The `truncated' check runs INSIDE the inner `while' loop, once per
poll, not after it exits (fix round R6, tail review -- the first
version of this function checked only after the loop had already run
to completion, so `truncated' being set on some iteration did NOT stop
the loop early; for a job that keeps producing output with no gap
between polls -- exactly the runaway case `compile-max-output-chars'
exists to catch -- the loop kept right on polling and inserting past
the cap until `shell-process-poll' finally returned nil or an exit
event on its own, with nothing bounding how far past the cap it could
overshoot in the meantime). Checking every iteration, the same
structure `shell-command-process-pending-all' already uses, means the
loop stops on the SAME iteration that crosses the cap.

That timing change has a knock-on effect on the \"cap crossed and the
process's own exit event were both already sitting in the poll queue\"
case (tail review Finding 2, observed under the OLD out-of-loop check):
since a single `shell-process-poll' call only ever returns ONE event
(a chunk XOR an `(exit . CODE)', never both), and the loop now stops
on the very iteration where a chunk crosses the cap, the primary loop
here NEVER reaches a queued exit event in that scenario -- it would
need a separate, later poll call this function no longer makes.
`compile--drain-before-kill' (below) does its own follow-up polling,
and if IT is the one that reaches the queued exit event, it just stops
draining (its own `t' branch is a no-op past `(setq looping nil)') and
proceeds straight to `shell-process-kill' -- it does NOT call
`compile--finish'. Net effect after this fix: on the truncation path,
`compile--finish' is now NEVER called and the process's real exit code
is discarded (not just superseded) -- the truncation message and a
single `compile--parse-buffer-errors' re-parse are the only outcome,
never overwritten by (or overwriting) a separate finish message. This
replaces the OLD path's double-message/double-reparse bug outright
rather than leaving a race -- the two are now structurally impossible
to interleave, not just less likely to."
  (let ((procs compile--procs)
        (remaining nil))
    (while procs
      (let* ((entry (car procs))
             (proc (aref entry 0))
             (dir (aref entry 1))
             (keep t)
             (truncated nil))
        (if (not (get-buffer compile-output-buffer-name))
            (progn (shell-process-kill proc) (setq keep nil))
          (let ((looping t))
            (while looping
              (let ((ev (shell-process-poll proc)))
                (cond
                 ((null ev) (setq looping nil))
                 ((stringp ev)
                  (when (compile--count entry (length ev))
                    (setq truncated t))
                  (let ((buf (get-buffer compile-output-buffer-name)))
                    (when buf (shell-command--insert-output buf ev))))
                 (t ; (exit . CODE)
                  (compile--finish (cdr ev) dir)
                  (setq looping nil)
                  (setq keep nil))))
              ;; R6: checked every iteration (not after the loop exits)
              ;; so a cap crossing stops polling on the SAME iteration
              ;; it happened, matching `shell-command-process-pending-
              ;; all''s structure -- see this function's own doc comment
              ;; above for the resulting exit-event-discarding behavior.
              (when truncated
                (setq looping nil)
                (setq keep nil)
                ;; R3: fold in whatever was already buffered before
                ;; killing, rather than killing first and losing it
                ;; (same reasoning as `shell-command--drain-before-kill').
                (compile--drain-before-kill entry)
                (let ((buf (get-buffer compile-output-buffer-name)))
                  (when buf
                    (shell-command--insert-output
                     buf "\n*** Output truncated (too large) ***\n")))
                (let* ((result (compile--parse-buffer-errors dir))
                       (dropped (cdr result)))
                  (setq compile--errors (car result))
                  (setq compile--current-index nil)
                  (message "Compile output truncated (too large); process killed%s"
                           (compile--dropped-suffix dropped)))))))
        (when keep
          (setq remaining (cons entry remaining))))
      (setq procs (cdr procs)))
    (setq compile--procs remaining))
  nil)

;; --- M-g n / M-g p dispatcher --------------------------------------------
;; Defined here, not in lsp.el (out of scope for this milestone -- see
;; the task spec), and bound over the OLD `next-diagnostic'/
;; `previous-diagnostic' bindings in simple.el instead of adding a
;; second pair of keys, so `M-g n' keeps meaning exactly one thing
;; regardless of which source currently has something to show.

(defun next-error ()
  "M-g n: if the most recent compile job found any errors AND
`*compilation*' is still around, walk those (`compile-next-error');
otherwise fall back to `next-diagnostic' (lsp.el)'s LSP-diagnostic walk
-- unchanged behavior for anyone who never runs `compile' at all. Either
way, the echo message is prefixed with which source it came from
(`[compile]' / `[lsp]') so there's no ambiguity about what just
happened -- `next-diagnostic' and `compile-next-error' both always
return their own message text as their function value (relying on
`message' itself returning the formatted string), which is how the
`[lsp]' prefix gets attached here without lsp.el needing to know
anything about this file."
  (interactive)
  (if (and compile--errors (get-buffer compile-output-buffer-name))
      (compile-next-error)
    (let ((result (next-diagnostic)))
      (when (stringp result)
        (message "[lsp] %s" result)))))

(defun previous-error ()
  "M-g p: the `previous-diagnostic'/`compile-previous-error' counterpart
to `next-error' above -- see its doc comment."
  (interactive)
  (if (and compile--errors (get-buffer compile-output-buffer-name))
      (compile-previous-error)
    (let ((result (previous-diagnostic)))
      (when (stringp result)
        (message "[lsp] %s" result)))))

(provide 'compile)
