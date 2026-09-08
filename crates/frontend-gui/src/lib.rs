// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! egui frontend (M16 modernization). Paint discipline: everything the
//! frame needs is computed before painting (the grid), text is drawn in
//! batched per-style runs for the ASCII bulk (per-char only for CJK,
//! whose cell advance differs from glyph advance), and the paint path
//! never calls elisp. Ground colors come from the theme's `default`
//! face, so `(load-theme 'light)` relights the whole frame live.

pub mod shaping;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Vec2};

use core::buffer::Buffer;
use core::commands::{handle_key, Key};
use core::editor::{buffer_local_value, Editor};
use core::keymap::{ctrl_encode, META};
use core::redisplay::{frame_base_style, render, Underline};
use elisp::Interp;
use shaping::{FaceRole, GlyphAtlasCache, ShapeCache, ShapingFace};

/// Fallbacks when no theme has set the `default` face (never in
/// practice — themes.el loads at startup).
const FALLBACK_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const FALLBACK_FG: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);

const BOLD_FAMILY: &str = "mono-bold";
const ITALIC_FAMILY: &str = "mono-italic";

/// The shaping-ready faces `install_fonts` loaded, alongside (not instead
/// of) the copies it handed to egui's `FontData` -- see `shaping`'s
/// module doc for why the GUI must keep its own copy. `bold`/`italic` are
/// `None` when neither a same-family variant nor Menlo.ttc was found on
/// disk, in which case the paint loop's shaping attempt for a bold/italic
/// run simply has nothing to shape with and falls back to `painter.text`,
/// same as it always has.
#[derive(Default)]
pub struct FontSet {
    regular: Option<ShapingFace>,
    bold: Option<ShapingFace>,
    italic: Option<ShapingFace>,
}

impl FontSet {
    /// The shaping-ready regular face, or `None` when nothing loaded
    /// (never happens in practice -- some primary candidate always
    /// resolves, at minimum to a built-in). Exposed read-only: nothing
    /// outside this module needs to construct or mutate a `FontSet`
    /// directly, only `font_tests.rs` needs to read `regular` back to
    /// confirm which bytes actually got loaded.
    pub fn regular(&self) -> Option<&ShapingFace> {
        self.regular.as_ref()
    }

    fn for_role(&self, role: FaceRole) -> Option<&ShapingFace> {
        match role {
            FaceRole::Regular => self.regular.as_ref(),
            FaceRole::Bold => self.bold.as_ref(),
            FaceRole::Italic => self.italic.as_ref(),
        }
    }
}

/// `JetBrainsMono-Regular.ttf` -> `JetBrainsMono-Bold.ttf` /
/// `JetBrainsMono-Italic.ttf`: the same-family variant naming convention
/// this milestone's bold/italic fix prefers over Menlo.ttc's face
/// indices. `None` when `base` doesn't follow the `*-Regular.*` pattern
/// (a font file named some other way, e.g. `Monaco.ttf`, which has no
/// separate bold/italic file to look for).
pub fn variant_file_name(base: &str, variant: &str) -> Option<String> {
    // Fix 7 (cold review): `replacen(.., 1)` replaces the FIRST
    // occurrence of "Regular". A family name that itself contains
    // "Regular" before the variant marker (e.g. a hypothetical
    // "RegularWidthMono-Regular.ttf") would then have the wrong
    // occurrence replaced and silently fall back to Menlo. The variant
    // marker is always the last "Regular" in the stem (immediately
    // before the extension), so find and replace that one instead.
    let idx = base.rfind("Regular")?;
    let mut result = String::with_capacity(base.len() - "Regular".len() + variant.len());
    result.push_str(&base[..idx]);
    result.push_str(variant);
    result.push_str(&base[idx + "Regular".len()..]);
    Some(result)
}

/// Fix D (trailing review): the window after input during which the
/// cursor is forced fully opaque (blink/fade suppressed) -- used by both
/// the wake-cadence pick (`update`'s `blink_suppressed`) and
/// `cursor_alpha`'s own suppression check. Previously each hard-coded its
/// own `Duration::from_millis(300)`; nothing kept the two literals in
/// sync, so tuning one without the other would desync "the cadence code
/// thinks the cursor is static" from "the cursor is actually animating"
/// (or the reverse).
const BLINK_SUPPRESS_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

/// The window icon, embedded at compile time. 256px is the largest size in
/// `assets/icon/png/` that still stays a trivial number of bytes to bake into
/// the binary (1024px is the master, but a taskbar/titlebar icon is never
/// shown anywhere near that large, and `with_icon` rescales down from
/// whatever it is given anyway).
///
/// This drives the window and taskbar icon on Linux and Windows. On macOS the
/// Dock icon is not sourced from here at all -- it comes from the `.app`
/// bundle's `Contents/Resources/reticle.icns`, set via `CFBundleIconFile` in
/// `Info.plist`, so this call is a no-op on that platform. That is why
/// `dev/make-app-bundle.sh` exists: it is the only thing that actually
/// changes what a macOS user sees in the Dock.
const ICON_PNG_BYTES: &[u8] = include_bytes!("../../../assets/icon/png/reticle-256.png");

/// M105: the two bundled body fonts, embedded at compile time the same
/// way `ICON_PNG_BYTES` is -- so a fresh checkout has a working default
/// font with nothing to install, and the result does not depend on
/// which fonts (if any) happen to be present on the machine this runs
/// on. Both are SIL Open Font License 1.1 (see `assets/fonts/*-OFL.txt`
/// and the "Fonts" entry `dev/gen-third-party-licenses.py` adds to
/// `THIRD_PARTY_LICENSES.md`), which permits redistribution as part of a
/// larger work.
///
/// Fira Code's upstream release ships no italic face at all (only
/// Bold/Light/Medium/Regular/Retina/SemiBold) -- there is deliberately
/// no `FIRA_CODE_ITALIC` constant. `embedded_font_bytes` returning `None`
/// for `"FiraCode-Italic.ttf"` is what lets `install_fonts`' existing
/// Menlo.ttc fallback fill that gap in, exactly as it already does for
/// any other font missing a same-family italic.
const JETBRAINS_MONO_REGULAR: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");
const JETBRAINS_MONO_BOLD: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf");
const JETBRAINS_MONO_ITALIC: &[u8] =
    include_bytes!("../../../assets/fonts/JetBrainsMono-Italic.ttf");
const FIRA_CODE_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/FiraCode-Regular.ttf");
const FIRA_CODE_BOLD: &[u8] = include_bytes!("../../../assets/fonts/FiraCode-Bold.ttf");

/// The embedded bytes for one of the five bundled font files by name, or
/// `None` when `name` isn't one of them -- the caller's cue to fall back
/// to `find_font`'s disk search instead. Embedded files are matched by
/// name and NEVER also looked up on disk, even when a same-named file
/// happens to exist there: `gui-font`'s `jetbrains-mono`/`fira-code`
/// choices must render identically regardless of what a given machine
/// has installed, which is also why the bundled OFL license files travel
/// with the binary via `dev/gen-third-party-licenses.py` rather than
/// this being "just another disk candidate".
pub fn embedded_font_bytes(name: &str) -> Option<&'static [u8]> {
    match name {
        "JetBrainsMono-Regular.ttf" => Some(JETBRAINS_MONO_REGULAR),
        "JetBrainsMono-Bold.ttf" => Some(JETBRAINS_MONO_BOLD),
        "JetBrainsMono-Italic.ttf" => Some(JETBRAINS_MONO_ITALIC),
        "FiraCode-Regular.ttf" => Some(FIRA_CODE_REGULAR),
        "FiraCode-Bold.ttf" => Some(FIRA_CODE_BOLD),
        _ => None,
    }
}

/// `embedded_font_bytes` first, `find_font` (disk search) second. Used
/// ONLY for `gui-font`'s built-in candidates (`font_search_order`'s
/// output, plus their same-family bold/italic variants), never for a
/// `gui-font-family` override -- see `build_font_definitions`'s
/// `regular_is_override` split, which routes an override's lookups
/// through `find_font` directly instead. Fix (cold review): an earlier
/// version of this function was also used for the override, which meant
/// setting `gui-font-family` to a name that happened to match one of the
/// five embedded files (e.g. a Nerd-Font-patched build a user installed
/// under the same name) silently returned this project's own bundled
/// bytes instead of the user's file -- contradicting both this function's
/// intent and `gui-font-family`'s own docstring in `gui.el`.
fn resolve_font(name: &str) -> Option<Vec<u8>> {
    if let Some(bytes) = embedded_font_bytes(name) {
        return Some(bytes.to_vec());
    }
    find_font(name)
}

/// Default window size (M-visual-quality): big enough that a Verilog
/// module instantiation with a wide port list isn't immediately
/// scrollbar territory. `min_inner_size` keeps a user-shrunk window from
/// collapsing into an unusable sliver. Both the size and the position are
/// then persisted across launches by eframe's own storage (the
/// `persistence` Cargo feature turns this on; `persist_window` in
/// `NativeOptions` defaults to `true`), so these two numbers are only the
/// fallback used on first launch or when the store is empty.
pub fn run_gui(interp: Interp, ed: Rc<RefCell<Editor>>) -> Result<(), eframe::Error> {
    // A failure to decode the embedded icon must not stop the editor from
    // starting -- it is cosmetic, not functional, so this falls back to no
    // icon (`with_icon` accepts `None`) rather than propagating an error or
    // panicking. Library code in this project never unwraps.
    //
    // Nothing in the test suite calls `run_gui` -- it opens a real OS
    // window, which has no headless path here (see `dev/gui-shot.sh`'s doc
    // comment) -- so this `.ok()` fallback has zero test coverage and no
    // mutation of it can be caught by `cargo test`. Recorded here rather
    // than left to look covered.
    let icon = eframe::icon_data::from_png_bytes(ICON_PNG_BYTES).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([480.0, 320.0])
        .with_title("Reticle")
        // M111: always request a transparent-capable viewport, not
        // conditionally on `gui-opacity` -- that variable is read once per
        // frame (see `App::update`), so a startup-time branch here could
        // never react to a later runtime change anyway, and at the
        // default 100% every background paints fully opaque regardless,
        // so the window looks identical to before this milestone either
        // way. If the platform's GL config can't actually do it, eframe
        // logs "Cannot create transparent window" and falls back to an
        // ordinary opaque window -- not a crash, not an error surfaced
        // to the user.
        .with_transparent(true);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "reticle",
        options,
        Box::new(move |cc| {
            let font_family = str_var(&interp, "gui-font-family");
            let named_font = sym_var(&interp, "gui-font")
                .unwrap_or("jetbrains-mono")
                .to_string();
            let (have_bold, have_italic, fonts) =
                install_fonts(&cc.egui_ctx, font_family.as_deref(), &named_font);
            Ok(Box::new(App {
                interp,
                ed,
                last_input: std::time::Instant::now(),
                have_bold,
                have_italic,
                fonts,
                active_font: (named_font, font_family),
                shape_cache: ShapeCache::default(),
                glyph_cache: GlyphAtlasCache::default(),
                last_frame_ms: 0.0,
                frontend_started: false,
                drag: None,
            }))
        }),
    )
}

/// Preferred body-text fonts, base search order (before `gui-font`
/// reorders it -- see `font_search_order`), each tried under every
/// directory in `FONT_DIRS` for the two that aren't embedded. JetBrains
/// Mono and Fira Code are the two monospace faces routinely singled out
/// for readability at RTL-editing sizes (wide port lists, long
/// parameterized module names) -- since M105 both are embedded
/// (`embedded_font_bytes`), so they no longer depend on `FONT_DIRS` at
/// all; SF Mono, Menlo and Monaco are the macOS system monospace faces
/// this project already shipped with, kept as the fallback chain and
/// still disk-searched (SF Mono's license does not permit bundling it).
pub const FONT_CANDIDATES: [&str; 5] = [
    "JetBrainsMono-Regular.ttf",
    "FiraCode-Regular.ttf",
    "SFNSMono.ttf",
    "Menlo.ttc",
    "Monaco.ttf",
];

/// `FONT_CANDIDATES`, with the file matching `gui-font`'s selection
/// moved to the front (stable: everything else keeps its relative
/// order). `named_font` is `gui-font`'s symbol name
/// (`"jetbrains-mono"`/`"fira-code"`/`"sf-mono"`); anything else
/// (unset, or a typo in a user's init.el) is treated as
/// `"jetbrains-mono"`, the documented default. Moving the pick to the
/// front only changes which candidate becomes `install_fonts`'s
/// "regular" face (used for shaping and for the same-family bold/italic
/// search) and its position in the fallback chain -- every other
/// candidate is still tried afterward exactly as before, so a pick that
/// fails to resolve (`sf-mono` not installed) still falls through to a
/// working font rather than leaving the grid unrenderable.
pub fn font_search_order(named_font: &str) -> Vec<&'static str> {
    let preferred = match named_font {
        "fira-code" => "FiraCode-Regular.ttf",
        "sf-mono" => "SFNSMono.ttf",
        _ => "JetBrainsMono-Regular.ttf",
    };
    let mut order: Vec<&'static str> = Vec::with_capacity(FONT_CANDIDATES.len());
    order.push(preferred);
    order.extend(FONT_CANDIDATES.iter().copied().filter(|c| *c != preferred));
    order
}

/// Directories searched for `FONT_CANDIDATES` (and for `gui-font-family`
/// when set), in order. `pub` (M105 fix round) so `font_tests.rs` can
/// probe the exact same directories `find_font` searches, instead of
/// hard-coding its own guess -- and so it can detect when this list
/// drifts from `gui.el`'s `gui--font-dirs`, which duplicates this list by
/// hand because there is no channel from Rust to elisp before the first
/// frame paints (see that function's own docstring).
pub fn font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join("Library/Fonts"));
    }
    dirs.push("/Library/Fonts".into());
    dirs.push("/System/Library/Fonts".into());
    dirs.push("/System/Library/Fonts/Supplemental".into());
    dirs
}

/// Search `font_dirs()` for `file_name`, returning the first match's
/// bytes. `pub` (M105 fix round) so tests can independently confirm
/// whether a given machine has, e.g., `SFNSMono.ttf` installed, and
/// assert the correct branch of `gui-font`'s `sf-mono` fallback instead
/// of assuming one particular machine's state.
pub fn find_font(file_name: &str) -> Option<Vec<u8>> {
    for dir in font_dirs() {
        let path = dir.join(file_name);
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(bytes);
        }
    }
    None
}

/// The grid's monospace body font, plus Arial Unicode as CJK fallback,
/// plus real bold/italic families from Menlo's font collection when
/// available (Menlo.ttc: 0=regular 1=bold 2=italic on macOS). Returns
/// which real variants loaded; bold falls back to double-draw when
/// absent. `font_family` is `gui-font-family` (a file name), tried
/// before the built-in candidate list when set; `named_font` is
/// `gui-font`'s symbol name, which only reorders that built-in candidate
/// list (`font_search_order`) -- `gui-font-family` still wins outright
/// when set, matching its docstring in `gui.el`.
///
/// M105: called again, not just once at startup -- see `App::active_font`
/// and its check in `update`. Callers must not assume this only runs
/// before the first frame.
///
/// Fix E (trailing review, narrowed by M105): the candidate search
/// (`find_font`/`font_dirs`) and the fallback-chain construction below it
/// still depend on which real font files happen to exist on the running
/// machine's filesystem for `gui-font-family`, `sf-mono`, Menlo and
/// Monaco, so those parts still have no test or mutation point --
/// asserting on the result would mean either shipping fixture font files
/// or hard-coding assumptions about what's installed on the CI/dev
/// machine, both of which this project has chosen not to do. This is an
/// acknowledged, deliberate gap, not an oversight. The `jetbrains-mono`/
/// `fira-code` half of this function no longer has that problem: since
/// both are embedded (`embedded_font_bytes`), their result is the same on
/// every machine, and `font_tests.rs` asserts on it directly.
///
/// Thin wrapper over `build_font_definitions` (M105): the split exists so
/// tests can inspect the resulting `FontDefinitions` directly, which
/// `ctx.set_fonts` below would otherwise consume with nothing handed
/// back. This function is what every real caller (`run_gui`'s startup
/// closure, `App::update`'s runtime-switch check) still calls -- the
/// split changes nothing about their behavior.
pub fn install_fonts(
    ctx: &egui::Context,
    font_family: Option<&str>,
    named_font: &str,
) -> (bool, bool, FontSet) {
    let (have_bold, have_italic, font_set, fonts) = build_font_definitions(font_family, named_font);
    ctx.set_fonts(fonts);
    (have_bold, have_italic, font_set)
}

/// The pure half of `install_fonts`: builds the `FontDefinitions` and
/// `FontSet` egui/the shaper need, without touching an `egui::Context` at
/// all. See `install_fonts`'s doc for why this is split out.
pub fn build_font_definitions(
    font_family: Option<&str>,
    named_font: &str,
) -> (bool, bool, FontSet, FontDefinitions) {
    let mut fonts = FontDefinitions::default();
    let mut loaded: Vec<String> = Vec::new();
    let mut font_set = FontSet::default();
    // The primary body font chain (fix 6b): gui-font-family first, then
    // EVERY built-in candidate actually present (embedded, or on disk),
    // in search order -- not just the first hit. A previous version of
    // this function took only the first hit as "primary" and dropped the
    // rest, so a primary font missing a glyph had nothing left to fall
    // back to except the CJK font. Each hit gets its own font_data key
    // (`primary-0`, `primary-1`, ...) so all of them can be chained.
    let candidate_order = font_search_order(named_font);
    // `gui-font-family` (when set) is ALWAYS disk-searched, never
    // resolved via embedded bytes, even when its name happens to match
    // one of the five embedded files (fix, cold review): embedding exists
    // to serve `gui-font`'s three named choices, not to silently
    // substitute a user's own explicitly-named font file with the copy
    // this project bundles. Every built-in candidate after it keeps
    // going through `resolve_font` (embedded first, disk second) exactly
    // as before -- only the override entry's resolver changes.
    let search_order: Vec<(&str, bool)> = font_family
        .into_iter()
        .map(|f| (f, true)) // true = "this is the gui-font-family override"
        .chain(candidate_order.iter().map(|c| (*c, false)))
        .collect();
    // The very first hit is also the "regular" face for shaping (task:
    // coding ligatures) -- its name is remembered so the bold/italic
    // search below can look for a same-family variant of exactly this
    // file, not of the search order's first *candidate* (which may not
    // be what was actually found). `regular_is_override` records whether
    // that hit came from `gui-font-family`, so the variant search below
    // knows to stay disk-only too, for the same reason.
    let mut regular_name: Option<String> = None;
    let mut regular_is_override = false;
    for (name, is_override) in search_order {
        let found = if is_override {
            find_font(name)
        } else {
            resolve_font(name)
        };
        if let Some(bytes) = found {
            let key = format!("primary-{}", loaded.len());
            if regular_name.is_none() {
                regular_name = Some(name.to_string());
                regular_is_override = is_override;
                // Font-bytes trap: `FontData::from_owned` takes
                // ownership of `bytes` below and does not give it back,
                // so the shaping-side copy is taken from this same read
                // before that happens -- see `shaping`'s module doc.
                font_set.regular = ShapingFace::new(Arc::from(bytes.clone().into_boxed_slice()), 0);
            }
            fonts
                .font_data
                .insert(key.clone(), FontData::from_owned(bytes));
            loaded.push(key);
        }
    }
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/Supplemental/Arial Unicode.ttf") {
        fonts
            .font_data
            .insert("arial-unicode".to_string(), FontData::from_owned(bytes));
        loaded.push("arial-unicode".to_string());
    }
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        for name in loaded.iter().rev() {
            mono.insert(0, name.clone());
        }
        // Stable sort: keeps the primary chain's own relative order
        // (established by the insertion loop above) while still
        // guaranteeing the CJK fallback lands after every primary
        // candidate and before whatever egui's defaults already had.
        mono.sort_by_key(|n| {
            if n.starts_with("primary") {
                0
            } else if n == "arial-unicode" {
                1
            } else {
                2
            }
        });
    }

    let mut have_bold = false;
    let mut have_italic = false;

    // Bold/italic (task: same-family variants). Prefer
    // `<Family>-Bold.ttf`/`<Family>-Italic.ttf` next to the regular face
    // actually found above -- with JetBrains Mono as the primary font,
    // drawing its bold/italic runs in Menlo (a different family
    // entirely) is a visible mismatch. Falls back to Menlo.ttc's face
    // indices 1/2 exactly as before when no same-family variant exists.
    //
    // `resolve_variant`: same disk-only-vs-embedded split as the primary
    // search above, keyed on `regular_is_override` -- a `gui-font-family`
    // override's bold/italic variant must be searched for on disk too,
    // never substituted with this project's own bundled bytes just
    // because the variant's file name happens to match one.
    let resolve_variant = |n: &str| -> Option<Vec<u8>> {
        if regular_is_override {
            find_font(n)
        } else {
            resolve_font(n)
        }
    };
    let same_family_bold = regular_name
        .as_deref()
        .and_then(|n| variant_file_name(n, "Bold"))
        .and_then(|n| resolve_variant(&n).map(|b| (n, b)));
    let same_family_italic = regular_name
        .as_deref()
        .and_then(|n| variant_file_name(n, "Italic"))
        .and_then(|n| resolve_variant(&n).map(|b| (n, b)));

    if let Some((_, bytes)) = &same_family_bold {
        font_set.bold = ShapingFace::new(Arc::from(bytes.clone().into_boxed_slice()), 0);
        fonts
            .font_data
            .insert("family-bold".into(), FontData::from_owned(bytes.clone()));
        let mut bold_chain = vec!["family-bold".to_string()];
        if loaded.contains(&"arial-unicode".to_string()) {
            bold_chain.push("arial-unicode".into());
        }
        fonts
            .families
            .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_chain);
        have_bold = true;
    }
    if let Some((_, bytes)) = &same_family_italic {
        font_set.italic = ShapingFace::new(Arc::from(bytes.clone().into_boxed_slice()), 0);
        fonts
            .font_data
            .insert("family-italic".into(), FontData::from_owned(bytes.clone()));
        let mut italic_chain = vec!["family-italic".to_string()];
        if loaded.contains(&"arial-unicode".to_string()) {
            italic_chain.push("arial-unicode".into());
        }
        fonts
            .families
            .insert(FontFamily::Name(ITALIC_FAMILY.into()), italic_chain);
        have_italic = true;
    }

    if !have_bold || !have_italic {
        if let Ok(bytes) = std::fs::read("/System/Library/Fonts/Menlo.ttc") {
            if !have_bold {
                let mut bold = FontData::from_owned(bytes.clone());
                bold.index = 1;
                fonts.font_data.insert("menlo-bold".into(), bold);
                font_set.bold = ShapingFace::new(Arc::from(bytes.clone().into_boxed_slice()), 1);
                let mut bold_chain = vec!["menlo-bold".to_string()];
                if loaded.contains(&"arial-unicode".to_string()) {
                    bold_chain.push("arial-unicode".into());
                }
                fonts
                    .families
                    .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_chain);
                have_bold = true;
            }
            if !have_italic {
                let mut italic = FontData::from_owned(bytes.clone());
                italic.index = 2;
                fonts.font_data.insert("menlo-italic".into(), italic);
                font_set.italic = ShapingFace::new(Arc::from(bytes.into_boxed_slice()), 2);
                let mut italic_chain = vec!["menlo-italic".to_string()];
                if loaded.contains(&"arial-unicode".to_string()) {
                    italic_chain.push("arial-unicode".into());
                }
                fonts
                    .families
                    .insert(FontFamily::Name(ITALIC_FAMILY.into()), italic_chain);
                have_italic = true;
            }
        }
    }

    (have_bold, have_italic, font_set, fonts)
}

