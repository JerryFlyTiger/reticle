# M89 (SystemVerilog declaration and type-reference highlighting) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m89-sv-highlight.py \
#         -p core --test-target font_lock_philosophy_tests
#
# Deleting a query rule is the natural replacement-style mutation for a `.scm`
# file: the rule line is replaced by a comment, so the query still compiles and
# the only thing that changes is that one pattern no longer matches.
#
# K9 and K10 are the two that matter most -- they are not "delete a feature"
# mutations but "put the defect back", restoring the exact over-matching shapes
# the cold read found.

PACKAGE = "core"
TEST_TARGET = "font_lock_philosophy_tests"

MUTATIONS = [
    {
        "label": "K1 typedef names lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(type_declaration type_name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K2 package names lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(package_declaration name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K3 class names lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(class_declaration name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K4 ANSI interface header names lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(interface_ansi_header name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K5 non-ANSI interface header names lose their face (was uncovered before the fix round)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(interface_nonansi_header name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K6 covergroup names lose their face (was uncovered before the fix round)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(covergroup_declaration name: (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K7 enum member names lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(enum_name_declaration (simple_identifier) @constant)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K8 bare type references in data_type position lose their face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(data_type (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K9 the interconnect over-match is put back (net name painted as a type)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(net_declaration (simple_identifier) @type (list_of_net_decl_assignments))",
        "new": "(net_declaration (simple_identifier) @type)",
        "test": "verilog_font_lock_philosophy",
        "note": (
            "Restores the exact defect the cold read found: without the "
            "`list_of_net_decl_assignments' sibling constraint, the "
            "`interconnect' form puts the net's own NAME in the captured "
            "position. The guard is `ic_net' staying uncoloured."
        ),
    },
    {
        "label": "K10 the 3-segment chain paints its middle segment again",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(data_type (class_type (simple_identifier) (simple_identifier) @type .))",
        "new": "(data_type (class_type (simple_identifier) (simple_identifier) @type))",
        "test": "verilog_font_lock_philosophy",
        "note": (
            "Without the last-child anchor, `a::b::c' captures both `b' and "
            "`c'. The guard is the assertion that `b' stays uncoloured."
        ),
    },
    {
        "label": "K11 a parameterised qualified type loses its face (the trailing-anchor regression)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(data_type (class_type (simple_identifier) (simple_identifier) @type . (parameter_value_assignment) .))",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
        "note": (
            "This rule exists because the first fix round's last-child "
            "anchor silently stopped matching `pkg::t #(8) x;' -- "
            "`class_type''s last named child is then the "
            "`parameter_value_assignment', not the identifier. Removing it "
            "puts that regression back."
        ),
    },
    {
        "label": "K12 a bare parameterised type reference loses its face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(data_type (class_type . (simple_identifier) @type . (parameter_value_assignment) .))",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
    {
        "label": "K13 a nettype used as a port type loses its face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(net_port_type (simple_identifier) @type)",
        "new": ";; mutated away",
        "test": "verilog_font_lock_philosophy",
    },
]
