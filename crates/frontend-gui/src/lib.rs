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

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Vec2};

use core::commands::{handle_key, Key};
use core::editor::Editor;
use core::keymap::{ctrl_encode, META};
use core::redisplay::{frame_base_style, render, Underline};
use elisp::Interp;

/// Fallbacks when no theme has set the `default` face (never in
/// practice — themes.el loads at startup).
const FALLBACK_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const FALLBACK_FG: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);

const BOLD_FAMILY: &str = "mono-bold";
const ITALIC_FAMILY: &str = "mono-italic";

/// Fix D (trailing review): the window after input during which the
/// cursor is forced fully opaque (blink/fade suppressed) -- used by both
/// the wake-cadence pick (`update`'s `blink_suppressed`) and
/// `cursor_alpha`'s own suppression check. Previously each hard-coded its
/// own `Duration::from_millis(300)`; nothing kept the two literals in
/// sync, so tuning one without the other would desync "the cadence code
/// thinks the cursor is static" from "the cursor is actually animating"
/// (or the reverse).
const BLINK_SUPPRESS_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

/// Default window size (M-visual-quality): big enough that a Verilog
/// module instantiation with a wide port list isn't immediately
/// scrollbar territory. `min_inner_size` keeps a user-shrunk window from
/// collapsing into an unusable sliver. Both the size and the position are
/// then persisted across launches by eframe's own storage (the
/// `persistence` Cargo feature turns this on; `persist_window` in
/// `NativeOptions` defaults to `true`), so these two numbers are only the
/// fallback used on first launch or when the store is empty.
pub fn run_gui(interp: Interp, ed: Rc<RefCell<Editor>>) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([480.0, 320.0])
            .with_title("Reticle"),
        ..Default::default()
    };
    eframe::run_native(
        "reticle",
        options,
        Box::new(move |cc| {
            let font_family = str_var(&interp, "gui-font-family");
            let (have_bold, have_italic) = install_fonts(&cc.egui_ctx, font_family.as_deref());
            Ok(Box::new(App {
                interp,
                ed,
                last_input: std::time::Instant::now(),
                have_bold,
                have_italic,
                last_frame_ms: 0.0,
            }))
        }),
    )
}

/// Preferred body-text fonts, in search order, each tried under every
/// directory in `FONT_DIRS`; the first one found on disk wins. JetBrains
/// Mono and Fira Code are the two monospace faces routinely singled out
/// for readability at RTL-editing sizes (wide port lists, long
/// parameterized module names); the rest are the macOS system monospace
/// faces this project already shipped with, kept as the fallback chain.
const FONT_CANDIDATES: [&str; 5] = [
    "JetBrainsMono-Regular.ttf",
    "FiraCode-Regular.ttf",
    "SFNSMono.ttf",
    "Menlo.ttc",
    "Monaco.ttf",
];

