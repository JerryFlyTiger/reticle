# Mutation list for M142 (complete Verilog/SystemVerilog keyword highlighting).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m142.py -p core
#
# Per M117's rule, a deletion entry is required per FEATURE. M142 ships:
#
#   A, non-ANSI port directions and `ref` -> A1..A4
#   B, the bare @keyword block             -> B1..B12
#   C, the @type block (types + gates)     -> C1..C3
#   D, the operator/connective cut          -> D1..D8 (D6-D8: the allowlist)
#   E, the sweep's own floors               -> E1..E3
#   T, the tick_until hang-guard ceiling    -> T1 (declared survivor)
#
# B1/B2 and C1/C2 delete a whole block. Their `old` text is read from the query
# file at load time, from the section's marker comment up to the next blank
# line, so the entry removes exactly what is shipped instead of a hand-copied
# snapshot that could drift. The runner still refuses an `old` that occurs zero
# times or more than once.
#
# C3 swaps a face instead of deleting a rule. The sweep accepts either keyword
# or type face, so only the named gate test can see it; C3 is the executed
# evidence for the comment saying so.

import pathlib

PACKAGE = "core"
TEST_TARGET = "highlight_tests"

_Q = "crates/core/queries/verilog-highlights.scm"
_T = "crates/core/tests/highlight_tests.rs"
_SWEEP = "verilog_keyword_sweep_covers_every_reserved_word_leaf"

# dev/mutate.py exec()s this file with no `__file__`, from the repo root.
_query = pathlib.Path(_Q).read_text()


def _block(marker):
    start = _query.index(marker)
    end = _query.index("\n\n", start)
    return _query[start:end + 1]


_KEYWORD_BLOCK = _block(";; -- everything else reachable as a bare leaf")
_TYPE_BLOCK = (_block(";; -- gate/switch instantiation TYPE words")
               + "\n" + _block(";; -- type keywords, no second role"))

