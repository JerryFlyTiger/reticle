# M69's gui_features_tests pass. The only reason this is a separate file is
# that `--test-target` applies to the whole config (precedent from
# m60/m66/m68). The main list and header explanation are in
# dev/mutations/m69.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m69-gui.py -p core \
#         --test-target gui_features_tests

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "M1 left_limit only makes room for half the right segment (symmetric but wrong guard)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let left_limit = rect_w.saturating_sub(right_w);",
        "new": "    let left_limit = rect_w.saturating_sub(right_w / 2);",
        "expect_fail": ["m69_narrow_frame_keeps_line_number"],
        "note": "Looks like a reasonable 'leave half' pattern, but actually lets "
                "the left segment eat two columns from the right one. "
                "Designed by the reviewer; M68's M11 is an example of the same type.",
    },
    {
        "label": "M2 the drawing loop's put_wide branch removed (reverts defect 2)",
        "file": "crates/core/src/redisplay.rs",
        "old": "        let st = style_of(mcol);\n"
               "        if w == 2 {\n"
               "            grid.put_wide(row, col0 + mcol, c, st);\n"
               "        } else {\n"
               "            grid.put(row, col0 + mcol, c, st);\n"
               "        }",
        "new": "        let st = style_of(mcol);\n"
               "        grid.put(row, col0 + mcol, c, st);",
        "expect_fail": ["m69_dired_cjk_dir_right_segment_wide_and_column_exact"],
        "note": "The next cell no longer gets marked as a continuation, so that row's terminal column count comes out one column too many.",
    },
]
