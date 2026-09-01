# M68 -- the `*Help*` half (migrating M67's `help--source-buffer` to the
# shared `quit-source`).
#
# The only reason this is split from `dev/mutations/m68.py` into two files is
# that `mutate.py`'s `--test-target` applies to the whole config (precedent
# from the `m60`/`m66` series). This entry's observation point is in
# `help_tests`, the rest are in `dired_tests`.
#
# How to run:
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m68-help.py -p core \
#         --test-target help_tests
#
# ## An accounting note in passing: `dev/mutations/m67.py`'s M11 entry is
#    now permanently dead
#
# That entry reverts the self-reference guard M67 wrote for `*Help*`
# (`(if (equal (buffer-name) "*Help*") ...)`), and M68 replaced the whole
# block with `(quit-source-of-current)`. **m67.py's M11 will now no-match
# and get SKIPped.** This is a known cost of the migration, flagged by the
# architect at design time, not a missed update. Keeping one historical
# mutation entry alive isn't worth leaving an isomorphic one-off duplicate in
# the product code. This file's M8 is its successor -- it verifies the same
# claim (a second `C-h b` should not record `*Help*` itself as the source),
# just against the new shared mechanism.

PACKAGE = "core"
TEST_TARGET = "help_tests"

MUTATIONS = [
    {
        "label": "M8 quit-source-of-current no longer inherits, records the current buffer every time (*Help* self-reference)",
        "file": "crates/core/lisp/simple.el",
        "old": "  (or quit-source (current-buffer)))",
        "new": "  (current-buffer))",
        "expect_fail": [
            "describe_bindings_called_again_inside_help_still_returns_to_original_source"
        ],
        "note": (
            "Successor to m67.py's M11. Also \"swap in a version that looks "
            "plausible but is wrong\" rather than \"remove a guard\": pressing "
            "`C-h b` again inside `*Help*` overwrites the source with "
            "`*Help*` itself, getting stuck after `q`."
        ),
    },
]
