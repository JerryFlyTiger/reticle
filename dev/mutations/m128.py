# M128 (`@"(lisp-expr)"' evaluated AUTO_TEMPLATE tokens, Part A; and
# `verilog-delete-auto' failing loudly on an unreachable AUTOINST site,
# Part B) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m128.py \
#         -p core --test-target verilog_auto_tests
#
# Part A: five deletion-style entries (D1-D5, one per distinct effect this
# milestone's spec section 5 names) plus three boundary entries (D6-D8).
# Part B: one deletion-style entry (D9, the skipped count never surfacing at
# all) plus one boundary entry (D10, the boundary spec section 6b itself
# warns about -- counting EVERY AUTOINST site as unreachable, which the
# negative-control tests exist specifically to catch). D11-D14 were added
# in the fix round after cold review, one per `vl-*'-binding code path the
# review judged load-bearing but previously unmutated. None of these were
# run through `dev/mutate.py' by the implementer (that tool is reserved for
# the main conversation, per this project's own delegation rule) -- the main
# conversation executes this list. D1/D3/D9/D10/D11/D12/D13/D14 were each
# hand-verified during development/fix rounds (file backup + targeted Edit
# + `touch' + run the named test + revert + diff against the backup).

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        # Deletion-style: without the lisp pass, `@"..."' tokens pass
        # through the ordinary `@'/`[]' substitutions unchanged -- the
        # construct has no effect at all.
        "label": "D1 the lisp-token-evaluation pass removed entirely",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let* ((lisp-result (verilog-auto--template-eval-lisp-tokens expr inst-number vl-bindings))"
        ),
        "new": "  (let* ((lisp-result (cons expr nil))",
        "test": "lisp_token_minimal_exact_rule_end_to_end",
    },
    {
        # Deletion-style (ordering, spec section 3.1's own load-bearing
        # claim): running the plain `@' substitution BEFORE the lisp pass
        # consumes the token's own leading `@' first, so `@"..."' is never
        # recognized as a token at all -- the exact failure mode section 0
        # describes for the pre-M128 state.
        "label": "D2 the lisp pass moved to AFTER plain `@' substitution",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let* ((lisp-result (verilog-auto--template-eval-lisp-tokens expr inst-number vl-bindings))"
        ),
        "new": (
            "  (let* ((expr (replace-regexp-in-string \"@\" (verilog-auto--literal-replacement inst-number) expr))\n"
            "         (lisp-result (verilog-auto--template-eval-lisp-tokens expr inst-number vl-bindings))"
        ),
        "test": "lisp_token_minimal_exact_rule_end_to_end",
    },
    {
        # Deletion-style (spec section 3.2's own central design decision):
        # renaming just the `defvar' target leaves `vl-name' un-special,
        # so the `let'-bound `vl-name' inside `verilog-auto--template-
        # eval-lisp-tokens' becomes an ordinary LEXICAL binding under this
        # file's own `lexical-binding: t' -- invisible to `eval'd code,
        # which then signals `void-variable' and falls back.
        "label": "D3 `vl-name's `defvar' removed (renamed, so the symbol is no longer special)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(defvar vl-name nil",
        "new": "(defvar vl-name--disabled nil",
        "test": "vl_vars_name_is_the_declared_port_name",
    },
    {
        # Deletion-style: an evaluation error now propagates uncaught
        # instead of falling back -- the WHOLE `verilog-auto' call aborts,
        # exactly GNU's own (undesired) behavior this milestone diverges
        # from.
        "label": "D4 the error fallback removed (the `condition-case' handler no longer catches `error')",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "                    (error\n"
            "                     (setq failure (format \"AUTO_TEMPLATE lisp expression %S signaled %S\"\n"
            "                                            with-at err)))))))))))"
        ),
        "new": (
            "                    (m128-mutation-disabled-condition\n"
            "                     (setq failure (format \"AUTO_TEMPLATE lisp expression %S signaled %S\"\n"
            "                                            with-at err)))))))))))"
        ),
        "test": "error_signal_falls_back_with_notice",
    },
    {
        # Deletion-style: the failed-expression annotation becomes
        # indistinguishable from a successful one -- section 2.5's own
        # point 2, the whole reason the annotation branch exists.
        "label": "D5 the failed-expression annotation reduced to a plain `// Templated'",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '  (let ((suffix (if failed-p " (expression failed)" "")))',
        "new": '  (let ((suffix ""))',
        "test": "annotation_failed_expression_vs_success",
    },
    {
        "label": "D6 boundary: `nil' result not converted to the empty string",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '   ((null value) "")',
        "new": "   ((null value) nil)",
        "test": "result_conversion_nil_becomes_empty_string",
    },
    {
        "label": "D7 boundary: only the first `@\"...\"' token in an expression substituted",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                        (if text\n                            (setq out (concat out text) i (1+ close))",
        "new": "                        (if text\n                            (setq out (concat out text) i n)",
        "test": "lisp_token_two_tokens_in_one_expression",
    },
    {
        "label": "D8 boundary: the lisp-evaluated result's own `@' re-scan dropped",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            '      (let* ((with-at (replace-regexp-in-string "@" (verilog-auto--literal-replacement inst-number) lisp-text))'
        ),
        "new": "      (let* ((with-at lisp-text)",
        "test": "lisp_token_result_rescanned_for_at",
        "expect": "SURVIVED",
        "note": (
            "DECLARED SURVIVOR. Written expecting a FAIL; it SURVIVED when the main "
            "conversation ran the list on 2026-09-10, and the reason is worth stating "
            "because it is not a coverage gap. The mutation lands (the runner enforces a "
            "unique match), but it is semantically inert: the lisp pass runs BEFORE the "
            "ordinary `@' substitution over the whole replacement text, so a lisp result "
            "containing a bare `@' is substituted by that following pass whether or not "
            "this inner replacement exists. The inner call is redundant-by-construction "
            "with the pass that comes after it, and is kept as defence against a future "
            "reordering rather than because it does work today. The behaviour the named "
            "test pins is real and IS killed -- by D2, which moves the lisp pass to AFTER "
            "the `@' substitution and thereby stops the result from ever being re-scanned. "
            "So `lisp_token_result_rescanned_for_at' is not an unkillable test; it is "
            "killed by the entry that mutates the ordering, which is where the effect "
            "actually lives."
        ),
    },
    {
        # Part B, deletion-style: the unreachable-AUTOINST count is
        # computed but never surfaced through the return value -- back to
        # the pre-M128 silent `(0 . 0)'-shaped blindness the spec's own
        # section 6b exists to fix.
        "label": "D9 Part B: the unreachable-AUTOINST count never surfaced",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "      (list (length good) (length bad) (length unreachable)))))",
        "new": "      (list (length good) (length bad) 0))))",
        "test": "unreachable_autoinst_string_adjacent_to_bracket_is_reported_not_silent",
    },
    {
        # Part B, boundary (spec section 6b's own explicit warning): a fix
        # that counts EVERY `/*AUTOINST*/' marker as unreachable, without
        # actually checking whether it has a `hierarchical_instance'
        # ancestor, would also make the deletion-style test above pass --
        # only the negative-control tests (locally-recovering shapes that
        # must NOT be counted) catch this.
        "label": "D10 Part B boundary: every AUTOINST site counted as unreachable, not just the truly-unreachable ones",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '         (unreachable (verilog-auto--unreachable-autoinst-markers root)))',
        "new": '         (unreachable (verilog-auto--find-comments root "/*AUTOINST*/")))',
        "test": "unreachable_autoinst_negative_control_unbalanced_paren_recovers_locally",
    },
    {
        # Fix-round entry (cold review): the MODPORT clause dropped --
        # `vl-modport' would come back nil even for a genuine interface
        # port with a modport, e.g. `some_if.mst bus_i'.
        "label": "D11 `verilog-auto--ansi-port-dims's own MODPORT clause dropped (always nil)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "          (and mp-node (treesit-node-text mp-node)))))",
        "new": "          nil)))",
        "test": "vl_vars_modport_and_dir_on_interface_port",
    },
    {
        # Fix-round entry (cold review): `vl-mbits' widened to include the
        # LAST packed dimension too -- for a 2-D port like `[1:0][7:0]'
        # this makes `vl-mbits' equal `vl-bits' concatenated with itself
        # instead of just the outer dimension.
        "label": "D12 `vl-mbits' includes the last packed dimension too (should exclude it)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '         (mbits (if (> (length packed-dims) 1)\n                    (string-join (reverse (cdr (reverse packed-dims))) "")\n                  "")))',
        "new": '         (mbits (if (> (length packed-dims) 1)\n                    (string-join packed-dims "")\n                  "")))',
        "test": "vl_vars_2d_packed_bits_is_last_dim_mbits_is_the_rest",
    },
    {
        # Fix-round entry (cold review): `vl-dir' hardcoded to `"input"'
        # regardless of the port's own actual declared direction.
        "label": "D13 `verilog-auto--connection-text's own DIRECTION hardcoded to `input'",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "         (direction (nth 1 port))",
        "new": "         (direction 'input)",
        "test": "vl_vars_dir_input_output_inout",
    },
    {
        # Fix-round entry (cold review): `vl-width''s own numeric branch
        # off by one -- `[7:0]' (width 8) would read back as `9'.
        "label": "D14 `verilog-auto--vl-width-of's own numeric branch off-by-one",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        (number-to-string (verilog-auto--dimension-width msb lsb)))",
        "new": "        (number-to-string (1+ (verilog-auto--dimension-width msb lsb))))",
        "test": "vl_vars_width_bits_numeric_range",
    },
    {
        # Trailing-review entry: `verilog-delete-auto's OWN `(message ...)'
        # call is a non-tail side effect inside a `when' -- no test
        # asserting its RETURN VALUE (every unreachable_autoinst_* test
        # except this one) can ever pin this specific wording, so it was
        # previously unmutated. Captured via `i.output'
        # (`unreachable_autoinst_verilog_delete_auto_echoes_its_own_message').
        "label": "D15 `verilog-delete-auto's own echoed message wording reverted to the pre-fix-round text",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '        (message "verilog-delete-auto: %d /*AUTOINST*/ marker(s) have no enclosing instantiation (a parse error reclassified the site, or the marker is a stray comment not actually inside one), left untouched"',
        "new": '        (message "verilog-delete-auto: %d AUTOINST site(s) could not be located because of a parse error, left untouched"',
        "test": "unreachable_autoinst_verilog_delete_auto_echoes_its_own_message",
    },
]
