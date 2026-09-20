# M149 -- scope the "already declared" search to module level.
#
# Designed from the cold reviewer's checklist (its M1-M13) merged with the
# main conversation's own draft, plus the fix round's two new effects
# (the rewritten AUTOREG assertions and the new `par_block' test).
# Executed by the main conversation -- the implementer does not verify its
# own fix.
#
#     python3 dev/mutate.py --config dev/mutations/m149.py
#
# The milestone ships ONE mechanism (an ancestor walk) applied at THREE call
# sites, with SIX scope types hanging off one list. Per this project's
# per-FEATURE rule (M117), one deletion entry for the whole list is not
# enough: D1 deletes the mechanism outright, D2-D5 delete it one call site
# at a time (killing one leaves the other two working, so D1 alone would
# never notice a site that lost its filter), and S1-S6 neuter one scope type
# each.
#
# THREE entries are DECLARED EXPECTED SURVIVORS, recorded rather than
# omitted (M115 S7/S8 precedent -- leaving them out is indistinguishable,
# to anyone later reading only this file, from nobody having thought about
# them):
#
#   S5 (`tf_port_list')  -- unreachable from all three call sites. The
#       grammar (`tree-sitter-systemverilog-0.4.0/src/node-types.json', the
#       version pinned in `Cargo.lock') exposes `tf_port_item''s identifier
#       through a direct `name' field; `net_decl_assignment' and
#       `variable_decl_assignment' never occur under `tf_port_list'. All
#       three call sites search only for those two node types, so the
#       ancestor walk can never meet a `tf_port_list' ancestor from them.
#       The entry is kept as defensive coverage for a future wider caller.
#       `autowire_declares_despite_function_formal_argument_shadow' pins
#       the BEHAVIOUR but does NOT cover this entry -- it passes because a
#       formal never enters the already-declared set at all.
#   F1 (the fail-safe tail) -- `module_declaration' is a genuine ancestor of
#       every candidate at all three call sites, so the ran-out-of-parents
#       branch is structurally unreachable, not merely untested.
#   X1 (condition order inside the walk) -- order-independent by
#       construction, since `module_declaration' is never itself a member of
#       `verilog-auto--nested-scope-types'. Recorded as a checked boundary.
#
# NOTE on D4/S2/S4 and the AUTOREG tests: in the first round the two AUTOREG
# tests asserted `text.contains("reg o;")' while their own fixtures already
# contained that exact string, so they were true before `verilog_auto' ran
# and would have SURVIVED every entry below. The fix round re-anchored them
# on the emitted `// Beginning of automatic regs' block. If either AUTOREG
# test SURVIVES an entry that names it, suspect the assertion again before
# reading it as a coverage gap.

# NOTE on D5: it came back SURVIVED on the first run. Two-part check --
# it LANDED (preflight: exactly one occurrence; the runner rebuilt core,
# 77s) but did NOT change the semantics of the test it was bound to,
# because that test shadows with `reg [2:0] acc' and a `reg' reaches
# `--reset-decl-for-name' through the `variable_decl_assignment' loop
# (D5b), never through the `net_decl_assignment' loop D5 neuters. No
# AUTORESET test had a NET-kind nested shadow at all. That was a real
# coverage gap, not a redundant mutation, so
# `autoreset_uses_module_level_width_despite_generate_block_net_shadow'
# was added and D5 re-bound to it.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_A = "crates/core/lisp/verilog-auto.el"

# ---------------------------------------------------------------------
# Shared anchors. The per-loop `when' guards are NOT unique on their own
# (four identical lines across two functions), so each anchor carries
# enough trailing context to pin the function it belongs to:
#   `--declared-names'      ends its pair with a `list_of_port_identifiers' loop
#   `--body-declared-names' ends its pair with `(nreverse acc)))'
# ---------------------------------------------------------------------

_DECL_PAIR = """    (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child n 0)) acc)))
    (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))
    (dolist (n (verilog-auto--find-all-of-type module-decl "list_of_port_identifiers"))"""

_BODY_PAIR = """    (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child n 0)) acc)))
    (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))
    (nreverse acc)))"""

_NET_LOOP = """    (dolist (n (verilog-auto--find-all-of-type module-decl "net_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child n 0)) acc)))"""

_VAR_LOOP = """    (dolist (n (verilog-auto--find-all-of-type module-decl "variable_decl_assignment"))
      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))"""

_SCOPE_HEAD = """  '("generate_block" "seq_block" "par_block\""""
_SCOPE_TAIL = """    "function_body_declaration" "task_body_declaration" "tf_port_list")"""


