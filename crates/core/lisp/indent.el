;;; indent.el --- M36: language-aware auto-indentation -*- lexical-binding: t -*-
;;; M38 extends the tree-sitter block-depth engine to Verilog.
;;; M100 adds enum-member/struct-field lines to the verilog block-node-type
;;; list (see `indent--block-node-types''s own comment, and the M100 note
;;; near the M97 discussion below, for the fix and for the node shapes it
;;; relies on). Known gap still open after M100: the documented one-level-
;;; too-deep quirk on `module'/`interface'/`package'/`class' HEADER lines
;;; (e.g. `package soc_pkg;' computes column 2 against an on-disk column 0)
;;; is unchanged -- it is a different, already-pinned tradeoff, not part of
;;; this milestone's scope.

;; Before this file, TAB only ever inserted a literal tab character (no
;; buffer had an indentation engine at all) -- the single biggest gap in
;; day-to-day programming feel. Loaded right after modes.el (needs
;; `prog-mode-hook'/`treesit--prog-mode-setup', and extends the latter's
;; signature -- see modes.el's own per-mode functions) and before evil.el
;; (whose `evil-open-below'/`evil-open-above' call `indent-current-line-
;; if-supported', see below).
;;
;; --- Two algorithms, chosen per language ---------------------------------
;;
;; c/c++/java/rust/emacs-lisp/verilog: a generic TREE-SITTER BLOCK-DEPTH
;; engine (`indent--treesit-depth-column'), reusing the M12 synchronous
;; primitives (`treesit-parser-create'/`treesit-node-at'/
;; `treesit-node-parent'/`treesit-node-type') -- a full fresh reparse only
;; on the first TAB press for a buffer/language pair, after a language
;; switch, or (rarely) via `incremental_parse''s own defensive
;; length-mismatch fallback (2-6ms/50k characters, measured; see PLAN.md's
;; M12 entry). M108 added an O(1) cache keyed on the buffer's edit
;; generation, so a TAB press with no intervening edit is an `Rc' clone,
;; and M109 made a TAB press after an edit reparse INCREMENTALLY via
;; tree-sitter's `Tree::edit' rather than from scratch (see
;; `treesit.rs''s module doc and `parse' for the cache/incremental-reparse
;; split). Not a per-language hand-written indenter. The idea: walk from
;; the target position up
;; through `treesit-node-parent' and count how many ancestors (inclusive
;; of the starting node itself -- see `indent--block-depth') are one of
;; that language's "block" node types (`indent--block-node-types', below
;; -- dump-verified against real parses for each language, the same
;; one-off "dump, verify, delete" method M33/M34/M38 used; the dump tool
;; itself is not part of this codebase, it existed only long enough to
;; print `(treesit-node-string root)' for a representative snippet in
;; each language and to probe `treesit-node-at' boundary behavior). Depth
;; times that mode's `standard-indent-width' is the target column; a
;; line whose own first non-blank character is a closing TOKEN (`}'/`)'/
;; `]' for the four brace languages, just `)' for elisp, whole WORDS like
;; `end'/`endmodule' for verilog -- see the M38 section below) dedents by
;; one level first.
;;
;; python: indentation IS the syntax, so a tree-sitter parse of
;; currently-invalid-looking (still being typed) Python is far less
;; reliable than for a brace language -- this file uses a plain textual
;; HEURISTIC instead (`python-indent-line'; see its own docstring),
;; documented as a v1 approximation, not a real parse.
;;
;; bash/perl: v1 is just `indent--copy-previous-indentation' -- their
;; block syntax (case/esac, heredocs, perl's many quoting forms, ...) is
;; hairy enough that a real structural engine is future work, not this
;; milestone; copying the nearest non-blank line above is still strictly
;; better than today's "always zero" (a literal tab character).
;;
;; text-mode/fundamental-mode and everything else that never sets
;; `indent-line-function' (org, dired, eshell, ielm, the minibuffer, ...)
;; is completely unaffected -- see `indent-for-tab-command''s own
;; docstring for exactly how it falls back for these.
;;
;; --- The MISSING-token trap (why blank lines get special handling) -------
;;
;; The single most common real-time trigger for auto-indent is typing
;; `{' and pressing RET *before* the matching `}' exists yet. Dump-tool
;; verification turned up a non-obvious tree-sitter behavior here: a
;; `compound_statement' (etc.) with no matching closer parses cleanly as
;; `(compound_statement (MISSING "}"))' -- no ERROR node -- but its own
;; BYTE RANGE ends exactly at the point of the missing token, not at
;; wherever the still-blank rest of the buffer trails off to. Querying
;; `treesit-node-at' at the blank line's own position therefore resolves
;; all the way out to the root node, not the block it's visually "inside"
;; -- naively computing depth there would wrongly give 0, not one level
;; in. The fix (`indent--query-pos-and-depth'): when the target line is
;; blank, query at the nearest real (non-whitespace) character at or
;; before it instead (`indent--prev-nonblank-char-pos'); depth AT that
;; position, followed by the SAME closing-bracket dedent rule applied to
;; THAT character (not the blank line's, which has none), turns out to
;; give the right answer uniformly whether that nearest real character
;; is an opener, a closer, or an ordinary mid-statement character --
;; see that function's docstring for the case-by-case reasoning. This is
;; also why the ERROR-tree fallback (below) is scoped to the ancestor
;; walk from wherever that query position resolves, not "does the whole
;; buffer contain an ERROR anywhere": an unrelated syntax error elsewhere
;; in a large file must not degrade every other line's indentation.
;;
;; A MULTI-level unclosed brace (e.g. `{' then another `{' then RET, two
;; levels deep with neither closed yet) is a harder case: dump testing
;; showed tree-sitter's error recovery can give up and wrap the entire
;; enclosing construct in one flat ERROR node, discarding the nested
;; structure. `indent--block-depth' still terminates safely then (an
;; ERROR is always seen on the way up), just falling back to
;; `indent--copy-previous-indentation' -- a documented, accepted "not
;; wrong, just not maximally smart" v1 outcome (see the ERROR-tree
;; section below), not a crash.
;;
;; rust specifically has it worse than the other three brace languages
;; even at ONE level: tree-sitter-rust's error recovery for a still-open
;; `fn' wraps the ENTIRE signature+body in a flat ERROR node -- no
;; MISSING-token recovery at all, unlike C's clean
;; `(compound_statement (MISSING "}"))' -- and this happens REGARDLESS
;; of whether the unclosed `fn' is preceded by other, already-complete
;; functions (confirmed with a throwaway debug binary alongside the
;; dump tool, same "use once, delete" convention). So "RET right after
;; typing `{'" for a brand new Rust function can never reach the smart
;; depth engine at all -- it always exercises the ERROR-tree fallback
;; instead, same as genuinely broken code would (see
;; `rust_ret_after_open_brace_falls_back_to_error_tree_recovery' in
;; indent_tests.rs). Not a bug: the fallback handles it exactly as
;; designed, just less smartly than C manages for the identical
;; keystroke -- a real, language-specific v1 gap worth knowing about,
;; not silently papered over.
;;
;; --- ERROR-tree fallback ---------------------------------------------------
;;
;; Whenever the ancestor walk (from wherever the query position resolves,
;; up to the root) passes through a node of type "ERROR",
;; `indent--block-depth' returns nil and every per-language `*-indent-
;; line' function falls back to `indent--copy-previous-indentation' --
;; "don't guess, just don't make it worse" for genuinely broken syntax.
;; Free for all six tree-based languages (c/c++/java/rust/elisp/verilog)
;; since they all funnel through the same shared walk; only C is in the
;; required test list, but the safety net isn't C-specific.
;;
;; The SAME fallback also fires (M36 review fix) for a buffer bigger
;; than `indent-treesit-max-chars' (skips the reparse entirely -- see
;; its own docstring) -- an unbounded-size guard the original version
;; of this file didn't have.
;;
;; The closing-token dedent (`indent--query-pos-and-depth') requires the
;; query position's tree-sitter NODE to literally BE that closing token,
;; not just a character match (M36 review fix, see
;; `indent--closer-token-at-p') -- a `}' appearing inside a comment or
;; string literal's own text no longer falsely dedents.
;;
;; --- M38: Verilog -- word closers, and two "flat container" exclusions --
;;
;; Verilog's block-closing keywords (`end'/`endmodule'/`endfunction'/
;; `endtask') are whole WORDS, unlike the single-CHARACTER closers
;; (`)'/`}'/`]') every other tree-based language here uses -- but
;; dump-verified to be plain anonymous leaf tokens with no wrapper node,
;; exactly like a real `}' token (M36/M37 precedent: an anonymous token's
;; `treesit-node-type' IS its own literal text). So `indent--closer-
;; token-at-p' (below) was generalized from "the character at POS,
;; stringified" to "CLOSERS is now always a list of STRINGS" -- each
;; existing language's `-closers' constant changed from a list of
;; CHARACTERS to a list of one-character STRINGS (`?\}' -> `"}"'), and
;; the redundant `(memq (char-after query-pos) closers)' pre-check in
;; `indent--query-pos-and-depth' was dropped entirely: it was always
;; implied by the node-type check anyway (a node whose type is the
;; string "}" necessarily starts with the character ?\}), so dropping it
;; is a pure simplification, not a behavior change, for the five
;; pre-existing languages -- and it's what makes a multi-character word
;; closer work AT ALL, since a bare `char-after' can only ever look at
;; ONE character. `indent--verilog-closers' below is `("end" "endmodule"
;; "endfunction" "endtask") -- deliberately NOT "endcase"/"endgenerate";
;; see the next paragraph for why.
;;
;; Verilog's grammar puts `case'/`generate'/`module' keywords and their
;; matching `endcase'/`endgenerate'/`endmodule' as FLAT SIBLINGS of the
;; content in between, all direct children of ONE node
;; (`case_statement'/`generate_region'/`module_declaration') -- unlike
;; c's `if (x) { ... }', where the opening keyword and the `{...}' block
;; are ALSO siblings, but of a DIFFERENT, uncounted node
;; (`if_statement'), with the brace pair itself a SEPARATE, dedicated
;; child (`compound_statement') that holds ONLY the block's contents.
;; Verilog has no such dedicated "just the body" node for `case'/
;; `generate' -- dump-verified, not assumed. Counting `case_statement'/
;; `generate_region' as blocks would therefore ALSO bump the depth of
;; their own OPENING line (`case (x)'/`generate'), since that line's own
;; `case'/`generate' token is itself a descendant of the very node being
;; counted -- unlike c's `if', which never is. So `indent--block-node-
;; types' below deliberately excludes `case_statement' and
;; `generate_region': `case_item' (one per case arm) and
;; `loop_generate_construct'/`if_generate_construct' (one per for/if
;; generate) already supply the ONE extra level their own contents need,
;; without that self-referential double-count, and `endcase'/
;; `endgenerate' are excluded from the closer list to match -- since
;; nothing counted their parent, nothing needs to be cancelled back out;
;; dump-verified (hand-derived expected columns for a nested module/
;; always/if-else/case/generate fixture, matched exactly) that this
;; combination lines up `case'/`endcase' and `generate'/`endgenerate' at
;; the SAME column, with case items and generate bodies one level in.
;;
;; Documented v1 gap (review round, found by re-testing against a MIXED
;; generate region -- legal SV, just not this milestone's own
;; representative snippet): when a `generate'...`endgenerate' region's
;; direct children are a MIX of a plain module-or-generate item (e.g. a
;; bare `wire' with no enclosing construct) and a `loop_generate_construct'/
;; `if_generate_construct', the two sibling kinds land at DIFFERENT
;; columns, not the one level both would ideally share:
;;
;;   module gm;
;;   generate
;;   wire plain_w;
;;   if (1) begin : blk
;;   wire w2;
;;   end
;;   endgenerate
;;   endmodule
;;
;; dump/engine-verified to compute `generate' at col 4, the plain `wire
;; plain_w;' ALSO at col 4 (same level as `generate' itself -- it has no
;; enclosing block-type node of its own, since `generate_region' is
;; deliberately excluded, see above), `if (1) begin : blk' at col 8 (one
;; level in, from `if_generate_construct'), and `endgenerate' back at col
;; 4 -- see `verilog_mixed_generate_region_plain_item_and_if_generate_
;; construct_indent_at_different_columns' in indent_tests.rs, which pins
;; exactly these three columns as a known, intentional v1 outcome, not an
;; accident.
;;
;; NOT fixed, and deliberately not attempted, because the two candidate
;; fixes both make the FAR MORE COMMON case worse, not better: reversing
;; which side of this line owns the count (make `generate_region' a block
;; type instead, and exclude `loop_generate_construct'/
;; `if_generate_construct') would indeed bring `wire plain_w;' and `if
;; (1) begin' to the SAME column in the mixed case above, but at the cost
;; of `generate_region' now suffering the EXACT self-referential problem
;; `case_statement'/`module_declaration' were excluded/specially-handled
;; to avoid: the `generate' keyword is itself a descendant of
;; `generate_region', so its own line would be wrongly bumped one level
;; deep (the same class of bug this section spent three paragraphs
;; avoiding for `case'/`module') -- and this time in the ORDINARY, no-
;; plain-items-at-all case that is by far the common one, not just the
;; rare mixed one being traded away. A simple ancestor-counting depth
;; model has no way to give one sibling a level while denying it to
;; another sibling of a DIFFERENT node kind without one of those two
;; kinds also being (wrongly) counted from its own opening line's
;; perspective -- a genuine, structural two-sided tradeoff, not a bug
;; left unfixed for lack of trying. Left as a documented, tested
;; imprecision, the same policy as every other quirk in this section.
;;
;; `module_declaration' has the exact same "flat container" shape (the
;; `module' keyword is a descendant of the very node whose body needs
;; the extra level) but, unlike `case'/`generate', has no separate
;; per-item wrapper node to lean on instead -- module items are flat,
;; undifferentiated children of `module_declaration' itself. Excluding
;; it (parallel to `case_statement') would leave an ENTIRE MODULE BODY
;; unindented, which is far worse than the alternative: `module_
;; declaration' IS in `indent--block-node-types', `endmodule' IS in the
;; closer list, module bodies and `endmodule' both indent/dedent
;; correctly (dump-verified), and the one accepted, documented side
;; effect is that re-TABbing a module's own single-line header
;; (`module foo (...);' itself, not its body) computes one level deeper
;; than ideal -- see `verilog_module_header_line_has_a_documented_one_
;; level_indent_quirk' in indent_tests.rs, which pins this as a known,
;; intentional v1 gap rather than an accident (function/task do NOT have
;; this quirk: their own `function'/`task' keyword sits in the OUTER
;; `function_declaration'/`task_declaration' node, structurally separate
;; from the `function_body_declaration'/`task_body_declaration' block
;; node, exactly like c's if/compound_statement split).
;;
;; M97 fix round (FF2): `interface_declaration'/`package_declaration'/
;; `class_declaration' (added to `indent--block-node-types' at M97, see
;; that variable's own comment) inherit this EXACT quirk, for the exact
;; same structural reason -- each one's own opening keyword (`interface'/
;; `package'/`class') is a descendant of the very node whose BODY needs
;; the extra level, so re-TABbing `package soc_pkg;' (as one line, no
;; body yet on that same line) also computes one level deeper than ideal,
;; verified against real content: `demo/rtl/pkg/soc_pkg.sv' line 7,
;; `package soc_pkg;', computes column 2 against an on-disk column 0.
;; Same accepted tradeoff as `module_declaration' above, for the same
;; reason (no separate per-item wrapper node to exclude the header line
;; via) -- pinned by `verilog_package_interface_and_class_header_lines_
;; share_the_documented_one_level_indent_quirk' in indent_tests.rs,
;; alongside the module case rather than as a parallel, separate note.
;;
;; FIXED at M100 (was recorded, not chased, at M97): the same real-file
;; check against `demo/rtl/pkg/soc_pkg.sv' had found 16 of that file's 54
;; lines -- the bodies of its `typedef enum logic [...] {...} alu_op_e;'
;; and `typedef struct packed {...} req_t;' forms -- reindenting to 2
;; columns instead of their on-disk 4, because a SystemVerilog enum's
;; member list and a packed struct's field list were not in
;; `indent--block-node-types' at M97 (that milestone's scope was
;; interface/package/class as BLOCK CONTAINERS, not every node kind that
;; happens to nest inside one). M100 closes this by adding
;; `enum_name_declaration'/`struct_union_member' to the verilog list --
;; see that variable's own comment, immediately above, for the node
;; shapes and for why the tempting alternative (`data_type' itself) is
;; wrong. Pinned by the whole-file comparison test in indent_tests.rs
;; that walks every non-blank line of `demo/rtl/pkg/soc_pkg.sv' against
;; its on-disk column.
;;
;; A wrapped multi-line ANSI port or parameter list (`module foo #(\n
;; parameter W = 8\n) (\n input logic clk,\n ...\n);') needs NO extra
;; block-type entry at all: `list_of_port_declarations'/
;; `parameter_port_list' are dump-verified to already land their
;; continuation lines at the module body's own depth (one level in)
;; purely from `module_declaration''s own count above -- matching real-
;; world style, where ports/parameters align with the module body, not
;; one level deeper. The one accepted imprecision: since bare `)' is
;; deliberately NOT a verilog closer (unlike c, which DOES treat a bare
;; `)' as a generic closer character -- see `indent--c-like-closers' --
;; doing the same for verilog would falsely fire on every multi-line
;; if-condition, function call, and module instantiation, which are far
;; more common than a wrapped header), a closing `);'/`)' line of a
;; wrapped header lands at the body's depth rather than column 0.
;;
;; `always_construct'/`initial_construct' are deliberately NOT block
;; types (same finding as this file's earlier MISSING-token discussion):
;; neither carries its own `begin'/`end' -- the `seq_block' nested inside
;; does (and doesn't exist at all for a single unbraced statement body,
;; same "no block node, no extra level" gap this file already documents
;; for c/rust's unbraced control flow) -- so counting them would double
;; the level `seq_block' already supplies.
;;
;; Verilog's error recovery for an unclosed `begin' is even less
;; forgiving than rust's (see this file's earlier rust paragraph): where
;; C at least produces a clean `(compound_statement (MISSING "}"))' and
;; rust flattens just the still-open `fn', tree-sitter-systemverilog
;; dump-verified to flatten the ENTIRE ENCLOSING MODULE into one ERROR
;; node the instant ANY `begin' inside it is unclosed -- so RET right
;; after typing `begin' always exercises the ERROR-tree fallback for
;; verilog, never the smart depth engine, regardless of how much
;; well-formed code precedes it. Not a bug, same documented, tested
;; (`verilog_ret_after_open_begin_falls_back_to_error_tree_recovery' in
;; indent_tests.rs) v1 outcome as rust's own gap.
;;
;; --- Always spaces -----------------------------------------------------
;;
;; `indent-line-to' always fills with SPACE characters -- equivalent to
;; GNU's `indent-tabs-mode' permanently nil, though this codebase doesn't
;; implement that variable at all (v1: nothing here ever had a reason to
;; insert a tab character for indentation, so there was nothing for a
;; toggle to toggle). Pre-existing tab characters a file already
;; contained are untouched by this engine (it only ever rewrites a
;; line's LEADING whitespace) and still render at conventional 8-column
;; stops (see redisplay.rs's `char_width'/tab-rendering match arms,
;; unchanged by M36 and not obviously broken -- checked, not touched).
;;
;; --- Commands, point semantics, and RET wiring ----------------------------
;;
;; `indent-for-tab-command' is bound globally to "TAB" (see the very
;; bottom of this file). It reimplements "self-insert a literal tab"
;; itself for the no-`indent-line-function' case (text-mode,
;; fundamental-mode, ...) -- every buffer's TAB behavior before M36,
;; still exactly true for these. Where a buffer DOES have one, point
;; placement matches GNU: inside the line's OLD leading indentation (or
;; exactly at its end) moves to the NEW indentation's end; inside the
;; line's TEXT keeps point's offset from the text unchanged (see its own
;; docstring for the exact mechanics). `indent-line-function' in THIS
;; codebase is a pure computation -- it returns a target column (an
;; integer) rather than mutating the buffer and moving point itself the
;; way real GNU Emacs's `indent-line-function' does; `indent-for-tab-
;; command'/`newline-and-indent'/`indent-current-line-if-supported' are
;; the only callers, and centralize the actual mutation
;; (`indent-line-to') and point-placement once instead of duplicating it
;; in every one of the eight per-language functions. A deliberate v1
;; internal-contract simplification, not a compatibility promise with
;; upstream `indent-line-function' values (none exist in this codebase).
;;
;; `newline-and-indent' is bound to RET, but only LOCALLY, from
;; `prog-mode-hook' -- so only the eight `treesit--prog-mode-setup'-based
;; major modes (rust/c/c++/python/sh/java/perl/emacs-lisp; see modes.el)
;; get it. Every other RET stays exactly as before: text-mode/
;; fundamental-mode get plain `newline' (global, simple.el, untouched);
;; org-mode has no RET binding of its own and is not a prog-mode, so it
;; ALSO keeps plain `newline'; dired/eshell/ielm bind RET to their own
;; command in their own local keymap and never run `prog-mode-hook' at
;; all (they're independent major modes, not built on
;; `treesit--prog-mode-setup') -- local always wins over global
;; regardless, so even if some future change made them prog-mode-derived
;; this would still be safe.
;;
;; Evil-mode review fix (M36 review, severity high): a first draft of
;; this file argued RET/TAB "behave the same regardless of evil state,
;; since indentation isn't an evil concept" -- WRONG for the vim
;; emulation layer specifically, and caught in review. Global keybindings
;; (this file's own `indent-for-tab-command' included) are ordinary
;; KEYMAP COMMANDS, not the self-insert fallback M34's buffer-local
;; `inhibit-self-insert' guards (see commands.rs's `dispatch_key') --
;; so without an explicit binding, evil's normal/visual states would
;; silently let TAB reindent (or, worse, self-insert a literal tab in a
;; buffer with no `indent-line-function') and let RET insert text,
;; neither of which is a "normal state edits nothing" guarantee evil is
;; supposed to uphold. evil.el now binds both explicitly per state:
;; normal/visual TAB -> `evil--tab-undefined' (touches nothing, echoes
;; "TAB is undefined"); normal/visual RET -> `evil-ret' (vim's own `+'-
;; equivalent: first non-blank of the next line, a pure motion); both
;; TAB and RET in `evil--op-pending-map' -> the existing `evil--op-
;; invalid' (cancels the pending operator -- `d<CR>' as a linewise
;; operator target is out of v1 scope, see evil.el's header). See
;; evil.el itself for all of the above -- nothing else in THIS file
;; changed to support it. Insert state is the one place the original
;; claim still holds: evil's `evil--insert-map' does NOT bind RET or TAB
;; (only ESC/C-n/C-p), so both fall through to the same local/global
;; resolution as if evil-mode were off entirely -- `newline-and-indent'/
;; `indent-for-tab-command' in prog buffers, matching real Emacs (and,
;; for TAB specifically, deliberately NOT vim's own "literal tab in
;; insert mode" convention -- this codebase doesn't implement that).
;; `emacs' state (dired/eshell/ielm) is likewise unaffected: those
;; modes' own local RET binding already wins over anything evil or this
;; file could add, and none of them are prog-mode buffers to begin with.
;;
;; org-mode's own TAB (`org-cycle', folding) and the minibuffer's own TAB
;; (completion) are both handled on paths that never reach this file's
;; global binding at all -- org's is a buffer-local keymap entry (always
;; consulted before the global map, see commands.rs's `dispatch_key');
;; the minibuffer's is a completely separate branch in `handle_key' that
;; runs before `dispatch_key' is even called. Neither is touched by M36;
;; both have their own pinned regression tests in indent_tests.rs.
;;
;; --- evil.el integration (o/O) -------------------------------------------
;;
;; `evil-open-below'/`evil-open-above' (o/O) now call `indent-current-
;; line-if-supported' right after inserting their newline (vim's own
;; autoindent convention for opening a line) -- see evil.el itself for
;; the two call sites. A no-op wherever `indent-line-function' is nil
;; (any buffer that isn't one of the eight prog modes), so the existing
;; pinned `o'/`O' tests (evil_tests.rs, plain fundamental-mode buffers)
;; are unaffected. Evil's `==' reindent-current-line(s) operator is NOT
;; implemented (v1 scope cut, per the M36 plan): `=' is simply absent
;; from `evil--normal-map' (never registered via `evil--op-start' the
;; way `d'/`c'/`y' are), so pressing it does nothing but echo
;; "= is undefined" -- the ordinary M34 normal-state input-rejection
;; path any other unbound key (e.g. `q') already takes, not a new gap
;; this file introduces.
;;
;; --- What's NOT implemented (documented, not silently missing) -----------
;; - GNU elisp-mode's real argument-alignment artistry (per-form special
;;   indentation rules for `if'/`let'/`cond'/etc, `lisp-indent-function'
;;   properties, ...): this engine only ever does bracket-depth times two,
;;   with a closing-`)' dedent. `(if (> x 0)\n    ...)' will NOT align
;;   the consequent under the test the way real Emacs does.
;; - Continuation-line argument alignment for the brace languages (a
;;   multi-line function call/parameter list does not align to the
;;   opening paren's column) -- v1 lands everything at the enclosing
;;   block's plain depth, full stop.
;; - An unbraced single-statement control-flow body (`if (x) foo();' with
;;   no `{}') does not indent that statement any deeper -- there is no
;;   block node to count there at all; only real brace/bracket
;;   containers add a level.
;; - Python: only ONE level of dedent per line is ever computed (a dedent
;;   keyword always subtracts exactly one `standard-indent-width' from
;;   the base line, regardless of how many levels a real Python parser
;;   might close there) -- GNU python.el's own multi-level cycling TAB is
;;   not implemented.
;; - Elisp vectors (`[...]') are not depth-contributing and `]' is not a
;;   dedent character -- only parenthesized forms are counted, matching
;;   the plan's explicit "bracket depth" (parens) framing.

;; --- M73: `standard-indent-width' detection from the file's own content --
;;
;; Depth times `standard-indent-width' is the target column (see above),
;; but the per-mode default in `treesit--prog-mode-setup' is one fixed
;; number per LANGUAGE, not per FILE -- real-world RTL is 2/3/4 spaces
;; depending on the author, same as C/C++/Java/etc. Opening a 2-space
;; file with a mode whose default is 4 makes every line this engine
;; touches disagree with every line it doesn't (repro: `demo/rtl/core/
;; alu.sv', an `o' RET in the module body lands at column 4 while its
;; neighbors sit at 2 -- `verible-verilog-format' then diffs exactly that
;; one line). `indent--maybe-detect-width' (called from
;; `treesit--prog-mode-setup', see modes.el) scans the buffer's own
;; leading whitespace and, when it can determine a width with
;; confidence, `setq-local' overrides `standard-indent-width' with it.
;;
;; `indent--detect-width' walks at most `indent--detect-max-lines' lines
;; from `point-min'. Per line: blank (only whitespace) lines are
;; skipped entirely; a line whose leading whitespace CONTAINS a tab
;; bumps a `tab-lines' counter and is not sampled (tabs and spaces don't
;; mix meaningfully into one width number); otherwise the count of
;; leading space characters, if > 0, is one sample. `current-indentation'
;; is deliberately NOT used here -- it tab-stop-expands to a visual
;; column (see its own docstring), which is exactly wrong for this: we
;; need the raw leading-space COUNT, and a tab-containing line must be
;; identified as such, not silently folded into some expanded number.
;;
;; If `tab-lines' outnumbers the collected samples, or there are fewer
;; than `indent--detect-min-samples' samples, the file gives no usable
;; signal and detection returns nil (mode default wins, today's
;; behavior unchanged). Otherwise the candidate widths
;; `indent--detect-width-candidates' (8 4 3 2, checked in that order)
;; are each scored by what fraction of samples they evenly divide; the
;; first candidate clearing a 90% threshold
;; (`(>= (* 10 ok) (* 9 total))', integer arithmetic, no floats) wins.
;;
;; Why "what fraction of sampled indents this width evenly divides"
;; instead of the seemingly simpler "most common indent DELTA between
;; adjacent lines": delta-histogramming gets fooled by continuation-line
;; alignment. `demo/rtl/top/soc_top.sv' has a 2-space module body but
;; 4-space-aligned port-connection continuation lines inside module
;; instantiations; the delta method's mode is 4 (wrong -- it answers the
;; *outlier* continuation style, not the body), the divisibility method
;; answers 2 (right, since 2 also evenly divides every 4-space sample).
;; This shape is exactly why `soc_top.sv' is one of this milestone's
;; required test fixtures, not an incidental example.
;;
;; Documented gaps (v1, not silently missing):
;; - CRLF files are not handled by this editor AT ALL beyond the narrow
;;   fix described above (a blank line ending in a lone `\r' does not
;;   pollute detection's samples) -- every `\r' still displays as a
;;   literal `^M' character, same as before this milestone; this is NOT
;;   "CRLF support", just "detection isn't fooled by CRLF blank lines".
;; - Tab-indented files detect as nil and fall back to the mode default;
;;   this engine only ever writes spaces (see "Always spaces" above), so
;;   editing such a file still produces mixed tab/space indentation --
;;   detection does not (and cannot, without also changing the engine to
;;   emit tabs) fix that.
;; - Continuation-alignment-heavy files (`demo/tools/crc32.c', most
;;   elisp) detect as nil, same as tab files -- there is no width that
;;   evenly divides an alignment-column sample set with any reliability.
;; - `find-file' to an ALREADY-OPEN buffer reuses it without rerunning
;;   mode setup (`editing.rs:757') -- no re-detection. Intentional: the
;;   buffer may already carry user edits, and silently changing
;;   `standard-indent-width' under an in-progress edit would be a worse
;;   surprise than leaving it alone.
;; - This codebase has no `revert-buffer' (`editing.rs:652'), so there is
;;   no "file changed on disk -> re-detect" path either.
;; - Detection only ever runs from `treesit--prog-mode-setup', i.e. only
;;   for prog-mode major modes; `fundamental-mode'/org-mode/dired/etc.
;;   never call it and keep the global default.
;; - `set-indent-width' (below) is the escape hatch when detection
;;   guesses wrong -- this codebase has no `describe-variable' (M73
;;   survey confirmed), so there is otherwise no way for a user to even
;;   inspect, let alone correct, the buffer-local value.

