# M148 -- lexical-scope-aware read detection for `/*AUTOUNUSED*/'.
#
# Designed from the cold reviewer's checklist (its M1-M9), plus the fix
# round's two new effects (F1 multi-variable `for' init, F2 `net_declaration'
# in a generate block) and the F5 shape the fix round documented but did not
# pin until the main conversation added a test for it. Executed by the main
# conversation -- the implementer does not verify its own fix.
#
#     python3 dev/mutate.py --config dev/mutations/m148.py
#
# The milestone ships ONE mechanism (an ancestor walk) with EIGHT declaring
# shapes hanging off one `cond'. Per this project's per-FEATURE rule, each
# shape gets its own entry rather than relying on the single whole-effect
# deletion at the top: M117's lesson was that one deletion entry per list is
# not enough when a milestone ships several distinct effects, and a shape
# whose clause could be deleted with every test still green is exactly the
# hole that rule exists to find.
#
# One entry (S1) is a DECLARED EXPECTED SURVIVOR. It is not a coverage gap:
# the only input that could distinguish "the walk stops at `module_decl'" from
# "the walk runs past it" is a module-level declaration colliding with a port
# name, and that is illegal SystemVerilog -- measured 2026-09-20 with
# slang-server on `shadow.sv', which answers `redefinition of 'e_i'' (an
# error, not a shadow). Recording SURVIVED as an executed fact is the point;
# leaving the entry out would be indistinguishable, to anyone later reading
# only this file, from nobody having thought about the boundary at all.
#
# Note on the whole-effect entry D1: the two fail-safe tests
# (`autounused_shadow_is_per_occurrence_not_per_name' and
# `autounused_inner_scope_does_not_shadow_an_outer_read') stay GREEN under it
# BY DESIGN. They distinguish "present and correct" from "present and
# over-eager", not "present" from "absent" -- with no shadow check at all,
# their ports are never excluded and so never listed either way. D1 is bound
# to tests that do distinguish it; do not read those two as covering it.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_A = "crates/core/lisp/verilog-auto.el"


