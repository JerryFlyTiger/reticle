# Mutation list for M119 (Verilog indentation: four divergences from real
# `verible-verilog-format`). Run by the main conversation, never by the
# implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m119.py -p core
#
# Per the rule M117 wrote into CLAUDE.md, the deletion entry is per FEATURE.
# M119 ships four fixes plus one structural check, and each has an entry that
# removes the whole thing:
#
#   D1 wrapped ANSI header list -> V1 (the node types), V2 (the depth cancel),
#                                  V3 (the closing-paren dedent),
#                                  V4a/V4b (the two opener dedents)
#   D2 generate depth           -> V5 (the whole adjustment), V6/V7 (the nesting)
#   D2 nesting linearity        -> V12
#   D3 class constructor        -> V8
#   D4 covergroup / coverpoint  -> V9 (the block types), V10 (`endgroup'),
#                                  V11 (the bins-specific `}' closer)
#
# All are real; none is declared a survivor. Indentation is plain elisp driven
# by `cargo test`, so a screenshot-only answer here would mean something had
# gone wrong with the placement.
#
# V3 exists because the cold read found that the four contextual closer conses
# could ALL be deleted with both D1 unit tests still green -- they asserted only
# the continuation-content lines, never the header's own closing-paren lines.
# The fix round added those assertions; V3 is what proves they bite.
#
# V4a/V4b are a correction to this list itself. The first run had ONE entry that
# deleted both opener conses at once and named the interface unit test, which
# witnesses neither -- it reported SURVIVED, and the honest reading was "this
# entry is mis-aimed", not "these entries are dead". Split apart, each half has
# a real witness: `#(' is load-bearing on `demo/rtl/top/soc_top.sv:40', and the
# port-list `(' needs a shape `demo/rtl' does not contain (an `import' clause
# with no parameter list), which now has its own named test.
#
# V10 is the one the reviewer flagged as most load-bearing. `indent--verilog-
# closers` deliberately mixes plain strings with `(TOKEN . PARENT-TYPE)` conses,
# and `indent--closer-token-at-p` takes the first entry that matches. Turning
# the bins-specific `("}" . "bins_or_empty")` into a blanket `"}"` is the exact
# shape of the mistake a future maintainer would make, and it silently breaks an
# UNRELATED construct (a `typedef struct packed { ... } req_t;` closing brace).

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    # ---- D1: the wrapped ANSI parameter/port list ---------------------
    {
        "label": "V1 the header's own lists stop taking a wrap step (deletion entry: D1)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "list_of_arguments" "parameter_port_list" "list_of_port_declarations")))',
        "new": '                "list_of_arguments")))',
        "test": "verilog_ansi_header_wrapped_port_and_parameter_lists_get_a_wrap_step_to_four",
    },
    {
        "label": "V2 module_declaration's block depth is no longer cancelled for header content",
        "file": "crates/core/lisp/indent.el",
        "old": '  (if (or (indent--node-has-ancestor-type-p node "parameter_port_list")\n          (indent--node-has-ancestor-type-p node "list_of_port_declarations"))\n      -1\n    0))',
        "new": '  (if (or (indent--node-has-ancestor-type-p node "parameter_port_list")\n          (indent--node-has-ancestor-type-p node "list_of_port_declarations"))\n      0\n    0))',
        "test": "verilog_ansi_header_wrapped_port_and_parameter_lists_get_a_wrap_step_to_four",
    },
    {
        "label": "V3 the header's closing-paren line loses its dedent",
        "file": "crates/core/lisp/indent.el",
        "old": '    (")" . "parameter_port_list")\n    (")" . "list_of_port_declarations")',
        "new": "",
        "test": "verilog_ansi_header_wrapped_port_and_parameter_lists_get_a_wrap_step_to_four",
    },
    {
        # First run mis-targeted this: it removed BOTH opener entries at once and
        # aimed at the interface unit test, which covers neither. Split, and each
        # half now names the test that actually witnesses it.
        "label": "V4a the `#(' opener stops dedenting (witnessed on real showcase material)",
        "file": "crates/core/lisp/indent.el",
        "old": '    ("#" . "parameter_port_list")\n',
        "new": "",
        "test": "verilog_every_demo_rtl_file_reindents_to_its_own_on_disk_columns_except_named_divergences",
    },
    {
        "label": "V4b the port-list `(' opener stops dedenting (import clause, no parameter list)",
        "file": "crates/core/lisp/indent.el",
        "old": '    ("(" . "list_of_port_declarations"))',
        "new": "    )",
        "test": "verilog_import_clause_with_no_parameter_list_pushes_port_list_open_paren_to_its_own_line_at_column_zero",
    },
    # ---- D2: generate depth ------------------------------------------
    {
        "label": "V5 the generate depth correction is removed entirely (deletion entry: D2)",
        "file": "crates/core/lisp/indent.el",
        "old": "  (+ (indent--verilog-generate-depth-adjust node)\n     (indent--verilog-header-wrap-depth-adjust node))",
        "new": "  (+ 0\n     (indent--verilog-header-wrap-depth-adjust node))",
        "test": "verilog_unwrapped_module_scope_if_generate_is_one_level_shallower_than_the_wrapped_form",
    },
    {
        "label": "V6 the ancestor COUNT collapses to 'is there at least one' (the pre-fix-round bug)",
        "file": "crates/core/lisp/indent.el",
        # This is the flat -1 the first implementation used: stop counting after
        # the first generate ancestor. Single-level shapes stay correct, nested
        # ones go one level too deep per extra level -- exactly the defect the
        # cold read found, so the nested tests are the ones that must bite.
        "old": "      (when (member (treesit-node-type n) indent--verilog-generate-construct-types)\n        (setq count (1+ count)))",
        "new": "      (when (and (= count 0)\n                 (member (treesit-node-type n) indent--verilog-generate-construct-types))\n        (setq count (1+ count)))",
        "test": "verilog_nested_unwrapped_if_generate_indents_one_level_per_nesting_level",
    },
    {
        "label": "V7 the same collapse, seen through the WRAPPED nested form",
        "file": "crates/core/lisp/indent.el",
        "old": "      (when (member (treesit-node-type n) indent--verilog-generate-construct-types)\n        (setq count (1+ count)))",
        "new": "      (when (and (= count 0)\n                 (member (treesit-node-type n) indent--verilog-generate-construct-types))\n        (setq count (1+ count)))",
        "test": "verilog_nested_wrapped_if_generate_indents_one_level_per_nesting_level",
    },
    {
        # Added in the trailing round. V6/V7 collapse the count to "at least
        # one", which is wrong at every depth >= 2; this entry instead breaks
        # only the LINEARITY, staying correct at depth 2 and wrong at depth 3.
        # V6/V7 cannot distinguish "linear in count" from "right at depth 2",
        # and neither could any test before this one existed.
        "label": "V12 the nesting correction stops being linear in the ancestor count",
        "file": "crates/core/lisp/indent.el",
        "old": "        (- (+ (1- count)",
        "new": "        (- (+ (min 1 (1- count))",
        "test": "verilog_three_level_nested_generate_stays_linear_in_the_nesting_count",
    },
    # ---- D3: the class constructor ------------------------------------
    {
        "label": "V8 a class constructor stops being a block (deletion entry: D3)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "class_constructor_declaration"\n',
        "new": "",
        "test": "verilog_class_constructor_body_indents_the_same_as_an_ordinary_method_in_the_same_class",
    },
    # ---- D4: covergroup / coverpoint ----------------------------------
    {
        "label": "V9 covergroup and its bin list stop being blocks (deletion entry: D4)",
        "file": "crates/core/lisp/indent.el",
        "old": '                "covergroup_declaration" "bins_or_empty")))',
        "new": "                )))",
        "test": "verilog_covergroup_coverpoint_wrapped_bins_indent_one_level_deeper_than_coverpoint_header",
    },
    {
        "label": "V10 `endgroup' stops dedenting",
        "file": "crates/core/lisp/indent.el",
        "old": '    "endgroup"\n',
        "new": "",
        "test": "verilog_covergroup_coverpoint_wrapped_bins_indent_one_level_deeper_than_coverpoint_header",
    },
    {
        "label": "V11 the bins-specific `}' closer becomes a blanket one, breaking struct/enum",
        "file": "crates/core/lisp/indent.el",
        # The single most load-bearing entry: this is the mistake the closers
        # docstring now warns a future maintainer about, and its damage lands on
        # a construct that has nothing to do with covergroups.
        "old": '    ("}" . "bins_or_empty")',
        "new": '    "}"',
        "test": "verilog_enum_and_struct_closing_brace_lines_unaffected_still_two",
    },
]
