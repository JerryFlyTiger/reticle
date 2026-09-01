# M54 mutation list (Verilog port-name local completion + LSP capabilities gate).
#
# List designed by the reviewer, executed by the main conversation. Kept as a
# regression list: after changing the context detection / cache / message
# branches of `verilog-complete.el`, or the three-tier dispatch and capability
# gate in `lsp.el`, run `dev/mutate.py --config dev/mutations/m54.py` to
# confirm these guards are still being watched by tests. If an entry turns
# into SKIP, the code has drifted from the original string; update or delete
# the entry.
#
# The bug itself: `verible-verilog-ls` does not support
# `textDocument/completion` at all (the `initialize` response has no
# `completionProvider` key, and an actual request gets an `Unhandled method`
# on its stderr). So Verilog port-name completion can never come from LSP,
# and dabbrev only scans the current buffer, so it can't reach module port
# names defined in other files -- at `fifo u_fifo ( .wr| );`, before the fix,
# both paths returned nil.
#
# -- Three spots that are easy to misjudge, written down here so they don't
#    get re-derived next time --------------------------------------------
#
# * **M10 was SURVIVED before the fix-up round; that's not a mistake in the
#   list, it's a genuine coverage gap.** After the first implementation round
#   handed off on 2026-08-10, the main conversation tested it directly:
#   replacing the two lines in `lsp-connect` that store capabilities with a
#   plain `nil` still passed all 14 tests at the time. The reason: every
#   capabilities test builds the struct manually with
#   `(make-lsp--client :capabilities ...)`, bypassing `lsp-connect`, and the
#   only e2e test that actually talks to verible exercises a
#   **port-connection position** -- that kind of position is always caught
#   first by tier-1's `verilog-complete-at-point`, so the capability gate is
#   never reached. In other words, the exact scenario this milestone was
#   meant to fix wasn't itself under test. The fix-up round added
#   `manual_e2e_verible_capability_gate_falls_to_dabbrev_at_a_non_port_position`,
#   after which this entry turns FAIL as expected. **This e2e test requires
#   `verible-verilog-ls` on PATH**; on a machine without it, the test skips
#   and M10 falls back to SURVIVED -- that's an environment limitation, not
#   a regression.
#
# * **Two guards are black-box unobservable, deliberately left out of the
#   list.** Don't hard-code entries just to pad the count -- that would just
#   become permanent SURVIVED noise:
#   1. The guard in `verilog-complete-at-point` against a nil `instance_type`
#      field. The fix-up round tried about 12 kinds of malformed Verilog and
#      could not construct an input with a `module_instantiation` node whose
#      `instance_type` is nil: malformed input either still fills in
#      `instance_type`, or never produces a `module_instantiation` node at
#      all (short-circuiting earlier). The guard was added anyway (the cost
#      is negligible, and the docstring makes a never-signals promise), but
#      no test can reach it.
#   2. The FILTER field of `verilog-complete--port-item`. Prefix filtering
#      happens via `verilog-complete-at-point`'s own `string-prefix-p`, not
#      via the popup item's FILTER; FILTER is only used by the Rust side's
#      `refilter_completion_popup` after the popup is already open and the
#      user keeps typing, and no existing test simulates that scenario.
#
# * **M3 / M4 each cover a different tree-sitter shape, both need to stay.**
#   Whether `.`'s parent is `named_port_connection` (another port connection
#   already exists, syntax complete) or `ERROR` (only one connection so far,
#   tree-sitter goes into error recovery and `.` lands inside an ERROR node
#   under `hierarchical_instance`) is two different paths. Removing either
#   one does not turn the other's test red.

PACKAGE = "core"
TEST_TARGET = "verilog_complete_tests"

VC = "crates/core/lisp/verilog-complete.el"
LSP = "crates/core/lisp/lsp.el"

