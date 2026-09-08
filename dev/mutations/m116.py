# Mutation list for M116 (trailing whitespace, fill-column ruler, ligature
# toggle). Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m116.py -p core
#
# Every entry here exists because the cold read demonstrated the same thing:
# before this milestone's fix round, deleting the ENTIRE trailing-whitespace
# highlight block failed no test, and hardcoding the ligature flag to always-on
# failed no test. The feature worked and nothing connected the tests to it.
# These are the connections.
#
# M117 appended W9-W11. W9 watches the `gui-ligatures` read itself: W6 above
# proves `shape_calt_only` honours the flag it is handed, but until M117
# nothing checked that the live elisp value ever reached that call -- the read
# was inline in `App::update`, so hardcoding it to `true` passed the entire
# workspace. It is now `ligatures_enabled`. W10 covers the shape cache key,
# which had a test but no mutation entry. W11 is the fill-column ruler's
# deletion entry, declared as a survivor for the same screenshot-only reason
# M115's S7/S8 are -- M116's list previously had no entry for it at all, which
# read the same as never having considered it.

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "W1 the point-at-end-of-line exemption never fires",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let mut point_at_line_end = point == cur_line_end;",
        "new": "    let mut point_at_line_end = false;",
        "test": "trailing_whitespace_point_at_line_end_is_exempted",
    },
    {
        "label": "W2 the exemption is not recomputed when a new line starts",
        "file": "crates/core/src/redisplay.rs",
        "old": "                point_at_line_end = point == cur_line_end;",
        "new": "                point_at_line_end = false;",
        "test": "trailing_whitespace_exemption_is_recomputed_on_a_later_line",
    },
    {
        "label": "W3 tabs stop counting as trailing whitespace",
        "file": "crates/core/src/redisplay.rs",
        "old": "            Some(' ') | Some('\\t') => ws_start = pos,",
        "new": "            Some(' ') => ws_start = pos,",
        "test": "trailing_ws_start_trailing_tabs_and_spaces_mixed",
        "package": "core",
        "test_target": None,
    },
    {
        "label": "W4 the trailing-whitespace background is never applied",
        "file": "crates/core/src/redisplay.rs",
        "old": "            if let Some(bg) = trailing_ws_bg {",
        "new": "            if let Some(bg) = None::<(u8, u8, u8)> {",
        "test": "trailing_whitespace_highlights_the_run_at_end_of_line",
    },
    {
        "label": "W5 the ruler is drawn past the right edge of the text area",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "fn fill_column_visible(fill_column: usize, text_cols: usize) -> bool {",
        "new": "fn fill_column_visible(fill_column: usize, text_cols: usize) -> bool {\n    let _ = (fill_column, text_cols);\n    return true;\n    #[allow(unreachable_code)]",
        "test": "fill_column_visible_false_at_the_boundary_column",
        "package": "frontend-gui",
        "test_target": None,
    },
    {
        "label": "W6 the ligature flag becomes a no-op",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "            enable_calt as u32,",
        "new": "            1u32,",
        "test": "shape_calt_only_with_calt_disabled_differs_from_calt_enabled",
        "package": "frontend-gui",
        "test_target": "font_tests",
    },
    {
        "label": "W7 prog-mode stops turning trailing whitespace on",
        "file": "crates/core/lisp/modes.el",
        "old": "          (lambda () (setq-local show-trailing-whitespace t)))",
        "new": "          (lambda () (setq-local show-trailing-whitespace nil)))",
        "test": "prog_mode_hook_turns_on_trailing_whitespace_and_fill_column_indicator_by_default",
        "package": "core",
        "test_target": "modes_tests",
    },
    {
        "label": "W8 fill-column reverts to GNU's 70",
        "file": "crates/core/lisp/simple.el",
        "old": "(defvar fill-column 100",
        "new": "(defvar fill-column 70",
        "test": "fill_column_defaults_to_100_not_gnus_70",
        "package": "core",
        "test_target": "modes_tests",
    },
    {
        "label": "W9 DELETION: the ligature flag is ignored and shaping is always on",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    var_truthy(interp, \"gui-ligatures\")",
        "new": "    let _ = interp;\n    true",
        "test": "ligatures_enabled",
        "package": "frontend-gui",
        "test_target": None,
    },
    {
        "label": "W10 the shape cache stops distinguishing the two ligature states",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "        let key = (role, text.to_string(), enable_calt);",
        "new": "        let key = (role, text.to_string(), true);",
        "test": "shape_cache_distinguishes_enable_calt",
        "package": "frontend-gui",
        "test_target": None,
    },
    {
        # The fill-column ruler's deletion entry. Screenshot-only: the paint
        # loop runs inside `App::update`, which only exists inside eframe's
        # real `CreationContext` closure. `fill_column_visible` (W5) and
        # `fill_column_x` are reachable and tested; whether their answers reach
        # the screen is not. What sees it is dev/gui-shot.sh -- PLAN.md's M116
        # record measures the rule at x=2005 (and x=1051 after driving
        # fill-column 100 -> 50 at runtime with dev/gui-drive.sh).
        "expect": "survived",
        "label": "W11 DELETION: the ruler is never painted (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "                        if !fill_column_visible(fill_column, text_cols) {",
        "new": "                        if true || !fill_column_visible(fill_column, text_cols) {",
        "test": "fill_column_visible_false_at_the_boundary_column",
        "package": "frontend-gui",
        "test_target": None,
    },
    {
        # The ligature feature's LAST HOP, and the entry M117 itself nearly
        # left out -- the trailing cold read caught that M117's own record
        # claimed four declared entries covered "all four features" when they
        # covered three, with this wire in exactly the undocumented state the
        # milestone exists to prevent. W9 proves `ligatures_enabled` reads the
        # variable and W10 proves the cache keys on it; NEITHER proves the
        # value reaches the shaping call. Screenshot-only: both ends live in
        # `App::update`, built only inside eframe's real `CreationContext`
        # closure. What sees it is dev/gui-drive.sh with a driver that flips
        # `gui-ligatures` mid-run (the shape of dev/drivers/switch-font.el);
        # PLAN.md's M116 record measured the two shaping outcomes by hand
        # ([.., 1742, 1574, ..] against [.., 1049, 1051, ..]).
        "expect": "survived",
        "label": "W12 DELETION: the shaping call ignores the flag (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "                                    gui_ligatures,\n                                ) else {",
        "new": "                                    true,\n                                ) else {",
        "test": "shape_cache_distinguishes_enable_calt",
        "package": "frontend-gui",
        "test_target": None,
    },
]