MUTATIONS = [
    # ---- D1: the whole mechanism, deleted at the helper -------------------
    # With the nested-scope test never firing, `--module-level-node-p'
    # always returns t and all three call sites are back to their pre-M149
    # blanket search. This is the deletion entry for the milestone as a
    # whole; D2-D5 below exist because it cannot see a single site that
    # loses its filter.
    {
        "label": "D1 DELETION the nested-scope test never fires (M149 undone wholesale)",
        "file": _A,
        "old": """        (when (member (treesit-node-type n) verilog-auto--nested-scope-types)
          (throw 'done nil))""",
        "new": """        (when nil
          (throw 'done nil))""",
        "test": "autowire_declares_despite_function_local_shadow",
    },
    {
        "label": "D1b the same wholesale deletion, seen from the AUTORESET side",
        "file": _A,
        "old": """        (when (member (treesit-node-type n) verilog-auto--nested-scope-types)
          (throw 'done nil))""",
        "new": """        (when nil
          (throw 'done nil))""",
        "test": "autoreset_uses_module_level_width_despite_function_local_shadow",
    },
    # ---- D2/D3: the filter at --declared-names (AUTOWIRE, AUTOOUTPUT) -----
    {
        "label": "D2 --declared-names `net_decl_assignment' loop unfiltered again",
        "file": _A,
        "old": _DECL_PAIR,
        "new": _DECL_PAIR.replace(
            """      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child n 0)) acc)))""",
            """      (when t
        (push (treesit-node-text (treesit-node-child n 0)) acc)))""",
        ),
        "test": "autowire_declares_despite_generate_block_local_shadow",
    },
    {
        "label": "D3 --declared-names `variable_decl_assignment' loop unfiltered again",
        "file": _A,
        "old": _DECL_PAIR,
        "new": _DECL_PAIR.replace(
            """      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))""",
            """      (when t
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))""",
        ),
        "test": "autowire_declares_despite_task_local_shadow",
    },
    {
        "label": "D3b the same, seen from the AUTOOUTPUT side",
        "file": _A,
        "old": _DECL_PAIR,
        "new": _DECL_PAIR.replace(
            """      (when (verilog-auto--module-level-node-p n module-decl)
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))""",
            """      (when t
        (push (treesit-node-text (treesit-node-child-by-field-name n "name")) acc)))""",
        ),
        "test": "autooutput_propagates_despite_function_local_shadow",
    },
    # ---- D4: the filter at --body-declared-names (AUTOTIEOFF, AUTOREG) ----
    {
        "label": "D4 --body-declared-names unfiltered again (both loops)",
        "file": _A,
        "old": _BODY_PAIR,
        "new": _BODY_PAIR.replace(
            "      (when (verilog-auto--module-level-node-p n module-decl)\n",
            "      (when t\n",
        ),
        "test": "autotieoff_ties_off_despite_function_local_shadow",
    },
    {
        "label": "D4b the same, seen from the AUTOREG side (re-anchored assertion)",
        "file": _A,
        "old": _BODY_PAIR,
        "new": _BODY_PAIR.replace(
            "      (when (verilog-auto--module-level-node-p n module-decl)\n",
            "      (when t\n",
        ),
        "test": "autoreg_declares_despite_function_local_shadow",
    },
    # D4c/D4d isolate the two loops, the way D2/D3 and D5/D5b do at the
    # other two call sites. Added by the trailing cold read, which found
    # that D4/D4b neuter BOTH loops at once and so only prove the pair
    # matters together -- a bug that removed the filter from the net loop
    # alone would have been caught by nothing. That is the same shape of
    # gap D5 turned out to have; this list should not have had it at one
    # call site and not the others.
    {
        "label": "D4c --body-declared-names NET loop only (variable loop still filtered)",
        "file": _A,
        "old": _BODY_PAIR,
        "new": _BODY_PAIR.replace(
            _NET_LOOP,
            _NET_LOOP.replace(
                "      (when (verilog-auto--module-level-node-p n module-decl)\n",
                "      (when t\n",
            ),
            1,
        ),
        "test": "autotieoff_ties_off_despite_generate_block_local_shadow",
    },
    {
        "label": "D4d --body-declared-names VARIABLE loop only (net loop still filtered)",
        "file": _A,
        "old": _BODY_PAIR,
        "new": _BODY_PAIR.replace(
            _VAR_LOOP,
            _VAR_LOOP.replace(
                "      (when (verilog-auto--module-level-node-p n module-decl)\n",
                "      (when t\n",
            ),
            1,
        ),
        "test": "autotieoff_ties_off_despite_function_local_shadow",
    },
    # ---- D5: the filter at --reset-decl-for-name (AUTORESET) --------------
    {
        "label": "D5 --reset-decl-for-name `net_decl_assignment' search unfiltered",
        "file": _A,
        "old": """       (when (and (equal (treesit-node-text (treesit-node-child n 0)) name)
                  (verilog-auto--module-level-node-p n module-decl))""",
        "new": """       (when (and (equal (treesit-node-text (treesit-node-child n 0)) name)
                  t)""",
        "test": "autoreset_uses_module_level_width_despite_generate_block_net_shadow",
    },
    {
        "label": "D5b --reset-decl-for-name `variable_decl_assignment' search unfiltered",
        "file": _A,
        "old": """       (when (and (equal (treesit-node-text (treesit-node-child-by-field-name n "name")) name)
                  (verilog-auto--module-level-node-p n module-decl))""",
        "new": """       (when (and (equal (treesit-node-text (treesit-node-child-by-field-name n "name")) name)
                  t)""",
        "test": "autoreset_uses_module_level_width_despite_function_local_shadow",
    },
    # ---- S1-S6: one entry per scope type ----------------------------------
    {
        "label": "S1 `generate_block' dropped from the scope list",
        "file": _A,
        "old": _SCOPE_HEAD,
        "new": _SCOPE_HEAD.replace("generate_block", "no_such_generate_block"),
        "test": "autowire_declares_despite_generate_block_local_shadow",
    },
    {
        "label": "S1b `generate_block' dropped, seen from the begin-less `if' generate side",
        "file": _A,
        "old": _SCOPE_HEAD,
        "new": _SCOPE_HEAD.replace("generate_block", "no_such_generate_block"),
        "test": "autowire_declares_despite_beginless_if_generate_shadow",
    },
    {
        "label": "S2 `seq_block' dropped (named `begin : blk' block)",
        "file": _A,
        "old": _SCOPE_HEAD,
        "new": _SCOPE_HEAD.replace('"seq_block"', '"no_such_seq_block"'),
        "test": "autowire_declares_despite_named_block_local_shadow",
    },
    {
        "label": "S3 `par_block' dropped (`fork'/`join')",
        "file": _A,
        "old": _SCOPE_HEAD,
        "new": _SCOPE_HEAD.replace('"par_block"', '"no_such_par_block"'),
        "test": "autowire_declares_despite_fork_join_local_shadow",
    },
    {
        "label": "S4 `function_body_declaration' dropped",
        "file": _A,
        "old": _SCOPE_TAIL,
        "new": _SCOPE_TAIL.replace(
            '"function_body_declaration"', '"no_such_function_body"'
        ),
        "test": "autowire_declares_despite_function_local_shadow",
    },
    {
        "label": "S4b `function_body_declaration' dropped, seen from the AUTOTIEOFF side",
        "file": _A,
        "old": _SCOPE_TAIL,
        "new": _SCOPE_TAIL.replace(
            '"function_body_declaration"', '"no_such_function_body"'
        ),
        "test": "autotieoff_ties_off_despite_function_local_shadow",
    },
    {
        "label": "S5 `task_body_declaration' dropped",
        "file": _A,
        "old": _SCOPE_TAIL,
        "new": _SCOPE_TAIL.replace('"task_body_declaration"', '"no_such_task_body"'),
        "test": "autowire_declares_despite_task_local_shadow",
    },
    # S6 is a DECLARED EXPECTED SURVIVOR -- see the header note on `tf_port_list'.
    {
        "label": "S6 `tf_port_list' dropped -- DECLARED EXPECTED SURVIVOR (unreachable)",
        "file": _A,
        "old": _SCOPE_TAIL,
        "new": _SCOPE_TAIL.replace('"tf_port_list"', '"no_such_tf_port_list"'),
        "test": "autowire_declares_despite_function_formal_argument_shadow",
        "expect": "survived",
    },
    # ---- G1: the `generate_region' decision --------------------------------
    # The one direction in which this milestone can BREAK working code:
    # `generate wire done; endgenerate' is a module-level declaration, so
    # treating `generate_region' as a scope would make AUTOWIRE emit a
    # duplicate declaration -- a hard compile error, not a silent gap.
    {
        "label": "G1 `generate_region' wrongly ADDED to the scope list",
        "file": _A,
        "old": _SCOPE_TAIL,
        "new": _SCOPE_TAIL.replace(
            '"tf_port_list")', '"tf_port_list" "generate_region")'
        ),
        "test": "autowire_still_skips_bare_generate_region_declaration",
    },
    # ---- F1/X1: declared expected survivors --------------------------------
    {
        "label": "F1 the fail-safe tail flipped to nil -- DECLARED EXPECTED SURVIVOR",
        "file": _A,
        "old": """      ;; Ran out of parents without ever meeting MODULE-DECL -- defensive
      ;; fail-safe, fails toward "module-level" (see docstring above).
      t)))""",
        "new": """      ;; Ran out of parents without ever meeting MODULE-DECL -- defensive
      ;; fail-safe, fails toward "module-level" (see docstring above).
      nil)))""",
        "test": "autowire_declares_despite_function_local_shadow",
        "expect": "survived",
    },
    {
        "label": "X1 the two loop conditions swapped -- DECLARED EXPECTED SURVIVOR",
        "file": _A,
        "old": """        (when (treesit-node-eq n module-decl)
          (throw 'done t))
        (when (member (treesit-node-type n) verilog-auto--nested-scope-types)
          (throw 'done nil))""",
        "new": """        (when (member (treesit-node-type n) verilog-auto--nested-scope-types)
          (throw 'done nil))
        (when (treesit-node-eq n module-decl)
          (throw 'done t))""",
        "test": "autowire_declares_despite_generate_block_local_shadow",
        "expect": "survived",
    },
]
