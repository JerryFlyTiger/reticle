;; SPDX-License-Identifier: MIT
;;
;; Portions of this file are derived from tree-sitter-bash's
;; queries/highlights.scm, used under the MIT License.
;; Copyright (c) 2017 Max Brunsfeld
;; Upstream: https://github.com/tree-sitter/tree-sitter-bash
;; The full MIT permission notice is reproduced in THIRD_PARTY_LICENSES.md.
;;
;; Trimmed for M33 from tree-sitter-bash's `HIGHLIGHT_QUERY` toward GNU
;; sh-mode's own conventions: only the *declaration* site of a shell
;; variable (`NAME=...`) is colored, never a `$NAME`/`${NAME}` *use* site
;; -- a different node entirely (`simple_expansion`/`expansion`, not
;; `variable_assignment`), so this falls out for free from anchoring on
;; `variable_assignment` specifically, rather than upstream's catch-all
;; `(variable_name) @property` (which colored a variable's name the same
;; way whether it was being assigned or merely read).
;;
;; Deleted from upstream: `(command_name) @function`, which colored
;; *every* command invocation -- `echo`, `cd`, a user's own function
;; being called -- as if it were a function definition (the exact
;; "call sites get colored" problem this whole milestone exists to fix;
;; a function *definition* itself is still colored, see below); the
;; `(command (_) @constant (#match? @constant "^-"))` heuristic that
;; colored any command argument starting with `-` (no such convention
;; exists in real sh-mode); `@embedded` on command/process substitution
;; (already produces no face at all -- see `face_for_capture`'s own doc
;; comment -- so keeping it added nothing); `@number` on file
;; descriptors; and the whole `@operator` token list.
;;
;; Kept unchanged: the keyword list, `(comment) @comment`, and the
;; string-ish category (plain/raw strings and heredocs).

(function_definition name: (word) @function)

(variable_assignment name: (variable_name) @variable)

[
  "case"
  "do"
  "done"
  "elif"
  "else"
  "esac"
  "export"
  "fi"
  "for"
  "function"
  "if"
  "in"
  "select"
  "then"
  "unset"
  "until"
  "while"
] @keyword

(comment) @comment

[
  (string)
  (raw_string)
  (heredoc_body)
  (heredoc_start)
] @string
