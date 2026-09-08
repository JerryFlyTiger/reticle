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
;;  - Server-to-client requests (M123, closing both gaps this note used
;;    to describe as open): `lsp--dispatch` now distinguishes a message
;;    carrying BOTH `id` and `method` (a server-initiated REQUEST) from
;;    one carrying `id` alone (a RESPONSE to a request WE sent) before
;;    it ever looks at `lsp--client-pending`, so the two can no longer
;;    collide on the same code path -- this also closes the id-collision
;;    hazard the previous version of this note described as
;;    hypothetical: there is no longer any way for a server REQUEST to
;;    be handed to `lsp--await` as if it were our own RESPONSE, because
;;    a request never reaches `lsp--client-pending` at all. A request is
;;    answered immediately, from `lsp--dispatch` itself
;;    (`lsp--respond-to-request`): `client/registerCapability` and
;;    `window/workDoneProgress/create` get `{"result": null}` (accepting
;;    a no-op, which is all this client's own capabilities ever promise
;;    to do with either); anything else gets a JSON-RPC
;;    `MethodNotFound` (-32601) error response, per spec -- a server
;;    request is now NEVER left unanswered.
;;
;;    Measured 2026-09-08 (`dev/lsp-probe.py --sweep`): `slang-server`
;;    sends `client/registerCapability` with id 0 during an ordinary
;;    session; `verible-verilog-ls` sends no server-initiated request at
;;    all. So the request branch above is exercised by one of this
;;    project's two configured servers today, not a hypothetical.
;;
;;    What remains, deliberately: a plain NOTIFICATION (`method`, no
;;    `id`, and not `publishDiagnostics`) is still silently dropped --
;;    unchanged by M123, and not a bug: a notification has no id to
;;    answer and nothing here consumes it, matching the M46-era default
;;    for every notification type this client doesn't specifically
;;    handle.
;;
;;    A RESPONSE with no registered callback (typically `lsp--await`
;;    having already timed out on it, or a caller that fired the
;;    request via a path with no waiter at all) still lands in
;;    `lsp--client-pending`, exactly as before -- but that list is now
;;    bounded at `lsp--client-pending-limit` entries (M123), dropping
;;    the OLDEST entry once the cap is hit, rather than growing without
;;    limit for the connection's lifetime.
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
;;  - M99: `lsp-start' gained a third, optional CWD argument, passed by
;;    `lsp-connect'/`lsp--autostart-begin' as the project root (when it
;;    exists on disk) -- so a server's own per-project config file with
;;    relative paths (e.g. `slang-server''s `.slang/server.json', whose
;;    `flags' can read `"-I include"') resolves correctly regardless of
;;    where the editor process itself was launched from, rather than
;;    only by accident. Known gaps, documented rather than silently
;;    fixed:
;;      - `lsp-merge-diagnostics-from-all-clients' (default t) now
;;        paints the UNION of every attached client's diagnostics for a
;;        buffer, since this project's two Verilog servers are
;;        complementary rather than redundant (see that variable's own
;;        docstring for the measured numbers). There is deliberately NO
;;        cross-server dedup: if two servers each flag the same real
;;        syntax error in their own wording, both diagnostics are shown
;;        -- see `lsp--diagnostics-for-uri''s docstring for why nothing
;;        here can tell that case apart from two genuinely different
;;        defects that happen to read similarly.
;;      - M87 stage 3's inline diagnostic rows involve TWO separate
;;        budgets in `crates/core/src/redisplay.rs', and merging two
;;        servers' diagnostics interacts with only one of them: each
;;        individual diagnostic message is capped at 3 rendered lines
;;        (`diag_message_lines', a `take(3)` on the message split on
;;        `\n`, unrelated to how many clients published it) -- that cap
;;        is untouched by M99. What DOES get more likely to bind is the
;;        other budget, `text_rows` (the window's own text-row count,
;;        computed from window height, not a fixed constant): the inline
;;        rows for every diagnostic on every line share that one
;;        per-WINDOW budget, so a buffer with two servers' diagnostics
;;        merged onto it produces more total rows competing for the same
;;        budget than a single server would have, and rows past the
;;        budget are silently dropped (`emit_block_rows`'s `break
;;        'outer`) exactly as they already were pre-M99 for a single
;;        chatty server. This milestone does not raise or otherwise
;;        change that budget.
;;      - The `.slang/server.json' config file's `flags' relative paths
;;        are resolved by slang-server itself against ITS OWN cwd, which
;;        M99 pins to the project root -- a user who writes an absolute
;;        path there instead is unaffected either way.

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
  "Store DIAGS (CLIENT's own publish for URI) unconditionally, then let
`lsp--decorate-buffer' decide whether CLIENT is also allowed to repaint
the buffer's squiggles/gutter/inline rows (M94: a buffer with two
attached clients would otherwise flicker between them after every
edit -- see `lsp--diagnostics-authoritative-command's own doc).
Storage is unconditional and per-CLIENT (`lsp--client-diagnostics')
regardless of which client gets to decorate, so a non-decorating
client's diagnostics are never lost, only not painted."
  (setf (lsp--client-diagnostics client)
        (cons (cons uri diags)
              (delq (assoc uri (lsp--client-diagnostics client))
                    (lsp--client-diagnostics client))))
  (lsp--decorate-buffer client uri diags))

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

;; --- M94: diagnostics decoration authority -------------------------------

(defvar lsp--diagnostics-authoritative-command nil
  "Buffer-local (M94): the server COMMAND string (matching
`lsp--client-command') whose `textDocument/publishDiagnostics' is
allowed to DECORATE this buffer -- overlays, gutter dots, the mode-
line count, M87 stage 3's inline diagnostic rows. nil (the default)
means \"whichever client is `lsp--buffer-client', the primary\", so a
buffer with only one attached client behaves exactly as before this
variable existed -- decoration was never gated by anything before
M94, and a single client is trivially always its own primary.

A second attached client's diagnostics are still STORED on its own
`lsp--client-diagnostics' either way (`lsp--merge-diagnostics' never
consults this variable, only `lsp--decorate-buffer' does) -- only
DECORATION is gated, so `next-diagnostic'/`previous-diagnostic'/
`lsp--diagnostics-at-point' could still reach a non-decorating
client's diagnostics explicitly in a future extension. As of M94 v1
they do NOT: all three read only `lsp--buffer-client''s (the
primary's) diagnostics, so a secondary's publishes are stored but not
navigable -- an accepted scope limit, not an oversight. Not consulted
by anything except `lsp--diagnostics-authoritative-client'.

M94 review AA5: this variable is a STUB, not live configuration -- it
is `defvar'd here and read by `lsp--diagnostics-authoritative-client',
but as of M94 v1 NOTHING ever `setq'/`setq-local's it anywhere in this
file or its tests. It exists so the override path is already wired for
a future milestone that wants to let a buffer (or a user) name which
attached server's diagnostics get decorated instead of always
defaulting to the primary; until that milestone, this variable is
always nil in practice and every buffer's decoration authority is
decided purely by `lsp--buffer-client' liveness, as described above.")

(defun lsp--diagnostics-authoritative-client ()
  "The current buffer's diagnostics-authoritative client (M94), if
LIVE, else nil. Either the attached client (`lsp--effective-buffer-
clients') whose own `command' `equal's `lsp--diagnostics-authoritative-
command' (when that variable is set), or `lsp--buffer-client' (the
primary) otherwise. Returns nil -- \"authority unestablished\", NOT
\"nothing may decorate\" -- whenever no such client is currently live:
no primary has ever attached, or the one that did has since died. See
`lsp--diagnostics-authoritative-p', the only caller, for why that
distinction is the whole fix for M94 review Z1/Z4: it must never be
read as an all-or-nothing gate."
  (if lsp--diagnostics-authoritative-command
      (let (found)
        (dolist (client (lsp--effective-buffer-clients) found)
          (when (and (not found)
                     (lsp--client-p client)
                     (equal (lsp--client-command client)
                            lsp--diagnostics-authoritative-command)
                     (lsp--client-conn-live-p client))
            (setq found client))))
    (and lsp--buffer-client (lsp--client-conn-live-p lsp--buffer-client)
         lsp--buffer-client)))

(defun lsp--diagnostics-authoritative-p (client)
  "Non-nil if CLIENT may decorate the current buffer (M94). DEFAULT
OPEN: true unless a DIFFERENT, LIVE authoritative client already
exists for this buffer (`lsp--diagnostics-authoritative-client') --
this only ever REJECTS a client that is meant to be non-authoritative
while another live one already holds that role; it never requires
proof of primacy before a buffer's first (and possibly only)
publisher is allowed to decorate it.

M94 review Z1: the original version compared CLIENT against
`lsp--buffer-client' directly and returned nil whenever that was nil
(no primary attached at all) -- exactly backwards for the ordinary
case of a single client with nothing yet calling it \"primary\"
(`gui_features_tests.rs's `lsp_diagnostics_decorate_the_visiting_
buffer' dispatches from a synthetic client with no `M-x lsp' ever run
in the buffer, and that publish must still decorate). Z4: the same
raw `eq' also had no liveness check, so a primary that died left every
later publish from a live secondary permanently rejected -- fixed for
free by routing through `lsp--diagnostics-authoritative-client', whose
own liveness check is what makes authority nil (hence \"decorate\")
once the one-time authoritative client is gone.

M94 review AA5 (boundary, not a bug -- v1 as designed): this gate is
flicker-proof ONLY because a buffer today has AT MOST one primary and
one secondary attached (`lsp-server-alist' plus `lsp-secondary-server-
alist' each contribute at most one entry per mode). If a future
milestone ever attaches a THIRD client, every one of them would
default-open decorate whenever the (single) authoritative slot is
empty, which is exactly the flicker between publishers this gate
exists to prevent -- default-open only avoids flicker for the
\"zero or one authoritative client\" case, not \"more than two
clients total\". Likewise, a MODE that registers only a secondary
(no `lsp-server-alist' entry at all, so the primary slot stays
PERMANENTLY nil -- see `lsp--client-role-is-primary-p') never
establishes authority at all, so its secondary decorates
unconditionally forever; harmless with exactly one such client, but
the same flicker risk the moment a second one is added for that
mode. Neither case is reachable with this milestone's own default
configuration (Verilog: one primary, one secondary), so this is
recorded as a known boundary for a future extension to respect, not
fixed here."
  (let ((authoritative (lsp--diagnostics-authoritative-client)))
    (or (not authoritative) (eq client authoritative))))

(defvar lsp-merge-diagnostics-from-all-clients t
  "M99: when non-nil (the default), `lsp--decorate-buffer' paints the
UNION of every attached client's stored diagnostics for a buffer's URI,
not just the diagnostics-authoritative one -- see
`lsp--diagnostics-for-uri'. This exists because, measured against real
Verilog LSP servers, the two this project talks to are COMPLEMENTARY,
not redundant: on a tab-indented file, `verible-verilog-ls' reports 3
`no-tabs' style-lint diagnostics and `slang-server' reports 0; on
`demo/rtl/top/soc_top.sv', verible reports 0 and slang reports 17
(undriven-output-port, never-assigned, and other elaboration-level
diagnostics verible doesn't perform at all). Routing every buffer's
decoration through only the M94 authoritative client would silently
throw away one server's entire category of diagnostics.

Setting this to nil restores the exact M94 behavior: only the
diagnostics-authoritative client's own publish decorates the buffer,
and a secondary client's diagnostics are stored (`lsp--merge-
diagnostics' always stores, regardless of this variable) but never
painted. Kept as an escape hatch rather than deleting the M94 gate
outright, since the flicker M94 was written to prevent is a real
failure mode this variable's own default merely routes around instead
of eliminating -- see `lsp--diagnostics-for-uri''s doc comment for what
merging costs in exchange (no cross-server dedup) and this file's M99
header note for the inline-diagnostic-row consequence.")

(defun lsp--diagnostics-for-uri (client uri)
  "The list of diagnostic hash-tables that should be considered
'published for URI' from CLIENT's point of view, for painting
(`lsp--decorate-buffer') or navigation (`next-diagnostic'/`previous-
diagnostic'/`lsp--diagnostics-at-point').

When `lsp-merge-diagnostics-from-all-clients' is nil: exactly CLIENT's
own stored diagnostics for URI (`lsp--client-diagnostics'), as a list
in their original published order -- the pre-M99 behavior, just
returned as a list instead of a vector so callers don't need to know
which storage shape they got.

When non-nil (the default): the diagnostics stored for URI by every
client in `(lsp--effective-buffer-clients)', in that list's order, each
client's own diagnostics kept in their original published order, CLIENT
appended at the end if it isn't already among them (this matters: a
caller can pass a synthetic CLIENT that was never attached via `M-x lsp'
and so never joined `lsp--buffer-clients' -- a test fixture does exactly
this -- and dropping its diagnostics here would make `lsp-merge-
diagnostics-from-all-clients' silently narrower than advertised).

Dedup happens at the CLIENT-LIST level (the walked list of clients is
made unique by `memq' before anything is read from it), NOT at the
diagnostic-content level -- every diagnostic actually stored on a
client that's walked exactly once is kept unconditionally, with no key
built from its fields at all. This is deliberate, not an omission: a
content-derived key (say, start line/character/severity/message) can
never tell apart \"the same diagnostic walked twice\" (the only real
duplication this function needs to guard against -- CLIENT or another
client appearing more than once in the walked list) from \"two
genuinely different diagnostics that happen to start at the same place
with the same severity and the same wording, but a different `range.
end''\ -- or, across two different servers, two independent diagnoses
of the same real defect that happen to read identically. Both of those
are real diagnostics a user needs to see; silently dropping the second
one because its key collided with the first would be exactly the kind
of quiet data loss this function exists to avoid. See the file header's
M99 note for the same point made about cross-server near-duplicates.

Deliberately NOT filtered by `lsp--client-conn-live-p': a single dead
client's decorations already persist untouched until something
publishes fresh ones (there was never a liveness filter on this path
before M99), and adding one here would be an unrelated behavior change
this milestone isn't scoped to make."
  (if (not lsp-merge-diagnostics-from-all-clients)
      ;; `diags' is a VECTOR (verbatim from `json-parse-string'), and
      ;; this elisp implementation's `append' -- unlike real Emacs --
      ;; only accepts a proper list in a non-final argument position, so
      ;; the vector is walked by hand rather than via `(append diags
      ;; nil)'. No dedup here: this is a single client's own publish,
      ;; verbatim, matching the pre-M99 caller contract exactly.
      (let ((diags (cdr (assoc uri (lsp--client-diagnostics client))))
            (out nil))
        (when diags
          (let ((n (length diags)) (i 0))
            (while (< i n)
              (push (aref diags i) out)
              (setq i (1+ i)))))
        (nreverse out))
    ;; Dedup the CLIENT LIST itself (not diagnostic content -- see the
    ;; docstring above): walk `lsp--effective-buffer-clients', appending
    ;; CLIENT at the end if absent, then drop any client already seen
    ;; earlier in that same walk via `memq', preserving first-occurrence
    ;; order throughout.
    (let ((raw-clients (lsp--effective-buffer-clients)))
      (unless (memq client raw-clients)
        (setq raw-clients (append raw-clients (list client))))
      (let ((clients nil))
        (dolist (c raw-clients)
          (unless (memq c clients)
            (push c clients)))
        (setq clients (nreverse clients))
        (let ((out nil))
          (dolist (c clients)
            (let ((diags (cdr (assoc uri (lsp--client-diagnostics c)))))
              (when diags
                (let ((n (length diags)) (i 0))
                  (while (< i n)
                    (push (aref diags i) out)
                    (setq i (1+ i)))))))
          (nreverse out))))))

(defun lsp--decorate-buffer (client uri diags)
  "M16: turn published diagnostics into wavy underlines + gutter data
for the buffer visiting URI, if any. Old decorations are replaced.

M94: only does any of that when CLIENT is the buffer's diagnostics-
authoritative client (`lsp--diagnostics-authoritative-p') -- a
non-authoritative CLIENT's publish was already stored by
`lsp--merge-diagnostics' before this was ever called, so this function
existing at all changes nothing for a buffer with only one attached
client (its lone client is always authoritative by that function's own
default).

M99: when `lsp-merge-diagnostics-from-all-clients' is non-nil (the
default), the authoritative-client gate above is SKIPPED entirely, and
what actually gets painted is `(lsp--diagnostics-for-uri client uri)'
-- the union across every attached client -- rather than DIAGS (CLIENT's
own fresh publish, still the parameter that got this function called at
all, but no longer what determines what's drawn). This is what removes
the M94 flicker risk rather than merely working around it: whichever
client just published, the repaint always redraws the same union, so
there is nothing left to flicker between. DIAGS itself was already
stored into `lsp--client-diagnostics' by `lsp--merge-diagnostics' before
this function runs, so `lsp--diagnostics-for-uri' sees it."
  (let* ((path (if (string-prefix-p "file://" uri) (substring uri 7) uri))
         (buf (get-file-buffer path)))
    (when buf
      (with-current-buffer-internal buf
        (lambda ()
          (when (or lsp-merge-diagnostics-from-all-clients
                    (lsp--diagnostics-authoritative-p client))
            ;; Drop our old squiggles only.
            (dolist (ov (overlays-in (point-min) (point-max)))
              (when (overlay-get ov 'lsp-diag)
                (delete-overlay ov)))
            (let ((gutter nil))
              (dolist (d (lsp--diagnostics-for-uri client uri))
                (let* ((range (gethash "range" d))
                       (start (gethash "start" range))
                       (end (gethash "end" range))
                       (sev (or (gethash "severity" d) 1))
                       (sev (if (eq sev :null) 1 sev))
                       (from (lsp--pos-at (gethash "line" start)
                                          (gethash "character" start)))
                       (to (lsp--pos-at (gethash "line" end)
                                        (gethash "character" end)))
                       (ov (make-overlay from (if (> to from) to (1+ from))))
                       (msg (or (gethash "message" d) "")))
                  (overlay-put ov 'lsp-diag t)
                  (overlay-put ov 'face
                               (list ':underline
                                     (list ':style 'wave
                                           ':color (lsp--severity-color sev))))
                  ;; M87 stage 3: (LINE . (SEVERITY . MESSAGE)) -- adds the
                  ;; message text `lsp--set-buffer-diagnostics' needs to feed
                  ;; inline diagnostic rows, alongside the gutter dot/count it
                  ;; already fed.
                  (setq gutter (cons (cons (gethash "line" start) (cons sev msg)) gutter))))
              (lsp--set-buffer-diagnostics buf gutter))))))))

