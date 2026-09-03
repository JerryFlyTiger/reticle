# M94 (a second language server per buffer, routed by capability) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m94-multi-client.py \
#         -p core --test-target lsp_mode_tests
#
# The autostart half runs against a different target and lives in
# dev/mutations/m94-multi-client-autostart.py.
#
# The first cold read's checklist is deliberately NOT reused: the fix rounds
# rewrote `lsp--attach-current-buffer`, `lsp--decorate-buffer` (which gained a
# `client` parameter) and `lsp--autostart-maybe-begin` outright, so every
# line-anchored item from that round pointed at moved code. Regenerated here
# against the current source rather than patched.
#
# Two defences are recorded elsewhere as unobservable and are not listed: the
# `redisplay.rs` comment correction (comment-only, nothing to revert), and
# `lsp--diagnostics-authoritative-command`, which nothing ever sets -- it is a
# stub for a later milestone and its own docstring now says so.

PACKAGE = "core"
TEST_TARGET = "lsp_mode_tests"

MUTATIONS = [
    {
        "label": "AB1 a would-be secondary may take the primary slot again",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (set-primary (and is-primary-candidate (not had-primary))))",
        "new": "         (set-primary (not had-primary)))",
        "test": "primary_slot_self_heals_after_dying_while_a_secondary_stays_attached",
        "note": (
            "The defect this restores is the one the first cold read found by "
            "tracing alone: a dead primary lets a secondary be promoted, so "
            "formatting -- which slang has none of -- would start going to "
            "slang, and reconnecting verible afterwards would never reclaim "
            "the slot."
        ),
    },
    {
        "label": "AB2 the authority check stops testing liveness (a dead primary blocks the secondary forever)",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (and lsp--buffer-client (lsp--client-conn-live-p lsp--buffer-client)\n         lsp--buffer-client)))",
        "new": "    lsp--buffer-client))",
        "test": "a_dead_primary_reopens_decoration_to_a_live_secondary",
    },
    {
        "label": "AB3 capability routing stops preferring the primary (list order wins instead)",
        "file": "crates/core/lisp/lsp.el",
        "old": "   ((and lsp--buffer-client\n         (lsp--client-conn-live-p lsp--buffer-client)\n         (lsp--capability-supported-p lsp--buffer-client key))\n    lsp--buffer-client)",
        "new": "   ((and nil lsp--buffer-client)\n    lsp--buffer-client)",
        "test": "hover_prefers_a_capable_primary_over_a_capable_secondary_in_real_attach_order",
    },
    {
        "label": "AB4 dead clients are no longer pruned from the buffer's client list",
        "file": "crates/core/lisp/lsp.el",
        # First attempt hit `lsp--capable-client''s own liveness filter, a
        # different line from the prune block, and SURVIVED -- the note below
        # predicted exactly that. Re-anchored onto the prune itself.
        "old": "  (when lsp--buffer-clients\n    (dolist (client lsp--buffer-clients)\n      (unless (lsp--client-conn-live-p client)\n        (lsp--clear-client-synced-tick client)))",
        "new": "  (when nil\n    (dolist (client lsp--buffer-clients)\n      (unless (lsp--client-conn-live-p client)\n        (lsp--clear-client-synced-tick client)))",
        "test": "idle_pump_prunes_a_dead_client_out_of_buffer_clients_and_its_watermark",
        "note": (
            "Aimed at the liveness filter the prune path shares. If this "
            "SURVIVES it means the filter and the pruning are separate lines "
            "and the mutation is mis-anchored -- re-anchor onto the prune "
            "block itself rather than reading it as a coverage gap."
        ),
    },
]