MUTATIONS = [
    # ---- A: non-ANSI port directions and ref ------------------------------
    {
        "label": "A1 query fails to compile (whole-file effect removed)",
        "file": _Q,
        "old": '"defparam" @keyword\n',
        "new": '"defparam @keyword\n',
        "test": "verilog_nonansi_ports_defparam_ref_get_keyword_face",
    },
    {
        "label": "A2 non-ANSI input/output/inout lose their face (deletion entry)",
        "file": _Q,
        "old": '"input" @keyword\n"output" @keyword\n"inout" @keyword\n',
        "new": "",
        "test": "verilog_nonansi_ports_defparam_ref_get_keyword_face",
    },
    {
        "label": "A3 same deletion, seen by the sweep over demo/ files",
        "file": _Q,
        "old": '"input" @keyword\n"output" @keyword\n"inout" @keyword\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "A4 `ref' loses its face (no wrapper left to cover it)",
        "file": _Q,
        "old": '"ref" @keyword\n',
        "new": "",
        "test": "verilog_const_ref_gets_exact_span_keyword_faces",
    },
    # ---- B: the bare @keyword block ---------------------------------------
    {
        "label": "B1 whole @keyword block deleted (deletion entry), named test",
        "file": _Q,
        "old": _KEYWORD_BLOCK,
        "new": "",
        "test": "verilog_package_import_export_endpackage_get_keyword_face",
    },
    {
        "label": "B2 whole @keyword block deleted, seen by the sweep",
        "file": _Q,
        "old": _KEYWORD_BLOCK,
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "B3 `static' loses its face (the wrapper double-capture is gone)",
        "file": _Q,
        "old": '"static" @keyword\n',
        "new": "",
        "test": "verilog_class_qualifiers_get_keyword_face",
    },
    {
        "label": "B4 `super' loses its face",
        "file": _Q,
        "old": '"super" @keyword\n',
        "new": "",
        "test": "verilog_super_gets_keyword_face",
    },
    {
        "label": "B5 `weak' loses its face",
        "file": _Q,
        "old": '"weak" @keyword\n',
        "new": "",
        "test": "verilog_weak_gets_keyword_face",
    },
    {
        "label": "B6 bare `posedge' deleted: the specify timing-check leaf goes unfaced",
        "file": _Q,
        "old": '"posedge" @keyword\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "B7 unique/unique0/priority lose their face",
        "file": _Q,
        "old": "(unique_priority) @keyword\n",
        "new": "",
        "test": "verilog_unique_priority_unique0_get_keyword_face",
    },
    {
        "label": "B8 `bins' loses its face",
        "file": _Q,
        "old": '"bins" @keyword\n',
        "new": "",
        "test": "verilog_bins_gets_keyword_face",
    },
    {
        "label": "B9 `endproperty' loses its face",
        "file": _Q,
        "old": '"endproperty" @keyword\n',
        "new": "",
        "test": "verilog_property_sequence_closers_get_keyword_face",
    },
    {
        "label": "B10 `while' loses its face (found only by the sweep, in demo/verif)",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "B11 bare `negedge' deleted: the specify $hold leaf goes unfaced",
        "file": _Q,
        "old": '"negedge" @keyword\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "B12 a removed wrapper comes back: `const ref' faced as one span",
        "file": _Q,
        "old": '"ref" @keyword\n',
        "new": '"ref" @keyword\n(tf_port_direction) @keyword\n',
        "test": "verilog_const_ref_gets_exact_span_keyword_faces",
    },
    # ---- C: the @type block -----------------------------------------------
    {
        "label": "C1 whole @type block deleted (deletion entry), type-word test",
        "file": _Q,
        "old": _TYPE_BLOCK,
        "new": "",
        "test": "verilog_typedef_enum_struct_union_signed_unsigned_get_type_face",
    },
    {
        "label": "C2 whole @type block deleted, gate test",
        "file": _Q,
        "old": _TYPE_BLOCK,
        "new": "",
        "test": "verilog_gate_instantiation_type_gets_type_face",
    },
    {
        "label": "C3 gate type face swapped to keyword (the sweep cannot see this)",
        "file": _Q,
        "old": "(n_input_gatetype) @type\n",
        "new": "(n_input_gatetype) @keyword\n",
        "test": "verilog_gate_instantiation_type_gets_type_face",
    },
    # ---- D: the operator/connective cut ------------------------------------
    {
        "label": "D1 bare `or' faced: sensitivity-list connective colored",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": '"while" @keyword\n"or" @keyword\n',
        "test": "verilog_sensitivity_or_stays_plain",
    },
    {
        "label": "D2 bare `and' faced: property connective colored",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": '"while" @keyword\n"and" @keyword\n',
        "test": "verilog_property_and_stays_plain",
    },
    {
        "label": "D3 bare `with' faced: randomize() with colored",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": '"while" @keyword\n"with" @keyword\n',
        "test": "verilog_with_and_dist_stay_plain",
    },
    {
        "label": "D4 bare `dist' faced: constraint dist colored",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": '"while" @keyword\n"dist" @keyword\n',
        "test": "verilog_with_and_dist_stay_plain",
    },
    {
        "label": "D5 bare `s_until_with' faced again (fix round 2 moved it to the cut)",
        "file": _Q,
        "old": '"while" @keyword\n',
        "new": '"while" @keyword\n"s_until_with" @keyword\n',
        "test": "verilog_property_and_stays_plain",
    },
    {
        "label": "D6 `dist' dropped from the sweep allowlist (the fixture's dist leaf)",
        "file": _T,
        "old": '    ("dist", "expression_or_dist"),\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "D7 `with' dropped from the sweep allowlist",
        "file": _T,
        "old": '    ("with", "randomize_call"),\n',
        "new": "",
        "test": _SWEEP,
    },
    {
        "label": "D8 `randomize' dropped from the sweep allowlist",
        "file": _T,
        "old": '    ("randomize", "randomize_call"),\n',
        "new": "",
        "test": _SWEEP,
    },
    # ---- E: the sweep's floors ---------------------------------------------
    {
        "label": "E1 corpus floor unreachable",
        "file": _T,
        "old": "corpus.len() >= 10,",
        "new": "corpus.len() >= 999,",
        "test": _SWEEP,
    },
    {
        "label": "E2 discovery finds nothing (extension list broken)",
        "file": _T,
        "old": '["sv", "svh", "v", "vh"]',
        "new": '["svXX"]',
        "test": _SWEEP,
    },
    {
        "label": "E3 keyword-leaf floor unreachable",
        "file": _T,
        "old": "total_leaves >= 1200,",
        "new": "total_leaves >= 999999,",
        "test": _SWEEP,
    },
    # ---- T: the tick_until ceiling (fix round 2) ----------------------------
    # Declared survivor. Putting the old ~2 s budget back turns tests red only
    # when the machine is loaded (measured: 17 tests green at load 19-20, 31
    # tests with 20 red at load ~14), and this runner executes one filtered test
    # at a time, which is exactly the shape that never reproduces it. Running it
    # records that the ceiling is a hang guard `cargo test` cannot observe, not a
    # claim that it is covered.
    {
        "label": "T1 tick_until back to a ~2 s ceiling (only observable under load)",
        "file": _T,
        "old": "std::time::Instant::now() + std::time::Duration::from_secs(30);",
        "new": "std::time::Instant::now() + std::time::Duration::from_secs(2);",
        "test": "verilog_bins_gets_keyword_face",
        "expect": "survived",
    },
]
