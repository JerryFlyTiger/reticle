# Mutation list for M153 (demo/rtl/core/exec_unit.sv: port-list AUTOINPUT/
# AUTOOUTPUT and part-select AUTOWIRE, pinned byte-for-byte by
# demo_rtl_exec_unit_port_list_auto_matches_editor_output).
# Run by the main conversation, never the implementer.
#
#     PYTHONUNBUFFERED=1 python3 -u dev/mutate.py --config dev/mutations/m153.py -p core
#
# Per M117's rule, a deletion entry per shipped effect:
#   D1 port-list AUTO insertion removed (the demo's whole point)
#   D2 AUTOWIRE back to bare identifiers only (the pre-M152 defect)
#   D3 the demo file loses its AUTOWIRE expansion on disk
#   D4 declared survivor: the demo file loses /*AUTOWIRE*/ marker AND expansion.
#      Regenerating then produces no wires either, so the byte check agrees
#      with the broken file. The file is then illegal Verilog (operand_a/b
#      undeclared); slang sees it (`dev/lsp-probe.py --diagnostics`, severity 1),
#      `cargo test` does not.
#
# Not listed, because no entry can reach it: the test's banner-position
# assertion runs only after the byte-for-byte assert has passed, so every
# product regression that moves the banners fails the byte assert first.
# It protects the fixture, not the product (see the comment on it).

PACKAGE = "core"
TEST_TARGET = "demo_smoke_tests"

_F = "crates/core/lisp/verilog-auto.el"
_D = "demo/rtl/core/exec_unit.sv"
_T = "demo_rtl_exec_unit_port_list_auto_matches_editor_output"

_WIRES = """  /*AUTOWIRE*/
  // Beginning of automatic wires (for undeclared instantiated-module outputs)
  wire [DataWidth-1:0] operand_a;
  wire [DataWidth-1:0] operand_b;
  // End of automatics
"""

MUTATIONS = [
    {
        "label": "D1 port-list AUTO insertion removed (markers inside the ANSI list no longer expand)",
        "file": _F,
        "old": "  (let ((range (verilog-auto--header-port-list-paren-range header)))",
        "new": "  (let ((range nil))",
        "test": _T,
    },
    {
        "label": "D2 AUTOWIRE back to bare identifiers only (operand_a/b undeclared again)",
        "file": _F,
        "old": "\n                              (verilog-auto--connection-candidate-name ctext))))",
        "new": "\n                              (and (verilog-auto--bare-identifier-p ctext) ctext))))",
        "test": _T,
    },
    {
        "label": "D3 the demo file loses its AUTOWIRE expansion on disk (marker kept)",
        "file": _D,
        "old": _WIRES,
        "new": "  /*AUTOWIRE*/\n",
        "test": _T,
        "needs_rebuild": False,
    },
    {
        "label": "D4 the demo file loses the AUTOWIRE marker and expansion (declared survivor: slang-only)",
        "file": _D,
        "old": _WIRES,
        "new": "",
        "test": _T,
        "needs_rebuild": False,
        "expect": "survived",
    },
]
