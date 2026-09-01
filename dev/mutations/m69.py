# M69 -- mode line layout: preserve the line number in narrow windows, wide
# characters in the right segment, and the boundary of `~` abbreviation.
#
# Source of this list: the first-round reviewer designed 5 entries (M1-M5)
# from a cold read of the diff; the main conversation added M6/M8, and the
# trailing re-review brought out M9 as well. M6 and M9 both guard **code
# that only grew in later rounds** (F3's gap reservation, and finding 1 of
# the trailing re-review's name_cols back-computation) -- they didn't exist
# yet when the earlier round's reviewer read the diff, so its list couldn't
# possibly cover them.
#
# The original M7 (measuring name_cols after the modified marker) no longer
# has a corresponding code shape after the trailing re-review's fix; what it
# was guarding is now covered together by M9 (which hits both u10 and u14).
#
# How to run (three passes, since `--test-target` applies to the whole
# config; the guards span three test targets):
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m69.py -p core \
#         --test-target lib
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m69-gui.py -p core \
#         --test-target gui_features_tests
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m69-eshell.py -p core \
#         --test-target eshell_tests
#
# **`--test-target lib` was added to `mutate.py` for this list by M69**:
# three of M69's guards are watched by `redisplay.rs`'s own `#[cfg(test)]`
# unit tests (pure-function-level tests belong in the source file per
# CLAUDE.md), and that isn't a `--test` target. Leaving the target empty
# makes cargo run through 77 test binaries, turning one mutation into tens
# of minutes.
#
# **`PYTHONUNBUFFERED=1` is not optional** (see the header of m68.py: the
# whole M67 batch was lost to this).
#
# The two fields `expect_fail` and `note` are **not read** by `mutate.py`,
# they're purely for humans (it only recognizes
# `label`/`file`/`old`/`new`/`expect`/`test`/`timeout`).
#
# ## The most important entry in this list is M6
#
# It reverts **the `ML_GAP` reservation only added by the fix-up round**.
# The first version's fallback only guaranteed "total width doesn't exceed
# width", and the result was that on an 80-column terminal, pressing `C-x 3`
# three times in a row (-> a window of ~9 columns) showed `...r.svL1:0` on
# screen -- the title and line number were completely glued together,
# recreating this milestone's first symptom at an even narrower width. None
# of the 12 unit tests at the time caught it, because they only asserted
# "doesn't exceed the width", not "there's a gap". If M6 survives, it means
# u13 isn't really watching that guard.
#
# ## M1 is a "symmetric but wrong guard" type (M68's M11 is an example of
#    the same type)
#
# `right_w` -> `right_w / 2` looks like a reasonable "leave half" pattern,
# and doesn't make the code look broken, it just makes the right segment
# render two characters short. A mistaken change that has been proposed or
# is imaginable is more worth guarding against than one constructed out of
# thin air.

PACKAGE = "core"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "M3 `~` abbreviation reverted to raw prefix comparison (no separator boundary)",
        "file": "crates/core/src/redisplay.rs",
        "old": "        .map(|d| crate::complete::abbreviate_home_with(d, p.home))",
        "new": "        .map(|d| match p.home {\n"
               "            Some(h) if d.starts_with(h) => format!(\"~{}\", &d[h.len()..]),\n"
               "            _ => d.to_string(),\n"
               "        })",
        "expect_fail": ["u11_home_abbreviation_boundary"],
        "note": "After reverting, /home/u2/rtl with home=/home/u gets abbreviated to ~2/rtl (this milestone's defect 3).",
    },
    {
        "label": "M5 name_cols uses character count instead of column count (reverts defect 5)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let name_cols = ml_width(&left).saturating_sub(left_suffix_w);",
        "new": "    let name_cols = left.chars().count().saturating_sub(left_suffix_w);",
        "expect_fail": ["u10_name_cols_is_columns_not_chars"],
        "note": "Under a CJK title the bold boundary undercounts. u10's correct "
                "value is 5 (1 column for \" \" + 4 columns for \"project\" in Chinese), "
                "and after reverting it's 3 (1 space + 2 characters) -- "
                "an undercount, but not exactly half, and a title mixed "
                "with ASCII skews by a different ratio.",
    },
    {
        "label": "M6 remove the fallback's ML_GAP reservation (reverts the fix-up round's F3 fix)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let avail_left = width.saturating_sub(right_w + ML_GAP);",
        "new": "    let avail_left = width.saturating_sub(right_w);",
        "expect_fail": ["u13_gap_reserved_even_in_degenerate_widths"],
        "note": "This is exactly the `...r.svL1:0` screen after C-x 3 three times. The most important entry in the whole list.",
    },
    {
        "label": "M8 ml_sanitize becomes an identity function (reverts control-character handling)",
        "file": "crates/core/src/redisplay.rs",
        "old": "fn ml_sanitize(s: &str) -> String {\n"
               "    let mut out = String::with_capacity(s.len());",
        "new": "fn ml_sanitize(s: &str) -> String {\n"
               "    if true {\n"
               "        return s.to_string();\n"
               "    }\n"
               "    let mut out = String::with_capacity(s.len());",
        "expect_fail": ["u8"],
        "note": "Raw control bytes would go straight into the grid, and the terminal would receive bare ESC/TAB.",
    },
    {
        "label": "M9 name_cols no longer subtracts the suffix (reverts trailing re-review finding 1)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let name_cols = ml_width(&left).saturating_sub(left_suffix_w);",
        "new": "    let name_cols = ml_width(&left);",
        "expect_fail": [
            "u14_name_cols_excludes_the_marker_after_the_final_truncation",
            "u10_name_cols_is_columns_not_chars",
        ],
        "note": "Before the trailing re-review, the code computed name_cols "
                "from the start and took `.min(ml_width(&left))`; once "
                "`left` is truncated from the front keeping the tail, that "
                "min always equals the whole of left, so the bold styling "
                "spills onto \" *\" and the mode name. This mutation writes "
                "that degenerate result directly -- looks like a harmless "
                "simplification, exactly the \"symmetric but wrong\" type.",
    },
]
