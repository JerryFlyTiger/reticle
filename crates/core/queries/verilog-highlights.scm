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
;;   - The sensitivity-list `or` in `@(posedge clk or negedge rst_n)` --
;;     arguably keyword-ish, but closer to a logical connective than a
;;     structural keyword, left plain like every operator/connective
;;     elsewhere in this file.
;;   - System tasks/functions (`$display`, ...) -- not asked for, left
;;     uncolored.
;;   - M89 closed most of the M38 SystemVerilog-OOP scope cut above
;;     (`typedef`/`package`/`class`/`interface`/`covergroup` declaration
;;     names, enum member names, and structural type REFERENCES -- see the
;;     "OOP declarations" and "type references" sections below), but two
;;     pieces stay out on purpose:
;;       * `struct_union_member`'s own field name -- dump-verified the node
;;         has no `name:` field at all (the member's data_type and its
;;         `list_of_variable_decl_assignments` are its only real children),
;;         and neither this file's own M38 conventions nor the unused
;;         upstream query has any precedent for reaching it structurally.
;;       * The trailing label in `endmodule : alu` / `endpackage : soc_pkg`
;;         -- whether the grammar exposes it as a distinct field was never
;;         established by M89's reconnaissance, and M89 was not asked to
;;         find out.
;;       * A custom `nettype`'s own DECLARATION (`nettype real_net real;` --
;;         the `nettype` statement that introduces `real_net` as a name in
;;         the first place) gets no face at all here, only a subsequent USE
;;         of that name as a net's type does (see the "type references"
;;         section below, which discusses this net/nettype ambiguity in
;;         detail). `demo/rtl/` has no `nettype` declaration to motivate
;;         adding one, and it is a separate grammar production from
;;         everything else this file already anchors on.

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

;; --- M89: SystemVerilog OOP declaration names -> @type ----------------------
;; House style, not upstream: the unused upstream query tags package/class/
;; interface names @function (and has no rule at all for covergroup's own
;; name), but this codebase's own convention for a container/type
;; declaration is @type -- see java-highlights.scm's class/interface/enum
;; rule and python-highlights.scm's class rule. `typedef`'s declared name is
;; on its OWN `type_declaration` node via a `type_name:` field (NOT `name:`
;; -- dump-verified, see header), sibling to the type_declaration's data_type
;; child, so this can never also reach a nested USE inside that data_type
;; (e.g. `typedef soc_pkg::req_t local_req_t;`'s `soc_pkg`/`req_t`).
;; `covergroup_declaration`'s `name:` field is optional in the grammar (an
;; unnamed covergroup is legal SV) but that's a query-time non-issue: a
;; pattern with no matching field on a given node simply doesn't fire there.

(type_declaration type_name: (simple_identifier) @type)
(package_declaration name: (simple_identifier) @type)
(class_declaration name: (simple_identifier) @type)
(interface_ansi_header name: (simple_identifier) @type)
(interface_nonansi_header name: (simple_identifier) @type)
(covergroup_declaration name: (simple_identifier) @type)

;; --- M89: enum member names -> @constant ------------------------------------
;; Matches java-highlights.scm's `enum_constant` and c-highlights.scm's
;; `enumerator` -- an enum member is a fixed value, not a variable. The
;; identifier is `enum_name_declaration`'s only (positional) child --
;; dump-verified against `typedef enum logic [1:0] { OP_A, OP_B } alu_op_e;`.

(enum_name_declaration (simple_identifier) @constant)

