# M97 mutation list -- the indent half, which runs against a different target.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m97-interface-indent.py \
#         -p core --test-target indent_tests
#
# This is the half that was already shipping a defect: `package_declaration'
# was never in the block list, so pressing TAB inside `demo/rtl/pkg/soc_pkg.sv'
# -- this project's own showcase package -- flattened its body to column 0.
# The SUSPECTED inventory entry was about interfaces; the package case is what
# reconnaissance found on the way.

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    {
        "label": "GH1 package/interface/class bodies stop counting as blocks",
        "file": "crates/core/lisp/indent.el",
        "old": '    (verilog . ("module_declaration" "interface_declaration" "package_declaration"\n                "class_declaration" "seq_block" "function_body_declaration"',
        "new": '    (verilog . ("module_declaration" "seq_block" "function_body_declaration"',
        "test": "verilog_package_body_indents_one_level_and_endpackage_dedents",
    },
    {
        "label": "GH2 the new closers stop dedenting",
        "file": "crates/core/lisp/indent.el",
        "old": '\'("end" "endmodule" "endfunction" "endtask" "endinterface" "endpackage" "endclass")',
        "new": '\'("end" "endmodule" "endfunction" "endtask")',
        "test": "verilog_package_body_indents_one_level_and_endpackage_dedents",
    },
]
