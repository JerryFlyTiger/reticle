#!/usr/bin/env python3
"""Generate the Reticle app icon from its parametric description.

The icon is not a traced drawing.  The letter is a centreline -- a chain of
cubic Beziers -- plus a half-width profile sampled along it; the renderer
offsets the centreline by that width to both sides and closes the two offsets
into one filled outline.  Weight, lean and proportion are therefore numbers in
this file, not control points somebody has to push around in an editor.

Three things here look arbitrary and are not:

* The ribbon runs back DOWN the stem after the bowl closes, before leaving as
  the leg.  Writing an R in one stroke otherwise needs a 180 degree hairpin at
  that junction, and offsetting a centreline through a hairpin folds the
  outline over itself.  The return is invisible because it lies on the stem.

* Because the ribbon overlaps itself there, the lit and shaded edges cannot be
  drawn by stroking the letter path -- stroking traces the internal crossings
  as well, and the letter grows creases.  They are cut out of the silhouette
  instead (shape minus the same shape shifted), which self-intersection cannot
  reach.

* The ride-down anchors are the stem cubic evaluated at those heights.  A
  couple of units off and the ribbon grows a visible step on one edge.

Colours: the lit corner is sampled from the GNU Emacs icon that ships in
etc/images/icons/hicolor/scalable/apps/emacs.svg, and the shadow side is taken
well past it, because a rim light reads as a rim light only against something
dark.

Requires rsvg-convert (librsvg) for rasterising, and on macOS iconutil for the
.icns.  Run from the repository root:  python3 dev/gen-icon.py
"""

import json
import math
import os
import shutil
import subprocess
import sys

SIZE = 1024
CX = CY = SIZE / 2
TILE_R = 430.0                      # leaves room in the canvas for the shadow

LIT, MID, DEEP = "#8380c4", "#8f44bd", "#330f57"
RIM = "#e6cbff"
EDGE = 12.5                         # how far in the lit/shaded edge bands cut

BOX_W, BOX_H, BOX_DY = 445.0, 572.0, 4.0
SLANT = 10.0

# One ribbon: up the stem from the foot, over the arch at the head, round the
# bowl, back down the stem, out as the leg, and a tail that stops rather than
# flourishes.
SPINE = [
    (17, 95), (18, 72), (22, 44), (26, 12),
              (27, 1),  (44, -1), (62, 2),
              (82, 5),  (89, 18), (87, 32),
              (85, 46), (52, 51), (22, 48),
              (20.4, 53), (20.1, 58), (20, 62),
              (33, 70), (55, 82), (84, 97),
              (89, 99), (93, 97), (94, 91),
]
PROFILE = [(0.0, 4.5), (0.09, 8.6), (0.34, 9.4), (0.52, 9.0), (0.62, 8.4),
           (0.76, 9.2), (0.93, 7.2), (1.0, 6.0)]


# --------------------------------------------------------------- geometry

def bez(p0, p1, p2, p3, t):
    u = 1 - t
    return (u * u * u * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t * t * t * p3[0],
            u * u * u * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t * t * t * p3[1])


def sample_chain(segs, n_per):
    pts = []
    for i, s in enumerate(segs):
        for k in range(0 if i == 0 else 1, n_per + 1):
            pts.append(bez(*s, k / n_per))
    return pts


def normalised_arclength(pts):
    acc = [0.0]
    for a, b in zip(pts, pts[1:]):
        acc.append(acc[-1] + math.hypot(b[0] - a[0], b[1] - a[1]))
    total = acc[-1] or 1.0
    return [v / total for v in acc]


def width_at(profile, t):
    for (t0, w0), (t1, w1) in zip(profile, profile[1:]):
        if t0 <= t <= t1:
            u = 0.0 if t1 == t0 else (t - t0) / (t1 - t0)
            u = u * u * (3 - 2 * u)         # smoothstep: no kinks in the width
            return w0 + (w1 - w0) * u
    return profile[-1][1]


def normals(pts):
    out = []
    for i, _ in enumerate(pts):
        a, b = pts[max(i - 1, 0)], pts[min(i + 1, len(pts) - 1)]
        dx, dy = b[0] - a[0], b[1] - a[1]
        d = math.hypot(dx, dy) or 1.0
        out.append((-dy / d, dx / d))
    return out


