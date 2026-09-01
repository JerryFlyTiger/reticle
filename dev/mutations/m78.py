# M78 (CLI facade: --help / --version / man page) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m78.py \
#         -p reticle --test-target cli_tests
#
# The target is the root package (src/main.rs's argv pre-scan) + its
# integration tests, so PACKAGE must explicitly say "reticle". Leaving
# it empty degenerates to just running the root package in this workspace --
# this time that happens to be exactly what's wanted, but it's still spelled
# out explicitly, not relied on by coincidence (see lesson 6 in mutate.py's
# header).
#
# List designed by an independent reviewer, executed by the main
# conversation (implementers don't verify their own fix). The reviewer's
# original list had two entries flagged as "black-box unobservable"
# (deleting the `-h, --help` block from the man page / help text still lets
# the test PASS, because "-h" is a substring of "--help"). Only after the
# fix-up round's R2 changed those two assertions to check the complete form
# of the alias pair did they become truly observable -- that's M7 and M8
# below, **which double as the verification points for "did the fix-up
# round actually fix it"**.

PACKAGE = "reticle"
TEST_TARGET = "cli_tests"

MUTATIONS = [
    {
        "label": "M1 breaks \"first occurrence wins\": scan direction reversed",
        "file": "src/main.rs",
        "old": "    for (i, a) in args.iter().enumerate() {",
        "new": "    for (i, a) in args.iter().enumerate().rev() {",
        "test": "first_of_help_or_version_wins",
        "note": "Reverse scanning becomes \"last occurrence wins\", so `--version --help` prints help.",
    },
    {
        "label": "M2 version string changed to a hard-coded wrong value",
        "file": "src/main.rs",
        "old": '    println!("reticle {}", env!("CARGO_PKG_VERSION"));',
        "new": '    println!("reticle 0.0.0");',
        "test": "version_exits_zero_with_exact_output",
        "note": (
            "short_version_flags_match_long_version_verbatim would not FAIL "
            "because of this -- it only checks that the three flags agree "
            "with each other, not against the real version number. This is "
            "a deliberate division of labor, not a gap."
        ),
    },
    {
        "label": "M3 help's usage block goes to stderr instead",
        "file": "src/main.rs",
        "old": 'fn print_help() {\n    println!("{}", SYNOPSIS);',
        "new": 'fn print_help() {\n    eprintln!("{}", SYNOPSIS);',
        "test": "help_exits_zero_with_usage_and_no_stderr",
        # This same entry should also turn help_stdout_has_full_synopsis red.
        "note": "Pins down the split: help goes to stdout, only an error usage goes to stderr.",
    },
    {
        "label": "M4 remove the value-position skip (reverts the regression from before fix R1)",
        "file": "src/main.rs",
        "old": "        if Some(i) == eval_or_script_value_index {\n            continue;\n        }\n",
        "new": "",
        "test": "eval_help_is_evaluated_as_elisp_not_treated_as_a_flag",
        # This same entry should also turn red:
        # eval_dash_v_is_evaluated_as_elisp_not_treated_as_a_flag,
        # script_dash_v_is_treated_as_a_filename_not_a_flag
        "note": (
            "This is the real regression the reviewer caught: the pre-scan "
            "also consumes --eval/--script's argument value, silently "
            "swapping an exit-1 error for an exit-0 help/version output."
        ),
    },
    {
        "label": "M5 the skipped position changed to the wrong index",
        "file": "src/main.rs",
        "old": '        Some("--eval") | Some("--script") => Some(1),',
        "new": '        Some("--eval") | Some("--script") => Some(2),',
        "test": "script_dash_v_is_treated_as_a_filename_not_a_flag",
        # This same entry should also turn red:
        # eval_help_is_evaluated_as_elisp_not_treated_as_a_flag,
        # eval_dash_v_is_evaluated_as_elisp_not_treated_as_a_flag
        "note": "Confirms it guards \"index 1 is specifically the value "
                "position\", not just \"something gets skipped\".",
    },
    {
        "label": "M6 the skip range over-expands: nothing in argv gets scanned at all in batch mode",
        "file": "src/main.rs",
        "old": "        if Some(i) == eval_or_script_value_index {",
        "new": "        if eval_or_script_value_index.is_some() && i >= 1 {",
        "test": "eval_expr_then_help_still_overrides_and_still_skips_evaluation",
        # This same entry should also turn help_overrides_batch_mode_flags red.
        "note": (
            "The opposite direction of mistake: if fixing R1 also broke the "
            "deliberate behavior of \"--eval EXPR --help prints help\", a "
            "test needs to catch it."
        ),
    },
    {
        "label": "M7 man page removes the -h short alias (reviewer's original M8, unobservable before the fix)",
        "file": "doc/reticle.1",
        "old": ".B \\-h, \\-\\-help",
        "new": ".B \\-\\-help",
        "test": "man_page_documents_every_flag",
        "note": (
            "Before R2 this entry would SURVIVE: the token is the bare "
            "\"-h\", and since it's a substring of \"--help\", the leftover "
            "--help in the EXIT STATUS section alone is enough to make the "
            "assertion always hold."
        ),
    },
    {
        "label": "M8 help text removes the -h short alias (reviewer's original M9, unobservable before the fix)",
        "file": "src/main.rs",
        "old": '    println!("  -h, --help      print this help and exit");',
        "new": '    println!("  --help          print this help and exit");',
        "test": "help_mentions_every_flag",
        "note": "Same as M7, just on the help-text side.",
    },
    {
        "label": "M9 the single source of truth SYNOPSIS gets altered",
        "file": "src/main.rs",
        "old": '       reticle --script FILE";',
        "new": '       reticle --script PATH";',
        "test": "unknown_option_stderr_has_full_synopsis",
        # This same entry should also turn help_stdout_has_full_synopsis red.
        "note": (
            "The test side hard-codes an independent copy of FULL_SYNOPSIS, "
            "so changing a single character in SYNOPSIS turns both sides "
            "red -- exactly the guarding power R6 wanted. The first version "
            "of this mutation changed FILE to FILENAME, and the harness "
            "reported SURVIVED: because \"--script FILENAME\" still "
            "contains the substring \"--script FILE\", contains() still "
            "held -- **the mutation didn't hit the target, it wasn't that "
            "nobody guards it**. Changing it to PATH actually changes the "
            "string. When testing with contains, a mutation must replace "
            "characters, not append after them."
        ),
    },
    {
        "label": "M10 the man page's .TH version number drifts",
        "file": "doc/reticle.1",
        "old": '.TH RETICLE 1 "2026-08-27" "reticle 0.1.0" "User Commands"',
        "new": '.TH RETICLE 1 "2026-08-27" "reticle 0.9.9" "User Commands"',
        "test": "man_page_th_version_matches_cargo_version",
        "note": "roff has no way to interpolate the version at build time, so a test pins it against Cargo.toml instead.",
    },
    {
        "label": "M11 the exit code for an unknown flag",
        "file": "src/main.rs",
        "old": "            usage();\n            return 2;\n        }\n    };\n    let (interp, ed) = start_session(&opts);\n    match frontend_gui::run_gui(interp, ed) {",
        "new": "            usage();\n            return 1;\n        }\n    };\n    let (interp, ed) = start_session(&opts);\n    match frontend_gui::run_gui(interp, ed) {",
        "test": "unknown_option_still_errors",
        "note": (
            "The usage()+return 2 shape looks the same on both the run_gui "
            "and run_tui paths, so the `old` string has to extend a few more "
            "lines down into the part specific to run_gui to match uniquely."
        ),
    },
    {
        "label": "M12 the message for --eval missing its argument",
        "file": "src/main.rs",
        "old": 'eprintln!("usage: reticle --eval EXPR");',
        "new": 'eprintln!("usage: reticle --eval");',
        "test": "eval_missing_argument_still_errors",
        "note": "This line deliberately does not share SYNOPSIS (it's more helpful than the generic usage), so it needs to be pinned separately.",
    },
    {
        "label": "M13 the message for --script missing its argument",
        "file": "src/main.rs",
        "old": 'eprintln!("usage: reticle --script FILE");',
        "new": 'eprintln!("usage: reticle --script");',
        "test": "script_missing_argument_still_errors",
        "note": "Same as M12.",
    },
    {
        "label": "M14 the value-position list mistakenly counts --repl too",
        "file": "src/main.rs",
        "old": '        Some("--eval") | Some("--script") => Some(1),',
        "new": '        Some("--eval") | Some("--script") | Some("--repl") => Some(1),',
        "test": "repl_help_prints_help_and_does_not_start_a_repl",
        "note": (
            "Designed by the trailing re-review. --repl doesn't consume any "
            "following argument, so it shouldn't have a value position; if "
            "mistakenly added, `--repl --help`'s --help gets skipped and it "
            "launches a REPL instead. This entry was originally totally "
            "black-box (no test touched it at all); it only became "
            "observable after a test was added in the same round."
        ),
    },
]

