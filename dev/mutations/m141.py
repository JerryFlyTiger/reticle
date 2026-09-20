# Mutation list for M141: `M-x compile` follows make's Entering/Leaving
# directory lines and counts error lines dropped for a missing file
# (crates/core/lisp/compile.el), plus merged process output sharing one
# pipe (crates/elisp/src/shell.rs).
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m141.py
#
# Designed from the two cold reads' checklists. Run from the main
# conversation only, with no agent running.

PACKAGE = "core"
TEST_TARGET = "compile_tests"

MUTATIONS = [
    {
        "label": "C1 whole directory-tracking effect: Entering never pushes",
        "file": "crates/core/lisp/compile.el",
        "old": "(push (expand-file-name (cdr dirmatch) cur-dir) stack)",
        "new": "(ignore (expand-file-name (cdr dirmatch) cur-dir) stack)",
        "test": "compile_follows_gmake_entering_directory_apostrophe_style",
    },
    {
        "label": "C2 Leaving never pops",
        "file": "crates/core/lisp/compile.el",
        "old": "(when stack (setq stack (cdr stack)))",
        "new": "(ignore stack)",
        "test": "compile_tracks_nested_directory_changes_with_real_files_at_each_level",
    },
    {
        "label": "C3 relative Entering resolves against the job dir (GNU's rule)",
        "file": "crates/core/lisp/compile.el",
        "old": "(push (expand-file-name (cdr dirmatch) cur-dir) stack)",
        "new": "(push (expand-file-name (cdr dirmatch) dir) stack)",
        "test": "compile_tracks_nested_directory_changes_with_real_files_at_each_level",
    },
    {
        "label": "C4 the empty-stack guard on pop removed (inert: (cdr nil) is nil)",
        "file": "crates/core/lisp/compile.el",
        "old": "(when stack (setq stack (cdr stack)))",
        "new": "(setq stack (cdr stack))",
        "test": "compile_leaving_with_empty_stack_is_a_no_op",
        "expect": "survived",
    },
    {
        "label": "C5 directory regexp without backquote-open (make 3.81 -w)",
        "file": "crates/core/lisp/compile.el",
        "old": "directory [`']",
        "new": "directory [']",
        "test": "compile_follows_make_entering_directory_backquote_style",
    },
    {
        "label": "C6 directory regexp without apostrophe-open (gmake 4.x)",
        "file": "crates/core/lisp/compile.el",
        "old": "directory [`']",
        "new": "directory [`]",
        "test": "compile_follows_gmake_entering_directory_apostrophe_style",
    },
    {
        "label": "C7 directory regexp tried before the error header (F1 reverted)",
        "file": "crates/core/lisp/compile.el",
        "old": "(raw (compile--parse-error-line-raw line cur-dir)))\n              (cond\n               (raw",
        "new": "(raw (and (not (compile--match-directory-line line))\n                             (compile--parse-error-line-raw line cur-dir))))\n              (cond\n               (raw",
        "test": "compile_error_line_containing_entering_directory_text_is_not_swallowed",
    },
    {
        "label": "C8 whole dropped-count effect: nothing is ever counted",
        "file": "crates/core/lisp/compile.el",
        "old": "(setq dropped (1+ dropped))",
        "new": "(ignore dropped)",
        "test": "compile_unannounced_directory_change_is_dropped_but_counted_in_message",
    },
    {
        "label": "C9 all-digits FILE filter removed (timestamps counted)",
        "file": "crates/core/lisp/compile.el",
        "old": "(not (compile--all-digits-p (aref raw 5)))",
        "new": "t",
        "test": "compile_timestamp_line_does_not_inflate_dropped_count",
    },
    {
        "label": "C10 sorry no longer counted",
        "file": "crates/core/lisp/compile.el",
        "old": "'(error warning sorry)",
        "new": "'(error warning)",
        "test": "compile_dropped_sorry_line_is_counted",
    },
    {
        "label": "C11 severity filter removed (note lines counted)",
        "file": "crates/core/lisp/compile.el",
        "old": "(memq (aref raw 3) '(error warning sorry))",
        "new": "t",
        "test": "compile_unannounced_directory_change_is_dropped_but_counted_in_message",
    },
    {
        "label": "C12 truncation message loses the dropped count",
        "file": "crates/core/lisp/compile.el",
        "old": "process killed%s\"\n                           (compile--dropped-suffix dropped))",
        "new": "process killed%s\"\n                           \"\")",
        "test": "compile_output_cap_truncation_message_reports_dropped_count",
    },
    {
        "label": "C13 trailing CR not stripped before the directory regexp",
        "file": "crates/core/lisp/compile.el",
        "old": "(stripped (compile--strip-trailing-cr line))",
        "new": "(stripped line)",
        "test": "compile_follows_directory_change_with_crlf_line_endings",
    },
]
