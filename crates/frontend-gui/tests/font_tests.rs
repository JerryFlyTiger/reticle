// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! M105: user-selectable GUI font. Guards the five font files `lib.rs`
//! embeds with `include_bytes!` (same pattern `icon_tests.rs` uses for the
//! window icon), the embedded-vs-disk resolution rule
//! (`jetbrains-mono`/`fira-code` never touch the filesystem; `sf-mono`
//! always does), and `build_font_definitions`'s machine-independent
//! result for the two bundled families. Ligature behavior for both
//! bundled fonts is asserted against the actual measurement recorded in
//! `crates/frontend-gui/src/shaping.rs`'s module doc, not against a
//! hoped-for result.

use std::sync::Arc;

use ab_glyph::Font as _;
use frontend_gui::shaping::{
    shape_calt_only, FaceRole, GlyphAtlasCache, LoadedFont, ShapeCache, ShapingFace,
};
use frontend_gui::{
    apply_font_switch, build_font_definitions, embedded_font_bytes, font_switch_needed,
    variant_file_name, FontSet,
};

/// Guards every test in this file that either mutates the process-global
/// `$HOME` environment variable or spawns a subprocess
/// (`gen-third-party-licenses.py`, which shells out to `cargo metadata`
/// and could plausibly consult `$HOME`) -- `cargo test`'s default
/// multi-threaded runner executes `#[test]` functions concurrently in the
/// SAME process, and env vars are process-wide state, not per-thread.
/// Without this, a test that temporarily points `$HOME` at a scratch
/// directory (see `gui_font_family_override_succeeds_via_a_real_disk_file`)
/// could race against another test's subprocess spawn or filesystem
/// search that happens to read `$HOME` mid-mutation.
static ENV_MUTATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Mirrors `lib.rs`'s own embedded constants byte-for-byte -- if the
// `include_bytes!` path there ever points somewhere else, this and
// `lib.rs` diverge and the length/content comparisons below catch it,
// same reasoning as `icon_tests.rs`'s copy of `ICON_PNG_BYTES`.
const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
const JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf");
const JETBRAINS_MONO_ITALIC: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf");
const FIRA_CODE_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/FiraCode-Regular.ttf");
const FIRA_CODE_BOLD: &[u8] = include_bytes!("../../../assets/fonts/FiraCode-Bold.ttf");

/// `(file name, embedded bytes, expected on-disk length)` for every
/// embedded font. The on-disk lengths are pinned to the exact byte counts
/// recorded when these files were vendored (see the M105 task spec) --
/// asserting an exact number, not just "> 0", so a corrupted or
/// wrong-version re-vendor of the same file name is caught too.
fn embedded_fixtures() -> [(&'static str, &'static [u8], usize); 5] {
    [
        ("JetBrainsMono-Regular.ttf", JETBRAINS_MONO_REGULAR, 273_900),
        ("JetBrainsMono-Bold.ttf", JETBRAINS_MONO_BOLD, 277_828),
        ("JetBrainsMono-Italic.ttf", JETBRAINS_MONO_ITALIC, 276_840),
        ("FiraCode-Regular.ttf", FIRA_CODE_REGULAR, 289_624),
        ("FiraCode-Bold.ttf", FIRA_CODE_BOLD, 319_368),
    ]
}

#[test]
fn embedded_font_bytes_match_the_vendored_file_sizes_exactly() {
    for (name, bytes, expected_len) in embedded_fixtures() {
        assert_eq!(
            bytes.len(),
            expected_len,
            "{name}: embedded byte count drifted from the vendored file"
        );
        // Same length as what's actually on disk right now, not just the
        // hard-coded fixture number above -- catches `include_bytes!`
        // pointing at a stale copy while `assets/fonts/` itself has moved
        // on.
        let disk_path = format!("{}/../../assets/fonts/{name}", env!("CARGO_MANIFEST_DIR"));
        let disk_len = std::fs::metadata(&disk_path)
            .unwrap_or_else(|e| panic!("{disk_path}: {e}"))
            .len() as usize;
        assert_eq!(
            bytes.len(),
            disk_len,
            "{name}: embedded length disagrees with the file currently on disk"
        );
    }
}

#[test]
fn embedded_font_bytes_all_parse_as_valid_ttf() {
    for (name, bytes, _) in embedded_fixtures() {
        ab_glyph::FontArc::try_from_vec(bytes.to_vec())
            .unwrap_or_else(|e| panic!("{name} failed to parse as a font: {e:?}"));
    }
}

#[test]
fn embedded_font_bytes_lookup_resolves_exactly_the_five_bundled_names() {
    for (name, bytes, _) in embedded_fixtures() {
        let resolved = embedded_font_bytes(name)
            .unwrap_or_else(|| panic!("{name} must resolve via embedded_font_bytes"));
        assert_eq!(resolved, bytes, "{name}: resolved bytes must be identical");
    }
    // Fira Code ships no italic upstream (only Bold/Light/Medium/Regular/
    // Retina/SemiBold) -- there is deliberately no embedded entry for it,
    // and the name must not silently fall through to disk either (a
    // machine that happens to have some unrelated FiraCode-Italic.ttf
    // installed must not be picked up).
    assert_eq!(embedded_font_bytes("FiraCode-Italic.ttf"), None);
    // A name outside the five embedded files (the disk-searched
    // candidates) must not resolve to embedded bytes.
    assert_eq!(embedded_font_bytes("SFNSMono.ttf"), None);
    assert_eq!(embedded_font_bytes("Menlo.ttc"), None);
    assert_eq!(embedded_font_bytes("Monaco.ttf"), None);
}

#[test]
fn variant_file_name_finds_fira_codes_bold_but_not_an_italic() {
    let bold = variant_file_name("FiraCode-Regular.ttf", "Bold");
    assert_eq!(bold.as_deref(), Some("FiraCode-Bold.ttf"));
    assert_eq!(
        embedded_font_bytes(&bold.unwrap()),
        Some(FIRA_CODE_BOLD),
        "FiraCode-Bold.ttf must resolve to the embedded bytes"
    );

    let italic = variant_file_name("FiraCode-Regular.ttf", "Italic");
    assert_eq!(italic.as_deref(), Some("FiraCode-Italic.ttf"));
    assert_eq!(
        embedded_font_bytes(&italic.unwrap()),
        None,
        "FiraCode-Italic.ttf has no embedded bytes -- there is no such file upstream"
    );
}