# Honestly recorded: the following guards have no mutation watching them,
# don't claim they're covered.
#
# - The RETICLE_REMOTE_TIMEOUT clamp range [0.001, 86400] that R3 added
#   to the man page: no test verifies the correctness of the man page's
#   prose, only existence checks like "do these 12 tokens appear". Really
#   guarding it would need a test that cross-references remote.rs's
#   constants against the man page text, which is a cross-crate test that
#   reads source code -- cost outweighs benefit this time, not done.
#
# - **This entry was originally something I got wrong, kept as a record**:
#   the first version said "`--repl --help` has no test, currently covered
#   indirectly by M6". The trailing re-review pointed out the second half is
#   false -- M6's condition is `eval_or_script_value_index.is_some()`, and
#   when `args[0] == "--repl"` that's always `None`, so M6 has no effect on
#   this path at all. Writing a totally black-box item as "partially
#   covered" is worse than not writing it at all. The test
#   `repl_help_prints_help_and_does_not_start_a_repl` and M14 have since
#   been added, and this is no longer a gap.
#
# - The man page's `--repl` `.TP` section and `print_help()`'s Notes
#   section: their **full sentence meaning** is likewise unguarded by any
#   test -- `help_mentions_every_flag` and `man_page_documents_every_flag`
#   only do token-by-token existence checks, so a sentence can be worded
#   wrong while its tokens remain and both stay green. Same category as the
#   REMOTE_TIMEOUT gap above.
