"""Tests for dev/mutate.py's crash-recovery journal and preflight (M146) --
plain asserts, discovery by naming convention (same shape as
dev/test_gate.py and dev/test_pixdiff.py: `main` collects every `test_*`
callable off this module's own namespace, sorted by name, and runs it).

Run with: python3 dev/test_mutate.py

Every test drives the real `dev/mutate.py` as a subprocess, pointed (via
`--root`) at a throwaway directory standing in for the repo root, and (for
anything that reaches a cargo invocation) via `--cargo` at
`dev/fake-cargo.py` instead of the real `cargo` -- the same substitution
dev/test_gate.py uses. Nothing about dev/mutate.py itself is mocked.

Requires python3 on PATH (this file is only ever invoked via python3 in the
first place) and, on the platforms this project targets, a `ps` binary
(used by the live-pid test to confirm a real process's command line) --
if `ps` is missing dev/mutate.py's own liveness check degrades to "not
confirmed to be mutate.py" rather than failing, so that one test's
assumption is the only place this matters.
"""

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

DEV_DIR = os.path.dirname(os.path.abspath(__file__))
MUTATE_PY = os.path.join(DEV_DIR, "mutate.py")
FAKE_CARGO_PY = os.path.join(DEV_DIR, "fake-cargo.py")
NONEXISTENT_CARGO = "/nonexistent/cargo-should-not-run-dev-test-mutate"

MINIMUM_TESTS = 35


class TempRepo:
    """A throwaway directory standing in for the repo root. Unlike
    dev/test_gate.py's TempRepo, this needs no git init -- dev/mutate.py,
    unlike dev/gate.py, never shells out to git."""

    def __enter__(self):
        self.dir = tempfile.mkdtemp(prefix="mutate_test_repo_")
        return self.dir

    def __exit__(self, *exc):
        shutil.rmtree(self.dir, ignore_errors=True)
        return False


def write_file(path, content):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(content)


def write_config(path, entries, package=None, test_target=None):
    lines = []
    if package is not None:
        lines.append(f"PACKAGE = {package!r}")
    if test_target is not None:
        lines.append(f"TEST_TARGET = {test_target!r}")
    lines.append(f"MUTATIONS = {entries!r}")
    write_file(path, "\n".join(lines) + "\n")


