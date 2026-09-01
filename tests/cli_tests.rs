//! Tests for the CLI facade added in M78: `--help`, `--version`, and their
//! consistency with `doc/reticle.1`.
//!
//! These tests only ever invoke the binary with `--help`, `--version`, `-h`,
//! `-v`, `-V`, `--eval`, or an unknown flag — all paths that necessarily
//! `process::exit` before any GUI window or TUI session would be entered.
//! Anything that reaches `run_gui`/`run_tui` (bare `-nw`, `-q`, a bare FILE,
//! `--gui`, `--repl` without further flags, ...) either opens a real GUI
//! window or needs a real tty, and would hang the test runner forever — do
//! not add such a call here.

use std::process::Command;

/// Mirrors the helper in `tests/worker_tests.rs`: run the binary under test
/// and collect (exit_code, stdout, stderr) as UTF-8.
fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_reticle"))
        .args(args)
        .output()
        .expect("failed to run reticle");
    (
        output.status.code().expect("process should exit normally"),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// M78 review R2: bare "-h" is a substring of "--help" and bare "-v" is a
// substring of "--version", so a `contains("-h")`/`contains("-v")` check is
// vacuously true regardless of whether the short alias is actually
// documented on its own. Check the full alias-pair spelling instead, which
// only appears if the short form is genuinely listed.
const HELP_TOKENS: &[&str] = &[
    "-nw",
    "--tui",
    "--gui",
    "-q",
    "--repl",
    "--eval",
    "--script",
    "-h, --help",
    "--help",
    "-v, -V, --version",
    "-V",
    "--version",
];

// M78 review R6: written independently of `main.rs`'s `SYNOPSIS` const so
// that a drift between `usage()` (stderr) and `--help` (stdout) — or a
// silent edit to only one of them — makes this test fail. An integration
// test can't reach the bin crate's private `SYNOPSIS` item, hence the
// duplication is deliberate.
const FULL_SYNOPSIS: &str = "\
usage: reticle [FILE] [-nw|--tui|--gui] [-q]
       reticle --repl
       reticle --eval EXPR
       reticle --script FILE";

#[test]
fn help_exits_zero_with_usage_and_no_stderr() {
    let (code, stdout, stderr) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");
    assert!(stderr.is_empty(), "stderr was: {stderr}");
}

#[test]
fn short_help_matches_long_help_verbatim() {
    let (_, stdout_long, _) = run(&["--help"]);
    let (_, stdout_short, _) = run(&["-h"]);
    assert_eq!(stdout_long, stdout_short);
}

#[test]
fn version_exits_zero_with_exact_output() {
    let (code, stdout, stderr) = run(&["--version"]);
    assert_eq!(code, 0);
    assert_eq!(stdout, format!("reticle {}\n", env!("CARGO_PKG_VERSION")));
    assert!(stderr.is_empty(), "stderr was: {stderr}");
}

#[test]
fn short_version_flags_match_long_version_verbatim() {
    let (_, stdout_long, _) = run(&["--version"]);
    let (_, stdout_v, _) = run(&["-v"]);
    let (_, stdout_cap_v, _) = run(&["-V"]);
    assert_eq!(stdout_long, stdout_v);
    assert_eq!(stdout_long, stdout_cap_v);
}

#[test]
fn help_mentions_every_flag() {
    let (_, stdout, _) = run(&["--help"]);
    for tok in HELP_TOKENS {
        assert!(
            stdout.contains(tok),
            "help output is missing token {:?}; stdout was: {}",
            tok,
            stdout
        );
    }
}

#[test]
fn help_overrides_batch_mode_flags() {
    let (code, stdout, _) = run(&["--eval", "(+ 1 2)", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");
    assert!(
        !stdout.contains('3'),
        "--eval must not have been evaluated; stdout was: {stdout}"
    );
}

#[test]
fn first_of_help_or_version_wins() {
    let (_, stdout, _) = run(&["--help", "--version"]);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");

    let (_, stdout, _) = run(&["--version", "--help"]);
    assert_eq!(stdout, format!("reticle {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn man_page_documents_every_flag() {
    let man_path = concat!(env!("CARGO_MANIFEST_DIR"), "/doc/reticle.1");
    let raw = std::fs::read_to_string(man_path).expect("doc/reticle.1 should exist");
    // roff escapes hyphens as `\-` for proper minus-sign rendering (required
    // by the milestone spec for NAME/option flags); undo that escaping
    // before substring-matching plain option tokens like `--tui`.
    let contents = raw.replace("\\-", "-");
    for tok in HELP_TOKENS.iter().chain(std::iter::once(&"--worker")) {
        assert!(
            contents.contains(tok),
            "man page is missing token {:?}",
            tok
        );
    }
}

// M78 review R4: the `.TH` line's version is a roff literal (roff can't
// interpolate `CARGO_PKG_VERSION` at build time), so nothing enforces it
// staying in sync with a `Cargo.toml` version bump except this test.
#[test]
fn man_page_th_version_matches_cargo_version() {
    let man_path = concat!(env!("CARGO_MANIFEST_DIR"), "/doc/reticle.1");
    let contents = std::fs::read_to_string(man_path).expect("doc/reticle.1 should exist");
    // Tail review: a bare `contents.contains(VERSION)` would also pass if the
    // version string merely happened to appear somewhere else in the page, so
    // pin it to the `.TH` line itself.
    let th_line = contents
        .lines()
        .find(|l| l.starts_with(".TH "))
        .expect("doc/reticle.1 should have a .TH line");
    assert!(
        th_line.contains(env!("CARGO_PKG_VERSION")),
        "doc/reticle.1's .TH line does not carry the current crate version {}; line was: {}",
        env!("CARGO_PKG_VERSION"),
        th_line
    );
}

// Tail review: `--repl --help` was the one flag-position path with no test at
// all. It is safe to drive here because `Command::output()` gives the child a
// null stdin, so even if the pre-scan regressed and a REPL did start, it would
// hit EOF immediately instead of hanging.
#[test]
fn repl_help_prints_help_and_does_not_start_a_repl() {
    let (code, stdout, stderr) = run(&["--repl", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");
    assert!(
        !stdout.contains("elisp>"),
        "a REPL prompt was printed, so --help did not win: {stdout}"
    );
    assert!(stderr.is_empty(), "stderr was: {stderr}");
}

// M78 review R6: the two synopsis emission sites (usage() on stderr,
// print_help() on stdout) must both carry the *entire* four-line synopsis,
// not merely a shared "usage:" prefix -- the old two-line format would also
// have passed a mere `contains("usage:")` check.
#[test]
fn unknown_option_stderr_has_full_synopsis() {
    let (_, _, stderr) = run(&["--bogus"]);
    assert!(
        stderr.contains(FULL_SYNOPSIS),
        "stderr did not contain the full synopsis; stderr was: {stderr}"
    );
}

#[test]
fn help_stdout_has_full_synopsis() {
    let (_, stdout, _) = run(&["--help"]);
    assert!(
        stdout.contains(FULL_SYNOPSIS),
        "stdout did not contain the full synopsis; stdout was: {stdout}"
    );
}

// -----------------------------------------------------------------------
// Regression: behavior that predates M78 and must not be disturbed by the
// new --help/--version pre-scan.
// -----------------------------------------------------------------------

#[test]
fn unknown_option_still_errors() {
    let (code, stdout, stderr) = run(&["--bogus"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("unknown option: --bogus"),
        "stderr was: {stderr}"
    );
    assert!(stderr.contains("usage:"), "stderr was: {stderr}");
    assert!(stdout.is_empty(), "stdout was: {stdout}");
}

#[test]
fn eval_missing_argument_still_errors() {
    let (code, _, stderr) = run(&["--eval"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("usage: reticle --eval EXPR"),
        "stderr was: {stderr}"
    );
}

#[test]
fn script_missing_argument_still_errors() {
    let (code, _, stderr) = run(&["--script"]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("usage: reticle --script FILE"),
        "stderr was: {stderr}"
    );
}

#[test]
fn eval_still_works() {
    let (code, stdout, _) = run(&["--eval", "(+ 1 2)"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "3");
}

// -----------------------------------------------------------------------
// M78 review R1: the --help/--version pre-scan must not swallow the VALUE
// that --eval/--script consume at args[1] -- only genuine flag positions
// are pre-scanned.
// -----------------------------------------------------------------------

#[test]
fn eval_help_is_evaluated_as_elisp_not_treated_as_a_flag() {
    let (code, stdout, _stderr) = run(&["--eval", "--help"]);
    assert_eq!(code, 1);
    assert!(
        !stdout.contains("usage:"),
        "--eval --help must not print help; stdout was: {stdout}"
    );
}

#[test]
fn eval_dash_v_is_evaluated_as_elisp_not_treated_as_a_flag() {
    let (code, stdout, _stderr) = run(&["--eval", "-v"]);
    assert_eq!(code, 1);
    assert!(
        !stdout.contains("reticle 0.1.0")
            && !stdout.contains(&format!("reticle {}", env!("CARGO_PKG_VERSION"))),
        "--eval -v must not print version; stdout was: {stdout}"
    );
}

#[test]
fn script_dash_v_is_treated_as_a_filename_not_a_flag() {
    let (code, stdout, _stderr) = run(&["--script", "-v"]);
    assert_eq!(code, 1);
    assert!(
        !stdout.contains("reticle 0.1.0")
            && !stdout.contains(&format!("reticle {}", env!("CARGO_PKG_VERSION"))),
        "--script -v must not print version; stdout was: {stdout}"
    );
}

#[test]
fn eval_expr_then_help_still_overrides_and_still_skips_evaluation() {
    let (code, stdout, _stderr) = run(&["--eval", "(+ 1 2)", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage:"), "stdout was: {stdout}");
    assert!(
        !stdout.contains('3'),
        "--eval must not have been evaluated; stdout was: {stdout}"
    );
}
