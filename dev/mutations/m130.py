# M130 (vim-style j/k/G motion in emacs-state special-mode local keymaps)
# mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m130.py -p core
#
# M130 adds `j' -> `next-line' / `k' -> `previous-line', and `G' -> a
# last-line landing command, to FIVE local keymaps (dired-mode, help-mode,
# shell-command-mode, compilation-mode, search-mode's read-only view).
# `search-edit-mode' was bound in the first draft too, making six, but
# fix round FIX-1 reverted it entirely -- see below. Fix rounds FIX-1
# through FIX-8 (two cold-read reviews of the original implementation)
# changed the shape from that first draft -- see each entry's own comment
# below for what changed and why.
#
# `search.el' originally contributed two of the six (the READ-ONLY view
# keymap installed by `search--install-view-keymap' and the WRITABLE edit
# keymap installed by `search--install-edit-keymap'). Fix round FIX-1
# reverted the edit keymap's `j'/`k'/`G' entirely: `search-edit-mode' is
# M83's wgrep-level free-typing edit buffer (the same category as
# `eshell-mode'/`ielm-mode', which were never touched), and binding motion
# there would silently eat the letters `j'/`k'/`G' out of every typed
# replacement line -- common in Verilog identifiers (`clk', `jtag',
# `join_none'). D4/D4b/D4c below each guard the ABSENCE of ONE of those
# three bindings -- fix round FIX-8 split this from a single `j'-only
# entry after cold review found the other two letters had no regression
# coverage of their own (the test D4 originally pointed at,
# `search_edit_j_still_self_inserts', only ever pressed `j'). All three
# are deliberately REPLACEMENT-style (old text unique on its own, new
# text is old text plus one appended line) rather than a bare insertion
# -- this project's own delegation-discipline notes warn that insertion-
# style mutations are prone to missing their target (a same-named later
# definition silently shadows an earlier one; a binding threaded into
# the middle of a chain gets rebuilt away by what follows). Anchoring on
# the immediately-preceding, already-unique `C-c C-c'/`C-c C-k' lines
# sidesteps both failure modes.
#
# Per this project's per-FEATURE rule, `j'/`k' (one effect) and `G' (a
# SEPARATE effect, on the five modes that have it) each get their own
# deletion-style entry. `G' was not covered by a dedicated entry in the
# first draft of this file -- cold review pointed out that the six
# original entries only ever deleted `j'/`k'/`G' together, so "only `G'
# silently reverted to `end-of-buffer''s wrong landing spot" had zero
# mutation coverage. G1/G3 below close that gap for the two modes where it
# actually matters.
#
# `G' is NOT a plain `end-of-buffer' alias in dired-mode or search-mode's
# read-only view: `dired-insert-listing' (files.rs) and `search--render'/
# `search--insert-line' (search.el) both end EVERY row, including the
# last, in its own "\n" -- so plain `end-of-buffer' lands one line PAST the
# last real row, where `dired--entry-at-point'/`search--result-at-buffer-
# pos' find nothing (reproduced by direct execution before either fix was
# written). `dired-goto-last-entry' and `search-goto-last-result' (both
# new in this fix round) land ON the last real line instead;
# `compile'/`shell-command'/`help-mode' are plain scrolling text buffers
# with no such trailing empty row, so plain `end-of-buffer' is correct
# there, unmodified -- G2/G4/G5 below cover those three's `G' bindings
# with their own deletion entries even though the command itself didn't
# change, since the earlier point stands: nothing named `G' anywhere was
# covered on its own before this fix round.
#
# No entry targets `gg' or the `eshell.el'/`ielm.el' exclusion -- neither
# was implemented (see dired.el's header note for why), so there is
# nothing to mutate.

PACKAGE = "core"

