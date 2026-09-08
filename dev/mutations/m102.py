# Mutation list for M102 (window sizing: split ratios, resize commands,
# minimum-size clamp). Designed by the reviewer in step 4, extended by the
# main conversation with three entries for the fix round's own changes
# (F1-F3), and run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m102.py \
#         -p core --test-target core_tests
#
# F1 is the one that matters most: the epsilon in `split_lengths` is what
# stops `enlarge-window` from stalling, and the ORIGINAL test suite passed
# with that defect present -- the expectations had been copied from the
# broken output.

PACKAGE = "core"
TEST_TARGET = "core_tests"

MUTATIONS = [
    {
        "label": "F1 split_lengths goes back to plain truncation (the stall defect)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let raw_a = ((((avail as f32) * frac) + SPLIT_LEN_EPS).floor() as usize).min(avail);",
        "new": "    let raw_a = (((avail as f32) * frac) as usize).min(avail);",
        "test": "enlarge_window_never_stalls_across_frame_sizes",
    },
    {
        "label": "F2 resize arithmetic goes back to a panicking add",
        "file": "crates/core/src/redisplay.rs",
        "old": "                    let new_a = clamp_side_len((a_len as i64).saturating_add(delta), avail, min);",
        "new": "                    let new_a = clamp_side_len(a_len as i64 + delta, avail, min);",
        "test": "window_resize_selected_does_not_panic_on_extreme_delta",
    },
    {
        "label": "F3 split SIZE is no longer range-checked",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "                if sz < min as i64 || sz > (avail - min) as i64 {",
        "new": "                if false {",
        "test": "split_window_below_refuses_a_size_of_zero_or_negative",
    },
    {
        "label": "R1 default split ratio is no longer a half",
        "file": "crates/core/src/editor.rs",
        "old": "            frac: frac.unwrap_or(0.5),",
        "new": "            frac: frac.unwrap_or(0.4),",
    },
    {
        "label": "R2 minimum-size clamp removed from split_lengths",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let a = raw_a.clamp(min, avail - min);",
        "new": "    let a = raw_a;",
        # Survived the first run: the resize path clamps again in
        # `clamp_side_len`, so this guard's real job is a FRAME SHRINK that
        # makes an already-stored `frac` produce a sub-minimum window --
        # which nothing tested. The test named here was written for it.
        "test": "window_min_height_survives_a_frame_shrink_after_a_skewed_resize",
    },
    {
        "label": "R3 the too-small-to-split guard never fires",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "        if raw < 2 * min + sep {\n            return Ok(Value::Nil);\n        }",
        "new": "        if false {\n            return Ok(Value::Nil);\n        }",
    },
    {
        "label": "R4 the horizontal separator column is not deducted when resizing",
        "file": "crates/core/src/redisplay.rs",
        "old": "                let avail = rect.width.saturating_sub(sep);",
        "new": "                let avail = rect.width;",
    },
    {
        "label": "R5 resize applies to a split of any orientation",
        "file": "crates/core/src/redisplay.rs",
        "old": "                if is_horiz == want_horizontal {\n                    let new_a = clamp_side_len((a_len as i64).saturating_add(delta), avail, min);",
        "new": "                if true {\n                    let new_a = clamp_side_len((a_len as i64).saturating_add(delta), avail, min);",
    },
    {
        "label": "R6 balance-windows does not recurse into subtrees",
        "file": "crates/core/src/editor.rs",
        "old": "            a.balance();\n            b.balance();",
        "new": "            let _ = (&a, &b);",
    },
    {
        "label": "T1 SIZE lower bound off by one (rejects SIZE == min)",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "                if sz < min as i64 || sz > (avail - min) as i64 {",
        "new": "                if sz <= min as i64 || sz > (avail - min) as i64 {",
        "test": "split_window_below_size_at_the_minimum_boundary_succeeds",
    },
    {
        "label": "T2 SIZE upper bound off by one (rejects SIZE == avail - min)",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "                if sz < min as i64 || sz > (avail - min) as i64 {",
        "new": "                if sz < min as i64 || sz >= (avail - min) as i64 {",
        "test": "split_window_below_size_at_the_maximum_boundary_succeeds",
    },
]
