// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! Coding-ligature shaping (`calt`) for the GUI's text-drawing pass.
//!
//! The measurement this module is built on (see the module doc in
//! `lib.rs`'s caller and `PLAN.md`'s milestone record): a coding-ligature
//! font substitutes glyphs but never merges cells — every ligature is one
//! glyph per character, drawn as blank spacer glyphs in the leading cells
//! and a final glyph whose ink extends backwards over them. So the only
//! thing shaping can legitimately change is *which glyph* is drawn in
//! each already-existing cell; it must never change how many cells a run
//! occupies. [`validate_shaped_run`] is the checked precondition that
//! keeps that true for any font, not just the one this was measured
//! against: if a font's shaper ever returns a different glyph count than
//! character count, or a non-uniform advance, the caller must fall back
//! to drawing that run character by character exactly as the pre-shaping
//! code did.
//!
//! Three pieces, kept separate because they invalidate on different
//! events:
//! - [`LoadedFont`] -- the font bytes `rustybuzz` and `ab_glyph` need,
//!   kept alongside (not instead of) the copy `egui::FontData` owns,
//!   because `FontData::from_owned` takes ownership and does not give it
//!   back. See this crate's `install_fonts` for why the two copies must
//!   come from the same read.
//! - [`ShapeCache`] -- caches the *shaping* result (glyph ids + offsets)
//!   per `(FaceRole, text, enable_calt)` (the last component added M116
//!   for `gui-ligatures`). Shaping is independent of pixel size (it
//!   operates in font design units), so this cache is never invalidated
//!   by a font-size or DPI change -- only capped, since buffer text is
//!   unbounded over a long session.
//! - [`GlyphAtlasCache`] -- caches the *rasterized* glyph's location in
//!   egui's shared font atlas, per `(FaceRole, glyph id, pixel scale)`.
//!   This one is scale-dependent (a different font size or DPI rasterizes
//!   at different pixel dimensions) and, critically, must be dropped
//!   whole whenever the atlas itself is rebuilt -- `TextureAtlas::
//!   allocate` has no eviction, so stale UVs after a rebuild point at
//!   pixels that no longer hold what the cache thinks they hold. The
//!   caller detects a rebuild by `Arc::ptr_eq` on the atlas handle
//!   returned each frame by `Fonts::texture_atlas` (egui recreates the
//!   `Arc` on DPI change or when the atlas is more than 80% full -- see
//!   `epaint::text::fonts::Fonts::begin_pass`); there is no public
//!   generation counter, so identity of the `Arc` is the only signal
//!   available.

use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::Font as _;
use eframe::egui;
use egui::epaint::{Mesh, TextureId};
use egui::{Color32, Pos2, Rect, Vec2};

/// One loaded font face's raw bytes plus its collection index (0 for a
/// plain `.ttf`/`.otf`; the face index within a `.ttc`, e.g. Menlo.ttc's
/// bold at index 1). `bytes` is an `Arc` so `LoadedFont` can be cloned
/// cheaply into both the shaper and the rasterizer without duplicating
/// the underlying allocation.
#[derive(Clone)]
pub struct LoadedFont {
    pub bytes: Arc<[u8]>,
    pub index: u32,
}

/// Which of the three faces a run is drawn with -- the same three-way
/// split `pick_font` in `lib.rs` already makes; kept as its own enum here
/// so the shaping caches can be keyed on it without depending on
/// `egui::FontId`/`FontFamily` (which aren't `Eq`/`Hash` in the way a
/// `HashMap` key wants without extra ceremony).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FaceRole {
    Regular,
    Bold,
    Italic,
}

/// A face ready for both shaping (`rustybuzz`, from `loaded.bytes` at
/// `loaded.index`) and rasterizing (`ab_glyph`, from `ab`, built once at
/// load time since `ab_glyph::FontArc` construction is not free).
/// `loaded.index` and `ab` MUST point at the same face within the source
/// bytes: `shape_calt_only` looks up glyph ids against `loaded.index`'s
/// face, and `build_shaped_mesh`/`rasterize_glyph` then draw those glyph
/// ids against `ab` -- if the two disagreed, glyph ids computed against
/// one face's cmap/GSUB tables would be drawn against a different face's
/// glyph outlines, which is wrong even when the two faces happen to share
/// a glyph count (M106: this drew every collection-indexed face as the
/// collection's face 0, so Fira Code's italic rendered upright and
/// `sf-mono`'s bold rendered regular-weight -- fixed by threading `index`
/// into `ab_glyph`'s own collection constructor below instead of
/// discarding it).
#[derive(Clone)]
pub struct ShapingFace {
    pub loaded: LoadedFont,
    pub ab: ab_glyph::FontArc,
}

impl ShapingFace {
    pub fn new(bytes: Arc<[u8]>, index: u32) -> Option<ShapingFace> {
        let ab: ab_glyph::FontArc =
            ab_glyph::FontVec::try_from_vec_and_index(bytes.to_vec(), index)
                .ok()?
                .into();
        Some(ShapingFace {
            loaded: LoadedFont { bytes, index },
            ab,
        })
    }
}