MUTATIONS = [
    # ---- D1: the whole effect, deleted -----------------------------------
    # The shadow check IS the milestone. Without this conjunct the scan is
    # back to M136's flat text-equality and every shadowing declaration
    # counts as a read of the outer port again.
    {
        "label": "D1 DELETION the shadow conjunct removed -- back to flat text equality",
        "file": _A,
        "old": "                 (not (verilog-auto--identifier-shadowed-p id name module-decl)))",
        "new": "                 t)",
        "test": "autounused_function_local_shadow_is_resolved",
    },
    {
        "label": "D1b the same deletion, seen from the unnamed-block side",
        "file": _A,
        "old": "                 (not (verilog-auto--identifier-shadowed-p id name module-decl)))",
        "new": "                 t)",
        "test": "autounused_unnamed_block_local_shadow_is_resolved",
    },
    # ---- D2-D9: one entry per declaring shape ----------------------------
    # Each neuters exactly one `cond' clause of
    # `verilog-auto--scope-declares-name-p' by pointing its node-type test at
    # a type the grammar does not have. The clause still compiles and still
    # runs; it simply never matches.
    {
        "label": "D2 the `data_declaration' clause never matches",
        "file": _A,
        "old": '           ((string= ctype "data_declaration")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_generate_block_body_declaration_shadow_is_resolved",
    },
    {
        "label": "D3 the `net_declaration' clause never matches (fix round F2)",
        "file": _A,
        "old": '           ((string= ctype "net_declaration")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_generate_block_wire_declaration_shadow_is_resolved",
    },
    # D4 was written expecting FAIL and came back SURVIVED on the first run.
    # Per this project's rule, SURVIVED is checked in two parts before it is
    # read as a coverage gap: did the mutation land, and did it change the
    # semantics. It landed (preflight: exactly one occurrence; the runner
    # rebuilt core). It did not change the answer, and the reason is that the
    # two clauses are REDUNDANT ON THE SAME ANCESTOR CHAIN, not that either
    # is dead code. For `logic a_i;' inside a function, the shadowing
    # declaration's own identifier has BOTH the `block_item_declaration' node
    # and its parent (`function_body_declaration'/`seq_block') among its
    # ancestors. Examining the parent fires the `block_item_declaration'
    # clause; examining the `block_item_declaration' itself fires the
    # `data_declaration' clause. Killing either one alone leaves the other to
    # catch it.
    #
    # Measured 2026-09-20 by hand (file backup + targeted Edit + `touch',
    # restored, `git diff --stat' unchanged): neutering ONLY
    # `data_declaration' -> all four of the function/task/named/unnamed tests
    # stay green; neutering ONLY `block_item_declaration' -> same; neutering
    # BOTH -> all four FAIL. So D4 and D4b below are the honest pair: the
    # single-clause entry is a declared survivor with a reason, and D4c is
    # the entry that actually discriminates.
    {
        "expect": "survived",
        "label": "D4 only the `block_item_declaration' clause is killed (expected to SURVIVE -- redundant with `data_declaration' on the same chain)",
        "file": _A,
        "old": '           ((string= ctype "block_item_declaration")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_task_local_shadow_is_resolved",
    },
    {
        "expect": "survived",
        "label": "D4b only the `data_declaration' clause is killed, seen from the block-local side (expected to SURVIVE -- the mirror of D4)",
        "file": _A,
        "old": '           ((string= ctype "data_declaration")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_named_block_local_shadow_is_resolved",
    },
    {
        "label": "D4c BOTH declaration clauses killed -- the block-local shape really does lose its only two paths",
        "file": _A,
        "old": '           ((string= ctype "data_declaration")\n'
               "            (when (verilog-auto--data-declaration-declares-p child name)\n"
               "              (setq found t)))",
        "new": '           ((string= ctype "no_such_node_type")\n'
               "            (when (verilog-auto--data-declaration-declares-p child name)\n"
               "              (setq found t)))\n"
               '           ((string= ctype "block_item_declaration")\n'
               "            (setq found nil))",
        "test": "autounused_task_local_shadow_is_resolved",
    },
    {
        "label": "D5 the `tf_port_list' clause never matches (function formal)",
        "file": _A,
        "old": '           ((string= ctype "tf_port_list")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_function_formal_argument_shadow_is_resolved",
    },
    {
        "label": "D5b the same clause, seen from the task-formal side",
        "file": _A,
        "old": '           ((string= ctype "tf_port_list")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_task_formal_argument_shadow_is_resolved",
    },
    {
        "label": "D6 the `for_initialization' clause never matches",
        "file": _A,
        "old": '           ((string= ctype "for_initialization")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_for_int_loop_variable_shadow_is_resolved",
    },
    {
        "label": "D7 the `genvar_initialization' clause never matches",
        "file": _A,
        "old": '           ((string= ctype "genvar_initialization")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_generate_for_genvar_shadow_is_resolved",
    },
    {
        "label": "D8 the `genvar_declaration' clause never matches",
        "file": _A,
        "old": '           ((string= ctype "genvar_declaration")',
        "new": '           ((string= ctype "no_such_node_type")',
        "test": "autounused_generate_block_body_bare_genvar_declaration_shadow_is_resolved",
    },
    # ---- D9: the fix round's F1, put back --------------------------------
    # The cold read reproduced this one by running it: with `find-first', a
    # `for (int i = 0, int j_i = 0; ...)' header has its SECOND loop variable
    # ignored, and the dead port it shadows is silently left off the list.
    # This entry is the regression guard for that exact revert.
    {
        "label": "D9 the multi-variable `for' init regresses to first-match-only (fix round F1)",
        "file": _A,
        # CAVEAT on this entry's `new' text, found by the second trailing
        # cold read: `(list (car L))' is only equivalent to "keep the first"
        # while L is non-empty. `verilog-auto--find-all-of-type' legitimately
        # returns nil here for a DIFFERENT real shape -- a procedural loop
        # reusing an already-declared variable, `for (i = 0; ...)', whose
        # `for_initialization' holds a `list_of_variable_assignments' and no
        # `for_variable_declaration' at all. Under the mutation that binds
        # `fvd' to nil, and `treesit-node-child-count' signals
        # `wrong-type-argument' rather than doing nothing -- still a cargo
        # FAIL, but through the test helper's own `ERROR:' assertion instead
        # of the `block.contains' one this entry is about. The bound test's
        # single loop always has two `for_variable_declaration' children, so
        # the FAIL observed on 2026-09-20 came through the intended path.
        # If that fixture ever gains an untyped/reused loop in the same
        # scope, re-derive this entry before trusting its result.
        # Re-anchored after the trailing cold read's fix (D11) rewrote this
        # clause's body: the old multi-line anchor quoted a `let' that no
        # longer exists. Truncating the list to its first element is the
        # same regression in one line, and survives future edits to the body.
        "old": '            (dolist (fvd (verilog-auto--find-all-of-type child "for_variable_declaration"))',
        "new": '            (dolist (fvd (list (car (verilog-auto--find-all-of-type child "for_variable_declaration"))))',
        "test": "autounused_multi_variable_for_loop_shadow_is_resolved",
    },
    # ---- D11: the trailing cold read's find, put back --------------------
    # Reproduced end to end by the trailing reviewer and then by the main
    # conversation (test added first, observed RED, fix applied, observed
    # GREEN): a depth-first `find-first-of-type' for the loop variable's name
    # returns the TYPE's name when the type is user-defined, because
    # `data_type' wraps its own `simple_identifier' and it comes first. The
    # fix scans only the DIRECT children. This entry is that revert.
    {
        "label": "D11 the loop variable's name is found depth-first again -- a user-defined type's name wins",
        "file": _A,
        "old": "              (let ((fvd-n (treesit-node-child-count fvd)))\n"
               "                (dotimes (fvd-i fvd-n)\n"
               "                  (let ((c (treesit-node-child fvd fvd-i)))\n"
               '                    (when (and (string= (treesit-node-type c) "simple_identifier")\n'
               "                               (equal (treesit-node-text c) name))\n"
               "                      (setq found t)))))))",
        "new": '              (let ((id (verilog-auto--find-first-of-type fvd "simple_identifier")))\n'
               "                (when (and id (equal (treesit-node-text id) name))\n"
               "                  (setq found t)))))",
        "test": "autounused_user_typed_for_loop_variable_shadow_is_resolved",
    },
    # ---- D10: the anonymous-token trick, isolated ------------------------
    # `for (genvar k = 0; ...)' and `for (k = 0; ...)' produce the IDENTICAL
    # named-node sexp; only the anonymous `genvar' token tells them apart.
    # D7 above kills the whole clause; this one kills only the token
    # comparison, so it separates "the dispatch is wired up" from "the token
    # detection actually works".
    {
        "label": "D10 the anonymous `genvar' token comparison never matches",
        "file": _A,
        "old": '           (when (equal (treesit-node-type (treesit-node-child gi idx)) "genvar")',
        "new": '           (when (equal (treesit-node-type (treesit-node-child gi idx)) "no_such_token")',
        "test": "autounused_generate_for_genvar_shadow_is_resolved",
    },
    # ---- S1: declared expected survivor ----------------------------------
    # See the header. Removing the `module_decl' stop makes the walk continue
    # into the module node itself and above it. No legally elaborating file
    # can distinguish that, because a module-level declaration sharing a
    # port's name is `redefinition', an error. SURVIVED here is the executed
    # form of the docstring's own reasoning, not a hole in the test suite.
    {
        "expect": "survived",
        "label": "S1 the `module_decl' stop boundary removed (expected to SURVIVE)",
        "file": _A,
        "old": "      (while (and n (not (treesit-node-eq n module-decl)))",
        "new": "      (while n",
        "test": "autounused_function_local_shadow_is_resolved",
    },
]
