;;; search.el --- M-x search-project + editable results (M82/M83) -*- lexical-binding: t -*-

;; This is milestone 1 of a five-milestone "text search system" -- see
;; the task spec this file was built against. The whole point: `M-x
;; search-project' prompts once, runs an external search engine
;; asynchronously, and streams matches into `*search*' AS THEY ARRIVE --
;; the user can switch away and keep editing while the search is still
;; running, same asynchronous shape `compile'/`recompile' (compile.el,
;; M80) already established for builds.
;;
;; --- Why this is NOT just `compile.el' with a different command -------
;; `compile-process-pending-all' (compile.el) parses its ENTIRE buffer
;; ONCE, at process-exit time -- see that function's own header comment
;; for why: a streamed chunk from `shell-process-poll' can split a line
;; at any byte offset, so incremental per-chunk parsing would have to
;; buffer and reassemble partial lines anyway, and nobody expects to
;; navigate a BUILD's errors before the build finishes, so paying that
;; cost once at the end is free.
;;
;; A search result buffer does NOT get that luxury: results must be
;; navigable (`n'/`p'/RET') WHILE the job is still streaming -- that is
;; this milestone's entire reason to exist (see the task's own framing:
;; "the user can switch away immediately and keep editing, with results
;; streaming in as it runs"). So this file has to do the thing
;; compile.el's header explains away: buffer a possibly-
;; partial trailing line across `search-process-pending-all' calls
;; (`search--feed-chunk') and only ever hand a COMPLETE line to the
;; FILE:LINE:COL: parser. This is the core new piece of engineering in
;; this file; everything else (the two-list-shape job entry, the pump
;; wired into `idle-tick', the `n'/`p'/RET' triad, the column-clamp
;; jump) is a structural copy of a pattern compile.el already
;; established, adapted to search's own independent state.
;;
;; --- Why an independent job list/pump, not compile's or shell-
;; command's -----------------------------------------------------------
;; Same reasoning `compile.el''s own header gives for not sharing
;; `shell-command--procs': three independent workflows (a one-shot `M-!'
;; command, a build, a project-wide search) sharing one job slot would
;; mean starting any one of them SILENTLY KILLS whichever of the other
;; two happened to still be running. `search--procs' is its own list
;; with its own pump (`search-process-pending-all', wired into
;; `idle-tick' in lib.rs next to `compile-process-pending-all' and
;; `shell-command-process-pending-all'); none of the three ever look at
;; either of the other two's state.
;;
;; --- Why the search engine is a plain shell command line, not argv ----
;; `search-program' (default "rg") and `search-arguments-literal'/
;; `search-arguments-regexp' (the flag strings, each ending in the `--'
;; that separates flags from the pattern -- see the next section for why
;; there are TWO of these, not one) are all plain strings spliced into
;; ONE command line handed to `start-shell-process' (which always runs
;; it through `sh -c', see shell.rs's `spawn'), the exact same shape
;; `compile-command' already uses. Two reasons this needs to be
;; swappable at all, not hardcoded to `rg':
;;   1. Architecturally, a later milestone is expected to add an
;;      ast-grep-based structural search engine alongside (or instead
;;      of) plain-text `rg' -- keeping the engine and its flags in
;;      user-visible variables means that milestone edits data, not this
;;      file's control flow.
;;   2. This file's OWN test suite (`search_tests.rs') cannot depend on
;;      a real `rg' binary being installed on whatever machine runs
;;      `cargo test' -- `search-program'/`search-arguments-literal'/
;;      `search-arguments-regexp' let every test but a handful
;;      substitute a fake generator (`printf'/`bash -c', same technique
;;      `compile_tests.rs' already uses for a fake compiler) for the
;;      real thing. The tests that genuinely need REAL `rg' (real regex
;;      engine behavior can't be faked by a canned-output generator --
;;      see the next section) skip themselves when `rg' isn't on PATH,
;;      the same precedent this codebase already has for
;;      `verible-verilog-ls' (see CLAUDE.md).
;; The output SHAPE this file's parser depends on --
;; `FILE:LINE:COL:TEXT' -- is exactly what both arguments variables'
;; shared flags (`--line-number --column --no-heading --color never
;; --smart-case') produce from real `rg'; a fake test generator only has
;; to match that shape, not actually be `rg'.
;;
;; --- Why literal (fixed-string) search is the DEFAULT, and regex is a
;; separate, explicitly-opted-into command -------------------------------
;; `rg''s PATTERN argument is a regular expression by default. This
;; editor's target user (see CLAUDE.md: real Verilog RTL, not toy
;; examples) types bracket/paren/dot-heavy literal text into a search
;; box constantly and it is almost never intended as a regex:
;; `[7:0]' (a bus width) is a valid bracket EXPRESSION matching any
;; single occurrence of the literal characters `7', `:', or `0' --
;; three completely unrelated characters, not a range, since `:' isn't a
;; valid range separator -- so a plain regex search for `[7:0]' silently
;; returns a much broader (and WRONG) result set with no error of any
;; kind. `top.core.alu' (a hierarchical path) has `.' meaning
;; "any character" three times over. `$display'/`$clog2' (system
;; tasks) has `$' meaning "end of line" in most positions. None of these
;; fail loudly -- they just quietly match the wrong thing, which is
;; worse than an error: a build/lint tool crashing on bad input gets
;; noticed immediately, a search silently returning a plausible-looking
;; but wrong result set does not. So `search-project' passes
;; `search-arguments-literal' (which includes `--fixed-strings' /`-F')
;; by default, and treating PATTERN as a real regex is a SEPARATE
;; command, `search-project-regexp', using `search-arguments-regexp'
;; (identical flags minus `-F') instead -- opt-in, never silent.
;;
;; The task this file was built against called for a `C-u' PREFIX-ARG
;; variant of `search-project' instead of a separate command. That
;; mechanism does not exist anywhere in this editor: `interactive'
;; spec codes `P'/`p' are hardcoded to nil/1 (see
;; `crates/core/src/builtins/ui.rs'), and `shell-command.el''s own
;; header already documents hitting this exact wall and choosing
;; separate commands (`M-!'/`M-&'/`M-|') over a `C-u' variant for
;; exactly this reason. `search-project-regexp' follows that existing,
;; already-established precedent rather than this file inventing a
;; prefix-argument subsystem (an architecture-level decision well
;; outside a single milestone's scope) to support one flag.
;;
;; Key binding (fix round R2, coordinator decision): the whole search
;; family moved from two flat keys (`C-c s'/`C-c S') to a `C-c s' PREFIX
;; -- `C-c s s' (`search-project'), `C-c s r' (`search-project-regexp'),
;; `C-c s a' (`search-again') -- mirroring `C-c l ...' (the LSP family,
;; simple.el) structurally, so later milestones in this series (editable
;; results, live-as-you-type search, export) each get their own key
;; under this same prefix instead of claiming unrelated flat keys. See
;; `simple.el''s own comment at these bindings for the full rationale
;; and the LOAD-BEARING note on why `C-c s' itself must never be bound
;; directly to a command again.
;;
;; --- Why the search root is found via `.git', not `lsp--project-root'
;; (lsp.el) --------------------------------------------------------------
;; `lsp--project-root' walks up looking for ANY of several markers
;; (`.git', `Cargo.toml', `verible.filelist', ...) because an LSP
;; server's own indexing scope is a per-language, per-tool question.
;; This file's own concern is narrower and language-agnostic: find
;; "the top of whatever repository this file lives in" so a
;; project-wide search doesn't silently stop at the first subdirectory
;; boundary. `demo/rtl/' (this project's own reference RTL, see
;; CLAUDE.md) is intentionally split into `pkg/ core/ mem/ bus/ top/'
;; sibling directories with no single non-`.git' marker file anywhere
;; above all of them -- a search started from a buffer visiting
;; `demo/rtl/core/alu.sv' MUST still see a hit in `demo/rtl/top/top.sv',
;; which only `.git'-rooted search (not e.g. "nearest directory
;; containing a Cargo.toml") reliably delivers for an arbitrary
;; real-world RTL tree.
;;
;; --- M83: editable search results (wgrep-level) ------------------------
;; `C-x C-q' in `*search*' makes the buffer writable; editing a result
;; line's TEXT and `C-c C-c' writes every changed line back to its own
;; file, at its own line, batched across however many files the edits
;; span; `C-c C-k' discards and restores the buffer verbatim. This is
;; the most dangerous of the five milestones in this series -- it
;; rewrites the user's own RTL source -- so every design choice below
;; is driven by "what happens when this assumption is wrong", not just
;; "what's the simplest thing that works".
;;
;; D1: Writes ALWAYS go through a real buffer + `save-buffer', never
;; `write-region' or any direct-to-disk path. Three independent reasons,
;; any ONE of which would be sufficient on its own:
;;   (a) `save-buffer' is the ONLY path in this editor with an
;;       external-change conflict guard (mtime+size locally, a content
;;       digest over `/ssh:', see `save-buffer''s own doc comment,
;;       editing.rs) -- `write-region' has none at all.
;;   (b) `write-region' does not update `disk_state' -- using it on a
;;       file that ALSO happens to be open in a buffer would poison that
;;       buffer's next `save-buffer' with a false conflict report
;;       against a write the user made themselves (documented gap in
;;       `save-buffer''s own comment, editing.rs).
;;   (c) There is NO `revert-buffer' in this editor (editing.rs's own
;;       comment says so plainly). If a target file is already open with
;;       unsaved edits and this feature wrote straight to disk instead
;;       of into that buffer, the open buffer would silently now
;;       disagree with disk, and the NEXT time the user saves THAT
;;       buffer (for an entirely unrelated reason) it would overwrite
;;       this feature's own edit with stale in-memory content -- data
;;       loss with no error, no warning, and no way to reload from disk
;;       to notice it happened. Routing through `find-file' (which
;;       reuses an already-open buffer, see `find-file-internal',
;;       editing.rs) instead makes this structurally impossible: our
;;       edit and the user's prior unsaved edits both live in the same
;;       buffer, and one `save-buffer' call persists both together.
;;
;; D2: Applying NEVER trusts `search--results''s `BUFFER-POS' (a plain
;; integer, exact-match only -- see that variable's own doc comment).
;; Editing any EARLIER line in `*search*' shifts every following line's
;; character offset, and this feature's entire premise is "the user
;; just edited a bunch of result lines" -- BUFFER-POS drift here isn't
;; an edge case, it is what always happens on the very first edit.
;; Instead, applying re-parses `*search*' FRESH, line by line, using the
;; same `FILE:LINE:COL:' header shape M82 already established
;; (`search--result-regexp') -- immune to position drift by construction,
;; and needs no changes to M82's own already-three-gates-passed data
;; structures (`search--results' stays exactly as M82 left it, used only
;; by `n'/`p'/RET', never by the edit path).
;;
;; Re-parsing alone isn't enough, though: PATTERN-matching a
;; possibly-EDITED line against `search--result-regexp' can't tell "this
;; row was never a result" apart from "this row WAS a result but the
;; user typed over its header too". The mechanism actually used is
;; ROW-INDEXED, not content-keyed: `search-edit-mode' entry snapshots
;; the buffer's raw text ONE time (`search--edit-original-text'); v1
;; does not support inserting or deleting whole lines (see the "v1 does
;; not include" list below), so the Nth line of the original snapshot
;; and the Nth line of the buffer at apply time are ALWAYS the same
;; logical row, however its text changed. `search-edit-apply' walks both
;; line lists in lockstep: unchanged rows are skipped outright (D3);
;; changed rows have BOTH their original and current text parsed as a
;; header, and only a row whose header parses to the exact SAME
;; (FILE LINE COL) triple on both sides is a genuine text edit -- a
;; row whose header failed to parse in either version is reported, not
;; applied (D6), and neither is content-keyed matching (a global search
;; for "does some OTHER row's original entry happen to match this text
;; now") ever attempted -- that would risk silently applying an edit to
;; the wrong file/line entirely.
;;
;; D3: Only a row whose text CHANGED (`string=' against the row-indexed
;; original) is treated as an edit at all -- untouched result lines
;; never even reach the per-line disk-conflict check (D4), let alone get
;; written anywhere.
;;
;; D4: Immediately before writing a changed row, its target file's
;; current LINE N (read from the buffer `find-file' just produced --
;; freshly re-read from disk if that buffer wasn't already open, or the
;; ALREADY-in-memory content, unsaved edits and all, if it was) must
;; equal the snapshot's ORIGINAL text for that row, or the edit is
;; skipped and reported, never forced. This is a LINE-granularity guard,
;; not a whole-file hash -- deliberately: two edits to the SAME file's
;; two DIFFERENT lines are independent facts, and one line having
;; changed on disk since the search ran must not block writing back the
;; other, unaffected line. (`save-buffer' below still ALSO runs its own
;; whole-file mtime+size guard, which is coarser and can independently
;; refuse the write for reasons this line-level check can't see, e.g. a
;; change to a completely different line in the same file made AFTER
;; this feature's own `find-file' already read the current one in --
;; that failure is caught the same way as any other per-file exception,
;; see D7.)
;;
;; D5: Edits to the SAME file are applied in DESCENDING line order.
;; Same reasoning `replace-region-contents' (editing.rs) already uses
;; for its own back-to-front hunk application: an edit to a LATER line
;; never shifts the position of an EARLIER line still waiting to be
;; applied, so line numbers computed once, up front, all stay valid
;; without any offset bookkeeping.
;;
;; D6: A row whose header can no longer be parsed at apply time -- the
;; user typed over the `FILE:LINE:COL:' prefix itself, not just the text
;; after it -- is reported, never silently dropped. `compile.el' has an
;; already-documented gap of exactly this shape (a line that only
;; accidentally looks like a header gets silently treated as ordinary
;; output, see that file's header); this feature does not repeat it,
;; because here the cost of a silent drop is an edit the user believes
;; they made simply never reaching disk, with no indication why.
;;
;; D7: Applying is NOT atomic across files, or even across a single
;; file's own multiple edited lines relative to OTHER files. Each
;; target file is visited and saved inside its own `condition-case'
;; (same per-item try/skip/keep-going shape `dired.el''s own batch
;; delete/rename already use, see `dired--delete-entries'/
;; `dired--rename-entries' and `dired--error-first-line', reused here
;; directly rather than reinvented -- dired.el loads before this file,
;; see lib.rs's load order): one file being read-only, deleted, or
;; failing `save-buffer''s own conflict guard is recorded and skipped,
;; never aborts the rest of the batch. Unlike dired's own batch
;; operations, though, this one does NOT blindly trust that a target
;; isn't already open with state that matters -- see D1(c) above; that
;; is the one gap in dired's own precedent this file deliberately does
;; NOT copy.
;;
;; D8: `*search*' is read-only by DEFAULT now (`search--install-view-
;; keymap' calls `set-buffer-read-only t') -- M82 never set this at all
;; (documented as a known gap in that milestone, see below). Two places
;; already needed `inhibit-read-only' to keep working the moment this
;; flips on, both already present in the M82 code, neither touched by
;; this milestone: `search--insert-line' inserts via `shell-command--
;; insert-output' (shell-command.el), which already wraps its own
;; insertion in `(let ((inhibit-read-only t)) ...)'; `search--reset'
;; already wraps its own `erase-buffer' the same way. Both were dead
;; defensive code before this milestone (there was nothing to inhibit);
;; from here on they are load-bearing -- removing either would break
;; the M82 streaming-insert path outright the instant a search ran
;; against a read-only `*search*'. Verified directly, not just reasoned
;; about (see `search_streams_fake_engine_output_into_search_buffer' and
;; friends in `search_tests.rs', all still green against a read-only
;; buffer -- test 12 in this milestone's own list makes this explicit).
;;
;; D9: `search-edit-mode' refuses to run at all while `search--procs' is
;; non-nil ("a search is still running"), instead of e.g. queuing the
;; edit-mode entry for later. Entering edit mode takes a one-time text
;; snapshot; `search-process-pending-all' (M82) keeps appending to the
;; SAME buffer on every idle tick for as long as a job runs, which would
;; either invalidate that snapshot the instant more output arrived, or
;; (worse) let a user start editing text that is about to be silently
;; appended past. Refusing outright is simpler and safer than trying to
;; keep a live snapshot in sync with a still-streaming buffer.
;;
;; D10/D11: `search-mode''s local keymap gains `C-x C-q' ->
;; `search-edit-mode' (GNU/`wgrep''s own vocabulary for the same
;; operation). Entering/leaving edit mode swaps the ENTIRE local keymap
;; via two small, explicit, idempotent functions (`search--install-view-
;; keymap'/`search--install-edit-keymap'), NOT folded into `search--
;; ensure-output-buffer''s own `unless (eq ... \\='search-mode)' guard --
;; that guard is a ONE-TIME buffer initialization check (only ever true
;; the very first time `*search*' is created), so re-running it can
;; never be the mechanism that switches keymaps back and forth on every
;; edit-mode entry/exit. `search-edit-mode' (M83's own new major mode,
;; the writable state) is added to `evil-emacs-state-modes' (evil.el)
;; for the exact same reason `search-mode' itself already is (M82) --
;; its own `C-c C-c'/`C-c C-k' local bindings would otherwise lose to
;; evil normal-state's bindings for those same keys the moment the user
;; is actually IN insert state editing text and drops back to normal
;; state to save.
;;
;; --- M83: what "the report" actually says --------------------------------
;; `search-edit-apply' ends with ONE `message' call summarizing what
;; happened: how many lines were written across how many files, and --
;; only if anything didn't make it -- how many were skipped and why
;; (each skip reason is a short, self-contained string: `FILE:LINE:
;; reason' for a line-level D4 conflict, or `FILE: reason' for a
;; per-file D7 exception, `dired--error-first-line''s own one-line-only
;; truncation reused directly). There is no separate report buffer --
;; v1 keeps this to the echo area, same as `compile--finish'/`dired--
;; summarize' already do for their own batch outcomes.
;;

;; --- Known gaps (v1, not attempted here) --------------------------------
;; - No `M-g n'/`next-error' integration. `M-g n' already has an
;;   established priority order (compile errors, then LSP diagnostics --
;;   see compile.el's own dispatcher and its priority-order tests in
;;   compile_tests.rs); folding search results into that chain is a
;;   separate decision this milestone does not make. `n'/`p'/RET' here
;;   are BUFFER-LOCAL to `*search*' only.
;; - A result's LINE/COL is a snapshot from PARSE time, not a marker --
;;   editing the target file between a search finishing and jumping to a
;;   result drifts the same way an unfixed `compile.el' entry does (see
;;   that file's own header for the identical gap and why fixing it
;;   needs marker machinery not attempted here either).
;; - (SUPERSEDED by M83) `*search*' is now read-only by default
;;   (D8) with an explicit, opt-in writable `search-edit-mode'
;;   (`C-x C-q') -- the M82-era gap this bullet used to describe (an
;;   ordinary editable buffer, hand-edits desyncing `BUFFER-POS') no
;;   longer applies: M83's own apply path (D2) never reads `BUFFER-POS'
;;   at all, re-parsing the buffer fresh instead. `compile--error-at-
;;   buffer-pos''s own BUFFER-POS-drift caveat (compile.el) still
;;   applies unchanged to `n'/`p'/RET' navigation in THIS file (M82's
;;   own code, untouched by M83), just not to write-back.
;; - No mid-search pattern editing -- once `search-project' starts a
;;   job, the only way to change the pattern is to start a NEW search
;;   (which kills the old one, see `search--reset'). Live-as-you-type
;;   minibuffer search is a later milestone in this series.
;; - Like `compile-process-pending-all' (compile.el, see that function's
;;   own header for the identical tradeoff), this file's pump only
;;   checks whether `*search*' still exists ONCE per idle tick, at the
;;   top -- not on every streamed chunk the way
;;   `shell-command-process-pending-all' (shell-command.el) does. If the
;;   user kills `*search*' mid-job, the running engine process isn't
;;   killed until the NEXT idle tick notices the buffer is gone.
;; - `search--shell-quote' quotes PATTERN for POSIX `sh -c' (single
;;   quotes, `'\'''-escaping any embedded single quote) but does nothing
;;   for a PATTERN containing a NUL byte or other `sh'-hostile control
;;   byte -- not reachable from this editor's own minibuffer input path
;;   today, so not defended against here.
;; - A FILE name containing a literal `:' defeats `search--result-
;;   regexp''s FILE group (`[^:\n]+', which stops at the first colon) --
;;   the header would parse with the wrong FILE/LINE/COL split (or fail
;;   to match `search--line-prefix-limit''s numeric groups at all,
;;   depending on where the colon falls), so the line degrades to the
;;   normal "displayed but not jumpable" outcome the unparseable-line
;;   path already handles gracefully (see `search--parse-result-line').
;;   Not defended against with a smarter FILE pattern here -- a colon in
;;   a filename is vanishingly rare for this editor's target workflow
;;   (real Verilog RTL trees, see CLAUDE.md), and `compile.el' accepts
;;   the identical limitation in its own `compile--error-prefix-regexp'
;;   for the same reason.
;; - `*search*' has no visual distinction (a face, a prefix glyph,
;;   anything) between a line that IS in `search--results' (jumpable)
;;   and one that isn't (an unparseable line, or the eventual
;;   truncation banner) -- the only way to find out which is pressing
;;   RET and reading the resulting message (\"No search result on this
;;   line\" vs. an actual jump). Acceptable for v1 (results are the
;;   overwhelming majority of any real search's output), but worth
;;   naming: a future milestone adding `font-lock'/overlay support to
;;   this buffer should highlight jumpable lines.
;; - `shell-process-kill' (shell.rs) contains an UNBOUNDED, no-timeout
;;   blocking `child.wait()' call (see that function's own code,
;;   `crates/elisp/src/shell.rs' -- pre-existing, not introduced by this
;;   file). Every consumer of `start-shell-process' inherits this same
;;   risk (a child that ignores SIGKILL/SIGTERM and never actually
;;   exits would hang the calling `kill' call, and everything after it,
;;   forever), but this file's own exposure to it is LARGER than
;;   `compile.el''s or `shell-command.el''s: those two only ever call
;;   `shell-process-kill' in response to a DELIBERATE user action
;;   (`compile'/`recompile' starting a new job, `q'/killing the output
;;   buffer) or a cap crossing that itself requires the user to have
;;   asked for an enormous amount of output. This file adds a THIRD,
;;   fully automatic trigger that fires on ANY idle tick with no user
;;   action at all: `search-max-results' being crossed inside
;;   `search-process-pending-all', which runs on every single idle tick
;;   for the lifetime of any search job. A pathological search engine
;;   that hangs on SIGKILL would therefore surface as the WHOLE EDITOR
;;   freezing on some ordinary idle tick, not just as a specific command
;;   failing to return.
;; - Misleading truncation wording under a specific timing overlap
;;   (fix round R3, tail review -- REASONED, NOT REPRODUCED: this
;;   project's own "repro before you fix" rule means this is recorded
;;   as a gap, not acted on). `search-process-pending-all' checks
;;   `truncated' (from `search--drain-queue') BEFORE checking whether
;;   the job's DONE flag is set with an empty QUEUE. If a job's process
;;   has ALREADY exited cleanly (DONE, CODE=0 -- it found everything it
;;   was going to find and finished on its own) but QUEUE still has
;;   more buffered lines than `search-max-lines-per-tick' can drain in
;;   one tick, and draining the REST of that backlog happens to cross
;;   `search-max-results' on some later tick, the truncated branch fires
;;   and prints "Search results truncated (too many); process killed"
;;   -- wording that implies the job was cut off mid-search, when it had
;;   actually already finished normally; `shell-process-kill' on an
;;   already-exited process is a harmless no-op, but the MESSAGE is
;;   wrong about why the job stopped. Not fixed here because the
;;   reviewer who raised it could not construct a real, observable
;;   sequence exercising it (`search--drain-queue' checks the cap on
;;   EVERY line, not once per tick, so crossing it happens on the same
;;   line insertion regardless of which tick reaches it, and DONE plus
;;   QUEUE-still-large-enough-to-cross-the-cap-LATER is a narrow window
;;   that would need very specific relative timing between an engine's
;;   own exit and this pump's tick cadence to land on).
;;
;; --- M83 v1 does not include ---------------------------------------------
;; - Adding or deleting whole lines while in `search-edit-mode'. Only
;;   modifying an EXISTING result row's text is supported -- `search-
;;   edit-apply' walks the original and current line lists in lockstep
;;   by ROW INDEX (see D2), so a row count mismatch at apply time means
;;   every row from the first mismatch onward is either an extra row
;;   with nothing to compare against (ignored, reported) or an original
;;   row with no counterpart left to check (silently has nothing done
;;   for it -- there is no reasonable "which of these did the user mean
;;   to delete" to report).
;; - Cross-file atomicity: applying can leave some files written and
;;   others not (D7) -- local writes are truncate+overwrite, not
;;   write-to-temp-then-rename (a deliberate, already-documented M77
;;   tradeoff elsewhere in this editor), so there is no rollback for a
;;   file that fails partway through its own write either.
;; - Cross-buffer undo: each file that gets edited ends up with its OWN
;;   single undo group (see the header's D-list intro for why no extra
;;   code is needed to make that true) -- undoing a batch apply means
;;   undoing in each affected file's buffer separately, same as GNU
;;   Emacs's own `wgrep' offers no more than.
;; - A result line whose FILE contains a literal `:' was already outside
;;   `search--results' before this milestone (M82's own documented gap,
;;   `search--result-regexp''s FILE group stops at the first colon) --
;;   M83's row-indexed apply path parses headers with the exact same
;;   regexp, so such a line still can't be identified as a result at
;;   apply time either, and any edit to its text is silently invisible
;;   to `search-edit-apply' (not reported: its ORIGINAL row never
;;   parsed as a header to begin with, so it was never a candidate for
;;   write-back -- see the header's D2 discussion of "a row whose
;;   original header never parsed at all").
;; - Buffers opened by `search-edit-apply' (via `find-file', for a file
;;   that wasn't already open) are left open afterward, same as `M-.'
;;   (`lsp-definition-at-point') and every other cross-file jump in this
;;   editor already leaves its target buffer open.
;;
;; --- M83 known gaps added by fix round R2 (tail review) -----------------
;; - `save-buffer''s own `save_conflict_ack' mechanism (editing.rs) is
;;   inherited as-is by this batch apply path, with a consequence that
;;   mechanism was never designed for: `save_conflict_ack' exists so a
;;   HUMAN who was just refused a save and consciously decided to retry
;;   doesn't get refused AGAIN for the identical already-seen disk
;;   state (see that field's own doc comment) -- it is a one-shot,
;;   deliberate "I saw the warning, let me through" latch. If a target
;;   buffer already carries a matching `save_conflict_ack' from an
;;   EARLIER, unrelated `save-buffer' attempt earlier in the same
;;   session (the user hit the conflict, decided not to force it, and
;;   moved on without the disk state changing again since), THIS
;;   feature's own automatic `save-buffer' call
;;   (`search--edit-apply-to-file') silently sails through that latch
;;   with no human in the loop the second time -- turning a safety
;;   valve that assumes a person is reading the warning into a fully
;;   automated bypass of itself. Not fixed here: `search--edit-apply-
;;   to-file' has no way to distinguish "conflict guard passed because
;;   disk genuinely still matches" from "conflict guard passed because
;;   a stale ack from an unrelated earlier session happened to still
;;   match" without either clearing the ack unconditionally (which
;;   changes `save-buffer''s own behavior for every OTHER caller too,
;;   out of scope for this file) or duplicating `save-buffer''s
;;   internal disk-state comparison here (out of scope for the same
;;   reason D1 exists at all -- one code path that knows how to
;;   conflict-check a save, not two).
;; - D4's skip-reason wording does not distinguish "this line's TEXT
;;   changed" from "this file got shorter, or was deleted, since the
;;   search ran" -- both surface as the identical "changed since
;;   search, skipped" message (`search--edit-apply-to-file'), because
;;   both are detected the exact same way: `forward-line'/`buffer-
;;   substring' either lands on different text or, past-end-of-buffer,
;;   an empty string, either of which simply fails the `string=' check
;;   against the snapshot's ORIGINAL text. Behaviorally safe either way
;;   (nothing is force-written, nothing is fabricated), just imprecise
;;   about WHICH of the two actually happened.
;; - The `(unless (string= orig-line cur-line) ...)' skip in `search-
;;   edit-apply--do' (D3) is a pure optimization with no test covering
;;   the specific case it exists for -- every existing test's fixture
;;   either edits every result line present, or edits none at all; none
;;   constructs MULTIPLE result rows where only SOME are touched. Its
;;   correctness is a direct, simple consequence of the `string='
;;   semantics involved (an unchanged row trivially cannot produce a
;;   header mismatch OR a genuine edit), not something a removal of
;;   this line alone would even change the OUTCOME of -- removing it
;;   would just make every untouched row redundantly re-parse and
;;   re-compare its own header to itself, which `search--edit-apply-to-
;;   file''s own D4 check would then also find textually identical and
;;   apply as a no-op write. Kept for the obvious performance reason
;;   (skip untouched rows outright), not correctness.
;;
;; --- M83 known gaps added by fix round R3 (tail review) -----------------
;; - G6: the `(when (> applied 0) (with-current-buffer buf (save-
;;   buffer)))' guard in `search--edit-apply-to-file' has no test
;;   covering it either -- every existing test that reaches this line
;;   at all has at least one successfully-applied edit by the time it
;;   does, so replacing this with an unconditional `save-buffer' call
;;   would not turn any test red. The guard exists to avoid calling
;;   `save-buffer' (and touching `disk_state'/`save_conflict_ack',
;;   running before/after-save-hook, printing "Wrote ...") on a file
;;   for which NOTHING actually changed in this batch -- every one of
;;   its own lines got skipped by D4 (G1/D4) or the file itself doesn't
;;   exist (G4) -- which would be a pointless (though not incorrect)
;;   save of unchanged content. Constructing a test would need a file
;;   whose every edit is skipped while `search--edit-apply-to-file'
;;   itself is still reached (i.e. `find-file' succeeds), which none of
;;   the existing per-file skip scenarios (G1's save failure, G4's
;;   missing file) happen to isolate on their own.

;; --- M85 v1 does not include / known gaps (independent cold-read -----
;; --- review, fix round) --------------------------------------------------
;; - F6: the truncation banner ("*** Search results truncated (too
;;   many) ***", `search-process-pending-all') is silently DROPPED
;;   under an active filter (`search--filter' non-nil) instead of
;;   appearing in `*search*'. Cause: it goes through `search--insert-
;;   line', which (once filtering is engaged) delegates to `search--
;;   insert-filtered-line' -- that function parses the line via
;;   `search--parse-result-line' FIRST and does nothing at all if it
;;   doesn't match `search--result-regexp' (the banner text obviously
;;   never does), so the line is neither recorded nor rendered, unlike
;;   the UNFILTERED path (plain `search--insert-line'), which always
;;   inserts every line verbatim regardless of whether it parses. The
;;   `message' echo-area notice ("Search results truncated (too many);
;;   process killed") still fires either way -- only the PERMANENT,
;;   re-readable banner inside `*search*' itself disappears once a
;;   filter session has been engaged. Pinned by `search_filter_
;;   swallows_the_truncation_banner_but_keeps_the_echo_message'
;;   (search_tests.rs) -- not fixed here: doing so would need `search--
;;   insert-filtered-line' (or `search--render') to track banner/
;;   non-result lines as a SEPARATE list from `search--results', which
;;   is more machinery than this fix round's own scope (three HIGH-
;;   severity bugs) budgets for.
;; - F7: cancelling a `search-filter-start' (`/') prompt mid-typing with
;;   `C-g' or `ESC' does NOT restore `search--filter' or `*search*''s
;;   content to whatever they were before `/' was pressed -- `search--
;;   filter' and `*search*' are left holding whatever partial filter
;;   text and rendered subset were current at the moment of
;;   cancellation, permanently (until the next full `search--reset').
;;   Pinned by `search_filter_c_g_leaves_the_partial_filter_in_place'
;;   (search_tests.rs).
;;
;;   The two cancel keys are NOT symmetric, and an earlier version of
;;   this note got that wrong (fix-round tail re-review correction,
;;   H2): `C-g' is handled in `handle_key' (commands.rs, the KEY_CTRL_G
;;   branch near the top of that function, NOT `minibuffer_key') and
;;   UNCONDITIONALLY calls `run_hook_by_name(interp,
;;   "keyboard-quit-hook")' on its way out -- a real, existing hook a
;;   FUTURE fix could attach a filter-revert function to. `ESC', by
;;   contrast, is handled entirely inside `minibuffer_key' (its own
;;   `Key::Char(27)' branch) and runs no hook of any kind by design --
;;   that branch's own comment notes ESC deliberately skips `keyboard-
;;   quit-hook', unlike C-g. So `C-g' alone is technically fixable
;;   today by hanging a revert off the hook that already exists; `ESC'
;;   is not, without this editor gaining a new minibuffer-cancel
;;   notification mechanism it doesn't have.
;;
;;   Deliberately fixing NEITHER half here, even though one of them is
;;   cheaper than the note above's earlier (wrong) framing suggested:
;;   (1) this is this fix round's OWN tail re-review -- anything typed
;;   past this point goes uncommitted-and-uncold-read one loop further
;;   than the round's own review budget allows, and hanging new
;;   BEHAVIOR (not a bug fix) off `keyboard-quit-hook' here would be
;;   exactly that; (2) fixing only `C-g' would make the two cancel keys
;;   behave DIFFERENTLY from each other (revert on one, not on the
;;   other) -- a user who reaches for whichever key is at hand would
;;   see inconsistent results depending on which one they pressed,
;;   which is harder to explain and defend than "neither reverts yet".
;;   Left as a candidate for a future milestone (recorded in PLAN.md,
;;   not decided here) rather than attempted piecemeal in this note's
;;   own margin.
;; - F8: every keystroke while filtering re-renders the ENTIRE `*search*'
;;   buffer from `search--results' (`search--render', O(result count)
;;   per keystroke) -- `minibuffer-input-changed-hook' is deliberately
;;   NOT included in `KEYSTROKE_HOOKS' (commands.rs), so none of that
;;   module's per-hook time-budget/eviction machinery applies here
;;   either. Near `search-max-results' (10000 by default), this means
;;   every single keystroke in the filter prompt redraws on the order
;;   of ten thousand lines. Measured, not just asserted:
;;   `search_filter_render_cost_at_max_results_scale'
;;   (completing_read_perf_tests.rs -- alongside this project's other
;;   `_perf_tests.rs' scale probes, `#[ignore]`d by default, run via
;;   `cargo test --release -p core --test completing_read_perf_tests --
;;   --ignored --nocapture') times one `search--render' call directly
;;   over 10000 synthetic `search--results' entries (2 of which match
;;   the probe filter). Measured on the machine this was written on
;;   (release build): ~6.7ms for that single call -- comfortably within
;;   one keystroke's worth of latency today, but a number future
;;   scaling work on this file should know is here, not automatically
;;   gated on a pass/fail threshold (the project's own perf-test
;;   precedent, `completing_read_perf_tests.rs`'s OTHER probes, does
;;   the same) since the acceptable threshold depends on the
;;   interactive budget a later milestone would set, not a number this
;;   fix round can pick in isolation.
;; - F9: after `search--render' redraws `*search*', point in that
;;   buffer ends at BUFFER's END (the last inserted line's own trailing
;;   newline), not preserved on whatever result line it was resting on
;;   before the redraw -- `erase-buffer' resets point to `point-min',
;;   and each subsequent `insert' during the redraw moves point forward
;;   with it (elisp `insert' always advances point past the inserted
;;   text), so by the time the `dolist' loop finishes, point is
;;   wherever the LAST `insert' left it: the very end of the buffer.
;;   Checked directly (`search_filter_render_leaves_point_at_buffer_
;;   end', search_tests.rs), not assumed. Accepted as v1 behavior: a
;;   live filter's whole point is narrowing down to a small, useful
;;   result set, and point landing at the end (right after the last,
;;   most memorable match) is a reasonable place to land even though
;;   it isn't the position that was under point before the keystroke.
;; - H5 (tail re-review): `search--filter-input-changed's `(minibuffer-
;;   prompt)' string comparison (F3) has no protection against a
;;   DIFFERENT, future feature that happens to open a minibuffer prompt
;;   with the EXACT same text as `search--filter-prompt' ("Search
;;   filter: ") for some entirely unrelated purpose while `*search*' is
;;   the current buffer -- that coincidence alone would be enough to
;;   make this function treat it as ITS OWN filter session and start
;;   redrawing `*search*' underneath it. F3's own docstring currently
;;   only names the narrower mid-edit sub-case it guards against
;;   (guard 3), not this broader "two features share one identifier
;;   string" collision class -- worth naming explicitly here so a
;;   later feature author picking a prompt string knows there is a
;;   real (if currently only theoretical) reason not to reuse this
;;   one. Not fixed here: there is no unique-session-token / prompt-
;;   identity primitive in this editor to reach for instead, and
;;   inventing one is out of scope for a search.el-only fix round.
;; - H6 (tail re-review): `search--nav-results' re-runs `orderless-
;;   rank' over EVERY entry in `search--results' on EVERY `n'/`p'
;;   press while a filter is engaged, even though the filter text
;;   itself hasn't changed since the last press -- unlike `search--
;;   render' (F8's own measured cost, ~6.7ms at `search-max-results''s
;;   10000-entry default for ONE call), this path re-does that same
;;   O(result count) scan on EVERY navigation keystroke too, not just
;;   every FILTER keystroke. Ten consecutive `n' presses at that scale
;;   costs roughly ten times F8's own measured number, not measured or
;;   asserted here. Not fixed here (e.g. by caching the last computed
;;   nav list keyed on the current `search--filter' value) -- `n'/`p'
;;   are typically pressed far less often than filter text is typed at
;;   this scale, and this fix round's own budget went to the three
;;   HIGH-severity bugs (F1-F3) plus H1-H4's corrections instead.

(defvar search-program "rg"
  "External search engine `search-project'/`search-project-regexp'/
`search-again' invoke. Must print one match per line, shaped
`FILE:LINE:COL:TEXT' (`search-arguments-literal'/`search-arguments-
regexp''s shared flags make real `rg' do exactly that) -- see this
file's header for why this and the two arguments variables are all
swappable rather than hardcoded.")

(defvar search-arguments-literal
  "--line-number --column --no-heading --color never --smart-case --fixed-strings --"
  "Flags `search--start' uses for `search-project' (the DEFAULT
command) -- PATTERN is matched as a literal fixed string
(`--fixed-strings'/`-F'), not a regular expression. See this file's
header (\"Why literal search is the DEFAULT\") for why this is the
default and why `-F' lives HERE, in the swappable-arguments variable,
rather than being hardcoded into `search--start''s own string-building.
Deliberately ends in `--' -- everything after it is `rg''s own
convention for \"no more flags, what follows is the pattern (and then
paths)\", which also means a PATTERN that itself starts with `-' is
searched for literally instead of being misparsed as another flag.")

(defvar search-arguments-regexp
  "--line-number --column --no-heading --color never --smart-case --"
  "Flags `search--start' uses for `search-project-regexp' (the opt-in
regex command) -- identical to `search-arguments-literal' minus
`--fixed-strings', so PATTERN is matched as an `rg'-flavored regular
expression. See this file's header for why this is a separate command
rather than a `C-u' variant of `search-project'.")

(defvar search-output-buffer-name "*search*"
  "Output buffer for `search-project'/`search-again'. Like `compile-
output-buffer-name' (compile.el), this editor tracks at most ONE
running search at a time -- see `search--procs''s doc comment.")

(defvar search-max-lines-per-tick 200
  "Max number of complete result lines a single idle tick's
`search-process-pending-all' will insert into `*search*' for one job.
Exists because a search that hits three orders of magnitude more lines
than a typical build's error output (a common grep across a large
repository routinely does) would otherwise, inserted all in one idle
tick, stall the whole editor for that tick -- `shell-command--insert-
output' (shell-command.el), which this file does NOT reuse for its
per-line insertion, has no such cap because neither of ITS two callers
(a one-shot command's output, a build log) can plausibly produce a
result set this large this fast. Any lines beyond this budget in a
single tick stay queued (`search--procs' entry's QUEUE slot) for the
NEXT tick, not dropped.")

(defvar search-max-results 10000
  "Exact cap on how many result lines (parseable or not -- see
`search--drain-queue') a single search job may insert into `*search*'
before it gets killed, its process torn down, and a truncation notice
appended. `search--drain-queue' stops the moment its running count
reaches this value -- i.e. AT MOST `search-max-results' lines are ever
inserted for one job, never `search-max-results' + 1 (fix round R1: an
earlier version of this cap compared with `>' instead of `>=' and let
exactly one line through past the limit on every truncated job, off by
one from what this docstring already promised). Same reasoning as
`compile-max-output-chars' (compile.el) and `shell-command-max-output-
chars' (shell-command.el): none of the underlying primitives
(`start-shell-process') enforce any cap of their own (see shell.rs's
own header), so an overly broad pattern against a huge tree would
otherwise grow `*search*' and `search--results' without bound.")

(defvar search--last-pattern nil
  "The PATTERN most recently passed to `search--start' -- what `search-
again' reruns unchanged, the same \"reuse the last invocation\"
relationship `compile--dir'/`compile-command' have to `recompile'
(compile.el).")

(defvar search--last-root nil
  "The ROOT directory most recently passed to `search--start' --
`search-again''s companion to `search--last-pattern'.")

(defvar search--last-regexp-p nil
  "Whether the most recent `search--start' call was a
`search-project-regexp' (t) or plain `search-project' (nil) invocation
-- `search-again''s third piece of remembered state, alongside
`search--last-pattern'/`search--last-root': rerunning the exact same
search must also rerun it in the exact same literal-vs-regexp mode, not
silently default back to literal.")

(defvar search--edit-original-text nil
  "M83: `*search*''s FULL text at the moment `search-edit-mode' was last
entered, or nil when not currently editing. Two uses: `search-edit-
discard' (`C-c C-k') restores this verbatim; `search-edit-apply'
(`C-c C-c') splits it into lines and walks them in lockstep against the
buffer's CURRENT lines (row-indexed, not content-keyed -- see this
file's header, D2, for why). Cleared by `search--reset' -- a fresh
`search-project'/`search-again' invocation while mid-edit must not
leave this pointing at text that `*search*' no longer contains.")

;; --- Entry vector layout ------------------------------------------------
;; `search--procs' entries: #[PROC DIR PENDING QUEUE COUNT DONE CODE]
;;   PROC    -- the `start-shell-process' handle.
;;   DIR     -- the job's own working directory (the search ROOT),
;;              carried through to line-parse time so a relative FILE in
;;              the engine's own output resolves the same way
;;              `compile--parse-error-line' (compile.el) resolves a
;;              relative FILE against the compiler's own cwd.
;;   PENDING -- a possibly-incomplete trailing line carried over from
;;              the last chunk this job produced (see `search--feed-
;;              chunk') -- the core state that makes incremental,
;;              line-safe parsing possible at all.
;;   QUEUE   -- complete lines already split out of PENDING/chunks, not
;;              yet inserted into `*search*' because a previous tick's
;;              `search-max-lines-per-tick' budget ran out first.
;;   COUNT   -- running total of lines actually inserted so far for this
;;              job, checked against `search-max-results'.
;;   DONE    -- non-nil once this job's `(exit . CODE)' event has been
;;              seen. Kept as its own persistent flag rather than a
;;              per-tick local, because the process can genuinely finish
;;              (and stop being pollable at all -- `shell-process-poll'
;;              delivers its exit event exactly once, see shell.rs) on
;;              some tick BEFORE `QUEUE' has been fully drained by
;;              `search-max-lines-per-tick' -- the entry has to stay in
;;              `search--procs' for however many further ticks it takes
;;              to finish draining, and a per-tick local `finished'
;;              variable would forget the job had ended the moment that
;;              tick's poll loop stopped seeing new exit events.
;;   CODE    -- the process's own exit code once DONE is set, nil until
;;              then. Fix round R1: an earlier version of this file
;;              read `(exit . CODE)' only far enough to set DONE, then
;;              discarded CODE outright -- `search--report-finish'
;;              needs it to tell a normal "ran fine, found nothing" (rg
;;              exit 1) or a genuinely abnormal exit (a bad regex, `rg'
;;              exit 2; `search-program' not found, shell exit 127;
;;              anything else) apart from an ordinary successful finish
;;              (exit 0) -- see that function's own doc comment.
;;

;; `search--results' entries: #[FILE LINE COL BUFFER-POS TEXT]
;;   FILE       -- absolute path (`expand-file-name'-resolved against
;;                 the job's DIR), confirmed to exist on disk at PARSE
;;                 time (same ambiguity guard `compile--parse-error-
;;                 line' uses, and the same TOCTOU caveat that entry's
;;                 doc comment already accepts).
;;   LINE, COL  -- 1-based, exactly as the engine reported them.
;;   BUFFER-POS -- this line's starting character position inside
;;                 `*search*' at insert time, used only by
;;                 `search-goto-result-at-point' (RET) to map "which
;;                 line is point on" back to an entry -- identical
;;                 mechanism and identical hand-edit-drift caveat as
;;                 `compile--errors''s own BUFFER-POS slot
;;                 (compile.el). M85 fix round F1: nil whenever this
;;                 entry is currently HIDDEN by an active `search--
;;                 filter' -- see `search--render's own doc comment for
;;                 why a stale, no-longer-true position here is a real
;;                 bug (a hidden entry's old position can collide
;;                 EXACTLY with a currently-visible entry's new one)
;;                 rather than the harmless leftover an earlier version
;;                 of this comment claimed.
;;   TEXT       -- M85: the result line's own full raw text (the
;;                 exact string that was/would be inserted into
;;                 `*search*', i.e. `FILE:LINE:COL:' plus the engine's
;;                 own matched text), used by `search--matching-p' as
;;                 the orderless-match candidate (D3: a filter can key
;;                 off the file name, the line number, or the matched
;;                 text indifferently) and by `search--render' to
;;                 redraw a previously-streamed entry without re-
;;                 reading anything from disk (D2).
;;
;; Stored NEWEST-FIRST (`cons'-prepended by `search--insert-line' as
;; results stream in) rather than in on-screen order -- prepending is
;; O(1) per result, which matters because a single job can legitimately
;; produce `search-max-results' (10000 by default) of these; appending
;; to the end of a growing list on every single result would be
;; quadratic in the worst case. `search--ordered-results' reverses this
;; back into on-screen (first-found-first) order on demand, for
;; `n'/`p'/RET' -- an O(n) reversal per keypress, cheap even at the
;; 10000-entry cap, and it is FAR rarer to press `n' than it is for a
;; result to stream in.
(defvar search--procs nil
  "Active search job (at most one; see `search-output-buffer-name''s
doc comment); see the header comment above for the per-entry vector
layout.")

(defvar search--results nil
  "Parsed result entries from the current/most recent search job, newest
first -- see the header comment for the per-entry vector layout and why
this order, and `search--ordered-results' for the on-screen order
`n'/`p'/RET' actually walk.")

(defvar search--current-index nil
  "0-based index into `search--nav-results''s return value (M85 fix
round F2 -- corrected here; this used to say `search--ordered-
results') of the entry `search-next-result'/`search-previous-result'/
`search-goto-result-at-point'/`search-goto-module-declaration' last
jumped to, or nil before any of them has been called since the last
`search--reset'. `search--nav-results' is `search--ordered-results'
UNFILTERED (`search--filter' nil), but only the currently-VISIBLE
subset once a filter session is engaged -- so this index's own range
shrinks and its MEANING shifts the moment filtering starts: it always
indexes whatever `n'/`p' actually cycle through right now, never a
position into the full, possibly-larger, possibly-partly-hidden
result list. Mirrors `compile--current-index''s own doc comment
(compile.el) -- `search--reset' always clears this alongside
`search--results', so it can never point past the end of a since-
shrunk list.")

(defvar search--filter nil
  "M85: the live filter text currently narrowing `*search*' (D2/D3), or
nil when no filter session has been engaged this search -- the
DEFAULT state, in which every streamed result is inserted directly,
unfiltered, exactly as M82/M83 already did (`search--insert-line's own
fast path). Becomes a string (possibly \"\", meaning \"filtering is on
but nothing typed yet -- show everything\") the moment `search-filter-
start' (`/') is invoked, and stays a string for the rest of this
search session even if the filter text is backspaced back to empty --
re-entering the unfiltered FAST PATH mid-session is not supported (v1
scope); the only visible difference at an empty filter is that every
result still routes through `search--render' (D2) instead of a direct
buffer append. Reset to nil by `search--reset', so a fresh `search-
project'/`search-again' always starts unfiltered again.")

(defvar search--filter-blocked-message
  "Search results are being edited -- C-c C-c to apply or C-c C-k to discard first"
  "Shared rejection message for `search-filter-start' (D5's second,
asymmetric half, fix-round-free from day one unlike `search--edit-
blocked-message' this text is copied from) when `*search*' is
currently in `search-edit-mode' -- filtering redraws the buffer and
recomputes every visible entry's BUFFER-POS (`search--render'), which
would invalidate M83's row-indexed edit-apply snapshot
(`search--edit-original-text') out from under an edit already in
progress.")

(defun search--ordered-results ()
  "`search--results' in on-screen (first-found-first) order -- see that
variable's own doc comment for why it is stored reversed internally."
  (reverse search--results))

;; --- Root detection: nearest ancestor `.git', or a sensible fallback ---

(defun search--find-root (start-dir)
  "Walk up from START-DIR (a directory name, trailing slash) looking for
an ancestor directory that directly contains a `.git' entry; return
that ancestor, or START-DIR itself if none is found. See this file's
header for why `.git' specifically, not `lsp--project-root''s (lsp.el)
wider marker list."
  (let ((dir start-dir) (found nil))
    (while (and dir (not found))
      (if (file-exists-p (concat dir ".git"))
          (setq found dir)
        (let ((parent (file-name-directory (directory-file-name dir))))
          (setq dir (if (and parent (not (string= parent dir))) parent nil)))))
    (or found start-dir)))

(defun search--default-root ()
  "The root `search-project' searches from: the nearest `.git'-rooted
ancestor of the current buffer's own file, that file's own directory if
no `.git' is found anywhere above it, or `shell-command--default-dir'
(shell-command.el) if the current buffer has no associated file at all
-- same three-tier fallback shape `compile''s own doc comment (compile.el)
uses for choosing a working directory."
  (let ((file (buffer-file-name)))
    (if file
        (search--find-root (file-name-directory (expand-file-name file)))
      (shell-command--default-dir))))

;; --- Shell-quoting the search pattern -----------------------------------

(defun search--shell-quote (s)
  "Wrap S in single quotes for the `sh -c' command line `start-shell-
process' always runs through (shell.rs's `spawn'), escaping any
embedded single quote with the standard POSIX idiom: close the quote,
emit a backslash-escaped literal quote, reopen. Written as a plain
character loop (`aref'/`concat') rather than `replace-regexp-in-string'
-- that builtin's REP argument treats a bare `\\'' specially only when
followed by `&'/a digit/another backslash (see `crate::regex::
replace_all'), so getting the `'\\'''-idiom's own literal backslash
through it correctly needs escaping tricks this loop avoids outright."
  (let ((out "'") (i 0) (n (length s)))
    (while (< i n)
      (let ((c (aref s i)))
        (if (= c (string-to-char "'"))
            (setq out (concat out "'\\''"))
          (setq out (concat out (char-to-string c)))))
      (setq i (1+ i)))
    (concat out "'")))

;; --- Incremental line-safe parsing: the core new mechanism here --------

(defvar search--line-prefix-limit 300
  "Same purpose and same value as `compile--line-prefix-limit'
(compile.el) -- bounds how many characters of a candidate result line
`search--parse-result-line' ever hands to `search--result-regexp', to
cap regexp backtracking cost. See that variable's own doc comment for
the exact stack-overflow risk this guards against
(`crates/elisp/src/regex.rs' backtracks via real Rust call recursion);
identical risk here since `search--result-regexp''s FILE group is the
same unbounded `[^:\\n]+' shape.")

(defvar search--result-regexp
  "^\\([^:\n]+\\):\\([0-9]+\\):\\([0-9]+\\):"
  "FILE:LINE:COL: header pattern for one `rg'-shaped result line (see
`search-arguments-literal'/`search-arguments-regexp''s doc comments for
the shared flags that produce exactly this shape). Unlike `compile--
error-prefix-regexp' (compile.el), COL is NOT optional here -- both
arguments variables' shared flags always request a column, and a line
missing one is exactly the kind of not-actually-a-result line (an
engine's own banner/progress output, if any leaks into the merged
stream) this file wants to leave unparsed rather than mis-taken for a
jumpable entry. Group 1 = FILE, group 2 = LINE, group 3 = COL. Must
always be matched against a `search--line-prefix-limit'-bounded PREFIX
of a line, never the whole line -- see that variable's own doc
comment.")

(defun search--parse-result-line (line dir)
  "Parse one complete LINE of search output, resolving a relative FILE
against DIR (the job's own working directory). Returns a #[FILE LINE
COL] vector, or nil if LINE doesn't match `search--result-regexp'
within the first `search--line-prefix-limit' characters, OR if the
match succeeds but FILE doesn't exist on disk (same ambiguity guard
`compile--parse-error-line', compile.el, uses, and the same ordinary-
output-line-that-happens-to-look-like-a-header case it exists to
reject). A non-matching line is NOT dropped by this function's caller
-- see `search--insert-line' -- it is only ever excluded from
`search--results', never from the buffer itself."
  (let* ((bound (min search--line-prefix-limit (length line)))
         (prefix (substring line 0 bound)))
    (when (string-match search--result-regexp prefix)
      (let* ((file-raw (match-string 1 prefix))
             (file (expand-file-name file-raw dir))
             (line-num (string-to-number (match-string 2 prefix)))
             (col (string-to-number (match-string 3 prefix))))
        (when (file-exists-p file)
          (vector file line-num col))))))

(defun search--feed-chunk (entry chunk)
  "Fold CHUNK (a raw, possibly mid-line `shell-process-poll' string)
into ENTRY's PENDING/QUEUE slots (see the header's entry-layout
comment): split on newlines, keep the trailing segment (complete or
not -- `split-string' on a string ending in a newline yields a trailing
empty string, which is exactly the \"no partial line pending\" case, and
folds into this uniformly) as the new PENDING, and append every line
BEFORE it onto QUEUE, in the order they were printed. This is the one
piece of state that makes chunk-boundary-safe incremental parsing
possible at all -- see this file's header for why `compile.el' gets to
skip it and this file cannot."
  (let* ((combined (concat (aref entry 2) chunk))
         (lines (split-string combined "\n" nil))
         (complete nil)
         (rest lines))
    (while (cdr rest)
      (setq complete (cons (car rest) complete))
      (setq rest (cdr rest)))
    (aset entry 3 (append (aref entry 3) (nreverse complete)))
    (aset entry 2 (car rest))))

(defun search--insert-line (buf line dir)
  "Append one complete LINE (plus its trailing newline) to BUF via
`shell-command--insert-output' (shell-command.el, also reused as-is by
`compile.el' -- see that file's header) and, if LINE parses as a result
(`search--parse-result-line', DIR resolves a relative FILE), record it
in `search--results' with this line's position BEFORE insertion as its
BUFFER-POS. An unparseable LINE is still inserted -- never silently
dropped, unlike the already-documented gap in `compile.el' (see that
file's header, first known gap) -- it just never becomes a `search--
results' entry, so `n'/`p'/RET' skip over it.

M85 D6: this is the UNFILTERED fast path -- taken only while
`search--filter' is nil, i.e. `search-filter-start' (`/') has never
been invoked this search session. Every `search--results' entry gets a
5th slot (LINE itself, verbatim) regardless of filter state -- cheap,
and it is what lets a LATER `/' invocation redraw entries that streamed
in before filtering was ever engaged. Once `search--filter' is non-nil
(a filter session IS in progress, D2/D3), this function hands off to
`search--insert-filtered-line' instead: a blind, unconditional append
here would show a result that no longer matches the active filter (D6
requires the opposite -- a new result's own visibility is decided by
the CURRENT filter, exactly like every result already on screen)."
  (if search--filter
      (search--insert-filtered-line line dir)
    (let ((pos (with-current-buffer buf (point-max))))
      (shell-command--insert-output buf (concat line "\n"))
      (let ((parsed (search--parse-result-line line dir)))
        (when parsed
          (setq search--results
                (cons (vector (aref parsed 0) (aref parsed 1) (aref parsed 2) pos line)
                      search--results)))))))

(defun search--insert-filtered-line (line dir)
  "M85 D6: the filtered-mode counterpart of `search--insert-line''s own
unfiltered fast path -- taken for every streamed LINE once a
`search-filter-start' (`/') session has been engaged for this search
(`search--filter' non-nil). An unparseable LINE (one `search--parse-
result-line' can't turn into a #[FILE LINE COL] triple) is dropped
outright here, unlike the unfiltered path -- there is no `search--
results' entry to redraw it from on a later filter change, and (v1
scope, documented rather than silently accepted) a raw non-result line
streamed in WHILE filtering is active simply never becomes visible. A
parseable LINE is recorded (BUFFER-POS 0, a placeholder --
`search--render', called unconditionally right below, immediately
overwrites it with a real position for every entry that ends up
SHOWN) and then the WHOLE buffer is redrawn from `search--results' via
`search--render', which alone decides -- against the CURRENT
`search--filter' -- whether this new entry (or any other) actually
lands on screen. Known cost, accepted for v1: this makes each
streamed line while filtering is active O(current result count) rather
than O(1), unlike the unfiltered path -- a full re-render per line
during a large, actively-filtered, actively-streaming search is
quadratic in the worst case. Not a concern this milestone's own test
suite exercises (filtering a search already in flight is a narrow,
interactive-only combination), but a real cost a future milestone
revisiting search performance should know is here."
  (let ((parsed (search--parse-result-line line dir)))
    (when parsed
      (setq search--results
            (cons (vector (aref parsed 0) (aref parsed 1) (aref parsed 2) 0 line)
                  search--results))
      (search--render))))

(defun search--matching-p (entry)
  "Non-nil if ENTRY (a `search--results' vector) should currently be
visible in `*search*' (M85 D2/D3): true when no filter session is in
progress, or `search--filter' is empty/whitespace-only (`orderless-
rank''s own zero-token convention, reused unchanged), or ENTRY's own
raw TEXT (slot 4) `orderless-rank's non-nil against `search--filter' --
i.e. the SAME multi-token, smart-case, any-order matcher `completing-
read'/`M-x' already use (M84), applied here to the WHOLE result line
(`FILE:LINE:COL:TEXT', D3) rather than a bare candidate name, so a
filter can key off the file name, the line number, or the matched text
indifferently, and `alu clk' matches regardless of which of the two
words appears first in the line."
  ;; M85 fix round F5 (cold-read finding, honesty note -- keep, do not
  ;; remove without re-checking every call site): both of the first two
  ;; disjuncts below are either unreachable or redundant TODAY, and
  ;; this comment says so rather than silently claiming otherwise.
  ;; `(not search--filter)' is UNREACHABLE at every existing call site
  ;; -- `search--render'/`search--insert-filtered-line'/`search--nav-
  ;; results' are only ever invoked once `search--filter' is already
  ;; non-nil (`search-filter-start' sets it before calling any of
  ;; them). Kept anyway as cheap defensive insurance for a
  ;; hypothetical future caller that asks `search--matching-p' about
  ;; an entry with NO filter session active at all -- removing it
  ;; would make this function silently wrong the day such a caller
  ;; appears, rather than trivially correct. `(string-empty-p
  ;; search--filter)' is REDUNDANT with `orderless-rank''s own empty-
  ;; input convention: `(orderless-rank anything "")' already returns
  ;; 0 (truthy), so this disjunct's own return value is never actually
  ;; needed for correctness -- kept only for a cheap short-circuit
  ;; (skip a token split for the single most common filter state: just
  ;; opened, nothing typed yet) and readability, not because dropping
  ;; it would change behavior.
  (or (not search--filter)
      (string-empty-p search--filter)
      (orderless-rank (aref entry 4) search--filter)))

(defun search--nav-results ()
  "The candidate list `search-next-result' (`n')/`search-previous-
result' (`p')/`search-goto-result-at-point' (RET)/`search-goto-
module-declaration' (`M-.') all navigate, and what `search--current-
index' indexes into (M85 fix round F2): `search--ordered-results'
unchanged when no filter session is active, but narrowed to only the
entries `search--matching-p' currently accepts once one is -- so
`n'/`p' (and a fresh RET/M-. jump) can never land on a file that isn't
even visible in `*search*' right now. Before this existed, `n'/`p'
walked `search--ordered-results' directly, completely ignoring
`search--filter': a `*search*' buffer showing \"1/50\" would still
cycle through all 50 entries on `n', silently jumping to files with no
line on screen at all.

Order within each case is unchanged from `search--ordered-results'
(on-screen, first-found-first) -- filtering only removes entries, it
never reorders the ones that remain."
  (let ((ordered (search--ordered-results)))
    (if search--filter
        (let ((kept nil))
          (dolist (e ordered)
            (when (search--matching-p e)
              (setq kept (cons e kept))))
          (nreverse kept))
      ordered)))

(defun search--render ()
  "Redraw `*search*''s entire buffer content from `search--results'
(M85 D2: a REDRAW from already-in-memory data, never a re-run of the
search engine or a disk read), keeping only entries `search--matching-
p' currently accepts. Called every time `search--filter' changes
(`search--filter-input-changed', live as-you-type) and every time a
NEW result streams in while a filter session is already active
(`search--insert-filtered-line', D6).

Each SHOWN entry's own BUFFER-POS (slot 3) is overwritten with its
position in the FRESHLY rebuilt buffer -- `search-goto-result-at-
point' (RET) and `search-goto-module-declaration' (M-.) key off this
slot via `search--result-at-buffer-pos' (`n'/`p' do NOT: they walk
`search--nav-results' by `search--current-index', never by BUFFER-POS
at all -- see that variable's own doc comment). A stale position left
over from before a re-render would silently point `RET'/`M-.' at the
wrong line.

M85 fix round F1 (independent cold-read review, HIGH severity, real
bug -- corrected here, this comment used to claim the opposite): a
HIDDEN entry's BUFFER-POS is set to NIL, not left at whatever it was
before -- `search--result-at-buffer-pos' skips nil positions on sight.
An earlier version of this function left a hidden entry's OLD position
untouched, reasoning (WRONGLY) that nothing on screen could ever ask
about a position that isn't the start of any CURRENTLY displayed line.
That reasoning breaks under a NON-monotonic filter change (narrow to
subset A, then to a DIFFERENT subset B, not a further narrowing of A):
a newer entry's stale, now-hidden position can land EXACTLY on the
same character offset a different, currently-VISIBLE, OLDER entry now
occupies after ITS OWN position was just recomputed -- `search--
result-at-buffer-pos' scans `search--results' newest-first and returns
the FIRST exact match, so the stale (hidden, wrong) entry wins over
the real (visible, right) one every time this collision occurs.
Concretely (search_tests.rs's own repro, pinned by
`search_filter_ret_does_not_jump_to_a_stale_hidden_entrys_position'):
two results, B newer than A; filter to only B (B's own position
becomes line 1); then filter to only A instead (A's position becomes
line 1 too, and B -- now hidden -- keeps its STALE \"line 1\" from the
moment it was last shown); `RET' on the only visible line scans
`[B, A]' newest-first, finds B's stale \"line 1\" BEFORE ever reaching
A's real one, and jumps to B's file instead of A's -- silently, with
no error, while A's line is the only thing actually on screen. Setting
a hidden entry's own position to nil instead of leaving it stale makes
this collision structurally impossible: a hidden entry can no longer
ever equal a real on-screen position, because it no longer holds a
number at all.

Always operates on the READ-ONLY view state (`inhibit-read-only'
bound around the rewrite) -- `search-filter-start' itself refuses to
run at all while `*search*' is in `search-edit-mode' (D5's own
asymmetric second half), so this function is never expected to run
against the WRITABLE edit-mode buffer; see `search--filter-input-
changed's own doc comment for the defense-in-depth guard against a
stray hook firing there anyway.

Messages the visible/total count (D4) every time it runs, so the user
is never left wondering whether a short result list means \"the search
found little\" or \"the filter is hiding the rest\"."
  (let* ((buf (search--ensure-output-buffer))
         (ordered (search--ordered-results))
         (total (length ordered))
         (shown 0))
    (with-current-buffer buf
      (let ((inhibit-read-only t))
        (erase-buffer)
        (dolist (e ordered)
          (if (search--matching-p e)
              (let ((pos (point-max)))
                (insert (aref e 4))
                (insert "\n")
                (aset e 3 pos)
                (setq shown (1+ shown)))
            ;; F1: nil, not a stale leftover position -- see this
            ;; function's own doc comment.
            (aset e 3 nil))))
      (set-buffer-modified-p nil))
    (message "Filter: showing %d/%d result%s" shown total (if (= total 1) "" "s"))))

(defun search--drain-queue (entry buf)
  "Insert up to `search-max-lines-per-tick' lines from ENTRY's QUEUE
into BUF, incrementing ENTRY's COUNT (results-so-far, slot 4) as it
goes. Returns non-nil the moment COUNT reaches `search-max-results' --
the caller (`search-process-pending-all') is responsible for killing
the process and leaving a truncation notice, same two-cap shape
`compile-process-pending-all' (compile.el) uses for its own (character,
not line) cap: the check runs INSIDE this loop, once per line, so
insertion stops on the SAME line that crosses the cap rather than
overshooting an unbounded amount further within one tick.

Compares with `>=', not `>' (fix round R1 -- an earlier version used
`>', which let exactly ONE line past `search-max-results' through on
every truncated job: with the cap at N, COUNT reached N+1 before the
old `(> count N)' check ever fired true, so N+1 lines were inserted,
off by one from what `search-max-results''s own docstring already
promised (\"at most N\")).

`search-max-lines-per-tick' is clamped to a minimum of 1 here, at read
time (fix round R1) -- a user-set 0 or a negative value would otherwise
make this function's own `while' condition false unconditionally,
`QUEUE' would never drain even one line no matter how many idle ticks
run, ENTRY would never leave `search--procs', and `has_async_work'
(lib.rs) would report async work forever: the editor would be visibly
stuck on the fast poll cadence for a search that looks started but can
structurally never finish, with nothing anywhere explaining why."
  (let ((n 0) (truncated nil) (budget (max 1 search-max-lines-per-tick)))
    (while (and (aref entry 3)
                (< n budget)
                (not truncated))
      (let ((line (car (aref entry 3))))
        (aset entry 3 (cdr (aref entry 3)))
        (search--insert-line buf line (aref entry 1))
        (aset entry 4 (1+ (aref entry 4)))
        (setq n (1+ n))
        (when (>= (aref entry 4) search-max-results)
          (setq truncated t))))
    truncated))

(defun search-process-pending-all ()
  "Idle-tick pump for `search-project'/`search-again' -- own list
(`search--procs'), own pump function, wired into `idle-tick' (lib.rs)
alongside `compile-process-pending-all'/`shell-command-process-pending-
all', none of which share state with this one (see this file's header).
Unlike `compile-process-pending-all', parsing here is fully
incremental -- see this file's header for why a search job cannot defer
parsing to process-exit the way a compile job can.

Per idle tick, for each entry: drain every currently-available
`shell-process-poll' event into PENDING/QUEUE (`search--feed-chunk'),
flush a still-pending trailing partial line into QUEUE once the job's
DONE flag is set (a line with no trailing newline at all -- the
engine's own last line, or an early-exiting fake test generator that
never wrote one -- would otherwise sit in PENDING forever, visible
nowhere), then insert up to `search-max-lines-per-tick' lines from
QUEUE (`search--drain-queue'). A job whose process has exited AND whose
QUEUE has been fully drained is dropped from `search--procs' this tick;
one that produced more than `search-max-lines-per-tick' this tick or
is still running keeps its entry for the next tick.

Same known gap as `compile-process-pending-all' (compile.el, see that
function's own header): `*search*''s existence is checked ONCE per
entry per tick, at the top, not on every streamed chunk the way
`shell-command-process-pending-all' (shell-command.el) does -- killing
`*search*' mid-job delays this pump noticing by up to one tick."
  (let ((procs search--procs) (remaining nil))
    (while procs
      (let* ((entry (car procs))
             (proc (aref entry 0))
             (keep t))
        (if (not (get-buffer search-output-buffer-name))
            (progn (shell-process-kill proc) (setq keep nil))
          (progn
            (let ((looping t))
              (while looping
                (let ((ev (shell-process-poll proc)))
                  (cond
                   ((null ev) (setq looping nil))
                   ((stringp ev) (search--feed-chunk entry ev))
                   (t ; (exit . CODE) -- fix round R1: CODE is now kept
                    ; (slot 6), not discarded -- see `search--report-
                    ; finish' for why.
                    (aset entry 5 t)
                    (aset entry 6 (cdr ev))
                    (setq looping nil))))))
            (when (and (aref entry 5) (> (length (aref entry 2)) 0))
              (aset entry 3 (append (aref entry 3) (list (aref entry 2))))
              (aset entry 2 ""))
            (let* ((buf (get-buffer search-output-buffer-name))
                   (truncated (and buf (search--drain-queue entry buf))))
              (cond
               (truncated
                (setq keep nil)
                (shell-process-kill proc)
                (search--insert-line
                 buf "*** Search results truncated (too many) ***" (aref entry 1))
                (message "Search results truncated (too many); process killed"))
               ((and (aref entry 5) (null (aref entry 3)))
                (setq keep nil)
                (search--report-finish (aref entry 6) (length search--results)))))))
        (when keep (setq remaining (cons entry remaining))))
      (setq procs (cdr procs)))
    (setq search--procs remaining))
  nil)

(defun search--report-finish (code n)
  "Echo how a just-finished search job's process actually exited (fix
round R1 -- before this function existed, EVERY exit code produced the
IDENTICAL \"Search finished (N results)\" message, so a typo'd
`search-program' or a malformed regex looked indistinguishable from a
normal zero-result search; repro'd directly by pointing `search-
program' at a nonexistent command, and separately by handing real `rg'
a syntactically invalid regex (`foo(bar', unbalanced paren) via
`search-project-regexp' -- both printed a plain \"finished\" message
with zero results, identical to a real, successful, merely-empty
search). Mirrors `compile--finish''s (compile.el) 0-vs-other exit-code
split, but ALSO gives exit code 1 its own message: `rg' (and most
grep-family tools) use exit 1 to mean \"ran fine, matched nothing\" --
an entirely ordinary outcome for a real search, not a failure, and
worth a message that says so rather than folding it into the generic
\"exited abnormally\" bucket `compile--finish' would use for any
nonzero code from a compiler (where a nonzero exit almost always DOES
mean something went wrong)."
  (cond
   ((and (integerp code) (= code 0))
    (message "Search finished (%d result%s)" n (if (= n 1) "" "s")))
   ((and (integerp code) (= code 1))
    (message "Search finished: no matches"))
   (t
    (message "Search exited abnormally with code %s" code))))

;; --- `*search*' mode: n/p/RET/q ------------------------------------------

(defun search--goto-column (col)
  "Duplicate of `compile--goto-column' (compile.el) -- same COL
convention (1-based character offset, clamped to both ends of the
current line) and same reasoning for the low-end clamp. Copied rather
than shared per this milestone's own design decision: `*search*' and
`*compilation*' keep entirely independent state, and a shared helper
would be the one place that quietly coupled the two."
  (when col
    (let ((bol (point)))
      (goto-char (max bol (min (+ bol (1- col)) (line-end-position)))))))

(defun search--goto-result (entry)
  "Jump to ENTRY (a `search--results' vector), switching buffers via
`find-file' if needed -- duplicate of `compile--goto-error''s shape
(compile.el), independent state. The echo message's own count (M85
fix round F2 -- corrected here; this used to be `(length (search--
ordered-results))', the FULL unfiltered count) is `search--nav-
results''s length -- the same navigable set `search--current-index'
indexes into, so the printed \"(k/N)\" always agrees with what `n'/`p'
actually cycle through right now, filtered or not."
  (find-file (aref entry 0))
  (goto-char (point-min))
  (forward-line (1- (aref entry 1)))
  (search--goto-column (aref entry 2))
  (message "[search] (%d/%d) %s:%d"
           (1+ search--current-index)
           (length (search--nav-results))
           (aref entry 0)
           (aref entry 1)))

(defun search-next-result ()
  "`n' in `*search*': jump to the next entry in on-screen order, wrapping
to the first after the last. M85 fix round F2: walks `search--nav-
results' (the CURRENTLY VISIBLE entries, once a filter is engaged),
not `search--ordered-results' (every entry, filtered or not) -- an
earlier version of this function ignored `search--filter' entirely,
so `n' could cycle onto a file with no line on screen at all while
`*search*' visibly showed only a handful of results."
  (interactive)
  (let ((nav (search--nav-results)))
    (if (not nav)
        (message "No search results")
      (setq search--current-index
            (if search--current-index
                (mod (1+ search--current-index) (length nav))
              0))
      (search--goto-result (nth search--current-index nav)))))

(defun search-previous-result ()
  "`p' in `*search*': jump to the previous entry in on-screen order,
wrapping to the last before the first. See `search-next-result''s own
doc comment (M85 fix round F2) for why this walks `search--nav-
results', not `search--ordered-results'."
  (interactive)
  (let ((nav (search--nav-results)))
    (if (not nav)
        (message "No search results")
      (setq search--current-index
            (if search--current-index
                (mod (1- search--current-index) (length nav))
              (1- (length nav))))
      (search--goto-result (nth search--current-index nav)))))

(defun search--index-of (entry ordered)
  "0-based position of ENTRY (compared by `eq') within ORDERED, or nil."
  (let ((entries ordered) (idx 0) (found nil))
    (dolist (e entries)
      (when (and (not found) (eq e entry))
        (setq found idx))
      (setq idx (1+ idx)))
    found))

(defun search--recalibrate-current-index (old-nav)
  "M85 fix round H1: keep `search--current-index' pointing at the SAME
entry it pointed to before a filter change, or clear it to nil if that
entry is no longer visible -- called after `search--filter'/`search--
render' have ALREADY been updated to the NEW filter, with OLD-NAV
being whatever `search--nav-results' returned just BEFORE that change
(the caller's own responsibility to capture at the right moment; this
function only ever reads `search--current-index'/`search--nav-
results' as they stand NOW).

Why this exists at all: before M85, `n'/`p' always walked `search--
ordered-results', a list whose EXISTING entries never change position
-- new results only ever get consed onto `search--results' (newest-
first) and appended at the END of the on-screen order, so an index
into it was permanently stable once set. F2 (M85's own fix round)
changed `n'/`p' to walk `search--nav-results' instead, which -- once a
filter is engaged -- is a SUBSET whose own COMPOSITION changes on
every keystroke; an index that stays numerically fixed across such a
change silently starts pointing at a different (usually unrelated)
entry. Concretely (this file's own test, `search_filter_changes_
composition_recalibrates_current_index'): five results A-B-C-D-E,
`n' pressed three times lands on C (index 2); filtering down to
{A, C, E} shrinks `search--nav-results' to length 3 -- the STALE index
2 now names E instead of C purely by coincidence of list length, and
the NEXT `n' (`(mod (1+ 2) 3)' = 0) would land on A, not (as a user
who was standing on C would expect) the next visible entry after C,
which is E.

Recalibration strategy (a deliberate choice among two obvious ones,
see this fix round's own record for the alternative considered and
rejected): re-locate the SAME entry object (`eq', via `search--index-
of') in the NEW `search--nav-results' and point the index at wherever
it landed -- OLD-NAV's own order doesn't matter, only which ENTRY
`search--current-index' named in it. If that entry is no longer
visible in the new list at all, the index is cleared to nil (matching
`search--reset''s own \"nil = no current entry\" convention) rather
than left dangling at some other, unrelated entry's position -- an `n'
from nil starts fresh at index 0, exactly the same experience as
never having navigated at all this session, which is a reasonable
fallback when the entry the user was actually standing on has been
filtered away entirely.

A nil `search--current-index' (never navigated at all this session)
is left alone -- `(when search--current-index ...)' below is not just
a null guard, it is also correct behavior: there is no \"same entry\"
to preserve when there never was a current one to begin with."
  (when search--current-index
    (let ((entry (nth search--current-index old-nav)))
      (setq search--current-index
            (and entry (search--index-of entry (search--nav-results)))))))

(defun search--result-at-buffer-pos (pos)
  "The entry in `search--results' whose recorded BUFFER-POS is exactly
POS, or nil -- same exact-match-only reasoning (and the same hand-edit-
drift caveat) as `compile--error-at-buffer-pos' (compile.el). M85 fix
round F1: a HIDDEN entry's BUFFER-POS is nil (`search--render' sets it
that way, never a stale leftover number), so `(aref e 3)' is checked
non-nil BEFORE the numeric `=' -- comparing `nil' against an integer
POS would otherwise signal `wrong-type-argument', and skipping hidden
entries here (rather than letting a stale position collide with a
real, currently-visible one) is exactly what closes F1's own repro."
  (let ((entries search--results) (found nil))
    (dolist (e entries)
      (when (and (not found) (aref e 3) (= (aref e 3) pos))
        (setq found e)))
    found))

(defun search-goto-result-at-point ()
  "RET in `*search*': jump to whichever result started on the line point
is currently on, or report there is none. M85 fix round F2: indexes
into `search--nav-results', not `search--ordered-results' -- so a
subsequent `n'/`p' continues from THIS jump using the same navigable
set `search--current-index' is defined against (see that variable's
own doc comment); indexing against the full unfiltered list here would
silently desync `search--current-index''s meaning from what `n'/`p'
themselves use."
  (interactive)
  (let* ((bol (line-beginning-position))
         (entry (search--result-at-buffer-pos bol)))
    (if (not entry)
        (message "No search result on this line")
      (setq search--current-index
            (search--index-of entry (search--nav-results)))
      (search--goto-result entry))))

(defun search-goto-module-declaration ()
  "`M-.' in `*search*' (M85, D7): jump to the MODULE DECLARATION the hit
at point names, rather than the hit line itself -- the natural next
step after grepping for a module name. Works by first jumping to the
hit's own file:line:col exactly like `search-goto-result-at-point'
(RET) would, then running `verilog-goto-module-at-point' (verilog-
nav.el, M55) AT that real position -- the same treesit-based \"is
point on an instantiation's own type name\" check and buffer-then-
library lookup `M-.' already uses everywhere else in this editor,
reused rather than reimplemented. See verilog-nav.el's own header for
why point has to be resolved in the REAL target buffer, not guessed
from the grep result line's raw text: only a live treesit parse of the
actual file can tell a type name apart from an instance name, a port
name, or plain prose that happens to contain the same identifier --
this is also why a search hit on a `module' DECLARATION line's own
name (rather than an instantiation) does not resolve here: that name
is not inside a `module_instantiation' node at all, the only shape
`verilog-goto-module-at-point' recognizes (v1 scope, same as every
other `verilog-nav.el' caller).

Unlike `verilog-goto-module-at-point' used as a `local-definition-
function' (which stays silent on a miss so `lsp-definition-at-point'
can fall through to an LSP tier), THIS caller has no further tier to
fall through to, so a miss always gets an explicit message (D7) --
either \"No search result on this line\" (nothing to jump to at all)
or \"Not on a module name\" (jumped to the hit, but it isn't one,
covering both a non-Verilog hit and a Verilog hit that just isn't an
instantiation's type name -- optimizing for \"never silent\" over
distinguishing the two)."
  (interactive)
  (if (not (search--require-search-buffer))
      nil
    (let* ((bol (line-beginning-position))
           (entry (search--result-at-buffer-pos bol)))
      (if (not entry)
          (message "No search result on this line")
        ;; M85 fix round F2: `search--nav-results', not `search--
        ;; ordered-results' -- see `search-goto-result-at-point''s own
        ;; identical comment.
        (setq search--current-index (search--index-of entry (search--nav-results)))
        (search--goto-result entry)
        (unless (verilog-goto-module-at-point)
          (message "Not on a module name"))))))

(defun search--install-view-keymap ()
  "Install `search-mode''s READ-ONLY, navigation-only local keymap on
the CURRENT buffer (D8/D10) -- the state `*search*' is in whenever it
is not actively being edited: streaming results, after `search-edit-
mode' was entered and then either applied or discarded, or right after
a brand new search resets it. Idempotent and callable any number of
times, unlike `search--ensure-output-buffer''s own one-time init guard
-- see that function's doc comment for why the two are separate."
  (major-mode-internal-set 'search-mode)
  (let ((map (make-sparse-keymap)))
    (define-key map "q" 'quit-source-return)
    (define-key map "n" 'search-next-result)
    (define-key map "p" 'search-previous-result)
    (define-key map "RET" 'search-goto-result-at-point)
    (define-key map "C-x C-q" 'search-edit-mode)
    (define-key map "M-." 'search-goto-module-declaration)
    (define-key map "/" 'search-filter-start)
    (use-local-map map))
  (set-buffer-read-only t))

(defvar search--filter-prompt "Search filter: "
  "M85 fix round F3: the exact, distinctive prompt string
`search-filter-start' always opens its filter prompt with --
`search--filter-input-changed' compares `(minibuffer-prompt)' against
this to tell \"the minibuffer currently open is the filter prompt I
myself started\" apart from \"some entirely unrelated minibuffer prompt
happens to be open right now while `*search*' is still the current
buffer underneath it\" (F3's own repro: opening a minibuffer never
changes the current buffer at all, so `(buffer-name)' alone reads
`*search*' for EVERY prompt opened while `*search*' is current, not
just this one). Not `defcustom'-worthy (nothing should ever need to
change it), but a `defvar' rather than an inline literal so the SAME
string is used at both the open site and the compare site by
construction, never two copies that could drift apart."
  )

(defun search-filter-start ()
  "`/' in `*search*' (M85, D8: VIEW keymap only -- never installed in
`search-edit-mode''s own keymap, see `search--install-edit-keymap').
Opens a live-filter prompt: every keystroke re-renders `*search*'
in place (`search--filter-input-changed', the `minibuffer-input-
changed-hook' function that does the actual narrowing; `search--
render' is the redraw itself, D2), keeping only entries whose own
`FILE:LINE:COL:TEXT' line `orderless-rank's non-nil against the text
typed so far (`search--matching-p', D3) -- the SAME multi-token,
smart-case matcher `completing-read'/`M-x' already use (M84/M85),
not a second hand-rolled implementation.

Refuses while `*search*' is in `search-edit-mode' (D5's second,
asymmetric half): filtering redraws the buffer and recomputes every
visible entry's BUFFER-POS, which would invalidate M83's row-indexed
edit-apply snapshot (`search--edit-original-text') out from under an
edit already in progress -- the OTHER direction (filter first, THEN
`C-x C-q' to edit) needs no guard at all here, since `search-edit-
mode' simply snapshots whatever `*search*' currently shows, filtered
or not.

Re-invoking `/' mid-session (filter already engaged) reuses the
CURRENT filter text as the prompt's initial contents, rather than
resetting to empty -- refining an existing filter is the expected
repeat use, not starting over.

Opens the prompt with `search--filter-prompt' specifically (M85 fix
round F3), not a generic literal -- `search--filter-input-changed'
depends on that EXACT string to recognize this particular minibuffer
session as its own, see that function's own doc comment.

M85 fix round H1: captures `search--nav-results' BEFORE `search--
filter' changes and recalibrates `search--current-index' against it
afterward (`search--recalibrate-current-index') -- a no-op the FIRST
time this runs (nil -> \"\" changes `search--filter' from unfiltered to
an empty-but-engaged filter, which `search--matching-p' treats
identically, so the nav list's own COMPOSITION is unchanged and
whatever entry the index named, if any, is still at the same spot),
but kept here rather than assumed harmless: `search-filter-start' is
also the RE-ENTRY point when a filter session is already active
(\"Re-invoking `/' mid-session\", above) -- reusing the CURRENT filter
text as the prompt's initial contents does not itself change
`search--filter' or the nav list, so this call is a no-op then too,
but the guarantee (index always valid against whatever `search--
render' JUST drew) is worth keeping uniform across every call site
that touches the filter, not just the ones a reviewer happens to
construct a repro for."
  (interactive)
  (if (not (search--require-search-buffer))
      nil
    (if (eq (major-mode-internal-get) 'search-edit-mode)
        (message "%s" search--filter-blocked-message)
      (let ((old-nav (search--nav-results)))
        (setq search--filter (or search--filter ""))
        (search--render)
        (search--recalibrate-current-index old-nav))
      (read-string search--filter-prompt (lambda (_text) nil) search--filter))))

(defun search--filter-input-changed (input)
  "`minibuffer-input-changed-hook' function (M85 D1/D2/D3): live-
narrows `*search*' as INPUT (the minibuffer's own current text)
changes, while a `search-filter-start' (`/') session is in progress.
Registered UNCONDITIONALLY and globally at load time, below -- there
is only one `minibuffer-input-changed-hook', shared by every prompt in
the editor (see simple.el's own doc comment on that variable) -- so
every OTHER minibuffer prompt (`M-x', `find-file', a plain `search-
project' pattern prompt, ...) must pass through here as a cheap no-op
rather than accidentally redrawing `*search*' while the user is typing
something unrelated to filtering.

THREE guards (M85 fix round F3 -- an earlier version of this function
had only the latter two, and both of THOSE turned out to be
insufficient on their own, see below):

1. `(equal (minibuffer-prompt) search--filter-prompt)' -- is the
   minibuffer open RIGHT NOW actually the filter prompt `search-
   filter-start' itself opened, as opposed to some entirely unrelated
   LATER prompt (`M-x', `find-file', a plain `search-project' pattern
   prompt, ...)? THIS is the guard that actually answers \"is this MY
   prompt\" -- F3's own repro: opening a minibuffer never changes the
   current buffer (there is no separate \"*Minibuffer*\" buffer in this
   editor's model, see `(minibuffer-prompt)''s own doc comment,
   builtins/ui.rs), so `(buffer-name)' alone reads `*search*' for
   EVERY prompt opened while `*search*' is current underneath it, not
   just the filter one -- filtering to `RET' to close it, then later
   opening `M-x' on `*search*' and typing, used to silently overwrite
   `search--filter' with whatever command name was being typed and
   redraw `*search*' out from under it, no error, no message.
2. `search--filter' non-nil -- a filter session has been engaged at
   all THIS search (cheap short-circuit; also doubles as \"has this
   editor even loaded a filter session ever\", though guard 1 alone
   already rules out any OTHER prompt).
3. `*search*' must not currently be in `search-edit-mode' -- defense
   in depth for a narrower case guard 1 alone doesn't cover: if some
   FUTURE code path ever reused `search--filter-prompt' as ITS OWN
   prompt string while `*search*' happened to be mid-edit (nothing
   today does), this still refuses to corrupt M83's row-indexed edit-
   apply snapshot the way D5's second half exists to prevent.

M85 fix round H1: also recalibrates `search--current-index'
(`search--recalibrate-current-index') against the nav list as it
stood JUST BEFORE this keystroke's own filter change -- every OTHER
caller of `search--render' that can change `search--nav-results''s
own composition (this is currently the only one reachable after the
FIRST `search--render' call in `search-filter-start', which itself
already recalibrates against a nav list that hasn't changed yet) must
keep the index in sync the same way, or `n'/`p' silently land on
whatever ENTRY happens to now sit at the OLD numeric index -- not the
entry the user was actually last standing on."
  (when (and (equal (minibuffer-prompt) search--filter-prompt)
             search--filter
             (not (eq (major-mode-internal-get) 'search-edit-mode)))
    (let ((old-nav (search--nav-results)))
      (setq search--filter input)
      (search--render)
      (search--recalibrate-current-index old-nav))))

(add-hook 'minibuffer-input-changed-hook 'search--filter-input-changed)

(defun search--install-edit-keymap ()
  "Install `search-edit-mode''s WRITABLE local keymap on the CURRENT
buffer (D10) -- entered only via `search-edit-mode' (`C-x C-q'), left
only via `search-edit-apply' (`C-c C-c') or `search-edit-discard'
(`C-c C-k'), both of which call `search--install-view-keymap' to
return to the read-only state."
  (major-mode-internal-set 'search-edit-mode)
  (let ((map (make-sparse-keymap)))
    (define-key map "C-c C-c" 'search-edit-apply)
    (define-key map "C-c C-k" 'search-edit-discard)
    (use-local-map map))
  (set-buffer-read-only nil))

(defun search--ensure-output-buffer ()
  "Get-or-create `*search*', installing `search-mode' and its local
keymap the first time (`search--install-view-keymap'). Parallel to
`compile--ensure-output-buffer' (compile.el). The `unless' guard here
is a ONE-TIME buffer-creation check, never the mechanism that switches
between `search-mode''s and `search-edit-mode''s keymaps later (D10) --
`search--reset' handles forcing the view keymap back on for every
subsequent search, unconditionally, including the case where a new
search starts while `*search*' happened to be mid-edit."
  (let ((buf (get-buffer-create search-output-buffer-name)))
    (with-current-buffer buf
      (unless (eq (major-mode-internal-get) 'search-mode)
        (search--install-view-keymap)))
    buf))

;; --- M83: editable results (search-edit-mode) ---------------------------

(defun search--parse-header (line root)
  "Parse LINE's `FILE:LINE:COL:' header (`search--result-regexp'), FILE
resolved against ROOT. Returns a list (FILE LINE COL TEXT), or nil if
LINE doesn't match within `search--line-prefix-limit' characters. Pure
function of LINE/ROOT, unlike the streaming-insert path's own
`search--parse-result-line' -- this does NOT require FILE to exist on
disk (D2): existence and content are both re-verified for real, per
line, at apply time (D4); at snapshot/apply-scan time this only cares
about what the header SAYS."
  (let* ((bound (min search--line-prefix-limit (length line)))
         (prefix (substring line 0 bound)))
    (when (string-match search--result-regexp prefix)
      (list (expand-file-name (match-string 1 prefix) root)
            (string-to-number (match-string 2 prefix))
            (string-to-number (match-string 3 prefix))
            (substring line (match-end 0) (length line))))))

(defun search--require-search-buffer ()
  "Non-nil if the CURRENT buffer is `*search*' itself; otherwise messages
and returns nil. Shared precondition for all three `search-edit-*'
commands below (F3, fix round R2): each is `(interactive)' with no
argument, so `M-x' can invoke ANY of them from ANY buffer, entirely
bypassing the local keymap that would otherwise guarantee `*search*'
is current. Without this guard, `M-x search-edit-discard' called from
some unrelated buffer would still run its full body against WHATEVER
buffer happens to be current -- `erase-buffer' and all."
  (if (equal (buffer-name) search-output-buffer-name)
      t
    (message "Not in a *search* buffer")
    nil))

(defun search-edit-mode ()
  "`C-x C-q' in `*search*': make the buffer writable so result lines'
TEXT can be hand-edited, snapshotting the buffer's current full text
first (`search--edit-original-text') -- `search-edit-apply'/`search-
edit-discard' both need it, see this file's header (D2/D3). Refuses
outright while a search is still running (D9, `search--procs'), while
already editing (F3, fix round R2 -- re-snapshotting here would
silently replace the ORIGINAL baseline `search-edit-discard' is
supposed to restore, with whatever's on screen right now instead), or
when `*search*' isn't even the current buffer (F3, `search--require-
search-buffer')."
  (interactive)
  (cond
   ((not (search--require-search-buffer)) nil)
   (search--procs
    (message "Cannot edit search results while a search is still running"))
   ((eq (major-mode-internal-get) 'search-edit-mode)
    (message "Already editing search results"))
   (t
    (setq search--edit-original-text (buffer-string))
    (search--install-edit-keymap)
    (message "search-edit-mode: C-c C-c to apply, C-c C-k to discard"))))

(defun search-edit-discard ()
  "`C-c C-k' in `search-edit-mode': throw away every edit, restoring
`*search*' to exactly the text `search-edit-mode' snapshotted, and
return to the read-only view state. Refuses if `*search*' isn't
current, or if not currently editing (F3, fix round R2) -- repro
before this guard existed: `M-x search-edit-discard' while NOT editing
ran `erase-buffer' FIRST and only found out there was nothing to
restore (`search--edit-original-text' being nil) afterward, wiping out
real, un-applied search results with no way back."
  (interactive)
  (cond
   ((not (search--require-search-buffer)) nil)
   ((not (eq (major-mode-internal-get) 'search-edit-mode))
    (message "Not currently editing search results"))
   (t
    (let ((inhibit-read-only t))
      (erase-buffer)
      (insert search--edit-original-text))
    (set-buffer-modified-p nil)
    (setq search--edit-original-text nil)
    (search--install-view-keymap)
    (message "Search edit discarded"))))

(defun search--edit-group-by-file (edits)
  "Group EDITS (a list of #[FILE LINE COL ORIG-TEXT NEW-TEXT] vectors,
see `search-edit-apply') into an alist FILE -> (edit ...), preserving
first-seen file order. Per-file ordering within each group does not
matter here -- `search-edit-apply' sorts each file's own list by LINE,
descending (D5), right before applying it."
  (let ((groups nil))
    (dolist (e edits)
      (let* ((file (aref e 0))
             (cell (assoc file groups)))
        (if cell
            (setcdr cell (cons e (cdr cell)))
          (setq groups (cons (cons file (list e)) groups)))))
    (nreverse groups)))

(defun search--edit-rollback (buf file skips)
  "Undo the in-memory batch edits `search--edit-apply-to-file' just made
to BUF, for FILE, after something (typically `save-buffer''s own
whole-file conflict guard) failed AFTER those edits already landed in
BUF's text (G1, fix round R3) -- see that function's own `error'
handler for the data-corruption path this closes. Relies entirely on
the `undo-boundary' pair `search--edit-apply-to-file' already brackets
its edit loop with (F1) to make `undo-internal' revert EXACTLY this
batch, never anything the user had typed into BUF before this function
touched it. Returns SKIPS, with one more entry appended if the
rollback itself fails -- NOT swallowed: a buffer reported as failed to
apply that is ALSO silently still dirty is worse than one that is
merely dirty with an explicit warning attached."
  (condition-case rollback-err
      (progn
        (with-current-buffer buf (undo-internal))
        skips)
    (error
     (cons (format "%s: rollback after save failure also failed (%s) -- this buffer may still hold unsaved edits from this batch"
                   file (dired--error-first-line rollback-err))
           skips))))

(defun search--edit-apply-to-file (file edits)
  "Apply EDITS (all for FILE, see `search--edit-group-by-file') to FILE
via `find-file' + `save-buffer' (D1), descending by LINE (D5), skipping
(not forcing) any line whose CURRENT content no longer matches its
snapshot original (D4). Returns (APPLIED-COUNT . SKIP-REASONS) --
SKIP-REASONS a list of human-readable strings, one per skipped line or
per whole-file failure. The entire body runs inside ONE `condition-
case' (D7, same per-item try/skip/keep-going shape `dired.el''s own
batch operations use, `dired--error-first-line' reused directly,
dired.el loads before this file) -- a file that's read-only, deleted,
or fails `save-buffer''s own external-conflict guard is reported as a
single skip covering all of FILE's edits, not one skip per line.

F9 (fix round R2, tail review, HONESTY NOTE -- keep, do not remove):
the descending-LINE `sort' below is currently DEAD in the sense that no
test can observe its direction mattering, and independently confirmed
by tail review, not just this comment's own claim: every edit's
position is recomputed from `point-min' via `forward-line' fresh, for
EVERY edit, and NEW-TEXT can never itself contain a newline (it comes
from `split-string' on a literal newline separator, see `search-edit-apply' -- a split
piece is by construction newline-free), so no edit here can ever shift
another edit's own line NUMBER regardless of application order.
Ascending order would pass every existing test identically. Kept
anyway per D5's own stated rationale (`replace-region-contents',
editing.rs): the day a future revision supports replacing a MULTI-line
span (letting NEW-TEXT itself span multiple lines, which WOULD shift
subsequent line numbers), this is free, already-correct insurance
against exactly that -- removing it now to chase coverage would be
removing the one thing standing between works-today and silently-
wrong-the-day-this-files-own-scope-changes."
  (let ((sorted (sort (copy-sequence edits) (lambda (a b) (> (aref a 1) (aref b 1)))))
        (applied 0)
        (skips nil)
        (buf nil))
    (condition-case err
        (if (not (file-exists-p file))
            ;; G4 (fix round R3): `find-file' on a path that doesn't
            ;; exist opens an EMPTY new buffer rather than signaling an
            ;; error (editing.rs) -- if this file's search result's own
            ;; ORIG-TEXT happens to be the empty string (an edge case,
            ;; but not a contrived one: a blank line matched by a
            ;; pattern that allows one), D4's own `string=' check would
            ;; find that empty buffer's content matching, apply the
            ;; edit, and `save-buffer' would then WRITE THE FILE BACK
            ;; INTO EXISTENCE -- recreating something the user (or
            ;; something else) deliberately deleted after the search
            ;; ran. Checked once, up front, rather than relying on D4's
            ;; per-line content check to happen to catch it.
            (push (format "%s: file no longer exists, skipped" file) skips)
          (progn
            (setq buf (find-file file))
            ;; A SINGLE `with-current-buffer' wraps the whole `dolist'
            ;; here -- one closure covers the WHOLE batch of edits to
            ;; this file, rather than re-entering `buf' fresh on every
            ;; single edit, which is both simpler to read and cheaper
            ;; (no repeated buffer-switch overhead per line).
            (with-current-buffer buf
            ;; F1 (fix round R2): a boundary BEFORE any edit closes off
            ;; whatever undo group the target buffer's OWN prior editing
            ;; session left open -- `find-file' reusing an already-open
            ;; buffer (editing.rs) does not touch undo at all, and
            ;; `execute_command' (commands.rs) only ever puts a boundary
            ;; on the DISPATCHING buffer (`*search*'), never on a buffer
            ;; this function merely visits via a plain function call.
            ;; Repro before this fix: type a few characters into an
            ;; already-open target buffer, don't save, then apply an
            ;; edit to it from `*search*' -- the batch write landed in
            ;; the SAME undo group as those unsaved keystrokes, so a
            ;; single `(undo)' afterward reverted BOTH at once. This
            ;; boundary is also what G1's rollback (below) depends on to
            ;; revert EXACTLY this batch and nothing the user typed
            ;; before it -- without it, G1's `undo-internal' call would
            ;; have no clean group boundary to stop at.
            (undo-boundary)
            (dolist (e sorted)
              (let ((line-num (aref e 1))
                    (orig-text (aref e 3))
                    (new-text (aref e 4)))
                (goto-char (point-min))
                (forward-line (1- line-num))
                (let ((bol (line-beginning-position))
                      (eol (line-end-position)))
                  (if (string= (buffer-substring bol eol) orig-text)
                      (progn
                        (delete-region bol eol)
                        (goto-char bol)
                        (insert new-text)
                        (setq applied (1+ applied)))
                    (push (format "%s:%d: changed since search, skipped" file line-num)
                          skips)))))
            ;; ...and a boundary AFTER every edit closes THIS batch off
            ;; in turn, so whatever the user types into `buf' next
            ;; doesn't retroactively merge with it either -- the batch
            ;; write ends up as its own clean, independent undo group.
            (undo-boundary))
            (when (> applied 0)
              (with-current-buffer buf (save-buffer)))))
      (error
       (push (format "%s: %s" file (dired--error-first-line err)) skips)
       ;; G1 (fix round R3): if anything already reached BUF's in-memory
       ;; text before the error fired -- most commonly `save-buffer'
       ;; itself failing on its OWN whole-file conflict guard, after
       ;; every individual line here already passed D4 -- that partial
       ;; batch must not be left sitting unsaved in BUF. Without this,
       ;; the user is told "0 lines applied" while BUF is silently dirty
       ;; with exactly the edits just reported as failed; the NEXT
       ;; unrelated `save-buffer' on BUF (the user's own `C-x C-s', or a
       ;; retried apply after resolving the conflict) would persist them
       ;; anyway, with no further warning -- the same silent-corruption
       ;; shape D1(c) already guards against, just reached from the
       ;; opposite direction. The F1 boundaries above are what make this
       ;; safe: `undo-internal' reverts exactly the group between them,
       ;; never anything the user had already typed into BUF first.
       (when (and buf (> applied 0))
         (setq skips (search--edit-rollback buf file skips)))
       (setq applied 0)))
    (cons applied (nreverse skips))))

(defun search-edit-apply ()
  "`C-c C-c' in `search-edit-mode': write every changed result line back
to its own file/line, batched across however many files the edits
span (D1-D7), then return to the read-only view state. See this file's
header for the full design (`--- M83: editable search results
(wgrep-level) ---'). Refuses if `*search*' isn't current, or if not
currently editing (F3, fix round R2, same reasoning as `search-edit-
discard''s own identical guard -- `M-x' can reach this from anywhere)."
  (interactive)
  (if (not (search--require-search-buffer))
      nil
    (if (not (eq (major-mode-internal-get) 'search-edit-mode))
        (message "Not currently editing search results")
      (search-edit-apply--do))))

(defun search-edit-apply--do ()
  "The body of `search-edit-apply', once its preconditions already
passed -- see that function's own doc comment."
  (let* ((root (or search--last-root default-directory))
         (original-lines (split-string search--edit-original-text "\n" nil))
         (current-lines (split-string (buffer-string) "\n" nil))
         (row-count (min (length original-lines) (length current-lines)))
         (extra (- (length current-lines) (length original-lines)))
         (edits nil)
         (broken nil)
         (i 0))
    (while (< i row-count)
      (let ((orig-line (nth i original-lines))
            (cur-line (nth i current-lines)))
        (unless (string= orig-line cur-line)
          (let ((oparsed (search--parse-header orig-line root)))
            (when oparsed
              (let ((cparsed (search--parse-header cur-line root)))
                (if (and cparsed
                         (string= (nth 0 cparsed) (nth 0 oparsed))
                         (= (nth 1 cparsed) (nth 1 oparsed))
                         (= (nth 2 cparsed) (nth 2 oparsed)))
                    (push (vector (nth 0 oparsed) (nth 1 oparsed) (nth 2 oparsed)
                                  (nth 3 oparsed) (nth 3 cparsed))
                          edits)
                  (push (format "%s:%d: prefix corrupted by edit, skipped"
                                (nth 0 oparsed) (nth 1 oparsed))
                        broken))))))
        (setq i (1+ i))))
    (setq edits (nreverse edits))
    (setq broken (nreverse broken))
    (let ((applied-files 0)
          (applied-lines 0)
          (skip-reasons nil))
      (dolist (group (search--edit-group-by-file edits))
        (let* ((result (search--edit-apply-to-file (car group) (cdr group)))
               (n-applied (car result)))
          (when (> n-applied 0)
            (setq applied-files (1+ applied-files))
            (setq applied-lines (+ applied-lines n-applied)))
          (setq skip-reasons (append skip-reasons (cdr result)))))
      (setq skip-reasons (append skip-reasons broken))
      ;; CRITICAL (fix round R1, found by this milestone's own undo
      ;; test, not reasoned out in advance): `find-file' (inside
      ;; `search--edit-apply-to-file') PERMANENTLY switches `ed.current'
      ;; to whichever target file it last visited -- unlike `with-
      ;; current-buffer', it does not restore the previous current
      ;; buffer afterward. Without switching back to `*search*'
      ;; explicitly HERE, `search--install-view-keymap' below (which
      ;; acts on the CURRENT buffer, not a named one) would install
      ;; `search-mode' and flip `set-buffer-read-only t' onto the LAST
      ;; TARGET FILE's buffer instead of `*search*' -- silently making
      ;; the user's own RTL source buffer read-only and switching its
      ;; major mode to `search-mode'. Caught by `search_edit_apply_
      ;; produces_a_single_undo_group_per_target_buffer' (search_
      ;; tests.rs): the immediately-following `(undo)' on that target
      ;; buffer failed with "Buffer is read-only", not because undo
      ;; itself was broken.
      (switch-to-buffer-internal search-output-buffer-name)
      (search--install-view-keymap)
      ;; F4 (fix round R2, tail review): `search-edit-discard' already
      ;; clears this; applying never did, so a SUCCESSFUL apply left
      ;; the mode line showing `*search*' as permanently modified --
      ;; the user's own in-buffer edits (typed while writable) really
      ;; did modify it, but once committed there is nothing left
      ;; unsaved about `*search*' ITSELF (its own text was never
      ;; written anywhere; only the target FILES were).
      (set-buffer-modified-p nil)
      (setq search--edit-original-text nil)
      (let ((extra-note
             (cond
              ((> extra 0) (format "; %d extra line(s) in *search* ignored" extra))
              ((< extra 0) (format "; %d line(s) missing from *search*, ignored" (- extra)))
              (t ""))))
        (if skip-reasons
            (message "Applied %d line(s) across %d file(s); %d skipped (%s)%s"
                     applied-lines applied-files (length skip-reasons)
                     (string-join skip-reasons "; ") extra-note)
          (message "Applied %d line(s) across %d file(s)%s"
                   applied-lines applied-files extra-note))))))

;; --- M-x search-project / search-again -----------------------------------

(defun search--reset ()
  "Kill any previous search job, clear `*search*', drop the previously
parsed result list, and force `*search*' back to the read-only view
state (D8/D9) -- called at the start of every `search--start'.

Updated reasoning (fix round R2, F2/F6): this function used to be the
ONLY thing standing between a mid-edit `*search*' and a new search
silently erasing it -- `search-project'/`search-project-regexp'/
`search-again' now refuse outright while `search--editing-p' is true
(F2), so under the ONLY entry points this file itself exposes, this
function should never actually observe `*search*' in `search-edit-
mode' anymore. The force-view-keymap call below is kept anyway, as
defense in depth for any FUTURE or direct caller of `search--start'/
`search--reset' that doesn't go through those three guarded commands
-- `search_reset_forces_view_mode_even_if_search_edit_mode_was_left_
installed' (search_tests.rs) exercises this function directly for
exactly that reason, bypassing the now-guarding commands on purpose.
Mirrors `compile--reset' (compile.el) on `search--procs' instead, its
own list -- see this file's header for why the two never share state."
  (dolist (entry search--procs)
    (shell-process-kill (aref entry 0)))
  (setq search--procs nil)
  (setq search--results nil)
  (setq search--current-index nil)
  (setq search--edit-original-text nil)
  (setq search--filter nil)
  (let ((buf (get-buffer search-output-buffer-name)))
    (when buf
      (with-current-buffer buf
        (let ((inhibit-read-only t))
          (erase-buffer))
        (set-buffer-modified-p nil)
        (search--install-view-keymap)))))

(defun search--start (pattern root regexp-p)
  "Shared body of `search-project'/`search-project-regexp'/`search-
again': reset any previous job, spawn `search-program' with
`search-arguments-regexp' (if REGEXP-P) or `search-arguments-literal'
against (shell-quoted) PATTERN in ROOT, and show `*search*' immediately
-- like `compile--start' (compile.el), not `shell-command' (M-!, silent
unless there was output): a project-wide search is something you
expect to watch stream in.

Callers are responsible for rejecting an empty PATTERN before calling
this (`search--start-if-nonempty') -- this function itself does not
guard against it."
  (let ((source (quit-source-of-current)))
    (search--reset)
    (setq search--last-pattern pattern)
    (setq search--last-root root)
    (setq search--last-regexp-p regexp-p)
    (let* ((args (if regexp-p search-arguments-regexp search-arguments-literal))
           (cmd (format "%s %s %s" search-program args (search--shell-quote pattern)))
           (proc (condition-case nil
                     (start-shell-process cmd root)
                   (error nil))))
      (if (not proc)
          (message "Cannot start process: %s" cmd)
        (let ((buf (search--ensure-output-buffer)))
          (shell-command--maybe-show buf source)
          (setq search--procs (list (vector proc root "" nil 0 nil nil))))))))

(defun search--start-if-nonempty (pattern root regexp-p)
  "Shared guard for `search-project'/`search-project-regexp': refuse an
empty PATTERN outright instead of ever handing it to `search--start'
(fix round R1). Repro before this guard existed: submitting the
\"Search for:\" prompt with an empty string (just pressing RET) built
the command `rg ... --fixed-strings -- ''' (PATTERN shell-quoted to an
empty string), which -- unlike a human typing an empty pattern at
a real `rg' terminal invocation and immediately noticing something is
wrong -- silently matched EVERY line of EVERY file under ROOT, raced
straight past `search-max-results', and got killed with a truncation
notice as the only visible sign anything unusual had even been
searched for."
  (if (string-empty-p pattern)
      (message "Search: empty pattern")
    (search--start pattern root regexp-p)))

(defvar search--edit-blocked-message
  "Search results are being edited -- C-c C-c to apply or C-c C-k to discard first"
  "Shared rejection message for `search-project'/`search-project-
regexp'/`search-again' (F2, fix round R2) when `*search*' is currently
in `search-edit-mode' -- one string so all three say exactly the same
thing.")

(defun search--editing-p ()
  "Non-nil if `*search*' exists and is currently in `search-edit-mode'
(F2, fix round R2) -- the mirror image of D9 (`search-edit-mode'
itself refusing to run while `search--procs' is non-nil). Before this
guard existed, `search--reset' (called by every `search--start')
unconditionally erased `*search*' and forced it back to the read-only
view state regardless of what was on screen -- starting a new search
while mid-edit silently discarded whatever the user had just typed and
not yet applied, with no warning."
  (let ((buf (get-buffer search-output-buffer-name)))
    (and buf
         (eq (with-current-buffer buf (major-mode-internal-get))
             'search-edit-mode))))

(defun search-project ()
  "Bound to `C-c s s' (simple.el). Prompt for a LITERAL (fixed-string,
not regex) search pattern and run it asynchronously against the
current buffer's project root
(`search--default-root' -- see this file's header for why that walks up
to the nearest `.git', not `lsp--project-root''s wider marker list).
See this file's header (\"Why literal search is the DEFAULT\") for why
this command treats PATTERN literally rather than as a regular
expression -- use `search-project-regexp' for the latter. Remembers the
pattern, root, and literal-vs-regexp mode
(`search--last-pattern'/`search--last-root'/`search--last-regexp-p')
for `search-again'. Refuses while `*search*' is mid-edit (F2, `search--
editing-p')."
  (interactive)
  (if (search--editing-p)
      (message "%s" search--edit-blocked-message)
    (let ((root (search--default-root)))
      (read-string "Search (literal) for: "
                   (lambda (pattern) (search--start-if-nonempty pattern root nil))
                   nil "search"))))

(defun search-project-regexp ()
  "Like `search-project', but PATTERN is matched as an `rg'-flavored
regular expression instead of a literal string. Bound to `C-c s r'
(simple.el). See this file's header (\"Why literal search is the
DEFAULT...\") for why this is a SEPARATE command rather than a `C-u'
variant of `search-project'. Refuses while `*search*' is mid-edit (F2,
`search--editing-p')."
  (interactive)
  (if (search--editing-p)
      (message "%s" search--edit-blocked-message)
    (let ((root (search--default-root)))
      (read-string "Search (regexp) for: "
                   (lambda (pattern) (search--start-if-nonempty pattern root t))
                   nil "search"))))

(defun search-again ()
  "Bound to `C-c s a' (simple.el). Rerun the last `search-project'/
`search-project-regexp' invocation
(same pattern, root, AND literal-vs-regexp mode -- `search--last-
regexp-p') without prompting -- the `recompile' (compile.el) of this
file. Bypasses `search--start-if-nonempty''s empty-pattern guard
deliberately: `search--last-pattern' can only ever hold a value that
already passed that guard once. Refuses while `*search*' is mid-edit
(F2, `search--editing-p') -- checked BEFORE the no-previous-search
check, since \"there is nothing to rerun\" is never true while a
previous search's results are still sitting there being edited."
  (interactive)
  (cond
   ((search--editing-p) (message "%s" search--edit-blocked-message))
   ((not search--last-pattern) (message "No previous search"))
   (t (search--start search--last-pattern search--last-root search--last-regexp-p))))

(provide 'search)