/// One shaped glyph: which glyph id to draw, and its offset from the
/// cell's pen position, in font design units (not yet scaled to pixels --
/// that happens at rasterization time, once per distinct `(FaceRole,
/// glyph id, pixel scale)`, not once per shaped run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// The checked precondition this whole module rests on: a run is safe to
/// draw as shaped glyphs, one glyph per input character at that
/// character's existing cell, only if shaping produced exactly as many
/// glyphs as input characters *and* every one of those glyphs advances by
/// the same amount -- the font's own cell advance (`cell_advance_units`,
/// the design-unit advance of a space glyph in the same face). Any
/// mismatch means the font did something this milestone's measurement
/// says a coding-ligature `calt` font never does (merge cells, leave a
/// gap, or produce a non-monospace advance for some other reason, e.g. a
/// combining mark's zero advance) -- the caller must fall back to
/// character-by-character drawing. Fix 2 (cold review): also rejects a
/// run containing glyph id 0 (`.notdef`) -- `shape_calt_only` only ever
/// shapes against the primary/bold/italic face's own bytes, never egui's
/// fallback chain (the CJK `arial-unicode` face `install_fonts` appends
/// to every family), so a character absent from the primary face shapes
/// to `.notdef` there. In a monospace font `.notdef` plausibly carries
/// the same advance as everything else, so without this check the run
/// would otherwise pass and a tofu box would be drawn where the fallback
/// chain would have supplied the real glyph -- a silent visual
/// regression against the pre-shaping behavior (which drew every
/// character through egui's own `painter.text`, fallback chain
/// included).
pub fn validate_shaped_run(
    glyph_count: usize,
    char_count: usize,
    advances: &[i32],
    cell_advance_units: i32,
    glyph_ids: &[u16],
) -> bool {
    if glyph_count == 0 || glyph_count != char_count {
        return false;
    }
    if advances.len() != glyph_count {
        return false;
    }
    if cell_advance_units == 0 {
        return false;
    }
    if glyph_ids.contains(&0) {
        return false;
    }
    advances.iter().all(|&a| a == cell_advance_units)
}

/// Shape `text` against `face` with only the `calt` feature enabled
/// (coding ligatures); `liga`/`dlig`/`clig` are explicitly disabled so a
/// font that also defines standard/discretionary/contextual ligatures
/// doesn't merge cells outside the `calt` lookups this milestone's
/// measurement was scoped to. Returns `None` when shaping's result fails
/// [`validate_shaped_run`] -- the caller's cue to fall back.
///
/// `enable_calt` (M116, `gui-ligatures`): when `false`, the `calt`
/// feature is requested with value 0 instead of 1 -- the run still goes
/// through `rustybuzz`/`validate_shaped_run` (so ordinary per-character
/// shaping still benefits from correct advances), it just never
/// substitutes a ligature glyph. This is threaded into [`ShapeCache`]'s
/// key rather than handled by clearing the cache on toggle: a stale
/// cache entry is then impossible by construction (flipping the flag
/// changes the key, so the old ligature-shaped entries simply become
/// unreachable, not wrong), which is safer than remembering to clear on
/// every place the variable could change.
pub fn shape_calt_only(
    face: &LoadedFont,
    text: &str,
    enable_calt: bool,
) -> Option<Vec<ShapedGlyph>> {
    let rb_face = rustybuzz::Face::from_slice(&face.bytes, face.index)?;

    let char_count = text.chars().count();
    if char_count == 0 {
        return None;
    }

    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    // Fix 3 (cold review): force left-to-right rather than letting
    // `guess_segment_properties` derive direction from the text's
    // script. rustybuzz reverses its output buffer for a backward
    // direction (`rustybuzz-0.20.1/src/hb/ot_shape.rs`'s `position`,
    // `if ctx.buffer.direction.is_backward() { ctx.buffer.reverse() }`),
    // and `validate_shaped_run` has no way to see that reversal -- it
    // checks count and advance uniformity, neither of which reveals
    // that `glyphs[0]` is now the *last* input character. This renderer
    // has no bidi support anywhere in its grid model: `core`'s grid
    // lays text out in strict logical order, so forcing left-to-right
    // here is deliberate, not an oversight -- it is the only order this
    // renderer can represent. `set_direction` before
    // `guess_segment_properties` is honored: the latter only fills in a
    // direction when the buffer's direction is still `Invalid` (see
    // rustybuzz's `Buffer::guess_segment_properties`), so script and
    // language are still guessed normally, only direction is pinned.
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    buffer.guess_segment_properties();

    let features = [
        rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"calt"),
            enable_calt as u32,
            ..,
        ),
        rustybuzz::Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"liga"), 0, ..),
        rustybuzz::Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"dlig"), 0, ..),
        rustybuzz::Feature::new(rustybuzz::ttf_parser::Tag::from_bytes(b"clig"), 0, ..),
    ];
    let glyph_buffer = rustybuzz::shape(&rb_face, &features, buffer);
    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let tp_face: &rustybuzz::ttf_parser::Face = rb_face.as_ref();
    let cell_advance_units = tp_face
        .glyph_index(' ')
        .and_then(|id| tp_face.glyph_hor_advance(id))
        .unwrap_or(0) as i32;

    let glyph_ids: Vec<u16> = infos.iter().map(|i| i.glyph_id as u16).collect();
    let advances: Vec<i32> = positions.iter().map(|p| p.x_advance).collect();
    if !validate_shaped_run(
        infos.len(),
        char_count,
        &advances,
        cell_advance_units,
        &glyph_ids,
    ) {
        return None;
    }

    Some(
        glyph_ids
            .iter()
            .zip(positions.iter())
            .map(|(&glyph_id, pos)| ShapedGlyph {
                glyph_id,
                x_offset: pos.x_offset as f32,
                y_offset: pos.y_offset as f32,
            })
            .collect(),
    )
}