def offset_outline(segs, profile, n_per=150):
    pts = sample_chain(segs, n_per)
    ts = normalised_arclength(pts)
    ns = normals(pts)
    left, right = [], []
    for p, n, t in zip(pts, ns, ts):
        w = width_at(profile, t)
        left.append((p[0] + n[0] * w, p[1] + n[1] * w))
        right.append((p[0] - n[0] * w, p[1] - n[1] * w))
    return left + right[::-1]


def catmull_path(pts, step, closed=True):
    """Decimate, then run Catmull-Rom through the survivors as cubics."""
    q = pts[::step]
    if closed and q[0] != q[-1]:
        q.append(q[0])
    n = len(q)

    def at(i):
        return q[i % (n - 1)] if closed else q[min(max(i, 0), n - 1)]

    d = ["M %.2f %.2f" % q[0]]
    for i in range(n - 1):
        p0, p1, p2, p3 = at(i - 1), at(i), at(i + 1), at(i + 2)
        b1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        b2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        d.append("C %.2f %.2f %.2f %.2f %.2f %.2f" % (b1 + b2 + p2))
    if closed:
        d.append("Z")
    return " ".join(d)


def superellipse(n, samples=240):
    pts = []
    for i in range(samples):
        th = 2 * math.pi * i / samples
        c, s = math.cos(th), math.sin(th)
        pts.append((CX + TILE_R * math.copysign(abs(c) ** (2.0 / n), c),
                    CY + TILE_R * math.copysign(abs(s) ** (2.0 / n), s)))
    return pts


def letter_path():
    k = math.tan(math.radians(SLANT))
    bx, by = CX - BOX_W / 2, CY - BOX_H / 2 + BOX_DY

    def L(p):
        x, y = bx + p[0] / 100.0 * BOX_W, by + p[1] / 100.0 * BOX_H
        return (x + k * (CY - y), y)

    segs = [tuple(L(q) for q in SPINE[i:i + 4]) for i in range(0, len(SPINE) - 3, 3)]
    prof = [(t, w / 100.0 * BOX_W) for t, w in PROFILE]
    return catmull_path(offset_outline(segs, prof), step=9)


# ------------------------------------------------------------------- svg