MUTATIONS = [
    # -- LSP capability gate ------------------------------------------
    {
        "label": "M1 capability gate always true (sends even when the key is absent)",
        "file": LSP,
        "old": """      (not (eq (gethash key caps 'lsp--capability-absent) 'lsp--capability-absent)))))""",
        "new": """      t)))""",
        "test": "capability_key_absent",
    },
    {
        "label": "M2 unknown capabilities treated as unsupported",
        "file": LSP,
        "old": """    (if (not (hash-table-p caps))
        t""",
        "new": """    (if (not (hash-table-p caps))
        nil""",
        "test": "capabilities_nil_still_sends",
    },
    {
        "label": "M10 lsp-connect does not store the initialize response's capabilities",
        "file": LSP,
        "old": """      (setf (lsp--client-capabilities client)
            (and (hash-table-p result) (gethash "capabilities" result))))""",
        "new": """      nil)""",
        "test": "manual_e2e_verible_capability_gate",
    },
    {
        "label": "M8 three-tier dispatch order: LSP placed before local-completion-function",
        "file": LSP,
        "old": """  (cond
   ((and local-completion-function (funcall local-completion-function)))
   ((let ((client (lsp--live-buffer-client)))
      (and client (lsp--capability-supported-p client "completionProvider")))
    (lsp-completion-at-point))""",
        "new": """  (cond
   ((let ((client (lsp--live-buffer-client)))
      (and client (lsp--capability-supported-p client "completionProvider")))
    (lsp-completion-at-point))
   ((and local-completion-function (funcall local-completion-function)))""",
        "test": "local_completion_function_tier_wins",
    },
    # -- port context detection (one entry per tree-sitter path) -----
    {
        "label": "M3 remove the named_port_connection branch",
        "file": VC,
        "old": """               ((and parent (string= (treesit-node-type parent) "named_port_connection"))
                (verilog-auto--enclosing-of-type parent "module_instantiation"))""",
        "new": """               ((and parent (string= (treesit-node-type parent) "no-such-node-type"))
                (verilog-auto--enclosing-of-type parent "module_instantiation"))""",
        "test": "same_buffer_module_offers_its_own_port_names",
    },
    {
        "label": "M4 remove the ERROR-recovery branch (shape of a single port connection)",
        "file": VC,
        "old": """               ((and parent (string= (treesit-node-type parent) "ERROR"))
                (let ((grandparent (treesit-node-parent parent)))
                  (when (and grandparent
                             (string= (treesit-node-type grandparent) "hierarchical_instance"))
                    (verilog-auto--enclosing-of-type grandparent "module_instantiation"))))""",
        "new": """               ((and parent (string= (treesit-node-type parent) "ERROR"))
                nil)""",
        "test": "library_file_module_offers_its_own_port_names",
    },
    # -- cache -------------------------------------------------------
    {
        "label": "M6 cache content-equality comparison always false (reparses every time)",
        "file": VC,
        "old": """        (if (and entry (string= (car entry) text))""",
        "new": """        (if (and entry nil)""",
        "test": "same_library_file_queried_twice_does_not_reparse",
    },
    # -- message branch added in the fix-up round ------------------
    {
        "label": "M11 module-not-found misreported as \"module exists but has no ports\"",
        "file": VC,
        "old": """             ((verilog-complete--module-found-p type-name)
              (message "Verilog port completion: module `%s' has no ports" type-name))""",
        "new": """             ((or t (verilog-complete--module-found-p type-name))
              (message "Verilog port completion: module `%s' has no ports" type-name))""",
        "test": "module_not_found",
    },
    {
        "label": "M12 prefix filtering to empty falls into the \"no ports\" branch",
        "file": VC,
        "old": """             (ports
              ;; PORTS non-nil here already proves the module resolved""",
        "new": """             (nil
              ;; PORTS non-nil here already proves the module resolved""",
        "test": "no_matching_prefix",
    },
    # -- two entries added in the trailing re-review (one per disjunct of module-found-p) --
    {
        "label": "M-A module-found-p's library branch never finds anything",
        "file": VC,
        "old": """            (when (and alist (assoc name alist))
              (setq found t)))""",
        "new": """            (when (and alist (assoc name alist))
              nil))""",
        "test": "empty_port_list_in_a_library_file",
    },
    {
        "label": "M-B module-found-p's current-buffer branch disabled",
        "file": VC,
        "old": """  (or (verilog-auto--find-module-in-buffer name)""",
        "new": """  (or nil""",
        "test": "empty_port_list_returns_t_without_opening_a_popup",
    },
]
