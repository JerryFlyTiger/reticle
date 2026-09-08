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
;; --- M91: module-name and parameter-name completion ----------------------
;;
;; M91 adds two more completion sources, wired into the same entry point
;; ahead of the port-connection one so ordering never matters (the three
;; detectors are structurally disjoint -- see each one's own docstring):
;;
;; - Module NAME completion at an instantiation's own type-name position
;;   (`fif|' about to become `fifo u_fifo (...)'), across the CURRENT
;;   buffer's own top-level modules and every `verilog-auto--library-
;;   files' entry. `verilog-complete--instantiation-type-context' is the
;;   detector; see its own docstring for the half-typed-buffer probe this
;;   was built against (real `verible-verilog-ls' grammar dumps of `fif',
;;   `fifo ', `fifo u_', a plain expression identifier, a procedural-
;;   block identifier, and a module's own declaration name -- the last
;;   four are exactly the shapes this detector must say nil to, and does).
;; - Parameter NAME completion inside an instantiation's own `#(...)'
;;   override list (`fifo #(.W|' -> `WIDTH'), for a module resolved the
;;   same way (current buffer, then library files) and CACHED the same
;;   way -- see "This file's own persistent cache" below, extended, not
;;   duplicated, for M91. `verilog-complete--param-context' is the
;;   detector.
;; - Both stay within the SAME nil-means-fall-through contract every
;;   other detector in this file already promises: neither one ever
;;   fires on a position it isn't confident about (see each detector's
;;   own docstring for exactly which grammar shapes it recognizes and
;;   which it deliberately leaves to fall through to dabbrev/LSP).
;;
;; What M91 still does NOT do, on purpose:
;; - Module-name completion requires a NON-EMPTY typed prefix (at least
;;   one character of the module name already typed). An empty-prefix
;;   position (cursor sitting at a bare statement start, nothing typed
;;   yet) has no anchoring token the way a port/parameter `.' does, and
;;   the M91 probe found that shape indistinguishable from countless
;;   other not-yet-typed statement starts (a net declaration, a task
;;   call, a keyword being typed letter by letter, ...) -- exactly the
;;   "can't distinguish confidently" case this file's own spec calls out
;;   by name. Falls through to dabbrev, same as before M91.
;; - Parameter-name completion only recognizes a module declared with an
;;   ANSI `#(parameter ...) (...)' header (`parameter_port_list' as a
;;   child of `module_ansi_header'). A module that declares its
;;   parameters as body-level `parameter' statements instead (legal
;;   Verilog, just a different, older style) is not resolved --
;;   `verilog-complete--parameters-of-module' returns nil for it, same
;;   as "this module has no parameters", not attempted to distinguish
;;   from the real "zero parameters" case. Not probed against that
;;   grammar shape at all; recorded here as a known v1 gap.
;; - Completing a module name from a file `verilog-auto--library-files'
;;   itself can't reach (outside `verilog-library-directories', beyond
;;   `verilog-library-max-depth'/`-max-files', or absent from
;;   `verible.filelist' when `verilog-library-use-filelist' expects one)
;;   is out of scope here too -- exactly the same reachability boundary
;;   the port-completion path already lives with, not a new cut.
;; - General identifier completion anywhere else in a Verilog buffer (a
;;   signal name, a local variable, ...) is a different question,
;;   deliberately being decided separately -- see the pre-existing bullet
;;   below, unchanged by M91.
;;
;; --- v1 scope: what this file does NOT do -------------------------------
;;
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
;; - M91 measured cost (release build, `cargo test -p core --test
;;   verilog_complete_tests --release -- --nocapture' timing around
;;   `(verilog-complete-at-point)' via `std::time::Instant', N library
;;   files -- superseded twice, see both fix-round notes below; ONLY
;;   the Y1 numbers describe the code as it ships):
;;     - V1 fix round (SUPERSEDED by Y1, kept for the historical
;;       comparison Y1's own numbers are measured against): the
;;       keyword-prefix VETO this round shipped rejected an ordinary
;;       keystroke in ~23 MICROseconds at N=500 -- but Y1 found that
;;       same veto also rejected a REAL module match whose name shared
;;       a keyword's own prefix (see `verilog-complete--any-module-
;;       name-matches-p''s own docstring), so this number describes
;;       code this file no longer ships.
;;     - Y1 fix round (final, current): the veto is gone, replaced by
;;       `verilog-complete--any-module-name-matches-p', a cheap
;;       existence check consulted UNCONDITIONALLY in place of it (see
;;       that function's own docstring). At N=500 reachable library
;;       files:
;;         - COLD (nothing cached yet), no module matches the typed
;;           prefix: ~64-70ms -- same order as the M56 port-path
;;           numbers above, unavoidable: every file must be read at
;;           least once to know whether it matches.
;;         - WARM (cache already populated by a prior pass), no module
;;           matches -- the common case, an ordinary statement-start
;;           keystroke shaped like a keyword being typed (`wi' toward
;;           `wire', etc.): ~5-6ms. Slower than the SUPERSEDED V1
;;           veto's ~23 microseconds (that number came from skipping
;;           the check entirely, which is exactly the bug Y1 fixes),
;;           but well under the WARM cost this same no-match case used
;;           to pay when `verilog-complete--all-modules' itself was
;;           the only way to answer "does anything match" (~13.8ms,
;;           see the M56-family port-path numbers this superseded) --
;;           `--any-module-name-matches-p' answers from already-cached
;;           per-file data with no file I/O, whereas the SUPERSEDED
;;           path it replaces still re-derived the full labeled
;;           candidate list even to answer "no".
;;         - WARM, a real module DOES match (e.g. `wi' typed toward a
;;           real `wi_bridge' module): ~15-17ms -- close to the OLD
;;           warm-with-full-scan number, expected: once the cheap
;;           check says "maybe", `verilog-complete--all-modules' still
;;           has to build the actual, fully-revalidated candidate list
;;           for the popup, same cost as before Y1. This is the price
;;           of correctness on the path that's ABOUT to show something
;;           anyway, not the common no-op keystroke.
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
as-string'); MODULE-ALIST is (NAME . (PORTS . PARAMS)) for every
top-level module `verilog-auto--top-level-modules' found in CONTENT,
PORTS in `verilog-auto--ports-of-module' shape, PARAMS in
`verilog-complete--parameters-of-module' shape (M91: both computed
together from the SAME parse/tree-walk of the SAME module-declaration
node, so caching one for free caches the other -- no second parse, no
second cache). See this file's header for why CONTENT-equality
substitutes for mtime-based invalidation. nil until first populated;
see `verilog-complete--library-file-modules'.")

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
                                         (cons (verilog-auto--ports-of-module m)
                                               (verilog-complete--parameters-of-module m))))
                                 mods)))
            (unless verilog-complete--library-cache
              (setq verilog-complete--library-cache (make-hash-table :test 'equal)))
            (puthash path (cons text alist) verilog-complete--library-cache)
            alist))))))