/// `build_font_definitions` for `jetbrains-mono` (the documented default,
/// `gui-font`'s default value in `gui.el`) must produce the same result
/// on every machine, since both bytes it needs are embedded. This is
/// exactly the test the pre-M105 "known gap" comment on `install_fonts`
/// said could not be written without either bundling font files or
/// hard-coding what's on the CI machine -- M105 did the former.
#[test]
fn build_font_definitions_for_jetbrains_mono_is_machine_independent() {
    let (have_bold, have_italic, font_set, fonts) = build_font_definitions(None, "jetbrains-mono");

    let primary = fonts
        .font_data
        .get("primary-0")
        .expect("jetbrains-mono must be loaded as the first primary candidate");
    assert_eq!(
        primary.font.as_ref(),
        JETBRAINS_MONO_REGULAR,
        "primary-0 must be exactly the embedded JetBrains Mono Regular bytes"
    );

    // Same-family bold/italic must resolve to the embedded variants, not
    // to Menlo's face-index fallback -- `have_bold`/`have_italic` being
    // true here is not itself machine-independent evidence (Menlo.ttc
    // could also produce `true`), so check the FontSet's actual bytes.
    assert!(have_bold, "JetBrains Mono ships a Bold face");
    assert!(have_italic, "JetBrains Mono ships an Italic face");
    let bold_bytes = fonts
        .font_data
        .get("family-bold")
        .expect("family-bold key must be present when have_bold is true");
    assert_eq!(bold_bytes.font.as_ref(), JETBRAINS_MONO_BOLD);
    let italic_bytes = fonts
        .font_data
        .get("family-italic")
        .expect("family-italic key must be present when have_italic is true");
    assert_eq!(italic_bytes.font.as_ref(), JETBRAINS_MONO_ITALIC);

    assert_font_set_uses_embedded_bytes(&font_set, JETBRAINS_MONO_REGULAR, "regular");
}

/// Same as the JetBrains Mono test above, but for `fira-code`, and
/// additionally confirming Fira Code's missing italic does not panic and
/// does not silently claim to be an embedded italic -- `have_italic` may
/// legitimately be `true` here (Menlo.ttc filling the gap on a machine
/// that has it), but if so `family-italic`'s bytes must be Menlo's, never
/// Fira Code's own (which don't exist).
#[test]
fn build_font_definitions_for_fira_code_has_no_embedded_italic() {
    let (have_bold, have_italic, font_set, fonts) = build_font_definitions(None, "fira-code");

    let primary = fonts
        .font_data
        .get("primary-0")
        .expect("fira-code must be loaded as the first primary candidate when selected");
    assert_eq!(
        primary.font.as_ref(),
        FIRA_CODE_REGULAR,
        "primary-0 must be exactly the embedded Fira Code Regular bytes"
    );

    assert!(have_bold, "Fira Code ships a Bold face");
    let bold_bytes = fonts
        .font_data
        .get("family-bold")
        .expect("family-bold key must be present when have_bold is true");
    assert_eq!(bold_bytes.font.as_ref(), FIRA_CODE_BOLD);

    if have_italic {
        // Only possible via the Menlo.ttc fallback on this machine, which
        // uses the "menlo-italic" key (not "family-italic" -- that key is
        // only ever inserted by the same-family-variant path, which
        // cannot fire here since there is no FiraCode-Italic.ttf to find).
        let italic_bytes = fonts
            .font_data
            .get("menlo-italic")
            .expect("have_italic true for fira-code must mean the Menlo.ttc fallback fired");
        assert_ne!(
            italic_bytes.font.as_ref(),
            FIRA_CODE_REGULAR,
            "a Fira Code italic cannot exist -- this must be Menlo's fallback, not Fira Code's"
        );
        assert!(
            !fonts.font_data.contains_key("family-italic"),
            "family-italic must never be populated for fira-code -- no such file exists"
        );
    }

    assert_font_set_uses_embedded_bytes(&font_set, FIRA_CODE_REGULAR, "regular");
}

/// `gui-font` picking `sf-mono` must never resolve via embedded bytes --
/// it is Apple's own font, never bundled. `build_font_definitions` must
/// still return a usable result (falling through to whichever of
/// JetBrains Mono/Fira Code/Menlo/Monaco is found), never panicking, on a
/// machine without SF Mono installed.
/// `gui-font` selecting `sf-mono` must resolve to whichever of the two
/// mutually-exclusive correct outcomes actually applies on the machine
/// running this test -- NOT to a fixed "must never equal the embedded
/// bytes" assertion, which is only true when SF Mono happens to be
/// installed (cold review, M105 fix round: the previous version of this
/// test was green ONLY because the dev machine it was written on has SF
/// Mono, which is exactly the "depends on what's on this machine" trap
/// `install_fonts`'s own doc comment says this project refuses to build
/// tests around). `find_font` is the same disk search `install_fonts`
/// itself performs, so this test asserts the real product-code
/// precondition rather than guessing at it.
#[test]
fn build_font_definitions_for_sf_mono_resolves_correctly_either_way() {
    let (_, _, _, fonts) = build_font_definitions(None, "sf-mono");
    let primary = fonts
        .font_data
        .get("primary-0")
        .expect("sf-mono selection must still resolve SOME primary font");

    match frontend_gui::find_font("SFNSMono.ttf") {
        Some(disk_bytes) => {
            assert_eq!(
                primary.font.as_ref(),
                disk_bytes.as_slice(),
                "SF Mono is installed on this machine -- primary-0 must be its actual bytes"
            );
        }
        None => {
            assert_eq!(
                primary.font.as_ref(),
                JETBRAINS_MONO_REGULAR,
                "SF Mono is NOT installed on this machine -- sf-mono must correctly fall \
                 back to the embedded JetBrains Mono, the next candidate in \
                 font_search_order(\"sf-mono\")"
            );
        }
    }

    // Whatever was actually found, the monospace family must be
    // non-empty -- the grid always has SOME font to draw with.
    let mono = fonts
        .families
        .get(&eframe::egui::FontFamily::Monospace)
        .expect("Monospace family must always be populated");
    assert!(
        !mono.is_empty(),
        "sf-mono selection must still leave a usable fallback chain"
    );
}

