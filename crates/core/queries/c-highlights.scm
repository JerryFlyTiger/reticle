;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-c's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2014 Max Brunsfeld
;; Upstream: https://github.com/tree-sitter/tree-sitter-c
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from tree-sitter-c's own `HIGHLIGHT_QUERY` (see the
;; M25 grammar survey / crates.io tree-sitter-c 0.24.2's
;; queries/highlights.scm) toward GNU cc-mode's own font-lock
;; conventions: only *definition/declaration* sites get a face, nothing
;; is guessed from a name's spelling, and numeric/operator/punctuation
;; tokens are left plain.
;;
;; Deleted from upstream: the catch-all `(identifier) @variable`, the
;; `#match?` "ALL_CAPS is a constant" heuristic, both call_expression
;; function-name rules (a call site, e.g. `add(1, 2)`, must never look
;; like a definition), the whole `@operator`/`@delimiter` token lists,
;; and `@number` on `number_literal`/`char_literal`. Also deleted the
;; generic `(field_identifier) @property` rule -- it fired on both a
;; struct member *declaration* and a `.`/`->` member *access*, and this
;; file doesn't color either (not asked for by the M33 plan; a member
;; access in particular would be a use site, the same thing call sites
;; are excluded for everywhere else here).
;;
;; Kept or changed to GNU semantics:
;;   - keywords, string literals, comments, and `(type_identifier)` /
;;     `(primitive_type)` / `(sized_type_specifier)` type positions are
;;     unchanged from upstream -- plain "keep" categories, not naming
;;     guesses. This is also why a plain C `struct`/`union`/`enum` tag
;;     name needs no dedicated rule of its own here: its own name field
;;     is already a `type_identifier`, so the blanket rule colors it for
;;     free (verified with `crates/core/examples/dump_tree.rs`, a
;;     one-time research tool since deleted -- see the M33 report).
;;   - preprocessor directive names (`#include`, `#define`, ...)
;;     recolored from `@keyword` to `@preproc`
;;     (font-lock-preprocessor-face), matching cc-mode's own dedicated
;;     preprocessor face rather than lumping them in with `if`/`while`.
;;   - `enumerator` (an enum member's own declaration) -> `@constant`:
;;     new, structural, replacing the deleted all-caps heuristic that
;;     used to reach these only by convention-guessing.
;;   - a `labeled_statement`'s own label (a `goto`/`case`-block target
;;     *definition*) -> `@constant`, mirroring cc-mode's real
;;     `c-label-face-name` default (font-lock-constant-face). Deliberate
;;     narrowing from upstream, which tagged bare `(statement_identifier)
;;     @label` wherever it occurred -- including `goto`'s *reference* to
;;     the label, a use site this file leaves uncolored, same
;;     "definition, not use" split as everywhere else here.
;;   - function names: `function_declarator`'s own `declarator:` field,
;;     wherever it occurs -- a plain top-level definition or prototype,
;;     or nested under one or more `pointer_declarator`s for a
;;     pointer-returning function -- but never `call_expression`'s
;;     `function:` field. This deliberately colors bare prototypes too
;;     (`int add(int, int);`, the dominant shape of a whole header
;;     file), matching real cc-mode (whose rule is "has a name in
;;     declarator position", not "has a body"). Confirmed safe against
;;     two grammar traps found while trimming this file: a
;;     `typedef void (*Callback)(int);` names its alias as a
;;     *`type_identifier`*, not `identifier`, so it can never match this
;;     rule; a function-pointer *parameter* like `void (*cb)(int)` puts
;;     a `parenthesized_declarator` directly between `function_declarator`
;;     and the name, so the direct `declarator: (identifier)` field
;;     match misses it too. Neither needed a special exclusion -- the
;;     grammar already disambiguates them structurally.
;;   - parameter and local/global declaration names -> `@variable`: the
;;     bare-declarator shape, the `init_declarator` "= value" shape (this
;;     one deliberately *not* anchored to `declaration` specifically, so
;;     it also reaches a for-loop's own init clause, e.g.
;;     `for (int i = 0; ...)`), and (M34) `array_declarator`/
;;     `pointer_declarator` at ANY nesting depth -- see the M34 paragraph
;;     below for why unanchoring these two, rather than unwrapping a fixed
;;     number of levels, is both correct and necessary.
;;   - M34 ("if it declares a variable, color it, whatever that variable
;;     is" -- color a declared variable's name no matter what the
;;     variable's type looks like): `char **argv`, `int arr[10]`, `int m[2][3]`, `int *pa[3]`,
;;     `int (*ap)[5]`, and a function-pointer variable/parameter like
;;     `void (*cb)(int)` all now get `@variable` on the declared name,
;;     via two rules --
;;       (array_declarator declarator: (identifier) @variable)
;;       (pointer_declarator declarator: (identifier) @variable)
;;     -- DELIBERATELY left unanchored to any specific parent, unlike
;;     every other rule in this file. This is safe for exactly the reason
;;     `function_declarator`'s own rule above already relies on: the
;;     whole declarator node family (function_declarator/
;;     pointer_declarator/array_declarator/init_declarator/
;;     parenthesized_declarator/...) is structurally disjoint from every
;;     expression shape (call_expression/subscript_expression/
;;     pointer_expression/...) in the grammar, so an unanchored rule here
;;     can never be mistaken for a call or subscript *use* site --
;;     verified against node-types.json and a one-time
;;     `crates/core/examples/dump_tree.rs` run over every shape listed
;;     above (see the M34 report). Being unanchored is what reaches
;;     arbitrary nesting with no per-depth special-casing: each rule
;;     fires at whichever array/pointer_declarator node is INNERMOST --
;;     the one whose own `declarator:` field has finally unwrapped down to
;;     the bare name -- regardless of how many more array/pointer layers
;;     (`char **argv`: only the inner `pointer_declarator` matches; `int
;;     m[2][3]`: only the inner `array_declarator` matches) or a
;;     `parenthesized_declarator` (which has no named field of its own at
;;     all -- see `function_declarator`'s own header paragraph above for
;;     the same node -- so an unanchored positional walk through it is
;;     exactly what falls out for free) sit above it. `int *pa[3]` (array
;;     of pointers) and `int (*ap)[5]` (pointer to array) resolve to
;;     *different* rules doing the work -- the array rule for `pa` (its
;;     own declarator field is the identifier; the wrapping
;;     pointer_declarator's is the array_declarator, not an identifier, so
;;     only the array rule fires), the pointer rule for `ap` (symmetric
;;     reasoning) -- without either rule needing to know which. A
;;     function-pointer variable (`void (*cb)(int);`) reaches "cb" via the
;;     pointer rule straight through the intervening
;;     `function_declarator`+`parenthesized_declarator`, and is why it
;;     gets `@variable`, never `@function`: the `function_declarator`
;;     rule above requires *its own* `declarator:` field to be a bare
;;     identifier directly, which this shape never is (a
;;     parenthesized_declarator sits there instead) -- the two rules are
;;     structurally exclusive on this node, never double-firing on the
;;     same name.
;;   - M34 also extends both rules to struct/union members wearing an
;;     array or pointer type (`struct S { int x[3]; int *y; };`), via two
;;     more rules identical to the above but matching `field_identifier`
;;     instead of `identifier` (a field_declaration's own `declarator:`
;;     field is typed `_field_declarator`, a distinct grammar supertype
;;     that substitutes `field_identifier` for `identifier` at the leaf --
;;     see node-types.json). Deliberately narrow: a BARE member
;;     declaration (`int x;`, no array/pointer) still gets no rule at all
;;     here, unchanged from M33's own deletion of upstream's generic
;;     `(field_identifier) @property` (see above) -- that deletion was
;;     specifically because a bare field_identifier is indistinguishable
;;     at that level from a `.`/`->` member *access*. That ambiguity
;;     doesn't reach the two array/pointer rules here: a member access
;;     never produces an array_declarator/pointer_declarator wrapping a
;;     field_identifier (those two node kinds are exclusively
;;     declarator-family), so both are exactly as use-site-safe as their
;;     identifier-based twins -- just scoped to the array/pointer-typed
;;     member the M34 plan actually asked for, not a wholesale reopening
;;     of M33's bare-member decision.

"break" @keyword
"case" @keyword
"const" @keyword
"continue" @keyword
"default" @keyword
"do" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"for" @keyword
"if" @keyword
"inline" @keyword
"return" @keyword
"sizeof" @keyword
"static" @keyword
"struct" @keyword
"switch" @keyword
"typedef" @keyword
"union" @keyword
"volatile" @keyword
"while" @keyword

"#define" @preproc
"#elif" @preproc
"#else" @preproc
"#endif" @preproc
"#if" @preproc
"#ifdef" @preproc
"#ifndef" @preproc
"#include" @preproc
(preproc_directive) @preproc

(string_literal) @string
(system_lib_string) @string

(null) @constant

(type_identifier) @type
(primitive_type) @type
(sized_type_specifier) @type

(enumerator name: (identifier) @constant)
(labeled_statement label: (statement_identifier) @constant)

(function_declarator
  declarator: (identifier) @function)

(parameter_declaration declarator: (identifier) @variable)
(declaration declarator: (identifier) @variable)
(init_declarator declarator: (identifier) @variable)

;; M34: unanchored -- see the header paragraph above for why this is safe
;; and reaches any nesting depth (arrays, pointers, function pointers,
;; array-of-pointers, pointer-to-array) in one pair of rules instead of
;; unwrapping a fixed number of levels per anchor.
(array_declarator declarator: (identifier) @variable)
(pointer_declarator declarator: (identifier) @variable)

;; M34: same two rules again for a struct/union member's own name, which
;; the grammar spells `field_identifier` instead of `identifier` here --
;; see the header paragraph above for why this can never reach a `.`/`->`
;; member access.
(array_declarator declarator: (field_identifier) @variable)
(pointer_declarator declarator: (field_identifier) @variable)

(comment) @comment
