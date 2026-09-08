# Mutation list for M104 (code formatting with selectable style), designed by
# the reviewer in step 4 and run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m104.py \
#         -p core --test-target format_tests
#
# G1/G2 are the two the reviewer predicted would SURVIVE the original suite
# (the process-group negation and the "style-alist is global" claim); the fix
# round added tests for both, so they are expected to be killed now.

PACKAGE = "core"
TEST_TARGET = "format_tests"

MUTATIONS = [
    {
        "label": "S1 the child's stdin is never closed after writing",
        "file": "crates/elisp/src/shell.rs",
        "old": "        cmd.process_group(0);",
        "new": "        let _ = 0;",
        "test": "call_process_string_timeout_kills_the_whole_process_group_not_just_the_direct_child",
    },
    {
        "label": "G1 the timeout kill drops the process-group negation",
        "file": "crates/elisp/src/shell.rs",
        "old": "                libc::kill(-(child.id() as i32), libc::SIGKILL);",
        "new": "                libc::kill(child.id() as i32, libc::SIGKILL);",
        "test": "call_process_string_timeout_kills_the_whole_process_group_not_just_the_direct_child",
    },
    {
        "label": "F1 formatted output is applied as a full replace instead of minimal edits",
        "file": "crates/core/lisp/format.el",
        "old": "      (replace-region-contents (point-min) (point-max) stdout)))))",
        "new": "      (delete-region (point-min) (point-max)) (insert stdout)))))",
        "test": "format_buffer_uses_minimal_edit_point_on_untouched_line_does_not_move",
    },
    {
        "label": "F2 format-on-save is off by default",
        "file": "crates/core/lisp/format.el",
        "old": "(defvar format-on-save t",
        "new": "(defvar format-on-save nil",
        "test": "format_on_save_formats_before_writing_to_disk",
    },
    {
        "label": "G2 the chosen style is stored buffer-locally instead of globally",
        "file": "crates/core/lisp/format.el",
        "old": "         (setq format-style-alist (format--alist-put format-style-alist mode sym))",
        "new": "         (setq-local format-style-alist (format--alist-put format-style-alist mode sym))",
        "test": "format_set_style_is_visible_from_a_second_buffer_of_the_same_mode",
    },
    {
        "label": "F3 a formatter that signals is no longer caught, so it blocks the save",
        "file": "crates/core/lisp/format.el",
        "old": "    (condition-case err\n        (format-buffer)\n      (error (message \"format-on-save: formatting failed: %S\" err)))))",
        "new": "    (format-buffer)))",
        # Survived the first run: `before-save-hook`'s own runner
        # (commands.rs:940) already swallows hook errors, so the save path
        # cannot see this guard at all. The test named here calls
        # `format--maybe-on-save' directly instead.
        "test": "format_maybe_on_save_itself_does_not_signal_when_the_args_function_errors",
    },
    {
        "label": "F4 clang-format is no longer told which file it is formatting",
        "file": "crates/core/lisp/format.el",
        "old": "     (list (concat \"-assume-filename=\" (buffer-file-name))))))",
        "new": "     nil)))",
        # Survived the first run: the working-directory half of the same fix
        # finds `.clang-format` on its own, so the end-to-end test cannot
        # isolate this flag. The pure-function test named here can.
        "test": "clang_args_includes_assume_filename_when_buffer_has_a_file",
    },
    {
        "label": "F5 verilog-mode goes back to a 4-space default",
        "file": "crates/core/lisp/modes.el",
        "old": "  (treesit--prog-mode-setup 'verilog-mode 'verilog 'verilog-mode-hook 2 'verilog-indent-line))",
        "new": "  (treesit--prog-mode-setup 'verilog-mode 'verilog 'verilog-mode-hook 4 'verilog-indent-line))",
        "test": "verilog_mode_default_indent_width_is_two",
    },
    {
        "label": "F6 indent-detect-width no longer switches detection off",
        "file": "crates/core/lisp/indent.el",
        "old": "  (when indent-detect-width",
        "new": "  (when t",
        "test": "find_file_four_space_sv_with_detection_disabled_keeps_mode_default_of_two",
    },
]
