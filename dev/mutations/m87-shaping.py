# M87 stage 2b -- text shaping, so coding ligatures render.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-shaping.py \
#         -p frontend-gui --test-target lib
#
# What this defends is not "does a ligature appear" -- that is visible in a
# screenshot and was measured against a pre-shaping build. It is the
# **guard rails**, which are invisible when they work and catastrophic
# when they don't.
#
# The milestone rests on a measurement: in JetBrains Mono, `calt` uses
# only chained-context lookups, so a coding ligature is one glyph per
# character with a uniform advance, and the character grid stays correct.
# That is true of that font. It is not a law. `gui-font-family` lets a
# user point this at any font on their machine, and the CJK fallback face
# could genuinely merge characters. If a font ever violates the
# assumption and nothing checks, the text silently stops lining up with
# the grid the rest of the editor -- cursor, selection, mouse hit-testing
# -- still believes in.
#
# So the validation is the load-bearing part, and each entry below
# removes one leg of it.
#
# All entries are replacement-style.

PACKAGE = "frontend-gui"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "S1 a shaped run with the wrong glyph count is accepted",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "    if glyph_count == 0 || glyph_count != char_count {\n        return false;\n    }",
        "new": "    if glyph_count == 0 {\n        return false;\n    }",
        "test": "validate_rejects_glyph_count_mismatch",
        "note": (
            "This is the check that a font merging characters -- the thing "
            "the whole grid model cannot represent -- is refused rather than "
            "drawn. Without it, a font with real ligature (GSUB type 4) "
            "lookups would produce fewer glyphs than cells and every "
            "character after the merge would sit one cell left of where the "
            "cursor, the selection and the mouse hit-test all think it is."
        ),
    },
    {
        "label": "S2 non-uniform advances are accepted",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "    advances.iter().all(|&a| a == cell_advance_units)",
        "new": "    true",
        "test": "validate_rejects_nonuniform_advance",
        "note": (
            "The other half of the same assumption: one glyph per character "
            "is not enough, each must also occupy exactly one cell. A "
            "proportional font passes the count check and fails this one. "
            "Note the code reads the cell advance from the face itself "
            "rather than hard-coding the 600/1000 em that the measurement "
            "happened to find."
        ),
    },
    {
        "label": "S3 the glyph atlas cache is never invalidated",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "        if self.atlas_identity != Some(identity) {\n            self.map.clear();\n            self.atlas_identity = Some(identity);\n        }",
        "new": "        let _ = identity;",
        "test": "glyph_atlas_cache_clears_on_atlas_identity_change",
        "note": (
            "`TextureAtlas::allocate` has no eviction, so epaint rebuilds "
            "the whole atlas when it fills, and again on a DPI or font "
            "change. Cached glyph-id -> UV entries point into the old "
            "texture at that moment. Keeping them draws whatever now "
            "occupies those coordinates: not a crash, not an error, just "
            "wrong glyphs. Pointer identity is the only signal available -- "
            "epaint exposes no generation counter."
        ),
    },
    {
        "label": "S4 the shape cache stops distinguishing bold from regular",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "        let key = (role, text.to_string());",
        "new": "        let key = (FaceRole::Regular, text.to_string());",
        "test": "shape_cache_distinguishes_face_role",
        "note": (
            "Shaping results are per-face: the same string shapes to "
            "different glyph ids in the regular, bold and italic faces. "
            "Collapsing the key would serve regular glyphs for bold text "
            "from the cache -- correct-looking text in the wrong weight, "
            "with no error anywhere. If the anchor no longer matches, the "
            "lookup was refactored; re-anchor rather than dropping the "
            "entry -- which is exactly what happened on the first run of "
            "this list: the anchor was guessed from the struct rather than "
            "read from the code, and the harness reported SKIP. SKIP is not "
            "a pass; it means the entry defended nothing that run."
        ),
    },
    {
        "label": "S5 `.notdef` glyphs pass validation and get drawn as tofu",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "    if glyph_ids.contains(&0) {\n        return false;\n    }",
        "new": "    if false {\n        return false;\n    }",
        "test": "validate_rejects_notdef_glyph_even_with_uniform_advance",
        "note": (
            "Shaping only ever consults the primary/bold/italic face; it has "
            "no equivalent of egui's fallback chain, where the CJK face is "
            "appended to every family. So a narrow character present only in "
            "the fallback font shapes to glyph id 0. In a monospace font "
            "`.notdef` plausibly carries the same advance as every other "
            "glyph, so the count and uniform-advance checks both pass and a "
            "tofu box is drawn where the pre-shaping path drew the real "
            "character. A silent visual regression, not an error."
        ),
    },
    {
        "label": "S6 UVs are normalised against a stale atlas size again",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "    let inv_w = 1.0 / atlas_size[0].max(1) as f32;\n    let inv_h = 1.0 / atlas_size[1].max(1) as f32;",
        "new": "    let inv_w = 1.0 / atlas_size[0].max(1) as f32;\n    let inv_h = 1.0 / 64.0f32;",
        "test": "fix1_deferred_normalization_survives_mid_frame_atlas_growth",
        "note": (
            "The highest-severity defect of this milestone, found by reading "
            "epaint's source rather than by running anything. "
            "`TextureAtlas::allocate` grows the atlas **in place**, doubling "
            "its height, so the `Arc` is unchanged and the pointer-identity "
            "check correctly does not fire -- that check guards full "
            "recreation, a different mechanism. Raw texel coordinates survive "
            "a resize; normalised UVs do not. And `Shape::Mesh` is appended "
            "as-is at tessellation time, unlike epaint's own text, whose UVs "
            "stay raw texels and are normalised once at the end of the "
            "frame.\n"
            "\n"
            "So every shaped glyph drawn before a mid-frame growth sampled "
            "the wrong region of the texture that actually got uploaded. "
            "Triggered by a cold start (the fresh atlas is 32px tall), a DPI "
            "change, or scrolling to unseen characters.\n"
            "\n"
            "Worth recording: the module's own doc comment already described "
            "the correct design -- normalise against the atlas size *at draw "
            "time* -- and the code hoisted that size once per frame instead. "
            "The comment was right and the code did something else, and "
            "nothing could see the gap."
        ),
    },
    {
        "label": "S7 the same-family variant filename matches the wrong occurrence",
        "file": "crates/frontend-gui/src/lib.rs",
        "old": "    let idx = base.rfind(\"Regular\")?;",
        "new": "    let idx = base.find(\"Regular\")?;",
        "test": "variant_file_name_ignores_an_earlier_regular_in_the_family_name",
        "note": (
            "Cosmetic in practice -- no font this project targets has "
            "'Regular' twice in its filename -- but the failure mode is a "
            "silent fallback to Menlo's bold, i.e. the mixed-family "
            "rendering this milestone set out to fix."
        ),
    },
    {
        "label": "S8 the forced left-to-right direction is removed",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "    buffer.set_direction(rustybuzz::Direction::LeftToRight);",
        "new": "    // mutation: let the direction be guessed from the script",
        "test": "fix3_forcing_left_to_right_overrides_guessed_backward_direction",
        "expect": "SURVIVED",
        "note": (
            "**Expected to SURVIVE, and the implementer said so before I ran "
            "it.** The test exercises the mechanism directly on a rustybuzz "
            "buffer, not through `shape_calt_only`, so removing the line "
            "from the real call site is invisible to it.\n"
            "\n"
            "The reason it could not be tested at the call site is worth "
            "keeping, because it is not laziness: reaching the code needs a "
            "script that guesses right-to-left, and every RTL-capable font "
            "on this machine is proportional, so the uniform-advance check "
            "rejects the run before direction matters -- while JetBrains "
            "Mono has no Hebrew coverage at all and now hits the `.notdef` "
            "rejection instead. The two guards that make RTL safe are "
            "exactly what makes RTL untestable here.\n"
            "\n"
            "So this entry documents a known blind spot rather than "
            "asserting coverage. If a monospace RTL-covering font is ever "
            "added to the fallback chain, this becomes testable and should "
            "be revisited."
        ),
    },
]
