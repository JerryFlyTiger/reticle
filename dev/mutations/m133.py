# M133 (the editor tells slang where headers live) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m133.py \
#         -p core --test-target lsp_verilog_include_tests
#
# Entries whose guard lives in another test binary carry their own
# `"test_target"' key (precedence: entry key > command line > config
# default -- see `dev/mutate.py's own docstring).
#
# This project's rule is one deletion-style entry per FEATURE, not per
# file (M117). M133's distinct effects, each with an entry that removes
# the WHOLE effect rather than perturbing a boundary inside it:
#
#   F1  the wire itself -- `slang.setBuildFile' actually reaching the
#       server, on BOTH handshake paths (D1 `lsp-connect', D2
#       `lsp--autostart-begin'). Two entries because the two paths are
#       independent code, and the autostart one is the path a user
#       actually takes when opening a file.
#   F2  reading `+incdir+' out of `verible.filelist' (D3)
#   F3  discovering header directories under the root (D4)
#   F4  the explicit-variable override (D5)
#   F5  the `auto' guard that stands down when the user configured slang
#       themselves -- the guard as a whole (D6) and each of its THREE
#       config paths (D7, D8, D9), because the cold review found only the
#       first path had a test and what this guard prevents is silently
#       replacing a whole-design build the user chose
#   F6  the capability gate that keeps this away from verible (D10)
#   F7  writing the build file at all (D11), plus the two properties of
#       its content that were learned the hard way: the `//' comment
#       prefix (D12) and the `-I "..."' directive with its quoting (D13)
#   F8  the cap and its truncation announcement (D14)
#   F9  the degenerate-root refusal, on the discovery branch (D15) and on
#       the explicit branch (D16)
#   F10 the session cache (D17)
#   F11 error isolation -- the fix-round `condition-case' that keeps a
#       build-file write failure from tearing down a healthy connection
#       (D18)
#   F12 `lsp-verilog-show-include-directories', the feature's only
#       observable surface (D19)
#   F13 the injective build-file filename (D20)
#   F14 the `make-directory' builtin this milestone added (D21)
#   F15 refusing a directory name the `.f' format cannot represent
#       -- a `"', a backslash or a newline (D22)
#
# D22 sits next to D20 in the list rather than after D21, because both
# concern the generated file and reading them together is easier than
# reading them in numeric order. The numbering is a stable label, not a
# position.
#
# D19 exists because the cold review found that this command -- which its
# own docstring calls the only thing that can observe this feature -- had
# NO test at all: its body could be deleted whole and the suite stayed
# green. That is the M116/M117 "feature and tests not wired" shape; the
# fix round added the tests, and this entry is what keeps them honest.
#
# D12 and D13 are not boundary perturbations either, even though each
# changes a format string. Each removes a property that was measured
# against the real `slang-server' and that got the feature WRONG in
# practice:
#   * `;;' (elisp's comment syntax, the obvious thing to reach for) is
#     not a comment in slang's `.f' format at all -- slang read every
#     whitespace-separated token of the header as a filename, and the
#     directive line's effect was lost with them. The first
#     implementation round shipped exactly this, passed all 17 of its own
#     tests, and did not work at all.
#   * `+incdir+' is the wrong directive: slang splits its argument on
#     BOTH whitespace and `+', so a directory named `.../inc+plus' came
#     back as two wrong include directories, and double-quoting rescues
#     the space case but NOT the `+' case. `-I "<dir>"' has neither
#     problem (measured correct for `+', for a space, and for both at
#     once).
#
# Every entry here is replacement-style. This project's measured lesson
# (M83's `C-x s' prefix, M85's four-in-one-round) is that insertion-style
# mutations miss their target: a same-named stub inserted ahead of the
# real definition does not shadow it, because both elisp's `defun' and
# Rust's `defun()' registration let a LATER definition overwrite an
# earlier one.

