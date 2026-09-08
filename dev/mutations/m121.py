# Mutation list for M121 (the SystemVerilog constructs this editor rendered
# blind). Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m121.py -p core
#
# Per M117's rule, the deletion entry is per FEATURE. M121 ships two tables:
#
#   A, keyword + name faces -> K1..K7 (one per construct group)
#   B, breadcrumb scope kinds -> S1..S5 (one per node kind)
#
# plus L1..L5, which guard the SVA LABEL logic rather than the table entries.
# L1 is the entry that matters most: the first version of this milestone
# assumed `concurrent_assertion_item` wraps exactly three statement kinds and
# labelled the other two `"assert"` -- a WRONG breadcrumb where there had
# previously been none, which is worse than the gap it replaced. The cold read
# found it by re-dumping instead of trusting the count.
#
# L3 is deliberately expected to SURVIVE and is here as a recorded fact, not an
# oversight: it perturbs the `else` fallback that the five explicit arms make
# unreachable. Executing it is how "unreachable" stops being an assumption.

PACKAGE = "core"
TEST_TARGET = "highlight_tests"

MUTATIONS = [
    # ---- A: faces ----------------------------------------------------
    {
        "label": "K1 SVA keywords lose their face (deletion entry: SVA highlighting)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"assert" @keyword\n',
        "new": "",
        "test": "verilog_sva_keywords_get_keyword_face",
    },
    {
        "label": "K2 `iff' loses its face (the fix-round addition)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"iff" @keyword\n',
        "new": "",
        "test": "verilog_sva_iff_gets_keyword_face",
    },
    {
        "label": "K3 `interface' loses its face (deletion entry: interface/modport)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"interface" @keyword\n',
        "new": "",
        "test": "verilog_interface_modport_keywords_and_modport_name",
    },
    {
        "label": "K4 the modport's own declared name loses its face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(modport_item . (simple_identifier) @constant)",
        "new": "",
        "test": "verilog_interface_modport_keywords_and_modport_name",
    },
    {
        "label": "K5 `class' loses its face (deletion entry: class/new)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"class" @keyword\n',
        "new": "",
        "test": "verilog_class_keywords_and_constructor_new",
    },
    {
        "label": "K6 the coverpoint label loses its face",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": "(cover_point name: (simple_identifier) @variable)",
        "new": "",
        "test": "verilog_covergroup_keywords_and_coverpoint_label",
    },
    {
        "label": "K7 `program' loses its face (deletion entry: program)",
        "file": "crates/core/queries/verilog-highlights.scm",
        "old": '"program" @keyword\n',
        "new": "",
        "test": "verilog_program_keywords_get_keyword_face",
    },
    # ---- B: scope kinds ----------------------------------------------
    {
        "label": "S1 a `program' block stops producing any scope (deletion entry)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "program_declaration"\n',
        "new": "",
        "test": "verilog_program_declaration",
        "test_target": None,
    },
    {
        "label": "S2 a covergroup stops producing a scope (deletion entry)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "covergroup_declaration"\n',
        "new": "",
        "test": "verilog_covergroup_declaration_named",
        "test_target": None,
    },
    {
        "label": "S3 a modport stops producing a scope (deletion entry)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "modport_declaration"\n',
        "new": "",
        "test": "verilog_modport_declaration",
        "test_target": None,
    },
    {
        "label": "S4 a class constructor stops producing a scope (deletion entry)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "class_constructor_declaration"\n',
        "new": "",
        "test": "verilog_class_constructor_declaration",
        "test_target": None,
    },
    {
        "label": "S5 SVA stops producing a scope (deletion entry)",
        "file": "crates/core/src/scope.rs",
        "old": '                | "concurrent_assertion_item"\n',
        "new": "",
        "test": "verilog_sva_assert_property_unlabeled",
        "test_target": None,
    },
    # ---- SVA label logic ---------------------------------------------
    {
        # The exact defect the cold read caught: `cover sequence` handled by
        # the same branch as `assert property` would relabel it wrongly.
        "label": "L1 `cover sequence' is mislabelled as an assertion (the caught defect)",
        "file": "crates/core/src/scope.rs",
        "old": '            } else if find_child_kind(node, "cover_sequence_statement").is_some() {\n                "cover sequence".to_string()',
        "new": '            } else if find_child_kind(node, "cover_sequence_statement").is_some() {\n                "assert property".to_string()',
        "test": "verilog_sva_cover_sequence",
        "test_target": None,
    },
    {
        # Swapping the FIRST branch's target means nothing matches
        # `assert_property_statement` any more, so an `assert property` input
        # falls all the way to the `else` and comes out labelled with the raw
        # grammar string `"assert_property_statement"`. (The first version of
        # this comment said it comes out as `"cover property"` -- that is what
        # happens to a `cover property` INPUT under this mutation, not to an
        # assert one. The entry and its named test were right; the reasoning
        # written beside them was backwards, which is worth as much correcting
        # as the code would be.)
        "label": "L2 the first SVA branch targets the wrong child kind",
        "file": "crates/core/src/scope.rs",
        "old": '            let kind_word = if find_child_kind(node, "assert_property_statement").is_some() {',
        "new": '            let kind_word = if find_child_kind(node, "cover_property_statement").is_some() {',
        "test": "verilog_sva_assert_property_unlabeled",
        "test_target": None,
    },
    {
        # Expected SURVIVED, recorded rather than omitted: the five explicit
        # arms above are `node-types.json`'s complete child list for
        # `concurrent_assertion_item`, so nothing reaches the fallback.
        #
        # **The first version of this entry was worthless and reported
        # SURVIVED for the wrong reason** -- it targeted the `restrict
        # property` branch's OWN return value (a demonstrably reachable
        # branch), and its "mutation" appended a Rust block comment to an
        # expression, which changes no semantics at all. So it was guaranteed
        # to survive whatever the truth was. That is precisely the two-part
        # check this project requires on every SURVIVED (did it land? did it
        # change meaning?), failed by the person who wrote the rule into the
        # spec. This version targets the actual `else` line and really does
        # change what it returns.
        "label": "L3 the honest fallback is perturbed (expected SURVIVED: unreachable)",
        "file": "crates/core/src/scope.rs",
        "old": "                node.kind().to_string()\n            };",
        "new": '                "UNREACHABLE_FALLBACK_PROBE".to_string()\n            };',
        "test": "verilog_sva_restrict_property",
        "test_target": None,
        "expect": "survived",
    },
    {
        # L3's predecessor left this test with no entry that could ever turn
        # it red. This one changes the label's actual content.
        "label": "L4 `restrict property' is relabelled (the coverage L3 used to fake)",
        "file": "crates/core/src/scope.rs",
        "old": '                "restrict property".to_string()',
        "new": '                "restrict property MUTATED".to_string()',
        "test": "verilog_sva_restrict_property",
        "test_target": None,
    },
    {
        # Trailing round: an SVA label may be an escaped identifier, and the
        # first version looked only for `simple_identifier`.
        "label": "L5 an escaped-identifier SVA label is dropped again",
        "file": "crates/core/src/scope.rs",
        "old": '                .or_else(|| find_child_kind(node, "escaped_identifier"))\n',
        "new": "",
        "test": "verilog_sva_escaped_identifier_label_is_kept",
        "test_target": None,
    },
]
