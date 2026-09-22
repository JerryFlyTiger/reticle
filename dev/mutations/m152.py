# Mutation list for M152 (AUTOWIRE / AUTOOUTPUT / AUTOINPUT / AUTOINOUT and
# AUTOINST connections that carry a bit/part-select). Drafted by the round-2
# cold reviewer, finalised by the round-3 reviewer, extended after round 4
# (g2). Run by the main conversation, never the implementer or a reviewer.
#
#     python3 -u dev/mutate.py --config <this file> -p core
#
# M152 overall: body /*AUTOWIRE*/ ignored AUTOINST connections carrying a
# bit/part select. Round 1 made AUTOWIRE accept them and take width from the
# connection's own select (matching GNU Emacs 30.2), with
# AUTOOUTPUT/AUTOINPUT/AUTOINOUT given the same rule, via
# `verilog-auto--range-int-bounds` + `verilog-auto--union-select-ranges`.
# Round 2 (this file's ancestor, reviewed in round 2) found the union
# flattened both bounds of every select regardless of direction, which
# silently flipped an ascending select (`[0:3]`+`[0:3]` -> `[3:0]`). Fix
# round 2 (this diff) makes the union classify each select as ascending,
# descending, or direction-less (a bit-select), and format an all-ascending
# union ascending, an all-descending (or all-bit-select) union descending
# (unchanged, GNU-matching), and refuse to merge (conflict, first-seen wins)
# when ascending and descending selects of the same name are mixed.
#
# Per-feature deletion entries (M117's rule):
#   (a) AUTOWIRE accepts select-carrying connections at all
#   (b) AUTOWIRE takes width from the connection's own select
#   (c) the union computation for AUTOWIRE
#   (d) the union computation for AUTOOUTPUT/AUTOINPUT/AUTOINOUT
#   (e) own-select width for AUTOOUTPUT/AUTOINPUT/AUTOINOUT
#   (f) AUTOWIRE's own conflict report
#   (g) "a bare connection contributes nothing to the merge"
#   (h) the ascending branch removed (every union formatted descending) --
#       NEW this round, the fix round's own headline feature
#   (i) the mixed-direction check removed (mixed now silently merges) --
#       NEW this round
#   (j) the singleton-not-conflict guard (a lone select is never pushed to
#       `verilog-auto--port-range-conflicts`) -- NEW this round
# Plus a boundary entry:
#   B1 the union function's singleton (no-merge) passthrough
#
# B2 from the round-2 draft (dropped/reworked here): it was a DECLARED
# SURVIVOR documenting the exact flatten bug this fix round repairs (swap
# `verilog-auto--range-int-bounds`'s two capture groups -- unions ALL FOUR
# raw numbers with flat max/min, coinciding with GNU only when every select
# is already descending). Re-run against the current code (byte-exact
# function text extracted from the working tree, driven via `reticle
# --script`, not retyped): the SAME swap now produces "[7:0]" instead of
# "[0:7]" for the new `autowire_merges_two_ascending_selects_ascending`
# fixture (`bus[0:3]`+`bus[4:7]`) and "[3:0]" instead of "[0:3]" for
# `autowire_merges_two_ascending_selects_identical` -- the direction split
# this round added makes both of those tests directly sensitive to which
# capture group is HIGH vs LOW, so B2 is folded into (h) below (same `old`
# anchor family, now a real kill) rather than kept as its own entry.
#
# All entries target crates/core/tests/verilog_auto_tests.rs ::
# verilog_auto_tests (PACKAGE="core", TEST_TARGET="verilog_auto_tests").
# Every `old` was checked with Python str.count against the working tree at
# review time (round-3 cold review, 2026-09-22) and occurs exactly once;
# h/i/j were additionally verified by extracting the two functions
# byte-exact from the working tree, applying the same `old`->`new` text
# substitution Python-side, and running the mutated copy standalone via
# `reticle --script` (not retyped by hand) to confirm the predicted output
# actually changes for the named test's own fixture inputs.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_F = "crates/core/lisp/verilog-auto.el"

