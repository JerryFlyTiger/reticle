# M136 (/*AUTOUNUSED*/, deliberately divergent) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m136.py \
#         -p core --test-target verilog_auto_tests
#
# One deletion-style entry per EFFECT, per M117's rule. Effects are numbered
# E1..E10 here, not F1..F8: `F<N>' already numbers this milestone's fix-round
# items in the source comments (`M136 fix round R2' style is used there, but
# M135 learned the hard way that reusing a letter that already means something
# in the same files costs a reviewer a wrong lookup).
#
#   E1  AUTOUNUSED existing at all (D1)
#   E2  registration in the adjacent-marker predicate (D2)
#   E3  registration in `verilog-delete-auto' (D3)
#   E4  running AFTER AUTOINST, so an instance-connected input counts as read
#       (D4)
#   E5  the `inout' lvalue subtraction, i.e. driven-but-never-read is listed
#       (D5)
#   E6  `verilog-auto-unused-ignore-regexp' (D6)
#   E7  the indent rule for the idiom's continuation lines, `wire' host (D7)
#       and `logic' host (D8)
#   E8  fix round R2: refusing a marker with no legal host instead of writing
#       invalid Verilog (D9)
#   E9  fix round R3.1/R3.2: a hierarchical path component, and a connection's
#       own name field, are not reads of a same-named port (D10, D11)
#   E10 fix round R1: NOT excluding the marker's whole statement from the read
#       scan (D12)
#   E11 a marker carrying an argument is reported and skipped, not expanded (D13)
#   E12 a port's own declaration is not a read of itself (D14)
#   E13 fix round R7: the legal host must be a REDUCTION of the concatenation,
#       not merely a concatenation inside an assignment (D15)
#
# Two effects deliberately have no entry, both recorded rather than left blank:
#
#   * Idempotency. It is no longer implemented by anything of its own. The
#     fix round measured that `verilog-auto' deletes every generated region
#     before it expands, which already makes a second run byte-identical --
#     that is why R1 could delete the self-range exclusion outright rather
#     than narrow it. A mutation would have to break delete-first, which
#     breaks every block-style command at once and is therefore not
#     attributable to this milestone. `autounused_is_idempotent_with_a_non_
#     empty_list' still pins the property.
#   * Function-local shadowing (R3.3). Known limitation, not fixed: telling a
#     shadowed local from the port needs real lexical scope resolution, which
#     this file has never had. `autounused_function_local_shadow_is_a_known_
#     limitation_not_fixed' pins the CURRENT (wrong) behaviour so that the day
#     someone fixes it, the test says so out loud.
#
# Every entry is replacement-style (M83's `C-x s' prefix, M85's four-in-one-
# round: insertion-style mutations miss their target, because a later `defun'
# simply overwrites an earlier one).