;; --- Configuration ---------------------------------------------------------

(defvar standard-indent-width 4
  "Number of columns one level of indentation occupies in the current
buffer. Global default 4; prog-mode major modes override it locally
\(2 for emacs-lisp-mode and sh-mode, 4 for the rest -- see modes.el's
`treesit--prog-mode-setup').")

(defvar indent-line-function nil
  "Buffer-local. When non-nil, a niladic function that computes (but
does NOT itself apply -- see this file's header) the target
indentation column for the current line: an integer >= 0. nil (the
default, and always true for text-mode/fundamental-mode and every
non-prog-mode buffer) means this buffer has no indentation engine at
all -- see `indent-for-tab-command'.")

(defvar indent-detect-width t
  "When non-nil (the default), `treesit--prog-mode-setup' scans a newly
opened prog-mode buffer's own leading whitespace and overrides
`standard-indent-width' with what it finds -- see this file's header
(M73) for the algorithm. Set to nil (e.g. in `init.el') to disable
detection entirely and always keep each mode's fixed default.")

(defconst indent--detect-max-lines 500
  "How many lines from `point-min' `indent--detect-width' scans at most.
See this file's header (M73) -- 100/200/500 all agree on every file the
algorithm was validated against, so this is headroom, not a tuned
cutoff.")

(defconst indent--detect-min-samples 5
  "`indent--detect-width' returns nil (no confident guess) when it
collects fewer than this many indented-line samples -- see this file's
header (M73).")

(defconst indent--detect-width-candidates '(8 4 3 2)
  "Candidate indent widths `indent--detect-width' tries, in this order
-- see this file's header (M73) for why order matters (8 and 4 are
checked before 3 and 2 since a narrower width trivially divides many
samples that were actually written at a wider one).")

;; --- Shared low-level helpers ----------------------------------------------

(defun indent--first-non-blank-pos ()
  "Buffer position of the first non-space/tab character on the current
line, or `line-end-position' if the line has none (a blank line)."
  (let ((pos (line-beginning-position)) (eol (line-end-position)))
    (while (and (< pos eol) (memq (char-after pos) '(?\s ?\t)))
      (setq pos (1+ pos)))
    pos))

(defun current-indentation ()
  "Column of the first non-whitespace character on the current line (the
column of `line-end-position' if the line is blank). TAB-STOP AWARE: a
literal tab character in the line's leading whitespace advances to the
next multiple-of-8 column, not just +1 (M36 review fix) -- unlike the
Rust `current-column' primitive (editing.rs), which counts one column
per character regardless of what it is, and has its own, unrelated
callers this file must not change the behavior of, so this walks the
leading-whitespace prefix itself instead of delegating to it. This
engine itself never PRODUCES a tab character (see this file's header),
but a file it opens can already have one, and `indent--copy-previous-
indentation'/`python-indent-line' both need this line's true visual
column as their BASE, not an undercount from treating a tab as a
single column."
  (let ((pos (line-beginning-position)) (end (indent--first-non-blank-pos)) (col 0))
    (while (< pos end)
      (setq col (if (= (char-after pos) ?\t) (+ col (- 8 (mod col 8))) (1+ col)))
      (setq pos (1+ pos)))
    col))

(defun indent-line-to (column)
  "Set the current line's indentation to COLUMN columns of SPACE
characters, replacing any existing leading whitespace -- this engine
always uses spaces, never tab characters (see this file's header).
Moves point to the new indentation's end (the GNU convention); a caller
that needs to preserve point's position within the line's TEXT instead
must save/restore that itself -- see `indent-for-tab-command'."
  (let ((bol (line-beginning-position))
        (end (indent--first-non-blank-pos)))
    (goto-char bol)
    (delete-region bol end)
    (insert (make-string (max 0 column) ?\s))))

(defun indent--prev-nonblank-line-start ()
  "Buffer position of the beginning of the nearest non-blank line
strictly above the current one, or nil if there is none (the current
line is the buffer's first line, or every line above it is blank)."
  (save-excursion
    (let ((found nil))
      (while (and (not found) (= (forward-line -1) 0))
        (unless (= (indent--first-non-blank-pos) (line-end-position))
          (setq found (point))))
      found)))

(defun indent--prev-nonblank-char-pos ()
  "Position of the nearest non-whitespace character at or before the
start of the current line, skipping blank lines entirely, or nil if
the buffer has none there (the current line is the first non-blank
content, or everything above it is blank). See this file's header
\(the MISSING-token trap) for why the tree-sitter engine needs this
instead of just `indent--prev-nonblank-line-start'."
  (save-excursion
    (beginning-of-line)
    (let ((found nil))
      (while (and (not found) (> (point) (point-min)))
        (backward-char 1)
        (unless (memq (char-after) '(?\s ?\t ?\n))
          (setq found (point))))
      found)))

(defun indent--line-indentation-at (pos)
  "Column of the first non-blank character of the line POS is on."
  (save-excursion (goto-char pos) (current-indentation)))

(defun indent--copy-previous-indentation ()
  "Target column: the same indentation as the nearest non-blank line
above the current one, or 0 if there is none. The whole algorithm for
languages without a structural engine (bash, perl) AND the ERROR-tree
safety net for the tree-sitter depth engines -- see this file's header."
  (let ((prev (indent--prev-nonblank-line-start)))
    (if prev (indent--line-indentation-at prev) 0)))

(defun indent-current-line-if-supported ()
  "If the current buffer has a non-nil `indent-line-function', apply it
to the current line (via `indent-line-to') and leave point at the end
of the new indentation; a no-op otherwise. Shared by `newline-and-
indent' and evil.el's `evil-open-below'/`evil-open-above' (vim
autoindent convention for `o'/`O') -- see this file's header."
  (when indent-line-function
    (indent-line-to (or (funcall indent-line-function) 0))))

;; --- M73: `standard-indent-width' detection -------------------------------
;; See this file's header for the algorithm and the reasoning behind it.

(defun indent--detect-width ()
  "Scan the current buffer (from `point-min', at most
`indent--detect-max-lines' lines) and return a guessed indent width
\(an integer), or nil if there is not enough confident signal. Read-
only: never moves point outside its own `save-excursion', never
changes any buffer-local variable -- see `indent--maybe-detect-width'
for the caller that actually applies the result."
  (save-excursion
    (goto-char (point-min))
    (let ((samples nil) (tab-lines 0) (lines-seen 0))
      (while (and (< lines-seen indent--detect-max-lines) (not (eobp)))
        (let* ((bol (line-beginning-position))
               (eol (line-end-position))
               (end (indent--first-non-blank-pos))
               (rest end))
          ;; CRLF fix (M73 review): `indent--first-non-blank-pos' only
          ;; skips ?\s/?\t, so on a CRLF file it stops AT a trailing
          ;; `\r', not at EOL -- a line that is otherwise nothing but
          ;; leading spaces/tabs plus that `\r' is still a blank line
          ;; for detection purposes, just from a CRLF file. Deliberately
          ;; NOT fixed in `indent--first-non-blank-pos' itself (a shared
          ;; M36 primitive nine other language engines depend on) --
          ;; scoped to this function's own blank-line test only.
          (while (and (< rest eol) (eq (char-after rest) ?\r))
            (setq rest (1+ rest)))
          (unless (= rest eol) ; blank (incl. CRLF-blank) line -> skip
            (let ((pos bol) (n 0) (has-tab nil))
              (while (< pos end)
                (if (= (char-after pos) ?\t) (setq has-tab t) (setq n (1+ n)))
                (setq pos (1+ pos)))
              (if has-tab
                  (setq tab-lines (1+ tab-lines))
                (when (> n 0) (push n samples))))))
        (setq lines-seen (1+ lines-seen))
        (forward-line 1))
      (let ((total (length samples)))
        (cond
          ((> tab-lines total) nil)
          ((< total indent--detect-min-samples) nil)
          (t
           (let ((result nil) (candidates indent--detect-width-candidates))
             (while (and candidates (not result))
               (let ((k (car candidates)) (ok 0))
                 (dolist (n samples)
                   (when (= 0 (mod n k)) (setq ok (1+ ok))))
                 (when (>= (* 10 ok) (* 9 total))
                   (setq result k)))
               (setq candidates (cdr candidates)))
             result)))))))

(defun indent--maybe-detect-width ()
  "When `indent-detect-width' is non-nil, run `indent--detect-width' on
the current buffer and, if it returns a width, `setq-local' override
`standard-indent-width' with it. Called from `treesit--prog-mode-setup'
\(modes.el) after the mode default is set but before `prog-mode-hook'
runs, so an explicit `setq-local' in a mode hook still wins -- see
modes.el's own comment for the full reasoning. A no-op (keeps the mode
default) when detection is disabled or inconclusive."
  (when indent-detect-width
    (let ((width (indent--detect-width)))
      (when width
        (setq-local standard-indent-width width)))))

(defun set-indent-width (width)
  "Set `standard-indent-width' to WIDTH (1..16) for the current buffer.
The manual escape hatch for when `indent--maybe-detect-width' guesses
wrong -- this codebase has no `describe-variable', so there is
otherwise no way to inspect or correct the buffer-local value once a
buffer is open. Errors on an out-of-range WIDTH; on success, reports
the new value the same way `goto-line' reports its own effect.
WIDTH must be an integer -- `(interactive \"n\")' happily hands this a
Float (e.g. 3.5, or even 4.0) when the minibuffer input parses as one
\(see `commands.rs''s `n' spec), and every later `standard-indent-width'
consumer (`make-string' via `indent-line-to') requires a true integer,
so a Float sailing through here silently breaks every subsequent
indent in the buffer with `wrong-type-argument integerp' instead of
failing loudly right here. `simple.el''s `goto-line' has this same gap
\(M73 review) -- out of scope here, left alone; see PLAN.md."
  (interactive "nIndent width: ")
  (cond
    ((not (integerp width))
     (error "Indent width must be an integer: %S" width))
    ((or (< width 1) (> width 16))
     (error "Indent width out of range (1..16): %d" width))
    (t
     (setq-local standard-indent-width width)
     (message "Indent width set to %d" width))))

;; --- Tree-sitter block-depth engine (c/c++/java/rust/emacs-lisp/verilog) -

;; Dump-verified node kind names (see this file's header) -- NOT a guess:
;;  c:      compound_statement (fn/if/while/for bodies), field_declaration_list
;;          (struct body), enumerator_list (enum body), initializer_list
;;          (array/struct literal `{...}').
;;  cpp:    identical four kinds to c (class body is ALSO field_declaration_list;
;;          cpp's grammar reuses c's node names for all four).
;;  java:   block (method/if/while/for bodies -- there is no separate
;;          "method_body" kind), class_body, switch_block.
;;  rust:   block (fn/if/while/for bodies), declaration_list (impl/trait/mod
;;          body), field_declaration_list (struct body), enum_variant_list
;;          (enum body), match_block (match body).
;;  elisp:  list (an ordinary form, e.g. a function call), special_form
;;          (if/let/let*/cond/dolist/dotimes/when/unless/condition-case/
;;          defvar/defvar-local/defconst/lambda/...), function_definition
;;          (defun ONLY -- it gets its own distinct kind, unlike every
;;          other def-form), macro_definition (defmacro ONLY, same
;;          story). `quote' (the `'x' reader macro) is deliberately NOT
;;          included: it doesn't represent an indentation level.
;;  verilog (M38): module_declaration (a module's own body -- see this
;;          file's header for the documented single-line-header quirk
;;          this entails), seq_block (a begin/end block, wherever it
;;          occurs: always/initial bodies, if/else branches, nested
;;          begin/end -- each nesting level dump-verified to count
;;          exactly once, never double-counted against the
;;          always_construct/initial_construct/conditional_statement it
;;          sits inside, none of which are block types themselves -- see
;;          this file's header), function_body_declaration/
;;          task_body_declaration (a function/task's own params+body,
;;          NOT wrapped in begin/end at all -- dump-verified: statements
;;          are direct children), case_item (one per case arm, INCLUDING
;;          `default:' -- dump-verified to be the exact same node kind),
;;          loop_generate_construct/if_generate_construct (a for/if
;;          generate's own HEADER line, one level -- see this file's
;;          header for why `case_statement'/`generate_region' themselves
;;          are deliberately EXCLUDED from this list) plus generate_block
;;          (the for/if generate's begin/end BODY, a second, separate
;;          level -- the same begin/end shape as seq_block, just under a
;;          different node name). interface_declaration/package_declaration/
;;          class_declaration (M97): each behaves exactly like
;;          module_declaration -- its own body, one level, dump-verified
;;          against `demo/rtl/pkg/soc_pkg.sv' (package) and hand-built
;;          interface/class snippets, INCLUDING inheriting module_
;;          declaration's own documented header-line quirk (see this
;;          file's header, M97 fix round FF2, for the details and the
;;          pinning test). Absent before M97: a body inside `package
;;          ... endpackage' (this project's own `soc_pkg.sv' is written
;;          that way) got ZERO block-depth increment, reproduced by
;;          `verilog_package_body_indents_one_level_and_endpackage_
;;          dedents' before this fix landed. See `indent--verilog-closers'
;;          below for the matching `endinterface'/`endpackage'/`endclass'
;;          addition.
(defvar indent--block-node-types
  '((c . ("compound_statement" "field_declaration_list" "enumerator_list" "initializer_list"))
    (cpp . ("compound_statement" "field_declaration_list" "enumerator_list" "initializer_list"))
    (java . ("block" "class_body" "switch_block"))
    (rust . ("block" "declaration_list" "field_declaration_list" "match_block" "enum_variant_list"))
    (elisp . ("list" "special_form" "function_definition" "macro_definition"))
    (verilog . ("module_declaration" "interface_declaration" "package_declaration"
                "class_declaration" "seq_block" "function_body_declaration"
                "task_body_declaration" "case_item" "loop_generate_construct"
                "if_generate_construct" "generate_block"
                ;; M100: an enum's member list and a packed struct's field
                ;; list have no dedicated "body" wrapper node in this
                ;; grammar -- `enum_name_declaration'/`struct_union_member'
                ;; sit directly under `data_type' (dump-verified: `typedef
                ;; enum logic [3:0] { OP_A, OP_B } alu_op_e;' parses as
                ;; `(data_declaration (type_declaration (data_type
                ;; (enum_base_type ...) (enum_name_declaration ...)
                ;; (enum_name_declaration ...)) type_name: ...)))'). Adding
                ;; each ITEM's own node type (not `data_type' itself) works
                ;; the same way `case_item' already does above: the shared
                ;; property is that the item's own node IS the body -- no
                ;; separate wrapper node exists to add instead, so counting
                ;; the item node itself "inclusive of the starting node"
                ;; (see `indent--block-depth') is how it gets its +1 at
                ;; all. Like `case_item', this is NOT limited to one line:
                ;; a member/field that wraps (`AluAdd = 4'h0 +\n
                ;; SOME_OFFSET,' as one `enum_name_declaration', or a
                ;; folded struct field declaration) has its node's span
                ;; cover every one of those lines, and `indent--block-
                ;; depth' walks up from wherever point sits, so each
                ;; wrapped continuation line gets the same +1 too -- that
                ;; is the point of reusing a self-referential node instead
                ;; of a fixed per-line rule.
                ;;
                ;; DO NOT add "data_type" itself here -- it is tempting
                ;; because it is the actual parent, but it is also the
                ;; parent of every ORDINARY declaration's leading token
                ;; (dump-verified: `logic [3:0] foo;' parses as
                ;; `(data_declaration (data_type_or_implicit (data_type
                ;; (integer_vector_type) (packed_dimension ...))) ...)').
                ;; Adding `data_type' would make every port and signal
                ;; declaration in the file indent one level too deep, not
                ;; just enum/struct members.
                "enum_name_declaration" "struct_union_member"
                ;; M119 D3: a class CONSTRUCTOR's own params+body is a
                ;; DISTINCT node kind, `class_constructor_declaration', not
                ;; `function_body_declaration' (dump-verified, M119: `class
                ;; c;\n  function new();\n    x = 0;\n  endfunction\n
                ;; endclass\n' parses as `(class_declaration (class_item
                ;; (class_method (class_constructor_declaration
                ;; (function_statement_or_null ...)))))', with no
                ;; `function_body_declaration' node anywhere in that
                ;; subtree -- an ordinary method in the SAME class DOES go
                ;; through `function_declaration (function_body_declaration
                ;; ...)', so this omission was silently under-indenting
                ;; every constructor while every other method in the same
                ;; class was already correct). Measured against real
                ;; `verible-verilog-format --indentation_spaces=2': a
                ;; constructor's body sits at column 4 (class_declaration
                ;; (1) + class_constructor_declaration (1) = 2 levels, the
                ;; same shape `function_body_declaration' already gets) and
                ;; its own `endfunction' dedents to column 2, symmetric
                ;; with every other `endfunction' (`indent--verilog-
                ;; closers' already lists the bare word `"endfunction"',
                ;; unconditionally -- both kinds of function share the
                ;; SAME literal closer token, so no closer-list change is
                ;; needed for this one, only a block-type addition).
                "class_constructor_declaration"
                ;; M122: `program_declaration' had NO block-depth entry at
                ;; all -- unlike `module_declaration'/`interface_
                ;; declaration'/`package_declaration'/`class_declaration',
                ;; it was never added at M38 or M97 because `demo/rtl/'
                ;; had no `program' block to catch the gap; M122 added the
                ;; first real one (`demo/verif/sram_bank_tb.sv''s
                ;; `program automatic axi_driver ... endprogram'), and its
                ;; entire body -- from the first `import' statement
                ;; onward -- computed at column 0 against verible's own
                ;; column 2 until this entry was added. Dump-verified
                ;; (M122: `program automatic my_prog;\n  import p::*;\n
                ;; initial begin\n    x = 1;\n  end\nendprogram : my_prog\n',
                ;; real `(treesit-node-string (treesit-parser-root-node
                ;; (treesit-parser-create 'verilog)))' output -- M122
                ;; review fix: an earlier version of this comment showed
                ;; anonymous tokens (`(program)', `(\";\")',
                ;; `(endprogram)', `(\":\")') that `to_sexp()' (what
                ;; `treesit-node-string' wraps) does not actually print;
                ;; it prints NAMED nodes only, with field labels this
                ;; comment now includes too) parses as `(source_file
                ;; (program_declaration (program_ansi_header (lifetime)
                ;; name: (simple_identifier)) (data_declaration
                ;; (package_import_declaration (package_import_item
                ;; (simple_identifier)))) (initial_construct
                ;; (statement_or_null ...))) (simple_identifier))' -- the
                ;; EXACT same flat-container shape as `module_declaration'
                ;; (the `program' keyword is a descendant of `program_ansi_
                ;; header', itself a child of the very node whose body
                ;; needs the +1, with body items as flat, undifferentiated
                ;; siblings after it -- no separate per-item wrapper node
                ;; to exclude the header line via, same as `module'/
                ;; `interface'/`package'/`class'). So this inherits the
                ;; EXACT same documented header-line quirk those four
                ;; already have -- see this file's header (M97 fix round
                ;; FF2) and `verilog_program_header_line_shares_the_
                ;; documented_one_level_indent_quirk' in indent_tests.rs,
                ;; which measures it against real content (`demo/verif/
                ;; sram_bank_tb.sv''s own `program automatic axi_driver
                ;; #(' header). See `indent--verilog-closers' below for
                ;; the matching `endprogram' addition.
                "program_declaration"
                ;; M122: a `property''s own SPEC (the `@(...) disable iff
                ;; (...) <property_expr>' portion, EXCLUDING the `property
                ;; NAME;'/`endproperty' wrapper) had NO block-depth entry
                ;; at all -- `demo/rtl/include/soc_defs.svh''s
                ;; `SOC_ASSERT' macro is the only place a `property_spec'
                ;; existed before M122, and it is never invoked (see
                ;; `scope.rs''s own comment), so no real file ever
                ;; exercised this. Dump-verified (M122: `interface my_if;\n
                ;; logic clk, a, b;\n  property p_a;\n    @(posedge clk)
                ;; disable iff (!a) (a && !b) |=> (a && $stable(\n b\n));\n
                ;; endproperty\n  a_x : assert property (p_a) else
                ;; $error(\"x\");\nendinterface\n' -- note NOT
                ;; `demo/rtl/bus/axi4_lite_if.sv''s exact text, a smaller
                ;; repro built to isolate this one node; real
                ;; `(treesit-node-string (treesit-parser-root-node
                ;; (treesit-parser-create 'verilog)))' output -- M122
                ;; review fix: an earlier version of this comment showed
                ;; anonymous tokens (`(property)', `(\";\")',
                ;; `(endproperty)') that `to_sexp()' does not actually
                ;; print) parses as `(source_file (interface_declaration
                ;; (interface_ansi_header name: (simple_identifier))
                ;; (data_declaration ...) (property_declaration name:
                ;; (simple_identifier) (property_spec (clocking_event ...)
                ;; (expression_or_dist ...) (property_expr ...)
                ;; (property_expr ...))) (concurrent_assertion_item ...)))'
                ;; -- `property_spec' is a FLAT child of
                ;; `property_declaration', a sibling of the header's own
                ;; `name:' field (there is no separate `endproperty' node
                ;; at all: `to_sexp()' omits every anonymous token,
                ;; `endproperty' among them, so its absence above is the
                ;; tool's own printing convention, not evidence it isn't
                ;; parsed -- the closer list below still matches on it by
                ;; literal text via `treesit-node-at', which does see
                ;; anonymous tokens), so adding it contributes exactly the
                ;; one extra
                ;; level its own (possibly multi-line, when a nested
                ;; `$stable(...)'/`$past(...)' call's own `list_of_
                ;; arguments' wraps) content needs, without affecting
                ;; `property NAME;''s own header line or `endproperty'
                ;; (neither is a `property_spec' descendant). Measured
                ;; against real `demo/rtl/bus/axi4_lite_if.sv': the
                ;; property body line (`@(posedge clk_i) disable iff
                ;; (!rst_ni) (aw_valid && !aw_ready) |=> (aw_valid &&
                ;; $stable(') sits at column 4 (interface_declaration (1)
                ;; + property_spec (1) = 2 levels), and a wrapped
                ;; `$stable(...)' argument inside it sits at column 8 (the
                ;; same 2 levels PLUS the pre-existing `list_of_arguments'
                ;; wrap step, 4) -- both already correct the moment this
                ;; one entry is added, no further wrap-type or closer
                ;; change needed."
                "property_spec"
                ;; M122 review fix (two rounds -- the first round's
                ;; comment here was WRONG, see below): a `{...}'
                ;; concatenation expression that verible chooses to wrap
                ;; onto its own line(s) had NO block-depth entry at all
                ;; -- reproduced two ways, both real: (1) a `coverpoint''s
                ;; own multi-member expression, `demo/verif/
                ;; sram_bank_tb.sv''s `cp: coverpoint {...}', wrapped
                ;; because verible always gives a MULTI-member coverpoint
                ;; concatenation its own line(s), not because of line
                ;; length; (2) a `case' statement's own selector,
                ;; `demo/verif/axi4_lite_monitor.sv''s `case ({...})',
                ;; wrapped by verible unconditionally too -- MEASURED
                ;; directly (`verible-verilog-format --indentation_spaces=
                ;; 2' on a 71-character `case ({a, b})' line, well under
                ;; the 100-column limit, still produces `case ({\n  a, b\n
                ;; })'), so this is a genuine verible style choice, not a
                ;; line-length artifact this engine could dodge by
                ;; shortening the source. Dump-verified (`{a, b}' parses
                ;; as `(primary (concatenation (expression ...) (expression
                ;; ...)))' in both contexts -- `concatenation' is the node
                ;; whose span covers exactly the wrapped `{...}' body,
                ;; including its own closing `}') that `concatenation'
                ;; contributes exactly ONE ordinary `standard-indent-
                ;; width' level (NOT `indent-wrap-width' -- measured:
                ;; verible puts wrapped members at the enclosing
                ;; construct's own column PLUS 2, matching one normal
                ;; block level, not the fixed 4-column hanging step
                ;; `modport_item'/`parameter_port_list' get).
                ;;
                ;; SECOND review round: the closing `}' DEDENTS back out
                ;; to the enclosing construct's own column in BOTH
                ;; contexts, not just the `case' one -- the first round's
                ;; claim that a coverpoint's closing `}' stays at the
                ;; wrapped-content column was wrong, read off what this
                ;; engine computed rather than off verible, the exact
                ;; mistake this project's rules warn about. Re-measured
                ;; directly: `verible-verilog-format --indentation_spaces=
                ;; 2' on
                ;; `covergroup cg_probe @(posedge clk);\n    cp:
                ;; coverpoint {aw_ok_signal_with_a_long_enough_name_x,
                ;; ar_ok_signal_with_a_long_enough_name_y} {\n      bins a
                ;; = {2'b10}; bins b = {2'b01};\n    }\n  endgroup\n'
                ;; produces `cp:' at column 4, the wrapped members at
                ;; column 6, and `} {' back at column 4 -- IDENTICAL
                ;; treatment to the `case' selector. So a plain
                ;; `(\"}\" . \"concatenation\")' contextual closer (see
                ;; `indent--verilog-closers' below) handles both shapes
                ;; uniformly; no per-context adjustment function is
                ;; needed at all (the earlier `indent--verilog-
                ;; concatenation-case-selector-closer-adjust' has been
                ;; deleted, not left as a no-op, since the generic closer
                ;; machinery now covers what it existed for).
                ;;
                ;; Open question, NOT chased: `multiple_concatenation'
                ;; (`{N{...}}') and `streaming_concatenation' are
                ;; distinct grammar node kinds, not covered by this entry
                ;; -- tried forcing verible to wrap a replication
                ;; expression (`{8{a}}' and longer variants, at several
                ;; line lengths) and could not get it to ever break one
                ;; onto multiple lines, so it is unknown whether this is
                ;; a real gap or something verible simply never does. No
                ;; real material in this tree exercises either node kind.
                "concatenation"
                ;; M119 D4: a `covergroup''s own body (`covergroup_
                ;; declaration') and a wrapped `coverpoint's own bin list
                ;; (`bins_or_empty') get NO block-depth increment at all
                ;; before this milestone -- neither kind appeared in this
                ;; list. Dump-verified (M119, a `class c;\n  covergroup cg
                ;; @(posedge clk);\n    coverpoint LONG_NAME {\n
                ;; bins ...;\n    }\n  endgroup\nendclass\n' shape wide
                ;; enough to force verible to actually wrap the bin list
                ;; onto multiple lines -- a short one collapses to one
                ;; line and never exercises this path at all): the parse
                ;; is `class_declaration (class_item (covergroup_
                ;; declaration (coverage_spec_or_option (cover_point
                ;; (bins_or_empty (bins_or_options ...) (bins_or_options
                ;; ...))))))'. Measured against real `verible-verilog-
                ;; format --indentation_spaces=2': the `coverpoint ... {'
                ;; header line sits at column 4 (class_declaration (1) +
                ;; covergroup_declaration (1) = 2 levels -- `cover_point'
                ;; itself is deliberately NOT added, the same reasoning as
                ;; NOT adding `data_type' above: the header line's own
                ;; query position falls before the `bins_or_empty' node's
                ;; span starts, so `cover_point' alone would already be
                ;; redundant with `covergroup_declaration' for this line,
                ;; and adding it would double-count the BIN lines below,
                ;; which are real descendants of `cover_point' too), each
                ;; `bins ...;' line sits at column 6 (class_declaration (1)
                ;; + covergroup_declaration (1) + bins_or_empty (1) = 3
                ;; levels), and `endgroup' dedents back to column 2 (see
                ;; `indent--verilog-closers' below for the matching
                ;; addition -- unlike `endfunction', `"endgroup"' was not
                ;; already in that list under any name). The bin list's
                ;; own closing `}' needs a THIRD change, a contextual
                ;; closer (see `indent--closer-token-at-p''s M119
                ;; addition, and `indent--verilog-closers' below): a plain
                ;; `}' closer would also wrongly dedent the enum/struct
                ;; closing `}' pinned unaffected by
                ;; `verilog_enum_and_struct_closing_brace_lines_
                ;; unaffected_still_two', because unlike `bins_or_empty',
                ;; `enum_name_declaration'/`struct_union_member' are
                ;; siblings of their own closing `}', never its parent.
                "covergroup_declaration" "bins_or_empty")))
  "Alist of (LANG-SYMBOL . BLOCK-NODE-TYPE-STRINGS) -- see the comment
immediately above for how each language's list was determined.")

;; --- M90: a SEPARATE wrap-step axis for wrapped port/parameter/argument ----
;; lists (verilog only). See this file's header note below for the
;; reasoning; in short: `verible-verilog-format' -- the tool that produced
;; `demo/rtl''s committed formatting byte for byte at its default flags --
;; indents a continuation line inside a wrapped `.port(net)' list, a
;; `#(...)' parameter list, or an ordinary call's wrapped argument list by a
;; FIXED 4 columns (`--wrap_spaces', default 4), decoupled from
;; `--indentation_spaces' (default 2, i.e. this engine's own
;; `standard-indent-width'): measured at indentation widths 2/3/4/8, the
;; wrap delta stayed 4 every time, not `standard-indent-width' every time.
;; A single ancestor-counting `block_depth * standard-indent-width' model
;; (`indent--block-node-types', above) has no way to express a second,
;; independently-sized step, so this is a genuinely separate quantity, not
;; a second block-type list reusing the same multiplier.
;;
;; A NARROWER claim than it might look like at first, and the boundary
;; matters: the fixed step above reproduces verible only for a HANGING
;; list -- the wrap node's opening paren immediately followed by a
;; newline, which is what every instantiation in `demo/rtl' and
;; essentially all real RTL port lists look like (dump-verified: a call
;; nested inside a wrapped port connection lands at the SAME column, 10,
;; under both this engine and real verible output). When a call's
;; argument is ITSELF a call whose own paren is followed by more text on
;; the SAME line -- e.g. `do_call_with_a_long_name(other_call_with_a_
;; long_name(\n  arg1, arg2, arg3\n));' -- verible switches to PAREN-
;; COLUMN alignment for that continuation line (column 29, lining up
;; under `other_call_with_a_long_name('s own opening paren), while this
;; engine still computes the fixed-step answer, block 4 + wrap 8 = 12.
;; Measured on the built binary against real `verible-verilog-format'
;; output, M90 fix round. NOT chased: verible's conditional switch
;; depends on line-length lookahead (does the nested call's own opening
;; line still fit?) this engine has no mechanism for -- the exact same
;; gap that already rules out enabling this axis for c/clang-format,
;; below -- and the shape does not occur anywhere in `demo/rtl'. Pinned,
;; not fixed, by `verilog_nested_call_inside_wrapped_call_diverges_from_
;; verible_paren_alignment_a_documented_scope_decision' in
;; indent_tests.rs.
(defvar indent-wrap-width 4
  "Number of columns a continuation line inside a wrapped port/parameter/
argument list (see `indent--wrap-node-types') indents by, ON TOP OF its
enclosing block depth's own `standard-indent-width' columns. Deliberately
NOT derived from `standard-indent-width' -- this mirrors
`verible-verilog-format''s own two independent flags,
`--wrap_spaces' (default 4, what this variable's default matches) versus
`--indentation_spaces' (default 2, what `standard-indent-width' typically
is for a verible-formatted file): a real file indented at 2 columns per
block level still gets its wrapped continuation lines at +4, not +2,
because verible's own formatter -- the tool that produced `demo/rtl''s
committed formatting byte for byte -- does exactly that. Global, not
buffer-local; unlike `standard-indent-width', nothing in this file
attempts to detect it from a file's own content.")

;; Node-type list, parallel to `indent--block-node-types': node types whose
;; CONTENTS (a continuation line inside them, one ancestor-walk step) get
;; ONE `indent-wrap-width' step rather than one `standard-indent-width'
;; block level. Verilog only for now (see this file's header) -- real-parse
;; dump-verified (M90, `(insert \"module top;\\n  sub_module #(\\n
;; .W(8),\\n .D(4)\\n ) u_sub (\\n .clk(clk),\\n .rst(rst)\\n );\\n
;; initial begin\\n do_call(\\n a,\\n b\\n );\\n end\\nendmodule\\n\")'
;; followed by `(treesit-node-string root)'): `list_of_parameter_value_
;; assignments' sits inside `parameter_value_assignment' (the `#(...)'
;; form), `list_of_port_connections' sits inside `hierarchical_instance',
;; and `list_of_arguments' sits inside `tf_call' -- in every case the
;; list node's own parens are SIBLINGS emitted by the grammar rule that
;; wraps it (`hierarchical_instance', `parameter_value_assignment',
;; `tf_call'), not children of the list node itself, so a continuation
;; line (a descendant of the list node) gets exactly one wrap step and the
;; closing `)'/`);' line (a sibling of the list node, not a descendant)
;; gets none -- no separate closer-list entry needed for this. Does NOT
;; separately list `let_list_of_arguments'/`property_list_of_arguments'/
;; `sequence_list_of_arguments' -- real-parse dump-verified (M90 fix
;; round) that this grammar does not actually need them: `assert
;; (my_let(x,y));' (a `let'-flavored call) parses through the ORDINARY
;; `tf_call'/`list_of_arguments' path with no `let_list_of_arguments'
;; node appearing anywhere, so it already gets the wrap step through the
;; entry above; and both `assert property (my_prop(x,y));' AND `cover
;; property (my_seq(x,y));' route through `sequence_list_of_arguments',
;; with no distinct `property_list_of_arguments' node observed in either
;; dump -- so of the three names this defvar's earlier version claimed
;; were aliases, `let_list_of_arguments' was verified NOT to occur at
;; all (the ordinary path already covers that call shape) and
;; `property_list_of_arguments' was verified NOT to occur either
;; (`sequence_list_of_arguments' covers both property and sequence
;; calls); `sequence_list_of_arguments' itself was confirmed to occur
;; but is still left out of this list, deliberately, because a property/
;; sequence call's argument list is not RTL-critical -- can be added the
;; same way if a real file needs it. All three claims verified by real
;; parse, not read off `grammar.js' alone.
;; M119 D1: `parameter_port_list' (a module/interface's own `#(...)' ANSI
;; parameter header) and `list_of_port_declarations' (the ANSI port list
;; that follows) WERE deliberately excluded here through M118, on the
;; theory that they're covered by `module_declaration''s own block depth
;; instead. Real `verible-verilog-format --indentation_spaces=2' proves
;; that theory wrong: a wrapped module header's continuation lines sit at
;; column 4, the SAME wrap step every other wrapped list in this table
;; gets, not column 2 -- measured directly (`verible-verilog-format
;; --indentation_spaces=2' on `module top #(\\n  parameter W = 8\\n) (\\n
;; input logic clk,\\n  input logic rst\\n);\\n  wire w;\\nendmodule\\n'
;; puts `parameter W = 8'/`input logic clk'/`input logic rst' at column
;; 4, `wire w;' at column 2) and cross-checked against real material:
;; `demo/rtl/core/alu.sv' lines 7-24 sit at column 4 on disk and
;; `verible-verilog-format --verify demo/rtl/core/alu.sv' exits 0. Real-
;; parse dump-verified (M119, `(insert \"module top #(\\n  parameter W =
;; 8\\n) (\\n  input logic clk,\\n  input logic rst\\n);\\n  wire
;; w;\\nendmodule\\n\")' followed by `(treesit-node-string root)'): both
;; node kinds sit directly inside `module_ansi_header', as SIBLINGS of the
;; `#(''s and port list's own parens (never children of the list node
;; itself), the exact same shape as `list_of_port_connections' etc. above
;; -- so the closing `) ('/`);' lines need no separate closer-list entry,
;; same reasoning as the rest of this table. `interface_ansi_header'
;; dump-verified to use the identical two node kind NAMES for its own
;; wrapped header (`interface top_if #(\\n  parameter W = 8\\n) (\\n
;; input logic clk\\n);\\nendinterface\\n'), so one shared entry covers
;; both module and interface headers; `package_declaration'/
;; `class_declaration' have no parameter/port list production at all, so
;; nothing else needs an entry. Was pinned wrong by
;; `verilog_ansi_header_wrapped_port_and_parameter_lists_unaffected_by_wrap_step'
;; (renamed `..._wrap_step_to_four', asserting \"4\") -- that test's
;; expectation was read off its own hand-typed two-space fixture rather
;; than off verible, the exact trap M110 (see CLAUDE.md) warns about.
(defvar indent--wrap-node-types
  '((verilog . ("list_of_port_connections" "list_of_parameter_value_assignments"
                "list_of_arguments" "parameter_port_list" "list_of_port_declarations"
                ;; M124: `list_of_ports' is the NON-ANSI counterpart of
                ;; `list_of_port_declarations' above (`module top (clk,
                ;; rst);' vs. `module top (input clk, input rst);') --
                ;; dump-verified (M124: `module top (\n  clk,\n
                ;; rst\n);\n  input clk;\n  input rst;\nendmodule\n', real
                ;; `(treesit-node-string (treesit-parser-root-node
                ;; (treesit-parser-create 'verilog)))' output) to sit
                ;; directly inside `module_nonansi_header' as a SIBLING of
                ;; its own `('/`)' (the same shape `list_of_port_
                ;; declarations' has inside `module_ansi_header', not a
                ;; child of it), and with a `parameter_port_list' present
                ;; too (`module top #(\n  parameter W = 8,\n  parameter
                ;; DEPTH = 16\n) (\n  clk,\n  rst\n);\n  input
                ;; clk;\nendmodule\n', dump-verified the same way) the two
                ;; list nodes sit side by side inside the SAME
                ;; `module_nonansi_header', exactly like the ANSI shape
                ;; above. `interface_nonansi_header' and
                ;; `program_nonansi_header' dump-verified (M124) to use
                ;; the identical node kind name for their own wrapped
                ;; non-ANSI port lists, so this one entry covers module,
                ;; interface, and program non-ANSI headers alike -- see
                ;; `indent--verilog-header-wrap-depth-adjust' and
                ;; `indent--verilog-closers' below for the matching D1/D1-
                ;; closer treatment this needs. Closes the gap this
                ;; comment block used to document as a deliberate v1 scope
                ;; cut (M119 FIX-6) -- that gap no longer exists.
                "list_of_ports"
                ;; M122: a `modport''s own port list has no dedicated
                ;; "list" wrapper node at all -- dump-verified (M122:
                ;; `interface my_if;\n  logic a, b, c;\n  modport mst(\n
                ;; output a, b,\n      input c\n  );\nendinterface\n', real
                ;; `(treesit-node-string (treesit-parser-root-node
                ;; (treesit-parser-create 'verilog)))' output -- M122
                ;; review fix: an earlier version of this comment showed
                ;; anonymous tokens (`(modport)', `("(")', `(",")',
                ;; `(")")') that `to_sexp()' does not actually print)
                ;; parses as `(source_file (interface_declaration
                ;; (interface_ansi_header name: (simple_identifier))
                ;; (data_declaration ...) (modport_declaration
                ;; (modport_item (simple_identifier)
                ;; (modport_ports_declaration
                ;; (modport_simple_ports_declaration (port_direction)
                ;; (modport_simple_port (simple_identifier))
                ;; (modport_simple_port (simple_identifier))))
                ;; (modport_ports_declaration
                ;; (modport_simple_ports_declaration (port_direction)
                ;; (modport_simple_port (simple_identifier))))))))' --
                ;; each comma-separated `modport_ports_declaration' is a
                ;; DIRECT child of `modport_item' itself, unlike a
                ;; module's ports (which sit inside a dedicated
                ;; `list_of_port_declarations'). `modport_item' is
                ;; therefore the node to add: measured against real
                ;; `demo/rtl/bus/axi4_lite_if.sv', a wrapped modport port
                ;; line (e.g. `output aw_addr, aw_valid,') sits at column
                ;; 6 -- `interface_declaration''s own block depth (1
                ;; level = 2) PLUS the fixed `indent-wrap-width' step (4),
                ;; the exact same hanging-list treatment
                ;; `list_of_port_declarations' gets for a module's own
                ;; ports, not a second `standard-indent-width' block
                ;; level (which would compute column 4, not 6). See
                ;; `indent--verilog-closers' below for the matching
                ;; contextual closer this needs, on the same M119 D1
                ;; reasoning: `modport_item''s own closing `)' is a child
                ;; of the wrap node itself, not a sibling, so without a
                ;; closer entry that line would also wrongly get bumped
                ;; by the wrap step.
                "modport_item")))
  "Alist of (LANG-SYMBOL . WRAP-NODE-TYPE-STRINGS) -- see the comment
immediately above. Only verilog has an entry: c/c++/java/rust are
deliberately NOT given one. `rustfmt' always uses a fixed hanging step
like verible, but `clang-format' CONDITIONALLY aligns a wrapped argument
list to the opening paren's own column when the first argument still fits
on the opening line, falling back to a hanging step only when it doesn't
-- a decision that depends on line-length lookahead this engine has no
mechanism for (it only ever looks at ancestor node types, never column
positions or line lengths). Enabling a single fixed-step rule for c would
therefore be wrong in one of clang-format's two modes, and there is no
measurement here (unlike verilog's verible check, above) to justify
picking either one.

M124: non-ANSI (`list_of_ports') wrapped port lists (`module top (clk,
rst);' with the port list wrapped onto its own lines) are handled the
same way as the ANSI `list_of_port_declarations' shape above -- see
`indent--wrap-node-types''s own M124 comment for the dump-verified
grammar shape shared by module/interface/program non-ANSI headers, and
`indent--verilog-header-wrap-depth-adjust'/`indent--verilog-closers'
below for the matching depth-cancellation and closer treatment.")

(defconst indent--c-like-closers '(")" "}" "]")
  "Tokens that dedent a line by one level when they are its own first
non-blank character -- c/c++/java/rust. Each is a one-character STRING,
not a character (M38: generalized alongside `indent--closer-token-at-p'
so the same mechanism also fits Verilog's multi-character WORD closers
-- see this file's header).")

(defconst indent--elisp-closers '(")")
  "Like `indent--c-like-closers', for emacs-lisp-mode (parens only --
see this file's header on elisp vectors).")

(defconst indent--verilog-closers
  '("end" "endmodule" "endfunction" "endtask" "endinterface" "endpackage" "endclass"
    ;; M122: pairs with the `program_declaration' addition to
    ;; `indent--block-node-types' above -- `endprogram' is the SOLE
    ;; block-type node a program's own body sits inside, same shape as
    ;; `endmodule'/`endinterface'/`endpackage'/`endclass', so it dedents
    ;; exactly one level.
    "endprogram"
    ;; M119 D4: pairs with the `covergroup_declaration' addition to
    ;; `indent--block-node-types' -- `endgroup' is the SOLE block-type
    ;; node a covergroup's own body sits inside, same shape as
    ;; `endmodule'/`endclass' above, so it dedents exactly one level.
    "endgroup"
    ;; M119 D4: a CONTEXTUAL closer, not a plain word -- see
    ;; `indent--closer-token-at-p''s own docstring for why a blanket `}'
    ;; closer would be wrong (it would also wrongly dedent the enum/struct
    ;; closing `}', pinned unaffected by
    ;; `verilog_enum_and_struct_closing_brace_lines_unaffected_still_two').
    ;; This fires only when the `}' node's immediate parent is
    ;; `bins_or_empty' -- a wrapped `coverpoint's own bin-list closer.
    ("}" . "bins_or_empty")
    ;; M119 D1: pair with the `parameter_port_list'/`list_of_port_
    ;; declarations' additions to `indent--wrap-node-types' -- unlike
    ;; that table's other three entries, this grammar nests the closing
    ;; `)' of a module/interface ANSI header's `#(...)' and following
    ;; `(...)' INSIDE the list node itself (dump-verified, M119: ancestor
    ;; probe on a real multi-parameter/multi-port header), so without a
    ;; closer these two lines wrongly get a wrap step. See
    ;; `indent--query-pos-and-depth''s M119 note for how this same
    ;; closer-dedent now also reduces WRAP-DEPTH, not just DEPTH.
    (")" . "parameter_port_list")
    (")" . "list_of_port_declarations")
    ;; M124: `list_of_ports' (the non-ANSI counterpart) nests its own
    ;; opening/closing parens as children of itself the same way, per
    ;; the reconnaissance note this milestone reproduced against a real
    ;; parse ("its own `(' and `)' are children of `list_of_ports'
    ;; itself, the same nested-paren shape M119 documented" above) --
    ;; without this entry, a non-ANSI header's closing `);' would wrongly
    ;; inherit the wrap step too.
    (")" . "list_of_ports")
    ;; M119 D1 (continued): the OPENING delimiter of each list is ALSO a
    ;; child of the list node it opens, not a sibling -- dump-verified,
    ;; M119, against `demo/rtl/top/soc_top.sv' (real material: its
    ;; `import' clause between the module name and `#(' forces `#(' onto
    ;; its own physical line, so this shape is directly observable, not
    ;; theoretical). Left alone, that opening line would ALSO wrongly get
    ;; a wrap step; treating it exactly like its own list's closing token
    ;; keeps it at the header's own (already-quirky, already-documented)
    ;; base column, matching real verible's on-disk column 0 for that
    ;; line. `(' as `list_of_port_declarations''s own opener is included
    ;; for the same reason, on the same reasoning: real
    ;; `verible-verilog-format --indentation_spaces=2' pushes this token
    ;; onto its own line too when a module has an `import' clause but NO
    ;; parameter list (measured directly, M119 fix round -- see
    ;; `verilog_import_clause_with_no_parameter_list_pushes_port_list_
    ;; open_paren_to_its_own_line_at_column_zero' in indent_tests.rs for
    ;; the exact fixture and command). No file under `demo/rtl' currently
    ;; has this shape, but real verible does produce it.
    ("#" . "parameter_port_list")
    ("(" . "list_of_port_declarations")
    ;; M124 fix round: `list_of_ports' (the non-ANSI counterpart) nests
    ;; its own opening `(' as a child of itself too, exactly the same
    ;; shape as `list_of_port_declarations' above -- dump-verified
    ;; against a real `import' + non-ANSI header parse. Without this
    ;; entry, an `import' clause forcing a non-ANSI port list's `(' onto
    ;; its own physical line wrongly inherited the wrap step (column 4
    ;; instead of real verible's column 0). See
    ;; `verilog_non_ansi_import_clause_pushes_port_list_open_paren_to_
    ;; its_own_line_at_column_zero' in indent_tests.rs.
    ("(" . "list_of_ports")
    ;; M122: pairs with the `modport_item' addition to
    ;; `indent--wrap-node-types' above -- `modport_item''s own closing
    ;; `)' is nested INSIDE the wrap node it closes, not a sibling of
    ;; it, so without this entry that line wrongly inherits the wrap
    ;; step too. Needs `indent--verilog-modport-item-closer-adjust'
    ;; (M122, see that function's own doc) paired with it to cancel out
    ;; this SAME entry's unwanted side effect on plain DEPTH -- unlike
    ;; the `parameter_port_list'/`list_of_port_declarations' entries
    ;; above, `modport_item' was never a block-node-type in the first
    ;; place, so there is no pre-existing DEPTH contribution from it
    ;; for the closer's blanket -1 to harmlessly re-cancel.
    (")" . "modport_item")
    ;; M122 review fix (second round): pairs with the `"concatenation"'
    ;; addition to `indent--block-node-types' above -- UNLIKE
    ;; `modport_item', `concatenation' genuinely IS a block-node-type
    ;; (it contributes a real +1 to DEPTH for its own content lines), so
    ;; this is an ORDINARY contextual closer needing no compensating
    ;; adjust-fn at all: the closer's blanket -1 exactly cancels the one
    ;; level `concatenation' itself added, the same shape
    ;; `endclass'/`endinterface' already have with their own block
    ;; types. Re-measured directly (M122 second review round) that a
    ;; wrapped concatenation's closing `}' dedents back to the ENCLOSING
    ;; construct's own column in every real shape tried (a `coverpoint'
    ;; expression AND a `case' selector alike) -- there is no shape
    ;; where it stays at the wrapped-content column, so this entry is
    ;; unconditional, not contextual on what follows the concatenation.
    ("}" . "concatenation"))
  "Like `indent--c-like-closers', for verilog-mode -- mostly WHOLE-WORD
closers rather than single characters (dump-verified plain anonymous
leaf tokens, no wrapper node, same `treesit-node-type' == own-literal-
text shape as a brace language's `}'), plus (M119) three CONTEXTUAL
entries, each a `(TOKEN . PARENT-TYPE)' cons -- see
`indent--closer-token-at-p'. Deliberately excludes `endcase'/
`endgenerate' -- see this file's header for why those two must NOT be
treated as dedent triggers here. `endinterface'/`endpackage'/`endclass'
(M97) pair with the `interface_declaration'/`package_declaration'/
`class_declaration' entries added to `indent--block-node-types' at the
same time -- each is the SOLE block-type node their own body sits
inside (dump-verified same
shape as `module_declaration'/`endmodule'), so they dedent exactly one
level, symmetric with `endmodule'. `endgroup' (M119) is the same shape,
paired with `covergroup_declaration'.

Maintainer warning (M119 fix round, FIX-6): this list mixes plain-
STRING entries (matched by `treesit-node-type' alone) and `(TOKEN .
PARENT-TYPE)' CONS entries (matched contextually, see
`indent--closer-token-at-p'), and `indent--closer-token-at-p' stops at
the FIRST entry that matches -- it does not check whether a later,
more specific entry would also have matched. No token currently
appears in both forms (e.g. there is no plain `\")\"' entry alongside
the `(\")\" . \"parameter_port_list\")' one above), so there is no
ambiguity today. But if a future addition puts a PLAIN entry for a
token that already has a CONS entry here (or the reverse, a CONS entry
for a token some other CONS entry already narrows further), the
plain/first-listed one would silently win and SHADOW the more specific
one for every node it also happens to match, with no error and no
test failure to flag it -- check for an existing entry for the same
token before adding a new one.")

(defun indent--block-depth (node block-types)
  "Count of nodes among NODE and its ancestors (via `treesit-node-parent',
INCLUSIVE of NODE itself) whose `treesit-node-type' is a member of
BLOCK-TYPES (a list of strings). nil if NODE or any ancestor up to the
root is a tree-sitter ERROR node -- callers fall back to
`indent--copy-previous-indentation' then (see this file's header).

Inclusive-of-self counting is deliberate and dump-verified safe:
querying `treesit-node-at' exactly at a closing (or opening) bracket's
own position reliably returns that bracket's own TOKEN (a leaf, never
itself a block-type node) rather than its parent block node -- so
counting NODE itself never double-counts a block that would also be
reached by walking up from it."
  (let ((n node) (depth 0) (error-seen nil))
    (while (and n (not error-seen))
      (let ((type (treesit-node-type n)))
        (cond
          ((string= type "ERROR") (setq error-seen t))
          ((member type block-types) (setq depth (1+ depth)))))
      (setq n (treesit-node-parent n)))
    (if error-seen nil depth)))

(defun indent--closer-token-at-p (node closers)
  "Non-nil if NODE (as returned by `treesit-node-at') is ITSELF one of
the literal closing tokens in CLOSERS. Each element of CLOSERS is either
a STRING -- one-character (`\")\"'/`\"}\"'/`\"]\"', the brace languages/
elisp) or a whole WORD (`\"endmodule\"', verilog), matched whenever
`treesit-node-type' is EXACTLY that string (tree-sitter reports an
anonymous/literal token's type as its own text, e.g. a real `}' token's
type is the STRING \"}\", and verilog's `endmodule' keyword is likewise
ONE anonymous token whose type is the STRING \"endmodule\", not a
sequence of one-character tokens -- both dump-verified) -- or (M119) a
CONS `(TOKEN . PARENT-TYPE)', matched only when NODE's type is TOKEN
*and* its immediate `treesit-node-parent''s type is PARENT-TYPE. The
CONS form exists because `}' cannot be a plain closer for verilog: a
`typedef enum {...} e;'/`typedef struct packed {...} req_t;' closing `}'
is a SIBLING of the item nodes it closes (`enum_name_declaration'/
`struct_union_member' never wrap it -- see `indent--block-node-types'
comment), so a blanket `}' closer would wrongly dedent it (pinned by
`verilog_enum_and_struct_closing_brace_lines_unaffected_still_two'); a
`covergroup''s `coverpoint ... { ... }' closing `}', by contrast, really
is a child of `bins_or_empty' (the node counted for the bin lines inside
it -- see `indent--block-node-types'), so it DOES need dedenting, but
only in that one parent context. M36 review fix (severity medium),
generalized at M38 to whole-word closers: comparing `treesit-node-type'
rather than raw buffer text can't mistake a closing token that merely
appears inside a comment or string literal's TEXT for a real one -- e.g.
a line ending `// returns {1, 2, 3}' does not falsely read as ending in
`}': that `}' belongs to ONE comment node whose own type is \"comment\"
(or similar), never \"}\" itself, so this check correctly rejects it.
(M38 also drops the FORMER pre-check here, `(memq (char-after pos)
closers)': it was always implied by this same node-type comparison
anyway -- a node whose type is the string \"}\" necessarily starts with
the character ?\\} -- so removing it is a pure simplification for the
pre-existing languages, and it's the one change that makes a MULTI-
character word closer possible at all, since a bare `char-after' can
only ever compare a single character.)"
  (let ((type (treesit-node-type node)))
    (catch 'indent--closer-found
      (dolist (c closers)
        (cond
          ((and (stringp c) (string= type c)) (throw 'indent--closer-found t))
          ((consp c)
           (when (string= type (car c))
             (let ((parent (treesit-node-parent node)))
               (when (and parent (string= (treesit-node-type parent) (cdr c)))
                 (throw 'indent--closer-found t)))))))
      nil)))

(defun indent--node-has-ancestor-type-p (node type)
  "Non-nil if NODE or one of its ancestors (via `treesit-node-parent',
INCLUSIVE of NODE itself) has `treesit-node-type' TYPE (a string). Used
by `indent--verilog-generate-depth-adjust' (M119 D2) to distinguish an
implicit (module-scope) generate-if/for from one wrapped in `generate'/
`endgenerate'."
  (let ((n node))
    (while (and n (not (string= (treesit-node-type n) type)))
      (setq n (treesit-node-parent n)))
    (and n t)))

(defconst indent--verilog-generate-construct-types
  '("if_generate_construct" "loop_generate_construct")
  "The two verilog generate-construct HEADER node kinds -- see
`indent--verilog-generate-depth-adjust' (M119 D2).")

(defun indent--verilog-generate-construct-ancestor-count (node)
  "Count of NODE's ancestors (via `treesit-node-parent', INCLUSIVE of
NODE itself) whose `treesit-node-type' is a member of
`indent--verilog-generate-construct-types' -- i.e. how many nested
if/for-generate HEADER nodes NODE sits inside (or is itself). Used by
`indent--verilog-generate-depth-adjust' (M119 D2, fix round) to size
the nesting correction below: unlike `indent--block-depth', this walks
the FULL ancestor chain rather than stopping at the first match, since
D2 needs to know how many levels of generate-if/for nesting are in
play, not merely whether there is at least one."
  (let ((n node) (count 0))
    (while n
      (when (member (treesit-node-type n) indent--verilog-generate-construct-types)
        (setq count (1+ count)))
      (setq n (treesit-node-parent n)))
    count))

(defun indent--nearest-ancestor-of-types (node types)
  "Nearest ancestor of NODE (via `treesit-node-parent', INCLUSIVE of
NODE itself) whose `treesit-node-type' is a member of TYPES (a list of
strings), or nil if no ancestor up to the root matches. Used by
`indent--verilog-generate-depth-adjust' (M119 D2) alongside
`indent--verilog-generate-construct-ancestor-count' to get the nearest
generate-construct ancestor without duplicating that function's own
counting walk."
  (let ((n node))
    (while (and n (not (member (treesit-node-type n) types)))
      (setq n (treesit-node-parent n)))
    n))

(defun indent--verilog-generate-depth-adjust (node)
  "M119 D2 (fix round): correction for NODE's raw block-depth inside
nested `if_generate_construct'/`loop_generate_construct' HEADER and
BODY lines (`indent--verilog-generate-construct-types'), covering both
the unwrapped (module-scope, no `generate'/`endgenerate') and the
wrapped form, and any depth of NESTING within either.

0 when NODE is not inside a generate-if/for at all
\(`indent--verilog-generate-construct-ancestor-count' is 0). Otherwise:

  -(COUNT - 1) - (0 if wrapped, 1 if unwrapped)

where COUNT is `indent--verilog-generate-construct-ancestor-count' and
\"wrapped\" means the OUTERMOST generate-construct ancestor (equally,
any of them: see below for why they always agree) has a
`generate_region' ancestor above it.

Real `verible-verilog-format --indentation_spaces=2' measured all FOUR
combinations directly (M119 fix round, reproducing the ticket's own
fixture): unwrapped single-level (`if (P) begin : g_yes' col 2, `wire
x;' col 4), wrapped single-level (col 4 / col 6 for the same shapes --
already covered before this fix round, see
`verilog_generate_for_loop_body_indents_and_endgenerate_aligns_with_generate'),
and -- the fix round's new material -- NESTED unwrapped (`if (P)' col
2, nested `if (Q)' col 4, `wire x;' col 6) and NESTED wrapped (`if (P)'
col 4, nested `if (Q)' col 6, `wire x;' col 8; `generate' itself col
2). See `verilog_nested_unwrapped_if_generate_is_two_levels_shallower_
than_the_doubly_wrapped_form' and its wrapped sibling in
indent_tests.rs for the pinned fixtures and columns.

Why COUNT - 1, not a flat -1 regardless of nesting: dump-verified
\(M119 fix round, same `(treesit-node-string root)' method as the
original D2) that a NESTED generate-if's own HEADER node
\(`if_generate_construct') is a descendant of the OUTER construct's own
`generate_block' (its begin/end body) -- so `indent--block-depth'
counts BOTH the outer's `generate_block' (the body level the outer
construct's contents legitimately get) AND the inner's own
`if_generate_construct' (that inner header's own level) for what is
structurally the SAME one-level step down from the outer header to the
inner one -- exactly the kind of self-referential double count this
file's header spends several paragraphs on for `case_statement'/
`module_declaration', just one level removed: nesting two generate-ifs
directly (no ordinary statement between them) makes the inner header's
raw depth TWO deeper than the outer's, when real verible only goes ONE
deeper. This is independent of wrap/unwrap (confirmed: the SAME
over-count reproduces with the doubly-wrapped fixture above, using
`generate'/`endgenerate' around both levels) -- FIX-3, folded into this
same function since it is the identical mechanism, not a different bug.
Each additional generate-construct ancestor beyond the first
contributes exactly one such spurious double-count, hence `- (COUNT -
1)', on top of (not instead of) the original wrap/unwrap correction.

Why checking only the outermost (or, equivalently, ANY) construct
ancestor's wrapped-ness is safe: a `generate_region' ancestor, once
present, is inherited by EVERY node nested inside it (ordinary
ancestor-chain inclusion) -- there is no grammar shape where an outer
generate-if is unwrapped but something nested inside its own body
gains a `generate_region' ancestor the outer one lacks, or vice versa.
So every generate-construct ancestor of NODE has the same wrapped-ness,
and checking any single one of them (here, the nearest, i.e. the first
one found walking up from NODE) answers it for all of them."
  (let ((count (indent--verilog-generate-construct-ancestor-count node)))
    (if (= count 0)
        0
      (let ((nearest (indent--nearest-ancestor-of-types
                       node indent--verilog-generate-construct-types)))
        (- (+ (1- count)
              (if (indent--node-has-ancestor-type-p
                   (treesit-node-parent nearest) "generate_region")
                  0
                1)))))))

(defun indent--verilog-header-wrap-depth-adjust (node)
  "M119 D1 (extended M124 to also cover `list_of_ports'): -1 when NODE is
a descendant of (or is itself) a `parameter_port_list',
`list_of_port_declarations', or `list_of_ports' -- i.e. a continuation
line inside a module/interface/program's own wrapped ANSI OR non-ANSI
header
list (`#(...)' parameter list or the port list that follows). Real
`verible-verilog-format --indentation_spaces=2' computes such a line's
column as the header's OWN on-disk column (0 for a top-level module)
plus the fixed wrap step (`indent-wrap-width', 4) -- NOT the module
body's block-depth-derived column (2, one level: `module_declaration'
alone, the SAME already-documented header-line one-level quirk this
file's header describes -- a header line and the module's first real
body line both compute block-depth 1, even though their real on-disk
columns differ) plus the wrap step, which would double that quirk into
the wrap and land two columns too deep (block_depth 1 * width 2 +
wrap_depth 1 * 4 = 6, not verible's measured 4). This cancels exactly
the `module_declaration'/`interface_declaration' block-depth
contribution `parameter_port_list'/`list_of_port_declarations''s own
ancestor chain otherwise carries, so the two axes combine to the
observed 0 + wrap = 4 instead. Does not affect the module body's own
lines (`wire w;' etc -- no `parameter_port_list'/
`list_of_port_declarations' ancestor) or the header's own opening line
(`module top #(' -- its query position sits before either list node's
span starts, the same reasoning `indent--block-node-types''s enum/
struct comment gives for why `cover_point'/`data_type' themselves are
excluded rather than added)."
  (if (or (indent--node-has-ancestor-type-p node "parameter_port_list")
          (indent--node-has-ancestor-type-p node "list_of_port_declarations")
          ;; M124: `list_of_ports' is the non-ANSI counterpart -- same
          ;; module/interface/program_*_header ancestor-chain shape as
          ;; `list_of_port_declarations' above, so it needs the exact
          ;; same block-depth cancellation.
          (indent--node-has-ancestor-type-p node "list_of_ports"))
      -1
    0))

(defun indent--verilog-modport-item-closer-adjust (node)
  "M122: +1 when NODE literally IS `modport_item''s own closing `)'.
Pairs with the `(\")\" . \"modport_item\")' contextual entry in
`indent--verilog-closers', which -- like every entry in that list --
fires the SHARED closer-dedent in `indent--query-pos-and-depth' and
decrements BOTH DEPTH and WRAP-DEPTH by one. That blanket DEPTH
decrement is correct for the pre-existing `parameter_port_list'/
`list_of_port_declarations' entries (D1 above already cancelled THEIR
block-depth contribution down to the header's own natural column for
every line inside them, content and closer alike, so the closer's
further -1 on DEPTH is a harmless no-op clamped at 0) but wrong here:
`modport_item' is deliberately NOT a block-node-type (see
`indent--wrap-node-types''s own M122 comment -- a modport's content
lines need the INTERFACE's real, unadjusted block depth plus the wrap
step, not a cancelled-to-zero one), so DEPTH for a line inside
`modport_item' is already correct as-is, matching `modport master('
's own header line (dump/engine-verified, M122: both compute
interface_declaration's real depth, 1 level). Left uncorrected, the
shared closer's blanket -1 would double-dedent this one line (to 0,
one level shallower than the header it must match) while correctly
zeroing WRAP-DEPTH (1 -> 0, cancelling the wrap step, which IS wanted
-- the closing `)' should land at the header's own column, not the
wrapped-content column). This function's own +1 exactly cancels that
one unwanted DEPTH decrement, leaving WRAP-DEPTH's decrement as the
only net effect."
  (if (and (equal (treesit-node-type node) ")")
           (let ((parent (treesit-node-parent node)))
             (and parent (equal (treesit-node-type parent) "modport_item"))))
      1
    0))

(defun indent--verilog-bare-action-block-adjust (node)
  "M122 review fix: +1 when NODE sits inside an `action_block''s own
braceless `else' arm (`assert (...) else STATEMENT;', no `begin'/`end')
AND that STATEMENT is itself wrapped onto multiple lines (a long
`$error(...)' call, real material: `demo/verif/sram_bank_tb.sv''s
`assert (ok) else $error(...)' inside two nested `for' loops). Dump-
verified: the shape is `action_block' -> `statement_or_null' ->
`statement' -> `statement_item' -> (the bare statement, e.g.
`subroutine_call_statement' for `$error(...)') -- FLAT, no `seq_block'
anywhere in that chain (`else begin ... end' inserts one; `else
STATEMENT;' does not). Neither `statement_or_null' nor `action_block'
is a block-node-type (`statement_or_null' also wraps every ordinary
if/else branch and every statement inside a `seq_block' itself, so
adding it unconditionally would double-count practically everything in
this grammar), so this walks up looking specifically for an
`action_block''s own `statement_or_null' with no `seq_block' in
between NODE and it -- if one IS found along the way first, the
branch is the `begin'/`end' form, already correctly counted by
`seq_block' unconditionally being a block type elsewhere in this list,
and this contributes nothing (0) rather than double-counting.
Measured against real `verible-verilog-format --indentation_spaces=2':
with `assert'/`else' at column 8 (inside two nested `for' loops), the
wrapped call's own opening line (`$error(') sits at column 10 (one
level in, exactly what this function's +1 produces), each argument at
column 14 (the pre-existing `list_of_arguments' wrap step on top), and
the closing `);' back at column 10."
  (let ((n node) (bare nil) (stop nil))
    (while (and n (not stop))
      (let ((type (treesit-node-type n)))
        (cond
          ((equal type "seq_block") (setq stop t))
          ((and (equal type "statement_or_null")
                (let ((parent (treesit-node-parent n)))
                  (and parent (equal (treesit-node-type parent) "action_block"))))
           (setq bare t)
           (setq stop t))))
      (unless stop (setq n (treesit-node-parent n))))
    (if bare 1 0)))

(defvar indent--verilog-comment-wrap-sibling-types
  '("list_of_port_connections" "list_of_parameter_value_assignments"
    "list_of_arguments")
  "M127: wrap-node-types (see `indent--wrap-node-types') whose leading
comment(s), when the comment is the wrapping production's own child
(a SIBLING of the list node) rather than a descendant of the list node
itself, still need the same wrap step every other line inside the list
gets. Deliberately excludes `parameter_port_list'/`list_of_port_
declarations'/`list_of_ports'/`modport_item' -- dump-verified (M127
spec) that a module's own ANSI/non-ANSI header list nests its leading
comment(s) as a genuine descendant of the list node already, so those
four were already correct before this fix and must not be touched by it.

This check inspects the WRAP NODE's own type (`list_of_port_connections'/
`list_of_parameter_value_assignments'/`list_of_arguments'), not its
PARENT's type, so it generalises correctly to every grammar production
that happens to wrap one of these three list types -- not only the three
named as the motivating example in `indent--verilog-comment-before-wrap-
node-adjust''s own docstring (`hierarchical_instance', the `#(...)'
production, `tf_call'). `list_of_arguments' in particular is reused by
many more productions than a plain task/function call -- `system_tf_call'
(`$display(...)'), `class_new', ordinary method calls, `randomize()' and
others -- and a leading comment inside any of THEIR wrapped argument
lists gets the same fix for the same reason, with no extra code needed:
whichever production wraps `list_of_arguments', the comment's own walk
(skipping over any further leading comments -- see
`indent--verilog-comment-before-wrap-node-adjust') still lands on
`list_of_arguments' itself.")

(defun indent--treesit-next-sibling (node)
  "NODE's immediate next sibling by child index within its own PARENT, or
nil if NODE is the last child (or has no parent). This engine's Rust-side
`treesit-*' surface (M12/M39, `crates/core/src/builtins/treesit.rs') does
not implement GNU Emacs's `treesit-node-next-sibling' -- only
`treesit-node-parent'/`treesit-node-child'/`treesit-node-child-count'/
`treesit-node-eq' -- so this reconstructs it: find NODE's own index by
scanning PARENT's children with `treesit-node-eq' (identity, not type or
position, so it cannot be fooled by two same-typed nodes at different
positions), then return the child one past it. M127 dump-verified
(`hierarchical_instance', the `#(...)' production, and `tf_call', all
three) that no anonymous punctuation token ever sits between a wrap
list's leading comment and the wrap node that follows it -- the child
immediately after the comment BY INDEX already IS the wrap node itself,
not `(' or some other token -- so a plain by-index neighbor is exactly
the right answer here, without needing to first classify which children
are \"named\" the way real Emacs's version does."
  (let ((parent (treesit-node-parent node)))
    (if (not parent)
        nil
      (let ((count (treesit-node-child-count parent)) (k 0) (found nil))
        (while (and (< k count) (not found))
          (if (treesit-node-eq node (treesit-node-child parent k))
              (setq found k)
            (setq k (1+ k))))
        (if (and found (< (1+ found) count))
            (treesit-node-child parent (1+ found))
          nil)))))

(defun indent--verilog-comment-after-skipping-comments (node)
  "Walk forward from NODE (a comment) over `indent--treesit-next-sibling'
links, skipping every further comment (`one_line_comment'/
`block_comment') sibling, and return the first NON-comment sibling
found (or nil if NODE is the last child, or every remaining sibling is
itself a comment). Used by `indent--verilog-comment-before-wrap-node-
adjust' to see past a RUN of consecutive leading comments to whatever
substantive node actually follows them."
  (let ((n (indent--treesit-next-sibling node)))
    (while (and n (member (treesit-node-type n) '("one_line_comment" "block_comment")))
      (setq n (indent--treesit-next-sibling n)))
    n))

(defun indent--verilog-comment-before-wrap-node-adjust (node)
  "M127: +1 (a wrap step) when NODE is a comment (`one_line_comment' or
`block_comment') and the first NON-comment sibling reachable by walking
forward from NODE (`indent--verilog-comment-after-skipping-comments',
which itself skips over any further leading comments) has a
`treesit-node-type' that is a member of
`indent--verilog-comment-wrap-sibling-types'.

Root cause (dump-verified, M127 spec): a wrapped list's leading
comment(s) -- e.g. the `// Outputs' line immediately after `sub_module
u_sub (', before any `.port(sig)' connection -- are direct children of
the WRAPPING production (`hierarchical_instance' for a port-connection
list, the `#(...)' production for a parameter-value-assignment list,
`tf_call' for a call's argument list -- see `indent--verilog-comment-
wrap-sibling-types' for why this generalises past those three examples),
SIBLINGS of the list node (`list_of_port_connections' etc), not
descendants of it. Every OTHER comment or real line inside the list --
including a comment that appears AFTER at least one real
connection/argument, like a later `// Inputs' -- sits between two of the
list's own named children and so IS a genuine descendant, already
correctly picked up by the ordinary `indent--block-depth' ancestor walk
against `indent--wrap-node-types' in `indent--query-pos-and-depth'. Only
the leading comment(s)' own ancestor chain skips the wrap node entirely,
computing WRAP-DEPTH 0 instead of 1 (module block depth alone: column 2,
not the wrapped list's column 6).

Fix-round finding (trailing review): checking only the IMMEDIATE next
sibling handles a single leading comment but not a RUN of two or more --
dump-verified (fix round) that consecutive comments are spliced in as
ordinary positional siblings of one another (`(one_line_comment)
(one_line_comment) (list_of_port_connections ...)'). For a two-comment
run, the LAST comment's immediate next sibling already is the wrap node
(handled correctly before this fix-round change), but every EARLIER
comment's immediate next sibling is the NEXT comment in the run, not the
wrap node -- so checking only the immediate neighbor left every comment
but the last one in a run undetected and stuck at block depth, the exact
bug this fix exists to remove, one (or more) comment lines earlier.
`indent--verilog-comment-after-skipping-comments' fixes this by walking
PAST every further comment sibling to find the actual substantive
neighbor, so every comment in the run -- not just the last -- now sees
the same wrap node and gets the same +1.

Restricted to exactly the three types in `indent--verilog-comment-wrap-
sibling-types' (not the module-header wrap types) because those are
dump-verified (M127 spec) to have the OPPOSITE shape: a header list's
own leading comment(s) already ARE a descendant of the list node itself,
so adding a step here would double-count them.

Must NOT fire for a comment (or a run of comments) sitting ABOVE an
entire instantiation statement (outside its parens) -- dump-verified
(M127) that such a comment's own next sibling by index is
`module_instantiation' itself (the comment is a direct child of
`module_declaration', sitting between `module_ansi_header' and
`module_instantiation'), not one of the three wrap-node-types checked
here, so this function correctly returns 0 for it (and, via the same
comment-skipping walk, for every comment in a multi-line run above the
instantiation) and it stays at the module's own block depth. The check
is a plain `treesit-node-type' membership test on the final sibling's
own type, not a descendant walk, so no ancestor-chain shape this
milestone was not built for can accidentally satisfy it."
  (if (and (member (treesit-node-type node) '("one_line_comment" "block_comment"))
           (let ((next (indent--verilog-comment-after-skipping-comments node)))
             (and next
                  (member (treesit-node-type next)
                          indent--verilog-comment-wrap-sibling-types))))
      1
    0))

(defun indent--verilog-depth-adjust (node)
  "M119/M122: sum of `indent--verilog-generate-depth-adjust' (D2),
`indent--verilog-header-wrap-depth-adjust' (D1),
`indent--verilog-modport-item-closer-adjust' (M122), and
`indent--verilog-bare-action-block-adjust' (M122 review fix) -- the
single DEPTH-ADJUST-FN `verilog-indent-line' passes to
`indent--treesit-depth-column'/`indent--query-pos-and-depth', which
only accept one.

These are NOT mutually exclusive, and summing (rather than picking one)
is what makes the overlapping case come out right -- an earlier version
of this docstring claimed they never overlap, which a second review
round (M122) disproved with an ordinary real shape: a `case' whose
selector is a wrapped `{...}' concatenation, itself inside a generate-if
(a generated case-mux is completely unremarkable RTL). There,
`indent--verilog-generate-depth-adjust' (D2) fires on every line of the
`case' (all of it sits inside the generate-if), and the SAME closing
`})' line separately triggers the plain `(\"}\" . \"concatenation\")'
closer in `indent--verilog-closers' -- two different mechanisms
(DEPTH-ADJUST-FN's sum, and the closer's own separate -1 in
`indent--query-pos-and-depth') both touching the same line's DEPTH, not
a bug. Measured directly against real `verible-verilog-format
--indentation_spaces=2':
  generate\\n  if (1) begin : g\\n    always_comb begin\\n      case ({\\n
  a, b\\n      })\\n ... endcase\\n    end\\n  endgenerate
puts the wrapped members at column 10 and the closing `})' at column 8
-- both reproduced exactly by this engine (pinned by
`verilog_case_selector_concatenation_inside_a_generate_if_composes_
correctly' in indent_tests.rs), because D2's contribution and the
closer's -1 apply at two different stages of the SAME computation
(`indent--query-pos-and-depth' sums DEPTH-ADJUST-FN into DEPTH first,
THEN applies the closer's -1 on top) rather than competing for one slot."
  (+ (indent--verilog-generate-depth-adjust node)
     (indent--verilog-header-wrap-depth-adjust node)
     (indent--verilog-modport-item-closer-adjust node)
     (indent--verilog-bare-action-block-adjust node)))

(defun indent--query-pos-and-depth (lang-sym block-types closers &optional wrap-types depth-adjust-fn wrap-depth-adjust-fn)
  "(QUERY-POS DEPTH WRAP-DEPTH) for the current line under LANG-SYM, or
nil if a tree-sitter ERROR node is encountered. QUERY-POS is the
position actually queried: the current line's own first non-blank
character, or -- when the line is blank -- the nearest real character
before it (see this file's header, the MISSING-token trap). DEPTH
already has the closing-token dedent applied, based on the tree-sitter
NODE at QUERY-POS -- uniformly: whether QUERY-POS is this line's own
leading closer or the last real character of a PRECEDING line that
happens to END with one, either way it marks the end of a block, and
content at or after it belongs one level out. (This is what makes a
genuinely blank line between two top-level closing braces correctly
compute 0, not 1: its fallback query position IS that previous closer,
and the same dedent rule applies to it.) The dedent only fires when
QUERY-POS's own tree-sitter NODE literally IS one of CLOSERS
(`indent--closer-token-at-p') -- not just when the buffer text happens
to match -- so a closing token's ordinary TEXT appearance inside a
comment or string literal never falsely triggers it.

WRAP-DEPTH (M90) is `indent--block-depth' of the SAME NODE against
WRAP-TYPES (`indent--wrap-node-types'). For `list_of_port_connections'/
`list_of_parameter_value_assignments'/`list_of_arguments', a wrapped
list's closing `)'/`);' line is a SIBLING of the list node, not a
descendant (see `indent--wrap-node-types'' comment), so it is simply
never counted by this walk in the first place -- no dedent rule was
needed for those three, through M118. 0 when WRAP-TYPES is nil (the
caller's language has no wrap-node-type list) or absent, without
walking the tree a second time for languages that don't need it. This
`(and depth wrap-types ...)' short-circuit is true by inspection but
UNWATCHED BY THE TEST SUITE: for any language whose WRAP-TYPES is nil,
`indent--block-depth' called directly on an empty type list already
returns 0 by itself (nothing ever matches `member'), so deleting the
short-circuit would still compute the correct answer for those five
languages -- no test can distinguish `(and depth wrap-types
(indent--block-depth ...))' from an unconditional call here, because
both produce the same observable column. The short-circuit exists
purely to skip a wasted ancestor walk, not to change any language's
answer; that performance claim is not covered by mutation testing and
is recorded as such rather than implied to be.

M119 D1 breaks the closing-line-is-always-a-sibling assumption above:
dump-verified (M119, ancestor probe on a real multi-parameter/multi-port
module header) that `parameter_port_list'/`list_of_port_declarations''s
own closing `)' IS a genuine CHILD of the list node (`(source_file
module_declaration module_ansi_header parameter_port_list \")\")' for
the `) (' line between a wrapped `#(...)' and the port list that follows
it, and the same shape with `list_of_port_declarations' for the final
`);' line) -- unlike the other three wrap-types, whose grammar rules
emit the parens as the WRAPPING production's own children, this
grammar's ANSI header production nests the closing paren inside the
list itself. Left alone, that closing line would wrongly get a wrap
step (column 4 instead of the header's own real column, usually 0) on
top of `indent--verilog-header-wrap-depth-adjust' already cancelling
DEPTH's block-depth contribution -- so below, the SAME closer-dedent
that fires for DEPTH also dedents WRAP-DEPTH by one when it fires,
harmless for every pre-existing closer (none of them are ever a
descendant of a WRAP-TYPES node in the first place, so WRAP-DEPTH was
already 0 for all of them and stays 0) and exactly correcting this one.

DEPTH-ADJUST-FN (M119, optional) is called with NODE (the SAME node
DEPTH and WRAP-DEPTH are computed from) when non-nil, and its return
value (an integer, usually -1/0/1) is added to DEPTH before the closer
dedent is applied, clamped at 0. Only verilog uses this, for
`indent--verilog-generate-depth-adjust' (D2): a flat ancestor-type-
membership count has no way to make `if_generate_construct''s own
contribution CONDITIONAL on whether a `generate_region' sits above it,
so this hook lets a language supply that context-sensitive correction
without changing the DEPTH-computation's meaning for any other
language (nil here is a no-op, verified the same way the WRAP-TYPES
short-circuit above is: no test can distinguish `(if (and depth
depth-adjust-fn) ...)' from an unconditional call when DEPTH-ADJUST-FN
is nil, since `funcall' on nil never happens either way).

WRAP-DEPTH-ADJUST-FN (M127, optional) is the same shape as DEPTH-ADJUST-FN
but adds its result to WRAP-DEPTH instead of DEPTH, before the closer
dedent below is applied, clamped at 0. Only verilog uses this, for
`indent--verilog-comment-before-wrap-node-adjust' -- see that function's
own doc for why WRAP-DEPTH (an ancestor-chain walk against WRAP-TYPES,
computed above) has no way to see a wrap list's own leading comment,
which sits as that list's SIBLING rather than its descendant. nil here is
a no-op the same way DEPTH-ADJUST-FN's nil is."
  (let* ((line-pos (indent--first-non-blank-pos))
         (blank (= line-pos (line-end-position)))
         (query-pos (if blank (or (indent--prev-nonblank-char-pos) line-pos) line-pos))
         (parser (treesit-parser-create lang-sym))
         (node (treesit-node-at query-pos parser)))
    ;; M122 review fix: a conditional-compilation directive
    ;; (`` `ifndef ``/`` `endif ''/etc, verilog's
    ;; `conditional_compilation_directive') always sits at column 0 in
    ;; real `verible-verilog-format', REGARDLESS of how deeply nested
    ;; the surrounding block structure is -- dump/engine-verified (M122):
    ;; a `` `ifndef ``/`` `endif '' pair two levels deep (inside a
    ;; `generate_block' inside an `if_generate_construct') still lands
    ;; at column 0 against real verible, not one column per nesting
    ;; level like an ordinary block-type node would. This is UNLIKE the
    ;; `` `define '' macro-body exemption this file used to lean on for
    ;; the same construct family (a `` `define '' body genuinely
    ;; collapses into one opaque, structureless leaf token to
    ;; tree-sitter -- dump-verified separately -- so there was nothing
    ;; to give a column to in the first place): a directive line IS a
    ;; clean, distinctly-named, ordinary sibling node
    ;; (`conditional_compilation_directive'), just one whose real column
    ;; is always 0 rather than following the surrounding block depth.
    ;; Checked language-agnostically (no other bundled grammar has this
    ;; node type, so this is a no-op everywhere else) rather than
    ;; threading it through BLOCK-TYPES/WRAP-TYPES/DEPTH-ADJUST-FN, which
    ;; would need `depth' itself passed into DEPTH-ADJUST-FN to cancel
    ;; back to exactly 0 regardless of nesting depth (those three only
    ;; ever add/subtract a FIXED small integer, not reset to a constant).
    (if (indent--node-has-ancestor-type-p node "conditional_compilation_directive")
        (list query-pos 0 0)
      (let* ((depth (indent--block-depth node block-types))
             (depth (if (and depth depth-adjust-fn)
                        (max 0 (+ depth (funcall depth-adjust-fn node)))
                      depth))
             (wrap-depth (and depth wrap-types (indent--block-depth node wrap-types)))
             (wrap-depth (if (and wrap-depth wrap-depth-adjust-fn)
                              (max 0 (+ wrap-depth (funcall wrap-depth-adjust-fn node)))
                            wrap-depth)))
        (when depth
          (when (indent--closer-token-at-p node closers)
            (setq depth (max 0 (1- depth)))
            (when wrap-depth (setq wrap-depth (max 0 (1- wrap-depth)))))
          (list query-pos depth (or wrap-depth 0)))))))

(defvar indent-treesit-max-chars 300000
  "Buffers larger than this many characters skip the tree-sitter block-
depth engine entirely and fall back to `indent--copy-previous-
indentation' -- the SAME fallback path a genuine parse ERROR already
uses (see `indent--treesit-depth-column'). M36 review fix (severity
medium-high), against the cost model at the time: a full fresh reparse
ran on every single TAB/RET/o/O, measured at 2-6ms per 50k characters,
so linearly extrapolated this ceiling capped the worst case around
20-40ms, still inside a keystroke's budget; with no ceiling at all, a
pathologically large buffer could turn every keystroke into a
multi-second stall. That measurement is stale: M108/M109 (see this
file's header) made most TAB presses an O(1) cache hit or an
incremental `Tree::edit'-based reparse instead of a full reparse, so
the worst case this ceiling was sized against no longer happens on
every keystroke the way it used to. Nobody has re-measured the actual
worst case under the new cost model. A first parse of a huge buffer (or
a language switch) still walks the old from-scratch path
(`treesit.rs''s `parse'/`parse_text'); a huge single EDIT does not --
it goes through `incremental_parse' (`Tree::edit' plus tree-sitter's own
incremental reparse), and only falls back to the from-scratch path via
the rare defensive length-mismatch check inside `incremental_parse'.
So the worst-case-cost-of-a-huge-edit question this ceiling was sized
against is not obviously answered by M108/M109 either way -- `treesit.rs''s
module doc's `8000-arm-case' benchmark row shows the incremental gain
shrinking to 1.6x on a big-enough buffer even along the normal
(non-fallback) path -- so the number is left as-is rather than
re-chosen without data.
Buffer-local (`setq-local') override if a specific large file's parse
is still fast enough to be worth keeping smart indentation for.")

(defun indent--treesit-depth-column (lang-sym closers &optional depth-adjust-fn wrap-depth-adjust-fn)
  "Target column for the current line under LANG-SYM using the block-
depth heuristic, or nil (caller falls back to `indent--copy-previous-
indentation') on an ERROR tree or an oversized buffer -- see
`indent--query-pos-and-depth' and `indent-treesit-max-chars'. M90:
`block_depth * standard-indent-width + wrap_depth * indent-wrap-width'
-- two independently-sized axes (see `indent--wrap-node-types'), not
one; WRAP-DEPTH is always 0 for a language with no
`indent--wrap-node-types' entry, so this is unchanged from before M90
for every language except verilog. DEPTH-ADJUST-FN (M119, optional) is
passed straight through to `indent--query-pos-and-depth' -- see its own
docstring; nil (the default, every caller except `verilog-indent-line')
is a no-op. WRAP-DEPTH-ADJUST-FN (M127, optional) is likewise passed
straight through -- see `indent--query-pos-and-depth''s own docstring."
  (if (> (point-max) indent-treesit-max-chars)
      nil
    (let* ((block-types (cdr (assq lang-sym indent--block-node-types)))
           (wrap-types (cdr (assq lang-sym indent--wrap-node-types)))
           (r (indent--query-pos-and-depth
               lang-sym block-types closers wrap-types depth-adjust-fn
               wrap-depth-adjust-fn)))
      (when r
        (+ (* (nth 1 r) standard-indent-width)
           (* (nth 2 r) indent-wrap-width))))))

(defun c-indent-line ()
  (or (indent--treesit-depth-column 'c indent--c-like-closers)
      (indent--copy-previous-indentation)))

(defun c++-indent-line ()
  (or (indent--treesit-depth-column 'cpp indent--c-like-closers)
      (indent--copy-previous-indentation)))

(defun java-indent-line ()
  (or (indent--treesit-depth-column 'java indent--c-like-closers)
      (indent--copy-previous-indentation)))

(defun rust-indent-line ()
  (or (indent--treesit-depth-column 'rust indent--c-like-closers)
      (indent--copy-previous-indentation)))

(defun emacs-lisp-indent-line ()
  (or (indent--treesit-depth-column 'elisp indent--elisp-closers)
      (indent--copy-previous-indentation)))

(defun verilog-indent-line ()
  (or (indent--treesit-depth-column
       'verilog indent--verilog-closers #'indent--verilog-depth-adjust
       #'indent--verilog-comment-before-wrap-node-adjust)
      (indent--copy-previous-indentation)))

;; --- Python: textual heuristic (not a tree walk -- see this file's header)

(defconst indent--python-dedent-keywords '("else" "elif" "except" "finally")
  "A leading word among these dedents the CURRENT line by one width
relative to the base line -- see `python-indent-line'.")

(defun indent--strip-comment (s)
  "S with a trailing #-comment (if any) removed, naively: the first `#'
found anywhere starts the comment -- doesn't understand string
literals, so a `#' inside a Python string is (incorrectly) treated as a
comment start. A documented v1 simplification of this already-
heuristic engine (see this file's header)."
  (let ((i 0) (n (length s)) (hash nil))
    (while (and (< i n) (not hash))
      (when (= (aref s i) ?#) (setq hash i))
      (setq i (1+ i)))
    (if hash (substring s 0 hash) s)))

(defun indent--line-text (pos)
  "Text of the line POS is on, without its trailing newline."
  (save-excursion
    (goto-char pos)
    (buffer-substring (line-beginning-position) (line-end-position))))

(defun indent--ends-with-colon-p (line-text)
  "Non-nil if LINE-TEXT, after stripping a trailing #-comment and
surrounding whitespace, ends with `:'."
  (let ((trimmed (string-trim (indent--strip-comment line-text))))
    (and (> (length trimmed) 0)
         (= (aref trimmed (1- (length trimmed))) ?:))))

(defun indent--first-word-at (pos)
  "The leading run of ASCII letters starting at the first non-blank
character of the line POS is on, or \"\" if that line is blank or
starts with a non-letter."
  (save-excursion
    (goto-char pos)
    (let ((start (indent--first-non-blank-pos)) (end nil))
      (setq end start)
      (while (and (< end (line-end-position))
                  (let ((c (char-after end)))
                    (and c (or (and (>= c ?a) (<= c ?z)) (and (>= c ?A) (<= c ?Z))))))
        (setq end (1+ end)))
      (buffer-substring start end))))

(defun python-indent-line ()
  "Target column for the current line -- a documented v1 HEURISTIC, not
a real parse (see this file's header for why). BASE = the nearest
non-blank line above's own indentation:
 - This line's own first word is a dedent keyword (else/elif/except/
   finally): BASE - `standard-indent-width' (floored at 0) -- checked
   FIRST, so it wins even in the (practically nonsensical) case where
   the base line also happens to end in `:'.
 - Else, the base line ends in `:' (trailing comment/whitespace
   ignored): BASE + `standard-indent-width'.
 - Otherwise: BASE unchanged.
Only ever ONE level of dedent (see this file's header)."
  (let ((prev (indent--prev-nonblank-line-start)))
    (if (not prev)
        0
      (let* ((base (indent--line-indentation-at prev))
             (first-word (indent--first-word-at (line-beginning-position))))
        (cond
          ((member first-word indent--python-dedent-keywords)
           (max 0 (- base standard-indent-width)))
          ((indent--ends-with-colon-p (indent--line-text prev))
           (+ base standard-indent-width))
          (t base))))))

;; --- bash/perl: copy the previous line's indentation (v1 -- see header) --

(defun sh-indent-line () (indent--copy-previous-indentation))
(defun perl-indent-line () (indent--copy-previous-indentation))

;; --- Commands: TAB / RET ---------------------------------------------------

(defun indent-for-tab-command ()
  "The global TAB command (M36). With a buffer-local `indent-line-
function' (set by the prog-mode major modes; see modes.el), recompute
the current line's indentation and apply it, then place point per the
GNU convention: within the line's OLD leading indentation (or exactly
at its end) moves to the NEW indentation's end; within the line's TEXT
keeps point's offset from the text unchanged. With no `indent-line-
function' (text-mode, fundamental-mode, ...), inserts a literal tab
character -- the same behavior every buffer had before M36."
  (interactive)
  (if (not indent-line-function)
      (insert "\t")
    (let* ((old-end (indent--first-non-blank-pos))
           (in-text (> (point) old-end))
           (offset (- (point) old-end)))
      (indent-line-to (or (funcall indent-line-function) 0))
      (when in-text
        (goto-char (+ (indent--first-non-blank-pos) offset))))))

(defun newline-and-indent ()
  "Insert a newline, then indent the new (now current) line -- see
`indent-current-line-if-supported'."
  (interactive)
  (newline)
  (indent-current-line-if-supported))

(global-set-key "TAB" 'indent-for-tab-command)

;; Only programming-language buffers get RET rebound to auto-indent the
;; new line -- see this file's header for the full reasoning (why org/
;; dired/eshell/ielm/text-mode are all unaffected). Buffer-local, added
;; from `prog-mode-hook' alongside modes.el's own display-line-numbers
;; hook, so a user's `(remove-hook 'prog-mode-hook ...)' or a later
;; `(local-set-key "RET" ...)' in a mode's own hook still wins.
(add-hook 'prog-mode-hook (lambda () (local-set-key "RET" 'newline-and-indent)))

(provide 'indent)
