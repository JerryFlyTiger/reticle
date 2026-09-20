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

9. **A kill (SIGKILL, OOM) between "write the mutated content" and "restore
   the backup" leaves a file mutated on disk with no record anywhere of
   which file or which entry** — M64/M65/M67/M55/M143 all hit variants of
   this (see comments in `dev/mutations/m65.py:19-20`, `m66.py:18-19`,
   `m67.py:28-29`, `m55.py:28`). The old per-run `tempfile.mkdtemp` backup
   directory made it worse: it evaporates when the process dies (a fresh
   `mkdtemp` on the next run has no idea the old one ever existed), so the
   next run just silently ran against a mutated tree. This tool now keeps a
   durable journal at `target/mutate/journal.json` (pid, config path, every
   selected file's backup path and pristine sha256, and which single entry
   is currently "active"), written atomically (`os.replace`) before touching
   any file and updated atomically around each mutation. On startup, a
   leftover journal is inspected before anything else runs: a live
   `mutate.py` process owning it means refuse outright (don't run two
   copies against the same tree); otherwise each journaled file is checked
   against its pristine hash (already clean -> just `touch`) or against
   "pristine with the active entry's `old` replaced by `new`" (mid-mutation
   -> restore from backup, `touch`, and say so); anything matching neither
   is left alone on disk and reported with both paths and a `diff -u`
   command, keeping the journal so nothing is silently guessed at.
   `--recover-only` does just this step. Normal completion (including
   exceptions caught by the outer `finally`) deletes the journal and
   backups — a leftover journal always means the previous run didn't get
   to finish cleanly. A journal.json that exists but can't be read or
   parsed is never treated as "no journal" (that once let an unrelated run
   silently delete an earlier killed run's journal and backups while a
   product file was still sitting mutated on disk) — it refuses to run at
   all, on `--recover-only` and a normal run alike, and leaves the journal
   and backups exactly as found.

10. **A stale or misspelled entry is cheaper to catch before cargo runs than
    after.** The per-entry `hits != 1` check (lesson 3) only fires *during*
    the loop, so a bad entry near the end of a long list still burns every
    earlier item's full cargo run before anyone finds out — and a misspelt
    `test` filter (a `NOTESTS` verdict, lesson 6) is invisible until the
    whole thing has already run. `--preflight-only` (and an unconditional
    preflight pass before the baseline on every normal run) checks, for
    every selected entry: `file` exists, `old` occurs exactly once, `old !=
    new`, and — when `test` is set — that it's a substring of at least one
    `fn` name, `mod` name, or `mod::fn` path (cargo matches the
    module-qualified path, not the bare `fn` name — see
    `candidate_test_names`) in the entry's resolved test source (package ->
    crate directory, `"lib"` target -> `src/**/*.rs`, a named target ->
    `tests/<target>.rs`, no target -> every `tests/*.rs` in that crate). All
    problems are collected and printed together, and nothing runs (no
    cargo, no file write) if there are any. The old in-loop `hits != 1`
    check stays too, as a second line of defence — a file can still change
    between preflight and its own turn in the loop.