def lit_svg(letter, tile):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" height="{SIZE}">
  <title>Reticle</title>
  <defs>
    <linearGradient id="base" x1="0.08" y1="0" x2="0.92" y2="1">
      <stop offset="0" stop-color="#8a85d0"/>
      <stop offset="0.30" stop-color="{LIT}"/>
      <stop offset="0.62" stop-color="{MID}"/>
      <stop offset="0.84" stop-color="#63239f"/>
      <stop offset="1" stop-color="{DEEP}"/>
    </linearGradient>
    <radialGradient id="amb" cx="0.80" cy="0.04" r="0.95">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.34"/>
      <stop offset="0.40" stop-color="#ffffff" stop-opacity="0.02"/>
      <stop offset="1" stop-color="#120423" stop-opacity="0.60"/>
    </radialGradient>
    <linearGradient id="rim" x1="0.78" y1="0" x2="0.22" y2="1">
      <stop offset="0" stop-color="{RIM}" stop-opacity="0.95"/>
      <stop offset="0.26" stop-color="#c793f5" stop-opacity="0.55"/>
      <stop offset="0.60" stop-color="#a06adb" stop-opacity="0.10"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </linearGradient>
    <linearGradient id="face" x1="0.12" y1="0" x2="0.88" y2="1">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.44" stop-color="#f7f1ff"/>
      <stop offset="1" stop-color="#dbcaf0"/>
    </linearGradient>
    <linearGradient id="litpaint" x1="0.1" y1="0" x2="0.85" y2="1">
      <stop offset="0" stop-color="#ffffff" stop-opacity="1"/>
      <stop offset="0.45" stop-color="#ffffff" stop-opacity="0.46"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0.05"/>
    </linearGradient>
    <linearGradient id="shadepaint" x1="0.9" y1="1" x2="0.2" y2="0.05">
      <stop offset="0" stop-color="#43197c" stop-opacity="0.74"/>
      <stop offset="0.5" stop-color="#43197c" stop-opacity="0.28"/>
      <stop offset="1" stop-color="#43197c" stop-opacity="0"/>
    </linearGradient>
    <radialGradient id="sheen" cx="0.5" cy="0.5" r="0.5">
      <stop offset="0" stop-color="#ffffff" stop-opacity="0.82"/>
      <stop offset="1" stop-color="#ffffff" stop-opacity="0"/>
    </radialGradient>

    <path id="tile" d="{tile}"/>
    <path id="ltr" d="{letter}"/>
    <clipPath id="tileclip"><use href="#tile"/></clipPath>

    <filter id="cast" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="26"/>
    </filter>
    <filter id="ltrcast" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="15"/>
    </filter>
    <filter id="soft" x="-30%" y="-30%" width="160%" height="160%">
      <feGaussianBlur stdDeviation="24"/>
    </filter>
    <filter id="band" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="4.5"/>
    </filter>

    <mask id="m-face" maskUnits="userSpaceOnUse" x="0" y="0" width="{SIZE}" height="{SIZE}">
      <use href="#ltr" fill="#ffffff"/>
    </mask>
    <mask id="m-lit" maskUnits="userSpaceOnUse" x="0" y="0" width="{SIZE}" height="{SIZE}">
      <g filter="url(#band)">
        <use href="#ltr" fill="#ffffff"/>
        <use href="#ltr" fill="#000000" transform="translate({EDGE} {EDGE})"/>
      </g>
    </mask>
    <mask id="m-shade" maskUnits="userSpaceOnUse" x="0" y="0" width="{SIZE}" height="{SIZE}">
      <g filter="url(#band)">
        <use href="#ltr" fill="#ffffff"/>
        <use href="#ltr" fill="#000000" transform="translate({-EDGE} {-EDGE})"/>
      </g>
    </mask>
  </defs>

  <use href="#tile" fill="#1b0632" opacity="0.60" filter="url(#cast)" transform="translate(0 28)"/>
  <use href="#tile" fill="url(#base)"/>
  <use href="#tile" fill="url(#amb)"/>

  <g clip-path="url(#tileclip)">
    <use href="#tile" fill="none" stroke="url(#rim)" stroke-width="62" opacity="0.60" filter="url(#soft)"/>
    <use href="#tile" fill="none" stroke="url(#rim)" stroke-width="15"/>

    <use href="#ltr" fill="#1e0640" opacity="0.66" transform="translate(10 24)" filter="url(#ltrcast)"/>
    <use href="#ltr" fill="#7448b9" transform="translate(5 11)"/>
    <use href="#ltr" fill="url(#face)"/>

    <rect width="{SIZE}" height="{SIZE}" fill="url(#shadepaint)" mask="url(#m-shade)"/>
    <rect width="{SIZE}" height="{SIZE}" fill="url(#litpaint)" mask="url(#m-lit)"/>
    <g mask="url(#m-face)">
      <ellipse cx="368" cy="330" rx="270" ry="150" fill="url(#sheen)" opacity="0.52"
               transform="rotate(-30 368 330)"/>
    </g>
  </g>
</svg>
'''


def flat_svg(letter, tile):
    """Solid ink on a plain tile.  The lit version turns to fog below about
    48 pixels, and .icns and .ico both let each size carry its own image."""
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" height="{SIZE}">
  <title>Reticle (small sizes)</title>
  <defs>
    <linearGradient id="flat" x1="0.08" y1="0" x2="0.92" y2="1">
      <stop offset="0" stop-color="{LIT}"/>
      <stop offset="0.62" stop-color="{MID}"/>
      <stop offset="1" stop-color="#5b2091"/>
    </linearGradient>
  </defs>
  <path d="{tile}" fill="url(#flat)"/>
  <path d="{letter}" fill="#ffffff"/>
</svg>
'''