def run_mutate(root, extra_args, scenario, scratch, env_extra=None):
    env = dict(os.environ)
    if scenario is not None:
        scenario_path = os.path.join(scratch, "scenario.json")
        with open(scenario_path, "w") as f:
            json.dump(scenario, f)
        env["FAKE_CARGO_SCENARIO"] = scenario_path
    if env_extra:
        env.update(env_extra)
    cmd = [sys.executable, MUTATE_PY, "--root", root] + extra_args
    proc = subprocess.run(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return proc.returncode, proc.stdout, proc.stderr


def journal_path(root):
    return os.path.join(root, "target", "mutate", "journal.json")


def mutate_dir(root):
    return os.path.join(root, "target", "mutate")


# --- Part A: crash recovery -------------------------------------------------


def _setup_kill_scenario(root, scratch):
    """A one-entry list whose only cargo invocation gets SIGKILLed via
    fake-cargo's kill_parent outcome -- mimics an OOM kill landing on
    mutate.py itself, mid-mutation. Returns (mod_path, pristine_content)."""
    mod_path = os.path.join(root, "src", "mod.rs")
    pristine = "fn x() {\n    OLD_TOKEN\n}\n"
    write_file(mod_path, pristine)
    cfg_path = os.path.join(scratch, "cfg.py")
    write_config(
        cfg_path,
        [{"label": "M1", "file": "src/mod.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"}],
        package="app", test_target="sometest",
    )
    scenario = {
        "attempts_file": os.path.join(scratch, "attempts.json"),
        "targets": {"app/test/sometest": {"outcome": "kill_parent", "passed": 1, "failed": 0, "ignored": 0}},
    }
    code, out, err = run_mutate(
        root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
    )
    assert code != 0, f"mutate.py should have been killed itself, exit={code}, stdout={out}, stderr={err}"
    with open(mod_path) as f:
        mutated = f.read()
    assert mutated != pristine, "sanity: file must be left mutated after the kill"
    assert os.path.exists(journal_path(root)), "sanity: a journal must have been left behind"
    return mod_path, pristine


def test_kill_mid_entry_then_next_run_restores():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, pristine = _setup_kill_scenario(root, scratch)
        mtime_before = os.stat(mod_path).st_mtime_ns

        code2, out2, err2 = run_mutate(root, ["--recover-only"], None, scratch)
        assert code2 == 0, f"stdout={out2}, stderr={err2}"
        assert "RESTORED" in out2, out2
        assert "src/mod.rs" in out2, out2
        assert "M1" in out2, out2
        assert "left mutated by a killed run at entry" in out2, out2
        with open(mod_path) as f:
            assert f.read() == pristine, "content must be restored to pristine"
        mtime_after = os.stat(mod_path).st_mtime_ns
        assert mtime_after > mtime_before, "mtime must be pushed forward, not just content restored"
        assert not os.path.exists(journal_path(root)), "journal must be cleared after a clean recovery"
        assert not os.path.exists(mutate_dir(root)), "backups must be cleared too"


def test_kill_then_user_edit_blocks_overwrite():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, pristine = _setup_kill_scenario(root, scratch)
        edited = "fn x() { USER_EDITED_THIS_MEANWHILE }\n"
        with open(mod_path, "w") as f:
            f.write(edited)

        code2, out2, err2 = run_mutate(root, ["--recover-only"], None, scratch)
        assert code2 == 2, f"stdout={out2}, stderr={err2}"
        with open(mod_path) as f:
            assert f.read() == edited, "the user's edit must not be overwritten"
        assert os.path.exists(journal_path(root)), "journal must be kept on an unrecoverable state"
        assert "diff -u" in err2, err2
        assert mod_path in err2 or "src/mod.rs" in err2, err2
        assert ("current content matches neither the pristine backup nor "
                "the expected mid-mutation state") in err2, err2
        assert "unexpected error during startup recovery" not in err2, \
            f"must be refused by the specific guard, not the safety net:\n{err2}"


def test_live_pid_refuses():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        # A real, long-lived process whose command line contains "mutate.py"
        # -- dev/mutate.py's liveness check greps `ps -o command=` for that
        # substring, so a script literally named mutate.py stands in for a
        # still-running instance without actually running one.
        fake_bin_dir = os.path.join(scratch, "fakebin")
        os.makedirs(fake_bin_dir, exist_ok=True)
        fake_script = os.path.join(fake_bin_dir, "mutate.py")
        write_file(fake_script, "import time\ntime.sleep(60)\n")
        proc = subprocess.Popen([sys.executable, fake_script])
        try:
            time.sleep(0.3)
            os.makedirs(mutate_dir(root), exist_ok=True)
            with open(journal_path(root), "w") as f:
                json.dump(
                    {"pid": proc.pid, "started_at": "x", "config": "x", "files": {}, "active": None}, f,
                )
            code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
            assert code == 2, f"stdout={out}, stderr={err}"
            assert str(proc.pid) in err, err
            assert "still appears to be active" in err, err
            assert "unexpected error during startup recovery" not in err, \
                f"must be refused by the specific live-pid guard, not the safety net:\n{err}"
            assert os.path.exists(journal_path(root)), "journal must be left alone while the pid is live"
        finally:
            proc.kill()
            proc.wait()


def test_recover_touches_already_pristine_file():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        content = "fn x() {}\n"
        write_file(mod_path, content)
        old_time = time.time() - 1000
        os.utime(mod_path, (old_time, old_time))
        mtime_before = os.stat(mod_path).st_mtime_ns

        bdir = os.path.join(mutate_dir(root), "backup")
        os.makedirs(bdir, exist_ok=True)
        bak = os.path.join(bdir, "src__mod.rs")
        shutil.copy2(mod_path, bak)
        sha = hashlib.sha256(content.encode()).hexdigest()
        with open(journal_path(root), "w") as f:
            json.dump(
                {
                    "pid": 999999999,  # not alive: recovery must not refuse
                    "started_at": "x", "config": "x",
                    "files": {"src/mod.rs": {"backup": bak, "pristine_sha256": sha}},
                    # "active" names a *different* file than the one being
                    # checked here -- this is the "kill landed between
                    # restore and utime" case (header lesson 9 point 4):
                    # content already equals pristine, so the active-entry
                    # branch must never even be consulted.
                    "active": {"label": "M9", "file": "some/other/file.rs", "old": "x", "new": "y"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 0, f"stdout={out}, stderr={err}"
        assert "touched" in out, out
        assert "src/mod.rs" in out, out
        mtime_after = os.stat(mod_path).st_mtime_ns
        assert mtime_after > mtime_before, "already-pristine file must still get touched"
        assert not os.path.exists(journal_path(root))


def test_restore_failure_keeps_journal_and_stays_recoverable():
    # Review-round regression: if restore(bak, path) itself raises (a
    # transient PermissionError writing to `path` here, standing in for
    # ENOSPC or a vanished backup), the exception used to propagate straight
    # through the outer `finally`, which deleted target/mutate/
    # unconditionally -- destroying the only record of how to fix the file
    # it just failed to restore. Uses fake-cargo's `make_readonly` outcome
    # to chmod the *product* file (not its backup) mid-run, so the backup
    # stays intact and the scenario is genuinely recoverable once the
    # "permission problem" clears.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        pristine = "fn x() {\n    OLD_TOKEN\n}\n"
        write_file(mod_path, pristine)
        entries = [{"label": "M1", "file": "src/mod.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"}]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries, package="app", test_target="sometest")
        scenario = {
            "targets": {
                "app/test/sometest": {
                    "outcome": "make_readonly", "readonly_path": mod_path,
                    "passed": 1, "failed": 0, "ignored": 0,
                }
            },
        }
        try:
            code, out, err = run_mutate(
                root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
            )
            assert code != 0, f"mutate.py's own restore() must raise, exit={code}, stdout={out}"
            assert os.path.exists(journal_path(root)), \
                f"journal must survive a failed restore, stdout={out}, stderr={err}"

            # Simulate the transient permission problem clearing before the
            # next run -- the backup was never touched, so recovery must
            # actually be able to fix the file, not just leave the journal
            # sitting there.
            os.chmod(mod_path, 0o644)
            code2, out2, err2 = run_mutate(root, ["--recover-only"], None, scratch)
            assert code2 == 0, f"stdout={out2}, stderr={err2}"
            assert "RESTORED" in out2, out2
            with open(mod_path) as f:
                assert f.read() == pristine
            assert not os.path.exists(journal_path(root))
        finally:
            if os.path.exists(mod_path):
                os.chmod(mod_path, 0o644)


def test_recover_with_missing_backup_does_not_crash():
    # Review-round regression: recover()'s active-entry branch used to do
    # open(bak).read() unguarded -- a missing or unreadable backup raised a
    # raw FileNotFoundError out of the whole tool (traceback, exit 1). It
    # must instead be treated the same as "matches neither": don't touch the
    # product file, report what's missing, keep the journal, exit 2.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        mutated = "fn x() {\n    NEW_TOKEN\n}\n"
        write_file(mod_path, mutated)
        os.makedirs(os.path.join(mutate_dir(root), "backup"), exist_ok=True)
        # Deliberately never created -- the backup this journal points at
        # simply doesn't exist on disk.
        missing_bak = os.path.join(mutate_dir(root), "backup", "0000__src__mod.rs")
        unrelated_sha = hashlib.sha256(b"some other pristine content entirely\n").hexdigest()
        with open(journal_path(root), "w") as f:
            json.dump(
                {
                    "pid": 999999999,
                    "started_at": "x", "config": "x",
                    "files": {"src/mod.rs": {"backup": missing_bak, "pristine_sha256": unrelated_sha}},
                    "active": {"label": "M1", "file": "src/mod.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a missing backup must not crash with a raw traceback:\n{err}"
        assert "src/mod.rs" in err, err
        assert "cannot read backup or current file to check against the active entry" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by the specific guard, not the safety net:\n{err}"
        with open(mod_path) as f:
            assert f.read() == mutated, "file must not be touched when its backup can't be read"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_recover_with_non_utf8_backup_does_not_crash():
    # Review-round regression: recover()'s active-entry branch only caught
    # OSError around the two open(...).read() calls. UnicodeDecodeError is a
    # subclass of ValueError, not OSError, so a backup (or current file) that
    # isn't valid UTF-8 -- e.g. truncated mid-copy by an interrupted run --
    # still escaped as a raw traceback and exit 1, exactly the failure mode
    # the guard exists to prevent. It must be treated the same as "matches
    # neither": don't touch the product file, report it, keep the journal,
    # exit 2.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        mutated = "fn x() {\n    NEW_TOKEN\n}\n"
        write_file(mod_path, mutated)
        os.makedirs(os.path.join(mutate_dir(root), "backup"), exist_ok=True)
        bad_bak = os.path.join(mutate_dir(root), "backup", "0000__src__mod.rs")
        # Invalid UTF-8: a lone continuation byte, as an interrupted
        # shutil.copy2 could leave behind.
        with open(bad_bak, "wb") as f:
            f.write(b"fn x() {\n    OLD_TOK\x80EN\n}\n")
        unrelated_sha = hashlib.sha256(b"some other pristine content entirely\n").hexdigest()
        with open(journal_path(root), "w") as f:
            json.dump(
                {
                    "pid": 999999999,
                    "started_at": "x", "config": "x",
                    "files": {"src/mod.rs": {"backup": bad_bak, "pristine_sha256": unrelated_sha}},
                    "active": {"label": "M1", "file": "src/mod.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a non-UTF-8 backup must not crash with a raw traceback:\n{err}"
        assert "src/mod.rs" in err, err
        assert "cannot read backup or current file to check against the active entry" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by the specific guard, not the safety net:\n{err}"
        with open(mod_path) as f:
            assert f.read() == mutated, "file must not be touched when its backup can't be decoded"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_recover_with_non_utf8_current_file_does_not_crash():
    # Second case of the same defect: the current (mutated-on-disk) file is
    # the one that isn't valid UTF-8, not the backup.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        os.makedirs(os.path.dirname(mod_path), exist_ok=True)
        mutated_bad = b"fn x() {\n    NEW_TOK\x80EN\n}\n"
        with open(mod_path, "wb") as f:
            f.write(mutated_bad)
        os.makedirs(os.path.join(mutate_dir(root), "backup"), exist_ok=True)
        bak = os.path.join(mutate_dir(root), "backup", "0000__src__mod.rs")
        pristine = "fn x() {\n    OLD_TOKEN\n}\n"
        write_file(bak, pristine)
        unrelated_sha = hashlib.sha256(b"some other pristine content entirely\n").hexdigest()
        with open(journal_path(root), "w") as f:
            json.dump(
                {
                    "pid": 999999999,
                    "started_at": "x", "config": "x",
                    "files": {"src/mod.rs": {"backup": bak, "pristine_sha256": unrelated_sha}},
                    "active": {"label": "M1", "file": "src/mod.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a non-UTF-8 current file must not crash with a raw traceback:\n{err}"
        assert "src/mod.rs" in err, err
        assert "cannot read backup or current file to check against the active entry" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by the specific guard, not the safety net:\n{err}"
        with open(mod_path, "rb") as f:
            assert f.read() == mutated_bad, "file must not be touched when it can't be decoded"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_recover_only_with_non_utf8_journal_refuses_not_silently_discards():
    # Same defect one level up: load_journal()'s open(path).read() (via
    # json.load) used to be guarded by except (json.JSONDecodeError, OSError,
    # UnicodeDecodeError) and simply return None -- which main() cannot tell
    # apart from "no journal at all". A corrupt-but-present journal must
    # never be silently treated as "nothing to recover": it must refuse
    # (exit 2), without crashing with a raw traceback, and leave the journal
    # untouched so a human can inspect or hand-restore it.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "wb") as f:
            f.write(b'{"pid": 1, "files": {\x80}}')

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert "Traceback" not in err, f"a non-UTF-8 journal must not crash with a raw traceback:\n{err}"
        assert code == 2, f"a corrupt-but-present journal must refuse, not silently proceed: stdout={out}, stderr={err}"
        assert "could not be read as the journal" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by load_journal()'s own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "the corrupt journal must be left in place, not deleted"


def test_corrupt_journal_syntactically_invalid_json_also_refuses():
    # Same as above but with syntactically invalid JSON that *is* valid
    # UTF-8 -- a different exception (json.JSONDecodeError instead of
    # UnicodeDecodeError) must take the same refusing path.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            f.write('{"pid": 1, "files": {')  # truncated / malformed, but valid UTF-8

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert "Traceback" not in err, f"invalid JSON must not crash with a raw traceback:\n{err}"
        assert code == 2, f"a corrupt-but-present journal must refuse, not silently proceed: stdout={out}, stderr={err}"
        assert "could not be read as the journal" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by load_journal()'s own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "the corrupt journal must be left in place, not deleted"


def test_corrupt_journal_blocks_unrelated_normal_run_and_keeps_its_dir():
    # The reviewer's actual reproduction: a corrupt journal left by an
    # earlier killed run must not be silently discarded by a later,
    # completely unrelated run that never even asked to recover -- that
    # would erase the only record of the earlier run's mutated file. A
    # normal run (different, valid --config, touching a different file)
    # must also refuse before running anything, and target/mutate/ must
    # still exist afterward (today it gets deleted by the final `finally`,
    # because the files *this* run tracked are all pristine).
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "wb") as f:
            f.write(b'{"pid": 1, "files": {\x80}}')

        other_path = os.path.join(root, "other.rs")
        pristine = "fn y() {\n    OLD_TOKEN\n}\n"
        write_file(other_path, pristine)
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(
            cfg_path,
            [{"label": "M1", "file": "other.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"}],
            package="app", test_target="sometest",
        )
        scenario = {
            "attempts_file": os.path.join(scratch, "attempts.json"),
            "targets": {"app/test/sometest": {"outcome": "pass", "passed": 1, "failed": 0, "ignored": 0}},
        }

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
        )
        assert "Traceback" not in err, f"a non-UTF-8 journal must not crash with a raw traceback:\n{err}"
        assert code == 2, f"a corrupt journal must block an unrelated normal run, not be silently discarded: stdout={out}, stderr={err}"
        assert "could not be read as the journal" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by load_journal()'s own guard, not the safety net:\n{err}"
        assert os.path.exists(mutate_dir(root)), "target/mutate/ (journal + backups) must not be deleted"
        assert os.path.exists(journal_path(root)), "the corrupt journal must be left in place, not deleted"
        with open(other_path) as f:
            assert f.read() == pristine, "the unrelated file must never have been touched"


# --- Part B: preflight -------------------------------------------------


def test_preflight_reports_all_problems_together():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        fa = os.path.join(root, "a.rs")
        write_file(fa, "content with NEEDLE_ONE\n")
        fb = os.path.join(root, "b.rs")
        write_file(fb, "TWICE TWICE\n")
        fc = os.path.join(root, "c.rs")
        write_file(fc, "SAME\n")
        write_file(os.path.join(root, "crates", "app", "tests", "sometest.rs"), "fn real_fn() {}\n")
        fd = os.path.join(root, "d.rs")
        write_file(fd, "DTOKEN\n")

        entries = [
            {"label": "A", "file": "a.rs", "old": "MISSING_TOKEN", "new": "X"},
            {"label": "B", "file": "b.rs", "old": "TWICE", "new": "X"},
            {"label": "C", "file": "c.rs", "old": "SAME", "new": "SAME"},
            {"label": "D", "file": "d.rs", "old": "DTOKEN", "new": "X", "test": "no_such_fn_anywhere",
             "package": "app", "test_target": "sometest"},
        ]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries)
        before = {p: open(p, "rb").read() for p in (fa, fb, fc, fd)}

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--preflight-only", "--cargo", NONEXISTENT_CARGO], None, scratch,
        )
        assert code == 2, f"stdout={out}, stderr={err}"
        expect = {"A": "occurs 0 time", "B": "occurs 2 time", "C": "identical", "D": "not found"}
        for label, needle in expect.items():
            lines = [ln for ln in out.splitlines() if ln.startswith(f"{label}:")]
            assert lines, f"missing report line for {label} in:\n{out}"
            assert needle in lines[0], lines[0]
        assert "preflight: 4 entries, 4 problems" in out, out
        for p in (fa, fb, fc, fd):
            assert open(p, "rb").read() == before[p], f"{p} must not have been touched"


def test_preflight_unresolvable_test_target():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        fe = os.path.join(root, "e.rs")
        write_file(fe, "ETOKEN\n")
        entries = [
            {"label": "E", "file": "e.rs", "old": "ETOKEN", "new": "X", "test": "anything",
             "package": "ghost_pkg", "test_target": "whatever"},
        ]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries)

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--preflight-only", "--cargo", NONEXISTENT_CARGO], None, scratch,
        )
        assert code == 2, f"stdout={out}, stderr={err}"
        lines = [ln for ln in out.splitlines() if ln.startswith("E:")]
        assert lines, out
        assert "crate directory not found" in lines[0], lines[0]
        assert "preflight: 1 entries, 1 problems" in out, out


def test_preflight_lib_and_substring_ok():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        write_file(
            os.path.join(root, "crates", "core", "src", "lib.rs"),
            "#[cfg(test)]\nmod tests {\n    fn gc_ext_root_tests() {}\n}\n",
        )
        write_file(os.path.join(root, "crates", "core", "tests", "foo_tests.rs"), "fn truncat_helper() {}\n")
        ff = os.path.join(root, "f.rs")
        write_file(ff, "FTOKEN\n")
        fg = os.path.join(root, "g.rs")
        write_file(fg, "GTOKEN\n")

        entries = [
            {"label": "F", "file": "f.rs", "old": "FTOKEN", "new": "X", "test": "gc_ext_root",
             "package": "core", "test_target": "lib"},
            {"label": "G", "file": "g.rs", "old": "GTOKEN", "new": "X", "test": "truncat",
             "package": "core", "test_target": "foo_tests"},
        ]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries)

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--preflight-only", "--cargo", NONEXISTENT_CARGO], None, scratch,
        )
        assert code == 0, f"stdout={out}, stderr={err}"
        assert "preflight: 2 entries, 0 problems" in out, out


def test_preflight_mod_qualified_test_names_accepted_and_misspelt_rejected():
    # cargo's `test` filter matches the module-qualified path
    # (`mod_name::fn_name`), not the bare `fn` name -- a `test` entry naming
    # just the `mod`, or a full `mod::fn` path, is completely ordinary (see
    # m145.py's H6, which named a `mod`) and must not be flagged, while an
    # actually-misspelt name must still be caught.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        write_file(
            os.path.join(root, "crates", "app", "tests", "hygiene_tests.rs"),
            "mod scan_source_tests {\n    fn detects_missing_gate() {}\n}\n",
        )
        fi = os.path.join(root, "i.rs")
        write_file(fi, "ITOKEN\n")
        fj = os.path.join(root, "j.rs")
        write_file(fj, "JTOKEN\n")
        fk = os.path.join(root, "k.rs")
        write_file(fk, "KTOKEN\n")

        entries = [
            {"label": "I", "file": "i.rs", "old": "ITOKEN", "new": "X", "test": "scan_source_tests",
             "package": "app", "test_target": "hygiene_tests"},
            {"label": "J", "file": "j.rs", "old": "JTOKEN", "new": "X",
             "test": "scan_source_tests::detects_missing_gate",
             "package": "app", "test_target": "hygiene_tests"},
            {"label": "K", "file": "k.rs", "old": "KTOKEN", "new": "X", "test": "totally_bogus_name_xyz",
             "package": "app", "test_target": "hygiene_tests"},
        ]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries)

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--preflight-only", "--cargo", NONEXISTENT_CARGO], None, scratch,
        )
        assert code == 2, f"stdout={out}, stderr={err}"
        assert not [ln for ln in out.splitlines() if ln.startswith("I:")], \
            f"a `test` naming a `mod` must be accepted:\n{out}"
        assert not [ln for ln in out.splitlines() if ln.startswith("J:")], \
            f"a `test` naming a `mod::fn` path must be accepted:\n{out}"
        assert [ln for ln in out.splitlines() if ln.startswith("K:")], \
            f"a genuinely misspelt `test` must still be rejected:\n{out}"
        assert "preflight: 3 entries, 1 problems" in out, out


def test_normal_run_leaves_no_journal():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "h.rs")
        write_file(mod_path, "HTOKEN\n")
        entries = [{"label": "H", "file": "h.rs", "old": "HTOKEN", "new": "HTOKEN2", "needs_rebuild": False}]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries, package="app", test_target="sometest")
        scenario = {"targets": {"app/test/sometest": {"outcome": "fail", "passed": 1, "failed": 1, "ignored": 0}}}

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
        )
        assert code == 0, f"stdout={out}, stderr={err}"  # default expect FAIL, outcome fail -> as expected
        assert not os.path.exists(journal_path(root))
        assert not os.path.exists(mutate_dir(root))
        with open(mod_path) as f:
            assert f.read() == "HTOKEN\n", "file must be restored after a normal run"


