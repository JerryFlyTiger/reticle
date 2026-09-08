# Mutation list for M115 (diagnostic marks on the scrollbar).
# Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m115.py \
#         -p frontend-gui
#
# The tests live in `lib.rs`'s own `#[cfg(test)]` block because every function
# here is private, so the runner is pointed at the crate's lib target.
#
# Two entries are declared expected survivors, and they are the drawing itself.
# This project has no headless rendering path: `App` is constructed only inside
# eframe's real `CreationContext` closure, so nothing a `cargo test` can reach
# ever calls `painter.rect_filled`. Those two were verified by screenshot and
# pixel measurement instead -- error marks at y = 212/325/438/551/665 on a
# 174-line fixture, spacing 113/113/113/114 for a constant 28-line gap, within
# one pixel of the prediction, and 226/226/226/228 on a 2x capture of the same
# file. Declared rather than left looking covered.
#
# M117 appended S9 and S10. S9 watches the buffer-keyed diagnostics lookup,
# which until M117 was written inline inside `App::update` and therefore
# unreachable from any test: a wrong key (the other window's buffer in a split)
# would have silently dropped every mark with the whole suite still green. It
# is now `window_diagnostic_lines`. S10 is the deletion entry this project
# requires of every list -- for this feature the last hop really is
# screenshot-only, so it is declared rather than omitted.

PACKAGE = "frontend-gui"
TEST_TARGET = None

MUTATIONS = [
    {
        "label": "S1 the most severe no longer wins a merged pixel row",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": ".and_modify(|e| *e = (*e).min(sev))",
        "new": ".and_modify(|e| *e = (*e).max(sev))",
        "test": "diagnostic_marks_merge_same_pixel_row_and_most_severe_wins",
    },
    {
        "label": "S2 the out-of-range guard is off by one at the boundary",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        if line >= total_lines {",
        "new": "        if line > total_lines {",
        "test": "diagnostic_marks_line_exactly_at_total_lines_is_dropped",
    },
    {
        "label": "S3 the row bucket truncates instead of rounding",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        let row = top.round() as i64;",
        "new": "        let row = top as i64;",
        "test": "diagnostic_marks_merge_different_lines_that_round_onto_the_same_pixel_row",
    },
    {
        "label": "S4 the mark length is no longer clamped to a short track",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    let len = min_len.min(track_len);",
        "new": "    let len = min_len;",
        "test": "diagnostic_marks_length_never_exceeds_a_track_shorter_than_min_len",
    },
    {
        "label": "S5 the top clamp is dropped, so a mark can run past the track",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        let top = raw_top.clamp(0.0, max_top);",
        "new": "        let top = raw_top;",
        "test": "diagnostic_marks_at_first_and_last_line",
    },
    {
        "label": "S6 a zero-height track no longer short-circuits",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if total_lines == 0 || track_len <= 0.0 {",
        "new": "    if total_lines == 0 || track_len < 0.0 {",
        "test": "diagnostic_marks_zero_height_track_is_no_marks",
    },
    {
        # Declared expected survivor. The drawing itself is unreachable from
        # any test: `App` is built only inside eframe's real `CreationContext`
        # closure, so no `cargo test` in this workspace ever reaches a
        # `painter` call. Verified by screenshot instead -- inset to
        # x=1177..1180 inside a 6px track at x=1176..1181, measured.
        "expect": "survived",
        "label": "S7 the mark loses its inset and fills the track (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "                                    Pos2::new(bar_x + 1.0, track_top + mark_top),\n                                    Vec2::new(4.0, mark_len),",
        "new": "                                    Pos2::new(bar_x, track_top + mark_top),\n                                    Vec2::new(6.0, mark_len),",
        "test": "diagnostic_marks_placement_is_proportional_to_the_whole_buffer",
    },
    {
        # Declared expected survivor, same reason. This is the wiring the cold
        # reviewer named as the likeliest way to get the feature subtly wrong:
        # placing marks against the visible window instead of the whole buffer
        # would look plausible on a short file. The screenshot settles it --
        # error spacing was constant at 113px for a constant 28-line gap on a
        # 174-line file whose window shows 34 lines, which only holds if the
        # denominator is the whole buffer.
        "expect": "survived",
        "label": "S8 marks are placed against the visible window (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "                                let color = to_color(core::redisplay::severity_color(",
        "new": "                                let _ = visible_lines;\n                                let color = to_color(core::redisplay::severity_color(",
        "test": "diagnostic_marks_placement_is_proportional_to_the_whole_buffer",
    },
    {
        "label": "S9 the marks read the wrong buffer's diagnostics",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        .get(&(Rc::as_ptr(buf) as usize))",
        "new": "        .get(&(Rc::as_ptr(buf) as usize).wrapping_add(1))",
        "test": "window_diagnostic_lines",
    },
    {
        # The deletion entry. Screenshot-only: `App` is built only inside
        # eframe's real `CreationContext` closure, so the `painter.rect_filled`
        # that actually draws a mark cannot be reached by `cargo test`. What
        # sees it is dev/gui-shot.sh over dev/gen-diagnostic-fixture.py's
        # fixture -- PLAN.md's M115 record measures mark spacing at 113/113/
        # 113/114 px (226/226/226/228 at 2x), which is zero marks away from
        # nothing being drawn at all.
        "expect": "survived",
        "label": "S10 DELETION: no mark is ever painted (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "                                painter.rect_filled(snap_rect(rect, ppp), 1.0, color);\n                            }\n\n                            let sb_color",
        "new": "                                let _ = (rect, color, ppp);\n                            }\n\n                            let sb_color",
        "test": "diagnostic_marks_placement_is_proportional_to_the_whole_buffer",
    },
]