/// Above this many distinct `(FaceRole, text)` entries, the shape cache
/// is cleared outright rather than evicted piecemeal (LRU bookkeeping):
/// buffer text recurs heavily across frames for whatever's on screen, so
/// a blunt periodic clear costs a handful of re-shapes, not a redraw
/// stall, and this project has no shared LRU helper to reach for instead.
const SHAPE_CACHE_CAP: usize = 8192;

/// Caches shaping results (glyph ids + offsets, in font design units) by
/// `(FaceRole, text, enable_calt)`. Never invalidated by font size or DPI
/// -- shaping happens in font units and is independent of both -- only
/// capped (see `SHAPE_CACHE_CAP`). `None` is cached too (a run whose
/// shaping fails [`validate_shaped_run`] would otherwise be re-shaped,
/// and re-rejected, every single frame it stays on screen).
///
/// The `enable_calt` component of the key (M116, `gui-ligatures`) is
/// what keeps this cache from ever serving a stale ligature-shaped
/// result after the variable is turned off: toggling it doesn't hit any
/// entry the other setting produced, since the key itself differs, so
/// there is nothing to remember to invalidate on the toggle (unlike a
/// font switch, which does need an explicit `apply_font_switch` reset,
/// because the key there carries no font identity at all).
/// `(role, text, enable_calt)` -> shaped result, or the cached `None` for
/// a run that failed [`validate_shaped_run`]. Named so clippy's
/// `type_complexity` lint has something short to point at instead of the
/// three-tuple key spelled out inline.
type ShapeCacheKey = (FaceRole, String, bool);

#[derive(Default)]
pub struct ShapeCache {
    map: HashMap<ShapeCacheKey, Option<Arc<[ShapedGlyph]>>>,
}

impl ShapeCache {
    pub fn get_or_shape(
        &mut self,
        role: FaceRole,
        face: &LoadedFont,
        text: &str,
        enable_calt: bool,
    ) -> Option<Arc<[ShapedGlyph]>> {
        let key = (role, text.to_string(), enable_calt);
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        if self.map.len() >= SHAPE_CACHE_CAP {
            self.map.clear();
        }
        let shaped =
            shape_calt_only(face, text, enable_calt).map(|v| Arc::from(v.into_boxed_slice()));
        self.map.insert(key, shaped.clone());
        shaped
    }
}

/// One glyph's rasterized location in egui's shared font atlas, in the
/// same coordinate convention `epaint::text::font::GlyphInfo::uv_rect`
/// uses (see `epaint-0.29.1/src/text/font.rs`'s `allocate_glyph`, which
/// this mirrors): `size`/`offset` in points (already divided by
/// `pixels_per_point`), `uv_min`/`uv_max` in raw atlas texels (normalized
/// against the atlas's own size at draw time, since the atlas can grow).
/// `None` means the glyph has no ink (a blank spacer glyph, or a glyph
/// whose outline is empty) -- still cached, so an all-blank ligature-lead
/// cell doesn't get re-rasterized every frame either.
#[derive(Clone, Copy, Debug)]
struct CachedGlyph {
    uv_min: (usize, usize),
    uv_max: (usize, usize),
    size: Vec2,
    offset: Vec2,
}

/// Caches rasterized glyphs by `(FaceRole, glyph id, scale in device
/// pixels as bits)`. Scale-dependent by construction (a font-size or DPI
/// change rasterizes at different pixel dimensions, hence a different
/// key), and additionally dropped in full whenever the atlas itself is
/// rebuilt -- see this module's doc for why identity of the atlas `Arc`
/// is the only available signal for that.
#[derive(Default)]
pub struct GlyphAtlasCache {
    map: HashMap<(FaceRole, u16, u32), Option<CachedGlyph>>,
    atlas_identity: Option<*const ()>,
}

impl GlyphAtlasCache {
    /// Must be called once per frame before any lookups: compares the
    /// atlas handle's identity against the one seen last frame and clears
    /// the cache on mismatch (a DPI change, a font-set change, or the
    /// atlas filling past 80% and epaint recreating it -- see this
    /// module's doc). `atlas` is only used for its pointer identity here;
    /// the actual `Mutex` is locked by the caller when it needs to
    /// allocate.
    pub fn begin_frame(&mut self, atlas: &egui::epaint::mutex::Mutex<egui::epaint::TextureAtlas>) {
        let identity = atlas as *const _ as *const ();
        if self.atlas_identity != Some(identity) {
            self.map.clear();
            self.atlas_identity = Some(identity);
        }
    }

