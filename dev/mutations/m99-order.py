# Two extra mutations designed by the TRAILING reviewer (the one that
# cold-read M99's fix round), aimed at the one thing the main list does not
# touch: whether the ORDER `lsp--diagnostics-for-uri' returns is observable
# by any test at all.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m99-order.py \
#         -p core --test-target lsp_highlight_tests
#
# The reviewer predicted these may SURVIVE, and said so before they were run --
# and on the first run both did: the merge tests asserted overlay COUNTS, and
# `next-diagnostic' sorts by position itself (`lsp--buffer-diagnostic-positions'
# ends in a `sort' by car), so publication order was not observable anywhere
# downstream. Both mutations had landed (the runner reports SKIP, not SURVIVED,
# when the original string isn't found exactly once) and both genuinely reverse
# the returned order, so that was a real coverage hole, not a dead `nreverse'.
#
# Order is user-visible: when several diagnostics share a line, it decides which
# of them survive M87 stage 3's per-message truncation. So two tests were added
# that read the returned ORDER directly rather than its length --
# `diagnostics_for_uri_nil_branch_preserves_publish_order' and
# `diagnostics_for_uri_merge_branch_orders_by_client_then_publish_order' -- and
# these two items now name them. Re-running this list should report both as
# FAIL (as expected).

PACKAGE = "core"
TEST_TARGET = "lsp_highlight_tests"

MUTATIONS = [
    {
        "label": "O1 the nil (M94-compatible) branch returns its diagnostics reversed",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (nreverse out))\n",
        "new": "        out)\n",
        "test": "diagnostics_for_uri_nil_branch_preserves_publish_order",
    },
    {
        "label": "O2 the merged branch returns the union reversed",
        "file": "crates/core/lisp/lsp.el",
        "old": "          (nreverse out))))))",
        "new": "          out)))))",
        "test": "diagnostics_for_uri_merge_branch_orders_by_client_then_publish_order",
    },
]