struct App {
    interp: Interp,
    ed: Rc<RefCell<Editor>>,
    last_input: std::time::Instant,
    /// Whether a real bold face loaded. Both `jetbrains-mono` and
    /// `fira-code` have their own embedded `-Bold.ttf`, so this is `true`
    /// for both -- see `build_font_definitions`'s `same_family_bold`.
    have_bold: bool,
    /// Whether a real italic face loaded, from EITHER of two different
    /// sources depending on `gui-font`: `jetbrains-mono` has its own
    /// embedded `JetBrainsMono-Italic.ttf` (a true italic design for that
    /// family); `fira-code` has no italic upstream at all, so its
    /// `have_italic`/`family-italic` slot is never populated -- when this
    /// is `true` for `fira-code`, the FONT-DEFINITION layer registers
    /// Menlo.ttc's italic face (index 2) under the `"mono-italic"` family
    /// instead (confirmed by directly calling `build_font_definitions`:
    /// `have_italic` is `true` and `families[Name("mono-italic")]` is
    /// `["menlo-italic", "arial-unicode"]`), and `epaint` 0.29.1
    /// (`fonts.rs:202`) does pass that face index through to
    /// `ab_glyph::FontRef::try_from_slice_and_index`, so the registration
    /// path traces through correctly. `font_tests.rs`'s
    /// `build_font_definitions_for_fira_code_has_no_embedded_italic`
    /// covers exactly this registration-layer claim (the `family-italic`
    /// key stays absent, and when `have_italic` is true the bytes under
    /// `menlo-italic` are Menlo's, not Fira Code's) -- that test is about
    /// registration, not about what ends up on screen.
    ///
    /// KNOWN GAP, MEASURED then FIXED (M105 measured it, M106 found and
    /// fixed the cause): despite the registration above being correct,
    /// selecting `fira-code` used to render italic text (e.g. comments)
    /// UPRIGHT on screen, confirmed by a screenshot cropped to the
    /// comment line and scaled 3x for comparison against JetBrains
    /// Mono's visibly slanted italic. The cause was in `shaping.rs`'s
    /// `ShapingFace::new`: it built the rasterizing `ab_glyph::FontArc`
    /// with `try_from_vec`, which always parses face 0 of a font
    /// collection and silently ignores the `index` argument -- so
    /// Menlo.ttc's italic face (index 2) was shaped correctly (rustybuzz
    /// does honor `loaded.index`) but rasterized as face 0 (regular),
    /// which is why it came out upright. The same bug also made
    /// `sf-mono`'s bold (Menlo.ttc index 1) rasterize as regular weight.
    /// Fixed by rasterizing through
    /// `ab_glyph::FontVec::try_from_vec_and_index`, which does take the
    /// face index, so shaping and rasterizing now agree on which face of
    /// the collection they're using. Verified on screen after the fix
    /// (`dev/gui-shot.sh` against a freshly rebuilt binary, not the stale
    /// one an earlier attempt at this check mistakenly ran): a
    /// before/after pixel diff of the comment line shows 16,112 changed
    /// pixels for `fira-code` and 21,491 for `sf-mono`, both now visibly
    /// slanted -- only these two fonts' italic/bold-via-Menlo.ttc paths
    /// were actually looked at on screen, not every font this crate can
    /// load. See `shaping.rs`'s `ShapingFace` doc for the invariant this
    /// restores. See `gui-font`'s docstring in `gui.el` for the same
    /// note.
    have_italic: bool,
    /// Shaping-ready faces (task: coding ligatures) -- see `shaping`'s
    /// module doc. Rebuilt by `install_fonts` whenever `active_font`'s
    /// per-frame check (M105) finds `gui-font`/`gui-font-family` changed
    /// since the last frame -- unlike `glyph_cache`, this has no
    /// self-invalidating signal of its own, so `update` must compare
    /// explicitly rather than relying on an atlas-identity check.
    fonts: FontSet,
    /// The `(gui-font, gui-font-family)` pair that produced the `fonts`/
    /// `have_bold`/`have_italic` currently loaded (M105). Read once per
    /// frame in `update` and compared against the live elisp variables;
    /// on a mismatch, `install_fonts` reruns and rebuilds egui's font
    /// atlas via `ctx.set_fonts` -- an expensive call, which is exactly
    /// why this comparison exists rather than reinstalling every frame.
    active_font: (String, Option<String>),
    /// Shaping result cache, keyed on `(FaceRole, text, enable_calt)`.
    /// See `shaping`'s module doc for what invalidates it (nothing, but
    /// capped -- the `enable_calt` key component is what makes
    /// `gui-ligatures` toggling never see a stale entry).
    shape_cache: ShapeCache,
    /// Rasterized-glyph atlas-placement cache, keyed on `(FaceRole, glyph
    /// id, pixel scale)`. See `shaping`'s module doc for the per-frame
    /// atlas-identity check that invalidates it wholesale.
    glyph_cache: GlyphAtlasCache,
    last_frame_ms: f32,
    /// M88: true once `core::frontend_started` has been called -- set at
    /// the end of the first `update`, deliberately AFTER that frame's
    /// own `core::idle_tick` call, so the earliest an autostart can fire
    /// is the frame after something has actually been painted.
    frontend_started: bool,
    /// Mouse support (task 3): state carried between frames while the
    /// primary button is held, `None` otherwise. Drag-to-select must
    /// tell a plain click (no movement, no mark should be armed) apart
    /// from an actual drag (arm the mark at the press position, then the
    /// usual `PointerButton`-release just stops tracking -- the region
    /// that resulted stays exactly as `mark`/`mark_active` left it, same
    /// as every other region in this editor).
    drag: Option<DragState>,
}

/// See `App::drag`.
struct DragState {
    win_id: usize,
    /// Buffer byte offset under the press -- becomes the mark's position
    /// the first time the drag actually moves to a different cell.
    start_byte: usize,
    /// Whether the drag has moved to a different buffer position since
    /// the press (i.e. whether the mark has been armed yet).
    dragged: bool,
}

/// An elisp integer variable, or `None` when unset / not an integer.
fn int_var_opt(interp: &Interp, name: &str) -> Option<i64> {
    interp
        .intern_soft(name)
        .and_then(|id| interp.sym_value(id))
        .and_then(|v| match v {
            elisp::Value::Int(n) => Some(n),
            _ => None,
        })
}

/// An elisp integer variable, or `default`.
fn int_var(interp: &Interp, name: &str, default: i64) -> i64 {
    int_var_opt(interp, name).unwrap_or(default)
}

/// An elisp string variable, or `None` when unset / not a string. String
/// sibling of `int_var`, same shape.
fn str_var(interp: &Interp, name: &str) -> Option<String> {
    interp
        .intern_soft(name)
        .and_then(|id| interp.sym_value(id))
        .and_then(|v| match v {
            elisp::Value::Str(s) => Some((*s).clone()),
            _ => None,
        })
}

fn sym_var<'a>(interp: &'a Interp, name: &str) -> Option<&'a str> {
    let id = interp.intern_soft(name)?;
    match interp.sym_value(id)? {
        elisp::Value::Sym(s) => Some(interp.sym_name(s)),
        _ => None,
    }
}

