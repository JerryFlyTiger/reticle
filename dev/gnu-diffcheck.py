#!/usr/bin/env python3
"""Replay a `verilog-auto' fixture through the REAL reticle editor and print
what it generated, for comparison against what real GNU Emacs produced for
the same file (`dev/gnu-auto/run.sh').

    dev/gnu-diffcheck.py dev/gnu-auto/fixtures/autoreset-basic-two-flops.v
    dev/gnu-auto/run.sh  dev/gnu-auto/fixtures/autoreset-basic-two-flops.v

Why this exists, and why reading the code is not a substitute for it:
M134's worst defect was that `/*AUTORESET*/` expanded even when the marker
was not inside a conditional branch. Because the last assignment wins in a
procedural block, the inserted resets made every signal permanently zero --
valid Verilog, clean lint, dead circuit. Twenty-one targeted tests were green,
a full cold review had read the diff, and neither saw it. What saw it was
replaying the GNU fixtures through the built editor and diffing the output.

The reverse also happened in the same milestone: two defects that this tool
cannot see (a discarded skip-reason emitting `arr <= ;`, and a whole-branch
exclusion where GNU's is positional) were found by cold reading. Neither
method subsumes the other; run both.

Mechanics worth knowing:

* It turns `format-on-save' OFF before expanding, so what comes back is what
  the expander produced rather than what the formatter then rewrote. Without
  that, every comparison against GNU differs in alignment and reads as a
  mismatch.
* It drives the editor through `dev/tui-drive.py', i.e. the real binary in a
  pty, not a test harness -- so it exercises the same path a user does.
  **Build first** (`cargo build --workspace`); this script does not, and a
  stale binary silently measures the previous revision.
* The fixture is copied into a scratch directory first, so sibling modules
  resolve the same way `run.sh` arranges for GNU, and the original is never
  modified.
"""

import os
import shutil
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def expand(fixture, workdir):
    """Copy FIXTURE into WORKDIR, run `M-x verilog-auto', save, return text."""
    dst = os.path.join(workdir, os.path.basename(fixture))
    shutil.copy(fixture, dst)
    # sibling modules (sub.v and friends) have to be resolvable from there
    srcdir = os.path.dirname(os.path.abspath(fixture))
    for name in os.listdir(srcdir):
        if name.endswith((".v", ".sv")) and name != os.path.basename(fixture):
            shutil.copy(os.path.join(srcdir, name), os.path.join(workdir, name))

    script = os.path.join(workdir, "drive.tui")
    lines = ["OPEN -nw " + dst, "WAIT 3", "KEY M-:"]
    # `dev/tui-drive.py' drops whitespace inside a single KEY, so each token
    # goes in its own KEY with an explicit SPC between (its pitfall 5).
    for tok in "(setq format-on-save nil)".split(" "):
        lines += ["KEY '" + tok, "KEY SPC"]
    lines.pop()
    lines += ["KEY RET", "WAIT 1", "KEY M-x"]
    for ch in "verilog-auto":
        lines.append("KEY '" + ch)
    lines += ["KEY RET", "WAIT 3", "KEY C-x", "KEY C-s", "WAIT 2", "SHOT done"]
    with open(script, "w") as fh:
        fh.write("\n".join(lines) + "\n")

    subprocess.run(
        ["python3", "dev/tui-drive.py", script, "target/debug/reticle"],
        cwd=REPO, capture_output=True, timeout=180,
    )
    with open(dst) as fh:
        return fh.read()


def generated_block(text):
    """The `// Beginning of ...' ... `// End of automatics' spans, in order."""
    out, keep = [], False
    for line in text.split("\n"):
        if "// Beginning of" in line:
            keep = True
        if keep:
            out.append(line)
        if "// End of automatics" in line:
            keep = False
    return out


def main(argv):
    if not argv:
        sys.exit(__doc__)
    binary = os.path.join(REPO, "target/debug/reticle")
    if not os.path.exists(binary):
        sys.exit("target/debug/reticle is missing -- run `cargo build --workspace' first")
    for fixture in argv:
        print(f"===== {os.path.basename(fixture)}")
        with tempfile.TemporaryDirectory(prefix="gnu-diffcheck-") as workdir:
            block = generated_block(expand(fixture, workdir))
        if block:
            print("\n".join(block))
        else:
            print("  (nothing generated)")


if __name__ == "__main__":
    main(sys.argv[1:])
