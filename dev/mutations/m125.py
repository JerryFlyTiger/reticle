# M125 (AUTOOUTPUT/AUTOINPUT/AUTOINOUT) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m125.py \
#         -p core --test-target verilog_auto_tests
#
# Every entry below whose label starts with "D" is one of the 15 minimum
# entries the M125 spec itself required (one per shipped feature, removing
# the feature's whole effect, not a boundary perturbation). D1/D2/D3/D4/D5/
# D6/D9/D13 were each hand-verified during implementation (file backup +
# targeted Edit + `touch` to bump mtime, NOT `dev/mutate.py` -- see the
# implementer's own report for the exact revert/reapply cycle run for each).
# D7/D8/D10/D11/D12/D15 were not separately hand-verified beyond the passing
# test suite itself; each targets code that test 13/14/6&7/8/12/16
# respectively directly and unconditionally gates, so the same class of
# confidence applies, but the main conversation should still watch their
# actual PASS/FAIL transition when this list runs, same as any other entry.
#
# D14 is the one entry that needs a longer note. The spec asked for "the
# stale-end generalisation reverted to AUTOWIRE-only -> kills test 22." Two
# things were found while building this list that change that:
#
#   1. Test 22 as written (a SINGLE hand-corrupted AUTOOUTPUT block, no
#      other marker kind nearby) never reaches the code path the
#      generalisation touches at all -- there is nothing else in the buffer
#      for the "blocked" branch to even consider, so D14 against test 22
#      does not apply as stated.
#   2. A NEW test was written specifically to exercise the intended shape
#      (`delete_auto_adjacent_port_blocks_with_hand_deleted_end_still_
#      detects_staleness' -- AUTOOUTPUT's own End line hand-deleted, an
#      INTACT AUTOINPUT block immediately after it) and the mutation was
#      hand-verified against IT instead (file backup + Edit + touch). The
#      mutation SURVIVED: reverting the blocked-check's marker-comment
#      disjunct back to literal `/*AUTOWIRE*/' does not turn this new test
#      red either.
#
#      Root cause, worked out by hand: the scan's OTHER disjunct (any
#      `one_line_comment' whose text starts "// Beginning of automatic")
#      already catches every REAL case on its own. Any marker that has
#      content worth mis-attributing MUST have inserted a matching
#      "Beginning of automatic ..." line right after itself -- and that line
#      is a `one_line_comment', not the block-comment marker, so the
#      pre-existing (pre-M125) prefix check already recognises it as a
#      blocker regardless of which of the four marker KINDS precedes it. The
#      only way the block-comment-marker disjunct could matter is a marker
#      that expanded to NOTHING (bare, no candidates) sitting between a
#      corrupted site and a real one -- but a bare marker has no "Beginning
#      of automatic" line of its own to skip past in the first place, so
#      skipping over it is always harmless (the scan simply continues to
#      whatever follows).
#
#      Conclusion: `verilog-auto--any-auto-port-block-marker-p''s use
#      inside `verilog-auto--autowire-stale-end' is genuinely redundant
#      given the pre-existing "Beginning of automatic" prefix check -- kept
#      anyway as defensive/explicit code (a future change to comment
#      insertion order could conceivably matter), but its OWN mutation is
#      unobservable from `cargo test' today. Recorded honestly as
#      "expect": "survived" below, per this project's own rule that a
#      genuinely-unobservable entry must still appear in the list rather
#      than being silently omitted.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        "label": "D1 AUTOOUTPUT emits nothing at all",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        ((eq kind 'output) (and (member 'output dirs) (not (member 'input dirs)) (not (member 'inout dirs))))",
        "new": "        ((eq kind 'output) nil)",
        "test": "autooutput_declares_output_for_undriven_submodule_output",
    },
    {
        "label": "D2 AUTOINPUT emits nothing at all",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        ((eq kind 'input) (and (member 'input dirs) (not (member 'output dirs)) (not (member 'inout dirs))))",
        "new": "        ((eq kind 'input) nil)",
        "test": "autoinput_declares_input_for_unconnected_submodule_input",
    },
    {
        "label": "D3 AUTOINOUT emits nothing at all",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        ((eq kind 'inout) (and (member 'inout dirs) t))",
        "new": "        ((eq kind 'inout) nil)",
        "test": "autoinout_declares_inout_for_submodule_inout",
    },
    {
        "label": "D4 the cross-instance exclusion (O \\ (I u B)) removed for AUTOOUTPUT",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "        ((eq kind 'output) (and (member 'output dirs) (not (member 'input dirs)) (not (member 'inout dirs))))",
        "new": "        ((eq kind 'output) (and (member 'output dirs) t))",
        "test": "autooutput_skips_signal_consumed_by_another_instance",
        "note": (
            "Hand-verified during implementation: with this mutation, "
            "`mid' (output of u_p, input of u_c) wrongly becomes an "
            "AUTOOUTPUT. Also kills autoinput_skips_signal_driven_by_"
            "another_instance's own sibling assertion in spirit, but that "
            "test's own OWN mutation target is D-equivalent on the INPUT "
            "branch, not this line."
        ),
    },
    {
        # Updated in the trailing fix round: the predeclared-name push
        # site changed shape (from a bare NAME `member'/`push' to a
        # (POSITION . NAME) cons via `verilog-auto--notice-contains-p',
        # see the "Position-ordered notice lists" section) when the
        # cross-kind "first" ordering bug was fixed; this entry's own
        # "old"/"new" strings were updated to match, still removing the
        # SAME exclusion (the whole `((member name declared) ...)' COND
        # clause).
        "label": "D5 the already-declared exclusion removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "              ((member name declared)\n"
            "               (unless (verilog-auto--notice-contains-p\n"
            "                        verilog-auto--predeclared-port-names name)\n"
            "                 (push (cons (treesit-node-start comment) name)\n"
            "                       verilog-auto--predeclared-port-names))\n"
            "               nil)\n"
            "              (t (verilog-auto--port-marker-signal-passes-p name arg))))"
        ),
        "new": "              (t (verilog-auto--port-marker-signal-passes-p name arg))))",
        "test": "autooutput_skips_name_already_declared_in_module",
        "note": "Hand-verified during implementation (file backup + Edit + touch, reverted after).",
    },
    {
        "label": "D6 the ANSI-header guard removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '''(defun verilog-auto--expand-port-propagation-site (kind comment)
  "Expand one AUTOOUTPUT/AUTOINPUT/AUTOINOUT site (per KIND).
ANSI-header guard (spec section 2.2) and the malformed-argument notice
(spec section 2.10) both happen here, before candidates are ever
computed for an ANSI-guarded module (nothing would use them)."
  (let* ((keyword (nth 1 (assoc kind verilog-auto--port-kind-specs)))
         (module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl)))
    (if (verilog-auto--ansi-header-p header)''',
        "new": '''(defun verilog-auto--expand-port-propagation-site (kind comment)
  "Expand one AUTOOUTPUT/AUTOINPUT/AUTOINOUT site (per KIND).
ANSI-header guard (spec section 2.2) and the malformed-argument notice
(spec section 2.10) both happen here, before candidates are ever
computed for an ANSI-guarded module (nothing would use them)."
  (let* ((keyword (nth 1 (assoc kind verilog-auto--port-kind-specs)))
         (module-decl (verilog-auto--enclosing-of-types
                       comment '("module_declaration" "interface_declaration")))
         (header (verilog-auto--header-node module-decl)))
    (if nil''',
        "test": "autooutput_in_ansi_header_module_expands_nothing_and_warns",
        "note": "Hand-verified during implementation.",
    },
    {
        "label": "D7 the regexp argument ignored entirely",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "  (let ((filter (plist-get arg :filter)) (invert (plist-get arg :invert)))\n    (if (not filter)\n        t\n      (let ((m (and (string-match-p filter name) t)))\n        (if invert (not m) m)))))",
        "new": "  (let ((filter (plist-get arg :filter)) (invert (plist-get arg :invert)))\n    (ignore filter invert)\n    t))",
        "test": "autooutput_regexp_argument_filters_signals",
    },
    {
        "label": "D8 the ?! inversion removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '        (if (string-prefix-p "?!" raw)\n            (list :filter (substring raw 2) :invert t :malformed nil)\n          (list :filter raw :invert nil :malformed nil))))',
        "new": '        (list :filter raw :invert nil :malformed nil)))',
        "test": "autooutput_inverse_regexp_argument_filters_signals",
    },
    {
        "label": "D9 type extraction always returns nil",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '  (let ((dt (verilog-auto--find-first-of-type decl "data_type")))\n    (when dt\n      (let ((head (treesit-node-child dt 0)))\n        (and head\n             (let ((txt (treesit-node-text head)))\n               (unless (member txt verilog-auto--net-type-keywords) txt)))))))',
        "new": "  (ignore decl)\n  nil)",
        "test": "autooutput_copies_type_and_range_from_submodule_port",
        "note": "Hand-verified during implementation.",
    },
    {
        # M125 fix round (spec section 1): D10 used to target `wire'
        # alone and was recorded `expect: survived' on the grounds that a
        # `wire' port produces no `data_type' node at all, so the string
        # check was dead code. That reasoning does NOT carry over to
        # `reg' (or the rest of `verilog-auto--net-type-keywords'), which
        # DOES demonstrably produce a real `data_type' node (dump-
        # verified: `output reg [WIDTH-1:0] bin_count' parses `data_type'
        # -> `integer_vector_type' "reg", the identical shape `logic'
        # produces) -- narrowing the omission set back to `("wire")' is
        # therefore a REAL, observable regression now, no longer a
        # no-op. Re-pointed at the new test and no longer `expect:
        # survived'.
        "label": "D10 the net-type omission set narrowed back to wire-only",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '''(defconst verilog-auto--net-type-keywords
  '("wire" "reg" "tri" "triand" "trior" "tri0" "tri1" "trireg" "wand" "wor" "supply0" "supply1" "uwire")''',
        "new": '''(defconst verilog-auto--net-type-keywords
  '("wire")''',
        "test": "autooutput_omits_reg_and_net_type_keywords",
    },
    {
        "label": "D11 parameter substitution removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(range (verilog-auto--substitute-params (or (nth 2 pinfo) \"\") overrides))",
        "new": "(range (or (nth 2 pinfo) \"\"))",
        "test": "autooutput_substitutes_instance_parameters_in_range",
    },
    {
        "label": "D12 the sort removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(sorted (sort (copy-sequence filtered) #'string<)))",
        "new": "(sorted (copy-sequence filtered)))",
        "test": "auto_port_declarations_are_sorted_by_name",
    },
    {
        "label": "D13 verilog-delete-auto no longer recognises the new blocks",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "(dolist (c (append (verilog-auto--find-comments root \"/*AUTOWIRE*/\")\n                        (verilog-auto--find-port-marker-comments root \"AUTOOUTPUT\")\n                        (verilog-auto--find-port-marker-comments root \"AUTOINPUT\")\n                        (verilog-auto--find-port-marker-comments root \"AUTOINOUT\")))",
        "new": "(dolist (c (verilog-auto--find-comments root \"/*AUTOWIRE*/\"))",
        "test": "delete_auto_removes_port_blocks_and_restores_original_text",
        "note": "Hand-verified during implementation.",
    },
    {
        "label": "D14 the stale-end generalisation reverted to AUTOWIRE-only",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "           ((or (verilog-auto--any-auto-port-block-marker-p n)",
        "new": "           ((or (verilog-auto--comment-p n \"/*AUTOWIRE*/\")",
        "test": "delete_auto_adjacent_port_blocks_with_hand_deleted_end_still_detects_staleness",
        "note": (
            "Hand-verified during implementation and found to SURVIVE -- "
            "see this file's own long header comment above for the full "
            "root-cause analysis (the pre-existing 'Beginning of "
            "automatic' prefix check already covers every real case; "
            "this disjunct is defensive, not load-bearing, under the "
            "current grammar). Recorded honestly rather than omitted."
        ),
        "expect": "survived",
    },
    {
        "label": "D15 the provenance comment dropped",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '         (comment (format "// %s %s of %s%s" verb inst mod (if (> count 1) ", ..." ""))))',
        "new": '         (comment ""))',
        "test": "autooutput_provenance_comment_names_instance_and_module",
    },
    {
        # M125 fix round, spec section 3.1: the source file's own column-
        # alignment padding inside a packed_dimension (verible's own
        # doing, e.g. demo/rtl/mem/sram_wrapper.sv's `[         AddrWidth-
        # 1:0]') used to be copied verbatim into a generated declaration
        # in a DIFFERENT file. Shared by AUTOINST/AUTOWIRE/AUTOOUTPUT/
        # AUTOINPUT/AUTOINOUT alike (all read range text through this one
        # function), so reverting the normalization call is a real,
        # single-line regression across every one of them.
        "label": "E1 range-text whitespace normalization removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "  (let ((pdim (verilog-auto--find-first-of-type node \"packed_dimension\")))\n    (and pdim (verilog-auto--normalize-range-whitespace (treesit-node-text pdim)))))",
        "new": "  (let ((pdim (verilog-auto--find-first-of-type node \"packed_dimension\")))\n    (and pdim (treesit-node-text pdim))))",
        "test": "auto_port_declarations_against_demo_rtl_sram_wrapper",
        "note": (
            "This test uses demo/rtl/mem/sram_wrapper.sv (real, verible-"
            "aligned material) specifically because it is the one fixture "
            "in this suite whose source actually carries alignment "
            "padding -- a hand-typed fixture would not observe this "
            "mutation at all."
        ),
    },
    {
        # M125 trailing fix round: a cold review (independent, out-of-
        # tree scratch cargo project) found E2's original shape wrong.
        # `push'/`nreverse' ordering only ever fixed "first" WITHIN one
        # `verilog-auto--expand-all-port-propagation' call (one KIND); it
        # said nothing about ordering ACROSS the three separate calls
        # ('output/'input/'inout), whose relative order is a fixed call
        # sequence in `verilog-auto', not buffer position -- a module
        # using AUTOOUTPUT earlier on screen could still lose "first" to
        # a module using AUTOINPUT later on, purely because the
        # AUTOINPUT pass runs after the AUTOOUTPUT pass. Replaced with
        # position tracking: each of the three lists is now a list of
        # (MARKER-COMMENT-START-POSITION . TEXT) conses, and
        # `verilog-auto--notice-first' picks the entry with the SMALLEST
        # position, independent of push order, traversal direction, or
        # which of the three passes contributed it. This mutation
        # reverts `--notice-first' to the OLD "first thing pushed" shape
        # (`(cdr (car list))', same as the pre-position-tracking `(car
        # ...)' read did) -- observable ONLY across-kind, which is why
        # the same-KIND test (`auto_port_notices_name_the_earlier_
        # modules_signal_not_the_later') is NOT this entry's target: it
        # was hand-verified to SURVIVE this exact mutation (both modules
        # there use the same kind, output-pass, whose own internal
        # rightmost-first push order happens to already produce the
        # right answer via `(car list)' too) -- only the CROSS-kind test
        # below observes the difference.
        "label": "E2 notice-first reverted to first-pushed instead of smallest-position",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": (
            "  (let (best-pos best-text)\n"
            "    (dolist (e list best-text)\n"
            "      (when (or (null best-pos) (< (car e) best-pos))\n"
            "        (setq best-pos (car e) best-text (cdr e))))))"
        ),
        "new": "  (cdr (car list)))",
        "test": "auto_port_notices_name_the_earlier_modules_signal_across_different_kinds",
        "note": "Hand-verified during implementation (file backup + Edit + touch, reverted after).",
    },
    {
        # Trailing fix round item 2: `trireg' was missing from IEEE
        # 1800-2017's 12-member `net_type' list. Added for LRM
        # completeness, NOT because it changes observable behavior --
        # hand-verified (real tree dump, `output trireg [3:0] foo') that
        # `trireg' parses under `net_port_type'/`net_type', the identical
        # shape `wire' does, never `data_type' -- so it was already,
        # structurally, never given a chance to reach the `member' check
        # this list gates, with or without being listed. This entry is
        # declared "survived" BY CONSTRUCTION, not discovered to survive
        # after the fact: removing `trireg' from the list cannot be
        # observed by any test, because no code path this project can
        # drive ever asks the list about `trireg' in the first place (see
        # `verilog-auto--net-type-keywords''s own doc string for the
        # full membership audit). What DOES see it, if this grammar's own
        # `trireg' shape ever changes to route through `data_type': a
        # future dump probe, not `cargo test' today.
        "label": "C1 trireg dropped from the net-type keyword list (no observable effect by construction)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '''  '("wire" "reg" "tri" "triand" "trior" "tri0" "tri1" "trireg" "wand" "wor" "supply0" "supply1" "uwire")''',
        "new": '''  '("wire" "reg" "tri" "triand" "trior" "tri0" "tri1" "wand" "wor" "supply0" "supply1" "uwire")''',
        "test": "autooutput_omits_reg_and_net_type_keywords",
        "note": (
            "No test names `trireg' specifically -- the pointed-at test "
            "is the closest existing coverage of this list's own "
            "observable behavior (reg/wire omission, logic survival) and "
            "is expected to stay green under this mutation, same as "
            "every other test in the suite. Declared `expect: survived' "
            "deliberately: the point of this entry is to make the "
            "no-effect claim a checked fact instead of an assertion in a "
            "comment."
        ),
        "expect": "survived",
    },
    {
        # Cold-review finding, spec section 3.3, entry A. "Load-bearing
        # twice over" (M125 spec's own words): this is how AUTOOUTPUT/
        # AUTOINPUT/AUTOINOUT avoid re-declaring a name on re-parse, AND
        # how a submodule's own TYPED non-ANSI body port (`output logic
        # rvalid_o;') gets its direction/range/type detected at all
        # (rather than silently defaulting to `input'/no-range/no-type).
        "label": "A the list_of_variable_port_identifiers branch dropped from --nonansi-port-info-full",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '               (idlist (or (verilog-auto--find-first-of-type decl "list_of_port_identifiers")\n                           (verilog-auto--find-first-of-type decl "list_of_variable_port_identifiers")))',
        "new": '               (idlist (verilog-auto--find-first-of-type decl "list_of_port_identifiers"))',
        "test": "nonansi_typed_output_reg_port_direction_and_range_detected_via_module_ports",
    },
    {
        # Cold-review finding, spec section 3.3, entry B. The other half
        # of the same "load-bearing twice over" pair: this is how
        # `verilog-auto--declared-names' recognizes a TYPED non-ANSI body
        # port as already-declared, which is what stops AUTOWIRE from
        # trying to declare a `wire' for a name AUTOOUTPUT just turned
        # into a port on the SAME re-parse.
        "label": "B the list_of_variable_port_identifiers dolist dropped from --declared-names",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '    (dolist (n (verilog-auto--find-all-of-type module-decl "list_of_variable_port_identifiers"))\n      (dolist (id (verilog-auto--find-all-of-type n "simple_identifier"))\n        (push (treesit-node-text id) acc)))\n    (dolist (n (verilog-auto--find-all-of-type module-decl "ansi_port_declaration"))',
        "new": '    (dolist (n (verilog-auto--find-all-of-type module-decl "ansi_port_declaration"))',
        "test": "autooutput_then_autowire_do_not_double_declare",
    },
]