/// `gui-font-family` (an explicit file name override) must NEVER be
/// satisfied by embedded bytes, even when the name it names happens to
/// match one of the five files this project bundles -- e.g. a user who
/// has a Nerd-Font-patched build installed under the exact same file
/// name (cold review, M105 fix round: the previous version of
/// `resolve_font` was used for every candidate including this one, so a
/// `gui-font-family` set to `"FiraCode-Bold.ttf"` silently returned this
/// project's own bundled copy instead of searching the disk for the
/// user's file, contradicting `gui-font-family`'s own docstring in
/// `gui.el`).
///
/// This test's precondition -- `FiraCode-Bold.ttf` is NOT present under
/// any of `font_dirs()` on the machine running it -- is asserted first,
/// so a future machine that happens to have such a file makes this test
/// fail loudly (precondition violated) rather than silently pass for the
/// wrong reason.
#[test]
fn gui_font_family_override_never_resolves_via_embedded_bytes() {
    assert_eq!(
        frontend_gui::find_font("FiraCode-Bold.ttf"),
        None,
        "test precondition: FiraCode-Bold.ttf must not actually be on disk here, \
         or this test would not distinguish embedded-bytes leakage from a real disk hit"
    );

    let (_, _, _, fonts) = build_font_definitions(Some("FiraCode-Bold.ttf"), "jetbrains-mono");

    // The override missed (nothing on disk), so `primary-0` must be
    // whatever the built-in candidate chain finds next -- jetbrains-mono
    // is the default candidate order's first entry, and it IS embedded,
    // so this is the correct, expected fallback.
    let primary = fonts
        .font_data
        .get("primary-0")
        .expect("a missed gui-font-family override must still fall through to a built-in");
    assert_eq!(
        primary.font.as_ref(),
        JETBRAINS_MONO_REGULAR,
        "primary-0 must be JetBrains Mono (the built-in fallback), not the missed override"
    );

    // The bug this test guards against: the override's OWN bytes must
    // never come from `embedded_font_bytes`, i.e. `FIRA_CODE_BOLD` must
    // not appear anywhere in the resulting font_data at all (it would
    // only be able to appear via the override slot, since `jetbrains-mono`
    // is the selected named font here, not `fira-code`).
    for (key, data) in fonts.font_data.iter() {
        assert_ne!(
            data.font.as_ref(),
            FIRA_CODE_BOLD,
            "{key}: gui-font-family=\"FiraCode-Bold.ttf\" must never resolve to this \
             project's own embedded Fira Code Bold bytes when the file isn't on disk"
        );
    }
}