(defun verilog-complete--library-entry (name)
  "(PORTS . PARAMS) cons for module NAME, searched across
`verilog-auto--library-files' in that function's own order (first file
whose module alist has NAME wins -- matches `verilog-auto--find-
module-in-libraries''s own resolution order). nil if NAME isn't found
in any of them. The CURRENT buffer's own definitions are never
consulted here -- see `verilog-complete--module-ports'/`--module-
parameters'. M91: both PORTS and PARAMS come from the same cached
alist entry (see `verilog-complete--library-cache''s own doc string),
so a caller that only wants one of the two pays no extra parse for
having both available."
  (let ((files (verilog-auto--library-files)) (found 'verilog-complete--miss))
    (while (and files (eq found 'verilog-complete--miss))
      (let* ((path (car files))
             (alist (verilog-complete--library-file-modules path))
             (entry (and alist (assoc name alist))))
        (when entry (setq found (cdr entry))))
      (setq files (cdr files)))
    (if (eq found 'verilog-complete--miss) nil found)))

(defun verilog-complete--library-ports (name)
  "Port list for module NAME -- see `verilog-complete--library-entry'."
  (car (verilog-complete--library-entry name)))

(defun verilog-complete--library-parameters (name)
  "Parameter list for module NAME -- see `verilog-complete--library-
entry'."
  (cdr (verilog-complete--library-entry name)))

(defun verilog-complete--library-cache-peek (path)
  "The cached (NAME . (PORTS . PARAMS)) alist for PATH -- WITHOUT
touching the filesystem at all: no `file-contents-as-string' read, no
content-equality check, just a `gethash' against `verilog-complete--
library-cache' as it stands right now. nil if PATH has never been
cached this session (a file nothing has visited yet), which is NOT the
same thing as \"PATH has no modules\" -- callers that need a definitive
answer must fall back to `verilog-complete--library-file-modules'
(which DOES read/validate) when this returns nil.

M91 fix round (Y1): this exists for exactly ONE caller, `verilog-
complete--any-module-name-matches-p' -- see that function's own
docstring for why an EXISTENCE pre-check is the one place in this file
where skipping the usual content-equality revalidation is an
acceptable trade, and every other caller (`--library-ports'/`--
library-parameters', via `--library-entry') must keep going through
`--library-file-modules' instead, for the reason `verilog-complete--
library-cache''s own doc string already gives."
  (and verilog-complete--library-cache
       (let ((entry (gethash path verilog-complete--library-cache)))
         (and entry (cdr entry)))))

(defun verilog-complete--any-module-name-matches-p (typed)
  "Non-nil the instant some module -- the CURRENT buffer's own, or any
`verilog-auto--library-files' entry -- has a name starting with TYPED.

M91 fix round (Y1): this is what replaced the keyword-prefix veto
(`verilog-complete--instantiation-type-context' no longer consults a
reserved-word list at all -- see that function's own docstring). The
earlier veto existed to avoid paying `verilog-complete--all-modules''s
own full library scan on every ordinary statement-start keystroke, but
it did so by REJECTING before ever checking for a real match, which
also rejected a genuine module whose name happened to share a
keyword's own prefix (`re' typed toward a module named `reset_ctrl',
blocked because `re' also prefixes `real'/`return'/`release') -- a
real regression for RTL, where long, descriptive module names are
routine. The fix is to make the MATCH check itself cheap enough to run
unconditionally, first, with no veto layer above it at all: a real
match always wins, and a keystroke that matches nothing is now
answered without ever calling `verilog-complete--all-modules' (whose
own job -- building the full, labeled candidate LIST for the popup --
is only worth doing once this function has already said something
will match).

Cheap because it is allowed to answer from a STALE cache: a library
file this session has ALREADY visited (via ANY completion path --
port, parameter, or module-name alike, they all share `verilog-
complete--library-cache') is answered straight from `verilog-complete--
library-cache-peek', no file I/O at all; only a file with NO cache
entry yet is actually read/parsed here, via `verilog-complete--
library-file-modules' (unavoidable -- there is no way to know what an
unvisited file contains without reading it once, and this also POPULATES
the cache for next time, same as every other caller of that function).
A t answer here is a \"probably\" hint, not the final word: `verilog-
complete--handle-instantiation-type-context' still builds the actual
popup contents from the fully-revalidated `verilog-complete--all-
modules', so a STALE-POSITIVE (a module renamed away since this
session cached that file) never reaches the user -- the freshly
rebuilt candidate list simply comes back empty for that name, and the
caller falls through to nil exactly as if this function had said nil
in the first place. A STALE-NEGATIVE (a module ADDED to an
already-cached file since this session last actually read it) is the
one direction this trade can miss a real match, until something else
revalidates that SAME file (another completion path touching it, or
`verilog-complete-clear-library-cache') -- documented here rather than
silently accepted."
  (or
   (let ((found nil) (mods (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer))))
     (while (and mods (not found))
       (when (string-prefix-p typed (verilog-auto--module-name (car mods)))
         (setq found t))
       (setq mods (cdr mods)))
     found)
   (let ((files (verilog-auto--library-files)) (found nil))
     (while (and files (not found))
       (let* ((path (car files))
              (peeked (verilog-complete--library-cache-peek path))
              (alist (or peeked (verilog-complete--library-file-modules path))))
         (when (verilog-auto--filter (lambda (e) (string-prefix-p typed (car e))) alist)
           (setq found t)))
       (setq files (cdr files)))
     found)))

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

