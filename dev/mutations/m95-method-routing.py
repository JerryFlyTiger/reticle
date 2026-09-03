# M95 (routing the five shared capabilities to the server that answers them
# correctly) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m95-method-routing.py \
#         -p core --test-target lsp_mode_tests
#
# CD2 is the one this milestone's history turns on. The first version routed
# these five through `lsp--capable-client', which gates on the capability --
# and that quietly capability-gated a path that was UNGATED before M95. A
# single-server buffer whose `initialize' reply omits, say, `renameProvider'
# would have lost the command and been told "No LSP server connected", which
# is false: a server is connected, it just did not declare that key. Every
# single-client test omitted `:capabilities' entirely, which under this
# codebase's asymmetric-trust rule means "trusted", so none of them could see
# it and the full gate stayed green. CD2 puts the regression back.

PACKAGE = "core"
TEST_TARGET = "lsp_mode_tests"

MUTATIONS = [
    {
        "label": "CD1 the secondary preference is gone (routing falls back to the primary)",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (or (and (eq (cdr (assoc method lsp-request-preferred-role-alist)) 'secondary)",
        "new": "    (or (and nil",
        "test": "four_of_the_five_route_to_a_capable_secondary_over_the_primary",
    },
    {
        "label": "CD2 the fallback is capability-gated again (the regression the cold read caught)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                   (setq found client)))))\n        primary)))",
        "new": "                   (setq found client)))))\n        (lsp--capable-client key))))",
        "test": "single_client_missing_one_capability_key_still_routes_there_ungated",
    },
    {
        "label": "CD3 a dead secondary is treated as usable",
        "file": "crates/core/lisp/lsp.el",
        "old": "                            (lsp--client-conn-live-p client)\n                            (lsp--capability-supported-p client key))",
        "new": "                            t\n                            (lsp--capability-supported-p client key))",
        "test": "a_dead_secondary_falls_back_to_the_primary",
    },
    {
        "label": "CD4 the primary is no longer excluded from the secondary scan",
        "file": "crates/core/lisp/lsp.el",
        "old": "                            (not (eq client primary))",
        "new": "                            t",
        "test": "the_non_primary_exclusion_is_what_picks_the_secondary_when_both_are_capable",
        "note": (
            "The first cold read predicted this would SURVIVE, because in the "
            "two-client fixture the primary was skipped for an unrelated "
            "reason anyway. The fix round added a test that puts a CAPABLE "
            "primary first in the list precisely so the exclusion is what "
            "decides -- deliberately the reverse of the real attach order, "
            "where the secondary is consed on first and position alone would "
            "hide the effect."
        ),
    },
    {
        "label": "CD5 formatting is dragged into the routing table",
        "file": "crates/core/lisp/lsp.el",
        "old": '  \'(("textDocument/definition" . secondary)',
        "new": '  \'(("textDocument/formatting" . secondary)\n    ("textDocument/definition" . secondary)',
        "test": "formatting_still_goes_to_the_primary_even_though_it_is_not_in_the_preference_table",
        "note": (
            "slang-server declares no formatting capability at all, so a "
            "routing table that widened to cover formatting would aim it at "
            "a server that cannot answer. This is the guard against that."
        ),
    },
]
