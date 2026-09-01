# M86 (GUI visual quality) mutation list -- the `frontend-gui` half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m86-gui.py \
#         -p frontend-gui --test-target lib
#
# The `core` half lives in dev/mutations/m86.py and runs against a
# different target; the harness takes one package/target per run, so the
# two lists are separate files rather than one list with per-entry
# targets.
#
# Everything here is a pure function extracted from the paint loop
# specifically so it could be tested and mutated. `App::update` itself is
# still a black box -- it needs a live event loop, so the actual draw
# calls are covered by none of these.
#
# The repaint cadence used to be in that black box too, and this header
# used to say so. The trailing re-review then found the cadence had a
# real defect (a focused window animating forever at ~30Hz), which is
# exactly the kind of thing an untestable black box hides. The stopping
# rule was therefore pulled out into `blink_decision`, and G11/G12 below
# now cover it. What remains genuinely untestable here: the draw calls
# themselves, and `install_fonts`' filesystem search (see its own doc
# comment, which says so).
#
# All entries are replacement-style. Insertion-style mutations have
# repeatedly missed their target on this project (a later definition
# overwrites an earlier one), so nothing here tries to shadow a function
# by defining another one in front of it.

PACKAGE = "frontend-gui"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "G1 snap_rect stops snapping (device-pixel alignment lost)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    let snap = |v: f32| (v * ppp).round() / ppp;",
        "new": "    let snap = |v: f32| v;",
        "test": "snap_rect_maps_fractional_coords_to_integers",
        "note": (
            "Deliberately paired with the *other* snap test. Identity still "
            "makes two adjacent cells agree on their shared edge, so "
            "`snap_rect_adjacent_cells_share_an_edge_exactly` would SURVIVE "
            "this and prove nothing -- the edge-sharing property is about "
            "consistency, not about snapping. Only the fractional->integer "
            "test can observe that the snapping itself is gone."
        ),
    },
    {
        "label": "G2 indent guides start at `step` instead of 0 (the original defect)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "            let mut cols = Vec::new();\n            let mut col = 0;",
        "new": "            let mut cols = Vec::new();\n            let mut col = step;",
        "test": "indent_guide_one_level_indent_gets_a_guide_at_column_zero",
        "note": (
            "This is the exact bug that shipped in the first round, and it "
            "came from the spec, not the code: 'positive multiples of step, "
            "strictly less than the leading-space count' gives a "
            "single-level-indented line no guide at all. Every line in "
            "demo/rtl/core/alu.sv's port list is indented exactly one level, "
            "so a whole real file rendered with zero guides and the tests of "
            "the time were all green."
        ),
    },
    {
        "label": "G3 blank-row continuation takes the deeper neighbour, not the shallower",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "            (Some(a), Some(b)) => a.min(b),",
        "new": "            (Some(a), Some(b)) => a.max(b),",
        "test": "indent_guide_blank_row_between_unequal_neighbors_uses_the_smaller",
        "note": (
            "The reviewer caught that this line had no test able to see it: "
            "every blank-row test used neighbours with equal indentation "
            "(8 and 8), where min, max and mean all agree. The named test "
            "was added in the fix round precisely to close that hole, so "
            "this entry is the one that proves the hole is closed."
        ),
    },
    {
        "label": "G4 `gui-indent-guide-step` of 0 no longer disables guides",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if step == 0 {\n        return vec![Vec::new(); rows.len()];\n    }",
        "new": "    if false {\n        return vec![Vec::new(); rows.len()];\n    }",
        "test": "indent_guide_step_zero_disables_guides",
        "note": "With the guard gone, `col += step` never advances and the loop hangs or yields garbage; either way the test must not pass.",
    },
    {
        "label": "G5 scrollbar appears when the buffer exactly fits",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if total_lines == 0 || visible_lines == 0 || total_lines <= visible_lines {",
        "new": "    if total_lines == 0 || visible_lines == 0 || total_lines < visible_lines {",
        "test": "scrollbar_absent_when_buffer_fits",
        "note": "The equal case is the boundary: a buffer whose line count is exactly the window height needs no scrollbar.",
    },
    {
        "label": "G6 thumb loses its minimum length (invisible in a huge file)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    let len = raw_len.max(min_len).min(track_len);",
        "new": "    let len = raw_len.min(track_len);",
        "test": "scrollbar_thumb_clamps_to_the_24px_minimum",
        "note": "In a 100k-line file the proportional thumb is a fraction of a pixel tall, i.e. gone.",
    },
    {
        "label": "G7 thumb can overrun the bottom of its track",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    let top = (track_len * (top_line as f32 / total_lines as f32)).min(track_len - len);",
        "new": "    let top = track_len * (top_line as f32 / total_lines as f32);",
        "test": "scrollbar_thumb_top_clamps_when_scrolled_to_the_very_bottom",
        "note": "Only observable at the very bottom of a large buffer, which is why the reviewer flagged this clamp as untested before the fix round added the case.",
    },
    {
        "label": "G8 cursor glyph fades into its own background again",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if t < 0.5 {\n        normal_fg\n    } else {\n        under_cursor_fg\n    }",
        "new": "    lerp_color(normal_fg, under_cursor_fg, t)",
        "test": "cursor_fade_glyph_and_background_never_collide",
        "note": (
            "This restores the defect the reviewer found by arithmetic: when "
            "the glyph and the background are lerped by the same t toward "
            "each other's colours, their difference is (1-2t)*(fg0-bg0), "
            "which is exactly zero at t=0.5 -- the character under the "
            "cursor disappears twice per blink cycle."
        ),
    },
    {
        "label": "G9 grid floors ignore what actually fits (cells painted outside the padded area)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if computed >= desired {\n        computed\n    } else {\n        computed.max(1)\n    }",
        "new": "    computed.max(desired)",
        "test": "dimension_floor_never_forces_a_count_larger_than_what_fits",
        "note": "`computed.max(desired)` is the old `cols.max(20)`/`rows.max(5)` behaviour, which a 480x320 window with 64px padding overflows.",
    },
    {
        "label": "G10 padding clamp ceiling removed",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    raw.clamp(0, 64)",
        "new": "    raw.clamp(0, 9999)",
        "test": "padding_clamp_out_of_range_both_sides",
        "note": "Unique in the file: the other two clamps are (80, 200) and (8, 72).",
    },
    {
        "label": "G11 cursor never stops blinking (33ms repaint forever while focused)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if blink_limit > 0 && period > 0.0 && elapsed_secs >= blink_limit as f32 * period {\n        return (false, 1.0);\n    }",
        "new": "    if false {\n        return (false, 1.0);\n    }",
        "test": "blink_decision_stops_and_goes_solid_after_the_limit",
        "note": (
            "This restores what the trailing re-review found: with no "
            "stopping rule, a focused window animates forever, so the "
            "repaint cadence stays at 33ms (~30Hz) for as long as the "
            "editor is open -- about 15x the old idle rate, purely to "
            "blink a cursor. GNU's `blink-cursor-blinks` (default 10) is "
            "the precedent for stopping."
        ),
    },
    {
        "label": "G12 `gui-cursor-blinks` clamp ceiling removed",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    raw.clamp(0, 100)",
        "new": "    raw.clamp(0, 100000)",
        "test": "cursor_blinks_clamp_out_of_range_both_sides",
        "note": "Unique in the file: the other clamps are (0, 64), (80, 200) and (8, 72).",
    },
]
