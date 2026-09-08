# Mutation list for M118 (sticky scope header + scope breadcrumb).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m118.py -p core
#
# Per the rule M117 wrote into CLAUDE.md, the deletion entry is per FEATURE,
# not per file. M118 ships three distinct effects and each has its own entry
# that removes the whole thing:
#
#   * the foundation, `Engine::scope_chain`  -> S1
#   * F1, the sticky header rows             -> S3
#   * F2, the breadcrumb mode-line segment   -> S10
#
# **None of the three is declared as a survivor**, and that is the point of
# where this milestone was placed. M115 and M116 had to declare screenshot-only
# entries because their last hop was a `painter` call inside `App::update`,
# which `cargo test` cannot reach. M118's three effects all land in
# `core::redisplay` and come out in the `Grid` that both frontends consume, so
# every one of them is reachable by a real named test. If a future change moves
# any of this into `frontend-gui`, that property is what gets lost.
#
# S9 and S13 are the fix round's two halves of the same defect: the header's
# `PaintRun` cleanup deleted a NEIGHBOURING window's runs on a side-by-side
# split (no column bound), and `Grid::buffer_pos_at`'s past-end-of-row fallback
# then resolved a click in one window to a position in another. The second was
# pre-existing code that no test had ever pinned; both are watched here now.

PACKAGE = "core"
TEST_TARGET = "scope_header_tests"

