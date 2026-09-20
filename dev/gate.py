#!/usr/bin/env python3
"""Resumable, segmented test gate for this workspace's definition of done.

Why this exists
----------------
The project's definition of done is
`cargo build --workspace` + `cargo fmt --check` +
`cargo clippy --workspace --all-targets -- -D warnings` +
`cargo test --workspace --no-fail-fast`. On a memory-constrained machine the
test step has been killed for low memory in each of the last two milestones,
always somewhere around `core`'s 67th test binary. Each time, recovery was
manual bookkeeping: read the dead log, work out which targets never printed a
`test result:` line, rerun those one at a time, add it all up by hand -- and
that bookkeeping produced a *wrong* total once, because one test binary's log
contains two `test result:` lines (a test's own subprocess echoes one), so
counting *lines* overcounts. This tool runs one `cargo test` invocation per
target, records a durable per-target result, and can resume after a kill
without rerunning what already passed.

Usage
-----
    python3 dev/gate.py                    # full gate: build+fmt+clippy+every target
    python3 dev/gate.py --fresh            # discard any saved state first
    python3 dev/gate.py --tests-only       # skip build/fmt/clippy (they must have
                                            # already passed for the current tree)
    python3 dev/gate.py --list             # print target keys and exit
    python3 dev/gate.py --only core        # restrict to one package (partial verdict)
    python3 dev/gate.py --floor N          # override the minimum target-count floor
    python3 dev/gate.py --jobs 2           # pass -j 2 to build/clippy/test invocations
    python3 dev/gate.py --cargo PATH       # use a different cargo binary (tests use this
                                            # to inject dev/fake-cargo.py)
    python3 dev/gate.py --root DIR         # repo root (default: parent of dev/)

Rules this tool enforces
-------------------------
1. **Count per invocation, never per line.** A target's result is the *last*
   `test result:` line in its own invocation's output, parsed into
   passed/failed/ignored -- not the count of matching lines, which a
   subprocess-under-test can inflate.
2. **A mechanism that decides what runs must fail loudly when it decides
   nothing does**. Target enumeration has a floor
   (`--floor`, default 90): fewer than that and this tool exits 2 instead of
   quietly reporting a green run over an empty set.
3. **State survives a kill.** Progress is written to
   `target/gate/state.json` after every target, atomically (write to a temp
   file, `os.replace`). Re-running skips targets already recorded `ok` for
   the *same* tree -- see the stamp rule below.
4. **The state is keyed to a content stamp, not a wall-clock guess.** The
   stamp is a sha256 over every tracked-or-untracked, non-ignored file's
   `(path, size, mtime_ns)` (via `git ls-files -co --exclude-standard`), plus
   `Cargo.lock`. If the tree has changed since the saved state, the state is
   discarded and the run starts fresh -- resuming a stale run against a
   changed tree would silently validate code nobody tested.
5. **`--tests-only` cannot be used to skip build/fmt/clippy speculatively.**
   It only works when the saved state says all three already passed under
   the *current* stamp; otherwise it refuses outright.
6. **`.claude/` is excluded from the stamp.** It is not in .gitignore, and
   this project's own reviewer/architect isolation worktrees live at
   `.claude/worktrees/<agent>/` -- each one is a directory `git ls-files -co
   --exclude-standard` reports, so spinning one up or tearing one down would
   otherwise invalidate the whole gate state with no source change at all.
7. **`--only PKG` never prints a passing verdict for zero targets.** An
   unknown package name is an error (exit 2, naming the package and listing
   the known ones), and the floor for `--only` is always that package's own
   pre-filter target count, so a genuinely empty match can't slip through.
8. **`GATE PASSED` never appears without saying whether the run was
   partial** -- `GATE PASSED (101 targets)` for a full run, `GATE PASSED
   (partial: only PKG)` for `--only`.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
# Reused rather than reimplemented -- see dev/mutate.py's own module
# docstring for why each of these rules exists.
from mutate import tests_actually_ran, rebuilt_crates, tail_of_interest  # noqa: E402

RESULT_RE = re.compile(
    r"test result: (?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;"
)


def default_root():
    return os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))


def run_cargo(cargo_bin, subcommand, args, root, jobs=None, log_path=None):
    """Run one cargo invocation, capturing combined stdout+stderr.

    Returns (exit_code, output, seconds). If log_path is given, the raw
    output is written there (working directory is always the repo root).
    """
    cmd = [cargo_bin, subcommand]
    if jobs and subcommand != "fmt":
        cmd += ["-j", str(jobs)]
    cmd += args
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(cmd, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    except FileNotFoundError:
        # Untested: no test in dev/test_gate.py points --cargo at a missing
        # binary (M140 review, recorded rather than pretended covered).
        print(f"error: {cargo_bin} not found on PATH", file=sys.stderr)
        sys.exit(2)
    seconds = time.perf_counter() - t0
    if log_path:
        os.makedirs(os.path.dirname(log_path), exist_ok=True)
        with open(log_path, "w") as f:
            f.write(proc.stdout or "")
    return proc.returncode, proc.stdout or "", seconds


def enumerate_targets(cargo_bin, root):
    """Return a sorted list of target dicts: {key, pkg, kind, name, cargo_args}."""
    exit_code, out, _ = run_cargo(
        cargo_bin, "metadata", ["--no-deps", "--format-version", "1"], root
    )
    if exit_code != 0:
        print(f"cargo metadata failed (exit {exit_code}):\n{out}", file=sys.stderr)
        sys.exit(2)
    meta = json.loads(out)
    targets = []
    for pkg in sorted(meta["packages"], key=lambda p: p["name"]):
        pkg_name = pkg["name"]
        # `proc-macro` targets are library-shaped from cargo test's point of
        # view (`--lib` is the right selector) exactly like `cdylib` --
        # neither exists in this workspace today, so this is untested against
        # the real workspace; see test_enumeration_treats_proc_macro_as_lib
        # against a metadata fixture instead.
        lib_targets = [
            t for t in pkg["targets"]
            if set(t["kind"]) & {"lib", "rlib", "cdylib", "dylib", "proc-macro"}
        ]
        # A cdylib- or proc-macro-only package (e.g. demo-module) has no
        # library target that `cargo test --doc` can run against -- verified
        # directly for cdylib: `cargo test -p demo-module --doc` errors with
        # "no library targets found in package `demo-module`" after warning
        # that doc tests are not supported for crate type `cdylib`.
        has_real_lib = any(set(t["kind"]) & {"lib", "rlib", "dylib"} for t in pkg["targets"])
        for t in lib_targets:
            targets.append({
                "key": f"{pkg_name}/lib",
                "pkg": pkg_name,
                "kind": "lib",
                "name": t["name"],
                "cargo_args": ["-p", pkg_name, "--lib"],
            })
        # cargo metadata's `test` field on a [[test]] target is true unless
        # the target opts out (e.g. `harness = false` combined with a
        # deliberately excluded test); no target in this workspace does that
        # today, so this is also untested against the real workspace -- see
        # test_enumeration_skips_test_false_targets.
        test_targets = sorted(
            (t for t in pkg["targets"] if "test" in t["kind"] and t.get("test", True)),
            key=lambda t: t["name"],
        )
        for t in test_targets:
            targets.append({
                "key": f"{pkg_name}/test/{t['name']}",
                "pkg": pkg_name,
                "kind": "test",
                "name": t["name"],
                "cargo_args": ["-p", pkg_name, "--test", t["name"]],
            })
        bin_targets = sorted(
            (t for t in pkg["targets"] if "bin" in t["kind"]), key=lambda t: t["name"]
        )
        for t in bin_targets:
            targets.append({
                "key": f"{pkg_name}/bin/{t['name']}",
                "pkg": pkg_name,
                "kind": "bin",
                "name": t["name"],
                "cargo_args": ["-p", pkg_name, "--bin", t["name"]],
            })
        if has_real_lib:
            targets.append({
                "key": f"{pkg_name}/doc",
                "pkg": pkg_name,
                "kind": "doc",
                "name": "doc",
                "cargo_args": ["-p", pkg_name, "--doc"],
            })
    return targets


def compute_stamp(root):
    try:
        proc = subprocess.run(
            ["git", "ls-files", "-co", "--exclude-standard"],
            cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
    except FileNotFoundError:
        # Untested, same as the cargo branch above: git is always present
        # where this runs.
        print("error: git not found on PATH", file=sys.stderr)
        sys.exit(2)
    if proc.returncode != 0:
        print(f"git ls-files failed:\n{proc.stderr}", file=sys.stderr)
        sys.exit(2)
    # `.claude/` is not in .gitignore -- it holds this project's own
    # reviewer/architect isolation worktrees
    # (`.claude/worktrees/<agent>/`), which `git ls-files -co
    # --exclude-standard` lists as ordinary directory entries. Spinning one
    # up or tearing one down would otherwise invalidate the whole gate state
    # with no source change at all, so paths under `.claude/` are excluded
    # from the stamp; everything else untracked is still included.
    files = {
        line.strip() for line in proc.stdout.splitlines()
        if line.strip() and not line.strip().startswith(".claude/")
    }
    files.add("Cargo.lock")
    h = hashlib.sha256()
    for path in sorted(files):
        full = os.path.join(root, path)
        try:
            st = os.stat(full)
            size, mtime_ns = st.st_size, st.st_mtime_ns
        except FileNotFoundError:
            size, mtime_ns = -1, -1
        h.update(f"{path}\0{size}\0{mtime_ns}\n".encode())
    return h.hexdigest()


def state_path(root):
    return os.path.join(root, "target", "gate", "state.json")


def load_state(root):
    path = state_path(root)
    if not os.path.exists(path):
        return None
    try:
        with open(path) as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return None


def save_state(root, state):
    path = state_path(root)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(state, f, indent=2, sort_keys=True)
    os.replace(tmp, path)


def parse_test_result(output):
    """Parse the LAST `test result:` line -- never count lines, see module doc."""
    matches = list(RESULT_RE.finditer(output))
    if not matches:
        return None
    m = matches[-1]
    return {
        "passed": int(m.group(1)),
        "failed": int(m.group(2)),
        "ignored": int(m.group(3)),
        "result_line": output[m.start():output.find("\n", m.start())].strip()
        if "\n" in output[m.start():] else output[m.start():].strip(),
    }


def log_path_for(root, key):
    return os.path.join(root, "target", "gate", "logs", key.replace("/", ".") + ".log")


def run_target(cargo_bin, root, jobs, target):
    log_path = log_path_for(root, target["key"])
    args = list(target["cargo_args"]) + ["--no-fail-fast"]
    exit_code, out, seconds = run_cargo(cargo_bin, "test", args, root, jobs=jobs, log_path=log_path)
    parsed = parse_test_result(out)
    ok = exit_code == 0 and parsed is not None
    rec = {
        "exit": exit_code,
        "passed": parsed["passed"] if parsed else 0,
        "failed": parsed["failed"] if parsed else 0,
        "ignored": parsed["ignored"] if parsed else 0,
        "result_line": parsed["result_line"] if parsed else "",
        "seconds": round(seconds, 2),
        "finished_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "ok": ok,
        # Debug-only fields, not used for the verdict.
        "tests_actually_ran": tests_actually_ran(out),
        "rebuilt_crates": rebuilt_crates(out),
    }
    if ok:
        print(f"ok  {target['key']}  {rec['passed']} passed  {rec['seconds']}s")
    else:
        print(f"FAIL {target['key']}  exit={exit_code}")
        print(tail_of_interest(out))
        print(f"    rerun: cargo test {' '.join(target['cargo_args'])} --no-fail-fast")
    return rec


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--fresh", action="store_true", help="discard saved state unconditionally")
    ap.add_argument("--tests-only", action="store_true",
                     help="skip build/fmt/clippy; requires them to have passed for this tree")
    ap.add_argument("--only", help="restrict to one package; verdict becomes partial")
    ap.add_argument("--list", action="store_true", help="print target keys and exit")
    ap.add_argument("--floor", type=int, default=None,
                     help="minimum target count required (default 90, or the "
                          "package's own target count when --only is given)")
    ap.add_argument("--jobs", type=int, default=None, help="passed as -j N to cargo")
    ap.add_argument("--cargo", default="cargo", help="cargo binary to use")
    ap.add_argument("--root", default=None, help="repo root (default: parent of dev/)")
    args = ap.parse_args()

    root = os.path.abspath(args.root) if args.root else default_root()

    all_targets = enumerate_targets(args.cargo, root)

    known_pkgs = sorted({t["pkg"] for t in all_targets})
    if args.only is not None and args.only not in known_pkgs:
        print(
            f"error: unknown package {args.only!r} for --only. "
            f"known packages: {', '.join(known_pkgs)}",
            file=sys.stderr,
        )
        sys.exit(2)

    targets = [t for t in all_targets if args.only is None or t["pkg"] == args.only]

    # The --only floor is the target count for that package BEFORE any
    # state-based filtering (i.e. this count, not "how many are left to
    # run") -- a legitimate `--only PKG` always has at least one target, so
    # 0 is always an enumeration error, never a vacuous pass.
    floor = args.floor
    if floor is None:
        floor = 90 if args.only is None else len(targets)
    if len(targets) < floor or (args.only is not None and len(targets) == 0):
        print(
            f"enumeration yielded only {len(targets)} target(s), floor is {floor} "
            f"-- a mechanism that decides what runs must fail loudly when it decides "
            f"nothing does",
            file=sys.stderr,
        )
        sys.exit(2)

    if args.list:
        for t in targets:
            print(t["key"])
        return 0

    stamp = compute_stamp(root)
    state = load_state(root)
    if args.fresh:
        state = None
        print("--fresh: discarding any saved state")
    elif state is not None and state.get("stamp") != stamp:
        print("tree changed since the last run: starting fresh")
        state = None

    if state is None:
        state = {"stamp": stamp, "steps": {}, "targets": {}}

    if args.tests_only:
        steps = state.get("steps", {})
        if not (steps.get("build", {}).get("ok") and steps.get("fmt", {}).get("ok")
                and steps.get("clippy", {}).get("ok")):
            print(
                "--tests-only but build/fmt/clippy have not passed for this tree",
                file=sys.stderr,
            )
            return 1
    else:
        for name, subcommand, cargo_args in (
            ("build", "build", ["--workspace"]),
            ("fmt", "fmt", ["--check"]),
            ("clippy", "clippy", ["--workspace", "--all-targets", "--", "-D", "warnings"]),
        ):
            log_path = os.path.join(root, "target", "gate", "logs", f"{name}.log")
            exit_code, out, seconds = run_cargo(
                args.cargo, subcommand, cargo_args, root,
                jobs=args.jobs if name != "fmt" else None, log_path=log_path,
            )
            state.setdefault("steps", {})[name] = {
                "ok": exit_code == 0, "exit": exit_code, "seconds": round(seconds, 2),
            }
            print(f"{name}: exit {exit_code} ({round(seconds, 2)}s)")
            save_state(root, state)

    for t in targets:
        prior = state.get("targets", {}).get(t["key"])
        if prior and prior.get("ok"):
            print(f"skip {t['key']} (passed in previous run)")
            continue
        rec = run_target(args.cargo, root, args.jobs, t)
        state.setdefault("targets", {})[t["key"]] = rec
        save_state(root, state)

    # Verdict.
    by_pkg = {}
    for t in targets:
        rec = state["targets"].get(t["key"], {})
        pk = by_pkg.setdefault(t["pkg"], {"targets": 0, "passed": 0, "failed": 0})
        pk["targets"] += 1
        pk["passed"] += rec.get("passed", 0)
        pk["failed"] += rec.get("failed", 0)

    print("\nPer-package summary:")
    for pkg in sorted(by_pkg):
        pk = by_pkg[pkg]
        print(f"  {pkg}: {pk['targets']} targets, {pk['passed']} passed, {pk['failed']} failed")

    missing_or_failed = [t["key"] for t in targets if not state["targets"].get(t["key"], {}).get("ok")]
    steps_ok = args.tests_only or all(
        state.get("steps", {}).get(n, {}).get("ok") for n in ("build", "fmt", "clippy")
    )

    partial_note = f" (partial: only {args.only})" if args.only else ""
    print(f"\n{len(targets)} targets{partial_note}")

    if steps_ok and not missing_or_failed and len(targets) >= floor:
        # "GATE PASSED" must never appear on a line that doesn't itself say
        # whether the run was partial -- a bare "GATE PASSED" reads the same
        # whether it covered 101 targets or an --only slice of 2.
        if args.only:
            print(f"GATE PASSED (partial: only {args.only})")
        else:
            print(f"GATE PASSED ({len(targets)} targets)")
        return 0

    print("GATE FAILED")
    if not steps_ok:
        for n in ("build", "fmt", "clippy"):
            st = state.get("steps", {}).get(n, {})
            if not st.get("ok"):
                print(f"  step failed: {n} (exit {st.get('exit')})")
    if missing_or_failed:
        print("  missing or failed targets:")
        for key in missing_or_failed:
            t = next(x for x in targets if x["key"] == key)
            print(f"    {key} -- rerun: cargo test {' '.join(t['cargo_args'])} --no-fail-fast")
    return 1


if __name__ == "__main__":
    sys.exit(main())