(defun verilog-complete--module-parameters (name)
  "Parameter list for module NAME: the CURRENT buffer's own top-level
module declarations first, else `verilog-complete--library-parameters'
(cached). nil if NAME isn't found anywhere, or is found but declares no
`parameter_port_list' -- see this file's header's M91 scope-cut note on
body-level `parameter' statements, which this does not recognize."
  (let ((buf-node (verilog-auto--find-module-in-buffer name)))
    (if buf-node
        (verilog-complete--parameters-of-module buf-node)
      (verilog-complete--library-parameters name))))

(defun verilog-complete--parameters-of-module (module-decl)
  "Alist-shaped list of (NAME DEFAULT) for every `param_assignment' in
MODULE-DECL's own `#(parameter ...)' header list (`parameter_port_list',
a child of `module_ansi_header' only -- see this function's own M91
scope-cut note in this file's header for the non-ANSI, body-level
`parameter' statement shape this does NOT recognize). DEFAULT is the
declared default value's own text (`constant_param_expression'), or nil
if a parameter has none (legal only in certain contexts, but this
function doesn't validate that -- a nil DEFAULT here just means \"no
text to show\", same non-judgmental spirit as `verilog-complete--port-
label's own RANGE-omitted case). nil if MODULE-DECL has no ANSI header,
or an ANSI header with no `parameter_port_list' at all (an ordinary
module with only ports, no parameters -- the overwhelmingly common
case)."
  (let ((header (verilog-auto--header-node module-decl)))
    (when (verilog-auto--ansi-header-p header)
      (let ((plist (verilog-auto--find-first-of-type header "parameter_port_list")))
        (when plist
          (mapcar
           (lambda (pa)
             (list (treesit-node-text (verilog-auto--find-first-of-type pa "simple_identifier"))
                   (let ((val (verilog-auto--find-first-of-type pa "constant_param_expression")))
                     (and val (treesit-node-text val)))))
           (verilog-auto--find-all-of-type plist "param_assignment")))))))

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

(defun verilog-complete--param-context (prefix-start)
  "The instantiated module's own type-NAME text (a string), if
PREFIX-START sits immediately after a parameter-override-connecting
`.' -- i.e. point is at `.NAME|' inside some instantiation's own
`#( ... )' override list -- else nil. Returns a STRING, not a node
(unlike `verilog-complete--port-context'): the M91 probe found that the
most common half-typed shape (see below) collapses the WHOLE buffer,
including the module header, into one `ERROR' at `source_file' level,
with no `module_instantiation' node anywhere left to return -- the
instantiated module's own type name has to be read directly off that
ERROR's own flat child list instead.

Probed (M91, same grammar as `verilog-complete--port-context''s own
M54 probe) against the shapes a `.' can parse into at this exact spot:
  - `fifo #(.WIDTH(8)) u_fifo (...)' (a fully closed override list, `.'
    already typed/complete): `.' is a direct child of a
    `named_parameter_assignment' node -- checked below via PARENT's own
    type, exactly `verilog-complete--port-context''s own first branch,
    just for the parameter grammar node instead of the port one. The
    enclosing `module_instantiation''s own `instance_type' field text
    is the answer.
  - `fifo #(.WIDTH(8), .DEPTH(16)) ...' probed too (a SECOND override
    after a comma): same `named_parameter_assignment' parent shape,
    no special casing needed -- matches `--port-context''s own second
    bullet.
  - `fifo #( .' (the buffer's FIRST, not-yet-closed override -- no `)'
    anywhere yet): tree-sitter's error recovery can't rebuild ANY
    structure above this point, so the ENTIRE remainder of the buffer
    from `module' onward becomes one `ERROR' node, a direct child of
    `source_file' -- `.' 's own immediate parent IS that ERROR, and its
    PRECEDING sibling (skipping nothing) is `(', and the one before
    THAT is `#', and the one before THAT is the type-name
    `simple_identifier' itself. `verilog-complete--param-type-name-
    from-error' below is exactly that three-step backward walk.
    CORRECTION (M91 fix round, V2): \"the entire remainder becomes ONE
    ERROR\" is not always true -- see the next bullet and `verilog-
    complete--param-type-name-unwrap''s own docstring for the shape
    where a SECOND broken statement nests its own ERROR one level
    inside the first, not as a flat sibling.
  - `fifo #(.WIDTH(8), .' (a not-yet-closed SECOND override): same
    collapsed top-level ERROR, but now `.' 's preceding sibling is `,'
    and the one before THAT is the ALREADY-parsed `named_parameter_
    assignment' subtree for `.WIDTH(8)' (tree-sitter still manages to
    parse a COMPLETE prior override even while everything downstream of
    it is unparseable) -- `--param-type-name-from-error' skips over any
    run of `,'/`named_parameter_assignment' siblings before applying
    the same `( #' check, so this shape needs no separate branch either.
  - `fifo #(.W\n  fifo2 #(.X' (TWO separate, simultaneously broken
    instantiations stacked on consecutive lines): the FIRST one's own
    leftover tokens (`#(.W') do NOT stay flat siblings of the top
    ERROR the way the single-broken-override case above does -- they,
    PLUS the next statement's own type-name identifier (`fifo2'),
    all nest inside a SECOND, INNER `ERROR' node, itself one flat
    sibling of the top ERROR. So by the time the backward walk from
    `.X' reaches the position where a bare `simple_identifier' sat in
    every shape above, it instead finds that whole nested ERROR --
    `verilog-complete--param-type-name-unwrap' is what recurses into
    it (via its own last child) to still find `fifo2'. Probed to work
    for exactly two stacked broken overrides; three or more is an
    unverified extrapolation -- see that function's own docstring.
  - `fifo #/*c*/(.W' (a comment between `#' and `('): the comment is
    its own flat sibling too, between `#' and `(' in the SAME ERROR's
    child list -- `--param-type-name-from-error' skips a run of
    `block_comment'/`one_line_comment' siblings there before requiring
    `#'.
A port-connection's own `.' (`fifo u_fifo (.wr_en|') is excluded by
construction: its own ERROR-collapse shape (see `--port-context''s own
third bullet) puts an instance NAME immediately before the `(', never
a bare `#' -- the backward walk below requires `#' right before `(',
so a port dot's differently-shaped predecessor sequence simply fails
the check and this function returns nil, exactly the fall-through
`verilog-complete--port-context' itself would already have taken over
first (see the M91 entry point's own ordering note)."
  (when (> prefix-start (point-min))
    (let ((before (char-before prefix-start)))
      (when (eq before ?.)
        (let* ((dot-pos (1- prefix-start))
               (parser (treesit-parser-create 'verilog))
               (dot-node (treesit-node-at dot-pos parser)))
          (when (and dot-node (string= (treesit-node-text dot-node) "."))
            (let ((parent (treesit-node-parent dot-node)))
              (cond
               ((and parent (string= (treesit-node-type parent) "named_parameter_assignment"))
                (let ((mi (verilog-auto--enclosing-of-type parent "module_instantiation")))
                  (when mi
                    (let ((type-node (treesit-node-child-by-field-name mi "instance_type")))
                      (and type-node (treesit-node-text type-node))))))
               ((and parent (string= (treesit-node-type parent) "ERROR"))
                (verilog-complete--param-type-name-from-error parent dot-node))
               (t nil)))))))))

(defun verilog-complete--param-type-name-from-error (error-node dot-node)
  "Backward walk over ERROR-NODE's own flat child list, starting just
before DOT-NODE's own index -- see `verilog-complete--param-context's
own docstring for the shapes this recognizes. Skips any run of `,' or
`named_parameter_assignment' siblings (a prior, already-parsed
override), then requires `(' then, skipping any comment sibling(s)
between `#' and `(' (M91 fix round V2: `fifo #/*c*/(.W' -- a comment
token sits between the two as its own flat sibling in this SAME
ERROR's child list, not nested inside either), requires `#', then
hands the one sibling before THAT to `verilog-complete--param-type-
name-unwrap' -- that identifier's own text is the answer, else nil."
  (let ((i (1- (or (verilog-auto--child-index error-node dot-node) 0))))
    (while (and (>= i 0)
                (member (treesit-node-type (treesit-node-child error-node i))
                        '("," "named_parameter_assignment")))
      (setq i (1- i)))
    (when (and (>= i 0) (string= (treesit-node-type (treesit-node-child error-node i)) "("))
      (setq i (1- i))
      (while (and (>= i 0)
                  (member (treesit-node-type (treesit-node-child error-node i))
                          '("block_comment" "one_line_comment")))
        (setq i (1- i)))
      (when (and (>= i 0) (string= (treesit-node-type (treesit-node-child error-node i)) "#"))
        (setq i (1- i))
        (when (>= i 0)
          (verilog-complete--param-type-name-unwrap (treesit-node-child error-node i)))))))

(defun verilog-complete--param-type-name-unwrap (node)
  "NODE's own text if NODE is a `simple_identifier' -- else, M91 fix
round (V2): when a SECOND `#(...)' override list is ALSO broken and
stacks directly after a first broken one (`fifo #(.W\n  fifo2 #(.X'),
tree-sitter's error recovery nests the first override's own leftover
tokens PLUS the next statement's own type-name identifier
(`fifo2') inside one ERROR node of their own, rather than leaving
`fifo2' as a plain sibling one step back the way a single broken
override does -- so the identifier actually wanted is that nested
ERROR's own LAST child (`verilog-auto--last-child', already reused
elsewhere in this file), not NODE itself. Unwraps through an
arbitrary run of nested ERRORs the same way (recursing on each one's
own last child), stopping at the first non-ERROR node: its text if
`simple_identifier', else nil.

Probed (M91 fix round, across two review rounds) through FOUR
consecutively stacked broken overrides, not just two -- each
confirmed to nest one level deeper than the last (four separate,
simultaneously-broken instantiations on consecutive lines, the FOURTH
one's own type name still correctly recovered) -- this function's own
recursive unwrap has no depth limit of its own, so this is confirmed
behavior through the depth actually tested, not a hard ceiling; five
or more in a row was not probed."
  (cond
   ((null node) nil)
   ((string= (treesit-node-type node) "simple_identifier") (treesit-node-text node))
   ((string= (treesit-node-type node) "ERROR")
    (verilog-complete--param-type-name-unwrap (verilog-auto--last-child node)))
   (t nil)))

;; M91 fix round (Y1): `verilog-complete--statement-keywords' and
;; `verilog-complete--keyword-prefix-p' USED to live here -- a
;; reserved-word list, consulted by `verilog-complete--instantiation-
;; type-context' before any structural check, to veto a keyword-
;; shaped prefix (`wi|' toward `wire') before it ever reached measure
;; 2's own real-module-match check. A trailing cold read against the
;; real function found that veto fires UNCONDITIONALLY, before measure
;; 2 ever runs -- so a real module whose name happens to share a
;; keyword's own prefix (a module named `reset_ctrl', typed as `re',
;; colliding with `real'/`return'/`release') was silently blocked from
;; ever completing, which is a real regression for RTL, where long,
;; descriptive module names sharing a few letters with a reserved word
;; is routine, not a corner case. Removed entirely, not patched: the
;; keyword list's whole REASON for existing was to avoid paying
;; `verilog-complete--all-modules''s own full scan on every ordinary
;; keystroke, and `verilog-complete--any-module-name-matches-p' (see
;; `verilog-complete--library-cache-peek's own docstring, above the
;; cache accessors) now answers \"does anything match\" cheaply enough
;; to run FIRST, unconditionally, with no veto layer needed above it
;; at all -- a real match always wins, by construction, because
;; nothing gets a chance to override it before it is even checked.
;; See `verilog-complete--instantiation-type-context''s own docstring
;; for the resulting (purely structural, no keyword check at all) flow.

(defun verilog-complete--instantiation-type-context (prefix-start point)
  "Non-nil if [PREFIX-START, POINT) is the type-name identifier of a
module instantiation being typed -- i.e. `fif|' about to become
`fifo u_fifo (...)' -- else nil. Requires a NON-EMPTY typed prefix (see
this file's header's M91 scope-cut note on why an empty prefix isn't
attempted here).

M91 fix round (V1, severe -- then Y1, final round): the structural
checks below, on their own, also match ORDINARY statement-start typing
-- `wi|' about to become `wire', `as|' -> `assign', `para|' ->
`parameter', ANY reserved word typed one letter at a time, and (with
an even wider net) a task-call identifier like `my_ta|' matching
nothing at all -- because tree-sitter's error recovery produces the
EXACT SAME lone-identifier-inside-an-ERROR shape for every one of
those, indistinguishable from a real instantiation's type name until
more is typed. This function alone cannot tell them apart (this
file's own header already named the EMPTY-prefix version of this
ambiguity; a non-empty prefix turned out to be equally ambiguous, just
with a plausible-looking word attached).

V1's first fix attempt added a keyword-prefix VETO here, checked
before any structural work: if TYPED was itself a prefix of a Verilog
reserved word, this function returned nil unconditionally. Y1 (final
round): a trailing cold read against the real function found that
veto fires BEFORE the real-module-match check ever runs, which means
it also rejects a GENUINE module whose name happens to share a
keyword's own prefix -- a module named `reset_ctrl' typed as `re' was
silently blocked, because `re' also prefixes `real'/`return'/
`release'; likewise `gen_ctrl' typed as `gen' (`generate'/`genvar')
and `as_ctrl' typed as `as' (`assign'/`assert'/`assume'). Long,
descriptive module names sharing a few letters with a reserved word is
routine in the RTL this project targets, so that veto traded one
regression for another -- not a fix.

The veto is GONE, not patched: this function is now purely
STRUCTURAL, exactly the checks below and nothing else. What decides
whether the position is actually claimed lives entirely in `verilog-
complete--handle-instantiation-type-context', via `verilog-complete--
any-module-name-matches-p' -- a real module match always wins, by
construction, because nothing runs before it that could veto it. The
COST problem the keyword list was originally trying to solve (don't
pay `verilog-complete--all-modules''s own full scan on every ordinary
keystroke) is solved differently now: `--any-module-name-matches-p'
answers \"does anything match\" from the already-cached per-file data
(no file I/O for a file this session has already visited any
completion path for), cheap enough to run unconditionally in place of
a veto -- see that function's own docstring, and this file's header's
own \"M91 measured cost\" section for the re-measured numbers.

The residual ambiguity that survives -- a typed prefix that genuinely
DOES match a real module name, even though the user meant an ordinary
signal/variable of their own naming -- is ACCEPTED, not fixed further:
this is the same ambiguity every prefix-based completion source lives
with everywhere else (a matching candidate is offered; the user is
free to ignore the popup and keep typing), and see this file's header
for why an EMPTY prefix specifically is not attempted at all (there
the ambiguity has no typed text to even filter by).

Structural probe (M91, same grammar as the other two context
detectors' own probes) against:
  - `fif' / `fifo ' (nothing else typed yet, first word of a NEW
    statement): tree-sitter's error recovery leaves a bare `ERROR' node
    whose ONLY child is the `simple_identifier' itself, that ERROR's
    own direct parent being `module_declaration' (top-level module
    body), `generate_region', or `generate_block' (an UNLABELED nested
    generate construct, e.g. `generate if (1) begin ... end
    endgenerate' -- a LABELED one, `begin : name ... end', instead
    wraps in `seq_block' under an `always_construct', same as a
    procedural block below, and is correctly excluded) -- all THREE
    listed types are places an instantiation may legally start.
    Checked below via PARENT-then-GRANDPARENT type, mirroring
    `verilog-complete--port-context''s own two-level check.
  - Already-complete, already-parsed `module_instantiation' (editing an
    existing instantiation's own type name in place): the identifier is
    a direct child of `module_instantiation'. M91 fix round (V3): an
    earlier version of this check also required the identifier to be
    THAT `module_instantiation' node's own `instance_type' FIELD
    specifically (via `treesit-node-child-by-field-name'+`treesit-
    node-eq'), reasoning that a `module_instantiation' has OTHER
    `simple_identifier' descendants too (instance names, port
    connections). Probed against TEN shapes total, across two review
    rounds -- a single instance (`fifo u1 ()'), TWO instances sharing
    one type in one statement (`fifo u1 (), u2 ()'), a parameterized
    instantiation (`fifo #(.WIDTH(8)) u1 (), u2 ()'), a generate-LOOP
    instantiation, a package-qualified type name, an IMPLICIT instance
    name, a MALFORMED instance name, `bind', an interface
    instantiation, and an ARRAY of instances -- and in EVERY one, the
    type-name identifier was the ONLY `simple_identifier' whose
    IMMEDIATE parent is `module_instantiation' itself (instance names
    nest inside their own `name_of_instance', port names inside
    `named_port_connection', parameter names inside `named_parameter_
    assignment' -- never a direct child of `module_instantiation').
    No shape was found where the extra field check was ever
    load-bearing, so it was removed as dead code by this evidence, not
    kept as unproven defense-in-depth
    the way this file's OTHER untested guard (the `instance_type'-
    missing black box, see `verilog-complete--handle-port-context's
    own docstring) still is -- that one guards against a SIGNAL
    (`treesit-node-text' on nil), this one guarded against nothing a
    real parse ever produced.
  - `fifo u_' (a SECOND word already following): tree-sitter commits
    this to a `net_declaration' shape (Verilog's own grammar makes
    \"TYPE NAME\" ambiguous between a net declaration and the start of
    an instantiation until a `(' disambiguates it) -- this function
    does NOT recognize that shape at all, on purpose: by the time a
    second word exists, this function is asked about THAT word's own
    position (an arbitrary instance name, never completed by this file)
    or, if asked about the FIRST word again (edited back into), that
    word's parent is now `net_declaration', not `ERROR' nor `module_
    instantiation' -- correctly falls through, no confident answer
    exists there (see this file's own \"can't distinguish, return nil\"
    directive).
  - Excluded by the same two-part check: a plain expression identifier
    (`assign y = fo'; the ERROR there has MANY children, not one -- M91
    fix round (V3): the SPECIFIC shape `assign y = foo bar' at `bar' is
    what proves this child-count-1 guard is load-bearing, not just
    belt-and-suspenders -- `bar' 's own immediate parent IS an ERROR
    directly under `module_declaration' (the SAME grandparent this
    function otherwise accepts), so only the child-count check
    excludes it; removing that check flips this exact shape from nil
    to t), a procedural-statement identifier (`always @(...) begin fo
    end'; the ERROR's own PARENT is `seq_block', not one of the three
    module-item contexts above), and a module's OWN declaration name
    (`module fif'; the ERROR there has TWO children, `module_keyword'
    and the identifier, not one) -- all three were probed and confirmed
    nil."
  (when (> point prefix-start)
    (let* ((parser (treesit-parser-create 'verilog))
           (node (treesit-node-at prefix-start parser)))
      (when (and node
                 (string= (treesit-node-type node) "simple_identifier")
                 (= (treesit-node-start node) prefix-start))
        (let ((parent (treesit-node-parent node)))
          (cond
           ((and parent (string= (treesit-node-type parent) "module_instantiation"))
            t)
           ((and parent
                 (string= (treesit-node-type parent) "ERROR")
                 (= (treesit-node-child-count parent) 1))
            (let ((grandparent (treesit-node-parent parent)))
              (and grandparent
                   (member (treesit-node-type grandparent)
                           '("module_declaration" "generate_region" "generate_block"))
                   t)))
           (t nil)))))))

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

(defun verilog-complete--param-label (param)
  "\"NAME = DEFAULT\" (bare NAME when PARAM has no default text), e.g.
\"WIDTH = 8\" or plain \"WIDTH\" -- mirrors `verilog-complete--port-
label''s own reasoning: showing the default inline answers \"what does
this override actually change\" without a trip to the submodule."
  (let ((name (nth 0 param)) (default (nth 1 param)))
    (if default
        (format "%s = %s" name default)
      name)))

(defun verilog-complete--param-item (param prefix-start)
  "One `show-completion-popup' ITEMS element for PARAM (a `verilog-
complete--parameters-of-module'-shaped pair). Same INSERT/FILTER
convention as `verilog-complete--port-item': the bare NAME, no leading
`.'."
  (let ((name (nth 0 param)))
    (list (verilog-complete--param-label param) name prefix-start name)))

(defun verilog-complete--all-modules ()
  "List of (NAME . SOURCE) across the CURRENT buffer's own top-level
modules (SOURCE nil) and every reachable `verilog-auto--library-files'
entry (SOURCE that file's own `file-name-nondirectory'), buffer first
-- matches `verilog-complete--module-ports''s own buffer-before-library
precedence. NOT de-duplicated by name: a module declared more than
once (buffer + library, or two different library files) is a
legitimate SEPARATE candidate per occurrence, since the engineer needs
to know WHICH file each one is, not just that a name matches. Every
reachable library file is visited (unlike `verilog-complete--library-
entry''s own first-match-wins early exit) -- module-name completion
has to answer \"every name starting with this prefix\", not \"the one
module named exactly this\", so there is no way to stop early; see
this file's header's own \"M91 measured cost\" section, below the M56
port-path numbers, for what this specifically costs (both the
instantiation case this function is on the path for, and the far more
common keyword-prefix-rejection case that `verilog-complete--
instantiation-type-context' answers WITHOUT ever calling this
function at all -- see that function's own docstring)."
  (append
   (mapcar (lambda (m) (cons (verilog-auto--module-name m) nil))
           (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer)))
   (apply #'append
          (mapcar (lambda (path)
                    (let ((alist (verilog-complete--library-file-modules path)))
                      (mapcar (lambda (e) (cons (car e) (file-name-nondirectory path))) alist)))
                  (verilog-auto--library-files)))))

(defun verilog-complete--module-item (entry prefix-start)
  "One `show-completion-popup' ITEMS element for ENTRY (a
`verilog-complete--all-modules'-shaped (NAME . SOURCE) pair). LABEL is
\"NAME (source-file)\" when SOURCE is non-nil, bare NAME for a
buffer-local module (there is no \"(this buffer)\" annotation -- a
module the engineer is looking at right now needs no reminder of where
it lives). UNCHANGED by M123 Part C -- see `verilog-complete--
instantiate-item' for the SECOND item Part C adds right after this
one's own call site, never by editing this function."
  (let* ((name (car entry)) (source (cdr entry))
         (label (if source (format "%s (%s)" name source) name)))
    (list label name prefix-start name)))

;; --- M123 Part C: "instantiate" items -------------------------------------
;;
;; A Verilog engineer typing a module's own name at a statement start
;; (the context `verilog-complete--handle-instantiation-type-context'
;; already claims) almost always wants the WHOLE instantiation, not
;; just the bare type name `verilog-complete--module-item' above
;; offers -- `slang-server' knows this (its own completion items ARE
;; full instantiation snippets, see the M123 spec), but a Verilog user
;; of THIS editor can never see that: `verilog-mode's
;; `local-completion-function' tier (this file) wins over LSP
;; (`lsp.el's own M54 note), and the OTHER server this project targets,
;; `verible-verilog-ls', has no completion at all. So this file grows
;; its own instantiation-skeleton generator, one candidate that reuses
;; the SAME resolved-module readers every other branch here already
;; uses (`verilog-complete--module-ports'/`--module-parameters' --
;; never a second parser), rendered through Part B's `lsp--expand-
;; snippet' (lsp.el) rather than a second insertion mechanism of its
;; own, exactly the way an LSP-sourced snippet completion is rendered.
;;
;; Placement: `verilog-complete--handle-instantiation-type-context'
;; below puts this SECOND, right after the plain-name item for the
;; SAME module, for every matching module -- never on its own, never
;; before the plain item. The plain item's own construction
;; (`verilog-complete--module-item', directly above) is untouched.
;;
;; Snippet-syntax escaping: every piece of LITERAL text folded into the
;; generated snippet (the module name, port/parameter names, parameter
;; defaults) is escaped first (`verilog-complete--snippet-escape') --
;; SystemVerilog identifiers/defaults can legally contain `$' (system-
;; task-adjacent names inside a default expression), and an unescaped
;; `$' there would be misread by `lsp--expand-snippet' as introducing a
;; tab stop of this function's own construction, silently eating real
;; port/parameter text. Only the `${N:...}'/`$0' markers THIS file
;; deliberately writes are ever left unescaped.
;;
;; Column alignment: every `.NAME(EXPR)' connection/override line is
;; padded to `verilog-auto-inst-column' the SAME way
;; `verilog-auto--connection-text' pads an AUTOINST connection line --
;; reused, not reinvented, per this milestone's own instruction to
;; match verilog-auto.el's convention rather than invent a second one.
;;
;; The zero-port, zero-parameter case (a module that resolves but
;; declares no ports and no `#(parameter ...)' header -- a real, if
;; unusual, shape: an empty placeholder or a bind-target module) is
;; OFFERED, not suppressed: the skeleton simply degenerates to `NAME
;; ${0:u_NAME} ();' with no `#(...)' header and an empty port list,
;; still a complete, pastable statement. This file already treats
;; "module resolves at all" as license to offer something everywhere
;; else -- the plain-name item right next to this one doesn't
;; special-case a zero-port module either -- and there is no cheaper
;; way to tell "genuinely zero ports" apart from "this file failed to
;; parse this module's ports" here that would make suppressing it a
;; safer default (the module already came from `verilog-complete--all-
;; modules', which only ever lists modules that DO resolve). Pinned by
;; `instantiate_item_for_a_zero_port_zero_parameter_module_offers_an_
;; empty_shell' in `verilog_complete_tests.rs'.

(defun verilog-complete--snippet-escape (s)
  "S with every `\\' doubled and every `$' escaped to `\\$' -- the exact
inverse of what `lsp--expand-snippet' undoes for those two characters,
so LITERAL identifier/default text this file builds (never received
from a server) can never be misread by that expander as introducing a
tab stop or an escape of its own. Verilog identifiers/defaults can
legally contain `$' but never `\\'; escaping both costs nothing and
covers a default expression more thoroughly than escaping only `$'
would.

M123 fix round (cold review): the \"exact inverse\" claim above used to
be false in practice -- `lsp--expand-snippet' only ever unescaped `\\$',
so a `\\' this function doubled came back out of the round trip STILL
DOUBLED, not restored to one. Fixed on the EXPANDER's side (it now
follows the real LSP/TextMate grammar and unescapes `\\\\'/`\\}' too,
not just `\\$'), not here -- this function's own doubling was already
correct; it just had no counterpart to undo it. See `lsp--expand-
snippet's own docstring for the fix and
`snippet_escape_and_expand_round_trip_a_backslash_bearing_identifier'
(completion_popup_tests.rs) for the pin."
  (let ((out "") (i 0) (len (length s)))
    (while (< i len)
      (let ((c (aref s i)))
        (cond
         ((eq c ?\\) (setq out (concat out "\\\\")))
         ((eq c ?$) (setq out (concat out "\\$")))
         (t (setq out (concat out (char-to-string c))))))
      (setq i (1+ i)))
    out))

(defun verilog-complete--instantiate-label (name ports)
  "\"NAME  (instantiate, N portS)\" -- distinguishable at a glance from
the plain-name item directly above it (`verilog-complete--module-item's
own bare-or-annotated NAME) and states up front that accepting this
one is the bigger action: a whole instantiation statement, not just a
type name. Two spaces between NAME and the parenthetical, matching
this file's own `verilog-complete--port-label'/`--param-label' habit
of a short, scannable annotation rather than a sentence."
  (format "%s  (instantiate, %d port%s)"
          name (length ports) (if (= (length ports) 1) "" "s")))

(defun verilog-complete--instantiate-port-line (port cont-indent)
  "One `.NAME(NAME)' port-connection line for the instantiation
skeleton -- same padded shape `verilog-auto--connection-text' emits
for an AUTOINST connection (`.NAME' padded to `verilog-auto-inst-
column'), except EXPR is always the port's own (escaped) NAME: there
is no already-declared signal to connect to yet, unlike AUTOINST's
fill-in-the-blanks job -- this is a brand-new instantiation."
  (let* ((name (verilog-complete--snippet-escape (nth 0 port)))
         (dotname (concat "." name)))
    (concat cont-indent
            (verilog-auto--pad-to-column dotname verilog-auto-inst-column (length cont-indent))
            "(" name ")")))

(defun verilog-complete--instantiate-param-line (param cont-indent n)
  "One `.NAME(${N:DEFAULT})' parameter-override line -- DEFAULT is
PARAM's own declared default text when it has one, else NAME itself
\(nothing better to suggest\), both escaped. The numbered tab stop is
real snippet syntax: `lsp--expand-snippet' treats an ordinary N here as
a `plain' stop (removed, DEFAULT's text inserted verbatim -- see that
function's own docstring), no different in THIS client's hands from N
being fixed at 1, but correct sequential numbering costs nothing and
matches the shape a real LSP snippet would use, in case a later
milestone adds real multi-stop editing. `.NAME' is column-padded
exactly like a port connection -- see `verilog-complete--instantiate-
port-line'."
  (let* ((name (verilog-complete--snippet-escape (nth 0 param)))
         (default (verilog-complete--snippet-escape (or (nth 1 param) (nth 0 param))))
         (dotname (concat "." name)))
    (concat cont-indent
            (verilog-auto--pad-to-column dotname verilog-auto-inst-column (length cont-indent))
            "(${" (number-to-string n) ":" default "})")))

(defun verilog-complete--instantiate-snippet (name ports params indent)
  "The full snippet-syntax TEXT `verilog-complete--instantiate-item'
hands to `lsp--expand-snippet' -- NAME (escaped) followed by an
optional `#(...)' parameter-override header (omitted entirely when
PARAMS is nil, never emitted empty), a `${0:u_NAME}' default instance
name (the one cursor stop this client surfaces, see `lsp--expand-
snippet's own docstring), and the `(...)' port-connection list (an
empty `()' when PORTS is nil -- see this section's own zero-port
decision above). INDENT is the whitespace already on the statement's
own line; continuation lines (inside `#(...)' and the port list) get
INDENT plus two more spaces, this project's own 2-space step (see
`CLAUDE.md's `verible-verilog-format --indentation_spaces=2' pin)."
  (let* ((cont-indent (concat indent "  "))
         (esc-name (verilog-complete--snippet-escape name))
         (inst-name (verilog-complete--snippet-escape (concat "u_" name)))
         (param-lines (let ((n 0))
                        (mapcar (lambda (p)
                                  (setq n (1+ n))
                                  (verilog-complete--instantiate-param-line p cont-indent n))
                                params)))
         (port-lines (mapcar (lambda (p) (verilog-complete--instantiate-port-line p cont-indent))
                              ports)))
    (concat
     esc-name
     (if param-lines
         (concat " #(\n" (string-join param-lines ",\n") "\n" indent ")")
       "")
     " ${0:" inst-name "} ("
     (if port-lines
         (concat "\n" (string-join port-lines ",\n") "\n" indent ")")
       ")")
     ";")))

(defun verilog-complete--instantiate-item (name prefix-start indent)
  "The M123 Part C \"instantiate\" `show-completion-popup' ITEMS element
for module NAME -- see this section's own header for the full design.
Rendered through `lsp--expand-snippet' (lsp.el): a non-nil OFFSET (the
skeleton's own `${0:...}' default-instance-name stop) becomes an
`\"offset:N\"' `PopupItem' PAYLOAD (the 5th list element -- see
`PopupItem::payload's own doc comment, `editor.rs'), exactly the same
convention `lsp.el's own snippet-bearing items already use; PREFIX-
START/NAME are this item's own START/FILTER, matching `verilog-
complete--module-item's own convention so the SAME typed-prefix filter
narrows both items for a module together as the user keeps typing."
  (let* ((ports (verilog-complete--module-ports name))
         (params (verilog-complete--module-parameters name))
         (label (verilog-complete--instantiate-label name ports))
         (snippet (verilog-complete--instantiate-snippet name ports params indent))
         (expanded (lsp--expand-snippet snippet))
         (text (car expanded))
         (offset (cdr expanded)))
    (if offset
        (list label text prefix-start name (format "offset:%d" offset))
      (list label text prefix-start name))))

(defun verilog-complete--items-for-entry (entry prefix-start indent)
  "The plain-name item for ENTRY (`verilog-complete--module-item',
UNCHANGED) followed by ENTRY's own \"instantiate\" item (M123 Part C,
`verilog-complete--instantiate-item') -- called once per candidate in
`verilog-complete--handle-instantiation-type-context', so every
matching module offers both, plain item always first."
  (list (verilog-complete--module-item entry prefix-start)
        (verilog-complete--instantiate-item (car entry) prefix-start indent)))

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
this already-failing path.

