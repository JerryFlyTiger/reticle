# Mutation list for M111 (GUI window transparency). Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m111.py \
#         -p frontend-gui --test-target ""
#
# The unit tests for this milestone live in `lib.rs`'s `#[cfg(test)]` block
# rather than under `tests/`, because every function here is private. So the
# runner is pointed at the crate's lib target, not a named integration test.
#
# Two entries are declared expected survivors. The GUI has no headless
# rendering path, so the two lines that decide whether the window is
# transparent AT ALL are reachable only by launching the editor and sampling
# a screenshot's alpha channel -- which is how this milestone was actually
# verified (`dev/gui-drive.sh` + PIL), but which no `cargo test` can do.
# Recorded honestly rather than left looking covered.
#
# M117 appended T8-T12. The original list tested `clamp_opacity`,
# `opacity_percent_to_frac`, `to_bg_color` and `cursor_bg_color` by calling
# them directly with hand-picked arguments -- not one entry touched the wire
# from the elisp variable to those helpers, so hardcoding the frame's opacity
# to 100 would have passed every one of them. M117 extracted that read into
# `frame_opacity_frac` / `frame_bg` so it is reachable from `cargo test`;
# T8-T10 are the entries that now watch it. T11 and T12 are the honest other
# half: the per-cell and panel paint calls really are screenshot-only, and are
# recorded as declared survivors rather than left absent -- an absent entry is
# indistinguishable from nobody having thought about it.

PACKAGE = "frontend-gui"
TEST_TARGET = None

MUTATIONS = [
    {
        "label": "T1 the opacity floor is removed (a 0% window is unrecoverable)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    raw.clamp(20, 100)",
        "new": "    raw.clamp(0, 100)",
        "test": "opacity_clamp_low_end_floors_at_20",
    },
    {
        "label": "T2 the opacity cap is removed",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    raw.clamp(20, 100)\n}",
        "new": "    raw.clamp(20, 1000)\n}",
        "test": "opacity_clamp_high_end_caps_at_100",
    },
    {
        "label": "T3 the percent-to-fraction divisor is wrong",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    clamp_opacity(pct) as f32 / 100.0",
        "new": "    clamp_opacity(pct) as f32 / 1000.0",
        "test": "opacity_percent_to_frac_covers_the_divisor",
    },
    {
        "label": "T4 background conversion drops the opacity entirely",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    with_alpha(to_color(c), opacity_frac)",
        "new": "    to_color(c)",
        "test": "to_bg_color_floor_opacity_gives_expected_alpha",
    },
    {
        "label": "T5 the box cursor lerps a premultiplied colour as if unmultiplied",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    lerp_color(normal_bg.to_opaque(), under_cursor_bg, t)",
        "new": "    lerp_color(normal_bg, under_cursor_bg, t)",
        "test": "cursor_bg_color_forces_a_translucent_normal_bg_opaque_before_lerping",
    },
    {
        # Declared expected survivor. `App::clear_color` is only ever called
        # by eframe on a live window; `App` itself is constructed solely
        # inside `run_gui`'s real `CreationContext` closure, so no test can
        # reach it. Verified instead by screenshot: at gui-opacity 40 the
        # window's own pixels come back with alpha 102, which they could not
        # if an opaque clear colour were painted underneath.
        "expect": "survived",
        "label": "T6 the clear colour is opaque again (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        egui::Rgba::TRANSPARENT.to_array()",
        "new": "        [0.0, 0.0, 0.0, 1.0]",
        "test": "opacity_percent_to_frac_covers_the_divisor",
    },
    {
        # Declared expected survivor, same reason: nothing in a test inspects
        # `ViewportBuilder` state, and this project has no headless render.
        "expect": "survived",
        "label": "T7 the viewport is no longer transparent (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        .with_transparent(true);",
        "new": "        ;",
        "test": "opacity_percent_to_frac_covers_the_divisor",
    },
    {
        "label": "T8 DELETION: the frame ignores gui-opacity and is always fully opaque",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    opacity_percent_to_frac(int_var(interp, \"gui-opacity\", 100))",
        "new": "    let _ = interp;\n    1.0",
        "test": "frame_opacity_frac",
    },
    {
        "label": "T9 the read points at a variable name that does not exist",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    opacity_percent_to_frac(int_var(interp, \"gui-opacity\", 100))",
        "new": "    opacity_percent_to_frac(int_var(interp, \"gui-opacty\", 100))",
        "test": "frame_opacity_frac_reads_the_gui_opacity_variable",
    },
    {
        "label": "T10 the theme background stops carrying the alpha",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        .map(|c| to_bg_color(c, opacity_frac))\n        .unwrap_or_else(|| with_alpha(FALLBACK_BG, opacity_frac))",
        "new": "        .map(to_color)\n        .unwrap_or_else(|| with_alpha(FALLBACK_BG, opacity_frac))",
        "test": "frame_bg_carries_the_opacity_onto_the_theme_background",
    },
    {
        # Screenshot-only, same class as T6/T7: `App` is built only inside
        # eframe's real `CreationContext` closure, so nothing that runs inside
        # `App::update` is reachable from `cargo test`. This is the line that
        # decides whether the CONTENT AREA renders translucent at all -- more
        # central to the feature than T6/T7 -- and until M117 it had no entry
        # of any kind. PLAN.md's M111 measurement (content area, gutter and all
        # four edges reading alpha 102 at gui-opacity 40) is what sees it, via
        # dev/gui-shot.sh.
        "expect": "survived",
        "label": "T11 the panel fill goes opaque (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        let panel_frame = egui::Frame::none().fill(bg);",
        "new": "        let panel_frame = egui::Frame::none().fill(bg.to_opaque());",
        "test": "frame_bg_carries_the_opacity_onto_the_theme_background",
    },
    {
        # Screenshot-only for the same reason. This is the per-cell call site,
        # where a themed face's own :background (mode-line, hl-line, selection)
        # picks up the opacity. Dropping the fraction here leaves the default
        # background still fading while every themed cell stays opaque -- the
        # exact shape of the double-paint defect PLAN.md records for M111,
        # which it also notes "neither defect is reachable by any unit test".
        # dev/gui-drive.sh with a driver that toggles gui-opacity while a
        # themed region is visible is what would see it; no such driver exists
        # yet, which is itself worth recording here rather than in nobody's
        # notes.
        #
        # This expression is TRIPLICATED across three paint paths -- the
        # pass-1 site mutated here, `draw_cell_fallback`, and the shaped-run
        # path -- each carrying its own independent copy of the
        # `with_alpha`-only-inside-`if eff.reverse` fix. The anchor below is
        # multi-line precisely because the single line is not unique. The
        # fallback copy is the worse one: it is reached only when shaping
        # validation fails or a run is not column-uniform, and nothing in
        # dev/ has ever screenshotted it under gui-opacity, so it is untested
        # at every level rather than merely screenshot-only.
        "expect": "survived",
        "label": "T12 themed cells stop fading (expected to SURVIVE -- screenshot-only)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": """                        let eff = cell.style.or_default(&base);\n                        let (mut cfg, mut cbg) = (\n                            eff.fg.map(to_color).unwrap_or(fg),\n                            eff.bg.map(|c| to_bg_color(c, opacity_frac)).unwrap_or(bg),""",
        "new": """                        let eff = cell.style.or_default(&base);\n                        let (mut cfg, mut cbg) = (\n                            eff.fg.map(to_color).unwrap_or(fg),\n                            eff.bg.map(to_color).unwrap_or(bg),""",
        "test": "frame_bg_carries_the_opacity_onto_the_theme_background",
    },
]
