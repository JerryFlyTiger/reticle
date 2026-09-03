# M87 stage 3 (inline diagnostic rows) mutation list -- the `frontend-gui` half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-inline-diag-gui.py \
#         -p frontend-gui --test-target lib
#
# The `core` half lives in dev/mutations/m87-inline-diag.py.
#
# G2 is expected to SURVIVE and is listed anyway rather than left out:
# the F4 cap in `row_at_y` is defence in depth, not a fix for a live bug
# (both call sites already guard `row_h > 0.0`, which is what makes the
# loop terminate on its own), so no test can observe its removal without
# constructing a caller that does not exist. Recording it as a known
# survivor is the honest form; deleting the entry would hide that this
# line is unwatched.

PACKAGE = "frontend-gui"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "G1 row_height ignores row_scale (every row back to a constant height)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    row_h * grid.row_scale.get(row).copied().unwrap_or(100) as f32 / 100.0",
        "new": "    row_h",
        "test": "pixel_to_buffer_pos_lands_on_the_right_line_below_a_block_row",
        "note": (
            "The round-trip test alone would be the weaker choice: an "
            "earlier draft of it derived its own expected value from "
            "`row_top`, which is self-consistent under this mutation. The "
            "hard-coded-pixel test is the one that can see it."
        ),
    },
    {
        "label": "G2 row_at_y loses its grid.rows cap (expected SURVIVED, see header)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    for row in 0..grid.rows {",
        "new": "    for row in 0..usize::MAX {",
        "test": "row_geometry_round_trips_through_a_75_percent_row",
    },
]
