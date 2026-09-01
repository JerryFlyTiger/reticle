# M86 (GUI visual quality) mutation list -- the `core` half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m86.py \
#         -p core --test-target gui_features_tests
#
# The `frontend-gui` half lives in dev/mutations/m86-gui.py and runs
# against a different target.
#
# What this half defends: the palette moved out of hard-coded Rust tuples
# and into faces, and `render()` now publishes the window layout it had
# already computed. Both are changes whose failure mode is silent -- a
# colour quietly falls back to a constant, or the GUI quietly draws its
# chrome in the wrong place -- so each one gets a mutation that makes the
# silence audible.
#
# All entries are replacement-style (or a whole-line deletion), never
# insertion. Inserting a same-named definition in front of a real one does
# not shadow it in either elisp or Rust on this project.

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "C1 diagnostic colours stop resolving from faces",
        "file": "crates/core/src/redisplay.rs",
        "old": '        1 => ("diagnostic-error", (244, 71, 71)),   // error: red',
        "new": '        1 => ("no-such-face", (244, 71, 71)),   // error: red',
        "test": "severity_color_resolves_from_the_diagnostic_error_face",
        "note": (
            "Pointing the lookup at a face nobody defines sends it down the "
            "hard-coded fallback -- which is exactly the pre-M86 behaviour, "
            "and is invisible unless a test asserts the face's own colour "
            "comes through."
        ),
    },
    {
        "label": "C2 echo row goes back to unstyled",
        "file": "crates/core/src/redisplay.rs",
        "old": '    let echo_style = face_or(interp, &editor, "echo-area", Style::default());',
        "new": "    let echo_style = Style::default();",
        "test": "echo_row_picks_up_the_echo_area_face",
        "note": "The variable is kept so the four downstream uses still compile; only what it resolves to changes.",
    },
    {
        "label": "C3 light theme drops a face the dark theme has",
        "file": "crates/core/lisp/themes.el",
        "old": '  (set-face \'cursor :background "#1a73c7" :foreground "#fbfbfd")\n',
        "new": "",
        "test": "both_themes_define_the_same_set_of_faces",
        "note": (
            "A whole-line deletion, not an insertion. This is the defect "
            "shape the paired-theme test exists for: adding a face to one "
            "theme and forgetting the other leaves the second theme "
            "inheriting whatever the first one set, which looks fine until "
            "someone starts in light mode."
        ),
    },
    {
        "label": "C4 published mode-line row is off by one",
        "file": "crates/core/src/redisplay.rs",
        "old": "        mode_line_row: mode_row,",
        "new": "        mode_line_row: mode_row + 1,",
        "test": "published_window_layout_matches_a_split_frame",
        "note": (
            "The GUI draws its hairline separator and anchors its scrollbar "
            "from this number. Off by one puts the separator through a line "
            "of text -- and in a split frame, through the wrong window's "
            "text. Nothing in `core` itself renders differently, so only a "
            "test that asserts the published layout can see it."
        ),
    },
    {
        "label": "C5 published gutter width is hard-coded to zero",
        "file": "crates/core/src/redisplay.rs",
        "old": "        gutter_cols: gutter_w,",
        "new": "        gutter_cols: 0,",
        "test": "published_window_layout_reports_the_real_gutter_width",
        "note": (
            "The GUI subtracts this to keep indent guides out of the "
            "line-number column. Zero puts them back on top of the numbers. "
            "The trailing re-review found this field had no test at all -- "
            "this mutation survived until "
            "`published_window_layout_reports_the_real_gutter_width` was "
            "added, so it is the entry that proves the gap is closed."
        ),
    },
]
