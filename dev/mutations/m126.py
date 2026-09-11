# M126 (AUTOREG/AUTOTIEOFF) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m126.py \
#         -p core --test-target verilog_auto_tests
#
# Every entry below is one of the 11 minimum entries the M126 spec itself
# required: one per shipped feature, removing the feature's whole effect
# (or a load-bearing dependency of it), not a boundary perturbation.
# None of these were run through `dev/mutate.py` by the implementer (that
# tool is reserved for the main conversation, per this project's own
# delegation rule) -- D2/D4/D6 below were hand-verified (file backup +
# targeted Edit + `touch` to bump mtime, reverted after) during
# implementation; the rest were not separately hand-verified beyond the
# passing test suite, but each targets code its own named test directly
# and unconditionally gates.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "D1 AUTOREG emits nothing at all",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "                (unless (or (verilog-auto--decl-has-type-keyword-p decl)\n"
            "                            (member nm body-declared) (member nm driven))\n"
            "                  (push (cons nm decl) candidates))))"
        ),
        "new": (
            "                (unless (or t (verilog-auto--decl-has-type-keyword-p decl)\n"
            "                            (member nm body-declared) (member nm driven))\n"
            "                  (push (cons nm decl) candidates))))"
        ),
        "test": "autoreg_basic_nonansi_emission_with_and_without_range",
    },
    {
        "label": "D2 AUTOTIEOFF emits nothing at all",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "             (t (push (cons nm decl) candidates)))))",
        "new": "             (t nil))))",
        "test": "autotieoff_numeric_constant_table",
        "note": "Hand-verified during implementation (file backup + Edit + touch, reverted after).",
    },
    {
        "label": "D3 the ANSI assign switch removed (divergence 2)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "                (decl-kw (cond (ansi \"assign\")\n"
            "                                ((equal verilog-auto-tieoff-declaration \"assign\") \"assign\")\n"
            "                                (t \"wire\"))))"
        ),
        "new": (
            "                (decl-kw (cond ((equal verilog-auto-tieoff-declaration \"assign\") \"assign\")\n"
            "                                (t \"wire\"))))"
        ),
        "test": "autotieoff_ansi_header_emits_assign_form_and_records_switch",
    },
    {
        "label": "D4 the port-reg skip removed (divergence 3)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '  (equal (verilog-auto--decl-raw-type-keyword decl) "reg"))',
        "new": "  (ignore decl)\n  nil)",
        "test": "autotieoff_port_reg_skipped_with_name_recorded",
        "note": "Hand-verified during implementation (file backup + Edit + touch, reverted after).",
    },
    {
        # Fix round coverage gap: AUTOTIEOFF's own candidate filter is
        # deliberately ASYMMETRIC with AUTOREG's (it only excludes a
        # port-`reg', never a port-`logic'/`wire' the way AUTOREG's own
        # per-port type check does). This entry inserts AUTOREG's rule
        # into AUTOTIEOFF's own cond, which would wrongly skip `a'
        # (`logic'-typed) too -- the new coverage test's own FIRST
        # assertion (about `a', not `b') is what this kills. `b's
        # assertion is never reached, since `assert!' panics on the
        # first failure.
        "label": "D13 AUTOTIEOFF wrongly adopts AUTOREG's own per-port type-keyword skip",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "            (cond\n"
            "             ((member nm body-declared) nil)\n"
            "             ((member nm driven) nil)\n"
            "             (port-reg"
        ),
        "new": (
            "            (cond\n"
            "             ((member nm body-declared) nil)\n"
            "             ((member nm driven) nil)\n"
            "             ((verilog-auto--decl-has-type-keyword-p decl) nil)\n"
            "             (port-reg"
        ),
        "test": "autotieoff_typed_ports_other_than_reg_are_still_tied_off",
    },
    {
        "label": "D5 the continuous-assign driver detection removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let (acc)\n"
            "    (dolist (ca (verilog-auto--find-all-of-type module-decl \"continuous_assign\"))\n"
            "      (dolist (na (verilog-auto--find-all-of-type ca \"net_assignment\"))\n"
            "        (dolist (nm (verilog-auto--net-lvalue-driven-names (treesit-node-child na 0)))\n"
            "          (push nm acc))))\n"
            "    (nreverse acc)))"
        ),
        "new": "  (ignore module-decl)\n  nil)",
        "test": "autotieoff_continuous_assign_driven_skipped",
    },
    {
        # Fix round coverage gap: every existing continuous-assign test
        # only used a bare identifier LHS (`assign a = ...;'), which
        # never exercises the nested-net_lvalue recursion this function
        # exists for at all (the non-nested base case, `find-first-of-
        # type lvalue "simple_identifier"', already answers a bare
        # identifier correctly on its own). This mutation disables ONLY
        # the recursion, falling straight to the base case for every
        # LVALUE -- for a concatenation `{a, b}' this still happens to
        # find `a' (the first `simple_identifier' anywhere in the
        # subtree, `find-first-of-type' does not stop at intervening
        # node types), so the new concatenation test's `a' assertion
        # alone would NOT catch this; its `b' assertion is what does.
        "label": "D14 the concatenation net_lvalue recursion removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let ((nested (verilog-auto--find-all-of-type lvalue \"net_lvalue\")))\n"
            "    (if nested\n"
            "        (apply #'append (mapcar #'verilog-auto--net-lvalue-driven-names nested))\n"
            "      (let ((id (verilog-auto--find-first-of-type lvalue \"simple_identifier\")))\n"
            "        (and id (list (treesit-node-text id)))))))"
        ),
        "new": (
            "  (let ((nested (verilog-auto--find-all-of-type lvalue \"net_lvalue\")))\n"
            "    (if nil\n"
            "        (apply #'append (mapcar #'verilog-auto--net-lvalue-driven-names nested))\n"
            "      (let ((id (verilog-auto--find-first-of-type lvalue \"simple_identifier\")))\n"
            "        (and id (list (treesit-node-text id)))))))"
        ),
        "test": "autoreg_concatenation_lvalue_drives_every_element",
    },
    {
        "label": "D6 the body-declaration skip removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let (acc)\n"
            "    (dolist (n (verilog-auto--find-all-of-type module-decl \"net_decl_assignment\"))\n"
            "      (push (treesit-node-text (treesit-node-child n 0)) acc))\n"
            "    (dolist (n (verilog-auto--find-all-of-type module-decl \"variable_decl_assignment\"))\n"
            "      (push (treesit-node-text (treesit-node-child-by-field-name n \"name\")) acc))\n"
            "    (nreverse acc)))\n"
            "\n"
            "(defun verilog-auto--net-lvalue-driven-names (lvalue)"
        ),
        "new": (
            "  (ignore module-decl)\n"
            "  nil)\n"
            "\n"
            "(defun verilog-auto--net-lvalue-driven-names (lvalue)"
        ),
        "test": "autoreg_body_reg_skipped",
        "note": (
            "Hand-verified during implementation (file backup + Edit + touch, reverted after). "
            "The `old' string is anchored on the FULL "
            "`verilog-auto--body-declared-names' body plus the following "
            "function's own header so it occurs exactly once -- the "
            "first two `dolist' lines alone are not unique in this file "
            "(`verilog-auto--declared-names' has near-identical ones)."
        ),
    },
    {
        "label": "D7 the [WIDTH-1:0] special case removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (if (and (string= lsb \"0\")\n"
            "           (string-match \"\\\\`\\\\([A-Za-z_$][A-Za-z0-9_$]*\\\\)[ \\t]*-[ \\t]*1\\\\'\" msb))\n"
            "      (format \"{%s{1'b0}}\" (match-string 1 msb))\n"
            "    (format \"{(1+(%s)%s){1'b0}}\" msb"
        ),
        "new": ("  (if nil\n" "      (format \"{%s{1'b0}}\" (match-string 1 msb))\n" "    (format \"{(1+(%s)%s){1'b0}}\" msb"),
        "test": "autotieoff_symbolic_forms_including_special_case_boundary",
    },
    {
        # Fix round: a cold review, reading GNU's own regex directly
        # (verilog-mode.el:11427), found the special case's own MSB
        # regex used to require `-1' with no whitespace at all -- GNU
        # tolerates whitespace around every token. This entry targets
        # ONLY that tolerance (not the special case's own existence,
        # already D7's job): reverting to the no-whitespace regex must
        # still let plain `[WIDTH-1:0]' (port `a') take the special
        # case, and must make `[WIDTH - 1:0]' (port `g') fall through to
        # the general form instead.
        #
        # Port `h' (`[ WIDTH-1 : 0 ]') does NOT distinguish the two
        # regexes and carries no marginal coverage here, contrary to
        # what this entry originally claimed -- found by the trailing
        # cold review, which reproduced the real pipeline under
        # `emacs -Q --batch'. `verilog-auto--normalize-range-whitespace'
        # plus `verilog-auto--range-bounds's own `string-trim' across
        # the `:' reduce `h's MSB to "WIDTH-1", with no internal
        # whitespace left, BEFORE either regex sees it -- so both the
        # old and the new regex match it identically. Only `g', whose
        # MSB genuinely arrives as "WIDTH - 1", separates them. The test
        # still goes red under this mutation, on `g's assertion.
        # `verilog-auto--symbolic-tieoff-body's own docstring had this
        # right from the start; this comment and the test's did not.
        "label": "D12 whitespace tolerance removed from the [WIDTH-1:0] special case's own MSB regex",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '"\\\\`\\\\([A-Za-z_$][A-Za-z0-9_$]*\\\\)[ \\t]*-[ \\t]*1\\\\\'" msb))',
        "new": '"\\\\`\\\\([A-Za-z_$][A-Za-z0-9_$]*\\\\)-1\\\\\'" msb))',
        "test": "autotieoff_symbolic_forms_including_special_case_boundary",
    },
    {
        "label": "D8 multi-dimensional handling reverted to GNU's own last-dimension-only bug",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        (let ((width (apply #'* (mapcar (lambda (b) (verilog-auto--dimension-width (car b) (cdr b))) bounds))))",
        "new": "        (let* ((last-b (car (last bounds))) (width (verilog-auto--dimension-width (car last-b) (cdr last-b))))",
        "test": "autotieoff_multidim_all_numeric_uses_product",
    },
    {
        "label": "D9 the argument warning removed (divergence 4, AUTOREG)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '        (if (not (string= text "/*AUTOREG*/"))',
        "new": "        (if nil",
        "test": "autoreg_regexp_argument_nothing_and_warning",
    },
    {
        "label": "D10 execution order reverted -- AUTOREG runs before AUTOTIEOFF",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "      (setq n-tieoff (verilog-auto--expand-all-autotieoff))\n"
            "      (setq n-wire (verilog-auto--expand-all-autowire))\n"
            "      (setq n-reg (verilog-auto--expand-all-autoreg))"
        ),
        "new": (
            "      (setq n-reg (verilog-auto--expand-all-autoreg))\n"
            "      (setq n-wire (verilog-auto--expand-all-autowire))\n"
            "      (setq n-tieoff (verilog-auto--expand-all-autotieoff))"
        ),
        "test": "autotieoff_suppresses_autoreg_in_same_module",
        "note": (
            "With AUTOREG running first, it declares `reg a; reg b;' "
            "(nothing has driven or declared them yet); AUTOTIEOFF's own "
            "subsequent fresh reparse then sees both names as already "
            "body-declared and emits NOTHING -- the named test's own "
            "first assertion (AUTOTIEOFF must still expand) is what this "
            "kills, the exact inverse of the intended O2/T9 dependency."
        ),
    },
    {
        "label": "D11 delete-auto no longer recognises the two new block kinds",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "    (dolist (c (append (verilog-auto--find-comments root \"/*AUTOWIRE*/\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOOUTPUT\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOINPUT\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOINOUT\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOTIEOFF\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOREG\")))"
        ),
        "new": (
            "    (dolist (c (append (verilog-auto--find-comments root \"/*AUTOWIRE*/\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOOUTPUT\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOINPUT\")\n"
            "                        (verilog-auto--find-port-marker-comments root \"AUTOINOUT\")))"
        ),
        "test": "delete_auto_removes_autoreg_and_autotieoff_blocks_and_restores_original_text",
        "note": (
            "Hand-verified during implementation (file backup + Edit + touch, reverted after). "
            "With this mutation, `verilog-delete-auto' no longer finds a "
            "close boundary for either new marker's own expanded block "
            "(`verilog-auto--autowire-stale-end' is never even called for "
            "them), so the two AUTOREG/AUTOTIEOFF blocks survive deletion "
            "and the named test's byte-identity assertion against the "
            "original, un-expanded source fails."
        ),
    },
]