def letter_glyph_svg(letter):
    """The macOS 26 .icon layer image: the letter alone, filled white, on a
    plain 1024x1024 canvas -- no tile, no gradients, no filters. The .icon
    package supplies the gradient and the glass material itself (see
    build_icon_json below); this SVG is only the glyph layer it composites
    on top."""
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" height="{SIZE}">
  <path d="{letter}" fill="#ffffff"/>
</svg>
'''


def hex_to_srgb_stop(hexcolor):
    """'#8380c4' -> 'srgb:0.51373,0.50196,0.76863,1.00000', the decimal
    encoding icon.json wants for a gradient stop. Kept as a function (not a
    second pair of hard-coded decimals) so the .icon package's gradient and
    the SVG renderers' LIT/MID gradient can never drift out of sync with
    each other."""
    r = int(hexcolor[1:3], 16) / 255.0
    g = int(hexcolor[3:5], 16) / 255.0
    b = int(hexcolor[5:7], 16) / 255.0
    return "srgb:%.5f,%.5f,%.5f,1.00000" % (r, g, b)


def build_icon_json():
    """The macOS 26 .icon package's source description: a two-stop linear
    gradient background (LIT -> MID, the same two constants the lit/flat SVG
    renderers use) with the letter glyph layered on top, letting the OS
    render the glass material itself."""
    return {
        "color-space-for-untagged-svg-colors": "display-p3",
        "fill": {
            "linear-gradient": [
                hex_to_srgb_stop(LIT),
                hex_to_srgb_stop(MID),
            ]
        },
        "groups": [
            {
                "layers": [
                    {
                        "fill": "automatic",
                        "image-name": "R.svg",
                        "name": "R",
                        "position": {"scale": 1.0, "translation-in-points": [0, 0]},
                    }
                ],
                "shadow": {"kind": "neutral", "opacity": 0.5},
                "translucency": {"enabled": True, "value": 0.5},
            }
        ],
        "supported-platforms": {"squares": ["macOS"]},
    }


# ----------------------------------------------------------------- output

# Below 48 pixels the lit rendering stops being legible; above it the flat
# one looks cheap.  LIT_SIZES, FLAT_SIZES and ICONSET each hardcode this
# split at the 48px boundary rather than deriving it from a shared constant
# -- there is nothing left here to tune, only a fact to state.
#
# 1024 is deliberately absent from LIT_SIZES: nothing in the tree consumes
# assets/icon/png/reticle-1024.png (the iconset's 1024px slot is rendered
# straight from reticle.svg below, and reticle.svg is the master anyway), so
# committing it would only add ~1MB of non-diffable weight for no consumer.
LIT_SIZES = [512, 256, 128, 64]
FLAT_SIZES = [32, 16]

# name -> (source, pixels).  macOS wants @2x variants of the small slots too,
# and at 32 physical pixels the flat drawing still wins.
ICONSET = [
    ("icon_16x16.png", "flat", 16),
    ("icon_16x16@2x.png", "flat", 32),
    ("icon_32x32.png", "flat", 32),
    ("icon_32x32@2x.png", "lit", 64),
    ("icon_128x128.png", "lit", 128),
    ("icon_128x128@2x.png", "lit", 256),
    ("icon_256x256.png", "lit", 256),
    ("icon_256x256@2x.png", "lit", 512),
    ("icon_512x512.png", "lit", 512),
    ("icon_512x512@2x.png", "lit", 1024),
]


def render(svg_path, png_path, px):
    try:
        subprocess.run(["rsvg-convert", "-w", str(px), "-h", str(px),
                        svg_path, "-o", png_path], check=True)
    except (subprocess.CalledProcessError, OSError) as exc:
        # Name the file it died on -- a bare CalledProcessError only names
        # the rsvg-convert argv, which is the same for every size and tells
        # you nothing about which of the dozen or so renders failed.
        sys.exit("rsvg-convert failed rendering %s: %s" % (png_path, exc))


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(root, "assets", "icon")
    png_dir = os.path.join(out, "png")
    os.makedirs(png_dir, exist_ok=True)

    if shutil.which("rsvg-convert") is None:
        sys.exit("rsvg-convert not found: brew install librsvg")

    try:
        from PIL import Image
    except ImportError:
        Image = None

    letter = letter_path()
    tile = catmull_path(superellipse(5.0), step=8)

    lit_path = os.path.join(out, "reticle.svg")
    flat_path = os.path.join(out, "reticle-flat.svg")
    with open(lit_path, "w") as fh:
        fh.write(lit_svg(letter, tile))
    with open(flat_path, "w") as fh:
        fh.write(flat_svg(letter, tile))
    print("wrote", lit_path)
    print("wrote", flat_path)

    # macOS 26's .icon package source: icon.json (the gradient, derived from
    # LIT/MID so it cannot drift from the SVG renderers above) plus the
    # letter-alone glyph layer. This is source, not a build artifact -- the
    # compiled Assets.car is dev/make-app-bundle.sh's job, not this
    # script's, and is never written here.
    iconpkg_dir = os.path.join(out, "Reticle.icon")
    iconpkg_assets_dir = os.path.join(iconpkg_dir, "Assets")
    os.makedirs(iconpkg_assets_dir, exist_ok=True)
    icon_json_path = os.path.join(iconpkg_dir, "icon.json")
    r_svg_path = os.path.join(iconpkg_assets_dir, "R.svg")
    with open(icon_json_path, "w") as fh:
        json.dump(build_icon_json(), fh, indent=2)
        fh.write("\n")
    with open(r_svg_path, "w") as fh:
        fh.write(letter_glyph_svg(letter))
    print("wrote", icon_json_path)
    print("wrote", r_svg_path)

    def render_and_optimize(svg_path, png_path, px):
        render(svg_path, png_path, px)
        if Image is not None:
            # Re-save through Pillow with optimize=True to shrink the
            # committed blob; this recompresses only -- it decodes and
            # re-encodes the exact pixels rsvg-convert produced, it does not
            # touch a single one of them.
            img = Image.open(png_path)
            img.load()
            img.save(png_path, format="PNG", optimize=True)

    for px in LIT_SIZES:
        render_and_optimize(lit_path, os.path.join(png_dir, "reticle-%d.png" % px), px)
    for px in FLAT_SIZES:
        render_and_optimize(flat_path, os.path.join(png_dir, "reticle-%d.png" % px), px)
    print("wrote %d pngs into %s" % (len(LIT_SIZES) + len(FLAT_SIZES), png_dir))
    if Image is None:
        print("Pillow not installed: emitted pngs left unoptimized")

    iconset = os.path.join(out, "reticle.iconset")
    # ignore_errors swallows a failed removal, so a stale directory that
    # could not be deleted would otherwise blow up here -- before the
    # try/finally below is even entered, where nothing can clean up.
    shutil.rmtree(iconset, ignore_errors=True)
    os.makedirs(iconset, exist_ok=True)
    filled = False
    try:
        for name, src, px in ICONSET:
            render(lit_path if src == "lit" else flat_path,
                   os.path.join(iconset, name), px)
        filled = True
    finally:
        # A render() failure mid-loop leaves iconset half-populated, which
        # iconutil could not turn into a usable .icns and a later run could
        # not safely resume from either -- clean it up rather than stranding
        # it. This only fires on the failure path: the two branches below
        # handle cleanup for the two ways a *complete* fill can end.
        if not filled:
            shutil.rmtree(iconset, ignore_errors=True)

    if shutil.which("iconutil"):
        icns = os.path.join(out, "reticle.icns")
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", icns], check=True)
        print("wrote", icns)
        # Only delete the iconset once it has actually been consumed into an
        # .icns. The else branch below is the one place this must NOT run --
        # deleting it there would falsify the "left ... for a later run"
        # message on the very next line.
        shutil.rmtree(iconset, ignore_errors=True)
    else:
        print("iconutil not found (not macOS): left", iconset, "for a later run")

    if Image is None:
        print("Pillow not installed: skipped reticle.ico")
        return
    frames = [Image.open(os.path.join(png_dir, "reticle-%d.png" % px)).convert("RGBA")
              for px in (256, 128, 64, 32, 16)]
    ico = os.path.join(out, "reticle.ico")
    frames[0].save(ico, format="ICO",
                   sizes=[(f.width, f.height) for f in frames], append_images=frames[1:])
    print("wrote", ico)


if __name__ == "__main__":
    main()
