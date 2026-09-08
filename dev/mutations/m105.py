# Mutation list for M105 (user-selectable fonts), designed by the reviewer in
# step 4 plus the three entries covering the runtime-switch logic the fix round
# extracted so that it could be tested at all. Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m105.py \
#         -p frontend-gui --test-target font_tests
#
# The reviewer's own caveat, confirmed and worth keeping in view: this machine
# has JetBrains Mono installed under ~/Library/Fonts with byte-identical files
# to the vendored ones, so "embedded, never touches disk" is NOT provable here
# for that family -- E1 targets Fira Code, whose disk copy is absent.

PACKAGE = "frontend-gui"
TEST_TARGET = "font_tests"

MUTATIONS = [
    {
        "label": "E1 the bundled Fira Code bold is no longer resolvable as embedded",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        \"FiraCode-Bold.ttf\" => Some(FIRA_CODE_BOLD),",
        "new": "        \"FiraCode-Bold.ttf\" => None,",
        "test": "embedded_font_bytes_lookup_resolves_exactly_the_five_bundled_names",
    },
    {
        "label": "E2 the fira-code choice maps to the wrong family",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        \"fira-code\" => \"FiraCode-Regular.ttf\",",
        "new": "        \"fira-code\" => \"JetBrainsMono-Regular.ttf\",",
        "test": "build_font_definitions_for_fira_code_has_no_embedded_italic",
    },
    {
        "label": "E3 an unknown gui-font value no longer falls back to JetBrains Mono",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        _ => \"JetBrainsMono-Regular.ttf\",",
        "new": "        _ => \"FiraCode-Regular.ttf\",",
        "test": "build_font_definitions_treats_an_unknown_named_font_as_jetbrains_mono",
    },
    {
        "label": "S1 a font switch is never detected",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    active.0 != wanted.0 || active.1 != wanted.1",
        "new": "    false",
        "test": "font_switch_needed_is_true_when_the_named_font_changed",
    },
    {
        "label": "S2 a change to gui-font-family alone no longer counts as a switch",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    active.0 != wanted.0 || active.1 != wanted.1",
        "new": "    active.0 != wanted.0",
        "test": "font_switch_needed_is_true_when_only_font_family_changed",
    },
    {
        "label": "S3 switching fonts no longer clears the shaping cache (stale glyph ids)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    *shape_cache = ShapeCache::default();\n    *glyph_cache = GlyphAtlasCache::default();",
        "new": "    let _ = (&shape_cache, &glyph_cache);",
        "test": "apply_font_switch_resets_the_shape_cache_so_stale_glyph_ids_are_not_reused",
    },
    {
        "label": "A the glyph atlas cache survives a font switch (stale UVs, wrong glyphs drawn)",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    *glyph_cache = GlyphAtlasCache::default();",
        "new": "    let _ = &glyph_cache;",
        # The first round's docs claimed this reset was not observable from
        # outside `shaping`. The trailing review disproved that by building a
        # standalone TextureAtlas and showing the UVs collide without it.
        "test": "apply_font_switch_resets_the_glyph_atlas_cache_so_stale_uvs_are_not_reused",
    },
    {
        "label": "B a gui-font-family override is intercepted by the bundled bytes again",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        let found = if is_override {\n            find_font(name)\n        } else {\n            resolve_font(name)\n        };",
        "new": "        let found = resolve_font(name);",
        "test": "gui_font_family_override_never_resolves_via_embedded_bytes",
    },
    {
        "label": "C an override's bold/italic variants fall back to the bundled bytes",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "        if regular_is_override {\n            find_font(n)\n        } else {\n            resolve_font(n)\n        }",
        "new": "        resolve_font(n)",
        # Survived the first run: the fixture was named Something-*.ttf, which
        # is not one of the five bundled names, so "disk only" and "embedded
        # first, then disk" reached the same file. The fixture now collides
        # with FiraCode-*.ttf on purpose -- that is the case the guard exists for.
        "test": "gui_font_family_override_succeeds_and_wins_over_a_colliding_embedded_name",
    },
]