(defconst lsp--client-pending-limit 200
  "Maximum number of RESPONSE messages `lsp--dispatch' will hold in
`lsp--client-pending' waiting for an `lsp--await' that never comes
(most concretely: an `lsp--await' call that already hit its own
`lsp-initialize-timeout' and gave up, leaving no waiter behind to ever
claim the answer). Past this many entries the OLDEST is dropped to make
room for the newest (M123) rather than growing without bound for the
rest of the connection's lifetime.

200 is deliberately generous, not tuned: ordinary use produces at most
a handful of these (one per timed-out request), so this cap is a
backstop against a pathological server, not a budget anyone should
expect to bump against in normal editing.")

(defconst lsp--server-request-null-result-methods
  '("client/registerCapability" "client/unregisterCapability"
    "window/workDoneProgress/create")
  "Server-to-client request METHODs this client answers with
`{\"result\": null}' (M123) -- accepting a no-op. Every one of these is
a request whose only content is \"the server would like to register/
create something with the client\"; this client tracks none of them
(no dynamic capability registration, no work-done progress UI), so
acknowledging with a null result is honest: it says \"received, no
objection\" without claiming to have done anything with it. Any OTHER
server-initiated request method gets a JSON-RPC `MethodNotFound' error
instead, via `lsp--respond-to-request'.")

(defun lsp--send-response (client id result)
  "Send a JSON-RPC RESPONSE for request ID with a `result' of RESULT --
never itself a `method' key, unlike every message `lsp--send-message'
builds; a JSON-RPC response is `{jsonrpc, id, result}', with no
`method' at all."
  (let ((h (make-hash-table)))
    (puthash "jsonrpc" "2.0" h)
    (puthash "id" id h)
    (puthash "result" result h)
    (lsp-send (lsp--client-conn client) h)))

(defun lsp--send-error-response (client id code message)
  "Send a JSON-RPC error RESPONSE for request ID: `{jsonrpc, id, error:
{code, message}}'."
  (let ((h (make-hash-table))
        (e (make-hash-table)))
    (puthash "code" code e)
    (puthash "message" message e)
    (puthash "jsonrpc" "2.0" h)
    (puthash "id" id h)
    (puthash "error" e h)
    (lsp-send (lsp--client-conn client) h)))

(defun lsp--respond-to-request (client id method)
  "Answer a server-to-client REQUEST (a message carrying both ID and
METHOD) -- M123. METHOD is looked up in
`lsp--server-request-null-result-methods'; a match gets `{\"result\":
null}' (accepting a no-op), anything else gets a JSON-RPC
`MethodNotFound' (-32601) error, per spec. Either way, the request is
answered here and now: it is never consed onto `lsp--client-pending'
-- that list holds only RESPONSES, waiting for `lsp--await'."
  (if (member method lsp--server-request-null-result-methods)
      (lsp--send-response client id :null)
    (lsp--send-error-response
     client id -32601 (format "Unhandled method: %s" method))))

(defun lsp--dispatch (client msg)
  "Handle one parsed JSON-RPC message: fold a `publishDiagnostics`
notification into CLIENT, answer a server-to-client REQUEST inline
(M123, `lsp--respond-to-request'), deliver an async RESPONSE to its
registered callback, or stash a RESPONSE under its id for `lsp--await`
to pick up.

