;;; lsp.el --- minimal LSP client (M14) over the lsp-* Rust primitives

;; The Rust side (crate::lsp) only spawns the server process and speaks
;; Content-Length-framed JSON-RPC; every protocol concept -- the
;; `initialize` handshake, request/response correlation, document sync,
;; hover, go-to-definition, diagnostics -- lives here in elisp, the same
;; division of labor as M12's treesit.el and M11's library layer.
;;
;; v1 scope, documented rather than silent:
;;  - `lsp-connect`'s initialize handshake and `lsp-hover`/`lsp-definition`
;;    block the caller (via `lsp-wait`) until their response arrives.
;;    That's inherent to "you can't do anything before initialize
;;    finishes" for the handshake; for hover/definition it trades away
;;    the never-blocks-the-editor ethos M8's workers deliver, in exchange
;;    for a client simple enough to fit this milestone. A fully async
;;    version (register a callback, keep editing, dispatch later via
;;    `lsp-process-pending`) is a natural follow-up, not a correctness gap.
;;  - Server-to-client requests other than the ones we send responses to
;;    (e.g. `window/workDoneProgress/create`) are silently dropped rather
;;    than answered. Real servers tolerate an unanswered optional request.
;;  - `textDocument/didChange` (M35) syncs the WHOLE buffer text on every
;;    change -- `contentChanges` is a single entry with no `range`, which
;;    the spec defines as "replace the entire document", legal under any
;;    `TextDocumentSyncKind` a server advertises. Simpler than tracking
;;    incremental ranges, and cheap enough here since the idle pump sends
;;    at most one per buffer per idle tick no matter how many keystrokes
;;    happened since the last one (`lsp--sync-buffer-now` compares
;;    `buffer-modified-tick` against the last-synced tick), not one per
;;    keystroke. Incremental sync is a natural follow-up, not a
;;    correctness gap.
;;  - Versions are `buffer-modified-tick` (M31's monotonic per-buffer edit
;;    counter, bumped by every insert/delete/undo) rather than a
;;    separately maintained counter: it is already exactly "goes up by at
;;    least one every time the text changes", all `version` needs to be.
;;    `lsp-did-open` uses it too, so didOpen and didChange share one
;;    version sequence per buffer instead of each keeping its own.
;;  - `textDocument/didSave` and `textDocument/didClose` (M40): sent by
;;    `lsp--on-after-save`/`lsp--on-kill-buffer`, hung off the editor's
;;    `after-save-hook`/`kill-buffer-hook` (M40's save/kill hook
;;    infrastructure). didSave's params carry only `textDocument` --
;;    no `includeText`/`text` -- so `lsp--on-after-save` runs
;;    `lsp--sync-buffer-now` first to get any unsaved-to-the-server
;;    edits across as a didChange; the server is expected to already
;;    have the saved text by the time didSave arrives. didClose also
;;    drops the closed buffer's URI from the client's diagnostics
;;    alist -- a server may or may not re-publish an empty diagnostics
;;    set for a closed document, so this side clears it either way.
;;  - `textDocument/completion` (M40-4, extended M44-3): `lsp-
;;    completion-at-point` (bound as `completion-at-point`, `C-M-i`)
;;    filters the server's candidates against the identifier prefix
;;    immediately before point and opens `show-completion-popup` (a
;;    cursor-anchored popup drawn on the shared character grid, with
;;    filter-as-you-type narrowing on the Rust side as more is typed --
;;    see `commands::refilter_completion_popup`) with the result.
;;    `completion-at-point` falls back to `dabbrev-expand` when the
;;    buffer has no live LSP client, so `C-M-i` is never a dead key.
;;    M44-3 additions:
;;      - `textEdit`: an item's `textEdit.newText`, when present, wins
;;        over `insertText`/`label` for what gets inserted, and
;;        `textEdit.range.start` (converted via `lsp--pos-at`) wins over
;;        the prefix scan for where a candidate's replacement starts --
;;        each candidate carries its OWN start, so one item can replace
;;        a span starting earlier than another's (member completion
;;        replacing "obj." is the motivating case). `textEdit.range.end`
;;        is NOT honored -- accepting a candidate always deletes
;;        START..point, clamping away from whatever the server's END
;;        was, and `additionalTextEdits` are never applied. Snippet
;;        syntax inside `newText` (`"${1:x}"` and friends) is inserted
;;        as literal text -- v2 territory, not parsed or expanded here.
;;      - `sortText`: candidates are sorted by `sortText` (falling back
;;        to `label` when absent) before filtering, a stable sort so
;;        candidates that tie keep the server's own relative order.
;;      - `isIncomplete`: when a `CompletionList` sets it, this side
;;        remembers that on the popup; typing further while it's open
;;        re-requests completion instead of narrowing the existing list
;;        (see `commands::refilter_completion_popup`).
;;      - Staleness is no longer a `(buffer, buffer-modified-tick,
;;        point)` triple: a reply landing while the user kept typing
;;        WITHIN the same identifier run (`lsp--completion-prefix-
;;        start` unchanged) now opens the popup filtered against
;;        whatever's typed by then, instead of being discarded -- that's
;;        the filter-as-you-type feature, not staleness. A different
;;        buffer, or a run boundary crossed (a space typed, an ESC back
;;        to evil normal state, point moved elsewhere), still discards
;;        the reply silently.
;;      - Fuzzy matching against `filterText` remains v2: filtering is
;;        still an exact, case-sensitive prefix match.
;;    M44 review fixes:
;;      - #1: the identifier-prefix/filter-as-you-type match (both the
;;        initial filter in `lsp--completion-items` and the Rust-side
;;        `commands::refilter_completion_popup`) is anchored at the
;;        POPUP-WIDE identifier-run prefix start, not each candidate's
;;        own `textEdit`-derived start as M44-3 first had it -- a
;;        candidate's `filterText` almost never covers what its own
;;        `textEdit` replaces ahead of the ordinary prefix (postfix
;;        completions replacing "obj." while filtering on just "if",
;;        say), so per-candidate anchoring silently dropped whole
;;        categories of real completions. The per-candidate start is
;;        still what gets deleted on accept -- only the FILTER anchor
;;        moved back to being popup-wide, `show-completion-popup`'s new
;;        PREFIX-START argument.
;;      - #2: `lsp--completion-items` drops non-hash-table elements
;;        (JSON `null` and friends) BEFORE sorting, not just inside the
;;        per-item loop after -- `sort`'s key function called `gethash`
;;        unconditionally, so one malformed element used to signal and
;;        lose the entire reply instead of just being skipped.
;;  - M46: TextEdit application (`replace-region-contents`,
;;    `textdiff.rs`), wired up to `textDocument/formatting`/
;;    `rangeFormatting` (`lsp-format-buffer`/`lsp-format-region`) and
;;    `textDocument/documentSymbol` (`lsp-next-symbol`/
;;    `lsp-previous-symbol`), plus one dispatch fix:
;;      - Two position-conversion systems now coexist on purpose:
;;        `lsp--pos-at`/`lsp--line-character-at` (pre-M46) treat LSP's
;;        `character` as a Unicode scalar-value offset, while the new
;;        `lsp--pos-at-utf16`/`lsp--utf16-character-at`/
;;        `lsp--line-utf16-at` treat it as the UTF-16 code-unit offset
;;        the LSP spec actually specifies. The two only disagree on
;;        lines containing an astral-plane character (emoji and other
;;        supplementary-plane codepoints) before the position in
;;        question. The new functions were used ONLY by the new M46
;;        code paths (TextEdit ranges, documentSymbol ranges) as of
;;        M46; M48's `lsp--diagnostics-at-point` now uses them too
;;        (see its own docstring for why it deliberately still
;;        diverges from `lsp--decorate-buffer` on this point).
;;        `lsp--pos-at`/`lsp--line-character-at` and their three
;;        existing call sites (diagnostic decoration, diagnostic
;;        navigation, completion) are deliberately left alone -- fixing
;;        their encoding is a real improvement but an orthogonal one,
;;        and bundling it into M46 would spread this diff across
;;        completion and diagnostics for no M46-shaped benefit. Fixing
;;        those three call sites (or just deleting the old functions
;;        once nothing depends on them) is a natural follow-up.
;;      - No server-capabilities gating (no "does this server even
;;        support formatting" check before sending the request):
;;        `verible-verilog-ls`, the server this milestone was built and
;;        tested against, advertises `hoverProvider: false` in its
;;        `initialize` response yet answers `textDocument/hover` fine
;;        (confirmed against the real binary) -- so trusting advertised
;;        capabilities would have silently disabled a working feature.
;;        Capability tracking is a real thing to add later, but only
;;        once it's clear which servers' advertisements can be trusted.
;;        M57 CORRECTION: this is no longer true for
;;        `documentFormattingProvider`/`documentRangeFormattingProvider`
;;        -- `lsp-format-buffer`/`lsp-format-region` now gate on those
;;        two keys specifically (see the M57 note further down this
;;        header). The `hoverProvider: false` finding above still stands
;;        and is still why no OTHER capability in this file is gated:
;;        M57's rule is "gate a key only once real-binary testing shows
;;        the gate is both needed and safe", not "gate everything now
;;        that gating exists".
;;      - `lsp-next-symbol'/`lsp-previous-symbol' only step forward/back
;;        through the flattened symbol list, wrapping at the ends, same
;;        as `next-diagnostic'/`previous-diagnostic'. The
;;        `completing-read'-style jump-to-any-symbol picker is
;;        `lsp-goto-symbol-by-name' (M47, below), once this editor grew
;;        `completing-read'/`read-from-minibuffer' at all.
;;      - `lsp--await' still has no timeout (a pre-existing limitation:
;;        a server that never answers a synchronous request hangs the
;;        whole editor) [M65: fixed -- `lsp--await' now bounds its wait
;;        via `lsp-initialize-timeout' and signals instead of blocking
;;        forever; the reasoning below for routing interactive commands
;;        through the async path still holds, since even a bounded
;;        multi-second wait would visibly freeze the editor]. M46's new
;;        interactive commands
;;        (`lsp-format-buffer', `lsp-format-region', `lsp-next-symbol',
;;        `lsp-previous-symbol') all go through `lsp-request-async'
;;        instead, same as every other M46-forward interactive command
;;        in this file -- that sidesteps the hang for anything a user
;;        can trigger from the keyboard, but does not fix `lsp--await'
;;        itself.
;;      - `lsp--dispatch' now surfaces a JSON-RPC error response (a
;;        message with an `"error"' key and no `"result"') via
;;        `message', instead of silently treating it as "succeeded with
;;        an empty result" -- previously a rejected request (unknown
;;        method, malformed params, ...) produced no feedback at all.
;;        The callback/pending contract is unchanged: it still receives
;;        nil (there simply is no `"result"' key to read), so no
;;        existing callback's assumptions change.
;;      - M46 review fix: `lsp-format-buffer'/`lsp-format-region' now
;;        capture `buffer-modified-tick' right after syncing, and refuse
;;        to apply the server's edits (message instead) if the tick
;;        moved before the reply arrived. Unlike hover/definition, these
;;        commands WRITE to the buffer, so applying edits computed
;;        against a snapshot the user has since typed into risks eating
;;        or misplacing the user's own keystrokes -- the existing
;;        buffer-identity check alone (same as hover/definition) only
;;        catches "switched to a different buffer", not "kept typing in
;;        this one".
;;      - M46 review fix: `lsp--flatten-symbols' had a double-reversal
;;        bug (an outer `nreverse' composing wrong with an already
;;        correctly-ordered recursive child result) that silently
;;        reversed each parent's own children relative to each other
;;        while still nesting them correctly between siblings -- masked
;;        end-to-end because the one caller,
;;        `lsp--document-symbol-positions', re-sorts by POS anyway. Fixed
;;        to build the list forward with `append' instead of
;;        push-then-reverse; see its docstring for the worked example.
;;  - `textDocument/didOpen`'s `languageId` (M44): for its whole life
;;    through M40, `lsp` called `lsp-did-open` without a `language-id`
;;    argument, so every buffer -- regardless of major mode -- reported
;;    itself to the server as `"rust"`. Undetected because the servers
;;    exercised by tests up to that point (clangd, pyright) infer the
;;    language from the URI's file extension rather than trusting this
;;    field. Fixed by `lsp-language-id-alist`, a MAJOR-MODE -> languageId
;;    string table `lsp` now consults at its `lsp-did-open` call site; a
;;    mode absent from that table still falls back to `lsp-did-open`'s
;;    own `"rust"` default, so this is purely additive. Also new in M44:
;;    Verilog/SystemVerilog support -- `verilog-mode` registered in
;;    `lsp-server-alist` (default server `verible-verilog-ls`), in
;;    `lsp-language-id-alist` (languageId `"verilog"`), and its project
;;    marker files (`verible.filelist`, `.svls.toml`) added to
;;    `lsp--project-root-markers`.
;;  - `textDocument/documentHighlight` (M49): `lsp-highlight-at-point`
;;    (`C-c l h`) draws every returned range as an `'lsp-highlight'-
;;    tagged overlay with the named face `lsp-highlight` (a background
;;    color, not an underline -- wavy underlines degrade to a plain
;;    line in the TUI, backgrounds render identically in both
;;    frontends); `lsp-highlight-clear` (`C-c l H`) removes them;
;;    `lsp-next-highlight'/`lsp-previous-highlight' (`C-c l N'/`C-c l
;;    P') step between them (wrapping at the ends, message noting the
;;    wrap), scanning `overlays-in' fresh each call rather than caching
;;    a position list that would drift from the live overlays as the
;;    buffer is edited. No `kind' handling (read/write coloring): the
;;    server this was built and tested against (verible-verilog-ls)
;;    never sends it -- confirmed against the real binary -- so relying
;;    on it would silently disable the whole feature; v2 territory once
;;    a `kind'-sending server is in scope. Every request/response
;;    coordinate uses the UTF-16 conversion pair
;;    (`lsp--line-utf16-at'/`lsp--pos-at-utf16', M46), NOT the older
;;    scalar-value pair the sibling hover/definition/rename commands
;;    still use for their own unrelated reasons (see the M46 note
;;    above) -- documentHighlight is a new-in-M49 call site with no
;;    existing behavior to preserve, so it gets the spec-correct
;;    encoding from the start. Highlights persist across cursor
;;    movement/navigation on purpose (unlike `hover_popup', which
;;    clears on the very next keystroke) -- they only disappear via
;;    `lsp-highlight-clear' or the next `lsp-highlight-at-point' reply
;;    (which always clears the old set first, even on an empty/`nil'/
;;    `:null' reply, so a keyword/whitespace query never leaves a
;;    stale highlight on screen). No server-capability gating and no
;;    idle-triggered auto-highlight, same reasoning as M46/M48's own
;;    "no capability gate" and "no timer primitive exists" notes above.
;;  - M54: a NARROW capability gate for `textDocument/completion' only,
;;    plus a buffer-local escape hatch (`local-completion-function') a
;;    mode can use to supply its own, non-LSP completion source ahead
;;    of the server. Both exist because of one concrete finding
;;    (M54 repro, real binary): `verible-verilog-ls' -- this editor's
;;    primary-language server -- has NO `completionProvider' key at all
;;    in its `initialize' response, and a real `textDocument/completion'
;;    request gets back `Unhandled method' on the server's own stderr;
;;    with the M46-era "never gate on capabilities" policy, `C-M-i' in a
;;    connected buffer just sent that doomed request every time and
;;    silently did nothing -- worse than not being connected at all
;;    (unconnected, dabbrev at least tries). `lsp--capability-supported-p'
;;    is still deliberately ASYMMETRIC, not a wholesale reversal of the
;;    M46 policy above: it only distrusts a capabilities hash's ABSENT
;;    key, never a key's own FALSY value, so M46's own finding
;;    (`hoverProvider: false' yet hover answers fine) stays exactly as
;;    trusted as before for every OTHER request this file sends --
;;    `textDocument/hover'/`formatting'/`documentSymbol'/
;;    `documentHighlight' all still go out ungated. See that function's
;;    own docstring for the full absent-vs-falsy distinction and why
;;    each direction is safe.
;;  - `local-completion-function' (M54): the first of `completion-at-
;;    point''s three tiers (see its own docstring) -- a buffer-local
;;    slot, nil by default, that a mode can `setq-local' to a function
;;    of no arguments returning non-nil once it has fully handled a
;;    completion request (popup opened, or a deliberate "nothing here"
;;    silence) and nil to defer to LSP/dabbrev. `verilog-complete.el'
;;    (loaded after this file, see lib.rs's load order) is the only
;;    user as of M54: it knows a Verilog instantiation's port names
;;    from the module's OWN declaration even when the connected server
;;    can't complete at all, or isn't connected yet -- see its own
;;    header for why this needed a purpose-built completion source
;;    rather than either a smarter LSP fallback or a capability check
;;    alone (verible not supporting completion is a real, permanent gap
;;    in the server, not a bug this editor's own LSP client could paper
;;    over generically).
;;  - M57: a capability gate for `textDocument/formatting'/
;;    `rangeFormatting' -- the SECOND narrow gate in this file, same
;;    asymmetric `lsp--capability-supported-p' as M54's completion gate,
;;    added for the same reason: a concrete, real-binary finding, not a
;;    general policy shift. Measured 2026-08-11 with `dev/lsp-probe.py'
;;    against `slang-server' 0.2.9 on `demo/rtl/alu.sv': its `initialize'
;;    response has no `documentFormattingProvider'/
;;    `documentRangeFormattingProvider' key at all, and a real
;;    `textDocument/formatting' request sent anyway gets back NOTHING --
;;    not an error response either, just silence for the full 8-second
;;    probe window. Before M57, `lsp-format-buffer'/`lsp-format-region'
;;    sent the request unconditionally: the callback registered in
;;    `lsp--client-callbacks' never fires, so the user sees nothing at
;;    all, ever -- worse than M54's `completionProvider' finding, which
;;    at least got back an `Unhandled method' error `lsp--dispatch'
;;    could `message'. `lsp-format-buffer'/`lsp-format-region' now check
;;    their own capability key via `lsp--capability-supported-p' before
;;    sending, and `message' instead when it's absent. `lsp--unsupported-
;;    features-note' additionally reports this once, at `M-x lsp' connect
;;    time, specifically because a `message' from inside `before-save-
;;    hook' is otherwise invisible -- `save-buffer' runs the hook and
;;    then unconditionally overwrites the echo area with "Wrote ..."
;;    (see `editing.rs''s `save-buffer'), so a hook that silently
;;    no-ops via `message' alone never gets seen by a user who only
;;    saves and never explicitly requests formatting.
;;
;;    Two things this milestone deliberately does NOT fix, both real
;;    gaps that remain after it:
;;      - `lsp-request-async' has no timeout mechanism at all (M65:
;;        `lsp--await', the SYNCHRONOUS wait, gained one via
;;        `lsp-initialize-timeout' -- this async, callback-based path
;;        did not, and still can't: there is no wait to bound, a
;;        callback is just registered and invoked whenever/if a
;;        matching reply arrives). The gate above only catches the "key
;;        is ABSENT" case; a server that ADVERTISES the capability and
;;        then never answers anyway (a bug, or a slow/stuck server) is
;;        not caught by anything here -- the request goes out, the
;;        callback is registered, and if no reply ever comes the
;;        callback just never fires. Silent to the user exactly as
;;        before M57, for that one remaining case.
;;      - `lsp--client-callbacks' (see the `cl-defstruct' field below)
;;        has no expiry: an entry is only ever removed by `lsp--dispatch'
;;        matching its id, never by time or by `lsp-shutdown'. A request
;;        that never gets answered leaves its callback parked in that
;;        alist for the client's entire lifetime. M57 reduces how often
;;        this happens in practice (formatting no longer sends the
;;        request to a server that can't answer it), but does not touch
;;        the underlying mechanism -- a server that silently drops some
;;        OTHER, ungated request type leaks a callback exactly as before.
;;  - `textDocument/references' (M59): `lsp-references-at-point' (`M-?')
;;    -- no capability gate (unlike M54/M57's two narrow gates above):
;;    `verible-verilog-ls' both DECLARES `referencesProvider' and
;;    genuinely answers the request, confirmed against the real binary
;;    on an 890-file/154k-line SystemVerilog tree (400/400 module-
;;    instantiation references found, zero false positives, sub-second).
;;    That same probe found the ONE real gap this milestone documents
;;    rather than fixes: with no `verible.filelist' directly in the
;;    project root, the server answers `[]' UNCONDITIONALLY, even for a
;;    reference in the very file/buffer being queried -- surprising
;;    enough that the empty-result `message' names the missing file and
;;    its expected directory explicitly (see `lsp--references-empty-
;;    message') rather than leaving a bare "No references found" to
;;    imply the query genuinely came up empty. A `verible.filelist' that
;;    lists only SOME project files makes the server silently answer an
;;    INCOMPLETE list instead -- undetectable from this side, and not
;;    handled. `includeDeclaration' is always sent `t', but
;;    verible-verilog-ls never returns the declaration site regardless
;;    (confirmed against the real binary); a server that does honor it
;;    (slang-server) will include it, so reply SHAPE is server-
;;    dependent, not something this command normalizes. Positions use
;;    the UTF-16 pair (`lsp--line-utf16-at'/`lsp--pos-at-utf16', M46),
;;    same as M49's documentHighlight, for the identical reason (a new
;;    call site with no existing scalar-position behavior to preserve).
;;    This command is pure LSP -- unlike `lsp-definition-at-point' (M55),
;;    it never consults `local-definition-function' or any other local/
;;    non-LSP tier.
;;  - M66: v1 does not support LSP for remote (`/ssh:') buffers, at all.
;;    `M-x lsp' (`lsp', below) now turns a `/ssh:' buffer away with a
;;    `message' before it ever calls `lsp--project-root' on it -- see
;;    that function's own docstring for the two independent reasons
;;    (ssh cost, and a doomed non-`file://' URI even if the walk
;;    finished). The auto-attach path (`lsp--auto-attach-client', used
;;    by both `lsp--maybe-auto-attach' on `find-file' and `lsp--auto-
;;    attach-backfill' from inside `lsp' itself) already had this guard
;;    since M63; M66 only closes the one path that lacked it. Together
;;    these two are the ONLY way a buffer gets a live
;;    `lsp--buffer-client' at all, so guarding both closes every route
;;    to a remote client for good -- confirmed by reading every call
;;    site, not just the two guarded here.
;;    Two `lsp--project-root' call sites remain UNGUARDED:
;;    `lsp--references-empty-message' (this file, `M-?''s empty-result
;;    wording) and the reference-jump display-path helper near
;;    `lsp--relative-path''s call site. Both are reachable only through
;;    `lsp-references-at-point', which itself only runs against
;;    `lsp--buffer-client' -- nil for any `/ssh:' buffer now that `lsp'
;;    refuses to set it -- so they're unreachable in practice, not
;;    fixed. Worse, unlike `lsp--project-root's one-time cost at
;;    connect, these two re-walk the ancestor directories on EVERY
;;    call, so if remote LSP is ever opened up in the future, both need
;;    their own guard at that point, not just `lsp'/`lsp--auto-attach-
;;    client'.
;;    Actually supporting remote buffers -- starting the server ON the
;;    remote host and speaking a URI scheme the spec actually defines
;;    for that (not `file://' over an editor-internal `/ssh:' path) --
;;    is out of scope for v1 entirely, not something this milestone
;;    attempts even partially.

(cl-defstruct lsp--client
  conn                  ; the raw lsp-connection handle
  (next-id 1)
  pending               ; alist: (id . parsed-json-rpc-message)
  callbacks             ; alist: (id . fn) for async requests (M15)
  diagnostics            ; alist: (uri . vector-of-diagnostic-hash-tables)
  capabilities            ; M54: the `initialize' reply's own `capabilities'
                          ; hash table, or nil -- a client connected before
                          ; this field existed (or a test's `make-lsp--client'
                          ; stub), NOT "capabilities known to be empty"; see
                          ; `lsp--capability-supported-p''s own docstring for
                          ; why that distinction matters.
  command)                ; M59: the server COMMAND string `lsp-connect' was
                          ; called with (e.g. "verible-verilog-ls"), or nil
                          ; for a client predating this field (or a test's
                          ; `make-lsp--client' stub) -- lets a caller tell
                          ; WHICH server this client is talking to, for
                          ; wording a message specific to one server's own
                          ; known behavior (see `lsp--references-empty-
                          ; message') without guessing from the buffer's
                          ; major mode, which only says the LANGUAGE, not
                          ; which of possibly several configured servers
                          ; for it (`lsp-server-alist') is actually running.

;; Every live client, so the editor's idle tick can pump all of them
;; without the user threading handles around (M15).
(defvar lsp--clients nil)

(defun lsp--path-to-uri (path)
  (concat "file://" path))

(defun lsp--send-message (client method params id)
  (let ((h (make-hash-table)))
    (puthash "jsonrpc" "2.0" h)
    (when id (puthash "id" id h))
    (puthash "method" method h)
    (puthash "params" params h)
    (lsp-send (lsp--client-conn client) h)))

(defun lsp--notify (client method params)
  (lsp--send-message client method params nil))

(defun lsp--request (client method params)
  "Send METHOD as a request, return the id it was sent with."
  (let ((id (lsp--client-next-id client)))
    (setf (lsp--client-next-id client) (1+ id))
    (lsp--send-message client method params id)
    id))

(defun lsp--merge-diagnostics (client uri diags)
  (setf (lsp--client-diagnostics client)
        (cons (cons uri diags)
              (delq (assoc uri (lsp--client-diagnostics client))
                    (lsp--client-diagnostics client))))
  (lsp--decorate-buffer uri diags))

(defun lsp--severity-color (sev)
  (cond ((eq sev 1) "#f44747")
        ((eq sev 2) "#ffcc00")
        (t "#3a96dd")))

(defun lsp--pos-at (line character)
  "Buffer position for 0-based LINE/CHARACTER in the current buffer."
  (save-excursion
    (goto-char (point-min))
    (forward-line line)
    (forward-char character)
    (point)))

;; --- M46: UTF-16-correct position conversion -----------------------------
;;
;; LSP's `character` is a UTF-16 code-unit offset, not a Unicode scalar
;; value: every character outside the Basic Multilingual Plane (emoji,
;; most other supplementary-plane codepoints) counts as 2 units, not 1.
;; `lsp--pos-at'/`lsp--line-character-at' above get this wrong on any
;; line with an astral-plane character before the position in question
;; -- see the file header for why they're left as-is and only the new
;; TextEdit/documentSymbol code paths use these.

(defun lsp--pos-at-utf16 (line character)
  "Buffer position for 0-based LINE and a UTF-16 code-unit CHARACTER
offset within that line -- the UTF-16-correct counterpart of
`lsp--pos-at'.

CHARACTER past the line's actual UTF-16 length clamps to the line's
end, per the LSP spec (a server may legitimately report a position one
past the last character of a line). CHARACTER landing in the middle of
a surrogate pair -- only possible if the server sent an odd offset
into an astral-plane character, itself already a spec violation --
stops right after that whole character rather than splitting it, since
this buffer has no representation of a lone surrogate half."
  (save-excursion
    (goto-char (point-min))
    (forward-line line)
    (let ((limit (line-end-position))
          (units 0))
      (while (and (< units character) (< (point) limit))
        (let ((c (char-after)))
          (setq units (+ units (if (>= c #x10000) 2 1)))
          (forward-char 1)))
      (point))))

(defun lsp--utf16-character-at (pos)
  "UTF-16 code-unit offset of POS within its own line -- the
UTF-16-correct counterpart of the CHARACTER half of
`lsp--line-character-at', and the inverse of `lsp--pos-at-utf16' (for
any POS reachable by it: `(lsp--utf16-character-at (lsp--pos-at-utf16 L
C))' returns C whenever C did not need clamping)."
  (save-excursion
    (let ((bol (progn (goto-char pos) (line-beginning-position)))
          (units 0))
      (goto-char bol)
      (while (< (point) pos)
        (let ((c (char-after)))
          (setq units (+ units (if (>= c #x10000) 2 1)))
          (forward-char 1)))
      units)))

(defun lsp--line-utf16-at (pos)
  "0-based (LINE . UTF16-CHARACTER) at buffer position POS -- the
UTF-16-correct counterpart of `lsp--line-character-at', for building
LSP `Position' params (`textDocument/rangeFormatting''s `range', mainly)
from buffer positions."
  (cons (1- (line-number-at-pos pos))
        (lsp--utf16-character-at pos)))

(defun lsp--decorate-buffer (uri diags)
  "M16: turn published diagnostics into wavy underlines + gutter data
for the buffer visiting URI, if any. Old decorations are replaced."
  (let* ((path (if (string-prefix-p "file://" uri) (substring uri 7) uri))
         (buf (get-file-buffer path)))
    (when buf
      (with-current-buffer-internal buf
        (lambda ()
          ;; Drop our old squiggles only.
          (dolist (ov (overlays-in (point-min) (point-max)))
            (when (overlay-get ov 'lsp-diag)
              (delete-overlay ov)))
          (let ((gutter nil) (i 0) (n (length diags)))
            (while (< i n)
              (let* ((d (aref diags i))
                     (range (gethash "range" d))
                     (start (gethash "start" range))
                     (end (gethash "end" range))
                     (sev (or (gethash "severity" d) 1))
                     (sev (if (eq sev :null) 1 sev))
                     (from (lsp--pos-at (gethash "line" start)
                                        (gethash "character" start)))
                     (to (lsp--pos-at (gethash "line" end)
                                      (gethash "character" end)))
                     (ov (make-overlay from (if (> to from) to (1+ from)))))
                (overlay-put ov 'lsp-diag t)
                (overlay-put ov 'face
                             (list ':underline
                                   (list ':style 'wave
                                         ':color (lsp--severity-color sev))))
                (setq gutter (cons (cons (gethash "line" start) sev) gutter)))
              (setq i (1+ i)))
            (lsp--set-buffer-diagnostics buf gutter)))))))

(defun lsp--dispatch (client msg)
  "Handle one parsed JSON-RPC message: fold a `publishDiagnostics`
notification into CLIENT, deliver an async response to its registered
callback, or stash it under its id for `lsp--await` to pick up.

M46: if MSG carries an `\"error\"` key (a JSON-RPC error response --
unknown method, malformed params, ...), announce it via `message` first
-- previously this was indistinguishable from \"succeeded with an empty
result\", so a rejected request produced no feedback at all. The
callback/pending contract below is unchanged: an error response has no
`\"result\"` key, so `(gethash \"result\" msg)` is nil either way --
this only adds visibility, it does not change what any existing
callback receives."
  (let ((id (gethash "id" msg))
        (method (gethash "method" msg))
        (err (gethash "error" msg)))
    (when (hash-table-p err)
      (message "lsp: %s" (gethash "message" err)))
    (cond
     ((equal method "textDocument/publishDiagnostics")
      (let ((params (gethash "params" msg)))
        (lsp--merge-diagnostics
         client (gethash "uri" params) (gethash "diagnostics" params))))
     (id
      (let ((cb (assq id (lsp--client-callbacks client))))
        (if cb
            (progn
              (setf (lsp--client-callbacks client)
                    (delq cb (lsp--client-callbacks client)))
              (funcall (cdr cb) (gethash "result" msg)))
          (setf (lsp--client-pending client)
                (cons (cons id msg) (lsp--client-pending client)))))))))

(defun lsp-request-async (client method params callback)
  "Send METHOD as a request; CALLBACK is called with the response's
`result` whenever it arrives — typing continues in the meantime. The
editor's idle tick delivers it (M15)."
  (let ((id (lsp--request client method params)))
    (setf (lsp--client-callbacks client)
          (cons (cons id callback) (lsp--client-callbacks client)))
    id))

(defvar lsp-initialize-timeout 10
  "Seconds `lsp--await' will wait for ANY single request/response round
trip before giving up -- most concretely `lsp-connect''s `initialize'
handshake, the one synchronous wait every connection goes through
before anything else can happen (M65).

10 is a generous upper bound, not a tuned performance number: a real
probe against verible-verilog-ls (this project's primary LSP server,
see CLAUDE.md) measured its `initialize' round trip at 6-14
MILLISECONDS. This is headroom for a slow machine or a server doing
real disk/index work on startup, not a value anyone should need to
tune for normal use.

When the timeout is hit, `lsp--await' signals an error instead of
blocking forever -- `M-x lsp' then reports \"LSP: failed to start ...\"
via its own `condition-case' (see `lsp'), same as any other spawn
failure, rather than freezing the editor with no way back (not even
`C-g': both frontends are single-threaded event loops that don't read
another key until `handle_key' returns, so a truly unbounded wait
inside it is unrecoverable except by killing the process from
outside).

Known limitation (M65 review): `lsp--await''s deadline is computed
from `float-time', a WALL clock (`SystemTime::now()' underneath, see
`crates/elisp/src/builtins/misc.rs'), not a monotonic one. A clock step
backward (NTP correction, manual `date -s', ...) during the wait makes
the remaining budget look larger than it should, so this timeout can
wait LONGER than TIMEOUT seconds in that case. It cannot wait forever:
each individual `lsp-wait' call still uses a monotonic clock on the
Rust side (`recv_timeout', `crates/elisp/src/lsp.rs') for its own
slice, so the worst case is \"this round's slice runs its full
(possibly-inflated) length, then the loop re-checks\", not \"never
returns\". Real Emacs' `float-time' has the identical wall-clock
property, so this isn't a regression from that baseline -- just worth
being honest about rather than silently assuming a monotonic clock
this codebase never actually asked `float-time' for.")

(defun lsp--await (client id &optional timeout)
  "Block until the message answering ID has arrived (dispatching
everything else seen along the way), then return its `result`.

TIMEOUT (seconds, default `lsp-initialize-timeout') bounds the TOTAL
wait, not each individual `lsp-wait' call: a chatty server that keeps
sending other messages (`$/progress' notifications, diagnostics for
files we don't care about yet, ...) must not be able to reset the
budget just by being talkative. Computed as an absolute deadline via
`float-time' up front, then re-derived as \"deadline minus now\" on
every iteration and handed to `lsp-wait' as ITS timeout for that one
receive -- so the remaining budget only ever shrinks."
  (let* ((deadline (+ (float-time) (or timeout lsp-initialize-timeout)))
         found)
    (while (not found)
      (setq found (assq id (lsp--client-pending client)))
      (unless found
        (let* ((remaining (- deadline (float-time)))
               (msg (lsp-wait (lsp--client-conn client) (max remaining 0))))
          (cond
           ((and (consp msg) (eq (car msg) 'error))
            (error "lsp: %s" (cdr msg)))
           ((null msg)
            (error "lsp: timed out waiting %ss for response to id %s"
                   (or timeout lsp-initialize-timeout) id))
           (t (lsp--dispatch client msg))))))
    (setf (lsp--client-pending client) (delq found (lsp--client-pending client)))
    (gethash "result" (cdr found))))

(defun lsp-process-pending (client)
  "Non-blocking: dispatch every message already received without
waiting for any particular one."
  (let (msg)
    (while (setq msg (lsp-poll (lsp--client-conn client)))
      ;; A dead server surfaces as an (error . MESSAGE) cons rather
      ;; than a parsed message hash; only dispatch real messages.
      (when (hash-table-p msg)
        (lsp--dispatch client msg)))))

(defun lsp-process-pending-all ()
  "Pump every live client (the editor idle tick calls this), pruning
clients whose server process has died; then (M35) give every buffer a
chance to send a `textDocument/didChange' via `lsp--sync-buffer-now',
so an edit gets to the server on the next idle tick after it's made
rather than waiting for a hover/definition request to trigger the
sync. Never signals: a malformed registry entry is pruned, not allowed
to break the idle tick, and `lsp--sync-buffer-now' is itself a no-op
for any buffer with no live client, no visited file, or nothing new to
sync."
  (let (live)
    (dolist (client lsp--clients)
      (let ((conn (lsp--client-conn client)))
        (when (and (lsp-connection-p conn) (lsp-live-p conn))
          (lsp-process-pending client)
          (push client live))))
    (setq lsp--clients (nreverse live)))
  ;; M35 review hardening, twice over: (a) skip the whole buffer walk
  ;; when no client exists at all -- with-current-buffer-internal swaps
  ;; every buffer-local in and out, a per-tick cost that shouldn't be
  ;; paid by sessions that never touched LSP; (b) a per-buffer
  ;; condition-case, because the liveness check inside
  ;; lsp--sync-buffer-now races the server dying: lsp-send hits a
  ;; closed stdin a beat later and signals, and without the handler
  ;; that one buffer would abort the dolist and starve every buffer
  ;; after it of its sync this tick (the next tick's prune would
  ;; self-heal, but "never signals" should be true, not approximately
  ;; true).
  (when lsp--clients
    (dolist (buf (buffer-list))
      (condition-case nil
          (with-current-buffer-internal buf (lambda () (lsp--sync-buffer-now)))
        (error nil)))))

(defun lsp-connect (command &optional args root-path)
  "Start COMMAND (ARGS) as an LSP server and perform the initialize
handshake. Returns a `lsp--client'.

M54: the `initialize' reply's own `result.capabilities' hash table is
stashed on the client (`lsp--client-capabilities') for
`lsp--capability-supported-p' -- previously `(lsp--await client id)''s
return value was discarded outright. Never signals over a malformed or
missing reply: anything other than a hash table (nil included) leaves
`capabilities' nil, same as a client that predates this field.

M65: the handshake -- the `initialize' request, `lsp--await', AND the
`initialized' notification that must follow it per spec -- is wrapped
in its own `condition-case' so a timeout or any other failure anywhere
in that sequence kills CONN (`lsp-kill') before re-signaling, instead
of leaving an orphaned server process with nobody left to talk to it.
Most concretely: `lsp--await' hitting `lsp-initialize-timeout' against
a server that connects but never answers; but a review fix caught that
a server dying right after it DOES answer `initialize' hits `lsp--
notify's `lsp-send' on a now-closed pipe, and that failure was
originally past the end of the `condition-case', so it skipped the
kill entirely (a `Drop'-triggered kill would still have happened
eventually, but only incidentally -- not the explicit, function-local
path every other failure here takes). This is INSIDE this function
rather than left to the caller (`lsp'/`lsp--maybe-auto-attach') because
CONN only exists here -- a caller catching the re-signaled error never
gets a handle to kill it itself. Not reached at all on a spawn failure
(`lsp-start' signaling): there is no CONN yet in that case, nothing to
kill."
  (let* ((conn (lsp-start command args))
         (client (make-lsp--client :conn conn :command command)))
    (condition-case err
        (let* ((id (lsp--request
                    client "initialize"
                    (let ((p (make-hash-table)))
                      (puthash "processId" :null p)
                      (puthash "rootUri" (if root-path (lsp--path-to-uri root-path) :null) p)
                      (puthash "capabilities" (make-hash-table) p)
                      p)))
               (result (lsp--await client id)))
          (setf (lsp--client-capabilities client)
                (and (hash-table-p result) (gethash "capabilities" result)))
          (lsp--notify client "initialized" (make-hash-table)))
      (error
       (lsp-kill conn)
       (signal (car err) (cdr err))))
    (setq lsp--clients (cons client lsp--clients))
    client))

(defun lsp--capability-supported-p (client key)
  "Non-nil if CLIENT's own advertised `initialize' capabilities support
KEY (a JSON key string from the `ServerCapabilities' object, e.g.
\"completionProvider\") well enough that sending the matching request is
worthwhile.

Deliberately ASYMMETRIC, and deliberately narrower than a blanket
capabilities check (see this file's M46 note on why NO gate existed
before M54, and the M54 note just above `cl-defstruct lsp--client' for
the full context):

  - CLIENT is not even a real `lsp--client' struct at all (`lsp--client-
    p' false) -- some tests stand in a plain symbol like `'fake-client'
    for CLIENT, same convention `lsp--live-buffer-client' already
    tolerates (see its own doc comment) -- or `lsp--client-capabilities'
    IS a struct field but reads nil (no `initialize' reply ever
    recorded: a client connected before this field existed, or a
    genuinely empty/malformed reply) => SUPPORTED either way. This is
    the M46-era default preserved exactly: trust the request will work
    absent any evidence otherwise, so nothing that already worked
    regresses (this function never calls the `lsp--client-capabilities'
    struct accessor on something that isn't a real struct -- doing so
    would signal `wrong-type-argument', not just answer nil).
  - Capabilities are known (a real hash table) and KEY is present in
    it, REGARDLESS of its value (`t', `:false', nil, or a nested hash
    table all count) => SUPPORTED. This is the M46 finding restated as
    code: `verible-verilog-ls' advertises `hoverProvider: false' yet
    answers `textDocument/hover' correctly (confirmed against the real
    binary) -- a capability's own FALSY value is not trustworthy
    evidence of anything, so this function never looks at it.
  - Capabilities are known and KEY is ABSENT entirely => UNSUPPORTED.
    This is the one case M46's blanket policy got wrong for M54's own
    finding: `verible-verilog-ls' has no `completionProvider' key at
    all in its `initialize' response (checked against the real
    binary), and a real `textDocument/completion' request sent anyway
    gets back `Unhandled method' on the server's own stderr -- silently
    doing nothing from the editor's side, worse than not being
    connected (an unconnected buffer at least falls through to
    dabbrev). Key ABSENT and key PRESENT-BUT-FALSE are genuinely
    different signals from this one server alone; this function is the
    only place in this file that acts on that difference, and only for
    whichever KEY a caller asks about -- every other capability
    (`hoverProvider' included) stays exactly as ungated as before this
    function existed. M57 added a second pair of callers,
    `lsp-format-buffer'/`lsp-format-region' (`\"documentFormattingProvider\"'/
    `\"documentRangeFormattingProvider\"'), for the same reason M54 added
    `completion-at-point''s: a concrete real-binary finding that an
    absent key means the request never gets answered at all (see the
    file header's M57 note). No other caller exists as of M57 either."
  (let ((caps (and (lsp--client-p client) (lsp--client-capabilities client))))
    (if (not (hash-table-p caps))
        t
      (not (eq (gethash key caps 'lsp--capability-absent) 'lsp--capability-absent)))))

(defconst lsp--gated-features-alist
  '(("documentFormattingProvider" . "lsp-format-buffer")
    ("documentRangeFormattingProvider" . "lsp-format-region"))
  "Alist of (CAPABILITY-KEY . COMMAND-NAME) for `lsp--unsupported-
features-note' to walk -- M57. Lists ONLY capability keys that are both
(a) actually gated by `lsp--capability-supported-p' somewhere in this
file, and (b) have no fallback path when absent, so their absence means
the command does nothing at all rather than something worse. That is
why `\"completionProvider\"' is deliberately NOT on this list even
though M54 gates it too:
  (a) it has a fallback -- `completion-at-point' falls through to
      `dabbrev-expand' (and, for `verilog-mode', M54's own port-name
      completion tier) when the server's completion is unsupported, so
      losing it is a degradation, not a feature going to zero; and
  (b) the default Verilog server, `verible-verilog-ls', has never
      advertised `completionProvider' at all (M54's own finding) --
      putting it on this list would print an unsupported-feature note
      on EVERY ordinary connect to this editor's primary-language
      server, which is exactly the kind of cried-wolf noise that trains
      users to stop reading `M-x lsp''s connect message.")

(defun lsp--unsupported-features-note (client)
  "Human-readable note listing which of `lsp--gated-features-alist''s
commands CLIENT's advertised capabilities do not support, or nil if all
of them are supported (including the M46/M54 asymmetric-trust cases:
CLIENT not a real struct, `lsp--client-capabilities' nil, or a listed
key present but falsy all count as \"supported\" here, via
`lsp--capability-supported-p' -- this function never inspects
`lsp--client-capabilities' directly, so it can never disagree with the
gates in `lsp-format-buffer'/`lsp-format-region' about what actually
gets sent). Called once by `lsp' right after a successful connect (M57)
specifically because a `message' from inside `before-save-hook' alone
is invisible: `save-buffer' runs the hook and then unconditionally
overwrites the echo area with \"Wrote ...\" (see `editing.rs''s
`save-buffer'), so a hook that silently no-ops via `message' never
reaches a user who only ever saves and never explicitly invokes
`lsp-format-buffer'."
  (let (missing)
    (dolist (entry lsp--gated-features-alist)
      (unless (lsp--capability-supported-p client (car entry))
        (push (cdr entry) missing)))
    (when missing
      (format "unsupported: %s" (string-join (nreverse missing) ", ")))))

(defun lsp-shutdown (client)
  "Send the `shutdown'/`exit' sequence to CLIENT's server and kill its
connection.

M65: waits for the `shutdown' response via `lsp--await', which now
signals an error after `lsp-initialize-timeout' seconds (default 10)
instead of blocking forever if the server never answers -- unhandled
here, so a stuck server surfaces as a signal from this call rather than
a hang, same as any other `lsp--await' caller post-M65."
  (setq lsp--clients (delq client lsp--clients))
  (lsp--await client (lsp--request client "shutdown" :null))
  (lsp--notify client "exit" :null)
  (lsp-kill (lsp--client-conn client)))

(defun lsp-did-open (client path text &optional language-id)
  "Version is `buffer-modified-tick' (M35), not a hardcoded 1: an
unedited buffer's tick is its own baseline, and `lsp--sync-buffer-now'
later diffs against it to decide whether a `textDocument/didChange' is
owed. Relies on the caller's current buffer being the one PATH/TEXT
came from -- true of the `lsp' command's own call site below; a call
site with no buffer to speak of (lsp_tests.rs's synchronous, buffer-
less smoke test) just gets whatever the current buffer's tick happens
to be, a valid version number regardless since didOpen never repeats
for the same document."
  (let ((p (make-hash-table))
        (td (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri path) td)
    (puthash "languageId" (or language-id "rust") td)
    (puthash "version" (buffer-modified-tick) td)
    (puthash "text" text td)
    (puthash "textDocument" td p)
    (lsp--notify client "textDocument/didOpen" p)))

(defun lsp-did-change (client path text version)
  "Send a full-text `textDocument/didChange': `contentChanges' is a
single-element array whose lone entry omits `range', meaning \"replace
the entire document\" (see the file header). VERSION should be greater
than whatever was last sent for PATH; `lsp--sync-buffer-now' passes
`buffer-modified-tick'."
  (let ((p (make-hash-table))
        (td (make-hash-table))
        (change (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri path) td)
    (puthash "version" version td)
    (puthash "text" text change)
    (puthash "textDocument" td p)
    (puthash "contentChanges" (list change) p)
    (lsp--notify client "textDocument/didChange" p)))

(defun lsp-did-save (client path)
  "Send `textDocument/didSave' for PATH. v1 params carry only
`textDocument': no `includeText'/`text', so the caller (`lsp--on-after-
save') must have already gotten the server's copy up to date via
`lsp--sync-buffer-now' -- a server that wants the saved text reads it
from the didChange it just received, not from this notification."
  (let ((p (make-hash-table))
        (td (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri path) td)
    (puthash "textDocument" td p)
    (lsp--notify client "textDocument/didSave" p)))

(defun lsp-did-close (client path)
  "Send `textDocument/didClose' for PATH."
  (let ((p (make-hash-table))
        (td (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri path) td)
    (puthash "textDocument" td p)
    (lsp--notify client "textDocument/didClose" p)))

(defun lsp--position (line character)
  (let ((h (make-hash-table)))
    (puthash "line" line h)
    (puthash "character" character h)
    h))

(defun lsp--text-document-position-params (path line character)
  (let ((p (make-hash-table))
        (td (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri path) td)
    (puthash "textDocument" td p)
    (puthash "position" (lsp--position line character) p)
    p))

(defun lsp--hover-text (result)
  "Extract the hover string from a textDocument/hover RESULT, or nil."
  (when (hash-table-p result)
    (let ((contents (gethash "contents" result)))
      (cond
       ((hash-table-p contents) (gethash "value" contents))
       ((stringp contents) contents)
       (t nil)))))

(defun lsp-hover (client path line character)
  "Synchronous hover request at 0-based LINE/CHARACTER; returns the
hover contents as a string, or nil.

M65: bounded by `lsp--await''s `lsp-initialize-timeout' (default 10s)
-- signals an error instead of blocking forever if the server never
answers. `lsp-hover-async' is the interactive path precisely to avoid
even that bounded wait; this synchronous entry point is for scripts
and tests."
  (let ((id (lsp--request client "textDocument/hover"
                          (lsp--text-document-position-params path line character))))
    (lsp--hover-text (lsp--await client id))))

(defun lsp-hover-async (client path line character &optional callback)
  "Async hover (M15): send the request and return immediately; when the
answer arrives (delivered by the editor idle tick), call CALLBACK with
the hover string, or echo it if CALLBACK is nil. Typing never blocks."
  (lsp-request-async
   client "textDocument/hover"
   (lsp--text-document-position-params path line character)
   (lambda (result)
     (let ((text (lsp--hover-text result)))
       (cond (callback (funcall callback text))
             (text (show-hover-popup text))
             ;; A null hover is a real answer (common while the server
             ;; is still indexing) -- tell the user rather than doing
             ;; nothing, so C-h . never feels like a dead key.
             (t (message "No hover info at point")))))))

(defun lsp--definition-location (result)
  "Parse a textDocument/definition RESULT -- a single Location
hash-table, or a vector of them, both valid per the LSP spec -- into
(URI . LINE) for the first location, or nil. Shared by the synchronous
`lsp-definition' and the async `lsp-definition-at-point' (M26)."
  (let ((loc (cond ((and (vectorp result) (> (length result) 0)) (aref result 0))
                    ((hash-table-p result) result))))
    (when loc
      (cons (gethash "uri" loc)
            (gethash "line" (gethash "start" (gethash "range" loc)))))))

(defun lsp-definition (client path line character)
  "Synchronous go-to-definition at 0-based LINE/CHARACTER; returns
(URI . LINE) of the first location, or nil.

M65: bounded by `lsp--await''s `lsp-initialize-timeout' (default 10s)
-- signals an error instead of blocking forever if the server never
answers. `lsp-definition-at-point' is the interactive path precisely
to avoid even that bounded wait; this synchronous entry point is for
scripts and tests."
  (let ((id (lsp--request client "textDocument/definition"
                          (lsp--text-document-position-params path line character))))
    (lsp--definition-location (lsp--await client id))))

(defun lsp-diagnostics (client path)
  "Diagnostics last published for PATH, or nil if none have arrived yet."
  (cdr (assoc (lsp--path-to-uri path) (lsp--client-diagnostics client))))

;;; --- M46: TextEdit application -------------------------------------------

(defun lsp--edit-range-start (edit)
  (let ((start (gethash "start" (gethash "range" edit))))
    (lsp--pos-at-utf16 (gethash "line" start) (gethash "character" start))))

(defun lsp--edit-range-end (edit)
  (let ((end (gethash "end" (gethash "range" edit))))
    (lsp--pos-at-utf16 (gethash "line" end) (gethash "character" end))))

(defun lsp--apply-text-edits (edits)
  "Apply EDITS -- a vector of LSP `TextEdit' hash-tables -- to the
current buffer. Each edit's `range' is converted to buffer positions
via `lsp--pos-at-utf16' (LSP's UTF-16 CHARACTER encoding -- see the
file header's M46 note on why this is a different function from
`lsp--pos-at'), then edits are applied in descending START order so
that applying one never shifts the still-to-be-applied ones' own
ranges -- the same back-to-front discipline `replace-region-contents'
itself uses for hunks within a single edit, just one level up.

This is the SAME code path for a single all-encompassing edit (the
common case for `textDocument/formatting' -- `verible-verilog-ls'
returns one TextEdit covering the whole document) and many small ones
(`textDocument/rangeFormatting', and later codeAction/rename) --
deliberately no separate \"is this actually the whole document\" fast
path or special case; a single edit is just the general case with one
element.

Signals an error if any two edits overlap (checked after sorting: edit
N's END must not exceed edit N-1's START, where N-1 is the
previously-processed, later-starting edit). The LSP spec forbids
overlapping TextEdits in one response; applying back-to-front over an
overlapping pair would silently corrupt the buffer (an already-consumed
range gets edited a second time) rather than raising anything, so this
check exists specifically to turn that into a loud, caught error
instead. Two ADJACENT edits (one's END equal to the next one's START)
are NOT overlapping and are accepted -- the check is a strict `>`, not
`>=`.

Multiple zero-length edits (an empty range) at the exact same position
are legal per the spec -- each represents a separate insertion at that
point -- and are not caught by the overlap check (their START and END
are equal, so consecutive ones never satisfy END > START). Their
relative order is not specified by LSP, and this implementation doesn't
pick a principled one either: `sort`'s stability plus the reversed
`positioned` build order (edits are `push`ed while iterating the input
forward, so `positioned` starts in reverse of the input) means that,
among same-position zero-length edits, the LAST one in the input array
ends up applied FIRST. Documented as the current behavior, not a
guarantee a caller should rely on."
  (let (positioned)
    (dotimes (idx (length edits))
      (let ((edit (aref edits idx)))
        (push (list (lsp--edit-range-start edit)
                    (lsp--edit-range-end edit)
                    (gethash "newText" edit))
              positioned)))
    (setq positioned (sort positioned (lambda (a b) (> (car a) (car b)))))
    (let (prev)
      (dolist (e positioned)
        (when (and prev (> (nth 1 e) (car prev)))
          (error "lsp: overlapping TextEdits in response"))
        (setq prev e)))
    (dolist (e positioned)
      (replace-region-contents (nth 0 e) (nth 1 e) (nth 2 e)))))

(defun lsp--formatting-options ()
  "`FormattingOptions' params for `textDocument/formatting'/
`rangeFormatting'. `tabSize' is `standard-indent-width' (this editor's
own indent-width variable, `indent.el'); `insertSpaces' is always `t'
-- this editor has no per-buffer tabs-vs-spaces setting to read instead
(see `indent.el''s note that `indent-tabs-mode' is permanently nil
here). Configurable formatting options are v1-not-included, tracked in
PLAN.md."
  (let ((h (make-hash-table)))
    (puthash "tabSize" standard-indent-width h)
    (puthash "insertSpaces" t h)
    h))

(defun lsp--text-document-only-params ()
  (let ((p (make-hash-table))
        (td (make-hash-table)))
    (puthash "uri" (lsp--path-to-uri (buffer-file-name)) td)
    (puthash "textDocument" td p)
    p))

(defun lsp-format-buffer ()
  "Send `textDocument/formatting' for the current buffer and apply the
result via `lsp--apply-text-edits'. Async (`lsp-request-async') --
even with M65's bound on `lsp--await' (`lsp-initialize-timeout',
default 10s), a synchronous call here could still visibly freeze the
editor for that long if the server never answers (see the file
header's M46 note); every M46 interactive command goes through the
async path for that reason.

A reply that arrives after the user has switched away from this buffer
is discarded (checked via `eq' against the buffer captured at request
time), same discipline as `lsp-completion-at-point'/
`lsp-definition-at-point' -- applying edits meant for text the user is
no longer looking at would be a worse surprise than silently dropping
the reply.

M46 review fix: unlike hover/definition (read-only -- a stale reply
just displays outdated information), this one WRITES to the buffer via
`lsp--apply-text-edits'. If the user keeps typing while the server is
still computing, the reply describes edits computed against a snapshot
that no longer matches the live buffer; applying it anyway could delete
or garble text typed in the meantime. So the buffer's
`buffer-modified-tick' is captured right after `lsp--sync-buffer-now'
(i.e. exactly what was sent) and compared again when the reply lands --
a mismatch means the buffer changed since the request went out, and the
edits are DROPPED with a `message' (not silently discarded, unlike the
plain buffer-switch case above, since this is the buffer the user is
still looking at and typed into).

M57: gated on `\"documentFormattingProvider\"' via
`lsp--capability-supported-p' before sending -- `slang-server' has no
such key and never answers the request at all (see the file header's
M57 note), so without this gate the callback registered below would
simply never fire. Same asymmetric trust as `lsp--capability-supported-p'
itself: a client with no known capabilities, or a client whose
capabilities hash has the key present but falsy, is still treated as
supported and sends the request exactly as before M57."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     ((not (lsp--capability-supported-p client "documentFormattingProvider"))
      (message "lsp: server does not advertise documentFormattingProvider -- formatting unavailable"))
     (t
      (lsp--sync-buffer-now)
      (let ((p (lsp--text-document-only-params))
            (buf (current-buffer))
            (tick (buffer-modified-tick)))
        (puthash "options" (lsp--formatting-options) p)
        (lsp-request-async
         client "textDocument/formatting" p
         (lambda (result)
           (when (and (eq (current-buffer) buf) (vectorp result))
             (if (= (buffer-modified-tick) tick)
                 (lsp--apply-text-edits result)
               (message "lsp: buffer changed since format request, discarding stale edits"))))))))))

(defun lsp-format-region ()
  "Send `textDocument/rangeFormatting' for the active region
(`region-beginning'/`region-end') and apply the result via
`lsp--apply-text-edits'. Same async/staleness discipline as
`lsp-format-buffer' -- see its docstring, including the M46 review fix
that discards the reply (with a `message') rather than applying it if
`buffer-modified-tick' moved between the request and the reply.

M57: gated on `\"documentRangeFormattingProvider\"' (its OWN key, not
`\"documentFormattingProvider\"' -- a server could in principle support
one without the other) via `lsp--capability-supported-p'. See
`lsp-format-buffer''s own M57 note for why."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     ((not (lsp--capability-supported-p client "documentRangeFormattingProvider"))
      (message "lsp: server does not advertise documentRangeFormattingProvider -- region formatting unavailable"))
     (t
      (lsp--sync-buffer-now)
      (let* ((beg (region-beginning))
             (end (region-end))
             (start-lc (lsp--line-utf16-at beg))
             (end-lc (lsp--line-utf16-at end))
             (p (lsp--text-document-only-params))
             (range (make-hash-table))
             (buf (current-buffer))
             (tick (buffer-modified-tick)))
        (puthash "start" (lsp--position (car start-lc) (cdr start-lc)) range)
        (puthash "end" (lsp--position (car end-lc) (cdr end-lc)) range)
        (puthash "range" range p)
        (puthash "options" (lsp--formatting-options) p)
        (lsp-request-async
         client "textDocument/rangeFormatting" p
         (lambda (result)
           (when (and (eq (current-buffer) buf) (vectorp result))
             (if (= (buffer-modified-tick) tick)
                 (lsp--apply-text-edits result)
               (message "lsp: buffer changed since format request, discarding stale edits"))))))))))

;;; --- M46: documentSymbol navigation ---------------------------------------

(defun lsp--flatten-symbols (symbols)
  "Recursively flatten nested `DocumentSymbol' vector SYMBOLS (as
returned by `textDocument/documentSymbol' -- each entry may carry a
`children' vector of more of the same) into a flat list of (POS NAME
KIND), POS from `selectionRange.start' via `lsp--pos-at-utf16', in
preorder (each symbol immediately followed by its own children, before
its next sibling). Not sorted by POS across siblings at different
depths -- `lsp--document-symbol-positions' does a real sort by POS once
over the whole tree; this function only guarantees tree (document)
order, which is what a caller wanting parent-before-child would want
this for.

M46 review fix: an earlier version pushed each symbol onto an
accumulator and prepended `(lsp--flatten-symbols children)' ahead of
it, then did a single `nreverse' at the very end. That composes wrong
across recursion levels -- a child list that came back already
correctly ordered from its own (already-reversed) recursive call gets
reversed a SECOND time by the outer call's final `nreverse', so a
node's children end up in reverse document order relative to each
other (though still correctly placed between their parent and their
next sibling). E.g. `[A(children=[A1,A2]), B]' produced `(A A2 A1 B)'
instead of `(A A1 A2 B)'. Building the list forward with `append'
instead avoids the double reversal (this editor has no `nconc';
`append' non-destructively copies, which is fine at documentSymbol
tree sizes)."
  (let (out)
    (dotimes (idx (length symbols))
      (let* ((sym (aref symbols idx))
             (start (gethash "start" (gethash "selectionRange" sym)))
             (pos (lsp--pos-at-utf16 (gethash "line" start) (gethash "character" start)))
             (children (gethash "children" sym)))
        (setq out (append out (list (list pos (gethash "name" sym) (gethash "kind" sym)))))
        (when (vectorp children)
          (setq out (append out (lsp--flatten-symbols children))))))
    out))

(defun lsp--document-symbol-positions (result)
  "Flattened, ascending-by-POS (POS NAME KIND) list from a
`textDocument/documentSymbol' RESULT (a possibly-nested `DocumentSymbol'
vector), or nil if RESULT isn't a vector (null result, or the older
flat `SymbolInformation[]' shape -- v1 only supports the nested
`DocumentSymbol[]' shape every server exercised so far returns)."
  (when (vectorp result)
    (sort (lsp--flatten-symbols result) (lambda (a b) (< (car a) (car b))))))

(defvar lsp--buffer-symbols nil
  "Cached (POS NAME KIND) list from the most recent
`textDocument/documentSymbol' reply for this buffer, set by
`lsp--goto-symbol'. Re-fetched on every `lsp-next-symbol'/
`lsp-previous-symbol' call -- there is no cache-invalidation story for
\"the buffer changed since the last fetch\", so each navigation asks
the server fresh rather than risk jumping to a stale position.")

(defun lsp--nearest-symbol (direction)
  "Entry from `lsp--buffer-symbols' to jump to, wrapping at the ends --
mirrors `next-diagnostic'/`previous-diagnostic''s own wrap-around logic
exactly. DIRECTION is `next' or `prev'."
  (let ((here (point)))
    (if (eq direction 'next)
        (let (found)
          (dolist (entry lsp--buffer-symbols)
            (when (and (> (car entry) here) (not found))
              (setq found entry)))
          (or found (car lsp--buffer-symbols)))
      (let (found last-entry)
        (dolist (entry lsp--buffer-symbols)
          (setq last-entry entry)
          (when (< (car entry) here)
            (setq found entry)))
        (or found last-entry)))))

(defun lsp--goto-symbol (direction)
  "Shared body of `lsp-next-symbol'/`lsp-previous-symbol': request
`textDocument/documentSymbol', then jump to the nearest symbol in
DIRECTION (`next' or `prev') once the reply lands. Async, same
staleness/buffer-identity discipline as `lsp-format-buffer'."
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      (lsp--sync-buffer-now)
      (let ((p (lsp--text-document-only-params))
            (buf (current-buffer)))
        (lsp-request-async
         client "textDocument/documentSymbol" p
         (lambda (result)
           (when (eq (current-buffer) buf)
             (setq-local lsp--buffer-symbols (lsp--document-symbol-positions result))
             (if (not lsp--buffer-symbols)
                 (message "No symbols")
               (let ((target (lsp--nearest-symbol direction)))
                 (goto-char (car target))
                 (message "%s" (nth 1 target))))))))))))

(defun lsp-next-symbol ()
  "Jump to the nearest symbol after point (`textDocument/documentSymbol',
flattened and sorted by position), wrapping to the first one if point is
at or after the last. Async -- see `lsp--goto-symbol'. To jump to any
symbol by name instead of stepping to the nearest one, see
`lsp-goto-symbol-by-name' (M47)."
  (interactive)
  (lsp--goto-symbol 'next))

(defun lsp-previous-symbol ()
  "Jump to the nearest symbol before point, wrapping to the last one if
point is at or before the first. See `lsp-next-symbol'."
  (interactive)
  (lsp--goto-symbol 'prev))

(defun lsp--symbol-alist (symbols)
  "Build a (DISPLAY . POS) alist from SYMBOLS (the (POS NAME KIND) list
`lsp--document-symbol-positions' returns), M47's
`lsp-goto-symbol-by-name' picker input. NAME collisions (overloaded
functions, same-named members on different classes, ...) get a
disambiguating \" (N)\" suffix appended to DISPLAY so every entry in the
returned alist maps back to exactly one POS -- without this,
`completing-read' REQUIRE-MATCH would still let the user pick a name,
but `assoc' below could only ever return the first of several matching
positions."
  (let ((counts (make-hash-table :test 'equal))
        (seen (make-hash-table :test 'equal))
        (out nil))
    (dolist (entry symbols)
      (let ((name (nth 1 entry)))
        (puthash name (1+ (gethash name counts 0)) counts)))
    (dolist (entry symbols)
      (let* ((pos (nth 0 entry))
             (name (nth 1 entry))
             (display name))
        (when (> (gethash name counts) 1)
          (let ((n (1+ (gethash name seen 0))))
            (puthash name n seen)
            (setq display (format "%s (%d)" name n))))
        (push (cons display pos) out)))
    (nreverse out)))

(defun lsp-goto-symbol-by-name ()
  "Jump to a symbol in the current buffer, chosen by name via
`completing-read' (M47) -- the picker `lsp-next-symbol'/
`lsp-previous-symbol' never had (see their own docstrings). Fetches
`textDocument/documentSymbol' fresh on every call, same no-cache
discipline as `lsp--goto-symbol' (see `lsp--buffer-symbols's doc for
why). Bound to `C-c l s' (M48 Part D, see simple.el)."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      (lsp--sync-buffer-now)
      (let ((p (lsp--text-document-only-params))
            (buf (current-buffer)))
        (lsp-request-async
         client "textDocument/documentSymbol" p
         (lambda (result)
           (when (eq (current-buffer) buf)
             (let ((symbols (lsp--document-symbol-positions result)))
               (if (not symbols)
                   (message "No symbols")
                 (let ((alist (lsp--symbol-alist symbols)))
                   (with-completing-read
                    (name "Go to symbol: " (mapcar #'car alist) t)
                    ;; Second staleness check: the first `(eq
                    ;; (current-buffer) buf)' above only guards
                    ;; opening the picker, but `goto-char' itself runs
                    ;; later, inside this callback, after the user has
                    ;; spent an arbitrary amount of time typing/
                    ;; picking in `completing-read' -- unlike
                    ;; `lsp--goto-symbol', whose check and jump are
                    ;; both in the same synchronous callback with no
                    ;; user-input gap in between.
                    (when (eq (current-buffer) buf)
                      (goto-char (cdr (assoc name alist))))))))))))))))

;;; --- M48: codeAction + rename ---------------------------------------------
;;
;; `lsp-code-action-at-point' and `lsp-rename' copy `lsp-goto-symbol-by-
;; name''s (M47) exact shape -- async request, callback, an optional
;; `with-completing-read' picker for a multi-item reply, a second
;; staleness check inside the picker's own callback -- plus
;; `lsp-format-buffer''s (M46) `buffer-modified-tick' write-command
;; discipline, since unlike a jump these two WRITE to the buffer.
;;
;; v1 scope, documented rather than silent:
;;  - A `CodeAction' response element without an `edit' (a plain
;;    `Command', or a `CodeAction' that only carries a `command' to run
;;    server-side -- both legal per the spec) is skipped: there is no
;;    `workspace/executeCommand' support here, and no `codeAction/
;;    resolve' round trip either (verible-verilog-ls, this milestone's
;;    target, always sends a ready-to-apply `edit' inline -- confirmed
;;    against the real binary, see PLAN.md's M48 pre-flight probe).
;;    Skipped elements are counted and reported in the final message
;;    rather than silently dropped.
;;  - A `WorkspaceEdit''s `changes' entry for a uri OTHER than the
;;    current buffer's own is skipped (`lsp--apply-workspace-edit-for-
;;    buffer' only ever touches the current buffer) -- also counted and
;;    reported, never silently. `documentChanges' (the spec's
;;    alternative to `changes', used by servers that need to express
;;    file creates/renames/deletes alongside edits) is not parsed at
;;    all; verible-verilog-ls sends `changes' for both codeAction and
;;    rename (confirmed against the real binary).
;;  - `lsp-rename' sends the request only after `with-read-string'
;;    delivers the new name, deliberately (not the other order): a
;;    server's `TextEdit' ranges describe positions in the document as
;;    of when it computed them, so they must be requested against
;;    (and applied to) the same snapshot -- reading the name first and
;;    syncing/sending after means no user keystroke can land between
;;    "snapshot the server saw" and "edits computed against it".

(defun lsp--diagnostics-at-point ()
  "Diagnostics (from `lsp-diagnostics', the hash-tables exactly as
published -- `code'/`source'/`relatedInformation' and everything else
untouched) whose range covers point in the current buffer. M48's
`lsp-code-action-at-point' sends this back to the server verbatim as
`context.diagnostics', so nothing here may drop or reshape a field even
though only `range' is read.

Containment is the LSP half-open convention [START, END): a diagnostic
covers POS when START <= POS < END, EXCEPT a zero-width diagnostic
\(START == END, e.g. some servers publish this for a whole-file issue
like `posix-eof') which instead covers exactly POS == START -- a
strictly half-open test can never match a zero-width range at all,
since START < START is always false.

Positions are converted via `lsp--pos-at-utf16' (M46's UTF-16-correct
path), NOT the older `lsp--pos-at' that `lsp--decorate-buffer' still
uses to draw the diagnostic's own squiggle underline -- see the file
header's M46 note on why the two coexist. This is a real, pre-existing
divergence carried forward rather than fixed here: on a line with an
astral-plane character before the diagnostic, the squiggle
`lsp--decorate-buffer' draws and the point range this function
considers \"inside\" the same diagnostic can disagree by a character or
two. Fixing `lsp--decorate-buffer' to match is the file header's own
tracked follow-up, not an M48-shaped change."
  (let ((client lsp--buffer-client)
        (file (buffer-file-name))
        (pos (point))
        (out nil))
    (when (and client file)
      (let* ((diags (lsp-diagnostics client file))
             (n (if diags (length diags) 0)))
        (dotimes (idx n)
          (let* ((d (aref diags idx))
                 (range (gethash "range" d))
                 (start (gethash "start" range))
                 (end (gethash "end" range))
                 (from (lsp--pos-at-utf16 (gethash "line" start) (gethash "character" start)))
                 (to (lsp--pos-at-utf16 (gethash "line" end) (gethash "character" end))))
            (when (if (= from to) (= pos from) (and (>= pos from) (< pos to)))
              (push d out))))))
    (nreverse out)))

(defun lsp--code-action-context (diags)
  "`CodeActionContext' params: `diagnostics' is DIAGS (a list, from
`lsp--diagnostics-at-point') as a vector -- an empty DIAGS still
produces a present-but-empty `diagnostics' vector, matching what a
`CodeActionContext' always carries per the spec (never an omitted
field)."
  (let ((h (make-hash-table)))
    (puthash "diagnostics" (apply #'vector diags) h)
    h))

(defun lsp--code-action-range (diags)
  "The `range' to send with `textDocument/codeAction': the first hit
diagnostic's own `range' when DIAGS (from `lsp--diagnostics-at-point')
is non-nil, otherwise a zero-width range at point."
  (if diags
      (gethash "range" (car diags))
    (let* ((lc (lsp--line-utf16-at (point)))
           (r (make-hash-table)))
      (puthash "start" (lsp--position (car lc) (cdr lc)) r)
      (puthash "end" (lsp--position (car lc) (cdr lc)) r)
      r)))

(defun lsp--code-action-usable (result)
  "Split RESULT (a `textDocument/codeAction' reply, expected a vector of
`CodeAction'/`Command' elements) into (USABLE . SKIPPED): USABLE is the
list, in RESULT's own order, of elements that are hash-tables carrying
an `edit' -- the only shape M48 v1 applies (see the file header for
why a `Command'-only element, or a `CodeAction' with no `edit' at all,
is not); SKIPPED is how many elements were not usable, for the
caller's final message."
  (let ((usable nil) (skipped 0) (n (if (vectorp result) (length result) 0)))
    (dotimes (idx n)
      (let ((action (aref result idx)))
        (if (and (hash-table-p action) (hash-table-p (gethash "edit" action)))
            (push action usable)
          (setq skipped (1+ skipped)))))
    (cons (nreverse usable) skipped)))

(defun lsp--code-action-alist (actions)
  "Build a (DISPLAY . ACTION) alist from ACTIONS (a list of usable
`CodeAction' hash-tables, from `lsp--code-action-usable'), M48's
`lsp-code-action-at-point' picker input. Same disambiguation discipline
as `lsp--symbol-alist' (M47) and for the identical reason: a
\" (N)\" suffix appended to DISPLAY on `title' collisions, so every entry maps
back to exactly one ACTION -- without it, `completing-read''s
REQUIRE-MATCH would still let the user pick a title, but `assoc' below
could only ever return the first of several actions sharing one."
  (let ((counts (make-hash-table :test 'equal))
        (seen (make-hash-table :test 'equal))
        (out nil))
    (dolist (action actions)
      (let ((title (gethash "title" action)))
        (puthash title (1+ (gethash title counts 0)) counts)))
    (dolist (action actions)
      (let* ((title (gethash "title" action))
             (display title))
        (when (> (gethash title counts) 1)
          (let ((n (1+ (gethash title seen 0))))
            (puthash title n seen)
            (setq display (format "%s (%d)" title n))))
        (push (cons display action) out)))
    (nreverse out)))

(defun lsp--workspace-edit-other-uris (edit this-uri)
  "URIs in EDIT's `changes' (a `WorkspaceEdit' hash-table) other than
THIS-URI -- the ones `lsp--apply-workspace-edit-for-buffer' will not
touch (v1 limitation, see the file header). Returns a list of uri
strings, or nil if there are none (including when EDIT carries no
`changes' at all)."
  (let ((changes (and (hash-table-p edit) (gethash "changes" edit)))
        (out nil))
    (when (hash-table-p changes)
      (maphash (lambda (uri _edits) (unless (equal uri this-uri) (push uri out)))
                changes))
    out))

(defun lsp--apply-workspace-edit-for-buffer (edit)
  "Apply EDIT (a `WorkspaceEdit' hash-table -- `{\"changes\":
{uri: [TextEdit]}}') to the CURRENT buffer via `lsp--apply-text-edits':
only the `changes' entry for THIS buffer's own `buffer-file-name' uri
is applied; any other uri is v1-not-supported (documented in the file
header, see `lsp--workspace-edit-other-uris' for the caller-facing way
to report them) and left completely untouched. Returns the number of
edits applied, or nil if EDIT carried none for this buffer's uri (no
`changes' at all, `changes' present but with no entry for this uri, or
an empty edits vector for it) -- the caller uses nil to tell \"nothing
to do here\" apart from \"did something\"."
  (let* ((changes (and (hash-table-p edit) (gethash "changes" edit)))
         (uri (lsp--path-to-uri (buffer-file-name)))
         (edits (and (hash-table-p changes) (gethash uri changes))))
    (when (and (vectorp edits) (> (length edits) 0))
      (lsp--apply-text-edits edits)
      (length edits))))

(defun lsp--code-action-skip-suffix (skipped other)
  "A \" (...)\" message suffix summarizing what
`lsp-code-action-at-point' did NOT apply: SKIPPED is the count of
response elements with no `edit' (from `lsp--code-action-usable'),
OTHER is the list of non-current-buffer uris in the applied action's
own `edit' (from `lsp--workspace-edit-other-uris'). Empty string when
both are nil/zero -- the common case, nothing to report."
  (let (parts)
    (when (> skipped 0)
      (push (format "%d action(s) skipped: no edit" skipped) parts))
    (when other
      (push (format "%d edit(s) in other file(s) skipped" (length other)) parts))
    (if parts (concat " (" (string-join (nreverse parts) "; ") ")") "")))

(defun lsp--apply-code-action (action tick skipped)
  "Apply ACTION's `edit' (a `WorkspaceEdit') to the current buffer via
`lsp--apply-workspace-edit-for-buffer', reporting what happened via
`message' -- always, unlike `lsp-format-buffer''s success case, since a
code action's own title is worth echoing back to confirm which one ran.

M48 write-command staleness discipline (same as `lsp-format-buffer',
see its docstring): if `buffer-modified-tick' no longer equals TICK
\(captured right after `lsp--sync-buffer-now', before the request was
sent), the edit is DROPPED with a `message' instead of applied -- the
user kept typing while the server was still computing, so the edit
describes a snapshot that no longer matches the live buffer. Checked
HERE, immediately before applying, rather than at the top of the
response callback: with a picker involved, an arbitrary amount of
additional typing can happen between the reply landing and the user
finishing their pick, so the check must be the last thing before the
write, not the first thing after the reply."
  (if (/= (buffer-modified-tick) tick)
      (message "lsp: buffer changed since code action request, discarding stale edit")
    (let* ((edit (gethash "edit" action))
           (title (gethash "title" action))
           (this-uri (lsp--path-to-uri (buffer-file-name)))
           (other (lsp--workspace-edit-other-uris edit this-uri))
           (n (lsp--apply-workspace-edit-for-buffer edit))
           (suffix (lsp--code-action-skip-suffix skipped other)))
      (if n
          (message "%s%s" title suffix)
        (message "lsp: %s: nothing to apply here%s" title suffix)))))

(defun lsp-code-action-at-point ()
  "Send `textDocument/codeAction' for the diagnostic(s) at point (via
`lsp--diagnostics-at-point'; a zero-width range at point if there are
none) and apply the resulting edit -- see the file header for the full
M48 v1 scope (no `Command' execution, no `codeAction/resolve', only the
current buffer's own uri out of a multi-file `edit').

Exactly one applicable action (an element with an `edit') applies it
immediately; more than one opens a `with-completing-read' picker keyed
by title (M47's `with-completing-read', same disambiguation/staleness
shape as `lsp-goto-symbol-by-name' -- see `lsp--code-action-alist' and
this function's own second staleness check below); none messages
\"No code actions here\" (an empty/absent reply) or \"No applicable
code actions here\" (a non-empty reply where every element lacked an
`edit').

Async (`lsp-request-async', same reason as every M46-forward
interactive LSP command -- see the file header's M46 note; M65 bounded
`lsp--await' itself, but a several-second freeze on every keystroke-
adjacent command is still worth avoiding, so the async path stays the
rule here). Same buffer-identity/`buffer-modified-
tick' write discipline as `lsp-format-buffer': TICK is captured right
after `lsp--sync-buffer-now', and checked again -- inside
`lsp--apply-code-action', immediately before applying, not at the top
of this callback -- since a picker can introduce an arbitrary
additional delay after the reply lands (identical to
`lsp-goto-symbol-by-name''s own second staleness check, for the same
reason: the FIRST `(eq (current-buffer) buf)' below only guards
opening the picker)."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      (lsp--sync-buffer-now)
      (let* ((diags (lsp--diagnostics-at-point))
             (p (lsp--text-document-only-params))
             (buf (current-buffer))
             (tick (buffer-modified-tick)))
        (puthash "range" (lsp--code-action-range diags) p)
        (puthash "context" (lsp--code-action-context diags) p)
        (lsp-request-async
         client "textDocument/codeAction" p
         (lambda (result)
           (when (eq (current-buffer) buf)
             (if (not (and (vectorp result) (> (length result) 0)))
                 (message "No code actions here")
               (let* ((split (lsp--code-action-usable result))
                      (usable (car split))
                      (skipped (cdr split)))
                 (cond
                  ((not usable)
                   (message "No applicable code actions here%s"
                            (lsp--code-action-skip-suffix skipped nil)))
                  ((= (length usable) 1)
                   (lsp--apply-code-action (car usable) tick skipped))
                  (t
                   (let ((alist (lsp--code-action-alist usable)))
                     (with-completing-read
                      (title "Code action: " (mapcar #'car alist) t)
                      ;; Second staleness check: see this function's own
                      ;; docstring.
                      (when (eq (current-buffer) buf)
                        (lsp--apply-code-action
                         (cdr (assoc title alist)) tick skipped))))))))))))))))

(defun lsp-rename ()
  "Read a new name (`with-read-string', M47) and send
`textDocument/rename' at point, applying the resulting `WorkspaceEdit'
the same way `lsp-code-action-at-point' does (`lsp--apply-workspace-
edit-for-buffer' -- current buffer's own uri only, see the file
header). Deliberately reads the name BEFORE syncing/sending the
request, not after (see the file header): a server's edits describe
positions as of the document snapshot it computed them against, so the
snapshot sent must be from right before the request, with no user
keystroke gap in between -- reading the name first, THEN syncing and
sending, guarantees that.

Async, same discipline as `lsp-code-action-at-point': BUF is captured
before `with-read-string' opens (an arbitrary delay while the user
types the new name is the same kind of gap `with-completing-read'
introduces for `lsp-goto-symbol-by-name'/`lsp-code-action-at-point', so
it gets the identical guard, checked again once the name is submitted);
TICK is captured right after `lsp--sync-buffer-now', and checked again
just before applying.

Empty `changes' (or a null result) messages \"Rename not available
here\" rather than applying nothing silently."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      (let ((buf (current-buffer)))
        (with-read-string
         (new-name "New name: ")
         (when (eq (current-buffer) buf)
           (lsp--sync-buffer-now)
           (let* ((lc (lsp--line-utf16-at (point)))
                  (p (lsp--text-document-position-params
                      (buffer-file-name) (car lc) (cdr lc)))
                  (tick (buffer-modified-tick)))
             (puthash "newName" new-name p)
             (lsp-request-async
              client "textDocument/rename" p
              (lambda (result)
                (when (eq (current-buffer) buf)
                  (cond
                   ((not (hash-table-p result))
                    (message "Rename not available here"))
                   ((/= (buffer-modified-tick) tick)
                    (message "lsp: buffer changed since rename request, discarding stale edit"))
                   (t
                    (let* ((this-uri (lsp--path-to-uri (buffer-file-name)))
                           (other (lsp--workspace-edit-other-uris result this-uri))
                           (n (lsp--apply-workspace-edit-for-buffer result)))
                      (cond
                       ((and (not n) other)
                        (message "lsp: rename touches other file(s) only, not applied (v1 limitation)"))
                       ((not n)
                        (message "Rename not available here"))
                       (other
                        (message "Renamed %d occurrence(s) here (also skipped %d edit(s) in other file(s))"
                                 n (length other)))
                       (t
                        (message "Renamed %d occurrence(s)" n)))))))))))))))))

;;; --- M26: lsp-server-alist, M-x lsp, async hover/definition/diagnostics ---
;;
;; Everything above this point is the M14 protocol client plus M15's
;; async plumbing (lsp-request-async/lsp-process-pending) and M16's
;; diagnostics decoration -- none of it connects to anything on its
;; own. This section is the major-mode wiring that actually drives it
;; from the keyboard: a registry of which server to start per major
;; mode, project-root detection, a connect command that degrades
;; cleanly when the server binary is missing or dies, and async
;; hover/definition/diagnostic-navigation commands that never block
;; editing (the blocking `lsp-hover'/`lsp-definition' above stay
;; around for scripts and tests; only the interactive path is async).

(defvar lsp-language-id-alist
  '((rust-mode . "rust")
    (c-mode . "c")
    (c++-mode . "cpp")
    (python-mode . "python")
    (sh-mode . "shellscript")
    (java-mode . "java")
    (perl-mode . "perl")
    (verilog-mode . "verilog"))
  "Alist of MAJOR-MODE -> the `textDocument/didOpen' `languageId' string
LSP servers expect for it (see the LSP spec's language identifier
list). Consulted by `lsp' when calling `lsp-did-open'; a mode with no
entry here yields nil from this alist, and `lsp-did-open' falls back
to its own default (\"rust\" -- see its docstring) in that case, so
modes that don't need this alist at all keep working unchanged (e.g.
clangd/pyright infer the language from the URI's file extension
instead of trusting this field). `verilog-mode' buffers use \"verilog\"
for both `.v' and `.sv' files (see `auto-mode-alist' in modes.el) --
v1 does not distinguish plain Verilog from SystemVerilog, matching
common LSP-server convention.")

(defvar lsp-server-alist
  '((rust-mode . ("rust-analyzer"))
    (c-mode . ("clangd"))
    (c++-mode . ("clangd"))
    ;; pyright-langserver, not the `pyright' CLI (that's a one-shot
    ;; checker that prints and exits -- doesn't speak LSP at all) and
    ;; not pylsp (weaker type inference). --stdio is mandatory: the
    ;; server refuses to start without an explicit transport flag.
    (python-mode . ("pyright-langserver" "--stdio"))
    (sh-mode . ("bash-language-server" "start"))
    (java-mode . ("jdtls"))
    (perl-mode . ("pls"))
    ;; verible-verilog-ls speaks LSP over stdio with no flags needed;
    ;; it covers both plain Verilog and SystemVerilog.
    (verilog-mode . ("verible-verilog-ls")))
  "Alist of MAJOR-MODE -> (COMMAND . ARGS), consulted by `lsp' to pick
a server for the current buffer. Registration only: an entry here
starts nothing by itself. Override the default for a mode the GNU
way, e.g.
  (add-to-list \\='lsp-server-alist \\='(rust-mode . (\"my-analyzer\" \"--flag\")))
or to switch `verilog-mode' to svls, another common SystemVerilog LSP
server, in place of the `verible-verilog-ls' default:
  (add-to-list \\='lsp-server-alist \\='(verilog-mode . (\"svls\")))

`slang-server' (built on the slang frontend) is the other one worth
knowing about, and the choice is NOT a strict upgrade in either
direction -- measured 2026-08-11 with `dev/lsp-probe.py' against
verible-verilog-ls and slang-server 0.2.9 on the same `demo/rtl/'
sources:

  * slang-server elaborates. `textDocument/hover' on a signal answers
    with the RESOLVED type and width (`logic[31:0]', `Width: 32',
    `Driver: Continuous'), and cross-file `definition' works off its own
    workspace index with no file list. It also has a real
    `completionProvider' (with `resolveProvider'), which verible does
    not advertise at all.
  * verible formats. slang-server declares NO
    `documentFormattingProvider' and no `documentRangeFormattingProvider'
    key at all -- and a real `textDocument/formatting' request sent to
    it anyway gets back nothing, not even an error, for the full 8-second
    probe window (measured 2026-08-11). verible has both, plus its style
    linter. As of M57, switching to slang-server does not send that
    doomed request any more: `lsp-format-buffer'/`lsp-format-region'
    check the capability first and `message' instead, and `M-x lsp'
    reports the gap once at connect time (since a `before-save-hook'
    message alone is invisible -- `save-buffer' overwrites the echo area
    right after). Before M57 this was a silent hang of the callback,
    forever.

So the honest summary is: verible for formatting and style, slang-server
for semantics. Neither dominates.
  (add-to-list \\='lsp-server-alist \\='(verilog-mode . (\"slang-server\")))

Note the local Verilog tiers (`verilog-complete.el' port-name
completion, `verilog-nav.el' module jump) run BEFORE any of these and
need no server at all -- and in the same probe run slang-server returned
zero candidates at a port-connection `.' (46 candidates at an ordinary
identifier position, so completion itself works), which is exactly the
position the local tier covers.")

;; --- Project root detection ---

(defvar lsp--project-root-markers
  '(".git" "Cargo.toml" "pyproject.toml" "compile_commands.json"
    "package.json" "verible.filelist" ".svls.toml" ".slang")
  "Files/directories `lsp--project-root' looks for. Walking upward from
a buffer's file, the nearest ancestor directory containing any one of
these wins.

`.slang' is slang-server's own configuration directory (it watches
`.slang/**/*.json'), and it is on this list for a reason worth writing
down, because getting it wrong looks exactly like the server being
broken. slang-server INDEXES THE WORKSPACE: given a root it reads every
`.sv'/`.svh'/`.v'/`.vh' underneath, and answers cross-file questions
from that index with no file list needed. Given the WRONG root -- the
edited file's own directory, say -- it indexes that one directory,
finds nothing, and returns an empty result for every cross-file request
while still advertising full capabilities.

Measured 2026-08-11 with `dev/lsp-probe.py' against slang-server 0.2.9
on `demo/rtl/': `textDocument/definition' on an instantiated module's
type name returned `[]' with rootUri at the file's own directory
(`demo/rtl/top/'), and the correct `demo/rtl/core/alu.sv' location with
rootUri at `demo/rtl/'. Same server, same file, same request -- the
only difference was this list. (That probe run is also why
`dev/lsp-probe.py' grew a `--root' flag: its old fixed
rootUri-is-the-file's-directory made every workspace-indexing server
look incapable.)")

(defun lsp--dir-has-marker-p (dir)
  "Non-nil if DIR (a directory name with trailing slash) directly
contains one of `lsp--project-root-markers'."
  (catch 'lsp--marker-found
    (dolist (marker lsp--project-root-markers)
      (when (file-exists-p (concat dir marker))
        (throw 'lsp--marker-found t)))
    nil))

(defun lsp--remote-path-p (file)
  "Non-nil if FILE names a remote (`/ssh:'-prefixed) path, nil if FILE
is nil. Both `lsp' and `lsp--auto-attach-client' must reject a remote
path BEFORE calling `lsp--project-root' -- see that function's own
docstring for why (the ancestor-directory marker walk shells out one
real `ssh' per `file-exists-p' call) -- so this predicate is shared
between the two rather than each reimplementing the same
`string-prefix-p' check, which would otherwise be free to drift apart
over time."
  (and file (string-prefix-p "/ssh:" file)))

(defun lsp--project-root (file)
  "Project root for FILE: the nearest ancestor directory -- starting at
FILE's own directory and walking up -- containing one of
`lsp--project-root-markers', or FILE's own directory if none do.
Returned without a trailing slash, matching `lsp-connect's ROOT-PATH."
  (let* ((start (file-name-directory (expand-file-name file)))
         (dir start)
         (found nil))
    (while (and dir (not found))
      (if (lsp--dir-has-marker-p dir)
          (setq found dir)
        (let ((parent (file-name-directory (directory-file-name dir))))
          (setq dir (if (and parent (not (string= parent dir))) parent nil)))))
    (directory-file-name (or found start))))

;; --- Connecting: M-x lsp ---

(defvar lsp--connections nil
  "Alist of ((COMMAND . ROOT) . CLIENT): one entry per live connection
`lsp' has started, so revisiting the same project reuses the existing
server instead of spawning a duplicate. See `lsp--get-connection'.")

(defun lsp--get-connection (command root)
  "Live client already connected to COMMAND for project ROOT, or nil.
An entry whose process has since died is discarded here rather than
handed back, so the caller always gets either a live client or nil.
Mirrors `lsp-process-pending-all's `lsp-connection-p' + `lsp-live-p'
guard: `lsp-live-p' signals wrong-type-argument on a non-connection,
which a client built with a nil :conn (only ever a hand-built test
fixture -- `lsp-connect' itself never produces one) would otherwise
trip over here."
  (let ((entry (assoc (cons command root) lsp--connections)))
    (when entry
      (let ((conn (lsp--client-conn (cdr entry))))
        (if (and (lsp-connection-p conn) (lsp-live-p conn))
            (cdr entry)
          (setq lsp--connections (delq entry lsp--connections))
          nil)))))

(defvar lsp--buffer-client nil
  "Buffer-local: the `lsp--client' this buffer talks to. Set by `lsp'
the first time it connects successfully in a buffer; nil if `lsp'
hasn't been run here (or failed). `lsp-hover-at-point',
`lsp-definition-at-point', and the diagnostic-navigation commands all
read this rather than taking a client argument.")

(defvar lsp--last-synced-tick nil
  "Buffer-local (M35): the `buffer-modified-tick' as of the last
`textDocument/didOpen' or `textDocument/didChange' sent for this
buffer. Set alongside `lsp--buffer-client' by `lsp' right after
didOpen, and updated by `lsp--sync-buffer-now' after each didChange.
Not a strict iff with `lsp--buffer-client' (a dead client clears that
variable but leaves this one; test stand-ins set that one without
this) -- the guarantee that matters is one-directional: this stays
nil until a real didOpen has been sent here, and
`lsp--sync-buffer-now' requires BOTH a live client and a non-nil tick
record, so a didChange can never precede its didOpen.")

(defun lsp--error-string (err)
  "Best-effort human-readable text for a `condition-case' ERR object.
Every error this client raises -- a spawn failure from `lsp-start' or
a dead-server report from `lsp--await' -- carries its message as a
single string in the condition data, so that string is unwrapped
directly; anything else falls back to printing the whole condition."
  (let ((data (cdr err)))
    (if (and (consp data) (stringp (car data)) (null (cdr data)))
        (car data)
      (format "%S" err))))

(defun lsp--server-for-mode (mode)
  "(COMMAND . ARGS) registered for MODE in `lsp-server-alist', or nil."
  (cdr (assq mode lsp-server-alist)))

(defvar lsp-auto-attach t
  "When non-nil, a newly visited file that's in a project already
`M-x lsp'-connected gets attached to that same connection automatically
-- no second `M-x lsp'. Purely a REUSE mechanism: it never starts a
server itself, so the first file in a project still needs one real
`M-x lsp' (see `lsp--maybe-auto-attach' and `lsp--auto-attach-client'
for why nothing here is allowed to spawn -- `lsp-connect'/`lsp--await'
are fully synchronous, all the way down to `crates/elisp/src/lsp.rs's
`recv_blocking'/`recv_timeout' (M65 gave `lsp--await' a bound via
`lsp-initialize-timeout', default 10s, so a dead-air server now signals
an error instead of hanging forever -- but nothing here is allowed to
spawn regardless: this note is about no code path below starting a NEW
connection, not about whether the existing ones still block).

\"Same project\" for this purpose is exact: (server COMMAND . project
ROOT) must match a live entry in `lsp--connections', where ROOT comes
from `lsp--project-root' walking up for one of
`lsp--project-root-markers'. In a tree with no marker at all, every
directory is its own root and auto-attach never crosses a directory
boundary -- an RTL project that wants its subdirectories to share one
connection needs a marker (e.g. `verible.filelist') at its top, same
as `M-x lsp' itself already requires for manual reuse across files in
different directories.

Set to nil to require an explicit `M-x lsp' in every buffer.")

(defun lsp--attach-current-buffer (client mode)
  "Associate the current buffer with CLIENT: `lsp-did-open's it and
records CLIENT buffer-locally as `lsp--buffer-client', with
`lsp--last-synced-tick' set to the tick just didOpen'd against. MODE is
the buffer's major mode, used to look up the `languageId' string in
`lsp-language-id-alist'.

Shared by `lsp' (the first, interactive attach) and
`lsp--maybe-auto-attach'/`lsp--auto-attach-backfill' (silent,
connection-reuse attaches) so the three can never drift apart.

If `lsp-did-open' signals -- most concretely the stdin-closed race in
`lsp-send' documented at its own definition -- both buffer-locals are
rolled back to nil before the error is re-signaled, so a failed attach
leaves the buffer in exactly the state it was in before this was
called. This fixes a latent bug `lsp' used to have on its own: it used
to `setq-local lsp--buffer-client' BEFORE calling `lsp-did-open', so a
didOpen failure left the buffer with a client set but
`lsp--last-synced-tick' still nil -- every other LSP command trusts
`lsp--buffer-client' (via `lsp--live-buffer-client', which only checks
the connection object, not the tick) and would have kept sending
requests for a document the server was never told was open. Callers
that want the previous user-visible degradation (a `message' instead
of a signal) should wrap this in `condition-case' themselves, same as
`lsp' below does.

Known v1 gap, NOT addressed here (M63 coordinator round 2): `lsp-did-
open' -> `lsp-send' -> the Rust `write_message' (`crates/elisp/src/
lsp.rs') is a synchronous `write_all' to the server's stdin. That's a
DIFFERENT blocking mechanism than the one this milestone's docstrings
elsewhere reason about and rule out (`lsp-connect'/`lsp--await'/
`recv_blocking', which only fire on a fresh connection, never on an
attach to an EXISTING one) -- a sufficiently large document and a slow-
draining server could still block here. Pre-existing (`M-x lsp' always
had this), not introduced by M63, but M63 does change how it's
triggered: from a deliberately-pressed command to something that can
fire from `find-file' and, via backfill, several times in a row."
  (setq-local lsp--buffer-client client)
  (condition-case err
      (progn
        (lsp-did-open client (buffer-file-name) (buffer-string)
                      (cdr (assq mode lsp-language-id-alist)))
        ;; Same tick didOpen just sent as its version (M35): no edit can
        ;; land between the two calls, so this is the baseline
        ;; `lsp--sync-buffer-now' diffs future edits against.
        (setq-local lsp--last-synced-tick (buffer-modified-tick)))
    (error
     (setq-local lsp--buffer-client nil)
     (setq-local lsp--last-synced-tick nil)
     (signal (car err) (cdr err)))))

(defun lsp--auto-attach-client (file mode)
  "Client to silently reuse for FILE/MODE, or nil if nothing should be
auto-attached. Pure lookup -- touches no buffer state and never
messages -- so its remote-path guard can be unit-tested directly with
no `find-file' involved at all (there's no `set-visited-file-name'
shortcut that fakes a `/ssh:' path, and actually `find-file'-ing one
would really shell out).

Guard order is significant -- cheapest and most-common-case-first:

1. `lsp-auto-attach' nil -> nil immediately.
2. `lsp--connections' nil (this session has never started any LSP
   server) -> nil immediately, so a session that never touches LSP
   pays exactly one variable read on every `find-file' (mirrors
   `lsp-process-pending-all's `(when lsp--clients ...)' short circuit).
3. FILE nil (buffer visiting no file) -> nil.
4. FILE has a `/ssh:' prefix (`lsp--remote-path-p') -> nil, checked
   BEFORE `lsp--project-root' (guard 6): that walks every ancestor directory
   calling `lsp--dir-has-marker-p', which tries up to 8 markers via
   `file-exists-p' per directory. For a remote path each of those
   calls shells out a real `ssh ... test -e' (see
   `crates/core/src/builtins/files.rs' and `crates/core/src/
   remote.rs'), so a depth-6 remote path would cost ~48 synchronous
   ssh invocations just to decide whether to auto-attach -- on the
   open-file path, no less.
5. No `lsp-server-alist' entry for MODE -> nil.
6. No live connection already open for (COMMAND . `lsp--project-root'
   of FILE) -> nil.

Deliberately NOT prefix-matching FILE's path against a connection's
root (\"root is an ancestor of file\"): that would be a second, looser
definition of \"same project\" than `lsp--project-root' itself uses,
and the two could disagree -- one buffer in a project attaching to a
different server instance than another buffer in the same project."
  (cond
   ((not lsp-auto-attach) nil)
   ((not lsp--connections) nil)
   ((not file) nil)
   ((lsp--remote-path-p file) nil)
   (t
    (let ((entry (lsp--server-for-mode mode)))
      (when entry
        (lsp--get-connection (car entry) (lsp--project-root file)))))))

(defun lsp--auto-attach-backfill (mode client)
  "Called by `lsp' right after it successfully attaches the current
buffer to CLIENT (major MODE): walks `buffer-list' and silently
attaches every OTHER file-visiting buffer `lsp--auto-attach-client'
would also route to CLIENT and isn't already live-attached to it.

Why this exists on top of `lsp--maybe-auto-attach': that hook only
fires on `find-file', so it only ever helps buffers opened AFTER a
project's connection already exists. A buffer opened BEFORE the first
`M-x lsp' in a project -- a CLI argument, several dired/`M-.' jumps
made before anyone thought to connect -- already passed through
`find-file-hook' while `lsp--auto-attach-client' still had nothing to
reuse, and `find-file-hook' never re-runs for a buffer that's already
visiting its file. Without this, \"one `M-x lsp' attaches the whole
project\" would only be true for files opened afterward.

Reuses `lsp--auto-attach-client' rather than re-deriving the guard
logic, so a remote buffer is skipped here for the same reason it's
skipped on the `find-file-hook' path, and any future guard change only
has one place to edit. Each buffer's attach is independently wrapped
in `condition-case': one failure can't take down the rest of the
sweep, and can't disturb `lsp''s own already-successful result for the
buffer that called this. Entirely silent, matching
`lsp--maybe-auto-attach'. `with-current-buffer' (the `lambda'-wrapping
macro over `with-current-buffer-internal', see simple.el) restores the
original current buffer when done, mirroring `lsp-process-pending-
all's own use of the same primitive.

The \"already attached\" guard checks `lsp--live-buffer-client', not the
raw `lsp--buffer-client' (M63 round 2 fix): a buffer whose OWN
connection died earlier still has a non-nil `lsp--buffer-client'
pointing at the corpse, and a plain truthy check would mistake that for
\"already covered\" and skip it forever -- even though the connection
CLIENT points to here is a fresh, live one the corpse's buffer should
now be talking to instead. `lsp--live-buffer-client's clearing side
effect (nil-ing out a dead reference) is exactly what's wanted here
too, and it fires on the right buffer since this runs inside
`with-current-buffer'.

The `condition-case' wraps the WHOLE per-buffer body, guard evaluation
included -- not just the attach call. Same shape, and for the same
reason, as `lsp--maybe-auto-attach': an error raised while merely
DECIDING about one buffer would otherwise unwind the entire `dolist',
so every remaining buffer would go un-backfilled and -- worse -- the
error would surface from `lsp's own `condition-case' as \"LSP: failed
to start ...\" even though the connection was already established and
the user's own buffer already attached. Today none of the four guards
can actually signal (checked one by one, M63 tail review), so this is
symmetry against a future failure path rather than a fix for a live
one; the asymmetry is exactly what the tail review flagged."
  (dolist (buf (buffer-list))
    (condition-case nil
        (with-current-buffer buf
          (let ((file (buffer-file-name)))
            (when (and file
                       (eq (major-mode-internal-get) mode)
                       (not (lsp--live-buffer-client))
                       (eq (lsp--auto-attach-client file mode) client))
              (lsp--attach-current-buffer client mode))))
      (error nil))))

(defun lsp ()
  "Connect the current buffer to the LSP server registered for its
major mode in `lsp-server-alist'. Reuses a live connection for the
same server command + project root if one already exists; otherwise
starts a new server and performs the `initialize' handshake via
`lsp-connect'. On success, `lsp-did-open's the current buffer via
`lsp--attach-current-buffer' (remembering the client buffer-locally in
`lsp--buffer-client'), then silently backfills every other
already-open buffer that connection now covers -- see
`lsp--auto-attach-backfill'.

Never signals: a mode with no `lsp-server-alist' entry, a buffer
visiting no file, a remote (`/ssh:') buffer, or a server command that
fails to start (spawn failure, the process dying mid-handshake, or
`lsp-did-open' itself failing) are all reported with `message' instead
of an error, so a missing or broken LSP server can never disrupt
editing.

M66: a buffer visiting a `/ssh:' path (`lsp--remote-path-p') is turned
away here, before `lsp--project-root' is ever called on it. Two
independent reasons, not one:
  (a) cost -- `lsp--project-root' walks every ancestor directory of
      FILE trying up to 8 markers each via `file-exists-p', and for a
      remote path every one of those calls really shells out an `ssh
      ... test -e'. Measured against a fake-ssh hook on the real TUI:
      a 4-directory-deep `/ssh:' path cost 32 synchronous ssh
      invocations and froze the editor for 6.15s at a 150ms RTT --
      and 32 * `ConnectTimeout=5' = 160s if the host is unreachable.
      See `lsp--auto-attach-client''s guard 4 docstring for the same
      finding on the auto-attach path (fixed first, M63-era).
  (b) even a successful root walk wouldn't help: the server this
      spawns runs on THIS machine, not the remote host, so its
      `initialize'/`didOpen' would carry a URI built from the raw
      `/ssh:host:/path' string -- `file:///ssh:host:/path'. That is a
      well-formed `file' URI (colons are legal inside a path segment),
      which is exactly what makes it dangerous rather than loudly
      broken: it is accepted silently and the server answers as if
      nothing were wrong, because everything derivable from the
      `didOpen' TEXT alone still works. What breaks is everything that
      resolves against the filesystem, since the path names a LOCAL
      file that does not exist: Verilog include directives, packages
      defined in sibling files (`soc_pkg' and friends, i.e. most of a
      real RTL tree), and cross-file go-to-definition. Measured
      against `slang-server' 0.2.9 on `demo/rtl/core/alu.sv': the same
      file goes from 2 diagnostics under its real absolute URI to 13
      under the `/ssh:'-prefixed one, 11 of them unknown-class-or-
      package errors for `soc_pkg', while `documentSymbol' --
      answerable from the text alone -- is identical either way. A server whose
      project discovery reads files under the root itself rather than
      through the protocol (`verible-verilog-ls' and its
      `verible.filelist') loses that too, for the same reason and by
      the same mechanism, though the numbers above are slang's only.
      So the failure mode is a server that looks connected and answers
      confidently about a fraction of the design. There's no v1 fix
      that makes remote LSP work; this guard only stops the doomed,
      expensive attempt."
  (interactive)
  (let* ((mode (major-mode-internal-get))
         (entry (lsp--server-for-mode mode))
         (file (buffer-file-name)))
    (cond
     ((not entry) (message "No LSP server registered for %s" mode))
     ((not file) (message "Buffer is not visiting a file"))
     ((lsp--remote-path-p file)
      (message "LSP: remote (/ssh:) files are not supported"))
     (t
      (let* ((command (car entry))
             (args (cdr entry))
             (root (lsp--project-root file)))
        (condition-case err
            (let ((client (or (lsp--get-connection command root)
                               (let ((new (lsp-connect command args root)))
                                 (push (cons (cons command root) new)
                                       lsp--connections)
                                 new))))
              ;; didOpen only on the first association of this buffer
              ;; with this client: the LSP spec forbids re-opening an
              ;; already-open document (updates go through didChange),
              ;; so a second M-x lsp in the same buffer just reports.
              (if (eq lsp--buffer-client client)
                  (message "LSP: already connected to %s" command)
                (lsp--attach-current-buffer client mode)
                ;; Backfill BEFORE the closing `message': `lsp' is
                ;; `interactive' and its return value is this whole `if'
                ;; form's value, which callers/tests read as the reported
                ;; text -- `lsp--auto-attach-backfill' must not become the
                ;; last form or that value silently changes to whatever
                ;; `dolist' returns (nil) instead of the message string.
                (lsp--auto-attach-backfill mode client)
                (let ((note (lsp--unsupported-features-note client)))
                  (if note
                      (message "LSP: connected to %s (%s)" command note)
                    (message "LSP: connected to %s" command)))))
          (error
           (message "LSP: failed to start %s: %s"
                    command (lsp--error-string err)))))))))

(defun lsp--live-buffer-client ()
  "This buffer's `lsp--buffer-client', unless its server has died --
then nil, clearing the stale buffer-local reference on the way out so
later calls stop probing a dead process. The interactive commands
(`lsp-hover-at-point', `lsp-definition-at-point') go through this so a
server that died after connecting degrades to a readable message
instead of leaking a raw \"stdin closed\" signal from `lsp-send'.

Only a real, probe-able connection that reports dead is rejected:
values that aren't `lsp--client' structs, or structs without a real
connection object, pass through untouched -- production code only ever
stores `lsp-connect'-produced clients here, while tests stub this slot
with stand-ins on purpose."
  (when lsp--buffer-client
    (let ((conn (and (lsp--client-p lsp--buffer-client)
                     (lsp--client-conn lsp--buffer-client))))
      (if (and (lsp-connection-p conn) (not (lsp-live-p conn)))
          (progn (setq-local lsp--buffer-client nil) nil)
        lsp--buffer-client))))

;; --- Edit sync: textDocument/didChange (M35) ---

(defun lsp--sync-buffer-now ()
  "If the current buffer has a live client (`lsp--live-buffer-client')
and has been edited since the tick last synced to the server, send a
full-text `textDocument/didChange' with the current `buffer-modified-
tick' as its version and record it as `lsp--last-synced-tick'.
Otherwise a silent no-op, same \"never signals\" spirit as
`lsp-process-pending-all': no live client, a buffer not visiting a
file, or `lsp--last-synced-tick' still nil (this buffer was never
didOpen'd -- only `lsp' ever sets it, right after didOpen, so a
didChange before the matching didOpen can't happen) or unchanged are
all just \"nothing to do\".

Two call sites, both wanting the server to answer against text it has
actually seen: the idle pump (`lsp-process-pending-all', once over
every buffer -- typing itself never triggers this, only a later idle
tick does, a natural debounce) and `lsp-hover-at-point'/
`lsp-definition-at-point', immediately before sending their request so
an edit is never left unsynced across a hover/definition round trip
even if the idle pump hasn't run yet."
  (let ((client (lsp--live-buffer-client)))
    (when (and client (lsp--client-p client)
               (buffer-file-name)
               lsp--last-synced-tick
               (/= (buffer-modified-tick) lsp--last-synced-tick))
      (let ((tick (buffer-modified-tick)))
        (lsp-did-change client (buffer-file-name) (buffer-string) tick)
        (setq-local lsp--last-synced-tick tick)))))

;; --- Save/kill hooks: textDocument/didSave, textDocument/didClose (M40) ---

(defun lsp--on-after-save ()
  "`after-save-hook' function: if this buffer has a live LSP client and
has been didOpen'd (`lsp--last-synced-tick' non-nil), gets the server's
copy caught up via `lsp--sync-buffer-now' -- `lsp-did-save' sends no
text of its own, so the server must already have the saved content
from a didChange -- then sends `textDocument/didSave'.

Wrapped in `condition-case' and a silent no-op on every other path
(no client, or a buffer never didOpen'd): `save-buffer' runs this on
every save in every buffer, LSP-connected or not, across this whole
editor's test suite, so anything else here would spam the echo area
on an ordinary save."
  (condition-case nil
      (let ((client (lsp--live-buffer-client)))
        (when (and client lsp--last-synced-tick (buffer-file-name))
          (lsp--sync-buffer-now)
          (lsp-did-save client (buffer-file-name))))
    (error nil)))

(defun lsp--on-kill-buffer ()
  "`kill-buffer-hook' function: if this buffer has a live LSP client and
has been didOpen'd, sends `textDocument/didClose' and drops this
buffer's URI from the client's diagnostics alist. Same silent-no-op/
`condition-case' discipline as `lsp--on-after-save' -- every buffer
kill in the test suite runs this hook, LSP-connected or not."
  (condition-case nil
      (let ((client (lsp--live-buffer-client)))
        (when (and client lsp--last-synced-tick (buffer-file-name))
          (let ((uri (lsp--path-to-uri (buffer-file-name))))
            (lsp-did-close client (buffer-file-name))
            (setf (lsp--client-diagnostics client)
                  (delq (assoc uri (lsp--client-diagnostics client))
                        (lsp--client-diagnostics client))))))
    (error nil)))

;; --- Auto-attach: reusing a live connection for a newly opened file (M63) ---

(defun lsp--maybe-auto-attach ()
  "`find-file-hook' function: silently attaches the current buffer to
an already-live LSP connection if `lsp--auto-attach-client' finds one
-- so opening a second (or third, ...) file in a project that's
already `M-x lsp'-connected doesn't need its own manual `M-x lsp'.
Only ever reuses, never spawns: the first file in a project still
needs one real `M-x lsp' (see `lsp-auto-attach's doc for why nothing
here can time-bound `lsp-connect').

Why `find-file-hook': it's the one place all of this editor's
open-file paths funnel through -- the CLI (`src/main.rs'), `C-x C-f'
(`simple.el'), evil `:e' (`evil.el'), dired (`dired.el'), org links
(`org.el'), Verilog module navigation (`verilog-nav.el', synchronous),
and even the two LSP-driven jumps themselves (`lsp-definition-at-
point's async callback and the references-list jump) -- all end up in
`find-file-internal' (`editing.rs'), which runs this hook AFTER
`run_normal_mode' has already dispatched the major mode, so
`lsp--server-for-mode' has an answer to give. Buffer-reuse (revisiting
an already-open buffer) skips those two lines entirely, so this hook
simply never runs a second time for the same buffer -- no separate
\"already attached\" flag needed.

Silent on every path, success included: this hook runs alongside
others on `find-file-hook' (`evil--maybe-init-current-buffer', and
Verilog navigation's own post-jump `message') that also write to the
single-slot echo area, so a message from here would only sometimes be
visible -- the same unreliable-echo-area problem `lsp.el' has already
documented three times over (see `lsp-hover-at-point',
`lsp-definition-at-point', and `lsp--goto-diagnostic''s neighbors). The
mode line's `LSP' segment (`redisplay.rs') is the reliable per-buffer
signal instead; a failed auto-attach is invisible by the same logic
that makes a failed `M-x lsp' invisible until you actually use it --
the next command that needs the connection reports \"No LSP server
connected in this buffer (M-x lsp first)\".

The ENTIRE body -- including the `lsp--auto-attach-client' lookup
itself, not just the attach -- runs inside one `condition-case' (M63
round 2). `lsp--auto-attach-client' is a pure judgment function with no
error handling of its own by design (its own doc), so a malformed
`lsp-server-alist' entry (e.g. `(mode . \"cmd\")' instead of `(mode .
(\"cmd\"))', which makes `(car entry)' signal wrong-type-argument) used
to escape this function entirely and get caught by `find-file-hook's
own runner (`run_hook_by_name' in commands.rs), which echoes hook
errors to `Interp::out' -- so every single `find-file' in the buggy
window printed an LSP error. `M-x lsp' itself (`lsp.el's `lsp' function,
around its own `let*') has the same shape of gap -- its `command'/
`args'/`root' bindings sit outside ITS `condition-case' too -- but that
one is triggered by a deliberately-pressed command, not by every
`find-file', so M63 leaves it alone rather than touching code outside
this milestone's scope."
  (condition-case nil
      (let* ((file (buffer-file-name))
             (mode (major-mode-internal-get))
             (client (lsp--auto-attach-client file mode)))
        (when client
          (lsp--attach-current-buffer client mode)))
    (error nil)))

(add-hook 'after-save-hook 'lsp--on-after-save)
(add-hook 'kill-buffer-hook 'lsp--on-kill-buffer)
(add-hook 'find-file-hook 'lsp--maybe-auto-attach)

;; --- Async hover: C-h . ---

(defun lsp--line-character-at (pos)
  "0-based (LINE . CHARACTER) at buffer position POS -- LSP's
coordinate system, the inverse of `lsp--pos-at'.

Known v1 limitation: CHARACTER counts Unicode scalar values (this
editor's native buffer positions), while LSP specifies UTF-16 code
units. The two agree for every BMP character -- all of ASCII, CJK,
etc. -- and drift by one per astral-plane character (emoji and other
supplementary-plane codepoints) earlier in the same line. Same caveat
applies to the inverse direction in `lsp--pos-at'."
  (save-excursion
    (goto-char pos)
    (cons (1- (line-number-at-pos))
          (- (point) (line-beginning-position)))))

(defun lsp-hover-at-point ()
  "Async `textDocument/hover' at point. Returns immediately; when the
server answers (delivered by the editor's idle pump), the result shows
via `show-hover-popup' -- a floating popup in the GUI, the echo area
in the TUI. Typing is never blocked waiting for the answer."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      ;; M35: an edit sitting unsynced since the last idle tick must
      ;; reach the server before this request, or the answer would be
      ;; about stale text.
      (lsp--sync-buffer-now)
      (let ((lc (lsp--line-character-at (point))))
        (lsp-hover-async client (buffer-file-name) (car lc) (cdr lc)))))))

;; --- Async completion: C-M-i --------------------------------------------

(defun lsp--completion-prefix-char-p (c)
  "Non-nil if C belongs to an LSP completion prefix -- letters, digits,
`_'. Deliberately narrower than `dabbrev--prefix-char-p': no `-', the
ordinary identifier character class for the languages registered in
`lsp-server-alist'."
  (and c (or (and (>= c ?a) (<= c ?z))
             (and (>= c ?A) (<= c ?Z))
             (and (>= c ?0) (<= c ?9))
             (= c ?_))))

(defun lsp--completion-prefix-start (pos)
  "The start of the run of `lsp--completion-prefix-char-p' characters
ending at POS -- POS itself when POS sits right after a non-prefix
character (or at `point-min'), i.e. an empty prefix. Same shape as
`dabbrev--prefix-start', just this file's own narrower character class."
  (let ((p pos))
    (while (and (> p (point-min))
                (lsp--completion-prefix-char-p (char-after (1- p))))
      (setq p (1- p)))
    p))

(defun lsp--completion-item-label (item)
  (gethash "label" item))

(defun lsp--completion-item-insert-text (item)
  "ITEM's insert text (M44-3): `textEdit.newText' when ITEM carries a
`textEdit', else `insertText', else `label'. `additionalTextEdits' are
never applied (v1, documented in the file header). Snippet syntax
inside `newText' (e.g. \"${1:x}\") is inserted as literal text -- v1
does not parse or expand snippets, also documented in the file header."
  (let* ((edit (gethash "textEdit" item))
         (new-text (and (hash-table-p edit) (gethash "newText" edit))))
    (or new-text (gethash "insertText" item) (gethash "label" item))))

(defun lsp--completion-item-start (item prefix-start)
  "ITEM's replacement start (M44-3): `textEdit.range.start', converted
to a buffer position via `lsp--pos-at', when ITEM carries a `textEdit';
PREFIX-START otherwise -- the identifier-run start every candidate
without its own `textEdit' falls back to, same as v1's original
prefix-only behavior. `textEdit.range.end' is never consulted: v1
always deletes START..point on accept, clamping away from whatever the
server's END actually was (documented in the file header)."
  (let* ((edit (gethash "textEdit" item))
         (range (and (hash-table-p edit) (gethash "range" edit)))
         (start (and range (gethash "start" range))))
    (if start
        (lsp--pos-at (gethash "line" start) (gethash "character" start))
      prefix-start)))

(defun lsp--completion-item-filter-text (item)
  "ITEM's `filterText', falling back to its `label' when absent -- what
`lsp--completion-items' prefix-matches against the popup-wide typed
span (PREFIX-START..POINT, M44 review fix #1: not each candidate's own
start, see its doc comment; v1 still no fuzzy matching, exact prefix
only, documented in the file header)."
  (or (gethash "filterText" item) (gethash "label" item)))

(defun lsp--completion-item-sort-key (item)
  "ITEM's sort key (M44-3): `sortText' when present, else `label' -- the
LSP spec's own fallback for ordering when a server omits `sortText'."
  (or (gethash "sortText" item) (lsp--completion-item-label item)))

(defun lsp--completion-result-vector (result)
  "The raw items VECTOR inside a textDocument/completion RESULT -- null,
a vector of CompletionItem directly, or a CompletionList
{isIncomplete, items} hash-table, both legal per the LSP spec -- paired
with its `isIncomplete' flag (M44-3). Returns (VECTOR . INCOMPLETE):
VECTOR is nil if RESULT has none; INCOMPLETE is a plain elisp boolean
(t/nil), normalized from JSON's true/`:false'/absent tri-state (see
`lsp-completion-at-point', which re-requests instead of narrowing the
existing popup further when this is set and the buffer changes)."
  (cond
   ((null result) (cons nil nil))
   ((vectorp result) (cons result nil))
   ((hash-table-p result)
    (let ((items (gethash "items" result))
          (inc (gethash "isIncomplete" result)))
      (cons (if (vectorp items) items nil) (and inc (not (eq inc :false))))))
   (t (cons nil nil))))

(defun lsp--completion-items (items prefix-start point)
  "Build the (LABEL INSERT START FILTER) list `show-completion-popup'
wants from ITEMS (a vector of CompletionItem hash-tables -- the `car'
of what `lsp--completion-result-vector' returns), PREFIX-START (the
identifier-run start `lsp-completion-at-point' recorded when it sent
the request), and POINT (current point as of when this answer is being
processed -- may be later than the point at request time, since typing
continues while a request is in flight, M44-3).

Non-hash-table elements (a malformed server reply -- JSON `null', a
bare string, ...) are dropped FIRST, before sorting (M44 review fix
#2): `sort' previously ran over the raw, unfiltered vector and its key
function (`lsp--completion-item-sort-key') calls `gethash' straight
into whatever it's handed, so ONE malformed element made the whole
reply signal and lose every well-formed item along with it -- pre-M44-3
had this same `hash-table-p' guard, but inside the loop below, which
sorting ahead of it silently bypassed.

Sorted by `sortText' (`lsp--completion-item-sort-key', falling back to
`label'), a stable sort so candidates that tie on that key keep the
server's own relative order (M44-3; `sortText' was previously not
consulted at all, see the file header).

Filtering (M44 review fix #1) matches every candidate against the SAME
popup-wide span -- the buffer text between PREFIX-START and POINT, i.e.
whatever the user actually TYPED -- not each candidate's own start, as
M44-3 originally had it. A `textEdit'-bearing candidate's own start can
sit well before PREFIX-START (member/postfix completion replacing
\"obj.\", say) while its `filterText' only covers the part after that
span (\"if\"/\"match\"/\"unwrap\" for a postfix template): matching
PREFIX-START..POINT's typed text against such a `filterText' works,
matching the candidate's OWN start..POINT span against it does not --
that mismatch was silently dropping whole categories of real-world
completions (postfix templates chief among them). Each candidate still
computes its OWN start (`lsp--completion-item-start') for the returned
START field -- accepting a candidate deletes THAT span, not
PREFIX-START..POINT, since a `textEdit' can legitimately replace more
than the ordinary prefix. A candidate whose own start is past POINT (a
malformed `textEdit') is dropped outright -- its START would make no
sense as a deletion range."
  (let (out)
    (dotimes (idx (if items (length items) 0))
      (when (hash-table-p (aref items idx))
        (push (aref items idx) out)))
    (setq out (nreverse out))
    (setq out (sort out (lambda (a b)
                           (string< (lsp--completion-item-sort-key a)
                                    (lsp--completion-item-sort-key b)))))
    (let ((typed (buffer-substring-no-properties prefix-start point))
          result)
      (dolist (item out)
        (let* ((start (lsp--completion-item-start item prefix-start))
               (filter (lsp--completion-item-filter-text item)))
          (when (and (<= start point)
                     (string-prefix-p typed filter))
            (push (list (lsp--completion-item-label item)
                        (lsp--completion-item-insert-text item)
                        start
                        filter)
                  result))))
      (nreverse result))))

(defun lsp-completion-at-point ()
  "Async `textDocument/completion' at point. Returns immediately; when
the server answers, opens `show-completion-popup' with candidates
filtered against the identifier prefix immediately before point
(`lsp--completion-prefix-start') -- accepting one there replaces
exactly that candidate's own span (`textEdit.range.start' when present,
the identifier prefix otherwise; see `lsp--completion-item-start').
Typing is never blocked waiting for the answer.

Staleness (M44-3): the buffer and the identifier-run start
(`lsp--completion-prefix-start') as of the request are captured and
re-checked when the answer arrives. A DIFFERENT buffer, or a recomputed
prefix-start that no longer matches -- point has moved outside the run
the request was about, e.g. a space was typed, ESC back to evil normal
state (also caught independently by `show-completion-popup' itself
checking `inhibit-self-insert', M40-4's second line of defense against
the same race), or point jumped elsewhere -- means the popup this would
open no longer corresponds to what's being typed, so the reply is
silently discarded, same dabbrev-session discipline as
`dabbrev--continue-p'. More typing WITHIN the same identifier run,
however, is no longer stale: it's exactly the filter-as-you-type case
this milestone adds, so candidates are filtered against whatever is
typed by the time the answer arrives, not what was typed when the
request was sent -- `buffer-modified-tick' equality is deliberately no
longer required, unlike M40-4's original three-way check."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      ;; M35: see the matching comment in `lsp-hover-at-point'.
      (lsp--sync-buffer-now)
      (let* ((file (buffer-file-name))
             (lc (lsp--line-character-at (point)))
             (buf (current-buffer))
             (prefix-start (lsp--completion-prefix-start (point))))
        (lsp-request-async
         client "textDocument/completion"
         (lsp--text-document-position-params file (car lc) (cdr lc))
         (lambda (result)
           (when (and (eq (current-buffer) buf)
                      (= (lsp--completion-prefix-start (point)) prefix-start))
             (let* ((parsed (lsp--completion-result-vector result))
                    (items (lsp--completion-items (car parsed) prefix-start (point)))
                    (incomplete (cdr parsed)))
               (if items
                   (show-completion-popup items prefix-start incomplete)
                 (message "No completions")))))))))))

(defvar local-completion-function nil
  "Buffer-local (nil by default): when non-nil, `completion-at-point'
calls this function of NO arguments FIRST, ahead of LSP and dabbrev. A
non-nil return means it fully handled this call (a popup opened, or a
deliberate silent no-op) and `completion-at-point' stops there; nil
means \"not applicable at point\", falling through to the next tier
exactly as if this variable were nil. M54: set by `verilog-mode' (see
`modes.el') to `verilog-complete-at-point' -- see verilog-complete.el's
own header for why Verilog needed a purpose-built, non-LSP completion
source ahead of the server rather than a smarter LSP fallback alone.

Deliberately named WITHOUT an `lsp--'/mode-specific prefix, unlike
almost every other variable in this file -- this one is a public
DISPATCH POINT any mode (or user) is meant to set, the buffer-local
equivalent of `major-mode-internal-set' installing a mode's keymap; an
`lsp--' prefix would misleadingly suggest it's LSP-internal machinery
never meant to be touched from outside this file, when `modes.el' (a
different file entirely) setting it for `verilog-mode' is the whole
point. Not an oversight -- recorded here so a future pass doesn't
\"fix\" it by adding one.

Known gap (M54 review, not a regression this milestone introduced --
a pre-existing, editor-wide limitation this is only the first feature
to actually depend on): this interpreter has no `kill-all-local-
variables' equivalent (`major-mode-internal-set' only overwrites the
buffer's own `major_mode' field, nothing else) -- so once a buffer has
ever run `verilog-mode-hook' and picked up a `verilog-complete-at-
point' local binding, switching that SAME buffer to a different major
mode afterward does not clear it back to nil. `C-M-i' in that buffer
keeps trying `verilog-complete-at-point' first even though the buffer
is no longer in `verilog-mode' (harmlessly: that function returns nil
immediately for any position that isn't a Verilog port connection, so
in practice it just falls through to the next tier a beat later --
but the buffer-local value itself is stale). Every other buffer-local
variable in this editor has the exact same residue-after-mode-switch
property; `local-completion-function' is merely the first place mode
DISPATCH itself was hung off of one, which is why it's worth spelling
out here specifically.")

(defun completion-at-point ()
  "Complete the symbol at point, three tiers in order:
1. `local-completion-function' (buffer-local, nil by default) -- a
   mode-specific completion source that may know something no LSP
   server can (M54: see `local-completion-function''s own docstring).
   A non-nil return ends the search here.
2. `lsp-completion-at-point', when this buffer has a live LSP client
   AND that client's own advertised capabilities support
   `textDocument/completion' (`lsp--capability-supported-p', M54) --
   see that function's docstring for the narrow, asymmetric rule this
   checks (an ABSENT `completionProvider' key blocks the request; a
   FALSY one, or capabilities never having been recorded at all, does
   not).
3. `dabbrev-expand' otherwise -- so `C-M-i' still does something useful
   before `M-x lsp' has been run, in a buffer with no server registered
   for its major mode, or against a server that doesn't support
   completion at all."
  (interactive)
  (cond
   ((and local-completion-function (funcall local-completion-function)))
   ((let ((client (lsp--live-buffer-client)))
      (and client (lsp--capability-supported-p client "completionProvider")))
    (lsp-completion-at-point))
   (t (dabbrev-expand))))

(global-set-key "C-M-i" 'completion-at-point)

;; --- Async go-to-definition + marker ring: M-. / M-, ---

(defvar lsp--marker-stack nil
  "Stack of (BUFFER . MARKER) `lsp-definition-at-point' pushes before
jumping, so `lsp-pop-definition-stack' can return.")

(defun lsp--uri-to-path (uri)
  (if (string-prefix-p "file://" uri) (substring uri 7) uri))

(defun lsp--make-definition-marker ()
  "A (BUFFER . MARKER) cons for the CURRENT buffer and point, in the
shape `lsp--marker-stack' stores -- see `lsp-push-definition-marker'.
Internal (`lsp--' prefixed): split out from that function because
`lsp-definition-at-point' must capture this BEFORE sending its async
request (point or the current buffer could change before the answer
arrives), while the PUSH itself must wait until a definition is
actually found (a \"no definition\" answer must never perturb the
ring) -- one combined function cannot serve both moments at once. A
caller with no such async gap (M55: `verilog-nav.el', whose whole
lookup is synchronous start to finish) just builds one of these
immediately before handing it straight to `lsp-push-definition-marker'."
  (cons (current-buffer) (point-marker)))

(defun lsp-push-definition-marker (origin)
  "Push ORIGIN -- a (BUFFER . MARKER) cons, see
`lsp--make-definition-marker' -- onto `lsp--marker-stack', so
`lsp-pop-definition-stack' (M-,) can return to it later.

M55: non-LSP jump sources call this too (`verilog-nav.el's
`verilog-goto-module-at-point', reached via `local-definition-function'
below) -- `M-,' has to return to wherever `M-.' actually jumped from
regardless of which tier answered the lookup, so there is exactly one
stack and one push entry point shared by all of them, not a separate
marker ring per source."
  (push origin lsp--marker-stack))

(defvar local-definition-function nil
  "Buffer-local (nil by default): when non-nil, `lsp-definition-at-point'
calls this function of NO arguments FIRST -- ahead of every LSP tier,
and in particular ahead of the `(not live)' check, not after it: the
entire point of a local-definition source is to answer `M-.' even when
no server is connected at all (`M-x lsp' never having been run in this
buffer). A non-nil return means it fully handled this call (jumped
somewhere -- pushing the origin onto `lsp--marker-stack' itself via
`lsp-push-definition-marker' first, so `M-,' can return -- or made a
deliberate silent no-op) and `lsp-definition-at-point' stops there;
nil means \"not applicable at this position\", falling through to the
LSP tiers exactly as if this variable were nil.

M55: set by `verilog-mode' (see `modes.el') to
`verilog-goto-module-at-point' -- jumping to a module instantiated at
point is answerable from `verilog-library-directories' alone, with no
server round trip needed at all. See that function's own header in
verilog-nav.el for the `verible-verilog-ls' repro this tier is built
against (a cross-file module lookup that comes back empty unless every
file involved has already been `didOpen'-ed, or the project root has
its own `verible.filelist').

Deliberately named WITHOUT an `lsp--' prefix, matching
`local-completion-function''s own naming just above in this file --
see that variable's docstring for the full reasoning (this is a public
DISPATCH POINT any mode is meant to set from OUTSIDE this file, not
LSP-internal machinery an `lsp--' prefix would misleadingly suggest is
off-limits). The same post-mode-switch buffer-local-residue gap
documented there (no `kill-all-local-variables' equivalent in this
interpreter) applies here identically -- not repeated a second time.")

(defun lsp-definition-at-point ()
  "Go to the definition of the symbol at point, checked in order:
1. `local-definition-function' (buffer-local, nil by default) -- a
   mode-specific, non-LSP lookup that may work with no server connected
   at all (M55: see that variable's own docstring). A non-nil return
   ends the search here.
2. Otherwise, async `textDocument/definition' against this buffer's
   live LSP client, if any. Returns immediately; when the server
   answers, jumps to the first location, pushing the origin onto
   `lsp--marker-stack' first (via `lsp-push-definition-marker') so
   `lsp-pop-definition-stack' (M-,) can return. Typing is never blocked
   waiting for the answer."
  (interactive)
  (let ((live (lsp--live-buffer-client)))
   (cond
    ((and local-definition-function (funcall local-definition-function)))
    ((not live)
     (message "No LSP server connected in this buffer (M-x lsp first)"))
    ((not (buffer-file-name))
     (message "Buffer is not visiting a file"))
    (t
     ;; M35: see the matching comment in `lsp-hover-at-point'.
     (lsp--sync-buffer-now)
     (let* ((client live)
            (file (buffer-file-name))
            (lc (lsp--line-character-at (point)))
            (origin (lsp--make-definition-marker)))
      (lsp-request-async
       client "textDocument/definition"
       (lsp--text-document-position-params file (car lc) (cdr lc))
       (lambda (result)
         (let ((loc (lsp--definition-location result)))
           (if (not loc)
               (message "No definition found")
             (lsp-push-definition-marker origin)
             (find-file (lsp--uri-to-path (car loc)))
             (goto-char (point-min))
             (forward-line (cdr loc)))))))))))

(defun lsp--buffer-live-p (buf)
  "Non-nil if BUF is still among `buffer-list' -- there is no
`marker-buffer' primitive in this editor, so `lsp--marker-stack'
remembers the buffer alongside each marker instead, and this is how
`lsp-pop-definition-stack' notices one was killed meanwhile."
  (and (memq buf (buffer-list)) t))

(defun lsp-pop-definition-stack ()
  "Return to the position `lsp-definition-at-point' (M-.) jumped from."
  (interactive)
  (if (not lsp--marker-stack)
      (message "No previous position to return to")
    (let* ((entry (pop lsp--marker-stack))
           (buf (car entry))
           (marker (cdr entry)))
      (if (not (lsp--buffer-live-p buf))
          (message "Buffer for previous position no longer exists")
        (switch-to-buffer buf)
        (goto-char (marker-position marker))))))

;; --- Diagnostic navigation: M-g n / M-g p ---

(defun lsp--buffer-diagnostic-positions ()
  "Ascending list of (POS . DIAG) for every diagnostic last published
for the current buffer, or nil if there's no LIVE connected client
(`lsp--live-buffer-client', not the raw `lsp--buffer-client' -- a dead
server shouldn't report stale diagnostics as current) or none have
arrived yet. Callers that need to tell those two nil cases apart
(`next-diagnostic', `previous-diagnostic') check
`lsp--live-buffer-client' themselves first rather than trying to infer
which case this was from a nil return alone."
  (let ((client (lsp--live-buffer-client))
        (file (buffer-file-name)))
    (when (and client file)
      (let* ((diags (lsp-diagnostics client file))
             (n (if diags (length diags) 0))
             (out nil))
        (dotimes (idx n)
          (let* ((d (aref diags idx))
                 (start (gethash "start" (gethash "range" d)))
                 (pos (lsp--pos-at (gethash "line" start) (gethash "character" start))))
            (push (cons pos d) out)))
        (sort (nreverse out) (lambda (a b) (< (car a) (car b))))))))

(defun lsp--goto-diagnostic (entry)
  (goto-char (car entry))
  (message "%s" (gethash "message" (cdr entry))))

(defun next-diagnostic ()
  "Jump to the nearest diagnostic after point in the current buffer,
wrapping to the first one if point is at or after the last. Messages
the same \"No LSP server connected in this buffer (M-x lsp first)\" as
the other 12 LSP commands if this buffer has no live client at all
(M63 -- distinguishing \"no connection\" from \"connected but clean\"
matters for RTL: seeing \"No diagnostics\" with no server attached
reads as \"your code has no problems\", which is simply false), or
\"No diagnostics\" if a client is live but none have been published."
  (interactive)
  (if (not (lsp--live-buffer-client))
      (message "No LSP server connected in this buffer (M-x lsp first)")
    (let ((positions (lsp--buffer-diagnostic-positions)))
      (if (not positions)
          (message "No diagnostics")
        (let ((here (point)) (next nil))
          (dolist (entry positions)
            (when (and (> (car entry) here) (not next))
              (setq next entry)))
          (lsp--goto-diagnostic (or next (car positions))))))))

(defun previous-diagnostic ()
  "Jump to the nearest diagnostic before point in the current buffer,
wrapping to the last one if point is at or before the first. Same
\"no client\" vs. \"no diagnostics\" distinction as `next-diagnostic' --
see its doc."
  (interactive)
  (if (not (lsp--live-buffer-client))
      (message "No LSP server connected in this buffer (M-x lsp first)")
    (lsp--previous-diagnostic-1)))

(defun lsp--previous-diagnostic-1 ()
  "Body of `previous-diagnostic' once a live client is confirmed --
split out only so the guard above reads as a single early return
rather than wrapping the whole existing wrap-around walk in another
level of `if'."
  (let ((positions (lsp--buffer-diagnostic-positions)))
    (if (not positions)
        (message "No diagnostics")
      (let ((here (point)) (prev nil) (last-entry nil))
        (dolist (entry positions)
          (setq last-entry entry)
          (when (< (car entry) here)
            (setq prev entry)))
        (lsp--goto-diagnostic (or prev last-entry))))))

;; --- M49: textDocument/documentHighlight -- C-c l h/H/N/P ---
;; M51 adds the idle auto-trigger (`lsp-idle-highlight-delay-ms') on top
;; of the same request path -- see `lsp--idle-highlight-tick' below.

(defvar lsp-idle-highlight-delay-ms 300
  "Milliseconds of quiescence since the last user input before
`lsp--idle-highlight-tick' automatically fires `lsp-highlight-at-point'.
`nil' disables the auto-trigger entirely (no separate enable flag --
this variable is both the switch and the threshold).

\"Quiescence since the last user input\" is not the same as \"since the
cursor last moved\": any input that resets the frontend's idle clock
(saving the buffer, a prefix key, switching windows, ...) restarts the
count even if point never moves, exactly like GNU Emacs's own
idle-timer facility, where any command -- not just point motion --
resets the idle timer. So this is a lower bound on latency after the
cursor visibly stops, only exact when no other input intervenes.

The *actual* delay observed is also this value plus up to one poll
cycle of slack (100ms with async LSP/eshell work pending, 500ms
otherwise -- see `has_async_work' in `lib.rs'), since `idle_tick' (and
therefore this check) only runs between input-poll timeouts in the
frontend event loop.

Values above `u32::MAX' milliseconds (~49.7 days) are silently clamped
to that ceiling on the Rust side (see `idle_tick' in `lib.rs'); a
threshold set at or above that ceiling means the `(>= quiet-ms delay)'
comparison in `lsp--idle-highlight-tick' is never satisfied, and the
auto-trigger never fires.")

(defvar lsp--idle-highlight-last-point nil
  "Buffer-local: `point' as of the last `textDocument/documentHighlight'
request sent from this buffer, whether sent by the interactive
`lsp-highlight-at-point' or automatically by
`lsp--idle-highlight-tick'. Used by the latter to avoid re-requesting
at a position it (or the user) already asked about. Does not account
for buffer edits at a fixed point (`buffer-modified-tick' is not
consulted) -- editing text at point without moving it will not trigger
a fresh request; this is a known, accepted gap (see `lsp--idle-
highlight-tick').")

(defvar lsp--highlight-request-serial 0
  "Buffer-local: incremented each time a `textDocument/documentHighlight'
request is sent from this buffer. Each request's callback closes over
the value current at send time and compares it against this variable
before applying its reply, so a reply to a stale (superseded) request
can never clobber a newer request's highlights with out-of-date
ranges -- see the M51 note in `lsp-highlight-at-point'.")

(defun lsp--clear-highlights ()
  "Delete every `'lsp-highlight'-tagged overlay in the current buffer,
leaving overlays with any other tag (`'lsp-diag', tree-sitter's
highlighting, ...) untouched."
  (dolist (ov (overlays-in (point-min) (point-max)))
    (when (overlay-get ov 'lsp-highlight)
      (delete-overlay ov))))

(defun lsp-highlight-at-point (&optional quiet)
  "Async `textDocument/documentHighlight' at point: draws every range in
the reply as an `'lsp-highlight'-tagged overlay in the `lsp-highlight'
face. Read-only request -- unlike the write commands
(`lsp-code-action-at-point', `lsp-rename', `lsp-format-buffer') this
has no `buffer-modified-tick' staleness check, only the ordinary
buffer-identity guard also used by `lsp-hover-at-point'/`lsp-
definition-at-point' (plus the M51 request-serial guard below, which
guards against a *different* hazard: replies to two of this command's
own requests arriving out of order).

The old highlight set is always cleared first, even when the reply is
empty/`nil'/`:null' (\"No highlights here\") -- otherwise moving point
onto a keyword and getting that message would leave a stale highlight
from wherever point was before sitting on screen, silently pointing at
the wrong thing.

Coordinates use the UTF-16 pair (`lsp--line-utf16-at'/`lsp--pos-at-
utf16', M46), not the older scalar-value pair the sibling hover/
definition commands still use -- see the file header's M49 note.

When QUIET is non-nil (only `lsp--idle-highlight-tick' passes this),
none of the three `message' calls below fire -- the two guard
rejections and the empty-reply \"No highlights here\" all go silent,
since an automatic background trigger nagging the echo area every time
the cursor rests on a keyword or an unconnected buffer would be worse
than not auto-triggering at all. Manual `M-x lsp-highlight-at-point'
still reports normally; this asymmetry is deliberate. Regardless of
QUIET, `lsp--idle-highlight-last-point' is updated on the request path
so a manual invocation also suppresses the next idle tick at the same
point (otherwise a 300ms-later auto-trigger would silently redo the
same request)."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (unless quiet
        (message "No LSP server connected in this buffer (M-x lsp first)")))
     ((not (buffer-file-name))
      (unless quiet
        (message "Buffer is not visiting a file")))
     (t
      (lsp--sync-buffer-now)
      (setq-local lsp--idle-highlight-last-point (point))
      (setq-local lsp--highlight-request-serial
                  (1+ lsp--highlight-request-serial))
      (let* ((serial lsp--highlight-request-serial)
             (lc (lsp--line-utf16-at (point)))
             (p (lsp--text-document-position-params
                 (buffer-file-name) (car lc) (cdr lc)))
             (buf (current-buffer)))
        (lsp-request-async
         client "textDocument/documentHighlight" p
         (lambda (result)
           (when (and (eq (current-buffer) buf)
                      (eq serial lsp--highlight-request-serial))
             (lsp--clear-highlights)
             (if (not (and (vectorp result) (> (length result) 0)))
                 (unless quiet (message "No highlights here"))
               (let ((n (length result)) (i 0))
                 (while (< i n)
                   (let* ((h (aref result i))
                          (range (gethash "range" h))
                          (start (gethash "start" range))
                          (end (gethash "end" range))
                          (from (lsp--pos-at-utf16 (gethash "line" start)
                                                    (gethash "character" start)))
                          (to (lsp--pos-at-utf16 (gethash "line" end)
                                                  (gethash "character" end)))
                          (ov (make-overlay from (if (> to from) to (1+ from)))))
                     (overlay-put ov 'lsp-highlight t)
                     (overlay-put ov 'face 'lsp-highlight))
                   (setq i (1+ i)))))))))))))

(defun lsp-highlight-clear ()
  "Remove every highlight `lsp-highlight-at-point' drew in the current
buffer. When a live client is connected, also records the current
point in `lsp--idle-highlight-last-point' (M51), so the idle
auto-trigger doesn't redraw the very highlights this command just
removed within its delay window -- clearing stays in effect until
point moves.

The recording is skipped when there is no live client (M51 second
round, F-review): the only reason to record `last-point' is to
suppress the *next* automatic trigger, and the automatic trigger
itself already requires a live client (`lsp--idle-highlight-tick'
bails out immediately without one), so recording it here has no
purpose -- it can only create a dead zone where connecting a client
later, at that same unmoved point, silently swallows the first
auto-trigger until the user moves point at least once.

One narrow case is deliberately left unaddressed: if the client dies
*after* a highlight was drawn and the user calls this command while
still disconnected, `last-point' is not recorded, so a later
reconnection at that same point *will* auto-trigger a fresh highlight
request. This is accepted, not a regression -- re-highlighting at
point after a server restart is reasonable behaviour, and there was
no live client to suppress an auto-trigger against in the first
place."
  (interactive)
  (lsp--clear-highlights)
  (when (lsp--live-buffer-client)
    (setq-local lsp--idle-highlight-last-point (point))))

(defun lsp--idle-highlight-tick (quiet-ms)
  "Called from `idle_tick' (Rust) on every frontend event-loop poll with
QUIET-MS milliseconds since the last user input. Fires a quiet
`lsp-highlight-at-point' at most once per cursor position, once the
cursor has been still for at least `lsp-idle-highlight-delay-ms'.

The request path is wrapped in a `condition-case' that catches and
silences any `error' condition (not `throw' or other non-error control
transfers), for the same reason `lsp-process-pending-all' does (see its
comment): `lsp--sync-buffer-now' can signal in the narrow race window
right after the server process has died but before this buffer's client
has been noticed as dead. Firing on every idle tick raises the odds of
landing in that window far above the ordinary case of an occasional
manual keystroke, so unlike `lsp-highlight-at-point' itself this path
must never let an `error' propagate to the editor.

The enable/threshold check is deliberately kept *outside* that
`condition-case'. Two reasons. (1) It is ordinary control flow, not the
server-death race the handler exists for, and a handler that also
covers the guard it is guarded by can no longer be reasoned about
locally. (2) Testability: `(>= QUIET-MS nil)' signals `wrong-type-
argument', so with the check inside the handler, deleting the `lsp-idle-
highlight-delay-ms' conjunct produced *exactly* the same observable
behaviour as keeping it (no request sent, error swallowed) -- the
disable switch had no reachable mutation target. Outside the handler
that same deletion propagates the type error to the caller, where a
test can see it. A malformed threshold value still cannot reach the
user: `idle_tick' discards this function's result with `let _ ='.

Does not consult `buffer-modified-tick': editing text at the same
point (without moving it) does not re-trigger a request, since the
position-based `lsp--idle-highlight-last-point' check only compares
`point'. Known, accepted gap -- see that variable's docstring."
  (when (and lsp-idle-highlight-delay-ms
             (>= quiet-ms lsp-idle-highlight-delay-ms))
    (condition-case nil
        (when (and (lsp--live-buffer-client)
                   (buffer-file-name)
                   (not (eq (point) lsp--idle-highlight-last-point)))
          (lsp-highlight-at-point t))
      (error nil))))

(defun lsp--highlight-overlays ()
  "Every `'lsp-highlight'-tagged overlay in the current buffer, in no
particular order -- callers compare `overlay-start' themselves rather
than relying on this list's order. Recomputed fresh on every call
instead of cached, so it can never drift from the live overlays as the
buffer is edited (see the file header's M49 note)."
  (let (out)
    (dolist (ov (overlays-in (point-min) (point-max)))
      (when (overlay-get ov 'lsp-highlight)
        (push ov out)))
    out))

(defun lsp-next-highlight ()
  "Jump to the nearest `lsp-highlight-at-point' highlight after point,
wrapping to the first one (and announcing the wrap) if point is at or
after the last. Messages \"No highlights\" if none are drawn."
  (interactive)
  (let ((ovs (lsp--highlight-overlays)))
    (if (not ovs)
        (message "No highlights")
      (let ((here (point)) (next nil) (first-ov nil))
        (dolist (ov ovs)
          (when (or (not first-ov) (< (overlay-start ov) (overlay-start first-ov)))
            (setq first-ov ov))
          (when (and (> (overlay-start ov) here)
                     (or (not next) (< (overlay-start ov) (overlay-start next))))
            (setq next ov)))
        (if next
            (goto-char (overlay-start next))
          (goto-char (overlay-start first-ov))
          (message "Wrapped to first highlight"))))))

(defun lsp-previous-highlight ()
  "Jump to the nearest `lsp-highlight-at-point' highlight before point,
wrapping to the last one (and announcing the wrap) if point is at or
before the first. Messages \"No highlights\" if none are drawn."
  (interactive)
  (let ((ovs (lsp--highlight-overlays)))
    (if (not ovs)
        (message "No highlights")
      (let ((here (point)) (prev nil) (last-ov nil))
        (dolist (ov ovs)
          (when (or (not last-ov) (> (overlay-start ov) (overlay-start last-ov)))
            (setq last-ov ov))
          (when (and (< (overlay-start ov) here)
                     (or (not prev) (> (overlay-start ov) (overlay-start prev))))
            (setq prev ov)))
        (if prev
            (goto-char (overlay-start prev))
          (goto-char (overlay-start last-ov))
          (message "Wrapped to last highlight"))))))

;; --- M59: textDocument/references -- M-? ---
;;
;; See the file header's M59 note for the real-server findings this is
;; built against; the short version: verible-verilog-ls answers
;; accurately and at symbol-table granularity (not text matching), but
;; ONLY when the project root has its own `verible.filelist' -- without
;; one it answers `[]' unconditionally, even for a reference in the
;; SAME file/buffer being queried. Known v1 gaps, not fixed here:
;;  - A `verible.filelist' that lists only SOME of the project's files
;;    makes the server silently answer an INCOMPLETE list -- nothing on
;;    this side can detect that, so it's not handled.
;;  - This command never consults `local-definition-function' or any
;;    other local/non-LSP tier (unlike `lsp-definition-at-point', M55) --
;;    it is pure LSP, always.
;;  - verible-verilog-ls never returns the declaration site itself, even
;;    with `includeDeclaration' sent as `t' (confirmed against the real
;;    binary); a server that does honor it (slang-server) will include
;;    it. Which is true for a given reply is entirely server-dependent
;;    and not something this command tries to infer or normalize.
;;  - Preview text comes from DISK; the jump lands in a LIVE buffer.
;;    `lsp--reference-line-text' reads each candidate's source line via
;;    `file-contents-as-string', straight off disk -- but
;;    `lsp--goto-reference''s own `find-file' reuses an EXISTING buffer
;;    for that path if the user already has one open with unsaved edits
;;    (`crates/core/src/builtins/editing.rs''s `find-file'), without
;;    re-reading it. So the candidate string's TEXT can describe a line
;;    that no longer matches what `M-?' actually lands on. Worse, the
;;    server's own LINE/CHARACTER in the reply may not correspond to
;;    EITHER disk or the live buffer in the first place: only the
;;    QUERYING buffer is guaranteed `lsp--sync-buffer-now'-ed
;;    (immediately before the request goes out) -- nothing sends a
;;    didChange for any OTHER file a reply happens to mention. Three
;;    different possible contents (disk, live buffer, server's own
;;    model) with no ordering guarantee between them means there is no
;;    single "correct" fix here -- documented as a known gap, not
;;    attempted.
;;  - URIs are never percent-decoded. `lsp--uri-to-path' just strips the
;;    `file://' prefix; a path containing a space or non-ASCII character
;;    comes back with the literal `%20'/etc still in it -- a pre-
;;    existing gap, not introduced by M59. M59 widens its blast radius
;;    considerably, though: it's the first command that can turn ONE
;;    query into hundreds of distinct file paths in a single reply,
;;    where every prior URI-consuming call site (hover/definition/
;;    codeAction/rename/...) only ever handled one or a handful.

(defun lsp--references-context ()
  "The `ReferenceContext' params object `lsp-references-at-point' sends
as `context': always `{\"includeDeclaration\": t}' -- there is no
user-facing toggle in v1."
  (let ((h (make-hash-table)))
    (puthash "includeDeclaration" t h)
    h))

(defun lsp--verilog-buffer-p (file)
  "Non-nil if FILE's extension marks it Verilog/SystemVerilog family --
`.v'/`.vh'/`.sv'/`.svh', the same four extensions `modes.el''s
`auto-mode-alist' entries, `verilog-auto--library-file-name-p', and
`verilog-nav.el''s own header all agree on (this function does not
invent a fifth definition -- reviewer round: the first version of this
function was missing `.vh'). Used only to decide whether
`lsp-references-at-point''s empty-result message should add the
`verible.filelist' explanation (see `lsp--references-empty-message') --
not a general-purpose mode predicate, and deliberately independent of
whatever major mode is actually active in FILE's buffer (there may not
even BE a live buffer for FILE at all)."
  (and file
       (or (string-suffix-p ".v" file) (string-suffix-p ".vh" file)
           (string-suffix-p ".sv" file) (string-suffix-p ".svh" file))))

(defun lsp--verible-command-p (command)
  "Non-nil if COMMAND (an `lsp--client-command' string, or nil) names a
verible binary -- decided by a substring match on COMMAND's own
basename (`file-name-nondirectory'), so a full path like
\"/usr/local/bin/verible-verilog-ls\" still matches, not just a bare
command name. nil COMMAND (a client that predates the `command' field,
or a test's `make-lsp--client' stub -- see that struct field's own
docstring) means \"don't know\", treated as NOT verible: silence over a
wrong guess."
  (and command (string-match-p "verible" (file-name-nondirectory command)) t))

(defun lsp--references-empty-message (file command)
  "The `message' `lsp-references-at-point' shows for an empty/absent
`textDocument/references' reply against FILE (the querying buffer's own
`buffer-file-name') from a client started with COMMAND (its
`lsp--client-command'). Always starts with \"No references found\";
when FILE is Verilog-family (`lsp--verilog-buffer-p'), AND COMMAND
names a verible binary (`lsp--verible-command-p'), AND the project root
(`lsp--project-root') has no `verible.filelist' file directly in it, a
second clause names both facts -- verible-verilog-ls answers `[]'
unconditionally without one, a real-server finding surprising enough
(even a same-file, same-buffer reference comes back empty) that v1
treats it as worth surfacing rather than leaving the plain \"No
references found\" to imply the query genuinely has no references.

Reviewer round fix: the first version of this function gated only on
FILE being Verilog-family, so a `.sv' buffer connected to slang-server
(also first-class per `lsp-server-alist''s own docstring) got told to
add a `verible.filelist' -- naming the wrong tool AND the wrong fix,
since slang-server's own empty-cross-file-reference failure mode is a
missing `.slang' project-root marker, unrelated to `verible.filelist'
entirely. A COMMAND that doesn't name verible (or is nil, unknown)
never adds the clause."
  (let ((base "No references found"))
    (if (and (lsp--verilog-buffer-p file) (lsp--verible-command-p command))
        (let ((root (lsp--project-root file)))
          (if (file-exists-p (expand-file-name "verible.filelist" root))
              base
            (format "%s (no verible.filelist in %s; verible only answers for files listed there)"
                    base root)))
      base)))

(defun lsp--relative-path (file root)
  "FILE relative to ROOT (`lsp--project-root''s own no-trailing-slash
convention) when FILE is actually under ROOT, else FILE itself
unchanged -- e.g. a reference the server returned in a file outside
the project root. Display-only (`lsp--reference-alist''s DISPLAY
strings); the PATH stored for the actual jump is always FILE, in full."
  (let ((prefix (concat root "/")))
    (if (string-prefix-p prefix file)
        (substring file (length prefix))
      file)))

(defun lsp--reference-line-text (path line cache)
  "Trimmed text of 0-based LINE in PATH, or nil if PATH can't be read at
all or LINE is out of range for it. CACHE is a hash-table (equal-keyed
on PATH, caller-owned and fresh per `lsp-references-at-point' call --
never a shared/global cache) mapping PATH to a vector of its lines (or
the symbol `lsp--unreadable' for a PATH that failed once), so a file
hit several times by the same reply -- a local signal is the common
case -- is only ever read from disk once, via
`file-contents-as-string' (NOT `find-file': that would leave a
permanent buffer behind per file and do an O(n) buffer-list scan on
top, for every one of what can be hundreds of distinct files in one
reply -- see the file header's M59 note)."
  (let ((cached (gethash path cache 'lsp--reference-miss)))
    (when (eq cached 'lsp--reference-miss)
      (setq cached
            (condition-case nil
                (apply #'vector (split-string (file-contents-as-string path) "\n"))
              (error 'lsp--unreadable)))
      (puthash path cached cache))
    (if (or (eq cached 'lsp--unreadable) (>= line (length cached)))
        nil
      (string-trim (aref cached line)))))

(defun lsp--reference-entry (loc root cache)
  "One `lsp--reference-alist' entry -- a (DISPLAY . (PATH LINE
CHARACTER)) cons, same shape as that function's own return value -- for
LOC (one element of a `textDocument/references' reply vector), or nil
if LOC isn't a well-formed `Location': not a hash-table at all, or
missing/mistyped `uri'/`range'/`range.start'/`range.start.line'/
`range.start.character'.

`gethash' on a value that isn't a hash-table SIGNALS
`wrong-type-argument' (`crates/elisp/src/builtins/data.rs') rather than
returning nil -- so unlike a plain `(gethash \"start\" range)' chain,
every step down into LOC here is guarded by its own `hash-table-p' (or
type check for the leaf `line'/`character'/`uri' values) before the
next `gethash', and a failure at any step returns nil rather than
signaling. M59 is the first command in this file that can hand a
single reply hundreds of `Location' elements across as many files, so
one malformed element must not abort the rest -- same discipline as
`lsp--code-action-usable''s own per-element `hash-table-p' check, one
section up in this file, applied one level deeper since a `Location'
nests three hash-tables where a `CodeAction' element only needed one.

Known gap (tail-review round, documented rather than fixed): `line'/
`character' are checked with `integerp', which is nil for a Float --
and `json-parse-string' hands back a Float for a JSON number written
with a decimal point (`{\"line\": 5.0}' parses to `5.0', not `5'), which
is syntactically legal JSON even though the LSP spec types `line'/
`character' as `uinteger'. A server that serializes them that way
(non-conformant, but not observed against verible-verilog-ls or
slang-server) would make this function treat an otherwise well-formed
`Location' as malformed and silently drop it -- one missing candidate,
not a crash, so the existing degrade-gracefully behavior stays safe;
just not exercised for this particular input shape."
  (when (hash-table-p loc)
    (let ((uri (gethash "uri" loc))
          (range (gethash "range" loc)))
      (when (and (stringp uri) (hash-table-p range))
        (let ((start (gethash "start" range)))
          (when (hash-table-p start)
            (let ((line (gethash "line" start))
                  (character (gethash "character" start)))
              (when (and (integerp line) (integerp character))
                (let* ((path (lsp--uri-to-path uri))
                       (rel (lsp--relative-path path root))
                       (text (lsp--reference-line-text path line cache))
                       (display (if text
                                    (format "%s:%d:%d  %s" rel (1+ line) (1+ character) text)
                                  (format "%s:%d:%d" rel (1+ line) (1+ character)))))
                  (cons display (list path line character)))))))))))

(defun lsp--reference-alist (result root)
  "Build the (DISPLAY . (PATH LINE CHARACTER)) alist
`lsp-references-at-point' offers via `completing-read' from RESULT (a
non-empty `textDocument/references' reply vector) and ROOT (the
querying buffer's `lsp--project-root', used only to shorten DISPLAY's
own path via `lsp--relative-path'). LINE/CHARACTER in each cons's cdr
stay 0-based, LSP's own convention -- the eventual jump goes straight
through `lsp--goto-reference'/`lsp--pos-at-utf16', neither of which
wants them converted first.

Each RESULT element is turned into an entry via `lsp--reference-entry',
which returns nil (dropped, not aborting the rest) for anything that
isn't a well-formed `Location' -- see that function's own docstring.

Sorted by (PATH, LINE, CHARACTER) for a stable, readable order: the
reply's own element order is not guaranteed sorted by the spec, and
was observed NOT sorted against verible-verilog-ls on a several-
hundred-file sweep.

Each DISPLAY is \"REL:LINE:COL  TEXT\" (LINE/COL 1-based for display,
TEXT that line's own trimmed source via `lsp--reference-line-text',
which memoizes per-PATH reads in a fresh local hash-table so a file
with several hits is only read once) -- or just \"REL:LINE:COL\", with
neither the two trailing spaces nor TEXT, when
`lsp--reference-line-text' returns nil (unreadable file, or a line
number past its end). Either way the command MUST keep working; a
missing source snippet is never a reason to abort. No ` (N)'
disambiguation suffix (unlike `lsp--symbol-alist'/`lsp--code-action-
alist'): PATH:LINE:COL is unique per reference by construction, so no
two DISPLAY strings can ever collide."
  (let ((cache (make-hash-table :test 'equal))
        (entries nil)
        (n (length result))
        (i 0))
    (while (< i n)
      (let ((entry (lsp--reference-entry (aref result i) root cache)))
        (when entry (push entry entries)))
      (setq i (1+ i)))
    (setq entries (nreverse entries))
    (sort entries
          (lambda (a b)
            (let* ((ea (cdr a)) (eb (cdr b))
                   (pa (nth 0 ea)) (pb (nth 0 eb))
                   (la (nth 1 ea)) (lb (nth 1 eb))
                   (ca (nth 2 ea)) (cb (nth 2 eb)))
              (cond ((not (string= pa pb)) (string< pa pb))
                    ((/= la lb) (< la lb))
                    (t (< ca cb))))))))

(defun lsp--goto-reference (path line character)
  "Jump to 0-based LINE/CHARACTER (LSP's own UTF-16 CHARACTER encoding)
in PATH, opening it via `find-file' first -- unlike
`lsp--reference-line-text', this IS a case for a real, permanent
buffer: the user asked to go here, exactly like `lsp-definition-at-
point''s own `find-file' call. Uses `lsp--pos-at-utf16' (M46's UTF-16-
correct conversion), NOT `lsp-definition-at-point''s `(goto-char
(point-min)) (forward-line LINE)' -- that pair only reaches the START
of LINE and drops CHARACTER entirely, fine for \"go to definition\"
where the line itself is usually enough, but references must land on
the exact reported column."
  (find-file path)
  (goto-char (lsp--pos-at-utf16 line character)))

(defun lsp-references-at-point ()
  "Send `textDocument/references' at point (async, `lsp-request-async',
same reasoning as every other M46-forward interactive LSP command --
see the file header's M46 note; M65 bounded the synchronous
`lsp--await' path, but a multi-second freeze is still undesirable for
something triggered from the keyboard) and either jump straight to the
one result, or open a `with-completing-
read' picker (M47's macro, same disambiguation-free-because-already-
unique shape as `lsp--reference-alist' -- see its own docstring) over
more than one. An empty/absent reply messages via
`lsp--references-empty-message' -- see that function's docstring for
the extra `verible.filelist' clause it can add.

Read-only, like `lsp-highlight-at-point'/`lsp-definition-at-point' --
no `buffer-modified-tick' write-staleness check (nothing here writes
to any buffer). Two ordinary staleness guards instead, both `(eq
(current-buffer) buf)' against the buffer captured at request time:
the first right where the reply lands (guards opening the picker, or
jumping directly on a single result), the second inside the picker's
own callback (guards the eventual jump; a picker sitting open is an
arbitrary amount of time during which the user could have switched
buffers -- identical reasoning to `lsp-goto-symbol-by-name''s own
second check, see that function's docstring).

Before jumping, pushes the ORIGIN position -- captured immediately
after `lsp--sync-buffer-now', before the request goes out, via
`lsp--make-definition-marker' -- onto `lsp--marker-stack' via
`lsp-push-definition-marker', so `M-,' (`lsp-pop-definition-stack')
returns here. Pushed only once a destination is actually known (either
the sole result, or the user's picker choice), never on a \"no
references\" or a picker the user never finishes -- exactly
`lsp-definition-at-point''s own discipline, so a \"no definition
found\"/\"no references found\" answer never perturbs the ring.

Bound to `M-?' (see simple.el), alongside `M-.'/`M-,'."
  (interactive)
  (let ((client (lsp--live-buffer-client)))
    (cond
     ((not client)
      (message "No LSP server connected in this buffer (M-x lsp first)"))
     ((not (buffer-file-name))
      (message "Buffer is not visiting a file"))
     (t
      (lsp--sync-buffer-now)
      (let* ((file (buffer-file-name))
             (lc (lsp--line-utf16-at (point)))
             (p (lsp--text-document-position-params file (car lc) (cdr lc)))
             (buf (current-buffer))
             (origin (lsp--make-definition-marker)))
        (puthash "context" (lsp--references-context) p)
        (lsp-request-async
         client "textDocument/references" p
         (lambda (result)
           (when (eq (current-buffer) buf)
             (if (not (and (vectorp result) (> (length result) 0)))
                 (message "%s" (lsp--references-empty-message file (lsp--client-command client)))
               (let* ((root (lsp--project-root file))
                      (alist (lsp--reference-alist result root)))
                 (cond
                  ;; RESULT was non-empty, but every element failed
                  ;; `lsp--reference-entry''s well-formedness check (see
                  ;; the file header's M59 tail-review note): ALIST is
                  ;; empty here even though RESULT was not. Falling
                  ;; through to the picker below with an empty
                  ;; collection and REQUIRE-MATCH t would open a
                  ;; `completing-read' no keystroke can ever submit
                  ;; (`crates/core/src/commands.rs''s
                  ;; `require_match_blocks' is deliberate about that
                  ;; combination being permanently unsubmittable) --
                  ;; the only escape would be `C-g'. This message is
                  ;; deliberately its OWN wording, not a call to
                  ;; `lsp--references-empty-message': that one's
                  ;; `verible.filelist' clause explains a server that
                  ;; answered with NOTHING, which is not what happened
                  ;; here -- the server answered with (length RESULT)
                  ;; elements, they just didn't parse as `Location's.
                  ((not alist)
                   (message "No usable references here (%d malformed entries in the reply)"
                            (length result)))
                  ((= (length alist) 1)
                   (let ((entry (cdr (car alist))))
                     (lsp-push-definition-marker origin)
                     (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry))))
                  (t
                   (with-completing-read
                    (name "References: " (mapcar #'car alist) t)
                    ;; Second staleness check: see this function's own
                    ;; docstring.
                    (when (eq (current-buffer) buf)
                      (let ((entry (cdr (assoc name alist))))
                        (lsp-push-definition-marker origin)
                        (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry)))))))))))))))))