/// Directories searched for `FONT_CANDIDATES` (and for `gui-font-family`
/// when set), in order.
fn font_dirs() -> Vec<std::path::PathBuf> {
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
/// bytes.
fn find_font(file_name: &str) -> Option<Vec<u8>> {
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
/// before the built-in candidate list when set.
///
/// Fix E (trailing review, known gap): the candidate search
/// (`find_font`/`font_dirs`) and the fallback-chain construction below it
/// both depend on which real font files happen to exist on the running
/// machine's filesystem, so neither has a test or a mutation point --
/// asserting on the result would mean either shipping fixture font files
/// or hard-coding assumptions about what's installed on the CI/dev
/// machine, both of which this project has chosen not to do. This is an
/// acknowledged, deliberate gap, not an oversight.
fn install_fonts(ctx: &egui::Context, font_family: Option<&str>) -> (bool, bool) {
    let mut fonts = FontDefinitions::default();
    let mut loaded: Vec<String> = Vec::new();
    // The primary body font chain (fix 6b): gui-font-family first, then
    // EVERY built-in candidate actually present on disk, in search
    // order -- not just the first hit. A previous version of this
    // function took only the first hit as "primary" and dropped the
    // rest, so a primary font missing a glyph had nothing left to fall
    // back to except the CJK font. Each hit gets its own font_data key
    // (`primary-0`, `primary-1`, ...) so all of them can be chained.
    let search_order: Vec<&str> = font_family
        .into_iter()
        .chain(FONT_CANDIDATES.iter().copied())
        .collect();
    for name in search_order {
        if let Some(bytes) = find_font(name) {
            let key = format!("primary-{}", loaded.len());
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
    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/Menlo.ttc") {
        let mut bold = FontData::from_owned(bytes.clone());
        bold.index = 1;
        fonts.font_data.insert("menlo-bold".into(), bold);
        let mut italic = FontData::from_owned(bytes);
        italic.index = 2;
        fonts.font_data.insert("menlo-italic".into(), italic);
        // Each variant family keeps the CJK fallback chain behind it.
        let mut bold_chain = vec!["menlo-bold".to_string()];
        let mut italic_chain = vec!["menlo-italic".to_string()];
        if loaded.contains(&"arial-unicode".to_string()) {
            bold_chain.push("arial-unicode".into());
            italic_chain.push("arial-unicode".into());
        }
        fonts
            .families
            .insert(FontFamily::Name(BOLD_FAMILY.into()), bold_chain);
        fonts
            .families
            .insert(FontFamily::Name(ITALIC_FAMILY.into()), italic_chain);
        have_bold = true;
        have_italic = true;
    }
    ctx.set_fonts(fonts);
    (have_bold, have_italic)
}

struct App {
    interp: Interp,
    ed: Rc<RefCell<Editor>>,
    last_input: std::time::Instant,
    have_bold: bool,
    have_italic: bool,
    last_frame_ms: f32,
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
    let snap = |v: f32| (v * ppp).round() / ppp;
    Rect::from_min_max(
        Pos2::new(snap(rect.min.x), snap(rect.min.y)),
        Pos2::new(snap(rect.max.x), snap(rect.max.y)),
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
fn cursor_bg_color(normal_bg: Color32, under_cursor_bg: Color32, t: f32) -> Color32 {
    lerp_color(normal_bg, under_cursor_bg, t)
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_start = std::time::Instant::now();
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
        let bg = base.bg.map(to_color).unwrap_or(FALLBACK_BG);
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

                let pick_font = |style: &core::redisplay::Style| -> &FontId {
                    if style.bold && self.have_bold {
                        &bold_font
                    } else if style.italic && self.have_italic {
                        &italic_font
                    } else {
                        &font
                    }
                };

                for (row, line) in grid.lines.iter().enumerate() {
                    let y = origin.y + row as f32 * row_h;
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
                            eff.bg.map(to_color).unwrap_or(bg),
                        );
                        if eff.reverse {
                            std::mem::swap(&mut cfg, &mut cbg);
                        }
                        let mut cursor_rounding = 0.0_f32;
                        if is_cursor && cursor_type == "box" {
                            // Task 6 / fix 3: continuous fade for the
                            // background, hard switch at t=0.5 for the
                            // glyph color (see `cursor_glyph_color`'s doc
                            // comment for why lerping both by the same t
                            // is a zero-contrast bug, not just a style
                            // choice) -- and a `cursor` face overrides the
                            // swap target when the theme defines one.
                            let (target_fg, target_bg) = if let Some(cf) = &cursor_face {
                                (
                                    cf.fg.map(to_color).unwrap_or(cbg),
                                    cf.bg.map(to_color).unwrap_or(cfg),
                                )
                            } else {
                                let mut swapped_bg = cfg;
                                if swapped_bg == bg {
                                    swapped_bg = fg;
                                }
                                (cbg, swapped_bg)
                            };
                            cfg = cursor_glyph_color(cfg, target_fg, cursor_alpha);
                            cbg = cursor_bg_color(cbg, target_bg, cursor_alpha);
                            cursor_rounding = 1.5;
                        }
                        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, row_h));
                        // Fix 6d: a non-box cursor (bar/hbar) never
                        // recolors `cbg` above, so for those cursor types
                        // `is_cursor` alone used to force a same-colored
                        // `rect_filled` on top of the plain background --
                        // a wasted paint call every frame on the cursor's
                        // cell. Only paint when the color actually
                        // differs from the frame background, or the box
                        // cursor really did recolor this cell.
                        if cbg != bg || (is_cursor && cursor_type == "box") {
                            painter.rect_filled(snap_rect(rect, ppp), cursor_rounding, cbg);
                        }
                        match cell.style.underline {
                            Underline::None => {}
                            Underline::Straight => {
                                let uc = cell.style.underline_color.map(to_color).unwrap_or(cfg);
                                painter.line_segment(
                                    [
                                        Pos2::new(x, y + row_h - 1.0),
                                        Pos2::new(x + cell_w, y + row_h - 1.0),
                                    ],
                                    egui::Stroke::new(1.0_f32, uc),
                                );
                            }
                            Underline::Wave => {
                                let uc = cell.style.underline_color.map(to_color).unwrap_or(cfg);
                                let n = ((cell_w / 2.0).max(2.0)) as usize;
                                let step = cell_w / n as f32;
                                let b = y + row_h - 1.5;
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

                    // Pass 2: text, batched into per-style ASCII runs. A run
                    // breaks on style change, the cursor cell, or any
                    // non-ASCII char (whose glyph advance may differ from
                    // the cell width, so it is placed individually).
                    let mut run = String::new();
                    let mut run_start = 0usize;
                    let mut run_style: Option<core::redisplay::Style> = None;
                    let flush = |run: &mut String,
                                 run_start: usize,
                                 style: &Option<core::redisplay::Style>,
                                 painter: &egui::Painter| {
                        if run.trim().is_empty() {
                            run.clear();
                            return;
                        }
                        if let Some(st) = style {
                            let eff = st.or_default(&base);
                            let (mut cfg, cbg) = (
                                eff.fg.map(to_color).unwrap_or(fg),
                                eff.bg.map(to_color).unwrap_or(bg),
                            );
                            if eff.reverse {
                                cfg = cbg;
                            }
                            let x = origin.x + run_start as f32 * char_w;
                            let pos = Pos2::new(x, y);
                            let f = pick_font(&eff);
                            painter.text(pos, egui::Align2::LEFT_TOP, run.as_str(), f.clone(), cfg);
                            if eff.bold && !self.have_bold {
                                // Fake bold: re-draw with a fractional offset.
                                painter.text(
                                    Pos2::new(x + 0.4, y),
                                    egui::Align2::LEFT_TOP,
                                    run.as_str(),
                                    f.clone(),
                                    cfg,
                                );
                            }
                        }
                        run.clear();
                    };
                    for (col, cell) in line.iter().enumerate() {
                        if cell.continuation {
                            continue;
                        }
                        let is_cursor = grid.cursor == (row, col);
                        let cursor_boxed = is_cursor && cursor_type == "box";
                        let breaks_run = !cell.ch.is_ascii()
                            || cursor_boxed
                            || run_style.map(|s| s != cell.style).unwrap_or(false);
                        if breaks_run {
                            flush(&mut run, run_start, &run_style, painter);
                            run_style = None;
                        }
                        if !cell.ch.is_ascii() || cursor_boxed {
                            // Individually placed: exact cell position.
                            let x = origin.x + col as f32 * char_w;
                            let eff = cell.style.or_default(&base);
                            let (mut cfg, mut cbg) = (
                                eff.fg.map(to_color).unwrap_or(fg),
                                eff.bg.map(to_color).unwrap_or(bg),
                            );
                            if eff.reverse {
                                std::mem::swap(&mut cfg, &mut cbg);
                            }
                            if cursor_boxed {
                                // Same hard-switch glyph color + `cursor`-
                                // face override as pass 1's cell
                                // background (fix 3), applied here to the
                                // character's own color (this is "the
                                // character drawn under a box cursor" from
                                // task 6).
                                let (target_fg, target_bg) = if let Some(cf) = &cursor_face {
                                    (
                                        cf.fg.map(to_color).unwrap_or(cbg),
                                        cf.bg.map(to_color).unwrap_or(cfg),
                                    )
                                } else {
                                    let mut swapped_bg = cfg;
                                    if swapped_bg == bg {
                                        swapped_bg = fg;
                                    }
                                    (cbg, swapped_bg)
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
                                    // Task 8: the same fake-bold
                                    // double-draw the batched ASCII path
                                    // uses -- this per-glyph path (every
                                    // non-ASCII char, and the char under a
                                    // box cursor) previously had no bold
                                    // handling at all, so bold CJK text
                                    // and a bold character under the
                                    // cursor silently lost their emphasis
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
                            continue;
                        }
                        if run_style.is_none() {
                            run_style = Some(cell.style);
                            run_start = col;
                        }
                        run.push(cell.ch);
                    }
                    flush(&mut run, run_start, &run_style, painter);
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
                            let y = origin.y + (win.row + r) as f32 * row_h;
                            for col in cols {
                                let x = origin.x + (text_col0 + col) as f32 * char_w;
                                let rect =
                                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(1.0, row_h));
                                painter.rect_filled(snap_rect(rect, ppp), 0.0, guide_color);
                            }
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
                        let y = origin.y + row as f32 * row_h;
                        let rect = Rect::from_min_size(
                            Pos2::new(origin.x, y - 1.0),
                            Vec2::new(cols as f32 * char_w, 1.0),
                        );
                        painter.rect_filled(snap_rect(rect, ppp), 0.0, sep_color);
                    }
                    for win in &grid.windows {
                        let y = origin.y + win.mode_line_row as f32 * row_h;
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
                        let track_top = origin.y + layout.row as f32 * row_h;
                        let track_len = visible_lines as f32 * row_h;
                        if let Some((thumb_top, thumb_len)) =
                            scrollbar_thumb(total_lines, visible_lines, top_line0, track_len, 24.0)
                        {
                            let sb_color = face_style(&self.interp, &ed_ref, "scroll-bar")
                                .and_then(|f| f.fg.or(f.bg))
                                .map(to_color)
                                .unwrap_or_else(|| with_alpha(fg, 0.20));
                            let text_right = origin.x + (layout.col + layout.cols) as f32 * char_w;
                            let bar_x = text_right - 2.0 - 6.0;
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
                    let y = origin.y + crow as f32 * row_h;
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
                                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, row_h)),
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
                                        Pos2::new(x, y + row_h - 2.0),
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
                    let py = origin.y + (crow + 1) as f32 * row_h + 2.0;
                    egui::Area::new(egui::Id::new("hover-popup"))
                        .fixed_pos(Pos2::new(px, py))
                        .order(egui::Order::Foreground)
                        .show(ctx, |ui| {
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
    }
}

fn to_color(c: core::redisplay::Color) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
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
}
