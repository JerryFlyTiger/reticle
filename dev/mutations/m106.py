# Mutation list for M106 (the font-collection face index was ignored when
# rasterizing). Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m106.py \
#         -p frontend-gui --test-target font_tests
#
# I1 is the defect itself. I2/I3 exist because the first round's tests only
# asserted "the two faces differ", which any wrong-but-different implementation
# would satisfy -- the architect's review asked for "it is exactly that face"
# and "it actually slants" instead.

PACKAGE = "frontend-gui"
TEST_TARGET = "font_tests"

MUTATIONS = [
    {
        "label": "I1 rasterization goes back to always parsing face 0 of a collection",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "            ab_glyph::FontVec::try_from_vec_and_index(bytes.to_vec(), index)",
        "new": "            ab_glyph::FontVec::try_from_vec(bytes.to_vec())",
        "test": "shaping_face_index_selects_a_face_that_is_actually_slanted_on_screen",
    },
    {
        "label": "I2 the index is clamped, so only face 0 and face 1 are reachable",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "            ab_glyph::FontVec::try_from_vec_and_index(bytes.to_vec(), index)",
        "new": "            ab_glyph::FontVec::try_from_vec_and_index(bytes.to_vec(), index.min(0))",
        "test": "shaping_face_index_selects_exactly_the_bytes_identical_face_not_merely_a_different_one",
    },
]
