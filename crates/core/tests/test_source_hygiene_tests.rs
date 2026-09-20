//! M145 Part A: a floor check for the exact failure shape that motivated
//! this milestone -- a test that silently prints a diagnostic naming a
//! deliberate early return, worded to say it is skipping, when an
//! external tool is missing, with no opt-out env var anywhere nearby.
//! `cargo test --workspace --no-fail-fast` (this project's own
//! definition of done, no `--nocapture`) discards a PASSING test's
//! stdout/stderr, so that shape lets a green gate mean the test never
//! ran at all -- exactly the failure `lisp_hygiene_tests.rs` closes for
//! `.el` docstrings, applied here to Rust test sources instead. Modeled
//! directly on that file's own walk-and-report shape
//! (`el_source_dirs`/`find_el_files`/the `scanned_files > 10` floor).
//!
//! The rule: a print-macro call line (actual code, not a comment) whose
//! text (that line or either of the next two) also contains the word
//! for "this is being skipped" requires one of the two opt-out env var
//! prefixes to appear somewhere in the preceding 15 lines -- either
//! directly, or via a same-file `const NAME: &str = "...";` whose
//! value contains one of those prefixes and whose NAME is referenced in
//! that window (the convention `dev_tools_tests.rs`/
//! `demo_smoke_tests.rs` already use: the env var name is a string
//! literal only at the `const` declaration, and call sites elsewhere in
//! the function mention only the identifier -- often more than 15 lines
//! below the declaration). A print inside a function carrying the
//! ignore attribute is exempt -- cargo already reports those as
//! ignored, not as passed, so it is not this failure shape.
//!
//! Deliberately checked only on non-comment lines: a doc comment
//! *describing* this rule (this file's own header, unavoidably) would
//! otherwise trip the very check it documents.
//!
//! **This test itself must scan a substantial number of files** (see
//! `MIN_SCANNED_FILES` below) -- a scanner that silently finds zero
//! `.rs` files passes trivially, which is the exact "mechanism that
//! decides what gets run must fail loudly when it decides nothing does"
//! trap this project's own M114 hit four times in one tool.
//!
//! **Known blind spots, left in deliberately rather than papered over**:
//! a skip with no print at all (the shape `shaping.rs` used before
//! M145 -- a bare early `return`, nothing printed anywhere); wording
//! that never contains "skipping" (any other phrasing of "this test
//! did not run"); `#[cfg(test)]` modules under `src/` (this scanner
//! only walks `crates/*/tests/*.rs`, never `src/`); and an opt-out env
//! var name that appears only inside a comment within the window --
//! this scanner checks text presence, not whether that text is live
//! gating logic reachable at runtime (`require_tool_tests` in each of
//! the five e2e files covers that half of the question instead).

use std::path::{Path, PathBuf};

/// Below this, something is wrong with the walk itself (repo layout
/// changed, `crates/` stopped resolving, etc.) -- comfortably below the
/// real file count under `crates/*/tests/` (which grows over time as
/// this workspace gains tests) while still being a floor that would
/// catch a walk gone badly wrong.
const MIN_SCANNED_FILES: usize = 50;

fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Every `.rs` file directly under `crates/<any>/tests/` (non-recursive,
/// matching the `crates/*/tests/*.rs` glob this test is specified
/// against -- none of this workspace's test directories nest further).
fn find_test_source_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates_dir = repo_root().join("crates");
    let crate_entries = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("failed to read {:?}: {}", crates_dir, e));
    for crate_entry in crate_entries {
        let crate_entry = crate_entry.unwrap_or_else(|e| panic!("failed to read entry: {}", e));
        let crate_path = crate_entry.path();
        if !crate_path.is_dir() {
            continue;
        }
        let tests_dir = crate_path.join("tests");
        if !tests_dir.is_dir() {
            continue;
        }
        let test_entries = std::fs::read_dir(&tests_dir)
            .unwrap_or_else(|e| panic!("failed to read {:?}: {}", tests_dir, e));
        for test_entry in test_entries {
            let test_entry = test_entry.unwrap_or_else(|e| panic!("failed to read entry: {}", e));
            let path = test_entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    out
}

/// An un-gated silent-skip diagnostic print with no opt-out env var
/// nearby, and not inside a `#[ignore]`d test -- see this file's header.
struct Violation {
    file: PathBuf,
    line: usize,
}

const HOW_MANY_LINES_AHEAD_TO_CHECK_FOR_SKIP_WORD: usize = 2;
const HOW_MANY_LINES_BACK_TO_CHECK_FOR_OPT_OUT: usize = 15;

/// Whether `lines[fn_line_idx]` (a `fn ...` declaration line) is the
/// start of a test function carrying `#[ignore]` directly above it --
/// walks upward over consecutive attribute (`#[...]`) lines, since
/// `#[test]`/`#[ignore]` can appear in either order and with other
/// attributes between them.
fn fn_is_ignored(lines: &[&str], fn_line_idx: usize) -> bool {
    let mut i = fn_line_idx;
    while i > 0 {
        let prev = lines[i - 1].trim();
        if prev.starts_with("#[") {
            if prev.contains("ignore") {
                return true;
            }
            i -= 1;
        } else if prev.is_empty() {
            // Blank lines between doc comments/attributes and `fn` are
            // not expected in this codebase's style, but tolerate one
            // rather than mis-detecting on it.
            i -= 1;
        } else if prev.starts_with("//") {
            // A doc comment (`///`) or plain comment (`//`) between
            // `#[ignore]` and `fn` -- keep walking up past it instead
            // of stopping, or `#[test]\n#[ignore]\n/// doc\nfn` would
            // be mis-detected as not ignored.
            i -= 1;
        } else {
            break;
        }
    }
    false
}

/// Search backward from `line_idx` (a line inside a function body) for
/// the nearest enclosing `fn` declaration, then decide if the test
/// carries `#[ignore]`. Returns `false` (not ignored) if no `fn` line
/// is found at all -- e.g. the `eprintln!` sits inside a bare helper
/// function rather than a `#[test]`, which must still be held to the
/// opt-out-env-var-nearby rule.
fn enclosing_fn_is_ignored(lines: &[&str], line_idx: usize) -> bool {
    let mut i = line_idx;
    loop {
        let trimmed = lines[i].trim_start();
        if trimmed.starts_with("fn ") || trimmed.starts_with("pub fn ") {
            return fn_is_ignored(lines, i);
        }
        if i == 0 {
            return false;
        }
        i -= 1;
    }
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether IDENT appears in HAYSTACK as a whole identifier (not merely
/// as a substring of a longer one) -- used to check whether a `const`'s
/// NAME (see `build_const_string_map`) is actually referenced in a
/// window of source, rather than e.g. `FOO_SKIP_ENV` false-matching
/// inside `BAR_FOO_SKIP_ENV_2`.
fn contains_identifier(haystack: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(ident) {
        let abs = start + pos;
        let before_ok = abs == 0 || !is_ident_char(bytes[abs - 1]);
        let after = abs + ident.len();
        let after_ok = after >= bytes.len() || !is_ident_char(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
        if start >= haystack.len() {
            break;
        }
    }
    false
}

/// Builds a NAME -> VALUE map of every `const NAME: &str = "VALUE";`
/// declaration in the file (single-line only -- every such declaration
/// in this workspace's test sources fits on one line). This lets the
/// opt-out check see through the `dev_tools_tests.rs`/
/// `demo_smoke_tests.rs` convention of naming the env var once at a
/// `const` far above the function, then referencing only the
/// identifier at each call site.
fn build_const_string_map(lines: &[&str]) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in lines {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("const ")
            .or_else(|| trimmed.strip_prefix("pub const "))
        else {
            continue;
        };
        let Some(colon_idx) = rest.find(':') else {
            continue;
        };
        let name = rest[..colon_idx].trim();
        let Some(eq_idx) = rest.find('=') else {
            continue;
        };
        let after_eq = &rest[eq_idx + 1..];
        let Some(open_quote) = after_eq.find('"') else {
            continue;
        };
        let rest_after_open = &after_eq[open_quote + 1..];
        let Some(close_quote) = rest_after_open.find('"') else {
            continue;
        };
        let value = &rest_after_open[..close_quote];
        map.insert(name.to_string(), value.to_string());
    }
    map
}

/// Whether a RETICLE opt-out env var is visible in WINDOW: directly (the
/// literal `RETICLE_ALLOW_MISSING_`/`RETICLE_SKIP_` prefix text), or
/// indirectly via a `const` (from CONST_MAP) whose value carries that
/// prefix and whose name is referenced by identifier in WINDOW.
fn window_has_opt_out(window: &str, const_map: &std::collections::HashMap<String, String>) -> bool {
    if window.contains("RETICLE_ALLOW_MISSING_") || window.contains("RETICLE_SKIP_") {
        return true;
    }
    const_map.iter().any(|(name, value)| {
        (value.contains("RETICLE_ALLOW_MISSING_") || value.contains("RETICLE_SKIP_"))
            && contains_identifier(window, name)
    })
}

/// The actual scan, over an in-memory source string rather than a real
/// file on disk -- split out from `scan_file` so `require_tool_tests`
/// style fixture tests (see `scan_source_tests` below) can exercise the
/// detection logic itself with inline source strings, instead of only
/// getting an end-to-end answer of 0/nonzero against the real tree.
/// NAME is only used to build the `Violation`s it returns; it need not
/// be a real path.
fn scan_source(name: &str, source: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let const_map = build_const_string_map(&lines);
    for (i, line) in lines.iter().enumerate() {
        // Only actual code lines are considered -- a comment (`//`,
        // `///`, `//!`) merely mentioning the print macro (this file's
        // own header does, unavoidably, describing the rule) is not a
        // silent-skip site.
        if line.trim_start().starts_with("//") {
            continue;
        }
        if !line.contains("eprintln!") && !line.contains("println!") {
            continue;
        }
        let window_end = (i + HOW_MANY_LINES_AHEAD_TO_CHECK_FOR_SKIP_WORD + 1).min(lines.len());
        let window = lines[i..window_end].join("\n");
        if !window.to_lowercase().contains("skipping") {
            continue;
        }
        let back_start = i.saturating_sub(HOW_MANY_LINES_BACK_TO_CHECK_FOR_OPT_OUT);
        let backward_window = lines[back_start..i].join("\n");
        if window_has_opt_out(&backward_window, &const_map) {
            continue;
        }
        if enclosing_fn_is_ignored(&lines, i) {
            continue;
        }
        violations.push(Violation {
            file: PathBuf::from(name),
            // 1-based, matching editors/`cargo`'s own line numbering.
            line: i + 1,
        });
    }
    violations
}

fn scan_file(path: &Path, violations: &mut Vec<Violation>) {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {:?}: {}", path, e));
    violations.extend(scan_source(&path.display().to_string(), &content));
}

#[test]
fn no_ungated_silent_skip_in_test_sources() {
    let files = find_test_source_files();
    assert!(
        files.len() >= MIN_SCANNED_FILES,
        "expected to scan at least {} files under crates/*/tests/, only found {} -- \
         did the walk stop resolving correctly (repo layout changed, wrong cwd, etc.)?",
        MIN_SCANNED_FILES,
        files.len()
    );

    let mut violations = Vec::new();
    for path in &files {
        scan_file(path, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "found {} ungated silent-skip diagnostic-print site(s) under crates/*/tests/ \
         (a print call whose own text, or one of the next two lines, says the test is \
         being skipped) -- none of RETICLE_ALLOW_MISSING_*/RETICLE_SKIP_* appears in \
         the preceding {} lines, and the enclosing test is not #[ignore]d. `cargo test \
         --workspace --no-fail-fast` (this project's definition of done, no \
         `--nocapture`) discards stdout/stderr from a PASSING test, so this shape lets \
         a green gate mean the check silently never ran. Add an opt-out env var\
         following `dev_tools_tests.rs`'s `require_*` convention, or mark the test \
         `#[ignore]` if it is genuinely meant to be a manual-only test:\n{}",
        violations.len(),
        HOW_MANY_LINES_BACK_TO_CHECK_FOR_OPT_OUT,
        violations
            .iter()
            .map(|v| format!("  {}:{}", v.file.display(), v.line))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Fixture tests for `scan_source` itself, on inline source strings --
/// item 1 of the M145 fix round. Without these, breaking the detection
/// (e.g. `if !line.contains("eprintln!")` changed to a string that
/// never matches) leaves `no_ungated_silent_skip_in_test_sources` green,
/// since the real tree has 0 violations either way.
#[cfg(test)]
mod scan_source_tests {
    use super::scan_source;

    // Built via `concat!` so this *file's own source text* never
    // literally contains a contiguous "eprintln!"/"println!" call
    // followed by "skipping" -- these fixtures deliberately reconstruct
    // the exact shape this scanner looks for, and if written as plain
    // string literals, running `no_ungated_silent_skip_in_test_sources`
    // against the real tree would trip on this file's own fixture data,
    // which is not a real defect (see item 6's "known blind spots" note
    // above: this scanner cannot tell code from a string literal
    // holding the same text).
    const EPRINTLN_MACRO: &str = concat!("epr", "intln!");
    const PRINTLN_MACRO: &str = concat!("prin", "tln!");
    const SKIP_WORD: &str = concat!("skip", "ping");
    const SKIP_WORD_CAP: &str = concat!("Skip", "ping");

    #[test]
    fn ungated_eprintln_is_reported_with_correct_line_number() {
        let src = format!(
            "fn helper() {{}}\n\n#[test]\nfn some_test() {{\n    if !have_tool() {{\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert_eq!(
            violations.len(),
            1,
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
        assert_eq!(violations[0].line, 6);
    }

    #[test]
    fn literal_opt_out_env_var_in_window_is_not_reported() {
        let src = format!(
            "#[test]\nfn some_test() {{\n    if !have_tool() {{\n        // RETICLE_ALLOW_MISSING_FOO opts out\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert!(
            violations.is_empty(),
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn const_shape_opt_out_far_above_is_not_reported() {
        // The `dev_tools_tests.rs`/`demo_smoke_tests.rs` convention: the
        // env var name is a string literal only at the `const`
        // declaration (here, well over 15 lines above the call site),
        // and the call site references only the identifier.
        let mut lines =
            vec!["const FOO_SKIP_ENV: &str = \"RETICLE_ALLOW_MISSING_FOO\";".to_string()];
        for i in 0..28 {
            lines.push(format!("// filler line {}", i));
        }
        lines.push("#[test]".to_string());
        lines.push("fn some_test() {".to_string());
        lines.push("    if !have_tool() {".to_string());
        lines.push("        let _ = FOO_SKIP_ENV;".to_string());
        lines.push(format!(
            "        {}(\"{}: foo not on PATH\");",
            EPRINTLN_MACRO, SKIP_WORD
        ));
        lines.push("        return;".to_string());
        lines.push("    }".to_string());
        lines.push("}".to_string());
        let src = lines.join("\n");
        let violations = scan_source("fixture.rs", &src);
        assert!(
            violations.is_empty(),
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unrelated_const_without_the_prefix_is_still_reported() {
        let src = format!(
            "const SOME_OTHER: &str = \"NOT_AN_OPT_OUT\";\n\n#[test]\nfn some_test() {{\n    if !have_tool() {{\n        let _ = SOME_OTHER;\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert_eq!(
            violations.len(),
            1,
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_with_ignore_attribute_is_not_reported() {
        let src = format!(
            "#[test]\n#[ignore]\nfn some_test() {{\n    if !have_tool() {{\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert!(
            violations.is_empty(),
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_with_ignore_attribute_then_doc_comment_is_not_reported() {
        let src = format!(
            "#[test]\n#[ignore]\n/// doc comment describing the manual test\nfn some_test() {{\n    if !have_tool() {{\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert!(
            violations.is_empty(),
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn multiline_eprintln_call_is_reported() {
        let src = format!(
            "#[test]\nfn some_test() {{\n    if !have_tool() {{\n        {}(\n            \"{{}}\",\n            \"{}: foo not on PATH\"\n        );\n        return;\n    }}\n}}\n",
            EPRINTLN_MACRO, SKIP_WORD
        );
        let violations = scan_source("fixture.rs", &src);
        assert_eq!(
            violations.len(),
            1,
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
        assert_eq!(violations[0].line, 4);
    }

    #[test]
    fn capitalised_skipping_and_println_variant_are_reported() {
        let src = format!(
            "#[test]\nfn some_test() {{\n    if !have_tool() {{\n        {}(\"{}: foo not on PATH\");\n        return;\n    }}\n}}\n",
            PRINTLN_MACRO, SKIP_WORD_CAP
        );
        let violations = scan_source("fixture.rs", &src);
        assert_eq!(
            violations.len(),
            1,
            "{:?}",
            violations.iter().map(|v| v.line).collect::<Vec<_>>()
        );
    }
}