Usage
    dev/mutate.py --config dev/mutations/m51.py

    # change crate / test target
    dev/mutate.py --config dev/mutations/m51.py -p core --test-target lsp_highlight_tests

    # only run certain items in the list (match by label prefix)
    dev/mutate.py --config dev/mutations/m51.py --only M1 --only N3

    # check the whole list without running any cargo command (recovery of a
    # leftover journal, if any, still runs first as always; see lesson 10)
    dev/mutate.py --config dev/mutations/m51.py --preflight-only

    # recover a journal left by a killed run, without starting a new one
    dev/mutate.py --recover-only

    # point at a different repo root (used by dev/test_mutate.py against a
    # throwaway repo; same spelling as dev/gate.py's --root)
    dev/mutate.py --config dev/mutations/m51.py --root /tmp/some-other-repo

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
    needs_rebuild default True; set False for a file the test reads at RUN
           time rather than one compiled into the binary (`demo/*.sv', a
           `verible.filelist', a shell script). For those, "cargo rebuilt
           nothing" is the normal case, not the danger case, and the NOBUILD
           guard below would report an untrustworthy verdict for an item
           whose mutation demonstrably did reach the test. Added on M122,
           whose list is the first to mutate demo material; before it, such
           an entry's verdict depended on whether the PREVIOUS item happened
           to restore a compiled source and so force a rebuild — which is
           luck, not evidence, in both directions.

Optional `PACKAGE` / `TEST_TARGET` string variables act as defaults, and an
individual entry may override either with its own `"package"` /
`"test_target"` key when one fix's guard lives in a different test binary
from the rest of the list. Before that override existed, such a key was
silently ignored and the entry ran against the wrong binary — `tests_actually_ran`
caught it as an invocation error rather than a fake verdict, but only because
the test name happened not to exist there. Precedence runs entry key >
command line > config default: an entry that names its own target is
describing where its guard actually lives, so `--test-target` cannot
override it (that is what lets one invocation run a list whose entries
span two binaries). Note the fallback is on the key being *absent* --
writing `"package": None` explicitly means "no -p", not "use the
default". The config file is executed (`exec`), so only put your own
files there.
"""

import argparse
import glob
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from typing import NoReturn

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


# --- crash-recovery journal (header lesson 9) -------------------------------


def mutate_dir():
    return os.path.join(REPO, "target", "mutate")


def journal_path():
    return os.path.join(mutate_dir(), "journal.json")


def backup_root():
    return os.path.join(mutate_dir(), "backup")


def backup_filename(index, relpath):
    """Collision-free backup filename for a journaled file's pristine copy.

    `relpath.replace("/", "__")` alone is not collision-free -- `a/b.rs` and
    `a__b.rs` would both map to `a__b.rs`. Prefixing with this file's index
    among the sorted, deduplicated list of selected files guarantees
    uniqueness regardless of what the paths themselves look like. Exposed
    (not just inlined in `main`) so `dev/test_mutate.py` can predict a
    backup's path the same way `dev/gate.py` already imports pure helpers
    from this module instead of reimplementing them.
    """
    return f"{index:04d}__{relpath.replace('/', '__')}"


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def write_journal(state):
    d = mutate_dir()
    os.makedirs(d, exist_ok=True)
    path = journal_path()
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=2, sort_keys=True)
    os.replace(tmp, path)


def validate_journal_shape(data):
    """Check that a parsed journal dict has the shape recover() and main()
    expect it to have. Returns a list of human-readable problem strings
    (empty means clean) -- same style as preflight()'s problem list.

    A journal that parses fine as JSON but has the wrong shape is a second
    way into the same disaster load_journal()'s read/parse guard exists to
    prevent (a cold review finding on top of that guard): `{}` -- or any
    dict missing a usable `"files"` mapping -- used to be accepted outright,
    `journal.get("files", {}).items()` then iterated nothing, recover()
    concluded every file was already pristine, and the whole target/mutate/
    directory (including the real backup of a file still sitting mutated on
    disk) got deleted. Exit 0, no warning. A dict-shaped journal with
    garbage *inside* it (a list where "files" should be, a file entry
    missing "backup", a non-integer "pid", a non-dict "active") used to
    crash with a raw traceback instead of refusing cleanly. This function is
    the one place that decides "is this shape usable", so both failure modes
    go through the same refuse-and-keep-everything path as an unparseable
    journal.

    Deliberately does not validate more than recover()/main() actually read
    (e.g. it doesn't require "pristine_sha256" per file, or "config",
    "started_at") -- tightening beyond what's used would make an otherwise
    fine journal refuse for no operational reason.
    """
    problems = []
    files = data.get("files")
    if not isinstance(files, dict):
        problems.append(
            f"'files' must be an object mapping relpath -> entry (got "
            f"{'missing' if 'files' not in data else type(files).__name__})"
        )
    else:
        for relpath, info in files.items():
            if not isinstance(info, dict):
                problems.append(f"files[{relpath!r}] must be an object (got {type(info).__name__})")
                continue
            if not isinstance(info.get("backup"), str):
                problems.append(f"files[{relpath!r}] is missing a string 'backup' path")
    pid = data.get("pid")
    # bool is a subclass of int in Python (isinstance(True, int) is True), so
    # a JSON boolean would otherwise pass this check outright.
    if pid is not None and (not isinstance(pid, int) or isinstance(pid, bool)):
        problems.append(f"'pid' must be an integer or null/absent (got {type(pid).__name__})")
    elif pid is not None and not (-(2**31) <= pid <= 2**31 - 1):
        # is_mutate_process_alive() passes this straight to os.kill(pid, 0).
        # On this project's platform (macOS), that pid goes through a
        # 32-bit pid_t, not a 64-bit C long -- a value outside that range
        # raises OverflowError, which is not an OSError and used to escape
        # the os.kill() call's except clause as a raw traceback. Measured
        # directly on this machine: os.kill(2147483647, 0) (2**31-1) raises
        # only ProcessLookupError; os.kill(2147483648, 0) (2**31) raises
        # OverflowError("signed integer is greater than maximum");
        # os.kill(-2147483648, 0) (-2**31) is fine, os.kill(-2147483649, 0)
        # overflows the other way. Refusing here, at shape-validation time,
        # keeps that check the single place that decides "usable shape",
        # same as every other field.
        problems.append(f"'pid' is out of range for a process id (got {pid})")
    active = data.get("active")
    if active is not None:
        if not isinstance(active, dict):
            problems.append(f"'active' must be null/absent or an object (got {type(active).__name__})")
        else:
            missing = [k for k in ("label", "file", "old", "new") if k not in active]
            if missing:
                problems.append(f"'active' is missing key(s): {', '.join(missing)}")
            else:
                # All four keys are required to be strings for shape
                # consistency, but only "old"/"new" would actually crash:
                # recover() does string-only operations on them
                # (pristine.count, str.replace), so a non-string value there
                # used to crash with a raw TypeError traceback instead of
                # refusing cleanly. "label" is only interpolated into a
                # message and "file" is only compared with `==`, so neither
                # would crash on its own -- they're still required here so a
                # malformed "active" is caught as one shape problem rather
                # than surfacing piecemeal later.
                non_str = [k for k in ("label", "file", "old", "new") if not isinstance(active[k], str)]
                if non_str:
                    problems.append(f"'active' has non-string value(s) for key(s): {', '.join(non_str)}")
    return problems


def load_journal():
    """Return the parsed journal, or None if there plainly isn't one.

    "No journal" (file absent) is the only case this returns None for. A
    journal.json that exists but can't be read or parsed, or that parses
    fine but has the wrong shape (see validate_journal_shape), is never
    treated as "nothing to recover" -- doing that once let a later,
    unrelated run silently delete an earlier killed run's journal and
    backups while a product file was still sitting mutated on disk (found
    by a cold reviewer, twice: once for "can't be read at all", once for
    "reads fine but the shape is wrong"). Instead this calls die() directly
    and never returns, so every caller -- a normal run and --recover-only
    alike -- stops before touching anything under target/mutate/.
    """
    path = journal_path()
    if not os.path.exists(path):
        return None
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError, UnicodeDecodeError) as e:
        # UnicodeDecodeError is a ValueError subclass, not an OSError -- its
        # own except clause. It can't come from this tool's own writes:
        # write_journal() writes to a temp file and os.replace()s it into
        # place, which is atomic, so an interrupted run of this tool can't
        # leave a torn journal.json. Realistic causes are external: disk
        # corruption, a hand edit, or another program writing that path.
        die(2, f"mutate.py: {path} exists but could not be read as the "
               f"journal ({type(e).__name__}: {e}). Not touching anything -- "
               f"journal kept at {path}, backups kept under {backup_root()} "
               f"(each backup filename encodes the original file's "
               f"repo-relative path, e.g. 0000__src__mod.rs is src/mod.rs, so "
               f"you can restore by hand). Inspect and fix or remove the "
               f"journal yourself before running mutate.py again.")
    if not isinstance(data, dict):
        die(2, f"mutate.py: {path} parsed as JSON but is not an object (got "
               f"{type(data).__name__}) -- not a valid journal. Not touching "
               f"anything -- journal kept at {path}, backups kept under "
               f"{backup_root()} (each backup filename encodes the original "
               f"file's repo-relative path, e.g. 0000__src__mod.rs is "
               f"src/mod.rs, so you can restore by hand). Inspect and fix or "
               f"remove the journal yourself before running mutate.py again.")
    problems = validate_journal_shape(data)
    if problems:
        die(2, f"mutate.py: {path} parsed as JSON but has an invalid shape for "
               f"a journal -- not touching anything:\n"
               + "\n".join(f"  - {p}" for p in problems) +
               f"\njournal kept at {path}, backups kept under {backup_root()} "
               f"(each backup filename encodes the original file's "
               f"repo-relative path, e.g. 0000__src__mod.rs is src/mod.rs, so "
               f"you can restore by hand). Inspect and fix or remove the "
               f"journal yourself before running mutate.py again.")
    return data


def is_mutate_process_alive(pid):
    """Is `pid` alive, and does its command line look like this tool.

    We don't just check liveness: a killed run's pid can be recycled by an
    unrelated process, and refusing to run because *something* now has that
    pid would be a false "still active" forever. `ps -o command=` gives the
    full command line on this project's platform (macOS); if `ps` itself is
    unavailable we can't confirm identity, so we conservatively treat that
    as "not confirmed to be mutate.py" rather than refuse to run forever.
    """
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except OSError:
        # Alive but owned by someone else (or another odd errno) — still try
        # to read its command line below rather than assuming either way.
        pass
    try:
        out = subprocess.run(
            ["ps", "-p", str(pid), "-o", "command="],
            capture_output=True, text=True,
        ).stdout
    except FileNotFoundError:
        return False
    return "mutate.py" in out


def die(code, msg) -> NoReturn:
    print(msg, file=sys.stderr)
    sys.exit(code)


def recover(journal, continuing):
    """Handle a journal left behind by a previous run (header lesson 9).

    On an unrecoverable state this exits the process directly (code 2) and
    never returns. On success it deletes the journal and backups and
    returns normally, so the caller can proceed with a fresh run (or, for
    `--recover-only`, just report and stop).
    """
    pid = journal.get("pid")
    if pid is not None and is_mutate_process_alive(pid):
        die(2, f"mutate.py: a previous run (pid {pid}) still appears to be active "
               f"(journal at {journal_path()}) -- refusing to run alongside it. "
               f"If pid {pid} is confirmed to not be mutate.py, remove the stale "
               f"journal directory by hand: {mutate_dir()}")

    active = journal.get("active")
    problems = []
    notes = []
    for relpath, info in journal.get("files", {}).items():
        path = os.path.join(REPO, relpath)
        bak = info["backup"]
        if not os.path.exists(path):
            problems.append((relpath, bak, "file no longer exists on disk"))
            continue
        try:
            already_pristine = sha256_of(path) == info.get("pristine_sha256")
        except OSError as e:
            # A journaled relpath that resolves to a directory
            # (IsADirectoryError) or an unreadable file (PermissionError) --
            # both OSError subclasses -- used to raise straight out of this
            # line instead of being treated as the "matches neither" case
            # below (the active-entry branch and main()'s end-of-run
            # pristine check already guard the same kind of read; this was
            # the odd one out). Don't touch it, report it, keep going.
            problems.append((relpath, bak, f"cannot read to check against "
                              f"pristine: {e}"))
            continue
        if already_pristine:
            os.utime(path, None)
            notes.append(f"    touched (already pristine): {relpath}")
            continue
        if active and active.get("file") == relpath:
            try:
                pristine = open(bak, encoding="utf-8").read()
                cur = open(path, encoding="utf-8").read()
            except (OSError, UnicodeDecodeError) as e:
                # A missing/unreadable backup (OSError) or a backup/current
                # file that isn't valid UTF-8 (UnicodeDecodeError -- e.g. one
                # truncated mid-copy by an interrupted run; it's a ValueError
                # subclass, not an OSError, so it needs its own arm) must not
                # raise out of this function (that would be a raw traceback
                # and exit 1, indistinguishable from a crash) -- it's exactly
                # the "matches neither" case: nothing to safely compare
                # against, so don't touch the file, report it, and keep the
                # journal.
                problems.append((relpath, bak, f"cannot read backup or current "
                                  f"file to check against the active entry: {e}"))
                continue
            if pristine.count(active["old"]) == 1 and cur == pristine.replace(
                active["old"], active["new"], 1
            ):
                restore(bak, path)
                notes.append(
                    f"    RESTORED {relpath} (left mutated by a killed run at "
                    f"entry {active['label']})"
                )
                continue
        problems.append((relpath, bak, "current content matches neither the "
                          "pristine backup nor the expected mid-mutation state"))

    if problems:
        print(f"mutate.py: recovery found {len(problems)} file(s) left in an "
              f"unrecognized state by a killed run -- NOT overwriting; journal "
              f"and backups kept at {mutate_dir()}:", file=sys.stderr)
        for relpath, bak, why in problems:
            path = os.path.join(REPO, relpath)
            print(f"  {relpath}: {why}", file=sys.stderr)
            print(f"    current: {path}", file=sys.stderr)
            print(f"    backup:  {bak}", file=sys.stderr)
            print(f"    diff -u {bak} {path}", file=sys.stderr)
        sys.exit(2)

    # Cold-review finding: an empty (or partial) "files" mapping is
    # shape-valid, so the loop above can conclude "nothing to recover" while
    # real backup files still sit under backup_root() unreferenced by any
    # entry -- e.g. `{"files": {}}` with a genuine mutated file and its
    # backup left by a killed run. Before deleting anything, cross-check the
    # backups actually on disk against what this journal references; if any
    # are orphaned, refuse and keep everything rather than silently erasing
    # the only record of how to fix the file they belong to. This check is
    # deliberately placed only here (recover()'s own cleanup), not in
    # main()'s end-of-run cleanup -- there, the backups under backup_root()
    # are exactly the ones *this run* just created and journaled, so that
    # path must not refuse on its own honest output.
    referenced = {
        os.path.abspath(info["backup"])
        for info in journal.get("files", {}).values()
        if isinstance(info, dict) and isinstance(info.get("backup"), str)
    }
    if os.path.isdir(backup_root()):
        try:
            names = os.listdir(backup_root())
        except OSError as e:
            # Every other filesystem read in recover() is guarded (sha256_of,
            # the active-entry open() calls) -- this os.listdir() was the odd
            # one out, and an unreadable backup_root() (e.g. permissions)
            # used to raise straight out of here as a raw traceback. Refuse
            # the same way the other problems in this function do: don't
            # delete anything, name the directory and the error, keep the
            # journal.
            print(f"mutate.py: cannot list {backup_root()} to check for orphaned "
                  f"backups ({type(e).__name__}: {e}) -- NOT deleting anything; "
                  f"journal and backups kept at {mutate_dir()}.", file=sys.stderr)
            sys.exit(2)
        on_disk = {
            os.path.abspath(os.path.join(backup_root(), name))
            for name in names
        }
        orphaned = sorted(on_disk - referenced)
        if orphaned:
            print(f"mutate.py: {len(orphaned)} backup file(s) under {backup_root()} are not "
                  f"referenced by any entry in this journal -- NOT deleting anything; journal "
                  f"and backups kept at {mutate_dir()}:", file=sys.stderr)
            for o in orphaned:
                print(f"  {o}", file=sys.stderr)
            sys.exit(2)

    print("recovery: " + ("\n".join(notes) if notes else "    nothing to recover "
                           "(journal present but every file already pristine)"))
    shutil.rmtree(mutate_dir(), ignore_errors=True)
    print("recovery: journal and backups cleared"
          + (", continuing with the requested run" if continuing else ""))


# --- preflight (header lesson 10) -------------------------------------------

FN_RE = re.compile(r"\bfn\s+(\w+)")
MOD_RE = re.compile(r"\bmod\s+(\w+)")


def candidate_test_names(text):
    """Every string cargo's `test` substring filter could plausibly match
    against a test in this source file.

    cargo matches against the module-qualified path (`mod_name::fn_name`),
    not the bare `fn` name — a `test` entry naming a `mod` (or a
    `mod::fn` path) is completely ordinary and must not be flagged. Real
    nesting-depth tracking (which `mod` a given `fn` is lexically inside)
    needs a brace-depth scan; instead this takes the cheaper
    over-approximation of every `mod` name joined with every `fn` name in
    the same file, plus the bare names themselves. Over-approximating is
    the safe direction here (this check exists to catch misspellings, so
    erring toward "accept" is fine) — see the M146 spec.
    """
    fn_names = FN_RE.findall(text)
    mod_names = MOD_RE.findall(text)
    names = set(fn_names) | set(mod_names)
    for mod_name in mod_names:
        for fn_name in fn_names:
            names.add(f"{mod_name}::{fn_name}")
    return names


def resolve_test_source(pkg, target):
    """Return the list of source file paths a `test` filter should live in.

    Raises ValueError (message names what was tried) if nothing resolves.
    Package -> crate directory (root package `"reticle"` or None -> repo
    root); target `"lib"` -> `src/**/*.rs`; a named target ->
    `tests/<target>.rs`; None/"" -> every `tests/*.rs` in that crate.
    """
    crate_dir = REPO if pkg in (None, "reticle") else os.path.join(REPO, "crates", pkg)
    if not os.path.isdir(crate_dir):
        raise ValueError(f"crate directory not found: {crate_dir} (package={pkg!r})")
    if target == "lib":
        src_dir = os.path.join(crate_dir, "src")
        paths = glob.glob(os.path.join(src_dir, "**", "*.rs"), recursive=True)
        if not paths:
            raise ValueError(f"no source files under {src_dir}")
        return paths
    if target:
        path = os.path.join(crate_dir, "tests", f"{target}.rs")
        if not os.path.isfile(path):
            raise ValueError(f"test file not found: {path}")
        return [path]
    tests_dir = os.path.join(crate_dir, "tests")
    paths = glob.glob(os.path.join(tests_dir, "*.rs"))
    if not paths:
        raise ValueError(f"no test files under {tests_dir}")
    return paths


def preflight(mutations, package, target):
    """Check every selected entry before touching anything. Returns a list
    of human-readable problem strings (empty means clean)."""
    problems = []
    for m in mutations:
        label = m["label"]
        path = os.path.join(REPO, m["file"])
        if not os.path.isfile(path):
            problems.append(f"{label}: file not found: {m['file']}")
            continue
        src = open(path, encoding="utf-8").read()
        hits = src.count(m["old"])
        if hits != 1:
            problems.append(f"{label}: old occurs {hits} time(s) in {m['file']} "
                             "(need exactly 1)")
        if m["old"] == m["new"]:
            problems.append(f"{label}: old and new are identical")
        if m.get("test"):
            pkg = m.get("package", package)
            tgt = m.get("test_target", target)
            try:
                paths = resolve_test_source(pkg, tgt)
            except ValueError as e:
                problems.append(f"{label}: cannot resolve test source for "
                                 f"test={m['test']!r} (package={pkg!r}, "
                                 f"test_target={tgt!r}): {e}")
                continue
            names = set()
            for p in paths:
                names |= candidate_test_names(open(p, encoding="utf-8", errors="replace").read())
            if not any(m["test"] in name for name in names):
                problems.append(f"{label}: test substring {m['test']!r} not found "
                                 f"in any fn/mod name (or mod::fn path) under "
                                 f"{', '.join(sorted(paths))}")
    return problems


def load_config(path):
    ns = {}
    with open(path, encoding="utf-8") as f:
        exec(compile(f.read(), path, "exec"), ns)  # noqa: S102 - our own config file
    if "MUTATIONS" not in ns:
        sys.exit(f"{path} does not define MUTATIONS")
    return ns["MUTATIONS"], ns.get("PACKAGE"), ns.get("TEST_TARGET")


def cargo_test(package, target, filter_str=None, timeout=None, cargo_bin="cargo"):
    cmd = [cargo_bin, "test"]
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
    ap.add_argument("--config", help="mutation list (a Python file); required unless --recover-only")
    ap.add_argument("-p", "--package", help="crate name for cargo -p")
    ap.add_argument("--test-target", help="test file name for cargo --test")
    ap.add_argument("--only", action="append", default=[],
                    help="only run items whose label starts with this, repeatable")
    ap.add_argument("--skip-baseline", action="store_true",
                    help="skip the baseline check (not recommended; see header lesson 2)")
    ap.add_argument("--timeout", type=float, default=600.0,
                    help="per-item cap in seconds for cargo test (default 600). "
                         "can be overridden per item via the timeout field")
    ap.add_argument("--preflight-only", action="store_true",
                    help="check the whole selected list and exit (see header lesson 10); "
                         "recovery of a leftover journal, if any, still runs first as "
                         "on every invocation (see header lesson 9) -- after that, this "
                         "runs no cargo command and touches no other file")
    ap.add_argument("--recover-only", action="store_true",
                    help="only recover a journal left by a killed run (see header lesson 9), then exit")
    ap.add_argument("--root", help="repo root (default: parent of dev/); same spelling as dev/gate.py")
    ap.add_argument("--cargo", default="cargo", help="cargo binary to use (dev/test_mutate.py "
                    "points this at dev/fake-cargo.py)")
    args = ap.parse_args()

    global REPO
    if args.root:
        REPO = os.path.abspath(args.root)

    # Safety net (M146 fix round item 3): load_journal() and recover() each
    # carry specific guards for every input problem found so far (a corrupt
    # journal, a wrong shape, an unreadable backup, ...), and those specific
    # guards are not retired by this net -- they give a precise, named
    # reason, this net only gives a generic one. But successive review
    # rounds each found one more thing nobody had thought of yet (most
    # recently: an `active["backup"]` containing an embedded null byte,
    # which raises ValueError -- not an OSError -- straight out of
    # recover()'s `open(bak, ...)` call). Rather than keep adding guards one
    # crash report at a time, catch anything from this phase that is not
    # already a deliberate SystemExit (SystemExit must pass through
    # untouched -- that is how die() and sys.exit(2) communicate a refusal)
    # and refuse the same way: nothing touched, name where the journal and
    # backups are kept, exit 2 instead of a raw traceback and exit 1.
    try:
        journal = load_journal()
        if journal is not None:
            recover(journal, continuing=not args.recover_only)
        elif args.recover_only:
            print(f"no journal found at {journal_path()} -- nothing to recover")
    except SystemExit:
        raise
    except Exception as e:
        die(2, f"mutate.py: unexpected error during startup recovery "
               f"({type(e).__name__}: {e}) -- nothing was touched. journal "
               f"kept at {journal_path()}, backups kept under {backup_root()} "
               f"(each backup filename encodes the original file's "
               f"repo-relative path, e.g. 0000__src__mod.rs is src/mod.rs, so "
               f"you can restore by hand). Inspect and fix or remove the "
               f"journal yourself before running mutate.py again.")

    if args.recover_only:
        return 0

    if not args.config:
        ap.error("--config is required unless --recover-only")

    mutations, cfg_pkg, cfg_target = load_config(args.config)
    package = args.package or cfg_pkg
    target = args.test_target or cfg_target

    if args.only:
        mutations = [m for m in mutations
                     if any(m["label"].startswith(p) for p in args.only)]
        if not mutations:
            sys.exit("--only did not match any items")

    # Part B (header lesson 10): check every selected entry before running
    # anything. A misspelt `test` filter or a stale `old` is much cheaper to
    # learn about here than after N-1 earlier items have already run cargo.
    problems = preflight(mutations, package, target)
    if args.preflight_only:
        for p in problems:
            print(p)
        print(f"preflight: {len(mutations)} entries, {len(problems)} problems")
        return 0 if not problems else 2
    if problems:
        for p in problems:
            print(p)
        print(f"preflight: {len(mutations)} entries, {len(problems)} problems")
        sys.exit(2)

    # One baseline per distinct (package, target) the list actually uses.
    # A single baseline over the default pair would leave any entry that
    # overrides them unbaselined, which is the same "every conclusion is
    # fake" hole lesson 2 is about, just narrower.
    pairs = []
    for m in mutations:
        pair = (m.get("package", package), m.get("test_target", target))
        if pair not in pairs:
            pairs.append(pair)

    if not args.skip_baseline:
        for bpkg, btgt in pairs:
            print(f"=== baseline (unmodified state) — package={bpkg!r} target={btgt!r} ===")
            code, out = cargo_test(bpkg, btgt, timeout=args.timeout, cargo_bin=args.cargo)
            print(tail_of_interest(out, 3))
            if code == TIMEOUT:
                sys.exit(f"\n!! baseline did not finish within {args.timeout}s. "
                         "Confirm the tests themselves are healthy before running mutations.")
            if code != 0:
                sys.exit("\n!! baseline is not green. Fix it before running mutations — "
                         "otherwise every item will \"FAIL as expected\" and every conclusion "
                         "is fake (see header lesson 2).")
            if not tests_actually_ran(out):
                sys.exit(f"\n!! not a single test ran this round for the baseline (package={bpkg!r}, "
                         f"target={btgt!r}). This workspace's root directory is itself a "
                         "package, so leaving `-p` blank only runs it — when a list spans "
                         "multiple crates, run it in separate passes each with its own -p "
                         "(see header lesson 6).")

    # Part A (header lesson 9): back up every distinct selected file to a
    # fixed, durable location and record it in a journal *before* mutating
    # anything, so a kill mid-run leaves a trail instead of silence.
    distinct_files = sorted({m["file"] for m in mutations})
    bdir = backup_root()
    os.makedirs(bdir, exist_ok=True)
    journal_files = {}
    for i, relpath in enumerate(distinct_files):
        path = os.path.join(REPO, relpath)
        bak = os.path.join(bdir, backup_filename(i, relpath))
        shutil.copy2(path, bak)
        journal_files[relpath] = {"backup": bak, "pristine_sha256": sha256_of(path)}
    journal_state = {
        "pid": os.getpid(),
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "config": args.config,
        "files": journal_files,
        "active": None,
    }
    write_journal(journal_state)

    def set_active(m):
        journal_state["active"] = {
            "label": m["label"], "file": m["file"], "old": m["old"], "new": m["new"],
        }
        write_journal(journal_state)

    def clear_active():
        journal_state["active"] = None
        write_journal(journal_state)

    results = []
    try:
        for m in mutations:
            path = os.path.join(REPO, m["file"])
            bak = journal_files[m["file"]]["backup"]
            try:
                src = open(path, encoding="utf-8").read()
                hits = src.count(m["old"])
                if hits != 1:
                    results.append((m["label"], "SKIP",
                                    f"    original string appears {hits} time(s) (need exactly 1) — "
                                    f"the list may be stale, not applied",
                                    m.get("expect", "FAIL").upper()))
                    continue
                set_active(m)
                with open(path, "w", encoding="utf-8") as f:
                    f.write(src.replace(m["old"], m["new"]))
                t0 = time.monotonic()
                code, out = cargo_test(m.get("package", package),
                                       m.get("test_target", target),
                                       m.get("test"),
                                       timeout=m.get("timeout", args.timeout),
                                       cargo_bin=args.cargo)
                elapsed = time.monotonic() - t0
                built = rebuilt_crates(out)
                if code == TIMEOUT:
                    verdict = "HANG"
                elif code != 0:
                    verdict = "FAIL"
                elif not built and m.get("needs_rebuild", True):
                    # No rebuild = this mutation never made it into the binary
                    # under test, so **no** verdict is trustworthy (including
                    # "FAILed as expected" — that could be failing for an
                    # unrelated reason). So this check comes first, not only
                    # checked for SURVIVED. See header lesson 7.
                    #
                    # `needs_rebuild: False' opts an item out, and only that:
                    # a file the test opens at run time (demo material, a
                    # filelist, a shell script) reaches the test whether or
                    # not cargo did anything.
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
                clear_active()
    finally:
        # Normal completion (including exceptions caught here) usually means
        # the journal and its backups no longer describe an in-progress run,
        # so delete them (header lesson 9). But `restore()` in the per-entry
        # `finally` above can itself raise (PermissionError, ENOSPC, a
        # backup that vanished) and propagate straight through to here --
        # in that case a product file can still be sitting mutated on disk,
        # and deleting the journal+backups now would erase the only record
        # of how to fix it. So only delete once every journaled file is
        # confirmed pristine; otherwise leave the journal for the next run
        # (or `--recover-only`) to pick up.
        unrecovered = []
        for relpath, info in journal_files.items():
            p = os.path.join(REPO, relpath)
            try:
                ok = os.path.exists(p) and sha256_of(p) == info["pristine_sha256"]
            except OSError:
                ok = False
            if not ok:
                unrecovered.append(relpath)
        if unrecovered:
            print(f"mutate.py: {len(unrecovered)} file(s) are not confirmed pristine "
                  f"after this run -- keeping {mutate_dir()} so the next run (or "
                  f"--recover-only) can fix it:", file=sys.stderr)
            for relpath in unrecovered:
                print(f"  {relpath}", file=sys.stderr)
        else:
            shutil.rmtree(mutate_dir(), ignore_errors=True)
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
