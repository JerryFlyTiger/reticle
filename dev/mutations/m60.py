# M60 -- TUI screen ownership (background output no longer bypasses the grid
# to write directly to the terminal). core side.
#
# The elisp side (`bglog`'s own cap) is a separate file, see
# `dev/mutations/m60-elisp.py` -- mutate.py's PACKAGE/TEST_TARGET apply to the
# whole config, so a cross-crate list has to be split into two runs.
#
# M1..M4 were designed by the reviewer from a cold read of the diff, executed
# by the main conversation (implementers don't verify their own fix).
#
# M5/M6 were **designed by the main conversation itself**: they target code
# that was written after the fix-up round (the truncation logic for
# `MAX_BACKGROUND_OUTPUT_LINES`), which didn't exist yet when the reviewer
# read the diff. This is the "one round limit" from step 7 of the milestone
# loop, recorded as required by the rules, not pretended to have been
# cold-read.
#
# How to run:
#     dev/mutate.py --config dev/mutations/m60.py
#
# Items that are black-box unobservable, honestly listed but **left out of
# the list**:
#
# 1. The two `eprintln!` -> `bglog::push` sites in
#    `crates/core/src/highlight.rs`: making tree-sitter's grammar/query
#    initialization fail would require breaking the vendored query files,
#    which are audited as hand-maintained. There's no test seat for that;
#    reverting it wouldn't turn any test FAIL.
# 2. The three `Stdio::null()` calls for `browse-url` in
#    `crates/core/src/builtins/ui.rs`: no test invokes the system `open`
#    command. Also unobservable by any test.
# 3. The four `eprintln!` calls behind `JIT_DEBUG` gates in
#    `crates/elisp/src/jit.rs` are **the same class of defect but
#    deliberately not fixed in this milestone** (see the comment there), so
#    they're out of scope for mutation testing.

PACKAGE = "core"
TEST_TARGET = "background_output_tests"

MUTATIONS = [
    {
        "label": "M1 one-shot notify flag starts as true (user would never learn about background output)",
        "file": "crates/core/src/editor.rs",
        "old": "            background_output_notified: false,",
        "new": "            background_output_notified: true,",
        "test": "idle_tick_drains_into_background_output_buffer_and_notifies_once",
    },
    {
        "label": "M2 flag is never set (every drain repeatedly floods the echo area)",
        "file": "crates/core/src/lib.rs",
        "old": "        ed.borrow_mut().background_output_notified = true;\n",
        "new": "",
        "test": "idle_tick_drains_into_background_output_buffer_and_notifies_once",
    },
    {
        "label": "M3 remove the env-var override for the worker executable (tests can't reach the real worker subprocess)",
        "file": "crates/elisp/src/worker.rs",
        "old": """        let exe = match std::env::var_os("RETICLE_WORKER_EXE") {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_exe()?,
        };""",
        "new": "        let exe = std::env::current_exe()?;",
        "test": "worker_stderr_is_captured_not_inherited",
    },
    {
        "label": "M4 LSP subprocess stderr is no longer a pipe (changed to null; downstream expect blows up, FAIL shows as a panic)",
        "file": "crates/elisp/src/lsp.rs",
        "old": "            .stderr(Stdio::piped())",
        "new": "            .stderr(Stdio::null())",
        "test": "lsp_server_stderr_is_captured_not_inherited",
    },
    {
        "label": "M5 buffer cap is effectively meaningless (designed by the main conversation: fix-up round code, not cold-read)",
        "file": "crates/core/src/lib.rs",
        "old": "pub const MAX_BACKGROUND_OUTPUT_LINES: usize = 5000;",
        "new": "pub const MAX_BACKGROUND_OUTPUT_LINES: usize = 500000;",
        "test": "background_output_buffer_is_bounded_and_keeps_the_tail",
    },
    {
        "label": "M6 truncation point cuts one newline short (designed by the main conversation: every trim leaves one blank line, cap stabilizes at 5001)",
        "file": "crates/core/src/lib.rs",
        "old": "                        cut_at = Some(char_idx + 1);",
        "new": "                        cut_at = Some(char_idx);",
        "test": "background_output_buffer_is_bounded_and_keeps_the_tail",
    },
    # M7/M8 were designed by **the reviewer of the trailing re-review**. The
    # reason they exist: M5/M6 (designed by the main conversation) only cover
    # the "trims too little" direction, while that test's original three
    # assertions (upper bound `<=`, does not contain the oldest line,
    # contains the newest line) structurally only check the upper bound and
    # presence, so the entire "trims too much" direction was invisible --
    # the reviewer used these two entries to prove they would be **falsely
    # reported as passing**, so the second fix-up round changed the
    # assertions to exact values plus head/tail completeness. These two must
    # now FAIL, or the new assertions haven't actually closed the gap.
    {
        "label": "M7 trims 100 extra lines every time (designed by the trailing reviewer: old assertions would falsely report passing)",
        "file": "crates/core/src/lib.rs",
        "old": "            let excess = total_lines - MAX_BACKGROUND_OUTPUT_LINES;",
        "new": "            let excess = total_lines - MAX_BACKGROUND_OUTPUT_LINES + 100;",
        "test": "background_output_buffer_is_bounded_and_keeps_the_tail",
    },
    {
        "label": "M8 truncation point bites one extra character (designed by the trailing reviewer: the surviving first line gets its head cut off, old assertions can't see it)",
        "file": "crates/core/src/lib.rs",
        "old": "                        cut_at = Some(char_idx + 1);",
        "new": "                        cut_at = Some(char_idx + 2);",
        "test": "background_output_buffer_is_bounded_and_keeps_the_tail",
    },
]