def test_backup_names_do_not_collide_across_look_alike_paths():
    # `a/b.rs` and `a__b.rs` flatten to the same name under a bare
    # `replace("/", "__")`. If their backups collide, the second copy
    # overwrites the first, and restoring `a/b.rs` after its entry writes
    # `a__b.rs`'s content into it -- a normal, unkilled run corrupts a file.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        nested = os.path.join(root, "a", "b.rs")
        flat = os.path.join(root, "a__b.rs")
        write_file(nested, "NESTED_TOKEN\n")
        write_file(flat, "FLAT_TOKEN\n")
        entries = [
            {"label": "N", "file": "a/b.rs", "old": "NESTED_TOKEN", "new": "NESTED2", "needs_rebuild": False},
            {"label": "F", "file": "a__b.rs", "old": "FLAT_TOKEN", "new": "FLAT2", "needs_rebuild": False},
        ]
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(cfg_path, entries, package="app", test_target="sometest")
        scenario = {"targets": {"app/test/sometest": {"outcome": "fail", "passed": 1, "failed": 1, "ignored": 0}}}

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
        )
        assert code == 0, f"stdout={out}, stderr={err}"
        with open(nested) as f:
            assert f.read() == "NESTED_TOKEN\n", "a/b.rs must get its own content back"
        with open(flat) as f:
            assert f.read() == "FLAT_TOKEN\n", "a__b.rs must get its own content back"


