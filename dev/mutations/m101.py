# Mutation list for M101 (expand-region), designed by the reviewer in step 4
# of the milestone loop and run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m101.py \
#         -p core --test-target expand_region_tests
#
# Three of the five entries (R2/R3/R4) are the reviewer's SURVIVED
# *predictions*: it claimed the shipped tests cannot see the history-freshness
# reset, the anchor's region branch, or `region-active-p''s activity clause.
# Those deliberately carry no "test" filter, so the whole 14-test file runs
# and the prediction is tested against every test at once, not one name.

PACKAGE = "core"
TEST_TARGET = "expand_region_tests"

MUTATIONS = [
    {
        "label": "R1 line-content candidate no longer widened to include the anchor",
        "file": "crates/core/lisp/expand-region.el",
        "old": "          (cons (min (+ beg first) anchor) (max (+ beg last 1) anchor)))))))",
        "new": "          (cons (+ beg first) (+ beg last 1)))))))",
        "test": "leading_whitespace_selects_the_line_not_the_whole_module",
    },
    {
        "label": "R7 contract never deactivates when it reaches a zero-width origin",
        "file": "crates/core/lisp/expand-region.el",
        "old": "        (if (= (car prev) (cdr prev))",
        "new": "        (if nil",
        "test": "contract_reverses_the_expand_sequence_then_deactivates",
    },
    {
        "label": "R2 history is never reset, so a broken sequence keeps the stale stack",
        "file": "crates/core/lisp/expand-region.el",
        "old": "    (unless (and active expand-region--last (equal expand-region--last cur))\n      (setq-local expand-region--history nil))",
        "new": "    (unless t\n      (setq-local expand-region--history nil))",
    },
    {
        "label": "R3 anchor ignores the active region and always uses point",
        "file": "crates/core/lisp/expand-region.el",
        "old": "         (anchor (if active (region-beginning) (point))))",
        "new": "         (anchor (point)))",
    },
    {
        "label": "R4 region-active-p drops the mark_active clause",
        "file": "crates/core/src/builtins/editing.rs",
        "old": "        Ok(Value::bool(bb.mark.is_some() && bb.mark_active, i.syms.t))",
        "new": "        Ok(Value::bool(bb.mark.is_some(), i.syms.t))",
    },
]
