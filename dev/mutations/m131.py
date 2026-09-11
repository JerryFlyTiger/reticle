# M131 (`M-?' project-root visibility + outside-root sweep) mutation list.
#
# One invocation runs all eight entries: D7's own `"test_target"' key
# (below) overrides TEST_TARGET for that one entry -- see `dev/mutate.py'
# itself (its docstring, `:144-157') for the precedence rule ("entry key >
# command line > config default") this relies on, and why it exists ("one
# fix's guard lives in a different test binary from the rest of the
# list", verbatim D7's own situation: it guards the autostart path, whose
# tests live in `lsp_autostart_tests', not `lsp_references_tests' like
# every other entry here).
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m131.py \
#         -p core --test-target lsp_references_tests
#
# M131 ships six distinct, independently breakable effects on top of the
# M131 Part A struct field (`lsp--client-root'). Per this project's own
# per-FEATURE rule, each gets its own deletion-style entry naming a
# real defect this milestone actually fixed or added, not a boundary
# perturbation inside it:
#
#   D1  the success message's/picker-prompt's own root clause
#   D2  reading the client's STORED root, vs. recomputing it fresh
#   D3  the outside-root sweep entirely (the M131 Part C headline effect)
#   D4  the Verilog-only glob restriction on that sweep
#   D5  the empty-result path's own root clause (the dead-code fix)
#   D6  `lsp-rename''s skipped-edit message naming the root
#   D7  the autostart path's own `:root root' -- the connection path a
#       real GUI session actually uses, never `lsp-connect' (`M-x lsp')
#   D8  the `lsp--verilog-buffer-p' gate on the sweep (2nd fix round)
#
# D2 is the one guarding the actual defect M131 exists to fix (a reused
# `lsp--connections' entry can be rooted differently than a fresh
# `lsp--project-root' call would compute) -- see `lsp--client-root's own
# docstring in `lsp.el' for why recomputing is wrong, not merely different.
#
# Fix round 1 (cold review): D7 was the one gap in the first cut -- the
# autostart creation point (`lsp.el:3979') had its own `:root root' with
# no test anywhere watching it (`lsp_autostart_tests.rs' had zero
# `lsp--client-root' hits at all), even though it is the connection path
# a real GUI session actually uses. `lsp-connect' (`M-x lsp') already had
# `lsp_connect_records_its_own_root_on_the_client' in `lsp_mode_tests.rs'
# from the same round; D7 is the autostart-side counterpart.
#
# Fix round 2 (cold review): D8 closes a gap the same review found in
# `lsp--symbol-at-point''s own reuse of `verilog-complete--ident-char-p'
# (added in fix round 1 for the `$'-in-`$clog2' case) -- that reuse
# assumes a SystemVerilog-only character class from a call site
# (`lsp-references-at-point') reachable from ANY buffer with an attached
# LSP client, exactly the shape `expand-region.el:98-106''s own
# `expand-region--ident-char-p' docstring says NOT to do ("this file has
# no language of its own to assume"). Fixed not by reverting to a
# hand-rolled character class, but by gating the sweep itself on
# `lsp--verilog-buffer-p' -- see `lsp--references-summary''s own
# docstring for why that gate is what makes the reuse correct rather
# than merely convenient.
#
# F5 (cold review, declared survivor, NOT a gap to close): `references_
# outside_root_sweep_never_adds_entries_to_the_result_list' has no
# matching mutation entry here on purpose. `outside' (the sweep's advisory
# list) and `(mapcar #'car alist)' (the actual candidate collection) are
# two data flows that structurally never touch -- there is no single
# guard line whose removal would merge them, so there is no natural
# one-line mutation that makes this test meaningfully go red. Recorded
# here, per the M115 precedent for a declared-survivor entry, rather than
# silently omitted (which would be indistinguishable from nobody having
# asked the question) or faked with an inert mutation. The test itself
# stays: it is still a real regression guard, just not one this file
# claims mutation coverage for.

PACKAGE = "core"
TEST_TARGET = "lsp_references_tests"

MUTATIONS = [
    {
        "label": "D1 success message's root clause removed",
        "file": "crates/core/lisp/lsp.el",
        "old": '(format "%d reference(s) in %d file(s) (project root: %s)"\n                        n nfiles root))',
        "new": '(format "%d reference(s) in %d file(s)" n nfiles))',
        "test": "references_success_message_names_the_root_actually_sent",
    },
    {
        # Replacement-style, not deletion: swapping the client's OWN
        # stored root for a fresh recomputation from FILE is exactly the
        # regression M131 Part A exists to prevent -- a pure deletion of
        # `lsp--client-root-or-computed' would break nearly every other
        # test in the file too and not isolate this specific defect.
        "label": "D2 root label recomputed fresh instead of read from the client",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (or (lsp--client-root client) (lsp--project-root file)))",
        "new": "  (lsp--project-root file))",
        "test": "references_root_label_uses_the_clients_stored_root_not_a_recomputation",
    },
    {
        "label": "D3 outside-root sweep removed entirely",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (outside (and symbol (lsp--verilog-buffer-p file)\n                       (lsp--references-outside-root-files symbol root)))",
        "new": "         (outside nil)",
        "test": "references_reports_verilog_files_outside_the_root_that_contain_the_name",
    },
    {
        "label": "D4 Verilog-only glob restriction removed from the sweep",
        "file": "crates/core/lisp/lsp.el",
        "old": '(args (list "-l" "--fixed-strings" symbol\n                     "-g" "*.sv" "-g" "*.svh" "-g" "*.v" "-g" "*.vh"\n                     scan-root))',
        "new": '(args (list "-l" "--fixed-strings" symbol scan-root))',
        "test": "references_outside_root_sweep_ignores_non_verilog_files",
    },
    {
        "label": "D5 empty-result path's own root clause removed",
        "file": "crates/core/lisp/lsp.el",
        "old": '(let ((base (format "No references found (project root: %s)" root)))',
        "new": '(let ((base "No references found"))',
        "test": "references_empty_message_names_the_root_for_a_non_verible_server",
    },
    {
        "label": "D6 rename's skipped-edit message no longer names the root",
        "file": "crates/core/lisp/lsp.el",
        "old": '(message "Renamed %d occurrence(s) here (also skipped %d edit(s) in other file(s); project root: %s)"\n                                 n (length other) root))',
        "new": '(message "Renamed %d occurrence(s) here (also skipped %d edit(s) in other file(s))"\n                                 n (length other)))',
        "test": "rename_skipped_message_names_the_root",
    },
    {
        # Own `"test_target"' key -- this entry's guard lives in
        # `lsp_autostart_tests', not `lsp_references_tests' like every
        # other entry here. See the file header for the precedence rule.
        "label": "D7 autostart path's own :root argument removed",
        "file": "crates/core/lisp/lsp.el",
        "old": "             (client (make-lsp--client :conn conn :command command :root root)))",
        "new": "             (client (make-lsp--client :conn conn :command command)))",
        "test": "autostart_records_the_root_it_connected_with_on_the_client",
        "test_target": "lsp_autostart_tests",
    },
    {
        "label": "D8 lsp--verilog-buffer-p gate on the sweep removed",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (outside (and symbol (lsp--verilog-buffer-p file)\n                       (lsp--references-outside-root-files symbol root)))",
        "new": "         (outside (and symbol\n                       (lsp--references-outside-root-files symbol root)))",
        "test": "references_outside_root_sweep_does_not_run_for_non_verilog_buffers",
    },
]
