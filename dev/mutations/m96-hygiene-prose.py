# M96 (closing the hygiene detector's blind spot) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m96-hygiene-prose.py \
#         -p core --test-target lisp_hygiene_tests
#
# This file guards every `.el' edit in the project against the bug class that
# once cost seven rounds and a wrong "compiler defect" conclusion. It had a
# shape it could not see -- its own header predicted it in writing -- and that
# shape bit during M95. Two cold reads then found the first two fixes were each
# still too narrow: `nil', numbers and contractions split a run, and so did
# vectors and dotted pairs, both of which are this codebase's own docstring
# conventions (`[BEG, END)', `(STRING . POS)').
#
# The threshold was re-derived from the real corpus three times, once per
# widening, and landed on the same number each time: the longest legitimate run
# is 1, in eshell.el's `eshell-process-pending-all', word `[nil]'.

PACKAGE = "core"
TEST_TARGET = "lisp_hygiene_tests"

MUTATIONS = [
    {
        "label": "EF1 the threshold comparison is off by one (an exact-threshold run stops being flagged)",
        "file": "crates/core/tests/lisp_hygiene_tests.rs",
        "old": "const PROSE_RUN_THRESHOLD: usize = 2;",
        "new": "const PROSE_RUN_THRESHOLD: usize = 3;",
        "test": "prose_run_of_exactly_threshold_is_flagged",
    },
    {
        "label": "EF2 vectors go back to breaking a run (the `[BEG, END)' docstring convention)",
        "file": "crates/core/tests/lisp_hygiene_tests.rs",
        "old": "        Value::Vector(_) => true,",
        "new": "        Value::Vector(_) => false,",
        "test": "atomlike_prose_run_vector_split_is_flagged",
    },
    {
        "label": "EF3 a proper list stops breaking a run (the false-positive guard)",
        "file": "crates/core/tests/lisp_hygiene_tests.rs",
        "old": "            let Some(elems) = v.list_to_vec() else {",
        "new": "            let Some(elems) = None::<Vec<Value>> else {",
        "test": "proper_list_body_form_stays_a_run_breaker",
        "note": (
            "The dotted-cons widening is the riskiest half of the last fix "
            "round: treating EVERY cons as atom-like would make the detector "
            "fire on ordinary bodies, because `(+ 1 2)' is code. This "
            "mutation forces exactly that mistake."
        ),
    },
    {
        "label": "EF4 defmacro drops out of the gate",
        "file": "crates/core/tests/lisp_hygiene_tests.rs",
        # The bare conjunct appears in a comment too, so the anchor carries
        # the whole `if' line.
        "old": 'if head_name != "defun" && head_name != "defmacro" {',
        "new": 'if head_name != "defun" {',
        "test": "defmacro_prose_run_is_flagged",
    },
]
