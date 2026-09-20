"""Tests for dev/gate.py -- plain asserts, no test framework beyond unittest's
assert helpers used ad hoc (same shape as dev/test_pixdiff.py: discovery is
by naming convention, `main` collects every `test_*` callable off this
module's own namespace, sorted by name, and runs it; a hand-written list
would reproduce the exact "defined but never run" failure this milestone
exists to close).

Run with: python3 dev/test_gate.py

Every test drives the real `dev/gate.py` as a subprocess, pointed (via
`--cargo`) at `dev/fake-cargo.py` instead of the real `cargo`, against a
throwaway git repository standing in for the workspace root. This exercises
gate.py's real target-enumeration parser, its `test result:` regex, its
stamp computation (a real `git ls-files` call), and its state file --
nothing about gate.py itself is mocked, only cargo.

Requires git and python3 on PATH. If git is missing, this prints why and
exits 1 (a *failure*, not a skip -- see dev/gate.py's own reliance on `git
ls-files` for the stamp: a machine without git cannot run the tool this file
tests, so silently skipping would hide that dev/gate.py is untestable here,
the same trap `crates/core/tests/dev_tools_tests.rs`'s module doc warns
about for a missing Pillow).
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

DEV_DIR = os.path.dirname(os.path.abspath(__file__))
GATE_PY = os.path.join(DEV_DIR, "gate.py")
FAKE_CARGO_PY = os.path.join(DEV_DIR, "fake-cargo.py")

MINIMUM_TESTS = 14


def require_git():
    if shutil.which("git") is None:
        print("SKIPPED: git is not on PATH, dev/gate.py cannot be tested here")
        sys.exit(1)


class TempRepo:
    """A throwaway git repo standing in for the workspace root."""

    def __enter__(self):
        self.dir = tempfile.mkdtemp(prefix="gate_test_repo_")
        subprocess.run(["git", "init", "-q"], cwd=self.dir, check=True)
        subprocess.run(["git", "config", "user.email", "t@example.com"], cwd=self.dir, check=True)
        subprocess.run(["git", "config", "user.name", "t"], cwd=self.dir, check=True)
        with open(os.path.join(self.dir, "README.txt"), "w") as f:
            f.write("placeholder\n")
        # gate.py writes its own state/logs under target/gate/ -- ignore it
        # the same way the real repo's .gitignore ignores /target, or the
        # stamp would change on every run just from gate.py having run once.
        with open(os.path.join(self.dir, ".gitignore"), "w") as f:
            f.write("/target\n")
        return self.dir

    def __exit__(self, *exc):
        shutil.rmtree(self.dir, ignore_errors=True)
        return False


def metadata(packages):
    return {"packages": packages, "target_directory": "target", "workspace_root": ".", "version": 1}


def pkg(name, targets):
    return {"name": name, "targets": targets}


def target(name, kind):
    return {"name": name, "kind": [kind] if isinstance(kind, str) else kind}


def run_gate(root, extra_args, scenario, scratch):
    scenario.setdefault("attempts_file", scenario.get("attempts_file"))
    scenario_path = os.path.join(scratch, "scenario.json")
    with open(scenario_path, "w") as f:
        json.dump(scenario, f)
    env = dict(os.environ)
    env["FAKE_CARGO_SCENARIO"] = scenario_path
    cmd = [sys.executable, GATE_PY, "--cargo", FAKE_CARGO_PY, "--root", root] + extra_args
    proc = subprocess.run(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return proc.returncode, proc.stdout, proc.stderr


def read_state(root):
    with open(os.path.join(root, "target", "gate", "state.json")) as f:
        return json.load(f)


def five_target_scenario(outcomes=None):
    meta = metadata([pkg("app", [target(n, "test") for n in "abcde"])])
    targets = {}
    for n in "abcde":
        targets[f"app/test/{n}"] = (outcomes or {}).get(
            n, {"outcome": "ok", "passed": 1, "failed": 0, "ignored": 0}
        )
    return {"metadata": meta, "steps": {}, "attempts_file": None, "targets": targets}


def test_enumeration_produces_expected_keys():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([
            pkg("alpha", [target("alpha", "lib"), target("t1", "test")]),
            pkg("betamod", [target("betamod", "cdylib")]),
        ])
        scenario = {"metadata": meta, "steps": {}, "attempts_file": None, "targets": {}}
        code, out, err = run_gate(root, ["--list", "--floor", "1"], scenario, scratch)
        assert code == 0, f"exit {code}, stderr={err}"
        keys = set(out.strip().splitlines())
        expected = {"alpha/lib", "alpha/test/t1", "alpha/doc", "betamod/lib"}
        assert keys == expected, f"got {keys}"
        assert "betamod/doc" not in keys, "cdylib-only package must not get a doc target"


def test_floor_rejects_too_few_targets():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("alpha", [target(n, "test") for n in ("a", "b", "c")])])
        scenario = {"metadata": meta, "steps": {}, "attempts_file": None, "targets": {}}
        code, out, err = run_gate(root, ["--list"], scenario, scratch)  # default floor 90
        assert code == 2, f"expected exit 2, got {code}: {err}"
        assert "3" in err, f"error should name the count 3: {err}"


def test_full_run_all_ok():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        scenario = five_target_scenario()
        code, out, err = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code == 0, f"exit {code}, stdout={out}, stderr={err}"
        assert "5 targets" in out, out
        assert "GATE PASSED (5 targets)" in out, out


def test_resume_after_killed_target():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        outcomes = {"c": {"outcome": "killed", "passed": 2, "failed": 0, "ignored": 0}}
        scenario = five_target_scenario(outcomes)
        scenario["attempts_file"] = os.path.join(scratch, "attempts.json")

        code1, out1, err1 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code1 != 0, f"first run should fail, stdout={out1}"
        assert "app/test/c" in out1
        assert "ok  app/test/d" in out1, out1
        assert "ok  app/test/e" in out1, out1

        code2, out2, err2 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code2 == 0, f"second run should pass, stdout={out2}, stderr={err2}"
        assert "skip app/test/a" in out2, out2
        assert "skip app/test/b" in out2, out2
        assert "skip app/test/d" in out2, out2
        assert "skip app/test/e" in out2, out2
        assert "ok  app/test/c" in out2, out2


def test_stamp_change_discards_state():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        scenario = five_target_scenario()
        code1, out1, err1 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code1 == 0, err1

        with open(os.path.join(root, "README.txt"), "a") as f:
            f.write("changed\n")

        code2, out2, err2 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code2 == 0, f"stdout={out2}, stderr={err2}"
        assert "starting fresh" in out2, out2
        for n in "abcde":
            assert f"ok  app/test/{n}" in out2, out2
        assert "skip" not in out2, out2


def test_double_result_line_counts_once_with_last_line():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [target("echo", "test")])])
        scenario = {
            "metadata": meta, "steps": {}, "attempts_file": None,
            "targets": {
                "app/test/echo": {
                    "outcome": "ok", "passed": 101, "failed": 0, "ignored": 0,
                    "double_result_line": True,
                }
            },
        }
        code, out, err = run_gate(root, ["--floor", "1"], scenario, scratch)
        assert code == 0, f"stdout={out}, stderr={err}"
        assert "1 targets" in out, out
        assert "101 passed" in out, out
        state = read_state(root)
        assert state["targets"]["app/test/echo"]["passed"] == 101, state
        assert state["targets"]["app/test/echo"]["failed"] == 0, state


def test_fail_target_makes_verdict_fail():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [target("good", "test"), target("bad", "test")])])
        scenario = {
            "metadata": meta, "steps": {}, "attempts_file": None,
            "targets": {
                "app/test/good": {"outcome": "ok", "passed": 1, "failed": 0, "ignored": 0},
                "app/test/bad": {"outcome": "fail", "passed": 1, "failed": 1, "ignored": 0},
            },
        }
        code, out, err = run_gate(root, ["--floor", "2"], scenario, scratch)
        assert code == 1, f"stdout={out}"
        assert "app/test/bad" in out, out
        assert "rerun: cargo test -p app --test bad" in out, out


def test_tests_only_refused_then_accepted():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [target("a", "test")])])
        scenario = {
            "metadata": meta, "steps": {"build": 0, "fmt": 0, "clippy": 0}, "attempts_file": None,
            "targets": {"app/test/a": {"outcome": "ok", "passed": 1, "failed": 0, "ignored": 0}},
        }
        code, out, err = run_gate(root, ["--floor", "1", "--tests-only"], scenario, scratch)
        assert code == 1, f"stdout={out}, stderr={err}"
        assert "have not passed" in err, err

        code2, out2, err2 = run_gate(root, ["--floor", "1"], scenario, scratch)
        assert code2 == 0, f"stdout={out2}, stderr={err2}"

        code3, out3, err3 = run_gate(root, ["--floor", "1", "--tests-only"], scenario, scratch)
        assert code3 == 0, f"stdout={out3}, stderr={err3}"
        assert "build:" not in out3, out3
        assert "fmt:" not in out3, out3
        assert "clippy:" not in out3, out3


def test_no_result_line_is_reported_as_fail():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [target("silent", "test")])])
        scenario = {
            "metadata": meta, "steps": {}, "attempts_file": None,
            "targets": {"app/test/silent": {"outcome": "no_result"}},
        }
        code, out, err = run_gate(root, ["--floor", "1"], scenario, scratch)
        assert code == 1, f"stdout={out}"
        assert "FAIL app/test/silent" in out, out


def test_only_unknown_package_exits_2():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        scenario = five_target_scenario()
        code, out, err = run_gate(root, ["--only", "nope"], scenario, scratch)
        assert code == 2, f"stdout={out}, stderr={err}"
        assert "nope" in err, err
        assert "app" in err, err  # known packages must be listed


def test_tests_only_refused_after_tree_change():
    # Reviewer mutation target: if the stamp mismatch branch kept the stale
    # state instead of setting state = None, this would wrongly accept
    # --tests-only against a tree that changed after the passing run.
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [target("a", "test")])])
        scenario = {
            "metadata": meta, "steps": {"build": 0, "fmt": 0, "clippy": 0}, "attempts_file": None,
            "targets": {"app/test/a": {"outcome": "ok", "passed": 1, "failed": 0, "ignored": 0}},
        }
        code1, out1, err1 = run_gate(root, ["--floor", "1"], scenario, scratch)
        assert code1 == 0, f"stdout={out1}, stderr={err1}"

        with open(os.path.join(root, "README.txt"), "a") as f:
            f.write("a real change\n")

        code2, out2, err2 = run_gate(root, ["--floor", "1", "--tests-only"], scenario, scratch)
        assert code2 == 1, f"stdout={out2}, stderr={err2}"
        assert "have not passed" in err2, err2


def test_claude_dir_excluded_from_stamp():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        scenario = five_target_scenario()
        code1, out1, err1 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code1 == 0, err1

        claude_dir = os.path.join(root, ".claude", "worktrees", "x")
        os.makedirs(claude_dir, exist_ok=True)
        with open(os.path.join(claude_dir, "f.txt"), "w") as f:
            f.write("noise\n")

        code2, out2, err2 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code2 == 0, f"stdout={out2}, stderr={err2}"
        assert "starting fresh" not in out2, out2
        assert "skip app/test/a" in out2, out2

        with open(os.path.join(root, "README.txt"), "a") as f:
            f.write("a real change\n")

        code3, out3, err3 = run_gate(root, ["--floor", "5"], scenario, scratch)
        assert code3 == 0, f"stdout={out3}, stderr={err3}"
        assert "starting fresh" in out3, out3


def test_enumeration_treats_proc_macro_as_lib():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("macroy", [target("macroy", "proc-macro")])])
        scenario = {"metadata": meta, "steps": {}, "attempts_file": None, "targets": {}}
        code, out, err = run_gate(root, ["--list", "--floor", "1"], scenario, scratch)
        assert code == 0, err
        keys = set(out.strip().splitlines())
        assert keys == {"macroy/lib"}, keys


def test_enumeration_skips_test_false_targets():
    with TempRepo() as root, tempfile.TemporaryDirectory() as scratch:
        meta = metadata([pkg("app", [
            {"name": "t_on", "kind": ["test"], "test": True},
            {"name": "t_off", "kind": ["test"], "test": False},
        ])])
        scenario = {"metadata": meta, "steps": {}, "attempts_file": None, "targets": {}}
        code, out, err = run_gate(root, ["--list", "--floor", "1"], scenario, scratch)
        assert code == 0, err
        keys = set(out.strip().splitlines())
        assert keys == {"app/test/t_on"}, keys


def main():
    require_git()
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
