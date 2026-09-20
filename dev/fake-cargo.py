#!/usr/bin/env python3
"""A fake `cargo` binary, for testing dev/gate.py without a real build.

`dev/test_gate.py` points `dev/gate.py --cargo` at this script instead of the
real `cargo`. It reads a scenario file (path in the `FAKE_CARGO_SCENARIO`
environment variable, JSON) describing what each subcommand should do, and
prints realistic-looking cargo output so gate.py's real parsers (the
`test result:` regex, the metadata JSON parse) are actually exercised rather
than bypassed.

Scenario JSON shape:

    {
      "metadata": { ... raw `cargo metadata --format-version 1` JSON ... },
      "steps": {"build": 0, "fmt": 0, "clippy": 0},
      "attempts_file": "/path/to/attempts.json",
      "targets": {
        "pkg/test/name": {"outcome": "ok", "passed": 3, "failed": 0, "ignored": 0},
        "pkg/test/other": {"outcome": "fail", "passed": 1, "failed": 1, "ignored": 0},
        "pkg/test/flaky": {"outcome": "killed", "passed": 1, "failed": 0, "ignored": 0},
        "pkg/test/echo": {"outcome": "ok", "passed": 101, "failed": 0, "ignored": 0,
                           "double_result_line": true}
      }
    }

`outcome: "killed"` mimics an OOM kill: this process prints the `Running`
line, then sends itself SIGKILL, exiting via signal rather than a normal
return -- but only on the *first* time a given target key is asked for,
tracked in `attempts_file` (a small JSON dict of key -> attempt count). Every
subsequent attempt at the same key instead succeeds, using the target's
`passed`/`failed`/`ignored` fields, so a test can drive gate.py's resume path
end to end without ever crashing every attempt at that target.

`outcome: "no_result"` prints the `Running` line, then exits 0 without any
`test result:` line at all -- pins gate.py's rule that exit 0 alone is not
"ran", it also requires a parsed result line.

`double_result_line: true` prints two `test result:` lines -- the first
mimicking a test that itself spawns a subprocess with its own nested test
harness, echoing a smaller (wrong) count, and the second (larger, correct)
line being the actual outer harness result. This exists to pin down
gate.py's "count per invocation, take the LAST `test result:` line, never
count lines" rule: a naive line-counting parser would get this target wrong
two different ways (double-counting, or picking the first/smaller line).

`outcome: "make_readonly"` (M146 review round, added for `dev/test_mutate.py`)
chmods the path named by the entry's `"readonly_path"` field (an absolute
path, meant to be the mutated product file itself, not its backup) to 0o444
before printing an ordinary result line -- it does not kill anything, it
exits normally. This exists to make `dev/mutate.py`'s own `restore(bak,
path)` fail with a real `PermissionError` writing to `path` (mimicking a
transient disk/permission problem), without an actual SIGKILL: `mutate.py`
keeps running afterward and reaches its own `finally` blocks, so a test can
observe how the tool itself reacts to a raised restore rather than to being
killed. The backup itself is left untouched, so the scenario is genuinely
recoverable once the permission problem clears (a test can chmod the path
back to writable before the next `mutate.py` invocation).

`outcome: "kill_parent"` (M146, added for `dev/test_mutate.py`) mimics an
OOM kill that takes down the *harness*, not just cargo: after printing the
`Running` line, this process SIGKILLs its own parent (`os.getppid()` --
`dev/mutate.py` itself, since it launches this script directly via
`subprocess.Popen`) instead of itself. `outcome: "killed"` above kills the
fake cargo process, which dev/gate.py or dev/mutate.py would see as an
ordinary crashed child and could retry; this one kills the calling
mutate.py process mid-mutation, leaving a mutated file on disk with no
`finally` block ever running -- exactly the scenario dev/mutate.py's
journal (header lesson 9) exists to recover from. Only ever attempted once
per key like `"killed"`, via the same `attempts_file` bookkeeping, though in
practice a test only needs the first attempt: the parent is dead, so there
is no second attempt from the same process.
"""

import json
import os
import signal
import sys


def load_scenario():
    path = os.environ.get("FAKE_CARGO_SCENARIO")
    if not path:
        print("fake-cargo.py: FAKE_CARGO_SCENARIO not set", file=sys.stderr)
        sys.exit(127)
    with open(path) as f:
        return json.load(f)


def target_key_from_args(args):
    pkg = None
    i = 0
    while i < len(args):
        if args[i] == "-p" and i + 1 < len(args):
            pkg = args[i + 1]
            i += 2
            continue
        if args[i] == "--test" and i + 1 < len(args):
            return f"{pkg}/test/{args[i + 1]}"
        if args[i] == "--lib":
            return f"{pkg}/lib"
        if args[i] == "--bin" and i + 1 < len(args):
            return f"{pkg}/bin/{args[i + 1]}"
        if args[i] == "--doc":
            return f"{pkg}/doc"
        i += 1
    return None


