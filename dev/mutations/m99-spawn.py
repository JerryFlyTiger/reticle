# Mutation list for M99's LSP-server cwd wiring (`LspConnection::spawn's new
# CWD parameter, and the two elisp call sites that feed it the project root).
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m99-spawn.py \
#         -p core --test-target lsp_tests
#
# Designed by the reviewer that cold-read the M99 diff plus the fix round;
# run from the main conversation.
#
# Why this exists: slang-server reads its per-project config from
# `<root>/.slang/server.json', and a relative path in that file's `flags'
# (this repo ships `-I include') is resolved by slang against ITS OWN process
# cwd -- not the workspace root, not the config file's directory. Before M99
# nothing ever called `current_dir` on an LSP spawn, so a committed config with
# relative paths only worked when the editor happened to be launched from the
# project root. Measured against the real binary: cwd at the repo root gives
# 18 diagnostics including a bogus `'soc_defs.svh': No such file or directory';
# cwd at demo/rtl gives 17, all honest.
#
# M8 and M9 are the two the first review round found had no coverage at all:
# `lsp-start' itself was tested, but nothing checked that `lsp-connect' and
# `lsp--autostart-begin' actually pass the root down -- which is the entire
# reason the milestone exists.

PACKAGE = "core"
TEST_TARGET = "lsp_tests"

MUTATIONS = [
    {
        "label": "M6 a relative cmd containing '/' is no longer pinned to the editor's cwd",
        "file": "crates/elisp/src/lsp.rs",
        "old": "                    resolved_cmd = here.join(cmd).to_string_lossy().into_owned().into();",
        "new": "                    let _ = here;",
        "test": "lsp_start_relative_cmd_containing_a_slash_still_resolves_against_the_editor_cwd_not_the_new_server_cwd",
    },
    {
        "label": "M7 the CWD argument is accepted and then ignored",
        "file": "crates/elisp/src/lsp.rs",
        "old": "        if let Some(dir) = effective_cwd {\n            command.current_dir(dir);\n        }",
        "new": "        if let Some(dir) = effective_cwd {\n            let _ = dir;\n        }",
        "test": "lsp_start_with_cwd_arg_sets_the_server_processs_working_directory",
    },
    {
        "label": "M8 lsp-connect stops passing its root-path down as the server cwd",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (let* ((conn (lsp-start command args\n                          (and root-path (file-directory-p root-path) root-path)))",
        "new": "  (let* ((conn (lsp-start command args))",
        "test": "lsp_connect_passes_its_root_path_argument_through_as_the_servers_cwd",
    },
    {
        "label": "M9 lsp--autostart-begin stops passing its root down as the server cwd",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (let* ((conn (lsp-start command args\n                              (and (file-directory-p root) root)))",
        "new": "      (let* ((conn (lsp-start command args))",
        "test": "lsp_autostart_begin_passes_its_root_argument_through_as_the_servers_cwd",
    },
]