MUTATIONS = [
    # ---- foundation -------------------------------------------------
    {
        "label": "S1 the scope chain is always empty (deletion entry: foundation)",
        "file": "crates/core/src/highlight.rs",
        "old": "            .filter(|s| s.start <= pos && pos < s.end)",
        "new": "            .filter(|_s| false)",
        "test": "chain_inside_a_module_instantiation_in_real_soc_top_sv",
    },
    {
        "label": "S2 the stale-parse generation guard never fires",
        "file": "crates/core/src/highlight.rs",
        # Deliberately three lines: the one-line guard is byte-identical to
        # `matching_pair`'s own a few dozen lines above, and the runner
        # (correctly) refuses a string that occurs more than once. The
        # `Vec::new()` body is what makes this one unique.
        "old": "        if *gen != buffer.borrow().edit_ticks {\n            return Vec::new();\n        }",
        "new": "        if false {\n            return Vec::new();\n        }",
        "test": "generation_guard_returns_empty_for_a_stale_cache",
    },
    # ---- F1, the sticky header rows ---------------------------------
    {
        "label": "S3 the header is never drawn (deletion entry: F1)",
        "file": "crates/core/src/redisplay.rs",
        "old": '    if var_on(interp, "scope-header") {',
        "new": "    if false {",
        "test": "header_row_spells_the_scrolled_off_module_source_line",
    },
    {
        "label": "S4 FIX-2 reverted: 'scrolled off' compares line numbers again",
        "file": "crates/core/src/redisplay.rs",
        "old": "            chain.iter().filter(|s| s.start < window_start).collect();",
        "new": "            chain.iter().filter(|s| s.start_line < b.text.line_number(window_start).saturating_sub(1)).collect();",
        "test": "a_wrapped_declaration_line_pins_once_the_window_scrolls_past_its_middle",
    },
    {
        "label": "S5 a scope starting exactly at window_start is pinned anyway",
        "file": "crates/core/src/redisplay.rs",
        "old": "            chain.iter().filter(|s| s.start < window_start).collect();",
        "new": "            chain.iter().filter(|s| s.start <= window_start).collect();",
        "test": "nothing_scrolled_off_at_the_top_of_the_file_produces_no_header_row",
    },
    {
        "label": "S6 over-cap truncation keeps the OUTERMOST rows instead of the innermost",
        "file": "crates/core/src/redisplay.rs",
        "old": "            scrolled_off.drain(0..drop);",
        "new": "            scrolled_off.truncate(max_lines);",
        "test": "max_lines_one_with_a_two_deep_chain_keeps_exactly_one_innermost_row",
    },
    {
        "label": "S7 the header may cover the row point was painted on",
        "file": "crates/core/src/redisplay.rs",
        "old": "        if is_selected {\n            if let Some((crow, _)) = cursor {\n                let point_row = crow.saturating_sub(rect.row);\n                n = n.min(point_row);\n            }\n        }",
        "new": "        if false {\n            if let Some((crow, _)) = cursor {\n                let point_row = crow.saturating_sub(rect.row);\n                n = n.min(point_row);\n            }\n        }",
        "test": "point_on_the_first_visible_row_caps_the_header_to_zero_rows",
    },
    {
        "label": "S8 a two-row window can become entirely header",
        "file": "crates/core/src/redisplay.rs",
        "old": "        n = n.min(text_rows.saturating_sub(1));",
        "new": "        n = n.min(text_rows);",
        "test": "a_two_row_window_never_becomes_entirely_header",
    },
    {
        "label": "S9 header rows keep their text runs, so a click maps back into the buffer",
        "file": "crates/core/src/redisplay.rs",
        "old": "                !(r.row >= hdr_lo && r.row < hdr_hi && r.col >= col_lo && r.col < col_hi)",
        "new": "                true",
        "test": "header_row_never_maps_back_to_a_buffer_position",
    },
    {
        "label": "S10 FIX-1 reverted: the run cleanup loses its column bound",
        "file": "crates/core/src/redisplay.rs",
        "old": "                !(r.row >= hdr_lo && r.row < hdr_hi && r.col >= col_lo && r.col < col_hi)",
        "new": "                !(r.row >= hdr_lo && r.row < hdr_hi)",
        "test": "header_cleanup_does_not_corrupt_a_neighbouring_split_window",
    },
    {
        "label": "S11 buffer_pos_at's past-end fallback stops being window-scoped",
        "file": "crates/core/src/redisplay.rs",
        "old": "            .find(|w| row >= w.row && row < w.row + w.rows && col >= w.col && col < w.col + w.cols)",
        "new": "            .find(|w| false && row >= w.row && row < w.row + w.rows && col >= w.col && col < w.col + w.cols)",
        "test": "header_cleanup_does_not_corrupt_a_neighbouring_split_window",
    },
    # ---- F2, the breadcrumb -----------------------------------------
    {
        "label": "S12 the breadcrumb never reaches the mode line (deletion entry: F2)",
        "file": "crates/core/src/redisplay.rs",
        # The four `compose_mode_line` unit tests build `ModeLineParts`
        # themselves and so cannot see a severed wire -- this is the exact
        # gap M117 was written about, and this entry is what proves the
        # integration test closes it.
        "old": "        breadcrumb: &breadcrumb,",
        "new": '        breadcrumb: "",',
        "test": "breadcrumb_actually_appears_on_the_rendered_mode_line_row",
    },
    {
        "label": "S13 the breadcrumb is no longer dropped first under width pressure",
        "file": "crates/core/src/redisplay.rs",
        # The two-line form of this is not unique -- the `dir` segment above
        # is written identically. The `p.breadcrumb` line is what pins it.
        "old": '        let seg = format!("  {}", ml_sanitize(p.breadcrumb));\n        let seg_w = ml_width(&seg);\n        if used + seg_w <= width {',
        "new": '        let seg = format!("  {}", ml_sanitize(p.breadcrumb));\n        let seg_w = ml_width(&seg);\n        if true {',
        "test": "breadcrumb_is_dropped_before_the_mode_name_under_width_pressure",
        "test_target": None,
    },
    {
        "label": "S14 the breadcrumb cap keeps the outermost four instead of the innermost",
        "file": "crates/core/src/redisplay.rs",
        "old": "    let start = labels.len().saturating_sub(4);",
        "new": "    let start = 0;",
        "test": "breadcrumb_six_element_chain_is_capped_to_the_innermost_four",
        "test_target": None,
    },
    # ---- label extraction (Verilog is the priority language) ---------
    {
        "label": "S15 declaration_name loses the header-child fallback module names live in",
        "file": "crates/core/src/scope.rs",
        # `node-types.json` claims `module_declaration` carries a `name`
        # field; at runtime it does not -- the name sits under the
        # `module_ansi_header` child. This entry is what keeps that
        # dump-verified fact from being refactored away on the strength of
        # the schema.
        "old": "    let mut cursor = node.walk();\n    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();\n    for child in children {\n        if let Some(n) = child.child_by_field_name(\"name\") {\n            return text(&n, src).to_string();\n        }\n    }\n",
        "new": "",
        "test": "verilog_soc_top_module_and_instantiation",
        "test_target": None,
    },
    {
        "label": "S16 always_ff / always_comb collapse to a bare 'always' label",
        "file": "crates/core/src/scope.rs",
        "old": '        "always_construct" => find_child_kind(node, "always_keyword")\n            .map(|n| text(&n, src).to_string())\n            .unwrap_or_else(|| "always".to_string()),',
        "new": '        "always_construct" => "always".to_string(),',
        "test": "verilog_always_ff_and_case_label_in_alu",
        "test_target": None,
    },
]
