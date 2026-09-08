# Mutation list for M123 (the LSP client stops mangling what the server
# actually sends). Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m123.py -p core
#
# Per the rule M117 wrote into CLAUDE.md, the deletion entry is per FEATURE,
# not per file. M123 ships eight distinct effects and each has an entry below
# that removes the whole thing rather than perturbing a boundary inside it:
#
#   F1 server->client requests are answered  -> V1, V2
#   F2 the pending table is bounded          -> V3
#   F3 the client declares what it honours   -> V4
#   F4 resolve-capable servers get resolved  -> V5
#   F5 snippet escapes (\$, \\, \})          -> V6
#   F6 `${N|a,b|}' vs `${N:default}'         -> V7
#   F7 a completion can never vanish         -> V8, V9
#   F8 the popup item's payload field        -> V10, V11
#   F9 Verilog "instantiate" completion      -> V12
#
# V13-V17 came from the trailing cold read: boundaries INSIDE those features
# that the deletion entries above do not isolate (the `:' branch as well as
# the `|' one, the negative resolve-capability direction, one escape
# character at a time, the final stop's offset ordering) plus V17, the brace
# matcher -- which the trailing reviewer found by EXECUTING the expander
# rather than reading it: `\}' worked in plain text, which a test covered,
# and not inside a `${N:default}', which nothing covered.
#
# F4 and F9 are the two that decide whether this milestone reached a user at
# all, and both were nearly missed. F4's gate (`lsp--resolve-provider-p') had
# no test whatsoever until the cold read asked the deletion question: it could
# be hardcoded to nil -- disabling the entire resolve round trip this
# milestone was built around -- with a fully green gate. F9 exists because the
# local Verilog completion tier claims a module name typed at statement start,
# so the LSP snippet could never reach a Verilog user however well the LSP
# half worked (measured before any code was written; the same M54 trap).
#
# Nothing here is a declared survivor: every effect is reachable from
# `cargo test'. That is itself a result -- M123's whole surface is elisp plus
# two small Rust decode sites, with no painter call or screenshot-only hop.
#
# Style note (M83/M85, restated in CLAUDE.md): every entry is
# REPLACEMENT-style. A stub inserted ahead of a real definition does not
# shadow it -- elisp's `defun' and the Rust `defun()' registration both let
# the LAST definition win.

PACKAGE = "core"
TEST_TARGET = "completion_popup_tests"

