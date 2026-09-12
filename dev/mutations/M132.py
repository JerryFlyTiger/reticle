# M132 (per-server rootUri for the Verilog LSP pair) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/M132.py \
#         -p core --test-target lsp_mode_tests
#
# Entries whose guard lives in another test binary carry their own
# `"test_target"' key (precedence: entry key > command line > config
# default -- see `dev/mutate.py's own docstring).
#
# M132 ships THREE distinct effects, and this project's rule is one
# deletion-style entry per FEATURE, not per file (M117: a list with a real
# deletion entry for one of three shipped features and only boundary
# perturbations for the other two is indistinguishable, to a later reader,
# from nobody having thought about them):
#
#   F1  the `workspace' root style itself -- the A-prime rule that widens a
#       slang root to cover the buffer's whole filelist connected component
#       (D1 kills the dispatch, D2 kills the component computation under it)
#   F1e primary and secondary actually GETTING different roots -- the six
#       connection-identity call sites (D3 the autostart computation itself,
#       D7 the backfill match, D8 the pending marker)
#   F2  the duplicate-declaration detector and its warning (D4 the detector,
#       D5/D6 its two real wiring points)
#
# D5/D6 exist because the first cut had none: every F2 test called the
# helpers directly and BOTH wiring lines could be deleted with nothing going
# red -- the M116/M117 "feature and tests not wired" shape, found by cold
# review, fixed in the fix round. D7/D8 are the same disease at two more
# sites: the review PREDICTED both would survive (it reasoned from the code
# and said so rather than claiming to have run them), and both did, until
# the fix round added assertions on `lsp--effective-buffer-clients' and
# `lsp--autostart-pending-here'.
#
# Every entry here is replacement-style. This project's measured lesson
# (M83's `C-x s' prefix, M85's four-in-one-round) is that insertion-style
# mutations miss their target: a same-named stub inserted ahead of the real
# definition does not shadow it, because both elisp's `defun' and Rust's
# `defun()' registration let a LATER definition overwrite an earlier one.

