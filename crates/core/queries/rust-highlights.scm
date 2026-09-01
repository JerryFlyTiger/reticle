;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-rust's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2017 Maxim Sokolov
;; Upstream: https://github.com/tree-sitter/tree-sitter-rust
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from tree-sitter-rust's `HIGHLIGHTS_QUERY` toward real
;; rust-ts-mode conventions (the tree-sitter-backed major mode built into
;; GNU Emacs 29+, the closest thing Rust has to a canonical GNU "mode"):
;; only a definition's own name gets a face, nothing is guessed from a
;; name's spelling.
;;
;; Deleted from upstream: the catch-all `(identifier) @variable`, the
;; Capitalized-is-a-`@constructor` and ALL_CAPS-is-a-`@constant`
;; heuristics (both of them -- plain identifiers *and* scoped/path
;; identifiers each had their own copy), every call-site rule
;; (`call_expression`, `generic_function`, both `function.method`
;; shapes), the `macro_invocation` rule (so `println!(...)` is not
;; colored -- call-site philosophy applies to macro invocations too),
;; `(self) @variable.builtin` (a receiver, dropped for the same reason
;; Python's `self` is -- *not* the same as `self`/`super`/`crate` in a
;; `use` path below, a genuinely different, keyword-shaped grammatical
;; position that upstream already treated separately), `(lifetime
;; (identifier) @label)`, `@punctuation.bracket`/`@punctuation.delimiter`,
;; bare `@escape`, `attribute_item`/`inner_attribute_item`
;; (`#[derive(...)]` -- not mentioned by the M33 plan, left unstyled),
;; the two `@comment.documentation` sub-rules (redundant once the plain
;; `@comment` rule below already covers a doc comment -- it's still just
;; a `line_comment`/`block_comment` node), and the symbolic `@operator`
;; list.
;;
;; Kept or changed to GNU semantics:
;;   - fn name -> `@function` (both a full `function_item` and a
;;     body-less `function_signature_item`, i.e. a trait method
;;     signature -- upstream's own positional pattern, not anchored to
;;     the `name:` field, but verified against
;;     `crates/core/examples/dump_tree.rs`, a one-time research tool
;;     since deleted, to only ever match the actual function name in
;;     practice).
;;   - `(type_identifier) @type` is unchanged from upstream, and (unlike
;;     const/static below) that's *why* struct/enum/trait names need no
;;     dedicated rule of their own here: all three name their own tag as
;;     a `type_identifier`, so the blanket rule already colors them for
;;     free.
;;   - const/static name -> `@constant.builtin`: new, and genuinely
;;     needed (unlike struct/enum/trait) since a `const`/`static` names
;;     itself with a plain `identifier`, not a `type_identifier`.
;;     rust-ts-mode's actual face for these was unverified from here (no
;;     running Emacs to check against); constant is the closest
;;     reasonable fit and is called out as an approximation per the M33
;;     plan itself.
;;   - lifetimes are deliberately left uncolored -- a simplification,
;;     not a considered GNU-fidelity claim.
;;   - function/closure parameters and simple (non-destructuring) `let`
;;     bindings -> `@variable`: new (the common cross-language rule;
;;     matches real rust-ts-mode, which does fontify these).
;;   - M34 ("if it declares a variable, color it, whatever that variable
;;     is"): `let`'s own
;;     destructuring patterns also reach their bound names now --
;;     `let (a, b) = ...` (`tuple_pattern`), `let [x, y] = arr`
;;     (`slice_pattern`), and `let Point { x: px, y: py } = p` /
;;     `let Point { x, y } = p` (`struct_pattern`, both the explicit
;;     `field: pattern` shape and the shorthand one -- confirmed with a
;;     one-time `crates/core/examples/dump_tree.rs` run that the shorthand
;;     form's bound name is its own dedicated node kind,
;;     `shorthand_field_identifier`, distinct from the `identifier` a
;;     STRUCT-LITERAL shorthand construction site (`Point { x, y }` as an
;;     *expression*, reading existing bindings) uses instead under a
;;     different parent (`shorthand_field_initializer`, never
;;     `field_pattern`) -- so anchoring to `field_pattern` specifically
;;     can never light up that use site). `let mut x = 1` needed no new
;;     rule at all: `mut` sits as a separate sibling token on
;;     `let_declaration` itself, not a wrapper around `pattern:`, so the
;;     existing bare-identifier rule below already matched it before M34.
;;     All four new rules stay anchored to `let_declaration`'s own
;;     `pattern:` field (unlike C's array/pointer_declarator, which M34
;;     left fully unanchored) so they don't also reach a `match` arm's or
;;     a function parameter's own tuple/slice/struct pattern -- shapes the
;;     M34 plan didn't ask to cover here. Two related shapes found while
;;     verifying this are deliberately NOT covered, degrading gracefully
;;     to plain text like every other simplification in this file: `mut`
;;     prefixing one element *nested inside* a tuple/slice/struct pattern
;;     (`let (mut a, b) = ...` wraps just that element in its own
;;     `mut_pattern` node, unlike top-level `let mut x`) is not unwrapped;
;;     and a tuple/slice/struct pattern nested inside another one is only
;;     unwrapped one level deep (only the outermost pattern reached
;;     directly from `let_declaration`'s `pattern:` field is matched).

(function_item (identifier) @function)
(function_signature_item (identifier) @function)

(type_identifier) @type
(primitive_type) @type

(const_item name: (identifier) @constant.builtin)
(static_item name: (identifier) @constant.builtin)

(line_comment) @comment
(block_comment) @comment

(char_literal) @string
(string_literal) @string
(raw_string_literal) @string

(boolean_literal) @constant.builtin

"as" @keyword
"async" @keyword
"await" @keyword
"break" @keyword
"const" @keyword
"continue" @keyword
"default" @keyword
"dyn" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"fn" @keyword
"for" @keyword
"gen" @keyword
"if" @keyword
"impl" @keyword
"in" @keyword
"let" @keyword
"loop" @keyword
"macro_rules!" @keyword
"match" @keyword
"mod" @keyword
"move" @keyword
"pub" @keyword
"raw" @keyword
"ref" @keyword
"return" @keyword
"static" @keyword
"struct" @keyword
"trait" @keyword
"type" @keyword
"union" @keyword
"unsafe" @keyword
"use" @keyword
"where" @keyword
"while" @keyword
"yield" @keyword
(crate) @keyword
(mutable_specifier) @keyword
(use_list (self) @keyword)
(scoped_use_list (self) @keyword)
(scoped_identifier (self) @keyword)
(super) @keyword

(parameter pattern: (identifier) @variable)
(let_declaration pattern: (identifier) @variable)

;; M34: `let`'s own destructuring patterns -- see the header for the
;; shape-by-shape reasoning and what's deliberately not covered.
(let_declaration pattern: (tuple_pattern (identifier) @variable))
(let_declaration pattern: (slice_pattern (identifier) @variable))
(let_declaration
  pattern: (struct_pattern
    (field_pattern pattern: (identifier) @variable)))
(let_declaration
  pattern: (struct_pattern
    (field_pattern name: (shorthand_field_identifier) @variable)))
;; Tuple-struct destructuring: `let Pair(a, b) = ...`, `let Some(x) =
;; ... else ...` (M34 review: this is a FOURTH pattern node kind,
;; tuple_struct_pattern, distinct from tuple_pattern — without this
;; rule it was entirely uncovered, top level included). `type: (_)`
;; consumes the leading type child first; tree-sitter sibling patterns
;; match an ordered subsequence, so the following (identifier) can
;; only bind to children AFTER the type — the type name itself can
;; never be captured as a variable.
(let_declaration
  pattern: (tuple_struct_pattern type: (_) (identifier) @variable))