MUTATIONS = [
    # ---- F1: a request is a request, not a response --------------------
    {
        "label": "V1 the server-request branch is dead again (deletion entry: F1)",
        "file": "crates/core/lisp/lsp.el",
        "old": "     ((and id method)\n",
        "new": "     ((and id method nil)\n",
        "test": "server_request_with_registered_method_is_answered_null_and_never_stashed",
        "test_target": "lsp_async_tests",
    },
    {
        "label": "V2 same branch, witnessed by the unknown-method error reply",
        "file": "crates/core/lisp/lsp.el",
        "old": "     ((and id method)\n",
        "new": "     ((and id method nil)\n",
        "test": "server_request_with_unknown_method_gets_method_not_found_error",
        "test_target": "lsp_async_tests",
    },
    # ---- F2: retention is bounded --------------------------------------
    {
        "label": "V3 the cap stops evicting anything (deletion entry: F2)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                    (nreverse (cdr (reverse (lsp--client-pending client))))",
        "new": "                    (lsp--client-pending client)",
        "test": "pending_list_is_capped_and_drops_the_oldest_entry",
        "test_target": "lsp_async_tests",
    },
    # ---- F3: the client says what it honours ---------------------------
    {
        "label": "V4 `snippetSupport' is no longer declared (deletion entry: F3)",
        "file": "crates/core/lisp/lsp.el",
        "old": '    (puthash "snippetSupport" t completion-item)',
        "new": '    (puthash "snippetSupport" nil completion-item)',
        "test": "client_capabilities_payload_declares_snippet_and_resolve_support",
    },
    # ---- F4: the resolve round trip actually happens -------------------
    {
        # The entry the cold read's deletion question produced. Before the
        # fix round, hardcoding this predicate to nil turned NO test red --
        # the whole mechanism could have been silently disabled.
        "label": "V5 no server is ever considered resolve-capable (deletion entry: F4)",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (and (lsp--client-p client)\n       (let ((caps (lsp--client-capabilities client)))\n         (and (hash-table-p caps)\n              (let ((cp (gethash \"completionProvider\" caps)))",
        "new": "  (and nil (lsp--client-p client)\n       (let ((caps (lsp--client-capabilities client)))\n         (and (hash-table-p caps)\n              (let ((cp (gethash \"completionProvider\" caps)))",
        "test": "resolve_provider_capability_drives_a_real_completion_reply_to_a_resolve_payload",
    },
    # ---- F5: the snippet escape set ------------------------------------
    {
        "label": "V6 only `\\$' is an escape again, not `\\\\' or `\\}' (deletion entry: F5)",
        "file": "crates/core/lisp/lsp.el",
        "old": "               (memq (aref snippet (1+ i)) '(?$ ?\\\\ ?})))",
        "new": "               (memq (aref snippet (1+ i)) '(?$)))",
        "test": "expand_snippet_escaped_backslash_is_a_literal_backslash",
    },
    # ---- F6: a choice list is not a default ----------------------------
    {
        # The bug this replaced was real and shipped: the shape used to be
        # decided by whichever of `:'/`|' came first anywhere in the string,
        # so a Verilog bit range inside a choice list (`${1|[7:0],[15:0]|}')
        # was read as a default -- and `string-to-number' on the garbage
        # prefix returned 0, which ALSO made it the final cursor stop.
        "label": "V7 the `|' shape is never recognised (deletion entry: F6)",
        "file": "crates/core/lisp/lsp.el",
        "old": "       ((and (< i len) (eq (aref inner i) ?|))",
        "new": "       ((and nil (eq (aref inner i) ?|))",
        "test": "expand_snippet_choice_list_containing_a_colon_is_not_misread_as_a_default",
    },
    # ---- F7: a completion never silently vanishes ----------------------
    {
        "label": "V8 the inline path's empty-expansion guard is gone (deletion entry: F7)",
        "file": "crates/core/lisp/lsp.el",
        "old": "       ((not (and (stringp text) (> (length text) 0)))",
        "new": "       ((and nil (not (and (stringp text) (> (length text) 0))))",
        "test": "insert_text_format_2_bare_placeholder_expanding_to_empty_falls_back_to_the_label",
    },
    {
        "label": "V9 the resolve path's empty-expansion guard is gone",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (if (not (and (stringp final-text) (> (length final-text) 0)))",
        "new": "    (if (and nil (not (and (stringp final-text) (> (length final-text) 0))))",
        "test": "resolve_reply_expanding_to_empty_falls_back_to_the_markers_fallback_text",
    },
    # ---- F8: the popup item's fifth field -------------------------------
    {
        "label": "V10 `show-completion-popup' rejects the fifth element again (deletion entry: F8)",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "            if fields.len() != 4 && fields.len() != 5 {",
        "new": "            if fields.len() != 4 {",
        "test": "resolve_payload_sends_resolve_and_inserts_the_resolved_text",
    },
    {
        "label": "V11 the expander's cursor offset is discarded on the way to the buffer",
        "file": "crates/core/src/commands.rs",
        "old": '        "offset" => (insert.to_string(), Some(n.max(0) as usize)),',
        "new": '        "offset" => (insert.to_string(), None),',
        "test": "insert_text_format_2_expands_the_snippet_and_places_point_at_the_dollar_zero",
    },
    # ---- F9: the Verilog user can actually reach it ---------------------
    {
        "label": "V12 module-name completion offers the bare name only, as before (deletion entry: F9)",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": "  (list (verilog-complete--module-item entry prefix-start)\n        (verilog-complete--instantiate-item (car entry) prefix-start indent)))",
        "new": "  (list (verilog-complete--module-item entry prefix-start)))",
        "test": "instantiate_item_for_a_real_demo_rtl_module_with_parameters_and_many_ports",
        "test_target": "verilog_complete_tests",
    },
    # ---- Entries the trailing cold read designed (boundaries inside
    # ---- features F5/F6/F4 that the deletion entries above do not isolate).
    {
        "label": "V13 the `:' default shape is never recognised (F6's other branch)",
        "file": "crates/core/lisp/lsp.el",
        "old": "       ((and (< i len) (eq (aref inner i) ?:))",
        "new": "       ((and nil (eq (aref inner i) ?:))",
        "test": "expand_snippet_default_text_and_no_dollar_zero_leaves_offset_nil",
    },
    {
        "label": "V14 every client is treated as resolve-capable (F4's negative direction)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                       (and rp (not (eq rp :false))))))))))",
        "new": "                       (and t (not (eq rp :false))))))))))",
        "test": "no_resolve_provider_capability_produces_a_plain_literal_item_not_a_resolve_payload",
    },
    {
        "label": "V15 only `\\}' is dropped from the escape set (F5, one character at a time)",
        "file": "crates/core/lisp/lsp.el",
        "old": "               (memq (aref snippet (1+ i)) '(?$ ?\\\\ ?})))",
        "new": "               (memq (aref snippet (1+ i)) '(?$ ?\\\\)))",
        "test": "expand_snippet_escaped_close_brace_is_a_literal_brace",
    },
    {
        "label": "V16 the final stop's offset is recorded after its default text, not before",
        "file": "crates/core/lisp/lsp.el",
        "old": "                (when (eq kind 'final)\n                  (setq offset (length out)))\n                (setq out (concat out text))",
        "new": "                (setq out (concat out text))\n                (when (eq kind 'final)\n                  (setq offset (length out)))",
        "test": "expand_snippet_dollar_zero_with_a_default_places_point_before_the_default_text",
    },
    {
        # The trailing cold read found this one by EXECUTING the expander
        # rather than reading it: `\}' worked in plain text (which the test
        # covered) and not inside a `${N:default}' (which nothing covered),
        # because the brace matcher counted raw braces. Both positions now
        # have a test; this entry is the one that bites on the fix.
        "label": "V17 the brace matcher counts escaped braces again (F5 inside a construct)",
        "file": "crates/core/lisp/lsp.el",
        "old": "         ((and (eq c ?\\\\) (< (1+ i) len))\n          (setq i (1+ i)))",
        "new": "         ((and nil (< (1+ i) len))\n          (setq i (1+ i)))",
        "test": "expand_snippet_escaped_close_brace_inside_a_default_is_a_literal_brace",
    },
]
