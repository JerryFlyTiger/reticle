;;; verilog-complete.el --- Verilog-local port-name completion (M54) -*- lexical-binding: t -*-

;; M54 repro (real `verible-verilog-ls' binary): this editor's primary
;; Verilog server has NO `completionProvider' key at all in its
;; `initialize' response, and a real `textDocument/completion' request
;; against it gets back `Unhandled method' on the server's own stderr --
;; connected, `C-M-i' silently did nothing. Unconnected, `dabbrev-expand'
;; (M31) can't help either at the one spot this file targets -- a port
;; connection inside a module instantiation (`fifo u_fifo ( .wr| )') --
;; because the port names live in the INSTANTIATED module's own
;; declaration, almost always a DIFFERENT file `dabbrev' never scans (it
;; only ever looks at the current buffer). This file is a purpose-built,
;; LSP-independent completion source for exactly that one spot, wired in
;; ahead of both LSP and dabbrev via `local-completion-function' (see
;; lsp.el's own M54 note and `completion-at-point''s new three-tier
;; docstring).
;;
;; --- v1 scope: what this file does NOT do -------------------------------
;;
;; - Module NAME completion (`fifo| u_fifo (...)', completing the module
;;   identifier itself): not attempted. Only PORT names, inside an
;;   already-named instantiation's own connection list.
;; - Ordinary identifier completion elsewhere in a Verilog buffer (a
;;   signal name, a parameter, anything not immediately after a port-
;;   connecting `.'): `verilog-complete-at-point' returns nil for all of
;;   that, on purpose, so `completion-at-point' falls through to
;;   dabbrev exactly as if this file didn't exist -- dabbrev's own
;;   current-buffer word list is a perfectly fine source for ordinary
;;   identifiers, and this file has no ambition to replace it.
;; - Already-connected ports are NOT excluded from the candidate list
;;   (unlike `verilog-auto--expand-autoinst-site', which does exclude
;;   them for AUTOINST). Completing a port name doesn't insert a
;;   connection -- the user still fills in `(EXPR)' or leaves it as an
;;   implicit `.NAME' -- so offering an already-connected name again is
;;   harmless, and skipping it would mean tracking which of this SAME
;;   instance's ports are already spoken for (a fair bit of extra work,
;;   see `verilog-auto--expand-autoinst-site's own `connected' list) for
;;   a v1 whose whole job is just "don't make the user go read the
;;   submodule's port list by hand".
;; - `#(...)' parameter override positions (`fifo #( .WIDTH| ) u_fifo
;;   (...)') are not completed -- `verilog-complete--port-context' only
;;   recognizes the port CONNECTION list's own `.', never a parameter
;;   override's (a different grammar shape entirely, `parameter_value_
;;   assignment'/`named_parameter_assignment' -- see verilog-auto.el's
;;   `--instance-param-overrides').
;; - `INCOMPLETE'/requery semantics: `show-completion-popup' is always
;;   called with exactly two arguments here -- NO third (INCOMPLETE)
;;   argument, ever. This is a deliberate, permanent property of every
;;   candidate list this file ever produces (a module's own port list is
;;   always complete the moment it's read -- there's no server paging
;;   protocol to be "incomplete" about), not an oversight: it also means
;;   `commands.rs:refilter_completion_popup''s hard-coded
;;   `lsp-completion-at-point' requery branch (only reachable when
;;   `popup.incomplete' is true) can never fire for a popup this file
;;   opened. Typing further while one of THIS file's popups is open
;;   narrows it locally (the ordinary, non-incomplete refilter path),
;;   same as any other local candidate list.
;; - `verilog-auto--declared-names' is not used anywhere in this file
;;   (nothing here needs "what's already declared" -- see the
;;   already-connected-ports note above), so its own documented gap
;;   (`parameter'/`localparam'/`genvar' excluded, per its own docstring
;;   in verilog-auto.el) doesn't carry over here. Noted only because the
;;   M54 spec asked this file to record the scope of anything from
;;   verilog-auto.el it reuses.
;; - `verilog-complete--library-file-modules''s own content-equality
;;   cache (see "This file's own persistent cache" below) only skips
;;   the tree-sitter PARSE on an unchanged library file -- it does NOT
;;   skip `verilog-auto--library-files' itself (M54 review correction):
;;   every single `verilog-complete-at-point' call where the instantiated
;;   module isn't in the CURRENT buffer re-runs a fresh, full
;;   `verilog-auto--library-files' collection -- since M56, a bounded
;;   BREADTH-FIRST RECURSIVE walk (`verilog-auto--library-files-bfs-dir')
;;   of every `verilog-library-directories' entry, not a single flat
;;   `directory-files' call per entry as before, PLUS (unless
;;   `verilog-library-use-filelist' is nil) one `file-contents-as-string'
;;   read and line-by-line parse of a project-root `verible.filelist' if
;;   one exists -- and for every candidate file scanned BEFORE the one whose
;;   module alist happens to contain the name being looked up, a full
;;   `file-contents-as-string' READ of that file's entire on-disk
;;   content (the cache is only consulted, and can only short-circuit
;;   the parse, once a file has already been read in full -- see
;;   `verilog-complete--library-file-modules''s own body). This cost is
;;   paid once per `C-M-i' trigger (i.e. once per
;;   `verilog-complete-at-point' call, not once per keystroke while a
;;   popup this file opened is already showing -- narrowing an open
;;   popup as the user types further is local Rust-side filtering, see
;;   `commands.rs', and never calls back into this file at all).
;;   Measured baseline (M56, release build, same machine): with N
;;   library files (flat, no recursion) sitting in front of the wanted
;;   module in scan order, a COLD `C-M-i' (nothing cached yet) costs
;;   ~24.9ms at N=50, ~92.4ms at N=200, ~244.4ms at N=500 -- a
;;   ~0.49ms/file slope, dominated by `file-contents-as-string' + the
;;   tree-sitter parse per candidate, and roughly FOUR times the
;;   ~0.12ms/file the same fixture costs on `verilog-nav.el''s `M-.'
;;   path, because a cold miss here walks the candidate list TWICE
;;   (once for the ports, then again in `verilog-complete--module-
;;   found-p' to tell "no such module" apart from "module with no
;;   ports"). Do not quote the `M-.' figure for this path: an earlier
;;   draft of this very paragraph did, and the two differ by 4x.
;;   A WARM `C-M-i' (cache already populated, so every candidate
;;   short-circuits to a content-equality check instead of a reparse)
;;   costs ~1.8ms/~6.5ms/~15.4ms at the same N -- a ~0.03ms/file slope. `verilog-auto--library-files' itself (M56's own recursive
;;   scan + optional `verible.filelist' read) adds directory-walk cost
;;   on top of this, separately measured (not repeated here) at ~2.7ms
;;   for 500 library files -- small next to the per-file parse cost
;;   above, which is why M56 did not add a cache layer for the scan
;;   itself (see verilog-auto.el's own `verilog-library-max-files' doc
;;   string for the actual reason a CEILING exists there, which is
;;   correctness -- unbounded symlink recursion -- not this cost).
;; - The CURRENT buffer itself is not spared from repeated cost either
;;   (M54 review correction, second half -- the above only covered the
;;   LIBRARY side): `treesit-parser-create'/`parse' (`treesit.rs') is a
;;   full, uncached, non-incremental reparse of the buffer's ENTIRE text
;;   on every single call (see `treesit.rs''s own `parse_text' and its
;;   module doc for why this is "a full reparse rather than an
;;   incremental one" -- no `Tree::edit'-based reuse exists anywhere in
;;   this interpreter yet). One `C-M-i' trigger on a port-connecting `.'
;;   pays this cost, per code path, as follows (after the M54 review's
;;   `cond'-reordering fix in `verilog-complete-at-point', which removed
;;   one previously-unconditional third parse from the last path below):
;;     - `verilog-complete--port-context' always reparses once, via
;;       `treesit-node-at', to find the enclosing `module_instantiation'
;;       -- paid on EVERY call, even ones that turn out not to be a
;;       port-connection position at all (the parse happens before that's
;;       known).
;;     - `verilog-complete--module-ports' (via `verilog-auto--find-
;;       module-in-buffer') reparses a SECOND time, if execution gets
;;       that far (i.e. `--port-context' returned non-nil) -- paid on
;;       every confirmed port-connection position, regardless of what
;;       the search finds.
;;     - `verilog-complete--module-found-p' (via its own `verilog-auto--
;;       find-module-in-buffer' call) reparses a THIRD time, but ONLY on
;;       the one path left that still calls it after the M54 `cond'
;;       reorder above: PORTS came back nil/empty AND the module wasn't
;;       already found in the buffer by the second parse -- i.e. only
;;       when distinguishing "module not found anywhere" from "module
;;       found, zero ports" is actually still undecided. The "items
;;       non-nil" and "ports non-nil but prefix matched nothing" paths
;;       no longer pay this third parse at all (that's the whole point
;;       of the reorder: PORTS non-nil already proves the module
;;       resolved, on the SAME buffer text just reparsed by the second
;;       call, so a third reparse to ask the identical question again
;;       would be pure waste).
;;   None of this is cached anywhere for the CURRENT buffer specifically
;;   (unlike library files, see the cache section below) -- same
;;   reasoning `verilog-auto--library-files' already documents for
;;   excluding the buffer's own visited file from library scanning: a
;;   buffer's content can change on every keystroke with no cheap "did
;;   it change" signal analogous to a library file's on-disk content
;;   equality check. Left as-is, not attempted in this milestone: fixing
;;   it for real needs either `Tree::edit'-based incremental reparsing
;;   in `treesit.rs' itself (out of this file's scope) or a per-buffer,
;;   per-command-invocation memoized tree passed explicitly between
;;   `--port-context' and `--module-ports' (a `verilog-complete-at-
;;   point'-local refactor that would still only save the FIRST
;;   redundant parse, not the second/third above, since those look up
;;   a DIFFERENT node -- the instantiated module's own definition, not
;;   the `.' 's enclosing instantiation).
;; - This file has no notion of SystemVerilog's `.*' wildcard port
;;   connection (`fifo u_fifo (.*);', auto-connecting every port by
;;   matching signal name -- distinct from the `.NAME' explicit-
;;   connection shape this file targets). `verilog-complete--port-
;;   context' was never probed against that shape's own treesit parse,
;;   so its behavior there (silently mismatch and return nil, fall
;;   through correctly, or something else) is UNTESTED. Recorded here
;;   as a known v1 gap, not attempted.
;;
;; --- Reused from verilog-auto.el (M39) -----------------------------------
;;
;; `verilog-auto--parse-current-buffer'/`--parse-string',
;; `--top-level-modules', `--module-name', `--find-module-in-buffer',
;; `--library-files', `--ports-of-module', `--filter',
;; `--enclosing-of-type' -- all pure functions of their own explicit
;; arguments, none of them reading or writing ANY of verilog-auto.el's
;; own dynamically-scoped, invocation-lifetime state
;; (`verilog-auto--module-cache'/`--missing-modules'/
;; `--ansi-autoarg-modules'/`--multi-autowire-modules').
;;
;; Deliberately NOT reused: `verilog-auto--module-ports' itself, the
;; one function in that file that IS coupled to that dynamic state --
;; every call site outside `verilog-auto' proper first `let'-binds
;; `verilog-auto--module-cache' to a fresh hash table (see `verilog-
;; auto''s own top-level `let*'); calling it from anywhere else hits a
;; `gethash' against a dynamically-unbound-to-nil variable, which
;; signals `wrong-type-argument' immediately (`data.rs' gethash on a
;; non-hash-table) -- not a style question, a guaranteed crash. The M54
;; spec offered two fixes for that coupling: (a) let-bind the dynamic
;; vars fresh at each of THIS file's own call sites before calling
;; `--module-ports', or (b) refactor `--module-ports' to take its cache
;; as an explicit argument. This file does neither, by construction:
;; every helper it needs (module lookup, port extraction) already
;; exists as one of the pure functions listed above, so there is no
;; call to `--module-ports' anywhere in this file to protect in the
;; first place, and `verilog-auto.el' itself is untouched. This isn't
;; picking a THIRD hidden option so much as noticing the two offered
;; ones both assume a call this file never needs to make.
;;
;; --- This file's own persistent cache ------------------------------------
;;
;; `verilog-complete--library-cache' (below) is what M39's own
;; `verilog-auto--module-cache' explicitly is NOT: it survives across
;; separate `verilog-complete-at-point' calls (M39's is `let'-bound
;; fresh every single `verilog-auto' invocation, on purpose -- see that
;; file's own header -- because AUTOINST/AUTOWIRE/AUTOARG all run once
;; per command and never again for a while; port completion, by
;; contrast, can fire on every single keystroke inside a `.NAME',
;; re-resolving and re-parsing the SAME library file over and over
;; inside one edit session would be wasteful for no benefit).
;;
;; The spec for this cache calls for invalidation keyed on the library
;; file's path AND mtime. No `file-attributes'/mtime-reading primitive
;; exists anywhere in this interpreter's elisp surface (checked:
;; `crates/core/src/builtins/files.rs' exposes `file-exists-p',
;; `file-directory-p', `directory-files', `file-contents-as-string' and
;; name-manipulation helpers only -- no timestamp of any kind), and
;; adding one is a `files.rs' change outside this milestone's file
;; scope. Substituted with CONTENT-string equality instead: the cache
;; key is the file's own path, the stored value pairs the exact
;; CONTENT string last read with that content's own already-computed
;; module/port data; every lookup re-reads the file (a plain, cheap
;; `read()') and compares the new content against the cached one
;; byte-for-byte before deciding whether to re-parse. This is STRICTLY
;; more correct than mtime would have been (a `touch'-only mtime bump
;; with byte-identical content would falsely invalidate an mtime-keyed
;; cache; content equality never does) at the cost of not skipping the
;; read() itself on a cache hit -- what it DOES skip is the tree-sitter
;; parse and the port extraction walk for a HIT file. M54 review
;; correction: this cache does NOT skip everything upstream of that --
;; see the "v1 scope" list's own new entry on `verilog-auto--library-
;; files' below for what still runs, uncached, on every single trigger.
;; `verilog-complete-clear-library-cache' (below) is a
;; manual escape hatch for the one case content-equality still can't
;; catch on its own: a library file deleted or renamed out from under a
;; long-running session (an absent path just fails
;; `file-contents-as-string' and returns nil, same as "never found",
;; not a crash -- but a STALE hit for a path that still happens to
;; exist with unrelated new content at that same name is exactly what
;; content-equality already handles, so the escape hatch is a
;; convenience for the human, not a correctness requirement).
;;
;; The CURRENT buffer's own module declarations are never entered into
;; this cache at all (`verilog-complete--module-ports' checks the
;; buffer first, unconditionally, before ever consulting the cache) --
;; a buffer's content changes on every keystroke with no cheap "did it
;; change" signal analogous to a library file's on-disk content, same
;; reasoning `verilog-auto--library-files' already documents for why it
;; excludes the buffer's own visited file from library-directory
;; scanning.

(defvar verilog-complete--library-cache nil
  "Hash table: absolute library file path -> (CONTENT . MODULE-ALIST).
CONTENT is the exact string last read from that path (`file-contents-
as-string'); MODULE-ALIST is (NAME . PORTS) for every top-level module
`verilog-auto--top-level-modules' found in CONTENT, PORTS in
`verilog-auto--ports-of-module' shape. See this file's header for why
CONTENT-equality substitutes for mtime-based invalidation. nil until
first populated; see `verilog-complete--library-file-modules'.")

(defun verilog-complete-clear-library-cache ()
  "Discard every cached library-file module/port entry
\(`verilog-complete--library-cache'). Never needed for correctness (see
this file's header) -- a manual escape hatch for a library file
deleted/renamed out from under a long session, or just to force a
clean re-read while debugging."
  (interactive)
  (setq verilog-complete--library-cache nil))

(defun verilog-complete--library-file-modules (path)
  "Alist (NAME . PORTS) for every top-level module in PATH -- from
`verilog-complete--library-cache' when PATH's on-disk content is
byte-identical to what was cached for it, else freshly parsed and
re-cached. nil (and, deliberately, no cache entry written) if PATH
can't be read at all -- a transient read failure should never poison
the cache with a permanent miss."
  (let ((text (condition-case nil (file-contents-as-string path) (error nil))))
    (if (not text)
        nil
      (let ((entry (and verilog-complete--library-cache
                         (gethash path verilog-complete--library-cache))))
        (if (and entry (string= (car entry) text))
            (cdr entry)
          (let* ((root (verilog-auto--parse-string text))
                 (mods (verilog-auto--top-level-modules root))
                 (alist (mapcar (lambda (m)
                                   (cons (verilog-auto--module-name m)
                                         (verilog-auto--ports-of-module m)))
                                 mods)))
            (unless verilog-complete--library-cache
              (setq verilog-complete--library-cache (make-hash-table :test 'equal)))
            (puthash path (cons text alist) verilog-complete--library-cache)
            alist))))))

(defun verilog-complete--library-ports (name)
  "Port list for module NAME, searched across
`verilog-auto--library-files' in that function's own order (first file
whose module alist has NAME wins -- matches `verilog-auto--find-
module-in-libraries''s own resolution order). nil if NAME isn't found
in any of them. The CURRENT buffer's own definitions are never
consulted here -- see `verilog-complete--module-ports'."
  (let ((files (verilog-auto--library-files)) (found 'verilog-complete--miss))
    (while (and files (eq found 'verilog-complete--miss))
      (let* ((path (car files))
             (alist (verilog-complete--library-file-modules path))
             (entry (and alist (assoc name alist))))
        (when entry (setq found (cdr entry))))
      (setq files (cdr files)))
    (if (eq found 'verilog-complete--miss) nil found)))

(defun verilog-complete--module-ports (name)
  "Port list for module NAME: the CURRENT buffer's own top-level module
declarations first (never cached -- see this file's header), else
`verilog-complete--library-ports' (cached, keyed by library file
content). nil if NAME isn't found anywhere -- callers must treat that
as \"no completions\", never signal."
  (let ((buf-node (verilog-auto--find-module-in-buffer name)))
    (if buf-node
        (verilog-auto--ports-of-module buf-node)
      (verilog-complete--library-ports name))))

;; --- Port-connection context detection -----------------------------------

(defun verilog-complete--ident-char-p (c)
  "Non-nil if C is a Verilog identifier constituent -- letters, digits,
`_', `$' (SystemVerilog allows `$' inside a simple identifier, unlike
`dabbrev--prefix-char-p''s own class; no `-', Verilog has none)."
  (and c (or (and (>= c ?a) (<= c ?z))
             (and (>= c ?A) (<= c ?Z))
             (and (>= c ?0) (<= c ?9))
             (= c ?_) (= c ?$))))

(defun verilog-complete--prefix-start (pos)
  "Start of the run of `verilog-complete--ident-char-p' characters
ending at POS -- POS itself when POS sits right after a non-identifier
character (or at `point-min'), i.e. an empty prefix (the user just
typed the `.' itself, nothing after it yet). Same shape as `dabbrev--
prefix-start', independent implementation: a different character
class (see `verilog-complete--ident-char-p') and this file has no
reason to share dabbrev's own internal state."
  (let ((p pos))
    (while (and (> p (point-min)) (verilog-complete--ident-char-p (char-before p)))
      (setq p (1- p)))
    p))

(defun verilog-complete--port-context (prefix-start)
  "The enclosing `module_instantiation' treesit node if PREFIX-START
(see `verilog-complete--prefix-start') sits immediately after a
port-connecting `.' -- i.e. point is at `.NAME|' (NAME possibly empty,
the user just typed the dot) inside some instantiation's own
`( ... )' connection list -- else nil.

Probed (M54, `verible-verilog-ls' grammar, `tree-sitter-systemverilog')
against three shapes a `.' can parse into at this exact spot:
  - `.wr' (an identifier already typed, or already complete): the `.'
    is a direct child of a `named_port_connection' node -- checked
    below via PARENT's own type.
  - `.wr(x), .' (a SECOND `.' right after an existing comma-separated
    connection): still a direct child of `named_port_connection', this
    time one whose own `port_name' field is a zero-width `MISSING'
    node -- same PARENT-type check catches it identically, no special
    casing needed.
  - `. ' (the buffer's FIRST, and so far only, port connection): parses
    with NO `named_port_connection' at all -- tree-sitter's error
    recovery leaves a bare `ERROR' node whose own direct parent is
    `hierarchical_instance' itself.
A value-side `.' -- `.addr(cfg.|base)', a hierarchical reference
inside a connection's own VALUE expression, not the port-connecting
dot at all -- sits nested inside a `hierarchical_identifier'/`primary'/
`expression' chain instead, so its own immediate parent is never
`named_port_connection' NOR `hierarchical_instance' directly; the two
`cond' branches below are what excludes it, not a bounded upward walk
(an upward walk alone would eventually reach the SAME enclosing
`module_instantiation' either way, which is why the immediate-parent
check, not just \"is there a `module_instantiation' ancestor
somewhere\", is the actual discriminator here)."
  (when (> prefix-start (point-min))
    (let ((before (char-before prefix-start)))
      (when (eq before ?.)
        (let* ((dot-pos (1- prefix-start))
               (parser (treesit-parser-create 'verilog))
               (dot-node (treesit-node-at dot-pos parser)))
          (when (and dot-node (string= (treesit-node-text dot-node) "."))
            (let ((parent (treesit-node-parent dot-node)))
              (cond
               ((and parent (string= (treesit-node-type parent) "named_port_connection"))
                (verilog-auto--enclosing-of-type parent "module_instantiation"))
               ((and parent (string= (treesit-node-type parent) "ERROR"))
                (let ((grandparent (treesit-node-parent parent)))
                  (when (and grandparent
                             (string= (treesit-node-type grandparent) "hierarchical_instance"))
                    (verilog-auto--enclosing-of-type grandparent "module_instantiation"))))
               (t nil)))))))))

;; --- Popup item construction ----------------------------------------------

(defun verilog-complete--module-found-p (name)
  "Non-nil if module NAME resolves anywhere -- the CURRENT buffer's own
top-level declarations, or any `verilog-auto--library-files' entry --
regardless of whether its port list is empty. Only used from
`verilog-complete-at-point''s message path, to tell \"module not
found at all\" apart from \"found, but it has zero ports\": both cases
make `verilog-complete--module-ports' return nil (an empty port list
IS nil in elisp, same as \"no module\"), so that function alone can't
distinguish them. Deliberately only called on the already-rare
no-candidates path -- it re-walks `verilog-auto--library-files' (a
second directory scan/file-read pass beyond `verilog-complete--module-
ports''s own), which would be wasteful to pay on every keystroke but
is fine on a path that's already about to fail anyway."
  (or (verilog-auto--find-module-in-buffer name)
      (let ((files (verilog-auto--library-files)) found)
        (while (and files (not found))
          (let* ((path (car files))
                 (alist (verilog-complete--library-file-modules path)))
            (when (and alist (assoc name alist))
              (setq found t)))
          (setq files (cdr files)))
        found)))

(defun verilog-complete--port-label (port)
  "\"NAME: DIRECTION RANGE\" (RANGE omitted when PORT has none, e.g.
\"wr_en: input\" or \"data: output [7:0]\") -- chosen over a bare NAME
so the popup itself answers \"which way does this port face, and how
wide is it\" without making the user go check the submodule; DIRECTION
first (matches how a human reads a port list -- direction is usually
the first thing that matters) and RANGE last (the part most likely to
be absent, so it never shifts where DIRECTION lands)."
  (let ((name (nth 0 port)) (dir (nth 1 port)) (range (nth 2 port)))
    (if range
        (format "%s: %s %s" name dir range)
      (format "%s: %s" name dir))))

(defun verilog-complete--port-item (port prefix-start)
  "One `show-completion-popup' ITEMS element for PORT (a `verilog-
auto--module-ports'-shaped triple). INSERT/FILTER are the bare port
NAME (no leading `.' -- the `.' is already in the buffer, this
candidate only ever replaces PREFIX-START..point, the identifier run
after it)."
  (let ((name (nth 0 port)))
    (list (verilog-complete--port-label port) name prefix-start name)))

;; --- Entry point ------------------------------------------------------------

(defun verilog-complete-at-point ()
  "`local-completion-function' for `verilog-mode' (see `modes.el' and
lsp.el's own M54 note on `completion-at-point''s three tiers).

Returns nil -- \"not applicable, try the next completion source\" --
whenever point isn't immediately after a port-connecting `.'
(`verilog-complete--port-context' returns nil): ordinary Verilog
identifiers elsewhere in the buffer fall straight through to
LSP/dabbrev exactly as if this file didn't exist.

Once point IS confirmed to be a port-connection position, this
function always returns t from there on -- INCLUDING when the
instantiated module can't be resolved anywhere (current buffer nor any
`verilog-library-directories' file) or its port list comes back empty:
deliberately NOT nil in that case, so `completion-at-point' does not
then fall through to `dabbrev-expand'. A dot-prefixed position is not
an ordinary identifier spot dabbrev's own heuristics are built for --
dabbrev has no notion of \"after a port-connecting dot\" at all, so it
would search the CURRENT buffer for words merely resembling whatever's
already typed and offer them as if they were plausible port names,
which they aren't (the actual port names live in the submodule, a
different file dabbrev never reads -- see this file's own header for
why that's the whole reason this file exists). Silence is the honest
answer once this function has confirmed the context but has nothing
real to offer; a misleading, unrelated dabbrev guess is not friendlier
than that -- it's actively worse, since it looks exactly as
plausible as a real port name until the user tries it.

Never signals, at any step: every helper below is either a plain
buffer/string scan or `verilog-complete--module-ports', whose own
worst case is nil, never an error (its own doc string) -- a malformed
tree, an unreadable library file, or a module found nowhere all
degrade to \"no candidates\", not a signal -- EXCEPT
`treesit-node-child-by-field-name''s own `instance_type' lookup below,
whose result feeds `treesit-node-text': a nil-guard covers the case
where tree-sitter's own ERROR-recovery leaves that field missing on an
otherwise-recognized `module_instantiation' node. M54 review: a dozen-
odd malformed-input shapes were tried against the real grammar (missing
instance name, missing module identifier, stray `#', a second bare `.'
after a comma, an unnamed instantiation, ...) and NONE produced a
`module_instantiation' node whose `instance_type' field came back nil
-- every one of them either still filled `instance_type' in (tree-
sitter's own ERROR-recovery is more forgiving than expected) or failed
to produce a `module_instantiation' node at all (so `mi' itself was
nil, short-circuited by the check above, never reaching this branch).
So: BLACK BOX, not exercised by any test -- `treesit-node-text' on a
nil node SIGNALS, `wrong-type-argument' (see `treesit.rs''s `node_arg'),
not \"never signals\" as this docstring otherwise promises, so the
guard stays as a defense-in-depth backstop despite the lack of a
reproduction, on the theory that a grammar shape this file's own
author didn't think to try might still exist.

When a port-connection position IS confirmed but nothing can be
offered, a `message' explains why -- silence alone (pre-review M54)
looked identical to \"there is nothing more to type\", indistinguishable
from an outright hang or a bug; distinguishing \"the instantiated module
couldn't be resolved at all\" from \"it resolved, but has zero ports\"
costs an extra `verilog-complete--module-found-p' lookup, paid only on
this already-failing path."
  (let* ((point (point))
         (prefix-start (verilog-complete--prefix-start point))
         (mi (verilog-complete--port-context prefix-start)))
    (if (not mi)
        nil
      (let ((type-node (treesit-node-child-by-field-name mi "instance_type")))
        (if (not type-node)
            ;; See this docstring's nil-guard note above -- black box,
            ;; no repro found despite trying, kept as a defensive
            ;; backstop only.
            (progn
              (message "Verilog port completion: instantiated module type not found in this position")
              t)
          (let* ((type-name (treesit-node-text type-node))
                 (ports (verilog-complete--module-ports type-name))
                 (typed (buffer-substring-no-properties prefix-start point))
                 (candidates (verilog-auto--filter
                              (lambda (p) (string-prefix-p typed (nth 0 p)))
                              ports))
                 (items (mapcar (lambda (p) (verilog-complete--port-item p prefix-start))
                                 candidates)))
            (cond
             (items
              ;; No INCOMPLETE (third) argument -- ever, see this file's
              ;; header's "INCOMPLETE/requery semantics" note.
              (show-completion-popup items prefix-start))
             (ports
              ;; PORTS non-nil here already proves the module resolved
              ;; (that's exactly where PORTS came from) -- calling
              ;; `verilog-complete--module-found-p' on this branch would
              ;; be a zero-information re-lookup that pays a second full
              ;; buffer reparse (and possibly a library rescan) for an
              ;; answer already known. See this file's header on why
              ;; `--module-found-p' is only worth its cost on the
              ;; genuinely-ambiguous branch below.
              (message "Verilog port completion: no port of `%s' starts with `%s'" type-name typed))
             ((verilog-complete--module-found-p type-name)
              (message "Verilog port completion: module `%s' has no ports" type-name))
             (t
              (message "Verilog port completion: module `%s' not found (current buffer or library dirs)" type-name)))
            t))))))

(provide 'verilog-complete)