MUTATIONS = [
    # ---- F1: the workspace root style ----
    {
        "label": "D1 workspace root style removed entirely (every command gets the old root)",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (if (eq (lsp--server-root-style command) 'workspace)
      (or (lsp--filelist-component-root file) (lsp--project-root file))
    (lsp--project-root file)))""",
        "new": "  (lsp--project-root file))",
        "test": "project_root_for_command_workspace_style_widens_to_the_filelist_component",
    },
    {
        "label": "D2 connected-component computation removed (always falls back)",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (let ((answer (lsp--filelist-component-root--compute file start cap)))",
        "new": "        (let ((answer nil))",
        "test": "filelist_component_root_merges_transitively_across_three_lists",
    },
    # ---- F1e: primary and secondary getting DIFFERENT roots ----
    {
        # The headline structural fix: `lsp--autostart-try-one' used to be
        # handed one ROOT computed once by `lsp--autostart-maybe-begin' and
        # passed to BOTH the primary and the secondary. Reverting just this
        # computation reproduces that shared root exactly.
        "label": "D3 autostart computes a command-blind root (primary and secondary share one again)",
        "file": "crates/core/lisp/lsp.el",
        "old": """           (root (lsp--project-root-for-command command file))
           (key (cons command root)))""",
        "new": """           (root (lsp--project-root file))
           (key (cons command root)))""",
        "test": "autostart_gives_primary_and_secondary_different_roots",
        "test_target": "lsp_autostart_tests",
    },
    {
        # Silent-attach failure, not an error: the connection is registered
        # under the widened root, this lookup asks for the narrow one, the
        # predicate returns nil and backfill just never attaches the buffer
        # that started the autostart to its own new server.
        "label": "D7 backfill matcher uses a command-blind root (secondary never attaches)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                   command (lsp--project-root-for-command command file))",
        "new": "                   command (lsp--project-root file))",
        "test": "autostart_gives_primary_and_secondary_different_roots",
        "test_target": "lsp_autostart_tests",
    },
    {
        "label": "D8 pending-marker predicate uses a command-blind root (mode-line indicator dead)",
        "file": "crates/core/lisp/lsp.el",
        "old": "       (equal (lsp--project-root-for-command command file) root)))",
        "new": "       (equal (lsp--project-root file) root)))",
        "test": "autostart_gives_primary_and_secondary_different_roots",
        "test_target": "lsp_autostart_tests",
    },
    # ---- F2: the duplicate-declaration detector ----
    {
        "label": "D4 duplicate-declaration detector removed entirely (always reports clean)",
        "file": "crates/core/lisp/lsp.el",
        "old": """                     "-e" lsp--workspace-declaration-rg-pattern
                     root))
         (result (call-process-string "rg" args "" 2000 nil)))""",
        "new": """                     "-e" lsp--workspace-declaration-rg-pattern
                     root))
         (result nil))""",
        "test": "workspace_duplicate_declarations_finds_a_name_declared_in_two_files",
    },
    {
        "label": "D5 detector unwired from lsp-connect (M-x lsp path)",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (when root-path (lsp--workspace-maybe-warn-duplicates command root-path))",
        "new": "    (when root-path nil)",
        "test": "lsp_connect_warns_about_duplicate_declarations_under_a_widened_root",
    },
    {
        "label": "D6 detector unwired from the autostart success path (the path a GUI session uses)",
        "file": "crates/core/lisp/lsp.el",
        "old": "               (lsp--workspace-maybe-warn-duplicates command root)",
        "new": "               nil",
        "test": "autostart_warns_about_duplicate_declarations_under_a_widened_root",
        "test_target": "lsp_autostart_tests",
    },
    # ---- Boundary perturbations inside the three features ----
    {
        "label": "B1 component merges file lists that share nothing (two designs become one)",
        "file": "crates/core/lisp/lsp.el",
        "old": "            (when shares",
        "new": "            (when t",
        "test": "filelist_component_root_disjoint_lists_do_not_merge",
    },
    {
        # NOTE: this entry first pointed at `filelist_component_root_is_
        # capped_at_git', which SURVIVED -- that test's fixture puts the file
        # list ABOVE the `.git', so it is stopped by the EARLIER guard in
        # `--compute' and never reaches this clamp at all. Two different
        # guards, only one of them watched. The clamp is reachable because a
        # file list's ENTRIES can escape the cap with `../..' even though the
        # list itself was found under it.
        "label": "B2 .git clamp removed from the computed root",
        "file": "crates/core/lisp/lsp.el",
        "old": """        (directory-file-name
         (if (or (string= root cap) (string-prefix-p cap root))
             root
           cap))))))""",
        "new": "        (directory-file-name root)))))",
        "test": "filelist_component_root_clamps_an_entry_that_escapes_the_git_root",
    },
    {
        "label": "B3 macromodule dropped from the declaration pattern (duplicate goes unreported)",
        "file": "crates/core/lisp/lsp.el",
        "old": '  "^[[:space:]]*(module|macromodule|package|interface(?:[[:space:]]+class)?|program)[[:space:]]+([A-Za-z_][A-Za-z0-9_$]*)"',
        "new": '  "^[[:space:]]*(module|package|interface(?:[[:space:]]+class)?|program)[[:space:]]+([A-Za-z_][A-Za-z0-9_$]*)"',
        "test": "workspace_duplicate_declarations_covers_macromodule",
    },
    {
        # `interface class Foo;' -- the identifier right after `interface'
        # is the keyword `class', so without matching the pair as a unit the
        # detector reports a duplicate of a thing called "class".
        "label": "B4 interface-class not matched as a unit (name captured is the keyword `class')",
        "file": "crates/core/lisp/lsp.el",
        "old": r'  "^[[:space:]]*\\(?:module\\|macromodule\\|package\\|interface\\(?:[[:space:]]+class\\)?\\|program\\)[[:space:]]+\\([A-Za-z_][A-Za-z0-9_$]*\\)"',
        "new": r'  "^[[:space:]]*\\(?:module\\|macromodule\\|package\\|interface\\|program\\)[[:space:]]+\\([A-Za-z_][A-Za-z0-9_$]*\\)"',
        "test": "workspace_duplicate_declarations_reads_the_name_of_an_interface_class",
    },
    {
        "label": "B5 warn-once dedup removed (the same root warns on every connection)",
        "file": "crates/core/lisp/lsp.el",
        "old": "             (not (member root lsp--workspace-duplicate-warned-roots)))",
        "new": "             t)",
        "test": "workspace_duplicate_warning_is_emitted_once_per_root",
    },
    {
        "label": "B6 filelist parser stops skipping `//' comment lines",
        "file": "crates/core/lisp/lsp.el",
        "old": '                      (string-prefix-p "//" line)\n',
        "new": "",
        "test": "filelist_entries_skips_comments_blanks_and_plusargs",
    },
    {
        # The extraction guard. `lsp--filelist-entries' was lifted OUT of
        # `verilog-auto--library-filelist-files' and is now shared; the two
        # extra filters that function applies on top are the only thing
        # keeping its behaviour unchanged across that refactor. A gate
        # cannot see a product-and-tests-changed-together refactor going
        # consistently wrong -- only a mutation or a cold read can.
        # NOTE: this entry first pointed at `verible_filelist_ignores_
        # comments_blank_lines_flags_and_missing_paths', which SURVIVED: that
        # test asserts downstream AUTOINST output, and dropping the filters
        # only lets a non-existent path into the candidate list, where it
        # yields no module and changes nothing observable. A test that cannot
        # reach what its own name claims.
        "label": "B7 verilog-auto's own two extra filters dropped after the parser extraction",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """            (when (and (verilog-auto--library-file-name-p
                        (file-name-nondirectory path))
                       (file-exists-p path))""",
        "new": "            (when t",
        "test": "verible_filelist_files_drops_missing_paths_and_non_library_names",
        "test_target": "verilog_auto_tests",
    },
]
