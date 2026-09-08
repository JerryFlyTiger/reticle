//! M114: puts `dev/`'s Python under the gate.
//!
//! `dev/pixdiff.py` (the pixel-diff summary `dev/gui-drive.sh` prints)
//! shipped four real bugs before this milestone -- two under-reporting
//! real change (an RGB-only comparison that made a pure alpha/opacity
//! change invisible, then a luma conversion that dropped alpha again
//! after the first fix, then the same luma mistake made a second time in
//! `rgb_px`), one silently dropping the bounding box on an RGB-only
//! change (`getbbox()`'s `alpha_only=True` default), and one
//! over-reporting change that was never visible (an RGB delta under
//! mutual full transparency) -- all because the summary used to live
//! inline in a shell heredoc, which cannot be tested, so nothing but a
//! human reading it could catch a defect. `dev/test_pixdiff.py` pins all
//! four down by name; this test is what makes sure that file actually
//! gets run, the same "nobody runs the tool" failure `lisp_hygiene_tests.rs`
//! exists to close for `.el` docstrings.
//!
//! **This test FAILS, not skips, when python3 or Pillow is unavailable.**
//! An earlier version of this test skipped silently on a missing Pillow,
//! matching `manual_e2e_*`'s PATH-check convention elsewhere in this
//! suite -- but libtest captures and discards a passing test's stdout, so
//! `cargo test --workspace --no-fail-fast` (this project's own definition
//! of done) could go fully green on a machine without Pillow while never
//! having run a single one of `dev/test_pixdiff.py`'s checks, including
//! the four historical-bug regression tests. A green run must mean the
//! checks ran. Set `RETICLE_ALLOW_MISSING_PILLOW=1` to deliberately opt
//! out on a machine that genuinely lacks Pillow -- this prints plainly
//! that it is skipping and why, so that choice is visible in the test
//! output rather than indistinguishable from a real pass.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Set this to any value to turn a missing python3/Pillow into a
/// deliberate, visible skip instead of a failure. See module doc comment.
const SKIP_ENV: &str = "RETICLE_ALLOW_MISSING_PILLOW";

fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Whether `python3` exists and can import Pillow -- `dev/pixdiff.py` and
/// `dev/test_pixdiff.py` both treat a missing Pillow as a skip condition
/// (matching `dev/gui-drive.sh`'s existing behavior when Pillow is
/// absent); this test decides separately (see module doc comment) whether
/// ITS OWN absence-handling is a skip or a failure.
fn python3_with_pillow_available() -> bool {
    match Command::new("python3")
        .arg("-c")
        .arg("import PIL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[test]
fn dev_pixdiff_tests_pass() {
    if !python3_with_pillow_available() {
        // Only an affirmative value opts out. `is_ok()` would treat
        // RETICLE_ALLOW_MISSING_PILLOW=0 -- or an empty string, or a
        // boolean-style "false" left over from some other tool -- as
        // "yes, skip", while the panic message below tells the reader to
        // set it to 1. A cold reviewer flagged that gap; an environment
        // that means "off" must not silently disable the check this
        // milestone exists to make un-disableable by accident.
        let opted_out = matches!(
            std::env::var(SKIP_ENV).as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        if opted_out {
            eprintln!(
                "skipping (opted out via {}): python3 with Pillow is not \
                 available, dev/test_pixdiff.py was not run",
                SKIP_ENV
            );
            return;
        }
        panic!(
            "python3 with Pillow is not available -- dev/test_pixdiff.py \
             was not run, including its four historical-bug regression \
             tests. Failing by default so a missing dependency cannot \
             silently pass as a green gate. Install Pillow (`pip install \
             Pillow` or `pip3 install Pillow`), or set {}=1 to \
             deliberately skip on a machine that genuinely lacks it.",
            SKIP_ENV
        );
    }

    let output = Command::new("python3")
        .arg("dev/test_pixdiff.py")
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn python3 dev/test_pixdiff.py: {}", e));

    if !output.status.success() {
        println!("stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        println!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        panic!(
            "dev/test_pixdiff.py exited with status {:?}",
            output.status.code()
        );
    }
}
