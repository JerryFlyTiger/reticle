;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-java's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2017 Ayman Nadeem
;; Upstream: https://github.com/tree-sitter/tree-sitter-java
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from tree-sitter-java's `HIGHLIGHTS_QUERY` toward
;; plain GNU java-mode/cc-mode conventions: only a declaration's own
;; *definition* site gets a face, nothing is guessed from a name's
;; spelling.
;;
;; Deleted from upstream: the catch-all `(identifier) @variable`,
;; `(method_invocation name: (identifier) @function.method)` (a call
;; site, e.g. `greet("world")`/`System.out.println(...)` -- only a
;; method's own *declaration* name is a function, see below),
;; `(super) @function.builtin`, the three `#match?`-on-capital-letter
;; heuristics guessing that a `field_access`/`scoped_identifier`/
;; `method_invocation`/`method_reference` object is really a type
;; reference, the all-caps-name-is-a-`@constant` heuristic, the `@`
;; annotation-sigil rule and `(annotation name:) @attribute`
;; (annotations aren't mentioned by the M33 plan for Java, left
;; unstyled -- a deliberate omission, not an oversight), and `@number`
;; on every integer/floating-point literal kind.
;;
;; Kept or changed to GNU semantics:
;;   - class/interface/enum *names* -> `@type` (unchanged from
;;     upstream -- these were never behind the deleted heuristics).
;;   - a method's own *declaration* name -> `@function.method` (still
;;     resolves to font-lock-function-name-face via
;;     `face_for_capture`'s dotted-prefix rule; unchanged capture name
;;     from upstream, just with its call-site sibling rule removed).
;;   - `constructor_declaration`'s name -> `@type`, unchanged from
;;     upstream: real java-mode fontifies a constructor's name the same
;;     as its enclosing class name (they're spelled identically), not
;;     as an ordinary method.
;;   - enum *constant* declarations -> `@constant`: new, structural --
;;     upstream had no dedicated rule for these at all, only reaching
;;     them incidentally via the all-caps heuristic deleted above.
;;   - true/false/null, built-in primitive/void types, and
;;     `(type_identifier)` type positions: unchanged from upstream --
;;     plain "keep" categories, not naming guesses.
;;   - method parameters and local variable declarations -> `@variable`
;;     (new: upstream had no rule for either, only the deleted
;;     catch-all).

(class_declaration name: (identifier) @type)
(interface_declaration name: (identifier) @type)
(enum_declaration name: (identifier) @type)
(enum_constant name: (identifier) @constant)

(constructor_declaration name: (identifier) @type)
(method_declaration name: (identifier) @function.method)

(type_identifier) @type
[
  (boolean_type)
  (integral_type)
  (floating_point_type)
  (void_type)
] @type.builtin

[
  (true)
  (false)
  (null_literal)
] @constant.builtin

[
  (line_comment)
  (block_comment)
] @comment

[
  (character_literal)
  (string_literal)
] @string
(escape_sequence) @string.escape

(formal_parameter name: (identifier) @variable)
(local_variable_declaration
  declarator: (variable_declarator name: (identifier) @variable))

[
  "abstract"
  "assert"
  "break"
  "case"
  "catch"
  "class"
  "continue"
  "default"
  "do"
  "else"
  "enum"
  "exports"
  "extends"
  "final"
  "finally"
  "for"
  "if"
  "implements"
  "import"
  "instanceof"
  "interface"
  "module"
  "native"
  "new"
  "non-sealed"
  "open"
  "opens"
  "package"
  "permits"
  "private"
  "protected"
  "provides"
  "public"
  "requires"
  "record"
  "return"
  "sealed"
  "static"
  "strictfp"
  "switch"
  "synchronized"
  "throw"
  "throws"
  "to"
  "transient"
  "transitive"
  "try"
  "uses"
  "volatile"
  "when"
  "while"
  "with"
  "yield"
] @keyword
