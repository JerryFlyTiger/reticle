# Mutation list for M109 (incremental tree-sitter parsing, B' derivation).
# Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m109.py \
#         -p core --test-target treesit_tests
#
# The whole point of this milestone's test design is that a WRONG InputEdit is
# silent: the architect measured a deliberately mis-reported deletion that
# produced identical node count, identical kinds, identical total length and
# has_error == false, with byte ranges wrong from node 72,922 onward. U1 is
# that failure mode injected directly -- under-reporting the edit -- and it
# must be caught.

PACKAGE = "core"
TEST_TARGET = "treesit_tests"

MUTATIONS = [
    {
        "label": "U1 the edit is under-reported (the silent-corruption case)",
        "file": "crates/core/src/treesit.rs",
        "old": "    let start_byte = prefix;",
        "new": "    let start_byte = (prefix + 3).min(old_b.len()).min(new_b.len());",
        "test": "incremental_reparse_survives_a_random_edit_sequence",
    },
    {
        "label": "U2 new_end_byte reuses old_end_byte",
        "file": "crates/core/src/treesit.rs",
        "old": "    let new_end_byte = new_b.len() - suffix;",
        "new": "    let new_end_byte = old_b.len() - suffix;",
        "test": "incremental_reparse_matches_fresh_parse_across_an_edit_script",
    },
    {
        "label": "U3 the prefix no longer retreats to a char boundary",
        "file": "crates/core/src/treesit.rs",
        "old": "    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {\n        prefix -= 1;\n    }",
        "new": "    while false {\n        prefix -= 1;\n    }",
        "test": "derive_edit_satisfies_its_reconstruction_invariant_on_fixed_cases",
    },
    {
        "label": "U4 derive_edit always reports 'no change' (stale tree for edited text)",
        "file": "crates/core/src/treesit.rs",
        "old": "    if old == new {\n        return None;\n    }",
        "new": "    if true {\n        return None;\n    }",
        "test": "edited_buffer_reparse_returns_a_different_tree",
    },
    {
        "label": "U5 the cached tree is reused across a language change",
        "file": "crates/core/src/treesit.rs",
        "old": "        Some((_, cached_lang, old_data)) if *cached_lang == parser.lang => {",
        "new": "        Some((_, _cached_lang, old_data)) => {",
        "test": "different_languages_on_the_same_buffer_do_not_share_a_cached_tree",
    },
    {
        # Declared as an expected survivor, so a clean run exits 0. Inverting
        # this check only forces the slower full-parse path, which still
        # produces a correct tree -- no correctness test can see it. Without
        # `expect`, every run of this list would print a `!!` line and exit 1
        # for an outcome that is entirely by design, which is exactly how an
        # operator learns to ignore `!!` lines.
        "expect": "survived",
        "label": "U6 the span fallback is inverted (expected to SURVIVE by construction)",
        "file": "crates/core/src/treesit.rs",
        "old": "    if new_tree.root_node().end_byte() != new_text.len() {",
        "new": "    if new_tree.root_node().end_byte() == new_text.len() {",
        "test": "incremental_reparse_matches_fresh_parse_across_an_edit_script",
    },
    {
        "label": "U7 the dispatch is forced back to full parsing (M109's own point)",
        "file": "crates/core/src/treesit.rs",
        "old": "        Some((_, cached_lang, old_data)) if *cached_lang == parser.lang => {",
        "new": "        Some((_, cached_lang, old_data)) if *cached_lang == parser.lang && false => {",
        "test": "an_edit_that_cancels_out_reuses_the_tree_without_reparsing",
        "package": "core",
        "test_target": "indent_perf_tests",
    },
]