/// The companion to `gui_font_family_override_never_resolves_via_embedded_bytes`
/// above: that test's precondition is that the override ALWAYS misses on
/// disk, so `regular_is_override` (the flag `build_font_definitions`
/// uses to route the override's lookups through `find_font` instead of
/// `resolve_font`) was never actually `true` anywhere in this test suite
/// -- the override-SUCCEEDS branch had no coverage at all (cold review,
/// M105 third fix round).
///
/// MUTATION-TESTING LESSON (M105 fourth fix round -- kept here so nobody
/// "fixes" the fixture name back): an earlier version of this test used
/// `Something-Regular.ttf`/`Something-Bold.ttf`, names chosen specifically
/// to NOT collide with anything embedded. That was wrong for what this
/// test needs to prove. `dev/mutations/m105.py`'s mutation run found it:
/// deleting the `regular_is_override` guard in the same-family variant
/// search (making it call `resolve_font` unconditionally, the same as
/// the primary-font bug this milestone's earlier fix round already
/// caught) left this test SURVIVING, because `Something-Bold.ttf` isn't
/// one of the five embedded files -- `resolve_font`'s embedded lookup
/// misses regardless of the guard, so `find_font` runs either way and
/// both code paths land on the identical disk read. A non-colliding
/// fixture name cannot distinguish "the guard ran" from "the guard was
/// deleted", because there is nothing embedded for it to wrongly prefer.
///
/// The fixture below therefore uses `FiraCode-Regular.ttf`/
/// `FiraCode-Bold.ttf` -- names that DO collide with two of the five
/// embedded files -- specifically so that removing the guard changes the
/// observed bytes: `family-bold` would silently become this project's
/// own embedded `FIRA_CODE_BOLD` instead of the disk file's fake content.
/// `$HOME` is temporarily pointed at a scratch directory containing
/// `Library/Fonts/FiraCode-Regular.ttf`/`FiraCode-Bold.ttf`, whose
/// contents are deliberately recognizable, wrong-looking bytes (this code
/// path never parses font files, it only reads and forwards raw bytes,
/// so garbage content is fine and, here, load-bearing: it is what makes
/// "disk bytes" and "embedded bytes" distinguishable by content).
/// Confirms both (a) the primary font is the disk file's own bytes, not
/// `FIRA_CODE_REGULAR`, and (b) the same-family variant search
/// (`FiraCode-Bold.ttf`) also went to disk, not to `embedded_font_bytes`
/// -- i.e. not `FIRA_CODE_BOLD`.
#[test]
fn gui_font_family_override_succeeds_and_wins_over_a_colliding_embedded_name() {
    let _guard = ENV_MUTATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let original_home = std::env::var_os("HOME");

    let scratch_home = std::env::temp_dir().join(format!(
        "reticle_m105_font_family_override_collision_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fonts_dir = scratch_home.join("Library/Fonts");
    std::fs::create_dir_all(&fonts_dir).expect("create scratch Library/Fonts");
    let disk_regular_bytes: &[u8] =
        b"FAKE DISK BYTES, NOT THE REAL FIRA CODE REGULAR -- if you see \
         this in a test failure the disk file's content leaked into the wrong place";
    let disk_bold_bytes: &[u8] =
        b"FAKE DISK BYTES, NOT THE REAL FIRA CODE BOLD -- if you see this \
         in a test failure the same-family variant search used embedded bytes instead of disk";
    std::fs::write(fonts_dir.join("FiraCode-Regular.ttf"), disk_regular_bytes)
        .expect("write scratch FiraCode-Regular.ttf");
    std::fs::write(fonts_dir.join("FiraCode-Bold.ttf"), disk_bold_bytes)
        .expect("write scratch FiraCode-Bold.ttf");

    // SAFETY (in the "this is risky, not the unsafe keyword" sense):
    // `ENV_MUTATION_LOCK` above serializes this against every other test
    // in this file that could observe `$HOME` mid-mutation. `HOME` is
    // restored and the scratch directory removed unconditionally below,
    // via `catch_unwind`, even if an assertion inside panics.
    std::env::set_var("HOME", &scratch_home);
    let result = std::panic::catch_unwind(|| {
        let (have_bold, _have_italic, _font_set, fonts) =
            build_font_definitions(Some("FiraCode-Regular.ttf"), "jetbrains-mono");

        let primary = fonts
            .font_data
            .get("primary-0")
            .expect("gui-font-family pointing at a real disk file must resolve as primary-0");
        assert_eq!(
            primary.font.as_ref(),
            disk_regular_bytes,
            "primary-0 must be exactly the disk file's own bytes"
        );
        assert_ne!(
            primary.font.as_ref(),
            FIRA_CODE_REGULAR,
            "primary-0 must NOT be this project's embedded Fira Code Regular -- the disk \
             file the user pointed gui-font-family at must win, even though its name \
             collides with a bundled font"
        );

        assert!(
            have_bold,
            "FiraCode-Bold.ttf sits next to the regular face on disk and must be found"
        );
        let bold = fonts
            .font_data
            .get("family-bold")
            .expect("family-bold must be populated when have_bold is true");
        assert_eq!(
            bold.font.as_ref(),
            disk_bold_bytes,
            "family-bold must be the disk file's own bytes -- the same-family variant \
             search must go to disk (regular_is_override), not to embedded_font_bytes. \
             This is the assertion the fixture-name collision exists to make meaningful: \
             deleting the regular_is_override guard makes this become FIRA_CODE_BOLD instead."
        );
        assert_ne!(
            bold.font.as_ref(),
            FIRA_CODE_BOLD,
            "family-bold must NOT be this project's embedded Fira Code Bold -- this is \
             exactly the mutation (deleting the regular_is_override guard) this test exists \
             to catch"
        );
    });

    match original_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    std::fs::remove_dir_all(&scratch_home).ok();

    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// An unrecognized `gui-font` value (a typo in a user's init.el, or a
/// symbol from a future version) must fall back to the documented
/// default (`jetbrains-mono`), not panic and not silently produce an
/// empty font chain.
#[test]
fn build_font_definitions_treats_an_unknown_named_font_as_jetbrains_mono() {
    let (_, _, _, fonts) = build_font_definitions(None, "not-a-real-font-choice");
    let primary = fonts
        .font_data
        .get("primary-0")
        .expect("an unrecognized gui-font value must still resolve a primary font");
    assert_eq!(primary.font.as_ref(), JETBRAINS_MONO_REGULAR);
}

fn assert_font_set_uses_embedded_bytes(font_set: &FontSet, expected: &[u8], role: &str) {
    let regular = font_set
        .regular()
        .unwrap_or_else(|| panic!("FontSet.{role} must be populated"));
    assert_eq!(
        regular.loaded.bytes.as_ref(),
        expected,
        "FontSet's shaping-ready {role} face must carry the same bytes as font_data"
    );
}

// --- Ligatures -------------------------------------------------------------
//
// Measured 2026-09-04 by parsing FiraCode-Regular.ttf's GSUB table
// directly (table offsets read by hand, same technique
// `PLAN.md`'s M87/stage-2b record used for JetBrainsMono-Regular.ttf):
// Fira Code's `calt` feature references 101 lookups, one of which
// (lookup index 260) is GSUB lookup type 2 (Multiple Substitution) --
// JetBrains Mono's `calt` uses ONLY type 6 (chained context). So the
// "calt only ever uses GSUB type 6" premise `shaping.rs`'s module doc
// states does NOT hold universally for Fira Code; it was measured
// against one font, not proven as a property of the `calt` feature in
// general.
//
// That said, actually shaping the ligature sequences this project draws
// (`<=`, `!=`, `=>`, `|->`, `-->`, plus `==`, `>=`, `===`, `&&`, `||`,
// `::`, `/*`, `*/`) against FiraCode-Regular.ttf with rustybuzz produces
// glyph_count == char_count with a uniform per-character advance for
// every one of them (verified with a one-off rustybuzz probe outside the
// repo, per this milestone's task spec) -- `validate_shaped_run` passes,
// so these specific sequences DO render as ligatures on Fira Code too.
// The type-2 lookup evidently never fires for this project's supported
// sequences. An untested character sequence could in principle hit that
// lookup and fail validation -- but that is not a regression: it is the
// exact fallback (per-character drawing) this milestone's own safety net
// exists to provide for any font, including JetBrains Mono, for any
// sequence it wasn't measured against.
fn shape(bytes: &'static [u8], text: &str) -> Option<Vec<frontend_gui::shaping::ShapedGlyph>> {
    let face = LoadedFont {
        bytes: std::sync::Arc::from(bytes),
        index: 0,
    };
    let mut cache = ShapeCache::default();
    cache
        .get_or_shape(FaceRole::Regular, &face, text, true)
        .map(|g| g.to_vec())
}

#[test]
fn jetbrains_mono_calt_ligatures_shape_to_one_glyph_per_character() {
    for text in ["<=", "!=", "=>", "|->", "-->"] {
        let shaped = shape(JETBRAINS_MONO_REGULAR, text);
        assert!(
            shaped.is_some(),
            "JetBrains Mono must shape {text:?} without failing validate_shaped_run"
        );
        assert_eq!(
            shaped.unwrap().len(),
            text.chars().count(),
            "{text:?}: shaped glyph count must equal character count (one glyph per cell)"
        );
    }
}

#[test]
fn fira_code_calt_ligatures_also_shape_successfully_despite_the_type_2_lookup() {
    for text in [
        "<=", "!=", "=>", "|->", "-->", "==", ">=", "===", "&&", "||", "::", "/*", "*/",
    ] {
        let shaped = shape(FIRA_CODE_REGULAR, text);
        assert!(
            shaped.is_some(),
            "Fira Code must shape {text:?} without failing validate_shaped_run \
             (see this file's ligature comment: the type-2 lookup in its calt \
             feature does not fire for this project's supported sequences)"
        );
        assert_eq!(
            shaped.unwrap().len(),
            text.chars().count(),
            "{text:?}: shaped glyph count must equal character count (one glyph per cell)"
        );
    }
}

// --- M116: gui-ligatures (review fix: nothing before this proved the
// flag changes shaping at all -- every existing call site in this file
// passes `true`, so hardcoding `enable_calt` back to always-1 inside
// `shape_calt_only` would have passed every test above it) ------------

#[test]
fn shape_calt_only_with_calt_disabled_differs_from_calt_enabled() {
    let face = LoadedFont {
        bytes: Arc::from(JETBRAINS_MONO_REGULAR),
        index: 0,
    };
    let text = "assign a = b != c;";
    let with_calt = shape_calt_only(&face, text, true).expect("calt-enabled shaping must succeed");
    let without_calt =
        shape_calt_only(&face, text, false).expect("calt-disabled shaping must still succeed");
    assert_eq!(
        with_calt.len(),
        without_calt.len(),
        "disabling calt must not change how many glyphs a run occupies --          only which glyph each already-existing cell draws"
    );
    let with_ids: Vec<u16> = with_calt.iter().map(|g| g.glyph_id).collect();
    let without_ids: Vec<u16> = without_calt.iter().map(|g| g.glyph_id).collect();
    assert_ne!(
        with_ids, without_ids,
        "gui-ligatures off must actually change the glyph ids `!=` shapes to,          not just be a flag nothing reads"
    );
}

/// `ShapeCache`'s key includes `enable_calt` specifically so a toggle
/// never replays a stale ligature-shaped (or stale non-ligature) result
/// -- the exact staleness this milestone was designed against. Same
/// shape as `without_apply_font_switch_the_shape_cache_would_incorrectly_
/// collide` below/above: without `enable_calt` in the key, a lookup at
/// one setting would collide with an entry already cached under the
/// other setting.
#[test]
fn shape_cache_distinguishes_enable_calt() {
    let face = LoadedFont {
        bytes: Arc::from(JETBRAINS_MONO_REGULAR),
        index: 0,
    };
    let mut cache = ShapeCache::default();
    let calt_on = cache
        .get_or_shape(FaceRole::Regular, &face, "assign a = b != c;", true)
        .expect("calt-enabled shaping must succeed");
    let calt_off = cache
        .get_or_shape(FaceRole::Regular, &face, "assign a = b != c;", false)
        .expect("calt-disabled shaping must succeed");
    assert_ne!(
        calt_on.as_ref(),
        calt_off.as_ref(),
        "the same (role, text) pair under the two different enable_calt          settings must be two distinct cache entries, not one collapsing          into the other"
    );
}

// --- Runtime font switching (M105's only genuinely new mechanism) ----------
//
// `App::update` has no public constructor and nothing in this crate can
// drive a real `eframe::App::update` frame, so the switch check that used
// to live inline there had zero test coverage (cold review, M105 fix
// round): flipping `!=` to `==`, deleting the cache resets, or deleting
// the `active_font` update would all have passed every existing test.
// `font_switch_needed`/`apply_font_switch` are the pulled-out pure/near-
// pure pieces that make this testable without a live `egui::Context`.

#[test]
fn font_switch_needed_is_false_when_nothing_changed() {
    let active = ("jetbrains-mono".to_string(), None);
    let wanted = ("jetbrains-mono".to_string(), None);
    assert!(!font_switch_needed(&active, &wanted));
}

#[test]
fn font_switch_needed_is_true_when_the_named_font_changed() {
    let active = ("jetbrains-mono".to_string(), None);
    let wanted = ("fira-code".to_string(), None);
    assert!(font_switch_needed(&active, &wanted));
}

#[test]
fn font_switch_needed_is_true_when_only_font_family_changed() {
    let active = ("jetbrains-mono".to_string(), None);
    let wanted = ("jetbrains-mono".to_string(), Some("Menlo.ttc".to_string()));
    assert!(
        font_switch_needed(&active, &wanted),
        "gui-font-family alone changing (gui-font unchanged) must still trigger a switch"
    );
}

#[test]
fn font_switch_needed_is_false_when_font_family_is_the_same_some_value() {
    let active = ("jetbrains-mono".to_string(), Some("Menlo.ttc".to_string()));
    let wanted = ("jetbrains-mono".to_string(), Some("Menlo.ttc".to_string()));
    assert!(!font_switch_needed(&active, &wanted));
}

#[test]
fn apply_font_switch_updates_active_font_to_the_wanted_pair() {
    let mut shape_cache = ShapeCache::default();
    let mut glyph_cache = GlyphAtlasCache::default();
    let mut active = ("jetbrains-mono".to_string(), None);
    let wanted = ("fira-code".to_string(), Some("ignored.ttf".to_string()));
    apply_font_switch(
        &mut shape_cache,
        &mut glyph_cache,
        &mut active,
        wanted.clone(),
    );
    assert_eq!(active, wanted);
}

/// The behavioral regression `apply_font_switch`'s cache reset exists to
/// prevent: `ShapeCache` is keyed on `(FaceRole, text)`, which does not
/// encode which font produced the cached glyph ids. Shape `"=>"` against
/// JetBrains Mono through a cache, "switch" to Fira Code via
/// `apply_font_switch`, then shape the identical text again through the
/// SAME cache instance -- the result must be Fira Code's own glyph ids,
/// not JetBrains Mono's stale ones replayed from before the switch.
#[test]
fn apply_font_switch_resets_the_shape_cache_so_stale_glyph_ids_are_not_reused() {
    let jb_face = LoadedFont {
        bytes: std::sync::Arc::from(JETBRAINS_MONO_REGULAR),
        index: 0,
    };
    let fira_face = LoadedFont {
        bytes: std::sync::Arc::from(FIRA_CODE_REGULAR),
        index: 0,
    };

    let mut shape_cache = ShapeCache::default();
    let jb_shaped = shape_cache
        .get_or_shape(FaceRole::Regular, &jb_face, "=>", true)
        .expect("JetBrains Mono must shape \"=>\"");

    let mut glyph_cache = GlyphAtlasCache::default();
    let mut active = ("jetbrains-mono".to_string(), None);
    apply_font_switch(
        &mut shape_cache,
        &mut glyph_cache,
        &mut active,
        ("fira-code".to_string(), None),
    );

    let fira_shaped = shape_cache
        .get_or_shape(FaceRole::Regular, &fira_face, "=>", true)
        .expect("Fira Code must shape \"=>\"");

    assert_ne!(
        jb_shaped.as_ref(),
        fira_shaped.as_ref(),
        "after apply_font_switch resets shape_cache, shaping the same text against a \
         different font must NOT replay the previous font's cached glyph ids"
    );
}

/// Sanity check for the test above: WITHOUT going through
/// `apply_font_switch` at all, reusing the same `ShapeCache` instance
/// across two different fonts for identical text DOES collide on the
/// `(FaceRole, text)` key and replay stale glyph ids. This confirms the
/// test above is actually exercising `apply_font_switch`'s reset, not a
/// property that would have held regardless of it.
#[test]
fn without_apply_font_switch_the_shape_cache_would_incorrectly_collide() {
    let jb_face = LoadedFont {
        bytes: std::sync::Arc::from(JETBRAINS_MONO_REGULAR),
        index: 0,
    };
    let fira_face = LoadedFont {
        bytes: std::sync::Arc::from(FIRA_CODE_REGULAR),
        index: 0,
    };

    let mut shape_cache = ShapeCache::default();
    let jb_shaped = shape_cache
        .get_or_shape(FaceRole::Regular, &jb_face, "=>", true)
        .expect("JetBrains Mono must shape \"=>\"");
    // No reset -- same cache instance, same key, different font.
    let stale = shape_cache
        .get_or_shape(FaceRole::Regular, &fira_face, "=>", true)
        .expect("cache hit path must still return something");

    assert_eq!(
        jb_shaped.as_ref(),
        stale.as_ref(),
        "sanity: without a reset, (FaceRole, text) collides across fonts and replays \
         stale glyph ids -- this is exactly the bug apply_font_switch prevents"
    );
}

/// `apply_font_switch`'s `glyph_cache` reset IS independently verifiable
/// after all (M105 third fix round -- an earlier version of this file
/// claimed it was NOT, on the grounds that `get_or_rasterize` is private
/// to `shaping` with no live egui texture atlas reachable from a test.
/// That claim was wrong: `shaping` is `pub mod shaping` (this milestone
/// made it so), `build_shaped_mesh`/`finish_shaped_mesh`/`GlyphAtlasCache`
/// are all `pub`, and `egui::epaint::TextureAtlas::new` needs no live GUI
/// at all -- `shaping.rs`'s own `glyph_atlas_cache_clears_on_atlas_identity_change`
/// and `fix1_deferred_normalization_survives_mid_frame_atlas_growth` unit
/// tests already construct one exactly this way.
///
/// This drives the real rasterize-and-mesh path
/// (`build_shaped_mesh`/`finish_shaped_mesh`) for the SAME
/// `(FaceRole::Regular, glyph_id, scale)` key under two different
/// embedded fonts, through the SAME `GlyphAtlasCache` instance, with NO
/// reset in between: the second font's glyph is never rasterized at all
/// -- the cache hit replays the first font's UV rectangle verbatim, which
/// is exactly the "wrong glyph gets drawn" corruption this milestone's
/// `glyph_cache` reset exists to prevent. Then the same sequence is
/// repeated going through `apply_font_switch`'s reset, and the UV must
/// differ, since the second font's glyph is rasterized fresh into a
/// different atlas slot.
///
/// `glyph_id = 560` was chosen because both `JetBrainsMono-Regular.ttf`
/// and `FiraCode-Regular.ttf` have a real (non-empty) outline at that
/// numeric id -- confirmed with a one-off `ab_glyph` probe kept outside
/// the repo, not asserted here (any id both fonts render ink for would
/// do; asserting outline presence here would just be re-deriving the
/// same fact `get_or_rasterize` already checks internally).
#[test]
fn apply_font_switch_resets_the_glyph_atlas_cache_so_stale_uvs_are_not_reused() {
    use frontend_gui::shaping::{build_shaped_mesh, finish_shaped_mesh, ShapedGlyph};

    let jb_face =
        ab_glyph::FontArc::try_from_vec(JETBRAINS_MONO_REGULAR.to_vec()).expect("valid font");
    let fira_face =
        ab_glyph::FontArc::try_from_vec(FIRA_CODE_REGULAR.to_vec()).expect("valid font");

    const GLYPH_ID: u16 = 560;
    const SCALE: f32 = 16.0;
    const PPP: f32 = 1.0;
    let shaped = [ShapedGlyph {
        glyph_id: GLYPH_ID,
        x_offset: 0.0,
        y_offset: 0.0,
    }];

    let render = |atlas: &eframe::egui::epaint::mutex::Mutex<
        eframe::egui::epaint::TextureAtlas,
    >,
                  cache: &mut GlyphAtlasCache,
                  face: &ab_glyph::FontArc| {
        let items = build_shaped_mesh(
            atlas,
            cache,
            FaceRole::Regular,
            face,
            &shaped,
            0.0,
            0.0,
            10.0,
            SCALE,
            PPP,
        );
        assert_eq!(
            items.len(),
            1,
            "glyph {GLYPH_ID} must rasterize to real ink in this font at scale {SCALE}              (the probe that picked this id confirmed an outline in both fonts)"
        );
        let atlas_size = atlas.lock().size();
        let mesh = finish_shaped_mesh(&items, atlas_size, eframe::egui::Color32::WHITE);
        mesh.vertices[0].uv
    };

    // Phase 1: no reset. Same atlas, same cache, glyph 560 rasterized
    // under JetBrains Mono, then the "switch" to Fira Code reuses the
    // SAME cache with no reset in between.
    let atlas_no_reset =
        eframe::egui::epaint::mutex::Mutex::new(eframe::egui::epaint::TextureAtlas::new([
            1024, 1024,
        ]));
    let mut cache_no_reset = GlyphAtlasCache::default();
    cache_no_reset.begin_frame(&atlas_no_reset);
    let jb_uv = render(&atlas_no_reset, &mut cache_no_reset, &jb_face);
    let fira_uv_no_reset = render(&atlas_no_reset, &mut cache_no_reset, &fira_face);
    assert_eq!(
        jb_uv, fira_uv_no_reset,
        "sanity: without a reset, the same (FaceRole, glyph_id, scale) key collides across          fonts and replays the stale UV -- this is exactly the corruption the glyph_cache          reset in apply_font_switch exists to prevent"
    );

    // Phase 2: the real mechanism. Same starting point, but the "switch"
    // goes through `apply_font_switch`, which replaces `glyph_cache` with
    // a fresh `GlyphAtlasCache::default()` (the exact line under test).
    let atlas_with_reset =
        eframe::egui::epaint::mutex::Mutex::new(eframe::egui::epaint::TextureAtlas::new([
            1024, 1024,
        ]));
    let mut cache_with_reset = GlyphAtlasCache::default();
    cache_with_reset.begin_frame(&atlas_with_reset);
    let _jb_uv_2 = render(&atlas_with_reset, &mut cache_with_reset, &jb_face);

    let mut shape_cache = ShapeCache::default();
    let mut active = ("jetbrains-mono".to_string(), None);
    apply_font_switch(
        &mut shape_cache,
        &mut cache_with_reset,
        &mut active,
        ("fira-code".to_string(), None),
    );
    cache_with_reset.begin_frame(&atlas_with_reset);

    let fira_uv_after_reset = render(&atlas_with_reset, &mut cache_with_reset, &fira_face);
    assert_ne!(
        jb_uv, fira_uv_after_reset,
        "after apply_font_switch resets glyph_cache, the same (FaceRole, glyph_id, scale) \
         key must rasterize fresh, not replay JetBrains Mono's stale UV"
    );
}

// --- License registration ---------------------------------------------------

/// Regenerating `THIRD_PARTY_LICENSES.md` must reproduce the file
/// currently checked in byte-for-byte -- a substring check (the previous
/// version of this test) only confirms the GENERATOR still emits the
/// right strings, never that the COMMITTED file matches what the
/// generator would produce, so a hand-edit that drifted the two apart
/// would never be caught (cold review, M105 fix round). A full-content
/// comparison closes that gap. Confirmed by direct comparison before
/// writing this test that the two are identical bytes right now, so this
/// is not expected to fail as committed.
///
/// TRIPWIRE, not a font defect (M105 third fix round, recorded so the
/// next person doesn't chase the wrong thing): this test fails whenever
/// `Cargo.lock` changes (a dependency bump in ANY crate, for ANY reason,
/// in ANY future milestone) without `THIRD_PARTY_LICENSES.md` being
/// regenerated to match -- `gen-third-party-licenses.py` reads the whole
/// dependency graph via `cargo metadata`, so its output shifts (crate
/// counts, license-summary table) on every such change, entirely
/// unrelated to fonts. That is the intended, deliberate purpose of this
/// test (it is the only thing in the repo that would notice the drift),
/// but a red result here means "regenerate the licenses file," not
/// "something about JetBrains Mono/Fira Code broke."
#[test]
fn committed_third_party_licenses_matches_the_generators_output_exactly() {
    // Serialized against `gui_font_family_override_succeeds_via_a_real_disk_file`
    // (see `ENV_MUTATION_LOCK`'s doc): this spawns `cargo metadata` via
    // the generator script, which could plausibly consult `$HOME`, and
    // must never run while that test has `$HOME` pointed at a scratch
    // directory.
    let _guard = ENV_MUTATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let script = format!(
        "{}/../../dev/gen-third-party-licenses.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let committed_path = format!(
        "{}/../../THIRD_PARTY_LICENSES.md",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = std::process::Command::new("python3")
        .arg(&script)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {script}: {e}"));
    assert!(
        output.status.success(),
        "gen-third-party-licenses.py exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let generated = String::from_utf8_lossy(&output.stdout);
    let committed = std::fs::read_to_string(&committed_path)
        .unwrap_or_else(|e| panic!("failed to read {committed_path}: {e}"));
    assert_eq!(
        generated, committed,
        "THIRD_PARTY_LICENSES.md has drifted from `python3 dev/gen-third-party-licenses.py`'s \
         output -- regenerate it with that exact command rather than hand-editing"
    );
    // Still assert the specific content this milestone added, so a
    // failure here names what's missing rather than just "files differ".
    assert!(committed.contains("JetBrains Mono"));
    assert!(committed.contains("Fira Code"));
    assert!(committed.contains("OFL-1.1"));
}

// M106: `ShapingFace::new` used to build its rasterizing `ab_glyph::FontArc`
// with `try_from_vec`, which always parses face 0 of a font collection and
// silently ignores the collection index -- so a face selected by index
// (Fira Code's italic falls back to Menlo.ttc index 2; `sf-mono`'s bold is
// Menlo.ttc index 1) shaped correctly but rasterized as face 0 (regular,
// upright). These tests pin the fix against a synthesized `.ttc` built from
// two of this project's own bundled fonts (`assets/fonts/JetBrainsMono-
// Regular.ttf`/`JetBrainsMono-Italic.ttf`), never against a system font --
// machine-independent by construction, unlike a `~/Library/Fonts` lookup.

/// Rewrites `font`'s sfnt table-directory offsets by adding `base`, so the
/// unmodified table *data* can be placed starting at byte `base` within a
/// bigger buffer while every offset a reader follows from that table
/// directory still resolves correctly. Per the OpenType spec, a table
/// directory record's `offset` field is measured from the start of the
/// whole file, not from the start of that font's own data -- so simply
/// concatenating two standalone `.ttf` files under one `ttcf` header would
/// leave the second (and, once a header precedes it, even the first)
/// font's table offsets pointing at the wrong place. Only the top-level
/// table-directory offsets need adjusting; offsets recorded *within* a
/// table (e.g. `loca` entries into `glyf`) are relative to that table's own
/// start, not the file's, so they need no adjustment.
fn rebase_sfnt_table_offsets(font: &[u8], base: u32) -> Vec<u8> {
    let mut buf = font.to_vec();
    let num_tables = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    for i in 0..num_tables {
        let offset_field = 12 + i * 16 + 8;
        let orig = u32::from_be_bytes([
            buf[offset_field],
            buf[offset_field + 1],
            buf[offset_field + 2],
            buf[offset_field + 3],
        ]);
        buf[offset_field..offset_field + 4].copy_from_slice(&(orig + base).to_be_bytes());
    }
    buf
}

/// Synthesizes a minimal two-face `.ttc` (font collection) out of two
/// standalone sfnt byte buffers: a `ttcf` header (tag, version, `numFonts`,
/// then one `Offset32` per face) followed by each face's data, rebased with
/// [`rebase_sfnt_table_offsets`] so its table directory resolves correctly
/// at its new position in the combined buffer.
fn synth_ttc(face0: &[u8], face1: &[u8]) -> Vec<u8> {
    // Header: tag(4) + majorVersion(2) + minorVersion(2) + numFonts(4) +
    // one Offset32 per face(4 * 2) = 20 bytes.
    let header_len: u32 = 4 + 2 + 2 + 4 + 4 * 2;
    let base0 = header_len;
    let rebased0 = rebase_sfnt_table_offsets(face0, base0);
    let base1 = base0 + rebased0.len() as u32;
    let rebased1 = rebase_sfnt_table_offsets(face1, base1);

    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(&base0.to_be_bytes());
    out.extend_from_slice(&base1.to_be_bytes());
    out.extend_from_slice(&rebased0);
    out.extend_from_slice(&rebased1);
    out
}

/// Rasterizes `ch` through `face.ab` at a fixed scale and returns its pixel
/// bounds -- `None` if the glyph has no outline (shouldn't happen for a
/// plain ASCII letter in either fixture face).
fn rasterize_bounds(face: &frontend_gui::shaping::ShapingFace, ch: char) -> ab_glyph::Rect {
    let glyph_id = face.ab.glyph_id(ch);
    let glyph = glyph_id.with_scale(64.0);
    face.ab
        .outline_glyph(glyph)
        .expect("fixture glyph must have an outline at this scale")
        .px_bounds()
}

#[test]
fn shaping_face_rasterizes_the_face_its_index_selects_not_always_face_zero() {
    // Regular at index 0, Italic at index 1 of the synthesized collection.
    let ttc = synth_ttc(JETBRAINS_MONO_REGULAR, JETBRAINS_MONO_ITALIC);
    let ttc: Arc<[u8]> = Arc::from(ttc.into_boxed_slice());

    let face0 = ShapingFace::new(ttc.clone(), 0).expect("face 0 (regular) must load");
    let face1 = ShapingFace::new(ttc, 1).expect("face 1 (italic) must load");

    let bounds0 = rasterize_bounds(&face0, 'a');
    let bounds1 = rasterize_bounds(&face1, 'a');

    // Before the fix, `ShapingFace::new` built `ab` with `try_from_vec`,
    // which always parses face 0 regardless of the `index` argument -- so
    // `face1.ab` would have rasterized identically to `face0.ab`, and this
    // assertion would fail (the italic slant would never show up). Compare
    // the full bounds rect, not just one coordinate, so a fix that
    // happened to leave one axis unchanged wouldn't slip through.
    assert_ne!(
        bounds0, bounds1,
        "index 0 (regular) and index 1 (italic) of the same collection must rasterize \
         differently -- if they match, `ShapingFace::new` is still ignoring `index` and always \
         rasterizing face 0"
    );
}

#[test]
fn shaping_face_index_is_honored_for_a_non_default_face_too() {
    // Same collection, but this time index 1 is loaded as if it were "the
    // bold face" (mirrors how `lib.rs` loads Menlo.ttc index 1 for
    // `sf-mono`'s bold) -- the point is that ANY nonzero index must take
    // effect, not just index 2 (the italic slot Fira Code's fallback uses
    // in `lib.rs`).
    let ttc = synth_ttc(JETBRAINS_MONO_REGULAR, JETBRAINS_MONO_ITALIC);
    let ttc: Arc<[u8]> = Arc::from(ttc.into_boxed_slice());

    let regular = ShapingFace::new(ttc.clone(), 0).expect("face 0 must load");
    let other = ShapingFace::new(ttc, 1).expect("face 1 must load");

    let regular_bounds = rasterize_bounds(&regular, 'g');
    let other_bounds = rasterize_bounds(&other, 'g');
    assert_ne!(
        regular_bounds, other_bounds,
        "loading index 1 of the collection must rasterize a different face than index 0"
    );
}

#[test]
fn shaping_face_loaded_index_still_matches_the_face_used_for_shaping() {
    // Guards the actual invariant the bug violated: `loaded.index` (what
    // `shape_calt_only` shapes against) and `ab` (what rasterizing draws)
    // must resolve to the SAME face. Shape a plain run against index 1
    // (italic) and rasterize the returned glyph ids through `face1.ab`;
    // every glyph id must have a nonzero outline extent under that face --
    // if `ab` were still silently face 0, this would still likely produce
    // *some* outline (JetBrains Mono Regular and Italic share very similar
    // coverage), so this test is a coverage floor, not the fix's primary
    // guard (that's the two tests above) -- kept because it exercises the
    // exact code path `lib.rs` uses shaping and rasterizing together.
    let ttc = synth_ttc(JETBRAINS_MONO_REGULAR, JETBRAINS_MONO_ITALIC);
    let ttc: Arc<[u8]> = Arc::from(ttc.into_boxed_slice());
    let face1 = ShapingFace::new(ttc, 1).expect("face 1 must load");

    let shaped =
        shape_calt_only(&face1.loaded, "ab", true).expect("plain text must shape against face 1");
    for g in shaped {
        let glyph = ab_glyph::GlyphId(g.glyph_id).with_scale(64.0);
        assert!(
            face1.ab.outline_glyph(glyph).is_some(),
            "glyph id {} shaped against loaded.index=1 must have an outline when rasterized \
             through the same face's `ab`",
            g.glyph_id
        );
    }
}

/// Returns the horizontal shift, in pixels, between the leftmost ink column
/// of `ch`'s topmost rasterized row and its bottommost rasterized row (v >
/// 0.5 threshold, to ignore antialiasing fringe). A perfectly upright glyph
/// (no slant) has the same leftmost-ink column top and bottom, so this is
/// ~0; a slanted (italic) glyph's stem leans, so its topmost and bottommost
/// rows disagree on where the leftmost ink is.
fn slant_shift(face: &frontend_gui::shaping::ShapingFace, ch: char) -> i32 {
    let glyph_id = face.ab.glyph_id(ch);
    let glyph = glyph_id.with_scale(64.0);
    let outlined = face
        .ab
        .outline_glyph(glyph)
        .expect("fixture glyph must have an outline at this scale");
    let bounds = outlined.px_bounds();
    let height = bounds.height().ceil() as usize + 1;
    let mut row_min_x: Vec<Option<i32>> = vec![None; height];
    outlined.draw(|x, y, v| {
        if v > 0.5 {
            if let Some(row) = row_min_x.get_mut(y as usize) {
                let x = x as i32;
                if row.is_none_or(|current| x < current) {
                    *row = Some(x);
                }
            }
        }
    });
    let mut first = None;
    let mut last = None;
    for x in row_min_x.iter().flatten() {
        first.get_or_insert(*x);
        last = Some(*x);
    }
    last.expect("glyph must have at least one inked row") - first.expect("checked above")
}

#[test]
fn shaping_face_index_selects_exactly_the_bytes_identical_face_not_merely_a_different_one() {
    // Upgrades the "two faces differ" tests above to "index 1 selects
    // EXACTLY the italic face, not just something different from face 0" --
    // a broken implementation like `index.min(1)` (still picks SOME face
    // other than always-0 for a nonzero index, but the wrong one whenever
    // more than two faces are collected) would pass the earlier `assert_ne!`
    // tests but must fail this one on a 3+-face collection; here we prove
    // the same much cheaper way -- an equality check against the SAME face
    // loaded standalone -- rather than growing a 3-face fixture.
    let ttc = synth_ttc(JETBRAINS_MONO_REGULAR, JETBRAINS_MONO_ITALIC);
    let ttc: Arc<[u8]> = Arc::from(ttc.into_boxed_slice());
    let face0 = ShapingFace::new(ttc.clone(), 0).expect("face 0 must load");
    let face1 = ShapingFace::new(ttc, 1).expect("face 1 must load");

    let standalone_italic = ShapingFace::new(
        Arc::from(JETBRAINS_MONO_ITALIC.to_vec().into_boxed_slice()),
        0,
    )
    .expect("standalone italic file must load at its own index 0");

    let from_ttc_index1 = rasterize_bounds(&face1, 'a');
    let from_standalone_italic = rasterize_bounds(&standalone_italic, 'a');
    let from_ttc_index0 = rasterize_bounds(&face0, 'a');

    assert_eq!(
        from_ttc_index1, from_standalone_italic,
        "index 1 of the synthesized collection must rasterize IDENTICALLY to the same italic          file loaded standalone -- 'merely different from face 0' is not enough to prove index          1 selected the italic face specifically"
    );
    assert_ne!(
        from_ttc_index1, from_ttc_index0,
        "and it must still differ from face 0 (regular), or the equality check above would be          vacuously satisfied by two faces that both happen to equal face 0"
    );
}

#[test]
fn shaping_face_index_selects_a_face_that_is_actually_slanted_on_screen() {
    // Upgrades "the two faces rasterize to different byte patterns" to "the
    // difference is the specific one a user cares about: italic actually
    // leans". Regression-shaped test for the real M106 symptom (Fira
    // Code's comments rendering upright): a wrong fix that changed SOME
    // pixel (e.g. swapped hinting flags, or picked a differently-spaced but
    // still-upright face) would satisfy the earlier bounds-inequality
    // tests without ever producing a slanted glyph, and would not satisfy
    // this one. Uses 'I' rather than 'l': JetBrains Mono's 'l' carries a
    // top/bottom serif that already shifts its own leftmost-ink column
    // even in the regular face (measured shift 14px), which would make
    // this test's "regular must be upright" half spuriously fail; 'I' has
    // no such serif and measured shift 0px in the regular face.
    let ttc = synth_ttc(JETBRAINS_MONO_REGULAR, JETBRAINS_MONO_ITALIC);
    let ttc: Arc<[u8]> = Arc::from(ttc.into_boxed_slice());
    let regular = ShapingFace::new(ttc.clone(), 0).expect("face 0 (regular) must load");
    let italic = ShapingFace::new(ttc, 1).expect("face 1 (italic) must load");

    let regular_shift = slant_shift(&regular, 'I');
    let italic_shift = slant_shift(&italic, 'I');

    assert_eq!(
        regular_shift, 0,
        "the regular face's 'I' must rasterize with no top-to-bottom horizontal shift -- if          this fails, the fixture or the measurement method changed, not just the fix under test"
    );
    assert_ne!(
        italic_shift, 0,
        "the italic face's 'I' must rasterize with a nonzero top-to-bottom horizontal shift          (it must actually lean) -- if this is 0, `ShapingFace::new` selected a face that          differs from face 0 in some way other than slant, which is not what this milestone's          defect (Fira Code's italic comments rendering upright) needs fixed"
    );
}
