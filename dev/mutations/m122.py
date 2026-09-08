# Mutation list for M122 (real SystemVerilog material in `demo/', and the
# editor branches it finally connects to). Run by the main conversation,
# never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m122.py -p core
#
# Per the rule M117 wrote into CLAUDE.md, the deletion entry is per FEATURE,
# not per file. M122 ships nine distinct effects, and each one below has an
# entry that removes the whole thing rather than perturbing a boundary inside
# it:
#
#   F1 the demo material itself      -> M1 (`always_latch' stops existing),
#                                       M2 (the cross-directory filelist entry)
#   F2 `program' block depth         -> V1 (the block type), V2 (`endprogram')
#   F3 `property_spec' body depth    -> V3
#   F4 wrapped `modport' port list   -> V4 (the wrap type), V5 (the closer),
#                                       V6 (the closer's depth cancel)
#   F5 `ifdef'/`ifndef' at column 0  -> V7 (the whole short-circuit)
#   F6 `{...}' concatenation depth   -> V8 (the block type),
#                                       V9 (the closing `}' stops dedenting)
#   F7 wrapped bare `else' statement -> V10
#   F8 the demo-file drift guards    -> V11 (indent-width array completeness),
#                                       V12 (major-mode table completeness)
#   F9 the real-material wiring      -> V13 (highlight query), V14 (scope kind)
#
# Three further effects ship real behaviour that NOTHING in this repo can
# observe, and they are declared survivors rather than omitted (S1 per-bank
# grant gating, S2 the monitor's outstanding counters, S3 the simulation
# floor in the shell script). Each entry carries the reason and names what
# would see it. Leaving them out would read, to anyone opening this file
# later, exactly like nobody having thought about them.
#
# F9 is the point of the milestone and deserves the explanation: the editor
# already handled every one of these constructs before M122, and every test
# that said so fed the parser a hand-typed string literal. V13/V14 delete a
# branch and name a test that reads `demo/' from disk -- if those two survive,
# the new tests are decorative and the milestone did not actually happen.
#
# Apart from those three, nothing here is a declared survivor. Indentation,
# faces and scope chains are all plain `cargo test' territory; a
# screenshot-only answer anywhere in F1-F9 would mean something had been put
# in the wrong layer.
#
# Style note, learned on M83/M85 and restated in CLAUDE.md: these are all
# REPLACEMENT-style mutations (rename a node type so it can never match, delete
# a data line), never insertion-style. A stub inserted ahead of a real
# definition does not shadow it -- both elisp's `defun' and the Rust `defun()'
# registration let the LAST definition win.

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    # ---- F1: the real material itself ---------------------------------
    {
        "label": "M1 the repo's only `always_latch' stops being one (deletion entry: F1)",
        "file": "demo/rtl/core/clk_gate.sv",
        "needs_rebuild": False,  # read at run time, never compiled in
        "old": "  always_latch begin",
        "new": "  always_comb begin",
        "test": "verilog_always_latch_in_real_clk_gate",
        "test_target": "highlight_tests",
    },
    {
        "label": "M2 the verif filelist stops naming the interface it points across to",
        "file": "demo/verif/verible.filelist",
        "needs_rebuild": False,  # read at run time, never compiled in
        "old": "../rtl/bus/axi4_lite_if.sv\n",
        "new": "",
        "test": "jumps_to_an_interface_declared_in_a_different_directory_via_the_verif_filelist",
        "test_target": "verilog_nav_tests",
    },
    # ---- F2: `program' blocks have a depth at all ---------------------
    {
        "label": "V1 `program_declaration' is no longer a block node (deletion entry: F2)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "program_declaration"',
        "new": '                "program_declaration_NEVER_MATCHES"',
        "test": "verilog_program_header_line_shares_the_documented_one_level_indent_quirk",
    },
    {
        "label": "V2 `endprogram' is no longer a closer",
        "file": "crates/core/lisp/indent.el",
        "old": '    "endprogram"',
        "new": '    "endprogram_NEVER_MATCHES"',
        "test": "verilog_every_demo_rtl_file_reindents_to_its_own_on_disk_columns_except_named_divergences",
    },
    # ---- F3: SVA property bodies --------------------------------------
    {
        "label": "V3 `property_spec' is no longer a block node (deletion entry: F3)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "property_spec"',
        "new": '                "property_spec_NEVER_MATCHES"',
        "test": "verilog_property_spec_body_indents_one_level_against_real_axi4_lite_if_sv",
    },
    # ---- F4: wrapped modport port lists -------------------------------
    {
        "label": "V4 `modport_item' takes no wrap step (deletion entry: F4)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "modport_item")))',
        "new": '                "modport_item_NEVER_MATCHES")))',
        "test": "verilog_modport_wrapped_port_list_gets_a_wrap_step_against_real_axi4_lite_if_sv",
    },
    {
        "label": "V5 the modport closing paren is no longer a contextual closer",
        "file": "crates/core/lisp/indent.el",
        "old": '    (")" . "modport_item")\n',
        "new": '    (")" . "modport_item_NEVER_MATCHES")\n',
        "test": "verilog_modport_wrapped_port_list_gets_a_wrap_step_against_real_axi4_lite_if_sv",
    },
    {
        "label": "V6 the modport closer's depth cancel is gone (its own side effect returns)",
        "file": "crates/core/lisp/indent.el",
        "old": "     (indent--verilog-modport-item-closer-adjust node)",
        "new": "     0",
        "test": "verilog_modport_wrapped_port_list_gets_a_wrap_step_against_real_axi4_lite_if_sv",
    },
    # ---- F5: compiler directives sit at column 0 ----------------------
    {
        "label": "V7 the `conditional_compilation_directive' short-circuit is gone (deletion entry: F5)",
        "file": "crates/core/lisp/indent.el",
        "old": '    (if (indent--node-has-ancestor-type-p node "conditional_compilation_directive")',
        "new": "    (if nil",
        "test": "verilog_conditional_compilation_directive_is_always_column_zero_against_real_files",
    },
    {
        "label": "V7b same short-circuit, witnessed two levels deep",
        "file": "crates/core/lisp/indent.el",
        "old": '    (if (indent--node-has-ancestor-type-p node "conditional_compilation_directive")',
        "new": "    (if nil",
        "test": "verilog_conditional_compilation_directive_nested_two_levels_deep_is_still_column_zero",
    },
    # ---- F6: `{...}' concatenation expressions ------------------------
    {
        "label": "V8 `concatenation' is no longer a block node (deletion entry: F6)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "concatenation"',
        "new": '                "concatenation_NEVER_MATCHES"',
        "test": "verilog_coverpoint_concatenation_expression_gets_a_block_level_and_its_closer_dedents",
    },
    {
        # The first version of this entry mutated a function,
        # `indent--verilog-concatenation-case-selector-closer-adjust', that
        # only ever existed because the closer was believed to dedent for a
        # `case' selector and NOT for a coverpoint. The trailing cold read
        # measured real verible and found both dedent, the special case was
        # deleted, and what is left to mutate is the plain closer entry.
        "label": "V9 a concatenation's closing `}' is no longer a closer (it stops dedenting)",
        "file": "crates/core/lisp/indent.el",
        "old": '    ("}" . "concatenation"))',
        "new": '    ("}" . "concatenation_NEVER_MATCHES"))',
        "test": "verilog_case_selector_concatenation_closer_dedents_the_same_way_as_a_coverpoint_expression",
    },
    # ---- F7: a wrapped bare statement after `else' --------------------
    {
        "label": "V10 the bare-action-block adjustment is gone (deletion entry: F7)",
        "file": "crates/core/lisp/indent.el",
        "old": "     (indent--verilog-bare-action-block-adjust node)))",
        "new": "     0))",
        "test": "verilog_bare_else_wrapped_statement_gets_a_block_level_against_real_sram_bank_tb_sv",
    },
    # ---- F8: the guards against a hand-maintained list drifting -------
    {
        "label": "V11 a demo file disappears from the hand-maintained indent-width array (deletion entry: F8)",
        "file": "crates/core/tests/demo_smoke_tests.rs",
        "old": '        ("rtl/bus/axi4_lite_if.sv", Some(2)),\n',
        "new": "",
        "test": "demo_rtl_verilog_files_detect_two_space_width_except_the_undersampled_svh",
        "test_target": "demo_smoke_tests",
    },
    {
        "label": "V12 a demo file disappears from the major-mode table",
        "file": "crates/core/tests/demo_smoke_tests.rs",
        "old": '        ("rtl/core/clk_gate.sv", "verilog-mode"),\n',
        "new": "",
        "test": "every_file_under_demo_opens_in_its_expected_major_mode",
        "test_target": "demo_smoke_tests",
    },
    # ---- F9: the wiring between the editor and the real material ------
    {
        "label": "V13 `program' loses its keyword rule -- caught by a test reading demo/ (deletion entry: F9)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"program" @keyword\n',
        "new": "",
        "test": "verilog_program_and_covergroup_in_real_sram_bank_tb",
        "test_target": "highlight_tests",
    },
    {
        "label": "V14 `program_declaration' stops being a scope kind -- caught by a real-file breadcrumb",
        "file": "crates/core/src/scope.rs",
        "old": '                | "program_declaration"\n',
        "new": "",
        "test": "chain_inside_the_program_in_real_sram_bank_tb_sv",
        "test_target": "scope_header_tests",
    },
    # ---- Declared survivors: real behaviour no tool in this repo sees ----
    #
    # These three are here because leaving them OUT is indistinguishable, to
    # anyone later reading only this file, from nobody having thought about
    # them. Each ships real behaviour; none of it is observable from
    # `cargo test', and for two of them not from `demo/tools/run_sim.sh'
    # either. That is a fact worth executing rather than an intention worth
    # recording, so they run and report SURVIVED on purpose.
    {
        "label": "S1 the per-bank grant gating is removed (expected SURVIVED: gnt_o is hardwired)",
        "file": "demo/rtl/mem/sram_bank.sv",
        "needs_rebuild": False,  # read at run time, never compiled in
        "old": "      StWriteIssue: if (bank_gnt[wr_bank_q]) state_d = StWriteResp;",
        "new": "      StWriteIssue: state_d = StWriteResp;",
        "test": "every_file_under_demo_opens_in_its_expected_major_mode",
        "test_target": "demo_smoke_tests",
        "expect": "survived",
        # `demo/rtl/mem/sram_wrapper.sv:37' is `assign gnt_o = 1'b1;' -- a
        # behavioural model that never withholds a grant. So the guard is
        # correct forward-looking RTL for a vendor macro that CAN stall, and
        # nothing in this repo can distinguish it from the unconditional
        # transition: not `cargo test', and not `demo/tools/run_sim.sh'
        # either, whose Icarus run drives the same never-stalling model.
        # What would see it is a wrapper with back-pressure, which this demo
        # deliberately does not have.
    },
    {
        "label": "S2 the monitor's write-outstanding counter stops counting (expected SURVIVED: no SVA simulator)",
        "file": "demo/verif/axi4_lite_monitor.sv",
        "needs_rebuild": False,  # read at run time, never compiled in
        "old": "        2'b10:   outstanding_wr <= outstanding_wr + 1;",
        "new": "        2'b10:   outstanding_wr <= outstanding_wr;",
        "test": "every_file_under_demo_opens_in_its_expected_major_mode",
        "test_target": "demo_smoke_tests",
        "expect": "survived",
        # `demo/verif/axi4_lite_monitor.sv' is excluded from
        # `demo/tools/run_sim.sh' for two independent reasons measured on
        # this machine: Icarus 13.0 rejects an interface-typed module port
        # (`Errors in port declarations.') and rejects concurrent assertions
        # (`syntax error' / `Invalid module item'). There is no SVA-capable
        # simulator here, so nothing executes this counter at all. The only
        # review this file will ever get is a human (or an agent) reading it
        # -- which is exactly how the one-cycle `$past()' bug that preceded
        # this counter was found, and it is the reason this entry says so out
        # loud instead of leaving the gap silent.
    },
    {
        "label": "S3 the testbench driver stops holding VALID (expected SURVIVED: the check is a shell script)",
        "file": "demo/tools/run_sim.sh",
        "needs_rebuild": False,  # read at run time, never compiled in
        "old": "if [[ $ran -lt 2 ]]; then",
        "new": "if [[ $ran -lt 0 ]]; then",
        "test": "every_file_under_demo_opens_in_its_expected_major_mode",
        "test_target": "demo_smoke_tests",
        "expect": "survived",
        # The simulation floor -- the thing that stops the script reporting
        # success after running nothing -- is enforced in bash and checked by
        # running the script, never by `cargo test'. Witness for this one:
        # `demo/tools/run_sim.sh' with a doctored PATH must exit non-zero and
        # name the missing tool; with `MIN_SIMULATIONS=0' it would exit 0
        # having run nothing. Recorded here so the gap is on the record
        # rather than in someone's head.
    },
]