# --- Part C: journal shape (reviewer finding on M146) -----------------------


def _leave_mutated_state_with_real_backup(root):
    """A tracked file genuinely left mutated on disk, with a real pristine
    backup already sitting under target/mutate/backup/ -- the state a killed
    run leaves. The journal itself is written separately by each test below,
    since the whole point here is to make the *journal* the broken part."""
    mod_path = os.path.join(root, "src", "mod.rs")
    mutated = "fn x() {\n    NEW_TOKEN\n}\n"
    write_file(mod_path, mutated)
    bdir = os.path.join(mutate_dir(root), "backup")
    os.makedirs(bdir, exist_ok=True)
    bak = os.path.join(bdir, "0000__src__mod.rs")
    write_file(bak, "fn x() {\n    OLD_TOKEN\n}\n")
    return mod_path, mutated, bak


def test_journal_empty_dict_refuses_recover_only_and_keeps_backups():
    # Finding 1 (serious): a journal of literally `{}` used to be accepted --
    # journal.get("files", {}).items() iterates nothing, recover() concludes
    # every file is already pristine, and the whole target/mutate/ directory
    # (including the real backup of a file still sitting mutated on disk)
    # gets deleted. Exit 0, no warning.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, mutated, bak = _leave_mutated_state_with_real_backup(root)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            f.write("{}")

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "'files' must be an object mapping relpath -> entry (got missing)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(mutate_dir(root)), "target/mutate/ must not be deleted"
        assert os.path.exists(journal_path(root)), "journal must be kept"
        assert os.path.exists(bak), "the real backup must not be deleted"
        with open(mod_path) as f:
            assert f.read() == mutated, "the mutated file must not be touched"