fn var_truthy(interp: &Interp, name: &str) -> bool {
    interp
        .intern_soft(name)
        .and_then(|id| interp.sym_value(id))
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// A buffer-local integer variable, honoring buffer-local bindings the
/// way `buffer_local_value` does -- `fill-column` (M116) is buffer-local
/// (a per-buffer line-length convention set by major mode / user), unlike
/// every `gui-*` variable `int_var` reads, which are all global. Mirrors
/// `core::redisplay::buffer_var_on`'s shape one crate up: that helper is
/// `pub(crate)` to `core` and not reachable from here, so this is the
/// frontend's own copy of the same buffer-local-aware read, specialized
/// to `Int` instead of truthiness.
fn buffer_int_var(
    interp: &Interp,
    ed: &Editor,
    buf: &Rc<RefCell<Buffer>>,
    name: &str,
    default: i64,
) -> i64 {
    interp
        .intern_soft(name)
        .and_then(|id| buffer_local_value(interp, ed, id, buf))
        .and_then(|v| match v {
            elisp::Value::Int(n) => Some(n),
            _ => None,
        })
        .unwrap_or(default)
}

/// A buffer-local boolean variable, honoring buffer-local bindings the
/// way `buffer_local_value` does -- `display-fill-column-indicator-mode`
/// (M116) is buffer-local, same reasoning as `buffer_int_var` above.
fn buffer_var_on(interp: &Interp, ed: &Editor, buf: &Rc<RefCell<Buffer>>, name: &str) -> bool {
    interp
        .intern_soft(name)
        .and_then(|id| buffer_local_value(interp, ed, id, buf))
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// A named face from the editor's face table (`ed.faces`), or `None`
/// when the theme hasn't defined it yet. Mirrors `redisplay::face_or`,
/// which is private to that crate -- this is the frontend's own copy of
/// the lookup, returning `Option` instead of a fallback `Style` since
/// each call site here has its own alpha-based degrade rather than a
/// single default style.
fn face_style(interp: &Interp, ed: &Editor, name: &str) -> Option<core::redisplay::Style> {
    interp
        .intern_soft(name)
        .and_then(|id| ed.faces.get(&id).copied())
}

/// `gui-padding-x` / `gui-padding-y` clamp (task 2): 0..=64 device-
/// independent px.
fn clamp_padding(raw: i64) -> i64 {
    raw.clamp(0, 64)
}

/// `gui-line-spacing` clamp (task 2): a percentage, 80..=200.
fn clamp_line_spacing(raw: i64) -> i64 {
    raw.clamp(80, 200)
}

/// `gui-font-size` clamp: 8..=72.
fn clamp_font_size(raw: i64) -> i64 {
    raw.clamp(8, 72)
}

/// `gui-cursor-blinks` clamp (fix A): 0..=100. `0` means "never stop
/// blinking" (GNU's own meaning for `blink-cursor-blinks` at 0), so it is
/// deliberately inside the clamped range rather than treated as "unset".
fn clamp_cursor_blinks(raw: i64) -> i64 {
    raw.clamp(0, 100)
}

/// `gui-opacity` clamp (M111): 20..=100. There is no in-app opacity
/// picker and no keybinding to raise it back -- the only way out of a
/// bad value is editing the init file -- so 0 (or anything close to it)
/// would hand a user a window that is invisible against a busy desktop
/// and cannot be found to fix. 20 was picked as a floor that stays
/// unambiguously "a window is there" (a dim but still fully outlined
/// rectangle) rather than trying to guess the darkest usable value for
/// every possible desktop background.
fn clamp_opacity(raw: i64) -> i64 {
    raw.clamp(20, 100)
}

/// `gui-opacity`'s raw percentage -> the 0.0..=1.0 fraction every color
/// conversion below actually consumes. Pulled out as its own named
/// function (M111 cold review, round 2) because inlining
/// `clamp_opacity(...) as f32 / 100.0` at the one call site left the
/// `/ 100.0` divisor unreachable by any test -- mutating it to, say,
/// `/ 1000.0` would make `gui-opacity 40` behave as 4% in the running
/// editor while every existing test still passed.
fn opacity_percent_to_frac(pct: i64) -> f32 {
    clamp_opacity(pct) as f32 / 100.0
}

/// Background-role conversion (M111): the same RGB mapping `to_color`
/// uses, composed with `with_alpha` so the result also carries
/// `gui-opacity`'s alpha. Kept as a separate function rather than adding
/// an alpha parameter to `to_color` itself, because `to_color` is also
/// used for foreground/text colors and underline colors, which must
/// stay fully opaque regardless of `gui-opacity` -- text over a
/// translucent background is how iTerm2/VS Code/JetBrains all do it, and
/// mixing the two roles into one conversion is exactly the failure mode
/// measured during recon: every face with an explicit `:background`
/// stayed fully opaque while only the frame's own default background
/// went translucent, because only the *default* used a hand-rolled
/// alpha and every per-cell background still went through the opaque
/// `to_color`.
fn to_bg_color(c: core::redisplay::Color, opacity_frac: f32) -> Color32 {
    with_alpha(to_color(c), opacity_frac)
}

/// The per-frame `gui-opacity` read (M117): pulled out of `App::update`
/// so the wire from the elisp variable to the fraction every color
/// conversion consumes is reachable from `cargo test` with a real
/// `Interp` -- before this extraction the whole conversion lived inline
/// in `update`, which only runs inside eframe's real `CreationContext`
/// closure and is therefore unreachable from any test.
fn frame_opacity_frac(interp: &Interp) -> f32 {
    opacity_percent_to_frac(int_var(interp, "gui-opacity", 100))
}

/// The per-frame frame-background computation (M117): the theme's own
/// background (if the active face set defines one) picks up
/// `opacity_frac`'s alpha via `to_bg_color`; otherwise `FALLBACK_BG`
/// picks it up via `with_alpha`. Extracted alongside `frame_opacity_
/// frac` for the same reason -- it was inline in `App::update` and
/// unreachable from `cargo test`.
fn frame_bg(base_bg: Option<core::redisplay::Color>, opacity_frac: f32) -> Color32 {
    base_bg
        .map(|c| to_bg_color(c, opacity_frac))
        .unwrap_or_else(|| with_alpha(FALLBACK_BG, opacity_frac))
}

/// The per-frame `gui-ligatures` read (M117): pulled out of `App::
/// update` for the same reason as `frame_opacity_frac` above -- so a
/// wrong variable name or a hardcoded `true` is caught by a test
/// instead of only being visible on a screenshot.
fn ligatures_enabled(interp: &Interp) -> bool {
    var_truthy(interp, "gui-ligatures")
}

/// The scrollbar's diagnostics lookup for one window's buffer (M117):
/// pulled out of the inline `ed_ref.diagnostics.get(&(Rc::as_ptr(&win.
/// buffer) as usize))...` chain in `App::update` so a wrong key (e.g.
/// looking up the other window's buffer in a split, which would
/// silently drop every mark) is caught by a test instead of only being
/// visible on a screenshot. Preserves the stored iteration order --
/// does not sort or dedup.
///
/// The `Vec` is the deliberate price of that testability: the inline
/// version borrowed lazily straight out of the `HashMap`, so this
/// allocates once per frame per window where the old code allocated
/// nothing. A diagnostics list is a handful of entries, and the cold
/// read that raised it agreed the trade is worth making -- recorded
/// here rather than left for someone to rediscover as an unexplained
/// per-frame allocation.
fn window_diagnostic_lines(ed: &Editor, buf: &Rc<RefCell<Buffer>>) -> Vec<(usize, u8)> {
    ed.diagnostics
        .get(&(Rc::as_ptr(buf) as usize))
        .into_iter()
        .flatten()
        .map(|(line, sev, _msg)| (*line, *sev))
        .collect()
}

/// Snap a rect's four edges to whole device pixels via `pixels_per_point`
/// (task 1), so a solid fill never lands its edge mid-pixel -- which
/// otherwise blends with whatever is behind it through antialiasing and
/// is exactly the "blended row" / "scattered single-pixel seams" the
/// survey measured on the mode line and between adjacent cell rects.
///
/// Every coordinate is snapped with the *same* function of its own
/// value (round to the nearest device pixel), not "floor the min, ceil
/// the max": floor-min/ceil-max would map a shared boundary to two
/// different device pixels depending on whether it's read as one rect's
/// right edge or its neighbor's left edge (floor != ceil whenever that
/// boundary isn't already an integer device pixel) -- i.e. it would
/// reintroduce, as a 1-device-pixel *overlap*, the exact seam this
/// function exists to remove. Rounding each coordinate independently
/// keeps the mapping a pure function of that one number, so two rects
/// built from the same shared boundary always land on the identical
/// device pixel: no gap, no overlap.
fn snap_rect(rect: Rect, ppp: f32) -> Rect {
    // Fix 5 (cold review): shares the exact rounding formula with
    // `shaping::build_shaped_mesh`'s glyph-position snap via
    // `shaping::snap_to_pixel`, rather than each maintaining its own
    // copy of `(v * ppp).round() / ppp`.
    Rect::from_min_max(
        Pos2::new(
            shaping::snap_to_pixel(rect.min.x, ppp),
            shaping::snap_to_pixel(rect.min.y, ppp),
        ),
        Pos2::new(
            shaping::snap_to_pixel(rect.max.x, ppp),
            shaping::snap_to_pixel(rect.max.y, ppp),
        ),
    )
}

/// `c` at `frac` opacity (a true alpha value, not `gamma_multiply`'s
/// brightness scaling) -- for overlays drawn on top of already-painted
/// content (indent guides, separators, the scrollbar's default color),
/// where the visual effect wanted is "blended with whatever's under it",
/// not "the same color, dimmer."
fn with_alpha(c: Color32, frac: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        c.r(),
        c.g(),
        c.b(),
        (frac.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

/// Linear-interpolate two colors (used for the cursor's continuous fade,
/// task 6): `t=0` is `a`, `t=1` is `b`.
fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// The color the box cursor's *glyph* is drawn in at fade fraction `t`
/// (fix 3 -- zero-contrast midpoint): a hard switch at `t = 0.5`, never a
/// lerp. The old code lerped both the glyph color and the background
/// toward each other's swapped counterpart with the SAME `t`, so
/// `fg(t) - bg(t) = (1 - 2t) * (fg0 - bg0)` -- exactly zero at `t = 0.5`,
/// making the character under the cursor briefly invisible every fade
/// cycle. Only the background may fade smoothly (`cursor_bg_color`
/// below); the glyph color has no legibility requirement to interpolate
/// toward, so it just switches.
fn cursor_glyph_color(normal_fg: Color32, under_cursor_fg: Color32, t: f32) -> Color32 {
    if t < 0.5 {
        normal_fg
    } else {
        under_cursor_fg
    }
}

/// The box cursor's background at fade fraction `t` -- unlike the glyph
/// color, this may keep the smooth interpolation (fix 3): there is no
/// "invisible" failure mode for a background fading into another
/// background, only for a glyph fading into the color right behind it.
///
/// `normal_bg` is forced `.to_opaque()` here (M111 cold review, round 2)
/// because under `gui-opacity` < 100 it may be a translucent `to_bg_color`
/// value -- a `Color32` stores gamma-premultiplied bytes, and `lerp_color`
/// reads `.r()/.g()/.b()` directly and treats them as plain channel
/// values, exactly the same "premultiplied read as unmultiplied" bug
/// already fixed for `with_alpha`'s double-application elsewhere in this
/// milestone, just reached through a different function. `.to_opaque()`
/// un-premultiplies by the color's own current alpha, so this is correct
/// whether `normal_bg` arrives already opaque (a no-op) or translucent.
/// `under_cursor_bg` is not treated the same way because every call site
/// already guarantees it opaque before calling this.
fn cursor_bg_color(normal_bg: Color32, under_cursor_bg: Color32, t: f32) -> Color32 {
    lerp_color(normal_bg.to_opaque(), under_cursor_bg, t)
}

/// Fix A: whether the cursor's smooth fade (task 6) is still animating at
/// `elapsed` (time since the last input event), and its fade fraction if
/// so -- pure, so it's testable without a window. Mirrors GNU Emacs's
/// `blink-cursor-blinks` (default 10): after roughly that many full fade
/// cycles with no input, the cursor stops animating and is left fully
/// solid (fade fraction 1.0, matching `cursor_alpha`'s own "forced
/// opaque" value) rather than left frozen mid-fade at some arbitrary
/// point in the cycle. `blink_limit <= 0` means "never stop" -- GNU's own
/// meaning for `blink-cursor-blinks` at 0.
///
/// The fade fraction itself is a raised cosine over `elapsed`'s position
/// in the cycle (`elapsed mod period`), the same shape `App::update` used
/// before this fix, just re-based on time-since-input instead of the
/// frame's absolute wall-clock time -- every fresh quiet period now
/// starts its fade from phase 0, which is unobservable in practice since
/// `BLINK_SUPPRESS_WINDOW` already forces the cursor fully opaque for the
/// first stretch after input regardless of phase.
fn blink_decision(elapsed: std::time::Duration, period: f32, blink_limit: i64) -> (bool, f32) {
    let elapsed_secs = elapsed.as_secs_f32();
    if blink_limit > 0 && period > 0.0 && elapsed_secs >= blink_limit as f32 * period {
        return (false, 1.0);
    }
    let phase = if period > 0.0 {
        elapsed_secs.rem_euclid(period) / period
    } else {
        0.0
    };
    let raised_cosine = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    (true, raised_cosine)
}

/// Grid dimension floor (fix 6a): `computed` is how many whole cells
/// actually fit in the padded area (`text_avail / char_w` or `/ row_h`);
/// `desired` is the old hard-coded preference (20 cols / 5 rows). The
/// old `cols.max(20)`/`rows.max(5)` forced the preference regardless of
/// `computed`, so a small window (the enforced 480x320 minimum) with
/// padding near its 64px clamp ceiling could have `computed` well under
/// `desired`, and the grid painted past the edge of the padded area.
/// `desired` is honored only when there's room for it; otherwise this
/// falls back to whatever fits, floored at 1 (a zero-size grid can't
/// render at all -- `core::redisplay::frame_layout`'s own
/// `.max(4)`/`.max(3)` is the next line of defense for a genuinely
/// degenerate frame).
fn dimension_floor(computed: usize, desired: usize) -> usize {
    if computed >= desired {
        computed
    } else {
        computed.max(1)
    }
}

/// Which columns (task 3) get an indent-guide line drawn, one `Vec` per
/// row of `rows`. `step` is the guide spacing (`gui-indent-guide-step`,
/// or `tab-width`, or 4); `step == 0` disables guides entirely.
///
/// A row's "leading-space count" is how many of its leading characters
/// are the space character (an empty row counts as 0). On a non-blank
/// row (one with at least one non-space character), a guide is drawn at
/// every column that is a multiple of `step` -- starting at 0 -- and
/// strictly less than that count (VS Code's rule: a single indent level
/// still gets one guide, at column 0; the earlier "positive multiple"
/// rule drew nothing at all for a 4-space/step-4 line, which is every
/// line in a Verilog port list indented one level). A blank row (all
/// spaces, or empty) instead uses the
/// SMALLER of the nearest non-blank row's leading-space count above and
/// below it, so guides run continuously through a run of blank lines
/// inside e.g. a port list rather than breaking up; with a non-blank row
/// on only one side, that side's count is used alone, and with neither,
/// nothing is drawn for that row.
fn indent_guide_columns(rows: &[&str], step: usize) -> Vec<Vec<usize>> {
    if step == 0 {
        return vec![Vec::new(); rows.len()];
    }
    let leading: Vec<usize> = rows
        .iter()
        .map(|r| r.chars().take_while(|&c| c == ' ').count())
        .collect();
    let is_blank: Vec<bool> = rows.iter().map(|r| r.chars().all(|c| c == ' ')).collect();
    let mut effective = vec![0usize; rows.len()];
    for i in 0..rows.len() {
        if !is_blank[i] {
            effective[i] = leading[i];
            continue;
        }
        let above = (0..i).rev().find(|&j| !is_blank[j]).map(|j| leading[j]);
        let below = (i + 1..rows.len())
            .find(|&j| !is_blank[j])
            .map(|j| leading[j]);
        effective[i] = match (above, below) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => 0,
        };
    }
    effective
        .into_iter()
        .map(|count| {
            let mut cols = Vec::new();
            let mut col = 0;
            while col < count {
                cols.push(col);
                col += step;
            }
            cols
        })
        .collect()
}

/// Overlay scrollbar thumb geometry (task 5): given the buffer's total
/// line count, how many lines are visible, the topmost visible line, the
/// track length in logical px, and the minimum thumb length, returns
/// `Some((thumb_top, thumb_len))` in the same units as `track_len`, or
/// `None` when the whole buffer already fits (no scrollbar needed).
fn scrollbar_thumb(
    total_lines: usize,
    visible_lines: usize,
    top_line: usize,
    track_len: f32,
    min_len: f32,
) -> Option<(f32, f32)> {
    if total_lines == 0 || visible_lines == 0 || total_lines <= visible_lines {
        return None;
    }
    let raw_len = track_len * (visible_lines as f32 / total_lines as f32);
    let len = raw_len.max(min_len).min(track_len);
    let top = (track_len * (top_line as f32 / total_lines as f32)).min(track_len - len);
    Some((top, len))
}

/// Overview-ruler diagnostic marks (M115): given every diagnostic's
/// 0-based line number and severity byte (same encoding as
/// `Editor::diagnostics` and `severity_color` -- 1=error, 2=warning,
/// anything else=info/hint, LOWER is MORE severe), the buffer's total
/// line count, and the track geometry (same `track_len` `scrollbar_thumb`
/// draws into, plus a `min_len` mark height), returns each mark's
/// `(top, len, severity)` in track-local px, sorted by `top`.
///
/// A diagnostic's `top` is proportional to its line within the WHOLE
/// buffer (`line / total_lines`), not within the visible window -- the
/// point of an overview ruler is showing what's off-screen, unlike the
/// thumb which tracks the visible window itself.
///
/// Marks are bucketed by their rounded-to-the-pixel `top` (`track_len`
/// is already in logical px, same units the caller feeds straight into
/// `painter.rect_filled`, so "same pixel row" is exactly "same rounded
/// top"); two diagnostics that land in the same bucket merge into one
/// mark, keeping the MOST severe (lowest byte) of the two -- otherwise a
/// later info-level diagnostic drawn on top of an earlier error would
/// visually erase it, which is the opposite of what a ruler is for.
///
/// `min_len` gives every mark a floor height (mirroring `scrollbar_thumb`'s
/// own `min_len` clamp on the thumb) -- a single diagnostic in a
/// 10,000-line file would otherwise round to a sub-pixel sliver. The
/// returned length is `min_len.min(track_len)` and `top` is clamped so
/// `top + len` never exceeds `track_len`, the same way `scrollbar_thumb`
/// clamps both its own `len` and `top`.
///
/// Returns no marks (rather than panicking or emitting an out-of-bounds
/// rect) for: `total_lines == 0`, `track_len <= 0.0`, or any individual
/// diagnostic whose line is `>= total_lines` (that diagnostic is simply
/// dropped, not treated as fatal for the rest).
fn scrollbar_diagnostic_marks<I>(
    diagnostics: I,
    total_lines: usize,
    track_len: f32,
    min_len: f32,
) -> Vec<(f32, f32, u8)>
where
    I: IntoIterator<Item = (usize, u8)>,
{
    if total_lines == 0 || track_len <= 0.0 {
        return Vec::new();
    }
    // Same clamp shape as `scrollbar_thumb`'s own `len`/`top`: the
    // returned length can never exceed the track (`min_len.min(track_len)`,
    // mirroring `raw_len.max(min_len).min(track_len)` there), and `top`
    // is bounded so `top + len` never exceeds `track_len` either --
    // review round 1 found the length itself wasn't clamped, so
    // `track_len < min_len` could return a mark whose bottom edge sat
    // below the track despite this doc's claim otherwise.
    let len = min_len.min(track_len);
    let max_top = (track_len - len).max(0.0);
    // Takes an `IntoIterator` rather than a slice (review round 1, item
    // 5) so the caller can feed the `Vec<(usize, u8, String)>` already
    // sitting in `Editor::diagnostics` straight through a `.map()`
    // without collecting an intermediate `Vec<(usize, u8)>` first --
    // the realistic diagnostic count is small, but the intermediate
    // Vec bought nothing.
    let mut buckets: std::collections::BTreeMap<i64, u8> = std::collections::BTreeMap::new();
    for (line, sev) in diagnostics {
        if line >= total_lines {
            continue;
        }
        let raw_top = track_len * (line as f32 / total_lines as f32);
        let top = raw_top.clamp(0.0, max_top);
        let row = top.round() as i64;
        buckets
            .entry(row)
            .and_modify(|e| *e = (*e).min(sev))
            .or_insert(sev);
    }
    buckets
        .into_iter()
        .map(|(row, sev)| (row as f32, len, sev))
        .collect()
}

// --- Row geometry (M87 stage 3) --------------------------------------
//
// `grid.row_scale` (a percentage of the base row height, `100` for an
// ordinary row, `75` for an inline-diagnostic block row -- see
// `core::redisplay::Grid::row_scale`'s own doc) means a row's pixel
// height is no longer a single frame-wide constant, so every
// `row as f32 * row_h` in this file (both directions: row index -> pixel
// position, and pixel position -> row index) has to walk cumulative
// per-row heights instead of doing one multiplication. These three
// functions are that single source of truth, replacing the old bare
// `row_h` arithmetic at every call site below.
//
// `grid.row_scale` is looked up with `.get(row).copied().unwrap_or(100)`
// throughout, not indexed directly: the hand-built `Grid`s this module's
// own tests construct (`test_grid`, `one_window_grid`) leave `row_scale`
// empty, and treating "no entry" as "ordinary row" keeps every existing
// pixel-math test passing unchanged -- a real `Grid` from `render()`
// always has `row_scale.len() == grid.rows`, so the fallback never
// actually triggers there.

/// Height in pixels of `row`: `row_h` for an ordinary row, less for a
/// shorter one (currently only `RowKind::Block`, at 75%; see D1 in the
/// M87 stage 3 spec for why a row is never *taller* than `row_h`).
fn row_height(grid: &core::redisplay::Grid, row_h: f32, row: usize) -> f32 {
    row_h * grid.row_scale.get(row).copied().unwrap_or(100) as f32 / 100.0
}

/// Top edge of `row`, in pixels relative to the grid's own origin (the
/// caller adds `origin.y`). Equivalent to `row as f32 * row_h` when
/// every row is the same height; walks the rows before it and sums their
/// actual heights otherwise.
fn row_top(grid: &core::redisplay::Grid, row_h: f32, row: usize) -> f32 {
    let mut y = 0.0f32;
    for r in 0..row {
        y += row_height(grid, row_h, r);
    }
    y
}

/// Whether the fill-column ruler (M116) is inside this window's text
/// area at all -- extracted so this guard (review fix: named as an
/// otherwise-uncovered mutation) is testable on its own, since the
/// drawing loop it lives in needs a live `egui::Painter`. `fill_column
/// == text_cols` counts as NOT visible (the ruler would land exactly on
/// the boundary column, one past the last real text column), matching
/// the `>=` the call site already used before this was extracted.
fn fill_column_visible(fill_column: usize, text_cols: usize) -> bool {
    fill_column < text_cols
}

/// The pixel `x` the fill-column ruler (M116) lands at: `text_col0` is
/// the window's own text origin column (already past its gutter, same
/// value the indent-guide block above it uses), `fill_column` is the
/// buffer-local column to draw at. Extracted from the drawing block so
/// this arithmetic is testable without a live `egui::Painter` -- the
/// indent-guide block does the identical `origin.x + (text_col0 + col)
/// as f32 * char_w` inline since its column set varies per row and is
/// already covered by `indent_guide_columns`'s own tests; this one is a
/// single fixed column per window, so factoring it out is the whole
/// test.
fn fill_column_x(origin_x: f32, text_col0: usize, fill_column: usize, char_w: f32) -> f32 {
    origin_x + (text_col0 + fill_column) as f32 * char_w
}

/// Inverse of `row_top`: which row a grid-relative pixel offset `rel_y`
/// (`>= 0`) falls inside. `row_h` must already be checked `> 0.0` by the
/// caller (both call sites below already guard this) -- otherwise this
/// never terminates. Equivalent to `(rel_y / row_h).floor() as usize`
/// when every row is the same height.
fn row_at_y(grid: &core::redisplay::Grid, row_h: f32, rel_y: f32) -> usize {
    let mut y = 0.0f32;
    // F4 (M87 stage 3 fix round): both call sites already guard `row_h >
    // 0.0` before calling this, which is what makes the loop provably
    // terminate on its own (`y` strictly increases each iteration) --
    // this cap is defence in depth, not a live bug, for any future
    // caller that doesn't. Capped at `grid.rows`, returning the last
    // valid row instead of looping past the end.
    for row in 0..grid.rows {
        let h = row_height(grid, row_h, row);
        if rel_y < y + h {
            return row;
        }
        y += h;
    }
    grid.rows.saturating_sub(1)
}

// --- Mouse support (M-mouse task 3) ---------------------------------
//
// Click-to-place-point, drag-to-select, wheel-to-scroll. All three need
// the same pixel-to-cell geometry the paint loop already uses
// (`origin`/`char_w`/`row_h`), plus the just-rendered `Grid`'s
// `windows` (for which window a pixel lands in) and `buffer_pos_at`
// (task 1's screen-to-buffer inverse mapping), so these run inside
// `App::update`'s `CentralPanel` closure, after both exist -- not in
// the keyboard event loop at the top of `update`, which runs before
// either does.

/// A screen pixel position mapped to `(win_id, buffer byte offset)`,
/// clamped to the window whose *text area* (`WindowLayout::row..
/// mode_line_row`, columns `col + gutter_cols..col + cols`) the pixel
/// falls inside. `None` for anything outside every window's text area
/// -- a mode line, the echo row, a gutter, or the padding margin around
/// the grid -- so a click there does nothing rather than mis-placing
/// point (guard rail from the milestone spec).
fn pixel_to_buffer_pos(
    grid: &core::redisplay::Grid,
    origin: Pos2,
    char_w: f32,
    row_h: f32,
    pixel: Pos2,
) -> Option<(usize, usize)> {
    if pixel.x < origin.x || pixel.y < origin.y || char_w <= 0.0 || row_h <= 0.0 {
        return None;
    }
    let col = ((pixel.x - origin.x) / char_w).floor() as usize;
    let row = row_at_y(grid, row_h, pixel.y - origin.y);
    for win in &grid.windows {
        let text_left = win.col + win.gutter_cols;
        let text_right = win.col + win.cols;
        if row >= win.row && row < win.mode_line_row && col >= text_left && col < text_right {
            let byte = grid.buffer_pos_at(row, col)?;
            return Some((win.win_id, byte));
        }
    }
    None
}

/// Which window (if any) a pixel position's *text area* falls inside --
/// the wheel handler's own lookup, separate from `pixel_to_buffer_pos`
/// because scrolling doesn't need a buffer byte offset at all (and
/// `buffer_pos_at` can legitimately return `None` over a blank area of
/// an otherwise-valid window row, which must not stop the wheel from
/// scrolling that window).
fn window_at_pixel(
    grid: &core::redisplay::Grid,
    origin: Pos2,
    char_w: f32,
    row_h: f32,
    pixel: Pos2,
) -> Option<usize> {
    if pixel.x < origin.x || pixel.y < origin.y || char_w <= 0.0 || row_h <= 0.0 {
        return None;
    }
    let col = ((pixel.x - origin.x) / char_w).floor() as usize;
    let row = row_at_y(grid, row_h, pixel.y - origin.y);
    grid.windows
        .iter()
        .find(|win| {
            row >= win.row
                && row < win.mode_line_row
                && col >= win.col + win.gutter_cols
                && col < win.col + win.cols
        })
        .map(|win| win.win_id)
}

/// Move point to `byte_pos` in window `win_id`'s buffer, selecting that
/// window first if it isn't already selected. Routed through
/// `core::editor::select_window` (the same entry point `C-x o`/evil's
/// `C-w w` use, `builtins/ui.rs`) for the window switch -- the one
/// public API this codebase has for it -- and then a direct
/// `Buffer::point` write for the point move itself: there is no
/// dedicated Rust-level `goto-char` function to call instead (the
/// `goto-char` *elisp* builtin is a closure registered on the
/// interpreter, not a plain Rust fn), so this mirrors that builtin's
/// own body exactly (`crates/core/src/builtins/editing.rs`: clamp, set
/// `point`, clear `goal_column`) -- the same direct-field-write idiom
/// `select_window`/`show_buffer_in_selected_window` themselves already
/// use internally (`crates/core/src/editor.rs`).
fn place_point(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, win_id: usize, byte_pos: usize) {
    if ed.borrow().selected_window != win_id {
        core::editor::select_window(interp, ed, win_id);
    }
    let buf = match ed.borrow().windows.get(&win_id) {
        Some(w) => w.buffer.clone(),
        None => return,
    };
    let char_pos = buf.borrow().text.byte_to_char(byte_pos);
    let mut b = buf.borrow_mut();
    let clamped = b.clamp(char_pos as i64);
    b.point = clamped;
    b.goal_column = None;
}

/// Arm the mark at `byte_pos` in window `win_id`'s buffer, the same
/// effect the `set-mark` elisp builtin has (`mark = Some(pos);
/// mark_active = true` -- `crates/core/src/builtins/editing.rs`). Only a
/// hand-set fallback; see `arm_mouse_selection`, its only caller, for
/// when this is used versus evil's own `evil-visual-char` path.
fn arm_mark(ed: &Rc<RefCell<Editor>>, win_id: usize, byte_pos: usize) {
    let buf = match ed.borrow().windows.get(&win_id) {
        Some(w) => w.buffer.clone(),
        None => return,
    };
    let char_pos = buf.borrow().text.byte_to_char(byte_pos);
    let mut b = buf.borrow_mut();
    b.mark = Some(char_pos);
    b.mark_active = true;
}

/// Arm a mouse-drag selection at `byte_pos` (the drag's press
/// position) in window `win_id`'s buffer, called once per drag -- right
/// before point is first moved away from the press position (see the
/// call site in `App::update`'s `PointerMoved` handling) -- and never
/// again for the rest of that same drag.
///
/// evil-mode is on by default in this editor and keeps its own state
/// machine on top of the plain mark: `evil--state` (a buffer-local
/// elisp variable, `'normal`/`'visual`/`'insert`/`'emacs`) and
/// `evil--visual-type` (`'char`/`'line`), both set by `evil-visual-char`
/// (`crates/core/lisp/evil.el`, the command bound to `v`) alongside
/// `set-mark`. A plain `set-mark`-equivalent write (`arm_mark`) leaves
/// `evil--state` untouched, so a mouse-drawn region highlights
/// correctly (`redisplay.rs`'s region painting reads `mark`/
/// `mark_active` directly) but evil's visual-state operators (`d`/`y`/
/// `c`/... while `evil--state` is `'visual`) do not recognize it as
/// their selection -- in `'normal` state (evil's default, and the state
/// a drag almost always starts a selection from) `y`/`d`/`c` are
/// operators waiting for a motion, and a bare mouse-made region is not
/// one.
///
/// So: when evil is on and the buffer is (at this exact moment) in
/// `'normal` state, this calls `evil-visual-char` itself via
/// `Interp::eval_source` -- the same public entry point
/// `core::idle_tick`'s async-process pumps already use to invoke a
/// named elisp function from Rust (`crates/core/src/lib.rs`) -- rather
/// than hand-setting `evil--state`/`evil--visual-type`/`mark` from Rust:
/// reusing evil's own transition keeps this in sync with whatever else
/// `evil--set-state` does (cursor shape, `inhibit-self-insert`, the
/// mode-line tag, ...) without duplicating that logic here. Point must
/// still be at `byte_pos` (the press position, not yet moved) when this
/// runs, because `evil-visual-char`'s own body is `(set-mark (point))`
/// -- it reads the *current* point to decide where the mark goes; the
/// caller is responsible for calling this before advancing point to the
/// drag's current position.
///
/// Every other case falls back to `arm_mark` (the plain hand-set write):
/// evil off; evil on but already in `'visual`/`'insert`/`'emacs` state
/// (reasoned about above -- keyboard-driven selection is untouched
/// either way, since this is only ever called from the mouse-drag path;
/// a drag that starts already inside evil's visual state does not get a
/// second, redundant `evil-visual-char` call, which would instead EXIT
/// visual state -- see that function's own `(if (and (eq evil--state
/// 'visual) ...` toggle; and a drag starting in insert/emacs state does
/// not unexpectedly switch evil state at all); **and also `'operator-
/// pending` and any other value `evil--state` can hold**, which this
/// function does not reason about at all -- `sym_var(...) == Some("normal")`
/// is false for those, so they take the same `else` branch as evil-off,
/// giving a plain mark with `evil--state` left wherever it was. Whether
/// a plain mark interacts sensibly with a *pending* operator (e.g. `d`
/// waiting for a motion, then the user drags the mouse instead of typing
/// one) has not been analysed or tested here -- flagged, not fixed, by
/// the mouse-support milestone review.
/// Fix 4 (mouse-support milestone review): whether a same-window
/// `PointerMoved` event during an active drag should call
/// `arm_mouse_selection` -- exactly once, the first time the drag
/// differs from its own press position. Factored out of the inline
/// `match` arm in `App::update` so this decision (previously entangled
/// with "should `place_point` run at all", the actual bug -- see the
/// call site's doc comment) has a unit test independent of a full
/// `eframe`/`egui` event-loop harness.
fn drag_should_arm(already_dragged: bool, byte_pos: usize, start_byte: usize) -> bool {
    !already_dragged && byte_pos != start_byte
}

fn arm_mouse_selection(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    win_id: usize,
    byte_pos: usize,
) {
    let evil_on = var_truthy(interp, "evil-mode");
    let evil_normal = sym_var(interp, "evil--state") == Some("normal");
    if evil_on && evil_normal {
        // Fix 5 (mouse-support milestone review): every other branch of
        // this function leaves `mark`/`mark_active` in a known state one
        // way or another. Silently swallowing a signal here (the old
        // `let _ = ...`) instead left neither `evil--state` transitioned
        // NOR any mark set at all -- the drag would proceed with no
        // selection and no diagnostic. Fall back to the plain hand-set
        // mark so a selection exists either way; evil's own state stays
        // whatever it was (unchanged by a failed `evil-visual-char`),
        // same as the `else` branch below already accepts for the
        // evil-off/non-normal cases.
        if interp.eval_source("(evil-visual-char)").is_err() {
            arm_mark(ed, win_id, byte_pos);
        }
    } else {
        arm_mark(ed, win_id, byte_pos);
    }
}

/// Whether `active`'s font pair needs to be replaced by `wanted` --
/// the "should we switch" half of M105's runtime font-switching check,
/// pulled out as a pure function so it (and the caching/bookkeeping half,
/// `apply_font_switch` below) can be tested without a live `eframe::App`
/// -- `App` has no public constructor and nothing in this crate can drive
/// `eframe::App::update` outside a real window, so the check that used to
/// live inline in `update` had no test at all (cold review, M105 fix
/// round): flipping `!=` to `==` (switch never fires), or deleting the
/// cache-reset calls (stale glyph ids replayed against a new atlas), or
/// deleting the `active_font` update (`install_fonts` reruns every single
/// frame, rebuilding egui's font atlas needlessly) would all have passed
/// every existing test.
pub fn font_switch_needed(
    active: &(String, Option<String>),
    wanted: &(String, Option<String>),
) -> bool {
    active.0 != wanted.0 || active.1 != wanted.1
}

/// The "what to do when we switch" half: resets `shape_cache`/
/// `glyph_cache` and records `wanted` as the new `active_font`. Callers
/// must have already installed the new fonts (`install_fonts`, which
/// needs a live `egui::Context` and so is not folded into this function)
/// before calling this -- this only updates the bookkeeping/cache state
/// that goes with that, which is why it takes no `Context` and is
/// trivially testable on its own.
///
/// Both resets are directly observable and covered by `font_tests.rs`.
/// `shape_cache`'s entries are keyed on `(FaceRole, text, enable_calt)`,
/// which does not encode which font produced them (shaping the same text through
/// the same cache under two different fonts must not replay the first
/// font's glyph ids for the second). `glyph_cache`'s entries are keyed on
/// `(FaceRole, glyph_id, scale)`, which likewise does not encode which
/// font rasterized them -- rasterizing the same numeric glyph id under
/// two different embedded fonts through the same cache with no reset
/// replays the first font's atlas UV rectangle for the second (confirmed
/// UV collision, not just a theoretical key clash).
///
/// M105 third fix round, corrected: an earlier version of this comment
/// claimed `glyph_cache`'s reset was "not independently observable
/// without a live egui texture atlas" and left it untested. That claim
/// was wrong -- `shaping` is `pub mod shaping` (this milestone made it
/// so), `build_shaped_mesh`/`finish_shaped_mesh`/`GlyphAtlasCache` are
/// all `pub`, and `egui::epaint::TextureAtlas::new` needs no live GUI at
/// all (`shaping.rs`'s own unit tests already construct one this way).
/// The gap was in what got tried, not in what is testable.
pub fn apply_font_switch(
    shape_cache: &mut ShapeCache,
    glyph_cache: &mut GlyphAtlasCache,
    active_font: &mut (String, Option<String>),
    wanted: (String, Option<String>),
) {
    *shape_cache = ShapeCache::default();
    *glyph_cache = GlyphAtlasCache::default();
    *active_font = wanted;
}

impl eframe::App for App {
    // M111: the trait default is `Color32::from_rgba_unmultiplied(12, 12,
    // 12, 180)` -- a fixed, non-configurable near-black wash eframe paints
    // behind the whole window before `update` draws anything, which would
    // show through at the window's edges (anywhere the `CentralPanel`'s
    // own fill doesn't reach, e.g. under macOS's rounded corners) and
    // fight with `gui-opacity`. Fully transparent here defers all of the
    // actual background painting to `CentralPanel`'s fill and the
    // per-cell backgrounds, which are the two places `gui-opacity` is
    // actually applied.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_start = std::time::Instant::now();

        // M105: runtime font switching. `gui-font`/`gui-font-family` are
        // read every frame (cheap: two symbol-table lookups), but
        // `install_fonts` -- which calls `ctx.set_fonts` and rebuilds
        // egui's font atlas -- only reruns when `font_switch_needed` says
        // the pair actually changed, since that rebuild is expensive.
        // See `font_switch_needed`/`apply_font_switch` for why this is
        // split into two pure-ish pieces rather than inlined here.
        let live_named_font = sym_var(&self.interp, "gui-font")
            .unwrap_or("jetbrains-mono")
            .to_string();
        let live_font_family = str_var(&self.interp, "gui-font-family");
        let wanted = (live_named_font, live_font_family);
        if font_switch_needed(&self.active_font, &wanted) {
            // Known gap (M105 fix round, not fixed -- see below):
            // `install_fonts`'s `ctx.set_fonts` only STORES the new
            // `FontDefinitions`; egui actually rebuilds the atlas and its
            // glyph metrics later, in its own `begin_pass` on the NEXT
            // frame. `char_w`/`row_h` are measured further down in this
            // same frame, via `ctx.fonts(|f| ...)`, so on the exact frame
            // a switch fires, that measurement can still reflect the OLD
            // font's metrics while glyphs are drawn from the NEW one --
            // a one-frame mismatch between grid geometry and glyph shape,
            // self-correcting on the very next frame once egui catches
            // up. Fixing this would mean reordering this call relative
            // to egui's own pass boundaries, which is not worth doing
            // for a single, self-correcting, sub-16ms frame.
            let (have_bold, have_italic, fonts) =
                install_fonts(ctx, wanted.1.as_deref(), &wanted.0);
            self.have_bold = have_bold;
            self.have_italic = have_italic;
            self.fonts = fonts;
            apply_font_switch(
                &mut self.shape_cache,
                &mut self.glyph_cache,
                &mut self.active_font,
                wanted,
            );
        }

        let (events, modifiers) = ctx.input(|i| (i.events.clone(), i.modifiers));
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    if modifiers.ctrl || modifiers.alt || modifiers.mac_cmd {
                        continue; // combos arrive as Key events
                    }
                    for c in text.chars() {
                        if c != '\n' && c != '\r' && c != '\t' {
                            handle_key(&mut self.interp, &self.ed, Key::Char(c as i64));
                        }
                    }
                }
                egui::Event::Paste(text) => {
                    for c in text.chars() {
                        let code = if c == '\n' { 13 } else { c as i64 };
                        handle_key(&mut self.interp, &self.ed, Key::Char(code));
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers: km,
                    ..
                } => {
                    if let Some(k) = convert_key(key, km) {
                        handle_key(&mut self.interp, &self.ed, k);
                    }
                }
                _ => {}
            }
        }
        if self.ed.borrow().quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // The unified idle tick (M15): pressure GC right after input,
        // async LSP pumping + idle GC on quiet frames.
        if !ctx.input(|i| i.events.is_empty()) {
            self.last_input = std::time::Instant::now();
            elisp::gc::maybe_auto_collect(&mut self.interp, false);
        } else {
            core::idle_tick(&mut self.interp, self.last_input.elapsed());
        }
        // Wake cadence (fix 4): fast while async results may arrive; a
        // fast ~33ms cadence while the cursor's smooth fade (task 6) is
        // actually animating AND the window has focus -- the 530ms fade
        // period sampled at the old idle 500ms landed about once per
        // cycle, which reads as the old hard blink with extra
        // arithmetic, not a smooth fade. Suppressed back to the plain
        // idle cadence right after input (`BLINK_SUPPRESS_WINDOW`, the
        // same window `cursor_alpha` uses to force the cursor fully
        // opaque -- fix D) and whenever the window isn't focused, so an
        // unfocused window doesn't spin at 33ms for no visible benefit.
        //
        // Fix A: uncapped, this 33ms cadence ran forever on any focused,
        // idle window -- about 15x the old idle repaint rate, purely to
        // animate a blinking cursor nobody is looking at mid-edit.
        // `blink_decision` (mirroring GNU's `blink-cursor-blinks`) reports
        // when the fade has run its course with no input; once it has,
        // `animating` is false and the cadence falls back to 500ms same
        // as if the window weren't focused at all.
        let focused = ctx.input(|i| i.focused);
        let elapsed_since_input = self.last_input.elapsed();
        let blink_suppressed = elapsed_since_input < BLINK_SUPPRESS_WINDOW;
        let cursor_period = 0.53_f32; // raised-cosine fade period, task 6
        let blink_limit = clamp_cursor_blinks(int_var(&self.interp, "gui-cursor-blinks", 10));
        let (animating, fade_fraction) =
            blink_decision(elapsed_since_input, cursor_period, blink_limit);
        let wake = if core::has_async_work(&mut self.interp) {
            100
        } else if focused && !blink_suppressed && animating {
            33
        } else {
            500
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(wake));

        let base = frame_base_style(&self.interp, &self.ed.borrow());
        // M111: read once per frame, same timing as every other `gui-*`
        // variable in this function -- `clamp_opacity`'s floor keeps the
        // window always findable, see its doc comment.
        let opacity_frac = frame_opacity_frac(&self.interp);
        let bg = frame_bg(base.bg, opacity_frac);
        let fg = base.fg.map(to_color).unwrap_or(FALLBACK_FG);
        let font_size = clamp_font_size(int_var(&self.interp, "gui-font-size", 16)) as f32;
        let cursor_type = sym_var(&self.interp, "cursor-type")
            .unwrap_or("box")
            .to_string();
        // Smooth fade (task 6) instead of a hard on/off blink: a raised
        // cosine over `cursor_period`, so the cursor eases in and out
        // rather than snapping. Suppressed (forced fully opaque) for
        // `BLINK_SUPPRESS_WINDOW` after input, same window the wake
        // cadence above uses (fix D); once `blink_decision` (fix A)
        // reports the cursor has stopped animating, `fade_fraction` is
        // already 1.0 (fully solid) so no separate check is needed here.
        let cursor_alpha = if blink_suppressed { 1.0 } else { fade_fraction };
        let cursor_face = face_style(&self.interp, &self.ed.borrow(), "cursor");
        // Padding + line spacing (task 2): read once per frame, applied
        // to both the grid's cols/rows computation and every pixel
        // position below, so the last column/row stays inside the
        // window instead of falling outside the padded area.
        let padding_x = clamp_padding(int_var(&self.interp, "gui-padding-x", 10)) as f32;
        let padding_y = clamp_padding(int_var(&self.interp, "gui-padding-y", 6)) as f32;
        let line_spacing =
            clamp_line_spacing(int_var(&self.interp, "gui-line-spacing", 100)) as f32 / 100.0;
        let indent_step = int_var_opt(&self.interp, "gui-indent-guide-step")
            .or_else(|| int_var_opt(&self.interp, "tab-width"))
            .unwrap_or(4)
            .max(0) as usize;
        // M116: read every frame, like every other `gui-*` variable.
        // Threaded straight into `ShapeCache::get_or_shape` below rather
        // than compared against a stored "last frame's value" to decide
        // whether to clear the cache -- see `shaping.rs`'s `ShapeCache`
        // doc for why baking it into the cache key makes a stale-cache
        // toggle impossible by construction instead of by remembering to
        // reset it.
        let gui_ligatures = ligatures_enabled(&self.interp);
        let ppp = ctx.pixels_per_point();

        let panel_frame = egui::Frame::none().fill(bg);
        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ctx, |ui| {
                let font = FontId::monospace(font_size);
                let bold_font = if self.have_bold {
                    FontId::new(font_size, FontFamily::Name(BOLD_FAMILY.into()))
                } else {
                    font.clone()
                };
                let italic_font = if self.have_italic {
                    FontId::new(font_size, FontFamily::Name(ITALIC_FAMILY.into()))
                } else {
                    font.clone()
                };
                let (char_w, base_row_h) =
                    ctx.fonts(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
                let row_h = base_row_h * line_spacing;
                let avail = ui.available_size();
                // Padding (task 2) is removed from the area before cols/rows
                // are derived, so the grid never claims a column/row that
                // would fall in the padded margin.
                let text_avail_x = (avail.x - 2.0 * padding_x).max(0.0);
                let text_avail_y = (avail.y - 2.0 * padding_y).max(0.0);
                let cols = dimension_floor((text_avail_x / char_w) as usize, 20);
                let rows = dimension_floor((text_avail_y / row_h) as usize, 5);
                self.ed.borrow_mut().frame = (cols, rows);

                let grid = render(&self.interp, &self.ed);
                let origin = ui.min_rect().min + Vec2::new(padding_x, padding_y);
                let painter = ui.painter();

                // Mouse (task 3): click-to-place-point, drag-to-select,
                // wheel-to-scroll. Uses this frame's just-rendered `grid`
                // for hit-testing, so a click's effect (point/mark move,
                // window selection) is applied here but only becomes
                // visible on the *next* frame's `render()` call, one
                // frame later -- the same latency every wake-cadence-
                // driven redraw in this app already has for anything
                // that isn't a keyboard key (those are handled earlier,
                // in `update`'s own event loop, before `render` runs).
                for event in ctx.input(|i| i.events.clone()) {
                    match event {
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            ..
                        } => {
                            if pressed {
                                if let Some((win_id, byte_pos)) =
                                    pixel_to_buffer_pos(&grid, origin, char_w, row_h, pos)
                                {
                                    place_point(&mut self.interp, &self.ed, win_id, byte_pos);
                                    self.drag = Some(DragState {
                                        win_id,
                                        start_byte: byte_pos,
                                        dragged: false,
                                    });
                                }
                            } else {
                                self.drag = None;
                            }
                        }
                        egui::Event::PointerMoved(pos) => {
                            // Fix 4 (mouse-support milestone review): this
                            // used to gate BOTH "arm the mark" and "move
                            // point at all" on the same `byte_pos !=
                            // drag.start_byte` condition. That's right for
                            // arming (the mark must be set exactly once,
                            // the first time the drag leaves the press
                            // position) but wrong for point: a drag that
                            // moves away and then comes back to exactly
                            // the start position must still update point
                            // TO the start position -- otherwise point is
                            // left wherever the drag last differed, and
                            // releasing there leaves a stale selection.
                            // Two separate decisions now: `moved`
                            // (confined to the originating window, same as
                            // before -- a drag never crosses windows) gates
                            // whether this event does anything at all;
                            // `!already_dragged && byte_pos != start_byte`
                            // gates arming, on its own; `place_point` runs
                            // on every `moved` event, unconditionally.
                            let moved = if let Some(drag) = &self.drag {
                                pixel_to_buffer_pos(&grid, origin, char_w, row_h, pos)
                                    .filter(|(win_id, _)| *win_id == drag.win_id)
                                    .map(|(win_id, byte_pos)| {
                                        (win_id, byte_pos, drag.dragged, drag.start_byte)
                                    })
                            } else {
                                None
                            };
                            if let Some((win_id, byte_pos, already_dragged, start_byte)) = moved {
                                let arm = drag_should_arm(already_dragged, byte_pos, start_byte);
                                if arm {
                                    if let Some(drag) = &mut self.drag {
                                        drag.dragged = true;
                                    }
                                    arm_mouse_selection(
                                        &mut self.interp,
                                        &self.ed,
                                        win_id,
                                        start_byte,
                                    );
                                }
                                place_point(&mut self.interp, &self.ed, win_id, byte_pos);
                            }
                        }
                        egui::Event::MouseWheel { unit, delta, .. } => {
                            if let Some(hover) = ctx.pointer_hover_pos() {
                                if let Some(win_id) =
                                    window_at_pixel(&grid, origin, char_w, row_h, hover)
                                {
                                    // `delta.y` follows egui's "content
                                    // moves with the gesture" convention
                                    // (positive = content moves down, see
                                    // `Event::MouseWheel`'s own doc) --
                                    // negated so scrolling down reveals
                                    // LATER text (window_start advances),
                                    // matching every other scrollable
                                    // view. `Line` units map 1:1 to a
                                    // scroll of that many lines; `Point`/
                                    // `Page` (trackpad / rare backends)
                                    // are converted via `row_h`/the
                                    // window's own text-row count so the
                                    // same physical gesture still moves a
                                    // sane number of lines instead of 0
                                    // (a fractional point delta below one
                                    // `row_h` truncating to zero) or an
                                    // entire screenful at once.
                                    let lines = match unit {
                                        egui::MouseWheelUnit::Line => -delta.y,
                                        egui::MouseWheelUnit::Point => -delta.y / row_h,
                                        egui::MouseWheelUnit::Page => {
                                            let win_rows = grid
                                                .windows
                                                .iter()
                                                .find(|w| w.win_id == win_id)
                                                .map(|w| w.rows.saturating_sub(1))
                                                .unwrap_or(1);
                                            -delta.y * win_rows as f32
                                        }
                                    };
                                    let delta_lines = lines.round() as i64;
                                    if delta_lines != 0 {
                                        core::redisplay::scroll_window_start(
                                            &self.ed,
                                            win_id,
                                            delta_lines,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                let pick_font = |style: &core::redisplay::Style| -> &FontId {
                    if style.bold && self.have_bold {
                        &bold_font
                    } else if style.italic && self.have_italic {
                        &italic_font
                    } else {
                        &font
                    }
                };

                // Shaping (task: coding ligatures): the shared font atlas
                // handle and its glyph-cache-invalidation check, both
                // fetched/run once per frame -- not once per row/run, see
                // `GlyphAtlasCache::begin_frame`'s doc for why a fresh
                // `Fonts::texture_atlas()` handle is the only available
                // signal that the atlas was rebuilt underneath the cache.
                let atlas = ctx.fonts(|f| f.texture_atlas());
                self.glyph_cache.begin_frame(&atlas);
                // Fix 1 (cold review, highest priority): every shaped
                // glyph's UVs must be normalized against the atlas size
                // as it stands after EVERY glyph this frame has been
                // rasterized -- reading `atlas.lock().size()` here,
                // before the row loop, and using that single value to
                // normalize each run as it's shaped (the previous
                // design) bakes stale UVs into any run shaped before a
                // later run grows the atlas (`TextureAtlas::allocate`
                // grows it in place, so `GlyphAtlasCache::begin_frame`'s
                // `Arc`-identity rebuild check does not fire). So
                // `build_shaped_mesh` below returns raw texel rects
                // (see its doc), collected here, and normalization
                // (`shaping::finish_shaped_mesh`) happens exactly once,
                // after the row loop, against the size the atlas
                // actually ends the frame at -- mirroring epaint's own
                // text path, which keeps `UvRect` raw through layout and
                // normalizes once at the very end.
                //
                // Deferring emission this way also means EVERY
                // background (pass 1, all rows) is painted before ANY
                // shaped text (pass 2, all rows) rather than
                // interleaved row by row, which only strengthens "text
                // must land on top of backgrounds" -- rows don't
                // overlap vertically, and a ligature's backward-bleeding
                // ink stays within its own run's Mesh, whose glyph
                // triangles are still added and drawn in the same
                // left-to-right order they always were.
                let mut pending_text: Vec<(Vec<shaping::RawGlyphRect>, Color32)> = Vec::new();

                for (row, line) in grid.lines.iter().enumerate() {
                    let y = origin.y + row_top(&grid, row_h, row);
                    // M87 stage 3: this row's own height (shorter for a
                    // `Block` diagnostic row) -- every use of `row_h`
                    // below as a *height* (not a multiplier applied to a
                    // row index) must use this instead.
                    let rh = row_height(&grid, row_h, row);
                    // Pass 1: backgrounds + underlines (per cell — rect fills
                    // are cheap tessellation, no batching needed).
                    for (col, cell) in line.iter().enumerate() {
                        if cell.continuation {
                            continue;
                        }
                        let x = origin.x + col as f32 * char_w;
                        let is_cursor = grid.cursor == (row, col);
                        let wide = col + 1 < grid.cols && line[col + 1].continuation;
                        let cell_w = if wide { char_w * 2.0 } else { char_w };
                        let eff = cell.style.or_default(&base);
                        let (mut cfg, mut cbg) = (
                            eff.fg.map(to_color).unwrap_or(fg),
                            eff.bg.map(|c| to_bg_color(c, opacity_frac)).unwrap_or(bg),
                        );
                        if eff.reverse {
                            std::mem::swap(&mut cfg, &mut cbg);
                            // M111 fix (measured: effective alpha 163
                            // instead of the requested 102 at gui-opacity
                            // 40 -- two 40% layers compositing): reassert
                            // role-correct alpha ONLY inside this branch,
                            // not unconditionally. Outside `eff.reverse`,
                            // `cbg` already carries `opacity_frac` from
                            // `to_bg_color` above; `with_alpha` reads
                            // `cbg`'s raw bytes and treats them as
                            // unmultiplied, but `Color32` stores gamma-
                            // premultiplied bytes -- re-applying `with_alpha`
                            // to an already-`with_alpha`'d color darkens the
                            // RGB a second time while leaving the alpha byte
                            // unchanged, which also makes `cbg` no longer
                            // bitwise-equal to the unmodified `bg` for a
                            // cell with no override, defeating the
                            // "skip painting a cell whose background
                            // matches the frame default" optimization below
                            // and causing a second, redundant `rect_filled`
                            // over the `CentralPanel` fill -- exactly the
                            // measured double-paint. Only the swap branch
                            // needs a fix-up: it can put an opaque
                            // `to_color` value (alpha 255, safe to
                            // `with_alpha` exactly once) into `cbg`.
                            cbg = with_alpha(cbg, opacity_frac);
                        }
                        // `to_opaque` un-premultiplies by the color's OWN
                        // current alpha rather than assuming unmultiplied
                        // input, so unlike `with_alpha` this is safe to
                        // call unconditionally -- a no-op on an
                        // already-opaque `cfg` (the non-reverse case).
                        cfg = cfg.to_opaque();
                        let mut cursor_rounding = 0.0_f32;
                        if is_cursor && cursor_type == "box" {
                            // Task 6 / fix 3: continuous fade for the
                            // background, hard switch at t=0.5 for the
                            // glyph color (see `cursor_glyph_color`'s doc
                            // comment for why lerping both by the same t
                            // is a zero-contrast bug, not just a style
                            // choice) -- and a `cursor` face overrides the
                            // swap target when the theme defines one.
                            //
                            // M111: `target_fg' is forced opaque in both
                            // branches -- it becomes the glyph color once
                            // the cursor is solid, and the glyph must never
                            // go translucent, even when it's sourced from
                            // `cbg' (a background-role color). `target_bg'
                            // is always already opaque (`to_color', or
                            // `cfg'/`fg' which are opaque too), so the box
                            // cursor itself fades toward a fully opaque
                            // highlight rather than blending with whatever
                            // is behind the window -- a translucent focus
                            // indicator would be exactly the "looks broken"
                            // failure mode this milestone's recon measured
                            // for other chrome, just relocated to the
                            // cursor.
                            let (target_fg, target_bg) = if let Some(cf) = &cursor_face {
                                (
                                    cf.fg.map(to_color).unwrap_or(cbg).to_opaque(),
                                    cf.bg.map(to_color).unwrap_or(cfg),
                                )
                            } else {
                                let mut swapped_bg = cfg;
                                if swapped_bg == bg.to_opaque() {
                                    swapped_bg = fg;
                                }
                                (cbg.to_opaque(), swapped_bg)
                            };
                            cfg = cursor_glyph_color(cfg, target_fg, cursor_alpha);
                            cbg = cursor_bg_color(cbg, target_bg, cursor_alpha);
                            cursor_rounding = 1.5;
                        }
                        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, rh));
                        // Fix 6d: a non-box cursor (bar/hbar) never
                        // recolors `cbg` above, so for those cursor types
                        // `is_cursor` alone used to force a same-colored
                        // `rect_filled` on top of the plain background --
                        // a wasted paint call every frame on the cursor's
                        // cell. Only paint when the color actually
                        // differs from the frame background, or the box
                        // cursor really did recolor this cell.
                        // M111: when `cbg != bg` (a cell with its own
                        // `:background', e.g. the mode line, `hl-line', a
                        // selection), this paints ON TOP OF the
                        // `CentralPanel' fill already sitting under it --
                        // two `gui-opacity'-alpha layers, compositing to
                        // `1 - (1 - p)^2', not `p'. That is intentional
                        // (see `gui-opacity''s docstring in `gui.el'), not
                        // a reappearance of the double-paint bug fixed
                        // earlier in M111 -- that bug hit cells with NO
                        // override, where this branch must NOT fire
                        // because `cbg == bg` exactly.
                        if cbg != bg || (is_cursor && cursor_type == "box") {
                            painter.rect_filled(snap_rect(rect, ppp), cursor_rounding, cbg);
                        }
                        match cell.style.underline {
                            Underline::None => {}
                            Underline::Straight => {
                                let uc = cell.style.underline_color.map(to_color).unwrap_or(cfg);
                                painter.line_segment(
                                    [
                                        Pos2::new(x, y + rh - 1.0),
                                        Pos2::new(x + cell_w, y + rh - 1.0),
                                    ],
                                    egui::Stroke::new(1.0_f32, uc),
                                );
                            }
                            Underline::Wave => {
                                let uc = cell.style.underline_color.map(to_color).unwrap_or(cfg);
                                let n = ((cell_w / 2.0).max(2.0)) as usize;
                                let step = cell_w / n as f32;
                                let b = y + rh - 1.5;
                                for k in 0..n {
                                    let x0 = x + k as f32 * step;
                                    let (y0, y1) = if k % 2 == 0 {
                                        (b, b - 1.5)
                                    } else {
                                        (b - 1.5, b)
                                    };
                                    painter.line_segment(
                                        [Pos2::new(x0, y0), Pos2::new(x0 + step, y1)],
                                        egui::Stroke::new(1.0_f32, uc),
                                    );
                                }
                            }
                        }
                    }

                    // Pass 2: text (task: coding-ligature shaping). Drawn
                    // from `grid.runs` (core's own same-style run
                    // grouping -- `PaintRun`, chrome included) instead of
                    // re-batching cells here: each run is tried through
                    // the shaper first, and only falls back to the
                    // pre-shaping per-cell path (`draw_cell_fallback`
                    // below, byte-for-byte the old per-glyph logic) when
                    // the run contains the box cursor, isn't
                    // column-uniform (a wide CJK char's cell width
                    // differs from its char count -- see `PaintRun`'s own
                    // doc), or shaping's result fails
                    // `shaping::validate_shaped_run`.
                    let draw_cell_fallback =
                        |col: usize, cell: &core::redisplay::Cell, painter: &egui::Painter| {
                            let is_cursor = grid.cursor == (row, col);
                            let cursor_boxed = is_cursor && cursor_type == "box";
                            let x = origin.x + col as f32 * char_w;
                            let eff = cell.style.or_default(&base);
                            let (mut cfg, mut cbg) = (
                                eff.fg.map(to_color).unwrap_or(fg),
                                eff.bg.map(|c| to_bg_color(c, opacity_frac)).unwrap_or(bg),
                            );
                            if eff.reverse {
                                std::mem::swap(&mut cfg, &mut cbg);
                                // M111 fix: see pass 1's identical comment
                                // -- only fix up `cbg` when the swap
                                // actually happened (it then holds an
                                // opaque `to_color` value, safe to
                                // `with_alpha` exactly once). Applying
                                // `with_alpha` unconditionally here double-
                                // darkens `cbg` when it already carries
                                // `opacity_frac` from `to_bg_color` above,
                                // same bug as pass 1's, even though this
                                // closure never paints `cbg` directly (it
                                // only paints glyph ink with `cfg`) -- `cbg`
                                // still feeds the cursor-swap target colors
                                // below, and a double-darkened value there
                                // would leave `cfg` duller than intended
                                // after `.to_opaque()` un-premultiplies it.
                                cbg = with_alpha(cbg, opacity_frac);
                            }
                            // `to_opaque` un-premultiplies by the color's
                            // OWN current alpha, so unlike `with_alpha`
                            // this is safe unconditionally -- a no-op on
                            // an already-opaque `cfg`.
                            cfg = cfg.to_opaque();
                            if cursor_boxed {
                                // Same hard-switch glyph color + `cursor`-
                                // face override as pass 1's cell
                                // background (fix 3), applied here to the
                                // character's own color (this is "the
                                // character drawn under a box cursor" from
                                // task 6). See pass 1's identical block for
                                // why `target_fg' is forced opaque (M111).
                                let (target_fg, target_bg) = if let Some(cf) = &cursor_face {
                                    (
                                        cf.fg.map(to_color).unwrap_or(cbg).to_opaque(),
                                        cf.bg.map(to_color).unwrap_or(cfg),
                                    )
                                } else {
                                    let mut swapped_bg = cfg;
                                    if swapped_bg == bg.to_opaque() {
                                        swapped_bg = fg;
                                    }
                                    (cbg.to_opaque(), swapped_bg)
                                };
                                let _ = target_bg; // only cfg (the glyph color) is drawn here
                                cfg = cursor_glyph_color(cfg, target_fg, cursor_alpha);
                            }
                            if cell.ch != ' ' {
                                let f = pick_font(&eff);
                                painter.text(
                                    Pos2::new(x, y),
                                    egui::Align2::LEFT_TOP,
                                    cell.ch,
                                    f.clone(),
                                    cfg,
                                );
                                if eff.bold && !self.have_bold {
                                    // Task 8: fake-bold double-draw for
                                    // when the real bold font wasn't
                                    // available.
                                    painter.text(
                                        Pos2::new(x + 0.4, y),
                                        egui::Align2::LEFT_TOP,
                                        cell.ch,
                                        f.clone(),
                                        cfg,
                                    );
                                }
                            }
                        };

                    for r in grid.runs.iter().filter(|r| r.row == row) {
                        if r.cols == 0 || r.text.trim().is_empty() {
                            continue;
                        }
                        let cursor_in_run = grid.cursor.0 == row
                            && cursor_type == "box"
                            && grid.cursor.1 >= r.col
                            && grid.cursor.1 < r.col + r.cols;
                        // Column-uniform: every char in this run occupies
                        // exactly one display column. A wide (CJK) char
                        // contributes 2 to `r.cols` but 1 to
                        // `text.chars().count()`, so this also rules out
                        // any run shaping could not map cell-for-cell even
                        // before asking the shaper.
                        let column_uniform = r.text.chars().count() == r.cols;
                        let eff = r.style.or_default(&base);
                        let role = face_role(&eff, self.have_bold, self.have_italic);

                        // Fix 6 (cold review): the box cursor used to
                        // disable shaping for the WHOLE run -- and a
                        // `PaintRun` is grouped by style and
                        // byte-contiguity with no cursor awareness, so
                        // that's usually the entire line. The
                        // pre-shaping code broke its batch only at the
                        // cursor's own cell and resumed immediately
                        // after; restore that granularity by splitting a
                        // column-uniform run into the segment(s) either
                        // side of the cursor's cell (in absolute display
                        // columns) and shaping each independently. A
                        // non-column-uniform run (e.g. containing a wide
                        // CJK char) still always falls back below,
                        // exactly as before.
                        let mut segments: Vec<(usize, usize, &str)> = Vec::new();
                        if column_uniform {
                            if cursor_in_run {
                                let split_col = grid.cursor.1;
                                let split_idx = split_col - r.col;
                                let before_end = r
                                    .text
                                    .char_indices()
                                    .nth(split_idx)
                                    .map(|(b, _)| b)
                                    .unwrap_or(r.text.len());
                                let after_start = r
                                    .text
                                    .char_indices()
                                    .nth(split_idx + 1)
                                    .map(|(b, _)| b)
                                    .unwrap_or(r.text.len());
                                let before_text = &r.text[..before_end];
                                let after_text = &r.text[after_start..];
                                if !before_text.trim().is_empty() {
                                    segments.push((r.col, split_col, before_text));
                                }
                                if !after_text.trim().is_empty() {
                                    segments.push((split_col + 1, r.col + r.cols, after_text));
                                }
                            } else {
                                segments.push((r.col, r.col + r.cols, r.text.as_str()));
                            }
                        }

                        let (mut cfg, cbg) = (
                            eff.fg.map(to_color).unwrap_or(fg),
                            eff.bg.map(|c| to_bg_color(c, opacity_frac)).unwrap_or(bg),
                        );
                        if eff.reverse {
                            // M111: `cbg` is background-role (translucent
                            // under `gui-opacity`); forced opaque here
                            // since this becomes glyph ink, and glyph ink
                            // must never go translucent.
                            cfg = cbg.to_opaque();
                        }

                        // Marks, relative to `r.col`, which columns of
                        // this run a successfully shaped segment already
                        // covers -- any column left `false` (the
                        // cursor's own cell, a segment whose shaping
                        // failed `validate_shaped_run`, or every column
                        // when the run wasn't column-uniform at all)
                        // falls through to `draw_cell_fallback` below.
                        let mut shaped_cols = vec![false; r.cols];
                        if let Some(face) = self.fonts.for_role(role) {
                            let scale = shaping::scale_in_pixels(&face.ab, font_size, ppp);
                            let ascent = shaping::ascent_in_points(&face.ab, scale, ppp);
                            let baseline_y = y + ascent;
                            for (seg_start, seg_end, seg_text) in &segments {
                                let Some(glyphs) = self.shape_cache.get_or_shape(
                                    role,
                                    &face.loaded,
                                    seg_text,
                                    gui_ligatures,
                                ) else {
                                    continue;
                                };
                                let row_x = origin.x + *seg_start as f32 * char_w;
                                let items = shaping::build_shaped_mesh(
                                    &atlas,
                                    &mut self.glyph_cache,
                                    role,
                                    &face.ab,
                                    &glyphs,
                                    row_x,
                                    baseline_y,
                                    char_w,
                                    scale,
                                    ppp,
                                );
                                pending_text.push((items, cfg));
                                if eff.bold && !self.have_bold {
                                    // Fake bold: re-draw with a
                                    // fractional offset, same as the
                                    // pre-shaping path.
                                    let items2 = shaping::build_shaped_mesh(
                                        &atlas,
                                        &mut self.glyph_cache,
                                        role,
                                        &face.ab,
                                        &glyphs,
                                        row_x + 0.4,
                                        baseline_y,
                                        char_w,
                                        scale,
                                        ppp,
                                    );
                                    pending_text.push((items2, cfg));
                                }
                                for col in *seg_start..*seg_end {
                                    shaped_cols[col - r.col] = true;
                                }
                            }
                        }

                        // Fallback: exactly the pre-shaping per-cell
                        // path, scoped to the columns of this run no
                        // shaped segment covered.
                        for (col, cell) in line.iter().enumerate().skip(r.col).take(r.cols) {
                            if cell.continuation || shaped_cols[col - r.col] {
                                continue;
                            }
                            draw_cell_fallback(col, cell, painter);
                        }
                    }
                }

                // Fix 1: normalize and paint every shaped run collected
                // above, now that rasterization for the whole frame is
                // done -- see the comment above `pending_text`'s
                // declaration.
                let atlas_size = atlas.lock().size();
                for (items, color) in &pending_text {
                    let mesh = shaping::finish_shaped_mesh(items, atlas_size, *color);
                    painter.add(egui::Shape::mesh(mesh));
                }

                // Indent guides (task 3): a 1-device-px vertical line at
                // the left edge of each indent stop. `indent_guide_columns`
                // is the pure, independently-tested part; this just maps
                // its output onto pixel positions. Task 7 (GUI fix 2a):
                // computed per published window (`grid.windows`), over
                // that window's own text rows/columns only, offset past
                // its gutter -- not over the whole grid, which used to
                // put guide columns inside the line-number gutter and
                // let one window's indentation bleed into another's rows.
                if indent_step > 0 {
                    let guide_color = face_style(&self.interp, &self.ed.borrow(), "indent-guide")
                        .and_then(|f| f.fg)
                        .map(to_color)
                        .unwrap_or_else(|| with_alpha(fg, 0.12));
                    for win in &grid.windows {
                        let text_rows = win.rows.saturating_sub(1); // exclude mode line row
                        let text_col0 = win.col + win.gutter_cols;
                        let text_cols = win.cols.saturating_sub(win.gutter_cols);
                        if text_rows == 0 || text_cols == 0 {
                            continue;
                        }
                        let row_texts: Vec<String> = (0..text_rows)
                            .map(|r| {
                                grid.lines[win.row + r][text_col0..text_col0 + text_cols]
                                    .iter()
                                    .filter(|c| !c.continuation)
                                    .map(|c| c.ch)
                                    .collect::<String>()
                            })
                            .collect();
                        let row_refs: Vec<&str> = row_texts.iter().map(|s| s.as_str()).collect();
                        for (r, cols) in indent_guide_columns(&row_refs, indent_step)
                            .into_iter()
                            .enumerate()
                        {
                            let y = origin.y + row_top(&grid, row_h, win.row + r);
                            let rh = row_height(&grid, row_h, win.row + r);
                            for col in cols {
                                let x = origin.x + (text_col0 + col) as f32 * char_w;
                                let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(1.0, rh));
                                painter.rect_filled(snap_rect(rect, ppp), 0.0, guide_color);
                            }
                        }
                    }
                }

                // Fill-column indicator (M116): a 1-device-px vertical
                // rule at `fill-column`, drawn per published window
                // (`grid.windows`), same shape as the indent guides just
                // above -- both are "a vertical rule at a column", and
                // this follows that code rather than inventing a second
                // way to turn a column into a pixel `x`. Two differences
                // from indent guides: (1) `fill-column` and
                // `display-fill-column-indicator-mode` are buffer-local
                // (GNU's own names), so each window's buffer is looked up
                // individually via `buffer_int_var`/`buffer_var_on`
                // rather than reading one global variable once for the
                // whole frame; (2) the rule spans the window's full
                // height unconditionally (it marks a column, not
                // indentation actually present on each row), so this
                // loops rows only to get each row's own `row_top`/
                // `row_height` (block/diagnostic rows scale differently,
                // same as indent guides do NOT need to -- but this does,
                // since it must still line up when a diagnostic row
                // changes `row_h` for one row in the middle of the
                // window).
                //
                // Collision with indent guides: at typical settings
                // (`fill-column' 100, indentation rarely past column 20)
                // the two essentially never land on the same column, but
                // nothing stops it if they do -- this block runs AFTER
                // the indent-guide block above, so `painter.rect_filled`
                // draws its shapes on top, meaning the fill-column-
                // indicator face wins the pixels on any column both would
                // otherwise draw.
                {
                    let fill_col_color =
                        face_style(&self.interp, &self.ed.borrow(), "fill-column-indicator")
                            .and_then(|f| f.fg)
                            .map(to_color)
                            .unwrap_or_else(|| with_alpha(fg, 0.2));
                    for win in &grid.windows {
                        let text_rows = win.rows.saturating_sub(1); // exclude mode line row
                        let text_col0 = win.col + win.gutter_cols;
                        let text_cols = win.cols.saturating_sub(win.gutter_cols);
                        if text_rows == 0 || text_cols == 0 {
                            continue;
                        }
                        let buf = self
                            .ed
                            .borrow()
                            .windows
                            .get(&win.win_id)
                            .map(|w| w.buffer.clone());
                        let Some(buf) = buf else { continue };
                        let ed = self.ed.borrow();
                        if !buffer_var_on(
                            &self.interp,
                            &ed,
                            &buf,
                            "display-fill-column-indicator-mode",
                        ) {
                            continue;
                        }
                        let fill_column =
                            buffer_int_var(&self.interp, &ed, &buf, "fill-column", 100).max(0)
                                as usize;
                        drop(ed);
                        if !fill_column_visible(fill_column, text_cols) {
                            continue;
                        }
                        for r in 0..text_rows {
                            let y = origin.y + row_top(&grid, row_h, win.row + r);
                            let rh = row_height(&grid, row_h, win.row + r);
                            let x = fill_column_x(origin.x, text_col0, fill_column, char_w);
                            let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(1.0, rh));
                            painter.rect_filled(snap_rect(rect, ppp), 0.0, fill_col_color);
                        }
                    }
                }

                // Hairline separators (task 4 / GUI fix 2b): immediately
                // above the echo row (always the frame's last row,
                // regardless of window layout) and above EACH window's own
                // mode-line row (from `grid.windows`, spanning only that
                // window's columns) -- not the frame's last-two-rows
                // guess, which was only correct for one unsplit window
                // with no panel open.
                {
                    let sep_color = with_alpha(fg, 0.08);
                    if grid.rows >= 1 {
                        let row = grid.rows - 1;
                        let y = origin.y + row_top(&grid, row_h, row);
                        let rect = Rect::from_min_size(
                            Pos2::new(origin.x, y - 1.0),
                            Vec2::new(cols as f32 * char_w, 1.0),
                        );
                        painter.rect_filled(snap_rect(rect, ppp), 0.0, sep_color);
                    }
                    for win in &grid.windows {
                        let y = origin.y + row_top(&grid, row_h, win.mode_line_row);
                        let x0 = origin.x + win.col as f32 * char_w;
                        let rect = Rect::from_min_size(
                            Pos2::new(x0, y - 1.0),
                            Vec2::new(win.cols as f32 * char_w, 1.0),
                        );
                        painter.rect_filled(snap_rect(rect, ppp), 0.0, sep_color);
                    }
                }

                // Overlay scrollbar (task 5 / GUI fix 2c): only when the
                // buffer has more lines than fit in the window.
                // `window_start`/`total_lines` both already exist on the
                // public `GapBuffer` API, so no new accessor was needed on
                // `Editor`/`Buffer`. Drawn beside the SELECTED window
                // specifically (matched by `win_id` in `grid.windows`),
                // spanning that window's own rows -- not the whole frame
                // height, which put the bar in the wrong place next to any
                // window other than a full-height, unsplit one.
                {
                    let ed_ref = self.ed.borrow();
                    let sel_id = ed_ref.selected_window;
                    if let (Some(win), Some(layout)) = (
                        ed_ref.windows.get(&sel_id),
                        grid.windows.iter().find(|w| w.win_id == sel_id),
                    ) {
                        let buf = win.buffer.borrow();
                        let total_lines = buf.text.total_lines();
                        let top_line0 = buf.text.line_number(win.window_start).saturating_sub(1);
                        let visible_lines = layout.rows.saturating_sub(1); // exclude mode line
                        let track_top = origin.y + row_top(&grid, row_h, layout.row);
                        // M87 stage 3: the exact pixel span of the text
                        // area (top of `layout.row` to top of its mode
                        // line), not `visible_lines as f32 * row_h` --
                        // a window showing inline diagnostic rows has
                        // some of its `visible_lines` budget spent on
                        // shorter (75%) rows, so the old uniform formula
                        // would overstate the track's pixel height.
                        let track_len = row_top(&grid, row_h, layout.mode_line_row)
                            - row_top(&grid, row_h, layout.row);
                        let text_right = origin.x + (layout.col + layout.cols) as f32 * char_w;
                        let bar_x = text_right - 2.0 - 6.0;
                        if let Some((thumb_top, thumb_len)) =
                            scrollbar_thumb(total_lines, visible_lines, top_line0, track_len, 24.0)
                        {
                            // Overview-ruler diagnostic marks (M115):
                            // follows the scrollbar's own visibility
                            // rule (this whole block is inside
                            // `scrollbar_thumb`'s `Some` arm) -- if the
                            // buffer fits and no scrollbar is shown,
                            // there's nothing off-screen to point at, so
                            // no marks either. Drawn BEFORE the thumb,
                            // so the thumb paints over whatever a mark
                            // occupies underneath it -- a mark that
                            // coincides with the thumb describes a
                            // diagnostic that's already in the visible
                            // window (the thumb *is* the visible range),
                            // so nothing is lost by letting the thumb
                            // win there; the marks that matter
                            // (off-screen ones) are, by construction,
                            // outside the thumb and stay fully visible.
                            // Inset a px on each side of the 6px track
                            // so a mark reads as a tick, not a second
                            // thumb, matching VS Code's overview ruler.
                            // Deliberately diagnostics-only: search hits
                            // aren't marked here because
                            // `Editor::isearch` only stores the current
                            // match, not every match in the buffer, so a
                            // full-buffer scan would be new plumbing --
                            // that belongs with search work, not this
                            // one.
                            let diag_lines = window_diagnostic_lines(&ed_ref, &win.buffer);
                            let marks = scrollbar_diagnostic_marks(
                                diag_lines.iter().copied(),
                                total_lines,
                                track_len,
                                3.0,
                            );
                            for (mark_top, mark_len, sev) in marks {
                                let color = to_color(core::redisplay::severity_color(
                                    &self.interp,
                                    &ed_ref,
                                    sev,
                                ));
                                let rect = Rect::from_min_size(
                                    Pos2::new(bar_x + 1.0, track_top + mark_top),
                                    Vec2::new(4.0, mark_len),
                                );
                                painter.rect_filled(snap_rect(rect, ppp), 1.0, color);
                            }

                            let sb_color = face_style(&self.interp, &ed_ref, "scroll-bar")
                                .and_then(|f| f.fg.or(f.bg))
                                .map(to_color)
                                .unwrap_or_else(|| with_alpha(fg, 0.20));
                            let rect = Rect::from_min_size(
                                Pos2::new(bar_x, track_top + thumb_top),
                                Vec2::new(6.0, thumb_len),
                            );
                            painter.rect_filled(snap_rect(rect, ppp), 3.0, sb_color);
                        }
                    }
                }

                // Non-box cursor shapes (bar / hbar), drawn over the text
                // as a true alpha-blended overlay (task 6's smooth fade) --
                // unlike the box cursor, there's no cell-colored background
                // to lerp toward, so this uses the alpha channel directly.
                if cursor_type != "box" {
                    let (crow, ccol) = grid.cursor;
                    let x = origin.x + ccol as f32 * char_w;
                    let y = origin.y + row_top(&grid, row_h, crow);
                    let crh = row_height(&grid, row_h, crow);
                    let accent_rgb = cursor_face
                        .as_ref()
                        .and_then(|cf| cf.bg.or(cf.fg))
                        .map(to_color)
                        .unwrap_or_else(|| {
                            to_color(
                                core::redisplay::frame_base_style(&self.interp, &self.ed.borrow())
                                    .fg
                                    .unwrap_or((212, 212, 212)),
                            )
                        });
                    let accent = Color32::from_rgba_unmultiplied(
                        accent_rgb.r(),
                        accent_rgb.g(),
                        accent_rgb.b(),
                        (cursor_alpha * 255.0).round() as u8,
                    );
                    match cursor_type.as_str() {
                        "bar" => {
                            painter.rect_filled(
                                snap_rect(
                                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, crh)),
                                    ppp,
                                ),
                                1.5,
                                accent,
                            );
                        }
                        _ => {
                            painter.rect_filled(
                                snap_rect(
                                    Rect::from_min_size(
                                        Pos2::new(x, y + crh - 2.0),
                                        Vec2::new(char_w, 2.0),
                                    ),
                                    ppp,
                                ),
                                1.5,
                                accent,
                            );
                        }
                    }
                }

                // Hover popup (M16): floating panel just under the cursor.
                let popup = self.ed.borrow().hover_popup.clone();
                if let Some(text) = popup {
                    let (crow, ccol) = grid.cursor;
                    let px = origin.x + ccol as f32 * char_w;
                    let py = origin.y
                        + row_top(&grid, row_h, crow)
                        + row_height(&grid, row_h, crow)
                        + 2.0;
                    egui::Area::new(egui::Id::new("hover-popup"))
                        .fixed_pos(Pos2::new(px, py))
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
                            // M111: `.to_opaque()` here is deliberate, not
                            // a leftover to "fix" once `gui-opacity` makes
                            // `bg` translucent -- VS Code and JetBrains
                            // both keep hover popups opaque even with a
                            // translucent editor background, because a
                            // translucent popup over translucent text
                            // underneath it is unreadable.
                            //
                            // Known cosmetic erosion (M111 cold review,
                            // round 2, not fixed -- the opacity/legibility
                            // guarantee above still holds): `gamma_multiply`
                            // scales the stored gamma-premultiplied bytes
                            // INCLUDING alpha, while `to_opaque`
                            // un-premultiplies in linear space -- the two
                            // do not commute once `bg` already carries an
                            // alpha < 255. Measured: the intended "popup is
                            // 10% darker than the background" effect is
                            // correct at `gui-opacity` 100, nearly
                            // cancelled at 40, and gone entirely at the
                            // floor of 20. The popup itself stays fully
                            // opaque at every setting either way -- only
                            // the cosmetic darkening relative to the
                            // background erodes as opacity drops.
                            egui::Frame::popup(ui.style())
                                .fill(bg.gamma_multiply(0.9).to_opaque())
                                .stroke(egui::Stroke::new(1.0_f32, fg.gamma_multiply(0.4)))
                                .rounding(8.0)
                                .inner_margin(10.0)
                                .shadow(egui::epaint::Shadow {
                                    offset: Vec2::new(0.0, 4.0),
                                    blur: 16.0,
                                    spread: 0.0,
                                    color: Color32::from_black_alpha(96),
                                })
                                .show(ui, |ui| {
                                    ui.set_max_width(560.0);
                                    ui.label(
                                        egui::RichText::new(text)
                                            .font(FontId::monospace(font_size - 2.0))
                                            .color(fg),
                                    );
                                });
                        });
                }

                // Frame-time overlay (M16 debug): (setq gui-debug-overlay t).
                if var_truthy(&self.interp, "gui-debug-overlay") {
                    let label = format!("{:.2} ms", self.last_frame_ms);
                    painter.text(
                        Pos2::new(origin.x + avail.x - 8.0, origin.y + 4.0),
                        egui::Align2::RIGHT_TOP,
                        label,
                        FontId::monospace(11.0),
                        Color32::from_rgb(0x88, 0xcc, 0x88),
                    );
                }
            });
        self.last_frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;

        // M88: after this frame's own `core::idle_tick` call above (and
        // after everything drawn this frame has been queued for
        // painting), tell the autostart machinery a real frame has
        // happened -- see `frontend_started`'s own doc for why this sits
        // at the very end rather than alongside the `idle_tick` call.
        if !self.frontend_started {
            core::frontend_started(&mut self.interp);
            self.frontend_started = true;
        }
    }
}

