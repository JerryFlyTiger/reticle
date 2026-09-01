;; M38: Verilog/SystemVerilog, dump-verified against tree-sitter-systemverilog
;; 0.4.0 (gmlarumbe, an IEEE 1800-2023 grammar -- SystemVerilog is a strict
;; superset of Verilog, and the go/no-go survey found no separate
;; plain-Verilog grammar worth using instead, so .v/.vh share this exact
;; grammar and query with .sv/.svh, matching real verilog-ts-mode's own
;; choice). Written from scratch toward GNU font-lock philosophy (M33:
;; only a definition/declaration's own name gets a face; nothing is
;; guessed from spelling; numeric/operator tokens stay plain) rather than
;; trimmed from an upstream query -- unlike the eight M25/M33 languages,
;; this grammar crate ships no `HIGHLIGHTS_QUERY` constant at all to start
;; from.
;;
;; Every node-kind/field name below was confirmed with a one-off dump tool
;; (a `(treesit-node-string)`-equivalent recursive walk over representative
;; snippets -- the tool itself is not part of this codebase, deleted after
;; use, the same "dump, verify, delete" convention M33/M34/M36 used), never
;; guessed from the grammar's published node-types.json alone. Key findings:
;;
;;   - Several OPENING keywords are wrapped in their own named node instead
;;     of being bare anonymous tokens the way c/rust/java's keywords are:
;;     `module`->`module_keyword`, `always`/`always_ff`/`always_comb`/
;;     `always_latch`->`always_keyword`, `case`(/`casex`/`casez`)->
;;     `case_keyword`, `input`/`output`/`inout`->`port_direction`,
;;     `posedge`/`negedge`->`edge_identifier`. Each of these wrapper nodes
;;     contains ONLY its one keyword token (dump-verified -- no other
;;     children), so capturing the wrapper node and capturing its bare
;;     token would produce byte-identical spans; this file captures the
;;     wrapper, once, rather than also duplicating a bare-literal rule for
;;     the same token. Every CLOSING keyword (`endmodule`/`endcase`/
;;     `endfunction`/`endtask`/`endgenerate`/`end`), and `if`/`else`/`for`/
;;     `return`/`assign`/`parameter`/`localparam`/`initial`/`function`/
;;     `task`/`default`/`generate`/`automatic`/`genvar`, are all plain
;;     anonymous tokens with no wrapper, matched directly as bare literals
;;     like every other language file in this codebase already does.
;;   - Identifiers are overwhelmingly `simple_identifier` (a hierarchical
;;     reference -- an assignment target, or a call/instance connection --
;;     is `hierarchical_identifier` wrapping one or more `simple_identifier`
;;     instead, and is never matched here, use-site or not) -- but WHICH
;;     container holds a declared name, and via a real field or a bare
;;     positional child, differs per declaration shape and was confirmed
;;     one container at a time:
;;       * `variable_decl_assignment` (logic/reg/bit/int/... locals, the
;;         entry `data_declaration`'s own list holds), `tf_port_item`
;;         (function/task parameters), and `function_body_declaration`/
;;         `task_body_declaration` (the function/task's OWN name) all use
;;         a real `name:` field.
;;       * `net_decl_assignment` (wire) and `param_assignment`
;;         (parameter/localparam, both inside a port list and as a plain
;;         module item) have NO field at all -- the identifier is simply
;;         their first positional child.
;;       * `ansi_port_declaration` (an ANSI module port) names its port via
;;         a `port_name:` field.
;;     Critically, an unpacked array dimension on a declaration (`logic mem
;;     [SIZE];`) attaches as a SIBLING `unpacked_dimension` node, not a
;;     second identifier child of the assignment node itself, and a
;;     parameter's own initializer expression (`parameter W2 = WIDTH * 2;`)
;;     is a sibling too (`constant_param_expression`) -- so anchoring to a
;;     field (`name:`/`port_name:`) or, where there is no field, to being a
;;     DIRECT child (tree-sitter query nesting is never a deep/recursive
;;     search) can never also reach a USE-SITE identifier sitting inside
;;     that dimension or initializer expression, no matter how deeply
;;     nested. Dump-verified with `logic mem [SIZE];`, `wire net_arr
;;     [SIZE];`, `wire x = 1'b0;`, `logic y = x;`, and `parameter WIDTH2 =
;;     WIDTH * 2;` -- in every case only the DECLARED name is matched,
;;     never the reference.
;;   - A module's own name is a `name:` field on EITHER `module_ansi_header`
;;     (has an ANSI `(input ..., output ...)` port list) or
;;     `module_nonansi_header` (everything else, including a module with a
;;     completely EMPTY `()` port list -- dump-verified: an empty port list
;;     parses as non-ANSI here, not as an ANSI header with zero ports) --
;;     both need their own rule.
;;   - Module INSTANTIATION (`sub_mod u1 (.clk(clk), ...);`) is
;;     deliberately left entirely uncolored -- like a function/method CALL
;;     everywhere else in this codebase, instantiating an existing module
;;     type and wiring an instance name to it is a USE of that type, not a
;;     fresh definition, so neither the instance name nor the module type
;;     name being instantiated gets a face here.
;;   - `always_construct`/`initial_construct` do NOT themselves carry a
;;     `begin`/`end` pair -- the nested `seq_block` inside does (an always/
;;     initial with a single, unbraced statement body has no seq_block at
;;     all), so neither is a useful anchor for anything here -- irrelevant
;;     to highlighting, but the same finding indent.el's own header
;;     documents for its block-depth table.
;;
;; Deliberately NOT covered (disclosed scope cuts, not oversights):
;;   - `genvar` declarations (a generate-loop index) -- outside the go/no-go
;;     representative snippet's five named declaration containers; the
;;     `genvar` keyword itself still gets @keyword, just not its name.
;;   - SystemVerilog OOP (`class`/`interface`/`package`/`covergroup`/...) --
;;     the go/no-go survey and this milestone's representative snippet are
;;     both scoped to core RTL (module/function/task/always/case/generate),
;;     not the full IEEE 1800 surface.
;;   - The sensitivity-list `or` in `@(posedge clk or negedge rst_n)` --
;;     arguably keyword-ish, but closer to a logical connective than a
;;     structural keyword, left plain like every operator/connective
;;     elsewhere in this file.
;;   - System tasks/functions (`$display`, ...) -- not asked for, left
;;     uncolored.

;; --- module/function/task's OWN name -> @function ------------------------

(module_ansi_header name: (simple_identifier) @function)
(module_nonansi_header name: (simple_identifier) @function)
(function_body_declaration name: (simple_identifier) @function)
(task_body_declaration name: (simple_identifier) @function)

;; --- declared variable/net/parameter/port names -> @variable -------------
;; (M34 principle: any DECLARATION counts, regardless of the variable's own
;; type -- see the header above for why none of these five can ever also
;; reach a use-site identifier.)

(variable_decl_assignment name: (simple_identifier) @variable)
(net_decl_assignment (simple_identifier) @variable)
(param_assignment (simple_identifier) @variable)
(ansi_port_declaration port_name: (simple_identifier) @variable)
(tf_port_item name: (simple_identifier) @variable)

;; --- type keywords -> @type ------------------------------------------------
;; `logic`/`reg`/`bit` -> integer_vector_type; `int`/`byte`/`shortint`/
;; `time` -> integer_atom_type; `real` -> non_integer_type; `wire` (and
;; other net kinds) -> net_type -- all dump-verified. `string` is the one
;; primitive type with NO wrapper node of its own (a bare token directly
;; under `data_type`), so it needs its own anchored rule instead.

(integer_vector_type) @type
(integer_atom_type) @type
(non_integer_type) @type
(net_type) @type
(data_type "string" @type)

;; --- keywords -> @keyword ----------------------------------------------------

(module_keyword) @keyword
(always_keyword) @keyword
(case_keyword) @keyword
(port_direction) @keyword
(edge_identifier) @keyword

"begin" @keyword
"end" @keyword
"endmodule" @keyword
"if" @keyword
"else" @keyword
"endcase" @keyword
"default" @keyword
"function" @keyword
"endfunction" @keyword
"task" @keyword
"endtask" @keyword
"initial" @keyword
"assign" @keyword
"parameter" @keyword
"localparam" @keyword
"return" @keyword
"generate" @keyword
"endgenerate" @keyword
"automatic" @keyword
"genvar" @keyword
"for" @keyword

;; --- comments / strings ------------------------------------------------------

(one_line_comment) @comment
(block_comment) @comment
(quoted_string) @string
