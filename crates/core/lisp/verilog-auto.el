;;; verilog-auto.el --- GNU verilog-mode's AUTO system core (M39) -*- lexical-binding: t -*-

;; M39 brings up the core of GNU verilog-mode's famous AUTO system:
;; `/*AUTOINST*/' (auto-connect an instantiation's ports to same-named
;; signals), `/*AUTOWIRE*/' (auto-declare wires for undeclared
;; instantiated-module outputs), `/*AUTOARG*/' (auto-fill a Verilog-1995
;; style module argument list), and the two commands that drive them,
;; `verilog-auto' (C-c C-a) and `verilog-delete-auto' (C-c C-k).
;;
;; v1 scope is deliberately narrow. EXCLUDED, matching this repo's own
;; disclosure convention (see e.g. treesit.rs's module doc): AUTO_TEMPLATE
;; (per-instance connection templates), AUTOSENSE (`always @*' sensitivity
;; lists -- SystemVerilog's `always_ff'/`always_comb'/`@*' mostly obsolete
;; it anyway), AUTOINPUT/AUTOOUTPUT/AUTOINOUT (top-of-hierarchy port
;; propagation), AUTOREG/AUTORESET/AUTOTIEOFF/AUTOUNUSED (register and
;; tie-off inference), and instance arrays (`u1[3:0] (...)' multi-instance
;; syntax). `verilog-auto' runs when the user asks for it via C-c C-a,
;; and additionally on every save -- opt-in, `verilog-auto-on-save' is
;; nil by default -- via `before-save-hook' (M40; see that variable's
;; own docstring).
;;
;; --- Tree shape notes (M39 probe dumps, tree-sitter-systemverilog 0.4) --
;;
;; A module header is either `module_ansi_header' (ports declared with
;; their directions right there, `input logic [7:0] foo') or
;; `module_nonansi_header' (Verilog-1995 style: just names in the
;; parens, `module foo (a, b)', with `input'/`output'/`inout'
;; declarations for those same names in the module body). AUTOARG only
;; makes sense for the latter; on the former it deliberately expands to
;; nothing (see `verilog-auto--expand-autoarg-site').
;;
;; Comments are ordinary named nodes (`block_comment'/`one_line_comment')
;; that can appear as children pretty much anywhere -- this is what makes
;; `/*AUTOINST*/'-inside-empty-parens and `/*AUTOARG*/'-inside-a-header
;; parse cleanly at all. But an instantiation with an EXPLICIT connection
;; immediately followed by `, /*AUTOINST*/' -- i.e. exactly the shape
;; this whole feature is built around -- parses with `root.has_error' = t:
;; the trailing comma before the comment lands in its own `ERROR' node,
;; a sibling of `list_of_port_connections' rather than nested inside it.
;; Verified (M39 probe) that this recovery stays strictly local: it never
;; disturbs the byte ranges of a second instance later in the same module
;; or the buffer tail. Consequently NONE of this file's logic may gate on
;; `has_error', and every search below walks the tree looking for node
;; KINDS (and, for AUTOINST/AUTOWIRE/AUTOARG markers, exact comment TEXT)
;; rather than assuming a fixed child shape -- `verilog-auto--find-all'/
;; `verilog-auto--find-first' (below) recurse through an ERROR node
;; exactly like any other, so the stray comma never has to be understood,
;; only stepped over.
;;
;; --- Policy choices worth documenting explicitly ------------------------
;;
;; - Parameter substitution in a range like `[WIDTH-1:0]': GNU's own
;;   `verilog-auto-inst-param-value' defaults to nil (never substitute,
;;   always keep the symbolic name) or t (always substitute using the
;;   submodule's OWN parameter defaults). This file takes a middle
;;   ground, hard-coded (no variable to toggle it): a parameter name is
;;   substituted with `(EXPR)' -- parenthesized, to protect precedence --
;;   ONLY when the instantiating instance itself overrides it via
;;   `#(.PARAM(EXPR))'; with no override, the symbolic name is kept
;;   verbatim. This needs no lookup of the submodule's own parameter
;;   declarations at all -- only the instantiation's own override list.
;;   Substitution is whole-word (`\\<PARAM\\>'), so `WIDTH' never matches
;;   inside `WIDTH2', and is a SINGLE left-to-right pass over the
;;   ORIGINAL range text (`verilog-auto--substitute-params') -- never a
;;   separate pass per override applied to the accumulating result,
;;   which used to let one override's own EXPR-TEXT (itself sometimes a
;;   bare reference to another parameter, e.g. `#(.WIDTH(DEPTH),
;;   .DEPTH(4))') get silently re-substituted a second time, with the
;;   result depending on override list order (M39 review fix). Order
;;   independence and no-double-substitution are both a direct
;;   consequence of never rescanning already-produced output, not
;;   special-cased.
;; - AUTOWIRE candidate signals: a submodule output port connected to a
;;   BARE identifier (`.out(foo)') that isn't already declared (as a net,
;;   a variable, or a port of the enclosing module) becomes a candidate
;;   wire. A connection to anything else -- indexed/selected
;;   (`.out(bus[3:0])'), concatenated (`.out({a,b})'), or any other
;;   non-trivial expression -- is skipped; GNU does something similar
;;   (`verilog-auto-wire-type' aside, it never synthesizes a wire for a
;;   non-lvalue-shaped connection either). Two instances driving the same
;;   name dedup to the FIRST one seen (buffer order), keeping its range.
;;   An AUTOWIRE site whose candidate set is empty inserts nothing at all
;;   (no empty Beginning/End markers), matching GNU.
;; - One AUTOWIRE per module (M39 review fix, GNU convention): only the
;;   FIRST `/*AUTOWIRE*/' comment in a given module ever expands; any
;;   later one in the SAME module is left as a bare, unexpanded marker,
;;   and the module's own name is folded into `verilog-auto''s final
;;   message. Blindly expanding every comment independently made every
;;   wire declaration appear once per comment (a real Verilog duplicate-
;;   declaration error as soon as a module had two), and separately left
;;   `verilog-delete-auto' with two fully-expanded Begin/End blocks in
;;   one module to tell apart -- see `verilog-auto--autowire-stale-end's
;;   own header for how that shape, on its own, could make one site's
;;   deletion range silently swallow another's.
;; - AUTOARG on an ALREADY-ansi header (ports declared in the header, so
;;   there is nothing left to fill in) expands to nothing and records a
;;   notice echoed at the end of `verilog-auto' -- see
;;   `verilog-auto--expand-autoarg-site'. A non-ANSI header where the
;;   `/*AUTOARG*/' comment sits alongside OTHER already-explicit arg names
;;   (rather than being the header's only content) is not attempted in v1
;;   -- undefined/unhandled, not silently wrong-but-plausible.
;; - Module resolution order: the current buffer's own top-level
;;   `module_declaration's (by name) first, then every `.v'/`.vh'/`.sv'/
;;   `.svh' file found by a BOUNDED, BREADTH-FIRST recursive walk of each
;;   of `verilog-library-directories' (M56; up to `verilog-library-max-
;;   depth' levels deep, capped at `verilog-library-max-files' total
;;   candidates, `.'-prefixed subdirectories skipped -- see that
;;   variable's own doc string and `verilog-auto--library-files' below
;;   for the full ordering contract), plus -- unless `verilog-library-
;;   use-filelist' is nil -- every file named by a `verible.filelist' at
;;   the project root (`lsp--project-root'), if one exists. Each
;;   `verilog-library-directories' entry is (still) resolved relative to
;;   the CURRENT BUFFER's own file directory (not `default-directory' in
;;   general, which could differ after `cd'/`dired'), EXCLUDING the
;;   current buffer's own visited file (M39
;;   review fix: `verilog-library-directories' defaults to the buffer's
;;   own directory, so without this exclusion a module the user just
;;   deleted from the BUFFER -- but hasn't saved -- would still be found
;;   by silently re-reading the stale on-disk copy of that very file;
;;   the buffer's own in-memory content, already checked first, is the
;;   sole authority for what IT defines. Any OTHER file is still read
;;   from disk regardless of unsaved edits in some OTHER buffer -- out
;;   of v1's scope, and GNU reads library files from disk too). One
;;   hash-table cache (module name -> port list) per `verilog-auto'
;;   invocation; nothing persists across commands, so a library file
;;   edited between two C-c C-a's is always picked up fresh.
;; - A module not found anywhere: that ONE instance's AUTOINST is skipped
;;   (and any of its outputs are simply invisible to AUTOWIRE) and a
;;   notice is recorded; every other AUTO in the buffer still expands
;;   normally. Multiple distinct missing modules collect into one
;;   end-of-command message: first name plus a total count -- see
;;   `verilog-auto''s own echo, which also folds in the ANSI/AUTOARG and
;;   multi-AUTOWIRE notices above, and any `verilog-delete-auto' overlap-
;;   skip count, using the same "can't show two things in one echo
;;   line" reasoning (a plain immediate `(message ...)' at discovery time
;;   would just be invisibly clobbered by whatever runs after it, most
;;   of all `verilog-auto''s own final summary -- so every notice kind
;;   is collected and folded into that one final message instead of
;;   ever being echoed on their own).
;; - `verilog-delete-auto' cross-checks every collected delete range for
;;   overlap with another before deleting anything
;;   (`verilog-auto--overlapping-ranges') and withholds any that overlap,
;;   counting them into its own return value and message. Normal
;;   operation never produces an overlapping range -- this is a
;;   structural safety net (defense in depth) independent of whatever
;;   might otherwise produce one, added after an M39 review found a
;;   concrete way to (a hand-deleted \"// End of automatics\" line;
;;   `verilog-auto--autowire-stale-end's own header has the full story).
;;
;; --- Idempotence and undo ------------------------------------------------
;;
;; `verilog-auto' always starts with `verilog-delete-auto', which strips
;; every existing machine-generated region back to bare AUTO comments,
;; then re-expands everything from scratch -- so running it twice leaves
;; the buffer byte-identical the second time, and `verilog-delete-auto'
;; followed by nothing reproduces the user's own original hand-written
;; text exactly (both pinned by tests). Both commands call
;; `undo-amalgamate-boundary' (M30's mechanism, already used by evil's
;; `o'/`O') before returning so the whole multi-step expansion undoes as
;; one group regardless of how many individual `insert'/`delete-region'
;; calls it took internally.
;;
;; --- AUTOINST's insertion strategy ---------------------------------------
;;
;; Delete-then-expand is a single pass per AUTO category, not a
;; reparse-after-every-single-insertion loop: every AUTOINST (and,
;; separately, every AUTOARG) site in one parse is collected up front and
;; then applied in DESCENDING order of the marker comment's own buffer
;; position -- rightmost first. Every site's own edit is a pure insertion
;; strictly BETWEEN its own comment and its own closing paren, so distinct
;; sites never overlap; processing right-to-left means each insertion
;; only ever shifts positions strictly to ITS OWN right, i.e. positions
;; already fully processed, so no earlier (leftward) site's stored
;; position is ever invalidated -- provably correct with zero reparses,
;; rather than needing either an O(sites) reparse loop or incremental
;; position fixup. AUTOWIRE is a separate pass on a fresh reparse (as
;; specified: it must see AUTOINST's own newly-generated connections),
;; and AUTOARG a further fresh reparse after that.

(defvar verilog-library-directories '(".")
  "Directories `verilog-auto' searches for a module definition it can't
find in the current buffer, in order, each resolved relative to the
CURRENT BUFFER's own file directory (`buffer-file-name', not
`default-directory'). Every `.v'/`.vh'/`.sv'/`.svh' file found by a
bounded, breadth-first recursive walk of a listed directory (M56 --
see `verilog-library-max-depth'/`verilog-library-max-files') is a
candidate; the first one whose top-level `module_declaration' matches
the wanted name wins. See `verilog-auto--library-files' for the exact
ordering contract across directories, depth levels, and (if
`verilog-library-use-filelist' is non-nil) a project-root
`verible.filelist'.")

(defvar verilog-library-max-depth 8
  "Maximum recursion depth `verilog-auto--library-files' descends into
each `verilog-library-directories' entry, counting the entry itself as
depth 0 (so the default of 8 reaches 8 levels of subdirectories below
it). `0' restores the pre-M56 behavior of only scanning the directory
itself, no recursion at all.

The sole reason this has a ceiling instead of scanning arbitrarily
deep: this interpreter has no `file-symlink-p' primitive (or any other
way to detect a symlink), so a directory tree containing a symlink
cycle would otherwise recurse forever. This bound exists to make that
failure mode merely slow-then-stop instead of a hang, not because deep
trees are expensive on their own (a few thousand library files cost
low single-digit milliseconds to scan).")

(defvar verilog-library-max-files 2000
  "Maximum number of candidate files `verilog-auto--library-files'
returns in total, across every `verilog-library-directories' entry and
(if enabled) `verible.filelist'. Collection stops as soon as this many
files have been found; a `message' reports the truncation (naming this
variable) so a real miss caused by it is never silent. Exists for the
same reason `verilog-library-max-depth' does: nothing here can detect a
symlink cycle, so an unbounded collection has no other backstop against
one.")

(defvar verilog-library-use-filelist t
  "Non-nil (the default) to also treat a `verible.filelist' at the
current buffer's project root (`lsp--project-root', lsp.el) as a source
of library files -- `verible-verilog-ls' itself is known to consult
that file when resolving cross-file definitions (M55 header,
verilog-nav.el; M56 probe, `verilog-auto--library-filelist-files''s
own doc string), so honoring it here brings a project's OWN existing
convention into this editor's local module lookup rather than
inventing a second one. What is NOT claimed: that this function's
parsing rules match verible's own byte for byte -- nothing this
project has run establishes how verible treats a malformed or
flag-bearing line, only that a well-formed filelist works.
See `verilog-auto--library-filelist-files' for the parsing rules. Set
to nil to skip reading it entirely (not just ignore its contents --
`file-exists-p'/`file-contents-as-string' are never called on it).")

(defvar verilog-auto-inst-column 40
  "Column `verilog-auto''s AUTOINST expansion pads each `.NAME' to
before opening the connection's own parenthesis (GNU verilog-mode's
own variable of the same name/purpose). Always at least one space is
kept even when NAME itself already reaches or passes this column.")

(defvar verilog-auto-wrap-width 4
  "Extra indentation AUTOARG adds for a wrapped port-list continuation
line, on top of the module header line's own indentation. This is a
CONTINUATION/WRAP width, corresponding to verible-verilog-format's
`--wrap_spaces' flag (default 4) -- deliberately NOT the same quantity
as `standard-indent-width' (verible's `--indentation_spaces', i.e. how
far each nested block steps in, and which as of M73 tracks the current
buffer's own detected style). The two happened to share the same
default (4) before M73, which was a coincidence of verilog-mode's old
hardcoded value, not a design fact -- verible itself keeps the two
flags independent, and M73 made a 2-space-indented buffer regress
AUTOARG's continuation lines to 2, which verible-verilog-format then
rewrote back to 4. Confirmed (M74) that wrap does NOT track block step
either: running verible-verilog-format with a non-default
`--indentation_spaces' still wraps at 4 unless `--wrap_spaces' is also
overridden. So AUTOARG must read a width of its own, not
`standard-indent-width'.

v1 scope cut: this is a fixed default, NOT detected from anything. It
matches verible's own `--wrap_spaces' default, which is what an
unconfigured `verible-verilog-format' run will reformat AUTOARG's
output to. A shop that overrides `--wrap_spaces' in its own format
invocation gets the same class of mismatch M74 just fixed, one flag
over -- there is no per-repo verible format config file to read it
from (the tool takes flags, and `verible.filelist' carries file lists,
not formatting options), so detecting it would mean guessing. Set this
variable in `init.el' instead.")

;; Internal, invocation-scoped state -- see the header above. Always
;; let-bound fresh by `verilog-auto' itself; the top-level `defvar' just
;; gives every helper a variable to dynamically refer to, and a harmless
;; default if one is ever called outside that scope (e.g. a stray manual
;; `M-:').
(defvar verilog-auto--module-cache nil)
(defvar verilog-auto--missing-modules nil)
(defvar verilog-auto--ansi-autoarg-modules nil)
(defvar verilog-auto--multi-autowire-modules nil)

;; --- Generic tree-sitter walking helpers ---------------------------------
;;
;; None of these are specific to Verilog; they exist because the M12/M38
;; primitives (`treesit-node-child'/`treesit-node-child-count'/
;; `treesit-node-parent'/`treesit-node-child-by-field-name') only give
;; index/field/parent access, not the "find me every X" queries this file
;; needs constantly. A `treesit-query-capture' pattern per shape would
;; work too, but a plain recursive walk needs no query-syntax quoting and
;; reads directly against the M39 probe dump notes above.

(defun verilog-auto--find-all (node pred)
  "All descendants of NODE (NODE itself excluded) satisfying PRED (a
function of one node), depth-first, left to right. Never recurses INTO
a node that itself satisfies PRED -- no caller in this file needs a
match nested inside another match of the same shape."
  (let ((n (treesit-node-child-count node)) (i 0) (acc nil))
    (while (< i n)
      (let ((child (treesit-node-child node i)))
        (setq acc
              (append acc
                      (if (funcall pred child)
                          (list child)
                        (verilog-auto--find-all child pred)))))
      (setq i (1+ i)))
    acc))

(defun verilog-auto--find-first (node pred)
  "First descendant of NODE (NODE itself excluded) satisfying PRED,
depth-first, left to right, or nil."
  (let ((n (treesit-node-child-count node)) (i 0) (found nil))
    (while (and (< i n) (not found))
      (let ((child (treesit-node-child node i)))
        (setq found (if (funcall pred child) child (verilog-auto--find-first child pred))))
      (setq i (1+ i)))
    found))

(defun verilog-auto--find-all-of-type (node type)
  (verilog-auto--find-all node (lambda (n) (string= (treesit-node-type n) type))))

(defun verilog-auto--find-first-of-type (node type)
  (verilog-auto--find-first node (lambda (n) (string= (treesit-node-type n) type))))

(defun verilog-auto--comment-p (node text)
  (and (string= (treesit-node-type node) "block_comment")
       (string= (treesit-node-text node) text)))

(defun verilog-auto--find-comment (node text)
  "First TEXT-matching block_comment descendant of NODE, or nil."
  (verilog-auto--find-first node (lambda (n) (verilog-auto--comment-p n text))))

(defun verilog-auto--find-comments (node text)
  "Every TEXT-matching block_comment descendant of NODE."
  (verilog-auto--find-all node (lambda (n) (verilog-auto--comment-p n text))))

(defun verilog-auto--enclosing-of-type (node type)
  "NODE itself if it already has treesit type TYPE, else its nearest
ancestor that does, walking up via `treesit-node-parent'; nil if none
does."
  (let ((n node))
    (while (and n (not (string= (treesit-node-type n) type)))
      (setq n (treesit-node-parent n)))
    n))

(defun verilog-auto--last-child (node)
  "NODE's own last child (by raw child index, so this includes
anonymous tokens -- in particular, the closing paren of an
instantiation or a module header's port list is always its container's
own last child), or nil if NODE has none."
  (let ((n (treesit-node-child-count node)))
    (and (> n 0) (treesit-node-child node (1- n)))))

(defun verilog-auto--child-index (parent child)
  "CHILD's index among PARENT's own children (by `treesit-node-eq'), or
nil if CHILD isn't one of them."
  (let ((n (treesit-node-child-count parent)) (i 0) (found nil))
    (while (and (< i n) (not found))
      (when (treesit-node-eq (treesit-node-child parent i) child)
        (setq found i))
      (setq i (1+ i)))
    found))

(defun verilog-auto--next-sibling (node)
  "NODE's next sibling (by raw child index, so this can return an
anonymous token), or nil if NODE has none or is the root."
  (let ((parent (treesit-node-parent node)))
    (when parent
      (let ((idx (verilog-auto--child-index parent node)))
        (when (and idx (< (1+ idx) (treesit-node-child-count parent)))
          (treesit-node-child parent (1+ idx)))))))

(defun verilog-auto--node-column (node)
  "The buffer column NODE starts at."
  (save-excursion (goto-char (treesit-node-start node)) (current-column)))

(defun verilog-auto--line-indent (pos)
  "The whitespace prefix of POS's own line, from the line's start up to
POS itself. Assumes POS is the first non-blank thing on its line --
true of every AUTOWIRE comment this file's convention expects (GNU's
own convention too: AUTOWIRE sits alone on its own declaration line)."
  (save-excursion
    (goto-char pos)
    (buffer-substring (line-beginning-position) pos)))

(defun verilog-auto--filter (pred list)
  (let (acc)
    (dolist (x list (nreverse acc))
      (when (funcall pred x) (push x acc)))))

;; --- Parsing entry points -------------------------------------------------

(defun verilog-auto--parse-current-buffer ()
  "Fresh root node for the current buffer's full text."
  (treesit-parser-root-node (treesit-parser-create 'verilog)))

(defun verilog-auto--parse-string (text)
  "Fresh root node for TEXT, not associated with any buffer (library
files -- see `verilog-auto--find-module-in-libraries')."
  (treesit-parse-string 'verilog text))

(defun verilog-auto--top-level-modules (root)
  (verilog-auto--find-all-of-type root "module_declaration"))

(defun verilog-auto--header-node (module-decl)
  "MODULE-DECL's own module_ansi_header or module_nonansi_header
child."
  (or (verilog-auto--find-first-of-type module-decl "module_ansi_header")
      (verilog-auto--find-first-of-type module-decl "module_nonansi_header")))

(defun verilog-auto--module-name (module-decl)
  (treesit-node-text
   (treesit-node-child-by-field-name (verilog-auto--header-node module-decl) "name")))

(defun verilog-auto--header-port-list (header)
  "HEADER's own port-parens container (`list_of_ports' for a
non-ANSI header, `list_of_port_declarations' for an ANSI one) -- NOT
the same as HEADER's own last child, which is the header's trailing
`;', past the closing paren this is used to find (M39 tree dump:
`module_ansi_header'/`module_nonansi_header' both end `... ) ;', the
`)' one level down inside this port-list node, not a direct child of
HEADER itself)."
  (or (verilog-auto--find-first-of-type header "list_of_ports")
      (verilog-auto--find-first-of-type header "list_of_port_declarations")))

;; --- Module lookup (current buffer, then library directories) -----------

(defun verilog-auto--find-module-in-buffer (name)
  (let ((mods (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer)))
        (found nil))
    (while (and mods (not found))
      (when (string= (verilog-auto--module-name (car mods)) name)
        (setq found (car mods)))
      (setq mods (cdr mods)))
    found))

(defun verilog-auto--library-dirs ()
  "Absolute library directories: `verilog-library-directories', each
resolved relative to the current buffer's own file directory (falling
back to `default-directory' for a buffer that visits no file)."
  (let ((base (or (and (buffer-file-name) (file-name-directory (buffer-file-name)))
                  (default-directory))))
    (mapcar (lambda (d) (expand-file-name d base)) verilog-library-directories)))

(defun verilog-auto--library-file-name-p (name)
  (or (string-suffix-p ".v" name) (string-suffix-p ".vh" name)
      (string-suffix-p ".sv" name) (string-suffix-p ".svh" name)))

(defun verilog-auto--library-files-bfs-dir (dir add)
  "Breadth-first walk of DIR (an absolute, existing directory) up to
`verilog-library-max-depth' levels deep (DIR itself is depth 0),
`.'-prefixed subdirectories (including `.'/`..' themselves) never
descended into. Every `.v'/`.vh'/`.sv'/`.svh' file found is handed to
\(funcall ADD PATH), in this exact order: DIR's own depth-0 files
first, in `directory-files' order, before ANY depth-1 file, which in
turn all come before any depth-2 file, and so on -- so the pre-M56
behavior (`verilog-library-max-depth' bound to 0) is a guaranteed
PREFIX of this order, never a reordering of it. ADD may perform a
non-local exit (`verilog-auto--library-files' throws
`verilog-auto--library-files-full' once `verilog-library-max-files' is
reached), which unwinds straight out of this function too -- there is
no local catch here for it.

Cost note, honestly recorded rather than fixed: QUEUE is grown with
`(append queue (nreverse subdirs))' once per directory node processed,
an O(current queue length) copy every time -- so a tree with very MANY
directories and very FEW files per directory pays an O(directories^2)
total cost that `verilog-library-max-files' (which counts FILES, not
directories visited) does nothing to cap. Not attempted here: no real
RTL tree this project has reason to target looks like that (source
trees are files-heavy, not directories-heavy), and this project's own
rule against unproven optimization applies just as much to a queue
data-structure swap as to anything else -- recorded so a future
profile that DOES find this hot has the shape of the fix already
pointed out, not so it looks unnoticed."
  (let ((queue (list (cons dir 0))))
    (while queue
      (let* ((entry (pop queue))
             (cur-dir (car entry))
             (cur-depth (cdr entry))
             (subdirs nil))
        (dolist (name (directory-files cur-dir))
          (unless (or (string= name ".") (string= name ".."))
            (let ((path (expand-file-name name cur-dir)))
              (if (file-directory-p path)
                  (when (and (< cur-depth verilog-library-max-depth)
                             (not (string-prefix-p "." name)))
                    (push (cons path (1+ cur-depth)) subdirs))
                (when (verilog-auto--library-file-name-p name)
                  (funcall add path))))))
        (setq queue (append queue (nreverse subdirs)))))))

(defun verilog-auto--library-filelist-files ()
  "Absolute paths named by a `verible.filelist' at the current buffer's
project root (`lsp--project-root', lsp.el) -- filtered to paths that
both pass `verilog-auto--library-file-name-p' and `file-exists-p' --
or nil if no such file exists (or `verilog-library-use-filelist' is
nil, in which case not even `file-exists-p' on the filelist itself is
ever called).

`lsp--project-root' takes a FILE, not a directory, and always returns
SOME directory (the file's own, if no project marker is found upward
-- never nil), so a buffer visiting no file gets a made-up filename
under `default-directory' purely to feed that argument; the returned
root is never nil-tested, only checked for whether it happens to
contain `verible.filelist'.

Each line of the filelist is `string-trim'med; blank lines, `#'- or
`//'-prefixed comments, and `+'/`-'-prefixed tool flags
\(`+incdir+...', `-f ...' -- not file paths, and not even partially
parsed: the WHOLE line is dropped, no attempt is made to split a
flag-with-argument line like `-y libdir' into flag and path) are
skipped. Every remaining line is `expand-file-name'-resolved relative
to the ROOT itself.

That resolution rule (relative to ROOT, not the filelist's own
directory -- the same thing here, since the filelist is always read
from directly inside ROOT) is not a guess: 2026-08-10, `dev/lsp-probe.py'
against a real `verible-verilog-ls' binary, on a nested fixture
\(`rtl/top.v' instantiating `rtl/core/alu_pipelined_stage.v'\) --
WITHOUT any `verible.filelist' at the project root,
`textDocument/definition' on the instantiated module's type name came
back `[]' (nothing). Dropping a `verible.filelist' at the SAME
directory `rootUri' points at, listing every source file with paths
relative to THAT directory (`top.v', `core/alu_pipelined_stage.v', ...,
no `./' prefix needed), made the identical request correctly return
`core/alu_pipelined_stage.v''s own location -- and did so with NO
`textDocument/didOpen' ever sent for the TARGET file: the filelist
alone is enough for verible to resolve a definition into a file the
client never opened. (The file the request is made FROM -- `top.v'
here -- is necessarily opened first; `dev/lsp-probe.py' always sends a
`didOpen' for its own `--file' before any request, and no probe can
avoid that. An earlier draft of this paragraph claimed neither file
was opened, which is not what was run.) This function's own
root-relative resolution mirrors that observed behavior directly, not
a citation of verible's own documentation (which this project doesn't
ship or vendor) -- and note the fixture cannot separate `relative to
ROOT' from `relative to the filelist's own directory', since the
filelist sits directly in ROOT in every case tried.

Known limitation, accepted rather than fixed: if a `lsp--project-root-
markers' hit (`.git' is the common case) sits BETWEEN the buffer and
the directory actually holding `verible.filelist', `lsp--project-root'
stops at the nearer marker and this function never reaches the
filelist at all -- silently, same as every other `verilog-library-*'
gap in this file. This is deliberately not treated as a bug: it is the
SAME root `verible-verilog-ls' itself would compute from the same
`rootUri' algorithm, so this function's blind spot matches the
server's own, rather than second-guessing it with a different, only
locally-motivated project-root heuristic."
  (when verilog-library-use-filelist
    (let* ((probe-file (if (buffer-file-name)
                            (buffer-file-name)
                          (expand-file-name "verilog-auto--filelist-probe"
                                             (default-directory))))
           (root (lsp--project-root probe-file))
           (filelist-path (expand-file-name "verible.filelist" root)))
      (when (file-exists-p filelist-path)
        (let ((text (condition-case nil
                        (file-contents-as-string filelist-path)
                      (error nil)))
              acc)
          (when text
            (dolist (raw (split-string text "\n"))
              (let ((line (string-trim raw)))
                (unless (or (string= line "")
                            (string-prefix-p "#" line)
                            (string-prefix-p "//" line)
                            (string-prefix-p "+" line)
                            (string-prefix-p "-" line))
                  (let ((path (expand-file-name line root)))
                    (when (and (verilog-auto--library-file-name-p
                                (file-name-nondirectory path))
                               (file-exists-p path))
                      (push path acc)))))))
          (nreverse acc))))))

(defun verilog-auto--library-files ()
  "Library-file candidates for module lookup, EXCLUDING the current
buffer's own visited file (compared as absolute paths) -- see the M39
review note below for why that exclusion exists, unchanged by M56.

Ordering (M56): each `verilog-library-directories' entry contributes
its own whole breadth-first subtree (`verilog-auto--library-files-bfs-
dir') before the next entry's tree starts -- i.e. directory-list order
is the OUTERMOST sort key, depth is the INNERMOST one, exactly matching
this variable's own doc string. If `verilog-library-use-filelist' is
non-nil, every NEW path (`verilog-auto--library-filelist-files', not
already found above -- a `verible.filelist' entry that duplicates a
directory-scan hit is silently deduplicated, first occurrence wins) is
appended after ALL directory-scan results, in filelist order.
Collection stops as soon as `verilog-library-max-files' distinct paths
have been kept, with one `message' noting the truncation -- see that
variable's own doc string for why an unbounded scan isn't safe here
even though the cost of a bounded one is negligible.

M39 review fix (severity: stale disk content silently wins over an
unsaved edit): `verilog-library-directories' defaults to `(\".\")', the
buffer's own directory, so without this exclusion a module the user
just deleted FROM THE BUFFER (but hasn't saved yet) would still be
found by re-reading the OLD on-disk copy of the very file the buffer
itself visits -- `verilog-auto--find-module-in-buffer' (checked first)
correctly sees the deletion, but this function used to hand the stale
disk copy right back on the very next lookup, with no warning at all.
The buffer's own in-memory content is the sole authority for what it
defines; any OTHER file is still read straight from disk regardless of
whether some OTHER buffer has unsaved edits to it -- that's out of
v1's scope, and GNU verilog-mode reads library files from disk too.
This exclusion applies identically to a subdirectory file reached by
M56's recursion or a file brought in only via `verible.filelist' -- it
is a plain absolute-path string comparison, with no dependence on
which of the three sources found the path."
  (let ((own (and (buffer-file-name) (expand-file-name (buffer-file-name))))
        (seen (make-hash-table :test 'equal))
        (count 0)
        acc)
    (catch 'verilog-auto--library-files-full
      (let ((add (lambda (path)
                   (unless (or (and own (string= path own)) (gethash path seen))
                     (when (>= count verilog-library-max-files)
                       (message
                        "Verilog library scan: stopped at %d files (verilog-library-max-files); results may be incomplete"
                        verilog-library-max-files)
                       (throw 'verilog-auto--library-files-full nil))
                     (puthash path t seen)
                     (push path acc)
                     (setq count (1+ count))))))
        (dolist (dir (verilog-auto--library-dirs))
          (when (file-directory-p dir)
            (verilog-auto--library-files-bfs-dir dir add)))
        (dolist (path (verilog-auto--library-filelist-files))
          (funcall add path))))
    (nreverse acc)))

(defun verilog-auto--find-module-in-libraries (name)
  (let ((files (verilog-auto--library-files)) (found nil))
    (while (and files (not found))
      (let* ((path (car files))
             (text (condition-case nil (file-contents-as-string path) (error nil))))
        (when text
          (let ((mods (verilog-auto--top-level-modules (verilog-auto--parse-string text))))
            (while (and mods (not found))
              (when (string= (verilog-auto--module-name (car mods)) name)
                (setq found (car mods)))
              (setq mods (cdr mods))))))
      (setq files (cdr files)))
    found))

(defun verilog-auto--module-ports (name)
  "Port list for module NAME: (NAME DIRECTION RANGE-TEXT) triples,
declaration order (DIRECTION a symbol, `input'/`output'/`inout';
RANGE-TEXT the submodule's own packed-dimension text verbatim, or nil).
Looks in the current buffer's own module declarations first, then
`verilog-library-directories'; cached in `verilog-auto--module-cache'.
Records NAME in `verilog-auto--missing-modules' (once) and returns nil
if it can't be found anywhere."
  (let ((cached (gethash name verilog-auto--module-cache 'verilog-auto--miss)))
    (if (not (eq cached 'verilog-auto--miss))
        cached
      (let* ((node (or (verilog-auto--find-module-in-buffer name)
                        (verilog-auto--find-module-in-libraries name)))
             (ports (and node (verilog-auto--ports-of-module node))))
        (puthash name ports verilog-auto--module-cache)
        (unless node
          (push name verilog-auto--missing-modules))
        ports))))

;; --- Port extraction: ANSI and non-ANSI headers --------------------------

(defun verilog-auto--port-direction-of (node)
  "'input/'output/'inout from the port_direction descendant of NODE, or
'input as a best-effort fallback if NODE has none (an ANSI port that
omits its own direction inherits the previous port's per the LRM; v1
doesn't track that carry-over, so this documented fallback stands in
-- not exercised by any v1 test, every ANSI port here declares its own
direction)."
  (let ((pd (verilog-auto--find-first-of-type node "port_direction")))
    (if pd (intern (treesit-node-text pd)) 'input)))

(defun verilog-auto--range-text-of (node)
  (let ((pdim (verilog-auto--find-first-of-type node "packed_dimension")))
    (and pdim (treesit-node-text pdim))))

(defun verilog-auto--one-ansi-port (decl)
  (list (treesit-node-text (treesit-node-child-by-field-name decl "port_name"))
        (verilog-auto--port-direction-of decl)
        (verilog-auto--range-text-of decl)))

(defun verilog-auto--ansi-ports (header)
  (mapcar #'verilog-auto--one-ansi-port
          (verilog-auto--find-all-of-type header "ansi_port_declaration")))

(defun verilog-auto--nonansi-port-info (module-decl)
  "Alist-shaped list of (NAME DIRECTION RANGE-TEXT) from MODULE-DECL's
own body port_declaration items (input_declaration/output_declaration/
inout_declaration), one entry per declared identifier. Deliberately
scoped to each declaration's OWN `list_of_port_identifiers' rather than
a blanket search of the whole declaration -- a range like
`[WIDTH-1:0]' also contains a `simple_identifier' (for `WIDTH' itself),
which a broader search would wrongly collect as if it were a port
name."
  (let (acc)
    (dolist (kind '(("input_declaration" . input)
                    ("output_declaration" . output)
                    ("inout_declaration" . inout)))
      (dolist (decl (verilog-auto--find-all-of-type module-decl (car kind)))
        (let* ((range (verilog-auto--range-text-of decl))
               (idlist (verilog-auto--find-first-of-type decl "list_of_port_identifiers"))
               (names (and idlist
                           (mapcar #'treesit-node-text
                                   (verilog-auto--find-all-of-type idlist "simple_identifier")))))
          (dolist (nm names)
            (push (list nm (cdr kind) range) acc)))))
    (nreverse acc)))

(defun verilog-auto--nonansi-ports (module-decl header)
  "Non-ANSI port list: names/order from HEADER's own `list_of_ports',
direction+range looked up (by name) from the body's port_declaration
items."
  (let ((names (mapcar #'treesit-node-text (verilog-auto--find-all-of-type header "port")))
        (info (verilog-auto--nonansi-port-info module-decl)))
    (mapcar (lambda (nm)
              (let ((entry (assoc nm info)))
                (list nm
                      (if entry (nth 1 entry) 'input)
                      (if entry (nth 2 entry) nil))))
            names)))

(defun verilog-auto--ports-of-module (module-decl)
  (let ((header (verilog-auto--header-node module-decl)))
    (if (string= (treesit-node-type header) "module_ansi_header")
        (verilog-auto--ansi-ports header)
      (verilog-auto--nonansi-ports module-decl header))))

;; --- Parameter override substitution --------------------------------------

(defun verilog-auto--instance-param-overrides (module-instantiation)
  "Alist of (PARAM-NAME . EXPR-TEXT) from MODULE-INSTANTIATION's own
#(...) parameter overrides (`parameter_value_assignment'), or nil if it
has none. Neither a `named_parameter_assignment's param name nor its
value expression has a treesit field in this grammar (M39 probe dump),
so both are found by node TYPE (`simple_identifier'/`param_expression')
instead of a fixed child index."
  (let ((pva (verilog-auto--find-first-of-type module-instantiation "parameter_value_assignment")))
    (when pva
      (mapcar
       (lambda (npa)
         (cons (treesit-node-text (verilog-auto--find-first-of-type npa "simple_identifier"))
               (treesit-node-text (verilog-auto--find-first-of-type npa "param_expression"))))
       (verilog-auto--find-all-of-type pva "named_parameter_assignment")))))

(defun verilog-auto--substitute-params (text overrides)
  "TEXT (a range-text string like \"[WIDTH-1:0]\") with each whole-word
parameter name replaced by \"(EXPR)\" per OVERRIDES -- an alist of
\(PARAM-NAME . EXPR-TEXT), as from `verilog-auto--instance-param-overrides'.
Whole-word (`\\<...\\>'), so `WIDTH' never matches inside `WIDTH2'.
With no matching override, TEXT comes back verbatim.

M39 review fix (severity: silently wrong RTL): this used to apply each
override in turn to the ACCUMULATING result, which is a real bug for a
combined override like `#(.WIDTH(DEPTH), .DEPTH(4))' -- EXPR-TEXT can
itself legitimately be another parameter's bare name, and the DEPTH
override would then also match (and re-substitute) the \"DEPTH\" that
WIDTH's OWN substitution had just inserted, giving `[((4))-1:0]'
instead of `[(DEPTH)-1:0]' -- with the result depending on override
list ORDER, which it must never do (each override is independent;
GNU's `#(...)' argument order carries no such meaning). Fixed by
building ONE combined `\\<WIDTH\\|DEPTH\\>'-shaped regex and doing a
SINGLE left-to-right pass over the ORIGINAL text, substituting each
match as it's found and never rescanning any already-substituted
output -- so one override's EXPR-TEXT occurring inside another's is
simply never seen a second time, and the result no longer depends on
OVERRIDES' own order at all."
  (if (null overrides)
      text
    (let* ((pat (concat "\\<\\(" (string-join (mapcar (lambda (ov) (regexp-quote (car ov))) overrides)
                                               "\\|")
                         "\\)\\>"))
           (pos 0) (len (length text)) (out ""))
      (while (and (< pos len) (string-match pat text pos))
        (let* ((ms (match-beginning 0))
               (me (match-end 0))
               (expr (cdr (assoc (match-string 0 text) overrides))))
          (setq out (concat out (substring text pos ms) "(" expr ")"))
          (setq pos me)))
      (concat out (substring text pos)))))

;; --- Shared Outputs/Inouts/Inputs grouping and line formatting -----------

(defun verilog-auto--group-by-direction (ports)
  "PORTS (a list of (NAME DIRECTION RANGE) triples) split into three
lists (OUTPUTS INOUTS INPUTS), each preserving PORTS' own relative
order -- the grouping both AUTOINST and AUTOARG use."
  (let (outputs inouts inputs)
    (dolist (p ports)
      (cond ((eq (nth 1 p) 'output) (push p outputs))
            ((eq (nth 1 p) 'inout) (push p inouts))
            (t (push p inputs))))
    (list (nreverse outputs) (nreverse inouts) (nreverse inputs))))

(defun verilog-auto--pad-to-column (s col &optional offset)
  "S with trailing spaces so it reaches column COL, measuring S as
starting at column OFFSET (default 0) -- callers pass OFFSET as the
length of whatever prefix (e.g. indentation) precedes S on the real
buffer line but isn't part of S itself. At least one space is always
added."
  (concat s (make-string (max 1 (- col (+ (or offset 0) (length s)))) ?\s)))

(defun verilog-auto--grouped-lines (groups indent format-fn)
  "GROUPS is (OUTPUTS INOUTS INPUTS); FORMAT-FN maps one port triple to
its own un-indented, uncomma'd text. Returns the finished list of lines
-- an INDENT + \"// Outputs\"/\"// Inouts\"/\"// Inputs\" header before
each non-empty group (a group with zero members contributes no header
and no lines at all), then INDENT + (FORMAT-FN PORT) + \",\" per
member -- except the very last connection line overall, which gets no
comma. Shared by AUTOINST and AUTOARG, whose grouping/comma/empty-group
rules are identical.

Note on the comma strip below: this deliberately does NOT use GNU's
own idiom `(setcar (last list) ...)' -- this interpreter's `last'
builds a fresh spliced-off list rather than returning a shared tail of
the original (unlike real Emacs), so `setcar' on its result wouldn't
touch LINES at all. Stripping the first element of the STILL-REVERSED
accumulator (which is exactly the last line in final order, since it
was the most recently `push'ed) sidesteps that entirely."
  (let ((labels '("// Outputs" "// Inouts" "// Inputs"))
        (gs groups)
        (lines nil))
    (while gs
      (let ((group (car gs)) (label (car labels)))
        (when group
          (push (concat indent label) lines)
          (dolist (p group)
            (push (concat indent (funcall format-fn p) ",") lines))))
      (setq gs (cdr gs) labels (cdr labels)))
    (when lines
      (setcar lines (substring (car lines) 0 (1- (length (car lines))))))
    (nreverse lines)))

;; --- AUTOINST --------------------------------------------------------------

(defun verilog-auto--connection-text (port overrides indent)
  "\".NAME  (EXPR)\" (INDENT NOT included in the returned text -- only
in the padding measurement, via `verilog-auto--pad-to-column's OFFSET;
the caller, `verilog-auto--grouped-lines', prepends INDENT itself to
every line uniformly, connections and group headers alike). EXPR is
NAME alone for a rangeless port, else NAME with its (param-substituted)
range appended, e.g. \"count[WIDTH-1:0]\"."
  (let* ((name (nth 0 port))
         (range (nth 2 port))
         (expr (if range (concat name (verilog-auto--substitute-params range overrides)) name))
         (dotname (concat "." name)))
    (concat (verilog-auto--pad-to-column dotname verilog-auto-inst-column (length indent))
            "(" expr ")")))

(defun verilog-auto--inst-lines (groups overrides indent)
  (verilog-auto--grouped-lines
   groups indent
   (lambda (p) (verilog-auto--connection-text p overrides indent))))

(defun verilog-auto--expand-autoinst-site (module-instantiation comment)
  "Expand the /*AUTOINST*/ site marked by COMMENT (a descendant of
MODULE-INSTANTIATION). Already-explicit connections are left alone and
excluded from the generated set; if that leaves nothing to add (every
port already connected), nothing at all is inserted. Returns 1 if the
instantiated module was found (whether or not anything was actually
inserted), 0 if it couldn't be resolved (GNU warn-and-skip; see
`verilog-auto--module-ports')."
  (let* ((type-name (treesit-node-text
                     (treesit-node-child-by-field-name module-instantiation "instance_type")))
         (ports (verilog-auto--module-ports type-name)))
    (if (member type-name verilog-auto--missing-modules)
        0
      (let* ((hier (verilog-auto--enclosing-of-type comment "hierarchical_instance"))
             (connected
              (mapcar (lambda (c) (treesit-node-text (treesit-node-child-by-field-name c "port_name")))
                      (verilog-auto--find-all-of-type hier "named_port_connection")))
             (overrides (verilog-auto--instance-param-overrides module-instantiation))
             (remaining (verilog-auto--filter
                         (lambda (p) (not (member (nth 0 p) connected)))
                         ports))
             (groups (verilog-auto--group-by-direction remaining))
             (open-paren (treesit-node-child hier 1))
             (indent (make-string (1+ (verilog-auto--node-column open-paren)) ?\s))
             (lines (verilog-auto--inst-lines groups overrides indent)))
        (when lines
          (goto-char (treesit-node-end comment))
          (insert "\n" (string-join lines "\n")))
        1))))

(defun verilog-auto--expand-all-autoinst ()
  "Expand every pending /*AUTOINST*/ site in the current buffer from a
single parse (see this file's header for why processing them
rightmost-first needs no reparse between sites). Returns the count of
instances processed."
  (let* ((root (verilog-auto--parse-current-buffer))
         (mis (verilog-auto--find-all-of-type root "module_instantiation"))
         (sites nil))
    (dolist (mi mis)
      (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
        (when c (push (list (treesit-node-start c) mi c) sites))))
    (setq sites (sort sites (lambda (a b) (> (car a) (car b)))))
    (let ((total 0))
      (dolist (site sites total)
        (setq total (+ total (verilog-auto--expand-autoinst-site (nth 1 site) (nth 2 site))))))))

;; --- AUTOWIRE ---------------------------------------------------------------

(defun verilog-auto--declared-names (module-decl)
  "Every signal name currently declared in MODULE-DECL: net_declaration
wires, data_declaration variables, and this module's own ports (ANSI
and non-ANSI). Each shape is walked narrowly rather than blanket-
searching for `simple_identifier': a `net_decl_assignment'/
`variable_decl_assignment' can carry a `= initializer' expression that
itself references OTHER names (M39 dump: `logic y = x;' nests a
`simple_identifier' for `x' inside `y's own `variable_decl_assignment'),
so only each assignment's OWN name (its first child for
`net_decl_assignment', which has no treesit field for it; the `name'
field for `variable_decl_assignment', which does) is ever collected."
  (let (acc)
    (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
      (push (treesit-node-text (treesit-node-child n 0)) acc))
    (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
      (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc))
    (dolist (n (verilog-auto--find-all-of-type module-decl "list_of_port_identifiers"))
      (dolist (id (verilog-auto--find-all-of-type n "simple_identifier"))
        (push (treesit-node-text id) acc)))
    (dolist (n (verilog-auto--find-all-of-type module-decl "ansi_port_declaration"))
      (push (treesit-node-text (treesit-node-child-by-field-name n "port_name")) acc))
    (nreverse acc)))

(defun verilog-auto--bare-identifier-p (text)
  "Non-nil if TEXT (a connection expression's own exact text) is
nothing but a plain identifier -- `foo(bar)'-shaped calls and any other
composite expression never match, sidestepping any need to know this
grammar's exact nested shape for `bus[3:0]' or `{a,b}'."
  (and (> (length text) 0) (string-match-p "^[A-Za-z_$][A-Za-z0-9_$]*$" text)))

(defun verilog-auto--expand-autowire-site (comment)
  "Expand one /*AUTOWIRE*/ site. Returns the number of wire
declarations inserted (0 if the candidate set is empty -- no
Beginning/End markers are inserted in that case, GNU style)."
  (let* ((module-decl (verilog-auto--enclosing-of-type comment "module_declaration"))
         (declared (verilog-auto--declared-names module-decl))
         (insts (verilog-auto--find-all-of-type module-decl "module_instantiation"))
         (seen (make-hash-table :test 'equal))
         (candidates nil))
    (dolist (mi insts)
      (let* ((type-name (treesit-node-text
                          (treesit-node-child-by-field-name mi "instance_type")))
             (ports (verilog-auto--module-ports type-name))
             (overrides (verilog-auto--instance-param-overrides mi))
             (hier (verilog-auto--find-first-of-type mi "hierarchical_instance")))
        (when hier
          (dolist (conn (verilog-auto--find-all-of-type hier "named_port_connection"))
            (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                   (cnode (treesit-node-child-by-field-name conn "connection"))
                   (ctext (string-trim (treesit-node-text cnode)))
                   (pinfo (assoc pname ports)))
              (when (and pinfo
                         (eq (nth 1 pinfo) 'output)
                         (verilog-auto--bare-identifier-p ctext)
                         (not (member ctext declared))
                         (not (gethash ctext seen)))
                (puthash ctext t seen)
                (push (list ctext (verilog-auto--substitute-params (or (nth 2 pinfo) "") overrides))
                      candidates)))))))
    (setq candidates (nreverse candidates))
    (when candidates
      (let ((indent (verilog-auto--line-indent (treesit-node-start comment))))
        (goto-char (treesit-node-end comment))
        (insert
         "\n" indent "// Beginning of automatic wires (for undeclared instantiated-module outputs)"
         (mapconcat
          (lambda (c)
            (concat "\n" indent
                    (if (string-empty-p (nth 1 c))
                        (format "wire %s;" (nth 0 c))
                      (format "wire %s %s;" (nth 1 c) (nth 0 c)))))
          candidates "")
         "\n" indent "// End of automatics")))
    (length candidates)))

(defun verilog-auto--first-autowire-per-module (comments)
  "COMMENTS (block_comment nodes, /*AUTOWIRE*/, in left-to-right
document order) split into (FIRSTS . EXTRAS): FIRSTS has exactly one
comment per enclosing module (the textually first), EXTRAS every
comment after that in a module with more than one. Grouped by each
module_declaration's own start position (a plain integer, stable and
directly comparable within one parse) rather than the module node
value itself -- two separate `treesit-node-parent'/`enclosing-of-type'
calls for the SAME underlying node produce distinct `Value::Ext'
wrapper objects here, so `eq'/`equal' can't identify \"same node\" the
way `treesit-node-eq' can, and a hash key needs something plainer than
that anyway."
  (let ((seen (make-hash-table :test 'eql))
        (firsts nil) (extras nil))
    (dolist (c comments)
      (let* ((m (verilog-auto--enclosing-of-type c "module_declaration"))
             (key (and m (treesit-node-start m))))
        (if (and key (gethash key seen))
            (push c extras)
          (progn
            (when key (puthash key t seen))
            (push c firsts)))))
    (cons (nreverse firsts) (nreverse extras))))

(defun verilog-auto--expand-all-autowire ()
  "Expand the FIRST /*AUTOWIRE*/ site in each module -- GNU convention
is one module, one AUTOWIRE. A module with more than one comment
leaves every comment after the first as a bare, unexpanded marker and
records its own name in `verilog-auto--multi-autowire-modules' (folded
into `verilog-auto''s final message).

M39 review fix (severity: silent duplicate declarations, and a related
data-corruption path elsewhere): blindly expanding every AUTOWIRE
comment in a module independently, each scanning the WHOLE module's
instantiations, made every wire declaration appear once PER comment --
i.e. duplicated (a real `wire foo; wire foo;' Verilog error) as soon as
a module had two. Worse, TWO fully-expanded AUTOWIRE blocks in one
module is exactly the shape `verilog-auto--autowire-stale-end's own
boundary check now has to defend `verilog-delete-auto' against (a
hand-edited-away \"// End of automatics\" line can make one site's
scan misidentify a LATER site's End marker as its own -- see that
function's header). Expanding only the first site per module removes
the normal-operation path to that shape entirely; the delete-auto-side
defenses (boundary check + `verilog-auto--overlapping-ranges') remain
regardless, since a buffer can still be hand-edited or inherited from
an older version of this code into that shape."
  (let* ((root (verilog-auto--parse-current-buffer))
         (comments (verilog-auto--find-comments root "/*AUTOWIRE*/"))
         (split (verilog-auto--first-autowire-per-module comments))
         (firsts (car split))
         (extras (cdr split)))
    (dolist (c extras)
      (let* ((m (verilog-auto--enclosing-of-type c "module_declaration"))
             (nm (and m (verilog-auto--module-name m))))
        (when (and nm (not (member nm verilog-auto--multi-autowire-modules)))
          (push nm verilog-auto--multi-autowire-modules))))
    (let ((sorted (sort (copy-sequence firsts)
                        (lambda (a b) (> (treesit-node-start a) (treesit-node-start b)))))
          (total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-autowire-site c)))))))

;; --- AUTOARG ----------------------------------------------------------------

(defun verilog-auto--arg-lines (groups indent)
  (verilog-auto--grouped-lines groups indent (lambda (p) (nth 0 p))))

(defun verilog-auto--expand-autoarg-site (module-decl header comment)
  "Expand one /*AUTOARG*/ site. If HEADER is already ANSI (ports
declared in the header itself -- nothing left for AUTOARG to add),
records a notice (folded into `verilog-auto''s final message) and
expands to nothing. Returns the number of ports placed in the arg
list.

Deliberately uses `verilog-auto--nonansi-port-info' (the BODY's own
input/output/inout declarations) rather than
`verilog-auto--nonansi-ports' (which takes NAMES from the header's own
`list_of_ports' and only fills in direction/range from the body) --
AUTOARG's whole job is to populate a header whose `list_of_ports'
holds nothing but the comment itself, so there ARE no header names to
start from; `verilog-auto--nonansi-ports' is for the opposite
situation (an already-complete, elsewhere-defined submodule, as
AUTOINST/AUTOWIRE look up via `verilog-auto--module-ports'). A
non-ANSI header where `/*AUTOARG*/' sits alongside OTHER, genuinely
explicit arg names is accordingly not handled specially -- see this
file's header."
  (if (string= (treesit-node-type header) "module_ansi_header")
      (progn
        (push (verilog-auto--module-name module-decl) verilog-auto--ansi-autoarg-modules)
        0)
    (let* ((ports (verilog-auto--nonansi-port-info module-decl))
           (groups (verilog-auto--group-by-direction ports))
           ;; Module-header-line indent (block step, follows the buffer's
           ;; own style) + a fixed wrap width for the continuation line --
           ;; two different quantities, see `verilog-auto-wrap-width''s
           ;; docstring. M73 collapsed them into one read of
           ;; `standard-indent-width' here, which regressed AUTOARG's
           ;; output to 2 columns on 2-space-style files and made
           ;; verible-verilog-format rewrite it right back to 4 (M74 fix).
           (indent (concat (verilog-auto--line-indent (treesit-node-start module-decl))
                            (make-string verilog-auto-wrap-width ?\s)))
           (lines (verilog-auto--arg-lines groups indent)))
      (when lines
        (goto-char (treesit-node-end comment))
        (insert "\n" (string-join lines "\n")))
      (length ports))))

(defun verilog-auto--expand-all-autoarg ()
  (let* ((root (verilog-auto--parse-current-buffer))
         (mods (verilog-auto--top-level-modules root))
         (sites nil))
    (dolist (m mods)
      (let* ((header (verilog-auto--header-node m))
             (c (verilog-auto--find-comment header "/*AUTOARG*/")))
        (when c (push (list (treesit-node-start c) m header c) sites))))
    (setq sites (sort sites (lambda (a b) (> (car a) (car b)))))
    (let ((total 0))
      (dolist (site sites total)
        (setq total (+ total (verilog-auto--expand-autoarg-site (nth 1 site) (nth 2 site) (nth 3 site))))))))

;; --- verilog-delete-auto -----------------------------------------------------

(defun verilog-auto--autowire-stale-end (comment)
  "End position of the matching \"// End of automatics\" line if
COMMENT (an /*AUTOWIRE*/ block_comment) is immediately followed by a
\"// Beginning of automatic\" marker line, else nil (nothing stale to
delete).

M39 review fix (severity: silent data corruption): the forward scan
used to have no stopping condition besides finding its own \"// End of
automatics\" or running out of siblings, so if THIS site's own End
line was ever missing (most plausibly hand-deleted -- e.g. two
AUTOWIRE sites in one module, both once expanded, and the user deletes
just the FIRST site's End line by hand), it would walk straight past
the first site's own wire declarations, through the second
instantiation, into the SECOND site's Beginning marker, its wires, and
finally attribute the SECOND site's End marker to the FIRST site's
range -- producing two overlapping delete ranges. Deleting the
(correctly-computed) second range first would then shift the buffer
so the first (stale, wrongly-computed) range's fixed end position,
originally aimed at the second site's End line, would get silently
clamped by the normal position-clamping every edit already goes
through, landing at or past the buffer's own end -- deleting
everything from the first comment onward, including any unrelated
module after it, with no warning. Fixed by treating another
/*AUTOWIRE*/ comment or another \"// Beginning of automatic\" marker,
met before this site's own End, as proof this site's End is missing:
stop and return nil (the same, already-safe answer as \"nothing stale
here at all\"), leaving this one site's stale content alone rather
than guessing. `verilog-delete-auto' additionally cross-checks every
collected range for overlap as a second, independent line of defense
(`verilog-auto--overlapping-ranges') -- this fix is what lets that
check see a clean, non-overlapping range for the second site in the
first place, rather than relying on the overlap check alone."
  (let ((next (verilog-auto--next-sibling comment)))
    (when (and next
               (string= (treesit-node-type next) "one_line_comment")
               (string-prefix-p "// Beginning of automatic" (treesit-node-text next)))
      (let ((n (verilog-auto--next-sibling next)) (found nil) (blocked nil))
        (while (and n (not found) (not blocked))
          (cond
           ((and (string= (treesit-node-type n) "one_line_comment")
                 (string= (treesit-node-text n) "// End of automatics"))
            (setq found (treesit-node-end n)))
           ((or (verilog-auto--comment-p n "/*AUTOWIRE*/")
                (and (string= (treesit-node-type n) "one_line_comment")
                     (string-prefix-p "// Beginning of automatic" (treesit-node-text n))))
            (setq blocked t))
           (t (setq n (verilog-auto--next-sibling n)))))
        found))))

(defun verilog-auto--ranges-overlap-p (a b)
  (and (< (car a) (cdr b)) (< (car b) (cdr a))))

(defun verilog-auto--overlapping-ranges (ranges)
  "The subset of RANGES (each a (START . END) cons, char positions)
that overlaps some OTHER range in RANGES. O(n^2), fine for the always-
small number of AUTO sites in a hand-written module.

M39 review addition (structural safety net, defense in depth): every
range `verilog-delete-auto' collects is SUPPOSED to be a disjoint,
independently-computed machine-generated region, and every one of this
file's own site-finding functions (in particular
`verilog-auto--autowire-stale-end', see its own header) is written to
guarantee that. This check doesn't know or care WHY two ranges might
still overlap regardless -- only that deleting either one while
trusting a stale absolute position for the other is exactly the shape
that silently ate an unrelated module in the bug the review that
prompted this caught (position-clamping quietly re-targets a stale
endpoint at whatever the buffer's new end happens to be, rather than
signaling anything). Any range found here is simply left undeleted."
  (let (bad)
    (dolist (a ranges)
      (dolist (b ranges)
        (when (and (not (eq a b))
                   (verilog-auto--ranges-overlap-p a b)
                   (not (memq a bad)))
          (push a bad))))
    bad))

(defun verilog-delete-auto ()
  "Delete every AUTOINST/AUTOWIRE/AUTOARG machine-generated region in
the current buffer, leaving the AUTO comments themselves untouched:
- AUTOINST: from right after the /*AUTOINST*/ comment through (but not
  including) its own instantiation's closing paren.
- AUTOARG: from right after the /*AUTOARG*/ comment through (but not
  including) its own header's closing paren.
- AUTOWIRE: from right after the /*AUTOWIRE*/ comment through the end
  of a following \"// Beginning of automatic\" .. \"// End of
  automatics\" block, if one is there.
Always ends the whole deletion as one undo group (`undo-amalgamate-
boundary'). Returns a cons (DELETED-COUNT . SKIPPED-COUNT):
SKIPPED-COUNT counts ranges withheld because they overlapped another
(see `verilog-auto--overlapping-ranges' -- normal operation never
produces this). Echoes a warning when SKIPPED-COUNT is nonzero; when
called from `verilog-auto' that gets folded into its own final
message instead (this one would otherwise just be invisibly clobbered
by the phases that run afterward)."
  (interactive)
  (let* ((root (verilog-auto--parse-current-buffer))
         (ranges nil))
    (dolist (mi (verilog-auto--find-all-of-type root "module_instantiation"))
      (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
        (when c
          (let* ((hier (verilog-auto--enclosing-of-type c "hierarchical_instance"))
                 (close (and hier (verilog-auto--last-child hier))))
            (when close
              (push (cons (treesit-node-end c) (treesit-node-start close)) ranges))))))
    (dolist (m (verilog-auto--top-level-modules root))
      (let* ((header (verilog-auto--header-node m))
             (c (verilog-auto--find-comment header "/*AUTOARG*/")))
        ;; An ANSI header's /*AUTOARG*/ never expands (see
        ;; `verilog-auto--expand-autoarg-site'), so the region after it
        ;; is never machine-generated -- it's the user's own explicit
        ;; ANSI port declarations, which must never be deleted.
        (when (and c (not (string= (treesit-node-type header) "module_ansi_header")))
          (let ((close (verilog-auto--last-child (verilog-auto--header-port-list header))))
            (when close
              (push (cons (treesit-node-end c) (treesit-node-start close)) ranges))))))
    (dolist (c (verilog-auto--find-comments root "/*AUTOWIRE*/"))
      (let ((end (verilog-auto--autowire-stale-end c)))
        (when end
          (push (cons (treesit-node-end c) end) ranges))))
    (let* ((bad (verilog-auto--overlapping-ranges ranges))
           (good (verilog-auto--filter (lambda (r) (not (memq r bad))) ranges)))
      (setq good (sort good (lambda (a b) (> (car a) (car b)))))
      (dolist (r good)
        (when (< (car r) (cdr r))
          (delete-region (car r) (cdr r))))
      (undo-amalgamate-boundary)
      (when bad
        (message "verilog-delete-auto: %d overlapping range(s) left untouched (buffer unchanged there)"
                  (length bad)))
      (cons (length good) (length bad)))))

;; --- verilog-auto -------------------------------------------------------------

(defun verilog-auto ()
  "Expand every /*AUTOINST*/, /*AUTOWIRE*/, and /*AUTOARG*/ construct in
the current buffer. Idempotent: always starts by deleting every
existing machine-generated region (`verilog-delete-auto') and
re-expanding from scratch, so running it twice in a row leaves the
buffer byte-for-byte unchanged the second time. The whole command is
one undo group."
  (interactive)
  (let* ((delete-result (verilog-delete-auto))
         (overlap-skipped (cdr delete-result)))
    (let ((verilog-auto--module-cache (make-hash-table :test 'equal))
          (verilog-auto--missing-modules nil)
          (verilog-auto--ansi-autoarg-modules nil)
          (verilog-auto--multi-autowire-modules nil)
          (n-inst 0) (n-wire 0) (n-arg 0))
      (setq n-inst (verilog-auto--expand-all-autoinst))
      (setq n-wire (verilog-auto--expand-all-autowire))
      (setq n-arg (verilog-auto--expand-all-autoarg))
      (undo-amalgamate-boundary)
      (setq verilog-auto--missing-modules (nreverse verilog-auto--missing-modules))
      (setq verilog-auto--ansi-autoarg-modules (nreverse verilog-auto--ansi-autoarg-modules))
      (setq verilog-auto--multi-autowire-modules (nreverse verilog-auto--multi-autowire-modules))
      ;; Folded onto whichever of the three messages below actually
      ;; fires -- see `verilog-delete-auto's own docstring for why an
      ;; immediate, separate echo for either of these would just be
      ;; invisibly clobbered by this function's own final message.
      (let ((suffix
             (concat
              (if (> overlap-skipped 0)
                  (format "; %d overlapping range(s) skipped by delete-auto" overlap-skipped)
                "")
              (if verilog-auto--multi-autowire-modules
                  (format "; module %s has multiple /*AUTOWIRE*/ (only the first expanded)%s"
                          (car verilog-auto--multi-autowire-modules)
                          (if (> (length verilog-auto--multi-autowire-modules) 1)
                              (format " (%d total)" (length verilog-auto--multi-autowire-modules))
                            ""))
                ""))))
        (cond
         (verilog-auto--missing-modules
          (message "verilog-auto: module %s not found%s; %d inst, %d wires, %d args%s"
                    (car verilog-auto--missing-modules)
                    (if (> (length verilog-auto--missing-modules) 1)
                        (format " (%d total)" (length verilog-auto--missing-modules))
                      "")
                    n-inst n-wire n-arg suffix))
         (verilog-auto--ansi-autoarg-modules
          (message "verilog-auto: AUTOARG in ANSI header (module %s)%s; %d inst, %d wires, %d args%s"
                    (car verilog-auto--ansi-autoarg-modules)
                    (if (> (length verilog-auto--ansi-autoarg-modules) 1)
                        (format " (%d total)" (length verilog-auto--ansi-autoarg-modules))
                      "")
                    n-inst n-wire n-arg suffix))
         (t
          (message "verilog-auto: %d inst, %d wires, %d args%s" n-inst n-wire n-arg suffix)))))))

;; --- Keybindings -------------------------------------------------------------
;; Buffer-local, added via `verilog-mode-hook' -- the same pattern
;; indent.el uses for RET on every `prog-mode' buffer (`(add-hook
;; 'prog-mode-hook (lambda () (local-set-key "RET" ...)))'), just scoped
;; to `verilog-mode-hook' since these two bindings are Verilog-specific.
;; evil-mode binds nothing under "C-c" in any state (checked directly:
;; neither `evil--normal-map' nor `evil--insert-map' has a "C-c" entry of
;; any length), so `emulation-keymap' lookup falls through to this
;; buffer-local map in both normal and insert state -- see
;; `commands::dispatch_key''s layered lookup.

(add-hook 'verilog-mode-hook
          (lambda ()
            (local-set-key "C-c C-a" 'verilog-auto)
            (local-set-key "C-c C-k" 'verilog-delete-auto)))

;; --- Expand on save: opt-in (M40) -----------------------------------------

(defvar verilog-auto-on-save nil
  "When non-nil, `verilog-auto' runs automatically before every save of
a `verilog-mode' buffer (via `before-save-hook'), so the AUTO-expanded
text -- not the un-expanded source -- is what reaches disk, same as
running C-c C-a by hand right before C-x C-s. nil (the default) leaves
saving a Verilog buffer exactly as untouched as saving any other
buffer.

Inspired by GNU verilog-mode's `verilog-auto-save-policy' (which also
offers `force'/`ask' around unsaved AUTO edits); this v1 is plain
nil/t. Enable in init.el:
  (setq verilog-auto-on-save t)")

(defun verilog-auto--maybe-on-save ()
  "`before-save-hook' function: runs `verilog-auto' when
`verilog-auto-on-save' is non-nil and the current buffer's major mode
is `verilog-mode'. A failed expansion is reported with `message', not
signaled -- a bug in AUTO expansion must never block a save. Every
other buffer, and every `verilog-mode' buffer when the variable is
nil, is a silent no-op: this hook runs on every save in every buffer
across this whole editor's test suite, Verilog or not."
  (when (and verilog-auto-on-save
             (eq (major-mode-internal-get) 'verilog-mode))
    (condition-case err
        (verilog-auto)
      (error (message "verilog-auto-on-save: expansion failed: %S" err)))))

(add-hook 'before-save-hook 'verilog-auto--maybe-on-save)

(provide 'verilog-auto)
