;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from the queries/highlights.scm of
;; the following projects, used under the MIT License:
;;   tree-sitter-c -- Copyright (c) 2014 Max Brunsfeld
;;     https://github.com/tree-sitter/tree-sitter-c
;;   tree-sitter-cpp -- Copyright (c) 2014 Max Brunsfeld
;;     https://github.com/tree-sitter/tree-sitter-cpp
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; M33: unlike M25 (which ran tree_sitter_c::HIGHLIGHT_QUERY concatenated
;; with tree_sitter_cpp::HIGHLIGHT_QUERY at runtime via an `OnceLock` --
;; see the removed `cpp_highlight_query()` in highlight.rs -- because
;; upstream's own cpp query assumes C's is layered underneath and is
;; nearly empty on its own), this file is fully self-contained: every C
;; rule below is deliberately duplicated from c-highlights.scm rather
;; than composed at load time, so C++ mode never depends on the C query
;; being compiled as a *separate* pass. Keep the two files in sync by
;; hand for any shared rule; see c-highlights.scm for the per-rule
;; rationale on the C-derived two-thirds of this file (kept/deleted for
;; the same reasons here).
;;
;; M34 sync: c-highlights.scm added two deliberately-unanchored rules --
;; `(array_declarator declarator: (identifier) @variable)` and the
;; `pointer_declarator` twin, plus a `field_identifier` copy of each for
;; struct/class members -- so a declared variable's name is colored no
;; matter how deep the array/pointer nesting goes (`char **argv`, `int
;; m[2][3]`, `int *pa[3]`, `int (*ap)[5]`, a function-pointer variable
;; like `void (*cb)(int)`, ...). Mirrored verbatim below (see
;; c-highlights.scm's own header for the full per-shape reasoning and the
;; node-types.json/dump_tree.rs verification) since this file duplicates
;; the C rules by hand rather than composing at load time.
;;
;; NOT extended to two C++-only declarator shapes found while verifying
;; this: `int &r` (a reference parameter/variable) puts the name as a
;; plain *positional* child of `reference_declarator` (no `declarator:`
;; field on that node at all -- confirmed with the M34 dump_tree run), so
;; today's `declarator:`-field-anchored rules don't reach it regardless of
;; anchoring; and C++17 structured bindings (`auto [a, b] = pair;`) were
;; never dumped at all. Neither is asked for by the M34 plan's own shape
;; list (arrays/pointers/function-pointers), so both are left as a
;; documented gap rather than an unrequested architectural expansion --
;; see the M34 report.
;;
;; C++-specific additions toward GNU c++-mode/cc-mode conventions:
;;   - a method's own inline definition name (`declarator:
;;     (field_identifier)`, e.g. `std::string greet() const { ... }`
;;     written inside the class body) and its out-of-line definition
;;     name (`declarator: (qualified_identifier name: (identifier))`,
;;     e.g. `std::string Greeter::greet() { ... }`) both -> `@function`.
;;     Note what *doesn't* need a dedicated rule: a constructor's own
;;     name (`Greeter() {}`) is a plain `identifier` in
;;     `function_declarator` position, already covered by the C-derived
;;     function rule below; and `class`/`struct` tag names need no rule
;;     at all, for the same reason plain C's don't (see
;;     c-highlights.scm) -- `class_specifier` names its own tag as a
;;     `type_identifier` too.
;;   - `(auto) @type`: a real type-position token (`auto x = 5;`), same
;;     "kept, not a guess" category as C's primitive/sized types.
;;   - the C++-only keyword list (namespaces, templates, access
;;     specifiers, exceptions, coroutines, ...).
;;   - `raw_string_literal` joins the string category.
;;
;; Deleted from upstream's own (non-C) half: both `template_function`/
;; `template_method` rules (call sites -- invoking with explicit
;; template arguments, e.g. `foo<int>()`/`obj.method<int>()` -- not
;; definitions), the `call_expression`-based function rule (another
;; call site), the `namespace_identifier` capital-letter heuristic,
;; `(this) @variable.builtin` (a receiver, dropped for the same reason
;; Python's `self` and Rust's `self` are -- see those files), and the
;; `(null "nullptr" @constant)` sub-rule -- redundant once the
;; C-derived `(null) @constant` rule below already covers both the
;; `NULL` and `nullptr` spellings (tree-sitter-c's grammar defines
;; `null` as `choice('NULL', 'nullptr')` and cpp inherits the same node
;; kind).

;; --- C-derived base (see c-highlights.scm for rationale) --------------

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

;; M34: unanchored -- see c-highlights.scm's header for why this is safe
;; and reaches any nesting depth (arrays, pointers, function pointers,
;; array-of-pointers, pointer-to-array) in one pair of rules instead of
;; unwrapping a fixed number of levels per anchor.
(array_declarator declarator: (identifier) @variable)
(pointer_declarator declarator: (identifier) @variable)

;; M34: same two rules again for a struct/class member's own name, which
;; the grammar spells `field_identifier` instead of `identifier` here --
;; see c-highlights.scm's header for why this can never reach a `.`/`->`
;; member access.
(array_declarator declarator: (field_identifier) @variable)
(pointer_declarator declarator: (field_identifier) @variable)

(comment) @comment

;; --- C++-specific -------------------------------------------------------

(function_declarator
  declarator: (field_identifier) @function)
(function_declarator
  declarator: (qualified_identifier
    name: (identifier) @function))

(auto) @type

(raw_string_literal) @string

[
 "catch"
 "class"
 "co_await"
 "co_return"
 "co_yield"
 "constexpr"
 "constinit"
 "consteval"
 "delete"
 "explicit"
 "final"
 "friend"
 "mutable"
 "namespace"
 "noexcept"
 "new"
 "override"
 "private"
 "protected"
 "public"
 "template"
 "throw"
 "try"
 "typename"
 "using"
 "concept"
 "requires"
 "virtual"
] @keyword
