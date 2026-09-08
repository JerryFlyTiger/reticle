# Mutation list for M108 (memoize the tree-sitter parse within one edit
# generation). Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m108.py \
#         -p core --test-target treesit_tests
#
# C2/C3 are the ones that matter: a cache keyed on a generation counter is
# wrong the moment it serves a stale tree, and `undo` is the path most likely
# to be missed because it bypasses the insert/delete entry points entirely.

PACKAGE = "core"
TEST_TARGET = "treesit_tests"

MUTATIONS = [
    {
        "label": "C1 the cache is never consulted, so every call reparses",
        "file": "crates/core/src/treesit.rs",
        "old": "        if *cached_gen == gen && *cached_lang == parser.lang {",
        "new": "        if false {",
        "test": "unedited_buffer_reparse_returns_the_same_cached_tree",
    },
    {
        "label": "C2 the generation is ignored, so an edited buffer serves a stale tree",
        "file": "crates/core/src/treesit.rs",
        "old": "        if *cached_gen == gen && *cached_lang == parser.lang {",
        "new": "        if *cached_lang == parser.lang {",
        "test": "undo_invalidates_the_cache_and_reflects_the_undone_text",
    },
    {
        "label": "C3 the language is ignored, so one buffer's tree is served for another language",
        "file": "crates/core/src/treesit.rs",
        "old": "        if *cached_gen == gen && *cached_lang == parser.lang {",
        "new": "        if *cached_gen == gen {",
        "test": "different_languages_on_the_same_buffer_do_not_share_a_cached_tree",
    },
]