def read_attempts(path):
    if not path or not os.path.exists(path):
        return {}
    with open(path) as f:
        return json.load(f)


def write_attempts(path, attempts):
    if not path:
        return
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(attempts, f)
    os.replace(tmp, path)


def result_line(outcome, passed, failed, ignored, seconds="0.10s"):
    status = "ok" if failed == 0 else "FAILED"
    return (
        f"test result: {status}. {passed} passed; {failed} failed; "
        f"{ignored} ignored; 0 measured; 0 filtered out; finished in {seconds}\n"
    )


def main():
    args = sys.argv[1:]
    if not args:
        print("fake-cargo.py: missing subcommand", file=sys.stderr)
        return 127
    subcommand = args[0]
    rest = args[1:]
    scenario = load_scenario()

    if subcommand == "metadata":
        print(json.dumps(scenario["metadata"]))
        return 0

    if subcommand in ("build", "fmt", "clippy"):
        return scenario.get("steps", {}).get(subcommand, 0)

    if subcommand != "test":
        print(f"fake-cargo.py: unhandled subcommand {subcommand}", file=sys.stderr)
        return 127

    key = target_key_from_args(rest)
    entry = scenario.get("targets", {}).get(key)
    if entry is None:
        print(f"fake-cargo.py: no scenario entry for target {key!r}", file=sys.stderr)
        return 127

    name = key.split("/")[-1]
    print(f"     Running tests/{name}.rs (target/debug/deps/{name}-abcdef)")
    sys.stdout.flush()

    outcome = entry.get("outcome", "ok")
    if outcome == "make_readonly":
        target_path = entry.get("readonly_path")
        if target_path:
            try:
                os.chmod(target_path, 0o444)
            except OSError:
                pass
        passed = entry.get("passed", 0)
        failed = entry.get("failed", 0)
        ignored = entry.get("ignored", 0)
        print(result_line("ok" if failed == 0 else "FAILED", passed, failed, ignored))
        return 0 if failed == 0 else 101
    if outcome == "kill_parent":
        attempts_file = scenario.get("attempts_file")
        attempts = read_attempts(attempts_file)
        count = attempts.get(key, 0)
        if count == 0:
            attempts[key] = count + 1
            write_attempts(attempts_file, attempts)
            sys.stdout.flush()
            os.kill(os.getppid(), signal.SIGKILL)
            # Reachable, unlike "killed" below: os.kill(getppid(), ...) kills
            # the *parent* (mutate.py), not this process. This one keeps
            # running and returns normally -- its exit code and output don't
            # matter to anything, since mutate.py (the thing reading them)
            # is already dead.
            return 137
        # Should not normally be reached (see module doc), but behave like
        # "killed"'s second attempt rather than loop forever if it is.
        passed = entry.get("passed", 1)
        failed = entry.get("failed", 0)
        ignored = entry.get("ignored", 0)
        print(result_line("ok", passed, failed, ignored))
        return 0
    if outcome == "no_result":
        # Exit 0 with no `test result:` line at all -- gate.py must treat
        # this as FAIL, not as a 0-test pass, since real cargo never exits 0
        # for a test binary without printing a result line.
        return 0
    if outcome == "killed":
        attempts_file = scenario.get("attempts_file")
        attempts = read_attempts(attempts_file)
        count = attempts.get(key, 0)
        if count == 0:
            attempts[key] = count + 1
            write_attempts(attempts_file, attempts)
            sys.stdout.flush()
            os.kill(os.getpid(), signal.SIGKILL)
            return 137  # unreachable; SIGKILL terminates before this
        # Second and later attempts at this key succeed.
        passed = entry.get("passed", 1)
        failed = entry.get("failed", 0)
        ignored = entry.get("ignored", 0)
        print(result_line("ok", passed, failed, ignored))
        return 0

    passed = entry.get("passed", 0)
    failed = entry.get("failed", 0)
    ignored = entry.get("ignored", 0)
    if entry.get("double_result_line"):
        # A smaller, wrong count from a "nested subprocess harness" first...
        print(result_line("ok", 1, 0, 0, seconds="0.02s"))
        # ...then the real, authoritative last line.
        print(result_line("ok" if failed == 0 else "FAILED", passed, failed, ignored))
    else:
        print(result_line("ok" if failed == 0 else "FAILED", passed, failed, ignored))

    return 0 if outcome == "ok" else 101


if __name__ == "__main__":
    sys.exit(main())
