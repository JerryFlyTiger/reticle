# Mutation list for M140: the resumable, segmented gate (`dev/gate.py`,
# `dev/fake-cargo.py`, `dev/test_gate.py`) and the retry loop in
# `tests/worker_tests.rs`.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m140.py
#
# Designed by the reviewer that cold-read the first batch; M4 and M6-M7 were
# added after the fix round closed the gaps that review found (a stale-stamp
# `--tests-only`, the `.claude/` stamp exclusion, an unknown `--only` name).
# Run from the main conversation, never by the implementer.
#
# Every python entry has `needs_rebuild: False`: the files are read at run
# time by `dev_gate_tests_pass`, which shells out to `python3 dev/test_gate.py`,
# so cargo rebuilding nothing is the normal case, not the danger case.
#
# The worker-test entry is declared SURVIVED: reducing five attempts to one is
# the pre-M140 shape of that test, which passed for weeks on an idle machine
# and only went red under load. Observing the mutation needs an artificially
# loaded CPU, which this runner does not arrange.

PACKAGE = "core"
TEST_TARGET = "dev_tools_tests"

MUTATIONS = [
    {
        "label": "M1 stamp ignores untracked files",
        "file": "dev/gate.py",
        "old": '"ls-files", "-co", "--exclude-standard"',
        "new": '"ls-files", "-c", "--exclude-standard"',
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_stamp_change_discards_state (README.txt is untracked in the fixture).",
    },
    {
        "label": "M2 first result line instead of last",
        "file": "dev/gate.py",
        "old": "m = matches[-1]",
        "new": "m = matches[0]",
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_double_result_line_counts_once_with_last_line.",
    },
    {
        "label": "M3 floor removed",
        "file": "dev/gate.py",
        "old": "if len(targets) < floor or (args.only is not None and len(targets) == 0):",
        "new": "if False:",
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_floor_rejects_too_few_targets.",
    },
    {
        "label": "M4 stale-stamp state kept alive",
        "file": "dev/gate.py",
        "old": '''        print("tree changed since the last run: starting fresh")
        state = None''',
        "new": '''        print("tree changed since the last run: starting fresh")
        pass''',
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_tests_only_refused_after_tree_change (survived the first batch; the fix round added this test).",
    },
    {
        "label": "M5 failed target skipped on resume",
        "file": "dev/gate.py",
        "old": 'if prior and prior.get("ok"):',
        "new": "if prior:",
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_resume_after_killed_target.",
    },
    {
        "label": "M6 .claude/ no longer excluded from the stamp",
        "file": "dev/gate.py",
        "old": 'if line.strip() and not line.strip().startswith(".claude/")',
        "new": "if line.strip()",
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": "python: test_claude_dir_excluded_from_stamp.",
    },
    {
        "label": "M7 unknown --only package accepted",
        "file": "dev/gate.py",
        "old": "if args.only is not None and args.only not in known_pkgs:",
        "new": "if False:",
        "test": "dev_gate_tests_pass",
        "needs_rebuild": False,
        "note": (
            "python: test_only_unknown_package_exits_2 -- it goes red on its "
            "MESSAGE assertions, not because the bad name is accepted: the "
            "floor's zero-target clause still exits 2 without this check. The "
            "guard's value is the useful error (naming the package, listing the "
            "known ones), which is what the test pins."
        ),
    },
    {
        "label": "M8 worker retry loop reduced to one attempt",
        "file": "tests/worker_tests.rs",
        "package": "reticle",
        "test_target": "worker_tests",
        "old": "for _ in 0..5 {",
        "new": "for _ in 0..1 {",
        "test": "genuine_parallelism",
        "expect": "survived",
        "note": (
            "One attempt is the pre-M140 test, which passes on an idle machine; "
            "the mutation is only observable under CPU load, which this runner "
            "does not arrange."
        ),
    },
]