MUTATIONS = [
    {
        "label": 'D1 AUTOUNUSED never expands anything',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '      (setq n-unused (verilog-auto--expand-all-autounused))',
        "new": '      (setq n-unused 0)',
        "test": 'autounused_alphabetical_order_and_exact_marker_text',
    },
    {
        "label": 'D2 AUTOUNUSED is not a known adjacent block marker (the M39 over-deletion shape)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '             (string-match-p "\\\\`/\\\\*AUTOUNUSED\\\\(?:\\\\*/\\\\|(\\\\)" text)))))',
        "new": '             nil))))',
        "test": 'autounused_bare_marker_alone_still_blocks_stale_end_detection',
        # First run of this entry SURVIVED against
        # `autounused_adjacent_autotieoff_marker_not_corrupted_on_delete', which
        # the cold read had already flagged as MEDIUM confidence: in that test
        # the marker is nested inside the `_unused_ok' concatenation with
        # AUTOTIEOFF's block above it, and the forward scan never looks back.
        # The two-part check said the mutation landed AND changed meaning, so it
        # was a real coverage gap, not a dead mutation. The fix round then found
        # the predicate IS reachable and wrote a shape that reaches it: every
        # other recognised marker removed, so a bare AUTOUNUSED marker is the
        # only thing that can halt the stale-end scan. Its own first attempt at
        # that shape did not distinguish either (AUTOREG's marker masked it) --
        # recorded because a mutation entry that quietly gets a second design is
        # indistinguishable from one that always worked.
    },
    {
        "label": 'D3 `verilog-delete-auto\' does not know about AUTOUNUSED',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '            (verilog-auto--find-port-marker-comments root "AUTOUNUSED")',
        "new": '            nil',
        "test": 'autounused_delete_auto_round_trip_returns_to_original_bytes',
    },
    {
        "label": 'D5 the `inout\' lvalue subtraction is gone (driven-only reads as used)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--all-lvalue-driven-id-nodes (module-decl)',
        "new": '(defun verilog-auto--all-lvalue-driven-id-nodes--disabled (module-decl)',
        "test": 'autounused_inout_never_mentioned_vs_driven_only_vs_read_only',
    },
    {
        "label": 'D6 `verilog-auto-unused-ignore-regexp\' is ignored',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """                       (or (null verilog-auto-unused-ignore-regexp)
                           (not (string-match-p verilog-auto-unused-ignore-regexp nm))))""",
        "new": """                       t)""",
        "test": 'autounused_ignore_regexp_suppresses_matching_names',
    },
    {
        "label": 'D9 a marker with no legal host expands anyway (writes invalid Verilog)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '(defun verilog-auto--autounused-legal-host-p (comment)',
        "new": '(defun verilog-auto--autounused-legal-host-p--disabled (comment)',
        "test": 'autounused_bare_marker_with_no_legal_host_is_refused_and_reported',
    },
    {
        "label": 'D10 a hierarchical path component counts as a read again',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 (not (verilog-auto--identifier-non-root-hierarchical-component-p id))',
        "new": '                 t',
        "test": 'autounused_hierarchical_reference_to_a_submodule_signal_is_not_counted_as_a_read',
    },
    {
        "label": 'D11 a connection\'s own name field counts as a read again',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 (not (verilog-auto--identifier-is-connection-name-field-p id)))',
        "new": '                 t)',
        "test": 'autounused_named_port_connection_name_field_is_not_a_read_of_a_same_named_candidate',
    },
    {
        "label": 'D7/D8 the idiom\'s continuation lines lose their indent rule (both hosts)',
        "file": "crates/core/lisp/indent.el",
        "old": """                          '("net_decl_assignment" "variable_decl_assignment"))))""",
        "new": """                          '("no_such_node_type"))))""",
        "test": 'verilog_status_regs_stub_sv_autounused_idiom_concatenation_lines_compute_column_2',
        "test_target": "indent_tests",
        # Two other named tests should go red with it: the `logic'-host one
        # (D8's effect) and the whole-directory sweep. Three independent tests
        # on one rule is the coverage this milestone wanted.
    },
    {
        "label": 'D8 only the `wire\' host keeps the indent rule (the `logic\' host regresses)',
        "file": "crates/core/lisp/indent.el",
        "old": """                          '("net_decl_assignment" "variable_decl_assignment"))))""",
        "new": """                          '("net_decl_assignment"))))""",
        "test": 'verilog_logic_host_autounused_idiom_concatenation_lines_compute_column_2',
        "test_target": "indent_tests",
    },
    {
        "label": 'D4 AUTOUNUSED runs BEFORE AUTOINST (instance-connected inputs look unread)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """      (setq n-inst (verilog-auto--expand-all-autoinst))""",
        "new": """      (setq n-unused (verilog-auto--expand-all-autounused))
      (setq n-inst (verilog-auto--expand-all-autoinst))""",
        "test": 'autounused_input_read_only_by_an_autoinst_expanded_connection_is_not_listed',
        # This one runs the expansion twice (once early, once at its normal
        # place). That is fine for what it measures: the early pass is the one
        # that decides, because by the time the late pass runs the marker's
        # block already exists and delete-first is long past. If it SURVIVES,
        # check that first -- a double call is a weaker mutation than a move,
        # and a move needs two edits, which this runner does not do.
    },
    {
        "label": 'D12 fix round R1 reverted: the marker\'s own statement is excluded from the read scan',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """                 (ranges (nth 2 c)))""",
        "new": """                 (ranges (let ((st (verilog-auto--enclosing-of-types
                                    comment '("continuous_assign" "net_decl_assignment"
                                              "variable_decl_assignment"))))
                           (if st
                               (cons (cons (treesit-node-start st) (treesit-node-end st))
                                     (nth 2 c))
                             (nth 2 c)))))""",
        "test": 'autounused_binary_operator_sibling_read_of_the_sink_is_not_swallowed_by_marker_statement_exclusion',
        # Re-creates, in one replacement, exactly what R1 deleted: a read that
        # happens to sit in the same statement as the sink stops counting, so a
        # signal that IS read gets listed as unused -- a falsehood written into
        # the user's RTL, which is the thing this whole milestone exists to
        # avoid.
    },
    {
        "label": 'D13 an AUTOUNUSED marker with an argument is silently expanded instead of reported',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """     ((not (string= text "/*AUTOUNUSED*/"))""",
        "new": """     (nil""",
        "test": 'autounused_takes_no_argument_reports_and_skips',
    },
    {
        "label": 'D14 a port\'s own declaration counts as a read of itself (nothing is ever listed)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """                 (ranges (nth 2 c)))""",
        "new": """                 (ranges nil))""",
        "test": 'autounused_non_ansi_body_declaration_is_not_itself_a_read',
        # The named test is the non-ANSI one, but the effect is not confined to
        # non-ANSI headers: `all-ids' is every `simple_identifier' in the module,
        # ANSI header port names included, so this mutation makes every
        # candidate look self-read. The trailing cold read pointed out that an
        # earlier wording of this label ("non-ANSI bodies list nothing") described
        # a narrower effect than the mutation actually has.
        #
        # This entry and D12 deliberately share one anchor string,
        # `(ranges (nth 2 c)))', with different replacements. That is safe only
        # because the runner is strictly serial -- apply, test, revert, touch,
        # one entry at a time -- which this project's rules already require and
        # `dev/mutate.py' already implements. Recorded so that nobody
        # parallelises the runner without noticing what it would break.
    },
    {
        "label": 'D15 the legal-host guard stops requiring a reduction operator (splices port names into a real bus pack)',
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                (let ((op (treesit-node-child-by-field-name n "operator")))\n                  (and op\n                       (equal (treesit-node-type op) "unary_operator")\n                       (member (treesit-node-text op)\n                               verilog-auto--autounused-reduction-operators)))',
        "new": '                t',
        "test": 'autounused_ordinary_bus_pack_assign_without_reduction_is_refused_and_reported',
        # The trailing cold read reproduced this by running it: before the guard
        # checked for the reduction operator, an ordinary
        # `assign bus_o = {a_i, /*AUTOUNUSED*/ b_i};' had two unrelated port
        # names spliced into it -- syntactically valid RTL whose computed value
        # had silently changed. Worse than the bug R2 was written to fix, which
        # at least produced something no tool would accept.
        #
        # A first cut of this entry appended \"\" and nil to the accepted operator
        # LIST, which changes nothing: the `and' in front still requires a
        # non-nil `unary_operator' node. Replacing the whole operator test with
        # `t' is what actually restores the pre-R7 behaviour. Recorded because a
        # mutation that cannot express the thing it names is worse than none --
        # it would have SURVIVED and been read as a coverage gap.
    },
]
