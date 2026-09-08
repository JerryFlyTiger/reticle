# Mutation list for M124 (SystemVerilog interfaces, non-ANSI port lists, and
# class prototypes). Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m124.py -p core
#
# Per the rule M117 wrote into CLAUDE.md, the deletion entry is per FEATURE, not
# per file. M124 ships five distinct effects and each has at least one entry
# below that removes the whole thing rather than perturbing a boundary inside it:
#
#   A non-ANSI `list_of_ports' indents like verible   -> V1, V2, V3, V4
#   B `M-.' on an interface-typed port                -> V5
#   C AUTOINST's `// Interfaces' category             -> V7
#   D the three prototype scope kinds                 -> V11
#   E the new non-ANSI demo module actually runs      -> V14 (declared SURVIVOR)
#
# V6, V8, V9, V10, V12 and V13 are boundaries INSIDE those features that the
# deletion entries do not isolate: which name the modport position resolves to,
# the label text, the group ORDER (GNU emits Interfaces first), the AUTOARG leak
# the shared helper makes possible, and the two label arms in `verilog_label'.
#
# V10 is the one worth explaining. `verilog-auto--group-by-direction' and
# `verilog-auto--grouped-lines' are shared by AUTOINST and AUTOARG, and GNU's
# AUTOARG has NO interfaces category (verilog-mode.el:12124-12143) while its
# AUTOINST does (:12852-12862). So the risk this milestone creates is not a
# missing feature, it is a leak: the fourth bucket showing up in AUTOARG output.
# V10 reclassifies plain inputs as interfaces, which is exactly what such a leak
# would look like from AUTOARG's side.
#
# V14 is declared `"expect": "survived"'. Part E's teeth are in a shell script:
# `demo/tools/run_sim.sh' compiles gray_ctr.v with Icarus and its testbench
# checks that consecutive Gray codes differ in exactly one bit. Nothing in
# `cargo test' simulates anything, so gutting the Gray encoding cannot turn any
# named Rust test red -- the reindent test in `indent_tests.rs' reads the file's
# COLUMNS, which this mutation does not change. Recording it as a declared
# survivor (rather than leaving it out) is what makes the deletion answer for
# Part E a fact that was executed instead of an intention. The main conversation
# additionally applies V14 by hand and runs `demo/tools/run_sim.sh' to show the
# testbench actually goes red; that result is written into the M124 record.
#
# Style note (M83/M85, restated in CLAUDE.md): every entry is REPLACEMENT-style.
# A stub inserted ahead of a real definition does not shadow it -- elisp's
# `defun' and the Rust `defun()' registration both let the LAST definition win,
# and a binding inserted into a series gets rebuilt away by the lines after it.

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    # ---- A: non-ANSI `list_of_ports' indentation ------------------------
    {
        "label": "V1 `list_of_ports' is not a wrap-list type again (deletion entry: A)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "list_of_ports"\n',
        "new": '                "list_of_ports_MUTANT"\n',
        "test": "verilog_non_ansi_module_header_wrapped_port_list_continuation_lands_at_wrap_step",
    },
    {
        "label": "V2 the header-wrap depth cancellation stops seeing `list_of_ports'",
        "file": "crates/core/lisp/indent.el",
        "old": '          (indent--node-has-ancestor-type-p node "list_of_ports"))',
        "new": '          (indent--node-has-ancestor-type-p node "list_of_ports_MUTANT"))',
        "test": "verilog_non_ansi_module_header_wrapped_port_list_continuation_lands_at_wrap_step",
    },
    {
        "label": "V3 the closing `);' loses its closer entry",
        "file": "crates/core/lisp/indent.el",
        "old": '    (")" . "list_of_ports")',
        "new": '    (")" . "list_of_ports_MUTANT")',
        "test": "verilog_non_ansi_module_header_closing_paren_line_is_column_zero",
    },
    {
        "label": "V4 the OPENING `(' loses its closer entry (the fix-round finding)",
        "file": "crates/core/lisp/indent.el",
        "old": '    ("(" . "list_of_ports")',
        "new": '    ("(" . "list_of_ports_MUTANT")',
        "test": "verilog_non_ansi_import_clause_pushes_port_list_open_paren_to_its_own_line_at_column_zero",
    },
    # ---- B: `M-.' from an interface-typed port ---------------------------
    {
        "label": "V5 the whole interface_port_header branch is dead (deletion entry: B)",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "                (when iph\n",
        "new": "                (when (and iph nil)\n",
        "test": "jumps_from_an_interface_typed_port_s_interface_name_to_the_interface_declaration",
        "test_target": "verilog_nav_tests",
    },
    {
        "label": "V6 the modport position resolves to the MODPORT's name, not the interface's",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "                     ((and modport-node (verilog-nav--point-in-node-p pos modport-node) iface-node)\n                      (treesit-node-text iface-node))",
        "new": "                     ((and modport-node (verilog-nav--point-in-node-p pos modport-node) iface-node)\n                      (treesit-node-text modport-node))",
        "test": "jumps_from_an_interface_typed_port_s_modport_name_to_the_interface_declaration",
        "test_target": "verilog_nav_tests",
    },
    # ---- C: AUTOINST's `// Interfaces' category --------------------------
    {
        "label": "V7 an interface-typed port is an input again (deletion entry: C)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '   ((verilog-auto--find-first-of-type node "interface_port_header") \'interface)',
        "new": '   ((verilog-auto--find-first-of-type node "interface_port_header_MUTANT") \'interface)',
        "test": "autoinst_interface_typed_port_gets_its_own_interfaces_category",
        "test_target": "verilog_auto_tests",
    },
    {
        "label": "V8 the header text is wrong (label list's first entry)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '  (let ((labels \'("// Interfaces" "// Outputs" "// Inouts" "// Inputs"))',
        "new": '  (let ((labels \'("// Interfacez" "// Outputs" "// Inouts" "// Inputs"))',
        "test": "autoinst_interface_typed_port_gets_its_own_interfaces_category",
        "test_target": "verilog_auto_tests",
    },
    {
        "label": "V9 the interfaces group is emitted LAST, not first (GNU emits it first)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "    (list (nreverse interfaces) (nreverse outputs) (nreverse inouts) (nreverse inputs))))",
        "new": "    (list (nreverse outputs) (nreverse inouts) (nreverse inputs) (nreverse interfaces))))",
        "test": "autoinst_interface_plus_input_plus_output_gets_all_groups_in_gnu_order",
        "test_target": "verilog_auto_tests",
    },
    {
        "label": "V10 the fourth bucket leaks into AUTOARG (inputs classified as interfaces)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "      (cond ((eq (nth 1 p) 'interface) (push p interfaces))",
        "new": "      (cond ((eq (nth 1 p) 'input) (push p interfaces))",
        "test": "autoarg_output_unchanged_by_the_interfaces_bucket",
        "test_target": "verilog_auto_tests",
    },
    # ---- D: the three prototype scope kinds ------------------------------
    {
        "label": "V11 none of the three prototype kinds is a scope again (deletion entry: D)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "class_constructor_prototype"\n                | "task_prototype"\n                | "function_prototype"',
        "new": '                | "class_constructor_prototype_MUTANT"\n                | "task_prototype_MUTANT"\n                | "function_prototype_MUTANT"',
        "test": "verilog_class_constructor_prototype_extern",
        "test_target": "lib",
    },
    {
        "label": "V12 the task/function prototype label arm is gone (label becomes the kind string)",
        "file": "crates/core/src/scope.rs",
        "old": '        "task_prototype" | "function_prototype" => node',
        "new": '        "task_prototype_MUTANT" | "function_prototype_MUTANT" => node',
        "test": "verilog_task_prototype_extern",
        "test_target": "lib",
    },
    {
        "label": "V13 a constructor PROTOTYPE no longer labels as `new'",
        "file": "crates/core/src/scope.rs",
        "old": '        "class_constructor_declaration" | "class_constructor_prototype" => "new".to_string(),',
        "new": '        "class_constructor_declaration" => "new".to_string(),',
        "test": "verilog_class_constructor_prototype_extern",
        "test_target": "lib",
    },
    # ---- E: the demo module's Gray encoding (declared survivor) ----------
    {
        "label": "V14 gray_ctr.v stops encoding Gray at all (deletion entry: E, declared SURVIVOR -- only demo/tools/run_sim.sh sees it)",
        "file": "demo/rtl-verilog2001/gray_ctr.v",
        "old": "  assign gray_count = (bin_count >> 1) ^ bin_count;",
        "new": "  assign gray_count = bin_count;",
        "test": "verilog_demo_rtl_verilog2001_gray_ctr_v_full_file_reindents_to_its_own_on_disk_columns",
        "expect": "survived",
        # The trailing cold read caught this one missing. `gray_ctr.v' is
        # read by the test at RUN time (`fs::read_to_string'), never
        # compiled into the binary, so mutating it produces no
        # `Compiling core' line and the runner's `not built' check would
        # report NOBUILD. In the M124 run it happened to report SURVIVED
        # anyway -- but only because the PRECEDING entry restored
        # `scope.rs' and bumped its mtime, which forced a rebuild on this
        # entry's own `cargo test'. That is entry ORDER doing the work,
        # not this entry; `--only V14' on its own would have said
        # NOBUILD. Same reason M122's demo-material entries set it.
        "needs_rebuild": False,
    },
]
