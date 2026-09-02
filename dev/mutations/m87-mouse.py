# M87 stage 2a -- paint runs and the GUI's first mouse support.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-mouse.py \
#         -p frontend-gui --test-target lib
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-mouse.py \
#         -p core --test-target gui_features_tests --only C1 --only C2
#
# Two runs, two targets, because the hit-testing guards live in the GUI
# crate and the inverse mapping lives in `core`.
#
# What is being defended: `Grid::buffer_pos_at` is the first
# screen-position -> buffer-position mapping this codebase has ever had.
# Every previous position mapping ran the other way, as a side effect of
# painting. The two hard cases are the ones where display columns and
# source bytes stop being proportional (a tab is one byte and up to eight
# columns; a control character is one byte and two columns) and the ones
# where a column has no character under it at all (the gutter, past the
# end of a line, an empty line).
#
# G1 is the most valuable entry here. `buffer_pos_at` given a gutter
# column finds no run covering it and falls through to "end of this
# row's last run" -- so a click on a line number would put point at the
# end of that line. The GUI's own guard is the only thing preventing it,
# and before this milestone nothing tested that guard at all.
#
# All entries are replacement-style.

MUTATIONS = [
    {
        "label": "G1 the gutter guard is dropped from the GUI hit test",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        let text_left = win.col + win.gutter_cols;",
        "new": "        let text_left = win.col;",
        "test": "pixel_to_buffer_pos_gutter_column_returns_none",
        "note": (
            "Run against `-p frontend-gui --test-target lib`. Without the "
            "guard a click on a line number reaches `buffer_pos_at` with a "
            "column no run covers, and the fallback puts point at the end of "
            "the line -- silently, and only for clicks in a narrow strip. "
            "`buffer_pos_at_gutter_column_falls_through_to_line_end_not_none` "
            "in the core suite pins that fallback so the two halves of this "
            "cannot drift apart."
        ),
    },
    {
        "label": "G2 the mode-line row stops being excluded from clicks",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        if row >= win.row && row < win.mode_line_row && col >= text_left && col < text_right {",
        "new": "        if row >= win.row && row <= win.mode_line_row && col >= text_left && col < text_right {",
        "test": "pixel_to_buffer_pos_mode_line_row_returns_none",
        "expect": "SURVIVED",
        "note": (
            "**This one SURVIVES, and that is the correct result.** Recorded "
            "rather than contrived around.\n"
            "\n"
            "The guard is redundant given how runs are built. Every cell on a "
            "mode-line row belongs to a chrome run with `src: None`, so even "
            "with the row admitted, `buffer_pos_at` finds no src-bearing run "
            "and returns `None`, and `pixel_to_buffer_pos`'s `?` propagates "
            "it. The core suite's `buffer_pos_at_chrome_row_returns_none` is "
            "what actually does the protecting; this guard is defence in "
            "depth behind it.\n"
            "\n"
            "Keep the guard: it stops depending on a property of run "
            "construction that a future change could alter. But do not read "
            "this SURVIVED as a coverage gap and go writing a test for it -- "
            "there is no observable behaviour to differ. The check on a "
            "SURVIVED result is two-part: did the mutation land, and did it "
            "change the semantics. This one landed and changed nothing."
        ),
    },
    {
        "label": "G3 mouse drag enters visual state regardless of the state it started in",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    if evil_on && evil_normal {",
        "new": "    if evil_on {",
        "test": "arm_mouse_selection_does_not_change_evil_insert_state",
        "note": (
            "Dropping the normal-state gate makes a drag yank the user out "
            "of insert state. The gate exists because entering visual from "
            "insert is not a selection, it is losing your place mid-edit."
        ),
    },
    {
        "label": "C1 tabs and control characters map proportionally again",
        "file": "crates/core/src/redisplay.rs",
        "old": "        if r.atomic {",
        "new": "        if false {",
        "test": "buffer_pos_at_tab_maps_every_column_to_the_tab_byte",
        "note": (
            "Run against `-p core --test-target gui_features_tests`. Marks the "
            "runs where one source byte expanded into several columns. "
            "Re-anchored: this used to test `r.text.len() != byte_len`, "
            "which inferred the property from a coincidence of two lengths "
            "and broke when an invisible region hid exactly three bytes -- "
            "see C3."
        ),
    },
    {
        "label": "C2 wide characters map by byte count instead of display width",
        "file": "crates/core/src/redisplay.rs",
        "old": "            let w = wide_char_width(ch).max(1);",
        "new": "            let w = 1;",
        "test": "buffer_pos_at_wide_char_continuation_maps_to_char_start",
        "note": (
            "A CJK character occupies two columns; treating it as one makes "
            "every click after the first wide character on a line land one "
            "character early, and the error accumulates across the row."
        ),
    },
    {
        "label": "C3 atomic runs stop being marked as atomic at the source",
        "file": "crates/core/src/redisplay.rs",
        "old": "            atomic: true,",
        "new": "            atomic: false,",
        "test": "buffer_pos_at_invisible_cjk_ellipsis_maps_every_column_to_the_hidden_span_start",
        "note": (
            "The panic this milestone's cold read found. The old code "
            "inferred 'this run expanded non-proportionally' from "
            "`text.len() != byte_len`. The invisible-region indicator is "
            "always the three bytes `...`, so when an overlay hid a span "
            "that was also exactly three bytes -- one CJK character is one "
            "char and three bytes, not an exotic case -- the two lengths "
            "matched, the proportional branch ran, and it returned "
            "`src.start + 1` and `src.start + 2`. Those are UTF-8 "
            "continuation bytes. They reach `GapBuffer::resolve_byte`, whose "
            "`debug_assert!(is_char_boundary_at(..))` panics in a debug "
            "build and silently misplaces point in release.\n"
            "\n"
            "The fix was to stop inferring and mark it explicitly at the "
            "point where the code already knows. C1 defends the reader of "
            "the flag; this entry defends the writer."
        ),
    },
    {
        "label": "C4 the char-boundary invariant is no longer upheld",
        "file": "crates/core/src/redisplay.rs",
        "old": "        if r.atomic {",
        "new": "        if false {",
        "test": "buffer_pos_at_result_is_always_a_char_boundary",
        "note": (
            "Same mutation as C1, different test, and deliberately kept "
            "separate. C1 checks one case (a tab). This one checks the "
            "property that every value the function can return is a "
            "character boundary, over a buffer holding a 2-byte Latin-1 "
            "character, a 3-byte CJK character, a tab and a control "
            "character. Case-by-case tests miss the next combination nobody "
            "thought of; the invariant does not."
        ),
    },
    {
        "label": "C5 chrome runs stop covering a wide character's second column",
        "file": "crates/core/src/redisplay.rs",
        "old": "                if let Some(r) = open.as_mut() {\n                    if r.col + r.cols == col {\n                        r.cols += 1;\n                        continue;\n                    }\n                }",
        "new": "                if open.is_some() {\n                    // mutation: skip the continuation cell entirely\n                }",
        "test": "chrome_runs_cover_every_column_of_a_row_with_a_wide_char",
        "note": (
            "Restores the old behaviour: skip a wide chrome character's "
            "continuation cell instead of extending the open run over it. "
            "The test that catches it asserts the runs tile the row exactly, "
            "with no gap and no overlap.\n"
            "\n"
            "The first version of this entry mutated `atomic: false` to "
            "`atomic: true` in the same function -- it SURVIVED, and not "
            "because of a coverage gap: `atomic` has no bearing on `cols`, "
            "so the mutation landed and changed nothing observable. Picking "
            "the nearest single line to disturb is not the same as picking "
            "a line the behaviour depends on. Before the fix a "
            "CJK character in the mode line left its continuation column "
            "claimed by no run at all. Harmless today, because chrome rows "
            "are excluded from click handling -- but the next milestone "
            "rebuilds each row from `runs` in order to shape and draw it, "
            "and that row would render one column short."
        ),
    },
    {
        "label": "C6 the wheel's scroll pin is never set",
        "file": "crates/core/src/redisplay.rs",
        "old": "        win.scroll_pin = Some(point);",
        "new": "        win.scroll_pin = None;",
        "test": "wheel_scroll_survives_the_next_render_when_point_does_not_move",
        "note": (
            "Without the pin, `ensure_point_visible` recentres on the very "
            "next frame whenever point is more than about three screens from "
            "`window_start` -- so a wheel scroll undoes itself and the view "
            "snaps back. A trackpad delivering many wheel events in one "
            "frame reaches that threshold in a single gesture."
        ),
    },
    {
        "label": "G4 the evil call's failure is swallowed again",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        if interp.eval_source(\"(evil-visual-char)\").is_err() {\n            arm_mark(ed, win_id, byte_pos);\n        }",
        "new": "        let _ = interp.eval_source(\"(evil-visual-char)\");",
        "test": "arm_mouse_selection_falls_back_to_plain_mark_when_evil_visual_char_errors",
        "note": (
            "Run against `-p frontend-gui --test-target lib`. If the elisp "
            "call signals, the drag would proceed with no mark at all and no "
            "diagnostic, while every other branch of this function leaves "
            "`mark`/`mark_active` in a known state."
        ),
    },
    {
        "label": "G5 a drag re-arms the mark every time it moves",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    !already_dragged && byte_pos != start_byte",
        "new": "    byte_pos != start_byte",
        "test": "drag_should_arm_not_rearmed_once_already_dragged",
        "note": (
            "Run against `-p frontend-gui --test-target lib`. Dropping the "
            "already-dragged guard resets the mark to wherever the pointer "
            "currently is on every mouse-move event, so the selection would "
            "collapse to nothing as soon as the user moved a second time."
        ),
    },
]