MUTATIONS = [
    {
        "label": 'a1 AUTOWIRE stops accepting select-carrying connections (reverts to bare-only)',
        "file": _F,
        "old": """(cand (and pinfo ctext (eq (nth 1 pinfo) 'output)
                              (verilog-auto--connection-candidate-name ctext))))""",
        "new": """(cand (and pinfo ctext (eq (nth 1 pinfo) 'output)
                              (verilog-auto--bare-identifier-p ctext) ctext)))""",
        "test": 'autowire_bit_select_declares_one_bit_range',
    },
    {
        "label": "b1 AUTOWIRE ignores the connection's own select width (always falls back to the submodule's declared range)",
        "file": _F,
        "old": """(if own-select
                      (aset existing 0 (cons own-select (aref existing 0)))
                    (aset existing 1 (cons fallback (aref existing 1)))))))))))""",
        "new": """(if nil
                      (aset existing 0 (cons own-select (aref existing 0)))
                    (aset existing 1 (cons fallback (aref existing 1)))))))))))""",
        "test": 'autowire_narrower_part_select_uses_connections_own_width',
    },
    {
        "label": "c1 AUTOWIRE's own union computation is disabled (forced to always fail to merge)",
        "file": _F,
        "old": """(let ((union (verilog-auto--union-select-ranges selects)))
                       (if union""",
        "new": """(let ((union nil))
                       (if union""",
        "test": 'autowire_merges_two_partselects_of_same_signal',
    },
    {
        "label": "d1 AUTOOUTPUT/AUTOINPUT/AUTOINOUT's own union computation is disabled (forced to always fail to merge)",
        "file": _F,
        "old": """(let ((union (verilog-auto--union-select-ranges selects)))
                  (if union""",
        "new": """(let ((union nil))
                  (if union""",
        "test": 'autooutput_merges_two_partselects_of_same_signal',
    },
    {
        "label": "e1 AUTOOUTPUT/AUTOINPUT/AUTOINOUT ignores the connection's own select width",
        "file": _F,
        "old": """(if own-select
                            (aset existing 6 (cons own-select (aref existing 6)))
                          (aset existing 7 (cons fallback (aref existing 7))))))))))))))""",
        "new": """(if nil
                            (aset existing 6 (cons own-select (aref existing 6)))
                          (aset existing 7 (cons fallback (aref existing 7))))))))))))))""",
        "test": 'autoinput_merges_bitselect_then_partselect',
    },
    {
        "label": "f1 AUTOWIRE's own conflict report is silenced (conflicts computed but never pushed to the global notice list)",
        "file": _F,
        "old": """table)
        (dolist (c conflicts)
          (unless (verilog-auto--notice-contains-p verilog-auto--port-range-conflicts c)
            (push (cons pos c) verilog-auto--port-range-conflicts))))""",
        "new": """table)
        (dolist (c nil)
          (unless (verilog-auto--notice-contains-p verilog-auto--port-range-conflicts c)
            (push (cons pos c) verilog-auto--port-range-conflicts))))""",
        "test": 'autowire_reports_conflict_for_symbolic_mixed_with_numeric_select',
    },
    {
        "label": "g1 AUTOWIRE's bare connections now contribute their own (8-bit) width to the merge instead of nothing",
        "file": _F,
        "old": """(aset existing 1 (cons fallback (aref existing 1)))""",
        "new": """(aset existing 0 (cons fallback (aref existing 0)))""",
        "test": 'autowire_bare_wide_connection_contributes_nothing_to_width',
    },
    {
        # Round 4: the AUTOOUTPUT/AUTOINPUT/AUTOINOUT side of "bare contributes
        # nothing" lives in `verilog-auto--port-propagation-candidates' (slot 7 =
        # bare fallbacks, slot 6 = own selects) and had no entry of its own.
        "label": "g2 AUTOOUTPUT's bare connections now contribute their own (8-bit) width to the merge instead of nothing",
        "file": _F,
        "old": """(aset existing 7 (cons fallback (aref existing 7)))""",
        "new": """(aset existing 6 (cons fallback (aref existing 6)))""",
        "test": 'autooutput_bare_wide_connection_contributes_nothing_to_width',
    },
    {
        # Picking a mutation that CHANGES the returned text matters here: an
        # earlier draft of this entry just corrupted the passthrough to
        # `(car (cdr range-texts))` (nil, for a 1-element list), and that
        # SURVIVED -- the caller's own conflict-fallback branch
        # (`(car selects)`) happens to recompute the exact same value for a
        # singleton list, so the wire text came out identical either way.
        # See (j) below for the real, round-3-added coverage of "a solitary
        # select must not be pushed onto
        # `verilog-auto--port-range-conflicts`" -- this entry (B1) only
        # covers the TEXT the passthrough returns, not whether it's flagged.
        "label": "B1 boundary: the union function's own single-entry (no-merge-needed) passthrough returns a wrong constant instead of the lone select verbatim",
        "file": _F,
        "old": """((null (cdr range-texts)) (car range-texts))""",
        "new": """((null (cdr range-texts)) "[9:9]")""",
        "test": 'autowire_preserves_singleton_ascending_select',
    },
    {
        # NEW this round -- this IS the fix: an all-ascending (or ascending
        # + bit-select) union is now formatted ascending ([MIN:MAX])
        # instead of unconditionally descending ([MAX:MIN]) the way every
        # union was formatted before this fix round (and the way GNU's own
        # verilog-signals-combine-bus is NOT used here on purpose -- see
        # this file's own docstring on `verilog-auto--union-select-ranges`
        # for why GNU's answer, `[4:3]` for this exact input, is rejected
        # as unusable). Deleting the ascending branch (always format
        # descending) also subsumes round-2's B2: it is the same
        # underlying defect (direction information computed but not used
        # to choose the format), now caught directly by a real test instead
        # of only by hand-verification.
        "label": "h ascending branch removed from the union formatter -- every union (ascending, descending, or mixed-with-bit-select) is now formatted descending, [MAX:MIN]",
        "file": _F,
        "old": """              (if ascending
                  (format \"[%d:%d]\" lo hi)
                (format \"[%d:%d]\" hi lo))))))))))""",
        "new": """              (format \"[%d:%d]\" hi lo)))))))))""",
        "test": 'autowire_merges_two_ascending_selects_ascending',
    },
    {
        # NEW this round -- without this check, an ascending select mixed
        # with a descending select of the same name silently merges via the
        # same max/min-over-all-bounds arithmetic as the pure cases, instead
        # of being refused and reported as a width conflict.
        "label": "i mixed-direction check removed -- an ascending select mixed with a descending select of the same name now merges instead of reporting a conflict",
        "file": _F,
        "old": """          (if (and ascending descending)
              nil""",
        "new": """          (if nil
              nil""",
        "test": 'autowire_reports_conflict_for_ascending_mixed_with_descending_select',
    },
    {
        # NEW this round -- forces the union function's single-entry
        # passthrough to report "unmergeable" (nil) instead of returning the
        # lone select's own text, which makes the CALLER (AUTOWIRE's own
        # `verilog-auto--expand-autowire-site`, and
        # `verilog-auto--port-propagation-candidates`) push the name onto
        # `verilog-auto--port-range-conflicts` even though there was only
        # ever one connection of that name -- exactly the false-positive
        # `autowire_singleton_select_is_not_reported_as_a_conflict` exists to
        # catch. NOTE: this shares its `old` anchor with B1 above (same
        # line, a different replacement) -- `dev/mutate.py` applies and
        # reverts one mutation at a time, so the shared anchor does not
        # conflict at run time.
        "label": "j singleton-not-a-conflict guard removed -- a lone select of a name is now treated as unmergeable and reported as a width conflict",
        "file": _F,
        "old": """((null (cdr range-texts)) (car range-texts))""",
        "new": """((null (cdr range-texts)) nil)""",
        "test": 'autowire_singleton_select_is_not_reported_as_a_conflict',
    },
]