M91: two more detectors are tried, in this order, only after the
port-connection one above has already said nil -- `verilog-complete--
param-context' (a `.' inside `#(...)'), then `verilog-complete--
instantiation-type-context' (a module's own type-name identifier).
Ordering never actually matters between them: all three detectors are
structurally disjoint (see each one's own docstring for exactly which
grammar shape it claims), so at most one of the three ever returns
non-nil for a given position -- this order is simply cheapest-check-
first (both `--port-context' and `--param-context' short-circuit
immediately when `char-before' isn't `.', before ever touching
treesit; `--instantiation-type-context' has no such cheap early exit,
so it goes last)."
  (let* ((point (point))
         (prefix-start (verilog-complete--prefix-start point))
         (mi (verilog-complete--port-context prefix-start)))
    (if mi
        (verilog-complete--handle-port-context mi prefix-start point)
      (let ((param-type-name (verilog-complete--param-context prefix-start)))
        (if param-type-name
            (verilog-complete--handle-param-context param-type-name prefix-start point)
          (if (verilog-complete--instantiation-type-context prefix-start point)
              (verilog-complete--handle-instantiation-type-context prefix-start point)
            nil))))))

(defun verilog-complete--handle-port-context (mi prefix-start point)
  "Body of the port-connection branch of `verilog-complete-at-point' --
