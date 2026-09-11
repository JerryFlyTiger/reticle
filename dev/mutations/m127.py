# M127 (AUTO_TEMPLATE's substitution language: `@' numbering, `[]'/`[][]'
# bit-range tokens, `// Templated' annotation, quoted-regexp template-head
# parse fix) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m127.py \
#         -p core --test-target verilog_auto_tests
#
# Five deletion-style entries (D1-D5, one per distinct effect this milestone
# ships, per the spec's own section 5 minimum) plus four boundary entries
# (D6-D9). None of these were run through `dev/mutate.py' by the implementer
# (that tool is reserved for the main conversation, per this project's own
# delegation rule) -- the main conversation executes this list.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "D1 `@' substitution removed entirely",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            '  (let* ((with-at (replace-regexp-in-string "@" (verilog-auto--literal-replacement inst-number) expr))'
        ),
        "new": "  (let* ((with-at expr)",
        "test": "autotemplate_at_substituted_globally_not_first_only",
    },
    {
        "label": "D2 `[]' substitution removed entirely",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            r'    (replace-regexp-in-string "\\[\\]" (verilog-auto--literal-replacement last-packed) with-2)))'
        ),
        "new": "    with-2))",
        "test": "autotemplate_bracket_matrix_single_bracket_takes_the_last_packed_dimension",
    },
    {
        # Must be a SEPARATE entry from D2 -- `[]' and `[][]' are different
        # rules (spec section 5's own instruction). Removing only the
        # `[][]' step leaves a literal `[][]' token in EXPR, which the
        # SUBSEQUENT (unmutated) `[]' step then eats as two independent
        # empty brackets -- exactly the "eaten as two empty []s" failure
        # mode section 2.3 warns the real ordering avoids.
        "label": "D3 `[][]' substitution removed entirely (separate from `[]')",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            '         (with-2 (replace-regexp-in-string\n'
            r'                  "\\[\\]\\[\\]" (verilog-auto--literal-replacement bracket2) with-at)))'
        ),
        "new": "         (with-2 with-at))",
        "test": "autotemplate_bracket_matrix_double_bracket_is_a_block_comment_only_when_multidimensional",
    },
    {
        "label": "D4 `// Templated' never emitted",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '   (t "// Templated")))',
        "new": "   (t nil)))",
        "test": "autotemplate_templated_connection_carries_the_annotation_identity_connection_does_not",
    },
    {
        "label": "D5 the quoted-regexp head skip reverted to the old first-`(' scan",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                 (scan-from (if q (cdr q) after-kw))",
        "new": "                 (scan-from after-kw)",
        "test": "autotemplate_custom_instance_number_regexp_overrides_default_digit_run",
        "note": (
            "This is the exact section 3.1 parse defect: with the quoted "
            "regexp's own text never skipped, the depth-balance scan finds "
            "its opening paren inside the regexp itself and the whole rule "
            "list misparses -- the named test's own assertion is that the "
            "rule is APPLIED (not dropped), which is precisely what breaks."
        ),
    },
    {
        "label": "D6 first-digit-run changed to last-digit-run",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '    (if (string-match "[0-9]+" inst-name)',
        "new": "    (if (string-match \"[0-9]+\\\\'\" inst-name)",
        "test": "autotemplate_at_first_digit_run_u2_ch3_is_the_first_run_not_the_last",
        "note": (
            "Anchoring the digit-run search to end-of-string grabs the "
            "LAST run instead of the first for every one of this "
            "milestone's own measured instance names (they all end in "
            "digits) -- `u2_ch3' would then yield 3, not 2."
        ),
    },
    {
        "label": "D7 `[]' takes the FIRST packed dimension instead of the last",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '         (last-packed (or (car (last packed-dims)) ""))',
        "new": '         (last-packed (or (car packed-dims) ""))',
        "test": "autotemplate_bracket_matrix_single_bracket_takes_the_last_packed_dimension",
    },
    {
        "label": "D8 the `[][]'/`[]' substitution ORDER swapped",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            '  (let* ((with-at (replace-regexp-in-string "@" (verilog-auto--literal-replacement inst-number) expr))\n'
            '         (last-packed (or (car (last packed-dims)) ""))\n'
            '         (multi-dim (or (> (length packed-dims) 1) unpacked-dims))\n'
            '         (bracket2 (if multi-dim\n'
            '                       (concat "/*" (string-join packed-dims "")\n'
            '                               (if unpacked-dims (concat "." (string-join unpacked-dims "")) "")\n'
            '                               "*/")\n'
            "                     last-packed))\n"
            '         (with-2 (replace-regexp-in-string\n'
            r'                  "\\[\\]\\[\\]" (verilog-auto--literal-replacement bracket2) with-at)))' + "\n"
            r'    (replace-regexp-in-string "\\[\\]" (verilog-auto--literal-replacement last-packed) with-2)))'
        ),
        "new": (
            '  (let* ((with-at (replace-regexp-in-string "@" (verilog-auto--literal-replacement inst-number) expr))\n'
            '         (last-packed (or (car (last packed-dims)) ""))\n'
            '         (multi-dim (or (> (length packed-dims) 1) unpacked-dims))\n'
            '         (bracket2 (if multi-dim\n'
            '                       (concat "/*" (string-join packed-dims "")\n'
            '                               (if unpacked-dims (concat "." (string-join unpacked-dims "")) "")\n'
            '                               "*/")\n'
            "                     last-packed))\n"
            r'         (with-2 (replace-regexp-in-string "\\[\\]" (verilog-auto--literal-replacement last-packed) with-at)))' + "\n"
            '    (replace-regexp-in-string\n'
            r'     "\\[\\]\\[\\]" (verilog-auto--literal-replacement bracket2) with-2)))'
        ),
        "test": "autotemplate_bracket_ordering_double_bracket_not_eaten_as_two_empty_brackets",
    },
    {
        # Stands in for "annotation applied before the comma strip" --
        # this implementation structurally separates the two (the strip
        # always happens before the padding/annotation loop even runs,
        # section 3.5's own instruction), so there is no single-line
        # mutation that reorders them directly. Disabling the strip
        # itself is the closest reachable equivalent: it corrupts BOTH
        # AUTOINST's own last-line handling AND AUTOARG's (which shares
        # this same function with no annotate-fn at all), so the
        # AUTOARG-unchanged test is what actually catches it.
        "label": "D9 the trailing-comma strip disabled (stand-in for annotating before it)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "    (when lines\n"
            "      (setcar lines (substring (car lines) 0 (1- (length (car lines))))))"
        ),
        "new": "    (when nil\n      (setcar lines (substring (car lines) 0 (1- (length (car lines))))))",
        "test": "autoarg_output_byte_identical_before_m127",
    },
    {
        # Fix round item A: a cold review found no test anywhere in this
        # milestone's own diff exercised a NON-ANSI-declared port
        # carrying an unpacked dimension -- every `[]'/`[][]' unpacked-
        # array test used an ANSI header. This mutation narrows
        # `verilog-auto--nonansi-port-dims''s own `find-all-of-type' to
        # only the FIRST name in each comma-separated declaration
        # (wrapped in a list, so the shape still superficially "works"
        # for a single-name declaration) -- exactly the gap the review
        # named, confirmed hand-verified during the fix round (file
        # backup + Edit + touch, reverted after): `b' in `input [7:0] a,
        # b [0:3];' loses its own dims entry entirely, so `[][]' on `b'
        # comes back empty instead of `/*[7:0].[0:3]*/'.
        "label": "D10 non-ANSI multi-name port-dims narrowed to the first name only",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '            (dolist (id (verilog-auto--find-all-of-type idlist "simple_identifier"))',
        "new": '            (dolist (id (list (verilog-auto--find-first-of-type idlist "simple_identifier")))',
        "test": "autotemplate_nonansi_multiname_unpacked_dimension_second_name_only",
        "note": "Hand-verified during the fix round (file backup + Edit + touch, reverted after).",
    },
    {
        # Fix round item B: the `'lhs' alternative in `verilog-auto--
        # trailing-templated-annotation-range''s own regex had no test --
        # the round-trip test only ever ran in the default `nil'
        # annotation mode, so removing the `\\(?: LHS: .*\\)?' branch
        # broke nothing observable. Confirmed hand-verified during the
        # fix round (file backup + Edit + touch, reverted after): with
        # this mutation, `verilog-delete-auto' no longer recognizes the
        # `'lhs'-mode annotation as machine-generated, so it survives
        # deletion and the named test's byte-identity assertion against
        # the original source fails.
        "label": "D11 the `'lhs' alternative removed from the trailing-annotation regex",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '      (when (string-match "[ \\t]+// Templated\\\\(?: LHS: .*\\\\)?\\\\\'" tail)',
        "new": '      (when (string-match "[ \\t]+// Templated\\\\\'" tail)',
        "test": "autotemplate_lhs_mode_templated_last_line_round_trips_through_delete_auto",
        "note": "Hand-verified during the fix round (file backup + Edit + touch, reverted after).",
    },
    {
        # Trailing cold review: every non-ANSI test added for item A used
        # the UNTYPED `input [7:0] ...' shape, so `verilog-auto--nonansi-
        # port-dims''s own `or' fallback to `list_of_variable_port_
        # identifiers' (the TYPED, `logic'/`reg' shape) was never
        # actually taken by anything in this milestone's test suite -- a
        # wrong node-type string there would have gone undetected.
        # Confirmed hand-verified during this fix round (file backup +
        # Edit + touch, reverted after): with the node-type string
        # corrupted, `idlist' comes back nil for the typed declaration,
        # so BOTH `a' and `b' lose their dims entirely and `[][]' comes
        # back empty for both instead of `wa[7:0]'/`wb/*[7:0].[0:3]*/'.
        "label": "D12 the `list_of_variable_port_identifiers' node-type string corrupted",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                           (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers"))))',
        "new": '                           (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers_XXX"))))',
        "test": "autotemplate_nonansi_typed_multiname_unpacked_dimension_second_name_only",
        "note": "Hand-verified during the fix round (file backup + Edit + touch, reverted after).",
    },
]