MUTATIONS = [
    # ---- F1: the wire, both handshake paths ----
    {
        "label": "D1 the push is never called from lsp-connect",
        "file": "crates/core/lisp/lsp.el",
        "old": "          (lsp--verilog-maybe-push-include-directories client))",
        "new": "          nil)",
        "test": "wire_sends_set_build_file_on_the_lsp_connect_path",
    },
    {
        "label": "D2 the push is never called from lsp--autostart-begin",
        "file": "crates/core/lisp/lsp.el",
        "old": "               (lsp--verilog-maybe-push-include-directories client)",
        "new": "               nil",
        "test": "wire_sends_set_build_file_on_the_autostart_path",
    },
    # ---- F2: reading +incdir+ out of verible.filelist ----
    {
        "label": "D3 +incdir+ lines in a file list contribute nothing",
        "file": "crates/core/lisp/lsp.el",
        "old": '            (when (string-prefix-p "+incdir+" line)',
        "new": '            (when (and nil (string-prefix-p "+incdir+" line))',
        "test": "filelist_incdirs_parses_a_single_entry",
    },
    # ---- F3: discovery ----
    {
        "label": "D4 discovery always takes the refusal branch (no directory is ever discovered)",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (if (lsp--verilog-degenerate-root-p root)",
        "new": "  (if t",
        "test": "discovery_unions_a_header_directory_and_a_filelist_incdir_deduplicated",
    },
    # ---- F4: the explicit-variable override ----
    {
        "label": "D5 the explicit variable is ignored and discovery always runs",
        "file": "crates/core/lisp/lsp.el",
        "old": "             (if lsp-verilog-include-directories\n                 (lsp--verilog-include-directories--explicit root)\n               (lsp--verilog-include-directories--discover root))))",
        "new": "             (lsp--verilog-include-directories--discover root)))",
        "test": "explicit_variable_wins_outright_and_discovery_does_not_run",
    },
    # ---- F5: the `auto' guard, as a whole and per config path ----
    {
        "label": "D6 the auto guard never fires (the user's own slang config is ignored entirely)",
        "file": "crates/core/lisp/lsp.el",
        "old": "               ((and (not (eq lsp-verilog-push-include-directories t))\n                     (lsp--verilog-user-configured-slang-p root))",
        "new": "               ((and nil\n                     (lsp--verilog-user-configured-slang-p root))",
        "test": "wire_not_sent_when_the_user_has_their_own_slang_config_under_auto",
    },
    {
        "label": "D7 the guard stops seeing <ROOT>/.slang/server.json",
        "file": "crates/core/lisp/lsp.el",
        "old": '  (or (and root (file-exists-p (concat (file-name-as-directory root) ".slang/server.json")))',
        "new": "  (or nil",
        "test": "wire_not_sent_when_the_user_has_their_own_slang_config_under_auto",
    },
    {
        "label": "D8 the guard stops seeing <ROOT>/.slang/local/server.json",
        "file": "crates/core/lisp/lsp.el",
        "old": '      (and root (file-exists-p (concat (file-name-as-directory root) ".slang/local/server.json")))',
        "new": "      nil",
        "test": "user_configured_slang_p_detects_root_slash_dot_slang_slash_local_slash_server_json",
    },
    {
        "label": "D9 the guard stops seeing ~/.slang/server.json",
        "file": "crates/core/lisp/lsp.el",
        "old": '      (file-exists-p (concat (lsp--home-directory) "/.slang/server.json"))))',
        "new": "      nil))",
        "test": "user_configured_slang_p_detects_home_slash_dot_slang_slash_server_json",
    },
    # ---- F6: the capability gate ----
    {
        "label": "D10 the capability gate always says the server supports slang.setBuildFile",
        "file": "crates/core/lisp/lsp.el",
        "old": "               ((not (lsp--verilog-server-declares-set-build-file-p client))",
        "new": "               ((and nil (not (lsp--verilog-server-declares-set-build-file-p client)))",
        "test": "wire_not_sent_when_the_server_declares_no_set_build_file_command",
    },
    # ---- F7: the build file, whole effect and its two measured properties ----
    {
        "label": "D11 the build file is never written (the push points at a file that does not exist)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                      (lsp--verilog-write-build-file path root dirs)",
        "new": "                      nil",
        "test": "wire_sends_set_build_file_on_the_lsp_connect_path",
    },
    {
        "label": "D12 the generated header uses `;;' again (elisp comment syntax, which slang reads as filenames)",
        "file": "crates/core/lisp/lsp.el",
        "old": '            (insert (format "// Generated by reticle -- Verilog include \\\ndirectories for %s\\n// Rewritten on every LSP connect (M133); editing \\\nthis file by hand is pointless.\\n" root))',
        "new": '            (insert (format ";; Generated by reticle -- Verilog include \\\ndirectories for %s\\n;; Rewritten on every LSP connect (M133); editing \\\nthis file by hand is pointless.\\n" root))',
        "test": "write_build_file_comment_header_uses_slash_slash_not_semicolon_semicolon",
    },
    {
        "label": "D13 the directive goes back to unquoted `+incdir+' (splits on whitespace and on `+')",
        "file": "crates/core/lisp/lsp.el",
        "old": '              (insert (format "-I \\"%s\\"\\n" d)))',
        "new": '              (insert (format "+incdir+%s\\n" d)))',
        "test": "write_build_file_content_is_exactly_the_dash_i_lines_absolute_quoted_one_per_line",
    },
    # ---- F8: the cap and its announcement ----
    {
        "label": "D14 the cap never truncates and never announces",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (if (> n lsp-verilog-include-directories-max)",
        "new": "    (if nil",
        "test": "discovery_cap_truncates_and_announces_the_truncation",
    },
    # ---- F9: the degenerate-root refusal, both branches ----
    {
        "label": "D15 discovery stops refusing a degenerate root and crawls it anyway",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (if (lsp--verilog-degenerate-root-p root)",
        "new": "  (if nil",
        "test": "discovery_refuses_a_degenerate_root",
    },
    {
        "label": "D16 the explicit branch resolves relative entries against a degenerate root anyway",
        "file": "crates/core/lisp/lsp.el",
        "old": '      (if (and degenerate (not (string-prefix-p "/" d)))',
        "new": '      (if (and nil (not (string-prefix-p "/" d)))',
        "test": "explicit_variable_branch_refuses_a_relative_entry_under_a_degenerate_root",
    },
    # ---- F10: the session cache ----
    {
        "label": "D17 the discovery result is never memoized (every call re-shells out)",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (puthash root answer lsp--verilog-include-directories-cache)",
        "new": "        (ignore answer)",
        "test": "discovery_result_is_cached_and_does_not_reshell_out_on_a_second_call",
    },
    # ---- F11: error isolation (the cold review's HIGH finding) ----
    {
        "label": "D18 the handler stops catching, so a build-file write failure escapes and kills the connection",
        "file": "crates/core/lisp/lsp.el",
        "old": '            (error\n             (list (cons :sent nil)\n                   (cons :reason (format "could not push include directories: %s"',
        "new": '            (lsp--verilog-never-signalled\n             (list (cons :sent nil)\n                   (cons :reason (format "could not push include directories: %s"',
        "test": "maybe_push_a_build_file_write_failure_does_not_kill_the_connection",
    },
    # ---- F12: the only observable surface ----
    {
        "label": "D19 the report command always claims nothing was sent",
        "file": "crates/core/lisp/lsp.el",
        "old": "         ((cdr (assq :sent info))",
        "new": "         ((and nil (cdr (assq :sent info)))",
        "test": "show_include_directories_success_message_names_the_build_file_and_directories",
    },
    # ---- F13: the injective generated filename ----
    {
        # Two naive schemes have now been wrong here, and this entry reverts to
        # the SECOND one -- the escape-then-map scheme the first fix round
        # shipped, which fixes `/tmp/x/proj-a/sub' vs `/tmp/x/proj/a-sub' and
        # then collides on `/foo/-bar' vs `/foo-/bar' instead, because `--'
        # (an escaped hyphen) and `-' (a mapped slash) share one alphabet with
        # no separator. Reverting to the FIRST naive scheme (no escaping at
        # all) would also be caught, but by the older half of the test; this
        # substitution is the one that pins the fix that was actually hard.
        #
        # This entry must revert BOTH halves of the expression. A first cut
        # changed only the escape tag (`-d' back to `--') and SURVIVED -- not
        # a coverage gap, but a mutation that landed without removing the
        # semantics: with `/' still mapping to the two-character `-s', the
        # encoding stays unambiguous no matter what the other tag is. It is
        # the PAIR of distinct tags that carries injectivity, so the pair is
        # what a deletion entry has to remove. (This project's standing
        # two-part rule for SURVIVED: did it land, and did it change meaning.
        # Here the answer was yes, then no.)
        "label": "D20 the WHOLE sanitisation goes back to escape-then-map (collides when `-' abuts `/')",
        "file": "crates/core/lisp/lsp.el",
        "old": '          (replace-regexp-in-string\n           "/" "-s"\n           (replace-regexp-in-string "-" "-d" root))',
        "new": '          (replace-regexp-in-string\n           "/" "-"\n           (replace-regexp-in-string "-" "--" root))',
        "test": "build_file_path_sanitisation_is_injective_for_roots_that_collided_under_either_naive_scheme",
    },
    # ---- F15: refusing a directory name the `.f' format cannot represent ----
    {
        # `-I "%s"' does no escaping, so a directory name holding a `"', a
        # backslash or a newline would emit a line that silently means
        # something else -- a newline worst of all, since it breaks the file's
        # own line structure. The code drops such a directory and says so
        # rather than writing the broken line.
        "label": "D22 directory names the .f format cannot represent are written out anyway",
        "file": "crates/core/lisp/lsp.el",
        "old": '      (if (string-match-p "[\\"\\\\\\n]" d)',
        "new": '      (if nil',
        "test": "write_build_file_drops_a_directory_name_containing_a_quote_and_announces_it",
    },
    # ---- F14: the make-directory builtin ----
    {
        "label": "D21 make-directory does nothing at all and never errors",
        "file": "crates/core/src/builtins/files.rs",
        "old": "    if parents {\n        std::fs::create_dir_all(dir)",
        "new": "    if true {\n        return Ok(());\n    }\n    if parents {\n        std::fs::create_dir_all(dir)",
        "test": "make_directory_without_parents_errors_on_a_missing_parent",
        "test_target": "dired_ops_tests",
    },
]