MUTATIONS = [
    # --- j/k: one effect per map, deletion-style -----------------------
    {
        "label": "D1 dired's j/k binding removed",
        "file": "crates/core/lisp/dired.el",
        "old": (
            '      (define-key map "C" \'dired-do-copy)\n'
            '      (define-key map "R" \'dired-do-rename)\n'
            '      (define-key map "j" \'next-line)\n'
            '      (define-key map "k" \'previous-line)\n'
        ),
        "new": (
            '      (define-key map "C" \'dired-do-copy)\n'
            '      (define-key map "R" \'dired-do-rename)\n'
        ),
        "test_target": "dired_tests",
        "test": "dired_j_and_k_move_by_line",
    },
    {
        "label": "D2 compilation's j/k binding removed",
        "file": "crates/core/lisp/compile.el",
        "old": (
            '          (define-key map "j" \'next-line)\n'
            '          (define-key map "k" \'previous-line)\n'
        ),
        "new": "",
        "test_target": "compile_tests",
        "test": "compilation_j_and_k_move_without_inserting",
    },
    {
        "label": "D3 search-mode's (read-only view) j/k binding removed",
        "file": "crates/core/lisp/search.el",
        "old": (
            '    (define-key map "j" \'next-line)\n'
            '    (define-key map "k" \'previous-line)\n'
        ),
        "new": "",
        "test_target": "search_tests",
        "test": "search_view_j_and_k_move_by_line",
    },
    {
        # Fix round FIX-1: this is NOT a deletion of an existing binding
        # (search-edit-mode has none, deliberately). It REINTRODUCES the
        # exact regression FIX-1 removed, to confirm the regression guard
        # actually fires if someone binds `j' back to motion here in the
        # future. Replacement-style, anchored on the already-unique
        # `C-c C-k' line -- see this file's header for why insertion-
        # style was avoided.
        "label": "D4 search-edit-mode's j binding REINTRODUCED (regression guard)",
        "file": "crates/core/lisp/search.el",
        "old": ('    (define-key map "C-c C-k" \'search-edit-discard)\n'),
        "new": (
            '    (define-key map "C-c C-k" \'search-edit-discard)\n'
            '    (define-key map "j" \'next-line)\n'
        ),
        "test_target": "search_tests",
        "test": "search_edit_j_k_capital_g_all_still_self_insert",
    },
    {
        # Fix round FIX-8: same reasoning as D4, for `k' instead of `j'.
        # Anchored on the `C-c C-c' line (also already unique) rather
        # than reusing D4's own anchor, so the two entries don't collide
        # if the runner ever ran them un-reverted against each other --
        # each mutation in this list is still applied and reverted one
        # at a time against the pristine file, but there is no reason to
        # make that an unstated assumption when a second unique anchor
        # is free.
        "label": "D4b search-edit-mode's k binding REINTRODUCED (regression guard)",
        "file": "crates/core/lisp/search.el",
        "old": ('    (define-key map "C-c C-c" \'search-edit-apply)\n'),
        "new": (
            '    (define-key map "C-c C-c" \'search-edit-apply)\n'
            '    (define-key map "k" \'previous-line)\n'
        ),
        "test_target": "search_tests",
        "test": "search_edit_j_k_capital_g_all_still_self_insert",
    },
    {
        # Fix round FIX-8: same reasoning as D4, for `G' instead of `j'.
        "label": "D4c search-edit-mode's G binding REINTRODUCED (regression guard)",
        "file": "crates/core/lisp/search.el",
        "old": ('    (define-key map "C-c C-k" \'search-edit-discard)\n'),
        "new": (
            '    (define-key map "C-c C-k" \'search-edit-discard)\n'
            '    (define-key map "G" \'end-of-buffer)\n'
        ),
        "test_target": "search_tests",
        "test": "search_edit_j_k_capital_g_all_still_self_insert",
    },
    {
        "label": "D5 help-mode's j/k binding removed",
        "file": "crates/core/lisp/simple.el",
        "old": (
            '        (define-key map "j" \'next-line)\n'
            '        (define-key map "k" \'previous-line)\n'
        ),
        "new": "",
        "test_target": "help_tests",
        "test": "help_j_and_k_move_by_line",
    },
    {
        "label": "D6 shell-command output buffer's j/k binding removed",
        "file": "crates/core/lisp/shell-command.el",
        "old": (
            '          (define-key map "j" \'next-line)\n'
            '          (define-key map "k" \'previous-line)\n'
        ),
        "new": "",
        "test_target": "shell_command_tests",
        "test": "shell_command_output_j_moves_and_does_not_insert_text",
    },
    # --- G: a SEPARATE effect from j/k, its own deletion-style entry per
    # --- map that has it. search-edit-mode is excluded -- it has no G
    # --- binding at all after FIX-1. ------------------------------------
    {
        "label": "G1 dired's G binding removed (falls through to global self-insert)",
        "file": "crates/core/lisp/dired.el",
        "old": (
            '      (define-key map "k" \'previous-line)\n'
            "      ;; M130 fix round FIX-2: `dired-goto-last-entry', not `end-of-\n"
            "      ;; buffer' -- see that function's own doc comment for why a plain\n"
            "      ;; end-of-buffer lands one line PAST the last real entry here.\n"
            '      (define-key map "G" \'dired-goto-last-entry)\n'
        ),
        "new": ('      (define-key map "k" \'previous-line)\n'),
        "test_target": "dired_tests",
        "test": "dired_capital_g_goes_to_last_line",
    },
    {
        "label": "G2 compilation's G binding removed",
        "file": "crates/core/lisp/compile.el",
        "old": (
            '          (define-key map "k" \'previous-line)\n'
            '          (define-key map "G" \'end-of-buffer)\n'
        ),
        "new": ('          (define-key map "k" \'previous-line)\n'),
        "test_target": "compile_tests",
        "test": "compilation_j_and_k_move_without_inserting",
    },
    {
        "label": "G3 search-mode's (read-only view) G binding removed",
        "file": "crates/core/lisp/search.el",
        "old": (
            '    (define-key map "k" \'previous-line)\n'
            "    ;; M130 fix round FIX-3: `search-goto-last-result', not `end-of-\n"
            "    ;; buffer' -- see that function's own doc comment for why a plain\n"
            "    ;; end-of-buffer lands one line PAST the last content line here.\n"
            '    (define-key map "G" \'search-goto-last-result)\n'
        ),
        "new": ('    (define-key map "k" \'previous-line)\n'),
        "test_target": "search_tests",
        "test": "search_view_j_and_k_move_by_line",
    },
    {
        "label": "G4 help-mode's G binding removed",
        "file": "crates/core/lisp/simple.el",
        "old": (
            '        (define-key map "k" \'previous-line)\n'
            '        (define-key map "G" \'end-of-buffer)\n'
        ),
        "new": ('        (define-key map "k" \'previous-line)\n'),
        "test_target": "help_tests",
        "test": "help_j_and_k_move_by_line",
    },
    {
        "label": "G5 shell-command output buffer's G binding removed",
        "file": "crates/core/lisp/shell-command.el",
        "old": (
            '          (define-key map "k" \'previous-line)\n'
            '          (define-key map "G" \'end-of-buffer)\n'
        ),
        "new": ('          (define-key map "k" \'previous-line)\n'),
        "test_target": "shell_command_tests",
        "test": "shell_command_output_j_moves_and_does_not_insert_text",
    },
]