fn to_color(c: core::redisplay::Color) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

/// Mirrors `pick_font`'s bold-before-italic selection (`lib.rs`'s paint
/// loop), as a plain function so it can be evaluated before `self.fonts`
/// is borrowed for the shaping lookup itself -- `pick_font` is a closure
/// over `self.have_bold`/`self.have_italic` and returns a `&FontId`,
/// which isn't what the shaping path (indexing `FontSet` by `FaceRole`)
/// needs.
fn face_role(style: &core::redisplay::Style, have_bold: bool, have_italic: bool) -> FaceRole {
    if style.bold && have_bold {
        FaceRole::Bold
    } else if style.italic && have_italic {
        FaceRole::Italic
    } else {
        FaceRole::Regular
    }
}

fn convert_key(key: egui::Key, m: egui::Modifiers) -> Option<Key> {
    let meta = m.alt || m.mac_cmd;
    let ctrl = m.ctrl;
    // Special keys work regardless of modifiers.
    let special: Option<Key> = match key {
        egui::Key::Enter => Some(Key::Char(13)),
        egui::Key::Tab => Some(Key::Char(9)),
        egui::Key::Backspace => Some(Key::Char(127)),
        egui::Key::Escape => Some(Key::Char(27)),
        egui::Key::ArrowUp => Some(Key::Sym("up".into())),
        egui::Key::ArrowDown => Some(Key::Sym("down".into())),
        egui::Key::ArrowLeft => Some(Key::Sym("left".into())),
        egui::Key::ArrowRight => Some(Key::Sym("right".into())),
        egui::Key::Home => Some(Key::Sym("home".into())),
        egui::Key::End => Some(Key::Sym("end".into())),
        egui::Key::PageUp => Some(Key::Sym("prior".into())),
        egui::Key::PageDown => Some(Key::Sym("next".into())),
        egui::Key::Delete => Some(Key::Sym("deletechar".into())),
        _ => None,
    };
    if let Some(k) = special {
        // M-RET and friends: meta applies to special chars too.
        if let (Key::Char(code), true) = (&k, meta) {
            return Some(Key::Char(code | META));
        }
        return Some(k);
    }
    // Letter/digit/punctuation combos only matter with C- or M- held
    // (plain presses arrive as Text events).
    if !ctrl && !meta {
        return None;
    }
    let base: char = key_base_char(key, m.shift)?;
    let mut code = if ctrl { ctrl_encode(base) } else { base as i64 };
    if meta {
        code |= META;
    }
    Some(Key::Char(code))
}

