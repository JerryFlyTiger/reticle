"""Tests for dev/pixdiff.py -- plain asserts, no test framework.

Run with: python3 dev/test_pixdiff.py

M114: this file exists because pixdiff.py's predecessor (an inline
`python3 -c '...'` heredoc in dev/gui-drive.sh) shipped four real bugs
that survived a fix and a self-check, and only a cold reviewer caught
them -- see dev/pixdiff.py's module docstring. Tests below are built on
synthetic in-memory images; no fixtures are checked in.

Test discovery is by naming convention, not a hand-maintained list: `main`
collects every callable named `test_*` off this module's own namespace,
sorted by name for determinism. A hand-written list is exactly the
"defined but never run" failure this whole milestone exists to close,
reproduced in its own harness -- a test added here and forgotten from a
list would never execute, and the runner would still print an all-green
summary.

Requires Pillow. If it is not installed, prints that and exits 0 (skip,
not fail) -- consistent with the rest of this project's "Pillow missing"
handling in dev/gui-drive.sh and dev/pixdiff.py itself. Note this is a
different policy from `crates/core/tests/dev_tools_tests.rs`, which calls
this file as a subprocess: that Rust test FAILS by default on a missing
Pillow (opt out with an explicit env var) so the compiled test suite can
never go green without actually having run these checks. This file's own
exit-0 skip is fine specifically because nothing downstream mistakes
it for "checks ran and passed" the way a passing `cargo test` result
would be read.
"""

import os
import sys

try:
    from PIL import Image
except ImportError:
    print("SKIPPED: Pillow is not installed")
    sys.exit(0)

import pixdiff  # noqa: E402

PIXDIFF_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pixdiff.py")


def solid(size, color):
    """An RGBA image, every pixel the same COLOR (a 4-tuple)."""
    return Image.new("RGBA", size, color)


def test_alpha_only_change_reports_nonzero_headline():
    """Bug 1 (M111): comparing after `.convert("RGB")` made an
    opacity-only change invisible -- 0 differing pixels for a real
    100%%->40%% opacity switch. Same RGB, different alpha must show up in
    the headline count."""
    a = solid((10, 10), (200, 50, 50, 255))
    b = solid((10, 10), (200, 50, 50, 102))  # ~40% opacity
    pd = pixdiff.summarize_pair(a, b)
    assert pd.diff_px == 100, pd
    assert pd.alpha_px == 100, pd
    assert pd.rgb_px == 0, pd


def test_rgb_only_change_reports_nonzero_with_alpha_zero():
    """Bug 2 (M111): the headline still went through `d.convert("L")`,
    which drops alpha but is not exercised by this case -- this pins down
    the complementary claim: an RGB-only change must show up with
    alpha=0, so a real colour change and a real opacity change are never
    confused with each other."""
    a = solid((10, 10), (0, 0, 0, 255))
    b = solid((10, 10), (255, 255, 255, 255))
    pd = pixdiff.summarize_pair(a, b)
    assert pd.diff_px == 100, pd
    assert pd.rgb_px == 100, pd
    assert pd.alpha_px == 0, pd


def test_single_channel_delta_of_one_is_not_lost_to_luma_rounding():
    """Bug 4 (M114 review): `rgb_px` went through `.convert("L")` on the
    difference image, the same lossy luma weighting as bug 2 -- a
    single-channel delta of 1 (red or blue) rounds to L=0 and vanishes.
    Both frames fully opaque so masking (rule 5) cannot be the reason this
    passes; this isolates the luma-rounding fix specifically."""
    a = solid((4, 4), (10, 20, 30, 255))
    b = solid((4, 4), (11, 20, 30, 255))  # red channel +1 only
    pd = pixdiff.summarize_pair(a, b)
    assert pd.rgb_px == 16, pd
    assert pd.diff_px == 16, pd
    assert pd.alpha_px == 0, pd

    a2 = solid((4, 4), (10, 20, 30, 255))
    b2 = solid((4, 4), (10, 20, 34, 255))  # blue channel +4 only
    pd2 = pixdiff.summarize_pair(a2, b2)
    assert pd2.rgb_px == 16, pd2


def test_identical_frames_report_zero_everywhere():
    a = solid((8, 8), (10, 20, 30, 255))
    b = solid((8, 8), (10, 20, 30, 255))
    pd = pixdiff.summarize_pair(a, b)
    assert pd.diff_px == 0, pd
    assert pd.rgb_px == 0, pd
    assert pd.alpha_px == 0, pd
    assert pd.bbox is None, pd


