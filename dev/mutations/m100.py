# Mutation list for M100 (SystemVerilog enum members and packed-struct fields
# finally reindent). Designed by the reviewer that cold-read the diff; run from
# the main conversation, because an implementer must not verify its own fix.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m100.py \
#         -p core --test-target indent_tests
#
# The fix is two strings added to one list, so the interesting question is not
# "does removing them break something" (M1) but "do the two new tests actually
# guard their own node type, or is one of them riding on the other" (M2/M3),
# and "is the DO-NOT-USE-data_type warning in the comment backed by a test, or
# is it just prose" (M4).
#
# M4 is the one worth keeping: `data_type' is the obvious-looking fix, it is
# what a future maintainer will reach for, and the comment warning against it
# is only as good as the test that catches it.

PACKAGE = "core"
TEST_TARGET = "indent_tests"

MUTATIONS = [
    {
        "label": "M1 both new node types removed (back to the M97 list)",
        "file": "crates/core/lisp/indent.el",
        "old": "                \"enum_name_declaration\" \"struct_union_member\")))",
        "new": "                )))",
        "test": "verilog_soc_pkg_sv_full_file_reindents_to_its_own_on_disk_columns_except_the_documented_package_header_quirk",
    },
    {
        "label": "M2 only enum_name_declaration removed (struct half left intact)",
        "file": "crates/core/lisp/indent.el",
        "old": "                \"enum_name_declaration\" \"struct_union_member\")))",
        "new": "                \"struct_union_member\")))",
        "test": "verilog_package_enum_member_line_indents_to_four_at_two_column_width",
    },
    {
        "label": "M3 only struct_union_member removed (enum half left intact)",
        "file": "crates/core/lisp/indent.el",
        "old": "                \"enum_name_declaration\" \"struct_union_member\")))",
        "new": "                \"enum_name_declaration\")))",
        "test": "verilog_package_struct_field_line_indents_to_four_at_two_column_width",
    },
    {
        "label": "M4 data_type added -- the trap the comment warns against",
        "file": "crates/core/lisp/indent.el",
        "old": "                \"enum_name_declaration\" \"struct_union_member\")))",
        "new": "                \"enum_name_declaration\" \"struct_union_member\" \"data_type\")))",
        "test": "verilog_ordinary_declarations_unaffected_by_the_new_enum_struct_node_types",
        "note": (
            "Every ordinary port and signal declaration's leading token is a "
            "descendant of `data_type', so this makes them all indent one "
            "level too deep. Without this item the warning in the comment "
            "would be prose with nothing enforcing it."
        ),
    },
]
