;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-elisp's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2021 Wilfred Hughes
;; Upstream: https://github.com/Wilfred/tree-sitter-elisp
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Originally vendored verbatim at M25 from the `tree-sitter-elisp` crate
;; (v1.6.1), file `queries/highlights.scm`. MIT License, Copyright (c)
;; 2021 Wilfred Hughes. Source: https://github.com/Wilfred/tree-sitter-elisp
;;
;; The 1.6.1 crates.io release ships this file inside the package but its
;; `bindings/rust/lib.rs` has the `HIGHLIGHTS_QUERY` constant commented
;; out, so `tree_sitter_elisp::HIGHLIGHTS_QUERY` does not exist in that
;; version. Rather than depend on an unreleased git revision, M25 vendored
;; this file into our own source tree and loads it with `include_str!`
;; (see crates/core/src/highlight.rs) -- the grammar (`LANGUAGE`) still
;; comes from the normal crates.io dependency, only the query text is
;; copied.
;;
;; M33 trims this from that verbatim copy toward real GNU
;; emacs-lisp-mode conventions: only a definition's own name gets a
;; face, and numeric literals/reader-syntax punctuation are left plain.
;; Deleted: `(integer)`/`(float)`/`(char) @number` (numbers uncolored,
;; per the M33 plan), the `@punctuation.bracket` token list, and the
;; `@operator` token list (reader syntax like `` ` ``/`#'`/`,`/`,@` --
;; punctuation, not an operator in any semantic sense, and either way
;; punctuation stays plain here). Also deleted the
;; `(function_definition docstring: (string) @comment)` /
;; `(macro_definition docstring: (string) @comment)` pair: a docstring
;; is still a `(string)` node, so the generic `(string) @string` rule
;; below already covers it; GNU Emacs has a dedicated
;; `font-lock-doc-face` for docstrings that this palette doesn't define,
;; so per the M33 plan docstrings fold into plain string face instead
;; (previously they were folded into *comment* face, which is what made
;; the dedicated rule necessary in the first place -- now that it's
;; string face like everything else, the dedicated rule became pure
;; duplication and was deleted rather than just recolored).
;;
;; Added for M33:
;;   - `defvar`/`defconst`'s own name -> `@variable`. Unlike
;;     `defun`/`defmacro` below, tree-sitter-elisp has no dedicated node
;;     type for these -- `(defvar my-var 1)` parses as a generic
;;     `special_form` with `"defvar"` and `(symbol "my-var")` as two
;;     unnamed, positional children (verified with
;;     `crates/core/examples/dump_tree.rs`, a one-time research tool
;;     since deleted -- see the M33 report). The `.` anchor pins the
;;     match to the symbol *immediately* following the "defvar"/
;;     "defconst" token, which matters: without it, `(defvar foo bar)`
;;     would also match the initial-value symbol `bar` as if it were a
;;     second declared name.
;;   - `:keyword`-style symbols -> `@builtin`. Not a naming-convention
;;     guess like the ones deleted elsewhere in this milestone -- a
;;     leading `:` is Lisp reader syntax that unconditionally makes a
;;     symbol self-evaluating, not a style convention that could be
;;     wrong.

;; Special forms
[
  "and"
  "catch"
  "cond"
  "condition-case"
  "defconst"
  "defvar"
  "function"
  "if"
  "interactive"
  "lambda"
  "let"
  "let*"
  "or"
  "prog1"
  "prog2"
  "progn"
  "quote"
  "save-current-buffer"
  "save-excursion"
  "save-restriction"
  "setq"
  "setq-default"
  "unwind-protect"
  "while"
] @keyword

;; Function definitions
[
 "defun"
 "defsubst"
 ] @keyword
(function_definition name: (symbol) @function)
(function_definition parameters: (list (symbol) @variable.parameter))

;; Highlight macro definitions the same way as function definitions.
"defmacro" @keyword
(macro_definition name: (symbol) @function)
(macro_definition parameters: (list (symbol) @variable.parameter))

;; `defvar`/`defconst` have no dedicated node type -- see the header.
(special_form "defvar" . (symbol) @variable)
(special_form "defconst" . (symbol) @variable)

(comment) @comment

(string) @string

;; A leading `:` unconditionally makes a symbol self-evaluating --
;; real reader syntax, not a naming guess.
((symbol) @builtin
 (#match? @builtin "^:"))

;; Highlight nil and t as constants, unlike other symbols
[
  "nil"
  "t"
] @constant.builtin
