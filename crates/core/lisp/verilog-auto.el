;;; verilog-auto.el --- GNU verilog-mode's AUTO system core (M39) -*- lexical-binding: t -*-

;; M39 brings up the core of GNU verilog-mode's famous AUTO system:
;; `/*AUTOINST*/' (auto-connect an instantiation's ports to same-named
;; signals), `/*AUTOWIRE*/' (auto-declare wires for undeclared
;; instantiated-module outputs), `/*AUTOARG*/' (auto-fill a Verilog-1995
;; style module argument list), and the two commands that drive them,
;; `verilog-auto' (C-c C-a) and `verilog-delete-auto' (C-c C-k).
;;
;; v1 scope is deliberately narrow. EXCLUDED, matching this repo's own
;; disclosure convention (see e.g. treesit.rs's module doc): AUTOSENSE
;; (`always @*' sensitivity lists -- SystemVerilog's `always_ff'/
;; `always_comb'/`@*' mostly obsolete it anyway), AUTORESET/AUTOUNUSED
;; (the rest of the register/tie-off inference bracket -- M125 shipped
;; AUTOINPUT/AUTOOUTPUT/AUTOINOUT, M126 shipped AUTOREG/AUTOTIEOFF; this
;; list is STALE the moment it names something already shipped, so it
;; is corrected here rather than left to imply the whole bracket is
;; still open). Instance arrays (`u1[3:0] (...)' multi-instance syntax)
;; used to be listed here too -- M127 reconnaissance MEASURED this
;; against real GNU Emacs 30.2 and found it needs NO handling anywhere
;; in this file: GNU treats an array instance structurally like a
;; scalar one for every purpose (connection widths are the port's own
;; declared widths, never multiplied by the array size; AUTOWIRE/`@'/
;; `[]'/`[][]' all behave identically for both), and this grammar
;; already matches that BY CONSTRUCTION -- the array range nests inside
;; `name_of_instance' (a second `unpacked_dimension' child), which every
;; AUTOINST/AUTOWIRE/AUTOOUTPUT/`verilog-delete-auto' call site treats
;; as an opaque single child. This was a misleading disclosure, not a
;; real gap; it is a correction, not new work. AUTO_TEMPLATE
;; (per-instance connection templates -- exact and wildcard-with-`\N'
;; rules, M92; `@' instance numbering, the `[]'/`[][]' bit-range tokens,
;; and the `// Templated' annotation, M127; `@"(lisp-expr)"' evaluated
;; templates, M128; see that section's own header below for the
;; sub-cuts still excluded: one template body shared by several module
;; names, and `verilog-auto-inst-template-numbers' set to `t') IS in scope, and is
;; the reason AUTOINST can produce anything beyond an identity
;; connection. `verilog-auto' runs when the user asks for it via C-c C-a,
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
;; M128 CORRECTION: the paragraph above's own claim that recovery "stays
;; strictly local" was true for the shapes it was written against, but
;; is FALSE as a general justification for "KIND-based search always
;; suffices" -- this project treats a false statement in its own source
;; as a defect in its own right, not something to leave standing once
;; found. Measured (M128 recon): a generated connection whose text
;; contains a COMPLETE string literal immediately followed by a
;; bracketed range (`.data (\"W\"[7:0])'), or an UNTERMINATED string
;; literal, makes tree-sitter's GLR recovery reclassify the WHOLE
;; enclosing statement -- not just the trailing punctuation -- so that
;; NO `module_instantiation'/`hierarchical_instance' node is produced
;; for that site at all. Recursing through an ERROR node to find a KIND
;; cannot help when that KIND was never emitted anywhere in the tree to
;; begin with -- this is not "the stray text has to be stepped over," it
;; is "the thing being searched for does not exist." `verilog-delete-
;; auto' accounts for this (rather than silently returning `(0 . 0)' and
;; leaving the site permanently, invisibly stuck expanded) by comparing
;; every `/*AUTOINST*/' marker comment found ANYWHERE in the tree
;; against the subset actually reached through the `module_instantiation'
;; path, and surfacing the difference as a skipped-and-warned count
;; (`verilog-auto--unreachable-autoinst-markers') -- see that function's
;; own doc string. The real boundary, stated plainly: recovery stays
;; local for the shapes this file's own AUTOINST/AUTOWIRE/AUTOARG marker
;; comments are found relative to (a marker comment itself is never
;; swallowed into an ERROR node in any measured shape), but the
;; STATEMENT the marker sits inside can, for at least this one input
;; shape, lose its own node kinds entirely -- a fact this file discovers
;; and reports, not one it can make disappear by searching harder.
;;
;; M129 CORRECTION: the M128 paragraph above shipped only the honest
;; report for a tree-unreachable `/*AUTOINST*/' site (counted, echoed,
;; left PERMANENTLY stuck expanded) and recorded a text-based fallback
;; scanner as "a genuine architectural departure from this file's
;; tree-based approach", deliberately out of scope. That line is now
;; FALSE: `verilog-auto--lex-paren-pairs' (a one-pass, string/comment/
;; escaped-identifier-aware lexer over the buffer's own raw text,
;; `verilog-auto--text-fallback-range', `verilog-auto--instantiation-
;; shaped-p') gives `verilog-delete-auto' a second, purely textual path
;; for exactly the markers the tree-based path could never reach, and
;; this is now this file's ONE AND ONLY documented exception to its
;; "search by node KIND, never scan raw text past a node boundary"
;; discipline stated above -- confined, by construction, to markers the
;; tree already failed to reach, i.e. exactly the text the tree-based
;; path has already given up trusting. Rescue is not unconditional: a
;; site stays unrecovered when the text itself is lexically broken (an
;; unterminated string or an unterminated block comment inside the
;; candidate region, OR an earlier, unrelated anomaly elsewhere in the
;; buffer that poisoned the lexer's own trust before this site's own
;; close), when no enclosing paren pair even exists for the marker, or
;; when `verilog-auto--instantiation-shaped-p' -- a positive
;; whitelist of exactly the two `MODULE INSTANCE ('/`MODULE #(...)
;; INSTANCE (' shapes, never a blacklist -- refuses the enclosing pair's
;; own preceding tokens. That whitelist exists because a FALSE ACCEPT
;; here would silently delete real hand-written text (a module's own
;; port list is the worst case) mistaking it for a stuck site's
;; generated connections, while a FALSE REJECT only costs one site
;; staying reported as unrecovered instead of rescued -- see
;; `verilog-auto--fallback-keywords's own doc string for the full
;; reasoning. "Unrecovered" does not always mean "permanently stuck
;; expanded" in the same sense as a generated-connection site does: a
;; genuinely STRAY marker comment (never inside any instantiation to
;; begin with, `verilog-auto--unreachable-autoinst-markers's own doc
;; string) was never expanded, so there is nothing for it to be stuck
;; in -- it is simply left alone, same as before M129 (see `verilog-
;; delete-auto's own doc string for that distinction spelled out).
;; Also accepted, not a bug: a gate-level primitive instantiation
;; (`and'/`or'/`buf'/`nand'/`bufif0'/`tranif1'/...) can never be
;; rescued this way, because the primitive's own keyword occupies the
;; guard's module-name slot and every gate keyword is on the blacklist
;; (`verilog-auto--fallback-keywords') -- see that keyword list's own
;; doc string for why this is accepted rather than special-cased.
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
;; --- M125: AUTOOUTPUT/AUTOINPUT/AUTOINOUT (port propagation) -------------
;;
;; GNU verilog-mode's three port-propagation commands: `/*AUTOOUTPUT*/'
;; declares a module-level `output' for every submodule OUTPUT that
;; reaches the enclosing module boundary undriven-elsewhere,
;; `/*AUTOINPUT*/' an `input' for every submodule INPUT the enclosing
;; module doesn't already supply, `/*AUTOINOUT*/' an `inout' for every
;; submodule bidirectional. AUTOREG/AUTOTIEOFF/AUTOSENSE remain out of
;; scope (M126+). Ground truth measured against real GNU Emacs 30.2 --
;; see the M125 spec's own section 1 for the exact commands run and
;; their exact output; every divergence below is deliberate, not an
;; oversight, and is cross-referenced to that spec's own section 2.
;;
;; - The single rule that matters most (measured, GNU and Reticle
;;   agree): a signal that is BOTH a driven submodule output AND a
;;   consumed submodule input (e.g. one instance's `count' output wired
;;   straight into a second instance's `count' input) is INTERNAL to the
;;   enclosing module -- AUTOWIRE's job, not AUTOOUTPUT's or AUTOINPUT's.
;;   Classification is O \\ (I u B) for AUTOOUTPUT, I \\ (O u B) for
;;   AUTOINPUT, and unconditionally B for AUTOINOUT (an inout is always
;;   declared regardless of what else touches the same name) --
;;   `verilog-auto--port-propagation-select'.
;; - Output formatting is SPACES, never GNU's own hard tabs (matching
;;   this whole file's existing AUTOINST/AUTOWIRE/AUTOARG convention).
;; - ANSI-header guard (Reticle divergence from GNU, which happily emits
;;   Verilog-1995 BODY declarations into an ANSI header and produces code
;;   that doesn't compile): on an ANSI header the three commands expand
;;   to nothing and record the module name in
;;   `verilog-auto--ansi-port-auto-modules', same shape as AUTOARG's own
;;   `verilog-auto--ansi-autoarg-modules' precedent.
;; - A candidate name already declared elsewhere in the enclosing module
;;   (Reticle divergence -- GNU emits a genuine duplicate declaration
;;   here, confirmed by a real run, spec section 1.5 case 21) is skipped
;;   and recorded in `verilog-auto--predeclared-port-names' instead of
;;   silently generating code that doesn't compile.
;;   `verilog-auto--declared-names' had to be EXTENDED for this to work
;;   at all: a typed non-ANSI body port declaration (`output logic
;;   rvalid_o;', as opposed to an untyped `output [7:0] dout;') parses
;;   under `list_of_variable_port_identifiers', a DIFFERENT node type
;;   than the `list_of_port_identifiers' the old collector already
;;   walked -- confirmed by a real tree dump (M125 recon), not assumed.
;;   Fixing only `--declared-names' would have left a SECOND,
;;   independently-broken path: `verilog-auto--nonansi-port-info' (which
;;   `verilog-auto--module-ports' calls for every non-ANSI submodule
;;   lookup, and which AUTOARG also calls directly) had exactly the same
;;   narrow-node-type gap, so a typed body port on a NON-ANSI SUBMODULE
;;   -- `gray_ctr.v''s own `output reg [WIDTH-1:0] bin_count;', real demo
;;   material -- used to come back with direction defaulted to `input'
;;   and no range at all, silently wrong classification for every
;;   caller, not just this milestone's own new commands. Both are fixed
;;   together (the underlying decl-walk is now shared, see
;;   `verilog-auto--nonansi-port-info-full').
;; - Parameter substitution in a propagated port's own range: same rule
;;   AUTOWIRE already uses (`verilog-auto--substitute-params', the FIRST
;;   contributing instance's own override list), not GNU's own
;;   always-symbolic behavior (spec section 1.3/2.4). A LATER
;;   contributing instance substituting a DIFFERENT range for the same
;;   signal name is a real design bug, not silently ignored -- recorded
;;   in `verilog-auto--port-range-conflicts' (first-seen range still
;;   wins, same as AUTOWIRE's own dedup).
;; - Candidate source (Reticle divergence -- GNU only ever sees
;;   AUTOINST-GENERATED connections, spec section 1.5 case 20): every
;;   `named_port_connection' of every instantiation on a FRESH reparse is
;;   a candidate, generated or hand-written alike -- the same rule
;;   AUTOWIRE already uses, and strictly more useful (a hand-wired
;;   `.gnt_o(gnt_o)' with `gnt_o' undeclared is exactly a signal that
;;   wants a port). Only a BARE identifier, or a bare identifier with a
;;   single non-nested bit-select/part-select of the WHOLE signal
;;   (`rdata_o[DataWidth-1:0]'), counts -- anything else (concatenation,
;;   an expression, a constant) contributes nothing
;;   (`verilog-auto--connection-candidate-name').
;; - A port whose direction classifies as `'interface' (an actual
;;   interface-modport ANSI port, `verilog-auto--port-direction-of')
;;   contributes to none of the three commands, matching the existing
;;   AUTOINST/AUTOWIRE treatment of that classification. NOTE, recorded
;;   honestly rather than silently assumed: the M125 spec's own section
;;   1.3 additionally reports that GNU's verilog-mode classifies a
;;   PACKAGE-SCOPED user-defined-type port (`input soc_pkg::alu_op_e
;;   op_i') the same way, as an interface, and so declares nothing for
;;   it. A real tree dump (M125 recon) shows this grammar does NOT parse
;;   that shape as `interface_port_header' at all -- it is an ordinary
;;   `variable_port_header' with an explicit `port_direction' and a
;;   `class_type' data type, indistinguishable at the treesit level from
;;   any other typed port. Reproducing GNU's specific behavior here would
;;   need a NEW interface-vs-user-type heuristic this milestone's own
;;   spec never asked for (`verilog-auto--module-ports''s reuse
;;   instruction, spec section 1.8, is explicit that no new lookup
;;   machinery is in scope) -- so Reticle's three commands DO declare a
;;   scoped-user-type port normally, using its own type text verbatim
;;   (e.g. `input soc_pkg::alu_op_e op_i;'), a further, previously
;;   undocumented divergence from the measured GNU table row. Flagged
;;   here rather than silently matched or silently diverged.
;; - Provenance uses the REAL file basename (Reticle divergence -- GNU
;;   hard-codes `.v' regardless of the real extension, spec section
;;   1.4): `verilog-auto--module-file' is a NEW hash, filled at the same
;;   lookup point as `verilog-auto--module-cache' inside
;;   `verilog-auto--module-ports' (that function's own RETURN CONTRACT is
;;   unchanged -- still `(NAME DIRECTION RANGE-TEXT)' triples -- this is
;;   purely an additional side effect at the same lookup site). A module
;;   resolved from the CURRENT BUFFER with no visited file of its own
;;   (the shape almost every test in this file uses) prints the bare
;;   module name with no extension.
;; - Execution order inside `verilog-auto', GNU's own order restricted to
;;   what this file implements (spec section 1.6/2.8): AUTOINST ->
;;   AUTOOUTPUT -> AUTOINPUT -> AUTOINOUT -> AUTOWIRE -> AUTOARG. Each of
;;   the three new passes does its OWN fresh `verilog-auto--parse-
;;   current-buffer' (like AUTOWIRE and AUTOARG already do), because each
;;   must see what the previous phase inserted -- AUTOINPUT must see
;;   AUTOOUTPUT's own newly-declared ports as already-declared names, and
;;   AUTOWIRE must see all three's.
;; - Marker arguments (`/*AUTOOUTPUT("^r")*/', inversion spelled
;;   `/*AUTOOUTPUT("?!^r")*/' -- the `?!' goes INSIDE the quotes, spec
;;   section 1.7's own measured trap): a malformed shape (`?!' OUTSIDE
;;   the quotes, or an unterminated string) is treated the same as "no
;;   filter" functionally (matching GNU's own observed behavior) but,
;;   unlike GNU, is NEVER silent about it -- recorded in
;;   `verilog-auto--port-marker-arg-warnings'.
;; - One AUTOOUTPUT/AUTOINPUT/AUTOINOUT per module, same GNU convention
;;   AUTOWIRE already follows in this file (see that section's own
;;   header): only the textually FIRST marker of EACH kind in a given
;;   module ever expands (`verilog-auto--first-autowire-per-module' is
;;   already generic over "which module encloses this comment," reused
;;   as-is here). Deliberately NOT given its own notice list the way
;;   AUTOWIRE's multi-marker case is -- the M125 spec's own test list and
;;   echo-format section (2.9) name no such notice, and no test fixture
;;   in this milestone's own list exercises more than one marker of the
;;   same kind in one module; adding an unrequested notice mechanism for
;;   an untested edge case risks a silent DIFFERENT bug more than it
;;   protects against this one. A later module DOES leave the extra
;;   marker as a bare, unexpanded comment (never a duplicate
;;   declaration) -- flagged here as a scope decision made under
;;   ambiguity, not something the spec explicitly ruled on.
;; - `verilog-delete-auto' generalized to all FOUR block-style markers
;;   (AUTOWIRE plus the three new ones) via one shared path
;;   (`verilog-auto--any-auto-port-block-marker-p',
;;   `verilog-auto--find-port-marker-comments'): `verilog-auto--autowire-
;;   stale-end''s own "another marker seen before my own End line proves
;;   MY End is missing" safety check used to test literally for
;;   `\"/*AUTOWIRE*/\"' text -- with two DIFFERENT marker kinds now
;;   legitimately adjacent in one module (AUTOOUTPUT immediately followed
;;   by AUTOINPUT is the NORMAL shape here, not a corrupted one), a scan
;;   that only recognized AUTOWIRE as a stop condition would walk straight
;;   past a hand-deleted AUTOOUTPUT End marker into AUTOINPUT's own block
;;   and misattribute ITS End line to AUTOOUTPUT -- the exact data-
;;   corruption shape this function's own M39 header already documents,
;;   just with the four-marker family instead of one.
;;
;; --- M126: AUTOREG/AUTOTIEOFF (register and tie-off inference) ----------
;;
;; `/*AUTOREG*/' declares a module-level `reg' for every undeclared
;; `output' the enclosing module doesn't already drive some other way;
;; `/*AUTOTIEOFF*/' ties an unconnected `output' to a constant zero.
;; AUTOSENSE/AUTORESET/AUTOUNUSED remain out of scope. Ground truth
;; measured against real GNU Emacs 30.2 -- see the M126 spec's own
;; section 1 for the exact commands run and their exact output; every
;; divergence below is deliberate, not an oversight, cross-referenced to
;; that spec's own section 2.
;;
;; - Both commands only ever look at `output_declaration' -- `inout' is
;;   NEVER considered by either one (R6/T11).
;; - A signal already declared in the enclosing module's own BODY (a net
;;   or variable declaration, `verilog-auto--body-declared-names' --
;;   deliberately NARROWER than `verilog-auto--declared-names', which
;;   also counts PORT identifiers and so would make every output look
;;   pre-declared by its own port) suppresses BOTH commands (R3/R4, T4).
;;   A signal driven by a submodule instance connection or a continuous
;;   assign (`verilog-auto--driven-output-names') likewise suppresses
;;   both (R5/R17, T5/T12); a PROCEDURAL (`always'-block) driver does
;;   NOT (W4) -- an `always'-driven output must legally be a `reg', so
;;   AUTOREG still declares it, and tying it off would fight a real
;;   driver.
;; - Divergence from each other, not just from GNU: a PORT-LEVEL type
;;   keyword (`output logic [3:0] a;'/`output wire b;'/`output reg [1:0]
;;   c;') suppresses AUTOREG PER-PORT (R7/R12/R13/R14 -- `signed' alone
;;   does not, R15), but does NOT suppress AUTOTIEOFF at all EXCEPT for
;;   the `reg' case specifically (T10, divergence 3 below) -- measured
;;   GNU asymmetry, reproduced deliberately, not a bug.
;; - Deliberate divergences from GNU (M126 spec section 2, all measured
;;   against real GNU Emacs 30.2):
;;   1. Spaces, never GNU's own hard tabs -- matches this file's existing
;;      AUTOINST/AUTOWIRE/AUTOARG/M125 convention.
;;   2. AUTOTIEOFF on an ANSI header: GNU emits a body `wire' declaration
;;      that DUPLICATES the port and does not compile (T2). Reticle
;;      emits `assign NAME = CONST;' instead -- legal there regardless
;;      of `verilog-auto-tieoff-declaration' -- and records the module
;;      in `verilog-auto--ansi-tieoff-assign-modules'. `demo/rtl/' is
;;      entirely ANSI SystemVerilog; a command that no-ops on ANSI would
;;      contribute ZERO to this project's primary language (the M54
;;      trap), and emitting illegal code is not an option either.
;;   3. AUTOTIEOFF ties off a port already declared `reg' ON THE PORT
;;      ITSELF (`output reg [1:0] c;', T10), duplicating the
;;      declaration. Reticle skips it and records the name in
;;      `verilog-auto--tieoff-port-reg-skips' -- same family as M125's
;;      divergence 3 (`verilog-auto--predeclared-port-names').
;;   4. A regexp argument (`/*AUTOREG(\"^a\")*/') silently no-ops the
;;      WHOLE command in GNU (R10/T7/T8). Reticle expands nothing AND
;;      records a warning naming the command, reusing the existing
;;      `verilog-auto--port-marker-arg-warnings' list (M125) rather than
;;      inventing a fourth notice list -- a silent no-op is precisely
;;      the failure mode this repo keeps getting burned by. Unlike the
;;      M125 commands' own regexp FILTER (which can be malformed but
;;      still apply as "no filter"), AUTOREG/AUTOTIEOFF take NO argument
;;      at all -- ANY argument, well-formed or not, triggers this path,
;;      checked by the simplest possible test ("is the comment's own
;;      text exactly `/*AUTOREG*/'/`/*AUTOTIEOFF*/', with nothing
;;      inside the parens"), not the full `verilog-auto--port-marker-arg'
;;      plist machinery that command family needs for its OWN filter
;;      semantics.
;;   5. A multi-dimensional range (`output [3:0][1:0] a;') tie-off in
;;      GNU uses ONLY the LAST dimension for its constant, silently
;;      UNDER-REPORTING the real width (T16 -- width 2, not 8). Reticle
;;      instead: if EVERY dimension is numeric, uses the PRODUCT of all
;;      of them (the correct width); if ANY dimension is symbolic,
;;      SKIPS the signal and records it in
;;      `verilog-auto--tieoff-symbolic-multidim-skips' rather than
;;      emitting a constant it cannot justify. AUTOREG is unaffected --
;;      it declares the port verbatim (`reg [3:0] [1:0] a;', R18), no
;;      constant to compute.
;;   6. `module dut;' with no port parenthesis throws a real elisp error
;;      in GNU (R19) -- identical to M125's divergence 7; the existing
;;      parse path already yields no ports here, so both commands simply
;;      expand nothing.
;;   7. Fix round: the `[WIDTH-1:0]' special case's own MSB regex used
;;      to require `-1' with NO whitespace at all, matched against MSB
;;      only. GNU's own regex (`verilog-mode.el:11427', `"^\\s
;;      *\\([a-zA-Z_][a-zA-Z0-9_]*\\)\\s *-\\s *1\\s *:\\s *0\\s *$"')
;;      tolerates whitespace at every position and matches the WHOLE
;;      range text. Confirmed against real GNU Emacs 30.2 (three ports,
;;      one `verilog-auto' run): `output [WIDTH - 1:0] a;' and `output [
;;      WIDTH-1 : 0 ] b;' BOTH take the special case (`{WIDTH{1'b0}}'),
;;      identical to plain `[WIDTH-1:0]'; `output [WIDTH-1:1] c;' still
;;      takes the general form (`{(1+(WIDTH-1)-(1)){1'b0}}'). Fixed by
;;      widening `verilog-auto--symbolic-tieoff-body''s own MSB regex to
;;      tolerate whitespace around the `-' (LSB already arrives clean --
;;      see that function's own doc string for why MSB is the only
;;      whitespace shape that needs handling here). GNU ALSO rewrites
;;      the emitted declaration's own range text to a canonical
;;      `[WIDTH-1:0]' in both cases; Reticle deliberately does NOT --
;;      the user's own range text is kept verbatim, same policy as R8's
;;      symbolic-range-copied-verbatim rule -- so `output [WIDTH - 1:0]
;;      a;' still prints `wire [WIDTH - 1:0] a = {WIDTH{1'b0}};' here,
;;      not GNU's rewritten `[WIDTH-1:0]'. A NEW divergence, not an
;;      oversight.
;; - Execution order (M126 spec section 1.4, confirmed against real GNU
;;   Emacs 30.2 running all three markers at once): AUTOINST ->
;;   AUTOOUTPUT -> AUTOINPUT -> AUTOINOUT -> AUTOTIEOFF -> ... ->
;;   AUTOWIRE -> AUTOREG -> AUTOARG. AUTOTIEOFF running BEFORE AUTOREG is
;;   not cosmetic: with both markers present, AUTOTIEOFF's own tie-off
;;   declaration (a body `wire ... = ...;', or a body `assign ... =
;;   ...;' on an ANSI header/`\"assign\"' knob) becomes visible to
;;   AUTOREG's OWN fresh reparse (every phase in `verilog-auto' reparses
;;   the buffer itself, M39/M125 convention, unchanged here) as either
;;   an already-body-declared name or a continuous-assign-driven one --
;;   either way AUTOREG then emits NOTHING AT ALL for that signal, not
;;   even the frame comments (O2/T9, measured GNU; T18 confirms document
;;   order of the two markers doesn't matter). This is an EMERGENT
;;   consequence of the ordering plus the shared `verilog-auto--driven-
;;   output-names'/`verilog-auto--body-declared-names' helpers, not a
;;   special case coded for it.
;; - Layout: AUTOREG's own declaration line is a single, un-padded space-
;;   separated line (`reg [signed] [range] name;'), matching this file's
;;   existing AUTOWIRE convention (`verilog-auto--expand-autowire-site'),
;;   which has no trailing element to align to either. AUTOTIEOFF's own
;;   line DOES pad its `decl-kw [signed] [range] name' prefix to
;;   `verilog-auto-inst-column' before `= CONST;' (`verilog-auto--tieoff-
;;   decl-line'), reusing `verilog-auto--pad-to-column' the same way
;;   M125's `verilog-auto--port-decl-line' pads before its OWN trailing
;;   `// From/To' comment -- a deliberate implementation choice (this
;;   file's M126 spec left the exact column convention open for
;;   AUTOTIEOFF, which has an `=' to align the way AUTOWIRE/AUTOREG have
;;   nothing to align), not a measured-GNU requirement (GNU itself uses
;;   two SEPARATE tab-stops there, one before the name and one before
;;   `=', which this single-column convention does not reproduce).
;; - One AUTOREG/AUTOTIEOFF per module, same GNU convention this file
;;   already follows for AUTOWIRE (M39) and AUTOOUTPUT/AUTOINPUT/
;;   AUTOINOUT (M125): only the textually FIRST marker of each kind in a
;;   given module ever expands (`verilog-auto--first-autowire-per-module'
;;   reused as-is); a later marker in the same module is left bare, with
;;   no dedicated notice -- the same scope decision under ambiguity M125
;;   made for its own three commands.
;; - `verilog-delete-auto' and `verilog-auto--any-auto-port-block-marker-
;;   p' are both widened from four block-style markers to six.

(defvar verilog-auto--module-file nil)
(defvar verilog-auto--module-full-ports nil)
(defvar verilog-auto--module-port-dims nil
  "M127: NAME -> `verilog-auto--ports-of-module-dims' result, the third
parallel cache filled at the ONE lookup point (`verilog-auto--module-
ports') -- (PORT-NAME PACKED-LIST UNPACKED-LIST) triples, needed only
by AUTOINST's `[]'/`[][]' template tokens (section 2.3). Same shape as
`verilog-auto--module-full-ports' (M125): this function's own 3-tuple
RETURN CONTRACT stays unchanged; this is purely an additional side
effect at the same site.")
(defvar verilog-auto--ansi-port-auto-modules nil)
(defvar verilog-auto--predeclared-port-names nil)
(defvar verilog-auto--port-range-conflicts nil)
(defvar verilog-auto--port-marker-arg-warnings nil)
(defvar verilog-auto--ansi-autoreg-modules nil
  "M126: module names where `/*AUTOREG*/' sat in an ANSI header and
expanded to nothing (same shape as `verilog-auto--ansi-autoarg-modules'
and M125's `verilog-auto--ansi-port-auto-modules') -- GNU-IDENTICAL
behaviour here (R2), not a Reticle safety refusal: an ANSI output
already carries its own type, so AUTOREG has nothing to add.")
(defvar verilog-auto--ansi-tieoff-assign-modules nil
  "M126: module names where `/*AUTOTIEOFF*/' switched from
`verilog-auto-tieoff-declaration''s own form to `assign' because the
enclosing header is ANSI (divergence 2 -- a body `wire' there would
duplicate the port's own declaration and not compile).")
(defvar verilog-auto--tieoff-port-reg-skips nil
  "M126: signal names `/*AUTOTIEOFF*/' skipped because the port itself
already carries an explicit `reg' type (divergence 3 -- GNU ties them
off anyway, duplicating the declaration, T10).")
(defvar verilog-auto--tieoff-symbolic-multidim-skips nil
  "M126: signal names `/*AUTOTIEOFF*/' skipped because the port has two
or more packed dimensions and at least one is symbolic, so no constant
width can be computed (divergence 5 -- GNU emits a constant using only
the LAST dimension, silently under-reporting the real width, T16).")

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

(defvar verilog-auto-tieoff-declaration "wire"
  "Declaration form `/*AUTOTIEOFF*/' uses on a NON-ANSI header:
\"wire\" (the default) emits `wire [RANGE] NAME = CONST;'; \"assign\"
emits `assign NAME = CONST;' (no keyword, no range -- T6, measured GNU
Emacs 30.2).

On an ANSI header the form is ALWAYS `assign', regardless of this
variable's value: a body `wire' declaration there would duplicate the
port's own declaration and not compile (M126 divergence 2 -- GNU itself
emits exactly that duplicate, T2), and a notice records that the form
was switched (`verilog-auto--ansi-tieoff-assign-modules').

GNU verilog-mode's own docstring for `verilog-auto-wire-type' claims
THAT variable changes AUTOTIEOFF's declaration keyword; the M126 spec's
own measurement against real GNU Emacs 30.2 found this false in GNU
too -- `verilog-auto-wire-type' has no effect on AUTOTIEOFF's own
output there. `verilog-auto-tieoff-declaration' is the knob that
actually works, here and in GNU.")

;; Internal, invocation-scoped state -- see the header above. Always
;; let-bound fresh by `verilog-auto' itself; the top-level `defvar' just
;; gives every helper a variable to dynamically refer to, and a harmless
;; default if one is ever called outside that scope (e.g. a stray manual
;; `M-:').
(defvar verilog-auto--module-cache nil)
(defvar verilog-auto--missing-modules nil
  "M125 fix round (spec section 3.2) investigated whether this list has
the SAME rightmost-first/single-`nreverse' inversion the three new M125
notice lists had (fixed -- see `verilog-auto''s own body, right after
`undo-amalgamate-boundary'). It does NOT, and the reason it doesn't is
worth recording rather than silently matching the other three: this
list is filled from `verilog-auto--module-ports' (a single, cache-gated
push point -- a given NAME only ever pushes once, on its FIRST lookup
attempt, ever), and that function is called from MULTIPLE, DIFFERENTLY-
ORDERED passes across the whole `verilog-auto' run, not one:
- `verilog-auto--expand-all-autoinst' (PHASE 1, runs first) looks up
  ONLY instantiations that carry their own `/*AUTOINST*/' comment,
  rightmost SITE first.
- `verilog-auto--expand-all-port-propagation' (PHASE 2, one sub-pass
  per KIND) looks up EVERY `module_instantiation' in a module
  regardless of whether it has any AUTOINST comment at all, rightmost
  MODULE first.
- `verilog-auto--expand-all-autowire' (PHASE 3) likewise looks up every
  instantiation in a module that carries an AUTOWIRE comment, rightmost
  MODULE first.
Because PHASE 1 only covers COMMENTED instantiations and phases run to
completion in sequence (not interleaved), an UNCOMMENTED instantiation
of a missing module earlier in the buffer can still be discovered LATER
than a commented one after it, if the earlier one's missing-module
lookup only happens in PHASE 2 or 3. The raw push order is therefore
the CONCATENATION of several per-phase orders, not a single reversed
walk of the whole buffer -- there is no single `nreverse' (or its
removal) that recovers \"true first missing module, by document
position\" from that shape. Left AS-IS rather than guessed at; a
correct fix would need to track (NAME . FIRST-ENCOUNTER-POSITION) pairs
and sort by position directly, which is a real, larger change out of
this fix round's own scope.

M125 trailing fix round: the three new notice lists (`verilog-auto--
predeclared-port-names', `verilog-auto--port-range-conflicts',
`verilog-auto--port-marker-arg-warnings') were switched to EXACTLY that
position-tracking approach after a cold review found the SAME class of
ordering bug in them, ACROSS the three port-propagation KINDS rather
than across `verilog-auto--module-ports''s several differently-ordered
phases -- see `verilog-auto''s own body, right after `undo-amalgamate-
boundary', for the working implementation
(`verilog-auto--notice-first'). The approach is not inapplicable here;
it is simply a larger change for THIS list specifically, because the
fill point (`verilog-auto--module-ports', a cache-gated push inside a
function called from every phase alike) sits past where each phase's
own position context is naturally at hand, and because more than three
phases would need threading through it instead of two comment-adjacent
call sites. Still out of this fix round's own scope for that reason,
not because the fix wouldn't work.")
(defvar verilog-auto--ansi-autoarg-modules nil)
(defvar verilog-auto--multi-autowire-modules nil)
(defvar verilog-auto--template-parse-warnings nil)

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

(defun verilog-auto--find-all-of-types (node types)
  "Like `verilog-auto--find-all-of-type', but TYPES is a list of node-type
strings, matched by membership rather than a single `string='. Document
order is preserved across the whole TYPES set (a single tree walk, not
one walk per type concatenated afterward -- see `verilog-auto--top-
level-modules', M97), unlike calling `verilog-auto--find-all-of-type'
once per type and appending the results, which would interleave two
declaration kinds out of buffer order."
  (verilog-auto--find-all node (lambda (n) (member (treesit-node-type n) types))))

(defun verilog-auto--find-first-of-type (node type)
  (verilog-auto--find-first node (lambda (n) (string= (treesit-node-type n) type))))

(defconst verilog-auto--ansi-header-types '("module_ansi_header" "interface_ansi_header")
  "Node types `verilog-auto--header-node'/`verilog-auto--ansi-header-p'
treat as an ANSI header -- ports declared with their directions right
there in the header itself, as opposed to a `*_nonansi_header' (names
only, directions declared separately in the body). M97: widened from
`module_ansi_header' alone once `verilog-auto--top-level-modules' was
widened to resolve `interface_declaration's too -- `interface_ansi_
header' is the exact same shape (dump-verified, M97 recon: same `name:'
field, same `list_of_port_declarations' child).")

(defun verilog-auto--ansi-header-p (header)
  "Non-nil if HEADER's own node type is one of
`verilog-auto--ansi-header-types'. Centralizes what used to be a bare
`(string= (treesit-node-type header) \"module_ansi_header\")' at three
call sites in this file plus one in verilog-complete.el -- widening
just the string literal at each site, without this predicate, would
have left every one of them silently treating an `interface_ansi_
header' as non-ANSI (the `else' branch), which is wrong: an ANSI
interface header's `/*AUTOARG*/'-adjacent region is user-written ports,
never machine-generated, exactly like an ANSI module header's (see
`verilog-delete-auto')."
  (member (treesit-node-type header) verilog-auto--ansi-header-types))

(defun verilog-auto--ansi-header-with-ports-p (header)
  "Non-nil if HEADER is an ANSI header (`verilog-auto--ansi-header-p')
AND it actually carries an inline port list -- i.e. it has a real port-
parens child, found via `verilog-auto--header-port-list' (defined
below, forward-referenced here in the doc string only).

This is a DIFFERENT question from what `verilog-auto--ansi-header-p'
answers, and conflating them is the M134 defect. The grammar makes a
`module_ansi_header''s whole port-parens group OPTIONAL: `module top;'
(no parens anywhere) and `module top #(parameter int W = 8);' (a
`parameter_port_list' but no `list_of_port_declarations') are BOTH
`module_ansi_header' by node TYPE alone -- so a bare node-type check
reads either of them as \"ports already declared in the header, nothing
left for AUTOOUTPUT/AUTOINPUT/AUTOINOUT/AUTOREG to add\" and silently
drops output/reg generation real GNU performs there (M134 recon,
measured against GNU Emacs 30.2: `module top;' with an unconnected
submodule instance gets the SAME `/*AUTOOUTPUT*/' body as `module top
();').

Call sites whose real question is \"does this header already declare
its own ports inline, so there's nothing left to add/convert\" must use
THIS predicate. Call sites asking a genuinely different question --
does this NODE happen to be the ANSI header shape at all, e.g. to find
a `parameter_port_list' for parameter completion
(`verilog-complete--parameters-of-module'), or to decide `assign' vs
`wire' tie-off declaration STYLE (`verilog-auto--expand-autotieoff-
site') -- must keep using `verilog-auto--ansi-header-p' instead. See
this file's M134 header for the full 11-call-site audit that decided
which of the two each one needed."
  (and (verilog-auto--ansi-header-p header)
       (verilog-auto--header-port-list header)
       t))

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

(defun verilog-auto--enclosing-of-types (node types)
  "Like `verilog-auto--enclosing-of-type', but TYPES is a list of node-
type strings, matched by membership. M97 fix round (FF1): the AUTOWIRE
call sites below need this -- their own AUTOINST/AUTOARG siblings were
deliberately left at the single-type `verilog-auto--enclosing-of-type'
\(the M97 spec's own words: an AUTOINST comment's enclosing declaration
\"is always a module\", never an interface, because AUTOINST fires
inside a `hierarchical_instance', not directly inside a module/interface
body). AUTOWIRE's own comment sits directly in the body instead, so
THAT reasoning does not carry over -- an `/*AUTOWIRE*/' inside `interface
foo; /*AUTOWIRE*/ endinterface' has an INTERFACE as its nearest
`module_declaration'-or-`interface_declaration' ancestor, and the
single-type version returned nil there, which every caller below then
fed unchecked into `treesit-node-child-count'-calling helpers -- a hard
crash (`Wrong type argument: treesit-node-p, nil'), not a graceful
no-op."
  (let ((n node))
    (while (and n (not (member (treesit-node-type n) types)))
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
  "Every top-level `module_declaration' AND `interface_declaration' under
ROOT, in document order (M97: widened from `module_declaration' alone
-- `interface_declaration' has the identical `name:'-field shape, dump-
verified, M97 recon). The name \"modules\" is kept for this function
\(every caller already spells it that way, and an interface IS a
resolvable instantiation target, the same role a module plays here) --
see this file's header for the M39 tree-shape notes this extends."
  (verilog-auto--find-all-of-types root '("module_declaration" "interface_declaration")))

(defun verilog-auto--header-node (module-decl)
  "MODULE-DECL's own ANSI or non-ANSI header child -- `module_ansi_header'/
`module_nonansi_header' for a `module_declaration', or `interface_ansi_
header'/`interface_nonansi_header' for an `interface_declaration' (M97:
dump-verified same `name:' field, same `list_of_port_declarations'/
`list_of_ports' children as the module headers they mirror)."
  (or (verilog-auto--find-first-of-type module-decl "module_ansi_header")
      (verilog-auto--find-first-of-type module-decl "module_nonansi_header")
      (verilog-auto--find-first-of-type module-decl "interface_ansi_header")
      (verilog-auto--find-first-of-type module-decl "interface_nonansi_header")))

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
locally-motivated project-root heuristic.

M132: this deliberately keeps calling `lsp--project-root' here, NOT
`lsp--project-root-for-command' or `lsp--filelist-component-root' --
this is a CLIENT-side lookup for where `verible.filelist' actually
sits ON DISK, and a `workspace'-style widened root does not contain
one (the filelist lives at the NARROWER, `lsp--project-root' directory
-- widening is exactly the operation that walks past it). Switching
this site to the widened root would make `expand-file-name
\"verible.filelist\" root' resolve to a directory with no such file at
all, silently breaking every `verilog-library-*' lookup that depends
on this function. See `lsp--project-root-for-command''s own docstring
for the general rule this is the documented exception to.

M132: the parsing loop itself now lives in `lsp--filelist-entries'
(`lsp.el') -- extracted verbatim, this function calls it and applies
its own two EXTRA filters (`verilog-auto--library-file-name-p' and
`file-exists-p') on top, so its behavior here is unchanged by the
extraction. `lsp--filelist-entries' resolves each line relative to the
file list's OWN directory rather than to a caller-supplied ROOT, but
that is the same directory as ROOT here (the filelist is always read
from directly inside ROOT, per the resolution-rule paragraph above),
so the two are not observably different in this call site."
  (when verilog-library-use-filelist
    (let* ((probe-file (if (buffer-file-name)
                            (buffer-file-name)
                          (expand-file-name "verilog-auto--filelist-probe"
                                             (default-directory))))
           (root (lsp--project-root probe-file))
           (filelist-path (expand-file-name "verible.filelist" root)))
      (when (file-exists-p filelist-path)
        (let (acc)
          (dolist (path (lsp--filelist-entries filelist-path))
            (when (and (verilog-auto--library-file-name-p
                        (file-name-nondirectory path))
                       (file-exists-p path))
              (push path acc)))
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
  "(NODE . PATH) for the first library file whose top-level module
matches NAME, or nil if none does -- PATH (M125) is that file's own
absolute path, needed by `verilog-auto--module-ports' to record
provenance (`verilog-auto--module-file'); NODE comes from
`verilog-auto--parse-string' and so carries no buffer/file identity of
its own, which is exactly why PATH has to be threaded back separately
rather than derived from NODE after the fact."
  (let ((files (verilog-auto--library-files)) (found nil) (found-path nil))
    (while (and files (not found))
      (let* ((path (car files))
             (text (condition-case nil (file-contents-as-string path) (error nil))))
        (when text
          (let ((mods (verilog-auto--top-level-modules (verilog-auto--parse-string text))))
            (while (and mods (not found))
              (when (string= (verilog-auto--module-name (car mods)) name)
                (setq found (car mods) found-path path))
              (setq mods (cdr mods))))))
      (setq files (cdr files)))
    (and found (cons found found-path))))

(defun verilog-auto--module-ports (name)
  "Port list for module NAME: (NAME DIRECTION RANGE-TEXT) triples,
declaration order (DIRECTION a symbol, `input'/`output'/`inout';
RANGE-TEXT the submodule's own packed-dimension text verbatim, or nil).
Looks in the current buffer's own module declarations first, then
`verilog-library-directories'; cached in `verilog-auto--module-cache'.
Records NAME in `verilog-auto--missing-modules' (once) and returns nil
if it can't be found anywhere.

M125: this is also the ONE lookup point for two NEW, parallel caches,
filled as a side effect -- this function's own RETURN CONTRACT (the
3-tuple list above) is unchanged:
- `verilog-auto--module-file': NAME -> the basename of the file NAME was
  actually found in (the current buffer's own `buffer-file-name' when
  found there, a library file's basename when found via
  `verilog-auto--find-module-in-libraries', or nil for a buffer-found
  module with no visited file at all -- see this file's M125 header for
  why bare-name provenance falls out of that last case).
- `verilog-auto--module-full-ports': NAME -> (NAME DIRECTION RANGE-TEXT
  TYPE-TEXT) 4-tuples (`verilog-auto--ports-of-module-full'), the same
  ports with their own declared type keyword text alongside -- needed by
  AUTOOUTPUT/AUTOINPUT/AUTOINOUT (spec section 2.1's `<type>' column)
  but not by anything that predates M125, which is why it isn't folded
  into the 3-tuple contract everything else already depends on.
- `verilog-auto--module-port-dims' (M127): NAME -> (PORT-NAME PACKED-
  LIST UNPACKED-LIST) triples, needed only by AUTOINST's `[]'/`[][]'
  template tokens (section 2.3)."
  (let ((cached (gethash name verilog-auto--module-cache 'verilog-auto--miss)))
    (if (not (eq cached 'verilog-auto--miss))
        cached
      (let* ((buf-node (verilog-auto--find-module-in-buffer name))
             (lib (and (not buf-node) (verilog-auto--find-module-in-libraries name)))
             (node (or buf-node (car lib)))
             (path (cond (buf-node (buffer-file-name))
                         (lib (cdr lib))
                         (t nil)))
             (full (and node (verilog-auto--ports-of-module-full node)))
             (ports (mapcar (lambda (e) (list (nth 0 e) (nth 1 e) (nth 2 e))) full))
             (dims (and node (verilog-auto--ports-of-module-dims node))))
        (puthash name ports verilog-auto--module-cache)
        (puthash name full verilog-auto--module-full-ports)
        (puthash name dims verilog-auto--module-port-dims)
        (puthash name (and path (file-name-nondirectory path)) verilog-auto--module-file)
        (unless node
          (push name verilog-auto--missing-modules))
        ports))))

;; --- Port extraction: ANSI and non-ANSI headers --------------------------

(defun verilog-auto--port-direction-of (node)
  "'input/'output/'inout from the port_direction descendant of NODE;
'interface (M124) if NODE has an `interface_port_header' descendant
instead -- an interface-typed ANSI port
(`interface_port_header interface_name: ... modport_name: ...',
dump-verified against the real `demo/verif/axi4_lite_monitor.sv' shape)
structurally never carries a `port_direction' node at all (of the three
ANSI port-header kinds, `net_port_header'/`variable_port_header' both
can, `interface_port_header' never can), so this is a positive
detection, not the fallback below firing on an absence. Falls back to
'input as a best-effort guess only for the remaining case, an ANSI port
that omits its own direction and so inherits the previous port's per
the LRM; v1 doesn't track that carry-over, so this documented fallback
stands in for it -- not exercised by any v1 test, every ANSI port here
declares its own direction. Left UNCHANGED from before M124: this
fallback is a different, still-open gap, not something this milestone
touches."
  (cond
   ((verilog-auto--find-first-of-type node "interface_port_header") 'interface)
   ((verilog-auto--find-first-of-type node "port_direction")
    (intern (treesit-node-text (verilog-auto--find-first-of-type node "port_direction"))))
   (t 'input)))

(defun verilog-auto--normalize-range-whitespace (text)
  "TEXT (a packed-dimension node's own raw text, e.g. \"[  AddrWidth-1:0
]\") with every internal whitespace RUN (spaces, tabs, or a newline for
a range that wraps a physical line) collapsed to a single space, then
trimmed away entirely right after the opening `[' and right before the
closing `]' -- i.e. exactly the shape a HAND-TYPED, non-column-aligned
range would already have. M125 fix round, spec section 3.1."
  (let ((collapsed (replace-regexp-in-string "[ \t\n\r]+" " " text)))
    (replace-regexp-in-string
     " +\\]" "]"
     (replace-regexp-in-string "\\[ +" "[" collapsed))))

(defun verilog-auto--range-text-of (node)
  "NODE's own `packed_dimension' descendant's text, whitespace-
normalized (`verilog-auto--normalize-range-whitespace'), or nil if NODE
has none. M125 fix round (spec section 3.1): a verible-column-aligned
source file (`demo/rtl/mem/sram_wrapper.sv''s own `[         AddrWidth-
1:0]') used to carry that alignment padding verbatim into a DIFFERENT
file's generated declaration -- AUTOINST's connection expression,
AUTOWIRE's wire declaration, and AUTOOUTPUT/AUTOINPUT/AUTOINOUT's
propagated port declaration all read range text through this ONE
function, so the fix is shared by all of them, not special-cased to
whichever caller happened to surface it first."
  (let ((pdim (verilog-auto--find-first-of-type node "packed_dimension")))
    (and pdim (verilog-auto--normalize-range-whitespace (treesit-node-text pdim)))))

(defconst verilog-auto--net-type-keywords
  '("wire" "reg" "tri" "triand" "trior" "tri0" "tri1" "trireg" "wand" "wor" "supply0" "supply1" "uwire")
  "Type keywords `verilog-auto--decl-type-text' omits from a propagated
port declaration (M125 fix round, spec section 1). `wire' alone is
under-specified: GNU also drops `reg' (measured 2026-09-09, GNU Emacs
30.2, against a real `output reg [WIDTH-1:0] bin_count;' submodule
port -- GNU's own AUTOOUTPUT emits `output [WIDTH-1:0] bin_count;', no
`reg'), and the reason generalizes: the declaration being generated
describes the ENCLOSING module's own port, whose driver is an instance
output -- a continuous driver -- so it must be a NET, and `reg' is a
variable keyword a continuously-driven port cannot legally carry in
Verilog-2001 (`logic' is the one exception -- legal on a continuously-
driven port in SystemVerilog -- which is exactly why it is NOT in this
list; a package-scoped user type like `soc_pkg::alu_op_e' likewise
survives verbatim, unaffected).

This list holds all TWELVE `net_type' keywords IEEE 1800-2017 defines
(`wire'/`tri'/`triand'/`trior'/`tri0'/`tri1'/`trireg'/`wand'/`wor'/
`supply0'/`supply1'/`uwire'), plus `reg' -- but trailing fix round: MOST
of the net-type keywords, `wire' and `trireg' dump-verified directly
among them, are never actually TESTED by the `member' check below at
all, because none of them can structurally produce a `data_type' node
in the first place (same reasoning as `wire' in the paragraph above --
a net_type keyword parses under `net_port_type'/`net_type', never
`data_type'). The ONLY keyword this list's `member' check does real
work catching is `reg' (a `data_type'/`integer_vector_type' \"reg\" node,
dump-verified, the identical shape `logic' produces -- see `verilog-
auto--decl-type-text''s own doc string). Every OTHER entry is listed for
LRM completeness and to keep this list in sync with the grammar if a
FUTURE tree-sitter-systemverilog version ever parses a net-type keyword
under `data_type' where it doesn't today -- not because removing any one
of them (other than `reg') changes this function's observable output on
the CURRENT grammar. `wand'/`wor'/`tri0'/`tri1'/etc. were never
individually dump-verified the way `wire'/`trireg'/`reg' were; they are
assumed to share `wire''s own `net_type' shape on the strength of the
LRM's single `net_type' grammar production covering all twelve, not on
a separate probe per keyword.")

(defun verilog-auto--decl-type-text (decl)
  "Declared type keyword text for port-declaration node DECL (an
`ansi_port_declaration', or a non-ANSI `input_declaration'/
`output_declaration'/`inout_declaration'), or nil (M125) when DECL names
no explicit type at all, or names one of
`verilog-auto--net-type-keywords' (M125 fix round: widened from `wire'
alone -- see that constant's own doc string). `output wire foo'/
`output [7:0] foo' both structurally lack any `data_type' node at all
(dump-verified, M125 recon: `wire' parses under `net_port_type'/
`net_type', an implicit/untyped port under `net_port_type'/
`data_type_or_implicit'/`implicit_data_type' -- neither is `data_type'),
so the absent-type half of the omission rule is free; `reg', by
contrast, DOES produce a real `data_type' node (dump-verified: `output
reg [WIDTH-1:0] bin_count' parses `data_type' -> `integer_vector_type'
\"reg\", the identical shape `logic' produces) -- the text-level check
below is what catches that half, not a structural absence.
When `data_type' IS present, its OWN FIRST child (`integer_vector_type'
for `logic'/`reg'/`bit', or `class_type' for a scoped user type like
`soc_pkg::alu_op_e') is the type keyword text alone -- a packed
dimension, when present, is `data_type''s own SECOND child (dump-
verified), never nested inside the first, so this never accidentally
appends a range onto the type text; range is `verilog-auto--range-text-
of''s own separate job."
  (let ((dt (verilog-auto--find-first-of-type decl "data_type")))
    (when dt
      (let ((head (treesit-node-child dt 0)))
        (and head
             (let ((txt (treesit-node-text head)))
               (unless (member txt verilog-auto--net-type-keywords) txt)))))))

;; --- M126: AUTOREG/AUTOTIEOFF shared port-typing/range/constant helpers --

(defun verilog-auto--decl-raw-type-keyword (decl)
  "DECL's own explicit type keyword text, with NONE of
`verilog-auto--decl-type-text''s omissions -- `reg' comes back as
itself here. That function deliberately HIDES `reg' (returns nil for
it) because its own caller generates a NEW propagated port declaration
where `reg' would be illegal on a continuously-driven port; AUTOREG's
and AUTOTIEOFF's own per-port-type checks (M126) need the raw answer,
not that filtered one -- `verilog-auto--decl-port-reg-p' below relies on
seeing \"reg\" itself. nil when DECL has no `data_type' node at all: an
implicit/untyped port (`signed' or not -- R15 -- still has no
`data_type', only `implicit_data_type') or an explicit net-type keyword
like `wire' (which never produces a `data_type' node either, M126 spec
section 3 probe: `output wire b;' parses `net_port_type' > `net_type' >
`wire', no `data_type' anywhere)."
  (let ((dt (verilog-auto--find-first-of-type decl "data_type")))
    (and dt (let ((head (treesit-node-child dt 0))) (and head (treesit-node-text head))))))

(defun verilog-auto--decl-has-type-keyword-p (decl)
  "Non-nil if DECL (a non-ANSI `output_declaration') carries an
explicit type on the PORT itself: a SystemVerilog variable type
(`verilog-auto--decl-raw-type-keyword' non-nil -- `logic'/`reg'/a
scoped user type) OR an explicit net-type keyword (`wire' and friends,
caught via a `net_type' descendant since it structurally never produces
a `data_type' node -- see that function's own doc string). AUTOREG
skips such a port PER-PORT (R7/R12/R13/R14, this file's M126 header);
`signed' alone does NOT count (R15 -- it lives under
`implicit_data_type', producing neither shape)."
  (or (verilog-auto--decl-raw-type-keyword decl)
      (verilog-auto--find-first-of-type decl "net_type")))

(defun verilog-auto--decl-port-reg-p (decl)
  "Non-nil if DECL's own raw type keyword
(`verilog-auto--decl-raw-type-keyword') is exactly \"reg\" -- M126
divergence 3: AUTOTIEOFF skips a port already declared `reg' ON THE
PORT ITSELF (T10 -- GNU ties it off anyway, a real duplicate
declaration); `logic'-typed and plain `wire'-typed ports are NOT
skipped here (T10's own \"all three tied off\" measurement, this
file's M126 header) -- only `reg' produces an actual re-declaration
conflict worth refusing."
  (equal (verilog-auto--decl-raw-type-keyword decl) "reg"))

(defun verilog-auto--decl-signed-p (decl)
  "Non-nil if DECL carries an explicit `signed' keyword child anywhere
\(a leaf token, treesit type \"signed\" -- M126 spec section 3 probe:
`output signed [3:0] a;' parses `signed' as its own child of
`implicit_data_type', a sibling of `packed_dimension', never nested
inside it) -- R15/T19."
  (and (verilog-auto--find-first-of-type decl "signed") t))

(defun verilog-auto--all-range-texts (decl)
  "Every `packed_dimension' descendant of DECL, whitespace-normalized
(`verilog-auto--normalize-range-whitespace'), in document order.
Unlike `verilog-auto--range-text-of' (which returns only the FIRST),
this returns ALL of them -- M126 spec section 3 probe 3 (real tree
dump): `output [3:0][1:0] a;' parses TWO SIBLING `packed_dimension'
nodes directly under `implicit_data_type' (or `data_type', for a
`logic'/`reg'-typed port), never one nested inside the other, and
`verilog-auto--find-all-of-type' (which `find-first-of-type' shares its
walk with) never recurses into a node that itself already matched, so a
plain `find-all-of-type decl \"packed_dimension\"' collects exactly the
sibling set with no risk of over-collecting a dimension bound's own
nested range expression (a `packed_dimension' is never itself nested
inside another)."
  (mapcar (lambda (n) (verilog-auto--normalize-range-whitespace (treesit-node-text n)))
          (verilog-auto--find-all-of-type decl "packed_dimension")))

(defun verilog-auto--joined-range-text (decl)
  "DECL's own dimensions (`verilog-auto--all-range-texts'), joined by a
single space -- `[3:0] [1:0]' shape, R18."
  (string-join (verilog-auto--all-range-texts decl) " "))

;; --- M127: AUTOINST's `[]'/`[][]' dimension lookup -----------------------

(defun verilog-auto--unpacked-dims-after (id-node)
  "Every `unpacked_dimension' node that is ID-NODE's own following
sibling, stopping at the first sibling that ISN'T one (a `,' token, or
the next name in a comma-separated list) -- section 3.3's `mirror
verilog-auto--all-range-texts' own documented reasoning about sibling
nodes', applied to the UNPACKED case: the grammar attaches a name's own
array dimensions directly after IT (`ansi_port_declaration: ...
field(\"port_name\", ...) repeat(unpacked_dimension) ...', and
`list_of_port_identifiers'/`list_of_variable_port_identifiers':
`identifier repeat(unpacked_dimension)' per comma-separated name,
tree-sitter-systemverilog 0.4.0 grammar.js -- dump-verified against
this crate's own vendored grammar source), so reading exactly the
dimensions immediately following ID-NODE -- never a LATER comma-
separated sibling name's own -- is what keeps a multi-name declaration
\(`output [7:0] a [0:3], b;') from attributing `a''s own unpacked range
to `b' or vice versa. ID-NODE's parent is the declaration node itself
for an ANSI port (`port_name' is a direct field child there) or the
`list_of_..._identifiers' node for a non-ANSI one; both shapes work
identically through `verilog-auto--child-index'."
  (let ((parent (treesit-node-parent id-node)))
    (when parent
      (let* ((idx (verilog-auto--child-index parent id-node))
             (n (treesit-node-child-count parent))
             (i (and idx (1+ idx)))
             (acc nil) (stop nil))
        (while (and i (< i n) (not stop))
          (let ((child (treesit-node-child parent i)))
            (if (string= (treesit-node-type child) "unpacked_dimension")
                (push child acc)
              (setq stop t)))
          (setq i (1+ i)))
        (nreverse acc)))))

(defun verilog-auto--all-unpacked-range-texts-after (id-node)
  "Text of every `verilog-auto--unpacked-dims-after' result for ID-NODE,
whitespace-normalized (`verilog-auto--normalize-range-whitespace')."
  (mapcar (lambda (n) (verilog-auto--normalize-range-whitespace (treesit-node-text n)))
          (verilog-auto--unpacked-dims-after id-node)))

(defun verilog-auto--ansi-port-dims (decl)
  "(NAME PACKED-LIST UNPACKED-LIST MODPORT) for ANSI `ansi_port_declaration'
DECL -- PACKED-LIST every packed dimension DECL carries
\(`verilog-auto--all-range-texts', document order), UNPACKED-LIST every
unpacked dimension following DECL's own `port_name' field
\(`verilog-auto--all-unpacked-range-texts-after'). Needed by AUTOINST's
`[]'/`[][]' template tokens (section 2.3/3.3) -- NOT part of the
existing 3-tuple/4-tuple port-list contracts, a separate, parallel
lookup (`verilog-auto--module-port-dims') filled at the same one lookup
point (`verilog-auto--module-ports') as those caches already are.

MODPORT (M128) is the text of an `interface_port_header' descendant's
own `modport_name' field, or nil for any port that isn't interface-typed
at all, or one that is but omits the (optional, per the LRM -- `axi_if
bus;' with no `.mst' modport is legal SV, see `verilog-highlights.scm's
own comment on this same field) modport clause. Feeds `vl-modport'
\(section 2.3) -- the fourth element added to what used to be a plain
3-tuple; every existing caller reads only the first three via `nth', so
this is additive, not a breaking change to the contract those callers
already depend on."
  (let* ((name-node (treesit-node-child-by-field-name decl "port_name"))
         (iph (verilog-auto--find-first-of-type decl "interface_port_header"))
         (mp-node (and iph (treesit-node-child-by-field-name iph "modport_name"))))
    (list (treesit-node-text name-node)
          (verilog-auto--all-range-texts decl)
          (verilog-auto--all-unpacked-range-texts-after name-node)
          (and mp-node (treesit-node-text mp-node)))))

(defun verilog-auto--nonansi-port-dims (module-decl)
  "Like `verilog-auto--ansi-port-dims', but for every non-ANSI body
`input_declaration'/`output_declaration'/`inout_declaration' in
MODULE-DECL -- one (NAME PACKED-LIST UNPACKED-LIST MODPORT) entry per
declared identifier, PACKED-LIST shared across every name in the SAME
comma-separated declaration (it precedes the identifier list, so it
isn't attached per-name the way UNPACKED-LIST is). MODPORT is always
nil here (M128): a non-ANSI header has no `interface_port_header' shape
at all -- that header kind is exclusively an ANSI-port construct.

M127 fix round: a cold review found no test anywhere in this
milestone's own diff exercised a NON-ANSI-declared port carrying an
unpacked dimension (every `[]'/`[][]' unpacked-array test used an ANSI
header instead) -- confirmed with a REAL dump of both non-ANSI shapes
(`module top(a, b); input [7:0] a, b [0:3]; endmodule' for
`list_of_port_identifiers', and the `output logic [7:0] a, b [0:3];'
typed variant for `list_of_variable_port_identifiers'): both parse with
`simple_identifier'/`,'/`simple_identifier'/`unpacked_dimension' as
FLAT, DIRECT children of the identifier-list node, exactly as
`verilog-auto--unpacked-dims-after's own doc string already claimed --
the grammar assumption was correct, this was purely a missing-test gap,
not a code bug. `dolist' below (`find-all-of-type', not `find-first-of-
type') is what makes the MULTI-name case work at all -- a mutation that
narrows it to only the first name in each declaration is now caught by
`autotemplate_nonansi_multiname_unpacked_dimension_second_name_only' in
verilog_auto_tests.rs, which pins EXACTLY that: `a' (no unpacked
dimension) and `b' (has one) sharing one `input [7:0] a, b [0:3];'
declaration must each get their own correct dims entry."
  (let (acc)
    (dolist (kind '("input_declaration" "output_declaration" "inout_declaration"))
      (dolist (decl (verilog-auto--find-all-of-type module-decl kind))
        (let* ((packed (verilog-auto--all-range-texts decl))
               (idlist (or (verilog-auto--find-first-of-type decl "list_of_port_identifiers")
                           (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers"))))
          (when idlist
            (dolist (id (verilog-auto--find-all-of-type idlist "simple_identifier"))
              (push (list (treesit-node-text id) packed
                          (verilog-auto--all-unpacked-range-texts-after id)
                          nil)
                    acc))))))
    (nreverse acc)))

(defun verilog-auto--ports-of-module-dims (module-decl)
  "Every port's (NAME PACKED-LIST UNPACKED-LIST) triple for MODULE-DECL,
ANSI or non-ANSI header alike -- see `verilog-auto--ansi-port-dims'/
`verilog-auto--nonansi-port-dims'."
  (let ((header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-p header)
        (mapcar #'verilog-auto--ansi-port-dims
                (verilog-auto--find-all-of-type header "ansi_port_declaration"))
      (verilog-auto--nonansi-port-dims module-decl))))

(defun verilog-auto--range-bounds (range-text)
  "RANGE-TEXT (one dimension's own bracketed, whitespace-normalized
text, e.g. \"[3:0]\") as a (MSB . LSB) cons of trimmed expression-text
strings. The bracket delimiters and the single top-level `:' splitting
the two bounds are structural: a `packed_dimension' is always `['
`constant_range' `]', and `constant_range' is always EXPR `:' EXPR
(dump-verified, M39/M126) -- no expression form this milestone's own
scope produces (numeric literals, bare identifiers, and `+'/`-'/`*'
arithmetic over them) contains a bare top-level `:' of its own."
  (let* ((inner (substring range-text 1 (1- (length range-text))))
         (colon (string-match ":" inner)))
    (cons (string-trim (substring inner 0 colon))
          (string-trim (substring inner (1+ colon))))))

(defun verilog-auto--numeric-p (text)
  "Non-nil if TEXT is nothing but decimal digits -- a bare numeric range
bound, as opposed to a parameter name or arithmetic expression."
  (string-match-p "\\`[0-9]+\\'" text))

(defun verilog-auto--dimension-width (msb lsb)
  "Numeric width of one dimension whose bounds are both
`verilog-auto--numeric-p': `abs(msb-lsb)+1' (R8 -- an ascending range
like `[0:3]' counts the same as a descending one)."
  (1+ (abs (- (string-to-number msb) (string-to-number lsb)))))

(defun verilog-auto--symbolic-tieoff-body (msb lsb)
  "The `{...}' tie-off constant for a single SYMBOLIC dimension (MSB .
LSB not both `verilog-auto--numeric-p') -- spec section 1.3, measured
GNU Emacs 30.2. Special case: MSB matches an identifier followed by
`-1' (whitespace tolerated around the `-', M126 fix round -- see
below), AND LSB is exactly \"0\" -> `{IDENT{1'b0}}' (`[WIDTH-1:0]' ->
`{WIDTH{1'b0}}'; `[2*W-1:0]' does NOT take this -- its own MSB text,
\"2*W-1\", is not a BARE identifier minus one, so it falls through to
the general form below, exactly matching the measured
`{(1+(2*W-1)){1'b0}}'). General form otherwise:
`{(1+(MSB)[-(LSB)]){1'b0}}', the `-(LSB)' term omitted only when LSB is
exactly \"0\" (`[WIDTH:0]' -> `{(1+(WIDTH)){1'b0}}'; `[N:1]' keeps the
term -> `{(1+(N)-(1)){1'b0}}'; `[7:LSB]' -- a NUMERIC msb with a
SYMBOLIC lsb -- still takes this general form, wrapping the numeric msb
in parens the same as a symbolic one -> `{(1+(7)-(LSB)){1'b0}}'). This
form never carries a `'h0'/`'sh0' suffix -- the braced expression IS
the whole constant (signed does not affect it, spec section 1.3's own
last row).

M126 fix round: a cold review, reading GNU's own regex directly
(`verilog-mode.el:11427', `\"^\\\\s *\\\\([a-zA-Z_][a-zA-Z0-9_]*\\\\)\\\\s
*-\\\\s *1\\\\s *:\\\\s *0\\\\s *$\"'), found it tolerates whitespace
around every token and matches the WHOLE range text, msb and lsb
together -- this file's original regex matched only MSB, and matched
`-1' with no whitespace at all. Confirmed against real GNU Emacs 30.2
(three ports, one `verilog-auto' run): `output [WIDTH - 1:0] a;' and
`output [ WIDTH-1 : 0 ] b;' BOTH take the special case
(`{WIDTH{1'b0}}'), same as plain `[WIDTH-1:0]'. The MSB-only match
below still suffices because `verilog-auto--range-bounds' already
`string-trim's each half across the `:' before this function ever sees
LSB (so `[ WIDTH-1 : 0 ]' arrives here as MSB \"WIDTH-1\", LSB \"0\",
already clean) -- the ONLY whitespace shape that reaches this function
un-trimmed is INSIDE MSB itself, around the `-' (`[WIDTH - 1:0]' ->
MSB \"WIDTH - 1\"), which is what the `[ \\t]*' additions below cover.

Also measured, and deliberately NOT changed: GNU rewrites the emitted
DECLARATION's own range text back to a canonical `[WIDTH-1:0]' for
both `[WIDTH - 1:0]' and `[ WIDTH-1 : 0 ]' (`wire [WIDTH-1:0] a = ...;'
in both cases) -- this file keeps the user's own range text verbatim
(`verilog-auto--normalize-range-whitespace' only collapses whitespace
RUNS and trims immediately inside the brackets, e.g. `[WIDTH - 1:0]'
prints as-is). Preserving what the user actually wrote is the more
conservative choice and matches this file's existing policy everywhere
else (R8's own symbolic-range-copied-verbatim rule) -- this is a new,
deliberate divergence, not an oversight."
  (if (and (string= lsb "0")
           (string-match "\\`\\([A-Za-z_$][A-Za-z0-9_$]*\\)[ \t]*-[ \t]*1\\'" msb))
      (format "{%s{1'b0}}" (match-string 1 msb))
    (format "{(1+(%s)%s){1'b0}}" msb
            (if (string= lsb "0") "" (format "-(%s)" lsb)))))

(defun verilog-auto--tieoff-constant (dims signed)
  "The tie-off constant text for DIMS (a possibly-empty list of
`verilog-auto--all-range-texts' bracketed dimension strings) and SIGNED
(`verilog-auto--decl-signed-p'). Returns (CONST . SKIP-REASON):
SKIP-REASON non-nil means DIMS could not be constant-folded and the
caller must skip the signal instead (M126 divergence 5 -- a
multi-dimensional range with at least one symbolic dimension).
- No dimension: `1'h0'/`1'sh0'.
- One dimension, both bounds numeric: `<w>\\='h0'/`<w>\\='sh0' (spec
  section 1.3).
- One dimension, symbolic: `verilog-auto--symbolic-tieoff-body' (no
  `'h0' suffix -- the braced form IS the whole constant; SIGNED has no
  effect on it).
- 2+ dimensions, every bound of every dimension numeric: the PRODUCT of
  each dimension's own width (divergence 5 -- GNU instead silently uses
  only the LAST dimension, T16), `<product>\\='h0'/`<product>\\='sh0'.
- 2+ dimensions, any dimension symbolic: (nil . 'symbolic-multidim) --
  Reticle refuses to emit a width it cannot justify, recording a notice
  instead of GNU's silently-under-reported one (divergence 5)."
  (cond
   ((null dims)
    (cons (if signed "1'sh0" "1'h0") nil))
   ((= (length dims) 1)
    (let* ((b (verilog-auto--range-bounds (car dims)))
           (msb (car b)) (lsb (cdr b)))
      (if (and (verilog-auto--numeric-p msb) (verilog-auto--numeric-p lsb))
          (cons (format "%d'%s0" (verilog-auto--dimension-width msb lsb) (if signed "sh" "h")) nil)
        (cons (verilog-auto--symbolic-tieoff-body msb lsb) nil))))
   (t
    (let ((bounds (mapcar #'verilog-auto--range-bounds dims)))
      (if (verilog-auto--filter
           (lambda (b) (not (and (verilog-auto--numeric-p (car b)) (verilog-auto--numeric-p (cdr b)))))
           bounds)
          (cons nil 'symbolic-multidim)
        (let ((width (apply #'* (mapcar (lambda (b) (verilog-auto--dimension-width (car b) (cdr b))) bounds))))
          (cons (format "%d'%s0" width (if signed "sh" "h")) nil)))))))

(defun verilog-auto--output-decl-names (decl)
  "Every name DECL (a non-ANSI `output_declaration') declares -- its own
`list_of_port_identifiers' (an untyped port, `output [3:0] a, b;') or
`list_of_variable_port_identifiers' (a typed one, `output logic a, b;'
-- M125 recon, the same two-shape split
`verilog-auto--nonansi-port-info-full' already handles), so a
comma-separated multi-name declaration contributes every name, not just
the first."
  (let ((idlist (or (verilog-auto--find-first-of-type decl "list_of_port_identifiers")
                     (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers"))))
    (and idlist (mapcar #'treesit-node-text (verilog-auto--find-all-of-type idlist "simple_identifier")))))

(defun verilog-auto--output-port-candidate-decls (module-decl header)
  "Every (NAME . DECL) pair for MODULE-DECL's own `output' ports, ANSI
or non-ANSI: an ANSI header contributes one pair per `ansi_port_
declaration' whose own direction (`verilog-auto--port-direction-of') is
'output, NAME from its `port_name' field; a non-ANSI header contributes
one pair per name in every body `output_declaration'
(`verilog-auto--output-decl-names', so a comma-separated multi-name
declaration contributes every name, not just the first).

Both AUTOREG and AUTOTIEOFF (M126) need this. A DECL node returned here
-- either shape -- is fed unchanged into `verilog-auto--decl-raw-type-
keyword'/`verilog-auto--decl-signed-p'/`verilog-auto--all-range-texts':
all three walk generically (a plain `find-first-of-type'/`find-all-of-
type' search for `data_type'/`net_type'/`signed'/`packed_dimension'
anywhere under DECL), which a real tree dump (M126 recon) confirms
already works unchanged on an `ansi_port_declaration' -- its own
`variable_port_header'/`net_port_header' wraps the identical `variable_
port_type'/`net_port_type' shape a non-ANSI `output_declaration' does,
just one level deeper. AUTOREG itself only ever calls this on a
NON-ANSI header (it bails out entirely on ANSI, R2, before reaching
candidate computation) -- but AUTOTIEOFF needs BOTH shapes, since
divergence 2 exists specifically so AUTOTIEOFF is useful on
`demo/rtl/', which is entirely ANSI SystemVerilog. Missing this ANSI
branch entirely was a real M126 fix-round bug: an ANSI-header
AUTOTIEOFF site silently expanded to NOTHING instead of the `assign'
form divergence 2 promises, because `output_declaration' (the non-ANSI
node type) never appears at all in an ANSI header.

M134: gated on `verilog-auto--ansi-header-with-ports-p', not the bare
`verilog-auto--ansi-header-p' -- a port-less ANSI header (`module top;')
has no `ansi_port_declaration' children to enumerate in the first
branch, but DOES have body-level `output_declaration' nodes once
AUTOOUTPUT has generated them (or if the user wrote any by hand, which
is legal Verilog even with no header port list). Falling into the
non-ANSI branch below is what lets AUTOREG/AUTOTIEOFF see those."
  (if (verilog-auto--ansi-header-with-ports-p header)
      (let (acc)
        (dolist (decl (verilog-auto--find-all-of-type header "ansi_port_declaration"))
          (when (eq (verilog-auto--port-direction-of decl) 'output)
            (push (cons (treesit-node-text (treesit-node-child-by-field-name decl "port_name")) decl) acc)))
        (nreverse acc))
    (let (acc)
      (dolist (decl (verilog-auto--find-all-of-type module-decl "output_declaration"))
        (dolist (nm (verilog-auto--output-decl-names decl))
          (push (cons nm decl) acc)))
      (nreverse acc))))

(defun verilog-auto--one-ansi-port (decl)
  (list (treesit-node-text (treesit-node-child-by-field-name decl "port_name"))
        (verilog-auto--port-direction-of decl)
        (verilog-auto--range-text-of decl)))

(defun verilog-auto--one-ansi-port-full (decl)
  "Like `verilog-auto--one-ansi-port', but a 4-tuple with
`verilog-auto--decl-type-text' appended (M125)."
  (list (treesit-node-text (treesit-node-child-by-field-name decl "port_name"))
        (verilog-auto--port-direction-of decl)
        (verilog-auto--range-text-of decl)
        (verilog-auto--decl-type-text decl)))

(defun verilog-auto--ansi-ports (header)
  (mapcar #'verilog-auto--one-ansi-port
          (verilog-auto--find-all-of-type header "ansi_port_declaration")))

(defun verilog-auto--ansi-ports-full (header)
  (mapcar #'verilog-auto--one-ansi-port-full
          (verilog-auto--find-all-of-type header "ansi_port_declaration")))

(defun verilog-auto--nonansi-port-info-full (module-decl)
  "Alist-shaped list of (NAME DIRECTION RANGE-TEXT TYPE-TEXT) from
MODULE-DECL's own body port_declaration items (input_declaration/
output_declaration/inout_declaration), one entry per declared
identifier. Deliberately scoped to each declaration's OWN
`list_of_port_identifiers'/`list_of_variable_port_identifiers' rather
than a blanket search of the whole declaration -- a range like
`[WIDTH-1:0]' also contains a `simple_identifier' (for `WIDTH' itself),
which a broader search would wrongly collect as if it were a port name.

M125 bugfix, folded in here rather than left as a separate pre-existing
gap: a TYPED body declaration (`output logic rvalid_o;') parses its own
identifier list under `list_of_variable_port_identifiers', NOT the
`list_of_port_identifiers' this function used to search exclusively --
confirmed by a real tree dump (M125 recon), the same gap
`verilog-auto--declared-names' had (see this file's M125 header for the
full story, including the real submodule this broke:
`demo/rtl-verilog2001/gray_ctr.v''s own `output reg [WIDTH-1:0]
bin_count;'). `verilog-auto--nonansi-port-info' (the pre-M125 3-tuple
function every earlier caller -- `verilog-auto--nonansi-ports', AUTOARG
-- still uses) is now a thin wrapper around this one, so the fix reaches
every caller at once rather than only the M125-specific full variant."
  (let (acc)
    (dolist (kind '(("input_declaration" . input)
                    ("output_declaration" . output)
                    ("inout_declaration" . inout)))
      (dolist (decl (verilog-auto--find-all-of-type module-decl (car kind)))
        (let* ((range (verilog-auto--range-text-of decl))
               (type (verilog-auto--decl-type-text decl))
               (idlist (or (verilog-auto--find-first-of-type decl "list_of_port_identifiers")
                           (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers")))
               (names (and idlist
                           (mapcar #'treesit-node-text
                                   (verilog-auto--find-all-of-type idlist "simple_identifier")))))
          (dolist (nm names)
            (push (list nm (cdr kind) range type) acc)))))
    (nreverse acc)))

(defun verilog-auto--nonansi-port-info (module-decl)
  "3-tuple (NAME DIRECTION RANGE-TEXT) compatibility view of
`verilog-auto--nonansi-port-info-full', preserving every pre-M125
caller's own contract exactly."
  (mapcar (lambda (e) (list (nth 0 e) (nth 1 e) (nth 2 e)))
          (verilog-auto--nonansi-port-info-full module-decl)))

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

(defun verilog-auto--nonansi-ports-full (module-decl header)
  "Like `verilog-auto--nonansi-ports', but 4-tuples with type text
(M125), looked up from `verilog-auto--nonansi-port-info-full'."
  (let ((names (mapcar #'treesit-node-text (verilog-auto--find-all-of-type header "port")))
        (info (verilog-auto--nonansi-port-info-full module-decl)))
    (mapcar (lambda (nm)
              (let ((entry (assoc nm info)))
                (list nm
                      (if entry (nth 1 entry) 'input)
                      (if entry (nth 2 entry) nil)
                      (if entry (nth 3 entry) nil))))
            names)))

(defun verilog-auto--ports-of-module (module-decl)
  (let ((header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-p header)
        (verilog-auto--ansi-ports header)
      (verilog-auto--nonansi-ports module-decl header))))

(defun verilog-auto--ports-of-module-full (module-decl)
  "Like `verilog-auto--ports-of-module', but 4-tuples with type text
(M125) -- see `verilog-auto--module-ports' for the one caller/cache."
  (let ((header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-p header)
        (verilog-auto--ansi-ports-full header)
      (verilog-auto--nonansi-ports-full module-decl header))))

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

;; --- Shared Interfaces/Outputs/Inouts/Inputs grouping and line formatting -

(defun verilog-auto--group-by-direction (ports)
  "PORTS (a list of (NAME DIRECTION RANGE) triples) split into four
lists (INTERFACES OUTPUTS INOUTS INPUTS), each preserving PORTS' own
relative order -- the grouping both AUTOINST and AUTOARG use.

M124: added the INTERFACES bucket (`verilog-auto--port-direction-of'
now returns 'interface for an interface-typed ANSI port). AUTOARG's own
port source, `verilog-auto--nonansi-port-info', only ever classifies
input_declaration/output_declaration/inout_declaration body items (an
interface can't be one of those), so this bucket is unconditionally
empty for AUTOARG and `verilog-auto--grouped-lines' below contributes no
header or lines for an empty group -- AUTOARG's own three-category
output is therefore unchanged by this addition, pinned by
`autoarg_output_unchanged_by_the_interfaces_bucket' in
verilog_auto_tests.rs. Reference: real GNU Emacs 30.2's own
`verilog-mode.el' AUTOARG (`verilog-auto-arg', :12124-12143) has NO
interfaces category at all -- only AUTOINST's `verilog-auto-inst'
(:12852-12862) does, from a dedicated `verilog-decls-get-interfaces'
call GNU never threads through AUTOARG. Sharing this one grouping
function (rather than forking AUTOARG a second, near-identical copy) is
therefore only safe because the bucket is structurally always empty on
that path, not because AUTOARG is meant to grow a fourth category."
  (let (interfaces outputs inouts inputs)
    (dolist (p ports)
      (cond ((eq (nth 1 p) 'interface) (push p interfaces))
            ((eq (nth 1 p) 'output) (push p outputs))
            ((eq (nth 1 p) 'inout) (push p inouts))
            (t (push p inputs))))
    (list (nreverse interfaces) (nreverse outputs) (nreverse inouts) (nreverse inputs))))

(defun verilog-auto--pad-to-column (s col &optional offset)
  "S with trailing spaces so it reaches column COL, measuring S as
starting at column OFFSET (default 0) -- callers pass OFFSET as the
length of whatever prefix (e.g. indentation) precedes S on the real
buffer line but isn't part of S itself. At least one space is always
added."
  (concat s (make-string (max 1 (- col (+ (or offset 0) (length s)))) ?\s)))

(defun verilog-auto--grouped-lines (groups indent format-fn &optional annotate-fn)
  "GROUPS is (INTERFACES OUTPUTS INOUTS INPUTS) (M124: extended with the
leading INTERFACES bucket -- see `verilog-auto--group-by-direction's own
doc); FORMAT-FN maps one port triple to its own un-indented, uncomma'd
text. Builds an INDENT + \"// Interfaces\"/\"// Outputs\"/\"// Inouts\"/
\"// Inputs\" header before each non-empty group (a group with zero
members contributes no header and no lines at all), then INDENT +
(FORMAT-FN PORT) + \",\" per member -- except the very last connection
line overall, which gets no comma. Shared by AUTOINST and AUTOARG,
whose grouping/comma/empty-group rules are identical; the INTERFACES
header is unreachable from AUTOARG in practice (see
`verilog-auto--group-by-direction's M124 note) but the label is listed
here regardless so a future AUTOARG source that DID supply an
interface-typed entry would still print a correctly-labelled group
rather than silently mislabeling it as an input.

ANNOTATE-FN is new and OPTIONAL (M127; AUTOARG passes none, so its own
output is byte-for-byte unchanged from before this milestone -- pinned
by `autoarg_output_byte_identical_before_m127' in
verilog_auto_tests.rs). When omitted, the return value is the bare list
of finished lines, exactly as before M127. When given, it maps the SAME
port triple FORMAT-FN was just called with to a trailing annotation
string (only `// Templated', section 2.4) or nil -- called AFTER
FORMAT-FN for that port, so a shared side channel FORMAT-FN populates
as its own side effect (the way `verilog-auto--inst-lines' records
'this port's EXPR came from a template rule') is already filled in by
the time ANNOTATE-FN reads it. Group header lines never take an
annotation. In this mode the return value is (LINES COLUMN
LAST-ANNOTATION), not the bare LINES list:
- Every connection line EXCEPT THE LAST already has its own annotation
  appended -- padded with spaces to one column past the longest
  TERMINATED CONNECTION line in this SAME call (group headers excluded
  from that measurement), minimum one space, section 2.4's own
  alignment rule, computed purely from the lines already in hand (no
  new user-facing column variable).
- The LAST connection line is left EXACTLY as it always was --
  comma-stripped, nothing appended -- because its own real terminator
  is the caller's pre-existing closing paren(s)/`;', which lives PAST
  this function's own return value in the buffer (section 2.4's
  \"after the `));' on the last line\" case, `verilog-auto--expand-
  autoinst-site' does the actual appending once it knows what real
  buffer column that terminator ends at); LAST-ANNOTATION is that
  line's own annotation text (or nil), and COLUMN is the SAME shared
  alignment column the caller must pad the last line to itself.

Note on the comma strip below: this deliberately does NOT use GNU's
own idiom `(setcar (last list) ...)' -- this interpreter's `last'
builds a fresh spliced-off list rather than returning a shared tail of
the original (unlike real Emacs), so `setcar' on its result wouldn't
touch LINES at all. Stripping the first element of the STILL-REVERSED
accumulator (which is exactly the last line in final order, since it
was the most recently `push'ed) sidesteps that entirely."
  (let ((labels '("// Interfaces" "// Outputs" "// Inouts" "// Inputs"))
        (gs groups)
        (lines nil)
        (kinds nil)) ;; lock-step with LINES: 'header, or the annotation string/nil
    (while gs
      (let ((group (car gs)) (label (car labels)))
        (when group
          (push (concat indent label) lines)
          (push 'header kinds)
          (dolist (p group)
            (push (concat indent (funcall format-fn p) ",") lines)
            (push (and annotate-fn (funcall annotate-fn p)) kinds))))
      (setq gs (cdr gs) labels (cdr labels)))
    (when lines
      (setcar lines (substring (car lines) 0 (1- (length (car lines))))))
    (setq lines (nreverse lines) kinds (nreverse kinds))
    (if (not annotate-fn)
        lines
      (let* ((n (length lines))
             (conn-lens
              (let (acc (ls lines) (ks kinds))
                (while ls
                  (unless (eq (car ks) 'header) (push (length (car ls)) acc))
                  (setq ls (cdr ls) ks (cdr ks)))
                acc))
             (col (1+ (apply #'max 0 conn-lens)))
             (last-ann (and (> n 0) (let ((k (nth (1- n) kinds))) (and (stringp k) k))))
             (i 0) (out nil))
        (while (< i n)
          (let* ((line (nth i lines)) (k (nth i kinds)) (is-last (= i (1- n))))
            (push (if (and (stringp k) (not is-last))
                      (concat (verilog-auto--pad-to-column line col) k)
                    line)
                  out))
          (setq i (1+ i)))
        (list (nreverse out) col last-ann)))))

;; --- AUTO_TEMPLATE (M92) ---------------------------------------------------
;;
;; GNU's own shape (verilog-mode.el's `verilog-read-auto-template-middle'/
;; `verilog-auto-inst-port'), read from the real 30.2 source rather than
;; guessed:
;;   /* InstModule AUTO_TEMPLATE (
;;      .name  (expr-with-\\1-etc),
;;      .other (expr),
;;      ); */
;; Two rule shapes inside the parens, distinguished by trying the EXACT
;; form's regexp first and only falling back to the WILDCARD form if that
;; fails (`cond' order in GNU's own `verilog-auto-inst-port', :12325-
;; :12334) -- so a plain-identifier LHS like `.clk_i' is always an exact
;; rule, never accidentally treated as a (trivial, single-literal-match)
;; wildcard:
;; - EXACT: `.NAME (EXPR)' where NAME is a bare identifier
;;   ([A-Za-z0-9_$]+, no regex metacharacters) -- matches by NAME equality
;;   against the port name, EXPR substituted in verbatim (no backrefs).
;; - WILDCARD: `.PATTERN (EXPR)' where PATTERN is itself an elisp regexp
;;   (GNU's own LHS charset for this form, `verilog-mode.el' :10232-
;;   :10234, allows ordinary identifier chars plus `+@^.*?|[]' and
;;   backslash-escaped `(', `)', `|', digits -- i.e. exactly the
;;   metacharacters a hand-written regexp needs, deliberately excluding a
;;   literal, unescaped `(' or `)' so the boundary between PATTERN and the
;;   following `(EXPR)' is never ambiguous). PATTERN is anchored `^...$'
;;   (GNU wraps it that way itself, :10232) and matched against the WHOLE
;;   port name; EXPR may reference `\\1'..`\\9' captured from that match.
;;
;; M92 v1 scope cut (see this file's own top-of-file header): `@'
;; per-instance numbering and the `[]'/`[][]' magic bit-range tokens
;; were both excluded here -- M127 ships both (section below), and M128
;; ships `@"(lisp-expr)"' evaluated templates (own section below) --
;; the escape hatch this M92 cut originally deferred, kept out of M127
;; because it is the one construct where GNU has ZERO error handling
;; (so this project had to design its own divergence from scratch, not
;; just measure GNU's) and because it depends on M127's own `@'
;; machinery (GNU textually substitutes `@' into the expression SOURCE
;; before ever reading it as lisp) -- keeping it out of M127 kept that
;; diff reviewable. Still EXCLUDED, per this milestone's own spec
;; section 0:
;; - One template body shared by several module names (GNU's own `,'
;;   module-name list before AUTO_TEMPLATE).
;; - `verilog-auto-inst-template-numbers' set to `t' (GNU's absolute-
;;   line-number annotation form, `// Templated 7') -- needs GNU's own
;;   two-pass resolution (emit a relative marker, then rewrite it once
;;   every phase has finished shifting the buffer's lines,
;;   `verilog-auto-templated-rel', verilog-mode.el:14728), disproportionate
;;   for a non-default debugging aid whose own question -- "which rule
;;   drove this line" -- the `'lhs' value (M127, shipped) already
;;   answers. Setting this variable to `t' behaves exactly like nil and
;;   records a notice naming the variable -- see that variable's own
;;   doc string -- never silently ignored.
;; `verilog-auto--substitute-params' (parameter-name substitution in a
;; RANGE like `[WIDTH-1:0]') is likewise never applied to a template
;; EXPR -- out of scope per this milestone's own spec, and GNU doesn't
;; do it either (parameter substitution there is a `verilog-mode' option
;; scoped to auto-declared wire types, a different mechanism).
;;
;; --- M127: `@' numbering, `[]'/`[][]' bit-range tokens, `// Templated' --
;;
;; Ground truth measured against real GNU Emacs 30.2 -- see the M127
;; spec's own section 2 for the exact commands run and their exact
;; output; every divergence below is deliberate, not an oversight.
;;
;; - `@' (section 2.1/2.2) substitutes the FIRST run of digits in the
;;   instance name, scanning LEFT TO RIGHT (`u2_ch3' -> `2', not `3'),
;;   unless an AUTO_TEMPLATE carries a quoted-string regexp between the
;;   keyword and its rule list's own opening paren, in which case `@' is
;;   capture group 1 of THAT regexp instead -- always group 1, even with
;;   two or more groups (GNU parity, measured). `@' has meaning ONLY
;;   inside an AUTO_TEMPLATE rule and, on the LHS, compiles into an
;;   ordinary `\\([0-9]+\\)' capture group that occupies a real,
;;   POSITIONAL numbering slot (`verilog-auto--compile-lhs-at') -- its
;;   own captured value is never carried to the RHS, which always uses
;;   the INSTANCE-derived number instead.
;; - `[]'/`[][]' (section 2.3) are functions of the CONNECTED PORT's own
;;   declared dimensionality, with NO relationship to instance arrays:
;;   `[]' is the port's own LAST (innermost) packed dimension, verbatim,
;;   or empty for a true scalar (`verilog-auto--range-text-of' returns
;;   the FIRST instead, which differs for a 2-D port); `[][]' is
;;   identical to `[]' unless the port is genuinely multi-dimensional,
;;   in which case it emits the full dimension spec wrapped in a
;;   `/* */' block comment -- a hint for a human to complete by hand,
;;   deliberately not usable Verilog.
;; - `// Templated' (section 2.4): every connection whose expression
;;   came from a template rule carries this trailing annotation; an
;;   identity connection (no rule matched) carries none. This was a
;;   real parity gap that M92's own header never even DISCLOSED -- not
;;   a sub-cut named and deferred, simply absent from the list, found
;;   only by M127 reconnaissance. GNU tab-pads it to an alignment
;;   column; this file pads with SPACES to one column past the longest
;;   terminated connection line in the same AUTOINST block (M127
;;   divergence 1 -- this file already emits spaces, never tabs,
;;   everywhere else, M126 divergence 1's own precedent).
;; - Deliberate divergences from GNU, beyond the space-padding one just
;;   above (collected in the M127 milestone record):
;;   2. `verilog-auto-inst-template-numbers' = `t' -- not implemented,
;;      behaves as nil, records a notice (never silently ignored).
;;   3. `@' resolving to the empty string (no digits, or a custom
;;      regexp that doesn't match) is silent in GNU; this file expands
;;      the same way AND records a positioned notice naming the
;;      instance -- this is the case that silently produces a WRONG
;;      signal name, not merely a cosmetic gap.
;;   4. A custom template regexp with NO capture group at all crashes
;;      GNU's entire `verilog-auto' call (`Wrong type argument: stringp,
;;      nil' out of `replace-match') and writes no output whatsoever;
;;      this file treats it as "no match" -- empty substitution, plus a
;;      notice naming the template (reusing `verilog-auto--port-marker-
;;      arg-warnings', M125's precedent for not inventing a new notice
;;      list per malformed-argument shape) -- never a crash, never an
;;      aborted run.
;;   5. GNU resolves a template by searching BACKWARD from the instance
;;      first and only then FORWARD (GNU's own source calls the forward
;;      search "not specced as working"), so a template written after
;;      instance A but before instance B is silently stolen by B. This
;;      file keeps the SAME resolution order (for GNU parity) but
;;      records a positioned notice whenever a template is resolved by
;;      the forward fallback, so the silent misattribution becomes
;;      visible instead of a quiet correctness trap.
;;
;; Known gap, pre-existing and outside this file's own scope (M92 fix
;; round S5): a LITERAL backslash written inside a wildcard EXPR is
;; silently dropped by the replacement-template handling
;; `verilog-auto--template-lookup' calls into (`replace-regexp-in-string'
;; -> `crates/elisp/src/regex.rs's `replace_all', :1643-1701) -- that
;; function's own template scanner only preserves a backslash when it is
;; immediately followed by `&', a digit, or ANOTHER backslash (two
;; backslashes in the EXPR text collapse to one literal backslash in the
;; output); any other single backslash is consumed as an escape
;; introducer and the character after it is emitted bare, unescaped. A
;; Verilog escaped identifier (e.g. `\my$signal ') begins with exactly
;; this kind of lone backslash, so a template EXPR that names one loses
;; it. Not fixable from this file alone -- the template-expansion
;; primitive is shared, generic regex machinery with its own test
;; coverage, and this milestone's spec scopes a Rust change out
;; explicitly ("if you conclude a Rust change is needed, stop and
;; report"); documented here instead of silently discovered later.
;;
;; --- M128: `@"(lisp-expr)"' evaluated AUTO_TEMPLATE tokens -----------------
;;
;; Ground truth measured against real GNU Emacs 30.2 (`verilog-mode'
;; 2024-03-01-7448f97-vpo-GNU) -- see the M128 spec's own section 2 for
;; the exact commands and their exact output. GNU's own pipeline order
;; (`verilog-auto-inst's docstring, matching what was observed): the
;; regexp template is expanded first, then `@"(...)"' is evaluated,
;; then `@'/`[]' substitution occurs -- `verilog-auto--template-
;; substitute' runs its lisp pass
;; (`verilog-auto--template-eval-lisp-tokens') FIRST for exactly this
;; reason.
;;
;; The nine bound variables (section 2.3, `verilog-auto--vl-bindings-of'):
;; `vl-name', `vl-width', `vl-bits', `vl-mbits', `vl-memory', `vl-dir',
;; `vl-modport', `vl-cell-name', `vl-cell-type' -- `vl-memory' (the
;; unpacked-array/"memory" dimension) is the real ninth; an earlier
;; reconnaissance round recorded only eight. `vl-signed'/`vl-decl'/
;; `vl-type'/`vl-array'/`vl-bounds' are NOT bound here, matching GNU
;; (probing any of them there signals `void-variable' too).
;;
;; Binding design (see `vl-name's own doc string for the full
;; reasoning): this interpreter's `eval' takes a BOOLEAN second
;; argument, not an environment, so a lexical `let' (this file is
;; `lexical-binding: t') is invisible to `eval'd code. The nine symbols
;; are `defvar'd (making them special/dynamically-scoped regardless of
;; the file's own lexical-binding setting) and `let'-bound around each
;; token's own `eval' call -- verified by execution on this checkout,
;; not re-derived from any documentation.
;;
;; Evaluation semantics (section 2.4): once per PORT, monotonically
;; across the whole buffer (never memoised, never once-per-instance);
;; `@' is substituted into the token's own unescaped source text before
;; it is read, as a blind textual replace, even inside a nested string
;; literal; the substituted RESULT is then re-scanned by the ordinary
;; `@'/`[][]'/`[]' passes (unchanged, running right after the lisp
;; pass), so a lisp expression returning `"sig@"' or `"sig[]"' still
;; picks up the instance number / port range. Whether GNU itself
;; rescans once or iterates to a fixpoint could not be determined by
;; execution (no substituted value in any measurement could contain a
;; further bare `@' or `[]' of its OWN to re-trigger the rule) --
;; reticle deliberately does a SINGLE pass; this is a considered choice
;; made where GNU's own behavior was unobservable, not a copied fact.
;;
;; Divergences from GNU (GNU has NO error handling around this
;; construct at all -- any evaluation error aborts the WHOLE
;; `verilog-auto' call and writes no output whatsoever, every other AUTO
;; in the file lost as collateral):
;;   1. reticle instead falls the ONE failing port back to an identity
;;      connection; every other port and every other AUTO in the file
;;      expands normally, and a positioned notice
;;      (`verilog-auto--template-lisp-eval-failures') records the port,
;;      the (unescaped, `@'-substituted) expression text, and the error.
;;   2. the fallen-back connection is annotated `// Templated (expression
;;      failed)' rather than a bare `// Templated'
;;      (`verilog-auto--templated-annotation'), so the failure is
;;      visible in the file itself, not only in the echo area.
;;   3. `@"..."' on a rule's LHS (port-name side) is a hard GNU parse
;;      error that aborts the whole run; here the LHS charset
;;      (`verilog-auto--template-rule-head-re', below) structurally
;;      never contains `"' at all, so such a line simply fails to match
;;      any rule head and falls through to this file's EXISTING
;;      warn-and-skip path (`verilog-auto--template-parse-warnings',
;;      M92) -- the rule is dropped, a notice names the unrecognized
;;      remainder, and every OTHER rule in the same template still
;;      applies. No new code was needed for this divergence: it falls
;;      directly out of the LHS charset already excluding `"'.
;;   4. GNU's two quote-escaping conventions (`\"..\"' for a plain rule,
;;      `\\"..\\"' when the SAME rule is also a regexp/wildcard
;;      template) are each silently FATAL in the other's context.
;;      reticle honors the same two conventions (an exact rule's EXPR
;;      reaches the lisp pass with no prior transformation; a wildcard
;;      rule's EXPR has already been through `replace-regexp-in-string'
;;      as a replacement string, which is what makes the two escaping
;;      conventions resolve differently by the time the lisp pass sees
;;      them), but a token whose resulting source cannot be READ
;;      (`verilog-auto--scan-lisp-token-close' finds no matching
;;      unescaped closing quote, or the unescaped text doesn't parse as
;;      a complete form) produces the SAME divergence-1 fallback plus a
;;      notice quoting the unreadable source -- never an aborted run.
;;   5. evaluation is unsandboxed and unbounded, UNCHANGED from GNU --
;;      deliberately NOT a divergence. An AUTO_TEMPLATE is source code
;;      the user already chose to open and run `verilog-auto' on, so
;;      letting its own `@"(...)"' clause run arbitrary elisp grants no
;;      capability opening the file didn't already grant. `ignore-
;;      errors'/`with-demoted-errors' are not implemented in this
;;      interpreter; `condition-case' (this file's own established
;;      catch-and-report idiom, `crates/core/lisp/ielm.el') is used
;;      instead.
;;
;; A non-string/non-number/non-nil `eval' result (a symbol, a list, ...)
;; is treated as a FAILURE (divergence 1), not as "splice the empty
;; string" or "splice its printed form" -- see
;; `verilog-auto--lisp-eval-result-to-text's own doc string for why.
;;
;; Fix-round finding, recorded rather than "fixed": Part A can manufacture
;; a NEW instance of Part B's own known-unreachable shape through ordinary
;; SUCCESS, not failure. A lisp expression that evaluates to a string
;; containing a literal `"' immediately adjacent to a `[]'/`[][]' token in
;; the SAME rule splices generated text shaped exactly like `.data
;; (\"W\"[7:0])' -- the shape that makes tree-sitter's GLR recovery
;; reclassify the whole statement, so no `module_instantiation' node
;; exists for `verilog-delete-auto' to ever find (see this file's own
;; top-of-file header correction, and `verilog-auto--unreachable-
;; autoinst-markers'). Nothing here validates or rejects such a result --
;; deliberately: GNU places no such restriction on what an AUTO_TEMPLATE
;; expression may return, and inventing one here would be adding a
;; capability GNU doesn't have, not fixing a defect. The connection is
;; genuinely generated, genuinely correct Verilog text; it is Part B's
;; unreachable-site accounting, not Part A, that is what catches the site
;; being permanently stuck afterward -- pinned by
;; `lisp_result_embedding_a_quote_adjacent_to_bracket_becomes_a_part_b_unreachable_site'
;; in verilog_auto_tests.rs.
;;
;; Also recorded rather than changed (fix-round finding): the nine `vl-*'
;; symbols become globally special (`defvar'-marked, per this
;; interpreter's own `crates/elisp/src/eval.rs' -- `special = true' is set
;; unconditionally, before any value is even supplied) the MOMENT this
;; file loads, and this file loads UNCONDITIONALLY at startup
;; (`crates/core/src/lib.rs'), unlike GNU's own `verilog-mode.el', which
;; is typically autoloaded and so only pays this cost for a buffer that
;; actually visits Verilog. The practical effect: ANY user code anywhere
;; in a running session that does `(let ((vl-name ...)) ...)' for some
;; entirely unrelated purpose now gets a DYNAMIC binding for the vl-name
;; symbol, for the whole session, whether or not that code has anything
;; to do with Verilog -- a wider blast radius than GNU's own equivalent
;; ever has, purely because of when this file is loaded, not because of
;; anything about the `defvar's themselves. This is NOT treated as a
;; defect: the names match GNU's own convention (a lisp expression
;; written against real `verilog-mode' AUTO_TEMPLATE documentation
;; should behave the same here), and section 3.2's own binding design is
;; unavoidable in this interpreter (see `vl-name's own doc string) --
;; recorded here honestly rather than silently narrower than it is.

(defvar verilog-auto--template-rule-head-re
  "^\\.\\(\\(?:[-A-Za-z0-9_$+@^.*?|]\\|\\[\\|\\]\\|\\\\[()|0-9]\\)+\\)[ \t]*("
  "Matches a template rule's own `.NAME-OR-PATTERN  (' head -- up
through, but NOT past, the opening paren that starts EXPR. The captured
group's own charset is GNU's wildcard LHS charset (identifier chars,
`+@^.*?|[]', and backslash-escaped `(', `)', `|', or a digit) -- a
strict SUPERSET of a bare identifier, so this one regex serves both
shapes; `verilog-auto--template-rule-at' (below) decides exact vs.
wildcard AFTERWARD by testing whether the captured text is nothing but
`[A-Za-z0-9_$]+'.

M92 fix round X1: this regex used to also try to capture EXPR itself
via a trailing `(\\(.*\\))[ \t]*[,;)]*\\(?:[ \t]*//.*\\)?$' anchored at
end-of-line. `.*' is greedy and nothing in it excludes `)', so a
trailing `// comment (with a paren)' made the match backtrack all the
way to THAT paren as if it were EXPR's own close -- `.done (finished),
// see (note)' silently captured EXPR as `\"finished), // see (note\"',
a syntactically broken connection with no warning at all (the line
still matched, so `verilog-auto--parse-template-body's `t' branch below
was never reached either). Fixed by never regex-capturing EXPR: this
regex only finds the HEAD, and `verilog-auto--template-rule-at' finds
EXPR's own close paren by depth-count balance-scanning instead (the
same technique `verilog-auto--template-body-text' already uses for the
AUTO_TEMPLATE block's own outer parens) -- a `)' inside a comment, or
belonging to a SECOND rule on the same line, can then never be mistaken
for EXPR's own boundary, and `.a (foo(bar))' -- a legitimate EXPR that
itself contains balanced parens -- parses correctly too, which a
non-greedy `.*?' fix would have gotten equally wrong (first-match is as
incorrect as last-match here).")

(defun verilog-auto--balanced-paren-end (s open)
  "Position in S of the `)' that balance-matches the `(' at position
OPEN (S's own char index), depth-counting nested parens along the way,
or nil if S runs out first. Deliberately scoped to ONE line's own text
(a caller-provided single physical line, or the trimmed remainder of
one after an earlier rule was already parsed off its front) -- EXPR is
never expected to span multiple lines, matching this file's existing
per-line template convention."
  (let ((depth 1) (i (1+ open)) (n (length s)) (close nil))
    (while (and (< i n) (not close))
      (cond
       ((eq (aref s i) ?\() (setq depth (1+ depth)))
       ((eq (aref s i) ?\))
        (setq depth (1- depth))
        (when (= depth 0) (setq close i))))
      (setq i (1+ i)))
    close))

(defun verilog-auto--template-rule-at (s)
  "If S (a string with no leading whitespace) begins with a well-formed
`.NAME-OR-PATTERN (EXPR)' rule, return (NAME-OR-PATTERN EXPR REST) --
REST the TRIMMED remainder of S after this one rule's own trailing
separator punctuation (`,'/`;'/`)', zero or more, matching this file's
existing leniency) and an optional `// ...' end-of-line comment; REST
may be empty. Returns nil if S doesn't even match
`verilog-auto--template-rule-head-re' at all (caller treats the whole
of S as a malformed/unrecognized rule in that case) -- note a
successfully parsed head with an UNBALANCED paren (no matching close
anywhere in S, `verilog-auto--balanced-paren-end' returns nil) also
returns nil here, same treatment.

M92 fix round X1 also resolves, as a side effect of parsing this way
instead of one whole-line regex: `.a (x), .b (y)' on a single line used
to match as ONE rule with EXPR captured as `\"x), .b (y\"' (the same
greedy-`)' defect as the trailing-comment case above). Chosen behavior
here -- since REST is handed back to the caller
(`verilog-auto--parse-template-body'), which loops calling this
function again on REST until it's exhausted or a call fails -- is to
parse BOTH rules correctly: `.a (x)' first, EXPR verbatim `\"x\"', REST
`\".b (y)\"'; then `.b (y)' on the next iteration. A trailing chunk that
ISN'T a valid second rule falls through to the caller's own warning
path instead, same as a whole malformed line would."
  (if (not (string-match verilog-auto--template-rule-head-re s))
      nil
    (let* ((name (match-string 1 s))
           (open (1- (match-end 0)))
           (close (verilog-auto--balanced-paren-end s open)))
      (if (not close)
          nil
        (let* ((expr (substring s (1+ open) close))
               (tail (substring s (1+ close)))
               (i 0) (n (length tail)))
          (while (and (< i n) (memq (aref tail i) '(?\s ?\t))) (setq i (1+ i)))
          (while (and (< i n) (memq (aref tail i) '(?, ?\; ?\)))) (setq i (1+ i)))
          (while (and (< i n) (memq (aref tail i) '(?\s ?\t))) (setq i (1+ i)))
          (when (and (< (1+ i) n) (eq (aref tail i) ?/) (eq (aref tail (1+ i)) ?/))
            (setq i n))
          (list name expr (string-trim (substring tail i))))))))

(defun verilog-auto--literal-replacement (text)
  "TEXT with every backslash doubled, so passing the result as REP to
`replace-regexp-in-string' places TEXT into the output VERBATIM rather
than having its own backslashes interpreted as `\\\\&'/`\\\\N'/`\\\\\\\\'
replacement-template escapes (this project's own `replace-regexp-in-
string', `crates/elisp/src/builtins/misc.rs', takes exactly 3
arguments -- there is no FIXEDCASE/LITERAL/SUBEXP/START to ask for this
directly, unlike real Emacs; `crates/elisp/src/regex.rs's own
`replace_all' is the same primitive this file's top-of-file AUTO_
TEMPLATE header already documents as a known gap for a DIFFERENT
reason -- a lone backslash the CALLER never intended as a template
escape gets silently eaten). Every M127 caller that substitutes
arbitrary text (an instance number, a port's own range text, `@'s
compiled capture group) into a template replacement wraps it with this
first."
  (let ((n (length text)) (i 0) (out ""))
    (while (< i n)
      (let ((c (aref text i)))
        (setq out (concat out (if (eq c ?\\) "\\\\" (char-to-string c)))))
      (setq i (1+ i)))
    out))

(defun verilog-auto--compile-lhs-at (name)
  "NAME (a wildcard template rule's own raw LHS text, before it gets
wrapped `^...$') with every literal `@' replaced by `\\\\([0-9]+\\\\)'
\(section 2.2/M127): GNU compiles an LHS `@' into a real capture group
that occupies an ORDINARY numbering slot POSITIONALLY, exactly like any
`\\\\(...\\\\)' group the rule's own author wrote by hand -- there is no
reserved slot for it (section 2.7 case 1: whichever group's own `\\\\('
appears FIRST in the compiled pattern text is `\\\\1', full stop, and a
plain textual substitution in place naturally preserves that). This
group's own captured value is never read back out anywhere -- the RHS
`@' is always the INSTANCE-derived number (`verilog-auto--instance-
number'), never a digit captured from the PORT name -- it exists only
so the LHS pattern can select the right ports at all (section 2.2's own
`.data_@ (in[@])' example: this group is what lets `data_0'/`data_1'
both match, and BOTH then take the instance's own `@', never `0'/`1')."
  (replace-regexp-in-string "@" (verilog-auto--literal-replacement "\\([0-9]+\\)") name))

(defun verilog-auto--parse-template-body (text)
  "TEXT is the substring between an AUTO_TEMPLATE block's own outermost
parens (see `verilog-auto--template-body-text'). Returns (EXACT . WILD):
EXACT an alist of (NAME . EXPR), most-recently-defined-in-TEXT first (so
a plain `assoc' lookup naturally prefers the LATEST rule for a NAME that
appears more than once, matching GNU's own `assoc'-into-a-forward-consed
list behavior); WILD a list of (ANCHORED-PATTERN . EXPR) pairs in
DOCUMENT (file) order -- callers must scan front to back and stop at the
FIRST pattern that matches (GNU's own `verilog-auto-inst-port' loops
every wildcard without an early exit and keeps overwriting its result,
which -- because ITS OWN list is built in reverse-of-file order -- nets
out to \"the earliest-written-in-the-template matching rule wins\"; this
function's list is built the other way around, so simple front-to-back
`first match wins' scanning reproduces the identical result without
replaying GNU's own reversed-overwrite trick).

Lines that are blank, or start with `//' once trimmed, are skipped (GNU
tolerates the same). Every OTHER line is handed to
`verilog-auto--template-rule-at' in a loop: on success, the returned
rule is filed into EXACT or WILD (an all-identifier-char NAME is exact,
anything else is wrapped `^...$' and filed as wildcard) and the loop
continues on that call's own REST -- so `.a (x), .b (y)' parses as TWO
rules (see `verilog-auto--template-rule-at's own doc string for why
this choice, not \"report the remainder as malformed,\" was made). The
FIRST call that fails on a given line's remaining text -- REST doesn't
even start with a recognizable `.HEAD (' shape, or its own paren never
balances -- pushes THAT REMAINING TEXT (not necessarily the whole
original line, if an earlier rule on the same line already parsed
successfully) onto `verilog-auto--template-parse-warnings', folded into
`verilog-auto''s own end-of-command message (the same \"can't show two
things in one echo line\" pattern as every other notice this file
collects) -- M92 fix round S1: this used to fall through a `cond'
silently (no warning path at all), which is exactly how a perfectly
well-formed rule with its own trailing `// comment' used to vanish
before X1/S1 together made this whole parse tolerant of one. A rule can
still be malformed for other reasons (stray punctuation, a paren that
never balances on this line -- GNU's own point-based scanner would
`error' outright on those, `verilog-read-auto-template-middle's `(t
(error ...))' branch, `verilog-mode.el' :10184-10267), so this
project's posture stays warn-and-skip rather than GNU's hard `error',
consistent with the rest of this file (a missing module, an ANSI
AUTOARG, a second AUTOWIRE per module -- none of those abort
`verilog-auto' either).

A `/* */' block comment EMBEDDED inside the AUTO_TEMPLATE parens is
still not tolerated, but not for the reason an earlier draft of this
docstring claimed (\"unhandled/undefined, not silently wrong-but-
plausible\") -- that was inaccurate. Confirmed with a real parse (M92
fix round S3): this grammar's `block_comment' node ends at the FIRST
`*/' it finds, full stop -- block comments do not nest, so `/* Mod
AUTO_TEMPLATE ( /* inner */ .foo (bar), ); */' parses as ONE
block_comment node whose own text is only `\"/* Mod AUTO_TEMPLATE ( /*
inner */\"' -- everything from `.foo' onward, including the template's
own real rules and the closing `); */', is NOT part of any comment node
at all. `verilog-auto--template-body-text' then finds `AUTO_TEMPLATE'
and its opening `(' inside that TRUNCATED text, but the paren-depth
scan never finds a matching close (there isn't one left in the
truncated string), so it returns nil -- and THIS function is never even
called in that case (`verilog-auto--find-template' short-circuits on a
nil body before reaching here). So the actual failure mode is \"the
template silently vanishes,\" never \"a commented-out rule gets
silently treated as live\" -- there IS no comment-swallowing to speak
of, because the grammar's own non-nesting `*/' rule means the surviving
fragment can't even see the rules that would have to be swallowed.

M92 fix round X2: because this function is the ONLY place that ever
pushes onto `verilog-auto--template-parse-warnings', the nested-`/* */'
case used to bypass the warning channel entirely -- indistinguishable
from \"no AUTO_TEMPLATE comment exists for this module at all,\" with no
notice of any kind. `verilog-auto--find-template' now pushes its own
warning when it finds a MATCHING AUTO_TEMPLATE comment whose body
extraction still fails, so the failure is no longer silent even though
the underlying truncation itself is still not fixed (see that
function's own doc string for why: fixing it would mean scanning raw
buffer text PAST a node's own boundary to find the REAL `*/', exactly
the search-by-node-KIND discipline this file's own top-of-file header
commits to never doing)."
  (let (exact wild)
    (dolist (raw (split-string text "\n"))
      (let ((line (string-trim raw)))
        (unless (or (string-empty-p line) (string-prefix-p "//" line))
          (let ((remaining line) (progress t))
            (while (and progress (> (length remaining) 0))
              (let ((parsed (verilog-auto--template-rule-at remaining)))
                (if parsed
                    (let ((name (nth 0 parsed)) (expr (nth 1 parsed)) (rest (nth 2 parsed)))
                      (if (string-match-p "\\`[A-Za-z0-9_$]+\\'" name)
                          (push (cons name expr) exact)
                        (push (cons (concat "^" (verilog-auto--compile-lhs-at name) "$") expr) wild))
                      (setq remaining rest))
                  (progn
                    (push remaining verilog-auto--template-parse-warnings)
                    (setq remaining "" progress nil)))))))))
    (cons exact (nreverse wild))))

(defun verilog-auto--template-quoted-regexp (text start)
  "If TEXT has a quoted-string instance-number regexp (section 2.1)
sitting between AUTO_TEMPLATE's own keyword and its rule list's opening
paren, scanning forward from START (a position right after the
keyword, skipping only whitespace before testing for a `\"'), return
\(REGEXP . AFTER-POS) -- REGEXP the text between the quotes, AFTER-POS
the position right past the closing quote. nil (not a cons) if no `\"'
sits there at all -- the ordinary, no-custom-regexp case. The symbol
`unterminated' (a third, distinct return shape) if an opening `\"' is
found but no closing one exists anywhere in TEXT -- a malformed head,
which `verilog-auto--template-body-text' below treats with this file's
existing inert-and-warn posture, never a hard error."
  (let ((i start) (n (length text)))
    (while (and (< i n) (memq (aref text i) '(?\s ?\t ?\n ?\r))) (setq i (1+ i)))
    (if (and (< i n) (eq (aref text i) ?\"))
        (let ((close (string-match "\"" text (1+ i))))
          (if close (cons (substring text (1+ i) close) (1+ close)) 'unterminated))
      nil)))

(defun verilog-auto--template-body-text (comment)
  "The substring of COMMENT's own text (a block_comment, already
confirmed to contain \"AUTO_TEMPLATE\" by `verilog-auto--template-for-
module') between the `(' that opens its rule list and the matching,
paren-depth-balanced `)', or nil if that shape isn't found (a malformed
template is left as inert, unparsed text -- same fallback posture as
\"no template found\" below, never a hard error that would abort the
whole `verilog-auto' run over one bad comment). Returns (BODY-TEXT .
CUSTOM-REGEXP) -- CUSTOM-REGEXP non-nil only when a quoted-string
instance-number regexp (section 2.1) sits between the AUTO_TEMPLATE
keyword and the rule list's own opening paren.

M127 fix (section 3.1): this used to find the FIRST `(' anywhere after
the keyword, which for `/* submod AUTO_TEMPLATE \"_ch\\\\([0-9]+\\\\)$\"
( .clk (...), ); */' lands on the `\\\\(' INSIDE the quoted regexp, not
the rule list's own opening paren -- the whole template then misparsed
\(the depth-balance scan from that wrong `(' never finds a real match,
or matches the wrong span), and every rule silently fell back to an
identity connection with no trace beyond a garbled parse-warning naming
a fragment of the regexp itself. Fixed by first testing for, and
skipping over, an optional quoted string right after the keyword
\(`verilog-auto--template-quoted-regexp') before ever searching for `('
at all."
  (let* ((text (treesit-node-text comment))
         (kw (string-match "AUTO_TEMPLATE" text)))
    (when kw
      (let* ((after-kw (+ kw (length "AUTO_TEMPLATE")))
             (q (verilog-auto--template-quoted-regexp text after-kw)))
        (unless (eq q 'unterminated)
          (let* ((custom (car q))
                 (scan-from (if q (cdr q) after-kw))
                 (open (string-match "(" text scan-from)))
            (when open
              (let ((depth 1) (i (1+ open)) (n (length text)) (close nil))
                (while (and (< i n) (not close))
                  (cond
                   ((eq (aref text i) ?\() (setq depth (1+ depth)))
                   ((eq (aref text i) ?\))
                    (setq depth (1- depth))
                    (when (= depth 0) (setq close i))))
                  (setq i (1+ i)))
                (and close (cons (substring text (1+ open) close) custom))))))))))

(defun verilog-auto--template-for-module (template-comments type-name inst-start)
  "The AUTO_TEMPLATE block_comment for module TYPE-NAME nearest to
INST-START: the one with the LARGEST start position at or before
INST-START (\"nearest preceding\"), else -- if none precedes it -- the
one with the SMALLEST start position after it (\"nearest following\";
GNU's own comment at :10286-10292 calls this fallback historical and not
really spec'd, but keeps it, so this does too). Returns (COMMENT .
FALLBACK-P) -- FALLBACK-P non-nil when the FOLLOWING fallback (rather
than a preceding comment) is what got returned (M127 divergence 5: this
is the shape that lets a template written after instance A but before
instance B be silently stolen by B; `verilog-auto--find-template' below
records a positioned notice whenever FALLBACK-P is non-nil, so the
silent misattribution becomes visible). nil (not a cons) if no comment in
TEMPLATE-COMMENTS (every `block_comment' node in the buffer -- see
`verilog-auto--expand-all-autoinst', which gathers this ONCE per
`verilog-auto' pass and hands the same list to every site, M92 fix
round S4: this function used to re-walk the WHOLE tree via
`verilog-auto--find-all-of-type' on every single call, i.e. once per
/*AUTOINST*/ site, an O(sites * tree-size) cost a file with several
sites paid for no reason -- AUTOINST did no comment-wide scan at all
before AUTO_TEMPLATE existed) matches TYPE-NAME's own header shape (`/*
TYPE-NAME AUTO_TEMPLATE ...'; GNU's own regex additionally tolerates
leading whitespace and an OMITTED `/*' -- both dropped here since every
v1 test fixture's own template comment is a real `block_comment' node,
which by definition always starts with a real `/*')."
  (let* ((pat (concat "\\`/\\*[ \t\n\r]*" (regexp-quote type-name) "[ \t\n\r]+AUTO_TEMPLATE"))
         (matching (verilog-auto--filter
                    (lambda (n) (string-match-p pat (treesit-node-text n)))
                    template-comments))
         before before-pos after after-pos)
    (dolist (c matching)
      (let ((s (treesit-node-start c)))
        (if (<= s inst-start)
            (when (or (null before-pos) (> s before-pos))
              (setq before c before-pos s))
          (when (or (null after-pos) (< s after-pos))
            (setq after c after-pos s)))))
    (cond (before (cons before nil))
          (after (cons after t))
          (t nil))))

(defun verilog-auto--find-template (template-comments type-name inst-start)
  "Parsed (EXACT WILD CUSTOM-REGEXP) template
\(`verilog-auto--parse-template-body' for EXACT/WILD, CUSTOM-REGEXP from
`verilog-auto--template-body-text') for an instantiation of TYPE-NAME
starting at INST-START, or nil if no matching AUTO_TEMPLATE comment
exists, or its own body can't be extracted (`verilog-auto--template-
body-text' failure -- see its own doc string, in particular the
nested-`/* */'-truncates-the-comment case). TEMPLATE-COMMENTS as in
`verilog-auto--template-for-module', which this simply forwards to.

M127 divergence 5: when `verilog-auto--template-for-module' resolved
this lookup via its own FORWARD fallback (no preceding AUTO_TEMPLATE;
the nearest FOLLOWING one used instead), a positioned notice is pushed
onto `verilog-auto--template-forward-fallback-notices' naming TYPE-NAME
-- GNU is silent here, and a template written after instance A but
before instance B can be silently stolen by B with no trace at all.

M92 fix round X2: when a MATCHING AUTO_TEMPLATE comment is found but its
own body can't be extracted, that used to return nil with no trace at
all -- indistinguishable from \"no AUTO_TEMPLATE comment exists for this
module,\" and silently bypassing the ONLY place
(`verilog-auto--parse-template-body') that ever pushes onto
`verilog-auto--template-parse-warnings', since that function is never
even reached in this case. A warning is now pushed HERE instead, naming
TYPE-NAME, so this failure surfaces in `verilog-auto''s own final
message same as every other malformed-template case -- the underlying
truncation itself is still not fixed (see
`verilog-auto--template-body-text's own doc string for why: it would
mean scanning raw buffer text past a node's own boundary, which this
file's own top-of-file header commits to never doing), only no longer
silent."
  (let* ((found (verilog-auto--template-for-module template-comments type-name inst-start))
         (comment (car found)))
    (when comment
      (when (cdr found)
        (push (cons inst-start
                    (format "AUTO_TEMPLATE for module %s resolved by the forward fallback (no preceding AUTO_TEMPLATE for this instantiation; the nearest FOLLOWING one was used instead, which a template meant for a LATER instance can silently steal)" type-name))
              verilog-auto--template-forward-fallback-notices))
      (let ((body (verilog-auto--template-body-text comment)))
        (if body
            (let ((parsed (verilog-auto--parse-template-body (car body))))
              (list (car parsed) (cdr parsed) (cdr body)))
          (push (format "AUTO_TEMPLATE for module %s: comment found but its own body could not be extracted (often an embedded /* */ inside the block, which ends the comment early)" type-name)
                verilog-auto--template-parse-warnings)
          nil)))))

(defun verilog-auto--template-lookup (template port-name)
  "(EXPR . RULE-ID) for PORT-NAME per TEMPLATE (`verilog-auto--find-
template's return value, a (EXACT WILD CUSTOM-REGEXP) list) -- EXPR nil
if no rule applies to PORT-NAME at all, in which case
`verilog-auto--connection-text' falls back to today's identity
connection; RULE-ID nil exactly when EXPR is (M127, needed for the
`// Templated' annotation and for `verilog-auto-inst-template-numbers'
='lhs', section 2.5): PORT-NAME itself (bare, no anchors -- section 2.7
case 2) for an EXACT match, or the wildcard's own compiled, `^...$'-
anchored pattern text for a WILDCARD match -- these are deliberately
DIFFERENT kinds of text, GNU's own real asymmetry, not something to
unify.

Exact match wins outright over any wildcard (see this section's own
header); a wildcard's EXPR has its own `\\N' substituted via
`replace-regexp-in-string' against PORT-NAME itself -- since the
wildcard's own stored pattern is `^...$'-anchored (see
`verilog-auto--parse-template-body'), that single call matches the
WHOLE of PORT-NAME exactly once, so its output IS the fully-substituted
EXPR, not a partial in-place replacement. Note this does NOT yet apply
`@'/`[][]'/`[]' substitution (section 2.3/3.4) -- that is a separate
step, `verilog-auto--template-substitute', applied by the caller
afterward; this function's own job is purely resolving WHICH rule
applies and its `\\N'-substituted text."
  (let ((exact (assoc port-name (nth 0 template))))
    (if exact
        (cons (cdr exact) port-name)
      (let ((wild (nth 1 template)) (result nil) (rule-id nil))
        (while (and wild (not result))
          (let ((lhs (caar wild)) (expr (cdar wild)))
            (when (string-match-p lhs port-name)
              (setq result (replace-regexp-in-string lhs expr port-name) rule-id lhs)))
          (setq wild (cdr wild)))
        (cons result rule-id)))))

(defun verilog-auto--instance-name-of (hier)
  "HIER's (a `hierarchical_instance') own instance name -- specifically
`name_of_instance''s `instance_name' FIELD text, never the whole
`name_of_instance' node's own text (M127 section 3.2): a
`name_of_instance' node is `field(\"instance_name\", identifier)
repeat(unpacked_dimension)' -- an ARRAY instance (`u_bank[3:0]') has a
SECOND, sibling `unpacked_dimension' child carrying the array range, so
reading the whole node's text would read `@' numbering out of the array
bound instead of the instance name proper. `u_bank[3:0]' and
`u_scalar' must both read as their own bare name here -- pinned by
`autoinst_template_at_instance_array_uses_name_not_array_range' in
verilog_auto_tests.rs (this milestone's own regression pin for the M127
spec section 0 finding that instance arrays need no special handling
anywhere else in this file)."
  (let ((noi (verilog-auto--find-first-of-type hier "name_of_instance")))
    (and noi (treesit-node-text (treesit-node-child-by-field-name noi "instance_name")))))

(defun verilog-auto--instance-number (inst-name custom-regexp)
  "The `@' substitution value for INST-NAME (section 2.1), plus any
notice this call needs recorded. Returns (VALUE KIND . TEXT):
- VALUE is what `@' actually expands to on the RHS -- always a string,
  possibly empty.
- KIND is nil (no notice), `no-match' (M127 divergence 3: no digits in
  INST-NAME under the default rule, or CUSTOM-REGEXP simply doesn't
  match it -- both cases GNU is silent about, VALUE the empty string),
  or `no-capture-group' (M127 divergence 4: CUSTOM-REGEXP matches but
  group 1 has nothing captured -- `(match-beginning 1)' nil, which
  covers TWO distinct shapes this symbol's own name doesn't
  distinguish and the notice text must not conflate: CUSTOM-REGEXP has
  no `\\\\(...\\\\)' group AT ALL, or it has one that is itself
  OPTIONAL (e.g. `\\\\(foo\\\\)?bar') and simply didn't participate in
  THIS match. GNU CRASHES THE WHOLE `verilog-auto' CALL in the first
  shape (`Wrong type argument: stringp, nil' out of `replace-match',
  writing no output at all); the second shape is not even documented
  in GNU's own source, since a template author writing an optional
  group is unusual but not malformed. This function returns an empty
  VALUE and never signals either way -- fix round: the notice text used
  to say \"has no capture group\" unconditionally, which is simply
  false for the second shape; reworded to name both.).
- TEXT is the notice's own message text when KIND is non-nil, else nil.

With CUSTOM-REGEXP nil (the default rule), VALUE is the FIRST run of
digits in INST-NAME, scanning LEFT TO RIGHT -- not the trailing run
\(measured: `u2_ch3' -> `2', not `3'; `u_1_2' -> `1', not `2').

With CUSTOM-REGEXP non-nil, VALUE is capture group 1 of CUSTOM-REGEXP
matched against INST-NAME -- ALWAYS group 1, even when CUSTOM-REGEXP
has two or more groups (section 2.7 case 3, GNU parity, measured:
`\"\\\\(u\\\\)_ch\\\\([0-9]+\\\\)\"' against `u_ch7' yields `@' = `u', not
`7', deterministically and with NO notice -- `@' substituting arbitrary
captured text is legitimate use, not a mistake to flag)."
  (if custom-regexp
      (if (string-match custom-regexp inst-name)
          (if (match-beginning 1)
              (list (match-string 1 inst-name) nil nil)
            (list ""
                  'no-capture-group
                  (format "AUTO_TEMPLATE instance-number regexp %S has no capture group (or its group 1 did not participate in this match); `@' expands to the empty string here (GNU signals `Wrong type argument: stringp, nil' and aborts the whole verilog-auto call when there is truly no capture group at all)"
                          custom-regexp)))
        (list ""
              'no-match
              (format "instance %s: AUTO_TEMPLATE instance-number regexp %S did not match; `@' expands to the empty string"
                      inst-name custom-regexp)))
    (if (string-match "[0-9]+" inst-name)
        (list (match-string 0 inst-name) nil nil)
      (list ""
            'no-match
            (format "instance %s has no digits; `@' expands to the empty string" inst-name)))))

;; --- AUTOINST --------------------------------------------------------------

(defvar verilog-auto-inst-template-numbers nil
  "How `/*AUTOINST*/' annotates a templated connection (section 2.5):
nil (the default, GNU-identical) emits a bare `// Templated'; `'lhs'
emits `// Templated LHS: PATTERN-OR-NAME' -- the compiled, anchored
wildcard pattern for a WILDCARD rule, the bare port name (no anchors)
for an EXACT rule; these two differ, and reproducing that asymmetry
rather than unifying it is GNU's own real, measured behaviour (section
2.7 case 2), not something to \"fix\".

`t' is GNU's absolute-LINE-NUMBER form (`// Templated 7',
`verilog-auto-templated-rel', verilog-mode.el:14728) -- NOT implemented
here: it needs GNU's own two-pass resolution (emit a relative marker,
then rewrite it once every phase has finished shifting the buffer's
lines), which is disproportionate for a non-default debugging aid whose
own question -- \"which rule drove this line\" -- `'lhs' already
answers. Setting this to `t' behaves EXACTLY like nil (a bare
`// Templated') and additionally records a notice naming this variable
in `verilog-auto--template-numbers-t-notices' -- never silently
ignored.")

(defvar verilog-auto--template-numbers-t-notices nil
  "M127 divergence 2: positioned notices (POSITION . TEXT) recorded
each time `verilog-auto-inst-template-numbers' is `t' and a templated
connection is annotated -- see that variable's own doc string.")

(defvar verilog-auto--template-instance-number-notices nil
  "M127 divergence 3: positioned notices (POSITION . TEXT) recorded
whenever `@' expands to the empty string -- no digits in the instance
name under the default rule, or a custom AUTO_TEMPLATE instance-number
regexp that simply doesn't match -- both cases GNU is silent about; see
`verilog-auto--instance-number'.")

(defvar verilog-auto--template-forward-fallback-notices nil
  "M127 divergence 5: positioned notices (POSITION . TEXT) recorded
whenever an AUTO_TEMPLATE lookup is resolved by
`verilog-auto--template-for-module's own FORWARD fallback (no preceding
comment; the nearest FOLLOWING one used instead) -- see
`verilog-auto--find-template'.")

(defvar verilog-auto--template-lisp-eval-failures nil
  "M128 divergence 1/2: positioned notices (POSITION . TEXT) recorded
whenever an `@\"(lisp-expr)\"' AUTO_TEMPLATE token could not be read or
signaled an error while being evaluated (section 2.5) -- the port falls
back to an identity connection and the annotation reads `// Templated
\(expression failed)' instead of a bare `// Templated'. See
`verilog-auto--template-eval-lisp-tokens'.")

(defvar vl-name nil
  "M128: the CONNECTED PORT's own declared name (section 2.3), bound
around the evaluation of an `@\"(lisp-expr)\"' AUTO_TEMPLATE token
\(`verilog-auto--template-eval-lisp-tokens'). Always the port's name as
DECLARED in the submodule, never the signal it happens to be connected
to on THIS instantiation (measured against a wildcard rule where the
two differ, section 2.3).

Why `defvar' at all, when this file is `lexical-binding: t' everywhere
else: `eval's second argument in this interpreter is a BOOLEAN, not an
environment (`crates/elisp/src/builtins/misc.rs' -- `let lexical =
opt(a, 1).truthy()'), so there is no way to hand `eval' a lexical
environment for the expression text read out of the template. Under
`lexical-binding: t', a plain `let' creates a LEXICAL binding, which is
invisible to code reached via `eval' (a fresh top-level read, with no
lexical scope of its own reaching back into this file). Verified by
execution on this checkout: a `defvar'd symbol `let'-bound here IS
visible to `(eval (car (read-from-string \"vl-name\")))'; a
non-`defvar'd one signals `void-variable'. `defvar' is what makes a
symbol SPECIAL (dynamically scoped) regardless of the file's own
`lexical-binding' setting -- that's the whole reason these nine
variables need it and ordinary local state in this file does not. Do
not try to pass `eval' an environment instead; there is nothing to pass
it to.")

(defvar vl-width nil
  "M128: the CONNECTED PORT's own bit width, as a STRING (section 2.3)
-- a plain scalar port is \"1\"; `[7:0]' is \"8\"; a symbolic range like
`[WIDTH-1:0]' is \"WIDTH\", the UNEVALUATED text (this file never
attempts to constant-fold a parameter expression here, matching this
section's own measured GNU behavior for exactly that one idiom -- see
`verilog-auto--vl-width-of'). Bound the same way as `vl-name'; see that
variable's own doc string for why `defvar' is required at all.")

(defvar vl-bits nil
  "M128: the CONNECTED PORT's own LAST (innermost) packed dimension,
bracketed text verbatim, or the empty string for a true scalar (section
2.3) -- identical in shape to `[]' (`verilog-auto--template-substitute's
own `last-packed'). Bound the same way as `vl-name'.")

(defvar vl-mbits nil
  "M128: the CONNECTED PORT's own packed dimensions EXCLUDING the last
\(innermost) one, concatenated, or the empty string when the port has
zero or one packed dimension (section 2.3 -- only a genuinely
multi-dimensional packed port like `[1:0][7:0]' produces a non-empty
value, `\"[1:0]\"' there). Bound the same way as `vl-name'.")

(defvar vl-memory nil
  "M128: the CONNECTED PORT's own unpacked (\"memory\") dimension text,
concatenated, or `nil' when the port has no unpacked dimension at all
\(section 2.3 -- an earlier reconnaissance round recorded only eight
`vl-*' variables; this is the real ninth, documented in GNU's own
source). `input [7:0] mem [0:3]' yields `\"[0:3]\"'. Bound the same way
as `vl-name'.")

(defvar vl-dir nil
  "M128: the CONNECTED PORT's own direction as a STRING -- `\"input\"',
`\"output\"', `\"inout\"', or `\"interface\"' for an interface-typed port
\(section 2.3, `(symbol-name DIRECTION)' of the same symbol
`verilog-auto--port-direction-of' already returns everywhere else in
this file). Bound the same way as `vl-name'.")

(defvar vl-modport nil
  "M128: the CONNECTED PORT's own modport name (section 2.3) for an
interface-typed port whose ANSI header carries one (`some_if.mst bus_i'
-> `\"mst\"'), or `nil' for any non-interface port, or an interface port
whose modport clause is itself omitted (`some_if bus_i', legal SV).
Bound the same way as `vl-name'."  )

(defvar vl-cell-name nil
  "M128: this AUTOINST site's own INSTANCE name (section 2.3) -- for an
array instance `u_bank[3:0] (...)' this is the bare `\"u_bank\"', the
`[3:0]' array range is NOT included (`verilog-auto--instance-name-of'
already reads exactly this shape for `@' numbering; `vl-cell-name'
reuses the same value, not a separate lookup). Bound the same way as
`vl-name'.")

(defvar vl-cell-type nil
  "M128: this AUTOINST site's own INSTANTIATED MODULE name (section 2.3)
-- e.g. `\"sram_bank\"' for `sram_bank u_bank (...)'. Bound the same way
as `vl-name'.")

(defun verilog-auto--vl-width-of (bits)
  "The `vl-width' STRING for BITS (`vl-bits'-shaped: a bracketed packed
dimension's own text, or the empty string for a scalar) -- section 2.3,
measured against GNU Emacs 30.2 for exactly the shapes this milestone's
own test list covers: a scalar (\"1\"), a purely numeric range
\(`verilog-auto--dimension-width', e.g. `[7:0]' -> \"8\"), and the
`[IDENT-1:0]' idiom (-> the bare IDENT text, unevaluated -- reusing the
SAME regexp `verilog-auto--symbolic-tieoff-body' already special-cases
for the tie-off constant, section 1.3's own precedent for this exact
pattern). Any OTHER symbolic shape (`[2*W-1:0]', an asymmetric symbolic
range with a non-zero LSB, etc.) is NOT covered by any GNU measurement
this milestone made -- this function falls back to a textual
`(1+(MSB)-(LSB))' form (LSB's own `-(LSB)' term omitted when LSB is
\"0\", mirroring `verilog-auto--symbolic-tieoff-body's own general form)
so `vl-width' is always some plausible STRING rather than nil, but this
fallback branch is a considered best-effort, not a measured fact --
documented here rather than silently presented as GNU parity."
  (if (string-empty-p bits)
      "1"
    (let* ((bounds (verilog-auto--range-bounds bits))
           (msb (car bounds)) (lsb (cdr bounds)))
      (cond
       ((and (verilog-auto--numeric-p msb) (verilog-auto--numeric-p lsb))
        (number-to-string (verilog-auto--dimension-width msb lsb)))
       ((and (string= lsb "0")
             (string-match "\\`\\([A-Za-z_$][A-Za-z0-9_$]*\\)[ \t]*-[ \t]*1\\'" msb))
        (match-string 1 msb))
       (t (format "(1+(%s)%s)" msb (if (string= lsb "0") "" (format "-(%s)" lsb))))))))

(defun verilog-auto--vl-bindings-of (name direction packed-dims unpacked-dims modport cell-name cell-type)
  "Alist of the nine `vl-*' symbols (section 2.3) to their STRING/nil
values for the CONNECTED PORT described by NAME/DIRECTION/PACKED-DIMS/
UNPACKED-DIMS/MODPORT (`verilog-auto--module-port-dims' shape, M128
4-tuple) and this AUTOINST site's own CELL-NAME/CELL-TYPE. Consulted by
`verilog-auto--template-eval-lisp-tokens' to `let'-bind the `defvar'd
symbols around each token's own `eval' -- see `vl-name's own doc string
for why `defvar' rather than a lexical environment."
  (let* ((bits (or (car (last packed-dims)) ""))
         ;; `butlast' is not implemented in this interpreter (v1) --
         ;; `(reverse (cdr (reverse packed-dims)))' is the same "every
         ;; element except the last" list, built from primitives this
         ;; file already relies on elsewhere.
         (mbits (if (> (length packed-dims) 1)
                    (string-join (reverse (cdr (reverse packed-dims))) "")
                  "")))
    (list (cons 'vl-name name)
          (cons 'vl-width (verilog-auto--vl-width-of bits))
          (cons 'vl-bits bits)
          (cons 'vl-mbits mbits)
          (cons 'vl-memory (and unpacked-dims (string-join unpacked-dims "")))
          (cons 'vl-dir (symbol-name direction))
          (cons 'vl-modport modport)
          (cons 'vl-cell-name cell-name)
          (cons 'vl-cell-type cell-type))))

(defun verilog-auto--scan-lisp-token-close (text start)
  "Position in TEXT of the unescaped closing `\"' that ends an
`@\"...\"' token whose content begins at START (right after the opening
`@\"'), honoring ordinary Lisp string-literal escaping -- a `\\' escapes
whatever character follows it (so `\\\"' never terminates the token,
and a lone trailing `\\' with nothing after it makes the token
unterminated). Returns nil if no unescaped `\"' is found before TEXT
runs out -- section 2.2's own \"a token whose source cannot be read\"
case (divergence 4), which the caller turns into a fallback rather than
an aborted run."
  (let ((i start) (n (length text)) (found nil))
    (while (and (< i n) (not found))
      (let ((c (aref text i)))
        (cond
         ((eq c ?\\) (setq i (+ i 2)))
         ((eq c ?\") (setq found i))
         (t (setq i (1+ i))))))
    found))

(defun verilog-auto--unescape-lisp-token-text (text)
  "TEXT (an `@\"...\"' token's own raw content, between the delimiters,
as found by `verilog-auto--scan-lisp-token-close') with every `\\X'
pair collapsed to bare `X' -- ordinary Lisp string-literal unescaping,
turning the ESCAPED source the user wrote (`\\\"' for a literal `\"',
section 2.2) back into the actual Lisp expression text to hand
`read-from-string'."
  (let ((i 0) (n (length text)) (out ""))
    (while (< i n)
      (let ((c (aref text i)))
        (if (and (eq c ?\\) (< (1+ i) n))
            (progn (setq out (concat out (char-to-string (aref text (1+ i)))))
                   (setq i (+ i 2)))
          (progn (setq out (concat out (char-to-string c)))
                 (setq i (1+ i))))))
    out))

(defun verilog-auto--lisp-eval-result-to-text (value)
  "The AUTO_TEMPLATE-splicing text for VALUE, an `@\"(lisp-expr)\"'
token's own `eval' result (section 2.4 result-conversion rule): a
string is used as-is; a number is converted with `number-to-string';
`nil' becomes the empty string (matching GNU's own measured behavior,
`@\"(nil)\"' reaching `concat' as an empty sequence). Anything else --
a symbol, a list, a cons that isn't a number/string/nil -- is NOT
converted at all: this function returns nil for that case, and the
caller (`verilog-auto--template-eval-lisp-tokens') treats a nil return
as a FAILURE (divergence 1 fallback), not as \"splice nothing\". This is
this milestone's own considered choice for the \"anything else\" case
the spec leaves open: splicing an arbitrary Lisp object's PRINTED
representation into generated Verilog source is far more likely to
produce silently-wrong, plausible-looking connection text than a loud,
visible failure is to lose real work -- and a failure here is exactly
as recoverable as any other (divergence 1: the rest of the file still
expands, the notice quotes what came back)."
  (cond
   ((stringp value) value)
   ((numberp value) (number-to-string value))
   ((null value) "")
   (t nil)))

(defun verilog-auto--template-eval-lisp-tokens (expr inst-number vl-bindings)
  "EXPR with every `@\"(lisp-expr)\"' token (section 2.1) replaced by its
own evaluated result, run BEFORE `@'/`[][]'/`[]' substitution
\(`verilog-auto--template-substitute' -- GNU's own documented pipeline
order, section 2). Several tokens in one EXPR are each evaluated in
turn, left to right; a token may sit embedded in surrounding text.

Per token: `@' is substituted into the token's own (unescaped) source
TEXT before it is read (section 2.4, a blind textual replace via
`verilog-auto--literal-replacement' so INST-NUMBER's own text can never
be misread as a regexp backreference -- this happens even INSIDE a
nested string literal the expression itself contains, matching the
measured `@\"(concat \\\"AT@HERE\\\")\"' -> `AT9HERE' shape). The
resulting text is read with `read-from-string' and evaluated with
`eval', the nine `vl-*' symbols (`verilog-auto--vl-bindings-of') bound
around the call via an ordinary dynamic `let' -- see `vl-name's own doc
string for why `defvar'+`let' rather than an environment argument.
`condition-case' catches any `error' `read-from-string' or `eval'
itself signals (including `end-of-file' from an incomplete read, which
is itself a condition of `error' -- verified by execution), matching
this codebase's own catch-and-report idiom (`crates/core/lisp/ielm.el').
`condition-case' is used rather than `ignore-errors'/`with-demoted-
errors' because neither of those is implemented in this interpreter.

Returns (TEXT . FAILURE): FAILURE nil and TEXT the fully-substituted
result on success; FAILURE a human-readable string (naming the
unreadable/failing token) and TEXT nil the MOMENT any single token
fails -- per section 2.5 divergence 1, one bad token fails the WHOLE
EXPR (the caller then falls back this PORT's own connection to an
identity connection; every OTHER port and every other AUTO in the file
is unaffected), not just that one occurrence. The result is
DELIBERATELY not re-scanned here for a further `@\"...\"' -- section 2.4
records this as reticle's own single-pass choice, GNU's own docstring
describing a single sequential pass and its own multi-token-per-
expression rescan behavior not being determinable by execution."
  (let ((i 0) (n (length expr)) (out "") (failure nil))
    (while (and (< i n) (not failure))
      (let ((at (string-match "@\"" expr i)))
        (if (not at)
            (progn (setq out (concat out (substring expr i))) (setq i n))
          (progn
            (setq out (concat out (substring expr i at)))
            (let* ((content-start (+ at 2))
                   (close (verilog-auto--scan-lisp-token-close expr content-start)))
              (if (not close)
                  (setq failure (format "AUTO_TEMPLATE lisp expression %S: unterminated (no matching closing quote found)"
                                         (substring expr at n)))
                (let* ((raw (substring expr content-start close))
                       (unescaped (verilog-auto--unescape-lisp-token-text raw))
                       (with-at (replace-regexp-in-string
                                 "@" (verilog-auto--literal-replacement inst-number) unescaped)))
                  (condition-case err
                      (let* ((parsed (read-from-string with-at))
                             (form (car parsed))
                             (vl-name (cdr (assq 'vl-name vl-bindings)))
                             (vl-width (cdr (assq 'vl-width vl-bindings)))
                             (vl-bits (cdr (assq 'vl-bits vl-bindings)))
                             (vl-mbits (cdr (assq 'vl-mbits vl-bindings)))
                             (vl-memory (cdr (assq 'vl-memory vl-bindings)))
                             (vl-dir (cdr (assq 'vl-dir vl-bindings)))
                             (vl-modport (cdr (assq 'vl-modport vl-bindings)))
                             (vl-cell-name (cdr (assq 'vl-cell-name vl-bindings)))
                             (vl-cell-type (cdr (assq 'vl-cell-type vl-bindings)))
                             (value (eval form))
                             (text (verilog-auto--lisp-eval-result-to-text value)))
                        (if text
                            (setq out (concat out text) i (1+ close))
                          (setq failure (format "AUTO_TEMPLATE lisp expression %S evaluated to %S, which is not a string/number/nil"
                                                 with-at value))))
                    (error
                     (setq failure (format "AUTO_TEMPLATE lisp expression %S signaled %S"
                                            with-at err)))))))))))
    (cons (and (not failure) out) failure)))

(defun verilog-auto--template-substitute (expr inst-number packed-dims unpacked-dims vl-bindings)
  "EXPR (a template rule's own RHS text, `\\N' backrefs already
resolved against the matched port name by `verilog-auto--template-
lookup') with `@'/`[][]'/`[]' substituted, IN THAT ORDER (section 2.3 --
`[][]' must be substituted before `[]' or it would be eaten as two
empty `[]'s). INST-NUMBER is the RHS `@' value (section 2.2 -- always
the INSTANCE-derived number, `verilog-auto--instance-number', never
anything captured on the LHS -- see `verilog-auto--compile-lhs-at').
PACKED-DIMS/UNPACKED-DIMS are the CONNECTED PORT's own dimension-text
lists (`verilog-auto--module-port-dims', filled by
`verilog-auto--module-ports').

`[]' is PACKED-DIMS' own LAST entry, verbatim, or the empty string when
PACKED-DIMS is nil (section 2.3: the port's own INNERMOST/LAST packed
dimension -- `verilog-auto--range-text-of' returns the FIRST instead,
which differs for a 2-D port like `[1:0][7:0]'; a port declared
explicitly `[0:0]' is NOT collapsed to empty here, because its own
PACKED-DIMS is a one-element list `(\"[0:0]\")', not nil -- only a TRUE
scalar with no packed dimension at all produces nil).

`[][]' behaves exactly like `[]' UNLESS the port is genuinely
multi-dimensional (two or more packed dimensions, or any unpacked
dimension) -- only then does it emit the port's FULL dimension spec,
packed dimensions concatenated then, if unpacked dimensions exist, a
`.' and the unpacked dimensions, the whole thing wrapped in a `/* */'
block comment (a hint for a human to complete by hand, deliberately not
usable Verilog): `/*[1:0][7:0]*/', `/*[7:0].[0:3]*/'.

VL-BINDINGS is `verilog-auto--vl-bindings-of's own alist, consulted only
by the M128 lisp-expression pass below.

M128: `@\"(lisp-expr)\"' evaluated tokens
\(`verilog-auto--template-eval-lisp-tokens') are substituted FIRST, BEFORE
`@'/`[][]'/`[]' below -- GNU's own documented `verilog-auto-inst'
pipeline order (section 2), and load-bearing for correctness: a lisp
token's own leading `@' (`@\"...\"') would otherwise be consumed by the
plain `@' substitution below before the token could ever be recognized
as one. Returns (TEXT . FAILURE) now, not a bare string: FAILURE is
non-nil (and TEXT nil) the moment the lisp pass reports a failure
\(section 2.5 divergence 1) -- the caller falls this PORT's own
connection back to an identity connection rather than aborting."
  (let* ((lisp-result (verilog-auto--template-eval-lisp-tokens expr inst-number vl-bindings))
         (lisp-text (car lisp-result))
         (lisp-failure (cdr lisp-result)))
    (if lisp-failure
        (cons nil lisp-failure)
      (let* ((with-at (replace-regexp-in-string "@" (verilog-auto--literal-replacement inst-number) lisp-text))
             (last-packed (or (car (last packed-dims)) ""))
             (multi-dim (or (> (length packed-dims) 1) unpacked-dims))
             (bracket2 (if multi-dim
                           (concat "/*" (string-join packed-dims "")
                                   (if unpacked-dims (concat "." (string-join unpacked-dims "")) "")
                                   "*/")
                         last-packed))
             (with-2 (replace-regexp-in-string
                      "\\[\\]\\[\\]" (verilog-auto--literal-replacement bracket2) with-at)))
        (cons (replace-regexp-in-string "\\[\\]" (verilog-auto--literal-replacement last-packed) with-2)
              nil)))))

(defun verilog-auto--templated-annotation (rule-id comment-start failed-p)
  "The `// Templated...' annotation text (section 2.4/2.5) for a
connection matched by RULE-ID (`verilog-auto--template-lookup's own
RULE-ID -- the bare port name for an exact rule, the compiled anchored
pattern for a wildcard), consulting `verilog-auto-inst-template-
numbers'. COMMENT-START (this AUTOINST site's own marker-comment
position) positions the divergence-2 notice when that variable is `t'.

FAILED-P (M128 divergence 2) is non-nil when RULE-ID's own EXPR
contained an `@\"(lisp-expr)\"' token that could not be evaluated
\(`verilog-auto--template-eval-lisp-tokens' reported a failure) -- the
connection itself fell back to an identity connection
\(`verilog-auto--connection-text'), but the annotation still names the
rule as templated, with ` (expression failed)' appended, so the failure
is visible in the FILE ITSELF and not only in the end-of-command echo
\(section 2.5's own point 2 -- a failed connection must never be
indistinguishable from a working one)."
  (let ((suffix (if failed-p " (expression failed)" "")))
    (cond
     ((eq verilog-auto-inst-template-numbers 'lhs) (concat "// Templated LHS: " rule-id suffix))
     ((eq verilog-auto-inst-template-numbers t)
      (let ((text "verilog-auto-inst-template-numbers is `t' (GNU's absolute-line-number form); this is not implemented here, `// Templated' was emitted instead"))
        (unless (verilog-auto--notice-contains-p verilog-auto--template-numbers-t-notices text)
          (push (cons comment-start text) verilog-auto--template-numbers-t-notices)))
      (concat "// Templated" suffix))
     (t (concat "// Templated" suffix)))))

(defun verilog-auto--connection-text (port overrides indent template inst-number dims-alist cell-name cell-type)
  "(TEXT RULE-ID FAILURE-TEXT) -- TEXT \".NAME  (EXPR)\" (INDENT NOT
included in the returned text -- only in the padding measurement, via
`verilog-auto--pad-to-column's OFFSET; the caller,
`verilog-auto--grouped-lines', prepends INDENT itself to every line
uniformly, connections and group headers alike). RULE-ID (M127) is nil
unless EXPR came from a template rule, in which case it is
`verilog-auto--template-lookup's own RULE-ID -- the caller
(`verilog-auto--inst-lines') uses this to drive the `// Templated'
annotation (section 2.4): an identity connection (no rule at all)
carries no annotation. FAILURE-TEXT (M128) is non-nil when RULE-ID is
non-nil AND the rule's own EXPR contained an `@\"(lisp-expr)\"' token
that failed to evaluate (section 2.5 divergence 1) -- TEXT then still
carries an IDENTITY connection (this function's own fallback branch,
same as \"no rule for this port\"), but RULE-ID stays non-nil so the
caller still emits a (failure-flavored) annotation, and FAILURE-TEXT
is the human-readable message the caller records into
`verilog-auto--template-lisp-eval-failures'.

TEMPLATE is this instantiation's own AUTO_TEMPLATE lookup structure
(`verilog-auto--find-template'), or nil if none applies. When TEMPLATE
has a rule for this port (`verilog-auto--template-lookup'), EXPR is that
rule's own text with `\\N', then `@\"(lisp-expr)\"', then `@'/`[][]'/`[]'
(`verilog-auto--template-substitute', INST-NUMBER/DIMS-ALIST feeding the
latter three, CELL-NAME/CELL-TYPE plus this port's own NAME/DIRECTION/
MODPORT feeding the M128 lisp pass via `verilog-auto--vl-bindings-of')
substituted -- OVERRIDES/param substitution is never applied to it (out
of scope, see this section's own header). Otherwise (no template, no
rule for this port, or a rule whose lisp expression failed) EXPR falls
back to today's identity behavior: NAME alone for a rangeless port,
else NAME with its (param-substituted) range appended, e.g.
\"count[WIDTH-1:0]\"."
  (let* ((name (nth 0 port))
         (direction (nth 1 port))
         (range (nth 2 port))
         (lookup (and template (verilog-auto--template-lookup template name)))
         (raw-expr (car lookup))
         (rule-id (and raw-expr (cdr lookup)))
         (dims (and raw-expr (assoc name dims-alist)))
         (packed (nth 1 dims)) (unpacked (nth 2 dims)) (modport (nth 3 dims))
         (subst-result (and raw-expr
                             (verilog-auto--template-substitute
                              raw-expr inst-number packed unpacked
                              (verilog-auto--vl-bindings-of
                               name direction packed unpacked modport cell-name cell-type))))
         (templated (car subst-result))
         (failure-text (and raw-expr (cdr subst-result)))
         (expr (or templated
                   (if range (concat name (verilog-auto--substitute-params range overrides)) name)))
         (dotname (concat "." name)))
    (list (concat (verilog-auto--pad-to-column dotname verilog-auto-inst-column (length indent))
                  "(" expr ")")
          rule-id
          failure-text)))

(defun verilog-auto--inst-lines (groups overrides indent template inst-number dims-alist comment-start cell-name cell-type)
  "(LINES COLUMN LAST-ANNOTATION) via `verilog-auto--grouped-lines'
(see that function's own doc string for the shape) -- FORMAT-FN builds
each connection's own text and records its (RULE-ID . FAILURE-TEXT)
into a per-call TEMPLATED-TABLE (name -> (RULE-ID . FAILURE-TEXT)) as a
side effect, ALSO pushing (COMMENT-START . FAILURE-TEXT) onto
`verilog-auto--template-lisp-eval-failures' (M128 divergence 1) when
FAILURE-TEXT is non-nil, so a failed `@\"(lisp-expr)\"' token is
recorded exactly once per port, the same \"record at the point the
failure is discovered\" shape every other M125/M127 notice list already
uses. ANNOTATE-FN, called right afterward for the SAME port
(`verilog-auto--grouped-lines' own contract), reads TEMPLATED-TABLE to
decide the `// Templated' annotation (`verilog-auto--templated-
annotation') -- nil for a port whose RULE-ID was nil, i.e. an identity
connection. CELL-NAME/CELL-TYPE (M128) are this instantiation's own
static `vl-cell-name'/`vl-cell-type' values, threaded down to
`verilog-auto--connection-text' unchanged for every port."
  (let ((templated-table (make-hash-table :test 'equal)))
    (verilog-auto--grouped-lines
     groups indent
     (lambda (p)
       (let ((r (verilog-auto--connection-text p overrides indent template inst-number dims-alist cell-name cell-type)))
         (puthash (nth 0 p) (cons (nth 1 r) (nth 2 r)) templated-table)
         (when (nth 2 r)
           (push (cons comment-start (nth 2 r)) verilog-auto--template-lisp-eval-failures))
         (nth 0 r)))
     (lambda (p)
       (let* ((entry (gethash (nth 0 p) templated-table))
              (rid (car entry)))
         (and rid (verilog-auto--templated-annotation rid comment-start (not (null (cdr entry))))))))))

(defun verilog-auto--expand-autoinst-site (template-comments module-instantiation comment)
  "Expand the /*AUTOINST*/ site marked by COMMENT (a descendant of
MODULE-INSTANTIATION). TEMPLATE-COMMENTS is every `block_comment' node
in the buffer, gathered ONCE by `verilog-auto--expand-all-autoinst' and
shared across every site (M92 fix round S4 -- see
`verilog-auto--template-for-module's own doc string for why), needed to
look up an AUTO_TEMPLATE for this instantiation's own module type
(`verilog-auto--find-template'). Already-explicit connections are left
alone and excluded from the generated set; if that leaves nothing to add
(every port already connected), nothing at all is inserted. Returns 1 if
the instantiated module was found (whether or not anything was actually
inserted), 0 if it couldn't be resolved (GNU warn-and-skip; see
`verilog-auto--module-ports').

M127: also computes this instantiation's own `@' number
(`verilog-auto--instance-number', section 2.1/2.2 -- `@' has meaning
ONLY inside an AUTO_TEMPLATE rule, so this is skipped entirely when
TEMPLATE is nil) and, for the LAST generated connection line ONLY,
inserts its `// Templated' annotation AFTER the instantiation's own
pre-existing closing paren(s)/`;' -- section 2.4's own measured
placement (\"after the `));' on the last line\"), which sits PAST this
function's own insertion point (the closing paren is untouched,
pre-existing buffer text). `verilog-delete-auto' is widened
correspondingly (`verilog-auto--trailing-templated-annotation-range')
so this stays a clean round trip."
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
             (template (verilog-auto--find-template
                        template-comments type-name (treesit-node-start module-instantiation)))
             (inst-name (and template (verilog-auto--instance-name-of hier)))
             (num-result (and template (verilog-auto--instance-number (or inst-name "") (nth 2 template))))
             (inst-number (or (nth 0 num-result) ""))
             (num-kind (nth 1 num-result))
             (num-text (nth 2 num-result))
             (dims-alist (gethash type-name verilog-auto--module-port-dims))
             (remaining (verilog-auto--filter
                         (lambda (p) (not (member (nth 0 p) connected)))
                         ports))
             (groups (verilog-auto--group-by-direction remaining))
             (open-paren (treesit-node-child hier 1))
             (indent (make-string (1+ (verilog-auto--node-column open-paren)) ?\s))
             (result (verilog-auto--inst-lines
                      groups overrides indent template inst-number dims-alist (treesit-node-start comment)
                      inst-name type-name))
             (lines (nth 0 result))
             (col (nth 1 result))
             (last-ann (nth 2 result)))
        (when num-kind
          (let ((pos (treesit-node-start comment)))
            (cond
             ((eq num-kind 'no-match)
              (push (cons pos num-text) verilog-auto--template-instance-number-notices))
             ((eq num-kind 'no-capture-group)
              (push (cons pos num-text) verilog-auto--port-marker-arg-warnings)))))
        (when lines
          (goto-char (treesit-node-end comment))
          (insert "\n" (string-join lines "\n"))
          (when last-ann
            (end-of-line)
            (insert (verilog-auto--pad-to-column "" col (current-column)))
            (insert last-ann)))
        1))))

(defun verilog-auto--expand-all-autoinst ()
  "Expand every pending /*AUTOINST*/ site in the current buffer from a
single parse (see this file's header for why processing them
rightmost-first needs no reparse between sites). TEMPLATE-COMMENTS
(every `block_comment' node in ROOT) is gathered exactly once here and
threaded through to every site (M92 fix round S4), rather than each
site re-walking the whole tree on its own. Returns the count of
instances processed."
  (let* ((root (verilog-auto--parse-current-buffer))
         (mis (verilog-auto--find-all-of-type root "module_instantiation"))
         (template-comments (verilog-auto--find-all-of-type root "block_comment"))
         (sites nil))
    (dolist (mi mis)
      (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
        (when c (push (list (treesit-node-start c) mi c) sites))))
    (setq sites (sort sites (lambda (a b) (> (car a) (car b)))))
    (let ((total 0))
      (dolist (site sites total)
        (setq total (+ total (verilog-auto--expand-autoinst-site template-comments (nth 1 site) (nth 2 site))))))))

;; --- AUTOOUTPUT / AUTOINPUT / AUTOINOUT (M125) -------------------------------
;; See this file's own M125 header comment (above, before "Idempotence and
;; undo") for the full design rationale and every deliberate GNU divergence.

(defconst verilog-auto--port-kind-specs
  '((output "AUTOOUTPUT" "// Beginning of automatic outputs (from unused autoinst outputs)" "output" "From")
    (input  "AUTOINPUT"  "// Beginning of automatic inputs (from unused autoinst inputs)"   "input"  "To")
    (inout  "AUTOINOUT"  "// Beginning of automatic inouts (from unused autoinst inouts)"    "inout"  "To/From"))
  "One entry per M125 command: (KIND-SYMBOL MARKER-KEYWORD
BEGIN-COMMENT-TEXT DECL-KEYWORD PROVENANCE-VERB). KIND-SYMBOL is
'output/'input/'inout -- the SAME symbols `verilog-auto--port-direction-
of' returns, deliberately, so a candidate's own recorded directions can
be tested with a plain `member' against this table's own key.")

(defun verilog-auto--find-port-marker-comments (node keyword)
  "Every `/*KEYWORD*/' or `/*KEYWORD(...)*/' block_comment descendant of
NODE -- KEYWORD one of \"AUTOOUTPUT\"/\"AUTOINPUT\"/\"AUTOINOUT\", with
or without a regexp argument (spec section 2.10)."
  (let ((pat (concat "\\`/\\*" (regexp-quote keyword) "\\(?:\\*/\\|(\\)")))
    (verilog-auto--find-all
     node
     (lambda (n) (and (string= (treesit-node-type n) "block_comment")
                       (string-match-p pat (treesit-node-text n)))))))

(defun verilog-auto--any-auto-port-block-marker-p (node)
  "Non-nil if NODE is any of the SEVEN block-style AUTO markers this
file recognizes -- `/*AUTOWIRE*/', an AUTOOUTPUT/AUTOINPUT/AUTOINOUT
marker, an AUTOREG/AUTOTIEOFF marker, or an AUTORESET marker (M126:
widened from four to six; M134: widened from six to seven; with or
without its own regexp argument -- AUTOREG/AUTOTIEOFF/AUTORESET never
actually accept one, but the marker-recognition SHAPE is identical, and
a malformed-argument marker still needs to be recognized here). Used by
`verilog-auto--autowire-stale-end' (M125: generalized) to recognize
ANY of the seven as proof that ITS OWN \"// End of automatics\" is
missing, not just another `/*AUTOWIRE*/' -- see this file's M125 header
for why that generalization matters now that several DIFFERENT marker
kinds sitting immediately adjacent (AUTOTIEOFF directly followed by
AUTOREG, per the M126 execution order) is the NORMAL shape, not a
corrupted one."
  (and (string= (treesit-node-type node) "block_comment")
       (let ((text (treesit-node-text node)))
         (or (string= text "/*AUTOWIRE*/")
             (string-match-p "\\`/\\*AUTOOUTPUT\\(?:\\*/\\|(\\)" text)
             (string-match-p "\\`/\\*AUTOINPUT\\(?:\\*/\\|(\\)" text)
             (string-match-p "\\`/\\*AUTOINOUT\\(?:\\*/\\|(\\)" text)
             (string-match-p "\\`/\\*AUTOREG\\(?:\\*/\\|(\\)" text)
             (string-match-p "\\`/\\*AUTOTIEOFF\\(?:\\*/\\|(\\)" text)
             (string-match-p "\\`/\\*AUTORESET\\(?:\\*/\\|(\\)" text)))))

(defun verilog-auto--port-marker-arg (comment keyword)
  "COMMENT's own optional regexp argument (spec section 2.10), as a
plist (:filter REGEXP-OR-NIL :invert BOOL :malformed BOOL). No argument
at all (`/*KEYWORDPLAIN*/') -> filter nil. A well-formed
`/*KEYWORD(\"PAT\")*/' -> filter PAT, with a leading `?!' inside the
quotes stripped and :invert t (the inversion spelling, spec section
1.7 -- `?!' OUTSIDE the quotes, `/*KEYWORD(?!\"PAT\")*/', is the
measured GNU trap and falls through to :malformed t here instead of
GNU's own silent \"matches everything\" -- see this file's M125
header). Any other shape after the keyword (an unterminated string, a
bare unquoted argument, stray text) is :malformed t, filter/invert both
nil."
  (let* ((text (treesit-node-text comment))
         (rest (substring text (+ 2 (length keyword)))))
    (cond
     ((string= rest "*/") (list :filter nil :invert nil :malformed nil))
     ((string-match "\\`(\"\\([^\"]*\\)\")\\*/\\'" rest)
      (let ((raw (match-string 1 rest)))
        (if (string-prefix-p "?!" raw)
            (list :filter (substring raw 2) :invert t :malformed nil)
          (list :filter raw :invert nil :malformed nil))))
     (t (list :filter nil :invert nil :malformed t)))))

(defun verilog-auto--port-marker-signal-passes-p (name arg)
  "Non-nil if NAME passes ARG's own filter (`verilog-auto--port-marker-
arg') -- always t when ARG has no :filter at all (including the
:malformed case, which is functionally \"no filter\" here, matching
GNU's own observed behavior -- see this file's M125 header for why
that's still recorded, just never silent)."
  (let ((filter (plist-get arg :filter)) (invert (plist-get arg :invert)))
    (if (not filter)
        t
      (let ((m (and (string-match-p filter name) t)))
        (if invert (not m) m)))))

(defun verilog-auto--connection-candidate-name (ctext)
  "CTEXT (a connection's own exact, already `string-trim'med text) as a
candidate signal name for AUTOOUTPUT/AUTOINPUT/AUTOINOUT (spec section
2.5, rule 2): CTEXT itself when it's a bare identifier
(`verilog-auto--bare-identifier-p'), or the identifier prefix of a bare
identifier followed by a SINGLE, non-nested bit-select/part-select of
the WHOLE signal (`rdata_o[DataWidth-1:0]', `rdata_o[3]') -- GNU strips
the bit-select and still counts the bare signal (measured, spec section
1.5 case 20). Anything else (concatenation, a constant, a nested or
otherwise composite expression) returns nil."
  (cond
   ((verilog-auto--bare-identifier-p ctext) ctext)
   ((string-match "\\`\\([A-Za-z_$][A-Za-z0-9_$]*\\)\\[[^][]*\\]\\'" ctext)
    (match-string 1 ctext))
   (t nil)))

(defun verilog-auto--port-propagation-candidates (module-decl)
  "Every bare-connection candidate signal for AUTOOUTPUT/AUTOINPUT/
AUTOINOUT in MODULE-DECL (spec section 2.5). Returns (TABLE ORDER
CONFLICTS):
- TABLE: a hash NAME -> vector [DIRS TYPE RANGE INST MOD COUNT]. DIRS is
  the list of port directions ('output/'input/'inout) NAME was EVER
  connected to, across every instance in MODULE-DECL (deduped, order
  irrelevant -- only membership is ever tested). TYPE/RANGE/INST/MOD are
  the FIRST contributing instance's own declared type text, param-
  substituted range text (`verilog-auto--substitute-params', using THAT
  instance's own overrides -- AUTOWIRE's own precedent, spec section
  2.4), instance name, and provenance module text
  (`verilog-auto--module-file', or the bare module name when unknown).
  COUNT is the number of contributing connections total, used for the
  `, ...' provenance ellipsis (spec section 1.4).
- ORDER: NAME list, first-CONNECTION order, deduped.
- CONFLICTS: NAME list (unordered w.r.t. ORDER, but each pushed once)
  for which some LATER contributing connection's own param-substituted
  range text disagreed with the FIRST one's -- the first-seen range
  still wins in TABLE (matching AUTOWIRE's own dedup), this list exists
  purely so the disagreement isn't silent (spec section 2.4).

A port whose OWN direction classifies as `'interface' (an actual
interface-modport ANSI port) contributes to none of the three commands
-- see this file's M125 header for the separate, honestly-flagged
divergence regarding a package-scoped user-defined-type port, which
this grammar does NOT classify as `'interface' the way GNU's own
verilog-mode does."
  (let ((table (make-hash-table :test 'equal))
        (order nil)
        (conflicts nil))
    (dolist (mi (verilog-auto--find-all-of-type module-decl "module_instantiation"))
      (let* ((type-name (treesit-node-text
                          (treesit-node-child-by-field-name mi "instance_type")))
             (ports (verilog-auto--module-ports type-name)))
        (when ports
          (let ((full (gethash type-name verilog-auto--module-full-ports))
                (mod-text (or (gethash type-name verilog-auto--module-file) type-name))
                (overrides (verilog-auto--instance-param-overrides mi)))
            (dolist (hier (verilog-auto--find-all-of-type mi "hierarchical_instance"))
              (let ((inst-name (treesit-node-text
                                 (verilog-auto--find-first-of-type hier "name_of_instance"))))
                (dolist (conn (verilog-auto--find-all-of-type hier "named_port_connection"))
                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))
                         (cand (and pinfo ctext (verilog-auto--connection-candidate-name ctext))))
                    (when (and cand (not (eq (nth 1 pinfo) 'interface)))
                      (let* ((dir (nth 1 pinfo))
                             (finfo (and full (assoc pname full)))
                             (range (verilog-auto--substitute-params (or (nth 2 pinfo) "") overrides))
                             (type (and finfo (nth 3 finfo)))
                             (existing (gethash cand table)))
                        (if (not existing)
                            (progn
                              (puthash cand (vector (list dir) type range inst-name mod-text 1) table)
                              (push cand order))
                          (progn
                            (unless (member dir (aref existing 0))
                              (aset existing 0 (cons dir (aref existing 0))))
                            (aset existing 5 (1+ (aref existing 5)))
                            (let ((existing-range (or (aref existing 2) "")))
                              (unless (or (string= range existing-range) (member cand conflicts))
                                (push cand conflicts)))))))))))))))
    (list table (nreverse order) (nreverse conflicts))))

(defun verilog-auto--port-propagation-select (kind table order)
  "ORDER filtered to the names TABLE classifies for KIND
('output/'input/'inout), per spec section 2.5 rule 4: AUTOOUTPUT is
O \\ (I u B), AUTOINPUT is I \\ (O u B), AUTOINOUT is unconditionally
B -- an inout is always declared regardless of what else touches the
same name."
  (verilog-auto--filter
   (lambda (name)
     (let ((dirs (aref (gethash name table) 0)))
       (cond
        ((eq kind 'output) (and (member 'output dirs) (not (member 'input dirs)) (not (member 'inout dirs))))
        ((eq kind 'input) (and (member 'input dirs) (not (member 'output dirs)) (not (member 'inout dirs))))
        ((eq kind 'inout) (and (member 'inout dirs) t))
        (t nil))))
   order))

(defun verilog-auto--module-own-port-names (module-decl header)
  "Names of MODULE-DECL's own header ports (ANSI or non-ANSI) -- the
\"enclosing module's own ports are excluded\" rule, spec section 1.5."
  (if (verilog-auto--ansi-header-p header)
      (mapcar (lambda (d) (treesit-node-text (treesit-node-child-by-field-name d "port_name")))
              (verilog-auto--find-all-of-type header "ansi_port_declaration"))
    (mapcar #'treesit-node-text (verilog-auto--find-all-of-type header "port"))))

;; --- Position-ordered notice lists (M125 trailing fix round) -------------
;;
;; `verilog-auto--predeclared-port-names', `verilog-auto--port-range-
;; conflicts' and `verilog-auto--port-marker-arg-warnings' are each a list
;; of (MARKER-COMMENT-START-POSITION . TEXT) conses, not bare TEXT --
;; deliberately, replacing an earlier "rely on push/traversal-direction
;; order" scheme that a cold review found broken: `verilog-auto--expand-
;; all-port-propagation' is called three times, once per KIND ('output/
;; 'input/'inout), each its OWN independent rightmost-first walk. Getting
;; ONE call's own push order to equal document order (which the earlier
;; scheme did, correctly, WITHIN one call) says nothing about ordering
;; ACROSS the three calls -- their relative order is decided by the fixed
;; 'output -> 'input -> 'inout sequence in `verilog-auto', which has
;; nothing to do with where either offending module actually sits in the
;; buffer. A module using AUTOOUTPUT earlier in the buffer could
;; therefore lose "first" to a module using AUTOINPUT later on, purely
;; because the AUTOINPUT pass runs after the AUTOOUTPUT pass. Storing
;; each entry's own marker-comment START POSITION alongside its text and
;; picking the SMALLEST position at message-formatting time
;; (`verilog-auto--notice-first') sidesteps traversal order and call
;; order entirely -- for ANY of the three lists, regardless of which
;; kind(s) contributed to it or in what order those kinds happened to
;; run, the earliest-on-screen offender always wins. This is why the
;; three lists are no longer `nreverse'd in `verilog-auto' either: an
;; unordered bag scanned for a minimum needs no particular push order at
;; all.

(defun verilog-auto--notice-contains-p (list text)
  "Non-nil if TEXT already appears as the `cdr' of some entry in LIST (a
list of (POSITION . TEXT) conses, as the three position-ordered notice
lists above use) -- the dedup check each of them runs before pushing a
new entry, unchanged in MEANING from the pre-position-tracking version
(dedup by TEXT, not by position)."
  (let (found)
    (dolist (e list found)
      (when (equal (cdr e) text)
        (setq found t)))))

(defun verilog-auto--notice-first (list)
  "TEXT of the entry in LIST (a list of (POSITION . TEXT) conses) whose
own POSITION is SMALLEST -- i.e., the earliest in the buffer, regardless
of push order, traversal direction, or which of several passes
contributed it. nil if LIST is empty."
  (let (best-pos best-text)
    (dolist (e list best-text)
      (when (or (null best-pos) (< (car e) best-pos))
        (setq best-pos (car e) best-text (cdr e))))))

(defun verilog-auto--port-decl-line (indent kind entry name)
  "One declaration line's own full text, INDENT included (spec section
2.1's layout: INDENT DECL-KEYWORD [TYPE] [RANGE] NAME; padded to
`verilog-auto-inst-column' then the `// From/To ... of ...' provenance
comment). ENTRY is TABLE's own vector for NAME
(`verilog-auto--port-propagation-candidates'); KIND selects DECL-KEYWORD
and the provenance VERB from `verilog-auto--port-kind-specs'."
  (let* ((spec (assoc kind verilog-auto--port-kind-specs))
         (decl-kw (nth 3 spec))
         (verb (nth 4 spec))
         (type (aref entry 1))
         (range (aref entry 2))
         (inst (aref entry 3))
         (mod (aref entry 4))
         (count (aref entry 5))
         (body (concat decl-kw " "
                       (if type (concat type " ") "")
                       (if (and range (> (length range) 0)) (concat range " ") "")
                       name ";"))
         (comment (format "// %s %s of %s%s" verb inst mod (if (> count 1) ", ..." ""))))
    (concat indent (verilog-auto--pad-to-column body verilog-auto-inst-column (length indent)) comment)))

(defun verilog-auto--expand-port-propagation-insert (kind comment header module-decl table order arg)
  "Insert KIND's own declaration block right after COMMENT, if ORDER's
selected, filtered, sorted names is non-empty (spec section 1.6: an
empty candidate set inserts nothing at all -- not even bare Begin/End
markers). Returns the number of declarations inserted."
  (let* ((own (verilog-auto--module-own-port-names module-decl header))
         (declared (verilog-auto--declared-names module-decl))
         (selected (verilog-auto--port-propagation-select kind table order))
         (spec (assoc kind verilog-auto--port-kind-specs))
         (begin-comment (nth 2 spec))
         (filtered
          (verilog-auto--filter
           (lambda (name)
             (cond
              ((member name own) nil)
              ((member name declared)
               (unless (verilog-auto--notice-contains-p
                        verilog-auto--predeclared-port-names name)
                 (push (cons (treesit-node-start comment) name)
                       verilog-auto--predeclared-port-names))
               nil)
              (t (verilog-auto--port-marker-signal-passes-p name arg))))
           selected))
         (sorted (sort (copy-sequence filtered) #'string<)))
    (when sorted
      (let ((indent (verilog-auto--line-indent (treesit-node-start comment))))
        (goto-char (treesit-node-end comment))
        (insert
         "\n" indent begin-comment
         (mapconcat
          (lambda (name)
            (concat "\n" (verilog-auto--port-decl-line indent kind (gethash name table) name)))
          sorted "")
         "\n" indent "// End of automatics")))
    (length sorted)))

(defun verilog-auto--expand-port-propagation-site (kind comment)
  "Expand one AUTOOUTPUT/AUTOINPUT/AUTOINOUT site (per KIND).
ANSI-header guard (spec section 2.2) and the malformed-argument notice
(spec section 2.10) both happen here, before candidates are ever
computed for an ANSI-guarded module (nothing would use them).

M134: the guard is `verilog-auto--ansi-header-with-ports-p', not the
bare `verilog-auto--ansi-header-p' -- a port-less ANSI header (`module
top;') declares no ports inline at all, so there is nothing for this
site to be redundant with; GNU (measured, M134 recon) expands it
exactly like a non-ANSI header. Only a header that actually carries an
inline port list has \"nothing left to add\"."
  (let* ((keyword (nth 1 (assoc kind verilog-auto--port-kind-specs)))
         (module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-with-ports-p header)
        (progn
          (let ((nm (verilog-auto--module-name module-decl)))
            (unless (member nm verilog-auto--ansi-port-auto-modules)
              (push nm verilog-auto--ansi-port-auto-modules)))
          0)
      (let* ((arg (verilog-auto--port-marker-arg comment keyword))
             (result (verilog-auto--port-propagation-candidates module-decl))
             (table (nth 0 result)) (order (nth 1 result)) (conflicts (nth 2 result))
             (pos (treesit-node-start comment)))
        (dolist (c conflicts)
          (unless (verilog-auto--notice-contains-p verilog-auto--port-range-conflicts c)
            (push (cons pos c) verilog-auto--port-range-conflicts)))
        (when (plist-get arg :malformed)
          (push (cons pos
                      (format "%s(...) in module %s: malformed regexp argument, treated as no filter"
                              keyword (verilog-auto--module-name module-decl)))
                verilog-auto--port-marker-arg-warnings))
        (verilog-auto--expand-port-propagation-insert kind comment header module-decl table order arg)))))

(defun verilog-auto--expand-all-port-propagation (kind)
  "Expand every KIND ('output/'input/'inout) AUTOOUTPUT/AUTOINPUT/
AUTOINOUT site in the current buffer from a fresh parse -- one module,
one marker of each kind, GNU convention (AUTOWIRE's own precedent, this
file's M39 header; `verilog-auto--first-autowire-per-module' is already
generic over \"which module encloses this comment\" and is reused here
unchanged). A later marker of the SAME kind in the SAME module is left
as a bare, unexpanded comment -- see this file's M125 header for why
that stays undocumented-by-notice, a deliberate scope decision under
ambiguity. Returns the count of declarations inserted."
  (let* ((keyword (nth 1 (assoc kind verilog-auto--port-kind-specs)))
         (root (verilog-auto--parse-current-buffer))
         (comments (verilog-auto--find-port-marker-comments root keyword))
         (split (verilog-auto--first-autowire-per-module comments))
         (firsts (car split))
         (sorted (sort (copy-sequence firsts)
                       (lambda (a b) (> (treesit-node-start a) (treesit-node-start b))))))
    (let ((total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-port-propagation-site kind c)))))))

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
    ;; M125: a TYPED non-ANSI body port declaration (`output logic
    ;; rvalid_o;') parses under `list_of_variable_port_identifiers', a
    ;; DIFFERENT node type than the untyped `output [7:0] dout;' shape
    ;; above -- confirmed by a real tree dump (M125 recon), not assumed.
    ;; Without this, AUTOOUTPUT would re-declare a name it itself just
    ;; declared on module_declaration re-parse (predeclared-name skip,
    ;; see this file's M125 header), and AUTOWIRE would try to declare a
    ;; `wire' for a name that is now a port.
    (dolist (n (verilog-auto--find-all-of-type module-decl "list_of_variable_port_identifiers"))
      (dolist (id (verilog-auto--find-all-of-type n "simple_identifier"))
        (push (treesit-node-text id) acc)))
    (dolist (n (verilog-auto--find-all-of-type module-decl "ansi_port_declaration"))
      (push (treesit-node-text (treesit-node-child-by-field-name n "port_name")) acc))
    (nreverse acc)))

(defun verilog-auto--body-declared-names (module-decl)
  "Every signal name declared by a BODY net/variable declaration in
MODULE-DECL -- `net_decl_assignment'/`variable_decl_assignment' only,
deliberately excluding every PORT-identifier source
`verilog-auto--declared-names' also walks (`list_of_port_identifiers',
`list_of_variable_port_identifiers', `ansi_port_declaration'). M126:
AUTOREG/AUTOTIEOFF both need \"is this name already declared in the
BODY\" (R3/R4, T4), which is a narrower question than AUTOWIRE/AUTOOUTPUT's
own \"is this name declared ANYWHERE, including as a port\" --
`verilog-auto--declared-names' is NOT reusable as-is here: `output [3:0]
a;' is itself a non-ANSI PORT declaration, so it would make `a' look
already-declared and BOTH new commands would emit nothing, ever (R1/T1
would both regress to R9/T13's \"no outputs\" shape). Confirmed by a
real tree dump (M126 recon, spec section 3 probes 1/2) that an
UNINITIALISED net/variable declaration (`wire [3:0] a;', `reg [3:0]
a;') still produces its own `net_decl_assignment'/`variable_decl_
assignment' node (just with no `=' child) -- so this walk, unlike
`verilog-auto--declared-names''s two analogous dolist blocks, needs no
special-casing for the no-initialiser case either; it is identical to
those two blocks, verbatim, with only the port-identifier blocks
dropped."
  (let (acc)
    (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
      (push (treesit-node-text (treesit-node-child n 0)) acc))
    (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
      (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc))
    (nreverse acc)))

(defun verilog-auto--net-lvalue-driven-names (lvalue)
  "Base signal name(s) driven by LVALUE, a `net_lvalue' node (the
left-hand side of one `net_assignment' inside a `continuous_assign', or
one of its own nested elements). M126 spec section 3 probe 4 (real tree
dump): a concatenation LHS (`{x, y}') parses as a `net_lvalue' whose OWN
children include further NESTED `net_lvalue' nodes, one per element --
recursed into here, so every element counts as driven. A part-select
LHS (`a[3:0]') parses as a `net_lvalue' with a `simple_identifier'
child (the base name) followed by a SIBLING `constant_select' child (the
index/range) -- NOT nested inside another `net_lvalue', so the
recursion bottoms out and the first `simple_identifier' found (document
order, i.e. the base name itself, before any identifier that might
appear symbolically INSIDE the index expression) is taken; a partial
drive is deliberately treated as a full drive, the conservative
direction (it suppresses a candidate declaration rather than risking a
real duplicate-declaration error)."
  (let ((nested (verilog-auto--find-all-of-type lvalue "net_lvalue")))
    (if nested
        (apply #'append (mapcar #'verilog-auto--net-lvalue-driven-names nested))
      (let ((id (verilog-auto--find-first-of-type lvalue "simple_identifier")))
        (and id (list (treesit-node-text id)))))))

(defun verilog-auto--continuous-assign-driven-names (module-decl)
  "Every base signal name driven by some `continuous_assign' anywhere in
MODULE-DECL (R17/T12) -- `verilog-auto--net-lvalue-driven-names' applied
to every `net_assignment''s own left-hand side (its child 0, dump-
verified: a `net_assignment' is always `net_lvalue' `=' `expression',
the SAME positional-child idiom `verilog-auto--declared-names' already
uses for `net_decl_assignment''s own name). Procedural (`always'-block)
drivers are deliberately NOT consulted here or anywhere else in AUTOREG/
AUTOTIEOFF -- W4 measured GNU emitting the `reg' anyway, which is the
correct answer: an `always'-driven output must legally be a `reg', so
AUTOREG must still declare it, and AUTOTIEOFF tying it off would be a
duplicate driver."
  (let (acc)
    (dolist (ca (verilog-auto--find-all-of-type module-decl "continuous_assign"))
      (dolist (na (verilog-auto--find-all-of-type ca "net_assignment"))
        (dolist (nm (verilog-auto--net-lvalue-driven-names (treesit-node-child na 0)))
          (push nm acc))))
    (nreverse acc)))

(defun verilog-auto--driven-output-names (module-decl)
  "Union of every name driven by a submodule instance connection (R5/T5)
and by a continuous assign (R17/T12, `verilog-auto--continuous-assign-
driven-names') in MODULE-DECL -- shared by AUTOREG and AUTOTIEOFF. The
instance-connection half REUSES `verilog-auto--port-propagation-
candidates' as-is (no refactor needed): that function already computes,
per candidate NAME, every direction it was EVER connected to across
every instance in MODULE-DECL, so \"driven by an instance\" is exactly
\"'output is among its recorded directions\" -- the recon flagged that
AUTOWIRE (`verilog-auto--expand-autowire-site') and this function
already contain two INDEPENDENTLY WRITTEN copies of the identical
named-port-connection walk; reusing the port-propagation one here
rather than writing a THIRD keeps that number at two, not three."
  (let* ((result (verilog-auto--port-propagation-candidates module-decl))
         (table (nth 0 result)) (order (nth 1 result))
         (inst-driven (verilog-auto--filter
                       (lambda (name) (member 'output (aref (gethash name table) 0)))
                       order)))
    (append inst-driven (verilog-auto--continuous-assign-driven-names module-decl))))

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
  (let* ((module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (declared (verilog-auto--declared-names module-decl))
         (insts (verilog-auto--find-all-of-type module-decl "module_instantiation"))
         (seen (make-hash-table :test 'equal))
         (candidates nil))
    (dolist (mi insts)
      (let* ((type-name (treesit-node-text
                          (treesit-node-child-by-field-name mi "instance_type")))
             (ports (verilog-auto--module-ports type-name))
             (overrides (verilog-auto--instance-param-overrides mi))
             ;; M92 review fix: the grammar allows a SINGLE
             ;; module_instantiation to hold several comma-separated
             ;; hierarchical_instance children (`sometype u1(...), u2(...);'
             ;; -- see M39's own tree-shape notes). `find-first-of-type'
             ;; used to only ever look at the FIRST one, so every
             ;; instance but the first silently lost its outputs' wire
             ;; candidacy; AUTOINST is unaffected since it scopes to the
             ;; hierarchical_instance enclosing its own /*AUTOINST*/
             ;; comment, never to `mi' as a whole.
             (hiers (verilog-auto--find-all-of-type mi "hierarchical_instance")))
        (dolist (hier hiers)
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
      (let* ((m (verilog-auto--enclosing-of-types c '("module_declaration" "interface_declaration")))
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
      (let* ((m (verilog-auto--enclosing-of-types c '("module_declaration" "interface_declaration")))
             (nm (and m (verilog-auto--module-name m))))
        (when (and nm (not (member nm verilog-auto--multi-autowire-modules)))
          (push nm verilog-auto--multi-autowire-modules))))
    (let ((sorted (sort (copy-sequence firsts)
                        (lambda (a b) (> (treesit-node-start a) (treesit-node-start b)))))
          (total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-autowire-site c)))))))

;; --- M126: AUTOTIEOFF (runs BEFORE AUTOWIRE -- see this file's M126 header
;;     for the execution-order dependency this creates with AUTOREG) --------

(defun verilog-auto--tieoff-decl-line (indent decl-kw signed range name const)
  "One AUTOTIEOFF declaration line: INDENT DECL-KW [`signed'] [RANGE]
NAME, padded to `verilog-auto-inst-column' (M125's own `verilog-auto--
port-decl-line' precedent -- this file's SPACES-not-tabs convention,
M126 divergence 1), then `= CONST;'. SIGNED and RANGE are only ever
printed when DECL-KW is \"wire\" -- the `assign' form (either the
`verilog-auto-tieoff-declaration' knob or the ANSI-header switch,
divergence 2) is not a declaration at all, so neither a type keyword
nor a range belongs on it (T2/T6, spec section 4.5)."
  (let* ((typed (equal decl-kw "wire"))
         (body (concat decl-kw
                       (if (and typed signed) " signed" "")
                       (if (and typed (> (length range) 0)) (concat " " range) "")
                       " " name)))
    (concat indent (verilog-auto--pad-to-column body verilog-auto-inst-column (length indent))
            "= " const ";")))

(defun verilog-auto--expand-autotieoff-site (comment)
  "Expand one /*AUTOTIEOFF*/ site. Returns the number of tie-off
declarations inserted (0 if nothing qualifies -- no Beginning/End
markers, T13, matching every other block-style AUTO command in this
file)."
  (let* ((module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl))
         (ansi (verilog-auto--ansi-header-p header))
         (text (treesit-node-text comment)))
    (if (not (string= text "/*AUTOTIEOFF*/"))
        (progn
          (push (cons (treesit-node-start comment)
                      (format "AUTOTIEOFF(...) in module %s: takes no argument, expansion skipped"
                              (verilog-auto--module-name module-decl)))
                verilog-auto--port-marker-arg-warnings)
          0)
      (let* ((body-declared (verilog-auto--body-declared-names module-decl))
             (driven (verilog-auto--driven-output-names module-decl))
             (pairs (verilog-auto--output-port-candidate-decls module-decl header))
             (candidates nil)
             (resolved nil))
        ;; Pass 1: which ports even qualify (T4/T5/T11/T12 are handled by
        ;; the caller's own restriction to OUTPUT ports; T10's divergence
        ;; 3 port-reg skip happens here).
        (dolist (p pairs)
          (let* ((nm (car p)) (decl (cdr p)) (port-reg (verilog-auto--decl-port-reg-p decl)))
            (cond
             ((member nm body-declared) nil)
             ((member nm driven) nil)
             (port-reg
              (unless (member nm verilog-auto--tieoff-port-reg-skips)
                (push nm verilog-auto--tieoff-port-reg-skips)))
             (t (push (cons nm decl) candidates)))))
        ;; Pass 2: fold each qualifying port's own range into a constant,
        ;; dropping (with a notice) any whose multi-dimensional range has
        ;; a symbolic dimension (divergence 5).
        (dolist (c candidates)
          (let* ((nm (car c)) (decl (cdr c))
                 (dims (verilog-auto--all-range-texts decl))
                 (signed (verilog-auto--decl-signed-p decl))
                 (cr (verilog-auto--tieoff-constant dims signed)))
            (if (cdr cr)
                (unless (member nm verilog-auto--tieoff-symbolic-multidim-skips)
                  (push nm verilog-auto--tieoff-symbolic-multidim-skips))
              (push (list nm (verilog-auto--joined-range-text decl) (car cr) signed) resolved))))
        (setq resolved (sort resolved (lambda (a b) (string< (car a) (car b)))))
        (when (and ansi resolved
                   (not (member (verilog-auto--module-name module-decl)
                                verilog-auto--ansi-tieoff-assign-modules)))
          (push (verilog-auto--module-name module-decl) verilog-auto--ansi-tieoff-assign-modules))
        (when resolved
          (let ((indent (verilog-auto--line-indent (treesit-node-start comment)))
                (decl-kw (cond (ansi "assign")
                                ((equal verilog-auto-tieoff-declaration "assign") "assign")
                                (t "wire"))))
            (goto-char (treesit-node-end comment))
            (insert
             "\n" indent "// Beginning of automatic tieoffs (for this module's unterminated outputs)"
             (mapconcat
              (lambda (r)
                (concat "\n" (verilog-auto--tieoff-decl-line
                              indent decl-kw (nth 3 r) (nth 1 r) (nth 0 r) (nth 2 r))))
              resolved "")
             "\n" indent "// End of automatics")))
        (length resolved)))))

(defun verilog-auto--expand-all-autotieoff ()
  "Expand the FIRST /*AUTOTIEOFF*/ site in each module -- same
one-marker-per-module convention this file already applies to AUTOWIRE
(M39) and AUTOOUTPUT/AUTOINPUT/AUTOINOUT (M125); a later marker in the
SAME module is left as a bare, unexpanded comment with no dedicated
notice, the same scope decision M125 made for its own three commands
(see this file's M125 header) rather than an untested new mechanism."
  (let* ((root (verilog-auto--parse-current-buffer))
         (comments (verilog-auto--find-port-marker-comments root "AUTOTIEOFF"))
         (split (verilog-auto--first-autowire-per-module comments))
         (firsts (car split))
         (sorted (sort (copy-sequence firsts)
                       (lambda (a b) (> (treesit-node-start a) (treesit-node-start b))))))
    (let ((total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-autotieoff-site c)))))))

;; --- M126: AUTOREG (runs AFTER AUTOWIRE, so it sees a fresh reparse of
;;     whatever AUTOTIEOFF/AUTOWIRE just inserted -- this is what makes the
;;     AUTOTIEOFF-suppresses-AUTOREG dependency, O2/T9, work with zero
;;     special-casing: see this file's M126 header) -----------------------

(defun verilog-auto--expand-autoreg-site (comment)
  "Expand one /*AUTOREG*/ site. Returns the number of `reg' declarations
inserted (0 if nothing qualifies -- no Beginning/End markers, R9).

M134: gated on `verilog-auto--ansi-header-with-ports-p', not the bare
`verilog-auto--ansi-header-p' -- see that predicate's doc string; a
port-less ANSI header (`module top;') has no inline ports to conflict
with AUTOREG's own job of adding `reg'/`logic' to undeclared outputs."
  (let* ((module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-with-ports-p header)
        (progn
          (let ((nm (verilog-auto--module-name module-decl)))
            (unless (member nm verilog-auto--ansi-autoreg-modules)
              (push nm verilog-auto--ansi-autoreg-modules)))
          0)
      (let ((text (treesit-node-text comment)))
        (if (not (string= text "/*AUTOREG*/"))
            (progn
              (push (cons (treesit-node-start comment)
                          (format "AUTOREG(...) in module %s: takes no argument, expansion skipped"
                                  (verilog-auto--module-name module-decl)))
                    verilog-auto--port-marker-arg-warnings)
              0)
          (let* ((body-declared (verilog-auto--body-declared-names module-decl))
                 (driven (verilog-auto--driven-output-names module-decl))
                 (pairs (verilog-auto--output-port-candidate-decls module-decl header))
                 (candidates nil))
            (dolist (p pairs)
              (let ((nm (car p)) (decl (cdr p)))
                (unless (or (verilog-auto--decl-has-type-keyword-p decl)
                            (member nm body-declared) (member nm driven))
                  (push (cons nm decl) candidates))))
            (setq candidates (sort candidates (lambda (a b) (string< (car a) (car b)))))
            (when candidates
              (let ((indent (verilog-auto--line-indent (treesit-node-start comment))))
                (goto-char (treesit-node-end comment))
                (insert
                 "\n" indent "// Beginning of automatic regs (for this module's undeclared outputs)"
                 (mapconcat
                  (lambda (c)
                    (let* ((nm (car c)) (decl (cdr c))
                           (range (verilog-auto--joined-range-text decl))
                           (signed (verilog-auto--decl-signed-p decl)))
                      (concat "\n" indent "reg"
                              (if signed " signed" "")
                              (if (> (length range) 0) (concat " " range) "")
                              " " nm ";")))
                  candidates "")
                 "\n" indent "// End of automatics")))
            (length candidates)))))))

(defun verilog-auto--expand-all-autoreg ()
  "Expand the FIRST /*AUTOREG*/ site in each module -- same
one-marker-per-module convention as `verilog-auto--expand-all-
autotieoff' above (see that function's own doc string)."
  (let* ((root (verilog-auto--parse-current-buffer))
         (comments (verilog-auto--find-port-marker-comments root "AUTOREG"))
         (split (verilog-auto--first-autowire-per-module comments))
         (firsts (car split))
         (sorted (sort (copy-sequence firsts)
                       (lambda (a b) (> (treesit-node-start a) (treesit-node-start b))))))
    (let ((total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-autoreg-site c)))))))

;; --- M134: AUTORESET ----------------------------------------------------
;;
;; GNU verilog-mode's `/*AUTORESET*/': expands, INSIDE the always-block
;; branch it sits in, one reset assignment per signal that always-block
;; assigns ANYWHERE -- except a signal already assigned in the marker's
;; OWN branch (spec section 1, measured GNU Emacs 30.2). Unlike every
;; other AUTO command in this file, the marker is scoped to its own
;; ENCLOSING ALWAYS BLOCK, not to the enclosing module -- two `always'
;; blocks in one module, each with its own `/*AUTORESET*/', both expand
;; independently (measured; this is why `verilog-auto--first-autowire-
;; per-module', which keeps only the first marker PER MODULE, must NOT
;; be reused here -- see this file's M134 header note at the top of the
;; expander below).

(defcustom verilog-auto-reset-widths t
  "How `/*AUTORESET*/' formats each signal's own reset constant (GNU
default `t', kept as the default here too, per this file's M134 header
divergence 2 -- even though `demo/rtl/' itself writes `\\='0' for
every reset, a reader following GNU's own manual should see GNU's own
output by default):
- `t' (GNU default): sized hex matching the signal's own declared
  width -- `16\\='h0'/`4\\='sh0' -- or, for a single SYMBOLIC packed
  dimension, a `{WIDTH{1\\='b0}}'-shaped brace expression
  (`verilog-auto--tieoff-constant', shared with AUTOTIEOFF).
- nil: a plain, unsized, untyped `0' for every signal, regardless of
  width.
- `unbased: SystemVerilog's unsized, unbased `\\='0' for every signal --
  not GNU's default, but the recommended setting for a parameterized
  width (this file's own M134 header): it survives a WIDTH change with
  no noise and needs no brace expression at all. `demo/rtl/' writes
  exactly this by hand."
  :type '(choice (const :tag "Sized (GNU default)" t)
                 (const :tag "Unsized 0" nil)
                 (const :tag "Unsized, unbased '0 (SystemVerilog)" unbased))
  :group 'verilog-auto)

(defcustom verilog-auto-reset-blocking-in-non t
  "Whether `/*AUTORESET*/' resets a signal that this always block
assigns with `=' (blocking) even though the block's own dominant idiom
is non-blocking (`<=' used somewhere else in the same always block) --
GNU default `t' (measured, M134 recon). With `t', such a signal is
reset with `=' too, mirroring how it's actually assigned; with nil, it
is excluded from the reset set entirely rather than mixing operators.
Has no effect on a block that uses only `=' throughout, or only `<='
throughout -- there is no \"otherwise\" idiom for either signal to be
an exception to."
  :type 'boolean
  :group 'verilog-auto)

(defvar verilog-auto--autoreset-memory-skips nil
  "Position-ordered notice list (M125 convention, see this file's
\"Position-ordered notice lists\" section) of (MARKER-START . TEXT) for
every unpacked-array (memory) signal `/*AUTORESET*/' skipped rather
than emitting GNU's own illegal `mem <= 8\\='h0;' (divergence 1,
`demo/rtl/core/regfile.sv' is exactly this shape).")

(defvar verilog-auto--autoreset-symbolic-multidim-skips nil
  "Names (plain list, AUTOTIEOFF's own `verilog-auto--tieoff-symbolic-
multidim-skips' convention -- NOT position-ordered) of every signal
`/*AUTORESET*/' skipped because its declared range is multi-dimensional
with at least one symbolic dimension (`verilog-auto--tieoff-constant''s
own `SKIP-REASON', M134 fix round). `logic [WIDTH-1:0][7:0] arr;' is
exactly this shape: measured GNU emits `arr <= 8'h0;' (its own
divergence-5 quirk of silently using only the LAST dimension, already
documented on `verilog-auto--tieoff-constant'); this file refuses
instead, same divergence 5 policy AUTOTIEOFF already applies, rather
than the alternative a cold review caught -- discarding the SKIP-REASON
and emitting a bare `arr <= ;', a syntax error written silently into
the user's file.")

(defun verilog-auto--reset-decl-for-name (module-decl header name)
  "DECL node that declares NAME somewhere in MODULE-DECL, whichever of
the three shapes AUTORESET's own candidates can come from (this file's
M134 header \"Declaration lookup is the real gap\" note): an ANSI port
(`ansi_port_declaration'), a non-ANSI port
(`input_declaration'/`output_declaration'/`inout_declaration'), or a
body `net_declaration'/`data_declaration' (found via its own
`net_decl_assignment'/`variable_decl_assignment' child, walked back up
to the enclosing declaration that actually carries the type/range/
signed information `verilog-auto--decl-raw-type-keyword'/
`verilog-auto--decl-signed-p'/`verilog-auto--all-range-texts' read).
nil if NAME is declared nowhere -- spec section 1's undeclared-signal
case, still reset, as 1 bit.

NOT scoped to module-level declarations only (M134 fix round, item 6):
the `net_decl_assignment'/`variable_decl_assignment' search below walks
EVERY such node anywhere under MODULE-DECL, including inside a `task'/
`function'/`generate' body. A same-named LOCAL inside one of those
would shadow the real module-level signal, first document-order match
winning -- low likelihood (AUTORESET only fires on names actually
assigned in an always block, and a task/function-local variable is
vanishingly unlikely to share a reset signal's name) and left
UNTESTED. Deliberately not scoped further: every existing DECL-node
caller in this file (`--output-port-candidate-decls', `--body-declared-
names') has the identical blanket-search shape, so narrowing only this
one caller would be new, unvalidated surface area for a corner this
project has no measured evidence about either way."
  (or
   (and (verilog-auto--ansi-header-p header)
        (catch 'found
          (dolist (decl (verilog-auto--find-all-of-type header "ansi_port_declaration"))
            (when (equal (treesit-node-text (treesit-node-child-by-field-name decl "port_name")) name)
              (throw 'found decl)))
          nil))
   (catch 'found
     (dolist (kind '("output_declaration" "input_declaration" "inout_declaration"))
       (dolist (decl (verilog-auto--find-all-of-type module-decl kind))
         (let ((idlist (or (verilog-auto--find-first-of-type decl "list_of_port_identifiers")
                            (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers"))))
           (when (and idlist
                      (member name (mapcar #'treesit-node-text
                                            (verilog-auto--find-all-of-type idlist "simple_identifier"))))
             (throw 'found decl)))))
     nil)
   (catch 'found
     (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
       (when (equal (treesit-node-text (treesit-node-child n 0)) name)
         (throw 'found (verilog-auto--enclosing-of-type n "net_declaration"))))
     (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
       (when (equal (treesit-node-text (treesit-node-child-by-field-name n "name")) name)
         (throw 'found (verilog-auto--enclosing-of-type n "data_declaration"))))
     nil)))

(defun verilog-auto--reset-decl-id-node (decl name)
  "The `simple_identifier' node spelling NAME inside DECL -- needed
(unlike every other AUTOREG/AUTOTIEOFF caller of DECL) to find NAME's
own `unpacked_dimension' siblings via `verilog-auto--all-unpacked-
range-texts-after', which takes the identifier node itself, not the
declaration. nil if DECL is nil (NAME undeclared) or, defensively,
if no identifier matching NAME is found in it."
  (and decl
       (catch 'found
         (dolist (id (verilog-auto--find-all-of-type decl "simple_identifier"))
           (when (equal (treesit-node-text id) name)
             (throw 'found id)))
         nil)))

(defun verilog-auto--variable-lvalue-driven-names (lvalue)
  "Base signal name(s) driven by LVALUE, a `variable_lvalue' node (the
left-hand side of a procedural `blocking_assignment'/`nonblocking_
assignment') -- the procedural-LHS mirror of `verilog-auto--net-
lvalue-driven-names' (continuous-assign LHS); same recursion for
concatenation (see that function's own doc string), a SEPARATE function
because the two are different node TYPES in this grammar (M134 recon,
dump-verified) and `verilog-auto--find-all-of-type' matches by exact
type string, so scoping each search to its own type is what keeps them
from cross-matching.

M134 fix round (item 4): a bit-select/part-select LHS (`c[3:0]')
dump-verifies as `variable_lvalue' > `hierarchical_identifier' (ONE
`simple_identifier' child) + a SIBLING `select' node -- so taking the
first `simple_identifier' found anywhere under LVALUE happens to give
the base name there. But a DOTTED hierarchical LHS (`top.inner.sig')
dump-verifies as `variable_lvalue' > `hierarchical_identifier' with
its PATH COMPONENTS as flat, ordered `simple_identifier' children
(`top' `.' `inner' `.' `sig') -- taking the FIRST one there grabs only
`top', the outermost component, not the actual driven signal. Measured
against real GNU Emacs 30.2 (scratchpad/gnu/p7.v): GNU resets
`top.inner.sig' by its own full dotted name, not `top' alone. So this
now branches on the `hierarchical_identifier' child's own
`simple_identifier' COUNT: more than one means a dotted path, and the
whole `hierarchical_identifier' node's own text (the full dotted name)
is the driven name; exactly one is the bit-select/part-select/plain
case, unchanged from before.

M134 fix round 2 (item 2): an ESCAPED identifier LHS (`\\esc+id <=
1\\='b1;', SystemVerilog's `\\NAME ' escape syntax for identifiers
containing characters an ordinary identifier can't) dump-verifies as
`hierarchical_identifier' > `escaped_identifier' with NO `simple_
identifier' descendant at all -- so the plain-case COUNT above is
zero, and without this clause the signal would be silently invisible
to both the candidate scan and the exclusion scan (this predates the
fix round; not a regression, just never covered). Handled here rather
than left as a documented gap, since it's a one-clause fallback: zero
`simple_identifier's under HIER falls through to its own `escaped_
identifier' child, if any, using that node's own text (which already
excludes the terminating whitespace -- dump-verified) as the name."
  (let ((nested (verilog-auto--find-all-of-type lvalue "variable_lvalue")))
    (if nested
        (apply #'append (mapcar #'verilog-auto--variable-lvalue-driven-names nested))
      (let ((hier (verilog-auto--find-first-of-type lvalue "hierarchical_identifier")))
        (if hier
            (let ((ids (verilog-auto--find-all-of-type hier "simple_identifier")))
              (cond
               ((> (length ids) 1) (list (treesit-node-text hier)))
               (ids (list (treesit-node-text (car ids))))
               (t (let ((esc (verilog-auto--find-first-of-type hier "escaped_identifier")))
                    (and esc (list (treesit-node-text esc)))))))
          (let ((id (verilog-auto--find-first-of-type lvalue "simple_identifier")))
            (and id (list (treesit-node-text id)))))))))

(defun verilog-auto--assigned-names-in (node)
  "Alist of (NAME . STYLE) for every procedural assignment anywhere
under NODE (an `always_construct', or any of its own sub-statements) --
STYLE is `nonblocking' for a bare `nonblocking_assignment', `blocking'
for a `blocking_assignment' (M134 recon: which of the two wraps the
assignment is the ONLY signal that matters -- `blocking_assignment'
always wraps `operator_assignment', `nonblocking_assignment' never
does, so the outer node type alone decides; the operator text itself
is never inspected). Descends into every nested `begin'/`end' block,
`case' branch, and `for' loop, unlike `verilog-auto--find-all-of-type'
on \"statement_or_null\" (which deliberately stops at direct children
to find a conditional's own two BRANCHES, not for this assignment
sweep) -- `verilog-auto--find-all-of-types' walks the whole subtree
with no such stop, which is exactly what a sweep for every assignment
anywhere inside NODE needs. The FIRST assignment found for a given
NAME wins its STYLE; this project doesn't invent behaviour for a
signal assigned with both operators in the same always block, since
nothing in spec section 1 measures that shape."
  (let (acc)
    (dolist (n (verilog-auto--find-all-of-types node '("nonblocking_assignment" "blocking_assignment")))
      (let* ((style (if (string= (treesit-node-type n) "nonblocking_assignment") 'nonblocking 'blocking))
             (lvalue (verilog-auto--find-first-of-type n "variable_lvalue")))
        (when lvalue
          (dolist (nm (verilog-auto--variable-lvalue-driven-names lvalue))
            (unless (assoc nm acc)
              (push (cons nm style) acc))))))
    (nreverse acc)))

(defun verilog-auto--assigned-names-before (node cutoff-pos)
  "Like `verilog-auto--assigned-names-in', but keeps only an assignment
whose own node START position is strictly before CUTOFF-POS -- the
marker's own `/*AUTORESET*/' comment start, M134 fix round item 2.

The spec this file was originally built from said the exclusion was
\"the branch the marker sits in\", and the first implementation excluded
every assignment ANYWHERE in that whole branch subtree, regardless of
where it sat relative to the marker. That spec was WRONG: measured
against real GNU Emacs 30.2 (scratchpad/gnu/p1.v vs p2.v, both share
one `if (!rst_n) begin ... end else begin a<=1'b1; b<=1'b1; end' shape,
differing only in whether `a <= 1'b0;' sits BEFORE or AFTER the marker
inside the if-branch), GNU excludes `a' only when the assignment comes
BEFORE the marker in the SAME branch (p2); when it comes AFTER (p1),
GNU resets `a' anyway, right alongside `b'. p3.v confirms nesting depth
is irrelevant to this, only position is: with `if (x) a<=1'b0;' BEFORE
the marker and `if (x) c<=1'b0;' AFTER it, both nested one level deeper
than the marker itself, GNU excludes `a' and keeps `c'. So the
exclusion set is not \"assigned anywhere in the branch\" but \"assigned
before the marker's own text position, anywhere in the branch\" --
this function, not `verilog-auto--assigned-names-in', is what the
expander's own OWN-BRANCH computation must use."
  (let (acc)
    (dolist (n (verilog-auto--find-all-of-types node '("nonblocking_assignment" "blocking_assignment")))
      (when (< (treesit-node-start n) cutoff-pos)
        (let* ((style (if (string= (treesit-node-type n) "nonblocking_assignment") 'nonblocking 'blocking))
               (lvalue (verilog-auto--find-first-of-type n "variable_lvalue")))
          (when lvalue
            (dolist (nm (verilog-auto--variable-lvalue-driven-names lvalue))
              (unless (assoc nm acc)
                (push (cons nm style) acc)))))))
    (nreverse acc)))

(defun verilog-auto--reset-marker-own-branch (comment always)
  "The `statement_or_null' branch of the nearest enclosing
`conditional_statement' that contains COMMENT, stopping the upward walk
at ALWAYS (COMMENT's own enclosing `always_construct') -- nil if
COMMENT sits directly in ALWAYS's body, outside any `if'/`else' at all
\(the nil case, per M134 fix round item 1, is now what makes the
EXPANDER refuse to run at all -- see `verilog-auto--expand-autoreset-
site').

M134 fix round (item 5) CORRECTION: an earlier version of this doc
string, following the spec it was written from, described an `else if'
chain as NESTED `conditional_statement's, with this function walking
into the innermost one. Dump-verified (M134 fix round) that this
grammar does NOT nest an `else if' chain at all -- `if (a) ... else if
(b) ... else if (c) ... else ...' parses as exactly ONE
`conditional_statement' node with N `statement_or_null' children, one
per branch, all flat SIBLINGS (`if' `(' cond `)' branch1 `else' `if'
`(' cond `)' branch2 `else' branch3 ... with no wrapper in between).
The ALGORITHM below is unaffected by the correction -- walking up from
COMMENT to the nearest ancestor whose PARENT is `conditional_statement'
still lands on exactly the one direct-child branch COMMENT sits in,
regardless of how many total branches that conditional_statement has --
only the prose describing the shape was wrong, and is fixed here rather
than in the spec, since this docstring is what the next reader
actually consults."
  (let ((n comment) (found nil))
    (while (and n (not found) (not (eq n always)))
      (let ((parent (treesit-node-parent n)))
        (when (and parent (string= (treesit-node-type parent) "conditional_statement"))
          (setq found n))
        (setq n parent)))
    found))

(defun verilog-auto--reset-constant-for (dims signed)
  "AUTORESET's own reset-constant for DIMS/SIGNED
(`verilog-auto--all-range-texts'/`verilog-auto--decl-signed-p'),
honoring `verilog-auto-reset-widths'. Returns (CONST . SKIP-REASON),
mirroring `verilog-auto--tieoff-constant''s own contract (M134 fix
round item 3): `t' defers to `verilog-auto--tieoff-constant' itself,
SKIP-REASON included unchanged -- a multi-dimensional DIMS with at
least one symbolic dimension comes back as (nil . 'symbolic-multidim),
which the caller MUST check, exactly as AUTOTIEOFF's own call site
already does (`verilog-auto--expand-autotieoff-site'). Discarding
SKIP-REASON here and blindly `concat'ing a nil CONST is what a cold
review caught: `logic [WIDTH-1:0][7:0] arr;' produced the syntax error
`arr <= ;', silently written into the user's file, worse than the skip
divergence 5 already establishes for this exact shape. `nil'/`unbased
never produce a SKIP-REASON -- both are unconditional regardless of
DIMS/SIGNED, so both return `(TEXT . nil)'."
  (cond
   ((eq verilog-auto-reset-widths nil) (cons "0" nil))
   ((eq verilog-auto-reset-widths 'unbased) (cons "'0" nil))
   (t (verilog-auto--tieoff-constant dims signed))))

(defun verilog-auto--reset-decl-line (indent op name const)
  "One AUTORESET assignment line's own full text: INDENT NAME OP CONST;
-- spec section 1's own measured layout has no `// From ...' provenance
comment (unlike AUTOOUTPUT/AUTOINPUT/AUTOINOUT/AUTOTIEOFF/AUTOREG,
every one of which names an instance or a port this file invented the
declaration FOR; an AUTORESET line just restates an assignment the
user's own always block already makes, so there's nothing to attribute
it to)."
  (concat indent name " " op " " const ";"))

(defun verilog-auto--expand-autoreset-site (comment)
  "Expand one /*AUTORESET*/ site. Returns the number of reset
assignments inserted (0 if nothing qualifies -- no Beginning/End
markers, same convention as every other block-style AUTO command).

M134 fix round 2 item 1 CORRECTION: an earlier version of this function
refused to expand at all unless COMMENT sat inside a `conditional_
statement' branch. That gate was itself wrong -- measured against real
GNU Emacs 30.2 (scratchpad/gnu/q1.v: `/*AUTORESET*/' as the FIRST
statement in a bare `always' body, no `if' anywhere, followed by
`a <= 1'b1; b <= 1'b1;'), GNU resets both `a' and `b' there; the gated
version emitted nothing. The two fixtures the gate was built from
(r6.v/r16.v) both happen to have the marker LAST, with nothing assigned
after it anywhere in scope -- \"refuse when there's no conditional\" and
\"the positional rule with OWN SCOPE falling back to the whole always
body\" give the same answer for THOSE two, which is why the gate looked
right until a marker-first fixture (q1.v) and a `for'/`fork'-body
fixture distinguished them.

The single rule, with no gate: OWN SCOPE is COMMENT's enclosing
`conditional_statement' branch (`verilog-auto--reset-marker-own-
branch') if one exists, else the enclosing `always_construct' itself.
A signal is EXCLUDED if it is assigned inside OWN SCOPE, textually
BEFORE the marker's own position (`verilog-auto--assigned-names-
before') -- nesting depth within OWN SCOPE is irrelevant (p3.v), and
critically the cutoff applies ONLY within OWN SCOPE, never across a
SIBLING branch (r8.v: an `if (x) a <= 1'b1;' branch that textually
precedes an `else if' marker branch does NOT exclude `a', because `a''s
assignment is not IN the marker's own branch at all)."
  (let* ((module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl))
         (always (verilog-auto--enclosing-of-type comment "always_construct"))
         (text (treesit-node-text comment)))
    (if (not (string= text "/*AUTORESET*/"))
        (progn
          (push (cons (treesit-node-start comment)
                      (format "AUTORESET(...) in module %s: takes no argument, expansion skipped"
                              (verilog-auto--module-name module-decl)))
                verilog-auto--port-marker-arg-warnings)
          0)
      (if (not always)
          0
        (let* ((own-scope (or (verilog-auto--reset-marker-own-branch comment always) always))
               (all-assigned (verilog-auto--assigned-names-in always))
               ;; M134 fix round 2: OWN-SCOPE is the branch if COMMENT
               ;; has one, else the whole always block -- see this
               ;; function's own doc string. POSITIONAL exclusion within
               ;; it, unchanged from fix round 1 (p1/p2/p3), now also
               ;; covers the marker-first/no-conditional shape (q1/r6/
               ;; r16) for free, since OWN-SCOPE degenerates to ALWAYS
               ;; itself there.
               (own-assigned (verilog-auto--assigned-names-before
                              own-scope (treesit-node-start comment)))
               (own-names (mapcar #'car own-assigned))
               (block-has-nonblocking (verilog-auto--filter
                                       (lambda (e) (eq (cdr e) 'nonblocking))
                                       all-assigned))
               (candidates nil))
          (dolist (e all-assigned)
            (let* ((nm (car e)) (style (cdr e)))
              (unless (member nm own-names)
                (if (and (eq style 'blocking) block-has-nonblocking
                         (not verilog-auto-reset-blocking-in-non))
                    nil
                  (push (cons nm style) candidates)))))
          (setq candidates (nreverse candidates))
          (setq candidates (sort (copy-sequence candidates) (lambda (a b) (string< (car a) (car b)))))
          (let (resolved)
            (dolist (c candidates)
              (let* ((nm (car c)) (style (cdr c))
                     (decl (verilog-auto--reset-decl-for-name module-decl header nm))
                     (id (verilog-auto--reset-decl-id-node decl nm))
                     (unpacked (and id (verilog-auto--all-unpacked-range-texts-after id))))
                (cond
                 (unpacked
                  (unless (verilog-auto--notice-contains-p verilog-auto--autoreset-memory-skips nm)
                    (push (cons (treesit-node-start comment)
                                (format "%s is an unpacked array (memory) -- AUTORESET cannot assign a scalar to it, skipped"
                                        nm))
                          verilog-auto--autoreset-memory-skips)))
                 (t
                  (let* ((dims (and decl (verilog-auto--all-range-texts decl)))
                         (signed (and decl (verilog-auto--decl-signed-p decl)))
                         (cr (verilog-auto--reset-constant-for dims signed))
                         (op (if (eq style 'blocking) "=" "<=")))
                    ;; M134 fix round item 3: SKIP-REASON (CDR) must be
                    ;; checked, exactly as AUTOTIEOFF's own call site
                    ;; does -- discarding it and using a nil CONST
                    ;; produced the syntax error `arr <= ;'.
                    (if (cdr cr)
                        (unless (member nm verilog-auto--autoreset-symbolic-multidim-skips)
                          (push nm verilog-auto--autoreset-symbolic-multidim-skips))
                      (push (list nm op (car cr)) resolved)))))))
            (setq resolved (nreverse resolved))
            (when resolved
              (let ((indent (verilog-auto--line-indent (treesit-node-start comment))))
                (goto-char (treesit-node-end comment))
                (insert
                 "\n" indent "// Beginning of autoreset for uninitialized flops"
                 (mapconcat
                  (lambda (r)
                    (concat "\n" (verilog-auto--reset-decl-line indent (nth 1 r) (nth 0 r) (nth 2 r))))
                  resolved "")
                 "\n" indent "// End of automatics")))
            (length resolved)))))))

(defun verilog-auto--expand-all-autoreset ()
  "Expand EVERY /*AUTORESET*/ site in the current buffer, independently
-- unlike every other block-style AUTO command in this file, which
keeps only the first marker per MODULE, AUTORESET is scoped to its own
enclosing always block (measured GNU behaviour: two always blocks each
with their own `/*AUTORESET*/' both expand -- this file's M134 header).
So there is no per-module `verilog-auto--first-autowire-per-module'
filtering step here at all -- every marker found is its own independent
site."
  (let* ((root (verilog-auto--parse-current-buffer))
         (comments (verilog-auto--find-port-marker-comments root "AUTORESET"))
         (sorted (sort (copy-sequence comments)
                       (lambda (a b) (> (treesit-node-start a) (treesit-node-start b))))))
    (let ((total 0))
      (dolist (c sorted total)
        (setq total (+ total (verilog-auto--expand-autoreset-site c)))))))

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
  (if (verilog-auto--ansi-header-p header)
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

(defun verilog-auto--begin-block-comment-p (node)
  "Non-nil if NODE's own text is a block-style AUTO command's Beginning
line: `// Beginning of automatic ...' (AUTOOUTPUT/AUTOINPUT/AUTOINOUT/
AUTOWIRE/AUTOREG/AUTOTIEOFF, spec section 2.1's shared layout) OR `//
Beginning of autoreset ...' (AUTORESET, M134 -- GNU's own measured text
does NOT share the other six's \"automatic\" spelling, so it needs its
own prefix, not a widened version of theirs)."
  (let ((text (treesit-node-text node)))
    (or (string-prefix-p "// Beginning of automatic" text)
        (string-prefix-p "// Beginning of autoreset" text))))

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
first place, rather than relying on the overlap check alone.

M125: the \"another marker proves my End is missing\" check used to
test literally for `/*AUTOWIRE*/'; generalized to
`verilog-auto--any-auto-port-block-marker-p' (any of the four
block-style markers this file now recognizes) since AUTOOUTPUT
immediately followed by AUTOINPUT immediately followed by AUTOINOUT is
now a NORMAL, expected shape in one module, not a corrupted one -- see
this file's M125 header for the full story.

M134: the literal prefix check below is generalized to
`verilog-auto--begin-block-comment-p', because AUTORESET's own
Beginning line (GNU-measured, this file's M134 header: \"// Beginning
of autoreset for uninitialized flops\") does NOT share the \"//
Beginning of automatic\" prefix every other block-style marker's own
Beginning line does -- a bare `string-prefix-p' check here would never
recognize an AUTORESET site's own Beginning line at all, silently
treating it as ordinary buffer text rather than proof of a stale
range."
  (let ((next (verilog-auto--next-sibling comment)))
    (when (and next
               (string= (treesit-node-type next) "one_line_comment")
               (verilog-auto--begin-block-comment-p next))
      (let ((n (verilog-auto--next-sibling next)) (found nil) (blocked nil))
        (while (and n (not found) (not blocked))
          (cond
           ((and (string= (treesit-node-type n) "one_line_comment")
                 (string= (treesit-node-text n) "// End of automatics"))
            (setq found (treesit-node-end n)))
           ((or (verilog-auto--any-auto-port-block-marker-p n)
                (and (string= (treesit-node-type n) "one_line_comment")
                     (verilog-auto--begin-block-comment-p n)))
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

(defun verilog-auto--trailing-templated-annotation-range (close)
  "If the physical line containing CLOSE's (a `hierarchical_instance's
own closing paren) end position ends with a `// Templated...' comment
\(M127's own annotation for the LAST connection of a templated
`/*AUTOINST*/', inserted PAST this hier's own closing paren -- section
2.4's own measured GNU placement, `verilog-auto--expand-autoinst-site'
above), return (START . END) -- START right before the padding spaces
that precede it, END the end of that physical line -- so
`verilog-delete-auto' can strip it back out as a SECOND, independent
range alongside the ordinary ports-region one; nil if this line carries
no such trailing annotation at all. Recognizes every
`verilog-auto-inst-template-numbers' shape (`// Templated', or
`// Templated LHS: ...' -- never `t', which behaves as the bare form).

Known, accepted limitation (recorded, not fixed, per a cold review):
this function does not check whether THIS instantiation was actually
templated -- it only pattern-matches the trailing text. A user who
hand-writes a comment reading exactly `// Templated' (or `// Templated
LHS: ...') on an AUTOINST closing-paren line will have it silently
deleted by `verilog-delete-auto' even though nothing here generated it.
Accepted as the same class of behavior as GNU deleting its own
generated comments: a hand-written string that is textually identical
to the generator's own output is indistinguishable from it, and
requiring some other marker (a hidden property, a different comment
shape) would be a bigger change than this gap is worth."
  (verilog-auto--trailing-templated-annotation-range-at (treesit-node-end close)))

(defun verilog-auto--trailing-templated-annotation-range-at (pos)
  "Position-taking core of `verilog-auto--trailing-templated-annotation-
range' (M129 refactor -- see that function's own doc string for the
regexp and the exact range returned; both callers share this one
regexp, never duplicated). POS is a buffer position: the ordinary
tree-based caller passes a `hierarchical_instance's own closing paren's
END (`(treesit-node-end close)'); M129's text-based fallback path
\(`verilog-delete-auto') passes the position right past a TEXT-fallback
close-paren index instead, since there is no tree node there to ask
for one."
  (save-excursion
    (goto-char pos)
    (let* ((start (point)) (eol (line-end-position))
           (tail (buffer-substring start eol)))
      (when (string-match "[ \t]+// Templated\\(?: LHS: .*\\)?\\'" tail)
        (cons (+ start (match-beginning 0)) eol)))))

(defun verilog-auto--unreachable-autoinst-markers (root)
  "Every `/*AUTOINST*/' marker `block_comment' in ROOT that
`verilog-delete-auto' cannot locate through its own tree-based path
\(M128, spec section 6b) -- a KIND-based search
\(`verilog-auto--find-comments', this file's own top-of-file header)
finds every SUCH marker anywhere in the tree irrespective of its
ancestor shape; a marker counts as unreachable here when it has no
`hierarchical_instance' ancestor at all, which is exactly the
condition `verilog-delete-auto's own `(dolist (mi ... \"module_
instantiation\"))' walk depends on to ever find it.

Compared by POSITION (`treesit-node-start'), not `eq'/`memq' on the
node objects themselves -- a fresh tree walk from ROOT and a walk
scoped to one already-found `module_instantiation' subtree return
their own, separately-constructed node wrappers even for the SAME
underlying position, so an identity-based comparison would silently
never match and report every marker as unreachable.

This function only OBSERVES an absent ancestor -- it does not, and
cannot, verify WHY one is absent, so its own caller's message must not
assert a cause it never checked (fix-round finding: a stray, hand-
written `/*AUTOINST*/' comment that simply isn't inside any
instantiation at all produces the EXACT SAME observation -- no
`hierarchical_instance' ancestor -- and would be misreported as \"a
parse error\" if the message asserted that unconditionally). Two known
shapes produce this, and there may be others this file has not
measured:
- A KNOWN, narrow parse-error cause (M128 recon, not re-derived from
  GNU): a generated connection whose text contains a COMPLETE string
  literal immediately followed by a bracketed range (`.data
  (\"W\"[7:0])'), or an UNTERMINATED string literal, makes tree-sitter's
  GLR recovery reclassify the WHOLE enclosing statement -- no
  `module_instantiation'/`hierarchical_instance' node is produced for it
  anywhere in the tree. Notably, section 2.5's own successful-eval path
  can MANUFACTURE this shape too: a `@\"(lisp-expr)\"' token that
  evaluates successfully to a string containing a literal `\"' adjacent
  to a `[]'/`[][]' token in the same rule splices out exactly this
  shape, with no error anywhere in Part A -- pinned by
  `lisp_result_embedding_a_quote_adjacent_to_bracket_becomes_a_part_b_unreachable_site'
  in verilog_auto_tests.rs.
- A genuinely STRAY marker comment, hand-written or left over some
  other way, that was never inside a `hierarchical_instance' to begin
  with -- pinned by `unreachable_autoinst_stray_marker_with_no_enclosing_instantiation'.
M128 shipped the MINIMUM HONEST fix here: this function only detects
and reports the observation, and every site it names stayed
permanently stuck expanded. M129 CORRECTION: `verilog-delete-auto'
itself (not this function -- this function still only observes) now
runs every marker this function names through a second, text-based
fallback path (`verilog-auto--lex-paren-pairs' /
`verilog-auto--text-fallback-range') and deletes there too when that
path can. A site named by THIS function can still end up unrecovered
after that attempt in exactly three cases: the candidate region is
lexically broken (an unterminated string or an unterminated block
comment before the enclosing pair's own close), no enclosing paren
pair exists for the marker at all, or `verilog-auto--instantiation-
shaped-p' refuses the enclosing pair's own preceding tokens. See this
file's own M129 top-of-file header correction for why that fallback is
this file's one documented exception to searching by node KIND."
  (let ((reachable-starts
         (let (acc)
           (dolist (mi (verilog-auto--find-all-of-type root "module_instantiation"))
             (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
               (when c (push (treesit-node-start c) acc))))
           acc)))
    (verilog-auto--filter
     (lambda (c) (not (member (treesit-node-start c) reachable-starts)))
     (verilog-auto--find-comments root "/*AUTOINST*/"))))

;; --- M129: text-based fallback scanner for tree-unreachable AUTOINST ----
;;
;; Everything from here to `verilog-delete-auto' below exists ONLY to
;; rescue `/*AUTOINST*/' sites `verilog-auto--unreachable-autoinst-
;; markers' names -- normal `verilog-delete-auto' operation on a clean
;; buffer never calls any of it, and `verilog-delete-auto' below only
;; runs the lexical pass at all when that list is non-empty. See this
;; file's own M129 top-of-file header correction for why a raw-text
;; scanner is this file's one documented exception to "search by node
;; KIND, never scan raw text past a node boundary", and GNU Emacs
;; 30.2's own measured behaviour (M129 spec sections 1a-1d) for the
;; reference this whole design is checked against.
;;
;; Index convention: 0-based indices into ONE `(buffer-substring-no-
;; properties (point-min) (point-max))' snapshot, the same convention
;; `verilog-auto--trailing-templated-annotation-range-at' already uses
;; (`(+ start (match-beginning 0))'-style); a caller converts to a
;; buffer position with `(+ (point-min) IDX)'.

(defun verilog-auto--fallback-ident-start-p (c)
  "Non-nil if character C can start a Verilog identifier (`[A-Za-z_]')."
  (or (and (>= c ?a) (<= c ?z))
      (and (>= c ?A) (<= c ?Z))
      (eq c ?_)))

(defun verilog-auto--fallback-ident-char-p (c)
  "Non-nil if character C can continue a Verilog identifier
\(`[A-Za-z0-9_$]')."
  (or (verilog-auto--fallback-ident-start-p c)
      (and (>= c ?0) (<= c ?9))
      (eq c ?$)))

(defun verilog-auto--fallback-number-char-p (c)
  "Non-nil if character C continues a number/based-literal token once
one has started -- deliberately generous (digits, letters, `_', `$',
an apostrophe, `.') since a based literal's own radix letter
(`4'b1010'), size prefix, and a real number's fractional/exponent
part all need to
survive as ONE token; this token's own TEXT is never inspected by
`verilog-auto--instantiation-shaped-p' (only plain/escaped identifiers
ever pass its guard), so over-consuming here costs nothing."
  (or (verilog-auto--fallback-ident-char-p c)
      (eq c ?\') (eq c ?.)))

(defun verilog-auto--fallback-push-token (tok tokens)
  "TOK pushed onto TOKENS (a `verilog-auto--lex-paren-pairs' token list,
most-recent-first), trimmed back down to at most 6 entries. Builds a
FRESH list rather than mutating TOKENS -- TOKENS's own tail may already
be referenced as an earlier, still-open pair's own recorded snapshot
\(`verilog-auto--lex-paren-pairs' below snapshots the current token
list by reference, not by copy, every time a `(' opens), and `nreverse'
or `nconc' on it would corrupt that snapshot out from under it."
  (let ((lst (cons tok tokens)) (kept 0) (acc nil))
    (while (and lst (< kept 6))
      (push (car lst) acc)
      (setq lst (cdr lst) kept (1+ kept)))
    (nreverse acc)))

(defun verilog-auto--fallback-skip-escaped-identifier (text i n)
  "TEXT's own escaped identifier starting at I (I is the leading
backslash itself), 0-based, N = (length TEXT). Returns the index right
past it -- the next whitespace character, or N. Shared by `verilog-auto--
lex-paren-pairs's own top-level scan and its bracket-group sub-scan (fix
round, cold review: those two used to duplicate this rule, and the
bracket sub-scan's own copy was simply missing) -- ONE lexing
discipline, not two."
  (let ((j (1+ i)))
    (while (and (< j n) (not (memq (aref text j) '(?\s ?\t ?\n ?\r))))
      (setq j (1+ j)))
    j))

(defun verilog-auto--fallback-skip-string (text i n)
  "TEXT's own string literal starting at I (I is the opening `\"'),
0-based, N = (length TEXT). Returns (NEW-I . ERROR-IDX-OR-NIL): on a
properly closed string, NEW-I is right past the closing `\"' and
ERROR-IDX-OR-NIL is nil; on a newline or end-of-text reached first
\(unterminated), NEW-I is the newline's own index (or N) and
ERROR-IDX-OR-NIL is I itself -- the CALLER decides whether to actually
record that as `:lex-error' (typically only if one isn't already set).
`\\X' inside the string consumes both characters, so `\\\"' never ends
it early. Shared by the top-level scan and the bracket-group sub-scan
(fix round, cold review) -- ONE lexing discipline, not two."
  (let ((j (1+ i)) (closed nil) (bad nil))
    (while (and (< j n) (not closed) (not bad))
      (let ((cj (aref text j)))
        (cond
         ((eq cj ?\\) (setq j (+ j 2)))
         ((eq cj ?\") (setq closed t))
         ((eq cj ?\n) (setq bad t))
         (t (setq j (1+ j))))))
    (cond
     (closed (cons (1+ j) nil))
     (bad (cons j i))
     (t (cons n i)))))

(defun verilog-auto--fallback-skip-line-comment (text i n)
  "TEXT's own `//' line comment starting at I (I is the first `/'),
0-based, N = (length TEXT). Returns the index right past it -- the next
newline, or N; never an error. Shared by the top-level scan and the
bracket-group sub-scan (fix round, cold review) -- ONE lexing
discipline, not two."
  (or (string-search "\n" text i) n))

(defun verilog-auto--fallback-skip-block-comment (text i n)
  "TEXT's own `/*...*/' block comment starting at I (I is the `/' of
`/*'), 0-based, N = (length TEXT). Returns (NEW-I . ERROR-IDX-OR-NIL):
on a `*/' found, NEW-I is right past it and ERROR-IDX-OR-NIL is nil; on
end-of-text reached first (unterminated -- Verilog block comments do
not nest), NEW-I is N and ERROR-IDX-OR-NIL is I. Shared by the
top-level scan and the bracket-group sub-scan (fix round, cold review)
-- ONE lexing discipline, not two."
  (let ((k (string-search "*/" text (+ i 2))))
    (if k (cons (+ k 2) nil) (cons n i))))

(defun verilog-auto--lex-paren-pairs (text)
  "One forward, O(n), string/comment/escaped-identifier-aware lexical
pass over TEXT (0-based char indices). Returns a plist `(:pairs PAIRS
:lex-error IDX-OR-NIL)'.

PAIRS is a list of records, one per `(' encountered at any depth, each
`(OPEN CLOSE . TOKENS)': OPEN the `('s own 0-based index; CLOSE the
matching `)'s own 0-based index, or nil if it was never closed by
end-of-text; TOKENS the up-to-6 significant tokens immediately
preceding OPEN, most-recent-first (so for `leaf u1 (', TOKENS is
`(\"u1\" \"leaf\")' -- read `verilog-auto--instantiation-shaped-p's own
doc string alongside this one; its two shapes are written out in
SOURCE order, which is why it walks TOKENS by dropping from the front).

Lexer rules (six, all pinned by the M129 spec's own GNU measurements,
section 1b):
- A leading backslash starts an ESCAPED IDENTIFIER, which runs to the
  next whitespace character verbatim -- a legal Verilog escaped
  identifier can itself contain `(', `)', `\"', `/' (`\\u1(0) ' is a
  legal instance name in a real gate-level netlist), so this check
  must win over every other branch below, including inside what would
  otherwise look like the start of a string or a comment.
- `\"' enters a STRING: ends at the next UNESCAPED `\"' (`\\X' inside a
  string consumes both characters, so `\\\"' never ends it early); a
  newline or end-of-text before the closing `\"' is a LEXICAL ERROR --
  :LEX-ERROR is set to the OPENING `\"'s own index (only the first such
  error in the whole pass is kept), and lexing resumes in normal state
  at the newline (or stops at end-of-text) so the rest of the file
  still lexes.
- `//' enters a LINE COMMENT: ends at the next newline, or end-of-text
  -- never an error.
- `/*' enters a BLOCK COMMENT: ends at the next `*/'. End-of-text first
  is a LEXICAL ERROR at the `/*'s own index (Verilog block comments do
  not nest -- this scanner does not implement nesting).
- `(' / `)' push/pop a paren-depth stack, producing one PAIRS record
  per `('; a `)' with nothing on the stack (a stray close) produces no
  record and is otherwise ignored.
- `[' / `]' are tracked as a BRACKET GROUP (nesting counted) so a whole
  `[3:0]'/`[1:0][7:0]' becomes the single token `\"[]\"'. An unbalanced
  `[' (no matching `]' before end-of-text) is not one of this
  function's named lexical-error shapes -- it falls back to the
  catch-all single-character-token rule below instead of aborting.

Token rules (normal state only -- a token is never produced from
inside a string or a comment): a completed identifier or escaped
identifier (recorded verbatim including its own leading backslash); a
number/based literal (`verilog-auto--fallback-number-char-p'); a
completed parenthesised group records the single token `\"()\"'; a
completed bracket group records the single token `\"[]\"'; any other
non-whitespace character (`#', `.', `;', `@', a quote character, `=',
`,', ...) records ITSELF as a one-character token."
  (let ((n (length text))
        (i 0)
        (pairs nil)
        (open-stack nil)
        (recent nil)
        (lex-error nil))
    (while (< i n)
      (let ((c (aref text i)))
        (cond
         ;; Escaped identifier -- must win over every other branch: it
         ;; can legally contain `(', `)', `"', `/'.
         ((eq c ?\\)
          (let ((j (verilog-auto--fallback-skip-escaped-identifier text i n)))
            (setq recent (verilog-auto--fallback-push-token (substring text i j) recent))
            (setq i j)))
         ;; String literal.
         ((eq c ?\")
          (let ((r (verilog-auto--fallback-skip-string text i n)))
            (when (cdr r) (unless lex-error (setq lex-error (cdr r))))
            (setq i (car r))))
         ;; Line comment.
         ((and (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?/))
          (setq i (verilog-auto--fallback-skip-line-comment text i n)))
         ;; Block comment.
         ((and (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?*))
          (let ((r (verilog-auto--fallback-skip-block-comment text i n)))
            (when (cdr r) (unless lex-error (setq lex-error (cdr r))))
            (setq i (car r))))
         ;; Open paren: snapshot the current (enclosing-scope) token
         ;; stream as this pair's own preceding tokens, then RESET
         ;; `recent' to a fresh, empty accumulator for whatever tokens
         ;; get produced INSIDE this new pair -- those belong to a
         ;; deeper nesting level and must never leak into the enclosing
         ;; scope's own token stream (they get discarded wholesale when
         ;; this pair closes below, replaced by a single `"()"' token
         ;; instead). Without this reset, `.W(8)' inside `leaf #(.W(8))
         ;; u1 (' would leave `\"8\"'/`\".\"'/`\"W\"' sitting in `recent'
         ;; ahead of the `"()"' token this pair's own close is about to
         ;; push, corrupting the OUTER `#(...)' pair's own TOKENS.
         ((eq c ?\()
          (push (cons i recent) open-stack)
          (setq recent nil)
          (setq i (1+ i)))
         ;; Close paren: pop the innermost still-open pair (if any),
         ;; complete its record, restore the ENCLOSING scope's own
         ;; token stream (discarding whatever accumulated inside this
         ;; pair), and feed a single `"()"' token back into it.
         ((eq c ?\))
          (when open-stack
            (let ((rec (pop open-stack)))
              (setq recent (cdr rec))
              (push (cons (car rec) (cons i (cdr rec))) pairs)
              (setq recent (verilog-auto--fallback-push-token "()" recent))))
          (setq i (1+ i)))
         ;; Bracket group -- honours the SAME string / line-comment /
         ;; block-comment / escaped-identifier sub-lexing as the
         ;; top-level scan above, via the same `verilog-auto--fallback-
         ;; skip-*' helpers (fix round, cold review: this sub-scan used
         ;; to count `['/`]' over raw text only, so a `]' inside e.g. a
         ;; string INSIDE a bracket group ended the group early -- one
         ;; lexing discipline now, not two).
         ((eq c ?\[)
          (let ((j (1+ i)) (depth 1) (bad nil))
            (while (and (< j n) (> depth 0) (not bad))
              (let ((cj (aref text j)))
                (cond
                 ((eq cj ?\\)
                  (setq j (verilog-auto--fallback-skip-escaped-identifier text j n)))
                 ((eq cj ?\")
                  (let ((r (verilog-auto--fallback-skip-string text j n)))
                    (when (cdr r)
                      (unless lex-error (setq lex-error (cdr r)))
                      (setq bad t))
                    (setq j (car r))))
                 ((and (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?/))
                  (setq j (verilog-auto--fallback-skip-line-comment text j n)))
                 ((and (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?*))
                  (let ((r (verilog-auto--fallback-skip-block-comment text j n)))
                    (when (cdr r)
                      (unless lex-error (setq lex-error (cdr r)))
                      (setq bad t))
                    (setq j (car r))))
                 ((eq cj ?\[) (setq depth (1+ depth)) (setq j (1+ j)))
                 ((eq cj ?\]) (setq depth (1- depth)) (setq j (1+ j)))
                 (t (setq j (1+ j))))))
            (if (and (= depth 0) (not bad))
                (progn
                  (setq recent (verilog-auto--fallback-push-token "[]" recent))
                  (setq i j))
              ;; Unbalanced `[', or a lexical anomaly (an unterminated
              ;; string/comment) surfaced while scanning inside it --
              ;; neither is a named top-level lexical-error shape by
              ;; itself; falls back to the catch-all single-char-token
              ;; rule, same as an ordinary unbalanced `['. `lex-error'
              ;; (if `bad' set it above) still propagates normally --
              ;; the rest of the buffer gets re-lexed from I+1 onward by
              ;; the outer loop, which re-encounters and handles the
              ;; same broken construct itself.
              (setq recent (verilog-auto--fallback-push-token "[" recent))
              (setq i (1+ i)))))
         ;; Whitespace: no token.
         ((memq c '(?\s ?\t ?\n ?\r))
          (setq i (1+ i)))
         ;; Identifier.
         ((verilog-auto--fallback-ident-start-p c)
          (let ((j (1+ i)))
            (while (and (< j n) (verilog-auto--fallback-ident-char-p (aref text j)))
              (setq j (1+ j)))
            (setq recent (verilog-auto--fallback-push-token (substring text i j) recent))
            (setq i j)))
         ;; Number / based literal.
         ((and (>= c ?0) (<= c ?9))
          (let ((j (1+ i)))
            (while (and (< j n) (verilog-auto--fallback-number-char-p (aref text j)))
              (setq j (1+ j)))
            (setq recent (verilog-auto--fallback-push-token (substring text i j) recent))
            (setq i j)))
         ;; Catch-all: any other non-whitespace character is its own
         ;; one-character token.
         (t
          (setq recent (verilog-auto--fallback-push-token (char-to-string c) recent))
          (setq i (1+ i))))))
    ;; Any `(' still on the stack at end-of-text never closed.
    (dolist (rec open-stack)
      (push (cons (car rec) (cons nil (cdr rec))) pairs))
    (list :pairs pairs :lex-error lex-error)))

(defconst verilog-auto--fallback-keywords
  '("module" "macromodule" "endmodule" "interface" "endinterface" "program" "endprogram"
    "package" "endpackage" "class" "endclass" "checker" "endchecker" "primitive"
    "endprimitive" "config" "endconfig" "function" "endfunction" "task" "endtask"
    "generate" "endgenerate" "specify" "endspecify" "table" "endtable" "property"
    "endproperty" "sequence" "endsequence" "covergroup" "endgroup" "clocking"
    "endclocking" "modport" "extern" "virtual" "pure" "static" "automatic" "local"
    "protected" "import" "export" "typedef" "parameter" "localparam" "defparam" "genvar"
    "const" "ref" "var" "input" "output" "inout" "wire" "reg" "logic" "bit" "byte" "int"
    "integer" "shortint" "longint" "time" "real" "realtime" "shortreal" "void" "string"
    "event" "chandle" "tri" "triand" "trior" "tri0" "tri1" "wand" "wor" "supply0"
    "supply1" "uwire" "signed" "unsigned" "enum" "struct" "union" "packed" "if" "else"
    "for" "while" "do" "repeat" "forever" "foreach" "case" "casex" "casez" "randcase"
    "endcase" "begin" "end" "fork" "join" "join_any" "join_none" "return" "break"
    "continue" "disable" "wait" "always" "always_comb" "always_ff" "always_latch"
    "initial" "final" "assign" "assert" "assume" "cover" "expect" "restrict" "posedge"
    "negedge" "edge" "new" "super" "this" "with" "inside" "dist" "solve" "before"
    "unique" "unique0" "priority" "randomize" "constraint" "coverpoint" "cross" "bins"
    "ignore_bins" "illegal_bins" "wildcard" "force" "release" "deassign" "and" "or"
    "not" "nand" "nor" "xor" "xnor" "buf" "bufif0" "bufif1" "notif0" "notif1" "pullup"
    "pulldown" "cmos" "nmos" "pmos" "rcmos" "rnmos" "rpmos" "tran" "tranif0" "tranif1"
    "rtran" "rtranif0" "rtranif1" "let" "type" "specparam" "trireg" "rand" "randc"
    "randsequence" "nettype" "bind" "alias" "default" "iff" "ifnone" "matches" "tagged"
    "soft" "until" "until_with" "within" "instance" "context" "cell" "design" "liblist"
    "library" "use" "null" "global" "eventually" "first_match" "implies" "implements"
    "intersect" "throughout" "nexttime" "s_always" "s_eventually" "s_nexttime" "s_until"
    "s_until_with" "sync_accept_on" "sync_reject_on" "reject_on" "pull0" "pull1"
    "pulsestyle_ondetect" "pulsestyle_onevent" "scalared" "vectored" "weak" "weak0"
    "weak1" "strong" "strong0" "strong1" "showcancelled" "noshowcancelled" "timeunit"
    "timeprecision" "untyped" "extends" "unpacked" "highz0" "highz1" "small" "medium"
    "large" "accept_on" "interconnect" "wait_order" "binsof")
  "Verilog/SystemVerilog reserved words `verilog-auto--instantiation-
shaped-p' (M129) refuses as either half of a candidate `MODULE INSTANCE
(' or `MODULE #(...) INSTANCE (' shape. This is a POSITIVE WHITELIST of
exactly those two shapes, never a blacklist, and this list exists to
make the whitelist's own two identifier slots conservative -- fix round
(cold review) widened this from an initial ~140-word list to IEEE
1800-2017's own full reserved-word set (Annex B), after the reviewer
found a concrete false accept: `let NAME(args) = expr;' tokenizes as
`\"let\" \"NAME\" \"(\"', and `let' was absent, so the guard accepted it
as shape 1. A FALSE
ACCEPT here silently deletes real hand-written text -- a module's own
port list is the worst case -- mistaking it for a tree-unreachable
AUTOINST site's generated connections, which is DATA CORRUPTION; a
FALSE REJECT only costs one stuck site staying reported as unrecovered
instead of rescued, i.e. exactly M128's own prior behaviour for that
site. That asymmetry is why this list errs toward including a word
whenever in doubt, and why the guard is shaped as a whitelist of two
shapes at all rather than trying to enumerate every way real Verilog
text can fail to look like an instantiation.

Accepted, NAMED limitation (fix round, cold review -- recorded here so
the next reader does not rediscover it as a defect): a GATE-LEVEL
PRIMITIVE instantiation (`and u1 (out, a, b);', `nand'/`buf'/`bufif0'/
`tranif1'/... -- IEEE 1800-2017's own primitive-gate keyword set, every
one of them on THIS list too) can never be rescued by this fallback,
because the primitive's own keyword occupies the exact slot `verilog-
auto--instantiation-shaped-p' checks for the MODULE name, and every
gate keyword is deliberately on this blacklist. There is no way to
special-case this without weakening the guard for everything else
built on the same two-token shape, and a real gate-level netlist's own
primitive instantiations are exactly the kind of large, mechanically-
generated text where a false accept would be most costly. A stuck gate
primitive site stays reported as unrecovered, same as before M129.")

(defun verilog-auto--fallback-plain-ident-p (tok)
  "TOK (a `verilog-auto--lex-paren-pairs' token, or nil) is a plain
identifier, or an escaped identifier (leading backslash, which is
never a keyword -- GNU-legally it cannot collide with a reserved
word), and not
a member of `verilog-auto--fallback-keywords'."
  (and (stringp tok)
       (> (length tok) 0)
       (or (eq (aref tok 0) ?\\)
           (and (verilog-auto--fallback-ident-start-p (aref tok 0))
                (not (member tok verilog-auto--fallback-keywords))))))

(defun verilog-auto--instantiation-shaped-p (tokens)
  "Non-nil only when TOKENS (a `verilog-auto--lex-paren-pairs' record's
own TOKENS, most-recent-first) matches one of exactly two shapes,
written here in SOURCE order (TOKENS itself is walked front-to-back,
i.e. nearest-the-paren first, which is the REVERSE of source order):

    IDENT_MODULE  IDENT_INSTANCE  [\"[]\"]  (
    IDENT_MODULE  \"#\"  \"()\"  IDENT_INSTANCE  [\"[]\"]  (

An optional leading `\"[]\"' (an instance-array bracket, `u1[3:0] (') is
dropped first if present; the next token must be a plain, non-keyword
identifier (`verilog-auto--fallback-plain-ident-p') -- the instance
name; then EITHER the following token is itself a plain non-keyword
identifier (shape 1, the module name), OR it is `\"()\"' followed by
`\"#\"' followed by a plain non-keyword identifier (shape 2, a
parameterised instantiation's own module name past its `#(...)' list).
Anything else, including TOKENS being too short to even try, is nil."
  (let ((toks tokens))
    (when (and toks (stringp (car toks)) (string= (car toks) "[]"))
      (setq toks (cdr toks)))
    (and toks
         (verilog-auto--fallback-plain-ident-p (car toks))
         (let ((rest (cdr toks)))
           (and rest
                (or (verilog-auto--fallback-plain-ident-p (car rest))
                    (and (stringp (car rest)) (string= (car rest) "()")
                         (cdr rest) (stringp (cadr rest)) (string= (cadr rest) "#")
                         (cddr rest)
                         (verilog-auto--fallback-plain-ident-p (car (cddr rest))))))))))

(defun verilog-auto--text-fallback-range (lex marker-end-idx)
  "Given LEX (the plist `verilog-auto--lex-paren-pairs' returns) and
MARKER-END-IDX (an AUTOINST marker comment's own 0-based END index in
the same text LEX was built from), return (START . CLOSE) -- both
0-based indices, START always MARKER-END-IDX itself -- for the region
`verilog-delete-auto' should delete via this text-based fallback path,
or nil if this marker cannot be rescued this way.

1. Find the INNERMOST pair in LEX's own :PAIRS with a non-nil CLOSE
   enclosing MARKER-END-IDX (OPEN < MARKER-END-IDX <= CLOSE);
   \"innermost\" means the LARGEST such OPEN. No such pair -> nil.
2. If :LEX-ERROR is non-nil and strictly before that pair's own CLOSE,
   the text from the lexer's first anomaly onward is untrustworthy ->
   nil. (A pair that closes BEFORE the anomaly is fine -- the anomaly
   never touched it.)
3. If the pair's own TOKENS don't pass `verilog-auto--instantiation-
   shaped-p' -> nil.
4. Otherwise (MARKER-END-IDX . CLOSE)."
  (let ((lex-error (plist-get lex :lex-error))
        (best nil))
    (dolist (rec (plist-get lex :pairs))
      (let ((open (nth 0 rec)) (close (nth 1 rec)))
        (when (and (< open marker-end-idx)
                   close
                   (>= close marker-end-idx)
                   (or (not best) (> open (nth 0 best))))
          (setq best rec))))
    (cond
     ((not best) nil)
     ((and lex-error (< lex-error (nth 1 best))) nil)
     ((not (verilog-auto--instantiation-shaped-p (cddr best))) nil)
     (t (cons marker-end-idx (nth 1 best))))))

(defun verilog-delete-auto ()
  "Delete every AUTOINST/AUTOWIRE/AUTOARG/AUTOOUTPUT/AUTOINPUT/AUTOINOUT/
AUTOREG/AUTOTIEOFF machine-generated region in the current buffer,
leaving the AUTO comments themselves untouched:
- AUTOINST: from right after the /*AUTOINST*/ comment through (but not
  including) its own instantiation's closing paren, PLUS (M127) any
  trailing `// Templated...' annotation sitting past that closing
  paren on the same physical line (`verilog-auto--trailing-templated-
  annotation-range' -- a SEPARATE range, since the closing paren itself
  sits between the two and must never be deleted).
- AUTOARG: from right after the /*AUTOARG*/ comment through (but not
  including) its own header's closing paren.
- AUTOWIRE/AUTOOUTPUT/AUTOINPUT/AUTOINOUT/AUTOREG/AUTOTIEOFF: from right
  after the marker comment through the end of a following \"// Beginning
  of automatic\" .. \"// End of automatics\" block, if one is there
  (M125: generalized from AUTOWIRE alone to four block-style markers;
  M126: widened to six).
Always ends the whole deletion as one undo group (`undo-amalgamate-
boundary'). Returns (DELETED-COUNT OVERLAP-SKIPPED-COUNT UNREACHABLE-
AUTOINST-UNRECOVERED-COUNT TEXT-RECOVERED-COUNT) -- a 4-element list
\(M129 WIDENED this from M128's 3-element shape; every pre-M129 caller
only ever inspected it via `nth'/`cdr'/`car', never destructured a
fixed-length tuple, so this too is additive):
- OVERLAP-SKIPPED-COUNT counts ranges withheld because they overlapped
  another (`verilog-auto--overlapping-ranges' -- normal operation never
  produces this).
- UNREACHABLE-AUTOINST-UNRECOVERED-COUNT (M128, spec section 6b; M129
  NARROWED its own meaning -- see below) counts `/*AUTOINST*/' marker
  comments that exist in the buffer but have no enclosing
  `hierarchical_instance' at all, so this function's own tree-based
  path can never reach them (see `verilog-auto--unreachable-autoinst-
  markers'), AND that M129's own text-based fallback scan
  (`verilog-auto--lex-paren-pairs' / `verilog-auto--text-fallback-
  range') could ALSO not recover -- the text itself was lexically
  broken (an unterminated string or an unterminated block comment
  before the enclosing pair's own close), no enclosing paren pair
  existed for the marker at all, or `verilog-auto--instantiation-
  shaped-p' refused the enclosing pair's own preceding tokens. Before
  M129 this counted every tree-unreachable marker; after M129 it counts
  only the ones text recovery ALSO failed on -- a marker text recovery
  DID handle instead counts toward TEXT-RECOVERED-COUNT and is no
  longer in this count at all. This function only OBSERVES the absent
  ancestor; it does not verify why one is absent -- a parse error
  reclassifying the enclosing statement is the KNOWN, narrow cause this
  project has measured (including one Part A of AUTO_TEMPLATE can
  itself manufacture through ordinary success, see that function's own
  doc string), but a genuinely stray marker comment that was simply
  never inside an instantiation produces the identical observation, so
  the message states what was seen, not an asserted cause. Either way
  these sites are left untouched, PERMANENTLY stuck in their expanded
  form (or, for a stray comment, simply never touched), and (unlike the
  overlap case) there's nothing to retry: no `module_instantiation'
  node exists for them to try again against, and the text-based path
  already had its one attempt.
- TEXT-RECOVERED-COUNT (M129, new) counts `/*AUTOINST*/' sites that had
  no enclosing `hierarchical_instance' but WERE recovered and deleted
  by the text-based fallback scan -- see this file's own M129
  top-of-file header correction for why this scan exists at all and
  what it is confined to.
Echoes a warning when any of the three non-DELETED-COUNT counts is
nonzero; when called from `verilog-auto' all three get folded into its
own final message instead \(this one would otherwise just be invisibly
clobbered by the phases that run afterward)."
  (interactive)
  (let* ((root (verilog-auto--parse-current-buffer))
         (ranges nil)
         (unreachable (verilog-auto--unreachable-autoinst-markers root))
         ;; M129: the lexical pass is the only part of this whole
         ;; function that costs anything beyond the tree walk, so it
         ;; runs AT MOST ONCE, and only when there is actually an
         ;; unreachable marker to try rescuing -- the normal (no
         ;; tree-unreachable site) path pays nothing extra at all.
         (fallback-lex
          (and unreachable
               (verilog-auto--lex-paren-pairs
                (buffer-substring-no-properties (point-min) (point-max)))))
         (fallback-ranges nil)
         (unrecovered nil))
    (dolist (c unreachable)
      (let* ((marker-end-idx (- (treesit-node-end c) (point-min)))
             (r (verilog-auto--text-fallback-range fallback-lex marker-end-idx)))
        (if (not r)
            (push c unrecovered)
          (let ((range (cons (+ (point-min) (car r)) (+ (point-min) (cdr r)))))
            (push range ranges)
            (push range fallback-ranges)
            (let ((ann-range (verilog-auto--trailing-templated-annotation-range-at
                               (+ (point-min) (cdr r) 1))))
              (when ann-range (push ann-range ranges)))))))
    (dolist (mi (verilog-auto--find-all-of-type root "module_instantiation"))
      (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
        (when c
          (let* ((hier (verilog-auto--enclosing-of-type c "hierarchical_instance"))
                 (close (and hier (verilog-auto--last-child hier))))
            (when close
              (push (cons (treesit-node-end c) (treesit-node-start close)) ranges)
              (let ((ann-range (verilog-auto--trailing-templated-annotation-range close)))
                (when ann-range (push ann-range ranges))))))))
    (dolist (m (verilog-auto--top-level-modules root))
      (let* ((header (verilog-auto--header-node m))
             (c (verilog-auto--find-comment header "/*AUTOARG*/")))
        ;; An ANSI header's /*AUTOARG*/ never expands (see
        ;; `verilog-auto--expand-autoarg-site'), so the region after it
        ;; is never machine-generated -- it's the user's own explicit
        ;; ANSI port declarations, which must never be deleted.
        (when (and c (not (verilog-auto--ansi-header-p header)))
          (let ((close (verilog-auto--last-child (verilog-auto--header-port-list header))))
            (when close
              (push (cons (treesit-node-end c) (treesit-node-start close)) ranges))))))
    ;; M125/M126/M134: all seven block-style markers (AUTOWIRE plus the
    ;; six other ones) share this one path -- `verilog-auto--autowire-
    ;; stale-end' doesn't care which marker COMMENT itself is, only what
    ;; follows it (see this file's M125/M126/M134 headers).
    (dolist (c (append (verilog-auto--find-comments root "/*AUTOWIRE*/")
                        (verilog-auto--find-port-marker-comments root "AUTOOUTPUT")
                        (verilog-auto--find-port-marker-comments root "AUTOINPUT")
                        (verilog-auto--find-port-marker-comments root "AUTOINOUT")
                        (verilog-auto--find-port-marker-comments root "AUTOTIEOFF")
                        (verilog-auto--find-port-marker-comments root "AUTOREG")
                        (verilog-auto--find-port-marker-comments root "AUTORESET")))
      (let ((end (verilog-auto--autowire-stale-end c)))
        (when end
          (push (cons (treesit-node-end c) end) ranges))))
    (let* ((bad (verilog-auto--overlapping-ranges ranges))
           (good (verilog-auto--filter (lambda (r) (not (memq r bad))) ranges))
           ;; A fallback range that lost an overlap check never actually
           ;; got deleted -- TEXT-RECOVERED-COUNT counts what this run
           ;; actually deleted via the text path, same DELETED-vs-
           ;; withheld distinction OVERLAP-SKIPPED-COUNT already draws
           ;; for the tree path.
           (text-recovered (length (verilog-auto--filter
                                     (lambda (r) (memq r good))
                                     fallback-ranges))))
      (setq good (sort good (lambda (a b) (> (car a) (car b)))))
      (dolist (r good)
        (when (< (car r) (cdr r))
          (delete-region (car r) (cdr r))))
      (undo-amalgamate-boundary)
      (when bad
        (message "verilog-delete-auto: %d overlapping range(s) left untouched (buffer unchanged there)"
                  (length bad)))
      (when unrecovered
        (message "verilog-delete-auto: %d /*AUTOINST*/ marker(s) have no enclosing instantiation (a parse error reclassified the site, or the marker is a stray comment not actually inside one); a text-based fallback scan was tried and could not recover them either (no enclosing paren pair, a lexical error, or the instantiation-shape guard refused), left untouched"
                  (length unrecovered)))
      (when (> text-recovered 0)
        (message "verilog-delete-auto: %d /*AUTOINST*/ marker(s) recovered and deleted by a text-based fallback scan (a parse error hid them from the tree)"
                  text-recovered))
      (list (length good) (length bad) (length unrecovered) text-recovered))))

;; --- verilog-auto -------------------------------------------------------------

(defun verilog-auto ()
  "Expand every /*AUTOINST*/, /*AUTOOUTPUT*/, /*AUTOINPUT*/,
/*AUTOINOUT*/, /*AUTOTIEOFF*/, /*AUTOWIRE*/, /*AUTOREG*/, /*AUTORESET*/,
and /*AUTOARG*/ construct in the current buffer, in that order (M125:
GNU's own ordering restricted to what this file implements; M126
inserts AUTOTIEOFF after AUTOINOUT and AUTOREG after AUTOWIRE -- see
this file's M126 header for why AUTOTIEOFF must run BEFORE AUTOREG;
M134 inserts AUTORESET after AUTOREG and before AUTOARG, GNU's own
ordering).
Idempotent: always starts by deleting every existing machine-generated
region (`verilog-delete-auto') and re-expanding from scratch, so
running it twice in a row leaves the buffer byte-for-byte unchanged the
second time. The whole command is one undo group."
  (interactive)
  (let* ((delete-result (verilog-delete-auto))
         (overlap-skipped (nth 1 delete-result))
         (unreachable-autoinst-skipped (nth 2 delete-result))
         (text-recovered-autoinst (nth 3 delete-result)))
    (let ((verilog-auto--module-cache (make-hash-table :test 'equal))
          (verilog-auto--module-full-ports (make-hash-table :test 'equal))
          (verilog-auto--module-port-dims (make-hash-table :test 'equal))
          (verilog-auto--module-file (make-hash-table :test 'equal))
          (verilog-auto--missing-modules nil)
          (verilog-auto--ansi-autoarg-modules nil)
          (verilog-auto--ansi-port-auto-modules nil)
          (verilog-auto--multi-autowire-modules nil)
          (verilog-auto--template-parse-warnings nil)
          (verilog-auto--predeclared-port-names nil)
          (verilog-auto--port-range-conflicts nil)
          (verilog-auto--port-marker-arg-warnings nil)
          (verilog-auto--ansi-autoreg-modules nil)
          (verilog-auto--ansi-tieoff-assign-modules nil)
          (verilog-auto--tieoff-port-reg-skips nil)
          (verilog-auto--tieoff-symbolic-multidim-skips nil)
          (verilog-auto--autoreset-memory-skips nil)
          (verilog-auto--autoreset-symbolic-multidim-skips nil)
          (verilog-auto--template-numbers-t-notices nil)
          (verilog-auto--template-instance-number-notices nil)
          (verilog-auto--template-forward-fallback-notices nil)
          (verilog-auto--template-lisp-eval-failures nil)
          (n-inst 0) (n-wire 0) (n-arg 0) (n-port 0) (n-tieoff 0) (n-reg 0) (n-reset 0))
      (setq n-inst (verilog-auto--expand-all-autoinst))
      (setq n-port (+ (verilog-auto--expand-all-port-propagation 'output)
                       (verilog-auto--expand-all-port-propagation 'input)
                       (verilog-auto--expand-all-port-propagation 'inout)))
      (setq n-tieoff (verilog-auto--expand-all-autotieoff))
      (setq n-wire (verilog-auto--expand-all-autowire))
      (setq n-reg (verilog-auto--expand-all-autoreg))
      (setq n-reset (verilog-auto--expand-all-autoreset))
      (setq n-arg (verilog-auto--expand-all-autoarg))
      (undo-amalgamate-boundary)
      (setq verilog-auto--missing-modules (nreverse verilog-auto--missing-modules))
      (setq verilog-auto--ansi-autoarg-modules (nreverse verilog-auto--ansi-autoarg-modules))
      (setq verilog-auto--ansi-port-auto-modules (nreverse verilog-auto--ansi-port-auto-modules))
      (setq verilog-auto--multi-autowire-modules (nreverse verilog-auto--multi-autowire-modules))
      (setq verilog-auto--template-parse-warnings (nreverse verilog-auto--template-parse-warnings))
      (setq verilog-auto--ansi-autoreg-modules (nreverse verilog-auto--ansi-autoreg-modules))
      (setq verilog-auto--ansi-tieoff-assign-modules (nreverse verilog-auto--ansi-tieoff-assign-modules))
      (setq verilog-auto--tieoff-port-reg-skips (nreverse verilog-auto--tieoff-port-reg-skips))
      (setq verilog-auto--tieoff-symbolic-multidim-skips (nreverse verilog-auto--tieoff-symbolic-multidim-skips))
      (setq verilog-auto--autoreset-symbolic-multidim-skips (nreverse verilog-auto--autoreset-symbolic-multidim-skips))
      ;; M134: `verilog-auto--autoreset-memory-skips' is ALSO a
      ;; (POSITION . TEXT) list (see its own doc string) and joins the
      ;; group below that is deliberately NOT `nreverse'd -- same
      ;; `verilog-auto--notice-first' reasoning applies unchanged.
      ;;
      ;; M125 trailing fix round: NOT `nreverse'd, unlike every other
      ;; notice list above. These three are (POSITION . TEXT) conses now
      ;; (see the "Position-ordered notice lists" section header, above
      ;; `verilog-auto--module-own-port-names'), and the "first" entry
      ;; shown in the echo is picked by `verilog-auto--notice-first'
      ;; scanning for the SMALLEST marker-comment position -- an
      ;; unordered bag scanned for a minimum needs no particular push
      ;; order, so reversing it (or not) changes nothing about which
      ;; entry gets shown, only which physical list position it starts
      ;; at. An earlier version of this fix relied on push/traversal-
      ;; direction order instead (skipping `nreverse' here because a
      ;; single rightmost-first WALK, on its own, already produces
      ;; document order) and a cold review found that account incomplete:
      ;; `verilog-auto--expand-all-port-propagation' is called three
      ;; times, once per KIND ('output/'input/'inout), each its OWN
      ;; independent rightmost-first walk -- getting one call's own push
      ;; order right says nothing about ordering ACROSS the three calls,
      ;; whose relative order is the fixed 'output -> 'input -> 'inout
      ;; sequence a few lines above, not buffer position. Position
      ;; tracking is invariant to both the within-call traversal
      ;; direction AND the across-call call order, which is why it
      ;; replaces the ordering scheme rather than patching it. Pinned by
      ;; `auto_port_notices_name_the_earlier_modules_signal_not_the_later'
      ;; (same KIND, two modules) and
      ;; `auto_port_notices_name_the_earlier_modules_signal_across_different_kinds'
      ;; (DIFFERENT kinds, two modules -- the shape the cold review's own
      ;; independent-scratch-project repro used) in verilog_auto_tests.rs.
      ;; Folded onto whichever of the three messages below actually
      ;; fires -- see `verilog-delete-auto's own docstring for why an
      ;; immediate, separate echo for either of these would just be
      ;; invisibly clobbered by this function's own final message.
      (let ((suffix
             (concat
              (if (> overlap-skipped 0)
                  (format "; %d overlapping range(s) skipped by delete-auto" overlap-skipped)
                "")
              ;; M128 spec section 6b: an `/*AUTOINST*/' marker with no
              ;; enclosing instantiation -- either because generated text
              ;; tripped tree-sitter's whole-statement reclassification
              ;; (this file's own M128 header correction) or because the
              ;; marker is simply a stray comment that was never inside
              ;; one -- could never be LOCATED by `verilog-delete-auto' at
              ;; all. For the parse-error case this is permanently stuck
              ;; expanded, unlike the overlap case above which is at least
              ;; retriable; for a stray marker there is nothing to delete
              ;; in the first place. This function only OBSERVES the
              ;; absent ancestor, so the wording states that observation
              ;; rather than asserting a cause it never checked -- see
              ;; `verilog-auto--unreachable-autoinst-markers's own doc
              ;; string.
              (if (> unreachable-autoinst-skipped 0)
                  (format "; %d /*AUTOINST*/ marker(s) have no enclosing instantiation (parse error or stray marker), a text-based fallback scan could not recover them either, left untouched"
                          unreachable-autoinst-skipped)
                "")
              ;; M129: sites the tree path never reached but the
              ;; text-based fallback scan recovered and deleted anyway.
              (if (> text-recovered-autoinst 0)
                  (format "; %d /*AUTOINST*/ marker(s) recovered by a text-based fallback scan"
                          text-recovered-autoinst)
                "")
              (if verilog-auto--multi-autowire-modules
                  (format "; module %s has multiple /*AUTOWIRE*/ (only the first expanded)%s"
                          (car verilog-auto--multi-autowire-modules)
                          (if (> (length verilog-auto--multi-autowire-modules) 1)
                              (format " (%d total)" (length verilog-auto--multi-autowire-modules))
                            ""))
                "")
              ;; M125: spec section 2.9's notice order.
              (if verilog-auto--ansi-port-auto-modules
                  (format "; AUTOOUTPUT/AUTOINPUT/AUTOINOUT in ANSI header (module %s)%s"
                          (car verilog-auto--ansi-port-auto-modules)
                          (if (> (length verilog-auto--ansi-port-auto-modules) 1)
                              (format " (%d total)" (length verilog-auto--ansi-port-auto-modules))
                            ""))
                "")
              (if verilog-auto--predeclared-port-names
                  (format "; %d signal(s) already declared, skipped (first: %s)"
                          (length verilog-auto--predeclared-port-names)
                          (verilog-auto--notice-first verilog-auto--predeclared-port-names))
                "")
              (if verilog-auto--port-range-conflicts
                  (format "; %d signal(s) with conflicting widths across instances (first: %s)"
                          (length verilog-auto--port-range-conflicts)
                          (verilog-auto--notice-first verilog-auto--port-range-conflicts))
                "")
              (if verilog-auto--port-marker-arg-warnings
                  (format "; %d malformed/unsupported AUTO marker argument(s) (first: %s)"
                          (length verilog-auto--port-marker-arg-warnings)
                          (verilog-auto--notice-first verilog-auto--port-marker-arg-warnings))
                "")
              ;; M92 fix round S1: a template rule line that matched
              ;; neither the exact nor the wildcard shape used to vanish
              ;; with no trace at all; recorded and folded in here now,
              ;; same "can't show two things in one echo line" reasoning
              ;; as every other notice above.
              (if verilog-auto--template-parse-warnings
                  (format "; %d AUTO_TEMPLATE line(s) not recognized (first: %S)"
                          (length verilog-auto--template-parse-warnings)
                          (car verilog-auto--template-parse-warnings))
                "")
              ;; M126: AUTOREG's own ANSI bail-out is GNU-identical
              ;; behaviour (an ANSI output already carries its own type,
              ;; AUTOREG has nothing to add), unlike AUTOARG's -- worded
              ;; separately from the AUTOARG notice above for that reason.
              (if verilog-auto--ansi-autoreg-modules
                  (format "; AUTOREG in ANSI header (module %s, has nothing to add there)%s"
                          (car verilog-auto--ansi-autoreg-modules)
                          (if (> (length verilog-auto--ansi-autoreg-modules) 1)
                              (format " (%d total)" (length verilog-auto--ansi-autoreg-modules))
                            ""))
                "")
              (if verilog-auto--ansi-tieoff-assign-modules
                  (format "; AUTOTIEOFF switched to `assign' form in ANSI header (module %s)%s"
                          (car verilog-auto--ansi-tieoff-assign-modules)
                          (if (> (length verilog-auto--ansi-tieoff-assign-modules) 1)
                              (format " (%d total)" (length verilog-auto--ansi-tieoff-assign-modules))
                            ""))
                "")
              (if verilog-auto--tieoff-port-reg-skips
                  (format "; %d signal(s) already `reg' on the port, tieoff skipped (first: %s)"
                          (length verilog-auto--tieoff-port-reg-skips)
                          (car verilog-auto--tieoff-port-reg-skips))
                "")
              (if verilog-auto--tieoff-symbolic-multidim-skips
                  (format "; %d signal(s) with a symbolic multi-dimensional range, tieoff skipped (first: %s)"
                          (length verilog-auto--tieoff-symbolic-multidim-skips)
                          (car verilog-auto--tieoff-symbolic-multidim-skips))
                "")
              ;; M134 divergence 1: an unpacked array (memory) AUTORESET
              ;; would otherwise have to assign a scalar to -- GNU emits
              ;; that anyway (illegal Verilog); this file skips it and
              ;; says so, rather than silently doing nothing or writing
              ;; code that doesn't compile.
              (if verilog-auto--autoreset-memory-skips
                  (format "; %d signal(s) an unpacked array, AUTORESET skipped (first: %s)"
                          (length verilog-auto--autoreset-memory-skips)
                          (verilog-auto--notice-first verilog-auto--autoreset-memory-skips))
                "")
              ;; M134 fix round item 3: same divergence-5 policy
              ;; AUTOTIEOFF already applies to a symbolic multi-
              ;; dimensional range -- refuse and report, rather than
              ;; either silently doing nothing or (the cold-review-
              ;; caught bug) emitting a bare `sig <= ;' syntax error.
              (if verilog-auto--autoreset-symbolic-multidim-skips
                  (format "; %d signal(s) with a symbolic multi-dimensional range, AUTORESET skipped (first: %s)"
                          (length verilog-auto--autoreset-symbolic-multidim-skips)
                          (car verilog-auto--autoreset-symbolic-multidim-skips))
                "")
              ;; M127 divergence 3: `@' resolved to the empty string
              ;; (no digits, or a custom regexp that didn't match) --
              ;; silent in GNU.
              (if verilog-auto--template-instance-number-notices
                  (format "; %d AUTO_TEMPLATE `@' substitution(s) resolved to empty (first: %s)"
                          (length verilog-auto--template-instance-number-notices)
                          (verilog-auto--notice-first verilog-auto--template-instance-number-notices))
                "")
              ;; M127 divergence 5: an AUTO_TEMPLATE resolved by the
              ;; forward fallback (no preceding comment) -- silent in
              ;; GNU, and the shape a later template can silently steal.
              (if verilog-auto--template-forward-fallback-notices
                  (format "; %d AUTO_TEMPLATE lookup(s) resolved by the forward fallback (first: %s)"
                          (length verilog-auto--template-forward-fallback-notices)
                          (verilog-auto--notice-first verilog-auto--template-forward-fallback-notices))
                "")
              ;; M127 divergence 2: `verilog-auto-inst-template-numbers'
              ;; set to `t' (GNU's absolute-line-number form) -- not
              ;; implemented, behaves as nil, never silently ignored.
              (if verilog-auto--template-numbers-t-notices
                  (format "; %s"
                          (verilog-auto--notice-first verilog-auto--template-numbers-t-notices))
                "")
              ;; M128 divergence 1/2: an `@\"(lisp-expr)\"' token that
              ;; could not be read or signaled an error -- GNU aborts the
              ;; whole `verilog-auto' run and writes nothing at all for
              ;; this; this file falls the one connection back to an
              ;; identity connection (annotated `// Templated (expression
              ;; failed)', section 2.5) and keeps going.
              (if verilog-auto--template-lisp-eval-failures
                  (format "; %d AUTO_TEMPLATE lisp expression(s) failed (first: %s)"
                          (length verilog-auto--template-lisp-eval-failures)
                          (verilog-auto--notice-first verilog-auto--template-lisp-eval-failures))
                ""))))
        (cond
         (verilog-auto--missing-modules
          (message "verilog-auto: module %s not found%s; %d inst, %d wires, %d args, %d ports, %d tieoffs, %d regs, %d resets%s"
                    (car verilog-auto--missing-modules)
                    (if (> (length verilog-auto--missing-modules) 1)
                        (format " (%d total)" (length verilog-auto--missing-modules))
                      "")
                    n-inst n-wire n-arg n-port n-tieoff n-reg n-reset suffix))
         (verilog-auto--ansi-autoarg-modules
          (message "verilog-auto: AUTOARG in ANSI header (module %s)%s; %d inst, %d wires, %d args, %d ports, %d tieoffs, %d regs, %d resets%s"
                    (car verilog-auto--ansi-autoarg-modules)
                    (if (> (length verilog-auto--ansi-autoarg-modules) 1)
                        (format " (%d total)" (length verilog-auto--ansi-autoarg-modules))
                      "")
                    n-inst n-wire n-arg n-port n-tieoff n-reg n-reset suffix))
         (t
          (message "verilog-auto: %d inst, %d wires, %d args, %d ports, %d tieoffs, %d regs, %d resets%s"
                    n-inst n-wire n-arg n-port n-tieoff n-reg n-reset suffix)))))))

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