def test_journal_empty_dict_blocks_unrelated_normal_run_too():
    # Same broken state as above, but reached via a normal run (not
    # --recover-only) with a valid, unrelated --config pointed at a
    # different file -- recovery runs unconditionally before anything else,
    # so this must refuse just the same.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, mutated, bak = _leave_mutated_state_with_real_backup(root)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            f.write("{}")

        other_path = os.path.join(root, "other.rs")
        pristine = "fn y() {\n    OLD_TOKEN\n}\n"
        write_file(other_path, pristine)
        cfg_path = os.path.join(scratch, "cfg.py")
        write_config(
            cfg_path,
            [{"label": "M1", "file": "other.rs", "old": "OLD_TOKEN", "new": "NEW_TOKEN"}],
            package="app", test_target="sometest",
        )
        scenario = {"targets": {"app/test/sometest": {"outcome": "pass", "passed": 1, "failed": 0, "ignored": 0}}}

        code, out, err = run_mutate(
            root, ["--config", cfg_path, "--skip-baseline", "--cargo", FAKE_CARGO_PY], scenario, scratch,
        )
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "'files' must be an object mapping relpath -> entry (got missing)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(mutate_dir(root)), "target/mutate/ must not be deleted"
        assert os.path.exists(bak), "the real backup must not be deleted"


