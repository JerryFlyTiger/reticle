# Mutation list for M151 (fixed ~2 s `tick_until` budgets -> 30 s hang guard,
# plus the guard test that keeps them from coming back).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m151.py -p core
#
# Per M117's rule, a deletion entry per FEATURE. M151 ships:
#
#   H, the 30 s ceiling in the six `tick_until` helpers -> H1, H2, H2b
#   G, `guard_every_tick_until_polls_under_a_ceiling`  -> G1..G4 (G3 declared)
#   X, the same conversion in lsp_mode_tests / shell_tests -> X1, X2
#      (declared survivors)
#
# H1/H2 mutate `rainbow_delimiters_tests.rs`, which the guard reads at RUN
# time; the highlight_tests binary does not compile it, so these entries set
# needs_rebuild=False (see dev/mutate.py's field docs).
#
# What cannot be seen from an unloaded `cargo test`: putting a helper back to
# 200 x 10 ms is green on an idle machine. Its real consequence was measured
# under load by the main conversation (30 `yes` burners, five binaries at
# once): scope_header_tests 21/24 red and font_lock_philosophy_tests 1/9 red
# in 3 of 3 rounds before M151, 0 red in 3 of 3 rounds after (load ~15).
# The before/after logs were lost (a read-only reviewer deleted the scratch
# directory); the numbers are in the M151 commit message, and the after-run was
# repeated at wrap-up. H2 is the executed evidence that the guard, not luck,
# is what keeps that shape out.

PACKAGE = "core"
TEST_TARGET = "highlight_tests"

_G = "crates/core/tests/highlight_tests.rs"
_R = "crates/core/tests/rainbow_delimiters_tests.rs"
_GUARD = "guard_every_tick_until_polls_under_a_ceiling"

_R_BODY = """    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}
"""

MUTATIONS = [
    {
        "label": "H1 a helper loses its deadline check (unbounded wait)",
        "file": _R,
        "old": """        if std::time::Instant::now() >= deadline {
            return false;
        }
""",
        "new": "",
        "test": _GUARD,
        "needs_rebuild": False,
    },
    {
        "label": "H2 a helper goes back to the pre-M151 200 x 10 ms budget (deletion entry)",
        "file": _R,
        "old": _R_BODY,
        "new": """    for _ in 0..200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}
""",
        "test": _GUARD,
        "needs_rebuild": False,
    },
    {
        # Only `has_bad_loop' can see this one: the body still carries a real
        # `Instant::now() >=' comparison and no `for _ in 0..', yet the
        # counter caps it at the pre-M151 ~2 s. The round-1 review's bypass.
        "label": "H2b a while-counter cap hidden behind a real deadline check",
        "file": _R,
        "old": _R_BODY,
        "new": """    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut polls = 0;
    while polls < 200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        polls += 1;
    }
    false
}
""",
        "test": _GUARD,
        "needs_rebuild": False,
    },
    {
        "label": "G1 the definitions floor is live",
        "file": _G,
        "old": "definitions_found >= 6,",
        "new": "definitions_found >= 7,",
        "test": _GUARD,
    },
    {
        "label": "G2 the file-count floor is live",
        "file": _G,
        "old": "rs_files.len() >= 50,",
        "new": "rs_files.len() >= 500,",
        "test": _GUARD,
    },
    {
        # The round-1 defect put back: the body ends at the first indented
        # `}', so no helper's deadline comparison is ever inside the text
        # checked and all six are reported.
        "label": "G4 body extraction ends at the first bare `}' again",
        "file": _G,
        "old": 'if *body_line == "}" {',
        "new": 'if body_line.trim() == "}" {',
        "test": _GUARD,
    },
    {
        # Whole-effect deletion of the guard. Declared survivor: nothing
        # tests the guard itself, and on a correct tree a vacuous guard is
        # as green as a real one. H1/H2 are what show it has an effect.
        "label": "G3 the guard stops comparing against the clock (declared survivor)",
        "file": _G,
        "old": 'let deadline_marker: String = ["Instant::", "now()", " >="].concat();',
        "new": 'let deadline_marker: String = ["fn ", ""].concat();',
        "test": _GUARD,
        "expect": "survived",
    },
    {
        # Declared survivor: the fixed 2 s budget only goes red under load,
        # which `cargo test` on an idle machine does not create, and no guard
        # covers inline loops (the guard's doc comment says so).
        "label": "X1 lsp_mode_tests delivery wait ceiling 30 s -> 2 s (declared survivor)",
        "file": "crates/core/tests/lsp_mode_tests.rs",
        "old": "    let mut delivered = false;\n    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);",
        "new": "    let mut delivered = false;\n    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);",
        "test_target": "lsp_mode_tests",
        "test": "",
        "expect": "survived",
    },
    {
        "label": "X2 shell_tests poll_until_exit ceiling 30 s -> 4 s (declared survivor)",
        "file": "crates/elisp/tests/shell_tests.rs",
        "old": "    let mut results = Vec::new();\n    let deadline = std::time::Instant::now() + Duration::from_secs(30);",
        "new": "    let mut results = Vec::new();\n    let deadline = std::time::Instant::now() + Duration::from_secs(4);",
        "package": "elisp",
        "test_target": "shell_tests",
        "test": "",
        "expect": "survived",
    },
]
