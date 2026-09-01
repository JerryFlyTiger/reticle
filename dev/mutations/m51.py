# M51 mutation list (documentHighlight auto-trigger on cursor dwell).
#
# Kept as an example and regression list: after changing the highlight block in
# lsp.el or idle_tick in lib.rs, run `dev/mutate.py --config dev/mutations/m51.py`
# to confirm these 12 guards are still being watched by tests.
# If an entry turns into SKIP, the code has drifted from the original string in
# the list; update it or delete the entry.

PACKAGE = "core"
TEST_TARGET = "lsp_highlight_tests"

EL = "crates/core/lisp/lsp.el"
RS = "crates/core/src/lib.rs"

MUTATIONS = [
    {
        "label": "M1 serial guard (blocks out-of-order replies)",
        "file": EL,
        "old": """(when (and (eq (current-buffer) buf)
                      (eq serial lsp--highlight-request-serial))""",
        "new": """(when (eq (current-buffer) buf)""",
        "test": "stale_out_of_order",
    },
    {
        "label": "M2 clear records last-point (whole block removed)",
        "file": EL,
        "old": """  (lsp--clear-highlights)
  (when (lsp--live-buffer-client)
    (setq-local lsp--idle-highlight-last-point (point))))""",
        "new": """  (lsp--clear-highlights))""",
        "test": "idle_tick_after_highlight_clear",
    },
    {
        "label": "M3 unless quiet: No highlights here",
        "file": EL,
        "old": """(unless quiet (message "No highlights here"))""",
        "new": """(message "No highlights here")""",
        "test": "empty_reply",
    },
    {
        "label": "M4 unless quiet: no-client guard",
        "file": EL,
        "old": """      (unless quiet
        (message "No LSP server connected in this buffer (M-x lsp first)")))""",
        "new": """      (message "No LSP server connected in this buffer (M-x lsp first)"))""",
        "test": "quiet_suppresses_messages_when_no_client",
    },
    {
        "label": "M5 last-point dedup",
        "file": EL,
        "old": """                   (not (eq (point) lsp--idle-highlight-last-point)))""",
        "new": """                   t)""",
        "test": "same_point",
    },
    {
        "label": "M6 threshold >= changed to > (boundary)",
        "file": EL,
        "old": """             (>= quiet-ms lsp-idle-highlight-delay-ms))""",
        "new": """             (> quiet-ms lsp-idle-highlight-delay-ms))""",
        "test": "at_threshold",
    },
    {
        "label": "M7 nil-delay switch",
        "file": EL,
        "old": """  (when (and lsp-idle-highlight-delay-ms
             (>= quiet-ms lsp-idle-highlight-delay-ms))""",
        "new": """  (when (>= quiet-ms lsp-idle-highlight-delay-ms)""",
        "test": "disabled_by_nil",
    },
    {
        "label": "M8 setq-local changed to setq (buffer-local isolation)",
        "file": EL,
        "old": """      (setq-local lsp--idle-highlight-last-point (point))""",
        "new": """      (setq lsp--idle-highlight-last-point (point))""",
        "test": "buffer_local",
    },
    {
        "label": "M9 whole eval line of the Rust->elisp bridge",
        "file": RS,
        "old": """    let _ = interp.eval_source(&format!("(lsp--idle-highlight-tick {quiet_ms})"));""",
        "new": """    let _ = quiet_ms;""",
        "test": "rust_idle_tick",
    },
    {
        "label": "N1 unless quiet: non-file-buffer guard",
        "file": EL,
        "old": """      (unless quiet
        (message "Buffer is not visiting a file")))""",
        "new": """      (message "Buffer is not visiting a file"))""",
        "test": "no_file_name",
    },
    {
        "label": "N2 live-client gate on clear (dead-zone fix)",
        "file": EL,
        "old": """  (when (lsp--live-buffer-client)
    (setq-local lsp--idle-highlight-last-point (point))))""",
        "new": """  (setq-local lsp--idle-highlight-last-point (point)))""",
        "test": "dead_zone",
    },
    {
        "label": "N3 clamp threshold value (u32::MAX)",
        "file": RS,
        "old": """.min(u32::MAX as u128)""",
        "new": """.min(3_600_000)""",
        "test": "above_the_old_wrong_clamp",
    },
]
