# Mutation list for M154 (the secondary LSP client gets code actions, related
# locations, and a seat when the primary is gone).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 python3 -u dev/mutate.py --config dev/mutations/m154.py
#
# Per M117's rule, a deletion entry per FEATURE. M154 ships:
#
#   A, code actions from every capable client (fan-out, merge, wait-for-all,
#      command execution)                          -> A1 (fan-out), A2 (wait),
#                                                     A3 (usable), A4/A4s (command)
#   B, diagnostics navigation from a lone secondary -> B1
#   C, related locations: inline row (C3), jump (C2), picker labels (F1),
#      show buffer keymap (F2), malformed-entry tolerance (F4) -> C3, C2, F1, F2, F4
#   C1, the *LSP Diagnostic* detail buffer        -> C1 (added after tail review 3)
#   D, build-file client found by capability       -> D1
#   E, the C-c l d / C-c l D bindings             -> E1, E1b
#
# Anchors were taken from the reviewer's round-1 checklist and re-read out of
# the staged lsp.el by the main conversation (exact whitespace). Entries for
# the fix-round items (F1, F2, F4) were appended after that round landed.

PACKAGE = "core"

_L = "crates/core/lisp/lsp.el"

MUTATIONS = [
    {
        "label": "A1 fan-out removed: only the primary is asked",
        "file": _L,
        "old": """    (dolist (client (lsp--effective-buffer-clients))
      (when (and (not (eq client primary))
                 (lsp--client-conn-live-p client)
                 (lsp--capability-supported-p client "codeActionProvider"))
        (push client out)))""",
        "new": """    (ignore (lsp--effective-buffer-clients))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_asks_every_client_that_declares_the_capability",
    },
    {
        "label": "A1b fan-out removed: lone secondary gets no code actions",
        "file": _L,
        "old": """    (dolist (client (lsp--effective-buffer-clients))
      (when (and (not (eq client primary))
                 (lsp--client-conn-live-p client)
                 (lsp--capability-supported-p client "codeActionProvider"))
        (push client out)))""",
        "new": """    (ignore (lsp--effective-buffer-clients))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_answers_from_a_lone_secondary_in_an_empty_primary_slot",
    },
    {
        "label": "A2 wait-for-all removed: picker opens on the first reply",
        "file": _L,
        "old": """             (when (= pending 0)
               (lsp--code-action-finish clients replies buf tick))""",
        "new": """             (when t
               (lsp--code-action-finish clients replies buf tick))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_opens_nothing_until_every_client_has_replied",
    },
    {
        "label": "A3 usable back to edit-only",
        "file": _L,
        "old": """                 (or (hash-table-p (gethash "edit" action))
                     (lsp--code-action-command action)))""",
        "new": """                 (hash-table-p (gethash "edit" action)))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_command_only_element_is_sent_as_execute_command_to_its_own_client",
    },
    {
        "label": "A4 command execution removed (fresh branch)",
        "file": _L,
        "old": """             (suffix (lsp--code-action-skip-suffix skipped other)))
        (when cmd (lsp--code-action-send-command client cmd))""",
        "new": """             (suffix (lsp--code-action-skip-suffix skipped other)))
        (when nil (lsp--code-action-send-command client cmd))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_with_edit_and_nested_command_applies_the_edit_then_sends_the_command",
    },
    {
        "label": "A4s command execution removed (stale-edit branch)",
        "file": _L,
        "old": """discarding stale edit")
          (when cmd (lsp--code-action-send-command client cmd)))""",
        "new": """discarding stale edit")
          (when nil (lsp--code-action-send-command client cmd)))""",
        "test_target": "lsp_action_tests",
        "test": "code_action_stale_edit_is_dropped_but_its_command_is_still_sent",
    },
    {
        "label": "B1 lone-secondary diagnostics gate reverted to primary-only",
        "file": _L,
        "old": """  (or (lsp--live-buffer-client)
      (let (found)
        (dolist (client (lsp--effective-buffer-clients) found)
          (when (and (not found) (lsp--client-conn-live-p client))
            (setq found client))))))""",
        "new": """  (lsp--live-buffer-client))""",
        "test_target": "lsp_mode_tests",
        "test": "next_diagnostic_reaches_a_lone_secondarys_diagnostic_in_an_empty_primary_slot",
    },
    {
        "label": "B1b same revert, diagnostics-at-point",
        "file": _L,
        "old": """  (or (lsp--live-buffer-client)
      (let (found)
        (dolist (client (lsp--effective-buffer-clients) found)
          (when (and (not found) (lsp--client-conn-live-p client))
            (setq found client))))))""",
        "new": """  (lsp--live-buffer-client))""",
        "test_target": "lsp_mode_tests",
        "test": "diagnostics_at_point_sees_a_lone_secondarys_diagnostics",
    },
    {
        "label": "C3 related lines never appended to the inline row",
        "file": _L,
        "old": """                              (if related
                                  (concat base "\\n" (mapconcat""",
        "new": """                              (if nil
                                  (concat base "\\n" (mapconcat""",
        "test_target": "lsp_highlight_tests",
        "test": "inline_row_message_carries_one_line_per_related_location",
    },
    {
        "label": "C2 related jump no longer pushes the return marker",
        "file": _L,
        "old": """    (lsp-push-definition-marker origin)
    (find-file (lsp--uri-to-path uri))
    (goto-char (lsp--pos-at-utf16 line character))))""",
        "new": """    (ignore origin)
    (find-file (lsp--uri-to-path uri))
    (goto-char (lsp--pos-at-utf16 line character))))""",
        "test_target": "lsp_mode_tests",
        "test": "goto_related_location_jumps_to_the_related_file_line_and_column_and_pushes_a_marker",
    },
    {
        "label": "D1 build-file client back to the `slang' name substring",
        "file": _L,
        "old": """`lsp-verilog-show-include-directories'."
  (lsp--verilog-server-declares-set-build-file-p client))""",
        "new": """`lsp-verilog-show-include-directories'."
  (and (lsp--client-p client) (let ((cmd (lsp--client-command client))) (and cmd (string-match-p "slang" (file-name-nondirectory cmd)) t))))""",
        "test_target": "lsp_verilog_include_tests",
        "test": "build_file_client_is_decided_by_the_set_build_file_capability_not_the_command_name",
    },
    {
        "label": "F1 related picker loses (N) disambiguation (naive alist)",
        "file": _L,
        "old": """      (let ((alist (lsp--related-location-alist client entries)))""",
        "new": """      (let ((alist (mapcar (lambda (e) (cons "same" e)) entries)))""",
        "test_target": "lsp_mode_tests",
        "test": "goto_related_location_with_identical_labels_jumps_to_the_one_chosen",
    },
    {
        "label": "F2 diagnostic buffer gets no help keymap",
        "file": _L,
        "old": """        (major-mode-internal-set 'help-mode)
        (help--install-quit-map)""",
        "new": """        (major-mode-internal-set 'help-mode)
        (ignore)""",
        "test_target": "lsp_mode_tests",
        "test": "show_diagnostic_at_point_buffer_quits_with_q",
    },
    {
        "label": "F4 malformed-entry guard removed (location not type-checked)",
        "file": _L,
        "old": """         (and (hash-table-p loc)
""",
        "new": """         (and t
""",
        "test_target": "lsp_highlight_tests",
        "test": "inline_row_survives_a_malformed_related_information_entry",
    },
    {
        "label": "C1 diagnostic detail buffer left empty",
        "file": _L,
        "old": """          (erase-buffer)
          (insert text)
          (goto-char (point-min)))""",
        "new": """          (erase-buffer)
          (insert "")
          (goto-char (point-min)))""",
        "test_target": "lsp_mode_tests",
        "test": "show_diagnostic_at_point_lists_severity_source_code_and_related_locations",
    },
    {
        "label": "E1 C-c l d unbound",
        "file": "crates/core/lisp/simple.el",
        "old": """(global-set-key "C-c l d" 'lsp-show-diagnostic-at-point)""",
        "new": """(ignore "C-c l d")""",
        "test_target": "lsp_action_tests",
        "test": "c_c_l_d_and_shift_d_reach_the_m154_diagnostic_detail_commands",
    },
    {
        "label": "E1b C-c l D unbound",
        "file": "crates/core/lisp/simple.el",
        "old": """(global-set-key "C-c l D" 'lsp-goto-related-location)""",
        "new": """(ignore "C-c l D")""",
        "test_target": "lsp_action_tests",
        "test": "c_c_l_d_and_shift_d_reach_the_m154_diagnostic_detail_commands",
    },
]
