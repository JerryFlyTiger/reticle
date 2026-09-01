#!/usr/bin/env python3
"""Mutation verification harness: revert one fix -> run the given test -> expect it to FAIL.

Steps 5 and 7 of the milestone loop do this every time (reviewer designs the
list, the main conversation runs it); before this tool existed, a throwaway
script was rewritten from scratch each time. Turning it into a fixed tool is
not just about saving effort — it's about hard-coding the rules we learned the
hard way, so we don't have to rely on memory next time:

1. **Restoration always uses a file backup via copy2, never `git checkout --`.**
   This project once had an agent restore a mutation with `git checkout --`,
   and it wiped out the whole file. Backups live in a scratch directory
   outside the repo, `finally` always restores, and at the end we print
   `git diff --stat` so "was the restore clean" is visible on the spot.

2. **Check the baseline before running anything.** If the tests are already
   red in the unmodified state, then every mutation will "FAIL as expected" —
   every conclusion would be fake. So we run the full target once first; if
   it isn't green, we stop immediately.

3. **The original string must appear exactly once.** Zero occurrences means
   the code has drifted (the list is stale); multiple occurrences means one
   change would touch several spots at once and the failure couldn't be
   attributed. Both cases are skipped and reported, never guessed at.

4. **After restoring, the mtime must be pushed forward (`os.utime`) — restoring
   the content alone is not enough.**
   `shutil.copy2` **preserves** the source's mtime, so "backup -> modify ->
   copy2 restore" leaves the file's mtime **rolled back** to before the
   change — older than the artifact cargo just built from the mutated source.
   cargo's freshness check looks at mtime, so it decides a rebuild isn't
   needed, and **the binary left on disk was built from the mutated source**.
   Consequence: every subsequent `cargo test` runs against a contaminated
   artifact, and both the mutation conclusions and the gate numbers become
   untrustworthy.
   Hit in practice on 2026-08-09: after wrapping up M51,
   `cargo test -p core --test lsp_highlight_tests` failed the same test three
   times in a row, while `cargo test --workspace` was all green and
   `git diff HEAD` was empty. A single `touch` to force a rebuild restored
   all-green. This is also why this tool pushes the mtime forward on every
   file it touches after it finishes — **do not** change it back to `copy2`.

5. **For some guards, the observable consequence is a "hang", not an assertion
   failure.**
   M52's GC mark has a `visited` dedup gate on `Value::Ext`; removing it
   makes a pure Ext<->Ext cycle send mark into an infinite loop — the test
   doesn't fail, it just never comes back. The original `subprocess.run` had
   no timeout, so the whole harness would hang along with it. Because of
   that, every cargo test item has a cap (`--timeout`, overridable per item
   via the `timeout` field), recorded as `HANG` on timeout; a list item can
   set `"expect": "hang"` to treat HANG as expected.

   **But the timeout must kill the whole process group, not rely on
   `subprocess.run(timeout=)`.** That only kills the direct child (`cargo`),
   while the infinite loop is running in the test binary that cargo forked
   (a grandchild) — that grandchild gets reparented to init and keeps burning
   100% CPU. The first version was written that way, and in practice it left
   an orphan alive for 18 minutes on the machine before anyone noticed. Fixed
   by using `Popen` with `start_new_session=True` and killing the group with
   `os.killpg` on timeout. This pitfall is nastier than the original "hang":
   a hang is visible, an orphan is not.

6. **"Not a single test actually ran" can disguise itself as "everything
   survived".** The first time M64 ran, all 7 items reported SURVIVED,
   which looked like the entire milestone's fixes were unguarded — the
   actual cause was that with `PACKAGE` left blank, `cargo test` in this
   workspace **only runs the root package** (the root `Cargo.toml` has its
   own `[package]`, this is not a pure virtual workspace), so the `frontend-tui`
   / `core` lib tests were never even compiled in, and the filter matched
   nothing. The output said so plainly: `0 passed; 0 failed;
   0 filtered out`. Because of this, the tool now classifies "not a single
   test actually ran this round" separately as `NOTESTS`, instead of folding
   it into SURVIVED — **the two call for opposite responses**: SURVIVED means
   add a test; NOTESTS means the invocation is wrong, and adding a test would
   only manufacture false confidence. The baseline is checked the same way.
   When a list spans multiple crates, run it in separate passes each with its
   own `-p`; don't leave it blank.

7. **No rebuild = this item's verdict is untrustworthy, no matter how much it
   looks like "as expected".** One M65 mutation "survived" when run as part
   of the batch but correctly hung when run alone, with a rebuild happening
   both times — digging in showed that mutation's observability itself is
   **load-dependent** (the code path with the guard removed only hangs
   forever when the fake server is consistently winning the race against the
   budget). The instrumentation added during that investigation is this very
   field: each item now prints its elapsed time and which crates got
   rebuilt. **But printing without judging isn't enough** — the trailing
   review round pointed out that an item that wasn't rebuilt but happened to
   "FAIL as expected" would get counted as "as expected" along with a warning
   string, with the exit code still 0 — a human had to actually spot that
   line. So `NOBUILD` is now its own verdict, checked before all others, and
   always counted as not-as-expected.
   `crates/core/lisp/*.el` are all compiled into the binary via `include_str!`
   (`crates/core/src/lib.rs:22`), so elisp mutations also need a rebuild to
   take effect — this rule applies to them equally.

8. **Items killed on timeout leave test scratch directories in `$TMPDIR`.**
   Test cleanup is a `Drop` guard (M63/M64), and `Drop` **is guaranteed not to
   run** under SIGKILL — this isn't a broken cleanup mechanism, it's the
   unavoidable cost of a hang having to end in SIGKILL. One M65 run left 30
   of them behind (all from the three `expect: hang` tests). After running a
   list with a lot of HANGs, sweep once:
       ls -d ${TMPDIR}reticle_* | wc -l   # confirm the count
       rm -rf ${TMPDIR}reticle_*          # confirm nothing is still running, then delete

**"Still PASS" is more worth looking at than "FAILed as expected"** — that
means that line of the fix has no test watching it at all, and the mutation
list would otherwise report a false pass. M51 caught three such gaps this
way (two fixes with zero coverage, one masked by an outer `condition-case`
and genuinely unobservable). So whenever any mutation survives, this tool
exits with code 1.

Usage
    dev/mutate.py --config dev/mutations/m51.py

    # change crate / test target
    dev/mutate.py --config dev/mutations/m51.py -p core --test-target lsp_highlight_tests

    # only run certain items in the list (match by label prefix)
    dev/mutate.py --config dev/mutations/m51.py --only M1 --only N3

The config file is a Python file defining `MUTATIONS` (list of dict), fields:

    label  display name; `--only` matches its prefix
    file   path relative to the repo root
    old    the original string to be replaced (must appear exactly once;
           use triple quotes for multi-line)
    new    what to replace it with
    test   the test name filter string passed to cargo test
    expect expected result, default "FAIL"; "hang" means the observable
           consequence of this guard is a hang
    timeout cap in seconds for this item, overrides --timeout

Optional `PACKAGE` / `TEST_TARGET` string variables act as defaults; command
line arguments take precedence. The config file is executed (`exec`), so only
put your own files there.
"""