see that function's own docstring for the full behavior contract (the
message-vs-popup cases, the never-nil-once-confirmed rule, and the
`instance_type'-missing black-box guard)."
  (let ((type-node (treesit-node-child-by-field-name mi "instance_type")))
    (if (not type-node)
        ;; See `verilog-complete-at-point's docstring's nil-guard note --
        ;; black box, no repro found despite trying, kept as a
        ;; defensive backstop only.
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
        t))))

(defun verilog-complete--handle-param-context (type-name prefix-start point)
  "Body of the M91 parameter-override branch of `verilog-complete-at-
point' -- TYPE-NAME is `verilog-complete--param-context''s own return
value (a string, not a node -- see that function's docstring for why).
Same shape as `verilog-complete--handle-port-context': a popup when
there's something to offer, else a `message' distinguishing \"no
parameter starts with this prefix\" from \"module has no parameters\"
from \"module not found\" -- reusing `verilog-complete--module-found-p'
unchanged (it is already name-only, oblivious to ports vs parameters)."
  (let* ((params (verilog-complete--module-parameters type-name))
         (typed (buffer-substring-no-properties prefix-start point))
         (candidates (verilog-auto--filter
                      (lambda (p) (string-prefix-p typed (nth 0 p)))
                      params))
         (items (mapcar (lambda (p) (verilog-complete--param-item p prefix-start))
                         candidates)))
    (cond
     (items (show-completion-popup items prefix-start))
     (params
      (message "Verilog parameter completion: no parameter of `%s' starts with `%s'" type-name typed))
     ((verilog-complete--module-found-p type-name)
      (message "Verilog parameter completion: module `%s' has no `#(parameter ...)' header" type-name))
     (t
      (message "Verilog parameter completion: module `%s' not found (current buffer or library dirs)" type-name)))
    t))

(defun verilog-complete--handle-instantiation-type-context (prefix-start point)
  "Body of the M91 module-name branch of `verilog-complete-at-point'.
Unlike the port/parameter branches, there is no single \"the module\"
to resolve first -- the candidates ARE the set of module names, from
`verilog-complete--all-modules'. Returns NIL, not t, when nothing
matches -- `verilog-complete--instantiation-type-context' confirming
the POSITION is structurally plausible is not enough on its own to
justify claiming it. No `message' on the no-match path either --
unlike the port/param branches' own \"no candidates\" message (which
fires only once the POSITION is already unambiguous), a nil return
here is a real, silent fall-through to `dabbrev', not a dead end the
user needs explaining.

M91 fix round (Y1, final round): gated on `verilog-complete--any-
module-name-matches-p' FIRST -- a cheap existence check (see its own
docstring) -- before ever building the full, labeled candidate list
via `verilog-complete--all-modules'. This is deliberately a TWO-STEP
lookup, not one: the cheap check answers \"is it worth bothering\"
(the answer for an ordinary keyword-shaped keystroke that matches no
module, the common case), and only once it says yes does this
function pay for the exhaustive, fully-revalidated scan that actually
builds what the popup shows. A t answer from the cheap check does NOT
skip straight to `show-completion-popup' with whatever it happened to
peek at -- `all' below is still built from `verilog-complete--all-
modules' in full, so the ACTUAL candidates shown are always current,
even on the rare occasion the cheap check's own answer was stale (see
its docstring).

M123 Part C: each matching module now contributes TWO items, not one
-- see `verilog-complete--items-for-entry' -- the plain-name item
first, then that module's own \"instantiate\" item, flattened via
`apply' + `append' so the popup sees one flat list in that exact
per-module order."
  (let ((typed (buffer-substring-no-properties prefix-start point)))
    (when (verilog-complete--any-module-name-matches-p typed)
      (let* ((all (verilog-complete--all-modules))
             (candidates (verilog-auto--filter
                          (lambda (e) (string-prefix-p typed (car e)))
                          all))
             (indent (verilog-auto--line-indent prefix-start))
             (items (apply #'append
                           (mapcar (lambda (e) (verilog-complete--items-for-entry e prefix-start indent))
                                   candidates))))
        (when items
          (show-completion-popup items prefix-start)
          t)))))

(provide 'verilog-complete)