def test_both_channels_changing_reports_consistent_counts():
    """diff_px (any channel moved) must be >= max(rgb_px, alpha_px), and
    since here every pixel moves in both, all three must be equal to the
    frame's pixel count."""
    a = solid((6, 6), (0, 0, 0, 255))
    b = solid((6, 6), (255, 255, 255, 100))
    pd = pixdiff.summarize_pair(a, b)
    total = 36
    assert pd.diff_px == total, pd
    assert pd.rgb_px == total, pd
    assert pd.alpha_px == total, pd


def test_differing_sizes_produce_resize_note_and_do_not_raise():
    a = solid((10, 10), (0, 0, 0, 255))
    b = solid((20, 15), (0, 0, 0, 255))
    pd = pixdiff.summarize_pair(a, b)  # must not raise
    assert pd.resized is True, pd
    assert pd.prev_size == (10, 10), pd
    assert pd.size == (20, 15), pd
    report = pixdiff.format_pair_report("shot-1s.png", "shot-2s.png", pd)
    assert "window resized" in report, report
    assert "(10, 10)" in report and "(20, 15)" in report, report


def test_fully_transparent_frame_alpha_summary():
    im = solid((5, 5), (1, 2, 3, 0))
    summary = pixdiff.alpha_summary(im)
    assert "a=0 100%" in summary, summary


def test_fully_opaque_frame_alpha_summary():
    im = solid((5, 5), (1, 2, 3, 255))
    summary = pixdiff.alpha_summary(im)
    assert "a=255 100%" in summary, summary


def test_uniform_frame_is_flagged_and_a_real_frame_is_not():
    """A sleeping display captures as a uniform frame, and `screencapture`
    reports success for it -- the file is non-empty, so a caller checking
    only the size accepts a picture of nothing. Cost a round trip during
    M116, diagnosing a permission problem that did not exist.
    """
    black = Image.new("RGBA", (20, 20), (0, 0, 0, 255))
    assert pixdiff.uniform_frame_warning(black) is not None

    # A frame uniform in ONE channel only must not be flagged -- otherwise
    # any solidly-tinted but real screenshot would trip it.
    red = Image.new("RGBA", (20, 20), (255, 0, 0, 255))
    for x in range(20):
        for y in range(20):
            red.putpixel((x, y), (255, x * 12 % 256, y * 12 % 256, 255))
    assert pixdiff.uniform_frame_warning(red) is None

    # Just under the threshold: 98% one colour is still a picture.
    mixed = Image.new("RGBA", (10, 10), (1, 2, 3, 255))
    mixed.putpixel((0, 0), (200, 200, 200, 255))
    mixed.putpixel((1, 0), (150, 150, 150, 255))
    assert pixdiff.uniform_frame_warning(mixed) is None


def test_alpha_summary_reports_bimodal_distribution_by_percentage():
    """`alpha_summary`'s top-2 selection (`[:2]` in pixdiff.py) is only
    exercised elsewhere in this file by uniform single-level images, where
    top-2 vs. top-1 is unobservable. This builds an explicit two-level
    distribution -- a background plus a translucent window, the realistic
    case -- and pins the exact string, split and percentages both."""
    w, h = 10, 10
    pixels = [(0, 0, 0, 255)] * 60 + [(0, 0, 0, 128)] * 40
    assert len(pixels) == w * h
    im = Image.new("RGBA", (w, h))
    im.putdata(pixels)
    summary = pixdiff.alpha_summary(im)
    assert summary == "a=255 60%  a=128 40%", summary


def test_bounding_box_matches_a_confined_rectangle():
    im = Image.new("RGBA", (20, 20), (0, 0, 0, 255))
    changed = im.copy()
    # Paint a known 4x3 rectangle: columns 5..8, rows 6..8 (inclusive).
    for x in range(5, 9):
        for y in range(6, 9):
            changed.putpixel((x, y), (255, 255, 255, 255))
    pd = pixdiff.summarize_pair(im, changed)
    # PIL's getbbox() returns (left, upper, right, lower) with right/lower
    # exclusive, so the rectangle above is (5, 6, 9, 9).
    assert pd.bbox == (5, 6, 9, 9), pd.bbox
    assert pd.diff_px == 4 * 3, pd


