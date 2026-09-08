;;; verilog-nav.el --- Verilog cross-file module-definition jump (M55) -*- lexical-binding: t -*-

;; M55 repro (real binaries, both directions -- see PLAN.md M55 record for
;; the harness transcripts this header summarizes):
;;
;; - NO LSP client connected at all: `M-.'/`g d' (both bound to
;;   `lsp-definition-at-point', lsp.el) print "No LSP server connected in
;;   this buffer (M-x lsp first)" and do nothing else -- buffer and point
;;   untouched, `lsp--marker-stack' untouched.
;; - Connected to `verible-verilog-ls' (this editor's own primary Verilog
;;   server, per `lsp-server-alist'): placing point on an instantiated
;;   module's TYPE name (`axi_lite_slave' in `axi_lite_slave #(...) u_slave
;;   (...)') and sending `textDocument/definition' comes back `[]' -- "No
;;   definition found" -- UNLESS either (a) every file involved has
;;   already had its own `didOpen' sent, or (b) the project root carries a
;;   `verible.filelist' naming every source file up front. Neither is the
;;   common case for a single file opened on its own, which is exactly
;;   the shape most edit sessions start in.
;;
;; Meanwhile this editor's own local primitives already answer the
;; question `verible-verilog-ls' can't, with no server round trip at all:
;; `verilog-auto--library-files' (verilog-auto.el) already enumerates
;; every neighboring `.v'/`.vh'/`.sv'/`.svh' file, and
;; `verilog-auto--find-module-in-buffer'/`--top-level-modules' already
;; parse and search them for a `module_declaration' by name. This file is
;; the missing wiring: a `local-definition-function' (see lsp.el) that
;; answers `M-.' straight from `verilog-library-directories', ahead of
;; (and independent of) any LSP client.
;;
;; --- Tree shape notes (M55 probe, tree-sitter-systemverilog, matches
;;     verilog-auto.el's own M39 notes) ---------------------------------
;;
;; `axi_lite_slave #(.AW(16)) u_slave (...)' and `fifo u_f (.a(b));' both
;; parse to a `module_instantiation' node whose `"instance_type"' field is
;; a `simple_identifier' node -- `treesit-node-start'/`-end' on THAT node
;; give exactly the type name's own character range (1-based, this
;; editor's `point' convention), independent of whether a `#(...)'
;; parameter override sits between the type name and the instance name.
;;
;; --- Context detection: the one genuinely new piece of logic -----------
;;
;; `verilog-complete--port-context' (verilog-complete.el) cannot be reused
;; here -- it answers a completely different question ("is point right
;; after a port-connecting `.'?"), not "is point ON an instantiation's own
;; type name?". `verilog-nav--type-name-at-point' is this file's own,
;; independent context check: find the identifier run under point (via
;; `verilog-complete--ident-char-p', reused as-is -- a plain character
;; class, no reason to duplicate it), resolve the treesit node AT that
;; run's start, walk up to the enclosing `module_instantiation'
;; (`verilog-auto--enclosing-of-type', reused), and confirm point actually
;; falls within that instantiation's OWN `"instance_type"' field's
;; `[start, end]' range (inclusive of END -- see
;; `verilog-nav--point-in-node-p''s own docstring for why, and the tests
;; that pin the boundary). Landing on the INSTANCE name, a port name, a
;; connection value, or an ordinary signal reference all fail this check
;; for the same reason: none of them IS the `"instance_type"' field node,
;; even though every one of them has a `module_instantiation' ancestor
;; somewhere above it.
;;
;; --- Lookup: buffer first, then library files, but keeping the PATH ----
;;
;; `verilog-auto--find-module-in-buffer' is reused verbatim (pure, current
;; buffer, no dynamic state) -- a hit there needs no file to open at all.
;;
;; `verilog-auto--find-module-in-libraries' is deliberately NOT reused for
;; the library-file half of the search: it throws away exactly the one
;; piece of information a JUMP needs that a port-name COMPLETION never
;; did -- which file the match came from. `verilog-nav--find-module-in-
;; libraries' below is the same shape (first file, in
;; `verilog-auto--library-files' order, whose top-level module list
;; contains the wanted name wins) but returns the file's own PATH
;; alongside the matching `module_declaration' node, both pulled from that
;; same string-based reparse (there is no way to point a treesit node
;; parsed from a bare string back at the file it came from otherwise).
;;
;; --- Jump target and the staleness problem ------------------------------
;;
;; The jump target is the module NAME node's own start
;; (`treesit-node-child-by-field-name' on `verilog-auto--header-node's
;; result, field `"name"'), never the enclosing `module_declaration''s own
;; start -- landing exactly on the name is what makes `M-.' land somewhere
;; recognizable rather than at the top of a (possibly long) module.
;;
;; `find-file-internal' (`editing.rs') REUSES an already-open buffer
;; visiting the same path rather than re-reading the file from disk. The
;; library-file half of this file's lookup necessarily reads the file
;; FROM DISK (there is no other way to search a file that might not even
;; have a buffer yet) to decide WHICH file to jump to -- but if that file
;; already has an unsaved, modified buffer open, the disk-computed
;; character position can point at the wrong place, or into the middle of
;; an identifier, once `find-file' hands back the EXISTING (in-memory,
;; divergent) buffer instead of the disk content just parsed. So the
;; lookup happens TWICE, deliberately:
;;   1. Disk scan (`verilog-nav--find-module-in-libraries') decides ONLY
;;      which file declares the wanted module -- its returned character
;;      position is never trusted for the final `goto-char'.
;;   2. Once `find-file' has opened (or switched to) that file's own
;;      buffer, `verilog-auto--find-module-in-buffer' runs AGAIN, this
;;      time against the target buffer's own live (possibly unsaved,
;;      possibly disk-divergent) content, to get the position actually
;;      used.
;; If step 2 comes back empty (the buffer's in-memory content no longer
;; has a module by that name -- the disk version step 1 saw did, but the
;; user has since edited it away without saving), this file refuses to
;; guess: `point-min' plus an explanatory `message', never a stale
;; position computed against content that's no longer there.
;;
;; --- Falling through to LSP on a total miss: NOT a bug --------------------
;;
;; When point IS confirmed to be on an instantiation's type name but the
;; module can't be found anywhere (current buffer nor any
;; `verilog-library-directories' entry), `verilog-goto-module-at-point'
;; returns nil -- deliberately, NOT a `message' first. `local-definition-
;; function''s own contract (lsp.el) treats nil as "not applicable here,
;; try the next tier", so `lsp-definition-at-point' falls through to
;; whatever LSP client (if any) this buffer has connected. This is a
;; DELIBERATE fallback, not a bug this milestone missed: a user running
;; `verible-verilog-ls' with a project-wide `verible.filelist' may have a
;; server that knows about files this buffer's own
;; `verilog-library-directories' setting was never told about at all --
;; falling through is strictly MORE capable than stopping here, never
;; less. And it is never silent either way: the LSP tier below always
;; ends in its own `message' ("No definition found" or "No LSP server
;; connected in this buffer (M-x lsp first)") -- there is no path where
;; a genuine miss produces no feedback at all. (M54 review flagged
;; exactly this failure mode -- "confirmed to be my context, but silent"
;; -- for a different function; recorded here so it isn't repeated.)
;;
;; --- v1 scope: what this file does NOT do -------------------------------
;;
;; - Module NAME completion at an instantiation site is verilog-complete.
;;   el's job, not this file's -- this file only ever JUMPS, never
;;   completes.
;; - No caching of any kind: every invocation re-scans
;;   `verilog-library-directories' and re-parses every candidate file up
;;   to and including the one that matches, from scratch. Deliberate,
;;   not an oversight: `M-.' is a user-initiated, low-frequency action
;;   (nothing like `C-M-i', which can fire on every keystroke), and a
;;   cache would trade a real, if unmeasured, correctness risk (a stale
;;   position surviving an on-disk edit between two jumps) for a
;;   performance gain that has never been measured to matter. If this
;;   ever needs a cache, it should probably be the SAME cache
;;   verilog-complete.el already built (`verilog-complete--library-
;;   cache'), not a second independent one -- not attempted here.
;; - `verilog-library-directories' DOES recurse into subdirectories as
;;   of M56 (bounded by `verilog-library-max-depth'/`verilog-library-
;;   max-files', `.'-prefixed subdirectories skipped, plus an optional
;;   project-root `verible.filelist' via `verilog-library-use-filelist')
;;   -- inherited unchanged from `verilog-auto--library-files', which
;;   this file's own `verilog-nav--find-module-in-libraries' reuses
;;   verbatim for the candidate-file list. See that function's own doc
;;   string (verilog-auto.el) for the exact ordering contract this
;;   file's "first match wins" note above depends on.
;; - `module' AND `interface' declarations are jump targets (M97: this
;;   file reuses `verilog-auto--top-level-modules'/`--find-module-in-
;;   buffer'/`--header-node'/`--module-name' verbatim, and M97 widened
;;   all four to resolve `interface_declaration' alongside `module_
;;   declaration' -- so an instantiated interface's TYPE name, e.g.
;;   `axi_if' in `axi_if u_if();', now jumps the same way a module type
;;   name does, with no changes needed in this file itself). `package'/
;;   `program'/`class' declarations are still not searched for or landed
;;   on -- out of M97's scope. (M124: an interface used only as a PORT
;;   TYPE, e.g. `axi4_lite_if.monitor bus', is now ALSO a jump target --
;;   both from the interface name itself and from the modport name,
;;   which lands on the same interface declaration, not the modport --
;;   see `verilog-nav--type-name-at-point''s own M124 doc.) Jumping to
;;   the MODPORT ITSELF (a specific line inside the interface
;;   declaration, not just the declaration's own start) or to a member
;;   reached THROUGH an interface port remains out of scope, even with
;;   an LSP client attached -- see PLAN.md's M97 record for the
;;   original scope note this one extends.
;; - Does not jump to the INSTANCE's own declaration, and has no notion
;;   of `import pkg::*' at all.
;; - Multiple library files declaring the SAME module name: the first
;;   one in `verilog-auto--library-files' order wins, silently -- no
;;   attempt to detect or warn about the ambiguity, matching
;;   `verilog-auto--find-module-in-libraries's own existing behavior.
;;   Post-M56, that order is: for each `verilog-library-directories'
;;   entry in turn, its own directory (`directory-files' order) before
;;   any of its subdirectories, shallower subdirectories before deeper
;;   ones -- so a module redeclared one level down loses to the SAME
;;   directory's own copy, and a directory listed earlier in
;;   `verilog-library-directories' beats a later one's, however deep.
;;   Anything reachable only via `verible.filelist' (`verilog-library-
;;   use-filelist') always loses to anything found by the directory
;;   scan, regardless of depth, since the filelist pass runs last.
;; - Other buffers' unsaved edits never affect WHICH file is judged to
;;   declare a wanted module (that decision is always made from disk
;;   content) -- they only affect WHERE inside that file the final jump
;;   lands, once it's actually open (see the staleness section above).
;;
;; --- Reparse cost (M55 review correction, same shape verilog-complete.el
;;     already documents for its own file -- this file had no matching
;;     section, which the review flagged as a gap; corrected M120 --
;;     M108/M109 landed after M55 and changed what each call below
;;     actually costs, though not how many calls there are) -------------
;;
;; `treesit-parser-create'/`treesit-node-at' (`treesit.rs') go through
;; `treesit::parse', which is now CACHED and INCREMENTAL, not a full
;; reparse every time: within one unchanged edit generation for the
;; buffer it's an O(1) `Rc' clone, and when the generation moved it
;; reparses INCREMENTALLY via tree-sitter's `Tree::edit' rather than
;; from scratch (M108/M109; see `treesit.rs''s module doc and `parse').
;; Only a language switch, the very first parse of a buffer, or the rare
;; defensive length-mismatch fallback inside `incremental_parse' (see its
;; own comment in `treesit.rs') pays a full from-scratch reparse.
;; `verilog-auto--parse-string' (used for
;; library-candidate files below) is DIFFERENT: it has no buffer to key
;; a generation on, so it is still a full, uncached parse on every call
;; -- see `treesit.rs''s own doc on `parse_string'. One `M-.' on an
;; instantiation's type name pays this cost as follows:
;;   - `verilog-nav--type-name-at-point' always calls into `treesit::parse'
;;     for the CURRENT buffer once (`treesit-node-at', to find the
;;     enclosing `module_instantiation') -- paid on every call, even ones
;;     that turn out not to be on a type name at all, but now usually an
;;     O(1) cache hit or an incremental reparse rather than a full one.
;;   - `verilog-auto--find-module-in-buffer' then calls into
;;     `treesit::parse' for the CURRENT buffer a SECOND time (a different
;;     node target -- the module declaration, not the instantiation -- so
;;     this one can't be folded into the first; same generation as the
;;     first call, so it's the SAME cached/incremental tree, an O(1) `Rc'
;;     clone off what the first call just produced, not a second reparse
;;     of any kind). This call is made UNCONDITIONALLY: it is the `let'
;;     initializer `verilog-goto-module-at-point' branches on, so it runs
;;     before either branch is chosen, INCLUDING the library branch, where
;;     it comes up empty by definition (the module isn't in this buffer)
;;     and its whole cost is wasted -- though with the cache, that wasted
;;     cost is now an `Rc' clone, not a reparse. Folding it into the
;;     library branch's miss path is not possible without first knowing
;;     the answer it exists to compute.
;;   - Same-buffer jump STOPS THERE, at two `treesit::parse' calls total:
;;     the `module_declaration' node the second call produced is reused
;;     DIRECTLY for the actual `goto-char' (`verilog-nav--goto-decl-name')
;;     -- M55 review fix: an earlier version of this file discarded that
;;     node and called `verilog-auto--find-module-in-buffer' a THIRD time,
;;     from inside the goto helper, purely to re-derive something already
;;     in hand. Two is not reducible further within this file's own logic
;;     (out of this milestone's scope, same as verilog-complete.el's own
;;     unresolved note on the identical call shape) -- though as of
;;     M108/M109 both calls are cheap regardless.
;;   - LIBRARY-file jump adds, on top of those same two: one FULL,
;;     uncached string-based parse per candidate file scanned up to and
;;     including the match (`verilog-nav--find-module-in-libraries', via
;;     `verilog-auto--parse-string', which has no buffer to key a cache
;;     on -- this part of the cost model is unchanged by M108/M109), THEN
;;     one more `treesit::parse' call against the TARGET buffer once it's
;;     open (`verilog-auto--find-module-in-buffer' again, inside
;;     `verilog-goto-module-at-point's own library branch) to get a
;;     position valid against that buffer's actual (possibly unsaved)
;;     content -- see the staleness section above for why that last call
;;     is not optional, regardless of what it costs (it is a full parse
;;     the first time that buffer is visited, then cached/incremental on
;;     any later call for the same buffer). Total N+3 CALLS for N
;;     candidate files scanned, NOT N+2: the wasted current-buffer call
;;     in the bullet above is easy to miss when counting, and the first
;;     version of this very section did miss it (caught in the M55 tail
;;     re-review, which is also why the count is spelled out as a number
;;     here rather than left to be re-derived from the prose).
;; None of this is measured to matter in practice (`M-.' is a low-
;; frequency, user-initiated action, see the "No caching" scope note
;; above) -- recorded here purely so the actual number of reparses per
;; jump is documented rather than left to be rediscovered by mutation
;; testing, as it was in review.

;; --- Context detection ----------------------------------------------------

(defun verilog-nav--point-in-node-p (pos node)
  "Non-nil if POS falls within NODE's own `[treesit-node-start,
treesit-node-end]' range -- INCLUSIVE of both ends, so a cursor resting
right after a type name's own last character (`axi_lite_slave|', with
the type name immediately to its left and only whitespace/`#'/the
instance name to its right) still counts as \"on\" it, the same way
`point' sitting right after a word is still considered to be in that
word almost everywhere else in this editor (e.g. `verilog-complete--
prefix-start'). The START end is inclusive for the ordinary reason
(the character AT pos is the first one of the identifier). Tested
explicitly for BOTH ends (`type_name_start_counts_as_on_it' for START,
`type_name_tail_counts_as_on_it' for END) so this inclusive-both-ends
choice is pinned rather than assumed. Only the END end additionally has
a negative pin one position past it (`instance_name_start_does_not_
count'), and that asymmetry is deliberate rather than a missing test:
one position past END is a genuinely DIFFERENT situation, because it
lands inside the NEXT identifier (the instance name), so that pin
exercises the discriminator -- which node `verilog-nav--type-name-at-
point' ends up querying -- and not merely this range check. One
position before START exercises nothing new: it fails on the very same
`>=' comparison that `type_name_start_counts_as_on_it' already pins
from the other side, and no other node becomes reachable there. (M55
tail re-review: this used to claim a negative pin \"just past each
boundary\", which overstated what exists.)"
  (and node (>= pos (treesit-node-start node)) (<= pos (treesit-node-end node))))

(defun verilog-nav--type-name-at-point ()
  "The instantiated module's type name (a string), if point sits within
an instantiation's own `\"instance_type\"' field node; OR (M124) the
interface name (a string), if point sits within a port's own
`interface_port_header' -- either on the `interface_name' field itself
(`axi4_lite_if.monitor bus', point on `axi4_lite_if') or on the
`modport_name' field (point on `monitor'), in which case this still
returns the INTERFACE's name, not the modport's: jumping to the
modport itself is a separate, unimplemented feature (M124 deliberately
stops at the interface declaration -- see this file's header for the
disclosed-gap list). Returns nil otherwise. See this file's header for
exactly which OTHER positions this excludes (instance name, port name,
connection value, an ordinary signal reference) and why none of them
qualify despite all sharing a `module_instantiation' ancestor with the
type name.

Dump-verified (M124, real `axi4_lite_monitor.sv' port shape
`axi4_lite_if.monitor bus'): `interface_port_header' is a child of
`ansi_port_declaration' with two fields, `interface_name' and
`modport_name' -- structurally never a descendant of
`module_instantiation', so the pre-M124 gate (which only ever looked
for a `module_instantiation' ancestor) could never succeed from a port
list at all; this is a structural gap, not a boundary bug."
  (let* ((pos (point))
         ;; on-ident: point must have an identifier character on at
         ;; least one side -- otherwise it's sitting on whitespace or
         ;; punctuation, nowhere near any name at all. `ident-start' is
         ;; then the actual start of that identifier: `prefix-start'
         ;; walks backward from POS over every contiguous identifier
         ;; character, which correctly reaches the run's true start
         ;; whether POS itself is mid-word, at the run's end (cursor
         ;; right after the last character typed), or at the run's own
         ;; start (no characters before POS belong to it at all, so the
         ;; walk stops immediately and `ident-start' comes back equal to
         ;; POS -- still a valid, in-range query position).
         (on-ident (or (verilog-complete--ident-char-p (char-before pos))
                       (verilog-complete--ident-char-p (char-after pos))))
         (ident-start (and on-ident (verilog-complete--prefix-start pos))))
    (when on-ident
      (let* ((parser (treesit-parser-create 'verilog))
             (node (treesit-node-at ident-start parser))
             (mi (and node (verilog-auto--enclosing-of-type node "module_instantiation"))))
        (or (when mi
              (let ((type-node (treesit-node-child-by-field-name mi "instance_type")))
                (when (and type-node (verilog-nav--point-in-node-p pos type-node))
                  (treesit-node-text type-node))))
            ;; M124: an interface-typed port. `iph' is the nearest
            ;; `interface_port_header' ancestor (there is no
            ;; `module_instantiation' one here at all, so this branch is
            ;; independent of the one above, not a fallback within it).
            (when node
              (let ((iph (verilog-auto--enclosing-of-type node "interface_port_header")))
                (when iph
                  (let ((iface-node (treesit-node-child-by-field-name iph "interface_name"))
                        (modport-node (treesit-node-child-by-field-name iph "modport_name")))
                    (cond
                     ((and iface-node (verilog-nav--point-in-node-p pos iface-node))
                      (treesit-node-text iface-node))
                     ((and modport-node (verilog-nav--point-in-node-p pos modport-node) iface-node)
                      (treesit-node-text iface-node))))))))))))

;; --- Lookup: current buffer, then library files (keeping the PATH) ------

(defun verilog-nav--find-module-in-libraries (name)
  "Like `verilog-auto--find-module-in-libraries', but returns (PATH
. MODULE-DECL) -- the file NAME was found in, alongside the matching
node -- instead of throwing the path away. See this file's header for
why a jump (unlike a port-name completion) needs the path. Same
resolution order (`verilog-auto--library-files', first match wins),
same `condition-case'-swallowed read failures."
  (let ((files (verilog-auto--library-files)) (found nil))
    (while (and files (not found))
      (let* ((path (car files))
             (text (condition-case nil (file-contents-as-string path) (error nil))))
        (when text
          (let ((mods (verilog-auto--top-level-modules (verilog-auto--parse-string text))))
            (while (and mods (not found))
              (when (string= (verilog-auto--module-name (car mods)) name)
                (setq found (cons path (car mods))))
              (setq mods (cdr mods))))))
      (setq files (cdr files)))
    found))

;; --- Jump ------------------------------------------------------------------

(defun verilog-nav--goto-decl-name (decl)
  "`goto-char' to DECL's (a `module_declaration' node) own name node
start, and return t. Pure positioning only -- no lookup, no parsing --
so a caller that already HAS a decl node in hand (from whatever search
found it) never has to re-derive it just to jump. M55 review fix: an
earlier version of this file took a NAME string instead and re-ran
`verilog-auto--find-module-in-buffer' internally to get back a node the
caller had already thrown away -- a third, entirely avoidable reparse
of the current buffer on every same-buffer jump (see this file's
header's \"Reparse cost\" section).

CALLER PRECONDITION, deliberately not guarded here: DECL must be a
`module_declaration' whose header and `\"name\"' field both resolve.
`treesit-node-child-by-field-name'/`treesit-node-start' SIGNAL on a nil
node rather than returning nil (`builtins/treesit.rs's `node_arg'), so a
malformed DECL would signal out of here instead of returning nil -- and
because both callers push onto `lsp--marker-stack' BEFORE calling this,
that would leave a pushed marker behind and break `local-definition-
function''s \"nil means not handled\" contract. Both existing callers
obtain DECL from `verilog-auto--find-module-in-buffer', which reaches it
only by successfully reading that same header/name pair through
`verilog-auto--module-name', so the precondition holds by construction
today. It is NOT guarded because a guard returning nil would be worse,
not better: the marker is already pushed and point may already have
moved, so nil would send `lsp-definition-at-point' on to its LSP tier
for a SECOND jump. A new caller that obtains DECL some other way (say
straight from `verilog-auto--top-level-modules', unvalidated) is the
thing to watch for -- M55 tail re-review flagged this as safe-by-
caller-discipline with nothing in the code holding that discipline in
place."
  (goto-char
   (treesit-node-start
    (treesit-node-child-by-field-name (verilog-auto--header-node decl) "name")))
  t)

(defun verilog-nav--goto-module-name-in-buffer (name)
  "If a module NAME is found among the CURRENT buffer's own top-level
declarations, `goto-char' to its name node's own start and return t;
else leave point untouched and return nil. Used only where there is no
already-found decl node to reuse (the library-file branch's target
buffer, once `find-file' has opened it -- see this file's header's
staleness section for why THAT lookup can't be skipped or shared with
the disk-side one)."
  (let ((decl (verilog-auto--find-module-in-buffer name)))
    (when decl
      (verilog-nav--goto-decl-name decl))))

(defun verilog-goto-module-at-point ()
  "`local-definition-function' for `verilog-mode' (see `modes.el' and
lsp.el's own M55 note on `local-definition-function'). Jumps to the
`module' declaration of the instance type name at point, searching the
current buffer first, then `verilog-library-directories' -- entirely
without an LSP server. See this file's header for the full repro,
design, staleness handling, reparse-cost accounting, and v1 scope this
docstring only summarizes.

Returns nil (a SILENT no-op) in two different situations that must
never be confused with each other in behavior, only in return value:
- Point is not on an instantiation's own type name at all -- ordinary
  navigation, nothing to report.
- Point IS on a type name, but the module can't be found anywhere this
  file knows to look -- deliberately falls through to LSP instead of
  stopping here (see this file's header's \"NOT a bug\" section); the
  LSP tier below is what actually messages the user in that case, so
  this function itself must stay silent to avoid a duplicate or
  premature message before that tier has had its own chance.

Returns t after a successful jump, having already pushed the origin
onto `lsp--marker-stack' (`lsp-push-definition-marker') so `M-,' can
return -- the CURRENT-buffer branch pushes before it moves point (there
is no async gap to worry about here, unlike LSP's own
`lsp-definition-at-point'), and the library-file branch captures the
same origin before `find-file' can switch away from it."
  (let ((type-name (verilog-nav--type-name-at-point)))
    (when type-name
      ;; Looked up ONCE and reused for the jump itself below -- see
      ;; `verilog-nav--goto-decl-name's own docstring for the extra
      ;; reparse this collapsing removes.
      (let ((decl (verilog-auto--find-module-in-buffer type-name)))
        (if decl
            (progn
              (lsp-push-definition-marker (lsp--make-definition-marker))
              (verilog-nav--goto-decl-name decl))
          (let ((hit (verilog-nav--find-module-in-libraries type-name)))
            (when hit
              (let ((origin (lsp--make-definition-marker))
                    (path (car hit)))
                (lsp-push-definition-marker origin)
                (find-file path)
                (unless (verilog-nav--goto-module-name-in-buffer type-name)
                  (goto-char (point-min))
                  (message
                   "Verilog module jump: %s declares `%s' on disk, but this buffer's current contents have none by that name"
                   path type-name)))
              t)))))))

(provide 'verilog-nav)
