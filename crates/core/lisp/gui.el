;;; gui.el --- egui frontend settings -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
;; Copyright 2026 Jerry Chen
;;
;; Reticle is source-available software, licensed under the Functional
;; Source License 1.1 with an Apache 2.0 future grant. It is not open source.
;; See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
;; of the dependencies it links against.

;; M105: gives every `gui-*' variable the GUI frontend
;; (`crates/frontend-gui/src/lib.rs') already reads by name a `defvar' with
;; a docstring, and adds `gui-font'/`set-font' -- the first of those
;; variables a user can change without knowing the underlying file name.
;; Before this file, all of them lived only as string literals passed to
;; `int_var'/`str_var'/`sym_var'/`var_truthy' on the Rust side; a user
;; wanting to change padding or line spacing had no way to discover the
;; variable existed short of reading `lib.rs' itself.
;;
;; This file is loaded once, at startup, alongside every other `.el' file
;; in this list (`crates/core/src/lib.rs'); nothing here needs anything
;; else on it (no other file references any `gui-*' symbol).

(defvar gui-font 'jetbrains-mono
  "Which of the three named GUI body fonts to use: `jetbrains-mono' (the
default), `fira-code', or `sf-mono'. Read once per frame by the GUI
frontend (`crates/frontend-gui/src/lib.rs', M105) -- changing it with
`setq' takes effect on the very next frame, no restart needed, though
`ctx.set_fonts' (the underlying font-atlas rebuild) is comparatively
expensive, which is why the frontend only reruns it when this value (or
`gui-font-family') actually changed since the last frame rather than on
every frame.

`jetbrains-mono' and `fira-code' are bundled inside the binary
(`assets/fonts/', SIL Open Font License 1.1) and always available.
`sf-mono' is Apple's own font: never bundled (its license does not
permit that), only used if already installed on the running machine --
selecting it when it isn't installed falls back through the SAME SET of
built-in candidates `gui-font-family' unset would try (JetBrains Mono,
Fira Code, SF Mono, Menlo, Monaco), just in a DIFFERENT ORDER: `sf-mono'
puts `SFNSMono.ttf' first in that search (see `font_search_order' in
`lib.rs'), while an unset/default `gui-font' puts `JetBrainsMono-Regular.ttf'
first -- so the fallback that actually gets used when SF Mono is missing
is JetBrains Mono either way, but by falling through the front of the
list rather than being the list's first entry.

Prefer `set-font' (`M-x set-font') over `setq'-ing this directly: it
checks `sf-mono' availability first and leaves this variable alone
(with a `message') when the pick can't actually be honored, instead of
silently taking effect only in the fallback chain.

`gui-font-family' (below), when set, takes priority over this variable
entirely -- it names an arbitrary font FILE, for a user who wants some
font other than the three offered here.

Bold/italic: `jetbrains-mono' has its own embedded true bold and italic
faces. `fira-code' ships no italic upstream at all (only Bold/Light/
Medium/Regular/Retina/SemiBold).

KNOWN GAP, measured then FIXED (M105 measured it, M106 found and fixed the
cause): selecting `fira-code' used to render italic text (e.g. comments)
UPRIGHT on screen, not slanted -- confirmed by a side-by-side screenshot,
cropped to the comment line and scaled 3x for comparison. Font-definition
REGISTRATION was already correct: calling `build_font_definitions' for
`fira-code' shows `have_italic' is true and the `"mono-italic"' family is
registered as `["menlo-italic", "arial-unicode"]' (Menlo's real italic
face, index 2 of Menlo.ttc). The bug was in RASTERIZING that face:
`crates/frontend-gui/src/shaping.rs''s `ShapingFace::new' built its
`ab_glyph::FontArc' with `try_from_vec', which always parses face 0 of a
font collection and silently ignores the face index -- so shaping
correctly used Menlo.ttc's italic face (index 2) to pick glyph ids, but
rasterizing always drew those glyph ids against face 0 (regular), which
is why the result came out upright. The same bug also made `sf-mono''s
bold (Menlo.ttc index 1) rasterize at regular weight. Fixed by building
the rasterizing font with `ab_glyph::FontVec::try_from_vec_and_index',
which does take the face index, so shaping and rasterizing now agree on
which face of the collection they use. Verified on screen after the fix
(`dev/gui-shot.sh' against a freshly rebuilt binary): a before/after pixel
diff of the comment line shows 16,112 changed pixels for `fira-code' and
21,491 for `sf-mono', both now visibly slanted -- only these two fonts'
italic/bold-via-Menlo.ttc paths were actually looked at on screen, not
every font this crate can load. See the `have_italic' field doc in
`lib.rs' and `shaping.rs''s `ShapingFace' doc for the same note.")

(defvar gui-font-family nil
  "An arbitrary font file name (e.g. \"Menlo.ttc\"), searched for under
`crates/frontend-gui/src/lib.rs''s `font_dirs' (macOS-only paths as of
M105 -- `~/Library/Fonts', `/Library/Fonts', `/System/Library/Fonts',
`/System/Library/Fonts/Supplemental'). `nil' by default.

When set, this takes priority over `gui-font' outright: it is tried
FIRST in the fallback chain `install_fonts' builds, ahead of whichever
of `jetbrains-mono'/`fira-code'/`sf-mono' `gui-font' names. This is the
escape hatch for a font `gui-font'/`set-font' don't offer a name for --
`gui-font' only ever offers the three bundled/system choices, this
variable can point at literally any font file, at the cost of typing
its exact file name yourself (no `completing-read' here, unlike
`set-font').

Read once per frame, same as `gui-font' -- see that variable's
docstring for the runtime-switch/atlas-rebuild cost note, which applies
identically here.")

(defvar gui-font-size 16
  "The GUI body font's point size, clamped to 8..=72 by
`clamp_font_size' in `crates/frontend-gui/src/lib.rs'. Read once per
frame; changing it with `setq' is reflected on the very next frame
(cols/rows, cursor rect, gutter and CJK double-width cells are all
re-measured every frame from the live font metrics, so nothing else
needs to change in step with this).")

(defvar gui-padding-x 10
  "Horizontal padding, in device-independent points, between the GUI
window's edge and the character grid, clamped to 0..=64 by
`clamp_padding'. Read once per frame; removed from the available area
BEFORE columns are derived from it, so the last column never falls
inside this margin.")

(defvar gui-padding-y 6
  "Vertical padding, in device-independent points, between the GUI
window's edge and the character grid, clamped to 0..=64 by
`clamp_padding'. Same per-frame timing and column/row interaction as
`gui-padding-x', but for rows.")

(defvar gui-line-spacing 100
  "Line spacing as a percentage of the font's natural row height (100 =
unchanged), clamped to 80..=200 by `clamp_line_spacing'. Read once per
frame; multiplies `row_height' before rows are derived from it, so
changing this also changes how many rows fit in the window.")

(defvar gui-indent-guide-step nil
  "How many columns apart the GUI's indent guides are drawn, or `nil' to
fall back to `tab-width' (and then to 4 if that is also unset). Unlike
every other `gui-*' variable in this file, this one intentionally has
no numeric default of its own -- `tab-width' is already the
language-appropriate source of truth for indentation width, and a
separate always-set default here would silently stop tracking it the
moment a mode sets `tab-width' buffer-locally. Read once per frame.")

(defvar gui-cursor-blinks 10
  "How many blink/fade cycles the GUI cursor animates through after a
period of no input before settling fully solid, clamped to 0..=100 by
`clamp_cursor_blinks' (0 means never stop animating; mirrors GNU's
`blink-cursor-blinks'). Read once per frame, but only consulted while
the cursor is actually idle-animating -- typing resets the idle timer
before this is checked again.")

(defvar gui-opacity 100
  "Window opacity as a percentage, clamped to 20..=100 by `clamp_opacity'
in `crates/frontend-gui/src/lib.rs' (M111). 100 (the default) is fully
opaque and looks identical to every screenshot taken before this
variable existed -- nothing changes for a user who never sets it. Read
once per frame and applied only to backgrounds, never to foreground
text (foreground always stays fully opaque, the same choice iTerm2/VS
Code/JetBrains all make, since translucent text over a translucent
background is unreadable). The floor is 20, not 0 or something close to
it: there is no in-app opacity picker and no keybinding to raise this
back once it's too low, so a value that makes the window effectively
invisible would be recoverable only by editing the init file by hand --
20 keeps the window unambiguously visible as a dim outlined rectangle no
matter what's behind it.

A cell with no `:background' of its own (plain text, the gutter, blank
rows) ends up at exactly this percentage -- it is painted once, as part
of the window's own fill. A cell that DOES set its own `:background'
(the mode line, `hl-line', a selection, `panel-selected') is painted a
SECOND time on top of that fill, so it composites more solidly than the
nominal percentage (two `p'-alpha layers compose to `1 - (1 - p)^2', not
`p' -- e.g. 40% measures out to about 64%). This is intentional, not a
bug: a solid-looking status bar and a legible selection read better than
literally honoring the same percentage everywhere, the way a selection
in a translucent terminal is usually more opaque than the surrounding
text area.

The LSP hover popup is a deliberate exception: it stays fully opaque
regardless of this variable, because a translucent popup floating over
translucent text is unreadable, same reasoning VS Code/JetBrains apply
to their own popups.")

(defvar gui-ligatures t
  "Non-nil (the default) shows coding ligatures (`=>', `!=', `->', ...)
in fonts that define a `calt' OpenType feature -- read every frame by
`crates/frontend-gui/src/lib.rs' (M116), same shape as every other
`gui-*' variable in this file. No clamp: any non-nil value means on,
same as `rainbow-delimiters-mode' and every other boolean toggle in
this codebase.

Unlike a numeric `gui-*' variable, flipping this one changes which
OpenType FEATURE `crates/frontend-gui/src/shaping.rs''s
`shape_calt_only' requests from the shaper, not just a value fed into a
layout formula -- so the risk is a stale cache still drawing ligatures
one frame after this is set to nil (or the reverse). That risk is
closed by construction, not by remembering to clear anything: this
value is threaded straight into `ShapeCache''s key
(`(FaceRole, text, enable_calt)'), so a toggle lands on a different key
than whatever was cached before, and the old entries are simply never
looked up again rather than needing an explicit reset the way a font
switch does (`apply_font_switch').

Turning this off does not fall back to drawing every character through
egui's own unshaped text path -- shaping still runs (with `calt'
requested off, `liga'/`dlig'/`clig' were already always off, see
`shape_calt_only''s doc), so ordinary per-character glyph placement is
unaffected; only ligature SUBSTITUTION stops happening.")

(defvar gui-debug-overlay nil
  "When non-nil, the GUI paints a small per-frame render-time readout
(e.g. \"2.34 ms\") in the corner of the window -- a manual, opt-in
profiling aid, never enabled by default. Read once per frame; toggling
it with `setq' takes effect on the next frame with no other side
effects on layout (the label is drawn on top of the grid, not counted
in it).")

(defvar gui--font-choices '("jetbrains-mono" "fira-code" "sf-mono")
  "The names `set-font' offers via `completing-read', as strings --
`gui-font''s value is the interned symbol of whichever one is chosen.
Kept as its own list (rather than inlined into `set-font') so a reader
looking for \"what can `gui-font' be\" finds one place, matching this
file's `gui-font' docstring.")

(defun gui--font-dirs ()
  "The directories `crates/frontend-gui/src/lib.rs''s `font_dirs'
searches for `SFNSMono.ttf' (and for `gui-font-family', and for
Menlo.ttc/Monaco.ttf). Duplicated here by hand rather than plumbed over
from Rust, because nothing in this codebase has a channel for the GUI
frontend to publish data INTO elisp before the very first frame paints
-- `set-font' needs this list before then, to answer `M-x set-font's
\"is `sf-mono' actually available\" question at command-invocation time,
which is well before any frame has run `install_fonts'.

RISK (M105 fix round, recorded honestly rather than fixed): this list and
`font_dirs' in `lib.rs' are two independent copies of the same four
paths, kept in sync BY HAND. Nothing in this codebase checks that they
agree -- if a future change adds or reorders a directory in one without
updating the other, `set-font' could report `sf-mono' unavailable when
`install_fonts' would actually have found it (or the reverse), and
neither side would fail loudly. There is no automated guard against
this drift as of M105."
  (list (expand-file-name "Library/Fonts" "~")
        "/Library/Fonts"
        "/System/Library/Fonts"
        "/System/Library/Fonts/Supplemental"))

(defun gui--sf-mono-available-p ()
  "Whether `SFNSMono.ttf' exists in any of `gui--font-dirs' -- the same
file `install_fonts' searches for when `gui-font' is `sf-mono'. Used by
`set-font' to warn instead of silently picking a font that will just
fall through to the built-in fallback chain."
  (let ((found nil))
    (dolist (dir (gui--font-dirs))
      (when (file-exists-p (concat (directory-file-name dir) "/SFNSMono.ttf"))
        (setq found t)))
    found))

(defun set-font ()
  "Pick a new value for `gui-font', via `completing-read' over
`gui--font-choices'. Choosing `sf-mono' when `gui--sf-mono-available-p'
reports it isn't installed leaves `gui-font' UNCHANGED and just
`message's why -- setting it anyway would silently render with
whatever `install_fonts' falls back to, which is not what was asked
for and would be confusing to discover only by looking at the result."
  (interactive)
  (with-completing-read
   (choice "Font: " gui--font-choices t)
   (if (and (string= choice "sf-mono") (not (gui--sf-mono-available-p)))
       (message "set-font: SF Mono not found on this machine, keeping %s" gui-font)
     (setq gui-font (intern choice))
     (message "Font: %s" choice))))
