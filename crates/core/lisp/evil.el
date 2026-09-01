;;; evil.el --- vim-style modal editing (M29-M31) -*- lexical-binding: t -*-

;; A self-made vim keybinding layer built entirely on the M28
;; foundation: buffer-local `emulation-keymap' (consulted ahead of the
;; local/global keymaps on every key -- see commands.rs's module doc),
;; `capture-next-key' (steals the very next raw key event for f/F/t/T/r),
;; `mode-line-prefix' (the "<N>"/"<I>"/"<V>"/"<O>" state tag), and
;; `cursor-type' ('box/'bar, GNU Emacs's own variable — see
;; `evil--refresh-cursor'; called `cursor-shape' before M32, TUI-only
;; back then since the GUI already read the differently-named
;; `cursor-type' and so never saw evil's cursor changes at all — a bug
;; fixed by this rename, not just a cosmetic one), and (M34) buffer-local
;; `inhibit-self-insert' (set in `evil--set-state', consulted only at
;; commands.rs's self-insert fallback -- see simple.el's docstring): the
;; normal/visual/operator-pending states' own ASCII-only vim keymaps can
;; never enumerate every possible printable character (CJK text above
;; all), so without this, any key those three states don't specifically
;; bind -- not just CJK, even a plain unbound ASCII key like `q' (a gap
;; the M30 review already flagged for normal state) -- fell through to
;; ordinary self-insert instead of being treated as the unbound key it
;; is. Nothing here touches Rust except three small, generic additions
;; made alongside this file (see commands.rs): `keyboard-quit-hook' (a
;; bare C-g is intercepted ahead of all keymap dispatch, so this hook is
;; the only way an emulation layer can react to it -- see
;; "operator-pending + C-g" below) and `inhibit-self-insert' itself.
;;
;; --- State machine -----------------------------------------------------
;; Five states, one buffer-local symbol `evil--state': `normal',
;; `insert', `visual', `operator-pending' (entered by d/c/y, never
;; user-visible as a standalone mode), `emacs' (evil's layer switched
;; off for this buffer -- dired/eshell/ielm by default, see
;; `evil-emacs-state-modes'). Each state owns one shared keymap
;; (`evil--normal-map' etc.); switching state = installing that keymap
;; as `emulation-keymap', setting the modeline tag, and syncing the
;; (global) `cursor-type' -- see `evil--set-state'.
;;
;; --- Operators and motions ----------------------------------------------
;; Every motion command (h/l/j/k/w/b/e/W/B/E/0/^/$/gg/G/{/}/f/F/t/T/;/,)
;; is a single function bound identically in the normal, visual, AND
;; operator-pending maps (see `evil--bind-motion'): it computes a target
;; position, then either moves point there directly (normal/visual) or,
;; when `evil--pending-operator' is non-nil (set by d/c/y before
;; entering operator-pending), feeds [point, target) to
;; `evil--operator-apply' instead. This is the one structural place this
;; implementation diverges from upstream evil's architecture (which
;; treats "motion" as a generic protocol overlaid with an operator by a
;; higher-level `evil-yank-lines'-style dispatcher) -- here, since
;; dispatch is a flat keymap lookup with no notion of "the motion that
;; just ran", each motion function is responsible for checking
;; `evil--pending-operator' itself. It is a direct, mechanical
;; consequence of the M28 substrate, not an arbitrary choice.
;;
;; Inclusive/exclusive/linewise char-region math (see
;; `evil--operator-apply'): a motion's "landing position" convention in
;; THIS implementation sometimes already matches Emacs's own
;; one-past-the-end style (e.g. `$' = `line-end-position', which already
;; sits just past the last real character) rather than vim's own
;; lands-on-the-character convention -- where that happens, the motion
;; is wired as exclusive (no +1) even though upstream vim's docs call it
;; inclusive, because the landing position itself already carries the
;; "+1" vim would otherwise need to add. See the per-motion bindings
;; below for which is which; this is documented per-motion rather than
;; asserted in one place because it genuinely differs motion by motion.
;;
;; One vim operator special case IS implemented despite the general
;; "keep motions uniform" design above: `:help cw' documents that
;; `cw'/`cW' do NOT swallow the trailing whitespace after a word the
;; way `dw'/`dW' deliberately do -- see `evil--word-forward-target'.
;;
;; --- What's NOT implemented (documented, not silently missing) -------
;; - No vim "cursor never rests past the last character of a line"
;;   invariant. `l', `$', word-motions, x, etc. may leave point one
;;   position further right than real vim (on the newline slot) --
;;   consistent with how this editor already treats point everywhere
;;   else (Emacs style), just not vim's stricter rule.
;; - `dw'/`cw' at/near a line's end does not special-case "stop at
;;   end-of-line instead of eating into the next line" the way real vim
;;   does; the plain word-boundary target is used as-is.
;; - `{'/`}' don't implement vim's "becomes linewise when both ends land
;;   at column 0" exception.
;; - i/a text objects (iw/aw/i"/a"/i(/a(/i{/a{/i[/a[) don't consult a
;;   pending count (`3iw' behaves like `iw').
;; - i/a/I/A/o/O ignore any pending count entirely (vim's "count
;;   repeats the typed text N times on ESC" is not implemented).
;; - `u'/C-r don't consume a count (`u' is bound straight to simple.el's
;;   `undo' -- see the redo section below for why that identity
;;   matters).
;; - C-d/C-u move point by an approximate half-page line count
;;   (`frame-height' / 2); they do not scroll the window viewport (no
;;   elisp-level window-start control is exposed here).
;; - Ex commands (`:', M30): a small, fixed, case-sensitive command set
;;   (:w/:q/:q!/:wq/:x/:e PATH/:N/:$), plus M42-II's `:s' substitute
;;   (its own range/pattern/replacement grammar -- see the "M42-II: :s
;;   substitute" section below) -- no OTHER ranged commands (`:1,5d'),
;;   no command abbreviation or completion. See the "Ex commands"
;;   section below for the full support table and the `:q'/`:q!'
;;   approximations. Visual state's `:' still drops the selection
;;   itself before the prompt even opens (this editor's minibuffer
;;   can't be prefilled the way vim's own `:'<,'>' convention needs),
;;   but M42-II adds a narrow exception for `:s' specifically: see
;;   `evil-ex-from-visual''s own doc comment.
;; - M42-II `:s' substitute: only `/' as a delimiter (no `:s#pat#rep#'
;;   etc.), only the `g' flag (no `c'/`i', no case-fold at all -- this
;;   engine has none), no dot-repeat, no `&' to repeat the last
;;   substitution, no numbered-only RANGE forms beyond what's listed in
;;   its own section comment. See the "M42-II: :s substitute" section
;;   below for the full grammar.
;; - M42-II keyboard macros (`q'/`@'): only lowercase `a'-`z' registers
;;   (shared namespace with M42-I's named registers/marks is NOT
;;   implied -- kbd-macro storage is its own separate table, see
;;   `Editor::kbd_macros', editor.rs); no persistent recording
;;   indicator beyond the start/stop echo messages; a macro's own
;;   recorded keys are pure Rust state (`Editor::kbd_macros'), not an
;;   elisp value, so they cannot be inspected, edited, or saved to
;;   init.el the way real Emacs's `kmacro-*' machinery allows. See the
;;   "M42-II: keyboard macros" section below.
;; - RET as an operator target (`d<CR>', real vim's linewise "delete
;;   this line and the next", M36 review fix): not implemented --
;;   `evil--op-pending-map' cancels the pending operator on RET instead
;;   (`evil--op-invalid'), same as any other unclaimed key there. Bare
;;   RET in normal/visual state is `evil-ret' (`+'-equivalent: first
;;   non-blank of the next line), which is deliberately NOT wired
;;   through `evil--bind-motion' the way every other motion is, exactly
;;   so it can't be composed with an operator.
;; - Search bridge (`/ ? n N', M30): `/'/`?' bind straight to the
;;   existing `isearch-forward'/`isearch-backward' (point lands wherever
;;   isearch itself already leaves it -- Emacs's just-past-a-forward-
;;   match/on-a-backward-match convention, NOT vim's own always-on-the-
;;   first-character one; bridging onto the existing isearch rather than
;;   reimplementing it was the point). `n'/`N' are plain motions (never
;;   recorded as a dot-repeatable change, per vim semantics) and are NOT
;;   usable as operator targets (`dn' is not implemented -- neither is
;;   bound in `evil--op-pending-map', so it falls to the existing
;;   unbound-printable-key catch-all).
;; - Dot-repeat (`.', M30) does not cover VISUAL-state operators (`v...
;;   d'/`c'/`y'): `evil--record-change' is only ever called from normal-
;;   state/operator-pending commands, so `evil--visual-apply' leaves
;;   `evil--last-change' completely untouched -- `.' after a visual
;;   change just repeats whichever NORMAL-state change happened last
;;   (or reports nothing to repeat, if none yet). Not in the M30 plan's
;;   enumerated recording list; a plausible future extension, not
;;   attempted here.
;; - A count typed before `.' (`3.') repeats the WHOLE recorded change
;;   three times in a row rather than overriding the count the change
;;   was originally recorded with, the way real vim's own `:help .'
;;   does -- see `evil-repeat-change''s docstring. A deliberate
;;   simplification per the M30 plan's own phrasing ("3. = replay three
;;   times"), not an oversight.
;; - i/a/I/A/o/O's dot-repeat replay of the typed text is best-effort if
;;   point moves outside the insert session's own span by some out-of-
;;   band means (this editor's insert-map only ever binds ESC and, as of
;;   M31, C-n/C-p -- so in practice the ESC-only case needs e.g. a mouse
;;   click; C-n/C-p have their OWN narrower way to reach outside the
;;   span, see dabbrev.el's file header) -- see
;;   `evil--finish-insert-session''s docstring.
;; - No yank (Y/yy/yw/y$/yiw/i(/etc. -- any motion, text object, or
;;   same-key-doubling with the `yank' operator) is EVER recorded for
;;   dot-repeat, matching real vim: `.' never replays a bare yank.
;;   (An early M30 draft recorded `Y' anyway, reasoning that it shares
;;   `evil--op-current-lines' with `dd'/`cc' regardless of operator --
;;   a mistake, caught in review: replaying a yank at a NEW position
;;   silently overwrites the kill-ring with different text, which is
;;   actively harmful, not merely non-authentic. `evil--record-change'
;;   now refuses to record while `evil--pending-operator' is `yank',
;;   centrally, for all four op-run-style functions at once.)
;; - Undo grouping for c/s/S/C/o/O (M30, M29 stretch candidate): DONE --
;;   `cw foo<ESC>' is now a single undo group (one `u' restores
;;   "hello world" in one step), via `undo-amalgamate-boundary' (see its
;;   docstring in editing.rs) called from `evil--operator-apply''s
;;   `change' branch and from `evil-open-below'/`evil-open-above' right
;;   after their own delete/newline. Plain i/a/I/A (no prior delete) were
;;   never affected -- `undo_boundary''s existing de-duplication (never
;;   pushes two boundaries back to back) already collapsed those into
;;   one group before M30. Pinned by
;;   `cw_then_typing_then_esc_then_a_single_undo_reverts_both' and
;;   friends in evil_tests.rs (the deliberate update the M29-era pinned
;;   test's own comment anticipated).
;; - A named key (arrow keys etc.) that isn't bound while `operator-
;;   pending' is active can't self-insert (unlike a printable ASCII
;;   character, which does get an explicit `evil--op-invalid' catch-all
;;   binding -- see the keymap population section), but it also can't
;;   trigger `evil--post-command' the way self-insert does
;;   (`commands.rs' `dispatch_key' only runs post-command-hook via
;;   `execute_command'/self-insert, not its bare "... is undefined"
;;   echo path) -- so `operator-pending' stays parked, harmlessly, until
;;   whatever command runs next reaches the command loop and the
;;   `evil--op-pending-fresh' check cancels it then. No data is at risk
;;   (nothing self-inserted), just a one-keystroke-longer recovery.
;; - `evil--current-yank-linewise-p' (the fix for evil's own p/P vs. a
;;   native kill/yank command changing the kill-ring's top without
;;   evil's knowledge -- see its docstring) compares kill-ring content
;;   by `equal', since nothing in this codebase can compare kill-ring
;;   entries by identity (`current-kill' allocates a fresh string every
;;   call). Content that happens to coincide byte-for-byte with evil's
;;   own last yank is indistinguishable from it -- an accepted, narrow
;;   false-negative, not a crash or data-loss risk either way (worst
;;   case: a charwise paste lands linewise, or vice versa).
;; - Visual mode's exit (ESC, or any operator completing) calls
;;   `deactivate-mark', which -- matching plain Emacs semantics -- clears
;;   only the mark's active *flag*, not its position; `(mark)' still
;;   returns the old position afterward. This is an existing, pre-M29
;;   mechanism edge (`region-beginning'/`region-end' and any command
;;   with an "r" interactive spec, e.g. `kill-region', read `mark'
;;   without checking whether it's active), not something introduced or
;;   fixed here: a region-based command run after leaving visual state
;;   can act on the stale mark position instead of erroring "mark not
;;   set". Noted, not changed.
;; - M42 marks (`m'/`` ` ''/`''): only lowercase `a'-`z' (no uppercase
;;   A-Z "global"/cross-buffer marks); no `` `` ''/`''' jump-back-to-
;;   before-the-last-jump; `'<'/`'>' (the visual-selection marks) are
;;   NOT real marks here -- the `:s' substitute command's own `'<,'>'
;;   range support (Part II) reads a dedicated snapshot variable
;;   instead, not `evil--local-marks'. See the "M42: marks" section for
;;   the rest of the design.
;; - M42 named registers (`"a'-`"z'): only lowercase `a'-`z' (no
;;   uppercase A-Z "append to register" variant); no numbered registers
;;   (`"0'-`"9'); no explicit `""' (unnamed register spelled out); no
;;   `:reg'/`:registers' inspection command. See the "M42: named
;;   registers" section.
;; - M45 `g'-prefix commands NOT implemented: `g;'/`g,' (the change
;;   list -- this editor tracks no such list at all); `g*'/`g#' (search
;;   for the word under point -- there is no "last search pattern
;;   derived from point" plumbing here, only the plain `/'/`?' bridge
;;   onto isearch); `gt'/`gT' (next/previous tab page -- this editor has
;;   no tab-page concept, only windows); `gq'/`gw' (reflow/format the
;;   region -- no fill engine here). Visual state's "swap the selection
;;   endpoints" (real vim: also reachable as `gv' a second time in some
;;   configurations) is already bound here, just not under `gv' at all
;;   -- see `evil-visual-swap' (`o', pre-M45) instead; `gv' itself
;;   (M45) is strictly "reselect the last selection", never an endpoint
;;   swap.
;;
;; --- Redo, and why `u' is bound to the bare `undo' symbol -------------
;; `undo-internal' (editing.rs) only continues its backward undo chain
;; when the *previous* command's name was literally "undo" (a hardcoded
;; string compare against `last-command', an internal Rust `Editor'
;; field with NO elisp accessor in this codebase -- confirmed by
;; grepping for it). Two consequences:
;; 1. Binding "u" to anything OTHER than the symbol `undo' itself would
;;    break multi-step undo chaining (u u u going back further each
;;    time) -- every keypress would look like a *fresh* undo to the Rust
;;    check, which (per `undo_step_from''s redo-group-at-the-tail
;;    design) actually means every second "u" would silently redo the
;;    first instead of continuing backward. So `evil--normal-map' binds
;;    "u" directly to `undo' (simple.el), unwrapped.
;; 2. Redo needs `last-command' to NOT be "undo" at the moment
;;    `undo-internal' runs, but there is no elisp setter for it -- the
;;    only way to influence it from elisp is to run a command through
;;    the real command loop (`command-execute', which is what actually
;;    updates it). `evil-redo' below runs a no-op marker command first
;;    for exactly this side effect, then calls `undo-internal' -- see
;;    `evil--redo-marker''s docstring.
;; Known limitation this leads to: a SINGLE `C-r' after any number of
;; `u's correctly redoes the most recent change (tested). A SECOND
;; consecutive `C-r' does not chain to a second redo the way consecutive
;; `u's chain backward -- that would require `last-command' to become
;; literally "undo" again after a redo (so the *next* `undo-internal'
;; call continues via the Rust-internal `pending_undo', the same
;; mechanism that makes multi-`u' chaining work), and the marker trick
;; can only force it to NOT be "undo", not force it TO be "undo"
;; (synthesizing that would mean actually running `undo' again, an
;; unwanted extra step). Not silently dropped -- see
;; evil_tests.rs's `double_undo_chains_backward_then_a_single_redo_...'
;; test and its comment for the full reasoning.
;;
;; --- M30: Ex commands, search bridge, dot-repeat ------------------------
;;
;; Ex commands (`:'): opening the prompt reuses the EXISTING M18/M28
;; minibuffer machinery unchanged -- `evil-ex-command' is just an
;; ordinary command with an `(interactive "s:")' spec (the same
;; mechanism `goto-line' (simple.el) and every M-x-style prompt already
;; use), so binding `:' to it makes `execute_command' (commands.rs) open
;; a minibuffer with prompt ":" and hand the typed string to
;; `evil--run-ex' on RET -- no new Rust-side minibuffer code at all.
;; Visual state's `:' (`evil-ex-from-visual') calls `evil--visual-exit'
;; ITSELF, synchronously, before the SAME `evil-ex-command' spec triggers
;; the minibuffer (via `command-execute', the identical "run a command
;; from inside a command" trick `evil-redo''s marker uses above) -- so
;; the mode-line tag is already back to `<N>' by the time the prompt
;; appears, not after.
;;
;; `evil-ex-quit'/`evil-ex-quit-force' (`:q'/`:q!') call `save-buffers-
;; kill-terminal' as a PLAIN FUNCTION CALL, deliberately NOT via
;; `command-execute' -- an earlier draft used `command-execute' for
;; `:q!' (twice in a row, to exploit the SAME "call again to force"
;; guard the interactive C-x C-c binding gets) the same way `evil-redo'
;; forces `last-command' above; review caught that this has a serious
;; side effect `evil-redo''s use does NOT share: `execute_command' sets
;; `last-command' as an ordinary consequence of running ANY command, so
;; the nested call left `last-command' pointing at `save-buffers-kill-
;; terminal' even after a single plain `:q', poisoning the SAME guard
;; for whatever ran next (a second plain `:q', or the real C-x C-c) into
;; silently force-quitting instead of warning again. `save-buffers-kill-
;; terminal' (editing.rs) now takes an explicit optional FORCE argument
;; instead, read once and touching no shared bookkeeping at all --
;; `evil-ex-quit-force' passes `t', `evil-ex-quit' doesn't call it any
;; differently from typing `:q' by hand would.
;;
;; Search bridge (`/ ? n N'): `n'/`N' needed to learn what the last
;; isearch searched for and which direction, but nothing in this
;; codebase exposed that (only `isearch-start'/`isearch-active-p'
;; existed, and the live query itself is dropped the instant a session
;; ends) -- so `Editor::last_search' (editor.rs) plus two minimal
;; builtins, `isearch-last-string'/`isearch-last-forward-p' (builtins/
;; ui.rs), were added. `isearch_exit' (editor.rs) populates it (a
;; non-empty query only; C-g/`isearch_cancel' never does -- an aborted
;; search wasn't "performed"). With that in hand, `n'/`N' are pure
;; elisp: `search-forward'/`search-backward' (already-existing
;; primitives built on the exact same `crate::gapbuffer::find_forward'/
;; `find_backward' isearch itself uses) with a manual wrap-around retry
;; on failure -- plus `evil--last-match-span' tracking (added in
;; review) so a direction REVERSAL doesn't re-match the span point is
;; already sitting at the edge of; see the search bridge section itself
;; for the full reasoning, it doesn't fit compactly here.
;;
;; Dot-repeat (`.'): `evil--last-change' (Global state section below)
;; holds a zero-arg closure that redoes the last change at wherever
;; point currently is. Every modifying command already existed as its
;; own standalone function (M29's architecture, not something added for
;; this); M30 threads a RECORDING call through each one, right at the
;; point it discovers it's about to complete a `delete' or `change'
;; operator (or, for the operator-less simple edits, unconditionally)
;; -- see `evil--record-change'/`evil--replay-thunk' and the dot-repeat
;; infrastructure section further down for the shared shapes, and
;; `evil--op-run'/`evil--op-run-linewise'/`evil--op-run-object'/
;; `evil--op-current-lines' for where the shared operator-engine
;; functions do this on every caller's behalf. A `yank' operator is
;; deliberately never recorded (see `evil--record-change''s own
;; docstring -- an early draft got this wrong for `Y' specifically,
;; caught in review). i/a/I/A/o/O and the `change' operator's entry
;; into insert state both fold the TEXT typed during that insert
;; session into the recorded replay once ESC lands -- see `evil--begin-
;; insert-session'/`evil--finish-insert-session'. Nothing here is
;; Rust-side: nothing needed to be, since every ingredient (per-command
;; functions, `evil--pending-operator', `evil--count'/`evil--op-count',
;; `capture-next-key'’s already-captured characters) was already
;; reachable from elisp.
;;
;; Undo grouping (c/s/S/C/o/O): the one M30 change that IS Rust-side --
;; `Editor::suppress_next_undo_boundary' (editor.rs) plus the
;; `undo-amalgamate-boundary' builtin (editing.rs) it backs, consulted
;; by `self_insert' (commands.rs) and cleared by `execute_command' AND
;; by both of `commands.rs' `handle_key''s C-g branches (review added
;; the latter: a C-g reaching `evil-insert-exit' via `keyboard-quit-
;; hook' never passes through `execute_command' at all, so that alone
;; wasn't clearing it -- see both docstrings for the exact one-shot
;; handshake). evil.el calls the new builtin from exactly two places --
;; `evil--operator-apply''s `change' branch and `evil-open-below'/
;; `evil-open-above' -- right after their own delete/newline, so the
;; self-insert that follows doesn't start a fresh undo group the way it
;; otherwise would.
;;
;; --- M31: dabbrev completion (`C-n'/`C-p' in insert state) --------------
;;
;; `evil-complete-next'/`evil-complete-previous' (defined right after
;; the "Insert state entry points" section below) are thin wrappers
;; around the engine in the NEW dabbrev.el -- candidate collection,
;; ordering, session/cycling, and `dabbrev-expand'/`M-/' itself all live
;; there; see its own file header for the complete design. This file
;; only adds the two vim-named entry points and their
;; `evil--insert-map' bindings (Keymap population, below). Load order
;; between the two files doesn't matter: every `define-key' call
;; anywhere in this codebase stores a bare SYMBOL (e.g.
;; `'evil-complete-next', never the function value itself), resolved
;; only at dispatch time -- long after every built-in lisp file has
;; loaded, see lib.rs's `init_editor'.

;; --- Global state --------------------------------------------------------

(defvar evil-mode nil
  "Non-nil while evil's keymap layer is active.")

(defvar evil-auto-enable t
  "If non-nil, an interactive session turns evil-mode on automatically
after loading init.el (see `start_session' in src/main.rs). Set to nil
in init.el to opt out.")

(defvar evil-emacs-state-modes '(dired-mode eshell-mode ielm-mode help-mode shell-command-mode compilation-mode search-mode search-edit-mode)
  "Major modes that start in `emacs' state (evil's keymap layer stays
out of the way for that buffer) instead of `normal' state -- modes
with their own well-established single-letter/control-key bindings.

`help-mode' (M67, `describe-bindings') is on this list for the same
reason as the other three: its OWN single-letter binding (`q' ->
`help-quit', installed via `use-local-map' in simple.el) sits in the
LOCAL keymap, which evil's normal-state map -- installed in
`emulation-keymap' once this buffer's state is `normal' -- outranks
outright (`emulation' beats `local' beats `global', see
`commands.rs''s `lookup_layered'). Without this entry, normal state's
own `q' (`evil-record-macro') intercepts the key first and
`help-quit' is never reached at all -- there is no fallthrough once a
higher layer claims a key.

`search-mode' (M82, `search.el') is on this list for the exact same
reason as `compilation-mode' -- its own `n'/`p'/RET'/`q' local keymap
would otherwise lose to evil normal-state's bindings for those same
keys.

`search-edit-mode' (M83, `search.el') -- the WRITABLE state `search-
mode' switches into via `C-x C-q' -- is on this list for the same
reason again: its own `C-c C-c'/`C-c C-k' local keymap would otherwise
lose to evil normal-state's bindings for those same keys the moment
the user drops from insert state back to normal state to save.")

(defvar keyboard-quit-hook nil
  "Functions run when C-g is pressed outside isearch/the minibuffer/a
pending `capture-next-key' callback (mirrors GNU Emacs 29's hook of
the same name). evil-mode's only use of this: recovering from a
pending operator (`d' left hanging) back to normal state, since a bare
C-g is intercepted ahead of all keymap dispatch and never reaches
`emulation-keymap' directly (see commands.rs's `handle_key').")

;; Per-buffer (all set with `setq-local'): current state, the pending
;; operator ('delete/'change/'yank) while in `operator-pending', the
;; count(s) being accumulated, visual selection type, the last
;; f/F/t/T search (for `;'/`,'), M42's local marks, and M42's pending
;; named-register prefix (`"').
(defvar evil--state 'emacs)
(defvar evil--visual-type 'char)
(defvar evil--last-visual nil
  "M45 `gv': (POINT MARK TYPE) snapshot of the most recently ENDED visual
selection, or nil before the first one -- POINT/MARK are buffer
positions, TYPE is `evil--visual-type''s value ('char/'line) at the
time. Written at BOTH places a visual selection can end: `evil--visual-
exit' (ESC / re-pressing v or V) and `evil--visual-apply' (an operator
like d/c/y/gu applied from visual state) -- the second site matters
because operator application never calls `evil--visual-exit' at all
(see that function's own doc comment), so a snapshot taken only in
`evil--visual-exit' would silently miss every `gv' after a visual `d'/
`gu'/etc. Read (never itself cleared) by `evil-visual-restore' (gv).

v1 SIMPLIFICATION, honestly noted here rather than just at the POINT/
MARK-are-positions line above: being plain buffer positions rather than
markers, they do NOT shift with edits made between this snapshot and
the `gv' that reads it back. Real vim's own `gv' uses marks that track
such edits, so it still reselects the SAME TEXT; this v1 reselects the
same NUMERIC RANGE against whatever text is there now, which can land
on unrelated content (or an out-of-range position, clamped by `evil-
visual-restore') if the buffer changed shape in between.")
(defvar evil--count nil)
(defvar evil--op-count nil)
(defvar evil--pending-operator nil)
(defvar evil--last-find-cmd nil)
(defvar evil--last-find-char nil)

;; M42: vim marks (`m'/`` ` ''/`''). Buffer-local, unlike
;; `evil--registers' below -- vim marks are per-buffer (a mark set in
;; one buffer is invisible from another, and killing the buffer takes
;; its marks with it for free, since this is just an ordinary
;; buffer-local variable -- no explicit cleanup needed). Alist of
;; (CHAR . MARKER) -- MARKER is a REGISTERED marker (`point-marker',
;; which pushes onto the owning buffer's `markers' vec -- see
;; buffer.rs), so it rides out edits before/after it exactly like any
;; other marker (`adjust_positions_insert'/`adjust_positions_delete',
;; buffer.rs) rather than evil.el needing to track it by hand.
;; Upserted by `evil--set-local-mark' (same CHAR overwrites), read by
;; `evil--handle-goto-mark'/`evil--replay-mark-thunk'.
(defvar evil--local-marks nil)

;; Set the instant a command legitimately leaves state in
;; `operator-pending' without itself resolving the operator (a count
;; digit, or arming a capture for f/F/t/T -- see
;; `evil--refresh-op-pending-fresh'); cleared by the next
;; `post-command-hook' firing (see `evil--post-command'). Needed
;; because entering (or remaining in) operator-pending is itself the
;; normal, successful completion of a command whose own finish also
;; fires post-command-hook -- that firing must NOT be treated as "a
;; stray key left the state stuck"; only a firing where nothing
;; refreshed the flag in between legitimately means that.
(defvar evil--op-pending-fresh nil)

;; Same pattern, for the count itself: set by `evil--digit', cleared by
;; `evil--post-command' -- so a count started (e.g. typing "5") and
;; then abandoned (switching buffers, running any other command that
;; isn't itself a digit) doesn't silently survive to combine with a
;; LATER, unrelated count (see the review that added this: "5" then
;; switch buffers then "3l" must move 3, not 53).
(defvar evil--count-fresh nil)

;; M42: a pending named-register prefix (`"', `evil-use-register').
;; UNLIKE `evil--op-pending-fresh'/`evil--count-fresh' above, this one
;; does NOT use a fresh-flag-plus-one-more-post-command-hook-firing
;; pattern -- the SETTING event here is a `capture-next-key' callback
;; (the register-name character of `"a''s `a'), which never itself
;; fires `post-command-hook' (see `commands.rs''s `handle_key': a
;; capture delivery bypasses the command loop entirely, the same fact
;; the M29 f/F/t/T comment above already leans on), so a naive port of
;; that pattern would grant the register ONE WHOLE EXTRA, unrelated
;; command's worth of survival after the register name arrives --
;; wrong: `"a' then `l' (an unrelated motion) then a bare `p' must
;; paste the UNNAMED register, not `a'. `evil--post-command' instead
;; clears `evil--pending-register' immediately UNLESS an operator is
;; genuinely in flight (`evil--pending-operator' non-nil -- the `d' of
;; `"ad{motion}' left it so) or `evil--state' is `visual' (`"a' then
;; `v' then any number of selection-extending keys before the `d'/`y'/
;; `c' that actually applies it) -- both are the ONLY ways a register
;; prefix legitimately needs to outlive the command that follows it;
;; anything else (`l', `w', an unrelated `"b' overwriting the pending
;; name, ...) is exactly like an unrelated count digit and must not
;; let the register leak forward. `evil--pending-register' itself is
;; normally consumed (read, then cleared) by whichever of
;; `evil--maybe-write-register'/`evil--do-paste' actually uses it --
;; see their own docstrings -- well before `evil--post-command''s
;; staleness check below would ever run in the common case.
(defvar evil--pending-register nil)

;; Global (like the kill-ring itself, which this pairs with): whether
;; the most recent kill-ring entry evil produced was linewise, so p/P
;; know whether to paste as whole line(s) or inline text -- paired with
;; `evil--yank-text' (the exact text evil itself last put there) so a
;; native kill/yank command in between (M-w, kill-line, ...) that
;; changes the kill-ring's top without evil's knowledge is detected
;; and treated as charwise rather than trusting a stale flag; see
;; `evil--current-yank-linewise-p'.
(defvar evil--yank-linewise nil)
(defvar evil--yank-text nil)

;; M42: named registers (`"a'-`"z'). Global, like the kill-ring itself
;; (which this sits alongside) -- unlike `evil--local-marks' above,
;; vim's named registers are editor-wide, not per-buffer. Alist of
;; (CHAR . (TEXT . LINEWISE)): unlike the kill-ring/`evil--yank-*' pair,
;; a register entry carries its OWN linewise flag permanently, so a
;; read never needs (or gets) `evil--current-yank-linewise-p''s
;; content-equality staleness guard -- there is no shared, natively
;; clobberable "top of the register" the way the kill-ring has. See
;; `evil--set-register' (write) and `evil--do-paste' (read).
(defvar evil--registers nil)

;; --- M30: dot-repeat state (`.') ---------------------------------------
;;
;; Global (like the yank state just above, and for the same reason:
;; vim's dot-register is a single, editor-wide "last change", not a
;; per-buffer thing) -- see `evil--record-change' and the file header's
;; dot-repeat section for the full design.
(defvar evil--last-change nil
  "A zero-arg function that redoes the most recent buffer-modifying
normal-state command (an operator+motion/text-object combo, one of the
`x X s S D C Y J r ~ p P' simple edits, or an i/a/I/A/o/O insert
session together with the text typed during it) at wherever point
currently is. nil until the first such command runs. Never set by a
bare motion (`h'/`l'/`f' used with no pending operator, etc.), by
visual-state operators (out of scope for M30 -- see the file header),
or by `undo'/`evil-redo' (vim semantics: `.' never repeats an undo).")

(defvar evil--pending-insert-entry nil
  "Transient hand-off from `evil--record-change' to `evil--operator-
apply''s `change' branch: set to the just-recorded replay thunk
exactly when the operator about to run is `change', nil otherwise
\(`delete'/`yank', or a visual-state change, which never calls
`evil--record-change' at all) -- see `evil--operator-apply' for how
it's consumed (read once, then always cleared, so nothing here ever
survives past the one `evil--operator-apply' call it was set for).")

(defvar evil--insert-start nil
  "Buffer position where the CURRENT dot-repeat-tracked insert
session's typed text begins, or nil when no such session is open --
set by `evil--begin-insert-session' (i/a/I/A/o/O) or `evil--operator-
apply''s `change' branch (cw, ciw, s, S, C, cc, ...), consumed by
`evil--finish-insert-session' (called from `evil-insert-exit'). Global
rather than buffer-local like the rest of this section, since an
insert session started in one buffer never sensibly continues in
another (the insert-map has no buffer-switch bindings) -- kept simple
rather than guarded against that.")

(defvar evil--insert-entry nil
  "Paired with `evil--insert-start': a zero-arg function that redoes
THIS session's entry (an i/a/I/A/o/O command symbol, or an operator+
motion/text-object/simple-edit replay thunk that ends by entering
insert state) -- see `evil--finish-insert-session'.")

(defvar evil--last-insert-pos nil
  "M45 `gi': buffer position where the MOST RECENT insert session ended
(point when `evil-insert-exit' ran), or nil before the first one.
Buffer-local. Set in `evil--finish-insert-session', UNCONDITIONALLY and
BEFORE that function nils out `evil--insert-start' -- unlike
`evil--insert-start' itself (which only tracks dot-repeat and is nil
whenever the ending session wasn't one of those), this is vim's own
`gi' target: \"wherever insert mode was last left\", with no dot-repeat
gating.")

(defvar evil--normal-map (make-sparse-keymap))
(defvar evil--insert-map (make-sparse-keymap))
(defvar evil--visual-map (make-sparse-keymap))
(defvar evil--op-pending-map (make-sparse-keymap))

;; --- Count handling --------------------------------------------------

(defun evil--refresh-op-pending-fresh ()
  "Re-arm `evil--op-pending-fresh' (see its docstring). Call this from
any command that legitimately leaves state in `operator-pending'
without itself resolving the operator -- count-digit accumulation
\(`evil--digit') and arming a capture for f/F/t/T
\(`evil-find-char-forward' and friends) both need it."
  (when (eq evil--state 'operator-pending)
    (setq-local evil--op-pending-fresh t)))

(defun evil--digit (d)
  (setq-local evil--count (+ (* (or evil--count 0) 10) d))
  (setq-local evil--count-fresh t)
  ;; Accumulating a second (or third...) count digit while an operator
  ;; is pending (e.g. the "2" of "d2w") is itself a complete, ordinary
  ;; command that deliberately leaves the state in `operator-pending' --
  ;; see `evil--refresh-op-pending-fresh'.
  (evil--refresh-op-pending-fresh))

(defun evil--total-count ()
  "Combined operator-count x motion-count (each default 1), consuming
\(resetting to nil) both. The one place every count-consuming command
in this file reads its effective repeat count from -- always call this
exactly once per command, even when the result is discarded, so a
count typed before a command that doesn't use one (e.g. a text object)
never leaks into whatever runs next."
  (let ((c (* (or evil--op-count 1) (or evil--count 1))))
    (setq-local evil--count nil)
    (setq-local evil--op-count nil)
    c))

;; --- M30: dot-repeat (`.') infrastructure -------------------------------
;;
;; `evil--last-change' (see its docstring, in the Global state section
;; above) is set from many different call sites below, all funneling
;; through the helpers here:
;;
;; - `evil--record-change': the ONE place that actually sets
;;   `evil--last-change'. Called from each motion/text-object/simple-
;;   edit command's own body at the exact point it discovers it is
;;   about to complete an `evil--pending-operator' (never from a bare,
;;   operator-less motion) or, for the operator-less simple edits
;;   (x/X/D/J/r/~/p/P), unconditionally (they're always a change) --
;;   EXCEPT when the operator in question is `yank': real vim's `.'
;;   never repeats a yank (Y/yy/yw/y$/yiw/... are motions with respect
;;   to dot-repeat, not changes), so `evil--record-change' itself
;;   refuses to record while `evil--pending-operator' is `yank',
;;   uniformly, no matter which of the four op-run-style functions
;;   below (or a bare `evil-yank-line') called it.
;; - `evil--replay-thunk': the shape almost every call site wants --
;;   "redo command CMD with pending-operator OP (whatever it was RIGHT
;;   NOW, captured eagerly since `evil--pending-operator' is reset to
;;   nil long before `.' ever runs the thunk) and effective count N".
;;   CMD is always a plain symbol naming an existing command -- since
;;   each function already knows its own name lexically (it's writing
;;   the call site), no runtime "what command is this" introspection
;;   is needed (and this codebase has none to offer -- `this-command'
;;   has no elisp accessor, same as `last-command'; see the redo
;;   section above).
;; - `evil--replay-goto-thunk': `gg'/`G' need one bit more than
;;   `evil--replay-thunk' offers -- see its own docstring.
;;
;; f/F/t/T are the one motion family that needs a THIRD shape
;; (`evil--replay-find-thunk', defined alongside them below instead of
;; here): the character they search for was captured via
;; `capture-next-key', not typed as part of a plain command invocation,
;; so replaying has nothing to re-arm a capture and wait for -- it must
;; reuse the remembered character directly.
(defun evil--record-change (thunk)
  "Set THUNK (a zero-arg function) as the dot-repeatable
`evil--last-change'. When the operator about to run is `change', also
arm `evil--pending-insert-entry' with THUNK so `evil--operator-apply''s
`change' branch can start a dot-repeat-tracked insert session (see
`evil--begin-insert-session' and the file header's dot-repeat section)
-- the text about to be typed then gets folded into THUNK's replay once
ESC lands (`evil--finish-insert-session').

M30 review fix (issue 4): a no-op while the operator about to run is
`yank' -- Y/yy/yw/y$/yiw/etc. must never become the dot-repeat target
(real vim's `.' never replays a yank), and centralizing the check HERE
rather than at each of the four op-run-style call sites (`evil--op-
run'/`evil--op-run-linewise'/`evil--op-run-object'/`evil--op-current-
lines') guarantees it can't be missed at a fifth one added later.

M42: when a named-register prefix is armed (`evil--pending-register'
non-nil -- read here, BEFORE whichever of `evil--maybe-write-register'/
`evil--do-paste' downstream actually consumes it), THUNK is wrapped to
re-arm the SAME register name at replay time -- the dot-repeat
equivalent of `evil--replay-goto-thunk''s count-freezing precedent
\(`\"add' then `.' deletes the NEXT line into register a again, not the
unnamed one). Deliberately freezes only the NAME, not the register's
current content: re-running THUNK re-invokes the original command,
which re-reads `evil--registers' fresh on its own, matching
`evil-paste-after''s own \"replay reads current content, not a frozen
snapshot\" doc note (see `evil--do-paste')."
  (unless (eq evil--pending-operator 'yank)
    (let ((wrapped
           (if evil--pending-register
               (let ((reg evil--pending-register))
                 (lambda ()
                   (setq-local evil--pending-register reg)
                   (funcall thunk)))
             thunk)))
      (setq evil--last-change wrapped)
      (when (eq evil--pending-operator 'change)
        (setq evil--pending-insert-entry wrapped)))))

(defun evil--replay-thunk (cmd n)
  "Build a zero-arg thunk that redoes CMD (a command symbol) as it
would run right now: the CURRENT `evil--pending-operator' (captured
eagerly -- see the section comment above) and effective count N."
  (let ((op evil--pending-operator))
    (lambda ()
      (setq-local evil--pending-operator op)
      (setq-local evil--count n)
      (setq-local evil--op-count nil)
      (funcall cmd))))

(defun evil--replay-goto-thunk (cmd has-count n)
  "Like `evil--replay-thunk', but HAS-COUNT (the pending count as it
was BEFORE `evil--total-count' consumed it, i.e. before N was derived
from it) decides whether a count is reproduced AT ALL -- `gg'/`G' with
no count behave differently from `1gg'/`1G' (first line vs. last line
for `G' specifically, see `evil-goto-line-or-last'), a distinction
`evil--total-count''s always-a-number return value alone can't
capture: blindly seeding `evil--count' with N even when NO count was
originally given would make the replay look like an explicit count was
typed, silently changing bare `G''s \"last line\" into \"line N\"."
  (let ((op evil--pending-operator))
    (lambda ()
      (setq-local evil--pending-operator op)
      (setq-local evil--count (and has-count n))
      (setq-local evil--op-count nil)
      (funcall cmd))))

;; --- Character classification / word motions --------------------------

(defun evil--blank-p (c)
  (and c (or (= c ?\s) (= c ?\t) (= c ?\n))))

(defun evil--word-char-p (c)
  (and c (or (and (>= c ?a) (<= c ?z))
             (and (>= c ?A) (<= c ?Z))
             (and (>= c ?0) (<= c ?9))
             (= c ?_))))

(defun evil--class-at (pos big)
  "Class of the character at buffer position POS: always 'space at or
past `point-max' (a boundary, so the word-motion loops below terminate
cleanly there); otherwise 'word for any non-blank when BIG (the
2-class blank/non-blank model behind W/B/E), else the 3-class
word/punct/space model (w/b/e) using `evil--word-char-p'."
  (if (>= pos (point-max))
      'space
    (let ((c (char-after pos)))
      (cond
        ((evil--blank-p c) 'space)
        (big 'word)
        ((evil--word-char-p c) 'word)
        (t 'punct)))))

(defun evil--fwd-word-pos (count big)
  "Target position for `w'/`W' from point, repeated COUNT times. Known
simplification (documented in the file header): blank-line runs are
just whitespace to this scan, unlike real vim's own paragraph-boundary
special case for `w'."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (let ((cls (evil--class-at pos big)))
        (unless (eq cls 'space)
          (while (and (< pos (point-max)) (eq (evil--class-at pos big) cls))
            (setq pos (1+ pos)))))
      (while (and (< pos (point-max)) (eq (evil--class-at pos big) 'space))
        (setq pos (1+ pos))))
    pos))

(defun evil--bwd-word-pos (count big)
  "Target position for `b'/`B' from point, repeated COUNT times."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (when (> pos (point-min)) (setq pos (1- pos)))
      (while (and (> pos (point-min)) (eq (evil--class-at pos big) 'space))
        (setq pos (1- pos)))
      (when (> pos (point-min))
        (let ((cls (evil--class-at pos big)))
          (while (and (> pos (point-min)) (eq (evil--class-at (1- pos) big) cls))
            (setq pos (1- pos))))))
    pos))

(defun evil--end-word-pos (count big)
  "Target position for `e'/`E' from point, repeated COUNT times --
lands ON the last character of the word (an inclusive-style landing:
see `evil--op-run' callers below)."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (when (< pos (point-max)) (setq pos (1+ pos)))
      (while (and (< pos (point-max)) (eq (evil--class-at pos big) 'space))
        (setq pos (1+ pos)))
      (when (< pos (point-max))
        (let ((cls (evil--class-at pos big)))
          (while (and (< pos (point-max)) (eq (evil--class-at (1+ pos) big) cls))
            (setq pos (1+ pos))))))
    pos))

(defun evil--bwd-end-word-pos (count big)
  "Target position for `ge'/`gE' from point, repeated COUNT times --
lands ON the last character of the PREVIOUS word/WORD run (an
inclusive-style landing, same convention as `e'/`E').

Each iteration is two steps. (1) `evil--class-at POS' -- like every
OTHER motion in this file, not `(1- POS)' -- reads the class of the
character point is CURRENTLY sitting on (straight ahead); when that's
`word'/`punct' (point is inside, or at the very start of, a run), walk
BACKWARD while the character just behind still matches, landing POS at
that run's own start. This step is what makes a `ge' pressed from the
MIDDLE of a word skip the rest of that same word first, rather than
stopping short of it -- skipping it only when point already sits
exactly at its start (the `while' condition is false on the very first
check, so this is a no-op then, correctly leaving POS where it is
instead of also swallowing whatever precedes it).

\(2) Step one further back -- off the run just found (or, if point
started ON blank space, into it) -- then skip any further run of
blanks by checking `evil--class-at POS' directly (not `(1- POS)'
here either): the position where THAT stops already IS the answer,
landing ON the previous run's last character, not one further from it.

An earlier version of this function determined the run class from
`evil--class-at (1- POS)' (the character BEHIND point) instead of
`evil--class-at POS' (the character point is ON) -- indistinguishable
from this version whenever point sits mid-run or on blank space, but
wrong the moment point sits EXACTLY at the start of a run immediately
adjacent to a DIFFERENTLY-classed run with no space between (e.g.
point on the `b' of `bar' in \"foo.bar\", `.' just behind it): that
version treated the single-character `.' run as \"the current run to
finish escaping\" and skipped straight past it to `foo', when `ge'
here should land ON the `.' itself, one run back, not two. Caught by
hand-simulating several worked examples during M45 review -- see
evil_wave3_tests.rs's `ge`/`gE' punctuation-vs-WORD tests, which pin
exactly this scenario."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (let ((cls (evil--class-at pos big)))
        (when (memq cls '(word punct))
          (while (and (> pos (point-min)) (eq (evil--class-at (1- pos) big) cls))
            (setq pos (1- pos)))))
      (when (> pos (point-min))
        (setq pos (1- pos))
        (while (and (> pos (point-min)) (eq (evil--class-at pos big) 'space))
          (setq pos (1- pos)))))
    pos))

;; --- Paragraph motions (blank-line delimited) --------------------------

(defun evil--line-blank-p (line)
  (save-excursion
    (evil--goto-line line)
    (= (line-beginning-position) (line-end-position))))

(defun evil--goto-line (n)
  (goto-char (point-min))
  (forward-line (1- n)))

(defun evil--total-lines ()
  (save-excursion (goto-char (point-max)) (line-number-at-pos)))

(defun evil--pos-paragraph-forward (count)
  "Target position for `}' from point, repeated COUNT times: the start
of the next blank line after point's line, or `point-max'. Simplified
\(documented in the file header): doesn't collapse a whole run of
blank lines into one hop the way real vim does."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (setq pos
            (save-excursion
              (goto-char pos)
              (let ((line (1+ (line-number-at-pos)))
                    (total (evil--total-lines)))
                (while (and (<= line total) (not (evil--line-blank-p line)))
                  (setq line (1+ line)))
                (if (> line total)
                    (point-max)
                  (progn (evil--goto-line line) (point)))))))
    pos))

(defun evil--pos-paragraph-backward (count)
  "Target position for `{' from point, repeated COUNT times."
  (let ((pos (point)))
    (dotimes (i (max 1 count))
      (setq pos
            (save-excursion
              (goto-char pos)
              (let ((line (1- (line-number-at-pos))))
                (while (and (>= line 1) (not (evil--line-blank-p line)))
                  (setq line (1- line)))
                (if (< line 1)
                    (point-min)
                  (progn (evil--goto-line line) (point)))))))
    pos))

;; --- First-non-blank / end-of-line (with count) ------------------------

(defun evil--pos-first-non-blank ()
  (let ((pos (line-beginning-position)) (eol (line-end-position)))
    (while (and (< pos eol) (memq (char-after pos) '(?\s ?\t)))
      (setq pos (1+ pos)))
    pos))

(defun evil--pos-first-non-blank-or-bol ()
  "Like `evil--pos-first-non-blank', but a line with NO non-blank
character at all (whitespace-only or fully empty) lands at
`line-beginning-position' instead of `line-end-position' -- vim's own
convention for where a blank line's \"first non-blank\" target lands
\(`evil-ret', M36 review fix, below -- the existing callers of
`evil--pos-first-non-blank' itself, gg/G/^, never needed this
distinction in practice, so it isn't retrofitted onto that shared,
already-tested helper)."
  (let ((pos (evil--pos-first-non-blank)))
    (if (= pos (line-end-position)) (line-beginning-position) pos)))

(defun evil--pos-eol (n)
  "End of the current line, or (with N > 1) N-1 lines further down --
vim's `N$' semantics."
  (save-excursion
    (when (> n 1) (forward-line (1- n)))
    (line-end-position)))

(defun evil--pos-eol-non-blank (n)
  "M45 `g_': position of the LAST non-blank character on the current
line, or (with N > 1) N-1 lines further down -- same count shape as
`evil--pos-eol' ($'s own helper), but scans backward from `line-end-
position' over trailing spaces/tabs and lands ON the character (like
`evil--end-word-pos''s landing convention) rather than one past it the
way `evil--pos-eol' does. A line with no non-blank character at all
(blank or empty) lands at `line-beginning-position', same as `evil--
pos-first-non-blank-or-bol''s empty-line fallback."
  (save-excursion
    (when (> n 1) (forward-line (1- n)))
    (let ((pos (line-end-position)) (bol (line-beginning-position)))
      (while (and (> pos bol) (memq (char-before pos) '(?\s ?\t)))
        (setq pos (1- pos)))
      (max bol (1- pos)))))

;; --- f/F/t/T character search + `;'/`,' repeat -------------------------

(defun evil--find-one (cmd char from)
  "One hop of CMD ('f/'t/'F/'T) for CHAR, scanning from just past/before
FROM within the current line only. Landing position matches vim's own:
ON the character for f/F, one-before/-after it for t/T. nil if not
found before eol/bol."
  (cond
    ((eq cmd 'f)
     (let ((pos (1+ from)) (eol (save-excursion (goto-char from) (line-end-position))))
       (catch 'evil--found
         (while (< pos eol)
           (when (= (char-after pos) char) (throw 'evil--found pos))
           (setq pos (1+ pos)))
         nil)))
    ((eq cmd 't)
     (let ((hit (evil--find-one 'f char from)))
       (and hit (1- hit))))
    ((eq cmd 'F)
     (let ((pos (1- from)) (bol (save-excursion (goto-char from) (line-beginning-position))))
       (catch 'evil--found
         (while (>= pos bol)
           (when (= (char-after pos) char) (throw 'evil--found pos))
           (setq pos (1- pos)))
         nil)))
    ((eq cmd 'T)
     (let ((hit (evil--find-one 'F char from)))
       (and hit (1+ hit))))))

(defun evil--find-target-from (cmd char count from)
  (let ((pos from))
    (catch 'evil--find-done
      (dotimes (i (max 1 count))
        (let ((next (evil--find-one cmd char pos)))
          (if next
              (setq pos next)
            (throw 'evil--find-done nil))))
      pos)))

(defun evil--find-target (cmd char count)
  (evil--find-target-from cmd char count (point)))

(defun evil--reverse-find-cmd (cmd)
  (cond ((eq cmd 'f) 'F) ((eq cmd 'F) 'f)
        ((eq cmd 't) 'T) ((eq cmd 'T) 't)))

(defun evil--repeat-find-target (cmd char count)
  "Like `evil--find-target' but nudges the starting position one char
in CMD's search direction first when CMD is `t'/`T' -- otherwise a
repeat of a till-motion would immediately re-find the same adjacent
occurrence it already landed next to (vim's own `;'-after-`t' rule)."
  (let ((from (point)))
    (cond ((eq cmd 't) (setq from (1+ from)))
          ((eq cmd 'T) (setq from (1- from))))
    (evil--find-target-from cmd char count from)))

(defun evil--replay-find-thunk (cmd char n)
  "Like `evil--replay-thunk' but for f/F/t/T as an operator target: CHAR
was captured via `capture-next-key' when the change was first made, so
replaying must reuse it directly rather than re-arming a capture and
waiting for another keystroke -- there is no more input coming."
  (let ((op evil--pending-operator))
    (lambda ()
      (setq-local evil--pending-operator op)
      (let ((target (evil--find-target-from cmd char n (point))))
        (if target
            (evil--op-run target 'inclusive)
          (message "%c not found" char))))))

(defun evil--handle-find (cmd k n)
  "The `capture-next-key' callback for f/F/t/T: K is the captured key
\(an integer, or 27 for ESC -- see `capture-next-key''s doc; a named
key arrives as a symbol, also treated as cancel here). N is the count
the arm command (`evil-find-char-forward' and friends) already
consumed EAGERLY, before arming this capture -- see their shared
docstring for why reading it LAZILY, from inside this callback, would
be wrong (the same hazard `evil--handle-execute-macro' documents for
`@')."
  (cond
    ((not (integerp k)) (evil--cancel-op))
    ((= k 27) (evil--cancel-op))
    (t
     (let* ((target (evil--find-target cmd k n)))
       (if (not target)
           (if evil--pending-operator (evil--cancel-op) (message "%c not found" k))
         (progn
           (setq-local evil--last-find-cmd cmd)
           (setq-local evil--last-find-char k)
           (if evil--pending-operator
               (evil--op-run target 'inclusive (evil--replay-find-thunk cmd k n))
             (goto-char target))))))))

;; M29 review fix (high severity): arming a capture is itself a
;; complete command -- its own completion fires post-command-hook just
;; like `evil--digit''s does, so it must re-arm `evil--op-pending-fresh'
;; the same way, or the very next post-command-hook firing (right after
;; f/F/t/T is pressed, before the target character even arrives) sees
;; state still `operator-pending' with a now-stale fresh flag and
;; cancels the operator -- by the time the captured target character
;; shows up, `evil--pending-operator' is already nil and e.g. `dfX'
;; silently degrades into a bare motion. (The captured key's own
;; delivery, in `commands.rs' `handle_key', bypasses the command loop
;; entirely and never fires post-command-hook, so no further refresh is
;; needed once the capture is armed.)
;;
;; M44 fix: `evil--total-count' is now consumed HERE, eagerly, rather
;; than later inside `evil--handle-find' -- the same move
;; `evil-replace-char' (`r') and `evil-execute-macro' (`@') already
;; make for the identical reason (see their own docstrings). Arming a
;; capture still finishes the command normally, so this command's own
;; `evil--post-command' pass runs the count-staleness cleanup before
;; the target character ever arrives; left unread, that cleanup would
;; clear `evil--count' out from under `evil--handle-find' for BOTH a
;; bare count (e.g. "3fX", no operator involved at all) and a count
;; typed after an operator already started operator-pending state
;; (e.g. "d2fX" -- `evil--digit' writes every digit into `evil--count'
;; regardless of state, so it's just as exposed as the bare case; only
;; a count typed BEFORE the operator, e.g. "2dfX", is safe, since
;; `evil--op-start' moves it into `evil--op-count' -- untouched by this
;; cleanup -- before "f" ever runs). Reading eagerly here, before
;; `capture-next-key' arms, catches the count while both
;; `evil--count' and `evil--op-count' are still whatever they were the
;; instant this command started, exactly like `evil--total-count''s own
;; contract promises.
(defun evil-find-char-forward ()
  (interactive)
  (evil--refresh-op-pending-fresh)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--handle-find 'f k n)))))
(defun evil-find-char-backward ()
  (interactive)
  (evil--refresh-op-pending-fresh)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--handle-find 'F k n)))))
(defun evil-till-char-forward ()
  (interactive)
  (evil--refresh-op-pending-fresh)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--handle-find 't k n)))))
(defun evil-till-char-backward ()
  (interactive)
  (evil--refresh-op-pending-fresh)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--handle-find 'T k n)))))

(defun evil--do-repeat-find (cmd self)
  "SELF is the top-level command (`evil-repeat-find' or `evil-repeat-
find-reverse') to re-invoke on `.' -- unlike f/F/t/T themselves,
replaying `;'/`,' by just re-running SELF is correct and simple: they
read `evil--last-find-cmd'/`evil--last-find-char' (persistent, buffer-
local state) fresh each time they run, the same as the ORIGINAL
invocation did, rather than needing a captured character threaded
through (best-effort if an intervening bare `f'/`F'/`t'/`T' changed
that remembered state without itself becoming a new `evil--last-
change' -- a bare find is a pure motion, per vim semantics -- see the
file header)."
  (if (not cmd)
      (message "No previous find")
    (let* ((char evil--last-find-char)
           (n (evil--total-count))
           (target (evil--repeat-find-target cmd char n)))
      (if (not target)
          (message "%c not found" char)
        (if evil--pending-operator
            (evil--op-run target 'inclusive (evil--replay-thunk self n))
          (goto-char target))))))

(defun evil-repeat-find ()
  (interactive)
  (evil--do-repeat-find evil--last-find-cmd 'evil-repeat-find))

(defun evil-repeat-find-reverse ()
  (interactive)
  (evil--do-repeat-find (evil--reverse-find-cmd evil--last-find-cmd) 'evil-repeat-find-reverse))

;; --- M42: marks (`m' / `` ` '' / `'') -----------------------------------
;;
;; `m' arms a `capture-next-key' callback for the mark's NAME (a-z),
;; the same one-shot-capture idiom as `r'/`f'/`t' above; `` ` ''/`''
;; are motions -- bound via `evil--bind-motion' into all three keymaps,
;; same as `f'/`t' themselves -- that ALSO capture a name character
;; before they can resolve a target, so `` d`a ''/`d'a' both work as
;; operator+motion combos the identical way `df<char>' does. Storage
;; is `evil--local-marks' (Global state section, above): buffer-local,
;; so marks in one buffer are invisible to another and vanish for free
;; when the buffer is killed -- no explicit cleanup needed. `evil--op-
;; run'/`evil--op-run-linewise'/`evil--cancel-op' (below, Operator
;; engine / Text objects sections) are forward references here, same
;; as the Text objects section just below already does for
;; `evil--operator-apply' -- this file's own established style, not
;; something new (symbols resolve at dispatch time, see the file
;; header's dabbrev.el load-order note).

(defun evil--set-local-mark (ch marker)
  "Upsert (CH . MARKER) into buffer-local `evil--local-marks' -- same
CH overwrites the previous marker (real vim's `m' semantics)."
  (let ((entry (assq ch evil--local-marks)))
    (if entry
        (setcdr entry marker)
      (setq-local evil--local-marks (cons (cons ch marker) evil--local-marks)))))

(defun evil--handle-set-mark (k)
  "The `capture-next-key' callback for `m'. K -- see `evil--handle-find'
for the non-integer/27 (ESC) cancel convention this mirrors. `(point-
marker)' is a REGISTERED marker (pushed onto the current buffer's
`markers' vec, buffer.rs) -- it rides out edits before it exactly like
any other marker, which is the entire point: a mark set BEFORE text is
later inserted ahead of it must still land back on the SAME character."
  (cond
    ((not (integerp k)) nil)
    ((= k 27) nil)
    ((and (>= k ?a) (<= k ?z))
     (evil--set-local-mark k (point-marker)))
    (t (message "Marks must be a-z"))))

(defun evil-set-mark ()  ; m
  (interactive)
  (capture-next-key (lambda (k) (evil--handle-set-mark k))))

(defun evil--replay-mark-thunk (linewise k)
  "Like `evil--replay-find-thunk' but for a `` ` ''/`'' operator target:
the mark name K is captured eagerly, same reasoning as f/F/t/T's own
replay thunk -- there is no further input for `.' to wait for. Unlike
K, the mark's POSITION is deliberately re-looked-up fresh at replay
time (marks are live, editable positions, not a static line-relative
offset the way f/t's target character is) -- see `evil--handle-goto-mark'."
  (let ((op evil--pending-operator))
    (lambda ()
      (setq-local evil--pending-operator op)
      (let ((entry (assq k evil--local-marks)))
        (if (not entry)
            (message "Mark not set: %c" k)
          (let ((pos (marker-position (cdr entry))))
            (if linewise
                (evil--op-run-linewise pos)
              (evil--op-run pos 'exclusive))))))))

(defun evil--handle-goto-mark (linewise k)
  "The `capture-next-key' callback shared by `` ` '' (LINEWISE nil) and
`'' (LINEWISE t). K -- see `evil--handle-set-mark'. Unlike `evil--handle-
find''s target-not-found branch (which stays silent when an operator is
pending), a missing mark ALWAYS messages -- the M42 plan calls this out
explicitly, unlike upstream `f'/`t' semantics."
  (cond
    ((not (integerp k)) (evil--cancel-op))
    ((= k 27) (evil--cancel-op))
    (t
     (let ((entry (assq k evil--local-marks)))
       (if (not entry)
           (progn
             (message "Mark not set: %c" k)
             (when evil--pending-operator (evil--cancel-op)))
         (let ((pos (marker-position (cdr entry))))
           (if evil--pending-operator
               (if linewise
                   (evil--op-run-linewise pos (evil--replay-mark-thunk linewise k))
                 (evil--op-run pos 'exclusive (evil--replay-mark-thunk linewise k)))
             (goto-char (if linewise
                            (save-excursion (goto-char pos) (evil--pos-first-non-blank))
                          pos)))))))))

(defun evil-goto-mark-exact ()  ; `
  (interactive)
  (evil--refresh-op-pending-fresh)
  (capture-next-key (lambda (k) (evil--handle-goto-mark nil k))))

(defun evil-goto-mark-line ()  ; '
  (interactive)
  (evil--refresh-op-pending-fresh)
  (capture-next-key (lambda (k) (evil--handle-goto-mark t k))))

;; --- Text objects (operator-pending only: i/a prefix) -------------------

(defun evil--word-run (pos)
  "(BEG . END) of the word/punct/space run containing POS -- the
3-class model, so `iw' on whitespace selects the whitespace run like
real vim."
  (let ((cls (evil--class-at pos nil)) (s pos) (e pos))
    (while (and (> s (point-min)) (eq (evil--class-at (1- s) nil) cls))
      (setq s (1- s)))
    (while (and (< e (point-max)) (eq (evil--class-at e nil) cls))
      (setq e (1+ e)))
    (cons s e)))

(defun evil--op-run-object (beg end &optional replay)
  "Like `evil--op-run' but for a text object already resolved to an
absolute [BEG,END) span. REPLAY, when given, is recorded as the dot-
repeat target (see `evil--record-change') BEFORE the operator mutates
the buffer -- text objects don't consult a pending count (file header),
so callers always pass `nil' as the count to `evil--replay-thunk'."
  (when replay (evil--record-change replay))
  (evil--operator-apply beg end 0 nil))

(defun evil-op-iw ()
  (interactive)
  (evil--total-count)
  (let ((r (evil--word-run (point))))
    (evil--op-run-object (car r) (cdr r) (evil--replay-thunk 'evil-op-iw nil))))

(defun evil-op-aw ()
  (interactive)
  (evil--total-count)
  (let* ((r (evil--word-run (point))) (beg (car r)) (end (cdr r)))
    (cond
      ;; Prefer trailing whitespace; fall back to leading (vim's rule
      ;; for a word at end-of-line).
      ((and (< end (point-max)) (eq (evil--class-at end nil) 'space))
       (while (and (< end (point-max)) (eq (evil--class-at end nil) 'space))
         (setq end (1+ end))))
      ((and (> beg (point-min)) (eq (evil--class-at (1- beg) nil) 'space))
       (while (and (> beg (point-min)) (eq (evil--class-at (1- beg) nil) 'space))
         (setq beg (1- beg)))))
    (evil--op-run-object beg end (evil--replay-thunk 'evil-op-aw nil))))

(defun evil--quote-pair (qc)
  "(BEG . END) -- absolute buffer positions of the two QC quote
characters -- for the first quoted span on the CURRENT LINE that
touches or comes after point (vim's same-line quote text object
search: quotes don't stretch across lines here). nil if none."
  (let ((eol (line-end-position)) (pos (line-beginning-position)) (positions nil))
    (while (< pos eol)
      (when (= (char-after pos) qc)
        (setq positions (cons pos positions)))
      (setq pos (1+ pos)))
    (setq positions (nreverse positions))
    (let ((pairs nil))
      (while (and positions (cdr positions))
        (setq pairs (cons (cons (car positions) (cadr positions)) pairs))
        (setq positions (cddr positions)))
      (setq pairs (nreverse pairs))
      (let ((found nil) (rest pairs))
        (while (and rest (not found))
          (when (>= (cdr (car rest)) (point))
            (setq found (car rest)))
          (setq rest (cdr rest)))
        found))))

(defun evil--cancel-op ()
  (interactive)
  (setq-local evil--pending-operator nil)
  (setq-local evil--count nil)
  (setq-local evil--op-count nil)
  ;; M42: abandon a pending named-register prefix along with the
  ;; operator itself -- a compound like `"ad<ESC>'/`"ad<C-g>' abandons
  ;; the WHOLE thing, register included, not just the delete.
  (setq-local evil--pending-register nil)
  (evil--set-state 'normal))

(defun evil-op-i-quote ()
  (interactive)
  (evil--total-count)
  (let ((p (evil--quote-pair ?\")))
    (if p
        (evil--op-run-object (1+ (car p)) (cdr p) (evil--replay-thunk 'evil-op-i-quote nil))
      (progn (message "No object found") (evil--cancel-op)))))

(defun evil-op-a-quote ()
  (interactive)
  (evil--total-count)
  (let ((p (evil--quote-pair ?\")))
    (if p
        (evil--op-run-object (car p) (1+ (cdr p)) (evil--replay-thunk 'evil-op-a-quote nil))
      (progn (message "No object found") (evil--cancel-op)))))

(defun evil--scan-forward-match (open close from)
  "First position >= FROM (searching the whole buffer, so brackets can
stretch across lines) holding CLOSE that balances against nested
OPEN/CLOSE seen along the way."
  (let ((pos from) (depth 0) (found nil))
    (while (and (< pos (point-max)) (not found))
      (let ((c (char-after pos)))
        (when c
          (cond
            ((= c close) (if (= depth 0) (setq found pos) (setq depth (1- depth))))
            ((= c open) (setq depth (1+ depth))))))
      (setq pos (1+ pos)))
    found))

(defun evil--scan-backward-match (open close from)
  (let ((pos from) (depth 0) (found nil))
    (while (and (>= pos (point-min)) (not found))
      (let ((c (char-after pos)))
        (when c
          (cond
            ((= c open) (if (= depth 0) (setq found pos) (setq depth (1- depth))))
            ((= c close) (setq depth (1+ depth))))))
      (setq pos (1- pos)))
    found))

(defun evil--bracket-pair (open close)
  "(BEG . END) -- absolute positions of the OPEN/CLOSE characters of
the balanced pair enclosing point (or, if point sits exactly on one of
them, that pair). Scans the whole buffer -- multi-line blocks work.
nil if unbalanced/not found."
  (let (ob cb)
    (cond
      ((eq (char-after (point)) open)
       (setq ob (point))
       (setq cb (evil--scan-forward-match open close (1+ (point)))))
      ((eq (char-after (point)) close)
       (setq cb (point))
       (setq ob (evil--scan-backward-match open close (1- (point)))))
      (t
       (setq ob (evil--scan-backward-match open close (1- (point))))
       (when ob
         (setq cb (evil--scan-forward-match open close (1+ ob))))))
    (when (and ob cb) (cons ob cb))))

(defmacro evil--def-bracket-object (iname aname open close)
  "Define text-object commands INAME/ANAME (i<x>/a<x>) for the OPEN/
CLOSE bracket pair -- shared shape for (), {}, []."
  `(progn
     (defun ,iname ()
       (interactive)
       (evil--total-count)
       (let ((p (evil--bracket-pair ,open ,close)))
         (if p
             (evil--op-run-object (1+ (car p)) (cdr p) (evil--replay-thunk ',iname nil))
           (progn (message "No object found") (evil--cancel-op)))))
     (defun ,aname ()
       (interactive)
       (evil--total-count)
       (let ((p (evil--bracket-pair ,open ,close)))
         (if p
             (evil--op-run-object (car p) (1+ (cdr p)) (evil--replay-thunk ',aname nil))
           (progn (message "No object found") (evil--cancel-op)))))))

(evil--def-bracket-object evil-op-i-paren evil-op-a-paren ?\( ?\))
(evil--def-bracket-object evil-op-i-brace evil-op-a-brace ?{ ?})
(evil--def-bracket-object evil-op-i-bracket evil-op-a-bracket ?\[ ?\])

;; --- State machine -----------------------------------------------------

(defun evil--tag-for-state (state)
  (cond
    ((eq state 'normal) "<N> ")
    ((eq state 'insert) "<I> ")
    ((eq state 'visual) (if (eq evil--visual-type 'line) "<V-LINE> " "<V> "))
    ((eq state 'operator-pending) "<O> ")
    (t nil)))

(defun evil--echo-for-state (state)
  "The vim-style `-- INSERT --'/`-- VISUAL --'/`-- VISUAL LINE --' echo-
area indicator for STATE, or nil for states that don't show one
\(normal/operator-pending/emacs) -- see `evil--set-state', which stores
this in the CURRENT buffer's `echo-area-fallback' (redisplay.rs's
lowest-priority echo-row layer, below a one-shot `editor.echo' message
-- see its own comment there): a real message (e.g. one of dabbrev.el's
\"No dynamic expansion...\") shows OVER this indicator and this
reappears on its own once that message is cleared (every key clears
`editor.echo' first -- see commands.rs's `handle_key'), matching real
vim's own behavior. Deliberately independent of `evil--tag-for-state'
\(the modeline's `<I>'/`<V>' tag): the two coexist, nothing here removes
the modeline indicator."
  (cond
    ((eq state 'insert) "-- INSERT --")
    ((eq state 'visual) (if (eq evil--visual-type 'line) "-- VISUAL LINE --" "-- VISUAL --"))
    (t nil)))

(defun evil--map-for-state (state)
  (cond
    ((eq state 'normal) evil--normal-map)
    ((eq state 'insert) evil--insert-map)
    ((eq state 'visual) evil--visual-map)
    ((eq state 'operator-pending) evil--op-pending-map)
    (t nil)))

(defun evil--set-state (state)
  (setq-local evil--state state)
  (setq-local emulation-keymap (evil--map-for-state state))
  (setq-local mode-line-prefix (evil--tag-for-state state))
  (setq-local echo-area-fallback (evil--echo-for-state state))
  (setq-local evil--op-pending-fresh (eq state 'operator-pending))
  ;; M34: normal/visual/operator-pending are evil's three modal states --
  ;; each owns a small, necessarily-incomplete ASCII-only vim keymap (see
  ;; `evil--map-for-state'), so any OTHER printable character (most
  ;; notably CJK text, but also a merely-unbound ASCII key like `q' --
  ;; the gap the M30 review already flagged for normal state specifically)
  ;; must not fall through to plain self-insert. `insert' and `emacs'
  ;; both want ordinary self-insert back, so both are correctly excluded
  ;; by this `memq' (matching neither leaves `inhibit-self-insert' nil).
  ;; See simple.el's own docstring and commands.rs's `dispatch_key' for
  ;; where this is actually consulted.
  (setq-local inhibit-self-insert (and (memq state '(normal visual operator-pending)) t))
  (evil--refresh-cursor))

(defun evil--refresh-cursor ()
  "Sync the (global) `cursor-type' to the CURRENT buffer's evil state --
correct as long as this runs whenever the current buffer (or its state)
might have changed, since only one buffer is ever current (and
cursor-type is only ever drawn for the selected window/buffer).

The states evil actively manages get concrete GNU values (insert ->
bar, normal/visual/operator-pending -> box); `emacs' state -- dired/
eshell/ielm buffers evil deliberately does not manage -- gets nil,
\"leave the terminal's own cursor alone\". The M32 review caught the
first migration mapping emacs state (and the evil-mode off switch) to
'box: a steady-block DECSCUSR override that silently replaced the
user's own terminal cursor (often a blinking block) everywhere evil
had bowed out -- the very regression the M28 review fix (reset on
clearing the shape) existed to prevent. nil routes through that reset
path, handing the cursor back."
  (setq cursor-type
        (cond ((eq evil--state 'insert) 'bar)
              ((eq evil--state 'emacs) nil)
              (t 'box))))

(defun evil--initial-state-for-mode (mode)
  (if (memq mode evil-emacs-state-modes) 'emacs 'normal))

(defun evil--maybe-init-current-buffer ()
  "If evil-mode is on and the CURRENT buffer has no evil state of its
own yet (created since evil-mode was last (re)enabled), give it one
per `evil-emacs-state-modes'. Cheap and idempotent -- safe to call on
every command (see `evil--post-command')."
  (when (and evil-mode (not (local-variable-p 'evil--state)))
    (evil--set-state (evil--initial-state-for-mode (major-mode-internal-get)))))

(defun evil--post-command ()
  (evil--maybe-init-current-buffer)
  ;; A stray, unbound key while an operator was pending fell through to
  ;; local/global dispatch (possibly self-insert) instead of completing
  ;; or explicitly cancelling it. Every legitimate operator-pending key
  ;; handler transitions state itself before returning, and an
  ;; incomplete multi-key sequence (the "d"/"i" of "d i w") never
  ;; reaches post-command-hook at all (a keymap Prefix hit doesn't run a
  ;; command) -- so landing here *still* in `operator-pending' on a
  ;; SECOND consecutive post-command-hook firing (see
  ;; `evil--op-pending-fresh') only happens on a genuinely unbound key.
  ;; Recover instead of leaving the buffer stuck with a stale mode-line
  ;; tag and the wrong keymap.
  (when (eq evil--state 'operator-pending)
    (if evil--op-pending-fresh
        (setq-local evil--op-pending-fresh nil)
      (evil--cancel-op)))
  ;; M29 review fix: a count left accumulating (e.g. "5" typed with no
  ;; following motion) must not silently survive an unrelated
  ;; intervening command -- otherwise switching buffers and back, then
  ;; typing "3l", would combine into "53l". Same fresh-flag pattern as
  ;; `evil--op-pending-fresh': `evil--digit' sets `evil--count-fresh' on
  ;; the cycle it runs; if a full post-command-hook cycle passes with
  ;; the flag still down (nothing having re-armed it) while a count is
  ;; still sitting there, it's stale.
  (when (and evil--count (not evil--count-fresh))
    (setq-local evil--count nil))
  (setq-local evil--count-fresh nil)
  ;; M42 review fix: a pending named-register prefix is stale (and must
  ;; be dropped) after ANY command except one that leaves an operator
  ;; genuinely in flight or that entered/continues `visual' state --
  ;; see `evil--pending-register''s own docstring (Global state
  ;; section) for the full timeline and why this is intentionally NOT
  ;; the `evil--op-pending-fresh'/`evil--count-fresh' fresh-flag
  ;; pattern.
  (when (and evil--pending-register
             (not evil--pending-operator)
             (not (eq evil--state 'visual)))
    (setq-local evil--pending-register nil))
  (evil--refresh-cursor))

(defun evil--on-keyboard-quit ()
  "keyboard-quit-hook handler: recover evil state after a C-g that
Rust's `handle_key' already reset pending_keys/minibuffer/mark_active
for, but has no direct way to tell evil.el about (see the file
header). Unlike `evil--post-command' (which must NOT touch `visual' --
staying in visual across many keystrokes is the whole point of the
state), a C-g is never a legitimate reason to stay in either
`operator-pending' or `visual'. Insert state: mirrors upstream evil,
where C-g behaves like ESC (back to normal, point moved left one
unless at bol) rather than doing nothing.

M42: a pending named-register prefix (`\"a', not yet consumed by
whatever operator/paste would read it) is abandoned by C-g
UNCONDITIONALLY, regardless of STATE -- the plain `\"a<C-g>' case never
touches `operator-pending'/`visual'/`insert' at all (`\"'/its capture
callback never change state), so neither branch of the `cond' below
would otherwise reach it; the `operator-pending'/`visual' branch below
also clears it via `evil--cancel-op', redundantly but harmlessly, for
the compound `\"ad<C-g>' case."
  (setq-local evil--pending-register nil)
  (cond
    ((memq evil--state '(operator-pending visual)) (evil--cancel-op))
    ((eq evil--state 'insert) (evil-insert-exit))))

;; These three hooks are registered/unregistered from `evil-mode'
;; itself, NOT unconditionally here at load time -- evil.el is always
;; loaded (part of the built-in lisp layer, see lib.rs), but plenty of
;; callers (every test that builds an interpreter via `init_editor'
;; directly, not through `start_session') never turn evil-mode on at
;; all and must see these global hook lists completely untouched (one
;; existing watchdog test asserts `post-command-hook' has exactly the
;; one function IT added). `evil--post-command'/
;; `evil--maybe-init-current-buffer' both already no-op when
;; `evil-mode' is nil, so this isn't load-bearing for correctness with
;; evil-mode on -- it's specifically so evil-mode OFF (the default,
;; see `evil-auto-enable') is truly invisible.
(defun evil--install-hooks ()
  (add-hook 'post-command-hook 'evil--post-command)
  ;; Explicit attachment point for the find-file path (normal-mode/
  ;; find-file-hook, per GNU's set-auto-mode/find-file-hook order) --
  ;; redundant with the post-command-hook safety net above in practice
  ;; (call_command always runs post-command-hook too) but documents
  ;; the specific path the M29 plan calls out; harmless either way
  ;; since `evil--maybe-init-current-buffer' is idempotent.
  (add-hook 'find-file-hook 'evil--maybe-init-current-buffer)
  (add-hook 'keyboard-quit-hook 'evil--on-keyboard-quit))

(defun evil--uninstall-hooks ()
  (remove-hook 'post-command-hook 'evil--post-command)
  (remove-hook 'find-file-hook 'evil--maybe-init-current-buffer)
  (remove-hook 'keyboard-quit-hook 'evil--on-keyboard-quit))

(defun evil-mode (&optional arg)
  "Toggle evil-mode, or turn it on/off with a numeric ARG (nil = toggle,
`t' or a positive number = on, else off) -- (evil-mode 1) / (evil-mode
-1) / (evil-mode)."
  (interactive)
  (let ((was evil-mode))
    (setq evil-mode
          (cond ((null arg) (not evil-mode))
                ((eq arg t) t)
                (t (> arg 0))))
    (when (and evil-mode (not was)) (evil--install-hooks))
    (when (and was (not evil-mode)) (evil--uninstall-hooks)))
  (if evil-mode
      (progn
        (dolist (buf (buffer-list))
          (with-current-buffer buf
            (evil--set-state (evil--initial-state-for-mode (major-mode-internal-get)))))
        ;; M29 review fix (low severity): `cursor-type' is global, and
        ;; the loop above just overwrote it once per buffer in
        ;; `buffer-list' order -- after it exits, `cursor-type' reflects
        ;; whichever buffer happened to be LAST in that list, not
        ;; necessarily the one actually current (each `with-current-buffer'
        ;; restores the selected buffer as current afterward, but doesn't
        ;; itself re-sync the global cursor-type once it's done). One
        ;; more refresh, now that the loop is over and `current-buffer'
        ;; is genuinely back to what it was before this call, fixes it.
        (evil--refresh-cursor))
    (dolist (buf (buffer-list))
      (with-current-buffer buf
        (setq-local evil--state 'emacs)
        (setq-local emulation-keymap nil)
        (setq-local mode-line-prefix nil)
        (setq-local echo-area-fallback nil)
        ;; M34: this loop bypasses `evil--set-state' (it sets `evil--state'
        ;; straight to 'emacs, not through it), so it must clear
        ;; `inhibit-self-insert' itself too -- otherwise a buffer switched
        ;; off mid-normal-state would keep swallowing self-insert forever,
        ;; the exact regression `evil_mode_off_restores_plain_self_insert'
        ;; (evil_tests.rs) guards against.
        (setq-local inhibit-self-insert nil)
        ;; nil, not 'box: turning evil off must hand the cursor back to
        ;; the terminal's own default (the TUI emits a DECSCUSR reset
        ;; when the shape clears), not pin a steady block over it --
        ;; the M32 review caught the 'box version regressing exactly
        ;; the symptom the M28 review fix was for.
        (setq cursor-type nil)))))

;; --- Operator engine -----------------------------------------------------

(defun evil--linewise-range (beg end)
  "Snap [BEG,END) to whole lines for delete/yank: start of BEG's line
through the start of the line after END's line (including its
trailing newline), clamped at `point-max' when END's line has none."
  (let ((s (save-excursion (goto-char beg) (line-beginning-position)))
        (e (save-excursion (goto-char end) (line-end-position))))
    (let ((e2 (save-excursion (goto-char e) (forward-line 1) (point))))
      (cons s (if (> e2 s) e2 (point-max))))))

(defun evil--linewise-change-range (beg end)
  "Like `evil--linewise-range' but for the `change' operator: spans
whole lines WITHOUT the final trailing newline, so exactly one empty
line survives for the replacement text (vim's cc/S convention) instead
of merging with whatever line follows."
  (let ((s (save-excursion (goto-char beg) (line-beginning-position)))
        (e (save-excursion (goto-char end) (line-end-position))))
    (cons s e)))

(defun evil--record-yank (linewise)
  "Record LINEWISE alongside the text just placed at the kill-ring's
top (call this immediately after `kill-region-internal'/
`kill-ring-save-internal') -- see `evil--current-yank-linewise-p' for
why the text is recorded too, not just the flag."
  (setq evil--yank-linewise linewise)
  (setq evil--yank-text (current-kill)))

(defun evil--current-yank-linewise-p ()
  "Whether the CURRENT top of the kill-ring is still the same text evil
itself last put there under `evil--yank-linewise' -- if a native kill/
yank command with no evil awareness at all (M-w, kill-line, ...) ran in
between and changed the kill-ring's top, the flag is stale and
charwise is the honest fallback (Emacs's own kill-ring carries no
linewise concept, so a plain kill-ring-save can never BE linewise).
Compared by content (`equal'), not identity: `current-kill' builds a
fresh string value on every call (see builtins/editing.rs), so nothing
in this codebase can compare kill-ring entries by pointer identity
anyway. This means content that happens to coincide with the last
evil-recorded yank byte-for-byte is indistinguishable from it -- a
known, narrow, and accepted false-negative (see the file header)."
  (and evil--yank-linewise
       (equal (current-kill) evil--yank-text)))

(defun evil--set-register (ch text linewise)
  "Upsert (CH . (TEXT . LINEWISE)) into the global `evil--registers' --
same CH overwrites the previous entry, mirroring `evil--set-local-mark'."
  (let ((entry (assq ch evil--registers)))
    (if entry
        (setcdr entry (cons text linewise))
      (setq evil--registers (cons (cons ch (cons text linewise)) evil--registers)))))

(defun evil--maybe-write-register (linewise)
  "Called from `evil--operator-apply' immediately after each of its
three branches' own kill + `evil--record-yank': if a register was
armed via `\"' (`evil-use-register'), mirror the text just killed into
`evil--registers' TOO -- vim: a named write ALSO updates the unnamed
register/kill-ring, never instead of it, and that unconditional
kill-ring write already happened above this call, in every branch.
Consumes (reads, then clears) `evil--pending-register' either way, so
it never survives past the ONE `evil--operator-apply' call it was
armed for."
  (when evil--pending-register
    (evil--set-register evil--pending-register (current-kill) linewise)
    (setq-local evil--pending-register nil)))

(defun evil--operator-apply (beg end inclusive-adj linewise)
  "Apply `evil--pending-operator' over the region spanned by BEG and
END (either order), adjusted by INCLUSIVE-ADJ (0 or 1, added to the
larger of BEG/END before clamping to `point-max'; ignored when
LINEWISE) or snapped to whole lines when LINEWISE is non-nil. Leaves
point at the start of the (post-adjustment) region for delete/change;
yank leaves point wherever the caller left it (vim: operator-pending
`y' never moves point; visual `y' moves it to the selection start --
different enough between callers that this function deliberately
doesn't decide it, see `evil--op-run' vs `evil--visual-apply').
Resets `evil--pending-operator'; switches state to `insert' for
`change', `normal' otherwise. M30: also the single place that consumes
`evil--pending-insert-entry' (see its docstring) -- captured into a
local and the global cleared UNCONDITIONALLY up front, before the
branches below even run, so it never survives past this one call
regardless of which operator this turns out to be (a `delete'/`yank',
or a VISUAL-state `change' -- `evil--visual-apply' never calls
`evil--record-change', so this is always nil for that caller -- simply
leaves it captured-but-unused)."
  (let (beg2 end2 (op evil--pending-operator)
        (insert-entry evil--pending-insert-entry))
    (setq evil--pending-insert-entry nil)
    (if linewise
        (let ((range (if (eq op 'change)
                          (evil--linewise-change-range (min beg end) (max beg end))
                        (evil--linewise-range (min beg end) (max beg end)))))
          (setq beg2 (car range))
          (setq end2 (cdr range)))
      (progn
        (setq beg2 (min beg end))
        (setq end2 (min (+ (max beg end) inclusive-adj) (point-max)))))
    (setq-local evil--pending-operator nil)
    (cond
      ((eq op 'yank)
       (kill-ring-save-internal beg2 end2)
       (evil--record-yank linewise)
       (evil--maybe-write-register linewise)
       (evil--set-state 'normal))
      ((eq op 'change)
       (kill-region-internal beg2 end2)
       (evil--record-yank linewise)
       (evil--maybe-write-register linewise)
       (goto-char beg2)
       ;; M30: merge this delete with the text about to be typed into
       ;; ONE undo group (real vim's `cw foo<ESC>' is a single `u' away
       ;; from "hello world" again, not two) -- see
       ;; `undo-amalgamate-boundary''s docstring (editing.rs) for the
       ;; suppression mechanism this leans on.
       (undo-amalgamate-boundary)
       (when insert-entry (evil--begin-insert-session insert-entry))
       (evil--set-state 'insert))
      ((memq op '(toggle-case downcase upcase))
       ;; M45 `g~'/`gu'/`gU': vim's case operators replace the region IN
       ;; PLACE -- unlike every other branch above, they never touch the
       ;; kill-ring or write a named register (real vim: `guw' doesn't
       ;; change what `p' would paste). Still must CONSUME a pending
       ;; named-register prefix (`"aguw') exactly like `evil--maybe-
       ;; write-register' does for the other three branches -- the
       ;; invariant is "an armed register prefix is always consumed by
       ;; the next operator-apply, written or not" (see `evil--pending-
       ;; register''s own docstring); here it's simply discarded instead
       ;; of written, since vim case ops don't have a register form at
       ;; all.
       (let* ((text (buffer-substring beg2 end2))
              (new (cond
                     ((eq op 'upcase) (upcase text))
                     ((eq op 'downcase) (downcase text))
                     (t (evil--case-toggle-string text)))))
         (delete-region beg2 end2)
         (goto-char beg2)
         (insert new)
         (goto-char beg2))
       (setq-local evil--pending-register nil)
       (evil--set-state 'normal))
      (t
       (kill-region-internal beg2 end2)
       (evil--record-yank linewise)
       (evil--maybe-write-register linewise)
       (goto-char beg2)
       (when linewise (goto-char (evil--pos-first-non-blank)))
       (evil--set-state 'normal)))))

(defun evil--op-run (target type &optional replay)
  "Feed [point, TARGET) to the pending operator as a charwise region;
TYPE is 'inclusive or 'exclusive (see the file header for what that
means concretely in this implementation). REPLAY, when given, is
recorded as the dot-repeat target (see `evil--record-change') before
the operator mutates the buffer -- recording after would see
`evil--pending-operator' already cleared by `evil--operator-apply'."
  (when replay (evil--record-change replay))
  (evil--operator-apply (point) target (if (eq type 'inclusive) 1 0) nil))

(defun evil--op-run-linewise (target &optional replay)
  (when replay (evil--record-change replay))
  (evil--operator-apply (point) target 0 t))

(defun evil--op-start (op)
  (setq-local evil--op-count (evil--total-count))
  (setq-local evil--pending-operator op)
  (evil--set-state 'operator-pending))

(defun evil-op-delete () (interactive) (evil--op-start 'delete))
(defun evil-op-change () (interactive) (evil--op-start 'change))
(defun evil-op-yank () (interactive) (evil--op-start 'yank))
;; M45: the three case-operator starters (`g~'/`gu'/`gU') -- same shape
;; as the three above, just a different `evil--pending-operator' symbol
;; for `evil--operator-apply''s new case-op branch to dispatch on.
(defun evil-op-toggle-case () (interactive) (evil--op-start 'toggle-case))
(defun evil-op-downcase () (interactive) (evil--op-start 'downcase))
(defun evil-op-upcase () (interactive) (evil--op-start 'upcase))

(defun evil--op-current-lines ()
  "dd/cc/yy: the operator already active in `evil--pending-operator'
\(set by whichever of d/c/y started this operator-pending session)
applied linewise to the current line, repeated by count. `evil-yank-
line' (Y) and `evil-change-line' (S) also delegate to this function --
recording the dot-repeat replay HERE (rather than separately in each of
those three entry points) is correct for all three: this function's
own behavior depends only on `evil--pending-operator' and the count,
never on which key sequence got here, so \"redo `evil--op-current-lines'
with the SAME operator and count\" reproduces any of dd/cc/yy/Y/S
faithfully regardless of which one originally ran."
  (interactive)
  (let* ((n (evil--total-count))
         (end (save-excursion (forward-line (max 0 (1- n))) (point))))
    (evil--record-change (evil--replay-thunk 'evil--op-current-lines n))
    (evil--operator-apply (point) end 0 t)))

;; --- Simple motions (normal + visual + operator-pending) ---------------

(defun evil-backward-char ()
  (interactive)
  (let* ((n (evil--total-count))
         (target (max (line-beginning-position) (- (point) n))))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-backward-char n))
      (goto-char target))))

(defun evil-forward-char ()
  (interactive)
  (let* ((n (evil--total-count))
         (target (min (line-end-position) (+ (point) n))))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-forward-char n))
      (goto-char target))))

(defun evil-next-line ()
  (interactive)
  (let ((n (evil--total-count)))
    (if evil--pending-operator
        (evil--op-run-linewise (save-excursion (forward-line n) (point))
                                (evil--replay-thunk 'evil-next-line n))
      (next-line n))))

(defun evil-previous-line ()
  (interactive)
  (let ((n (evil--total-count)))
    (if evil--pending-operator
        (evil--op-run-linewise (save-excursion (forward-line (- n)) (point))
                                (evil--replay-thunk 'evil-previous-line n))
      (previous-line n))))

;; M36 review fix (severity: high): vim's normal/visual-state RET moves
;; to the first non-blank character of the line COUNT lines below
;; (default 1) -- equivalent to `+'/`gj' (neither separately
;; implemented here). Deliberately NOT bound via `evil--bind-motion'
;; (which would also wire it into `evil--op-pending-map'): real vim's
;; `d<CR>' operator use is out of v1 scope (see the file header), so
;; `evil--op-pending-map' binds RET straight to `evil--op-invalid'
;; instead (see the keymap population section) -- meaning
;; `evil--pending-operator' is NEVER non-nil when this function
;; actually runs (only `evil--normal-map'/`evil--visual-map' ever
;; dispatch to it), unlike every `evil--bind-motion'-bound motion
;; above, which is why this has no operator branch of its own. Visual
;; state's selection extends automatically (the mark stays active; see
;; `evil--visual-range') -- no special-casing needed here either.
(defun evil-ret ()
  (interactive)
  (let* ((n (max 1 (evil--total-count)))
         (target (save-excursion
                   (forward-line n)
                   (evil--pos-first-non-blank-or-bol))))
    (goto-char target)))

(defun evil--word-forward-target (n big)
  "(TARGET . TYPE) for `w'/`W' as a motion. Real vim's documented cw/cW
special case (`:help cw'): when the pending operator is `change' and
point sits on a non-blank character, the effective target is the END
of the Nth word (like `e'/`E', inclusive) instead of the start of the
following word (exclusive) -- so `cw' doesn't eat the whitespace after
the changed word the way `dw' deliberately does."
  (if (and (eq evil--pending-operator 'change)
           (not (eq (evil--class-at (point) big) 'space)))
      (cons (evil--end-word-pos n big) 'inclusive)
    (cons (evil--fwd-word-pos n big) 'exclusive)))

(defun evil-word-forward ()
  (interactive)
  (let* ((n (evil--total-count)) (r (evil--word-forward-target n nil)))
    (if evil--pending-operator
        (evil--op-run (car r) (cdr r) (evil--replay-thunk 'evil-word-forward n))
      (goto-char (car r)))))
(defun evil-word-backward ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--bwd-word-pos n nil)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-word-backward n))
      (goto-char target))))
(defun evil-word-end ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--end-word-pos n nil)))
    (if evil--pending-operator
        (evil--op-run target 'inclusive (evil--replay-thunk 'evil-word-end n))
      (goto-char target))))
(defun evil-WORD-forward ()
  (interactive)
  (let* ((n (evil--total-count)) (r (evil--word-forward-target n t)))
    (if evil--pending-operator
        (evil--op-run (car r) (cdr r) (evil--replay-thunk 'evil-WORD-forward n))
      (goto-char (car r)))))
(defun evil-WORD-backward ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--bwd-word-pos n t)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-WORD-backward n))
      (goto-char target))))
(defun evil-WORD-end ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--end-word-pos n t)))
    (if evil--pending-operator
        (evil--op-run target 'inclusive (evil--replay-thunk 'evil-WORD-end n))
      (goto-char target))))

(defun evil-end-of-prev-word ()  ; ge
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--bwd-end-word-pos n nil)))
    (if evil--pending-operator
        (evil--op-run target 'inclusive (evil--replay-thunk 'evil-end-of-prev-word n))
      (goto-char target))))

(defun evil-end-of-prev-WORD ()  ; gE
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--bwd-end-word-pos n t)))
    (if evil--pending-operator
        (evil--op-run target 'inclusive (evil--replay-thunk 'evil-end-of-prev-WORD n))
      (goto-char target))))

(defun evil-digit-0-or-bol ()
  (interactive)
  (if evil--count
      (evil--digit 0)
    (let ((target (line-beginning-position)))
      (if evil--pending-operator
          (evil--op-run target 'exclusive (evil--replay-thunk 'evil-digit-0-or-bol nil))
        (goto-char target)))))

(defun evil-first-non-blank ()
  (interactive)
  (evil--total-count)
  (let ((target (evil--pos-first-non-blank)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-first-non-blank nil))
      (goto-char target))))

(defun evil-end-of-line ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--pos-eol n)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-end-of-line n))
      (goto-char target))))

(defun evil-last-non-blank ()  ; g_
  "M45: unlike `$' (`evil-end-of-line'), whose target already sits one
past the last real character (see `evil--pos-eol', and the file
header's note on why that motion is wired `exclusive'), `evil--pos-eol-
non-blank' lands ON the character itself -- so this needs the `+1'
`inclusive' provides to cover that character at all."
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--pos-eol-non-blank n)))
    (if evil--pending-operator
        (evil--op-run target 'inclusive (evil--replay-thunk 'evil-last-non-blank n))
      (goto-char target))))

(defun evil-goto-first-line ()
  (interactive)
  (let* ((has-count evil--count)
         (n (evil--total-count))
         (target (evil--pos-goto-line (if has-count n 1))))
    (if evil--pending-operator
        (evil--op-run-linewise target (evil--replay-goto-thunk 'evil-goto-first-line has-count n))
      (goto-char target))))

(defun evil-goto-line-or-last ()
  (interactive)
  (let* ((has-count evil--count)
         (n (evil--total-count))
         (target (evil--pos-goto-line (if has-count n (evil--total-lines)))))
    (if evil--pending-operator
        (evil--op-run-linewise target (evil--replay-goto-thunk 'evil-goto-line-or-last has-count n))
      (goto-char target))))

(defun evil--pos-goto-line (n)
  (save-excursion
    (evil--goto-line (max 1 (min n (evil--total-lines))))
    (evil--pos-first-non-blank)))

(defun evil-paragraph-forward ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--pos-paragraph-forward n)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-paragraph-forward n))
      (goto-char target))))
(defun evil-paragraph-backward ()
  (interactive)
  (let* ((n (evil--total-count)) (target (evil--pos-paragraph-backward n)))
    (if evil--pending-operator
        (evil--op-run target 'exclusive (evil--replay-thunk 'evil-paragraph-backward n))
      (goto-char target))))

(defun evil--halfpage ()
  "Approximate half-page line count for C-d/C-u -- `frame-height'/2 (a
window's actual height isn't exposed to elisp here); documented v1
simplification, see the file header."
  (max 1 (/ (frame-height) 2)))

(defun evil-scroll-down ()
  (interactive)
  (next-line (evil--halfpage)))
(defun evil-scroll-up ()
  (interactive)
  (previous-line (evil--halfpage)))

;; --- Digit arguments (normal + visual + operator-pending) --------------

(defun evil-digit-1 () (interactive) (evil--digit 1))
(defun evil-digit-2 () (interactive) (evil--digit 2))
(defun evil-digit-3 () (interactive) (evil--digit 3))
(defun evil-digit-4 () (interactive) (evil--digit 4))
(defun evil-digit-5 () (interactive) (evil--digit 5))
(defun evil-digit-6 () (interactive) (evil--digit 6))
(defun evil-digit-7 () (interactive) (evil--digit 7))
(defun evil-digit-8 () (interactive) (evil--digit 8))
(defun evil-digit-9 () (interactive) (evil--digit 9))

;; --- Simple normal-state edits ------------------------------------------

(defun evil-delete-char ()  ; x
  (interactive)
  (let* ((n (evil--total-count)) (end (min (+ (point) n) (line-end-position))))
    (setq-local evil--pending-operator 'delete)
    (evil--record-change (evil--replay-thunk 'evil-delete-char n))
    (evil--operator-apply (point) end 0 nil)))

(defun evil-delete-char-backward ()  ; X
  (interactive)
  (let* ((n (evil--total-count)) (beg (max (line-beginning-position) (- (point) n))))
    (setq-local evil--pending-operator 'delete)
    (evil--record-change (evil--replay-thunk 'evil-delete-char-backward n))
    (evil--operator-apply beg (point) 0 nil)))

(defun evil-substitute-char ()  ; s
  (interactive)
  (let* ((n (evil--total-count)) (end (min (+ (point) n) (line-end-position))))
    (setq-local evil--pending-operator 'change)
    (evil--record-change (evil--replay-thunk 'evil-substitute-char n))
    (evil--operator-apply (point) end 0 nil)))

(defun evil-delete-to-eol ()  ; D
  (interactive)
  (let* ((n (evil--total-count)) (end (evil--pos-eol n)))
    (setq-local evil--pending-operator 'delete)
    (evil--record-change (evil--replay-thunk 'evil-delete-to-eol n))
    (evil--operator-apply (point) end 0 nil)))

(defun evil-change-to-eol ()  ; C
  (interactive)
  (let* ((n (evil--total-count)) (end (evil--pos-eol n)))
    (setq-local evil--pending-operator 'change)
    (evil--record-change (evil--replay-thunk 'evil-change-to-eol n))
    (evil--operator-apply (point) end 0 nil)))

(defun evil-yank-line ()  ; Y
  (interactive)
  (setq-local evil--pending-operator 'yank)
  (evil--op-current-lines))

(defun evil-change-line ()  ; S
  (interactive)
  (setq-local evil--pending-operator 'change)
  (evil--op-current-lines))

(defun evil-join ()  ; J
  (interactive)
  (let* ((n0 (evil--total-count)) (n (max 2 n0)))
    (evil--record-change (evil--replay-thunk 'evil-join n0))
    (dotimes (i (1- n))
      (unless (= (line-number-at-pos) (evil--total-lines))
        (goto-char (line-end-position))
        (delete-char 1)
        (while (memq (char-after) '(?\s ?\t))
          (delete-char 1))
        (insert " ")
        (backward-char 1)))))

(defun evil-join-no-space ()  ; gJ
  "M45: `J''s variant -- vim's `gJ' joins the same COUNT lines `J' does
(same `n = max(2, count)', joining N-1 times, count shape) but WITHOUT
`J''s own \"eat trailing whitespace, insert exactly one space\" step:
just delete the newline itself, nothing else. `evil-join' left
untouched (`J' keeps its own vim-standard behavior)."
  (interactive)
  (let* ((n0 (evil--total-count)) (n (max 2 n0)))
    (evil--record-change (evil--replay-thunk 'evil-join-no-space n0))
    (dotimes (i (1- n))
      (unless (= (line-number-at-pos) (evil--total-lines))
        (goto-char (line-end-position))
        (delete-char 1)))))

(defun evil--do-replace-char (k n)
  "The `capture-next-key' callback for `r'. Dot-repeat: recorded HERE
\(not in `evil-replace-char') since K -- the captured replacement
character -- must be threaded into the replay directly, the same
reasoning as `evil--replay-find-thunk' for f/F/t/T: there is no further
input for `.' to wait for, so replaying just re-invokes this function
with the SAME K and N at the new point. Only recorded on a successful
replacement (ESC, or running out of characters on the line, leaves
`evil--last-change' untouched)."
  (unless (or (not (integerp k)) (= k 27))
    (let ((end (min (+ (point) n) (line-end-position))))
      (when (= end (+ (point) n))
        (let ((ch (char-to-string k)) (beg (point)) (text ""))
          (dotimes (i n) (setq text (concat text ch)))
          (delete-region beg end)
          (insert text)
          (goto-char (1- (point)))
          (evil--record-change (lambda () (evil--do-replace-char k n))))))))

(defun evil-replace-char ()  ; r
  (interactive)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--do-replace-char k n)))))

(defun evil--flip-case-char (c)
  (let ((s (char-to-string c)))
    (string-to-char (if (string= s (upcase s)) (downcase s) (upcase s)))))

(defun evil--case-toggle-string (s)
  "M45 `g~': `evil--flip-case-char', mapped per-character over string S
-- `evil--operator-apply''s `toggle-case' branch applies this to a
whole region at once, the same way plain `~' applies `evil--flip-case-
char' one character at a time.

Review fix: accumulates the flipped characters onto a LIST (consed on
front-to-back, then `nreverse'd -- the same collect-then-`nreverse'
idiom `evil--quote-pair' already uses for its own position list) and
joins it with ONE `apply'd `concat' at the end, rather than the more
obvious `(setq out (concat out ...))' loop. That loop is O(n^2) in the
length of S: `concat' allocates and copies a fresh string the length of
ALL its arguments on EVERY call, so rebuilding the whole accumulator
one character at a time makes a whole-line `g~~'/`g~$' quadratic in
line length. `cons' is O(1) per character and the single trailing
`concat' is one O(n) pass, so this is O(n) overall."
  (let ((out nil))
    (dotimes (i (length s))
      (setq out (cons (char-to-string (evil--flip-case-char (aref s i))) out)))
    (apply 'concat (nreverse out))))

(defun evil-flip-case ()  ; ~
  (interactive)
  (let ((n (evil--total-count)))
    (evil--record-change (evil--replay-thunk 'evil-flip-case n))
    (dotimes (i n)
      (unless (eobp)
        (let ((c (char-after)))
          (when c
            (delete-char 1)
            (insert (char-to-string (evil--flip-case-char c)))))))))

(defun evil--repeat-string (s n)
  (let ((out ""))
    (dotimes (i n) (setq out (concat out s)))
    out))

;; --- M42: named registers (`"a'-`"z') ------------------------------------
;;
;; `"' arms a `capture-next-key' callback for the register's NAME
;; (a-z), the same idiom as `m' above -- but unlike a mark, a register
;; selection is a ONE-SHOT PREFIX to the NEXT operator/paste command,
;; not a standalone target: `evil--pending-register' (Global state
;; section, an `evil--count'-fresh-flag-mirroring model, see there)
;; holds it until `evil--maybe-write-register' (Operator engine
;; section, above) or `evil--do-paste' (below) consumes it.
;; `evil--registers' itself (Global state section) is the storage.
;; Deliberately normal-map-only (not wired into `evil--bind-motion' or
;; `evil--visual-map'): real vim's register prefix always comes
;; FIRST, before the operator it modifies (`"ad', never `d"a'), so
;; `evil--op-pending-map''s printable-key catchall correctly cancels a
;; stray `"' mid-operator the same as any other unclaimed key (see the
;; keymap population section) -- v1 doesn't wire an explicit visual-
;; state `"' either (not in the M42 plan's test list), though a
;; register armed BEFORE entering visual state is still honored when
;; `evil--visual-apply' reaches `evil--operator-apply', an incidental
;; consequence of the storage being the same global variable, not
;; something specifically implemented here.

(defun evil--handle-use-register (k)
  "The `capture-next-key' callback for `\"'. K -- see `evil--handle-set-
mark'."
  (cond
    ((not (integerp k)) nil)
    ((= k 27) nil)
    ((and (>= k ?a) (<= k ?z))
     (setq-local evil--pending-register k))
    (t (message "Registers must be a-z"))))

(defun evil-use-register ()  ; "
  (interactive)
  (capture-next-key (lambda (k) (evil--handle-use-register k))))

(defun evil--paste-charwise (text before)
  (unless before
    (unless (eobp) (forward-char 1)))
  (let ((start (point)))
    (insert text)
    (goto-char (max start (1- (point))))))

(defun evil--paste-linewise (text before)
  (let ((body (if (string-suffix-p "\n" text) text (concat text "\n")))
        (start nil))
    (if before
        (goto-char (line-beginning-position))
      (progn
        (goto-char (line-end-position))
        (if (eobp) (insert "\n") (forward-char 1))))
    (setq start (point))
    (insert body)
    (goto-char start)
    (goto-char (evil--pos-first-non-blank))))

(defun evil--paste-text (text linewise n before)
  "Shared LINEWISE dispatch for `evil--do-paste' -- factored out since
M42 needs it from BOTH the unnamed (kill-ring) and named-register
branches now, not just one."
  (if linewise
      (evil--paste-linewise (evil--repeat-string text n) before)
    (evil--paste-charwise (evil--repeat-string text n) before)))

(defun evil--do-paste (cmd before)
  "Shared body of `evil-paste-after'/`evil-paste-before'. CMD is the
command symbol `evil--replay-thunk' redoes on `.'; BEFORE matches
`evil--paste-charwise'/`evil--paste-linewise's own BEFORE argument.

M42: if a register was armed via `\"' (`evil-use-register'), paste FROM
`evil--registers' instead of the kill-ring -- each register entry
carries its OWN linewise flag (set when it was WRITTEN, see
`evil--maybe-write-register'), so this is naturally immune to
`evil--current-yank-linewise-p''s M29 kill-ring-staleness guard: there
is no shared, natively-clobberable \"top of the register\" for a named
read to be stale against. `evil--pending-register' is read into REG up
front but deliberately left live in the global until AFTER
`evil--record-change' runs below -- `evil--record-change' itself reads
the still-armed global to freeze REG into the dot-repeat replay (see
its own M42 doc note); only once that's done is it actually cleared."
  (let* ((reg evil--pending-register)
         (n (evil--total-count))
         (entry (and reg (assq reg evil--registers))))
    (cond
      ((and reg (not entry))
       (setq-local evil--pending-register nil)
       (message "Nothing in register %c" reg))
      (reg
       (evil--record-change (evil--replay-thunk cmd n))
       (setq-local evil--pending-register nil)
       (evil--paste-text (cadr entry) (cddr entry) n before))
      (t
       (let ((text (current-kill)))
         (if (not text)
             (message "Nothing to paste")
           (progn
             ;; Dot-repeat just re-invokes CMD itself, which re-reads
             ;; `current-kill'/the linewise flag fresh each time --
             ;; matching vim's own `.' semantics for `p' (replays
             ;; "paste whatever the register holds NOW", not a frozen
             ;; copy of what it held originally).
             (evil--record-change (evil--replay-thunk cmd n))
             (evil--paste-text text (evil--current-yank-linewise-p) n before))))))))

(defun evil-paste-after ()  ; p
  (interactive)
  (evil--do-paste 'evil-paste-after nil))

(defun evil-paste-before ()  ; P
  (interactive)
  (evil--do-paste 'evil-paste-before t))

;; --- M42-II: keyboard macros (`q' / `@') --------------------------------
;;
;; Recording itself (which keys land in a macro) is entirely Rust-side
;; -- a tap in `handle_key' (commands.rs) that runs on EVERY key,
;; unconditionally, ahead of even `capture-next-key' dispatch, so a
;; macro that itself uses f/t/r/m/"/etc. captures its OWN keys
;; correctly (see the tap's own doc comment for the full ordering
;; argument). This file only arms/disarms that recording and picks the
;; register -- `start-kbd-macro'/`end-kbd-macro'/`execute-kbd-macro'/
;; `defining-kbd-macro-p'/`kbd-macro-p' (builtins/ui.rs) are the entire
;; primitive surface.
;;
;; Deliberately normal-map-only (not wired into `evil--bind-motion' or
;; `evil--visual-map', same reasoning as `m'/`"' above): neither `q' nor
;; `@' is a motion or composes with an operator.

(defvar evil--last-macro-register nil
  "Register character most recently played by `evil-execute-macro'
(`@') -- what a following `@@' replays. Global, like the kbd-macro
registers themselves (`Editor::kbd_macros' is not per-buffer).")

(defvar evil--recording-register nil
  "Register the CURRENTLY ACTIVE kbd-macro recording (if any) was
started under -- a plain elisp mirror of the first element of the
Rust-side `Editor::kbd_macro_recording' tuple, kept ONLY so
`evil-record-macro' can name the register in its own \"(recorded @X)\"
message when stopping (`defining-kbd-macro-p' is the actual source of
truth for \"is one active at all\" -- this variable is never consulted
for that)."
  nil)

(defun evil--handle-record-macro-start (k)
  "The `capture-next-key' callback for `q' when NOT already recording.
K -- see `evil--handle-set-mark' for the non-integer/27 (ESC) cancel
convention and the a-z-only restriction this mirrors."
  (cond
    ((not (integerp k)) nil)
    ((= k 27) nil)
    ((and (>= k ?a) (<= k ?z))
     (start-kbd-macro k)
     (setq evil--recording-register k)
     (message "(recording @%c)" k))
    (t (message "Macro registers must be a-z"))))

(defun evil-record-macro ()  ; q
  (interactive)
  (if (defining-kbd-macro-p)
      ;; M42-II: the STOP `q' itself already landed in the recording
      ;; via the Rust tap (it runs unconditionally, ahead of dispatch
      ;; even reaching this command) -- `(end-kbd-macro 1)' trims it
      ;; back off before storing the rest.
      (let ((reg evil--recording-register))
        (end-kbd-macro 1)
        (setq evil--recording-register nil)
        (message "(recorded @%c)" reg))
    (capture-next-key (lambda (k) (evil--handle-record-macro-start k)))))

(defun evil--do-execute-macro (reg n)
  "Shared body of `evil-execute-macro''s a-z and `@@' branches: replay
REG's macro N times and remember REG for a LATER `@@' to reuse."
  (if (not (kbd-macro-p reg))
      (message "No macro in register %c" reg)
    (progn
      (setq evil--last-macro-register reg)
      (execute-kbd-macro reg n))))

(defun evil--handle-execute-macro (k n)
  "The `capture-next-key' callback for `@'. K -- see `evil--handle-
set-mark'. N is the count `evil-execute-macro' already consumed BEFORE
arming this capture -- see its own doc comment for why reading it
LAZILY, from inside this callback, would be wrong. `@@' (K = `?@')
replays whichever register `evil--last-macro-register' remembers --
\"No previous macro\" if `@' has never successfully replayed one this
session."
  (cond
    ((not (integerp k)) nil)
    ((= k 27) nil)
    ((= k ?@)
     (if (not evil--last-macro-register)
         (message "No previous macro")
       (evil--do-execute-macro evil--last-macro-register n)))
    ((and (>= k ?a) (<= k ?z)) (evil--do-execute-macro k n))
    (t (message "Macro registers must be a-z"))))

(defun evil-execute-macro ()  ; @
  "M42-II review fix (self-caught while testing `3@a'): `evil--total-
count' is consumed HERE, eagerly, rather than later inside the capture
callback above. Arming a capture still finishes the command normally,
so this command's own `evil--post-command' pass runs the count-
staleness cleanup before the second keystroke -- the register name --
ever arrives; left unread, it would see `evil--count' still sitting
there with `evil--count-fresh' back to nil (reset by the FIRST
command's own `evil--post-command' pass, e.g. the \"3\" of \"3@a\") and
clear it out from under the capture callback. `3d@a' (count typed
BEFORE the operator) never hits this: `evil--op-start' already moves
the count into `evil--op-count', which `evil--post-command' never
touches, before any capture is armed -- unlike `d3@a' (count typed
AFTER the operator), which would share this command's OWN hazard
exactly, since `evil--digit' writes every digit into `evil--count'
regardless of state. (`d3@a' isn't actually a real sequence either
way -- `@' never composes with an operator, see above -- the
before/after contrast is illustrative only, mirroring `f'/`F'/`t'/`T',
where it IS real: `d3fx' -- count typed after the operator that arms
their own capture -- hit this exact hazard until M44 applied this same
eager-read move to `evil-find-char-forward' and friends; `3dfx' was
always safe, for the same `evil--op-count' reason as `3d@a' above. An
earlier version of this comment had that contrast backwards.) This
function's own eager read is unaffected by any of it: `3@a' has no
operator in the picture at all, and is the one case this docstring is
actually about -- pinned by
`a_count_prefixed_replay_runs_the_macro_that_many_times' in
evil_ex_macro_tests.rs."
  (interactive)
  (let ((n (evil--total-count)))
    (capture-next-key (lambda (k) (evil--handle-execute-macro k n)))))

;; --- Redo (see the file header for why `u' is bound to plain `undo') --

(defun evil--redo-marker ()
  "A no-op interactive command. Running it through `command-execute'
updates the editor's internal last-command to this symbol (definitely
not `undo'), which `evil-redo' immediately exploits: `undo-internal'
only continues its backward chain when last-command IS literally
`undo' (see editing.rs), so forcing it to anything else first makes
the *next* `undo-internal' call treat the redo-inverse group every
undo leaves at the tail of the undo log as fresh work, i.e. a redo.
There is no elisp accessor for last-command in this codebase; running
a command through the ordinary command loop is the only way to
influence it from elisp."
  (interactive))

(defun evil-redo ()  ; C-r
  (interactive)
  (command-execute 'evil--redo-marker)
  (if (undo-internal)
      (message "Redo!")
    (message "No further redo information")))

;; --- Insert state entry points -------------------------------------------

(defun evil--begin-insert-session (entry)
  "Arm dot-repeat's insert-text capture for a FRESH i/a/I/A/o/O entry:
ENTRY (a zero-arg function -- here always a bare command symbol) is
what `evil-insert-exit' will re-run, followed by re-typing whatever
gets typed during this session, once it finalizes `evil--last-change'
-- see `evil--finish-insert-session'. Called AFTER point has already
been moved to this command's own insert-start position (e.g. `a''s
`forward-char', `o''s newline-then-point-lands-on-it), since that
position -- not wherever point was when the command STARTED -- is
where the captured text begins.

The `change' operator's own entry into insert state (cw, ciw, s, S, C,
cc, ...) does NOT come through here -- see `evil--operator-apply''s
`change' branch, which sets `evil--insert-start'/`evil--insert-entry'
directly from `evil--pending-insert-entry' (the operator+motion/
text-object/simple-edit replay thunk `evil--record-change' already
built), since a bare command symbol wouldn't carry the operator/count/
captured-character information those replays need."
  (setq-local evil--insert-start (point))
  (setq evil--insert-entry entry))

(defun evil-insert ()  ; i
  (interactive)
  (evil--total-count)
  (evil--begin-insert-session 'evil-insert)
  (evil--set-state 'insert))

(defun evil-append ()  ; a
  (interactive)
  (evil--total-count)
  (unless (>= (point) (line-end-position)) (forward-char 1))
  (evil--begin-insert-session 'evil-append)
  (evil--set-state 'insert))

(defun evil-insert-bol ()  ; I
  (interactive)
  (evil--total-count)
  (goto-char (evil--pos-first-non-blank))
  (evil--begin-insert-session 'evil-insert-bol)
  (evil--set-state 'insert))

(defun evil-append-eol ()  ; A
  (interactive)
  (evil--total-count)
  (goto-char (line-end-position))
  (evil--begin-insert-session 'evil-append-eol)
  (evil--set-state 'insert))

(defun evil-open-below ()  ; o
  (interactive)
  (evil--total-count)
  (goto-char (line-end-position))
  (insert "\n")
  ;; M30: merge this newline with the text about to be typed into ONE
  ;; undo group -- same reasoning as the `change' operator branch in
  ;; `evil--operator-apply'.
  (undo-amalgamate-boundary)
  ;; M36: vim's own `o'/`O' autoindent convention -- a no-op wherever
  ;; the buffer has no `indent-line-function' (any buffer that isn't
  ;; one of indent.el's eight prog modes), so this doesn't change
  ;; anything for the plain fundamental-mode buffers this file's own
  ;; tests use. See indent.el's header.
  (indent-current-line-if-supported)
  (evil--begin-insert-session 'evil-open-below)
  (evil--set-state 'insert))

(defun evil-open-above ()  ; O
  (interactive)
  (evil--total-count)
  (goto-char (line-beginning-position))
  (insert "\n")
  (backward-char 1)
  (undo-amalgamate-boundary)
  (indent-current-line-if-supported)  ; M36, see `evil-open-below'
  (evil--begin-insert-session 'evil-open-above)
  (evil--set-state 'insert))

(defun evil-normal-esc ()  ; ESC in normal state
  "M29 review fix: vim convention -- ESC in normal state clears any
half-typed count (`5' with no motion following it yet) rather than
leaving it to linger. This is a stronger, explicit guarantee on top of
the general `evil--count-fresh' staleness check in
`evil--post-command' (which would eventually clear it too, on the next
unrelated command, but there's no reason to make ESC wait for that)."
  (interactive)
  (setq-local evil--count nil)
  (setq-local evil--op-count nil))

(defun evil--finish-insert-session ()
  "Called from `evil-insert-exit', BEFORE point moves: if the insert
session about to end was one dot-repeat is tracking
\(`evil--insert-start' non-nil -- set by `evil--begin-insert-session' or
`evil--operator-apply''s `change' branch; NOT set for a visual-state
change, matching the file header's \"visual dot-repeat out of scope\"
note, so this correctly does nothing for one of those), record
`evil--last-change' as \"redo the entry, then retype this session's
text, then leave insert state again\" -- vim's own `.' semantics for
i/a/I/A/o/O and for c-family operators alike.

Best-effort if point moved outside [insert-start, point) during the
session (this editor's insert-map only ever binds ESC -- see the file
header -- so in practice this needs an out-of-band motion like a mouse
click): `buffer-substring' just reflects whatever text now sits
between the two positions, which can come out empty or start from the
wrong end if point ended up before `evil--insert-start'. Documented,
not guarded against, matching this file's existing \"best effort\"
stance elsewhere (e.g. i/a/I/A/o/O's own count handling).

Replaying re-invokes ENTRY, which for an operator+motion/text-object
naturally re-triggers `evil--record-change'/`evil--begin-insert-session'
as ordinary side effects of running that command again -- so this
function itself runs again too as part of the replay's own `(evil-
insert-exit)' call, rebuilding an equivalent (if not `eq') replacement
for `evil--last-change'. Harmless: each rebuild reproduces the same
behavior, so repeated `.' presses keep working indefinitely."
  (when evil--insert-start
    (let ((text (buffer-substring evil--insert-start (point)))
          (entry evil--insert-entry))
      (setq evil--last-change
            (lambda ()
              (funcall entry)
              (insert text)
              (evil-insert-exit)))))
  ;; M45 `gi': snapshot BEFORE `evil--insert-start' is nil'd below --
  ;; unconditional (unlike the dot-repeat block above), since `gi' tracks
  ;; "where insert mode was last left" regardless of whether THIS session
  ;; happened to be dot-repeat-tracked.
  (setq-local evil--last-insert-pos (point))
  (setq-local evil--insert-start nil)
  (setq evil--insert-entry nil))

(defun evil-insert-exit ()  ; ESC in insert state
  (interactive)
  (evil--finish-insert-session)
  (unless (bolp) (backward-char 1))
  (evil--set-state 'normal))

(defun evil-goto-last-insert ()  ; gi
  "M45: vim's `gi' -- resume insert at `evil--last-insert-pos' (where
insert mode was last exited), clamped to `point-max' in case the buffer
has since shrunk. No prior insert session this buffer-local var has
ever seen (`evil--last-insert-pos' nil) leaves point where it already
is, same as plain `i' -- matching real vim's own behavior the very
first time. Body otherwise shaped exactly like `evil-insert' (count
consumed exactly once -- see `evil--total-count''s own docstring --
rather than delegating to `evil-insert' and risking a second,
redundant consume).

Review fix: the dot-repeat REPLAY entry passed to `evil--begin-insert-
session' is `evil-insert', NOT `evil-goto-last-insert' itself, even
though this command IS `evil-goto-last-insert'. `evil--last-insert-pos'
is a buffer-local var this SAME session's own exit unconditionally
overwrites (see `evil--finish-insert-session'), so a replay that re-
invoked `evil-goto-last-insert' would jump to whatever stale/self-
referential position that overwrite left behind instead of continuing
at `.''s own point. The jump-to-last-insert-position above is `g i''s
own-keypress behavior only, and never re-runs on replay -- matching
every other entry in this section (`i'/`a'/`I'/`A'/`o'/`O'), which all
recompute their target from the CURRENT point rather than replaying a
captured absolute one."
  (interactive)
  (evil--total-count)
  (when evil--last-insert-pos
    (goto-char (min evil--last-insert-pos (point-max))))
  (evil--begin-insert-session 'evil-insert)
  (evil--set-state 'insert))

;; --- M31: dabbrev completion (C-n/C-p in insert state) -------------------
;;
;; Upstream evil's own names AND its own direction pairing: `C-n' (vim's
;; `i_CTRL-N') searches AFTER point first; `C-p' (`i_CTRL-P', and GNU
;; `dabbrev-expand''s own traditional default -- see `M-/') searches
;; BEFORE point first. See dabbrev.el's file header for the shared
;; engine both of these -- and `M-/' -- call into.

(defun evil-complete-next ()  ; C-n
  (interactive)
  (dabbrev--complete 'backward))

(defun evil-complete-previous ()  ; C-p
  (interactive)
  (dabbrev--complete 'forward))

;; --- Dot-repeat (`.') ----------------------------------------------------

(defun evil-repeat-change ()  ; .
  "Replay `evil--last-change' (see its docstring) at point, COUNT times
\(default 1 -- typing a count before `.', e.g. `3.', repeats the WHOLE
recorded change three times in a row rather than overriding the count
the change itself was originally recorded with -- a deliberate
simplification of real vim's `:help .', which lets a fresh count
REPLACE the recorded one instead; see the file header)."
  (interactive)
  (let ((n (evil--total-count)) (thunk evil--last-change))
    (if (not thunk)
        (message "Nothing to repeat")
      (dotimes (i n)
        (funcall thunk)))))

;; --- Search bridge (`/ ? n N') -------------------------------------------
;;
;; `/' and `?' bind straight to the existing `isearch-forward'/
;; `isearch-backward' commands (Rust-side modal key handling lives in
;; commands.rs's `isearch_key', unchanged by evil.el) -- so an active
;; search is, deliberately, not represented in `evil--state' at all;
;; `commands.rs' `handle_key' intercepts isearch keys before consulting
;; `emulation-keymap' (see its module doc), the same way it intercepts
;; C-g and the minibuffer. Point lands wherever isearch itself already
;; leaves it on RET (Emacs convention: just PAST a forward match, ON the
;; first character of a backward match) -- not vim's own "always on the
;; first character of the match" convention; not changed here, since the
;; job is to bridge onto the EXISTING isearch, not reimplement it.
;;
;; `n'/`N' repeat the last search isearch actually COMMITTED
;; (`Editor::last_search' in editor.rs, set by `isearch_exit'; survives
;; past the search session itself, unlike the session's own live query
;; -- there was no elisp accessor for either the query string or its
;; direction before M30, so the minimal `isearch-last-string'/`isearch-
;; last-forward-p' builtins were added, see builtins/ui.rs) via the
;; plain-text `search-forward'/`search-backward' builtins (the same
;; primitives isearch itself is built on -- `crate::gapbuffer::
;; find_forward'/`find_backward' -- so a match `n' finds is exactly one
;; isearch would also have found). `n' repeats in the ORIGINAL
;; direction, `N' reverses it, both wrapping around the buffer with an
;; echoed notice, matching vim. Neither is dot-repeat-tracked (vim
;; semantics: a search is a motion, not a change) nor usable as an
;; operator target (out of scope for M30 -- neither is bound in
;; `evil--op-pending-map', so `dn' falls to the existing unbound-
;; printable-key catch-all, same as before this file added `n'/`N' at
;; all).
;;
;; M30 review fix (issue 2, high severity): `find_forward'/
;; `find_backward' (and so `search-forward'/`search-backward') are
;; INCLUSIVE of the boundary point already rests on. Continuing in the
;; SAME direction never actually hits this in practice (searching
;; forward from a match's own END, or backward from a match's own
;; START, can't re-find that same span by construction), but REVERSING
;; (`N', or `n' after an odd number of `N's) lands the very next
;; candidate search exactly on the span already under point --
;; accepting it at face value reports a self-match as if it were the
;; next/previous occurrence, which is wrong (and is what an earlier
;; version of this file's own test incorrectly pinned as "correct").
;; `evil--last-match-span' (with `evil--sync-match-span'/`evil--search-
;; one' below) fixes this: track the (START . END) of whatever match
;; point is currently sitting at the edge of, and when a fresh
;; candidate equals it, search past it once more before accepting.
(defvar evil--last-match-span nil
  "(START . END) -- 1-based, END exclusive -- of the match `n'/`N' (or
the isearch commit before them) most recently left point at the edge
of; nil until the first repeat. Global like the rest of the search-
bridge state above it (`Editor::last_search') and `evil--last-change'
et al: a per-session register, not a per-buffer one. There is no hook
into isearch's own commit to update this directly, so it's kept in
sync lazily instead -- see `evil--sync-match-span'.")

(defun evil--search-one (s forward)
  "Try ONE plain (non-wrapping) search for S in direction FORWARD from
point. (START . END) -- 1-based, END exclusive -- on success, nil on
failure. Does not itself guard against landing on `evil--last-match-
span' -- see `evil--search-repeat-1' for that."
  (let ((len (length s)))
    (if forward
        (let ((end (search-forward s nil t)))
          (and end (cons (- end len) end)))
      (let ((start (search-backward s nil t)))
        (and start (cons start (+ start len)))))))

(defun evil--goto-span (span forward)
  "Land point where FORWARD's own landing convention says it should:
just past the match (Emacs `search-forward' style) or on its first
character (`search-backward' style) -- see the section comment above
for why these differ from vim's own convention."
  (goto-char (if forward (cdr span) (car span))))

(defun evil--sync-match-span (s)
  "Make sure `evil--last-match-span' reflects reality before a repeat
of search string S: if point isn't sitting at either edge of the
cached span, something other than a previous `evil--search-repeat-1'
call moved it since -- in practice, always a FRESH `/'/`?' commit (or
this is the very first `n'/`N' since evil-mode turned on) -- so
rederive the span from point + the ORIGINAL isearch's direction + S's
length. Valid precisely because nothing else ever moves point this
way (isearch's own commit lands exactly at one edge of its match; so
does `evil--search-repeat-1', which keeps the cache in sync as it
goes)."
  (unless (and evil--last-match-span
               (or (= (point) (car evil--last-match-span))
                   (= (point) (cdr evil--last-match-span))))
    (let ((len (length s)))
      (setq evil--last-match-span
            (if (isearch-last-forward-p)
                (cons (- (point) len) (point))
              (cons (point) (+ (point) len)))))))

(defun evil--search-repeat-1 (s forward)
  "One repeat of search string S in direction FORWARD from point: skip
a candidate that is the SAME span as `evil--last-match-span' (can only
happen when reversing direction -- see the section comment above),
then wrap around the buffer (with an echoed notice) if the near end is
reached without a match. A wrapped match is accepted even if it turns
out to equal `evil--last-match-span' too -- that only happens when the
buffer holds exactly ONE occurrence, and landing back on it (with the
notice) is the correct, expected outcome then, not a self-match to
guard against."
  (let ((span (evil--search-one s forward)))
    (when (and span (equal span evil--last-match-span))
      (setq span (evil--search-one s forward)))
    (if span
        (progn
          (setq evil--last-match-span span)
          (evil--goto-span span forward))
      (progn
        (goto-char (if forward (point-min) (point-max)))
        (setq span (evil--search-one s forward))
        (if span
            (progn
              (setq evil--last-match-span span)
              (evil--goto-span span forward)
              (message "Search wrapped"))
          (message "Search failed: %s" s))))))

(defun evil--search-repeat (count invert)
  "Repeat the last COMMITTED isearch COUNT times; INVERT flips the
direction (`N') relative to how that search originally ran (`n')."
  (let ((s (isearch-last-string)))
    (if (not s)
        (message "No previous search")
      (progn
        (evil--sync-match-span s)
        (let ((forward (isearch-last-forward-p)))
          (when invert (setq forward (not forward)))
          (dotimes (i count)
            (evil--search-repeat-1 s forward)))))))

(defun evil-search-next ()  ; n
  (interactive)
  (evil--search-repeat (evil--total-count) nil))

(defun evil-search-previous ()  ; N
  (interactive)
  (evil--search-repeat (evil--total-count) t))

;; --- Visual state --------------------------------------------------------

(defun evil--visual-exit ()
  ;; M45 `gv': snapshot BEFORE `deactivate-mark' -- see `evil--last-
  ;; visual''s own docstring for why THIS site alone isn't enough (the
  ;; operator path below, `evil--visual-apply', bypasses this function
  ;; entirely and needs its own identical snapshot).
  (setq-local evil--last-visual (list (point) (mark) evil--visual-type))
  (deactivate-mark)
  (evil--set-state 'normal))

(defun evil-visual-char ()  ; v
  (interactive)
  (if (and (eq evil--state 'visual) (eq evil--visual-type 'char))
      (evil--visual-exit)
    (progn
      (unless (eq evil--state 'visual) (set-mark (point)))
      (setq-local evil--visual-type 'char)
      (evil--set-state 'visual))))

(defun evil-visual-line ()  ; V
  (interactive)
  (if (and (eq evil--state 'visual) (eq evil--visual-type 'line))
      (evil--visual-exit)
    (progn
      (unless (eq evil--state 'visual) (set-mark (point)))
      (setq-local evil--visual-type 'line)
      (evil--set-state 'visual))))

(defun evil--visual-range ()
  (if (mark)
      (cons (min (point) (mark)) (max (point) (mark)))
    (cons (point) (point))))

(defun evil--visual-apply (op)
  (let* ((r (evil--visual-range))
         (linewise (eq evil--visual-type 'line))
         (beg (car r)))
    (setq-local evil--pending-operator op)
    ;; M45 `gv': snapshot BEFORE `deactivate-mark' -- the OPERATOR path
    ;; (visual d/c/y/gu/gU/g~) never calls `evil--visual-exit' at all
    ;; (see that function's own doc comment), so without this line here
    ;; too, `gv' after a visual operator would silently see whatever
    ;; snapshot (if any) an EARLIER plain ESC left behind instead of
    ;; THIS selection -- both sites are required, not just one.
    (setq-local evil--last-visual (list (point) (mark) evil--visual-type))
    (deactivate-mark)
    (evil--operator-apply (car r) (cdr r) 1 linewise)
    (when (eq op 'yank) (goto-char beg))))

(defun evil-visual-delete () (interactive) (evil--visual-apply 'delete))
(defun evil-visual-change () (interactive) (evil--visual-apply 'change))
(defun evil-visual-yank () (interactive) (evil--visual-apply 'yank))
;; M45: visual-state forms of the case operators -- same wrapper shape
;; as delete/change/yank just above.
(defun evil-visual-toggle-case () (interactive) (evil--visual-apply 'toggle-case))
(defun evil-visual-downcase () (interactive) (evil--visual-apply 'downcase))
(defun evil-visual-upcase () (interactive) (evil--visual-apply 'upcase))

(defun evil-visual-swap ()  ; o
  (interactive)
  (let ((m (mark)))
    (when m
      (set-mark (point))
      (goto-char m))))

(defun evil-visual-restore ()  ; gv
  "M45: vim's `gv' -- reselect the most recent visual selection
(`evil--last-visual', snapshotted by `evil--visual-exit'/`evil--visual-
apply' -- see there). Both endpoints clamped to `point-max' in case the
buffer has since shrunk. No previous selection this session
\(`evil--last-visual' nil) just echoes vim's own message.

v1 SIMPLIFICATION (see `evil--last-visual''s own docstring): the
snapshot is two raw buffer POSITIONS, not markers, so the clamp above
only guards against the buffer having shrunk past them -- it does NOT
make this reselect the same TEXT if the buffer was edited in between.
An insertion/deletion before the snapshotted range shifts what is now
there without shifting these numbers, so `gv' can silently reselect
unrelated content instead of erroring or tracking the edit."
  (interactive)
  (if evil--last-visual
      (let ((pt (nth 0 evil--last-visual))
            (mk (nth 1 evil--last-visual))
            (ty (nth 2 evil--last-visual)))
        (set-mark (min mk (point-max)))
        (goto-char (min pt (point-max)))
        (setq-local evil--visual-type ty)
        (evil--set-state 'visual))
    (message "No previous visual selection")))

;; --- Ex commands (`:') ----------------------------------------------------
;;
;; A deliberately small v1: no ranged commands beyond M42-II's own `:s'
;; (`:1,5d' etc. still aren't implemented), no command abbreviation/
;; completion -- just the handful of fixed-name commands the M30 plan
;; calls out (matched EXACTLY, case-sensitively) plus `:s' itself (its
;; own grammar, matched structurally -- see the "M42-II: :s substitute"
;; section below). Typing anything else echoes "Not an editor command:
;; ..." the same way vim's own `:' does, though (since most ranges/
;; multiple commands per line still aren't parsed at all here) against
;; the WHOLE trimmed input rather than just the unrecognized token.
;;
;; Support table:
;;   :w          save-buffer
;;   :q          close this window (if others are open), else attempt
;;               to quit the editor -- reuses `save-buffers-kill-
;;               terminal''s existing "Unsaved: ... -- again to quit
;;               anyway" guard verbatim; see `evil-ex-quit'.
;;   :q!         like `:q' but discards unsaved changes -- see
;;               `evil-ex-quit-force' for the (documented) approximation.
;;   :wq / :x    save-buffer, then the same close/quit as `:q' (`:x' is
;;               NOT special-cased to skip an unmodified save -- a
;;               documented v1 simplification, not a semantic
;;               difference worth the extra code here).
;;   :e PATH     find-file PATH.
;;   :N          (all digits) go to line N.
;;   :$          go to the last line.
;;   :s/PAT/REP/, :%s/PAT/REP/g, :2,4s/PAT/REP/, :'<,'>s/PAT/REP/
;;               M42-II substitute -- see the "M42-II: :s substitute"
;;               section below for the full RANGE/PATTERN/REPL/FLAGS
;;               grammar and the collect-then-apply-descending
;;               execution model.
;; Anything else, including an empty `:' (bare RET), is a no-op other
;; than the unknown-command echo (empty input is silently ignored,
;; matching vim's own `:' + RET doing nothing).
;;
;; Visual state's `:' (`evil-ex-from-visual') leaves visual state FIRST
;; (before the minibuffer even opens, so the mode-line tag is already
;; back to `<N>' while typing the command) rather than after -- vim's
;; own `:' from visual state instead prefills the prompt with a
;; `'<,'>' range and supports range-taking commands like `:'<,'>d'; this
;; v1 still has no general range support (see the plan), so the general
;; approximation remains "drop the selection and run the SAME plain
;; ex-command parser as normal state" -- documented rather than
;; silently pretending ranges work everywhere. M42-II narrows that
;; approximation specifically for `:s': the selection is snapshotted
;; as a (BEG-LINE . END-LINE) pair BEFORE it's dropped, giving `:s'
;; something to fall back on when no explicit range was typed -- see
;; `evil-ex-from-visual' and `evil--ex-visual-range''s own doc comments.

(defun evil--ex-numeric-p (s)
  "Non-nil if S is non-empty and every character is a digit (0-9)."
  (and (> (length s) 0)
       (let ((ok t) (i 0) (n (length s)))
         (while (< i n)
           (let ((c (aref s i)))
             (unless (and (>= c ?0) (<= c ?9)) (setq ok nil)))
           (setq i (1+ i)))
         ok)))

(defun evil--ex-split (s)
  "(CMD . ARG): split S on its first whitespace run. ARG is the
trimmed remainder (\"\" when there is none) -- the `:e PATH' shape."
  (let ((n (length s)) (i 0))
    (while (and (< i n) (/= (aref s i) ?\s)) (setq i (1+ i)))
    (cons (substring s 0 i) (string-trim (substring s (min n (1+ i)) n)))))

(defun evil--ex-goto-line (n)
  (goto-char (evil--pos-goto-line n)))

(defun evil-ex-quit ()
  "Approximates vim's `:q': closes the current window if others remain
open (`delete-window' itself carries no modified-buffer guard to
bypass here -- vim proper actually does warn on a plain `:q' with
unsaved changes in a split window too, a nuance this editor's window
layer doesn't implement; noted, not added, out of scope for M30), else
attempts a full quit by calling `save-buffers-kill-terminal' AS A PLAIN
FUNCTION CALL -- reusing its existing \"Unsaved: ... -- again to quit
anyway\" guard rather than reimplementing an unsaved-changes check, but
deliberately NOT via `command-execute' (see `evil-ex-quit-force''s
docstring for why that would be actively wrong here): a direct call
never touches `this-command'/`last-command' at all, so `save-buffers-
kill-terminal''s own `again' check (which reads `last-command') always
sees whatever the ACTUAL previous command was -- meaning an unsaved
`:q' warns EVERY time it's pressed, never auto-confirming itself no
matter how many times in a row, matching vim's own `:q' (always errors
on modified; `:q!' is required to force it)."
  (if (> (window-count) 1)
      (delete-window)
    (save-buffers-kill-terminal)))

(defun evil-ex-quit-force ()
  "Approximates vim's `:q!' (discard-and-quit). Multiple windows: same
as `evil-ex-quit' (no guard to bypass at this level regardless of the
bang). Last window: force a full quit even with unsaved buffers by
calling `(save-buffers-kill-terminal t)' -- the FORCE argument added to
that builtin (editing.rs) specifically for this, skipping its unsaved-
changes guard outright.

M30 review fix (issue 1, high severity -- silent data loss): this used
to invoke `(command-execute (quote save-buffers-kill-terminal))' TWICE
in a row, exploiting the SAME \"call it again to force\" escape hatch
`save-buffers-kill-terminal' offers the interactive C-x C-c binding
\(mirroring how `evil-redo' forces `last-command' via `command-execute'
-- see the file header's redo section). That trick had a serious,
non-obvious side effect: `execute_command' (commands.rs) sets
`last-command' as an ORDINARY side effect of running ANY command, so
the nested `command-execute' call left `last-command' pointing at
`save-buffers-kill-terminal' even after just the FIRST `:q' (not `:q!')
a user typed -- poisoning `save-buffers-kill-terminal''s `again' check
for whatever ran next: a SECOND plain `:q' (typed with no `!', still
expecting just another warning) would silently force-quit and lose the
unsaved buffer instead, and so would the very next REAL, interactive
C-x C-c press. An explicit FORCE argument has no such cross-command
side effect -- it's read once, right there, and never touches
`last-command'. `evil-ex-quit' above was changed the same way (a plain
funcall, no `command-execute' at all) for the identical reason, even
though it doesn't force anything itself: merely being invoked was
enough to corrupt `last-command' for whatever ran afterward.

Approximation, not a byte-for-byte port of vim's semantics: real vim's
`:q!' discards ONLY the current buffer's changes and never touches
other buffers' unsaved state; this editor's only \"force quit\"
primitive is global (all buffers), so `:q!' here behaves closer to
vim's `:qa!' whenever OTHER buffers also have unsaved changes.
Documented, not silently approximated."
  (if (> (window-count) 1)
      (delete-window)
    (save-buffers-kill-terminal t)))

;; --- M42-II: :s substitute -----------------------------------------------
;;
;; Grammar: `(RANGE)?s/PATTERN/REPL(/FLAGS)?', matched structurally (not
;; by exact-string comparison the way the fixed-name commands above
;; are) BEFORE `evil--run-ex' tries anything else, since `evil--ex-
;; split''s whitespace-based split has no notion of this shape at all.
;; `/' is the only supported delimiter; `\/' inside PATTERN/REPL is a
;; literal `/' (the ONE piece of escaping this parser itself
;; understands -- any OTHER `\X', including the regex engine's own
;; `\(' `\)' `\|' `\{n,m\}' syntax, passes through untouched, both
;; characters, straight to `re-search-forward'). RANGE is empty
;; (current line), `%' (whole buffer), or `ADDR' / `ADDR,ADDR' where
;; ADDR is a bare line number, `$' (last line), `.' (current line),
;; `\='<' or `\='>' (the visual-selection snapshot -- see
;; `evil--ex-visual-range'). FLAGS supports only `g' (this engine has
;; no case-fold to give a `c'/`i' flag any meaning).
;;
;; Execution (`evil--ex-run-substitute') follows `verilog-delete-auto''s
;; own house precedent (verilog-auto.el): this codebase's
;; `re-search-forward' DOES accept a BOUND argument in its position
;; (`(defun(interp, "re-search-forward", 1, Some(3), ...)'), but the
;; builtin's body silently ignores it -- only `a[0]' (the pattern) and
;; `opt(a, 2)' (NOERROR) are ever read, `a[1]' (BOUND) is dropped on the
;; floor (`builtins/editing.rs'; implementing BOUND is a separate
;; milestone's work, not done here). Since there is no BOUND to lean on
;; -- whatever gets passed at that position is a no-op -- and
;; `Buffer::search_text''s snapshot is invalidated by every single edit,
;; EVERY match across the whole RANGE is collected FIRST against the
;; untouched buffer (one `evil--expand-replacement' call per match,
;; using that match's OWN, still-current match data), each line's own
;; "only up to its own `line-end-position'" bound enforced BY HAND
;; (`re-search-forward''s BOUND argument slot exists but does nothing).
;; Only once collection is complete are the
;; matches applied, back-to-front (descending START), as plain
;; `delete-region'+`insert' pairs -- descending order means every edit
;; happens at a position strictly AFTER every match still waiting to be
;; applied, so none of THEIR already-recorded positions are ever
;; invalidated by an earlier (in application order) edit.

(defvar evil--ex-visual-range nil
  "(BEG-LINE . END-LINE), 1-based -- the most recent visual selection's
line span, snapshotted by `evil-ex-from-visual' BEFORE `evil--visual-
exit' clears the selection. Backs BOTH the `\\='<'/`\\='>' RANGE
addresses in a `:s' typed at any later time (matching real vim: these
persist across a mode change until a NEW visual selection redefines
them) and the auto-range convenience below (see `evil--ex-from-visual').
Global, like the rest of this file's search/ex/dot-repeat state --
vim's `\\='<'/`\\='>' marks are editor-wide, not per-buffer, here.")

(defvar evil--ex-from-visual nil
  "Non-nil for exactly the ONE `evil--run-ex' call following an
`evil-ex-from-visual' invocation (`:' pressed FROM visual state) --
read once, then unconditionally cleared, by `evil--run-ex' itself, so
it can never leak into a later, unrelated `:' typed from normal state.
The sole reason this exists (as opposed to just always consulting
`evil--ex-visual-range'): a bare `s///' with NO explicit range should
auto-apply the just-left selection ONLY when it immediately followed
that selection -- vim's own `:'<,'>s///' prefill achieves the same
effect by literally typing the range into the prompt, which this
editor's minibuffer has no way to do (see `evil-ex-from-visual''s doc
comment) -- but a PLAIN `:s///' typed later, long after any visual
session, must default to the current line as usual, not silently
resurrect a stale selection from several commands ago.")

(defun evil--ex-range-char-p (c)
  "Non-nil if C can appear inside a `:s' RANGE (see `evil--ex-parse-
substitute'). Deliberately excludes `s' itself -- `s' is exactly what
marks where RANGE ends and the `s/PATTERN/REPL/FLAGS' command proper
begins, so a RANGE can never absorb it."
  (or (and (>= c ?0) (<= c ?9))
      (memq c (list ?, ?$ ?. ?\' ?< ?> ?%))))

(defun evil--ex-scan-delim (s start)
  "From S starting at START, collect characters up to (excluding) the
first unescaped `/' -- `\\/' becomes a literal `/' in the result (this
parser's OWN delimiter escape, distinct from whatever escape
convention the regex engine gives `\\(' etc. -- any other `\\X' passes
through untouched, both characters, so the regex/template layers
downstream see it exactly as the user typed it). Returns (SEGMENT FOUND
NEXT): FOUND is non-nil iff an unescaped `/' actually terminated the
segment (NEXT is the index just past it); when FOUND is nil, SEGMENT
ran to (length S) with no closing delimiter and NEXT equals that (the
trailing-FLAGS-less `:s/a/b' shape, valid only for the FINAL segment --
see `evil--ex-parse-substitute')."
  (let ((n (length s)) (i start) (out ""))
    (while (and (< i n) (/= (aref s i) ?/))
      (if (and (= (aref s i) ?\\) (< (1+ i) n) (= (aref s (1+ i)) ?/))
          (progn (setq out (concat out "/")) (setq i (+ i 2)))
        (progn (setq out (concat out (char-to-string (aref s i)))) (setq i (1+ i)))))
    (if (< i n) (list out t (1+ i)) (list out nil i))))

(defun evil--ex-parse-substitute (s)
  "Recognize S as `(RANGE)?s/PATTERN/REPL(/FLAGS)?' (see the section
comment above for the full grammar). Returns (RANGE-STR PATTERN REPL
FLAGS) on a syntactic match (RANGE-STR/FLAGS may be \"\"), or nil if S
doesn't have this shape at all -- `evil--run-ex' falls through to try S
as some other ex command in that case, exactly as if this function
didn't exist (a malformed `:s' -- e.g. PATTERN never closed -- is
indistinguishable, at this layer, from \"not a `:s' at all\": both
surface as the same existing \"Not an editor command\" echo)."
  (let ((n (length s)) (i 0))
    (while (and (< i n) (evil--ex-range-char-p (aref s i)))
      (setq i (1+ i)))
    (if (or (>= (1+ i) n) (/= (aref s i) ?s) (/= (aref s (1+ i)) ?/))
        nil
      (let* ((range-str (substring s 0 i))
             (pat-r (evil--ex-scan-delim s (+ i 2))))
        (if (not (nth 1 pat-r))
            nil
          (let* ((pattern (nth 0 pat-r))
                 (rep-r (evil--ex-scan-delim s (nth 2 pat-r))))
            (if (nth 1 rep-r)
                (list range-str pattern (nth 0 rep-r) (substring s (nth 2 rep-r)))
              (list range-str pattern (nth 0 rep-r) ""))))))))

(defun evil--ex-resolve-addr (str)
  "One RANGE endpoint STR -> a 1-based line number, or nil if STR isn't
one of the recognized address forms (a digit run, `$', `.', `\\='<',
`\\='>')."
  (cond
    ((string= str "$") (evil--total-lines))
    ((string= str ".") (line-number-at-pos))
    ((string= str "'<") (and evil--ex-visual-range (car evil--ex-visual-range)))
    ((string= str "'>") (and evil--ex-visual-range (cdr evil--ex-visual-range)))
    ((evil--ex-numeric-p str) (string-to-number str))
    (t nil)))

(defun evil--ex-resolve-range-endpoint (str total)
  "One RANGE endpoint STR -> a validated/clamped 1-based line number,
or nil if STR names an unrecognized address, OR an out-of-bounds
EXPLICIT NUMERIC address (see `evil--ex-resolve-range''s own doc
comment for why those two are treated alike here but a `\\='<'/`\\='>'
address out of bounds is handled completely differently, one call
below)."
  (let ((n (evil--ex-resolve-addr str)))
    (and n
         (if (evil--ex-numeric-p str)
             ;; A literal line number the user typed (`:0,1s', `:99s',
             ;; ...) outside the buffer is a plain user error -- vim's
             ;; own E16 "Invalid range" -- so this is rejected (nil)
             ;; rather than silently clamped: `evil--goto-line' happily
             ;; (mis)treats an out-of-range N by clamping via
             ;; `forward-line', which is exactly the bug this guards
             ;; against (`:0,1s' previously walked line 0 AND line 1
             ;; to the SAME real line 1, collecting -- and then
             ;; applying -- the same match twice).
             (and (>= n 1) (<= n total) n)
           ;; `$'/`.' can never actually leave [1,TOTAL] on their own
           ;; (they're derived from the CURRENT buffer), so clamping is
           ;; a no-op for them either way. A `\\='<'/`\\='>' address is
           ;; the one case this matters for: it names a SNAPSHOT
           ;; (`evil--ex-visual-range', taken by `evil-ex-from-visual')
           ;; that can legitimately go stale if lines were deleted
           ;; after the visual selection was taken but before `:s' ran
           ;; -- an ordinary, expected situation (see
           ;; `evil--ex-visual-range''s own doc comment), NOT a user
           ;; typo, so unlike the numeric-literal case above it
           ;; degrades gracefully into the nearest still-valid line
           ;; rather than aborting the whole command.
           (max 1 (min n total))))))

(defun evil--ex-resolve-range (range-str)
  "(BEG-LINE . END-LINE) for RANGE-STR (the first element of
`evil--ex-parse-substitute''s return value), or nil if RANGE-STR names
an unrecognized address OR an explicit numeric address outside
[1,`evil--total-lines'] -- see `evil--ex-resolve-range-endpoint' for
why THAT case is a hard rejection while an out-of-bounds `\\='<'/`\\='>'
snapshot address is clamped into range instead, right there rather
than rejected here too."
  (cond
    ((string= range-str "") (let ((n (line-number-at-pos))) (cons n n)))
    ((string= range-str "%") (cons 1 (evil--total-lines)))
    (t
     (let ((comma (string-match "," range-str))
           (total (evil--total-lines)))
       (if comma
           (let ((b (evil--ex-resolve-range-endpoint (substring range-str 0 comma) total))
                 (e (evil--ex-resolve-range-endpoint (substring range-str (1+ comma)) total)))
             (and b e (cons b e)))
         (let ((n (evil--ex-resolve-range-endpoint range-str total)))
           (and n (cons n n))))))))

(defun evil--ex-clamp-visual-range (r)
  "Clamp R (a raw (BEG-LINE . END-LINE) `evil--ex-visual-range'
snapshot) into [1,`evil--total-lines'] -- the auto-apply-from-visual
counterpart of `evil--ex-resolve-range-endpoint''s `\\='<'/`\\='>'
clamp, for `evil--ex-run-substitute''s direct-use shortcut (empty
RANGE-STR right after leaving visual state), which reads the same
snapshot without going through address-string resolution at all."
  (let ((total (evil--total-lines)))
    (cons (max 1 (min (car r) total)) (max 1 (min (cdr r) total)))))

(defun evil--ex-flags-global-p (flags)
  "Non-nil iff FLAGS contains `g' -- the only `:s' flag this engine
understands (see the section comment above)."
  (let ((n (length flags)) (i 0) (g nil))
    (while (< i n)
      (when (= (aref flags i) ?g) (setq g t))
      (setq i (1+ i)))
    g))

(defun evil--expand-replacement (repl)
  "Expand REPL (a `:s' REPL segment) against the CURRENT match data --
callers always invoke this immediately after the `re-search-forward'
whose match it should expand, before any LATER search overwrites that
match data (see `evil--ex-collect-substitutions'). `\\1'..`\\9' insert
capture group N (`match-string'), `\\&' the whole match (`match-string'
0), `\\\\' a literal backslash; any OTHER `\\X' is left exactly as
typed, both characters -- a deliberately permissive default for
whatever escapes aren't otherwise recognized (v1: not exercised by any
of this file's own `:s' tests, so not pinned more precisely than
that)."
  (let ((n (length repl)) (i 0) (out ""))
    (while (< i n)
      (if (and (= (aref repl i) ?\\) (< (1+ i) n))
          (let ((d (aref repl (1+ i))))
            (cond
              ((= d ?&)
               (setq out (concat out (or (match-string 0) ""))) (setq i (+ i 2)))
              ((and (>= d ?1) (<= d ?9))
               (setq out (concat out (or (match-string (- d ?0)) "")))
               (setq i (+ i 2)))
              ((= d ?\\) (setq out (concat out "\\")) (setq i (+ i 2)))
              (t (setq out (concat out "\\" (char-to-string d))) (setq i (+ i 2)))))
        (progn (setq out (concat out (char-to-string (aref repl i)))) (setq i (1+ i)))))
    out))

(defun evil--ex-collect-substitutions (beg-line end-line pattern repl global)
  "Collect every `:s' match in [BEG-LINE,END-LINE] (inclusive, 1-based)
against the CURRENT, still-unedited buffer. Returns (MATCHES TOTAL
HIT-LINES MAX-LINE): MATCHES is a list of (START END . TEXT) triples in
forward (top-to-bottom) order, TEXT already expanded
(`evil--expand-replacement') against each match's own data; TOTAL is
its length; HIT-LINES is how many distinct lines had >=1 match;
MAX-LINE is the highest line number that had one (nil if none --
i.e. TOTAL is 0), for `evil--ex-run-substitute' to land point on
afterward (vim: `:s' leaves point on the LAST substituted line).

`re-search-forward' accepts a BOUND argument position but ignores it
(the builtin only reads the pattern and NOERROR args -- see
`builtins/editing.rs' and the longer note above `evil--ex-run-
substitute') -- \"only within this line\" is therefore enforced by hand
below, by comparing every match's `match-beginning' against this
line's own `line-end-position' and discarding (not just stopping at)
any match that starts beyond it. GLOBAL nil stops after a line's first match;
GLOBAL non-nil keeps searching from `(max match-end (1+ match-
beginning))' (the `1+' is the zero-width-match advance guard -- without
it, a pattern that can match zero-width, e.g. `x*', would refind the
SAME position forever)."
  (let ((matches nil) (total 0) (hit-lines 0) (max-line nil) (line beg-line))
    (while (<= line end-line)
      (evil--goto-line line)
      (let ((eol (line-end-position)) (hit nil) (continue t))
        (while continue
          (setq continue nil)
          (when (re-search-forward pattern nil t)
            (let ((ms (match-beginning 0)) (me (match-end 0)))
              (when (< ms eol)
                (setq matches
                      (cons (cons ms (cons me (evil--expand-replacement repl))) matches))
                (setq total (1+ total))
                (setq hit t)
                (when global
                  (goto-char (max me (1+ ms)))
                  (setq continue (< (point) eol)))))))
        (when hit
          (setq hit-lines (1+ hit-lines))
          (setq max-line line)))
      (setq line (1+ line)))
    (list (nreverse matches) total hit-lines max-line)))

(defun evil--ex-run-substitute (parsed from-visual)
  "PARSED is (RANGE-STR PATTERN REPL FLAGS), `evil--ex-parse-
substitute''s return value; FROM-VISUAL mirrors `evil--ex-from-visual'
\(read by the caller before this runs -- see its own doc comment).
Resolves RANGE (auto-applying the M42-II visual-selection snapshot when
RANGE-STR is empty AND FROM-VISUAL is non-nil), an empty PATTERN
against `(isearch-last-string)' (vim's `:s//x/' -- reuse the last
search), collects every match (`evil--ex-collect-substitutions'), then
applies them back-to-front as a single `delete-region'+`insert' undo
group per `verilog-delete-auto''s own precedent (`undo-amalgamate-
boundary', matching this section's own doc comment for why one call
suffices even across many individual edits). Sets `Editor::last_search'
\(`isearch-set-last') to PATTERN on success, so `n'/`N' can repeat it."
  (let* ((range-str (nth 0 parsed))
         (pattern (nth 1 parsed))
         (repl (nth 2 parsed))
         (flags (nth 3 parsed))
         (global (evil--ex-flags-global-p flags))
         (real-pattern (if (string= pattern "") (isearch-last-string) pattern)))
    (if (not real-pattern)
        (message "No previous regular expression")
      (let ((range (if (and (string= range-str "") from-visual evil--ex-visual-range)
                        ;; Same staleness story as the `\\='<'/`\\='>'
                        ;; clamp in `evil--ex-resolve-range-endpoint' --
                        ;; this auto-apply shortcut reads the identical
                        ;; `evil--ex-visual-range' snapshot, just
                        ;; without going through the explicit
                        ;; `\\='<,\\='>' address syntax, so it can go
                        ;; stale the exact same way and must clamp
                        ;; rather than hand a since-shrunk buffer an
                        ;; out-of-range line to walk.
                        (evil--ex-clamp-visual-range evil--ex-visual-range)
                      (evil--ex-resolve-range range-str))))
        (if (not range)
            (message "Invalid range")
          (let* ((beg-line (min (car range) (cdr range)))
                 (end-line (max (car range) (cdr range)))
                 (result (evil--ex-collect-substitutions beg-line end-line real-pattern repl global))
                 (matches (nth 0 result))
                 (total (nth 1 result))
                 (hit-lines (nth 2 result))
                 (max-line (nth 3 result)))
            (if (= total 0)
                (message "Pattern not found: %s" real-pattern)
              (progn
                (dolist (m (sort (copy-sequence matches)
                                  (lambda (a b) (> (car a) (car b)))))
                  (delete-region (car m) (cadr m))
                  (goto-char (car m))
                  (insert (cddr m)))
                (undo-amalgamate-boundary)
                (isearch-set-last real-pattern t)
                (evil--goto-line max-line)
                (goto-char (evil--pos-first-non-blank))
                (message "Substituted %d occurrence(s) on %d line(s)" total hit-lines)))))))))

(defun evil--run-ex (input)
  ;; M42-II: `evil--ex-from-visual' is read here, ONCE, then
  ;; unconditionally cleared -- regardless of what INPUT turns out to
  ;; be -- so it can never survive to affect some LATER, unrelated `:'
  ;; (see its own doc comment).
  (let ((s (string-trim input))
        (from-visual evil--ex-from-visual)
        (sub nil))
    (setq evil--ex-from-visual nil)
    (setq sub (evil--ex-parse-substitute s))
    (cond
      ((string= s "") nil)
      (sub (evil--ex-run-substitute sub from-visual))
      ((evil--ex-numeric-p s) (evil--ex-goto-line (string-to-number s)))
      ((string= s "$") (evil--ex-goto-line (evil--total-lines)))
      (t
       (let* ((split (evil--ex-split s)) (cmd (car split)) (arg (cdr split)))
         (cond
           ((string= cmd "w") (save-buffer))
           ((string= cmd "w!") (save-buffer t))
           ((string= cmd "q") (evil-ex-quit))
           ((string= cmd "q!") (evil-ex-quit-force))
           ((or (string= cmd "wq") (string= cmd "x"))
            (save-buffer)
            (evil-ex-quit))
           ((string= cmd "e")
            (if (string= arg "")
                (message "e: file name required")
              (find-file arg)))
           (t (message "Not an editor command: %s" s))))))))

(defun evil-ex-command (input)  ; normal state `:'
  (interactive "s:")
  (evil--run-ex input))

(defun evil-ex-from-visual ()  ; visual state `:'
  "M42-II: snapshots the selection's line span into `evil--ex-visual-
range' and arms `evil--ex-from-visual' BEFORE `evil--visual-exit' drops
it -- `:s' reads both (see `evil--ex-run-substitute'). The snapshot is
just (line-number-at-pos of each end of `evil--visual-range''s raw
char-position pair): correct regardless of `evil--visual-type' (V's
selection already spans whole lines however point/mark happen to sit
within them; a plain `v' charwise selection is snapped to whole lines
too, matching vim's own `:s' range semantics -- it is always
line-based no matter which visual sub-mode produced it)."
  (interactive)
  (let ((r (evil--visual-range)))
    (setq evil--ex-visual-range
          (cons (line-number-at-pos (car r)) (line-number-at-pos (cdr r)))))
  (setq evil--ex-from-visual t)
  (evil--visual-exit)
  (command-execute 'evil-ex-command))

;; --- C-w window prefix (M45) ----------------------------------------------
;;
;; vim's `C-w' window-prefix commands, each a thin wrapper around a window
;; primitive that already existed before M45 (splitting/other-window/
;; delete-window/delete-other-windows are the same Rust-backed builtins
;; C-x 2/3/o/0/1 already use, see simple.el; `evil-ex-quit' above is
;; already vim's `:q' semantics) -- this section just rebinds them under
;; vim's OWN prefix, plus the one genuinely new primitive, `select-
;; window-in-direction' (builtins/ui.rs), for `h'/`j'/`k'/`l'.
;;
;; Bound ONLY in `evil--normal-map' (`evil--normal-map' IS the buffer's
;; `emulation-keymap' -- see commands.rs's layered keymap lookup): once a
;; SINGLE `C-w' combo is bound here, plain `C-w' resolves to `Prefix' at
;; the emulation layer, and a `Prefix' hit wins outright over the global
;; keymap's own `C-w' -> `kill-region' binding (a `Prefix' hit never
;; falls through to a lower-priority keymap, only `Undefined' does) --
;; so normal state's `C-w' is entirely shadowed the moment this section
;; loads: an unbound combo (e.g. `C-w z') echoes "... is undefined"
;; rather than falling back to `kill-region'. Insert state's `C-w' stays
;; the global `kill-region' (vim's `i_CTRL-W' isn't implemented, v1
;; scope, documented not silently missing); visual state doesn't bind
;; `C-w' either (same v1 scope). No count-prefix support (`3 C-w w'
;; doesn't repeat) -- v1 scope, same as several other places in this
;; file (`u'/C-r, i/a/I/A/o/O, ...).

(defun evil-window-split ()  ; C-w s
  (interactive)
  (split-window-below))

(defun evil-window-vsplit ()  ; C-w v
  (interactive)
  (split-window-right))

(defun evil-window-other ()  ; C-w w, C-w C-w
  (interactive)
  (other-window))

(defun evil-window-delete ()  ; C-w c
  "vim's `E444' semantics for the sole remaining window: `delete-window'
(builtins/ui.rs) already signals an error in that case rather than
deleting it -- an ordinary elisp error signaled from inside an
`interactive' command is caught and echoed by the command dispatcher
\(commands.rs's `call_command'), same as any other command error, so
this needs no special handling here to avoid crashing."
  (interactive)
  (delete-window))

(defun evil-window-quit ()  ; C-w q
  (interactive)
  (evil-ex-quit))

(defun evil-window-only ()  ; C-w o
  (interactive)
  (delete-other-windows))

(defun evil--window-move (dir)
  "Shared body for the four `C-w h/j/k/l' direction commands: DIR is one
of the symbols `left'/`right'/`up'/`down'. `select-window-in-direction'
(builtins/ui.rs) already performs the actual switch (through
`select_window', the one legal window-switch primitive) when it finds
a candidate; nil means it found none, so just echo -- selection itself
is left untouched either way."
  (unless (select-window-in-direction dir)
    (message "No window %s" dir)))

(defun evil-window-left ()  ; C-w h
  (interactive)
  (evil--window-move 'left))

(defun evil-window-down ()  ; C-w j
  (interactive)
  (evil--window-move 'down))

(defun evil-window-up ()  ; C-w k
  (interactive)
  (evil--window-move 'up))

(defun evil-window-right ()  ; C-w l
  (interactive)
  (evil--window-move 'right))

(define-key evil--normal-map "C-w s" 'evil-window-split)
(define-key evil--normal-map "C-w v" 'evil-window-vsplit)
(define-key evil--normal-map "C-w w" 'evil-window-other)
(define-key evil--normal-map "C-w C-w" 'evil-window-other)
(define-key evil--normal-map "C-w c" 'evil-window-delete)
(define-key evil--normal-map "C-w q" 'evil-window-quit)
(define-key evil--normal-map "C-w o" 'evil-window-only)
(define-key evil--normal-map "C-w h" 'evil-window-left)
(define-key evil--normal-map "C-w j" 'evil-window-down)
(define-key evil--normal-map "C-w k" 'evil-window-up)
(define-key evil--normal-map "C-w l" 'evil-window-right)
;; Arrow-key spellings of h/j/k/l -- optional per the M45 plan, included
;; since `select-window-in-direction' already takes the same symbol
;; either way.
(define-key evil--normal-map "C-w <left>" 'evil-window-left)
(define-key evil--normal-map "C-w <down>" 'evil-window-down)
(define-key evil--normal-map "C-w <up>" 'evil-window-up)
(define-key evil--normal-map "C-w <right>" 'evil-window-right)

;; --- Keymap population ---------------------------------------------------

(defun evil--bind-motion (key cmd)
  "Bind KEY to CMD in the normal, visual, AND operator-pending maps --
every motion here is usable from all three (each motion's own body
checks `evil--pending-operator' to know which)."
  (define-key evil--normal-map key cmd)
  (define-key evil--visual-map key cmd)
  (define-key evil--op-pending-map key cmd))

(evil--bind-motion "h" 'evil-backward-char)
(evil--bind-motion "l" 'evil-forward-char)
(evil--bind-motion "j" 'evil-next-line)
(evil--bind-motion "k" 'evil-previous-line)
(evil--bind-motion "w" 'evil-word-forward)
(evil--bind-motion "b" 'evil-word-backward)
(evil--bind-motion "e" 'evil-word-end)
(evil--bind-motion "W" 'evil-WORD-forward)
(evil--bind-motion "B" 'evil-WORD-backward)
(evil--bind-motion "E" 'evil-WORD-end)
(evil--bind-motion "0" 'evil-digit-0-or-bol)
(evil--bind-motion "^" 'evil-first-non-blank)
(evil--bind-motion "$" 'evil-end-of-line)
(evil--bind-motion "g g" 'evil-goto-first-line)
(evil--bind-motion "G" 'evil-goto-line-or-last)
(evil--bind-motion "{" 'evil-paragraph-backward)
(evil--bind-motion "}" 'evil-paragraph-forward)
(evil--bind-motion "f" 'evil-find-char-forward)
(evil--bind-motion "F" 'evil-find-char-backward)
(evil--bind-motion "t" 'evil-till-char-forward)
(evil--bind-motion "T" 'evil-till-char-backward)
(evil--bind-motion ";" 'evil-repeat-find)
(evil--bind-motion "," 'evil-repeat-find-reverse)
(evil--bind-motion "C-d" 'evil-scroll-down)
(evil--bind-motion "C-u" 'evil-scroll-up)
(evil--bind-motion "1" 'evil-digit-1)
(evil--bind-motion "2" 'evil-digit-2)
(evil--bind-motion "3" 'evil-digit-3)
(evil--bind-motion "4" 'evil-digit-4)
(evil--bind-motion "5" 'evil-digit-5)
(evil--bind-motion "6" 'evil-digit-6)
(evil--bind-motion "7" 'evil-digit-7)
(evil--bind-motion "8" 'evil-digit-8)
(evil--bind-motion "9" 'evil-digit-9)
;; M42: `` ` ''/`'' -- see the "M42: marks" section above for why these
;; go through `evil--bind-motion' (like `f'/`t') rather than being
;; normal-map-only the way `m' just below is.
(evil--bind-motion "`" 'evil-goto-mark-exact)
(evil--bind-motion "'" 'evil-goto-mark-line)
;; M45: g_ / ge / gE -- motions like every other `evil--bind-motion'
;; entry above, so `d g _'/`d g e'/`d g E' all work the same way `d $'/
;; `d e' already do.
(evil--bind-motion "g _" 'evil-last-non-blank)
(evil--bind-motion "g e" 'evil-end-of-prev-word)
(evil--bind-motion "g E" 'evil-end-of-prev-WORD)

;; Normal-state-only bindings.
(define-key evil--normal-map "d" 'evil-op-delete)
(define-key evil--normal-map "c" 'evil-op-change)
(define-key evil--normal-map "y" 'evil-op-yank)
(define-key evil--normal-map "x" 'evil-delete-char)
(define-key evil--normal-map "X" 'evil-delete-char-backward)
(define-key evil--normal-map "s" 'evil-substitute-char)
(define-key evil--normal-map "D" 'evil-delete-to-eol)
(define-key evil--normal-map "C" 'evil-change-to-eol)
(define-key evil--normal-map "Y" 'evil-yank-line)
(define-key evil--normal-map "S" 'evil-change-line)
(define-key evil--normal-map "J" 'evil-join)
(define-key evil--normal-map "r" 'evil-replace-char)
(define-key evil--normal-map "~" 'evil-flip-case)
(define-key evil--normal-map "p" 'evil-paste-after)
(define-key evil--normal-map "P" 'evil-paste-before)
;; M42: `m' (set a mark) and `"' (arm a named-register prefix) -- both
;; normal-map-only, see the "M42: marks"/"M42: named registers"
;; section comments above for why (marks are never operator targets
;; themselves; a register prefix always precedes the operator, never
;; follows it).
(define-key evil--normal-map "m" 'evil-set-mark)
(define-key evil--normal-map "\"" 'evil-use-register)
;; M42-II: `q' (toggle kbd-macro recording) / `@' (replay one) -- same
;; normal-map-only reasoning as `m'/`"' just above: neither composes
;; with an operator, so neither belongs in `evil--op-pending-map' or
;; `evil--bind-motion'.
(define-key evil--normal-map "q" 'evil-record-macro)
(define-key evil--normal-map "@" 'evil-execute-macro)
(define-key evil--normal-map "u" 'undo)
(define-key evil--normal-map "C-r" 'evil-redo)
(define-key evil--normal-map "." 'evil-repeat-change)
(define-key evil--normal-map "i" 'evil-insert)
(define-key evil--normal-map "a" 'evil-append)
(define-key evil--normal-map "I" 'evil-insert-bol)
(define-key evil--normal-map "A" 'evil-append-eol)
(define-key evil--normal-map "o" 'evil-open-below)
(define-key evil--normal-map "O" 'evil-open-above)
(define-key evil--normal-map "v" 'evil-visual-char)
(define-key evil--normal-map "V" 'evil-visual-line)
(define-key evil--normal-map "/" 'isearch-forward)
(define-key evil--normal-map "?" 'isearch-backward)
(define-key evil--normal-map "n" 'evil-search-next)
(define-key evil--normal-map "N" 'evil-search-previous)
(define-key evil--normal-map ":" 'evil-ex-command)
(define-key evil--normal-map "ESC" 'evil-normal-esc)

;; M45 g-prefix, normal-map-only entries: `gd' (go to definition), `gi'
;; (resume last insert), `gJ' (join without a space), `gv' (reselect
;; last visual), and the three case-operator STARTERS `g~'/`gu'/`gU'
;; (same normal-map-only reasoning as `d'/`c'/`y' above -- the
;; corresponding visual-state forms are bound in the visual-state
;; section below, and the operator-pending doubling shortcuts guu/gUU/
;; g~~ + gugu/gUgU/g~g~ are bound in the operator-pending section
;; further down). `gd' has no client of its own here -- `lsp-
;; definition-at-point' (lsp.el) already echoes when there is none, the
;; same as `M-.' (simple.el).
(define-key evil--normal-map "g d" 'lsp-definition-at-point)
(define-key evil--normal-map "g i" 'evil-goto-last-insert)
(define-key evil--normal-map "g J" 'evil-join-no-space)
(define-key evil--normal-map "g v" 'evil-visual-restore)
(define-key evil--normal-map "g ~" 'evil-op-toggle-case)
(define-key evil--normal-map "g u" 'evil-op-downcase)
(define-key evil--normal-map "g U" 'evil-op-upcase)

;; M36 review fix (severity: high -- newly introduced by M36 itself):
;; TAB is now a REAL global keybinding (`indent-for-tab-command', see
;; indent.el), not plain self-insert -- so M34's `inhibit-self-insert'
;; (which only ever guards the self-insert FALLBACK inside
;; `dispatch_key''s `Undefined' arm, never an actual keymap hit) can no
;; longer stop it the way it stops an ordinary unbound character. Left
;; unbound here, normal state's TAB would silently either insert a
;; literal tab (`*scratch*', no `indent-line-function') or reindent the
;; current line with zero visual feedback (any prog-mode buffer) --
;; neither is a thing vim's own normal-state TAB does (real vim uses it
;; for jumplist navigation, C-i/C-o -- not implemented here). Bound to
;; `evil--tab-undefined' instead: touches nothing, echoes exactly the
;; message plain self-insert-rejection already gives any other unbound
;; key in this state.
(define-key evil--normal-map "TAB" 'evil--tab-undefined)
;; M36 review fix (severity: high -- existing hole, M36 made it worse):
;; no evil map ever bound RET, so it fell through to local/global --
;; `newline'/`newline-and-indent' -- meaning normal state's RET
;; literally inserted (and, since M36, also silently reindented) text
;; into the buffer. Real vim's normal-state RET moves to the next
;; line's first non-blank character (`evil-ret', equivalent to `+').
;; The hole existed before M36 too (RET = plain `newline'); M36 is what
;; turned "wrong motion" into "wrong motion AND a silent reindent side
;; effect", so this fix belongs here.
(define-key evil--normal-map "RET" 'evil-ret)

;; Insert-state: ESC, plus M31's dabbrev completion (C-n/C-p). Every
;; other key still falls through to the local/global keymaps and
;; ordinary self-insert (documented simplification relative to upstream
;; evil, see the file header).
(define-key evil--insert-map "ESC" 'evil-insert-exit)
(define-key evil--insert-map "C-n" 'evil-complete-next)
(define-key evil--insert-map "C-p" 'evil-complete-previous)

;; Visual-state-only bindings (motions come from `evil--bind-motion').
(define-key evil--visual-map "ESC" 'evil--visual-exit)
(define-key evil--visual-map "d" 'evil-visual-delete)
(define-key evil--visual-map "x" 'evil-visual-delete)
(define-key evil--visual-map "c" 'evil-visual-change)
(define-key evil--visual-map "s" 'evil-visual-change)
(define-key evil--visual-map "y" 'evil-visual-yank)
(define-key evil--visual-map "o" 'evil-visual-swap)
(define-key evil--visual-map "v" 'evil-visual-char)
(define-key evil--visual-map "V" 'evil-visual-line)
(define-key evil--visual-map ":" 'evil-ex-from-visual)
;; M45: visual-state case operators -- same key, same `evil--visual-
;; apply' wrapper shape as d/c/y just above. `gv' itself is a MOTION-
;; like normal-state-only command (reselects and re-ENTERS visual, it
;; doesn't apply from inside one), so it's bound in the normal-map
;; section instead, not here.
(define-key evil--visual-map "g ~" 'evil-visual-toggle-case)
(define-key evil--visual-map "g u" 'evil-visual-downcase)
(define-key evil--visual-map "g U" 'evil-visual-upcase)

;; M36 review fixes, same reasoning as the normal-map bindings above --
;; visual state has the identical exposure (TAB would reindent/self-
;; insert with the selection still active; RET would insert text and
;; leave the selection in an undefined state). `evil-ret' in visual
;; state extends the selection the same way any other motion here does
;; (the mark stays active; see `evil--visual-range' -- no special-
;; casing needed).
(define-key evil--visual-map "TAB" 'evil--tab-undefined)
(define-key evil--visual-map "RET" 'evil-ret)

;; Operator-pending-only bindings: ESC/C-g cancel (C-g via
;; `keyboard-quit-hook', see the file header), same-key doubling
;; (dd/cc/yy), and i/a text objects.
(define-key evil--op-pending-map "ESC" 'evil--cancel-op)
(define-key evil--op-pending-map "d" 'evil--op-current-lines)
(define-key evil--op-pending-map "c" 'evil--op-current-lines)
(define-key evil--op-pending-map "y" 'evil--op-current-lines)
;; M45: `u'/`U'/`~' inside operator-pending are a SHARED, cross-operator
;; keymap slot -- vim's own `guu'/`gUU'/`g~~' whole-line doubling
;; shortcuts (mirroring dd/cc/yy above) -- but ONLY when the operator
;; actually pending is one of the three case ops itself. Without the
;; guard below, binding these three keys unconditionally to `evil--op-
;; current-lines' (the way d/c/y are bound just above) would make e.g.
;; `du' -- `u' is not a motion or text object this implementation has --
;; silently delete the WHOLE CURRENT LINE instead of cancelling the
;; pending `delete' the way any other unclaimed operator-pending key
;; does: `evil--op-current-lines' doesn't look at what key triggered it,
;; only at `evil--pending-operator', so it would happily "double" a
;; `delete' too. That is silent data corruption (a user typing `du'
;; expecting nothing to happen loses the whole line), not just a wrong
;; motion -- hence `evil--case-doubled' guards on `evil--pending-
;; operator' itself before ever calling `evil--op-current-lines',
;; falling back to the exact same cancel+message every other unclaimed
;; key gets (`evil--op-invalid') otherwise. Also bound as the two-key
;; `g u'/`g U'/`g ~' sequences (vim's `gugu'/`gUgU'/`g~g~' spelling of
;; the identical shortcut) -- same guard, same function.
(defun evil--case-doubled ()
  (interactive)
  (if (memq evil--pending-operator '(toggle-case downcase upcase))
      (evil--op-current-lines)
    (progn
      (message "Not an operator or motion")
      (evil--cancel-op))))
(define-key evil--op-pending-map "u" 'evil--case-doubled)
(define-key evil--op-pending-map "U" 'evil--case-doubled)
(define-key evil--op-pending-map "~" 'evil--case-doubled)
(define-key evil--op-pending-map "g u" 'evil--case-doubled)
(define-key evil--op-pending-map "g U" 'evil--case-doubled)
(define-key evil--op-pending-map "g ~" 'evil--case-doubled)
(define-key evil--op-pending-map "i w" 'evil-op-iw)
(define-key evil--op-pending-map "a w" 'evil-op-aw)
(define-key evil--op-pending-map "i \"" 'evil-op-i-quote)
(define-key evil--op-pending-map "a \"" 'evil-op-a-quote)
(define-key evil--op-pending-map "i (" 'evil-op-i-paren)
(define-key evil--op-pending-map "i )" 'evil-op-i-paren)
(define-key evil--op-pending-map "a (" 'evil-op-a-paren)
(define-key evil--op-pending-map "a )" 'evil-op-a-paren)
(define-key evil--op-pending-map "i {" 'evil-op-i-brace)
(define-key evil--op-pending-map "i }" 'evil-op-i-brace)
(define-key evil--op-pending-map "a {" 'evil-op-a-brace)
(define-key evil--op-pending-map "a }" 'evil-op-a-brace)
(define-key evil--op-pending-map "i [" 'evil-op-i-bracket)
(define-key evil--op-pending-map "i ]" 'evil-op-i-bracket)
(define-key evil--op-pending-map "a [" 'evil-op-a-bracket)
(define-key evil--op-pending-map "a ]" 'evil-op-a-bracket)

;; M36 review fixes: TAB was never claimed by `evil--op-pending-claimed'
;; (it's char code 9, outside the 32..126 sweep `evil--install-op-
;; invalid-catchall' covers below) -- bind it explicitly rather than
;; leave it to fall through to the global `indent-for-tab-command' and
;; silently reindent the buffer mid-operator. RET is deliberately NOT
;; wired to `evil-ret' here: real vim's `d<CR>' is a linewise "delete
;; this line and the next" motion (`evil-ret' generalized to an
;; operator target) -- out of v1 scope (see the file header) -- so both
;; just cancel the pending operator exactly like any other unclaimed
;; key, via the existing `evil--op-invalid'.
(define-key evil--op-pending-map "TAB" 'evil--op-invalid)
(define-key evil--op-pending-map "RET" 'evil--op-invalid)

;; M29 review fix (medium-high severity): every printable ASCII
;; character NOT already meaningfully bound above gets `evil--op-invalid'
;; -- without this, pressing e.g. "d p" (`p' isn't an operator motion)
;; fell through emulation/local/global dispatch all the way to ordinary
;; self-insert, INSERTING "p" into the buffer before post-command-hook
;; got a chance to notice the stray key and cancel the pending
;; operator -- silent data corruption, not just a wrong motion. Named
;; keys (arrow keys etc.) are deliberately left alone: they can't
;; self-insert (`commands.rs' `is_self_insert_char' only ever applies to
;; a `Key::Char'), so the worst they do is echo "... is undefined" and
;; leave `operator-pending' parked until the NEXT command that reaches
;; the command loop notices and cancels it -- documented, not fixed
;; (see the file header), since dispatch_key's Undefined branch never
;; runs post-command-hook except via self-insert.
(defconst evil--op-pending-claimed
  (list ?h ?l ?j ?k ?w ?b ?e ?W ?B ?E ?0 ?^ ?$ ?f ?F ?t ?T ?\; ?,
        ?1 ?2 ?3 ?4 ?5 ?6 ?7 ?8 ?9 ?d ?c ?y ?i ?a ?\` ?\' ?g ?u ?U ?~)
  "Printable ASCII characters already meaningfully bound at the TOP
LEVEL of `evil--op-pending-map' by this file's own bindings above:
motions, same-key doubling, and the i/a text-object prefixes (the
bracket/quote characters themselves, e.g. `(' in \"i (\", are bound
inside the i/a sub-keymaps, not here, so they're irrelevant to this
top-level sweep and correctly absent from this list -- a bare `(' with
no `i'/`a' first is not a motion this implementation has, and falls to
`evil--op-invalid' same as any other unclaimed character). Everything
in 32..126 not in this list gets `evil--op-invalid' by
`evil--install-op-invalid-catchall', below.

M42: `` ` ''/`'' (mark motions, bound via `evil--bind-motion' above)
are listed here for the identical reason `f'/`F'/`t'/`T' already are --
without this, `evil--install-op-invalid-catchall' below would silently
overwrite their `evil--op-pending-map' binding with `evil--op-invalid'
right after this file just set it, since it runs LAST and only skips
characters already in this list. `m'/`\"' are deliberately NOT here:
neither is bound in `evil--op-pending-map' at all (both are normal-
map-only, see the keymap population section above), so they correctly
fall through to `evil--op-invalid' via the catchall, same as any other
unclaimed key.

M45: `g' is listed for the SAME prefix-preservation reason as `` ` ''/
`'' above, just one level up -- `g' itself became a top-level PREFIX
entry of `evil--op-pending-map' the moment `evil--bind-motion' bound
`\"g g\"' (long before M45); without `g' in this list, the catchall
below would blow that whole prefix keymap away, taking `d g g'/`d g _'/
`d g e'/`d g E'/`guu''s own `g u' spelling down with it, not just `g'
alone (this was already a latent bug for plain `d g g' before M45 ever
touched this file -- fixed here as a side effect of needing the same
protection for the M45 entries under the same prefix). `u'/`U'/`~' are
listed because M45 bound them directly (see `evil--case-doubled''s own
comment above) -- without this, the catchall would immediately
overwrite that binding with `evil--op-invalid' right after this file
just set it, the same failure mode as every other entry in this list.")

(defun evil--op-invalid ()
  (interactive)
  (message "Not an operator or motion")
  (evil--cancel-op))

(defun evil--tab-undefined ()
  "M36 review fix: bound to TAB in `evil--normal-map'/`evil--visual-map'
so the global `indent-for-tab-command' binding (a REAL keymap command,
not the self-insert fallback `inhibit-self-insert' guards -- see
commands.rs's `dispatch_key') can never run while normal/visual state
is active. Touches nothing; echoes the same message an ordinary
unbound key in these states already gets."
  (interactive)
  (message "TAB is undefined"))

(defun evil--install-op-invalid-catchall ()
  (let ((c 32))
    (while (<= c 126)
      (unless (memq c evil--op-pending-claimed)
        (define-key evil--op-pending-map (char-to-string c) 'evil--op-invalid))
      (setq c (1+ c)))))
(evil--install-op-invalid-catchall)

(provide 'evil)