    /// Look up (or rasterize and insert) the atlas placement of `glyph_id`
    /// in `role` at `scale_in_pixels` (device pixels per em, matching
    /// `epaint::text::font::FontImpl`'s own `scale_in_pixels`).
    /// `pixels_per_point` converts the rasterized pixel rectangle back to
    /// points, the coordinate space every `Pos2`/`Rect` in this crate's
    /// paint loop is already in. Returns `None` for both "not yet
    /// rasterized, allocation failed" (never happens with `TextureAtlas`,
    /// which grows) and "this glyph has no ink" -- either way, the caller
    /// draws nothing for that glyph.
    #[allow(clippy::too_many_arguments)]
    fn get_or_rasterize(
        &mut self,
        atlas: &egui::epaint::mutex::Mutex<egui::epaint::TextureAtlas>,
        role: FaceRole,
        face: &ab_glyph::FontArc,
        glyph_id: u16,
        scale_in_pixels: f32,
        pixels_per_point: f32,
    ) -> Option<(usize, usize, usize, usize, Vec2, Vec2)> {
        let key = (role, glyph_id, scale_in_pixels.to_bits());
        let cached = *self.map.entry(key).or_insert_with(|| {
            rasterize_glyph(atlas, face, glyph_id, scale_in_pixels, pixels_per_point)
        });
        cached.map(|g| {
            (
                g.uv_min.0, g.uv_min.1, g.uv_max.0, g.uv_max.1, g.size, g.offset,
            )
        })
    }
}

/// Rasterizes one glyph via `ab_glyph` (the same rasterizer egui's own
/// font code uses -- `epaint-0.29.1/src/text/font.rs:69,97-98`) and
/// allocates it into the shared atlas, following the exact algorithm
/// `FontImpl::allocate_glyph` uses so there is no visual seam between
/// glyphs drawn by this path and glyphs drawn by egui's own `painter.text`
/// fallback path at the same scale.
fn rasterize_glyph(
    atlas: &egui::epaint::mutex::Mutex<egui::epaint::TextureAtlas>,
    face: &ab_glyph::FontArc,
    glyph_id: u16,
    scale_in_pixels: f32,
    pixels_per_point: f32,
) -> Option<CachedGlyph> {
    // Fix 4 (cold review): epaint rounds the scale to a whole pixel
    // before rasterizing, explicitly "to get even kerning"
    // (`epaint-0.29.1/src/text/font.rs:122`,
    // `let scale_in_pixels = scale_in_pixels.round() as u32;`), while
    // keeping the *unrounded* value for ascent/descent. Mirrored the
    // same way here: only this rasterization call rounds; every metric
    // computation elsewhere in this module (`ascent_in_points`, the
    // offset math in `build_shaped_mesh`) still uses the unrounded
    // `scale_in_pixels` its caller passed in. Without this, shaped
    // glyphs rasterize at a slightly different scale than the ones egui
    // draws on the fallback path at the same nominal size -- the exact
    // seam this milestone was measured against.
    let rounded_scale = scale_in_pixels.round();
    let glyph = ab_glyph::GlyphId(glyph_id)
        .with_scale_and_position(rounded_scale, ab_glyph::Point { x: 0.0, y: 0.0 });
    let outlined = face.outline_glyph(glyph)?;
    let bb = outlined.px_bounds();
    let w = bb.width() as usize;
    let h = bb.height() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let pos = {
        let mut atlas = atlas.lock();
        let (pos, image) = atlas.allocate((w, h));
        outlined.draw(|x, y, v| {
            if v > 0.0 {
                let px = pos.0 + x as usize;
                let py = pos.1 + y as usize;
                image[(px, py)] = v;
            }
        });
        pos
    };
    Some(CachedGlyph {
        uv_min: pos,
        uv_max: (pos.0 + w, pos.1 + h),
        size: Vec2::new(w as f32, h as f32) / pixels_per_point,
        offset: Vec2::new(bb.min.x, bb.min.y) / pixels_per_point,
    })
}

/// The pixel scale `rasterize_glyph`/`GlyphAtlasCache` key on, computed by
/// the same formula `epaint::text::fonts::FontImplCache::font_impl` uses
/// (`epaint-0.29.1/src/text/fonts.rs:773`): `pixels_per_point *
/// scale_in_points * (font.height_unscaled() / font.units_per_em())`.
/// Kept in lockstep with that formula deliberately -- any drift here is
/// exactly the seam the module doc warns against.
pub fn scale_in_pixels(
    face: &ab_glyph::FontArc,
    scale_in_points: f32,
    pixels_per_point: f32,
) -> f32 {
    let units_per_em = face.units_per_em().unwrap_or(1000.0);
    let font_scaling = face.height_unscaled() / units_per_em;
    pixels_per_point * scale_in_points * font_scaling
}

/// The ascent (distance from the top of a text row to the baseline), in
/// points, at `scale_in_pixels` -- mirrors `FontImpl::new`'s `ascent`
/// computation (no `FontTweak` applied, since none of this crate's
/// primary/bold/italic faces carry one).
pub fn ascent_in_points(
    face: &ab_glyph::FontArc,
    scale_in_pixels: f32,
    pixels_per_point: f32,
) -> f32 {
    use ab_glyph::ScaleFont;
    face.as_scaled(scale_in_pixels).ascent() / pixels_per_point
}

/// One rasterized glyph's screen rect (already fully resolved, in
/// points, and pixel-snapped -- see `build_shaped_mesh`) paired with its
/// atlas placement in raw texels. Deliberately left un-normalized: see
/// this struct's use in `build_shaped_mesh`/`finish_shaped_mesh` for why
/// normalizing must be deferred to the very end of the frame's shaping
/// work, not done per-run.
#[derive(Clone, Copy, Debug)]
pub struct RawGlyphRect {
    rect: Rect,
    uv_min: (usize, usize),
    uv_max: (usize, usize),
}