;; --- M89: type REFERENCES (structural, not spelling; M34) -> @type ---------
;; The names declared above get USED all over the rest of a real design
;; (`demo/rtl/`'s pkg/core/bus/mem/top split is exactly this: soc_pkg.sv
;; declares, every other file references). Dump-verified shapes:
;;   * A BARE reference (`alu_op_e op;`, or after `import soc_pkg::*;` a
;;     bare `alu_op_e alu_op_i` port) parses as `data_type` directly
;;     wrapping a positional `simple_identifier`, no wrapper node at all.
;;     This rule is anchored to `data_type` so it can never reach a
;;     value-position identifier (a cast's `casting_type`, a case-item/
;;     expression's `hierarchical_identifier`, and a parameter/bit-range
;;     USE like `[NumMasters-1:0]` are all sibling/cousin node kinds, never
;;     `data_type` itself -- dump-verified with `int'(1)`, `IdxWidth'(1)`,
;;     `soc_pkg::AluAdd` as a value, and `[NumMasters-1:0]`, none of which
;;     this rule can reach).
;;   * The SAME bare-identifier shape (a plain, unwrapped `simple_identifier`
;;     as a positional child) ALSO shows up one level up, directly under
;;     `net_declaration` itself, for a user-defined-nettype declaration --
;;     e.g. `real_net my_signal;`. IMPORTANT: this is not "the same rule
;;     applying twice" by design so much as a coincidence of this grammar's
;;     ambiguity resolution -- the identical source text `alu_op_e x;`
;;     parses as `data_type` (inside a `data_declaration`) in isolation but
;;     as a *bare `net_declaration` child* when an earlier statement in the
;;     same scope has already established enough context (dump-verified:
;;     preceding it with `soc_pkg::req_t r;` flips the parse). Both
;;     productions are real and both need their own rule; this file just
;;     documents honestly that which one fires for a given piece of source
;;     is an accident of surrounding context, not something this query
;;     controls.
;;     This position is NOT safe to match unconditionally, though: the
;;     grammar's `interconnect` form (`interconnect net_flag;`, LRM-2017)
;;     ALSO puts a bare, wrapper-less `simple_identifier` as a direct
;;     `net_declaration` child -- but there it's the declared NET's own
;;     NAME, not a type (dump-verified with `interconnect net_flag;` and
;;     `interconnect w1, w2;`: no `list_of_net_decl_assignments` node
;;     exists in this production at all, the identifiers ARE the names).
;;     The structural discriminator: in the nettype-type-position case, the
;;     bare identifier is immediately followed by a `list_of_net_decl_assignments`
;;     sibling holding the actual declared name(s); in the `interconnect`
;;     case there is no such sibling. Anchoring on that adjacency (dump-
;;     verified against both shapes, and against plain `wire net_flag;`
;;     which has neither -- its type is a wrapped `net_type` node, not a
;;     bare identifier, so this rule never fires there either) is what
;;     lets this rule capture only the type.
;;     MUTATION-OBSERVABILITY NOTE (M89 second fix round, Q2): the test
;;     file's `not_type_at` assertions on the declared NAMEs next to a type
;;     reference (`real_sig`, `net_flag`, `local_op`) document intent but
;;     CANNOT FAIL by construction -- every declaration shape this grammar
;;     produces puts the declared name one container level deeper than the
;;     type reference (inside `net_decl_assignment`/`variable_decl_assignment`,
;;     itself inside a `list_of_*` wrapper), and every rule in this file
;;     requires a direct parent-child relationship, so no mutation of any
;;     rule above can ever reach that deeper position. The ONE assertion in
;;     that block that IS mutation-observable for this rule is
;;     `not_colored_at(..., "ic_net", 0)` -- reverting the
;;     `(list_of_net_decl_assignments)` sibling constraint really does turn
;;     it red.
;;   * A package-QUALIFIED reference (`soc_pkg::req_t`, `soc_pkg::alu_op_e`)
;;     parses as `data_type` wrapping a `class_type` node, which itself
;;     holds each dot/`::`-separated segment as a POSITIONAL
;;     `simple_identifier` sibling (no `package_scope` wrapper node in this
;;     position, contrary to this milestone's anchor table -- that table's
;;     expectation doesn't match the real parse here, so the query below
;;     follows the parse), each OPTIONALLY followed by its own
;;     `parameter_value_assignment` (`pkg#(1)::sub#(2)::final_t`-shaped is
;;     legal -- dump-verified). A chain can be longer than two segments
;;     (`a::b::c local_var;` dump-verifies as three sibling identifiers
;;     inside one `class_type`); M89's fix-round decision is that only the
;;     FINAL segment is the type name being referenced -- every segment
;;     before it is a scope qualifier (package or, in a 3+-segment chain,
;;     an intermediate class/package), matching the 2-segment case's
;;     existing "leave the qualifier plain" precedent rather than treating
;;     a middle segment as a type use in its own right.
;;     M89's SECOND fix round found the first round's single rule
;;     (anchoring the capture to the LAST child of `class_type` via a
;;     trailing `.`) was itself a regression for the parameterized case:
;;     `parameter_value_assignment` is its own named node, not inlined, and
;;     the grammar attaches an optional one after EVERY segment including
;;     the final one, so for `soc_pkg::fifo_t #(8) f;` the true last child
;;     of `class_type` is the `parameter_value_assignment`, not the
;;     `fifo_t` identifier -- the trailing-`.` rule fixed the middle-segment
;;     over-capture but silently stopped capturing this case at all. Two
;;     rules now cover the FINAL-segment position, split on whether it has
;;     a trailing parameterization:
;;       - no trailing param: the identifier itself is the last child
;;         (unchanged from the first fix round).
;;       - trailing param: the identifier is immediately followed (anchored
;;         `.`) by a `parameter_value_assignment` that IS the last child.
;;         Anchoring the identifier-to-param adjacency (not just "a param
;;         exists somewhere after this identifier") is required: without
;;         it, a chain where a MIDDLE segment also carries its own
;;         parameterization (`pkg::mid#(1)::final_t`) would match every
;;         earlier identifier against that middle param too (dump-verified
;;         as a real false-positive risk while iterating this rule, not
;;         shipped). Both rules dump-verified across 2/3/4-segment chains,
;;         with and without a trailing param on the final segment, and with
;;         a param on a NON-final segment -- in every case exactly one
;;         capture, the final segment, and never a middle one.
;;     Neither rule mis-fires on a single-identifier `class_type` (e.g. a
;;     `class ... extends bar;` superclass reference) because those never
;;     have a SECOND `simple_identifier` sibling for either pattern to
;;     anchor after.
;;   * A BARE parameterized type reference (`req_t #(8) x;`, a
;;     single-identifier `class_type` with its own `parameter_value_assignment`)
;;     is covered too, closing the second half of finding 4: the identifier
;;     must be the class_type's ONLY identifier (leading `.` anchor) AND
;;     immediately followed by the `parameter_value_assignment` as the
;;     class_type's last child (trailing `.` anchor) -- both anchors
;;     together require exactly two children, ruling out any multi-segment
;;     chain (dump-verified: `a::b::c #(8) x;` and `a::b #(8) x;` produce
;;     ZERO captures from this rule, correctly leaving that case to the
;;     final-segment rules above instead). This can't reach the
;;     `class_declaration`/`extends` superclass reference either, even
;;     though THAT can also carry a `parameter_value_assignment`
;;     (`class foo extends bar #(1);` dump-verified) -- that `class_type`'s
;;     parent is `class_declaration`, never `data_type`, and this rule is
;;     anchored to `data_type` exactly like every other rule in this
;;     section. Like the plain bare-identifier rule above, this is ALSO
;;     context-dependent: a mutation run in the M89 third fix round found
;;     that the fixture's own `req_t #(8) bare_param;` (preceded by other
;;     declarations in the same module scope) does not exercise this rule
;;     at all -- it resolves as `net_declaration`+`delay_control` instead
;;     (`req_t` the net type, `#(8)` a net delay, not a parameterization),
;;     which the H1 rule above already covers. Only as a module's FIRST
;;     statement (no preceding declaration in scope) does the identical
;;     text take the `data_type`/`class_type` shape this rule targets
;;     (dump-verified; see the test file's `bare_param_m` fixture module).

(data_type (simple_identifier) @type)
(net_declaration (simple_identifier) @type (list_of_net_decl_assignments))
(data_type (class_type (simple_identifier) (simple_identifier) @type .))
(data_type (class_type (simple_identifier) (simple_identifier) @type . (parameter_value_assignment) .))
(data_type (class_type . (simple_identifier) @type . (parameter_value_assignment) .))

;; --- M89 Q3: a user-defined nettype used as a PORT type -> @type -----------
;; Distinct grammar position from every rule above: `net_port_type`'s own
;; `nettype_identifier` alternative (aliased straight to a bare
;; `simple_identifier`, no wrapper) is a SEPARATE production from
;; `data_type`/`class_type` entirely. Dump-verified it is reachable, but
;; only through the OLDER non-ANSI "separate port declaration" form and
;; only via `inout` (`module m(a); inout real_net a; endmodule` parses as
;; `net_port_type` directly wrapping the bare identifier, no `data_type` in
;; the path at all) -- an ANSI `inout real_net a` port, and every `input`/
;; `output` form tried (ANSI and non-ANSI, bare and with an explicit `wire`
;; forcing net-type resolution), all resolve through `variable_port_header`/
;; `data_type` instead (already covered above), because this grammar's own
;; documented GLR conflict table gives `variable_port_header` precedence
;; over `net_port_header` for a bare identifier. Cheap and safe to add: no
;; other `net_port_type` alternative ever leaves a bare `simple_identifier`
;; as its direct child (the built-in-net-type and `interconnect`
;; alternatives always wrap through `net_type`/`data_type_or_implicit`
;; nodes instead), so anchoring on `net_port_type`'s direct child cannot
;; reach anything but this one alternative.

(net_port_type (simple_identifier) @type)

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

;; --- M97: interface port header -> @type / @constant ------------------------
;; A module (or another interface) port declared with an interface type,
;; e.g. `module bar(axi_if.mst bus, input clk);`, has an `ansi_port_declaration'
;; whose header child is `interface_port_header' (a DIFFERENT header kind
;; from `net_port_header'/`variable_port_header', all three siblings under
;; the same `ansi_port_declaration' position) -- dump-verified (M97 recon):
;; real fields `interface_name:'/`modport_name:', both `simple_identifier',
;; `modport_name:' optional (`axi_if bus;' with no `.mst' is legal SV, a
;; port typed by the interface as a whole). The interface's OWN declaration
;; name already gets @type via the `interface_ansi_header'/`interface_
;; nonansi_header' rule above (M89) -- this section is the missing
;; REFERENCE-site rule, the same declaration/reference split every other
;; type in this file already draws (see the "type REFERENCES" section
;; above).
;;
;;   - `interface_name:' is a genuine type reference (which interface this
;;     port is shaped like) -> @type, same face as every other type
;;     reference in this file.
;;   - `modport_name:' is NOT a type -- it names one of the interface's
;;     own `modport' VIEWS (a fixed, named subset of that interface's
;;     signals/directions), closer in kind to an enum member reference
;;     than to a type reference. This deliberately does NOT follow the
;;     package-qualifier precedent above ("leave the qualifier plain"):
;;     that precedent applies to an intermediate SCOPE on the way to a
;;     further name (`soc_pkg' in `soc_pkg::req_t', itself uncaptured);
;;     `modport_name:' is not a scope leading to anything else, it is
;;     itself the last, and only, thing selected here, the same role
;;     `enum_name_declaration' plays for its own declaration -- so this
;;     file gives it the same face, @constant, as a distinct, deliberate
;;     choice rather than reusing @type by default.
;;   - The port's OWN name (`bus' above, `ansi_port_declaration''s
;;     `port_name:' field) is unaffected: it already gets @variable from
;;     the blanket `(ansi_port_declaration port_name: ...)' rule near the
;;     top of this file, which fires regardless of which of the three
;;     header kinds sits next to it -- pinned by this milestone's
;;     `not_colored_at'-style guard so a future change to this section
;;     can't accidentally widen `interface_name:'/`modport_name:' to also
;;     swallow the sibling `port_name:' field.

(interface_port_header interface_name: (simple_identifier) @type)
(interface_port_header modport_name: (simple_identifier) @constant)

;; --- comments / strings ------------------------------------------------------

(one_line_comment) @comment
(block_comment) @comment
(quoted_string) @string
