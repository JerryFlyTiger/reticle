# M93 (a nearer .git no longer shadows the verible.filelist that defines a
# Verilog project) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m93-project-root.py \
#         -p core --test-target lsp_mode_tests
#
# Two defences in this milestone are deliberately absent from this list because
# no test can observe them, and both are documented as unwatched in their own
# docstrings rather than papered over here:
#
#   * the filelist memo -- reverting it entirely fails nothing, because every
#     assertion checks the final root string and a fresh walk returns the same
#     answer; only call count or wall time would distinguish them.
#   * the `(not (lsp--live-buffer-client))` reorder in the autostart path -- a
#     pure boolean-AND rearrangement whose only observable effect is the
#     measured 212.59us -> 5.82us per call, which no test asserts.

PACKAGE = "core"
TEST_TARGET = "lsp_mode_tests"

MUTATIONS = [
    {
        "label": "P1 the filelist ancestor stops outranking a nearer marker",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (found (or filelist-root (lsp--nearest-marker-root start))))",
        "new": "         (found (lsp--nearest-marker-root start)))",
        "test": "project_root_verilog_filelist_outranks_a_nearer_dot_git",
    },
    {
        "label": "P2 the Verilog guard is dropped (every language gets filelist precedence)",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (filelist-root (and (lsp--verilog-buffer-p file)",
        "new": "         (filelist-root (and t",
        "test": "project_root_non_verilog_buffer_still_prefers_the_nearer_marker",
    },
    {
        "label": "P3 the fallback to the ordinary marker walk is dropped",
        "file": "crates/core/lisp/lsp.el",
        "old": "         (found (or filelist-root (lsp--nearest-marker-root start))))",
        "new": "         (found filelist-root))",
        "test": "project_root_verilog_buffer_with_no_filelist_anywhere_behaves_as_today",
    },
    {
        "label": "P4 the filelist probe never finds anything",
        "file": "crates/core/lisp/lsp.el",
        "old": '          (if (file-exists-p (concat dir "verible.filelist"))',
        "new": "          (if nil",
        "test": "project_root_verilog_filelist_outranks_a_nearer_dot_git",
    },
    {
        "label": "P5 the home bound is removed (the walk can escape the project)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                    (not (string= (directory-file-name dir) home)))",
        "new": "                    t)",
        "test": "project_root_verilog_filelist_walk_is_bounded_at_home_and_does_not_shadow_a_nearer_dot_git",
    },
]