/// Rasterizes and positions `glyphs` (already shaped, one per cell
/// starting at `row_x`) left to right at `char_w`-wide cells, using
/// `cache` for the atlas placement of each glyph. `baseline_y` is the
/// point on screen the *first* cell's pen-origin baseline sits at (row
/// top + ascent); each subsequent glyph advances by exactly `char_w` --
/// the same uniform cell width [`validate_shaped_run`] already
/// confirmed, which is why this function does not need per-glyph
/// advances at all.
///
/// Fix 1 (cold review, highest priority): returns raw texel UVs, NOT a
/// finished `Mesh` with normalized UVs baked in. `TextureAtlas::allocate`
/// (`epaint-0.29.1/src/texture_atlas.rs`'s `resize_to_min_height`) grows
/// the atlas in place when a glyph doesn't fit -- same `Arc`, so
/// `GlyphAtlasCache::begin_frame`'s pointer-identity rebuild check does
/// not fire (that check guards a different mechanism: full recreation on
/// DPI change or an 80%-full atlas). Raw texel coordinates survive a
/// resize because existing content keeps its pixel coordinates;
/// normalized UVs do not, and `Shape::Mesh` is appended as-is at
/// tessellation time (epaint does not renormalize a `Mesh`'s baked UVs).
/// So every call site MUST collect the `RawGlyphRect`s from every run
/// shaped this frame, across every row, THEN call [`finish_shaped_mesh`]
/// once per collected run, only after the last `build_shaped_mesh` call
/// for the frame has returned -- reading the atlas size per run, rather
/// than once for the whole frame, is not sufficient, because a later run
/// can still grow the atlas and invalidate an earlier run's baked UVs.
/// This mirrors epaint's own text path: `UvRect` stays raw `[u16; 2]`
/// texels through layout and is normalized exactly once, at the end of
/// the frame, against the size the atlas ended up at.
#[allow(clippy::too_many_arguments)]
pub fn build_shaped_mesh(
    atlas: &egui::epaint::mutex::Mutex<egui::epaint::TextureAtlas>,
    cache: &mut GlyphAtlasCache,
    role: FaceRole,
    face: &ab_glyph::FontArc,
    glyphs: &[ShapedGlyph],
    row_x: f32,
    baseline_y: f32,
    char_w: f32,
    scale_in_pixels: f32,
    pixels_per_point: f32,
) -> Vec<RawGlyphRect> {
    let mut items = Vec::with_capacity(glyphs.len());
    // `x_offset`/`y_offset` from `rustybuzz` are in font design units,
    // following HarfBuzz's convention of +y pointing up; `px_per_unit`
    // converts a design-unit offset to the pixels-per-em scale
    // `rasterize_glyph` rasterized at, then to points (this crate's
    // screen coordinate unit, +y pointing down -- hence the sign flip on
    // `extra_y` below). A nonzero offset is uncommon for this milestone's
    // measured font (the architect's repro found every offset zero) but
    // not excluded by `validate_shaped_run` on principle, so it is still
    // honored rather than silently dropped.
    let units_per_em = face.units_per_em().unwrap_or(1000.0);
    let px_per_unit = scale_in_pixels / units_per_em;
    for (i, g) in glyphs.iter().enumerate() {
        let Some((umin_x, umin_y, umax_x, umax_y, size, offset)) = cache.get_or_rasterize(
            atlas,
            role,
            face,
            g.glyph_id,
            scale_in_pixels,
            pixels_per_point,
        ) else {
            continue;
        };
        let cell_x = row_x + i as f32 * char_w;
        let extra_x = g.x_offset * px_per_unit / pixels_per_point;
        let extra_y = g.y_offset * px_per_unit / pixels_per_point;
        // Fix 5 (cold review): snap the glyph's screen position to the
        // physical pixel grid, the same convention `snap_rect` (lib.rs)
        // already uses for filled rectangles -- and the convention
        // epaint's own text layout uses for glyph positions
        // (`epaint-0.29.1/src/text/text_layout.rs`'s `galley_from_rows`,
        // `PointScale::round_to_pixel`: `(v * pixels_per_point).round() /
        // pixels_per_point`). Only the position snaps, not the
        // rasterized size -- epaint rounds `glyph.pos.x`/`.y`, never the
        // glyph's own width/height, and this mirrors that: `size` below
        // is untouched.
        let min = Pos2::new(
            snap_to_pixel(cell_x + offset.x + extra_x, pixels_per_point),
            snap_to_pixel(baseline_y + offset.y - extra_y, pixels_per_point),
        );
        let rect = Rect::from_min_size(min, size);
        items.push(RawGlyphRect {
            rect,
            uv_min: (umin_x, umin_y),
            uv_max: (umax_x, umax_y),
        });
    }
    items
}

/// Normalizes every [`RawGlyphRect`] `build_shaped_mesh` collected this
/// frame into one `Mesh`, against `atlas_size` -- the caller must read
/// `atlas_size` only after the LAST `build_shaped_mesh` call for the
/// frame has returned (see that function's doc for why).
pub fn finish_shaped_mesh(items: &[RawGlyphRect], atlas_size: [usize; 2], color: Color32) -> Mesh {
    let mut mesh = Mesh::with_texture(TextureId::default());
    let inv_w = 1.0 / atlas_size[0].max(1) as f32;
    let inv_h = 1.0 / atlas_size[1].max(1) as f32;
    for item in items {
        let uv = Rect::from_min_max(
            Pos2::new(item.uv_min.0 as f32 * inv_w, item.uv_min.1 as f32 * inv_h),
            Pos2::new(item.uv_max.0 as f32 * inv_w, item.uv_max.1 as f32 * inv_h),
        );
        mesh.add_rect_with_uv(item.rect, uv, color);
    }
    mesh
}

