# M97 (interfaces, and the block types the indent engine never learned)
# mutation list -- the verilog-auto half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m97-interface.py \
#         -p core --test-target verilog_auto_tests
#
# The indent, completion, nav and highlight halves run against other targets;
# see the sibling files.
#
# GG3 restores a crash, not a missing feature. The coordinator's own spec told
# the implementer NOT to widen the `--enclosing-of-type' calls, on the grounds
# that they find the module CONTAINING an AUTOINST comment, which is always a
# module. That holds for AUTOINST and is wrong for AUTOWIRE: when the comment
# sits inside an interface, the containing declaration IS an interface. The
# cold read found it by running the code.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "GG1 interfaces stop being resolvable instantiation targets",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "  (verilog-auto--find-all-of-types root '(\"module_declaration\" \"interface_declaration\")))",
        "new": "  (verilog-auto--find-all-of-type root \"module_declaration\"))",
        "test": "autoinst_against_an_interface_ansi_header_target",
    },
    {
        "label": "GG2 an interface's ANSI header is misrouted down the non-ANSI branch",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(defconst verilog-auto--ansi-header-types '(\"module_ansi_header\" \"interface_ansi_header\")",
        "new": "(defconst verilog-auto--ansi-header-types '(\"module_ansi_header\")",
        "test": "autoinst_against_an_interface_ansi_header_target",
    },
    {
        "label": "GG3 AUTOWIRE inside an interface crashes again (uncaught wrong-type error)",
        "file": "crates/core/lisp/verilog-auto.el",
        # The one-line form appears at two other sites; the crash-causing one
        # in `--expand-autowire-site' is wrapped across two lines and unique.
        "old": "  (let* ((module-decl (verilog-auto--enclosing-of-types\n                       comment '(\"module_declaration\" \"interface_declaration\")))",
        "new": "  (let* ((module-decl (verilog-auto--enclosing-of-types\n                       comment '(\"module_declaration\")))",
        "test": "autowire_inside_an_interface_is_a_clean_no_op_not_a_crash",
    },
    {
        "label": "GG4 the two-kind walk becomes two concatenated walks (document order lost)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(defun verilog-auto--find-all-of-types (node types)",
        "new": "(defun verilog-auto--find-all-of-types-unused (node types)",
        "test": "top_level_modules_preserves_document_order_across_interleaved_modules_and_interfaces",
        "note": (
            "Renaming the definition is blunt but genuine: nothing else "
            "provides an order-preserving walk across two node kinds, and a "
            "subtler replacement would mean reimplementing the concatenated "
            "version inline, which is a rewrite rather than a mutation."
        ),
    },
]