def test_rgb_change_under_mutual_transparency_is_not_counted():
    """Review finding (M114): the mirror image of the four historical
    bugs -- those under-reported real change, this over-reports change
    that was never visible. Two frames both fully transparent (alpha 0)
    everywhere but with different RGB bytes underneath must report ZERO
    -- RGB under alpha=0 is arbitrary padding, not a real change."""
    a = solid((10, 10), (0, 0, 0, 0))
    b = solid((10, 10), (255, 255, 255, 0))
    pd = pixdiff.summarize_pair(a, b)
    assert pd.rgb_px == 0, pd
    assert pd.diff_px == 0, pd
    assert pd.alpha_px == 0, pd


def test_rgb_change_visible_in_only_one_frame_still_counts():
    """Complement of the mutual-transparency test above: the masking rule
    is OR (visible in EITHER frame), not AND. A pixel opaque in one frame
    and transparent in the other was actually seen at some point, so an
    RGB difference there is real and must still be counted."""
    a = solid((10, 10), (0, 0, 0, 255))  # opaque
    b = solid((10, 10), (255, 255, 255, 0))  # transparent, different RGB
    pd = pixdiff.summarize_pair(a, b)
    assert pd.rgb_px == 100, pd
    assert pd.alpha_px == 100, pd
    assert pd.diff_px == 100, pd


def test_rgb_and_rgba_inputs_mixed_via_run():
    """`run()` must handle a mix of RGB and RGBA files in one shot list --
    pixdiff.py converts every frame to RGBA on open regardless of what it
    was saved as."""
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        p1 = os.path.join(d, "shot-1s.png")
        p2 = os.path.join(d, "shot-2s.png")
        Image.new("RGB", (4, 4), (10, 10, 10)).save(p1)
        Image.new("RGBA", (4, 4), (10, 10, 10, 128)).save(p2)
        shots_file = os.path.join(d, "shots.txt")
        with open(shots_file, "w") as f:
            f.write(p1 + "\n" + p2 + "\n")
        with open(shots_file) as f:
            paths = [line.rstrip("\n") for line in f if line.strip()]
        rc = pixdiff.run(paths)
        assert rc == 0, rc


def test_frame_that_fails_to_open_reports_nonzero_exit():
    rc = pixdiff.run(["/nonexistent/path/does-not-exist.png"])
    assert rc == 1, rc


def test_cli_runs_end_to_end_as_subprocess():
    """Exercises `pixdiff.py`'s `__main__` block directly by invoking it
    as a real subprocess -- nothing else in this file does, including
    argument parsing and `pillow_available()`'s only real call site."""
    import subprocess
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        p1 = os.path.join(d, "shot-1s.png")
        p2 = os.path.join(d, "shot-2s.png")
        Image.new("RGBA", (4, 4), (10, 10, 10, 255)).save(p1)
        Image.new("RGBA", (4, 4), (200, 10, 10, 255)).save(p2)
        shots_file = os.path.join(d, "shots.txt")
        with open(shots_file, "w") as f:
            f.write(p1 + "\n" + p2 + "\n")
        result = subprocess.run(
            [sys.executable, PIXDIFF_SCRIPT, shots_file],
            capture_output=True,
            text=True,
        )
        assert result.returncode == 0, result
        assert "differing px" in result.stdout, result.stdout