def test_journal_files_is_a_list_refuses_without_traceback():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": 1, "files": [1, 2, 3], "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a list-shaped 'files' must not crash:\n{err}"
        assert "'files' must be an object mapping relpath -> entry (got list)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_file_entry_missing_backup_refuses_without_traceback():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {"pid": 1, "files": {"src/mod.rs": {"pristine_sha256": "abc"}}, "active": None}, f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a file entry missing 'backup' must not crash:\n{err}"
        assert "is missing a string 'backup' path" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_pid_as_string_refuses_without_traceback():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": "not-a-pid", "files": {}, "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a string 'pid' must not crash:\n{err}"
        assert "'pid' must be an integer or null/absent (got str)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_active_not_a_dict_refuses_without_traceback():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": 1, "files": {}, "active": "oops"}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a non-dict 'active' must not crash:\n{err}"
        assert "'active' must be null/absent or an object (got str)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_active_as_a_number_refuses_without_traceback():
    # A string "active" happens to survive the key check below the isinstance
    # guard (`"label" not in "oops"` is a legal substring test), so the guard
    # itself is only observable with a value that has no `in` at all. Mutation
    # J16 SURVIVED against the string case and FAILs against this one.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": 1, "files": {}, "active": 5}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a numeric 'active' must not crash:\n{err}"
        assert "'active' must be null/absent or an object (got int)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_active_non_string_old_new_refuses_without_traceback():
    # Cold-review finding (live bug): validate_journal_shape checked that
    # label/file/old/new are *present* in "active", not that they're
    # strings. recover() then does pristine.count(active["old"]) and
    # pristine.replace(active["old"], active["new"], 1) -- a non-string old
    # (or new) crashes with a raw TypeError traceback instead of refusing
    # cleanly. pristine_sha256 is deliberately wrong so recover() takes the
    # active-entry branch (an already-pristine file never reaches the
    # str-only operations at all).
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, mutated, bak = _leave_mutated_state_with_real_backup(root)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {
                    "pid": None,
                    "files": {"src/mod.rs": {"backup": bak, "pristine_sha256": "doesnotmatch"}},
                    "active": {"label": "M1", "file": "src/mod.rs", "old": 123, "new": "NEW_TOKEN"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"non-string old/new must not crash:\n{err}"
        assert "'active' has non-string value(s) for key(s): old" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        with open(mod_path) as f:
            assert f.read() == mutated, "file must not be touched"
        assert os.path.exists(journal_path(root)), "journal must be kept"
        assert os.path.exists(bak), "the real backup must not be deleted"


def test_journal_empty_files_dict_does_not_delete_real_backups():
    # Cold-review finding (live bug): {"files": {}} is shape-valid (an empty
    # dict passes isinstance(files, dict)), so recover() iterates nothing,
    # prints "nothing to recover", and unconditionally
    # shutil.rmtree(mutate_dir()) -- deleting real backups under
    # target/mutate/backup/ that no journal entry references, with the
    # product file left mutated on disk forever.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path, mutated, bak = _leave_mutated_state_with_real_backup(root)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": None, "files": {}, "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "are not referenced by any entry in this journal" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by recover()'s own orphan cross-check, not the safety net:\n{err}"
        assert os.path.exists(mutate_dir(root)), "target/mutate/ must not be deleted"
        assert os.path.exists(bak), "the real backup must not be deleted"
        with open(mod_path) as f:
            assert f.read() == mutated, "the mutated file must not be touched"


def test_journal_file_entry_not_a_dict_refuses_without_traceback():
    # Coverage gap noted by cold review: validate_journal_shape's
    # isinstance(info, dict) branch was already correct but had no test.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": 1, "files": {"src/mod.rs": "oops"}, "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a non-dict file entry must not crash:\n{err}"
        assert "must be an object (got str)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_active_missing_one_key_refuses_without_traceback():
    # Coverage gap noted by cold review: the missing-key check in
    # validate_journal_shape ("active" present but missing exactly one key,
    # here "new") was already correct but had no test building that shape.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {"pid": 1, "files": {}, "active": {"label": "M1", "file": "src/mod.rs", "old": "X"}}, f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"an 'active' missing a key must not crash:\n{err}"
        assert "new" in err, err
        assert "'active' is missing key(s): new" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_pid_true_refuses():
    # Tightening: isinstance(True, int) is True in Python, so a JSON boolean
    # pid used to pass the 'pid' shape check outright.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": True, "files": {}, "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a boolean 'pid' must not crash:\n{err}"
        assert "'pid' must be an integer or null/absent (got bool)" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_pid_absurdly_large_refuses_without_traceback():
    # Cold-review finding: validate_journal_shape accepted any int for 'pid',
    # so a pid far outside what os.kill can take reached
    # is_mutate_process_alive()'s os.kill(pid, 0) and raised OverflowError --
    # not an OSError subclass, so the existing `except (ProcessLookupError,
    # OSError)` missed it and it escaped as a raw traceback (exit 1) instead
    # of a clean refusal. This value (10**29) is astronomically large --
    # see test_journal_pid_just_over_32bit_refuses_without_traceback below
    # for the boundary case that actually pins the bound's correctness.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {"pid": 100000000000000000000000000000, "files": {}, "active": None}, f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"an absurdly large 'pid' must not crash:\n{err}"
        assert "'pid' is out of range for a process id" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_pid_just_over_32bit_refuses_without_traceback():
    # Fix-round finding: the bound in validate_journal_shape was
    # `-sys.maxsize - 1 <= pid <= sys.maxsize` (a 64-bit long, per its old
    # comment about "the C long os.kill() converts to"). On this project's
    # platform (macOS), os.kill()'s pid actually goes through a 32-bit
    # pid_t, not a 64-bit long. Measured directly on this machine:
    # os.kill(2147483647, 0) (2**31-1) raises only ProcessLookupError (fine),
    # os.kill(2147483648, 0) (2**31) raises OverflowError("signed integer is
    # greater than maximum"). So a value like 3000000000 -- comfortably
    # inside the old 64-bit bound, since sys.maxsize is 2**63-1 on this
    # platform -- sailed past validate_journal_shape and crashed with a raw
    # traceback. Unlike the absurdly-large case above, this value is small
    # enough that only the *correct* (32-bit) bound catches it -- a
    # generously-wrong bound would still pass this test.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": 3000000000, "files": {}, "active": None}, f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a pid just over the 32-bit range must not crash:\n{err}"
        assert "'pid' is out of range for a process id" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by validate_journal_shape's own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_recover_with_directory_where_file_expected_does_not_crash():
    # Cold-review finding: recover()'s `sha256_of(path) ==
    # info.get("pristine_sha256")` line has no exception guard, unlike the
    # active-entry branch just below it and main()'s end-of-run pristine
    # check. A journaled relpath that resolves to a directory (or an
    # unreadable file) raises IsADirectoryError (an OSError) straight out of
    # this line -- a raw traceback and exit 1 -- instead of being treated as
    # the "matches neither" case.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        dir_path = os.path.join(root, "src")
        os.makedirs(dir_path, exist_ok=True)
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {
                    "pid": None,
                    "files": {"src": {"backup": "/nonexistent-backup", "pristine_sha256": "deadbeef"}},
                    "active": None,
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a directory in place of a file must not crash:\n{err}"
        assert "src" in err, err
        assert "cannot read to check against pristine" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by recover()'s own guard, not the safety net:\n{err}"
        assert os.path.isdir(dir_path), "the directory must not be touched"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_recover_with_unreadable_backup_root_does_not_crash():
    # Fix-round finding: recover()'s orphaned-backup cross-check does
    # `os.listdir(backup_root())` with no exception guard -- every other
    # filesystem read in recover() (sha256_of, the active-entry open()
    # calls) is guarded, this was the odd one out. Make backup_root()
    # unreadable (chmod 000) so os.listdir() raises PermissionError.
    #
    # `files: {}` is required to reach this line at all: the per-file loop
    # above the cross-check exits via `sys.exit(2)` on the first problem it
    # finds, so any files-loop problem (a mismatched sha256, a missing
    # file) would return 2 without ever calling os.listdir() -- the same
    # trap test_journal_empty_files_dict_does_not_delete_real_backups is
    # built around, for the same reason.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        bdir = os.path.join(mutate_dir(root), "backup")
        os.makedirs(bdir, exist_ok=True)
        write_file(os.path.join(bdir, "0000__src__mod.rs"), "fn x() {}\n")
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump({"pid": None, "files": {}, "active": None}, f)
        os.chmod(bdir, 0)
        try:
            code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        finally:
            os.chmod(bdir, 0o755)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"an unreadable backup dir must not crash:\n{err}"
        assert bdir in err or "backup" in err, err
        assert "cannot list" in err and "to check for orphaned backups" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by recover()'s own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"
        assert os.path.exists(mutate_dir(root)), "target/mutate/ must not be deleted"


def test_startup_recovery_unexpected_exception_refuses_via_safety_net():
    # Fix-round item 3: main()'s startup recovery phase (load_journal() +
    # recover()) is wrapped so any exception not already a deliberate
    # SystemExit is turned into the same kind of refusal, as a net for
    # inputs nobody has specifically guarded against yet.
    #
    # Confirmed (by direct experiment, not reasoning) that this is a real
    # gap none of the specific guards cover: `active["backup"]` containing
    # an embedded null byte reaches `open(bak, encoding="utf-8")` in
    # recover()'s active-entry branch, which raises `ValueError: embedded
    # null byte` -- ValueError is not a subclass of OSError, so the
    # existing `except (OSError, UnicodeDecodeError)` around that exact
    # line does not catch it, and it used to propagate all the way out of
    # main() as a raw traceback (exit 1). This is distinct from
    # test_recover_with_unreadable_backup_root_does_not_crash above, which
    # is now caught by a specific guard, not the net.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        mod_path = os.path.join(root, "src", "mod.rs")
        write_file(mod_path, "fn x() {\n    NEW_TOKEN\n}\n")
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump(
                {
                    "pid": None,
                    "files": {"src/mod.rs": {"backup": "/nonexistent\x00path",
                                              "pristine_sha256": "doesnotmatch"}},
                    "active": {"label": "M1", "file": "src/mod.rs",
                               "old": "OLD_TOKEN", "new": "NEW_TOKEN"},
                },
                f,
            )

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"an unanticipated recovery exception must not crash:\n{err}"
        assert "ValueError" in err, err
        assert "nothing was touched" in err.lower() or "not touching anything" in err.lower(), err
        assert journal_path(root) in err, err
        assert "backup" in err.lower(), err
        # This is the one test that *must* see the net's own wording -- it is
        # specifically exercising the net, not one of the specific guards.
        assert "unexpected error during startup recovery" in err, err
        # And it must not be reachable via any of the specific guards' own
        # wording -- if one of those fired instead, this test would no longer
        # be exercising the net at all.
        for specific_wording in (
            "could not be read as the journal",
            "parsed as JSON but is not an object",
            "parsed as JSON but has an invalid shape for a journal",
            "cannot read backup or current file to check against the active entry",
            "cannot read to check against pristine",
            "current content matches neither the pristine backup nor the "
            "expected mid-mutation state",
            "are not referenced by any entry in this journal",
            "still appears to be active",
        ):
            assert specific_wording not in err, \
                f"must be refused by the safety net, not a specific guard ({specific_wording!r}):\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def test_journal_parses_to_a_list_refuses():
    # Pins the isinstance(data, dict) guard from the previous round (finding
    # 3 of this round's review): a journal.json whose *top level* is a JSON
    # array, not an object, had no test at all.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        os.makedirs(mutate_dir(root), exist_ok=True)
        with open(journal_path(root), "w", encoding="utf-8") as f:
            json.dump([1, 2, 3], f)

        code, out, err = run_mutate(root, ["--recover-only"], None, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "Traceback" not in err, f"a list-shaped journal must not crash:\n{err}"
        assert "parsed as JSON but is not an object" in err, err
        assert "unexpected error during startup recovery" not in err, \
            f"must be refused by load_journal()'s own guard, not the safety net:\n{err}"
        assert os.path.exists(journal_path(root)), "journal must be kept"


def main():
    tests = sorted(
        (name, obj) for name, obj in globals().items()
        if name.startswith("test_") and callable(obj)
    )
    if len(tests) < MINIMUM_TESTS:
        print(
            "DISCOVERY FAILURE: found %d test functions, expected at least %d. "
            "Either a test was deleted (lower MINIMUM_TESTS deliberately, in a "
            "commit that says why) or the `test_` naming convention broke and "
            "the checks below are silently not running." % (len(tests), MINIMUM_TESTS)
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
        print("%d/%d tests FAILED: %s" % (len(failures), len(tests), ", ".join(failures)))
        sys.exit(1)
    print("%d/%d tests passed" % (len(tests), len(tests)))
    sys.exit(0)


if __name__ == "__main__":
    main()
