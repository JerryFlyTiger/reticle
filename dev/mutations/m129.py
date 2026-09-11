# M129 (a text-based fallback scanner for `verilog-delete-auto') mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m129.py \
#         -p core --test-target verilog_auto_tests
#
# M129 ships ONE user-visible effect (a tree-unreachable /*AUTOINST*/ site
# can now actually be deleted) built out of several independently breakable
# mechanisms. Per this project's own per-FEATURE rule (not per-file, and not
# one entry per list), every distinct mechanism below gets its own entry that
# removes its WHOLE effect, not merely a boundary inside it:
#
#   D1  the fallback itself                     -- the milestone deleted
#   D2  string awareness (top-level scan)
#   D3  `//' awareness (top-level scan)
#   D4  `/* */' awareness (top-level scan)
#   D5  escaped-identifier awareness (top-level scan)
#   D6  the instantiation-shape safety guard    -- the data-corruption guard
#   D7  the `:lex-error' poisoning check
#   D8  trailing `// Templated' stripping for a RESCUED site
#   D9  the reworded unrecovered-marker message
#   D10 the `let' keyword (the cold review's concrete false-accept)
#   D11 string awareness inside the bracket-group sub-scan
#   D12 `\r' terminating an escaped identifier
#   D13 the once-only/gated lexical pass -- DECLARED SURVIVOR, see its comment
#   D14 escaped-identifier awareness inside the bracket-group sub-scan
#   D15 `//' awareness inside the bracket-group sub-scan
#   D16 `/* */' awareness inside the bracket-group sub-scan
#
# D7 and D9 exist because a cold review found that the ORIGINAL spec's own
# proposed mutation for each would have been INERT: the unterminated-string
# fixtures never reach the `:lex-error' cond clause (they return from the
# earlier `((not best) nil)' clause instead), and the message test asserted
# only substrings common to the old and new wording. Both claims were
# reproduced by execution before being fixed -- reverting each really did go
# unnoticed. That is the "SURVIVED because the mutation is inert, not because
# coverage is missing" trap, caught here by review rather than by the runner.
#
# D5 then hit the SAME trap inside the runner itself, on the first full
# run: it came back SURVIVED, and the two-part check showed the mutation
# had landed but was semantically inert. The test's fixture used the
# escaped identifier a real gate-level netlist actually contains, whose
# embedded parens BALANCE each other -- so removing escaped-identifier
# awareness left the depth accounting in exactly the same place and the
# test stayed green. It was rewritten around an UNBALANCED escaped
# identifier (equally legal: they run to the next whitespace and may hold
# any printable character), after which D5 kills. The lesson is one this
# project already writes down: a test can sit right beside the feature it
# names and still guard nothing -- only a mutation asks the question.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

