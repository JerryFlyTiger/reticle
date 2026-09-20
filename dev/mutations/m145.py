# Mutation list for M145 (tests that silently passed when their external tool
# was missing). Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m145.py
#
# M145 ships these effects, each with a deletion entry below:
#
#   P   Per-file `require_tool`: a missing tool with no affirmative opt-out
#       panics instead of returning. One deletion entry per file (P1-P5),
#       plus P6 for "only exactly 1/true/yes opts out".
#   W   The wrapper feeds the file's real probe into `require_tool` (W1,
#       killed because every tool is installed here). W2 and W3 are
#       declared survivors: bypassing a call site, or never reading the
#       opt-out variable, is only observable on a machine where the tool is
#       absent (or with the variable set), which `cargo test` here is not.
#   H   `test_source_hygiene_tests.rs`: detection (H1), case-insensitive
#       wording (H2), `#[ignore]` walk past doc comments (H3), literal
#       opt-out recognition (H4), the file-count floor (H5), the whole scan
#       (H6), `#[ignore]` exemption itself (H7), const-name opt-out
#       recognition (H8), and `println!` detection (H9). H7-H9 were added
#       after the round-2 trailing review found those effects had no entry.
#   S   shaping.rs font tests load the bundled font instead of
#       `~/Library/Fonts`. S1 is a declared survivor: this machine HAS
#       `~/Library/Fonts/JetBrainsMono-Regular.ttf` (same 273900 bytes as
#       the bundled one, per the M145 reviewer), so reverting to the
#       read-and-return shape still runs the assertions. The fix's value is
#       only observable on a machine without that file.
#
# Reading a SURVIVED result: confirm the mutation actually LANDED and actually
# changed semantics before reading it as a coverage gap.

PACKAGE = "core"

_PANIC_OLD = """    panic!(
        "{} is not on PATH -- this e2e test was not run."""
_PANIC_NEW = """    #[allow(unreachable_code)]
    return false;
    panic!(
        "{} is not on PATH -- this e2e test was not run."""


def _p(label, stem):
    return {
        "label": label,
        "file": f"crates/core/tests/{stem}.rs",
        "old": _PANIC_OLD,
        "new": _PANIC_NEW,
        "test": "absent_and_no_opt_out_panics_naming_the_env_var",
        "test_target": stem,
    }


_HY = "crates/core/tests/test_source_hygiene_tests.rs"