M123: a message carrying BOTH `id' and `method' is a server-initiated
REQUEST (per the JSON-RPC/LSP spec, a RESPONSE never carries `method')
and is handled by its own cond clause, ahead of the plain-`id' clause
below -- so it can never be mistaken for a response to one of OUR own
requests, closing both the \"server requests are retained forever\" and
the \"id collision\" gaps this file's header used to describe as open.
A plain NOTIFICATION (`method', no `id', not `publishDiagnostics') is
still silently dropped -- unchanged, and intentional (see the header).

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
     ((and id method)
      (lsp--respond-to-request client id method))
     (id
      (let ((cb (assq id (lsp--client-callbacks client))))
        (if cb
            (progn
              (setf (lsp--client-callbacks client)
                    (delq cb (lsp--client-callbacks client)))
              (funcall (cdr cb) (gethash "result" msg)))
          (progn
            (setf (lsp--client-pending client)
                  (cons (cons id msg) (lsp--client-pending client)))
            ;; M123: bound retention -- drop the OLDEST entry (the tail
            ;; of this alist, since new entries are consed onto the
            ;; front) once past the cap. No `butlast' in this elisp
            ;; subset, so reverse/cdr/reverse does the same job.
            (when (> (length (lsp--client-pending client))
                     lsp--client-pending-limit)
              (setf (lsp--client-pending client)
                    (nreverse (cdr (reverse (lsp--client-pending client)))))))))))))

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

(defun lsp--client-capabilities-payload ()
  "The `capabilities' object this client sends in every `initialize'
request (M123, Part B) -- previously an empty hash table. Declares
exactly the two things M123 makes this client actually honour:
`textDocument.completion.completionItem.snippetSupport' (true --
`lsp--expand-snippet' now understands `insertTextFormat' 2) and
`.resolveSupport.properties' (a `completionItem/resolve' reply may fill
in any of these fields beyond what the list item already had --
`documentation'/`detail' are purely informational and unused by this
client today, `additionalTextEdits' is declared honestly even though
this client's own v1 scope still never applies them, per this file's
own header note, because declaring support for a field this client
then ignores would be worse than not declaring it at all).

Measured 2026-09-08: `slang-server' returns the identical 11-snippet-
out-of-16 resolve behaviour whether this object is empty (as it was
pre-M123) or populated like this, so nothing here is required to make
that server work -- it is the other half of the protocol (declaring
what a caller can safely assume this client will do with a resolved
reply), which matters for a DIFFERENT server that gates its own
behaviour on the caller's declared capabilities."
  (let* ((resolve-support (make-hash-table))
         (completion-item (make-hash-table))
         (completion (make-hash-table))
         (text-document (make-hash-table))
         (caps (make-hash-table)))
    (puthash "properties"
             (vector "documentation" "detail" "additionalTextEdits")
             resolve-support)
    (puthash "snippetSupport" t completion-item)
    (puthash "resolveSupport" resolve-support completion-item)
    (puthash "completionItem" completion-item completion)
    (puthash "completion" completion text-document)
    (puthash "textDocument" text-document caps)
    caps))

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
kill.

M99: ROOT-PATH is also passed to `lsp-start' as the server process's
cwd, but only when it names a directory that actually exists on disk
(`file-directory-p'). This matters because some servers (slang-server's
`.slang/server.json') resolve per-project config relative paths against
the server's own cwd, not against `rootUri' or the config file's
location -- pinning the cwd is the only way such a config works
regardless of where the editor itself was launched from. The existence
guard exists so a bogus or stale ROOT-PATH can't turn a spawn that
would otherwise succeed (cwd unset, inherited from the editor) into one
that fails outright."
  (let* ((conn (lsp-start command args
                          (and root-path (file-directory-p root-path) root-path)))
         (client (make-lsp--client :conn conn :command command)))
    (condition-case err
        (let* ((id (lsp--request
                    client "initialize"
                    (let ((p (make-hash-table)))
                      (puthash "processId" :null p)
                      (puthash "rootUri" (if root-path (lsp--path-to-uri root-path) :null) p)
                      (puthash "capabilities" (lsp--client-capabilities-payload) p)
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
staleness/buffer-identity discipline as `lsp-format-buffer'.

M95: routes via `lsp--preferred-role-client' (\"textDocument/
documentSymbol\", \"documentSymbolProvider\") instead of reading
`lsp--live-buffer-client' directly, so a capable secondary answers this
in preference to the primary -- see that variable's own docstring for
why (verible omits ports/parameters and mistypes the module itself). A buffer with only one attached client occupying the PRIMARY slot
behaves identically to before -- see `lsp--preferred-role-client''s own
docstring for the one state that is NOT identical (a lone SECONDARY
sitting in an empty primary slot), which this now answers where it
previously refused."
  (let ((client (lsp--preferred-role-client "textDocument/documentSymbol"
                                             "documentSymbolProvider")))
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
why). Bound to `C-c l s' (M48 Part D, see simple.el).

M95: routes via `lsp--preferred-role-client', same as `lsp--goto-symbol'
-- see its own M95 note."
  (interactive)
  (let ((client (lsp--preferred-role-client "textDocument/documentSymbol"
                                             "documentSymbolProvider")))
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
tracked follow-up, not an M48-shaped change.

M99: reads the same union `lsp--diagnostics-for-uri' reads for painting
-- previously this only ever considered `lsp--buffer-client''s own
diagnostics, so a diagnostic drawn on screen from a secondary client
could be invisible to this function (and hence to `lsp-code-action-at-
point', which calls this) even though the squiggle was right there."
  (let ((client lsp--buffer-client)
        (file (buffer-file-name))
        (pos (point))
        (out nil))
    (when (and client file)
      (dolist (d (lsp--diagnostics-for-uri client (lsp--path-to-uri file)))
        (let* ((range (gethash "range" d))
               (start (gethash "start" range))
               (end (gethash "end" range))
               (from (lsp--pos-at-utf16 (gethash "line" start) (gethash "character" start)))
               (to (lsp--pos-at-utf16 (gethash "line" end) (gethash "character" end))))
          (when (if (= from to) (= pos from) (and (>= pos from) (< pos to)))
            (push d out)))))
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
here\" rather than applying nothing silently.

M95: routes via `lsp--preferred-role-client' (\"textDocument/rename\",
\"renameProvider\") instead of reading `lsp--live-buffer-client'
directly, so a capable secondary answers this in preference to the
primary -- see that variable's own docstring for why (verible drops the
`endmodule : LABEL' end-label on a module rename, an IEEE 1800 compile
error). A buffer with only one attached client occupying the PRIMARY slot
behaves identically to before -- see `lsp--preferred-role-client''s own
docstring for the one state that is NOT identical (a lone SECONDARY
sitting in an empty primary slot), which this now answers where it
previously refused."
  (interactive)
  (let ((client (lsp--preferred-role-client "textDocument/rename"
                                             "renameProvider")))
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

(defvar lsp-secondary-server-alist
  '((verilog-mode . ("slang-server")))
  "Alist of MAJOR-MODE -> (COMMAND . ARGS), mirroring `lsp-server-alist'
above but for a SECOND server attached to the same buffer (M94),
routed by capability rather than picked by the user. `lsp-server-alist'
itself is UNCHANGED -- its entry stays the buffer's PRIMARY client,
`lsp--buffer-client', with every one of its existing semantics and
every existing reader untouched. This table only ever adds to the new
`lsp--buffer-clients' list.

Only autostart (`lsp--autostart-maybe-begin') ever consults this
table -- `M-x lsp' and the `find-file-hook' reuse path
(`lsp--maybe-auto-attach') are unchanged and only ever look at
`lsp-server-alist'. A buffer opened before autostart has run, or with
`lsp-autostart' nil, never gets a secondary client attached; that is
an accepted v1 gap, not a bug (compare `lsp-autostart's own doc for
the equivalent gap the PRIMARY side had before M88).

For `verilog-mode', the default secondary is `slang-server': measured
2026-08-11/2026-09-02 against real binaries on `demo/rtl/', it is the
only one of the two Verilog servers this editor knows about that
advertises `completionProvider' or publishes real elaborated
diagnostics (undriven output ports, variables never assigned) that
verible cannot find structurally -- see `lsp-server-alist''s own
docstring for the fuller verible-versus-slang comparison this decision
is drawn from.

`lsp-completion-at-point' and `lsp-hover-at-point' (M94) route to
whichever attached client's advertised capabilities support the
request. M95 adds five more to that routed set -- definition,
references, documentSymbol, rename, documentHighlight -- each PREFERRING
the secondary over the primary (the opposite tie-break from
`lsp--capable-client''s default), per `lsp-request-preferred-role-alist'
and measured against real binaries: verible is silently incomplete on
references and documentSymbol, drops the `endmodule : LABEL' end-label
on a module rename (an IEEE 1800 compile error), and fails definition/
documentHighlight on a name reached through `import PKG::*'. codeAction
and formatting are UNCHANGED -- formatting because slang has no
formatting capability at all, codeAction because it was never
evaluated for this milestone -- both keep reading `lsp--buffer-client'
directly, so the primary (verible, by default) answers those exactly as
it did before either milestone existed. Picking the better of two
answers when BOTH servers could answer the same request, for any method
NOT in `lsp-request-preferred-role-alist', is still explicitly OUT OF
SCOPE -- that needs per-capability probing of both servers, not an
architecture change, and is left for a follow-up milestone.

Diagnostics: both attached clients' publishes are always STORED
(`lsp--client-diagnostics', already per-client), but only the buffer's
diagnostics-authoritative client's publish is DECORATED on screen --
see `lsp--diagnostics-authoritative-command' for why, and why this
table has no matching \"which one wins\" knob of its own.")

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

(defun lsp--nearest-marker-root (start)
  "Walk upward from START (a directory, trailing slash) looking for the
nearest ancestor -- START itself included -- containing any one of
`lsp--project-root-markers'. Returns that directory (trailing slash),
or nil if none of START's ancestors has one. The original, mode-
agnostic algorithm; kept as its own function so `lsp--project-root'
can fall back to it verbatim for non-Verilog buffers."
  (let ((dir start) (found nil))
    (while (and dir (not found))
      (if (lsp--dir-has-marker-p dir)
          (setq found dir)
        (let ((parent (file-name-directory (directory-file-name dir))))
          (setq dir (if (and parent (not (string= parent dir))) parent nil)))))
    found))

(defun lsp--home-directory ()
  "The user's home directory, no trailing slash, as `expand-file-name'
resolves \"~\". Computed fresh on every call rather than cached at
load time, because M93's own bound test overrides `$HOME' between
calls and must see the change take effect immediately."
  (directory-file-name (expand-file-name "~")))

(defvar lsp--nearest-filelist-root-cache (make-hash-table :test 'equal)
  "Per-(START . HOME) memo for `lsp--nearest-filelist-root'. Added in
M93's fix round: `lsp--autostart-maybe-begin' calls `lsp--project-root'
(hence this function, for a Verilog buffer) from `lsp--autostart-tick'
-- once per idle tick, i.e. once per frame/poll -- and for a buffer
that never attaches (no server registered, autostart already given up,
or simply not connected yet) that repeats the SAME filesystem walk on
every single tick with no caller-side throttling. Without this cache,
a Verilog buffer with no `verible.filelist' anywhere above it pays one
`file-exists-p' per ancestor up to `lsp--home-directory' every tick,
forever. (Reviewer-confirmed, M93 second fix round: reverting this
cache entirely fails no test in this file -- every assertion here
checks only the FINAL root string, and a fresh, uncached walk returns
the identical answer, so the cache's only observable effect is speed.
No test in this suite watches call count or wall time; the 212.59us ->
5.82us improvement recorded in the M93 fix-round report for the
already-attached case, and the smaller before/after difference measured
for this cache specifically, are not something a green test run
certifies on its own.)

Keyed on the PAIR (START . HOME), not START alone -- W2, M93 third fix
round: `lsp--home-directory' reads `$HOME' fresh on every call
specifically so a test overriding it takes effect immediately (see
that function's own docstring), but a cache keyed on START alone would
silently defeat that guarantee -- the same START directory probed
before and after `$HOME' changes within one process would return the
answer computed under the OLD home on the second call, disagreeing
with what a fresh walk would say. Including HOME in the key makes a
home change simply populate a new cache entry instead of returning a
stale one; this was unreachable in practice only because every test
that exercises this cache uses a directory unique to that test, never
reusing a probed START across a `$HOME' override.

The stored value is the walk's own result (a directory string or nil)
wrapped in a one-element list, so a cached \"found nothing\" answer
(value nil) is distinguishable from \"never computed\" via `gethash's
own DEFAULT argument -- a bare `gethash' with no default would
conflate the two, since a hash table's normal miss value is also nil.

Never invalidated in v1 apart from the HOME component above: creating,
deleting, or moving a `verible.filelist' during a live session, with
`$HOME' unchanged, leaves any already-probed directory's answer stale
until the editor restarts. Accepted because (a) a project's marker
layout changing shape underneath a running session is rare, (b) a
stale answer costs at most one wrong autostart root or one wrong
AUTO/nav lookup -- recoverable by restarting the editor or, in v1, not
otherwise -- never silent data loss, and (c) every OTHER repeated call
to `lsp--project-root' for the same file (every `hover'/`definition'
request, for instance) already re-walks the filesystem from scratch
with no caching at all; this cache is a savings layered on top of that
existing behavior, not a new correctness contract this function is
introducing.")

(defun lsp--nearest-filelist-root (start)
  "Walk upward from START (a directory, trailing slash) looking for the
nearest ancestor -- START itself included -- containing a
`verible.filelist' directly. Returns that directory (trailing slash),
or nil if no ancestor has one (memoized -- see
`lsp--nearest-filelist-root-cache'). Unlike `lsp--nearest-marker-root'
this checks ONLY `verible.filelist', ignoring every other entry in
`lsp--project-root-markers' -- so a `.git' or `Cargo.toml' sitting
between START and the filelist does not stop the walk here, which is
the entire point of calling this separately from `lsp--project-root'.

M93 fix round (R1): the walk never ascends past, and never returns,
`lsp--home-directory' -- checked BEFORE probing each directory, so
home itself is never accepted as an answer either. Without this bound
a stray `verible.filelist' left directly in `$HOME', or in a directory
ABOVE `$HOME' shared by several unrelated checkouts (a NAS mount point,
`/Users' itself, ...), would outrank the buffer's own much nearer
`.git' and hand `lsp-connect' a `rootUri' spanning that entire
unrelated tree -- this function's whole reason for existing is to look
PAST a nearer generic marker for a SPECIFIC project's own filelist, not
to wander into a different project (or no project at all) entirely.
Bounding at `$HOME' does not fully close this -- a stray filelist
somewhere under `$HOME' but still above the buffer's own project
remains possible -- but it closes the two concrete cases raised in
review, and a boundary any tighter than `$HOME' has no natural anchor
to use instead.

CONSEQUENCE, spelled out because the mechanism above only states the
means and not the effect (M93 third fix round, W1): a
`verible.filelist' placed directly AT `$HOME' is deliberately never
honoured by this function, even when it is the only filelist anywhere
above the buffer. A buffer under `$HOME' with its own nearer `.git'
and a `verible.filelist' sitting exactly at `$HOME' itself resolves to
the `.git' directory -- i.e. this function returns nil, and
`lsp--project-root' falls through to `lsp--nearest-marker-root' -- NOT
to `$HOME'. This is not an accident left over from the bound above; it
is the bound doing exactly its job. Accepting `$HOME' as an answer
here would hand `lsp-connect' the user's entire home directory as a
`rootUri', which is precisely the failure mode R1 exists to prevent --
a filelist one directory higher (in `$HOME's own parent) and a
filelist AT `$HOME' are the same class of danger, and both are
excluded by the same check. See
`project_root_verilog_filelist_at_home_itself_is_not_honoured'
(`lsp_mode_tests.rs'), which pins this as intended behavior, not
something to \"fix\" back to honouring it."
  (let* ((home (lsp--home-directory))
         (cache-key (cons start home))
         (cached (gethash cache-key lsp--nearest-filelist-root-cache
                           'lsp--filelist-root-not-cached)))
    (if (not (eq cached 'lsp--filelist-root-not-cached))
        (car cached)
      (let ((dir start) (found nil))
        (while (and dir (not found)
                    (not (string= (directory-file-name dir) home)))
          (if (file-exists-p (concat dir "verible.filelist"))
              (setq found dir)
            (let ((parent (file-name-directory (directory-file-name dir))))
              (setq dir (if (and parent (not (string= parent dir))) parent nil)))))
        (puthash cache-key (list found) lsp--nearest-filelist-root-cache)
        found))))

(defun lsp--project-root (file)
  "Project root for FILE: the nearest ancestor directory -- starting at
FILE's own directory and walking up -- containing one of
`lsp--project-root-markers', or FILE's own directory if none do.
Returned without a trailing slash, matching `lsp-connect's ROOT-PATH.

M93: for a Verilog/SystemVerilog FILE (`lsp--verilog-buffer-p'), a
`verible.filelist' outranks every other, nearer marker. The plain walk
above stops at the FIRST ancestor holding ANY marker, so a nearer
`.git' (by far the common case -- a Verilog subtree checked into a
larger repo) shadows a `verible.filelist' that sits further up and
defines the actual project; `verilog-auto.el''s filelist reader would
then silently look in the wrong place -- and worse, `lsp-connect'
sends this exact value as the server's own `rootUri', so the SERVER
itself gets misrooted, not just this editor's local AUTO/nav
convenience. Measured 2026-08-11 (see `lsp--project-root-markers''s
own docstring for the sibling `.slang' case, and M93's recon for this
one) against a real `verible-verilog-ls': `textDocument/definition'
against `rootUri' = the nearer `.git' ancestor returns `[]'; the same
request against `rootUri' = the filelist ancestor resolves correctly.

This deliberately diverges from `verilog-auto.el''s own documented
stance (see its `verilog-auto--library-filelist-files' docstring,
\"known limitation, accepted rather than fixed\") that matching
verible's own rootUri-relative-to-nearest-marker algorithm is correct
even when a `.git' sits in between. That stance holds for a CLIENT-
side reader deciding where to look for a file the SERVER already
computed its own root from independently -- but here the value this
function returns becomes the server's root too, and the measurement
above shows the server needs the filelist ancestor to answer
anything. Diverging is not second-guessing verible; it is refusing to
hand it the one rootUri that makes it blind. Do not \"fix\" this back
into agreement with that other docstring -- they are describing two
different roles for the same string.

Only the nearest `verible.filelist' ancestor wins when several are
nested (`lsp--nearest-filelist-root' stops at the first hit walking
up), never an outer one. A non-Verilog FILE, or a Verilog FILE with no
`verible.filelist' anywhere above it, resolves exactly as before this
milestone -- the marker-precedence list and its ordering are
unchanged for every other language. A non-Verilog FILE is structurally
unreachable from the new codepath at all (`filelist-root' below is
always nil for it), so it costs exactly what it cost before this
milestone, not merely \"resolves the same\". And the new codepath can
never SPLIT a previously shared root into two: `lsp--nearest-filelist-
root's answer, when non-nil, is always the SAME AS or FARTHER FROM
FILE than `lsp--nearest-marker-root's own answer would have been
(a `verible.filelist' ancestor at or nearer than the nearest generic
marker is already that nearest marker's own directory, since it's
itself one of `lsp--project-root-markers') -- so two files that used to
share a root because the SAME `.git' covered both keep sharing a root
after this milestone; the only thing that can change is which
directory that shared root actually is.

M93 fix round: this is bounded at `lsp--home-directory' (see
`lsp--nearest-filelist-root's own docstring for why and its limits) --
a stray filelist above `$HOME' can no longer outrank a buffer's own
much nearer marker.

Case sensitivity: `lsp--verilog-buffer-p' matches `.v'/`.vh'/`.sv'/
`.svh' exactly, so a file named e.g. `Top.V' is not recognized as
Verilog by this function and silently falls back to the pre-M93
shadowed walk for that one file -- the very bug this milestone exists
to fix, just for that extension spelling. Deliberately not changed
here: the predicate is shared with `lsp--references-empty-message' and
is out of this milestone's scope; recorded so the next reader finds it
stated rather than by surprise.

NOT handled here, and out of scope for M93: a `verible.filelist' that
exists but omits files actually on disk. The server then answers
incompletely with no signal on the wire at all (confirmed by capturing
the full JSON-RPC session: no `partialResultToken', no
`window/logMessage', nothing on the response envelope) -- detecting
that needs a disk walk cross-checked against the filelist, which lives
in `verilog-auto.el' and is not this function's job."
  (let* ((start (file-name-directory (expand-file-name file)))
         (filelist-root (and (lsp--verilog-buffer-p file)
                              (lsp--nearest-filelist-root start)))
         (found (or filelist-root (lsp--nearest-marker-root start))))
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
  "Buffer-local: the `lsp--client' this buffer talks to -- the buffer's
PRIMARY client (M94: see `lsp--buffer-clients' for every attached
client, primary included). Set by `lsp' the first time it connects
successfully in a buffer; nil if `lsp' hasn't been run here (or
failed). `lsp-definition-at-point' and the diagnostic-navigation
commands read this rather than taking a client argument, and every
request site except completion/hover (M94's two capability-routed
exceptions -- see `lsp--capable-client') still does too, unchanged.
`lsp-hover-at-point' now reads `lsp--capable-client' instead, which
falls back to exactly this variable whenever no OTHER attached client
supports `\"hoverProvider\"'.")

(defvar lsp--buffer-clients nil
  "Buffer-local (M94): every `lsp--client' attached to this buffer,
`lsp--buffer-client' (the primary) included. `lsp--buffer-client'
itself is UNCHANGED by this variable existing -- same semantics, same
readers -- this list only ever grows alongside it, in
`lsp--attach-current-buffer', so per-capability routing
(`lsp--capable-client') and per-client protocol sync
(`lsp--last-synced-tick') can walk every attached client without
disturbing anything that reads `lsp--buffer-client' directly. Order is
attach order, most-recently-attached first (the same `cons' discipline
`lsp--clients' already uses); `lsp--buffer-client' is always the
authoritative answer for \"which one is primary\", never this list's
first element, regardless of order.

M94 review Z5 correction: `lsp--capable-client' DOES rely on this
list's order for its fallback (non-primary) branch -- when the primary
itself isn't capable, it picks the first OTHER capable client in THIS
order, i.e. whichever attached most recently among the rest. It checks
the primary FIRST, separately, precisely so a merely-later-attached
secondary never wins a tie against an equally-capable primary; see
that function's own docstring for the concrete case (`hoverProvider:
false' on verible) this exists to prevent.
See `lsp--effective-buffer-clients' for how a caller (or a pre-M94
test that only ever sets `lsp--buffer-client' directly) that never
touches this list at all is still treated as \"one attached client\",
not \"none\".")

(defvar lsp--last-synced-tick nil
  "Buffer-local: alist of (CLIENT . TICK) -- M94, was a single integer
per buffer before a buffer could have more than one attached client.
TICK is the `buffer-modified-tick' as of the last `textDocument/
didOpen' or `textDocument/didChange' sent to CLIENT for THIS buffer.
Set alongside `lsp--buffer-client'/`lsp--buffer-clients' by
`lsp--attach-current-buffer' right after CLIENT's own didOpen (via
`lsp--set-client-synced-tick'), and updated per-client by
`lsp--sync-buffer-now' after each didChange it sends. A client with no
entry here has never been didOpen'd for THIS buffer -- most concretely,
one that attaches AFTER an edit a different, already-attached client
already saw: that edit must never be treated as already synced to the
newcomer, since it never actually received a didChange (or a didOpen
whose text already covered it) -- `lsp--attach-current-buffer' always
creates a fresh entry at the CURRENT tick from CLIENT's own didOpen, so
this invariant holds by construction rather than by comparing tick
numbers after the fact. `lsp--client-synced-tick'/
`lsp--set-client-synced-tick'/`lsp--clear-client-synced-tick' are the
only things that read or write this alist; nothing else in this file
inspects its shape directly.")

(defun lsp--client-synced-tick (client)
  "TICK last synced to CLIENT for the current buffer (M94), per
`lsp--last-synced-tick', or nil if CLIENT has never been didOpen'd
here."
  (cdr (assq client lsp--last-synced-tick)))

(defun lsp--set-client-synced-tick (client tick)
  "Buffer-locally record TICK as the tick last synced to CLIENT (M94),
replacing any existing entry for CLIENT in `lsp--last-synced-tick'."
  (setq-local lsp--last-synced-tick
              (cons (cons client tick)
                    (let (out)
                      (dolist (entry lsp--last-synced-tick (nreverse out))
                        (unless (eq (car entry) client)
                          (push entry out)))))))

(defun lsp--clear-client-synced-tick (client)
  "Remove CLIENT's entry from `lsp--last-synced-tick' (M94), if any."
  (setq-local lsp--last-synced-tick
              (let (out)
                (dolist (entry lsp--last-synced-tick (nreverse out))
                  (unless (eq (car entry) client)
                    (push entry out))))))

(defun lsp--client-conn-live-p (client)
  "Non-nil if CLIENT (M94) is usable for a protocol send: either not a
real `lsp--client' struct at all (a test's bare stand-in symbol, same
asymmetric trust `lsp--live-buffer-client' already extends to a
non-struct `lsp--buffer-client'), or a real struct whose own `conn' is
either not a probe-able connection object or reports alive."
  (or (not (lsp--client-p client))
      (let ((conn (lsp--client-conn client)))
        (not (and (lsp-connection-p conn) (not (lsp-live-p conn)))))))

(defun lsp--effective-buffer-clients ()
  "`lsp--buffer-clients' (M94), with `lsp--buffer-client' prepended if
it isn't already a member -- so a caller (or a pre-M94 test) that only
ever sets `lsp--buffer-client' directly, never touching the new list at
all, is still treated by every M94 routing/sync helper as \"this buffer
has one attached client\", exactly the single-client behavior every
reader of `lsp--buffer-client' has been allowed to assume since before
this milestone, rather than \"this buffer has none\"."
  (if (and lsp--buffer-client (not (memq lsp--buffer-client lsp--buffer-clients)))
      (cons lsp--buffer-client lsp--buffer-clients)
    lsp--buffer-clients))

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

(defun lsp--secondary-server-for-mode (mode)
  "(COMMAND . ARGS) registered for MODE in `lsp-secondary-server-alist'
(M94), or nil."
  (cdr (assq mode lsp-secondary-server-alist)))

(defun lsp--client-role-is-primary-p (client mode)
  "Non-nil if CLIENT (M94) is ELIGIBLE to occupy the PRIMARY slot,
`lsp--buffer-client', for a buffer in major MODE: its own advertised
`command' (M59) matches MODE's `lsp-server-alist' entry. A client
whose command only matches `lsp-secondary-server-alist' (or neither
table -- unreachable in production, since every attach path threads
its client's command through one of the two tables, but a test could
construct one) is never eligible, REGARDLESS of whether the primary
slot happens to be empty right now -- see `lsp--attach-current-
buffer''s own M94 Z2 note for why \"the slot is empty\" was never a
safe substitute for \"this client is actually the primary\".

M94 review AA3: CLIENT's `command' is required to be a real (non-nil)
string, checked explicitly -- without this, a MODE with no
`lsp-server-alist' entry at all makes `(lsp--server-for-mode mode)'
return nil, so `(car (lsp--server-for-mode mode))' is also nil, and a
client whose own `command' also happens to be nil (a pre-M59 client,
or a hand-built test stub that never set `:command') would satisfy
`(equal nil nil)' -- \"eligible\", directly contradicting this
docstring's own \"never eligible\" claim about anything not threaded
through a real table entry. No reachable production path can hit this
(every live-client producer supplies a real command string), so this
is closing a contract gap, not fixing a live bug."
  (and (lsp--client-p client)
       (lsp--client-command client)
       (equal (lsp--client-command client) (car (lsp--server-for-mode mode)))))

(defvar inline-diagnostics t
  "Non-nil shows each LSP diagnostic as an extra row drawn directly under
the buffer line it belongs to (M87 stage 3), in addition to the gutter
dot and modeline count, which this variable does not affect. A plain
global toggle, like `hl-line-mode' in simple.el. Set to nil to reproduce
the pre-stage-3 `Grid' exactly -- no block rows, no row-scale/row-kind
change on any row.")

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
fire from `find-file' and, via backfill, several times in a row.
M88 adds one more trigger: the autostart completion closure's own call
to `lsp--auto-attach-backfill', reachable from the ambient idle tick
with no user action at all.

M94: CLIENT is only ever recorded as `lsp--buffer-client' (the
PRIMARY) when the buffer doesn't already have one -- attaching a
SECOND client (from `lsp-secondary-server-alist', via autostart) must
never clobber a primary that's already there. CLIENT is unconditionally
added to `lsp--buffer-clients' on a successful didOpen either way (if
not already a member -- `lsp' itself calling this twice for the exact
same CLIENT is prevented one layer up, by its own already-connected
check, but nothing stops two DIFFERENT call sites, e.g. `M-x lsp' and a
concurrent autostart backfill, from racing to attach the same CLIENT,
so this guards it directly rather than trusting every caller not to).
A failed didOpen only rolls back `lsp--buffer-client' to nil when THIS
call is the one that set it (i.e. the buffer had no LIVE primary
before) -- an already-established primary must survive a SECONDARY's
own didOpen failure untouched.

\"Already has one\" is checked via `lsp--live-buffer-client', not a
raw truthy read of `lsp--buffer-client' (M63's reattach fix, carried
over): a buffer whose OWN primary connection died earlier still has a
non-nil `lsp--buffer-client' pointing at the corpse, and a plain
truthy check would refuse to ever replace it -- `lsp--live-buffer-
client's clearing side effect is exactly what lets a fresh CLIENT
become the new primary here, same as it always has.

M94 review Z2: \"already has one\" alone is not enough -- CLIENT must
also be ELIGIBLE for the primary slot, per `lsp--client-role-is-
primary-p' (its own `command' must match MODE's `lsp-server-alist'
entry, not just any entry). Before this fix, the ONLY thing deciding
primacy was whether the slot was empty: verible attaches (primary);
verible dies with nothing yet clearing the stale reference (the mode
line does not probe liveness every frame); a slang autostart already
in flight completes and reaches this function via `lsp--auto-attach-
backfill'; `lsp--live-buffer-client' probes, finds the dead verible,
nils the slot -- and slang, a SECONDARY, walked straight into the now-
empty primary slot. Every untouched request site (definition,
references, rename, documentSymbol, codeAction, documentHighlight,
formatting) would then talk to a server whose own docstring says it
has no formatting of any kind, and reconnecting verible afterwards
would find a live primary (slang) and merely append -- it would never
self-heal. Now a client that isn't primary-table-eligible always joins
`lsp--buffer-clients' but never touches the primary slot, empty or
not, leaving it open for the real primary to reclaim."
  (let* ((had-primary (lsp--live-buffer-client))
         (is-primary-candidate (lsp--client-role-is-primary-p client mode))
         (set-primary (and is-primary-candidate (not had-primary))))
    (when set-primary
      (setq-local lsp--buffer-client client))
    (condition-case err
        (progn
          (lsp-did-open client (buffer-file-name) (buffer-string)
                        (cdr (assq mode lsp-language-id-alist)))
          ;; Same tick didOpen just sent as its version (M35): no edit can
          ;; land between the two calls, so this is the baseline
          ;; `lsp--sync-buffer-now' diffs future edits against -- now
          ;; per-CLIENT (M94).
          (lsp--set-client-synced-tick client (buffer-modified-tick))
          (unless (memq client lsp--buffer-clients)
            (setq-local lsp--buffer-clients (cons client lsp--buffer-clients))))
      (error
       (when set-primary
         (setq-local lsp--buffer-client nil))
       (lsp--clear-client-synced-tick client)
       (signal (car err) (cdr err))))))

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

(defun lsp--buffer-has-live-client-for-command-p (command)
  "Non-nil if the current buffer already has a LIVE client (per
`lsp--effective-buffer-clients', M94) whose own advertised `command'
(M59) `equal's COMMAND. Unlike `lsp--live-buffer-client' (which only
ever answers for the PRIMARY, regardless of which server it talks to),
this is what M94's per-command guards need: a buffer with a live
PRIMARY for one server must not be mistaken for \"already covered\"
when a DIFFERENT server entirely is what's actually being asked about
-- the exact silent-sink failure `lsp--autostart-maybe-begin' and
`lsp--auto-attach-backfill' both had before this function existed (see
their own M94 notes)."
  (let (found)
    (dolist (client (lsp--effective-buffer-clients) found)
      (when (and (not found)
                 (lsp--client-p client)
                 (lsp--client-conn-live-p client)
                 (equal (lsp--client-command client) command))
        (setq found t)))))

(defun lsp--auto-attach-backfill-matches-p (file mode client)
  "Non-nil if FILE (a non-remote file-visiting buffer's path, whose
BUF-MODE the caller has already confirmed `eq' to MODE) should be
attached to CLIENT by `lsp--auto-attach-backfill' (M94): CLIENT's own
advertised `command' (M59) matches EITHER `lsp-server-alist''s or
`lsp-secondary-server-alist''s entry for MODE, and the live connection
already registered for that exact (COMMAND . ROOT) key is CLIENT
itself.

Generalizes the pre-M94 check, `(eq (lsp--auto-attach-client file
mode) client)' -- that helper only ever consulted `lsp-server-alist'
(the PRIMARY table), so it could never match a SECONDARY client at
all, not even for the very buffer whose own idle tick started the
secondary's autostart in the first place; backfill would silently
attach nothing for it. `lsp--auto-attach-client' itself is
deliberately left untouched (still primary-only) -- it also backs
`lsp--maybe-auto-attach', the `find-file-hook' reuse path, which stays
primary-only by design in this milestone (see `lsp-secondary-server-
alist''s own doc).

Checks `lsp-auto-attach' itself (guard 1 of `lsp--auto-attach-client',
carried over explicitly): the old `eq'-against-`lsp--auto-attach-
client' check got this for free since that helper's own first guard
is `(not lsp-auto-attach)'; this replacement calls neither
`lsp--auto-attach-client' nor `lsp--server-for-mode' the same way, so
without repeating the check here, `lsp-auto-attach' nil would no
longer suppress BACKFILL (only the `find-file-hook' path would still
honor it), regressing M88's F4b fix."
  (and lsp-auto-attach
       file
       (not (lsp--remote-path-p file))
       (lsp--client-p client)
       (let ((command (lsp--client-command client)))
         (and command
              (or (equal (car (lsp--server-for-mode mode)) command)
                  (equal (car (lsp--secondary-server-for-mode mode)) command))
              (eq (lsp--get-connection command (lsp--project-root file)) client)))))

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

The \"already attached\" guard checks `lsp--buffer-has-live-client-for-
command-p' (M94), not `lsp--live-buffer-client' (M63 round 2's fix,
generalized): a buffer whose OWN connection died earlier still has a
non-nil `lsp--buffer-client' pointing at the corpse, and a plain
truthy check would mistake that for \"already covered\" and skip it
forever -- even though the connection CLIENT points to here is a
fresh, live one the corpse's buffer should now be talking to instead.
Checking per-COMMAND rather than per-buffer-any-client is what M94
adds: a buffer with a live PRIMARY (verible, say) must not be
mistaken for \"already covered\" when CLIENT is a SECONDARY (slang)
autostarting for the first time -- `(not (lsp--live-buffer-client))'
would have silently sunk that for every buffer in the project,
including the very one whose idle tick started the handshake.

The MATCH itself uses `lsp--auto-attach-backfill-matches-p' (M94),
not the old `(eq (lsp--auto-attach-client file mode) client)' -- see
that function's own doc for why the old check could never match a
secondary client at all.

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
                       (not (lsp--buffer-has-live-client-for-command-p
                             (lsp--client-command client)))
                       (lsp--auto-attach-backfill-matches-p file mode client))
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
        ;; M88 F1 review fix: an explicit `M-x lsp' always wins a race
        ;; against an in-flight autostart for this exact key -- cancel
        ;; it first (kills nothing that's attached yet, since nothing
        ;; is: I2) so the connection this call is about to reuse-or-make
        ;; is the only one left standing, instead of the autostart's own
        ;; completion later shadowing it in `lsp--connections' while its
        ;; own server process leaks for the rest of the session.
        (lsp--autostart-cancel-pending command root)
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

(defun lsp--capable-client (key)
  "This buffer's PRIMARY (`lsp--buffer-client'), if it's LIVE and
supports KEY (a JSON key string, via `lsp--capability-supported-p' --
e.g. \"completionProvider\" or \"hoverProvider\"); otherwise the first
OTHER live, capable client in `lsp--effective-buffer-clients' (M94,
attach order -- most-recently-attached first, per that variable's own
doc). nil if the buffer has no attached client at all, or none of them
support KEY. Calls `lsp--live-buffer-client' first, for its usual
dead-PRIMARY-clearing side effect, same as every other buffer-local
client reader in this file, before looking anywhere else.

M94 review Z5: the primary is checked FIRST, deliberately, not simply
whichever attached client comes first in `lsp--buffer-clients' --
attach order is most-recent-first, so a secondary (attached AFTER the
primary, the common case) would otherwise always win a tie. verible
(the Verilog default primary) advertising `hoverProvider: false' still
counts as SUPPORTED under M46's asymmetric-trust rule, so without this
preference every hover in a two-client Verilog buffer would go to the
secondary even though the primary could have answered it too.

Two request sites call this DIRECTLY (M94, deliberately narrow):
completion and hover are the only two capabilities that milestone
treated as having exactly ONE possible answer among a buffer's attached
clients (see `lsp-secondary-server-alist''s own doc for why), with no
preferred role of their own -- primary-first is exactly right for both.
M95's five methods (definition, references, documentSymbol, rename,
documentHighlight) do NOT call this function at all -- see
`lsp--preferred-role-client''s own docstring for why its fallback is
deliberately the UNGATED `lsp--live-buffer-client' instead: those five
were never gated on `lsp--capability-supported-p' before M95, and a
fallback through THIS function would have re-gated them, which a
review round caught as a regression. codeAction and formatting keep
reading `lsp--buffer-client' directly and are unaffected by either
function existing at all, so the primary answers those exactly as it
did before M94."
  (lsp--live-buffer-client)
  (cond
   ((and lsp--buffer-client
         (lsp--client-conn-live-p lsp--buffer-client)
         (lsp--capability-supported-p lsp--buffer-client key))
    lsp--buffer-client)
   (t
    (let (found)
      (dolist (client (lsp--effective-buffer-clients) found)
        (when (and (not found)
                   (not (eq client lsp--buffer-client))
                   (lsp--client-conn-live-p client)
                   (lsp--capability-supported-p client key))
          (setq found client)))))))

(defvar lsp-request-preferred-role-alist
  '(("textDocument/definition" . secondary)
    ("textDocument/references" . secondary)
    ("textDocument/documentSymbol" . secondary)
    ("textDocument/rename" . secondary)
    ("textDocument/documentHighlight" . secondary))
  "Alist of (METHOD . ROLE), M95: which role (only `secondary' as of
M95; `primary' would be a legal value but nothing needs to say so
explicitly -- that is `lsp--capable-client''s own default) should answer
METHOD (a JSON-RPC method string, e.g. \"textDocument/definition\"),
consulted by `lsp--preferred-role-client'. A method with NO entry here
is entirely unaffected by this table existing -- `lsp--preferred-role-
client' falls straight through to `lsp--capable-client''s ordinary
primary-first order for it, exactly as before this variable existed.

Seeded with the five methods M95 measured verible-verilog-ls (Verilog's
default primary) answering worse than slang-server (Verilog's default
secondary) against real binaries on `demo/rtl/', cross-checked against
`grep' ground truth -- see PLAN.md's M95 record for the full probe:
references silently omits the symbol's own declaration and does no
cross-file lookup for a module name at all (1/3, 1/2, 4/5, 0/7 against
slang's exact 3/3, 2/2, 5/5, 7/7); rename drops the `endmodule : LABEL'
end-label on a module rename, which IEEE 1800 SS23.2.5 requires to
match -- a compile error, not a cosmetic gap; definition and
documentHighlight both fail on a name reached through `import PKG::*'
(the style `demo/rtl/top/soc_top.sv' itself uses), the latter also
producing a false-positive merge of two unrelated identically-spelled
identifiers; documentSymbol returns fewer than half as many symbols,
types the module itself wrong, and omits every port and parameter.

Neither server surfaces `typedef' declarations (enum or struct) in
documentSymbol at all, even though both resolve those same types fine
for `definition' -- a real, shared gap in both servers that routing
cannot fix and does not attempt to; it stays a known gap regardless of
which one answers documentSymbol.

codeAction and formatting are deliberately absent -- formatting because
slang has no formatting capability at all (see `lsp-secondary-server-
alist''s own doc), codeAction because it was never evaluated for this
milestone.")

(defun lsp--preferred-role-client (method key)
  "The client that should answer METHOD (a JSON-RPC method string),
given KEY (its capability JSON key, exactly as `lsp--capability-
supported-p' expects -- e.g. \"definitionProvider\" for
\"textDocument/definition\").

M95: if `lsp-request-preferred-role-alist' maps METHOD to `secondary',
the first live, capable, NON-primary client in `lsp--effective-buffer-
clients' wins outright, ahead of the primary -- the opposite tie-break
from `lsp--capable-client''s own default (see that function's
docstring for why the default exists at all: without it, a hover in a
two-client Verilog buffer would always go to the secondary even when
the primary could answer too). That default is still exactly right for
every method NOT listed in the preference table, which is why it is
never overridden globally, only for the five methods this alist names.

Falls back to the PRIMARY (`lsp--live-buffer-client', already captured
above as PRIMARY) whenever METHOD has no entry in the preference table,
prefers `primary', or the preferred secondary is missing, dead, or does
not declare KEY. Deliberately `lsp--live-buffer-client', NOT
`lsp--capable-client' -- these five methods were never gated on
`lsp--capability-supported-p' before M95 (see the M94-era docstring
this one replaced), and the fallback exists precisely to reproduce that
UNGATED behaviour exactly: a secondary has to EARN the request by
actually declaring KEY, but the primary is asked exactly as it always
was, capabilities hash or no. (`lsp-format-buffer' shows what an
explicit \"server doesn't advertise this\" message for an ungated
primary would look like, via `lsp--capability-supported-p' -- adding
one here for these five would be a genuine improvement, but a separate
decision from this fallback fix, not a side effect of it.) This is also
what keeps a single-server buffer WHOSE ONE CLIENT OCCUPIES THE PRIMARY
SLOT (every language here except Verilog -- a Rust buffer has only
rust-analyzer, and it is the primary) indistinguishable from before
this function existed: with no secondary attached at all,
`lsp--effective-buffer-clients' contains only the primary, so the
`secondary' branch below never finds a non-primary candidate and
control always falls through to PRIMARY, ungated, exactly as it did
before M95.

One single-client state is NOT identical, and this is deliberate: a
buffer whose SOLE attached client is a SECONDARY sitting in an EMPTY
primary slot -- exactly the state M94's Z2 self-heal produces (the
primary dies; a client whose own `command' doesn't match the mode's
primary table entry, per `lsp--client-role-is-primary-p', joins
`lsp--buffer-clients' without ever taking the now-empty slot). There
PRIMARY is nil, so `(not (eq client primary))' is vacuously true for
every candidate in the scan below, and a live, capable lone secondary
answers. Before M95 these five methods read `lsp--buffer-client' raw,
got nil, and reported \"No LSP server connected in this buffer\" even
though a live, capable server was attached -- so this is an
IMPROVEMENT over the pre-M95 behaviour in that one state, not a
regression, and it is kept rather than special-cased away. But it is
NOT the same behaviour, and any docstring or comment that claims a
single attached client is unconditionally indistinguishable from
before M95 is wrong; the accurate claim is conditioned on that client
occupying the primary slot. `lsp-code-action-at-point' and
`lsp-format-buffer'/`lsp-format-region' are UNCHANGED by any of this --
they still read `lsp--buffer-client' directly, so they still refuse in
exactly that lone-secondary-in-an-empty-primary-slot state."
  (let ((primary (lsp--live-buffer-client)))
    (or (and (eq (cdr (assoc method lsp-request-preferred-role-alist)) 'secondary)
             (let (found)
               (dolist (client (lsp--effective-buffer-clients) found)
                 (when (and (not found)
                            (not (eq client primary))
                            (lsp--client-conn-live-p client)
                            (lsp--capability-supported-p client key))
                   (setq found client)))))
        primary)))

;; --- Edit sync: textDocument/didChange (M35) ---

(defun lsp--sync-buffer-now ()
  "For EVERY client attached to the current buffer (M94:
`lsp--effective-buffer-clients', primary included -- was just the
primary alone before a buffer could have more than one), if it's live
and has been edited since the tick last synced to IT (per-client,
`lsp--client-synced-tick'), send a full-text `textDocument/didChange'
with the current `buffer-modified-tick' as its version and record it
via `lsp--set-client-synced-tick'. A client not yet didOpen'd for this
buffer (no entry at all) is skipped, same as before -- only now that's
decided per client, not once for the whole buffer. Otherwise a silent
no-op per client, same \"never signals\" spirit as
`lsp-process-pending-all'.

`lsp--live-buffer-client' is still called first, for its usual
dead-PRIMARY-clearing side effect -- every other buffer-local client
reader in this file does the same before looking anywhere else.

Two call sites, both wanting the server(s) to answer against text they
have actually seen: the idle pump (`lsp-process-pending-all', once
over every buffer -- typing itself never triggers this, only a later
idle tick does, a natural debounce) and `lsp-hover-at-point'/
`lsp-definition-at-point', immediately before sending their request so
an edit is never left unsynced across a hover/definition round trip
even if the idle pump hasn't run yet.

M94 review Z6: also opportunistically prunes any now-dead client out
of `lsp--buffer-clients' (and its `lsp--last-synced-tick' entry) while
it's already here checking every attached client's liveness anyway --
see the code's own comment just below the sync loop."
  (lsp--live-buffer-client)
  (when (buffer-file-name)
    (let ((tick (buffer-modified-tick)))
      (dolist (client (lsp--effective-buffer-clients))
        (when (lsp--client-conn-live-p client)
          (let ((synced (lsp--client-synced-tick client)))
            (when (and synced (/= tick synced))
              (lsp-did-change client (buffer-file-name) (buffer-string) tick)
              (lsp--set-client-synced-tick client tick)))))))
  ;; M94 review Z6: opportunistic pruning, not a new sweep -- this
  ;; function already walks every attached client for liveness on every
  ;; idle tick, so dropping a now-dead one out of `lsp--buffer-clients'
  ;; (and its own `lsp--last-synced-tick' entry) here costs nothing
  ;; extra. Every consumer already re-checks liveness itself, so leaving
  ;; a corpse in the list was never a correctness bug -- only an
  ;; unbounded list for the buffer's lifetime, which this bounds.
  (when lsp--buffer-clients
    (dolist (client lsp--buffer-clients)
      (unless (lsp--client-conn-live-p client)
        (lsp--clear-client-synced-tick client)))
    (setq-local lsp--buffer-clients
                (let (live)
                  (dolist (client lsp--buffer-clients (nreverse live))
                    (when (lsp--client-conn-live-p client)
                      (push client live)))))))

;; --- Save/kill hooks: textDocument/didSave, textDocument/didClose (M40) ---

(defun lsp--on-after-save ()
  "`after-save-hook' function: gets every attached client's copy caught
up via `lsp--sync-buffer-now' (M94: now every attached client, not
just the primary) -- `lsp-did-save' sends no text of its own, so the
server must already have the saved content from a didChange -- then
sends `textDocument/didSave' to every LIVE, didOpen'd attached client
(`lsp--effective-buffer-clients').

Wrapped in `condition-case' and a silent no-op on every other path (no
attached client, or none of them didOpen'd): `save-buffer' runs this
on every save in every buffer, LSP-connected or not, across this whole
editor's test suite, so anything else here would spam the echo area
on an ordinary save."
  (condition-case nil
      (when (buffer-file-name)
        (lsp--sync-buffer-now)
        (dolist (client (lsp--effective-buffer-clients))
          (when (and (lsp--client-synced-tick client)
                     (lsp--client-conn-live-p client))
            (lsp-did-save client (buffer-file-name)))))
    (error nil)))

(defun lsp--on-kill-buffer ()
  "`kill-buffer-hook' function: sends `textDocument/didClose' to every
LIVE, didOpen'd attached client (M94: `lsp--effective-buffer-clients',
was just the primary alone before) and drops this buffer's URI from
EACH of their `lsp--client-diagnostics' alists. Same silent-no-op/
`condition-case' discipline as `lsp--on-after-save' -- every buffer
kill in the test suite runs this hook, LSP-connected or not."
  (condition-case nil
      (when (buffer-file-name)
        (let ((uri (lsp--path-to-uri (buffer-file-name))))
          (dolist (client (lsp--effective-buffer-clients))
            (when (and (lsp--client-synced-tick client)
                       (lsp--client-conn-live-p client))
              (lsp-did-close client (buffer-file-name))
              (setf (lsp--client-diagnostics client)
                    (delq (assoc uri (lsp--client-diagnostics client))
                          (lsp--client-diagnostics client)))))))
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

;; --- Autostart: spawning a server on first open, asynchronously (M88) ---
;;
;; `lsp-auto-attach' (above) is a pure REUSE mechanism -- it never
;; spawns, so the first file in a project still needs one real `M-x
;; lsp' before any of this editor's diagnostic UI (gutter dots,
;; squiggles, the mode-line count, M87 stage 3's inline diagnostic
;; rows) ever lights up. This section is what actually starts a server
;; on its own, driven from the idle tick rather than `find-file-hook'
;; (see the milestone spec for why the trigger has to be the idle tick
;; and not a hook: several test files open real Verilog under `demo/'
;; via `find-file-internal' with a real `verible-verilog-ls' on the
;; test machine's PATH, and a hook trigger would spawn a real server in
;; every one of them).
;;
;; `lsp-connect' itself is untouched -- this is an ADDITIVE async path
;; built entirely from primitives that already exist and are already in
;; daily use: `lsp-request-async' plus the idle pump
;; (`lsp-process-pending-all') already deliver hover/documentHighlight/
;; definition without blocking. `initialize' here is just one more
;; async request, and attaching every buffer the new connection covers
;; once it completes is just one more call to the existing
;; `lsp--auto-attach-backfill'.
;;
;; No request queue: no buffer is attached to the pending client while
;; its handshake is in flight (`lsp--attach-current-buffer', which sets
;; `lsp--buffer-client', is never called until the completion callback
;; below runs), so there is nothing to queue against it in the
;; meantime. `didOpen' is sent by backfill at completion time, against
;; the buffer's text AS IT IS THEN -- an edit made during the wait is
;; therefore included in the didOpen text, not queued up stale behind
;; it.
;;
;; This is also the single invariant the whole design rests on: a
;; half-initialized client sits in `lsp--clients' during the wait (so
;; the existing idle pump can drain its `initialize' reply at all), and
;; that pump's own buffer walk calls `lsp--sync-buffer-now' against
;; EVERY buffer, unconditionally, every tick. That's safe here only
;; because `lsp--buffer-client' is still nil for every buffer this
;; pending client will eventually cover -- `lsp--sync-buffer-now'
;; requires a live `lsp--buffer-client' AND a non-nil
;; `lsp--last-synced-tick' before it will send anything, and neither is
;; set until `lsp--attach-current-buffer' runs, which only happens
;; inside the completion callback's call to `lsp--auto-attach-backfill'
;; -- well after `initialized' has already gone out. So no `didChange'
;; can ever precede its own `didOpen'.
;;
;; Two honest caveats to "nothing on this path blocks", added at
;; review (M88 fix round):
;;
;;  - `lsp-kill' (used to reap a dead-air pending handshake, and by
;;    `lsp--autostart-cancel-pending' when `M-x lsp' preempts one) is
;;    `child.kill()' followed by a BLOCKING `child.wait()' with no
;;    timeout (`crates/elisp/src/lsp.rs'). `SIGKILL' can't be caught or
;;    blocked by the child, but a process wedged in an uninterruptible
;;    kernel wait (blocked on a hung filesystem or device, say) would
;;    still make `wait()' block here regardless. This primitive
;;    predates M88 -- every prior caller reached it from a deliberately-
;;    pressed command (`lsp-shutdown', a failed `lsp-connect') -- but
;;    M88 is the first thing that can reach it from the AMBIENT idle
;;    tick, with no user action involved at all, in that pathological
;;    case.
;;  - The completion closure's `lsp--auto-attach-backfill' sends
;;    `didOpen' through `lsp-send', whose `write_all' can block on a
;;    large document with a slow-draining server -- already documented
;;    as a known v1 gap at `lsp--attach-current-buffer''s own docstring
;;    (M63 coordinator round 2), and now reachable from autostart
;;    completion as well as from `M-x lsp'/backfill/`find-file'.

(defvar lsp-autostart t
  "When non-nil, the idle tick (`lsp--autostart-tick', called once per
frontend frame/poll from the Rust side) spawns an LSP server on its own
the first time a buffer with no live connection and no server already
running for its project is seen, instead of requiring an explicit
`M-x lsp'. The handshake runs entirely asynchronously (`lsp-request-
async', never `lsp--await' or anything else that blocks) -- typing is
never held up waiting for a server, however slow or absent it is.

Coupled to `lsp-auto-attach': autostart is gated on `(and lsp-autostart
lsp-auto-attach)', because the only thing that ever attaches a buffer
to an autostarted connection IS the auto-attach machinery
(`lsp--auto-attach-backfill', run once the handshake completes). With
`lsp-auto-attach' nil, backfill would attach nothing, so spawning would
leave a server nobody ever talks to -- pure waste, and one more process
for the user to wonder about. Set either variable to nil to fall back
to requiring an explicit `M-x lsp' everywhere.

Every (COMMAND . PROJECT-ROOT) combination is only ever tried once per
session: `lsp--autostart-tried' (below) permanently remembers a give-up
(missing binary, or `lsp-autostart-timeout' seconds with no answer),
and nothing here retries it later. A user who installs the missing
server mid-session, or fixes whatever was wrong, runs `M-x lsp'
directly -- unaffected by any of this.")

(defvar lsp-autostart-timeout 10
  "Seconds `lsp--autostart-tick' gives a pending autostart handshake
before reaping it (`lsp-kill'-ing the connection and recording the
attempt in `lsp--autostart-tried' so it's never retried this session).

This is a DEADLINE FOR REAPING A PENDING ENTRY ON A LATER IDLE TICK,
not a blocking wait -- do not confuse it with `lsp-initialize-timeout',
which bounds `lsp--await''s synchronous wait inside `lsp-connect'.
Nothing in the autostart path ever calls `lsp--await' or blocks on
anything: this variable only controls how many seconds' worth of idle
ticks a dead-air server gets to answer before this side gives up and
moves on, with the editor fully responsive the entire time either way.

10 is a generous upper bound for the same reason `lsp-initialize-
timeout''s docstring gives: real `initialize' round trips measured on
this machine are single-digit milliseconds to low tens of milliseconds
(verible-verilog-ls, slang-server, rust-analyzer alike) -- this is
headroom for a slow machine or a server doing real startup work, not a
value meant to be tuned for normal use.")

(defvar lsp--autostart-pending nil
  "Alist of ((COMMAND . ROOT) . (CONN CLIENT DEADLINE)): one entry per
in-flight autostart handshake, keyed EXACTLY like `lsp--connections' --
by server command and project root, not by buffer -- so that two
buffers in the same project, seen on two different idle ticks before
the first handshake completes, discover the SAME pending entry and
only ever cause one spawn between them. CONN is the raw connection
(for `lsp-live-p'/`lsp-kill'), CLIENT is the half-initialized
`lsp--client' already sitting in `lsp--clients' so the ordinary idle
pump drains its replies, and DEADLINE is an absolute `float-time'
deadline (`lsp-autostart-timeout' seconds out from when the spawn was
started) for `lsp--autostart-tick' to reap it by.")

(defvar lsp--autostart-tried nil
  "List of (COMMAND . ROOT) pairs autostart has already given up on this
session -- either the spawn itself failed (missing binary, ...) or the
handshake never got an answer within `lsp-autostart-timeout' seconds.
Permanent for the session (M88 v1): nothing here ever retries. A user
who installs the server or otherwise fixes the problem mid-session runs
`M-x lsp' directly, which consults neither this list nor
`lsp--autostart-pending' at all.")

(defvar lsp--autostart-pending-here nil
  "Buffer-local (M88 D7): list of `(COMMAND . ROOT)' pending autostart
handshakes THIS buffer would be attached to once each completes, or
nil if none. Purely a MODE-LINE signal (`redisplay.rs' reads it with
the same buffer-local-aware `buffer_var_on' it already uses for
`lsp--buffer-client', truthy on any non-empty list -- no change needed
there for this to be a list rather than a single cons) -- nothing in
this file's attach/reuse logic ever consults it; only
`lsp--buffer-client'/`lsp--buffer-clients' (set by `lsp--attach-
current-buffer') means a buffer is actually attached.

M94 review AA1: was a SINGLE `(COMMAND . ROOT)' before M94 gave a
buffer two independently autostarting servers. `lsp--autostart-maybe-
begin' tries the primary and the secondary in the SAME idle tick, and
nothing orders which of two independent subprocess `initialize' round
trips answers first -- with a single slot, marking the second
unconditionally overwrote the first's key, and if the SECOND handshake
happened to complete before the first, its completion closure cleared
the slot by `equal' match while the first was still in flight, leaving
the mode line showing neither Attached nor Pending. A list fixes both:
marking conses a new key on (if not already present) instead of
clobbering, and clearing removes just its own key
(`lsp--autostart-mark-pending', below), so each handshake's pending
window is independent and the slot is only ever fully empty when NONE
are in flight.

Grown for every matching open buffer by `lsp--autostart-begin' at
spawn time (`lsp--autostart-mark-pending', the same buffer-list scan
`lsp--auto-attach-backfill' will independently do once the handshake
completes), and shrunk back -- by the SAME function, called with
MARKED nil -- from both ways a pending entry can end: the completion
closure in `lsp--autostart-begin', and `lsp--autostart-reap-one'.")

(defvar lsp--frontend-started nil
  "Set once by `core::frontend_started' (Rust) after the frontend has
drawn/painted at least one real frame -- `lsp--autostart-tick' returns
nil until this is non-nil, so nothing here can spawn a server before
there is an actual GUI/TUI event loop pumping the idle tick to drain
it. Exists specifically so a test harness that calls
`find-file-internal'/`eval-source' directly, never going through
`run_tui'/`run_gui' at all, can never trigger an autostart spawn no
matter how many times it might otherwise look like an idle tick ran.")

(defun lsp--autostart-buffer-matches-p (file buf-mode command root mode)
  "Non-nil if FILE (visited by a buffer in major BUF-MODE) would
actually be attached by `lsp--auto-attach-backfill' once the pending
handshake for COMMAND/ROOT/MODE completes -- the SAME exact-mode test
backfill itself applies (`(eq (major-mode-internal-get) mode)'), not
just \"any mode whose `lsp-server-alist' entry happens to point at the
same COMMAND\" (M88 F7 review fix).

Before this fix, this predicate matched on COMMAND alone: with
`c-mode' and `c++-mode' both mapped to `clangd' in the default
`lsp-server-alist', a `c++-mode' buffer in the same project as a
`c-mode' buffer that triggers autostart would get marked `LSP…' in the
mode line, but backfill -- which DOES require exact mode -- would never
actually attach it once the handshake completed: the indicator
promised something that never happened, then silently dropped back to
blank when the completion cleared every COMMAND-matching marker
regardless of whether backfill had touched it. Requiring `(eq buf-mode
mode)' here makes the marker never promise more than backfill will
actually deliver. (Verilog is unaffected either way -- `verilog-mode'
is the only mode mapped to `verible-verilog-ls' -- but the fix applies
generally, not just to Verilog.)

M94 review Z3: also checks `lsp-secondary-server-alist', not just
`lsp-server-alist' -- this predicate was never updated for M94's
addition of a second, independently autostarting server. Before this
fix, a SECONDARY's own handshake (COMMAND matching only the secondary
table) never matched here at all, so no buffer was ever marked
pending for it, anywhere, ever -- the mode line's `LSP…' indicator was
simply dead for the entire secondary autostart window, silently.
Mirrors `lsp--auto-attach-backfill-matches-p''s own two-table check
(added at the same review point, for the actual attach rather than
just this cosmetic marker)."
  (and file
       (not (lsp--remote-path-p file))
       (eq buf-mode mode)
       (or (equal (car (lsp--server-for-mode buf-mode)) command)
           (equal (car (lsp--secondary-server-for-mode buf-mode)) command))
       (equal (lsp--project-root file) root)))

(defun lsp--autostart-mark-pending (command root marked &optional mode)
  "Add (MARKED non-nil) or remove (MARKED nil) `(COMMAND . ROOT)' in/from
`lsp--autostart-pending-here''s LIST, on every open buffer (M94 review
AA1: was a single-slot set/clear before a buffer could have two
independent handshakes pending at once -- see that variable's own
docstring for why a list is what fixes it).

Adding (MARKED non-nil, MODE required) scans for buffers
`lsp--autostart-buffer-matches-p' says backfill will actually attach
once COMMAND/ROOT's handshake completes, and conses `(COMMAND . ROOT)'
onto each one's list, unless it's already there.

Removing (MARKED nil, MODE ignored -- M88 F5 review fix) does NOT
re-run that match test: it instead removes `(COMMAND . ROOT)' from any
buffer whose CURRENT `lsp--autostart-pending-here' list contains it
(`equal'), regardless of whether the buffer would still match today. A
buffer's major mode or visited file can change during the handshake
window (`M-x' into a different mode, a rename), and re-deriving \"does
this buffer match\" at removal time would leave a buffer that no
longer matches stuck showing that key in the mode line PERMANENTLY --
nothing else ever touches this variable once it's set, so a removal
that silently skips it is a removal that never happens.

Shared by `lsp--autostart-begin' (add, right after recording the
pending entry, and remove from its completion closure), `lsp--autostart-
cancel-pending' (remove, M88 F1), and `lsp--autostart-reap-one'
(remove). Each buffer's check+set is independently `condition-case'-
wrapped, same discipline as `lsp--auto-attach-backfill' -- one buffer's
failure can't stop the rest of the sweep.

CAUTION (G5 review, fix round 3): MODE is `&optional' only because
elisp requires every clearing call site (which never needs it) to be
able to omit it -- it is NOT optional in the sense of \"safe to leave
out.\" Passing MARKED non-nil with MODE nil (or omitted) is silently
wrong, not signaled: `lsp--autostart-buffer-matches-p''s `(eq buf-mode
mode)' check compares every buffer's major mode against nil, which is
never `eq' to any real major-mode symbol, so NO buffer is ever marked
-- the only symptom is a mode line that never shows `LSP…' for that
handshake, which no test here would notice, since every existing
marking call site (`lsp--autostart-begin', the only one there is)
already supplies a real MODE and always will unless a future call site
gets this wrong. Any future MARKED-non-nil call site MUST supply the
connecting buffer's actual major mode."
  (let ((key (cons command root)))
    (dolist (buf (buffer-list))
      (condition-case nil
          (with-current-buffer buf
            (if marked
                (when (and (lsp--autostart-buffer-matches-p
                            (buffer-file-name) (major-mode-internal-get)
                            command root mode)
                           (not (member key lsp--autostart-pending-here)))
                  (setq-local lsp--autostart-pending-here
                              (cons key lsp--autostart-pending-here)))
              (when (member key lsp--autostart-pending-here)
                (setq-local lsp--autostart-pending-here
                            (delete key lsp--autostart-pending-here)))))
        (error nil)))))

(defun lsp--autostart-cancel-pending (command root)
  "If a pending autostart handshake exists for `(COMMAND . ROOT)',
cancel it: `lsp-kill' its connection, drop the `lsp--autostart-pending'
entry, and clear `lsp--autostart-pending-here' on every buffer it had
marked (M88 F1 review fix).

Called by `lsp' right before it decides whether to reuse or spawn a
connection for the same key, so an explicit `M-x lsp' always wins a
race against an in-flight autostart instead of leaving it to complete
later and silently shadow the user's own connection with a leaked,
unreachable second server process -- before this fix, `lsp' consulted
only `lsp--get-connection'/`lsp--connections' and never looked at
`lsp--autostart-pending' at all, so it would spawn a SECOND server
through the fully synchronous `lsp-connect' while the async one was
still in flight; when the async one later completed it unconditionally
pushed itself onto `lsp--connections', silently shadowing the manual
connection from every future `assoc' lookup while the manual one's
process leaked for the rest of the session.

Nothing is attached to a pending autostart yet (I2), so cancelling one
here loses no state -- the buffer `lsp' is about to attach belongs to
IT now, not to whatever the autostart would eventually have backfilled.

Deliberately NOT added to `lsp--autostart-tried': this is a preemption,
not a give-up. `lsp' is about to establish `lsp--connections' for this
exact key itself, so `lsp--autostart-maybe-begin''s own `lsp--get-
connection' guard already prevents any future autostart attempt while
that connection lives; if it later dies, a future autostart SHOULD be
free to try again, which a permanent `lsp--autostart-tried' entry would
have wrongly blocked.

The completion closure inside `lsp--autostart-begin' also checks, when
it fires, whether its own pending entry has disappeared and whether
`lsp--get-connection' already answers for this key -- but G1 review
(fix round 2) found the FIRST of those two checks (entry gone) is
actually unreachable given this function's own behavior: this function
always `lsp-kill's CONN in the SAME call that removes the pending
entry, and `lsp-process-pending-all' checks a connection's liveness
BEFORE draining it, so a connection this function just killed is
dropped from `lsp--clients' on the very next pump pass without ever
being polled again -- there is no window left in which the entry is
gone but the closure still gets to run. See that check's own comment,
inside `lsp--autostart-begin', for the full explanation; it is kept as
defense-in-depth against that prune-before-drain ordering changing, not
because this function can currently produce the race it originally
described. The SECOND check (`lsp--get-connection' already answering)
is the one doing real work here and elsewhere: it is what stops a
duplicate connection if some future caller ever removes a pending entry
WITHOUT also killing its connection in the same step, unlike this
function and `lsp--autostart-reap-one', which both currently do."
  (let ((entry (assoc (cons command root) lsp--autostart-pending)))
    (when entry
      (setq lsp--autostart-pending (delq entry lsp--autostart-pending))
      (lsp--autostart-mark-pending command root nil)
      (lsp-kill (nth 0 (cdr entry))))))

(defun lsp--autostart-begin (command args root mode)
  "M88: start COMMAND ARGS as an LSP server for project ROOT
asynchronously, for a buffer in major MODE. Never blocks -- the only
synchronous primitive on this path is `(lsp-start command args (and
(file-directory-p root) root))' itself (M99 added the third CWD
argument; see this call's own inline comment below), i.e.
`Command::spawn' returning as soon as fork/exec completes;
everything the server sends back afterward arrives via the existing
non-blocking `lsp-poll'-driven idle pump, exactly like every other
`lsp-request-async' caller in this file.

The whole body is wrapped in `condition-case': a spawn failure (missing
binary, permission denied, ...) is caught right here, recorded in
`lsp--autostart-tried' so it's never retried, and reported once via
`message' -- never left to propagate and disrupt the idle tick that
called this.

On success: the half-initialized CLIENT is pushed onto `lsp--clients'
BEFORE the `initialize' request is even sent, so the ordinary idle pump
(`lsp-process-pending-all') starts draining its replies immediately --
see this section's header comment for why that's safe despite
`lsp--buffer-client' not being set anywhere in this function (I2: no
buffer is attached until the completion closure below runs, so no
`didChange' can ever precede the `didOpen' backfill sends at
completion). A `lsp--autostart-pending' entry is recorded last, once
the request is actually in flight, keyed `(COMMAND . ROOT)' (I3) so a
second buffer in the same project finds this same entry instead of
spawning its own server.

The completion closure (M88 F1 review fix) has two defensive `cond'
branches ahead of the ordinary success path, guarding against a race
with `M-x lsp'/`lsp--autostart-cancel-pending' -- see their own
comments below for what each one actually guards against today (G1
review, fix round 2: the first of the two is currently unreachable
under this file's own prune-before-drain ordering, kept only as
defense-in-depth; the second is the one doing real work). On the
ordinary path it drops the pending entry, stashes `capabilities' on
CLIENT (mirroring `lsp-connect''s own M54 handling), sends
`initialized', registers CLIENT under `(COMMAND . ROOT)' in
`lsp--connections' so it's indistinguishable from a connection `M-x
lsp' itself made, reports success (naming ROOT explicitly -- D9: when
no project marker was found, `lsp--project-root' silently falls back to
the file's own directory, and a server given the wrong root can answer
every cross-file query with an empty result while still advertising
full capabilities, so the root actually used has to be visible rather
than assumed), and finally calls `lsp--auto-attach-backfill', which
attaches every already-open buffer (the CLI-opened one included) that
this connection now covers.

M99: ROOT is also passed to `lsp-start' as the server process's cwd,
guarded the same way and for the same reason as `lsp-connect' -- see
its doc comment. Only passed when `file-directory-p' confirms ROOT
exists, so an unusual ROOT can never turn a spawn that would otherwise
succeed into a failure."
  (condition-case err
      (let* ((conn (lsp-start command args
                              (and (file-directory-p root) root)))
             (client (make-lsp--client :conn conn :command command)))
        (setq lsp--clients (cons client lsp--clients))
        (lsp-request-async
         client "initialize"
         (let ((p (make-hash-table)))
           (puthash "processId" :null p)
           (puthash "rootUri" (lsp--path-to-uri root) p)
           (puthash "capabilities" (lsp--client-capabilities-payload) p)
           p)
         (lambda (result)
           (let ((entry (assoc (cons command root) lsp--autostart-pending)))
             (cond
              ;; G1 review (fix round 2): this branch is UNREACHABLE as
              ;; the code stands, and deliberately uncovered by any
              ;; test -- do not go hunting for a repro. Both places
              ;; that ever remove this entry (`lsp--autostart-cancel-
              ;; pending' and `lsp--autostart-reap-one') `lsp-kill' the
              ;; SAME connection in the SAME call that removes the
              ;; entry, and `lsp-process-pending-all' (the pump that
              ;; would have to deliver this callback) checks liveness
              ;; BEFORE draining -- once `lsp-kill' marks CONN dead,
              ;; the very next pass skips it and drops it from
              ;; `lsp--clients' before ever polling it again, so a
              ;; message already sitting in its internal channel is
              ;; never drained and this callback can never fire a
              ;; second time. Confirmed by mutation: removing this
              ;; branch entirely leaves every test in
              ;; `lsp_autostart_tests.rs' green. Kept anyway as
              ;; defense-in-depth against that prune-before-drain
              ;; ordering ever changing -- if draining is ever made to
              ;; run before the liveness check, this branch is what
              ;; stops a stale reply from reviving a connection that
              ;; was supposed to be dead, instead of silently letting
              ;; a NEW `(not entry)' failure mode go unhandled.
              ((not entry) (lsp-kill conn))
              ;; A connection for this exact key already exists --
              ;; belt-and-suspenders against the same race from the
              ;; other side (our own pending entry technically still
              ;; here, but someone else's connection beat us to
              ;; `lsp--connections' anyway). Never push a duplicate.
              ((lsp--get-connection command root)
               (setq lsp--autostart-pending (delq entry lsp--autostart-pending))
               (lsp--autostart-mark-pending command root nil)
               (lsp-kill conn))
              (t
               (setq lsp--autostart-pending
                     (delq entry lsp--autostart-pending))
               (setf (lsp--client-capabilities client)
                     (and (hash-table-p result) (gethash "capabilities" result)))
               (lsp--notify client "initialized" (make-hash-table))
               (push (cons (cons command root) client) lsp--connections)
               (lsp--autostart-mark-pending command root nil)
               (message "LSP: autostarted %s for %s" command root)
               (lsp--auto-attach-backfill mode client))))))
        (push (cons (cons command root)
                    (list conn client (+ (float-time) lsp-autostart-timeout)))
              lsp--autostart-pending)
        (lsp--autostart-mark-pending command root t mode))
    (error
     (push (cons command root) lsp--autostart-tried)
     (message "LSP autostart: failed to start %s: %s"
              command (lsp--error-string err)))))

(defun lsp--autostart-reap-one (entry)
  "Give up on one `lsp--autostart-pending' ENTRY: `lsp-kill' its
connection, drop it from `lsp--autostart-pending', remember
`(COMMAND . ROOT)' in `lsp--autostart-tried' so it's never retried this
session, and `message' once."
  (let* ((key (car entry))
         (conn (nth 0 (cdr entry))))
    (setq lsp--autostart-pending (delq entry lsp--autostart-pending))
    (push key lsp--autostart-tried)
    (lsp-kill conn)
    (lsp--autostart-mark-pending (car key) (cdr key) nil)
    (message "LSP autostart: %s gave up waiting for %s"
             (car key) (cdr key))))

(defun lsp--autostart-try-one (entry root mode)
  "If ENTRY -- a (COMMAND . ARGS) pair, possibly nil -- is non-nil and
the current buffer has no LIVE client already attached for its COMMAND
(`lsp--buffer-has-live-client-for-command-p', M94), and there is no
live connection, pending handshake, or prior give-up for `(COMMAND .
ROOT)', begin an autostart attempt for it. A silent no-op for ENTRY
nil (MODE has no `lsp-server-alist'/`lsp-secondary-server-alist' entry
at all).

Shared by `lsp--autostart-maybe-begin' for BOTH `lsp-server-alist' and
`lsp-secondary-server-alist' entries (M94) -- the two calls are
independent, so a live primary never blocks a secondary from
autostarting, and vice versa."
  (when entry
    (let* ((command (car entry))
           (args (cdr entry))
           (key (cons command root)))
      (when (and (not (lsp--buffer-has-live-client-for-command-p command))
                 (not (lsp--get-connection command root))
                 (not (assoc key lsp--autostart-pending))
                 (not (member key lsp--autostart-tried)))
        (lsp--autostart-begin command args root mode)))))

(defun lsp--autostart-maybe-begin ()
  "If the current buffer qualifies for autostart -- `lsp-autostart' and
`lsp-auto-attach' both non-nil, a file-visiting non-remote buffer with
no live `lsp--buffer-client' already, a mode with an `lsp-server-alist'
entry, no existing connection or pending handshake for `(COMMAND .
ROOT)', and that pair not already given up on -- start one via
`lsp--autostart-begin'. A silent no-op otherwise, same discipline as
`lsp--maybe-auto-attach'.

M93 fix round (R2): `(not (lsp--live-buffer-client))' is checked
BEFORE `lsp--project-root' is ever called, not after -- this function
runs from `lsp--autostart-tick' on every idle tick (every poll
timeout in the TUI, every frame in the GUI), and an already-attached
buffer is by far the common steady state once autostart has done its
job once. `lsp--live-buffer-client' takes no argument and touches
nothing but the current buffer's own local variable, so checking it
first costs nothing extra and skips `lsp--project-root''s ancestor-
directory filesystem walk entirely for that whole common case, instead
of computing ROOT just to throw it away. (The other three guards below
-- `lsp--get-connection', the pending-handshake `assoc', and the
given-up-on `member' -- all key on `(COMMAND . ROOT)' itself, so they
cannot be reordered ahead of computing ROOT the same way; a buffer
that autostart has already given up on still pays for one
`lsp--project-root' call per idle tick, mitigated instead by
`lsp--nearest-filelist-root-cache' -- see that variable's own
docstring.)

Reviewer-confirmed, M93 second fix round: this reorder is a pure
boolean-AND reordering -- `lsp--live-buffer-client' is called exactly
once either way, and its buffer-local-clearing side effect (see its
own docstring) is identical regardless of where in the `and' it sits.
No functional test in this suite can distinguish the old ordering from
this one: both produce the exact same decision (start / don't start)
for every input, so nothing here changes what autostart DOES, only how
much filesystem work it does to decide. The only observable effect is
speed, measured by hand in the M93 fix-round report (already-attached
case: 212.59us -> 5.82us per call, `cargo test -p core --test
lsp_mode_tests', a since-removed scratch benchmark) -- a green test
run here certifies correctness, not the improvement.

M94 addendum: `(not (lsp--live-buffer-client))' is no longer allowed to
skip this function ENTIRELY once a SECONDARY is registered for MODE
(`lsp-secondary-server-alist') -- that was the exact silent-sink bug
this milestone fixes: the instant the primary attaches, the fast path
above would otherwise short-circuit forever, and a secondary could
never autostart for that buffer, for the whole session, with no error
and no message. The fast path is still taken, and still skips
`lsp--project-root' entirely, for the common case this M93 note
describes -- a mode with NO secondary registered, i.e. every mode
except `verilog-mode' by default -- since `lsp--secondary-server-for-
mode' is a plain `assq', cheaper even than the buffer-local read this
docstring already justifies skipping ahead of. Each of PRIMARY and
SECONDARY is then tried independently via `lsp--autostart-try-one',
whose own per-command \"already attached\" guard
(`lsp--buffer-has-live-client-for-command-p') is what actually lets
one attaching not block the other."
  (when (and lsp-autostart lsp-auto-attach)
    (let ((file (buffer-file-name))
          (mode (major-mode-internal-get)))
      (when (and file (not (lsp--remote-path-p file)))
        (let ((secondary (lsp--secondary-server-for-mode mode)))
          (when (or secondary (not (lsp--live-buffer-client)))
            (let ((primary (lsp--server-for-mode mode)))
              (when (or primary secondary)
                (let ((root (lsp--project-root file)))
                  (lsp--autostart-try-one primary root mode)
                  (lsp--autostart-try-one secondary root mode))))))))))

(defun lsp--autostart-tick ()
  "The idle-tick step (M88) for automatic LSP startup: called once per
frontend frame/poll from the Rust side (`core::idle_tick'), alongside
the other pumps. Returns nil, doing nothing at all, until
`lsp--frontend-started' is set (see its own doc).

First reaps any `lsp--autostart-pending' entry whose connection has
died or whose `lsp-autostart-timeout' deadline has passed
(`lsp--autostart-reap-one'), THEN considers starting a new one for the
current buffer (`lsp--autostart-maybe-begin'). Reaping first matters
for the same reason `lsp-process-pending-all' already runs before this
in `core::idle_tick': a reply that arrived this same tick is dispatched
by that earlier pump before this function ever runs, so a handshake
that just barely made its deadline is never mistakenly reaped out from
under a completion that already fired."
  (when lsp--frontend-started
    (dolist (entry (copy-sequence lsp--autostart-pending))
      (let* ((conn (nth 0 (cdr entry)))
             (deadline (nth 2 (cdr entry))))
        (when (or (not (lsp-live-p conn)) (> (float-time) deadline))
          (lsp--autostart-reap-one entry))))
    (lsp--autostart-maybe-begin))
  nil)

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
in the TUI. Typing is never blocked waiting for the answer.

M94: routes via `lsp--capable-client' (\"hoverProvider\") instead of
reading `lsp--buffer-client' directly, so a buffer with a second
attached client that supports hover -- and the primary either doesn't,
or also does (`hoverProvider' was never gated before M94 and still
isn't; a client whose capabilities are unknown, or that lists the key
at all regardless of its value, counts as supporting it -- see
`lsp--capability-supported-p') -- can get an answer from whichever one
matches first. A buffer with only one attached client (the common
case, and every case before this milestone) behaves identically:
`lsp--capable-client' falls back to exactly `lsp--buffer-client' via
`lsp--effective-buffer-clients'."
  (interactive)
  (let ((client (lsp--capable-client "hoverProvider")))
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
  ;; Recorded, not fixed (trailing cold review, M123 fix round): a
  ;; spec-violating item with no `label' at all makes this nil, which
  ;; is then used as the popup LABEL and the empty-text FALLBACK --
  ;; that one candidate silently vanishes from the popup rather than
  ;; ever inserting anything, LSP protocol violation notwithstanding.
  (gethash "label" item))

(defun lsp--completion-item-insert-text (item)
  "ITEM's RAW insert text (M44-3): `textEdit.newText' when ITEM carries a
`textEdit', else `insertText', else `label'. `additionalTextEdits' are
never applied (v1, documented in the file header). This is the text
BEFORE any snippet expansion -- callers that need the FINAL text a user
should see inserted want `lsp--completion-item-expanded-insert-text'
(M123) instead, which is this function plus `lsp--expand-snippet' when
ITEM's `insertTextFormat' says it needs it."
  (let* ((edit (gethash "textEdit" item))
         (new-text (and (hash-table-p edit) (gethash "newText" edit))))
    (or new-text (gethash "insertText" item) (gethash "label" item))))

;; --- M123 Part B: snippet expansion --------------------------------

(defun lsp--snippet-digit-p (c)
  (and (>= c ?0) (<= c ?9)))

(defun lsp--snippet-index (s c)
  "First index of character C in string S, or nil if absent -- this
elisp subset has no `string-search'/`cl-position', so `lsp--expand-
snippet''s own helpers get this hand-rolled linear scan instead."
  (let ((len (length s)) (i 0) found)
    (while (and (< i len) (not found))
      (when (eq (aref s i) c) (setq found i))
      (setq i (1+ i)))
    found))

(defun lsp--snippet-find-close-brace (s start)
  "Index (into S) of the `}' matching the `{' whose contents begin at
START -- START is the position right after the OPENING `{'. Tracks
nested `{'/`}' pairs so a DEFAULT/CHOICES text that itself contains
balanced braces doesn't close the construct early; returns nil if S
ends before a matching `}' is found (an unterminated construct --
`lsp--expand-snippet-tabstop' treats that as malformed, falling back to
literal text for its caller).

M123 fix round (trailing cold review): a `\\{' or `\\}' inside
DEFAULT/CHOICES text used to be counted as an ordinary, unescaped
brace -- wrong, since the whole point of that escape is to let a
literal `}' (or `{') appear without closing (or opening) a nesting
level it was never meant to. `${1:a\\}b}' used to compute `a\\}' with a
stray `b}' left over. Fixed the same way `lsp--expand-snippet' already
treats a top-level `\\$': a backslash consumes (skips, without
inspecting for brace-counting purposes) exactly the ONE character
after it, so `\\{'/`\\}' can never be mistaken for a real nesting
brace. Correct extraction alone is not the whole fix -- see
`lsp--expand-snippet-braced''s own docstring for the matching
unescape pass this enables on the DEFAULT/CHOICES text it returns."
  (let ((len (length s)) (i start) (depth 1) found)
    (while (and (< i len) (not found))
      (let ((c (aref s i)))
        (cond
         ((and (eq c ?\\) (< (1+ i) len))
          (setq i (1+ i)))
         ((eq c ?{) (setq depth (1+ depth)))
         ((eq c ?})
          (setq depth (1- depth))
          (when (= depth 0) (setq found i)))))
      (setq i (1+ i)))
    found))

(defun lsp--snippet-unescape-inner (s)
  "S with `\\$', `\\\\', `\\}' resolved to their own literal character --
applied to the DEFAULT/CHOICES text `lsp--expand-snippet-braced'
extracts, now that `lsp--snippet-find-close-brace' correctly SKIPS an
escaped brace when finding where that text ends but does not itself
strip the escaping backslash back out of the substring it returns.
Without this, `${1:a\\}b}' would extract the right span (`a\\}b') but
still render the escaping backslash literally, `a\\}b', not the
intended `a}b'.

Deliberately UNRELATED to \"nested placeholders are not recursively
expanded\" (`lsp--expand-snippet''s own header): a backslash escape is
never a tab stop, and always resolves to plain text; a nested
`${2:x}' is a structurally different construct (an unescaped `$'
followed by digits) that stays verbatim on purpose, and this function
does not touch it -- `${1:${2:x}}' still comes out with the inner
`${2:x}' untouched, since no `\\' precedes it anywhere."
  (let ((out "") (i 0) (len (length s)))
    (while (< i len)
      (let ((c (aref s i)))
        (if (and (eq c ?\\) (< (1+ i) len)
                 (memq (aref s (1+ i)) '(?$ ?\\ ?})))
            (progn
              (setq out (concat out (char-to-string (aref s (1+ i)))))
              (setq i (+ i 2)))
          (progn
            (setq out (concat out (char-to-string c)))
            (setq i (1+ i))))))
    out))

(defun lsp--expand-snippet-braced (inner next)
  "Parse INNER -- the text strictly between a tab stop's `${' and its
matching `}' (already located by `lsp--snippet-find-close-brace') --
into (KIND TEXT NEXT): INNER is one of `N', `N:default', or
`N|a,b,c|'. KIND is `final' for tab stop 0 (`$0'/`${0}', the one
cursor position this client actually surfaces), `plain' for any other
N. Deliberately does NOT recursively expand DEFAULT/CHOICES text --
see `lsp--expand-snippet''s own docstring for why nested placeholders
are left as literal text rather than expanded again.

M123 fix round: which of the three shapes INNER is used to be decided
by asking \"does `:' or `|' occur EARLIER in the whole string\" --
wrong, because DEFAULT/CHOICES text is free-form and can legally
contain either character as part of its own content (a Verilog bit
range like `${1|[7:0],[15:0]|}' has a `:' inside the choice list
itself, well after the `|' that actually introduces it). The LSP/
TextMate grammar disambiguates positionally, not by \"whichever
delimiter appears first\": right after N's own digits comes either `|'
(choice list) or `:' (default) or neither (bare `${N}'), and that
SINGLE character is the only one asked about here -- whatever `:' or
`|' shows up later, inside the default/choices text itself, is already
past that decision point and is therefore correctly left as literal
content.

M123 fix round (trailing cold review): DEFAULT (the `:' branch) and
FIRST-CHOICE (the `|' branch) are both passed through
`lsp--snippet-unescape-inner' before being returned -- see that
function's own docstring for why this is NOT the same thing as
recursively expanding a nested tab stop (which this function still
never does): `\\}'/`\\$'/`\\\\' inside either one are literal escapes,
never tab-stop syntax, and must always resolve to plain text for the
construct's own closing `}' (now correctly found even past an escaped
one, see `lsp--snippet-find-close-brace') to have been worth finding
correctly in the first place."
  (let ((i 0) (len (length inner)))
    (while (and (< i len) (lsp--snippet-digit-p (aref inner i)))
      (setq i (1+ i)))
    (let ((n (string-to-number (substring inner 0 i)))
          (kind-of (lambda (n) (if (= n 0) 'final 'plain))))
      (cond
       ((and (< i len) (eq (aref inner i) ?|))
        ;; `N|a,b,c|' -- INNER excludes the OUTER `}' but still includes
        ;; the trailing `|' before it, since the brace scan that found
        ;; INNER only tracks `{'/`}' nesting, not `|' pairs. Strip that
        ;; trailing `|' to get the comma-separated CHOICES list, THEN
        ;; split on the FIRST comma to get the first choice -- the two
        ;; `|' characters are only the delimiters around the whole
        ;; list, not per-choice separators (that's what the comma is
        ;; for); any `:' inside CHOICES (a bit range, say) is never
        ;; consulted again past this point.
        (let* ((rest (substring inner (1+ i)))
               (close-pipe (lsp--snippet-index rest ?|))
               (choices (if close-pipe (substring rest 0 close-pipe) rest))
               (comma (lsp--snippet-index choices ?,))
               (first-choice (if comma (substring choices 0 comma) choices)))
          (list (funcall kind-of n) (lsp--snippet-unescape-inner first-choice) next)))
       ((and (< i len) (eq (aref inner i) ?:))
        (list (funcall kind-of n)
              (lsp--snippet-unescape-inner (substring inner (1+ i)))
              next))
       (t
        (list (funcall kind-of n) "" next))))))

(defun lsp--expand-snippet-tabstop (snippet pos)
  "Helper for `lsp--expand-snippet': parses the tab-stop construct
starting at POS (SNIPPET's own index right after the `$' introducing
it). Returns (KIND TEXT NEXT) -- see `lsp--expand-snippet-braced' for
what KIND/TEXT mean -- or nil if POS doesn't actually start a
well-formed tab stop (a bare `$' with nothing recognizable after it,
or an unterminated `${'), which the caller falls back to treating as
a literal `$'."
  (let ((len (length snippet)))
    (cond
     ;; `$N' -- bare digits, no braces.
     ((and (< pos len) (lsp--snippet-digit-p (aref snippet pos)))
      (let ((j pos))
        (while (and (< j len) (lsp--snippet-digit-p (aref snippet j)))
          (setq j (1+ j)))
        (let ((n (string-to-number (substring snippet pos j))))
          (list (if (= n 0) 'final 'plain) "" j))))
     ;; `${...}'
     ((and (< pos len) (eq (aref snippet pos) ?{))
      (let ((close (lsp--snippet-find-close-brace snippet (1+ pos))))
        (and close
             (lsp--expand-snippet-braced (substring snippet (1+ pos) close)
                                          (1+ close)))))
     (t nil))))

(defun lsp--expand-snippet (snippet)
  "Expand LSP/TextMate snippet syntax in SNIPPET (`insertTextFormat' 2,
see `lsp--completion-item-expanded-insert-text') into (TEXT . OFFSET),
OFFSET nil meaning \"point belongs at the end of TEXT\" -- a NAMED
cursor stop, `$0'/`${0}', sets it explicitly to where that stop landed
in the OUTPUT; with no `$0' anywhere in SNIPPET, point simply lands
after the last character inserted, same as an ordinary literal
completion always has.

Handles, per the LSP/TextMate snippet grammar's core subset (measured
against `slang-server''s own real replies, quoted in the M123 spec --
`sram_bank''s full instantiation template and `always_ff @($0) begin\\n
end'):
  `\\$', `\\\\', `\\}' -> a literal `$', `\\', `}' respectively -- the
                      three escapes the LSP/TextMate snippet grammar
                      actually defines (M123 fix round: this used to
                      handle `\\$' only, which meant `verilog-complete.
                      el''s own `verilog-complete--snippet-escape' --
                      whose whole job is to double a literal `\\' in
                      LITERAL text, such as a SystemVerilog escaped
                      identifier, so this expander can never misread it
                      as introducing an escape of its own -- had no
                      counterpart to undo that doubling: a `\\\\' it
                      emitted came back out as `\\\\' STILL DOUBLED,
                      not `\\'. Handling all three here is what makes
                      that escaper's own docstring claim (\"the exact
                      inverse of what this undoes\") actually true).
  `$N', `${N}'     -> removed entirely (an unnamed tab stop this
                      client has no multi-stop editing UI for, so
                      nothing is inserted for it -- the SAME removal
                      `$0' gets, just without setting OFFSET).
  `${N:default}'   -> DEFAULT's own text, inserted as-is.
  `${N|a,b,c|}'    -> the FIRST choice, `a', inserted as-is.
  `$0', `${0}'     -> removed, and OFFSET is recorded as the output
                      length at this point.
  `${0:default}'   -> DEFAULT's own text is inserted (same as the
                      ordinary `${N:default}' case above), and OFFSET
                      is recorded BEFORE that text, i.e. at its START,
                      not its end (M123 fix round: this combination --
                      a NAMED final stop that ALSO carries default
                      text -- was previously undocumented and unpinned;
                      the behavior itself was already exactly this, an
                      accident of `offset' being set ahead of `text'
                      being appended for every stop, `plain' or `final'
                      alike, with no special-casing for `final' ever
                      having non-empty text of its own -- decided here,
                      deliberately, to KEEP that placement: it matches
                      the TextMate/VS Code convention of a final stop's
                      default text arriving pre-selected, ready to be
                      typed over, which needs point to start BEFORE the
                      default text, not after it). A real reply that
                      exercises this shape: this client has no field-
                      selection UI to actually highlight DEFAULT, so
                      the visible effect is only that point lands at
                      the start of the inserted default rather than at
                      its end -- still a deliberate, useful landing
                      spot (immediately ready to delete-forward and
                      retype), not a bug.
  anything malformed, or missing its closing `}' (an unterminated
  `${') -> passed through as a literal `$' and scanning resumes right
  after it, rather than raising an error or losing the rest of the
  snippet -- a construct too weird for this client to parse should
  degrade to \"looks a little odd\" over \"the completion vanishes\"
  or \"signals\".

Nested placeholders (`${1:${2:x}}') are NOT supported: DEFAULT/CHOICES
text is taken completely literally by `lsp--expand-snippet-braced', so
a nested `${2:x}' inside it is emitted VERBATIM (as the six characters
`${2:x}', not as `x') rather than expanded again -- documented here
per the M123 spec's own instruction to say plainly what happens
instead of pretending nesting is handled.

Pure and independently testable: a string in, a cons out, no buffer,
no server, no client."
  (let ((len (length snippet)) (i 0) (out "") offset)
    (while (< i len)
      (let ((c (aref snippet i)))
        (cond
         ((and (eq c ?\\) (< (1+ i) len)
               (memq (aref snippet (1+ i)) '(?$ ?\\ ?})))
          (setq out (concat out (char-to-string (aref snippet (1+ i)))))
          (setq i (+ i 2)))
         ((eq c ?$)
          (let ((res (lsp--expand-snippet-tabstop snippet (1+ i))))
            (if (not res)
                (progn (setq out (concat out "$")) (setq i (1+ i)))
              (let ((kind (nth 0 res)) (text (nth 1 res)) (next (nth 2 res)))
                (when (eq kind 'final)
                  (setq offset (length out)))
                (setq out (concat out text))
                (setq i next)))))
         (t
          (setq out (concat out (char-to-string c)))
          (setq i (1+ i))))))
    (cons out offset)))

(defun lsp--completion-item-expanded-insert-text (item)
  "ITEM's insert TEXT and cursor OFFSET as (TEXT . OFFSET), expanding
snippet syntax (`lsp--expand-snippet') when ITEM's `insertTextFormat'
is 2, otherwise identical to the old M44-3 behaviour -- ITEM's raw
`lsp--completion-item-insert-text', OFFSET nil (point at the end)."
  (let ((text (lsp--completion-item-insert-text item)))
    (if (eq (gethash "insertTextFormat" item) 2)
        (lsp--expand-snippet text)
      (cons text nil))))

(defvar lsp--completion-registry nil
  "Vector of raw CompletionItem hash-tables for the currently open
completion popup (M123) -- index N here is exactly the index a
`\"resolve:N\"' `PopupItem' payload (`lsp--completion-item-insert-
payload', `editor.rs''s `PopupItem::payload') names for the item at
that position, so `lsp--resolve-and-render-completion' can look the raw
item back up at ACCEPT time (only then, not at popup-open time, is a
single item singled out for the extra `completionItem/resolve' round
trip -- see the M123 spec's own reasoning for why the whole list is
never eagerly resolved). Rebuilt wholesale every time `lsp-completion-
at-point' gets a fresh answer; a payload from an already-closed popup
can't be accepted (there is nothing left to press RET on), so there is
no staleness window to defend against beyond `lsp--resolve-and-render-
completion''s own out-of-range bounds check.")

(defvar lsp--completion-registry-client nil
  "The CLIENT `lsp--completion-registry''s items came from -- resolve
must ask the SAME server that offered them, never whichever client
happens to be attached to the buffer by the time RET is pressed.")

(defun lsp--resolve-provider-p (client)
  "Non-nil only when CLIENT is a real `lsp--client' struct whose
advertised `completionProvider.resolveProvider' is truthy (measured
true for `slang-server', 2026-09-08).

Deliberately the OPPOSITE default from `lsp--capability-supported-p'
(which trusts an unknown capability, per its own M46/M54 finding):
`completionItem/resolve' is an EXTRA network round trip this client
never sent before M123, not a request whose absence merely degrades
gracefully to no answer -- guessing \"supported\" for an unknown or
fake client would fire a real request through every test using the
`'fake-client' convention (`lsp--live-buffer-client' tolerates a plain
symbol standing in for a client -- see its own doc comment) or through
a server that never advertised it, neither of which this milestone
intends. So: CLIENT not `lsp--client-p', capabilities unknown,
`completionProvider' absent or not a hash table, or `resolveProvider'
itself falsy (absent, nil, or `:false') all answer nil; only a
genuinely truthy `resolveProvider' opts in.

Known tradeoff, noted here rather than left implicit (fix-round cold
review): opting in makes ACCEPTING any completion from a resolve-
capable server a SYNCHRONOUS round trip (`lsp--resolve-and-render-
completion' uses `lsp--request'+`lsp--await', not `lsp-request-async',
per that function's own docstring) that blocks this single-threaded
editor for its duration -- even for a candidate whose LIST entry
already carried everything needed to insert it, where resolving learns
nothing new. This client has no cheap way to tell that case apart from
one that genuinely needs the round trip (a server may put fields
ONLY in the resolve reply, never the list, entirely at its own
discretion), so the tradeoff is accepted wholesale rather than
half-guessed at per item. Deliberate for the one server this was
measured against: `slang-server' omits `insertText'/`insertTextFormat'
from every list item and only fills them in on resolve, so for THAT
server the round trip is never wasted -- it is always the only way to
get real insert text at all, not an optional nicety."
  (and (lsp--client-p client)
       (let ((caps (lsp--client-capabilities client)))
         (and (hash-table-p caps)
              (let ((cp (gethash "completionProvider" caps)))
                (and (hash-table-p cp)
                     (let ((rp (gethash "resolveProvider" cp)))
                       (and rp (not (eq rp :false))))))))))

(defun lsp--completion-item-insert-payload (item idx client fallback)
  "Returns (INSERT . PAYLOAD) for one candidate -- INSERT is the STRING
`lsp--completion-items' stores as the candidate's `insert' field
(always literal, final, or fallback text -- see below); PAYLOAD is nil
(this candidate needs no special accept-time handling: exactly the
pre-M123 shape, byte-identical) or a string `PopupItem::payload' can
carry as-is (M123 review round: this used to be a Private-Use-Area-
prefixed convention folded into the single INSERT string;
`PopupItem::payload' is now its own field for the reasons given on its
doc comment -- overloading `insert' meant a hidden convention only one
comment in this file knew about, and no structural distinction between
an LSP item and a `verilog-complete.el' item). FALLBACK (M123 fix
round; the caller passes ITEM's own LABEL) is what INSERT becomes if
the computed text would otherwise be empty -- see below.

`\"resolve:N\"' (N a decimal index into `lsp--completion-registry')
when CLIENT's `completionProvider.resolveProvider' is truthy
(`lsp--resolve-provider-p') -- because a resolve-capable server may put
`insertText'/`insertTextFormat' ONLY in the resolve reply (measured
against `slang-server': the list item carries neither), so accept-time
resolve is the only way to even tell such an item apart from a
plain-text one; INSERT is the FALLBACK to use if that resolve fails
(`lsp--resolve-and-render-completion' has its own, later empty-text
guard for the resolved reply itself, via `lsp--completion-item-render').

Otherwise, INSERT is ITEM's own already-expanded text
(`lsp--completion-item-expanded-insert-text', which expands inline when
ITEM already carries `insertTextFormat' 2 without needing any resolve
round trip -- a hypothetical OTHER server that puts snippet syntax
directly in the list -- and is identical to the old M44-3 literal text
otherwise), and PAYLOAD is `\"offset:N\"' (N the cursor offset) when
that expansion produced a non-end OFFSET (a `$0' in the snippet), or
nil when it didn't -- an ordinary literal completion has PAYLOAD nil
and INSERT the plain text, exactly the pre-M123 shape.

M123 fix round: a snippet like a bare `\"$1\"' (an unnamed tab stop
with no default text) is non-empty RAW (`insertText' length 2) but
expands to the EMPTY STRING once `lsp--expand-snippet' strips the
placeholder -- accepting such a candidate used to delete the typed
prefix and insert nothing at all, silently. The guard checking for
emptiness has to run AFTER expansion, on TEXT, not on the raw
`insertText' the old code never re-examined post-expansion; on empty,
INSERT falls back to FALLBACK and PAYLOAD is forced to nil (an offset
computed against the now-discarded expanded text would point into text
that no longer exists)."
  (if (lsp--resolve-provider-p client)
      (cons (lsp--completion-item-insert-text item)
            (format "resolve:%d" idx))
    (let* ((expanded (lsp--completion-item-expanded-insert-text item))
           (text (car expanded))
           (offset (cdr expanded)))
      (cond
       ((not (and (stringp text) (> (length text) 0)))
        (cons fallback nil))
       (offset (cons text (format "offset:%d" offset)))
       (t (cons text nil))))))

(defun lsp--completion-item-render (item fallback)
  "Render ITEM (already resolved, or the original list item when a
server has no `completionItem/resolve' capability at all) into (TEXT .
OFFSET) -- OFFSET nil meaning \"end of TEXT\". Falls back to (FALLBACK
. nil) if ITEM's own computed text is empty (defense against a
genuinely empty resolve reply -- a completion must never simply
vanish).

M123 fix round: that emptiness check used to run on ITEM's RAW
`insertText' -- BEFORE `lsp--expand-snippet' ever ran -- so a raw
`insertText' of `\"$1\"' (a bare, unnamed tab stop with no default
text: non-empty, length 2) sailed straight through the guard and was
then expanded down to the EMPTY STRING, which the caller happily
returned as the item to insert: accepting it deleted the typed prefix
and inserted nothing, with no error and no visible sign anything went
wrong. The check must run on the EXPANDED text (what will actually be
inserted), not the pre-expansion one."
  (let* ((text (lsp--completion-item-insert-text item))
         (expanded (if (and (stringp text) (eq (gethash "insertTextFormat" item) 2))
                       (lsp--expand-snippet text)
                     (cons text nil)))
         (final-text (car expanded)))
    (if (not (and (stringp final-text) (> (length final-text) 0)))
        (cons fallback nil)
      expanded)))

(defun lsp--resolve-and-render-completion (idx fallback)
  "Accept-time resolve (M123): called from `commands::accept_completion'
(Rust, via `apply_function') when the popup item just accepted carries
a `PopupItem::payload' of `\"resolve:IDX\"' -- Rust passes IDX and the
item's own `insert' string (FALLBACK) straight through. Sends
`completionItem/resolve'
SYNCHRONOUSLY (`lsp--request' + `lsp--await', the same discipline
`lsp-hover-at-point' uses, NOT `lsp-request-async': accepting a
candidate is a direct user keystroke with no idle tick to deliver an
async callback into before that keystroke's own handling returns) for
`lsp--completion-registry''s IDXth raw item, and renders the RESOLVED
item's own fields in preference to the list item's -- per the LSP
spec, a resolve reply is a copy of the same CompletionItem with more
fields filled in, so it supersedes the list item wholesale rather than
merging field-by-field.

Never signals, and never loses the completion: IDX out of range for
`lsp--completion-registry' (a marker that somehow outlived its popup --
not reachable today, defended anyway), no live
`lsp--completion-registry-client', or `lsp--request'/`lsp--await'
erroring or timing out all fall back to rendering the UNRESOLVED list
item exactly as `lsp--completion-item-insert-payload' would have
without resolve capability at all; and if even that yields nothing
usable, (FALLBACK . nil) -- the plain text `lsp--completion-items'
already had in hand before ever attempting resolve.

Returns (TEXT . OFFSET), OFFSET nil meaning \"point at the end of
TEXT\" -- exactly the shape `commands::accept_completion' decodes."
  (let* ((client lsp--completion-registry-client)
         (item (and (lsp--client-p client)
                    (vectorp lsp--completion-registry)
                    (>= idx 0)
                    (< idx (length lsp--completion-registry))
                    (aref lsp--completion-registry idx))))
    (if (not (hash-table-p item))
        (cons fallback nil)
      (let* ((resolved
              (condition-case nil
                  (let ((id (lsp--request client "completionItem/resolve" item)))
                    (lsp--await client id))
                (error nil)))
             (final-item (if (hash-table-p resolved) resolved item)))
        (lsp--completion-item-render final-item fallback)))))

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

(defun lsp--completion-items (items prefix-start point &optional client)
  "Build the (LABEL INSERT START FILTER) list `show-completion-popup'
wants from ITEMS (a vector of CompletionItem hash-tables -- the `car'
of what `lsp--completion-result-vector' returns), PREFIX-START (the
identifier-run start `lsp-completion-at-point' recorded when it sent
the request), and POINT (current point as of when this answer is being
processed -- may be later than the point at request time, since typing
continues while a request is in flight, M44-3).

CLIENT (M123, optional -- omitting it reproduces the pre-M123 literal-
insert-only behaviour exactly, which is what every caller besides
`lsp-completion-at-point' itself still wants) also sets
`lsp--completion-registry'/`lsp--completion-registry-client' to ITEMS
and CLIENT wholesale, keyed by each item's ORIGINAL index into ITEMS
(preserved through the filter/sort below in a (ORIGINAL-INDEX . ITEM)
pairing, since a `\"r\"'-kind marker's index must survive both), so
`lsp--resolve-and-render-completion' can look a candidate's raw item
back up at accept time. Every surviving candidate's INSERT field is
built by `lsp--completion-item-insert-payload', which is where the
resolve-deferred/inline-expand decision actually happens.

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
  (when client
    (setq lsp--completion-registry (and (vectorp items) items))
    (setq lsp--completion-registry-client client))
  (let (out)
    (dotimes (idx (if items (length items) 0))
      (when (hash-table-p (aref items idx))
        (push (cons idx (aref items idx)) out)))
    (setq out (nreverse out))
    (setq out (sort out (lambda (a b)
                           (string< (lsp--completion-item-sort-key (cdr a))
                                    (lsp--completion-item-sort-key (cdr b))))))
    (let ((typed (buffer-substring-no-properties prefix-start point))
          result)
      (dolist (pair out)
        (let* ((orig-idx (car pair))
               (item (cdr pair))
               (start (lsp--completion-item-start item prefix-start))
               (filter (lsp--completion-item-filter-text item)))
          (when (and (<= start point)
                     (string-prefix-p typed filter))
            (let* ((label (lsp--completion-item-label item))
                   (ip (lsp--completion-item-insert-payload item orig-idx client label))
                   (insert (car ip))
                   (payload (cdr ip)))
              (push (if payload
                        (list label insert start filter payload)
                      (list label insert start filter))
                    result)))))
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
longer required, unlike M40-4's original three-way check.

M94: routes via `lsp--capable-client' (\"completionProvider\") instead
of reading `lsp--buffer-client' directly, so a buffer whose PRIMARY
doesn't advertise completion (verible, the Verilog default) but whose
SECONDARY does (slang, the Verilog default secondary) still gets an
answer. A buffer with only one attached client behaves identically:
`lsp--capable-client' falls back to exactly `lsp--buffer-client' via
`lsp--effective-buffer-clients', and the SAME `lsp--capability-
supported-p' gate this function already applied is what it checks
internally, so \"no such client\" degrades exactly as it always has."
  (interactive)
  (let ((client (lsp--capable-client "completionProvider")))
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
                    (items (lsp--completion-items (car parsed) prefix-start (point) client))
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
2. `lsp-completion-at-point', when this buffer has an attached LSP
   client whose own advertised capabilities support
   `textDocument/completion' (`lsp--capability-supported-p', M54) --
   see that function's docstring for the narrow, asymmetric rule this
   checks (an ABSENT `completionProvider' key blocks the request; a
   FALSY one, or capabilities never having been recorded at all, does
   not). M94: checked via `lsp--capable-client', which considers EVERY
   attached client (primary and secondary), not just the primary --
   Verilog's default primary (verible) has no `completionProvider' key
   at all, so without this a secondary that DOES advertise one (slang)
   would never be reachable through `C-M-i' at all.
3. `dabbrev-expand' otherwise -- so `C-M-i' still does something useful
   before `M-x lsp' has been run, in a buffer with no server registered
   for its major mode, or against a server (or servers) that don't
   support completion at all."
  (interactive)
  (cond
   ((and local-completion-function (funcall local-completion-function)))
   ((lsp--capable-client "completionProvider")
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
   waiting for the answer.

M95: step 2 routes via `lsp--preferred-role-client' (\"textDocument/
definition\", \"definitionProvider\") instead of reading
`lsp--live-buffer-client' directly, so a capable secondary answers this
in preference to the primary -- see that variable's own docstring for
why (verible returns no results at all for a name reached through
`import PKG::*'). A buffer with only one attached client occupying the PRIMARY slot
behaves identically to before -- see `lsp--preferred-role-client''s own
docstring for the one state that is NOT identical (a lone SECONDARY
sitting in an empty primary slot), which this now answers where it
previously refused."
  (interactive)
  (let ((live (lsp--preferred-role-client "textDocument/definition"
                                           "definitionProvider")))
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
which case this was from a nil return alone.

M99: DIAG ranges over `lsp--diagnostics-for-uri''s union, same as
`lsp--decorate-buffer' paints and `lsp--diagnostics-at-point' considers
-- previously this only walked `lsp--buffer-client''s own diagnostics,
so `next-diagnostic'/`previous-diagnostic' could fail to reach a
squiggle that was visibly drawn on screen from a secondary client."
  (let ((client (lsp--live-buffer-client))
        (file (buffer-file-name)))
    (when (and client file)
      (let ((out nil))
        (dolist (d (lsp--diagnostics-for-uri client (lsp--path-to-uri file)))
          (let* ((start (gethash "start" (gethash "range" d)))
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
same request).

M95: routes via `lsp--preferred-role-client' (\"textDocument/
documentHighlight\", \"documentHighlightProvider\") instead of reading
`lsp--live-buffer-client' directly, so a capable secondary answers this
in preference to the primary -- see that variable's own docstring for
why (verible false-positives, merging two unrelated identically-spelled
identifiers on a wildcard-imported name). A buffer with only one attached client occupying the PRIMARY slot
behaves identically to before -- see `lsp--preferred-role-client''s own
docstring for the one state that is NOT identical (a lone SECONDARY
sitting in an empty primary slot), which this now answers where it
previously refused."
  (interactive)
  (let ((client (lsp--preferred-role-client "textDocument/documentHighlight"
                                             "documentHighlightProvider")))
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
place.

M95: \"live client\" here means `lsp--preferred-role-client' for
`documentHighlightProvider' (same as `lsp-highlight-at-point' itself),
not the raw `lsp--live-buffer-client' -- a dead PRIMARY with a live,
capable SECONDARY still counts as connected for this purpose, exactly
as it does for the request path itself."
  (interactive)
  (lsp--clear-highlights)
  (when (lsp--preferred-role-client "textDocument/documentHighlight"
                                    "documentHighlightProvider")
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
`point'. Known, accepted gap -- see that variable's docstring.

M95: the liveness check is `lsp--preferred-role-client' for
`documentHighlightProvider', not the raw `lsp--live-buffer-client' --
same reasoning as `lsp-highlight-clear''s own M95 note, so a dead
primary with a live, capable secondary still fires this tick."
  (when (and lsp-idle-highlight-delay-ms
             (>= quiet-ms lsp-idle-highlight-delay-ms))
    (condition-case nil
        (when (and (lsp--preferred-role-client "textDocument/documentHighlight"
                                                "documentHighlightProvider")
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

Bound to `M-?' (see simple.el), alongside `M-.'/`M-,'.

M95: routes via `lsp--preferred-role-client' (\"textDocument/
references\", \"referencesProvider\") instead of reading
`lsp--live-buffer-client' directly, so a capable secondary answers this
in preference to the primary -- see that variable's own docstring for
why (verible silently omits the symbol's own declaration and does no
cross-file lookup at all for a module name). A buffer with only one attached client occupying the PRIMARY slot
behaves identically to before -- see `lsp--preferred-role-client''s own
docstring for the one state that is NOT identical (a lone SECONDARY
sitting in an empty primary slot), which this now answers where it
previously refused."
  (interactive)
  (let ((client (lsp--preferred-role-client "textDocument/references"
                                             "referencesProvider")))
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
