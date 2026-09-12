# M134 (AUTORESET, and the port-less module header) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m134.py \
#         -p core --test-target verilog_auto_tests
#
# One deletion-style entry per FEATURE, per M117's rule. M134's effects:
#
#   F1  AUTORESET existing at all (D1)
#   F2  the own scope falling back to the whole `always' when the marker has
#       no enclosing conditional branch (D2)
#   F3  the POSITIONAL exclusion within that scope (D3)
#   F4  alphabetical ordering (D4)
#   F5  per-marker, not per-module, expansion (D5)
#   F6  the own scope being the marker's conditional branch when it has one,
#       not the whole always block (D6)
#   F7  blocking/non-blocking operator mirroring (D7)
#   F8  LHS reduction to the base identifier (D9), the dotted-path exception
#       (D10), and an escaped identifier not vanishing (D10b)
#   F9  the unpacked-array skip and its notice (D15)
#   F10 the symbolic-multidim skip and its notice (D16) -- without it the
#       expander writes `arr <= ;' into the user's file
#   F11 `verilog-delete-auto' round-trip for a header the old stale-range
#       detector could not see (D17)
#   F12 the malformed-argument report (D18)
#   F13 Part B: the port-list-aware header predicate, at the AUTOOUTPUT
#       family's gate (D19) and at AUTOREG's (D20)
#
# `verilog-auto-reset-widths''s three modes and the signed form have named
# tests but no entry here on purpose: each is one arm of a single `cond' in
# `verilog-auto--reset-constant-for', so an entry would be "delete one cond
# arm" three times over, and D1 already removes the whole constant path.
# Recorded rather than left blank because M117's rule is that an unexplained
# omission is indistinguishable from nobody having thought about it.
#
# A first cut of this list carried a width-mode entry whose replacement only
# APPENDED an elisp comment. It landed and SURVIVED -- not a coverage gap, a
# mutation that changed no semantics. That is the two-part check for SURVIVED
# working as intended: did it land, and did it change meaning.
#
# D2, D3, D10 and D16 all come from fix rounds, and all four were defects
# that COMPILED while the milestone's own tests were green: resets written
# into a block with no reset branch (last-assignment-wins zeroes every
# signal), an exclusion that dropped reset lines GNU emits, a dotted LHS
# resetting a different signal than the one assigned, and a discarded skip
# reason emitting `arr <= ;'.
#
# Every entry is replacement-style. This project's measured lesson (M83's
# `C-x s' prefix, M85's four-in-one-round) is that insertion-style mutations
# miss their target: a same-named stub inserted ahead of the real definition
# does not shadow it, because both elisp's `defun' and Rust's `defun()'
# registration let a LATER definition overwrite an earlier one.