import argparse
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def restore(bak, path):
    """Restore content from a backup, **and push the mtime to now**.

    Deliberately not using `shutil.copy2` — it preserves the backup's mtime,
    which would leave the restored file looking older than the artifact
    cargo just built from the mutated source, so cargo wouldn't rebuild and
    every later test would run on a contaminated binary (see header lesson 4).
    """
    shutil.copyfile(bak, path)
    os.utime(path, None)


def load_config(path):
    ns = {}
    with open(path, encoding="utf-8") as f:
        exec(compile(f.read(), path, "exec"), ns)  # noqa: S102 - our own config file
    if "MUTATIONS" not in ns:
        sys.exit(f"{path} does not define MUTATIONS")
    return ns["MUTATIONS"], ns.get("PACKAGE"), ns.get("TEST_TARGET")


def cargo_test(package, target, filter_str=None, timeout=None):
    cmd = ["cargo", "test"]
    if package:
        cmd += ["-p", package]
    if target == "lib":
        # M69: some guards are watched by unit tests inside a source file's
        # `#[cfg(test)]` block (pure function-level stuff belongs there per
        # CLAUDE.md), and that isn't a `--test` target. Leaving target blank
        # would make cargo run every test binary (77 in this workspace),
        # turning one mutation into tens of minutes.
        cmd.append("--lib")
    elif target:
        cmd += ["--test", target]
    if filter_str:
        cmd.append(filter_str)
    # Deliberately not using subprocess.run(timeout=...): on timeout it only
    # kills the direct child, i.e. `cargo` itself. The infinite loop actually
    # runs in the test binary cargo forked (a grandchild), which gets
    # reparented to init and keeps burning 100% CPU forever. Confirmed
    # 2026-08-09: after M52's M2 (removing the GC's Ext dedup gate) ran, the
    # machine was left with a PPID=1, 96% CPU gc_ext_root_tests orphan that
    # survived 18 minutes until manually killed. Switched to building our own
    # process group and killing the whole group on timeout.
    p = subprocess.Popen(cmd, cwd=REPO, text=True, start_new_session=True,
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(p.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            p.kill()
        # Reap it, and grab whatever output it already wrote (the pipe end
        # is fully closed by now).
        out, _ = p.communicate()
        return TIMEOUT, out or ""


# Sentinel return code for cargo_test: not a test failure, it just never finished.
TIMEOUT = "TIMEOUT"


def tests_actually_ran(out):
    """Did this cargo test run actually execute any tests at all (see header lesson 6).

    If the `test result:` line's passed/failed are both 0, the filter matched
    nothing — cargo happily exits 0 anyway, and that looks identical to
    "mutation survived".
    """
    total = 0
    for line in out.splitlines():
        if "test result:" not in line:
            continue
        parts = line.split()
        for i, tok in enumerate(parts):
            if tok in ("passed;", "failed;") and i > 0 and parts[i - 1].isdigit():
                total += int(parts[i - 1])
    return total > 0


def rebuilt_crates(out):
    """Which crates did this cargo run actually rebuild (look at `Compiling <name>` lines).

    A mutation changes source code, and **no rebuild means that change never
    made it into the binary under test** — so a "survived" verdict would be
    fake, not because the fix is unguarded but because it was never applied
    at all. This project's `crates/core/lisp/*.el` are all compiled into the
    binary via `include_str!` (`crates/core/src/lib.rs:22`), so elisp
    mutations also need a rebuild to take effect — this field matters equally
    for them.
    """
    return sorted({
        line.split()[1] for line in out.splitlines()
        if line.strip().startswith("Compiling") and len(line.split()) > 1
    })


def tail_of_interest(out, n=6):
    keep = [
        line for line in out.splitlines()
        if "test result:" in line or "FAILED" in line or "left:" in line
        or "right:" in line or "error[" in line or "panicked" in line
    ]
    return "\n".join("    " + line.strip() for line in keep[-n:]) or "    (no output worth excerpting)"


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", required=True, help="mutation list (a Python file)")
    ap.add_argument("-p", "--package", help="crate name for cargo -p")
    ap.add_argument("--test-target", help="test file name for cargo --test")
    ap.add_argument("--only", action="append", default=[],
                    help="only run items whose label starts with this, repeatable")
    ap.add_argument("--skip-baseline", action="store_true",
                    help="skip the baseline check (not recommended; see header lesson 2)")
    ap.add_argument("--timeout", type=float, default=600.0,
                    help="per-item cap in seconds for cargo test (default 600). "
                         "can be overridden per item via the timeout field")
    args = ap.parse_args()

    mutations, cfg_pkg, cfg_target = load_config(args.config)
    package = args.package or cfg_pkg
    target = args.test_target or cfg_target

    if args.only:
        mutations = [m for m in mutations
                     if any(m["label"].startswith(p) for p in args.only)]
        if not mutations:
            sys.exit("--only did not match any items")

    if not args.skip_baseline:
        print("=== baseline (unmodified state) ===")
        code, out = cargo_test(package, target, timeout=args.timeout)
        print(tail_of_interest(out, 3))
        if code == TIMEOUT:
            sys.exit(f"\n!! baseline did not finish within {args.timeout}s. "
                     "Confirm the tests themselves are healthy before running mutations.")
        if code != 0:
            sys.exit("\n!! baseline is not green. Fix it before running mutations — "
                     "otherwise every item will \"FAIL as expected\" and every conclusion "
                     "is fake (see header lesson 2).")
        if not tests_actually_ran(out):
            sys.exit(f"\n!! not a single test ran this round for the baseline (package={package!r}, "
                     f"target={target!r}). This workspace's root directory is itself a "
                     "package, so leaving `-p` blank only runs it — when a list spans "
                     "multiple crates, run it in separate passes each with its own -p "
                     "(see header lesson 6).")

    backup_dir = tempfile.mkdtemp(prefix="mutate-backup-")
    results = []
    try:
        for m in mutations:
            path = os.path.join(REPO, m["file"])
            bak = os.path.join(backup_dir, m["file"].replace("/", "__"))
            shutil.copy2(path, bak)
            try:
                src = open(path, encoding="utf-8").read()
                hits = src.count(m["old"])
                if hits != 1:
                    results.append((m["label"], "SKIP",
                                    f"    original string appears {hits} time(s) (need exactly 1) — "
                                    f"the list may be stale, not applied",
                                    m.get("expect", "FAIL").upper()))
                    continue
                with open(path, "w", encoding="utf-8") as f:
                    f.write(src.replace(m["old"], m["new"]))
                t0 = time.monotonic()
                code, out = cargo_test(package, target, m.get("test"),
                                       timeout=m.get("timeout", args.timeout))
                elapsed = time.monotonic() - t0
                built = rebuilt_crates(out)
                if code == TIMEOUT:
                    verdict = "HANG"
                elif code != 0:
                    verdict = "FAIL"
                elif not built:
                    # No rebuild = this mutation never made it into the binary
                    # under test, so **no** verdict is trustworthy (including
                    # "FAILed as expected" — that could be failing for an
                    # unrelated reason). So this check comes first, not only
                    # checked for SURVIVED. See header lesson 7.
                    verdict = "NOBUILD"
                elif not tests_actually_ran(out):
                    # The invocation is wrong (usually -p / --test-target
                    # pointed at the wrong thing), not missing coverage.
                    # Folding it into SURVIVED would send someone to add a
                    # test that isn't actually missing, see header lesson 6.
                    verdict = "NOTESTS"
                else:
                    verdict = "SURVIVED"
                # Put whether it rebuilt and how long it took into the report:
                # no rebuild means this mutation never reached the binary
                # under test, so that "survival" is fake (see rebuilt_crates).
                stamp = (f"    [{elapsed:.0f}s, rebuilt {','.join(built) if built else '(none)'}]"
                         + ("  <- no rebuild, this item's verdict is not trustworthy" if not built else ""))
                results.append((m["label"], verdict,
                                stamp + "\n" + tail_of_interest(out),
                                m.get("expect", "FAIL").upper()))
            finally:
                # Always restore from the backup. Never git checkout -- (once
                # wiped out an entire file), and never copy2 (rolls back
                # mtime, see header lesson 4).
                restore(bak, path)
    finally:
        shutil.rmtree(backup_dir, ignore_errors=True)
        # Belt and suspenders: make sure every touched file's mtime is fresh,
        # so the next cargo run always rebuilds.
        for m in mutations:
            p = os.path.join(REPO, m["file"])
            if os.path.exists(p):
                os.utime(p, None)

    print("\n" + "=" * 68)
    for label, verdict, detail, expect in results:
        if verdict == "SKIP":
            mark = "SKIP"
        elif verdict == expect:
            mark = {"FAIL": "FAIL (as expected)",
                    "HANG": "HANG (as expected: this guard's observable consequence is a hang, not an assertion failure)",
                    "SURVIVED": "SURVIVED (as expected)"}[verdict]
        elif verdict == "NOBUILD":
            mark = ("!! cargo did not rebuild any crate this round — the mutation never "
                    "reached the binary under test, so this item's verdict (either way) "
                    "is not trustworthy (see header lesson 7)")
        elif verdict == "NOTESTS":
            mark = ("!! not a single test actually ran this round — the filter matched "
                    "nothing, most likely -p / --test-target is wrong (see header lesson 6). "
                    "This is not \"missing coverage\", do not add a test because of it")
        elif verdict == "SURVIVED":
            mark = "!! still PASSes — this fix has no coverage"
        else:
            mark = f"!! expected {expect} but got {verdict}"
        print(f"\n### {label}\n  {mark}\n{detail}")

    print("\n" + "=" * 68)
    diff = subprocess.run(["git", "diff", "--stat"], cwd=REPO,
                          capture_output=True, text=True).stdout.strip()
    print("git diff --stat after restore (should match pre-mutation state):")
    print(diff or "  (working tree clean)")

    # "As expected" = this fix genuinely has a test watching it (FAIL or HANG,
    # per the list's declaration). Anything not as expected needs a human
    # look: SURVIVED means no coverage, the rest mean the list has drifted
    # from reality.
    mismatched = [(r[0], r[1], r[3]) for r in results
                  if r[1] != "SKIP" and r[1] != r[3]]
    skipped = [r[0] for r in results if r[1] == "SKIP"]
    print(f"\n{len(results)} item(s) total: "
          f"as expected {len(results) - len(mismatched) - len(skipped)}, "
          f"mismatched {len(mismatched)}, SKIP {len(skipped)}")
    if mismatched:
        print("Not as expected (SURVIVED means no coverage, needs a test added or must "
              "be honestly recorded as unobservable black-box):")
        for label, verdict, expect in mismatched:
            print(f"  - {label}: expected {expect}, got {verdict}")
    return 1 if mismatched or skipped else 0


if __name__ == "__main__":
    sys.exit(main())