MUTATIONS = [
    # ---- P: require_tool fails loudly ---------------------------------------
    _p("P1 DELETION format_tests require_tool returns false instead of panicking", "format_tests"),
    _p("P2 DELETION lsp_format_tests require_tool returns false instead of panicking", "lsp_format_tests"),
    _p("P3 DELETION lsp_mode_tests require_tool returns false instead of panicking", "lsp_mode_tests"),
    _p("P4 DELETION lsp_tests require_tool returns false instead of panicking", "lsp_tests"),
    _p("P5 DELETION search_tests require_tool returns false instead of panicking", "search_tests"),
    {
        "label": "P6 search_tests: \"0\" also opts out",
        "file": "crates/core/tests/search_tests.rs",
        "old": 'let opted_out = matches!(env_value, Some("1") | Some("true") | Some("yes"));',
        "new": 'let opted_out = matches!(env_value, Some("0") | Some("1") | Some("true") | Some("yes"));',
        "test": "absent_and_env_set_to_0_still_panics",
        "test_target": "search_tests",
    },
    # ---- W: wiring ------------------------------------------------------------
    {
        "label": "W1 require_rg is fed a constant absent probe instead of has_rg()",
        "file": "crates/core/tests/search_tests.rs",
        "old": 'require_tool("rg", has_rg(), RG_SKIP_ENV, env_value.as_deref())',
        "new": 'require_tool("rg", false, RG_SKIP_ENV, env_value.as_deref())',
        "test": "_available",  # all six rg tests; "if_rg_available" misses search_e2e_with_real_rg_if_available
        "test_target": "search_tests",
    },
    {
        "label": "W2 DECLARED SURVIVOR connect_and_shutdown no longer calls require_rust_analyzer",
        "file": "crates/core/tests/lsp_tests.rs",
        "old": "if !require_rust_analyzer() {",
        "new": "if !true {",
        "test": "connect_and_shutdown",
        "test_target": "lsp_tests",
        "expect": "survived",
    },
    {
        "label": "W3 DECLARED SURVIVOR require_rg never reads RETICLE_ALLOW_MISSING_RG",
        "file": "crates/core/tests/search_tests.rs",
        "old": "let env_value = std::env::var(RG_SKIP_ENV).ok();",
        "new": "let env_value: Option<String> = None;",
        "test": "_available",  # all six rg tests; "if_rg_available" misses search_e2e_with_real_rg_if_available
        "test_target": "search_tests",
        "expect": "survived",
    },
    # ---- H: the hygiene scanner ----------------------------------------------
    {
        "label": "H1 scanner never looks at print lines",
        "file": _HY,
        "old": 'if !line.contains("eprintln!") && !line.contains("println!") {',
        "new": "if true {",
        "test": "ungated_eprintln_is_reported_with_correct_line_number",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H2 skip word matched case-sensitively",
        "file": _HY,
        "old": 'if !window.to_lowercase().contains("skipping") {',
        "new": 'if !window.contains("skipping") {',
        "test": "capitalised_skipping_and_println_variant_are_reported",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H3 #[ignore] walk stops at a doc comment",
        "file": _HY,
        "old": '} else if prev.starts_with("//") {',
        "new": '} else if prev.starts_with("//NEVER-MATCHES") {',
        "test": "test_with_ignore_attribute_then_doc_comment_is_not_reported",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H4 literal opt-out names in the window are not recognised",
        "file": _HY,
        "old": """    if window.contains("RETICLE_ALLOW_MISSING_") || window.contains("RETICLE_SKIP_") {
        return true;
    }""",
        "new": """    if window.contains("NEVER-MATCHES") {
        return true;
    }""",
        "test": "literal_opt_out_env_var_in_window_is_not_reported",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H5 file-count floor raised above the real tree",
        "file": _HY,
        "old": "const MIN_SCANNED_FILES: usize = 50;",
        "new": "const MIN_SCANNED_FILES: usize = 5000;",
        "test": "no_ungated_silent_skip_in_test_sources",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H6 DELETION scan_source reports nothing",
        "file": _HY,
        "old": """    let mut violations = Vec::new();
    let lines: Vec<&str> = source.lines().collect();""",
        "new": """    let violations = Vec::new();
    if !violations.is_empty() || name.is_empty() || !source.is_empty() {
        return violations;
    }
    let mut violations = Vec::new();
    let lines: Vec<&str> = source.lines().collect();""",
        "test": "scan_source_tests",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H7 DELETION #[ignore] no longer exempts a test",
        "file": _HY,
        "old": """            if prev.contains("ignore") {
                return true;
            }""",
        "new": """            if false {
                return true;
            }""",
        "test": "test_with_ignore_attribute_is_not_reported",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H8 DELETION a const holding the opt-out name is not recognised",
        "file": _HY,
        "old": """    const_map.iter().any(|(name, value)| {
        (value.contains("RETICLE_ALLOW_MISSING_") || value.contains("RETICLE_SKIP_"))
            && contains_identifier(window, name)
    })""",
        "new": """    let _ = const_map;
    false""",
        "test": "const_shape_opt_out_far_above_is_not_reported",
        "test_target": "test_source_hygiene_tests",
    },
    {
        "label": "H9 println! lines are no longer scanned (eprintln! still is)",
        "file": _HY,
        "old": 'if !line.contains("eprintln!") && !line.contains("println!") {',
        "new": 'if !line.contains("eprintln!") {',
        "test": "capitalised_skipping_and_println_variant_are_reported",
        "test_target": "test_source_hygiene_tests",
    },
    # ---- S: shaping.rs --------------------------------------------------------
    {
        "label": "S1 DECLARED SURVIVOR fix1 test reads ~/Library/Fonts and returns when absent",
        "file": "crates/frontend-gui/src/shaping.rs",
        "old": "        let bytes = crate::JETBRAINS_MONO_REGULAR.to_vec();",
        "new": """        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = std::path::PathBuf::from(home).join("Library/Fonts/JetBrainsMono-Regular.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };""",
        "test": "fix1_deferred_normalization_survives_mid_frame_atlas_growth",
        "package": "frontend-gui",
        "test_target": "lib",
        "expect": "survived",
    },
]
