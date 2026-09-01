;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-perl's
;; queries/highlights.scm, used under the MIT License.
;; Copyright 2025 Avishai "Veesh" Goldman
;; Upstream: https://github.com/tree-sitter-perl/tree-sitter-perl
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from ts-parser-perl's `HIGHLIGHTS_QUERY` -- by far the
;; largest and most editor-oriented of the eight upstream queries (deref
;; expressions, sigil-specific variable-reference faces, a giant
;; builtin-function-name regex, an `(ERROR)` rule, punctuation captures,
;; ...) -- down to the minimal set the M33 plan actually asks for
;; specifically for Perl: sub/method definitions, `my`/`our`/`local`
;; declarations, and keywords/strings/comments. This is a deliberate
;; simplification ("cperl simplification" per the M33 plan), not an attempt at a
;; from-scratch faithful perl-mode/cperl-mode port.
;;
;; Deleted from upstream: every capture-site rule
;; (`function_call_expression`, `method_call_expression`, the
;; `func0op_call_expression`/`func1op_call_expression` builtin-call
;; rules, the giant builtin-name `#match?` regex, `amper_deref_expression`),
;; every bare variable-*reference* rule (`varname`/`scalar`/`array`/
;; `hash`/`glob`/`filehandle` outside a declaration, string
;; interpolation's generic `array:`/`hash:` field capture), `(ERROR)`,
;; the shebang-comment and `eof_marker` preprocessor-ish rules (a shebang
;; line is still just a `(comment)` node structurally, so it still gets
;; plain comment face via the rule kept below -- just not a dedicated
;; one), `attribute_name`/`attribute_value`, statement labels, and all
;; punctuation/bracket/bare-`@operator` captures (including the ternary
;; `? :` tokens, which upstream oddly filed under `@conditional.ternary`
;; despite being pure punctuation).
;;
;; Kept unchanged, or changed to GNU semantics:
;;   - the keyword token lists (control flow, `use`/`no`/`require`,
;;     `package`/`class`/`role`, word-operators like `eq`/`lt`/`isa`/
;;     `and`/`or`) are kept close to verbatim, including upstream's own
;;     `@conditional`/`@repeat`/`@exception`/`@include`/`@keyword.operator`
;;     sub-names -- harmless, since `face_for_capture` folds every one
;;     of those dotted/sub-category names back to plain keyword face
;;     already.
;;   - `sub`/`method` definition name -> `@function`/`@method` (upstream's
;;     own names, both already resolving to font-lock-function-name-face;
;;     a forward declaration with no body, `sub foo;`, matches the same
;;     rule -- same "declaration position, not body" reasoning as C's
;;     prototypes).
;;   - `my`/`our`/`local` declared scalar/array/hash names -> `@variable`
;;     (new), matched *positionally* (`(variable_declaration [(scalar
;;     ...) (array ...) (hash ...)])`) rather than through the grammar's
;;     own `variable:`/`variables:` field names -- the sharpest
;;     tree-sitter-query-engine trap found while trimming this file (see
;;     the M33 report). `my $x = 1;` (a single bare name, no parens)
;;     puts the name under a singular `variable:` field, but
;;     `my ($name) = @_;` (parenthesized, even with only one name inside)
;;     puts it under a *plural* `variables:` field that's also shared by
;;     the literal `(`/`)` tokens around it -- and a query pattern of the
;;     shape `(variable_declaration variables: (scalar (varname)
;;     @variable))` compiles fine but silently matches *nothing* against
;;     that second shape (confirmed with a throwaway debug harness that
;;     dumped every capture the query produced against `my ($name) =
;;     @_;`: zero, versus one the moment the `variables:` field
;;     constraint was dropped and the same inner pattern matched
;;     positionally instead). Matching positionally sidesteps the field
;;     name entirely and turns out to already handle *both* shapes (and
;;     `our`) uniformly, so there's no need for two separate rules.
;;     `local` is a *different* node entirely, `localization_expression`,
;;     with the name as a plain positional child too (no named fields on
;;     that node at all) -- handled by the same positional-match style
;;     for consistency, not because it was hit by the same field bug.
;;     Modern `method` signature parameters (`method greet($name) {
;;     ... }`) are *not* covered -- only the classic `my ($name) = @_;`
;;     shape is, another deliberate
;;     simplification.
;;   - comment/pod -> `@comment`; string-ish literals -> `@string`.
;;
;; Deliberately not colored: package/class names (the common
;; type-position rule isn't extended to Perl's `package`/`class`
;; statements here -- "simplification"), attributes, labels.

[ "if" "elsif" "unless" "else" ] @conditional

[ "while" "until" "for" "foreach" ] @repeat
("continue" @repeat (block))

[ "try" "catch" "finally" ] @exception

"return" @keyword

[ "sub" "method" "async" "extended" ] @keyword

[ "package" "class" "role" ] @include
[ "use" "no" "require" ] @include

[
  "defer"
  "do" "eval"
  "my" "our" "local" "dynamically" "state" "field"
  "last" "next" "redo" "goto"
  "undef" "await"
] @keyword

[
  "or" "xor" "and" "not"
  "eq" "ne" "cmp" "lt" "le" "ge" "gt"
  "isa"
] @keyword.operator

(comment) @comment
(data_section) @comment
(pod) @comment

[
  (string_literal)
  (interpolated_string_literal)
  (quoted_word_list)
  (command_string)
  (heredoc_content)
] @string

(subroutine_declaration_statement name: (bareword) @function)
(method_declaration_statement name: (bareword) @method)

;; Positional, not `variable:`/`variables:` field-anchored -- see the
;; header for why the field-anchored shape silently matches nothing
;; against a parenthesized `my (...)`/`our (...)` declaration.
(variable_declaration
  [
    (scalar (varname) @variable)
    (array (varname) @variable)
    (hash (varname) @variable)
  ])
(localization_expression
  [
    (scalar (varname) @variable)
    (array (varname) @variable)
    (hash (varname) @variable)
  ])