def test_cli_wrong_argument_count_reports_usage_and_exits_nonzero():
    """The other branch of `__main__`'s argument check -- no shots-file
    argument at all."""
    import subprocess

    result = subprocess.run(
        [sys.executable, PIXDIFF_SCRIPT],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2, result
    assert "usage:" in result.stderr, result.stderr


def test_module_imports_cleanly_with_pillow_absent():
    """`pixdiff` defers every PIL import so it stays importable without
    Pillow -- that is what lets `dev/gui-drive.sh` print a clean "skipped"
    note instead of a traceback.

    Nothing tested it, and nothing in this file *can*: these tests import
    PIL at module scope, so they only ever run in an environment where
    Pillow is present. A cold reviewer confirmed that hoisting a PIL
    import to module level in pixdiff.py is a complete no-op against the
    rest of this suite. So this one runs a subprocess with PIL blocked at
    the import system, which is the only environment where the contract
    is observable.
    """
    import subprocess
    import os

    dev_dir = os.path.dirname(os.path.abspath(__file__))
    code = (
        "import importlib.abc, sys\n"
        "class Block(importlib.abc.MetaPathFinder):\n"
        "    def find_spec(self, fullname, path, target=None):\n"
        "        if fullname == 'PIL' or fullname.startswith('PIL.'):\n"
        "            raise ImportError('PIL blocked for this test')\n"
        "        return None\n"
        "sys.meta_path.insert(0, Block())\n"
        "sys.path.insert(0, %r)\n"
        "import pixdiff\n"
        "assert pixdiff.pillow_available() is False, 'pillow_available must say no'\n"
        "print('imported without Pillow')\n" % dev_dir
    )
    r = subprocess.run(
        [sys.executable, "-c", code], capture_output=True, text=True
    )
    assert r.returncode == 0, (r.returncode, r.stdout, r.stderr)
    assert "imported without Pillow" in r.stdout, (r.stdout, r.stderr)


def test_alpha_mask_is_spatially_aligned_with_the_rgb_difference():
    """A colour change confined to the TRANSPARENT half must not count,
    while the same change in the opaque half must.

    Every other masking test uses a uniform image, where flipping or
    transposing the mask is indistinguishable from doing nothing -- a cold
    reviewer transposed `alpha_visible` and all sixteen tests still passed.
    This one is non-uniform on purpose: left half transparent, right half
    opaque, with the colour change confined to one side at a time.
    """
    w, h = 8, 4
    def halves(left_rgb, right_rgb):
        im = Image.new("RGBA", (w, h))
        for x in range(w):
            for y in range(h):
                if x < w // 2:
                    im.putpixel((x, y), left_rgb + (0,))      # transparent half
                else:
                    im.putpixel((x, y), right_rgb + (255,))   # opaque half
        return im

    # Change only the transparent half's colour: invisible, must not count.
    a = halves((10, 20, 30), (200, 100, 50))
    b = halves((99, 99, 99), (200, 100, 50))
    pd = pixdiff.summarize_pair(a, b)
    assert pd.rgb_px == 0, pd
    assert pd.diff_px == 0, pd

    # Change only the opaque half's colour: visible, must count exactly the
    # opaque half's pixel count -- which is also what pins the ALIGNMENT, since
    # a flipped mask would zero these and keep the ones above.
    a = halves((10, 20, 30), (200, 100, 50))
    b = halves((10, 20, 30), (201, 100, 50))
    pd = pixdiff.summarize_pair(a, b)
    assert pd.rgb_px == (w // 2) * h, pd
    assert pd.diff_px == (w // 2) * h, pd


def test_bbox_is_deliberately_unmasked_even_when_nothing_was_visible():
    """`bbox` answers "where did bytes move", not "where was something
    visible", and is left unmasked on purpose. That is a judgement call, so
    it is pinned here: two fully transparent frames differing only in their
    (invisible) RGB bytes report every count as zero AND a non-None box.
    If that ever changes it should change because someone decided to, not
    because a mask quietly grew an extra consumer.
    """
    a = Image.new("RGBA", (10, 10), (0, 0, 0, 0))
    b = Image.new("RGBA", (10, 10), (255, 255, 255, 0))
    pd = pixdiff.summarize_pair(a, b)
    assert (pd.diff_px, pd.rgb_px, pd.alpha_px) == (0, 0, 0), pd
    assert pd.bbox == (0, 0, 10, 10), pd


# Discovery has to have a floor. Replacing the hand-maintained `tests = [...]`
# list with name-based discovery removed one way for a check to exist and never
# run -- and immediately introduced another: a cold reviewer changed the prefix
# to "Test_" and this file printed "0/0 tests passed" and exited 0. That is the
# fourth appearance of "defined but never executed" in this one tool. A count
# that can only go up, asserted here and re-checked by the Rust gate test that
# runs this file, is what stops the fifth.
MINIMUM_TESTS = 20


def main():
    mod = sys.modules[__name__]
    tests = sorted(
        (name, obj)
        for name, obj in vars(mod).items()
        if name.startswith("test_") and callable(obj)
    )
    if len(tests) < MINIMUM_TESTS:
        print(
            "DISCOVERY FAILURE: found %d test functions, expected at least %d. "
            "Either a test was deleted (lower MINIMUM_TESTS deliberately, in a "
            "commit that says why) or the `test_` naming convention broke and "
            "the checks below are silently not running."
            % (len(tests), MINIMUM_TESTS)
        )
        sys.exit(1)
    failures = []
    for name, t in tests:
        try:
            t()
            print("OK: %s" % name)
        except AssertionError as e:
            print("FAIL: %s: %s" % (name, e))
            failures.append(name)
        except Exception as e:  # noqa: BLE001
            print("ERROR: %s: %s" % (name, e))
            failures.append(name)
    print()
    if failures:
        print(
            "%d/%d tests FAILED: %s" % (len(failures), len(tests), ", ".join(failures))
        )
        sys.exit(1)
    print("%d/%d tests passed" % (len(tests), len(tests)))
    sys.exit(0)


if __name__ == "__main__":
    main()