MUTATIONS = [
    {
        "label": 'D1 AUTORESET never expands anything',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--expand-all-autoreset ()',
        "new": '(defun verilog-auto--expand-all-autoreset--disabled ()',
        "test": 'autoreset_basic_emission_with_exact_header_and_footer_text',
    },
    {
        # The first cut of this fix was a hard gate -- refuse unless the marker
        # sits inside a conditional branch -- justified by two fixtures where
        # the marker was LAST. In that position `refuse' and the real
        # positional rule both yield zero resets, so those fixtures could not
        # distinguish them, and the gate then refused three shapes GNU
        # accepts: marker first in a bare `always' body, inside a `for' body,
        # inside a `fork'/`join'. The rule is purely positional; the scope is
        # the enclosing conditional branch when there is one and the whole
        # `always' otherwise.
        "label": 'D2 the own scope stops falling back to the always block (GNU-accepted shapes refused)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '        (let* ((own-scope (or (verilog-auto--reset-marker-own-branch comment always) always))',
        "new": '        (let* ((own-scope (verilog-auto--reset-marker-own-branch comment always))',
        "test": 'autoreset_marker_first_with_no_conditional_resets_everything_after_it',
    },
    {
        "label": 'D3 the own-branch exclusion goes back to whole-branch (position ignored)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '               (own-assigned (verilog-auto--assigned-names-before',
        "new": '               (own-assigned (verilog-auto--assigned-names-in--ignoring-cutoff',
        "test": 'autoreset_includes_signal_assigned_after_the_marker_in_its_own_branch',
    },
    {
        "label": 'D4 candidates are emitted in discovery order, not alphabetically',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '          (setq candidates (sort (copy-sequence candidates) (lambda (a b) (string< (car a) (car b)))))',
        "new": '          (setq candidates (copy-sequence candidates))',
        "test": 'autoreset_alphabetical_order_not_declaration_order',
    },
    {
        "label": 'D5 only the first AUTORESET marker per module expands (the trap the spec warned about)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '         (sorted (sort (copy-sequence comments)',
        "new": '         (sorted (sort (copy-sequence (verilog-auto--first-autowire-per-module comments))',
        "test": 'autoreset_two_markers_in_two_always_blocks_both_expand',
    },
    {
        "label": 'D6 the own branch is never computed (scope collapses to the always block)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--reset-marker-own-branch (comment always)',
        "new": '(defun verilog-auto--reset-marker-own-branch--disabled (comment always)',
        "test": 'autoreset_else_if_still_catches_sibling_branch',
    },
    {
        "label": 'D7 the assignment operator is hardcoded to non-blocking',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                         (op (if (eq style \'blocking) "=" "<=")))',
        "new": '                         (op "<="))',
        "test": 'autoreset_operator_mirrors_original_assignment_style',
    },
    {
        "label": 'D9 the LHS is not reduced to its base identifier',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--variable-lvalue-driven-names (lvalue)',
        "new": '(defun verilog-auto--variable-lvalue-driven-names--disabled (lvalue)',
        "test": 'autoreset_part_select_and_for_loop_lhs_reduced_to_base_identifier',
    },
    {
        # D9 disables the whole lvalue walker, so it cannot distinguish `base
        # name reduction broken' from `dotted branch broken'. This entry
        # targets only the dotted branch.
        "label": 'D10 a dotted LHS falls back to the first identifier again (resets the wrong signal)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '               ((> (length ids) 1) (list (treesit-node-text hier)))',
        "new": '               ((> (length ids) 99) (list (treesit-node-text hier)))',
        "test": 'autoreset_hierarchical_lvalue_resets_the_full_dotted_name',
    },
    {
        "label": 'D10b an escaped-identifier LHS goes back to contributing no name at all',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '               (t (let ((esc (verilog-auto--find-first-of-type hier "escaped_identifier")))',
        "new": '               (t (let ((esc nil))',
        "test": 'autoreset_escaped_identifier_lvalue_is_not_invisible',
    },
    {
        "label": "D15 an unpacked array is reset like a scalar again (GNU's own illegal output)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 (unpacked\n                  (unless (verilog-auto--notice-contains-p verilog-auto--autoreset-memory-skips nm)',
        "new": '                 (nil\n                  (unless (verilog-auto--notice-contains-p verilog-auto--autoreset-memory-skips nm)',
        "test": 'autoreset_unpacked_array_skipped_with_message',
    },
    {
        "label": "D16 the symbolic-multidim skip reason is discarded again (emits `arr <= ;')",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                    (if (cdr cr)',
        "new": '                    (if nil',
        "test": 'autoreset_symbolic_multidim_range_skipped_not_syntax_error',
    },
    {
        "label": "D17 the stale-range detector stops recognising AUTORESET's own header",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--begin-block-comment-p (node)',
        "new": '(defun verilog-auto--begin-block-comment-p--disabled (node)',
        "test": 'autoreset_delete_auto_round_trip_returns_to_original_bytes',
    },
    {
        "label": 'D18 a marker carrying an argument is silently accepted instead of reported',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '    (if (not (string= text "/*AUTORESET*/"))',
        "new": '    (if nil',
        "test": 'autoreset_takes_no_argument_reports_and_skips',
    },
    {
        "label": "D19 the AUTOOUTPUT family's gate goes back to the bare type test",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--ansi-header-with-ports-p (header)',
        "new": '(defun verilog-auto--ansi-header-with-ports-p--disabled (header)',
        "test": 'autooutput_on_module_without_port_list_declares_real_candidates',
    },
    {
        "label": "D20 AUTOREG's gate goes back to the bare type test",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '    (if (verilog-auto--ansi-header-with-ports-p header)\n        (progn\n          (let ((nm (verilog-auto--module-name module-decl)))\n            (unless (member nm verilog-auto--ansi-autoreg-modules)',
        "new": '    (if (verilog-auto--ansi-header-p header)\n        (progn\n          (let ((nm (verilog-auto--module-name module-decl)))\n            (unless (member nm verilog-auto--ansi-autoreg-modules)',
        "test": 'autoreg_on_module_without_port_list_does_not_misreport_ansi_header',
    },
]