#[rustfmt::skip]
fn key_base_char(key: egui::Key, shift: bool) -> Option<char> {
    use egui::Key as K;
    let c = match key {
        K::A => 'a', K::B => 'b', K::C => 'c', K::D => 'd', K::E => 'e',
        K::F => 'f', K::G => 'g', K::H => 'h', K::I => 'i', K::J => 'j',
        K::K => 'k', K::L => 'l', K::M => 'm', K::N => 'n', K::O => 'o',
        K::P => 'p', K::Q => 'q', K::R => 'r', K::S => 's', K::T => 't',
        K::U => 'u', K::V => 'v', K::W => 'w', K::X => 'x', K::Y => 'y',
        K::Z => 'z',
        K::Num0 => '0', K::Num1 => '1', K::Num2 => '2', K::Num3 => '3',
        K::Num4 => '4', K::Num5 => '5', K::Num6 => '6', K::Num7 => '7',
        K::Num8 => '8', K::Num9 => '9',
        K::Space => ' ',
        K::Minus => if shift { '_' } else { '-' },
        // M101: `Ctrl' held with `=' was dropped by `convert_key'
        // before ever reaching the keymap, because this whitelist had no
        // arm for `Key::Equals' (nor for `Key::Plus', the numpad one);
        // both exist in egui 0.29, confirmed against its own
        // `data/key.rs'. `K::Minus' above is NOT part of that gap -- it
        // has been here since M5, so `C--' always decoded fine and was
        // merely unbound until `expand-region.el'. `C-=' and `C--' are
        // the two that file binds (to `expand-region' and
        // `contract-region' respectively); `Ctrl+Shift+=' now decodes to
        // `C-+' as well, but nothing binds that combination yet.
        K::Equals => if shift { '+' } else { '=' },
        K::Plus => '+',
        K::Slash => if shift { '?' } else { '/' },
        K::Period => if shift { '>' } else { '.' },
        K::Comma => if shift { '<' } else { ',' },
        K::Semicolon => if shift { ':' } else { ';' },
        _ => return None,
    };
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Task 1: device-pixel snapping -------------------------------

    #[test]
    fn snap_rect_maps_fractional_coords_to_integers() {
        let ppp = 2.0_f32; // Retina-like: 2 device px per logical px.
        let r = Rect::from_min_size(Pos2::new(10.3, 20.7), Vec2::new(8.4, 21.5));
        let snapped = snap_rect(r, ppp);
        for v in [snapped.min.x, snapped.min.y, snapped.max.x, snapped.max.y] {
            let device = v * ppp;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "{v} * ppp = {device} is not a whole device pixel"
            );
        }
    }

    #[test]
    fn snap_rect_adjacent_cells_share_an_edge_exactly() {
        let ppp = 2.0_f32;
        let char_w = 8.43_f32; // deliberately not a whole number of device px
        let row_h = 21.5_f32;
        let origin = Pos2::new(0.0, 0.0);
        let cell = |col: usize| {
            let x = origin.x + col as f32 * char_w;
            let r = Rect::from_min_size(Pos2::new(x, origin.y), Vec2::new(char_w, row_h));
            snap_rect(r, ppp)
        };
        let a = cell(3);
        let b = cell(4);
        assert_eq!(
            a.max.x, b.min.x,
            "cell 3's right edge must equal cell 4's left edge"
        );
        assert_eq!(a.max.y, b.max.y);
    }

    // --- M116: fill-column paint-block guards (review fix: named as
    // otherwise-uncovered mutations -- `fill_column_visible` and the
    // buffer-local reads the mode gate depends on) ---------------------

    #[test]
    fn fill_column_visible_true_when_strictly_inside_the_text_area() {
        assert!(fill_column_visible(100, 120));
    }

    #[test]
    fn fill_column_visible_false_at_the_boundary_column() {
        // `fill_column == text_cols` is one past the last real column,
        // not on it -- must not draw there.
        assert!(!fill_column_visible(100, 100));
    }

    #[test]
    fn fill_column_visible_false_past_the_text_area() {
        assert!(!fill_column_visible(150, 100));
    }

    #[test]
    fn buffer_int_var_reads_the_global_default() {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        let buf = ed.borrow().current.clone();
        // `fill-column` is already `defvar`'d to 100 in `simple.el`,
        // loaded by `init_editor` -- this reads that live global value,
        // not the fallback argument (a separate assertion below pins
        // the fallback itself, for a variable that isn't defined at all).
        assert_eq!(
            buffer_int_var(&interp, &ed.borrow(), &buf, "fill-column", 999),
            100
        );
    }

    #[test]
    fn buffer_int_var_falls_back_when_the_variable_does_not_exist() {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        let buf = ed.borrow().current.clone();
        assert_eq!(
            buffer_int_var(&interp, &ed.borrow(), &buf, "no-such-variable-m116", 42),
            42
        );
    }

    #[test]
    fn buffer_int_var_honors_a_buffer_local_override() {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        if let Err(flow) = interp.eval_source("(setq-local fill-column 72)") {
            panic!("eval failed: {}", interp.describe_flow(&flow));
        }
        let buf = ed.borrow().current.clone();
        assert_eq!(
            buffer_int_var(&interp, &ed.borrow(), &buf, "fill-column", 100),
            72
        );
    }

    #[test]
    fn buffer_var_on_reflects_the_fill_column_indicator_mode_toggle() {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        let buf = ed.borrow().current.clone();
        assert!(!buffer_var_on(
            &interp,
            &ed.borrow(),
            &buf,
            "display-fill-column-indicator-mode"
        ));
        if let Err(flow) = interp.eval_source("(setq-local display-fill-column-indicator-mode t)") {
            panic!("eval failed: {}", interp.describe_flow(&flow));
        }
        assert!(buffer_var_on(
            &interp,
            &ed.borrow(),
            &buf,
            "display-fill-column-indicator-mode"
        ));
    }

    // --- M116: fill-column ruler column-to-x mapping ------------------

    #[test]
    fn fill_column_x_no_gutter_no_padding() {
        assert_eq!(fill_column_x(0.0, 0, 100, 8.0), 800.0);
    }

    #[test]
    fn fill_column_x_accounts_for_gutter_and_window_origin() {
        // A window whose text starts at column 5 (gutter width 5) inside
        // a split layout whose own pixel origin is 120.0 (a second
        // window to the right of a vertical split, say).
        let x = fill_column_x(120.0, 5, 100, 8.43);
        assert_eq!(x, 120.0 + (5 + 100) as f32 * 8.43);
    }

    #[test]
    fn fill_column_x_zero_column_lands_at_the_text_origin() {
        assert_eq!(fill_column_x(10.0, 3, 0, 8.0), 10.0 + 3.0 * 8.0);
    }

    // --- Fix 7: variant_file_name last-occurrence replacement --------

    #[test]
    fn variant_file_name_replaces_the_final_occurrence() {
        assert_eq!(
            variant_file_name("JetBrainsMono-Regular.ttf", "Bold").as_deref(),
            Some("JetBrainsMono-Bold.ttf")
        );
    }

    #[test]
    fn variant_file_name_ignores_an_earlier_regular_in_the_family_name() {
        // Fix 7 (cold review): a family name that itself contains
        // "Regular" before the variant marker used to have the wrong
        // occurrence replaced by `replacen(.., 1)`. The marker is
        // always the LAST "Regular" in the stem.
        assert_eq!(
            variant_file_name("RegularWidthMono-Regular.ttf", "Bold").as_deref(),
            Some("RegularWidthMono-Bold.ttf")
        );
    }

    #[test]
    fn variant_file_name_none_without_regular_marker() {
        assert_eq!(variant_file_name("Monaco.ttf", "Bold"), None);
    }

    // --- Task 3: indent guide columns --------------------------------

    #[test]
    fn indent_guide_normal_indented_rows() {
        // 8-space indent, step 4 -> guides at columns 0 and 4.
        let rows = ["        foo", "        bar"];
        let cols = indent_guide_columns(&rows, 4);
        assert_eq!(cols, vec![vec![0, 4], vec![0, 4]]);
    }

    #[test]
    fn indent_guide_blank_row_between_two_indented_rows_is_continuous() {
        // port list: two 8-space-indented rows around a blank one.
        let rows = ["        a,", "", "        b"];
        let cols = indent_guide_columns(&rows, 4);
        // The blank row uses min(8, 8) = 8, same guide columns as its
        // neighbors -- the guide runs through without a gap.
        assert_eq!(cols, vec![vec![0, 4], vec![0, 4], vec![0, 4]]);
    }

    #[test]
    fn indent_guide_blank_row_between_unequal_neighbors_uses_the_smaller() {
        // Reviewer-flagged gap: every previous blank-row test used equal
        // indents (8 and 8), so `.min`, `.max`, and `(a+b)/2` all agree
        // and a regression from `.min` to `.max` would pass unnoticed.
        // Neighbors here are 8 (above) and 4 (below) -- `.min` must win.
        let rows = ["        a", "", "    b"];
        let cols = indent_guide_columns(&rows, 4);
        assert_eq!(cols, vec![vec![0, 4], vec![0], vec![0]]);
    }

    #[test]
    fn indent_guide_blank_row_at_start_of_buffer_uses_the_side_that_exists() {
        let rows = ["", "        a"];
        let cols = indent_guide_columns(&rows, 4);
        // No row above; only the row below has a non-blank count (8), so
        // the blank row borrows it alone.
        assert_eq!(cols, vec![vec![0, 4], vec![0, 4]]);
    }

    #[test]
    fn indent_guide_step_zero_disables_guides() {
        let rows = ["        a", "    b"];
        let cols = indent_guide_columns(&rows, 0);
        assert_eq!(cols, vec![Vec::<usize>::new(), Vec::new()]);
    }

    #[test]
    fn indent_guide_row_indented_less_than_one_step_gets_only_the_zero_guide() {
        // 2 spaces, step 4: 0 < 2 (guide at 0), but 4 is not < 2, so no
        // second guide. Column 0 is always eligible once there is any
        // leading space at all -- only a wholly unindented row (count 0)
        // gets no guide.
        let rows = ["  a"];
        let cols = indent_guide_columns(&rows, 4);
        assert_eq!(cols, vec![vec![0]]);
    }

    #[test]
    fn indent_guide_unindented_row_gets_no_guide() {
        let rows = ["a"]; // 0 leading spaces: 0 < 0 is false
        let cols = indent_guide_columns(&rows, 4);
        assert_eq!(cols, vec![Vec::<usize>::new()]);
    }

    #[test]
    fn indent_guide_one_level_indent_gets_a_guide_at_column_zero() {
        // The defect this milestone fixes: a line indented exactly one
        // step (4 leading spaces, step 4) used to get no guide at all,
        // because 4 is not "< 4". VS Code's rule (guides start at 0)
        // gives it one guide, at column 0. Every port-list line in
        // demo/rtl/core/alu.sv is this shape.
        let rows = ["    a"];
        let cols = indent_guide_columns(&rows, 4);
        assert_eq!(cols, vec![vec![0]]);
    }

    // --- Task 5: scrollbar thumb geometry ----------------------------

    #[test]
    fn scrollbar_absent_when_buffer_fits() {
        assert_eq!(scrollbar_thumb(30, 40, 0, 400.0, 24.0), None);
        assert_eq!(scrollbar_thumb(40, 40, 0, 400.0, 24.0), None);
    }

    #[test]
    fn scrollbar_thumb_top_and_length_are_proportional() {
        // 1000 lines, 100 visible, scrolled to line 500: thumb should be
        // 10% of the track, starting at 50% of the track.
        let (top, len) = scrollbar_thumb(1000, 100, 500, 1000.0, 24.0).unwrap();
        assert!((len - 100.0).abs() < 1e-3, "len={len}");
        assert!((top - 500.0).abs() < 1e-3, "top={top}");
    }

    #[test]
    fn scrollbar_thumb_clamps_to_the_24px_minimum() {
        // 100000 lines, 50 visible: 0.05% of a 300px track is far under
        // 24px, so it must clamp up to the minimum.
        let (_, len) = scrollbar_thumb(100_000, 50, 0, 300.0, 24.0).unwrap();
        assert_eq!(len, 24.0);
    }

    #[test]
    fn scrollbar_thumb_top_clamps_when_scrolled_to_the_very_bottom() {
        // Fix 6c: `top` has its own defensive `.min(track_len - len)`
        // clamp, separate from `len`'s `.min(track_len)` above -- with no
        // test, a mutation to either clamp would go unnoticed. 10000
        // lines, 100 visible, scrolled all the way down (top_line =
        // 9900, the last page): the raw proportional top
        // (`track_len * 9900/10000`) would land past `track_len - len`
        // without the clamp.
        let total = 10_000;
        let visible = 100;
        let top_line = total - visible; // scrolled to the very last page
        let track_len = 500.0_f32;
        let min_len = 24.0_f32;
        let (top, len) = scrollbar_thumb(total, visible, top_line, track_len, min_len).unwrap();
        assert!(
            top <= track_len - len + 1e-3,
            "thumb top {top} must stay within the track (track_len={track_len}, len={len})"
        );
        // And it should be pinned exactly at the bottom, not merely
        // "somewhere under" the ceiling -- scrolling to the last page
        // means the thumb's bottom edge should coincide with the
        // track's bottom edge.
        assert!(
            (top + len - track_len).abs() < 1e-3,
            "thumb bottom ({}) should coincide with the track bottom ({track_len})",
            top + len
        );
    }

    // --- Task M115: overview-ruler diagnostic marks ------------------

    #[test]
    fn diagnostic_marks_empty_list_is_no_marks() {
        assert_eq!(
            scrollbar_diagnostic_marks(Vec::<(usize, u8)>::new(), 1000, 1000.0, 3.0),
            vec![]
        );
    }

    #[test]
    fn diagnostic_marks_placement_is_proportional_to_the_whole_buffer() {
        // 1000-line buffer, a diagnostic at line 250: the mark should
        // land at 25% of the track, regardless of what's currently
        // scrolled into view -- that's the whole point of an overview
        // ruler (showing what's off-screen), so this function doesn't
        // even take a visible-window parameter the way `scrollbar_thumb`
        // does. Asserts the whole tuple (top, len, severity) -- review
        // round 1 found this test only checked top and len, so a
        // corrupted severity would have passed unnoticed.
        let marks = scrollbar_diagnostic_marks([(250, 1)], 1000, 1000.0, 3.0);
        assert_eq!(marks.len(), 1);
        assert!((marks[0].0 - 250.0).abs() < 1e-3, "top={}", marks[0].0);
        assert_eq!(marks[0].1, 3.0);
        assert_eq!(marks[0].2, 1);
    }

    #[test]
    fn diagnostic_marks_merge_same_pixel_row_and_most_severe_wins() {
        // Two diagnostics on the SAME source line whose proportional
        // position lands on the same pixel row: an info (3) landing on
        // the same row as an error (1) must not hide the error -- the
        // merged mark keeps severity 1. Feeding them in arrival order
        // info-then-error is deliberate: drawing in arrival order would
        // let the later info paint over the earlier error, which is
        // exactly the defect this rule guards against.
        let marks = scrollbar_diagnostic_marks([(500, 3), (500, 1)], 100_000, 500.0, 3.0);
        assert_eq!(marks.len(), 1, "same line must merge into one mark");
        assert_eq!(marks[0].2, 1, "the more severe (lower byte) must win");
    }

    #[test]
    fn diagnostic_marks_merge_different_lines_that_round_onto_the_same_pixel_row() {
        // Review round 1, item 3: the merge test above uses the same
        // source line for both diagnostics, which is worth pinning but
        // doesn't cover "a long file where many DIFFERENT lines map to
        // one row" -- the actual overview-ruler scenario. It also can't
        // distinguish `top.round()` from truncation, since both
        // diagnostics land on an identical raw top either way.
        //
        // Here line 4996 of 10,000 has a raw top of
        // 1000.0 * 4996/10000 = 499.6 (rounds to 500, truncates to 499),
        // and line 5000 has a raw top of exactly 500.0 (rounds and
        // truncates to 500). Under rounding both land in bucket 500 and
        // merge into one mark keeping the more severe error (1). Under
        // truncation they'd land in DIFFERENT buckets (499 and 500) and
        // stay as two marks -- so this fails if `round()` regresses to
        // truncation.
        let marks = scrollbar_diagnostic_marks([(4996, 3), (5000, 1)], 10_000, 1000.0, 3.0);
        assert_eq!(
            marks.len(),
            1,
            "different lines rounding onto the same pixel row must merge: {marks:?}"
        );
        assert_eq!(marks[0].2, 1, "the more severe (lower byte) must win");
    }

    #[test]
    fn diagnostic_marks_have_a_minimum_height() {
        // A single diagnostic in a 10,000-line file: its raw
        // proportional slice of the track is far under a pixel, so the
        // mark must be clamped up to `min_len`. Also pins `top` (line
        // 5000 of 10,000 on a 100px track: raw top 50.0, well clear of
        // the top/bottom clamps) and `severity`, since a bug that only
        // corrupted one of the other two fields would be invisible to a
        // test that checks length alone.
        let marks = scrollbar_diagnostic_marks([(5000, 2)], 10_000, 100.0, 3.0);
        assert_eq!(marks.len(), 1);
        assert!((marks[0].0 - 50.0).abs() < 1e-3, "top={}", marks[0].0);
        assert_eq!(marks[0].1, 3.0);
        assert_eq!(marks[0].2, 2);
    }

    #[test]
    fn diagnostic_marks_length_never_exceeds_a_track_shorter_than_min_len() {
        // Review round 1, item 2: `max_top` used to floor at zero
        // without also clamping the returned length, so a track shorter
        // than `min_len` (unreachable from the real call site today,
        // since row heights are far above 3px, but an unenforced
        // invariant this function's own doc claims to enforce) could
        // return a mark whose bottom edge sat below the track. 10 total
        // lines, a 2px track, a 3px min_len: length must clamp down to
        // the 2px track, and top must be 0 so top+len == track_len
        // exactly, not below it.
        let marks = scrollbar_diagnostic_marks([(5, 1)], 10, 2.0, 3.0);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, 0.0, "top={}", marks[0].0);
        assert_eq!(
            marks[0].1, 2.0,
            "len must clamp to the track, not exceed it"
        );
    }

    #[test]
    fn diagnostic_marks_at_first_and_last_line() {
        // Different severities on the two ends so this also verifies
        // marks keep their own field, not just their position -- review
        // round 1 found this test never asserted severity at all.
        let marks = scrollbar_diagnostic_marks([(0, 2), (999, 1)], 1000, 1000.0, 3.0);
        assert_eq!(marks.len(), 2);
        let first = marks[0];
        assert!((first.0 - 0.0).abs() < 1e-3, "first line top={}", first.0);
        assert_eq!(first.2, 2);
        // The last line's raw proportional top (999/1000 * 1000 = 999.0)
        // plus the 3px min height would overflow the track (1000.0) by
        // 2px without the same top-clamp `scrollbar_thumb` applies to
        // its own thumb top.
        let last = marks[1];
        assert!(
            last.0 + last.1 <= 1000.0 + 1e-3,
            "last-line mark must stay within the track: top={} len={}",
            last.0,
            last.1
        );
        assert_eq!(last.2, 1);
    }

    #[test]
    fn diagnostic_marks_line_out_of_range_is_dropped() {
        // 100 lines, a diagnostic claiming line 500: nonsensical input,
        // far past the valid range, must be dropped rather than
        // producing an out-of-bounds rect.
        let marks = scrollbar_diagnostic_marks([(500, 1)], 100, 500.0, 3.0);
        assert_eq!(marks, vec![]);
    }

    #[test]
    fn diagnostic_marks_line_exactly_at_total_lines_is_dropped() {
        // Review round 1, item 3: the out-of-range test above uses 500
        // against 100 lines, nowhere near the boundary -- changing the
        // `>=` guard to `>` still passes it. Valid lines for a
        // 100-line buffer are 0..=99, so line 100 (== total_lines) is
        // the exact off-by-one this guards against.
        let marks = scrollbar_diagnostic_marks([(100, 1)], 100, 500.0, 3.0);
        assert_eq!(marks, vec![]);
    }

    #[test]
    fn diagnostic_marks_zero_height_track_is_no_marks() {
        assert_eq!(scrollbar_diagnostic_marks([(0, 1)], 1000, 0.0, 3.0), vec![]);
    }

    #[test]
    fn diagnostic_marks_zero_total_lines_is_no_marks() {
        assert_eq!(scrollbar_diagnostic_marks([(0, 1)], 0, 1000.0, 3.0), vec![]);
    }

    #[test]
    fn diagnostic_marks_single_line_buffer() {
        // A 1-line buffer: the only diagnostic possible is at line 0,
        // and it must land at the very top of the track, not divide by
        // zero or produce NaN.
        let marks = scrollbar_diagnostic_marks([(0, 1)], 1, 200.0, 3.0);
        assert_eq!(marks.len(), 1);
        assert!((marks[0].0 - 0.0).abs() < 1e-3);
        assert_eq!(marks[0].1, 3.0);
        assert_eq!(marks[0].2, 1);
    }

    // --- M111 cold review round 2: cursor_bg_color and translucency ---

    #[test]
    fn cursor_bg_color_forces_a_translucent_normal_bg_opaque_before_lerping() {
        // Reproduces the reviewer's numeric example: logical (58,60,80) at
        // 40% opacity is a gamma-premultiplied translucent `Color32`
        // (stored bytes ~(35,37,50,102), NOT (58,60,80,102)). At t=0 (an
        // ordinary point in the blink cycle, not an edge case) the result
        // must equal the un-premultiplied (58,60,80) fully opaque -- if
        // `cursor_bg_color` fed those raw stored bytes straight into
        // `lerp_color` (the bug this guards against), it would instead
        // return a visibly darker opaque (35,37,50).
        let translucent_bg = to_bg_color((58, 60, 80), 0.4);
        let target_bg = Color32::from_rgb(200, 200, 200);
        let result = cursor_bg_color(translucent_bg, target_bg, 0.0);
        // `to_opaque`'s un-premultiply round-trips through gamma space, so
        // allow the same sub-2-ULP drift as `to_bg_color`'s own test
        // rather than asserting bit-exact equality.
        assert!(result.r().abs_diff(58) <= 1, "r drifted: {}", result.r());
        assert!(result.g().abs_diff(60) <= 1, "g drifted: {}", result.g());
        assert!(result.b().abs_diff(80) <= 1, "b drifted: {}", result.b());
        assert_eq!(result.a(), 255);
    }

    // --- Fix 3: cursor fade must never cross zero contrast ------------

    #[test]
    fn cursor_fade_glyph_and_background_never_collide() {
        // Reproduction (see the task-3 report): the old code set
        // target_fg = cell_bg, target_bg = cell_fg, then lerped BOTH by
        // the same t, so fg(t) - bg(t) = (1 - 2t) * (fg0 - bg0) is
        // exactly zero at t = 0.5. This sweeps t across a full cycle for
        // both the swap-fallback shape (target_fg = cbg, target_bg =
        // cfg) and the measured real-theme colors from the review
        // (cell fg/bg approximating a mid-fade glyph of ~(111,114,121)
        // against a cell background of ~(66,104,143)), and asserts the
        // two rendered colors are never equal at any sampled t.
        let cases: [(Color32, Color32); 2] = [
            (
                Color32::from_rgb(212, 212, 212), // normal_fg
                Color32::from_rgb(30, 30, 30),    // normal_bg
            ),
            (
                Color32::from_rgb(220, 223, 228), // theme fg (light text)
                Color32::from_rgb(66, 104, 143),  // theme bg (measured cell bg)
            ),
        ];
        for (normal_fg, normal_bg) in cases {
            // Swap-fallback targets: the glyph fades toward the old
            // background, the background fades toward the old
            // foreground -- exactly the shape the bug report describes.
            let target_fg = normal_bg;
            let target_bg = normal_fg;
            let mut steps = 0;
            let mut t = 0.0_f32;
            while t <= 1.0 {
                let glyph = cursor_glyph_color(normal_fg, target_fg, t);
                let bg = cursor_bg_color(normal_bg, target_bg, t);
                assert_ne!(
                    glyph, bg,
                    "glyph and background collide at t={t} (fg0={normal_fg:?} bg0={normal_bg:?})"
                );
                t += 0.01;
                steps += 1;
            }
            assert!(steps > 50, "sanity: the sweep must actually run");
        }

        // Fix C (trailing review): both built-in themes define a `cursor`
        // face, and `themes.el` loads one at startup, so in the running
        // editor it's the `cursor`-face branch above (`target_fg`/
        // `target_bg` taken from the face, not the swap fallback) that
        // actually executes -- the swap-fallback shape swept above is
        // close to dead code in practice. These two cases are the real
        // shipped colour pairs, taken directly from `themes.el`'s `cursor`
        // face in each theme, checked against that theme's own default
        // fg/bg (`normal_fg`/`normal_bg`).
        let shipped_cursor_cases: [(Color32, Color32, Color32, Color32); 2] = [
            (
                Color32::from_rgb(0xc5, 0xca, 0xd3), // dark: default fg
                Color32::from_rgb(0x19, 0x1b, 0x20), // dark: default bg
                Color32::from_rgb(0x6c, 0xb6, 0xff), // dark: cursor bg
                Color32::from_rgb(0x19, 0x1b, 0x20), // dark: cursor fg
            ),
            (
                Color32::from_rgb(0x2c, 0x31, 0x3a), // light: default fg
                Color32::from_rgb(0xfb, 0xfb, 0xfd), // light: default bg
                Color32::from_rgb(0x1a, 0x73, 0xc7), // light: cursor bg
                Color32::from_rgb(0xfb, 0xfb, 0xfd), // light: cursor fg
            ),
        ];
        for (normal_fg, normal_bg, cursor_bg, cursor_fg) in shipped_cursor_cases {
            // `target_fg`/`target_bg` here mirror the `cursor_face` branch
            // in `App::update`: `target_fg = cf.fg.unwrap_or(cbg)`,
            // `target_bg = cf.bg.unwrap_or(cfg)` -- with the cell at its
            // normal (non-cursor) colors, `cbg == normal_bg` and
            // `cfg == normal_fg`.
            let target_fg = cursor_fg;
            let target_bg = cursor_bg;
            let mut steps = 0;
            let mut t = 0.0_f32;
            while t <= 1.0 {
                let glyph = cursor_glyph_color(normal_fg, target_fg, t);
                let bg = cursor_bg_color(normal_bg, target_bg, t);
                assert_ne!(
                    glyph, bg,
                    "glyph and background collide at t={t} (fg0={normal_fg:?} bg0={normal_bg:?})"
                );
                t += 0.01;
                steps += 1;
            }
            assert!(steps > 50, "sanity: the sweep must actually run");
        }
    }

    // --- Fix 6a: grid dimension floors ---------------------------------

    #[test]
    fn dimension_floor_uses_the_desired_minimum_when_there_is_room() {
        assert_eq!(
            dimension_floor(40, 20),
            40,
            "already above desired: unchanged"
        );
        assert_eq!(dimension_floor(20, 20), 20, "exactly at desired: unchanged");
    }

    #[test]
    fn dimension_floor_never_forces_a_count_larger_than_what_fits() {
        // The defect this fixes: a small padded area computing 7 cells
        // that fit must NOT be bumped up to the desired 20 -- that would
        // paint 13 columns' worth of cells outside the padded area.
        assert_eq!(dimension_floor(7, 20), 7);
        assert_eq!(dimension_floor(0, 5), 1, "floored at 1, never 0");
    }

    // --- Task 2: padding / line-spacing clamps -----------------------

    #[test]
    fn padding_clamp_in_range_passes_through() {
        assert_eq!(clamp_padding(10), 10);
    }

    #[test]
    fn padding_clamp_out_of_range_both_sides() {
        assert_eq!(clamp_padding(-5), 0);
        assert_eq!(clamp_padding(1000), 64);
    }

    #[test]
    fn line_spacing_clamp_in_range_passes_through() {
        assert_eq!(clamp_line_spacing(100), 100);
    }

    #[test]
    fn line_spacing_clamp_out_of_range_both_sides() {
        assert_eq!(clamp_line_spacing(10), 80);
        assert_eq!(clamp_line_spacing(9999), 200);
    }

    // --- Task 8: font-size clamp --------------------------------------

    #[test]
    fn font_size_clamp_low_end() {
        assert_eq!(clamp_font_size(1), 8);
    }

    #[test]
    fn font_size_clamp_high_end() {
        assert_eq!(clamp_font_size(999), 72);
    }

    #[test]
    fn font_size_clamp_in_range_passes_through() {
        assert_eq!(clamp_font_size(16), 16);
    }

    // --- Fix A: bounded blink cadence (blink_decision / gui-cursor-blinks) --

    #[test]
    fn cursor_blinks_clamp_in_range_passes_through() {
        assert_eq!(clamp_cursor_blinks(10), 10);
    }

    #[test]
    fn cursor_blinks_clamp_out_of_range_both_sides() {
        assert_eq!(clamp_cursor_blinks(-5), 0);
        assert_eq!(clamp_cursor_blinks(9999), 100);
    }

    // --- M111: gui-opacity clamp / background-role conversion --------

    #[test]
    fn opacity_clamp_in_range_passes_through() {
        assert_eq!(clamp_opacity(70), 70);
    }

    #[test]
    fn opacity_clamp_low_end_floors_at_20() {
        assert_eq!(clamp_opacity(0), 20);
        assert_eq!(clamp_opacity(-100), 20);
        assert_eq!(clamp_opacity(19), 20);
    }

    #[test]
    fn opacity_clamp_high_end_caps_at_100() {
        assert_eq!(clamp_opacity(101), 100);
        assert_eq!(clamp_opacity(9999), 100);
    }

    #[test]
    fn opacity_clamp_exactly_at_bounds() {
        assert_eq!(clamp_opacity(20), 20);
        assert_eq!(clamp_opacity(100), 100);
    }

    #[test]
    fn opacity_percent_to_frac_covers_the_divisor() {
        // Catches a `/ 100.0` -> `/ 1000.0` mutation, which no other test
        // in this file would have noticed (the divisor was previously
        // inlined at the single call site and unreachable by any test).
        assert_eq!(opacity_percent_to_frac(100), 1.0);
        assert_eq!(opacity_percent_to_frac(40), 0.4);
        // Below the floor: clamps to 20 first, THEN divides.
        assert_eq!(opacity_percent_to_frac(0), 0.2);
    }

    #[test]
    fn to_bg_color_full_opacity_gives_alpha_255() {
        let c = to_bg_color((10, 20, 30), 1.0);
        assert_eq!(c, Color32::from_rgba_unmultiplied(10, 20, 30, 255));
    }

    #[test]
    fn to_bg_color_floor_opacity_gives_expected_alpha() {
        // clamp_opacity's floor, 20%, as a fraction.
        let c = to_bg_color((10, 20, 30), 0.20);
        assert_eq!(c, Color32::from_rgba_unmultiplied(10, 20, 30, 51));
    }

    #[test]
    fn to_bg_color_mid_value() {
        let c = to_bg_color((10, 20, 30), 0.50);
        assert_eq!(c, Color32::from_rgba_unmultiplied(10, 20, 30, 128));
    }

    #[test]
    fn to_bg_color_leaves_rgb_channels_untouched() {
        // `Color32` stores gamma-correct premultiplied bytes, so a
        // straight `.r()`/`.g()`/`.b()` read after `from_rgba_unmultiplied`
        // does NOT reproduce the input (that's egui's own representation,
        // not a bug introduced here) -- `to_srgba_unmultiplied` undoes the
        // premultiply and is the right decoder to check "did `to_bg_color`
        // touch the RGB channels", modulo the sub-1-ULP rounding a
        // gamma-space round trip through premultiplication introduces.
        let [r, g, b, a] = to_bg_color((200, 100, 5), 0.5).to_srgba_unmultiplied();
        assert!(r.abs_diff(200) <= 1, "r drifted: {r}");
        assert!(g.abs_diff(100) <= 1, "g drifted: {g}");
        assert!(b.abs_diff(5) <= 1, "b drifted: {b}");
        assert_eq!(a, 128);
    }

    #[test]
    fn blink_decision_still_animating_early_on() {
        // 10 blinks at a 0.53s period is a 5.3s budget; 1s in, well
        // within it, the cursor must still be animating and its fade
        // fraction must be somewhere in the raised cosine's [0, 1] range
        // (not pinned to the "stopped" value of 1.0).
        let period = 0.53_f32;
        let (animating, fraction) =
            blink_decision(std::time::Duration::from_millis(1000), period, 10);
        assert!(animating, "1s into a 5.3s budget: must still be animating");
        assert!(
            (0.0..=1.0).contains(&fraction),
            "fraction {fraction} out of range"
        );
    }

    #[test]
    fn blink_decision_stops_and_goes_solid_after_the_limit() {
        // 10 blinks at 0.53s = 5.3s budget; well past it (10s), the
        // cursor must have stopped animating and be left fully solid
        // (fraction 1.0), not frozen mid-fade at some arbitrary phase.
        let period = 0.53_f32;
        let (animating, fraction) =
            blink_decision(std::time::Duration::from_millis(10_000), period, 10);
        assert!(!animating, "well past the blink budget: must have stopped");
        assert_eq!(fraction, 1.0, "stopped cursor must be left fully solid");
    }

    #[test]
    fn blink_decision_zero_limit_never_stops() {
        // GNU's own meaning for `blink-cursor-blinks` at 0: blink
        // forever. Even at a very large elapsed time, the cursor must
        // still be reported as animating.
        let period = 0.53_f32;
        let (animating, _) = blink_decision(std::time::Duration::from_secs(3600), period, 0);
        assert!(animating, "blink_limit 0 must mean 'never stop'");
    }

    #[test]
    fn blink_decision_input_resets_it() {
        // The defect this fixes: without a cap, this was always true; the
        // cap must actually engage past the budget (already covered
        // above) and input resetting `elapsed` back near zero must bring
        // the cursor back to "still animating" -- exactly what happens
        // when `App::update` re-evaluates `blink_decision` with a fresh
        // `elapsed_since_input` after every keystroke.
        let period = 0.53_f32;
        let (stopped, _) = blink_decision(std::time::Duration::from_millis(10_000), period, 10);
        assert!(!stopped, "sanity: this elapsed must be past the budget");
        let (animating_after_input, _) =
            blink_decision(std::time::Duration::from_millis(0), period, 10);
        assert!(
            animating_after_input,
            "fresh input (elapsed reset to 0) must resume animating"
        );
    }

    // --- Mouse support (M-mouse task 3): pixel_to_buffer_pos /
    // window_at_pixel, the pure hit-testing functions. Built by hand
    // rather than through a live editor -- exactly the "pure mapping
    // functions" the task called for coverage by unit test rather than
    // by screenshot (the screenshot script cannot send mouse events at
    // all). ---------------------------------------------------------

    fn test_win(
        win_id: usize,
        row: usize,
        col: usize,
        rows: usize,
        cols: usize,
        gutter_cols: usize,
    ) -> core::redisplay::WindowLayout {
        core::redisplay::WindowLayout {
            win_id,
            row,
            col,
            rows,
            cols,
            gutter_cols,
            // Same convention `render_window` uses: the mode line is the
            // window's own last row.
            mode_line_row: row + rows - 1,
        }
    }

    fn test_run(
        row: usize,
        col: usize,
        text: &str,
        byte_start: usize,
    ) -> core::redisplay::PaintRun {
        core::redisplay::PaintRun {
            row,
            col,
            cols: text.chars().count(),
            text: text.to_string(),
            style: core::redisplay::Style::default(),
            src: Some(byte_start..byte_start + text.len()),
            atomic: false,
        }
    }

    fn test_grid(
        windows: Vec<core::redisplay::WindowLayout>,
        runs: Vec<core::redisplay::PaintRun>,
    ) -> core::redisplay::Grid {
        core::redisplay::Grid {
            cols: 0,
            // F4 (M87 stage 3 fix round): `row_at_y` now bounds its walk
            // by `grid.rows` (previously unbounded, relying only on
            // `row_h > 0.0`) -- this hand-built test grid's rows go
            // unused by every OTHER test here (none asserts on
            // `grid.rows` itself), so a generous constant, comfortably
            // past every pixel position any test below probes, keeps
            // that new bound from silently truncating them.
            rows: 100,
            lines: Vec::new(),
            cursor: (0, 0),
            windows,
            runs,
            row_scale: Vec::new(),
            row_kind: Vec::new(),
        }
    }

    /// One window: rows 0..6 are text, row 6 is the mode line;
    /// gutter_cols=3 (columns 0..3), text columns 3..50, with "hello"
    /// (bytes 0..5) painted on row 0 starting at column 3.
    fn one_window_grid() -> core::redisplay::Grid {
        test_grid(
            vec![test_win(1, 0, 0, 7, 50, 3)],
            vec![test_run(0, 3, "hello", 0)],
        )
    }

    const CHAR_W: f32 = 10.0;
    const ROW_H: f32 = 20.0;
    const ORIGIN: Pos2 = Pos2::new(0.0, 0.0);

    #[test]
    fn pixel_to_buffer_pos_gutter_column_returns_none() {
        // Column 1 is inside the 0..3 gutter -- must do nothing rather
        // than mis-position point (the guard rail this whole function
        // exists to provide; see `buffer_pos_at_gutter_column_falls_
        // through_to_line_end_not_none` in `gui_features_tests.rs` for
        // what `Grid::buffer_pos_at` alone would do without this guard).
        let grid = one_window_grid();
        let pixel = Pos2::new(1.5 * CHAR_W, 0.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            None
        );
    }

    #[test]
    fn pixel_to_buffer_pos_mode_line_row_returns_none() {
        // Row 6 is this window's mode line (mode_line_row = 0 + 7 - 1).
        let grid = one_window_grid();
        let pixel = Pos2::new(10.0 * CHAR_W, 6.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            None
        );
    }

    #[test]
    fn pixel_to_buffer_pos_echo_area_row_returns_none() {
        // Row 10 is well past the window entirely (its own rows only
        // span 0..7) -- the shared echo row in a real frame.
        let grid = one_window_grid();
        let pixel = Pos2::new(10.0 * CHAR_W, 10.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            None
        );
    }

    #[test]
    fn pixel_to_buffer_pos_plain_text_click_resolves_inside_the_window() {
        let grid = one_window_grid();
        // Column 5 (inside "hello", 2 columns into the text area at
        // column 3) on row 0.
        let pixel = Pos2::new(5.5 * CHAR_W, 0.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            Some((1, 2))
        );
    }

    #[test]
    fn pixel_to_buffer_pos_resolves_the_right_window_in_a_split() {
        // Two windows side by side: 1 spans columns 0..25 (no gutter),
        // 2 spans columns 25..50 (no gutter), both rows 0..6 text + row
        // 6 mode line.
        let grid = test_grid(
            vec![test_win(1, 0, 0, 7, 25, 0), test_win(2, 0, 25, 7, 25, 0)],
            vec![test_run(0, 0, "AAAA", 100), test_run(0, 25, "BBBB", 200)],
        );
        let left = Pos2::new(2.5 * CHAR_W, 0.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, left),
            Some((1, 102))
        );
        let right = Pos2::new(27.5 * CHAR_W, 0.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, right),
            Some((2, 202))
        );
    }

    #[test]
    fn pixel_to_buffer_pos_outside_every_window_returns_none() {
        let grid = test_grid(
            vec![test_win(1, 0, 0, 7, 25, 0), test_win(2, 0, 25, 7, 25, 0)],
            vec![test_run(0, 0, "AAAA", 100), test_run(0, 25, "BBBB", 200)],
        );
        // Column 60 is past both windows (0..25 and 25..50).
        let pixel = Pos2::new(60.0 * CHAR_W, 0.5 * ROW_H);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            None
        );
    }

    #[test]
    fn pixel_to_buffer_pos_above_and_left_of_origin_does_not_panic() {
        let grid = one_window_grid();
        let origin = Pos2::new(50.0, 50.0);
        assert_eq!(
            pixel_to_buffer_pos(&grid, origin, CHAR_W, ROW_H, Pos2::new(10.0, 10.0)),
            None,
            "pixel above/left of origin (negative-ish cell coordinates)"
        );
        assert_eq!(
            pixel_to_buffer_pos(&grid, origin, CHAR_W, ROW_H, Pos2::new(-100.0, -100.0)),
            None,
            "far above/left must also not panic or wrap"
        );
    }

    #[test]
    fn pixel_to_buffer_pos_non_positive_char_metrics_do_not_panic() {
        let grid = one_window_grid();
        for (cw, rh) in [
            (0.0_f32, ROW_H),
            (-5.0, ROW_H),
            (CHAR_W, 0.0),
            (CHAR_W, -5.0),
        ] {
            assert_eq!(
                pixel_to_buffer_pos(&grid, ORIGIN, cw, rh, Pos2::new(50.0, 50.0)),
                None,
                "char_w={cw} row_h={rh} must not panic and must report no hit"
            );
        }
    }

    #[test]
    fn window_at_pixel_gutter_and_mode_line_and_echo_return_none() {
        let grid = one_window_grid();
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(1.5 * CHAR_W, 0.5 * ROW_H)
            ),
            None,
            "gutter column"
        );
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(10.0 * CHAR_W, 6.5 * ROW_H)
            ),
            None,
            "mode line row"
        );
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(10.0 * CHAR_W, 10.5 * ROW_H)
            ),
            None,
            "echo-area row, well past the window"
        );
    }

    #[test]
    fn window_at_pixel_resolves_the_right_window_in_a_split() {
        let grid = test_grid(
            vec![test_win(1, 0, 0, 7, 25, 0), test_win(2, 0, 25, 7, 25, 0)],
            Vec::new(),
        );
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(2.5 * CHAR_W, 0.5 * ROW_H)
            ),
            Some(1)
        );
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(27.5 * CHAR_W, 0.5 * ROW_H)
            ),
            Some(2)
        );
        assert_eq!(
            window_at_pixel(
                &grid,
                ORIGIN,
                CHAR_W,
                ROW_H,
                Pos2::new(60.0 * CHAR_W, 0.5 * ROW_H)
            ),
            None
        );
    }

    #[test]
    fn window_at_pixel_non_positive_char_metrics_do_not_panic() {
        let grid = one_window_grid();
        for (cw, rh) in [
            (0.0_f32, ROW_H),
            (-5.0, ROW_H),
            (CHAR_W, 0.0),
            (CHAR_W, -5.0),
        ] {
            assert_eq!(
                window_at_pixel(&grid, ORIGIN, cw, rh, Pos2::new(50.0, 50.0)),
                None,
                "char_w={cw} row_h={rh} must not panic and must report no hit"
            );
        }
    }

    // --- F3 (M87 stage 3 fix round): row geometry with a non-default
    // `row_scale` -- every test above used `one_window_grid`, whose
    // `row_scale` is empty (`test_grid`'s default), which only exercises
    // `row_height`/`row_top`/`row_at_y`'s `unwrap_or(100)` fallback path,
    // never the actual percentage-scaling arithmetic this milestone
    // exists to add. Row 2 here is a 75%-scale block row (`RowKind::
    // Block`); text runs sit on row 0 ("hello") and row 3 ("world"), the
    // row directly below it. -------------------------------------------

    fn grid_with_block_row() -> core::redisplay::Grid {
        let mut row_scale = vec![100u8; 8];
        let mut row_kind = vec![core::redisplay::RowKind::Text; 8];
        row_scale[2] = 75;
        row_kind[2] = core::redisplay::RowKind::Block;
        core::redisplay::Grid {
            cols: 50,
            rows: 8,
            lines: Vec::new(),
            cursor: (0, 0),
            windows: vec![test_win(1, 0, 0, 8, 50, 3)],
            runs: vec![test_run(0, 3, "hello", 0), test_run(3, 3, "world", 100)],
            row_scale,
            row_kind,
        }
    }

    #[test]
    fn row_geometry_round_trips_through_a_75_percent_row() {
        let grid = grid_with_block_row();
        // First row.
        assert_eq!(row_at_y(&grid, ROW_H, row_top(&grid, ROW_H, 0) + 1.0), 0);

        // The 75%-scale row itself: rows 0 and 1 are still full height,
        // so its top edge is 2 * ROW_H, and its own height is 75% of
        // ROW_H, not ROW_H.
        let block_top = row_top(&grid, ROW_H, 2);
        assert_eq!(block_top, 2.0 * ROW_H);
        let block_h = row_height(&grid, ROW_H, 2);
        assert_eq!(block_h, 0.75 * ROW_H);
        assert_eq!(row_at_y(&grid, ROW_H, block_top + 1.0), 2);
        assert_eq!(row_at_y(&grid, ROW_H, block_top + block_h - 0.1), 2);
        // Exactly at its bottom edge: that pixel belongs to the NEXT
        // row's top edge, not "still row 2" (row_at_y's own `rel_y < y +
        // h` test is strict).
        assert_eq!(row_at_y(&grid, ROW_H, block_top + block_h), 3);

        // A row after the block row.
        let after_top = row_top(&grid, ROW_H, 3);
        assert_eq!(after_top, block_top + block_h);
        assert_eq!(row_at_y(&grid, ROW_H, after_top + 1.0), 3);
    }

    #[test]
    fn pixel_to_buffer_pos_lands_on_the_right_line_below_a_block_row() {
        let grid = grid_with_block_row();
        // Hard-coded pixel geometry, NOT derived via `row_top`/
        // `row_height` again -- a test that re-derives its own expected
        // y from the same functions under test can't actually catch a
        // regression in them (self-consistent either way). With
        // ROW_H == 20.0 and row 2 at 75%: row 0 spans [0, 20), row 1
        // [20, 40), row 2 (block, 15 tall) [40, 55), row 3 [55, 75).
        // y == 57.0 is inside row 3 under the REAL scaled geometry.
        // Two things this specifically rules out: (a) the pre-fix
        // `row as f32 * row_h` formula, which would floor(57 / 20) == 2
        // instead; (b) `row_height` ignoring `row_scale` entirely
        // (treating row 2 as a full 20px row), which would put row 3's
        // top at 60 instead of 55 and also resolve y == 57.0 to row 2.
        // Row 2 has no buffer-text run at all, so either wrong answer
        // comes back `None`, not just a differently-wrong `Some`.
        let pixel = Pos2::new(5.5 * CHAR_W, 57.0);
        assert_eq!(
            pixel_to_buffer_pos(&grid, ORIGIN, CHAR_W, ROW_H, pixel),
            Some((1, 102)),
            "click on row 3 (2 chars into \"world\") must resolve using the real (scaled) pixel geometry"
        );
    }

    // --- Mouse support: evil-visual-state entry on drag (`arm_mouse_
    // selection`) -- uses a REAL `Interp`/`Editor` (same construction
    // `core`'s own evil_tests.rs uses), not a hand-built `Grid`, since
    // this exercises `evil-mode`/`evil--state`, which only exist once
    // `evil.el` is actually loaded and running. -----------------------

    fn evil_test_editor(text: &str) -> (Interp, Rc<RefCell<Editor>>) {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        ed.borrow_mut().frame = (50, 8);
        interp
            .eval_source(&format!("(insert {:?})", text))
            .unwrap_or_else(|e| panic!("insert failed: {}", interp.describe_flow(&e)));
        interp
            .eval_source("(goto-char (point-min))")
            .unwrap_or_else(|e| panic!("goto-char failed: {}", interp.describe_flow(&e)));
        // Mirrors `core`'s `evil_tests.rs`'s `setup_evil`: `init_editor`
        // loads evil.el but does not auto-enable it (that's
        // `src/main.rs`'s `start_session`, gated on `evil-auto-enable`,
        // which a headless test never runs) -- so this call is what
        // actually turns evil-mode on and puts a fresh buffer into
        // `'normal` state.
        interp
            .eval_source("(evil-mode 1)")
            .unwrap_or_else(|e| panic!("evil-mode 1 failed: {}", interp.describe_flow(&e)));
        (interp, ed)
    }

    /// A real `Interp` + `Editor` with the shipped lisp loaded: just
    /// `new_interp` + `init_editor`, which is the first two lines of
    /// `evil_test_editor` and nothing else. It deliberately skips that
    /// helper's other three steps as well as `(evil-mode 1)` -- the
    /// frame sizing, the `(insert ...)` and the `goto-char` -- because
    /// the per-frame wire tests below (M117) read variables and never
    /// look at buffer text or window geometry.
    /// Using a real `Interp` here, rather than a hand-built stand-in, is
    /// the point: it means these tests also pin the shipped `defvar`
    /// defaults in `crates/core/lisp/gui.el` (e.g. `gui-opacity`'s 100,
    /// `gui-ligatures`'s `t`), not just whatever fallback the Rust call
    /// site happens to pass.
    fn wired_test_editor() -> (Interp, Rc<RefCell<Editor>>) {
        let mut interp = elisp::new_interp();
        let ed = core::init_editor(&mut interp);
        (interp, ed)
    }

    // --- Fix 4: `drag_should_arm` -----------------------------------

    #[test]
    fn drag_should_arm_first_move_away_from_start() {
        assert!(drag_should_arm(false, 5, 3));
    }

    #[test]
    fn drag_should_arm_not_rearmed_once_already_dragged() {
        assert!(!drag_should_arm(true, 5, 3));
    }

    #[test]
    fn drag_should_arm_no_movement_yet_does_not_arm() {
        assert!(!drag_should_arm(false, 3, 3));
    }

    /// The bug this fix is for: before it, the SAME condition this
    /// function expresses also gated whether `place_point` ran at all
    /// (see the `PointerMoved` handler's doc comment in `App::update`).
    /// A drag that moves away and then returns to exactly its start
    /// position must not re-arm (already dragged) -- but, unlike before
    /// the fix, that no longer suppresses `place_point`, which the call
    /// site now runs unconditionally on every same-window `moved` event.
    /// This test only pins the arm half; there is no unit-level way to
    /// pin the `place_point` half without a full `egui`/`eframe` event
    /// harness, which is out of scope here -- it's covered by reading
    /// the call site, where `place_point` sits outside the `if arm`
    /// block entirely.
    #[test]
    fn drag_should_arm_returning_to_start_after_already_dragged_does_not_rearm() {
        assert!(!drag_should_arm(true, 3, 3));
    }

    #[test]
    fn arm_mouse_selection_enters_evil_visual_state_from_normal() {
        let (mut interp, ed) = evil_test_editor("hello world");
        assert_eq!(
            sym_var(&interp, "evil--state"),
            Some("normal"),
            "evil-mode 1 must leave a fresh buffer in normal state"
        );
        let win_id = ed.borrow().selected_window;
        arm_mouse_selection(&mut interp, &ed, win_id, 0);
        assert_eq!(
            sym_var(&interp, "evil--state"),
            Some("visual"),
            "a drag starting in normal state must enter evil's visual              state, the same as pressing 'v'"
        );
        let buf = ed.borrow().windows.get(&win_id).unwrap().buffer.clone();
        let b = buf.borrow();
        assert_eq!(
            b.mark,
            Some(0),
            "mark must land at the drag's press position"
        );
        assert!(b.mark_active);
    }

    #[test]
    fn arm_mouse_selection_does_not_change_evil_insert_state() {
        let (mut interp, ed) = evil_test_editor("hello world");
        interp
            .eval_source("(evil-insert)")
            .unwrap_or_else(|e| panic!("evil-insert failed: {}", interp.describe_flow(&e)));
        assert_eq!(
            sym_var(&interp, "evil--state"),
            Some("insert"),
            "setup must actually reach insert state before the drag"
        );
        let win_id = ed.borrow().selected_window;
        arm_mouse_selection(&mut interp, &ed, win_id, 0);
        assert_eq!(
            sym_var(&interp, "evil--state"),
            Some("insert"),
            "a drag while already in insert state must not change evil state"
        );
        // The fallback plain mark-set still runs -- this is the same
        // pre-existing behavior every non-'normal' state already had
        // (a drag always armed `mark`/`mark_active` before this
        // milestone's evil integration); only the STATE transition is
        // gated on being in 'normal' state.
        let buf = ed.borrow().windows.get(&win_id).unwrap().buffer.clone();
        let b = buf.borrow();
        assert_eq!(b.mark, Some(0));
        assert!(b.mark_active);
    }

    /// Fix 5 (mouse-support milestone review): `arm_mouse_selection`'s
    /// `'normal`-state branch used to be `let _ = interp.eval_source(
    /// "(evil-visual-char)")`, discarding any error. Reproduced here by
    /// redefining `evil-visual-char` (in elisp, from the test) to signal
    /// instead of doing its real job -- before the fix this left `mark`/
    /// `mark_active` at their initial (unset) values with no fallback and
    /// no diagnostic; after the fix, the plain hand-set `arm_mark` path
    /// still runs.
    #[test]
    fn arm_mouse_selection_falls_back_to_plain_mark_when_evil_visual_char_errors() {
        let (mut interp, ed) = evil_test_editor("hello world");
        interp
            .eval_source("(defun evil-visual-char () (error \"boom\"))")
            .unwrap_or_else(|e| panic!("redefine failed: {}", interp.describe_flow(&e)));
        assert_eq!(
            sym_var(&interp, "evil--state"),
            Some("normal"),
            "setup must still be in normal state so the 'normal' branch runs"
        );
        let win_id = ed.borrow().selected_window;
        arm_mouse_selection(&mut interp, &ed, win_id, 0);
        // The error must not have propagated out of `arm_mouse_selection`
        // (no panic reaching this line is itself part of what's being
        // checked), and the fallback plain mark must be set.
        let buf = ed.borrow().windows.get(&win_id).unwrap().buffer.clone();
        let b = buf.borrow();
        assert_eq!(
            b.mark,
            Some(0),
            "a failed evil-visual-char must still leave a usable mark"
        );
        assert!(b.mark_active);
        // evil--state is left wherever the failed call left it -- since
        // the redefined function errors before doing anything, that's
        // still 'normal'; this pins that the fallback doesn't try to
        // force a state transition of its own.
        assert_eq!(sym_var(&interp, "evil--state"), Some("normal"));
    }

    // --- M101: convert_key / key_base_char for `=' and `-' -----------
    //
    // `convert_key' had no unit tests at all before this milestone.
    // These pin the `K::Equals'/`K::Plus' whitelist entries added for
    // expand-region's `C-=' binding against the SAME encoding
    // `core::keymap::parse_kbd' produces for the textual key
    // descriptions `expand-region.el' binds -- computing the expected
    // value with `parse_kbd' rather than hand-deriving the control-bit
    // arithmetic is what `ctrl_7_is_the_shared_undo_byte'
    // (frontend-tui/src/lib.rs) does for the same reason: it's the
    // encoding a real keymap lookup will actually be compared against.
    fn expect_char_key(desc: &str) -> Key {
        match core::keymap::parse_kbd(desc).unwrap().as_slice() {
            [k] => k.clone(),
            other => panic!("{desc:?} parsed to {other:?}, expected exactly one key"),
        }
    }

    #[test]
    fn ctrl_equals_matches_parse_kbd_c_dash_equals() {
        let got = convert_key(egui::Key::Equals, egui::Modifiers::CTRL);
        assert_eq!(got, Some(expect_char_key("C-=")));
    }

    #[test]
    fn ctrl_minus_matches_parse_kbd_c_dash_dash() {
        let got = convert_key(egui::Key::Minus, egui::Modifiers::CTRL);
        assert_eq!(got, Some(expect_char_key("C--")));
    }

    #[test]
    fn ctrl_shift_equals_matches_parse_kbd_c_dash_plus() {
        let got = convert_key(
            egui::Key::Equals,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        );
        assert_eq!(got, Some(expect_char_key("C-+")));
    }

    #[test]
    fn ctrl_numpad_plus_matches_parse_kbd_c_dash_plus() {
        // `egui::Key::Plus' is the numpad `+' (`KeyCode::NumpadAdd'), a
        // physically different key from `Key::Equals' (the top-row `='
        // that needs Shift for `+') -- both map to the SAME `key_base_char'
        // output `+' though, so `Ctrl' held with either must decode to
        // the same `C-+' this file's own `K::Plus' arm was left
        // completely unpinned by the three tests above it.
        let got = convert_key(egui::Key::Plus, egui::Modifiers::CTRL);
        assert_eq!(got, Some(expect_char_key("C-+")));
    }

    // --- M117: per-frame wires from elisp variables to the screen ---
    // (see the module doc for why these were previously unreachable
    // from `cargo test`).

    #[test]
    fn frame_opacity_frac_reads_the_gui_opacity_variable() {
        let (mut interp, _ed) = wired_test_editor();
        interp
            .eval_source("(setq gui-opacity 40)")
            .unwrap_or_else(|e| panic!("setq failed: {}", interp.describe_flow(&e)));
        let frac = frame_opacity_frac(&interp);
        assert!((frac - 0.40).abs() < 1e-6, "got {frac}");
    }

    #[test]
    fn frame_opacity_frac_defaults_to_fully_opaque() {
        // No `setq` at all -- this pins `gui.el`'s shipped default of
        // 100, not just the Rust fallback baked into `int_var`'s third
        // argument.
        let (interp, _ed) = wired_test_editor();
        let frac = frame_opacity_frac(&interp);
        assert!((frac - 1.0).abs() < 1e-6, "got {frac}");
    }

    #[test]
    fn frame_opacity_frac_applies_the_floor_through_the_variable() {
        let (mut interp, _ed) = wired_test_editor();
        interp
            .eval_source("(setq gui-opacity 5)")
            .unwrap_or_else(|e| panic!("setq failed: {}", interp.describe_flow(&e)));
        let frac = frame_opacity_frac(&interp);
        assert!((frac - 0.20).abs() < 1e-6, "got {frac}");
    }

    #[test]
    fn frame_opacity_frac_applies_the_cap_through_the_variable() {
        let (mut interp, _ed) = wired_test_editor();
        interp
            .eval_source("(setq gui-opacity 400)")
            .unwrap_or_else(|e| panic!("setq failed: {}", interp.describe_flow(&e)));
        let frac = frame_opacity_frac(&interp);
        assert!((frac - 1.0).abs() < 1e-6, "got {frac}");
    }

    #[test]
    fn frame_bg_carries_the_opacity_onto_the_theme_background() {
        // `Color32` stores gamma-correct premultiplied bytes, so a
        // straight `.r()`/`.g()`/`.b()` read does not reproduce the
        // input -- `to_srgba_unmultiplied` undoes the premultiply, same
        // decoder `to_bg_color_leaves_rgb_channels_untouched` uses.
        let base_bg: core::redisplay::Color = (10, 20, 30);
        let bg = frame_bg(Some(base_bg), 0.4);
        let [r, g, b, a] = bg.to_srgba_unmultiplied();
        assert_eq!(a, 102, "0.4 * 255 rounded is 102");
        assert!(r.abs_diff(10) <= 1, "r drifted: {r}");
        assert!(g.abs_diff(20) <= 1, "g drifted: {g}");
        assert!(b.abs_diff(30) <= 1, "b drifted: {b}");
    }

    #[test]
    fn frame_bg_fallback_also_carries_the_opacity() {
        // This is the branch that would silently go opaque if the
        // `unwrap_or_else` in `frame_bg` lost its fraction.
        let bg = frame_bg(None, 0.4);
        assert_eq!(bg.a(), 102, "0.4 * 255 rounded is 102");
    }

    #[test]
    fn with_alpha_clamps_a_fraction_outside_the_unit_range() {
        // Today every call site passes an already-clamped fraction, so
        // nothing else exercises this clamp.
        let c = Color32::from_rgb(1, 2, 3);
        assert_eq!(with_alpha(c, 1.5).a(), 255);
        assert_eq!(with_alpha(c, -1.0).a(), 0);
    }

    #[test]
    fn ligatures_enabled_defaults_to_on_from_the_shipped_gui_el() {
        // `var_truthy` returns `false` for an unbound symbol, so this
        // test also fails if the `defvar` for `gui-ligatures` disappears
        // from `gui.el` (not just if the default value changes).
        let (interp, _ed) = wired_test_editor();
        assert!(ligatures_enabled(&interp));
    }

    #[test]
    fn ligatures_enabled_follows_the_variable_when_turned_off() {
        let (mut interp, _ed) = wired_test_editor();
        interp
            .eval_source("(setq gui-ligatures nil)")
            .unwrap_or_else(|e| panic!("setq failed: {}", interp.describe_flow(&e)));
        assert!(!ligatures_enabled(&interp));
    }

    #[test]
    fn window_diagnostic_lines_returns_the_buffers_own_diagnostics() {
        let (_interp, ed) = wired_test_editor();
        let buf = Rc::new(RefCell::new(Buffer::new("a", "")));
        ed.borrow_mut().diagnostics.insert(
            Rc::as_ptr(&buf) as usize,
            vec![
                (3, 1, "error on line 3".to_string()),
                (7, 2, "warning on line 7".to_string()),
            ],
        );
        let ed_ref = ed.borrow();
        let got = window_diagnostic_lines(&ed_ref, &buf);
        assert_eq!(got, vec![(3, 1), (7, 2)], "must preserve stored order");
    }

    #[test]
    fn window_diagnostic_lines_does_not_return_another_buffers_diagnostics() {
        // The one that would catch a wrong `Rc::as_ptr` key -- e.g.
        // looking up the other window's buffer in a split.
        let (_interp, ed) = wired_test_editor();
        let buf_a = Rc::new(RefCell::new(Buffer::new("a", "")));
        let buf_b = Rc::new(RefCell::new(Buffer::new("b", "")));
        let buf_c = Rc::new(RefCell::new(Buffer::new("c", ""))); // no entry at all
        ed.borrow_mut().diagnostics.insert(
            Rc::as_ptr(&buf_a) as usize,
            vec![(1, 1, "a's diagnostic".to_string())],
        );
        ed.borrow_mut().diagnostics.insert(
            Rc::as_ptr(&buf_b) as usize,
            vec![(2, 2, "b's diagnostic".to_string())],
        );
        let ed_ref = ed.borrow();
        assert_eq!(window_diagnostic_lines(&ed_ref, &buf_a), vec![(1, 1)]);
        assert_eq!(window_diagnostic_lines(&ed_ref, &buf_b), vec![(2, 2)]);
        assert_eq!(window_diagnostic_lines(&ed_ref, &buf_c), Vec::new());
    }
}