MUTATIONS = [
    {
        # Deletion-style: the whole rescue path never produces a range, so
        # every unreachable marker falls into `unrecovered' -- exactly the
        # pre-M129 (M128-only) behaviour.
        "label": "D1 the text fallback removed entirely",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "             (r (verilog-auto--text-fallback-range fallback-lex marker-end-idx)))",
        "new": "             (r nil))",
        "test": "m129_text_fallback_deletes_a_string_adjacent_to_bracket_site",
    },
    {
        # Deletion-style: a `\"' no longer opens string state at the top
        # level, so a `)' inside a string literal counts as a real close
        # paren and the region ends early.
        "label": "D2 top-level string awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "         ((eq c ?\\\")\n          (let ((r (verilog-auto--fallback-skip-string text i n)))",
        "new": "         ((and nil (eq c ?\\\"))\n          (let ((r (verilog-auto--fallback-skip-string text i n)))",
        "test": "m129_text_fallback_ignores_a_close_paren_inside_a_string",
    },
    {
        "label": "D3 top-level line-comment awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "         ((and (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?/))\n          (setq i (verilog-auto--fallback-skip-line-comment text i n)))",
        "new": "         ((and nil (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?/))\n          (setq i (verilog-auto--fallback-skip-line-comment text i n)))",
        "test": "m129_text_fallback_ignores_parens_inside_a_line_comment",
    },
    {
        "label": "D4 top-level block-comment awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "         ((and (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?*))\n          (let ((r (verilog-auto--fallback-skip-block-comment text i n)))",
        "new": "         ((and nil (eq c ?/) (< (1+ i) n) (eq (aref text (1+ i)) ?*))\n          (let ((r (verilog-auto--fallback-skip-block-comment text i n)))",
        "test": "m129_text_fallback_ignores_parens_inside_a_block_comment",
    },
    {
        # A Verilog escaped identifier may legally contain `(', `)' and `\"'
        # -- `\\u1(0) ' is a legal name, and gate-level netlists really do
        # contain them. The test's own fixture deliberately uses an
        # UNBALANCED one instead; see this file's header for why the
        # balanced shape made this entry survive.
        "label": "D5 top-level escaped-identifier awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "         ((eq c ?\\\\)\n          (let ((j (verilog-auto--fallback-skip-escaped-identifier text i n)))",
        "new": "         ((and nil (eq c ?\\\\))\n          (let ((j (verilog-auto--fallback-skip-escaped-identifier text i n)))",
        "test": "m129_text_fallback_ignores_parens_inside_an_escaped_identifier",
    },
    {
        # THE data-corruption guard. With it always-true, a marker sitting
        # inside a module header's own port-list paren gets that port list
        # deleted.
        "label": "D6 the instantiation-shape safety guard always accepts",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "     ((not (verilog-auto--instantiation-shaped-p (cddr best))) nil)",
        "new": "     ((not t) nil)",
        "test": "m129_text_fallback_refuses_a_marker_inside_a_module_header",
    },
    {
        # An earlier lexical anomaly poisons the paren-depth stack for
        # everything lexed after it, so a pair that looks individually
        # well-formed downstream of one is not trustworthy.
        "label": "D7 the :lex-error poisoning check removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "     ((and lex-error (< lex-error (nth 1 best))) nil)",
        "new": "     ((and nil lex-error (< lex-error (nth 1 best))) nil)",
        "test": "m129_text_fallback_refuses_a_well_formed_site_after_an_earlier_lexical_anomaly",
    },
    {
        # A rescued site must lose its trailing `// Templated' exactly the
        # way a tree-reached one does. Note the expected count drops from
        # (2 0 0 1) to (1 0 0 1) as well -- element 0 counts RANGES, and a
        # templated site pushes two.
        "label": "D8 trailing // Templated stripping removed for rescued sites",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                               (+ (point-min) (cdr r) 1))))\n              (when ann-range (push ann-range ranges)))",
        "new": "                               (+ (point-min) (cdr r) 1))))\n              (when nil (push ann-range ranges)))",
        "test": "m129_text_fallback_strips_the_trailing_templated_annotation",
    },
    {
        # Reverts the message to its pre-M129 wording, which claimed the
        # site was merely "left untouched" without saying a rescue had been
        # attempted. Reproduced as unnoticed BEFORE the fix round added the
        # assertion; this entry is what keeps it noticed.
        "label": "D9 the reworded unrecovered-marker message reverted",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "; a text-based fallback scan was tried and could not recover them either (no enclosing paren pair, a lexical error, or the instantiation-shape guard refused), left untouched\"",
        "new": ", left untouched\"",
        "test": "unreachable_autoinst_verilog_delete_auto_echoes_its_own_message",
    },
    {
        # SystemVerilog's `let NAME(args) = expr;' tokenizes exactly like
        # `MODULE INSTANCE (', so `let' missing from the keyword list is a
        # false-accept in the delete-real-code direction. Found by cold
        # review; the whole reserved-word set was completed in the same
        # round.
        "label": "D10 `let' removed from the guard's keyword list",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "\"rtranif1\" \"let\" \"type\"",
        "new": "\"rtranif1\" \"type\"",
        "test": "m129_text_fallback_refuses_a_marker_inside_a_let_declaration",
    },
    {
        # The bracket-group sub-scan used to count `['/`]' over raw text
        # with no string/comment awareness at all -- two lexing disciplines
        # in one function. Cold review found it; this entry pins the fix.
        "label": "D11 bracket-group sub-scan string awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "                 ((eq cj ?\\\")\n                  (let ((r (verilog-auto--fallback-skip-string text j n)))",
        "new": "                 ((and nil (eq cj ?\\\"))\n                  (let ((r (verilog-auto--fallback-skip-string text j n)))",
        "test": "m129_text_fallback_ignores_a_close_bracket_inside_a_string_within_a_bracket_group",
    },
    {
        "label": "D12 carriage return no longer terminates an escaped identifier",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "    (while (and (< j n) (not (memq (aref text j) '(?\\s ?\\t ?\\n ?\\r))))",
        "new": "    (while (and (< j n) (not (memq (aref text j) '(?\\s ?\\t ?\\n))))",
        "test": "m129_text_fallback_escaped_identifier_ends_at_a_carriage_return",
    },
    {
        # DECLARED SURVIVOR -- expected to survive, and recorded here rather
        # than left out, per this project's own rule that leaving the entry
        # out is indistinguishable from nobody having thought about it.
        #
        # The spec required the lexical pass to run AT MOST ONCE and only
        # when an unreachable marker actually exists, so the normal path
        # pays nothing. Removing the `and unreachable' gate makes the pass
        # run on EVERY `verilog-delete-auto' call. That is a pure
        # performance property: the result is identical (the `dolist' over
        # an empty `unreachable' does nothing with it), so no assertion in
        # `cargo test' can see it, and no screenshot can either -- unlike
        # M115's S7/S8, there is no pixel that shows it. The honest record
        # is that this one is observable only by measurement, not by the
        # suite. It is a real property worth keeping: without the gate every
        # AUTO expansion in every Verilog buffer pays a full-buffer elisp
        # character scan.
        "label": "D13 the once-only gate on the lexical pass removed (declared survivor)",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": "          (and unreachable\n               (verilog-auto--lex-paren-pairs",
        "new": "          (and t\n               (verilog-auto--lex-paren-pairs",
        "test": "m129_text_fallback_deletes_a_string_adjacent_to_bracket_site",
        "expect": "survived",
    },
    {
        # Second fix round (trailing re-review): the bracket-group sub-scan
        # grew four branches mirroring the top-level scan, and only the
        # STRING one (D11) was ever tested -- the other three could be
        # deleted with no named test going red. D14-D16 close that. Each of
        # the three fixtures needed an UNBALANCED `(' inside the region that
        # gets stranded when the bracket closes early: without one, the
        # stranded leftover contains no real paren, the outer paren-matching
        # comes out identical, and the mutation lands but is inert -- the D5
        # shape, hit again three times in a row while deliberately trying to
        # avoid it.
        "label": "D14 bracket sub-scan escaped-identifier awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 ((eq cj ?\\\\)\n                  (setq j (verilog-auto--fallback-skip-escaped-identifier text j n)))',
        "new": '                 ((and nil (eq cj ?\\\\))\n                  (setq j (verilog-auto--fallback-skip-escaped-identifier text j n)))',
        "test": "m129_text_fallback_ignores_a_close_bracket_inside_an_escaped_identifier_within_a_bracket_group",
    },
    {
        "label": "D15 bracket sub-scan line-comment awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 ((and (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?/))\n                  (setq j (verilog-auto--fallback-skip-line-comment text j n)))',
        "new": '                 ((and nil (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?/))\n                  (setq j (verilog-auto--fallback-skip-line-comment text j n)))',
        "test": "m129_text_fallback_ignores_a_close_bracket_inside_a_line_comment_within_a_bracket_group",
    },
    {
        "label": "D16 bracket sub-scan block-comment awareness removed",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": '                 ((and (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?*))\n                  (let ((r (verilog-auto--fallback-skip-block-comment text j n)))',
        "new": '                 ((and nil (eq cj ?/) (< (1+ j) n) (eq (aref text (1+ j)) ?*))\n                  (let ((r (verilog-auto--fallback-skip-block-comment text j n)))',
        "test": "m129_text_fallback_ignores_a_close_bracket_inside_a_block_comment_within_a_bracket_group",
    },
]
