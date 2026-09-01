# M70's isearch pass. The main list and header explanation are in
# dev/mutations/m70.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m70-isearch.py -p core \
#         --test-target isearch_tests

PACKAGE = "core"
TEST_TARGET = "isearch_tests"

MUTATIONS = [
    {
        "label": "M7 isearch can no longer obtain the caret (Mut-E)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let caret = mb_caret.or_else(|| editor.isearch_live().then_some(cells.len()));",
        "new": "    let caret = mb_caret;",
        "expect_fail": ["s1_long_query_scrolls_echo_row_to_show_tail"],
        "note": "The reviewer used this entry to prove the original test was "
                "fake: it typed 90 identical 'z' characters, so an "
                "unscrolled window still ended in 20 z's and still passed. "
                "Only after the fix-up round changed the query to a "
                "non-periodic two-digit sequence does this mutation really "
                "FAIL.",
    },
]
