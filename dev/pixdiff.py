"""Summarize consecutive-frame pixel differences for dev/gui-drive.sh.

M114: extracted out of dev/gui-drive.sh's inline `python3 -c '...'` heredoc
because a heredoc cannot be tested, and this module shipped four real bugs
that only a cold reviewer caught (see dev/test_pixdiff.py, which pins all
four down by name):

1. Comparing frames after `.convert("RGB")` made a window-transparency
   change -- which moves only the alpha channel -- invisible by
   construction: 0 differing pixels reported for a real 100%->40% opacity
   switch (M111).
2. After (1) was fixed, the headline count still went through
   `d.convert("L")`. Converting RGBA to L applies luma weights to R/G/B and
   DROPS alpha, so an opacity-only change again collapsed to 0 in the
   headline, even though the per-frame alpha-histogram line (added for bug
   1) told the truth right next to it. Folding the four channels with
   `ImageChops.lighter` instead keeps alpha in the count.
3. `d.getbbox()` defaults to `alpha_only=True` for images that carry an
   alpha channel, computing the box from the ALPHA band alone -- an
   RGB-only change with unchanged alpha silently reported `bbox=None`.
   Fixed by passing `alpha_only=False`.
4. `rgb_px` used `.convert("L")` to fold R/G/B, which applies the same
   lossy luma weighting as bug 2 to the *difference image* -- a
   single-channel delta of 1 (well within antialiasing/subpixel-jitter
   range between two renders of the same content) rounds to L=0 and
   disappears, reporting `rgb_px=0` for a pixel that genuinely changed
   colour. Fixed the same way as `diff_px`: fold the three colour bands
   with `ImageChops.lighter`, which preserves the max raw per-channel
   delta and cannot be fooled by rounding.

There is also a fifth, non-historical rule implemented here on first
writing (see `summarize_pair`'s own comment): an RGB difference where BOTH
frames are fully transparent at that pixel is invisible padding, not a
real change, and is excluded from `rgb_px`/`diff_px`.

Usage as a library: `summarize_pair(prev_image, image)` and
`alpha_summary(image)`. Usage as a CLI: see `if __name__ == "__main__"`
below -- reads a newline-separated list of PNG paths from a file (same
`SHOTS_FILE` convention dev/gui-drive.sh already uses) and prints the same
report the old inline script did.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from typing import Optional


def pillow_available() -> bool:
    try:
        import PIL  # noqa: F401
    except ImportError:
        return False
    return True


@dataclass
class PairDiff:
    """Everything worth reporting about one consecutive pair of frames."""

    resized: bool
    prev_size: tuple
    size: tuple
    total_px: int = 0
    diff_px: int = 0
    rgb_px: int = 0
    alpha_px: int = 0
    bbox: Optional[tuple] = None


def uniform_frame_warning(im):
    """A frame that is essentially one colour is almost certainly not a
    picture of anything.

    The usual cause is that the display was asleep or the screen locked when
    `screencapture` ran: it exits 0 and hands back an all-black PNG, which is
    non-empty, so a caller checking only the file size accepts it. That
    happened during M116 and cost a round trip diagnosing a "permission"
    problem that was not one -- macOS reported the permission as granted the
    whole time (`CGPreflightScreenCaptureAccess` returned true), the screen was
    simply off.

    Returns a warning string, or None when the frame looks like a picture.
    """
    rgb = im.convert("RGB")
    total = rgb.size[0] * rgb.size[1]
    if total == 0:
        return "frame has zero pixels"
    hist = rgb.histogram()
    # Per-channel histograms. A single dominant colour shows up as one
    # near-total bucket in every channel; taking the minimum across the three
    # keeps a frame that is uniform in one channel only (a solid-red image,
    # say) from being flagged.
    dominant = min(max(hist[0:256]), max(hist[256:512]), max(hist[512:768]))
    share = dominant / float(total)
    if share >= 0.99:
        return (
            "frame is %.1f%% a single colour -- a sleeping display or a locked "
            "screen captures as a uniform frame, and `screencapture` reports "
            "success for it" % (100.0 * share)
        )
    return None


def alpha_summary(im) -> str:
    """The two most common alpha levels, as percentages of the frame,
    rounded to the nearest whole percent -- this is a coarse glance meant
    to make a translucency change visually obvious, not a precise
    histogram, so two distributions differing by under a percent can print
    identically; that is by design, not a bug to fix.

    Compared in RGBA, not RGB -- see module docstring, bug 1. Measured
    2026-09-05 during M111: a real 100%->40% opacity switch showed up here
    as 0 differing pixels until the RGB-only comparison was fixed.
    """
    h = im.getchannel("A").histogram()
    total = float(im.size[0] * im.size[1])
    top = sorted(((c, v) for v, c in enumerate(h) if c), reverse=True)[:2]
    return "  ".join("a=%d %.0f%%" % (v, 100.0 * c / total) for c, v in top)


def _fold_lighter(*channels):
    """OR-like fold of single-band `Image`s via per-pixel max: 0 where
    every band is 0 at that pixel, non-zero if any band is. The exact
    non-zero magnitude is not meaningful after folding -- every caller
    here only tests zero-vs-non-zero (via `.histogram()[1:]`, which sums
    counts for values 1-255 and ignores what the value actually is), so
    losing magnitude on the non-zero side costs nothing.

    Deferred `PIL` import (not module-level): this module must stay
    importable with Pillow absent -- `pillow_available()`, and the
    `__main__` skip check that calls it, would otherwise never get a
    chance to run before an ImportError.
    """
    from PIL import ImageChops

    merged = channels[0]
    for c in channels[1:]:
        merged = ImageChops.lighter(merged, c)
    return merged


def summarize_pair(prev_im, im) -> PairDiff:
    """Compare two already-opened, already-RGBA `PIL.Image` objects.

    Reports a pixel as differing if ANY of the four channels moved --
    `d.convert("L")` would be the obvious way to collapse the four-channel
    diff and is WRONG here, see module docstring, bug 2 (and bug 4, same
    mistake made again in `rgb_px`). Folding channels with
    `ImageChops.lighter` instead keeps every channel's real delta in the
    count regardless of luma weighting or rounding. `rgb_px` and
    `alpha_px` are also reported split apart, so a pure-alpha change
    cannot hide inside a colour-change number, and vice versa.

    One more rule, not from luma rounding: a colour difference where BOTH
    frames are fully transparent (alpha 0) at that pixel is invisible --
    RGB bytes under alpha=0 are arbitrary padding a renderer is free to
    leave dirty, not a real visual change -- so such pixels are excluded
    from `rgb_px` (and therefore from `diff_px`, which folds `rgb_px`'s
    masked result back in). The rule is OR, not AND: a pixel opaque in
    EITHER frame was actually seen at some point, so a colour change there
    counts even if the other frame is fully transparent. `alpha_px` itself
    is never masked this way -- a transparency change is always the real,
    visible thing being measured.
    """
    from PIL import ImageChops

    if prev_im.size != im.size:
        return PairDiff(resized=True, prev_size=prev_im.size, size=im.size)

    total = im.size[0] * im.size[1]
    d = ImageChops.difference(prev_im, im)

    prev_a = prev_im.getchannel("A")
    a = im.getchannel("A")
    alpha_diff = ImageChops.difference(prev_a, a)
    alpha_px = sum(alpha_diff.histogram()[1:])

    rgb_diff_folded = _fold_lighter(
        *ImageChops.difference(prev_im.convert("RGB"), im.convert("RGB")).split()
    )
    alpha_visible = _fold_lighter(prev_a, a).point(lambda p: 255 if p > 0 else 0)
    # `ImageChops.multiply` computes floor(a * b / 255): with `b` a 0/255
    # mask, this is exactly "keep `a` where visible, zero it out where
    # both frames are fully transparent" -- no per-pixel Python loop.
    rgb_masked = ImageChops.multiply(rgb_diff_folded, alpha_visible)
    rgb_px = sum(rgb_masked.histogram()[1:])

    diff_px = sum(_fold_lighter(rgb_masked, alpha_diff).histogram()[1:])

    return PairDiff(
        resized=False,
        prev_size=prev_im.size,
        size=im.size,
        total_px=total,
        diff_px=diff_px,
        rgb_px=rgb_px,
        alpha_px=alpha_px,
        # `alpha_only=True` is Pillow's default for images that carry an
        # alpha channel, and it computes the box from the ALPHA band
        # alone -- an RGB-only change with unchanged alpha (like this
        # function's own rgb_px case) would silently report `None` here.
        # This is bug 3 (module docstring) and is computed from the raw,
        # unmasked `d` deliberately: localizing WHERE bytes moved on disk
        # is a different question from whether the change was visible.
        bbox=d.getbbox(alpha_only=False),
    )


def format_pair_report(prev_name: str, name: str, pd: PairDiff) -> str:
    if pd.resized:
        return "%-14s -> %-14s window resized %s -> %s" % (
            prev_name,
            name,
            pd.prev_size,
            pd.size,
        )
    pct = 100.0 * pd.diff_px / pd.total_px if pd.total_px else 0.0
    return "%-14s -> %-14s %8d differing px (%5.2f%%)  rgb=%d alpha=%d  bbox=%s" % (
        prev_name,
        name,
        pd.diff_px,
        pct,
        pd.rgb_px,
        pd.alpha_px,
        pd.bbox,
    )


def run(paths) -> int:
    """Open PATHS in order, print the same report dev/gui-drive.sh used to
    print inline. Returns a process exit code: 1 if a frame fails to open
    (caught and reported with a clean message, not left to raise -- an
    improvement on the original inline script, which had no such
    handling at all), 0 otherwise."""
    from PIL import Image

    prev = None
    for p in paths:
        try:
            im = Image.open(p).convert("RGBA")
        except Exception as e:  # noqa: BLE001 -- report and abort, like the original
            sys.stderr.write("error: could not open %s: %s\n" % (p, e))
            return 1
        name = p.rsplit("/", 1)[-1]
        if prev is None:
            warn = uniform_frame_warning(im)
            if warn:
                sys.stderr.write("WARNING: %s: %s\n" % (name, warn))
            print("%-14s %s" % (name, alpha_summary(im)))
        else:
            pd = summarize_pair(prev[1], im)
            print(format_pair_report(prev[0], name, pd))
            if not pd.resized:
                print("%-14s %s" % (" " * 14 + name, alpha_summary(im)))
        prev = (name, im)
    return 0


if __name__ == "__main__":
    if not pillow_available():
        sys.stderr.write("(pixel summary skipped: Pillow is not installed)\n")
        sys.exit(0)
    if len(sys.argv) != 2:
        sys.stderr.write("usage: pixdiff.py SHOTS-LIST-FILE\n")
        sys.exit(2)
    with open(sys.argv[1]) as f:
        shot_paths = [line.rstrip("\n") for line in f if line.strip()]
    sys.exit(run(shot_paths))