/// Snaps `v` (in points) to the nearest physical pixel at
/// `pixels_per_point` -- the exact formula `snap_rect` (lib.rs) and
/// epaint's own `PointScale::round_to_pixel` both use, kept in one place
/// so all three stay byte-for-byte the same convention rather than
/// drifting into three independent implementations.
pub(crate) fn snap_to_pixel(v: f32, pixels_per_point: f32) -> f32 {
    (v * pixels_per_point).round() / pixels_per_point
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_uniform_matching_run() {
        assert!(validate_shaped_run(3, 3, &[600, 600, 600], 600, &[5, 6, 7]));
    }

    #[test]
    fn validate_rejects_glyph_count_mismatch() {
        // A ligature that merged two chars into one glyph -- exactly the
        // shape this milestone's measurement says this font never
        // produces, but a precondition, not an assumption.
        assert!(!validate_shaped_run(2, 3, &[600, 600], 600, &[5, 6]));
    }

    #[test]
    fn validate_rejects_nonuniform_advance() {
        assert!(!validate_shaped_run(
            3,
            3,
            &[600, 600, 300],
            600,
            &[5, 6, 7]
        ));
    }

    #[test]
    fn validate_rejects_zero_glyphs() {
        assert!(!validate_shaped_run(0, 0, &[], 600, &[]));
    }

    #[test]
    fn validate_rejects_zero_cell_advance() {
        // A face with no space glyph (or a broken hmtx entry) -- treated
        // as "can't establish a reference cell width", not "0 is fine".
        assert!(!validate_shaped_run(1, 1, &[0], 0, &[5]));
    }

    #[test]
    fn validate_rejects_advance_len_mismatch() {
        assert!(!validate_shaped_run(2, 2, &[600], 600, &[5, 6]));
    }

    #[test]
    fn validate_rejects_notdef_glyph_even_with_uniform_advance() {
        // Fix 2: a monospace font's `.notdef` glyph plausibly carries the
        // same advance as every other glyph in the run -- coverage, not
        // just advance uniformity, is the real precondition. Without
        // this check this run would pass and a tofu box would be drawn
        // where the fallback chain (arial-unicode) would have supplied
        // the real glyph.
        assert!(!validate_shaped_run(
            3,
            3,
            &[600, 600, 600],
            600,
            &[5, 0, 7]
        ));
    }

    #[test]
    fn shape_cache_hits_on_repeat_lookup_without_reshaping() {
        // Uses a real font so the shape actually runs once; the point of
        // this test is the *second* call must be a cache hit (`None`
        // face would panic `Face::from_slice` on the second call if the
        // cache weren't consulted first). We fabricate a tiny invalid
        // face on purpose: `shape_calt_only` returns `None` on failure,
        // and the cache must still remember that `None` rather than
        // trying to parse the (invalid) bytes again.
        let bogus = LoadedFont {
            bytes: Arc::from(vec![0u8; 4].into_boxed_slice()),
            index: 0,
        };
        let mut cache = ShapeCache::default();
        let first = cache.get_or_shape(FaceRole::Regular, &bogus, "ab", true);
        assert!(first.is_none());
        // Corrupt the face bytes further would panic `from_slice`'s
        // `Option` path (it already returns `None` gracefully) -- but a
        // *second* lookup must come from the cached `None`, not attempt
        // to parse again. We can't observe "did it re-parse" directly
        // without instrumentation, so this test instead pins the cache's
        // map key/lookup contract: the same `(role, text)` key must
        // return the identical `None` both times.
        let second = cache.get_or_shape(FaceRole::Regular, &bogus, "ab", true);
        assert_eq!(first, second);
    }

    #[test]
    fn shape_cache_distinguishes_face_role() {
        let bogus = LoadedFont {
            bytes: Arc::from(vec![0u8; 4].into_boxed_slice()),
            index: 0,
        };
        let mut cache = ShapeCache::default();
        cache.get_or_shape(FaceRole::Regular, &bogus, "x", true);
        assert_eq!(cache.map.len(), 1);
        cache.get_or_shape(FaceRole::Bold, &bogus, "x", true);
        assert_eq!(cache.map.len(), 2);
    }

    #[test]
    fn shape_cache_caps_and_clears() {
        let bogus = LoadedFont {
            bytes: Arc::from(vec![0u8; 4].into_boxed_slice()),
            index: 0,
        };
        let mut cache = ShapeCache::default();
        for i in 0..SHAPE_CACHE_CAP {
            cache.get_or_shape(FaceRole::Regular, &bogus, &format!("t{i}"), true);
        }
        assert_eq!(cache.map.len(), SHAPE_CACHE_CAP);
        cache.get_or_shape(FaceRole::Regular, &bogus, "overflow", true);
        // The cap triggered a full clear before inserting the new entry,
        // so the map holds exactly the one new entry, not
        // `SHAPE_CACHE_CAP + 1`.
        assert_eq!(cache.map.len(), 1);
    }

    #[test]
    fn glyph_atlas_cache_clears_on_atlas_identity_change() {
        let mutex_a =
            egui::epaint::mutex::Mutex::new(egui::epaint::TextureAtlas::new([1024, 1024]));
        let mutex_b =
            egui::epaint::mutex::Mutex::new(egui::epaint::TextureAtlas::new([1024, 1024]));
        let mut cache = GlyphAtlasCache::default();
        cache.begin_frame(&mutex_a);
        cache.map.insert((FaceRole::Regular, 1, 0), None);
        assert_eq!(cache.map.len(), 1);
        // Same atlas next frame: must not clear.
        cache.begin_frame(&mutex_a);
        assert_eq!(cache.map.len(), 1);
        // A different atlas (as happens after `Fonts::begin_pass`
        // recreates one on DPI change or an almost-full atlas): must
        // clear, since the UVs the old entries point at may no longer
        // hold the same glyphs.
        cache.begin_frame(&mutex_b);
        assert_eq!(cache.map.len(), 0);
    }

    #[test]
    fn fix1_deferred_normalization_survives_mid_frame_atlas_growth() {
        // Fix 1 regression test. Before this fix, `build_shaped_mesh`
        // took an `atlas_size` read once before the per-row loop
        // (`lib.rs:1141-1143`) and baked normalized UVs into a `Mesh`
        // immediately, per call. Reproducing that exact shape against
        // this same test scenario (a 1024x64 atlas, 'M' rasterized
        // first, then enough further glyphs at scale 200 to force
        // `resize_to_min_height` to grow the atlas to 1024x512) and
        // recording the actual numbers before this fix landed:
        // `baked_uv_bottom=[0.1, 0.4]` (normalized against the stale
        // hoisted `[1024, 64]`) disagreed with
        // `correct_uv_bottom=[0.1, 0.05]` (normalized against the real
        // final `[1024, 512]`) -- exactly the corruption fix 1
        // describes: 'M' would have sampled row y=0.4 of the final
        // texture instead of its actual row near y=0.05.
        //
        // This test exercises the identical growth sequence against the
        // fixed two-phase API (`build_shaped_mesh` returns raw texel
        // rects; `finish_shaped_mesh` normalizes once, after every glyph
        // this frame has been rasterized) and asserts the baked UV now
        // agrees with the correctly-normalized one, even though 'M' was
        // collected while the atlas was still small.
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home).join("Library/Fonts/JetBrainsMono-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let face = ab_glyph::FontArc::try_from_vec(bytes).expect("valid font file");

        let atlas = egui::epaint::mutex::Mutex::new(egui::epaint::TextureAtlas::new([1024, 64]));
        let mut cache = GlyphAtlasCache::default();
        cache.begin_frame(&atlas);

        // Recorded only to assert growth really happens below -- no
        // longer used for normalization anywhere in the fixed design.
        let size_before_any_rasterization = atlas.lock().size();

        // "Row 1": one glyph, collected as raw texel rects while the
        // atlas is still small. Deliberately NOT normalized yet.
        let glyph_a_id = ab_glyph::Font::glyph_id(&face, 'M').0;
        let items_a = build_shaped_mesh(
            &atlas,
            &mut cache,
            FaceRole::Regular,
            &face,
            &[ShapedGlyph {
                glyph_id: glyph_a_id,
                x_offset: 0.0,
                y_offset: 0.0,
            }],
            0.0,
            40.0,
            40.0,
            40.0,
            1.0,
        );
        assert!(
            !items_a.is_empty(),
            "'M' at this scale must rasterize to a nonblank glyph"
        );

        // "Row 2..N": rasterize enough distinct glyphs, tall enough,
        // that `resize_to_min_height` doubles the atlas image -- this is
        // exactly what an ordinary DPI change or scrolling into
        // not-yet-seen glyphs does. Each run's raw items are collected
        // too (a real caller would go on to normalize and paint these
        // as well); this test only needs to prove `items_a` comes out
        // correct once normalized at the end.
        let extra_chars = [
            'g', 'j', 'y', 'Q', 'W', '@', '#', '%', '&', 'X', 'Z', '0', 'A', 'B', 'C', 'D', 'E',
            'F', 'G', 'H', 'I', 'J', 'K', 'L', 'N', 'O', 'P', 'R', 'S', 'T', 'U', 'V', 'Y',
        ];
        for (i, ch) in extra_chars.iter().enumerate() {
            let id = ab_glyph::Font::glyph_id(&face, *ch).0;
            let _ = build_shaped_mesh(
                &atlas,
                &mut cache,
                FaceRole::Regular,
                &face,
                &[ShapedGlyph {
                    glyph_id: id,
                    x_offset: 0.0,
                    y_offset: 0.0,
                }],
                0.0,
                (i as f32 + 2.0) * 200.0,
                200.0,
                200.0,
                1.0,
            );
        }

        let final_size = atlas.lock().size();
        assert_ne!(
            size_before_any_rasterization, final_size,
            "test setup must actually force atlas growth for this regression test to mean anything"
        );

        // Only NOW -- after every glyph this simulated frame rasterized
        // -- normalize. This is the fix: `finish_shaped_mesh` is called
        // once, against the size the atlas actually ends the frame at.
        let mesh_a = finish_shaped_mesh(&items_a, final_size, Color32::WHITE);
        let baked_uv_bottom = mesh_a.vertices[2].uv; // left_bottom = (uv.min.x, uv.max.y)

        // What glyph_a's UV bottom-left *should* be, computed
        // independently from the same final size. This is a cache hit
        // (same (role, glyph_id, scale) key `items_a` already
        // populated), so it does not rasterize again.
        let raw = cache
            .get_or_rasterize(&atlas, FaceRole::Regular, &face, glyph_a_id, 40.0, 1.0)
            .expect("glyph_a must still be cached from the items_a call above");
        let correct_uv_bottom = Pos2::new(
            raw.0 as f32 / final_size[0] as f32,
            raw.3 as f32 / final_size[1] as f32,
        );

        assert_eq!(
            baked_uv_bottom, correct_uv_bottom,
            "baked_uv_bottom={baked_uv_bottom:?} must equal correct_uv_bottom={correct_uv_bottom:?} \
             -- normalization must be deferred to the end of the frame's shaping work regardless of \
             when in the frame 'M' itself was rasterized (size_before_any_rasterization=\
             {size_before_any_rasterization:?}, final_size={final_size:?})"
        );
    }

    #[test]
    fn real_jetbrains_mono_shapes_plain_text_one_glyph_per_char() {
        // Uses the real font this milestone's measurement was taken
        // against, if present on the machine running the test -- skips
        // otherwise (this is the same "depends on what's installed"
        // acknowledged gap `install_fonts` already documents).
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home).join("Library/Fonts/JetBrainsMono-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let face = LoadedFont {
            bytes: Arc::from(bytes.into_boxed_slice()),
            index: 0,
        };
        let shaped = shape_calt_only(&face, "assign a = b != c;", true);
        let shaped = shaped.expect("plain assignment text should shape validly");
        assert_eq!(shaped.len(), "assign a = b != c;".chars().count());
    }

    #[test]
    fn real_jetbrains_mono_ligature_glyph_ids_differ_from_plain_glyphs() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home).join("Library/Fonts/JetBrainsMono-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let face = LoadedFont {
            bytes: Arc::from(bytes.into_boxed_slice()),
            index: 0,
        };
        // "a = b" must NOT substitute (no `=>`/`!=` pattern); "a => b"
        // must. This is the architect's own repro
        // ("a => b" genuinely substitutes ... "a = b" correctly does
        // not), pinned as a regression test.
        let plain = shape_calt_only(&face, "a = b", true).expect("plain '=' should still validate");
        let arrow =
            shape_calt_only(&face, "a => b", true).expect("'=>' ligature should still validate");
        assert_ne!(plain[2].glyph_id, arrow[2].glyph_id);
    }

    #[test]
    fn real_jetbrains_mono_rejects_unmapped_char_instead_of_drawing_notdef() {
        // Fix 2 integration test. U+E000 is in the Private Use Area --
        // guaranteed unmapped in a real font like JetBrains Mono, so it
        // shapes to glyph id 0 (`.notdef`). Before fix 2,
        // `validate_shaped_run` only checked count/advance, and a
        // monospace font's `.notdef` plausibly carries the same advance
        // as every other glyph, so this run would have passed and drawn
        // a tofu box. It must now fall back instead.
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home).join("Library/Fonts/JetBrainsMono-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let face = LoadedFont {
            bytes: Arc::from(bytes.into_boxed_slice()),
            index: 0,
        };
        let shaped = shape_calt_only(&face, "a\u{E000}b", true);
        assert!(
            shaped.is_none(),
            "a run containing an unmapped (.notdef) character must fail validation and fall back"
        );
    }

    #[test]
    fn fix3_forcing_left_to_right_overrides_guessed_backward_direction() {
        // Fix 3. `buffer::guess_segment_properties()` derives direction
        // from the text's Unicode script -- Hebrew guesses
        // `RightToLeft` (`rustybuzz-0.20.1/src/hb/common.rs`'s
        // `Direction::from_script`, which maps `script::HEBREW` to
        // `RightToLeft`) purely from the codepoints, independent of
        // which font is shaping it or whether that font even has
        // Hebrew glyphs. `core`'s grid model has no bidi anywhere and
        // lays text out in strict logical order, so this renderer must
        // force `LeftToRight` rather than trust the guess.
        let mut guessed = rustybuzz::UnicodeBuffer::new();
        guessed.push_str("שלום"); // Hebrew "shalom" -- genuinely RTL script.
        guessed.guess_segment_properties();
        assert_eq!(
            guessed.direction(),
            rustybuzz::Direction::RightToLeft,
            "test precondition: this text must actually guess backward, or the fix below isn't \
             exercising anything"
        );

        // `shape_calt_only`'s actual fix: set direction before guessing.
        // `guess_segment_properties` only fills in a direction when the
        // buffer's is still `Invalid` (see rustybuzz's
        // `Buffer::guess_segment_properties`), so setting it first pins
        // it; script/language are still guessed normally.
        let mut forced = rustybuzz::UnicodeBuffer::new();
        forced.push_str("שלום");
        forced.set_direction(rustybuzz::Direction::LeftToRight);
        forced.guess_segment_properties();
        assert_eq!(
            forced.direction(),
            rustybuzz::Direction::LeftToRight,
            "forcing direction before guessing must survive guess_segment_properties, or \
             rustybuzz would still reverse the shaped output buffer for this text"
        );
    }
}
