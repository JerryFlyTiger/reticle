;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-python's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2016 Max Brunsfeld
;; Upstream: https://github.com/tree-sitter/tree-sitter-python
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from tree-sitter-python's `HIGHLIGHTS_QUERY` toward
;; GNU python.el's own font-lock conventions: only a `def`/`class`'s own
;; *definition* site gets a face, nothing is guessed from a name's
;; spelling, and numeric/symbolic-operator tokens are left plain.
;;
;; Deleted from upstream: the Capitalized-is-a-`@constructor` and
;; ALL_CAPS-is-a-`@constant` heuristics, every call-site rule (a bare
;; call, an attribute-method call, and the ~50-name `@function.builtin`
;; regex over `print`/`len`/`open`/...), `(type (identifier) @type)` (a
;; type *annotation reference*, e.g. the `int` in `x: int`, not a
;; definition -- dropped for the same "use site, not definition site"
;; reason as everything else here), `(attribute attribute: (identifier)
;; @property)`, `@punctuation.special`/`@embedded` on f-string braces,
;; and `@number` on integer/float literals.
;;
;; Kept or changed to GNU semantics:
;;   - `def` name -> `@function` (only the definition; see above for
;;     what's deleted on the call side).
;;   - `class` name -> `@type`: new, structural (upstream only reached
;;     class names incidentally through the deleted Capitalized-heuristic
;;     above, which is exactly the kind of guess GNU font-lock doesn't
;;     do).
;;   - a decorator's target -> `@type`, *not* `@function`: matches real
;;     python.el, which fontifies a `@decorator` line with
;;     font-lock-type-face rather than treating it as a call or a
;;     definition. Only the bare-name shape (`@staticmethod`) is
;;     handled, not a dotted/attribute decorator (`@app.route`) or a
;;     called one (`@app.route("/x")`) -- a deliberate simplification.
;;   - True/False/None -> `@constant.builtin` (unchanged from upstream).
;;   - function parameters -> `@variable`, *except* `self` -- python.el's
;;     own long-standing special case that the receiver isn't fontified
;;     as an ordinary bound name (the M33 plan calls this out
;;     explicitly). `cls` is not special-cased the same way here: this
;;     project couldn't verify whether real python.el does either, so
;;     it's a documented simplification rather than a considered
;;     omission. Only the bare-identifier parameter shape is handled,
;;     not a typed/defaulted one (`x: int`, `y=5`).
;;   - word-operators that are real Python keywords lexically
;;     (`and`/`or`/`not`/`in`/`is`, plus the two-word `is not`/`not in`
;;     tokens) move from upstream's `@operator` into `@keyword`:
;;     python.el's own keyword regexp lists them right alongside
;;     `if`/`def`/`class`; there's no separate "word-operator" face the
;;     way the tree-sitter query invents one.

(function_definition name: (identifier) @function)
(class_definition name: (identifier) @type)

(decorator (identifier) @type)

(function_definition
  parameters: (parameters
    (identifier) @variable
    (#not-eq? @variable "self")))

[
  (none)
  (true)
  (false)
] @constant.builtin

(comment) @comment
(string) @string

[
  "as"
  "assert"
  "async"
  "await"
  "break"
  "class"
  "continue"
  "def"
  "del"
  "elif"
  "else"
  "except"
  "exec"
  "finally"
  "for"
  "from"
  "global"
  "if"
  "import"
  "lambda"
  "nonlocal"
  "pass"
  "print"
  "raise"
  "return"
  "try"
  "while"
  "with"
  "yield"
  "match"
  "case"
  "and"
  "in"
  "is"
  "not"
  "or"
  "is not"
  "not in"
] @keyword
