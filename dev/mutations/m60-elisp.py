# M60 -- elisp side: the `bglog` collector's own 500-line cap.
#
# The only reason this is split from `dev/mutations/m60.py` into a separate
# file: mutate.py's PACKAGE/TEST_TARGET apply to the whole config, and this
# entry needs to run `crates/elisp`'s lib tests (the `#[cfg(test)]` block
# inside `bglog.rs`), not core's integration tests. TEST_TARGET is left empty
# = no `--test` flag added, letting cargo run all of that crate's tests, with
# the `test` field used as a filter string.
#
# This entry was designed by the reviewer from a cold read of the diff,
# executed by the main conversation.
#
# How to run:
#     dev/mutate.py --config dev/mutations/m60-elisp.py

PACKAGE = "elisp"

MUTATIONS = [
    {
        "label": "M7 collector cap off-by-one (>= changed to >, allows growing to 501 before dropping)",
        "file": "crates/elisp/src/bglog.rs",
        "old": "    if st.lines.len() >= MAX_LINES {",
        "new": "    if st.lines.len() > MAX_LINES {",
        "test": "overflow_drops_oldest_and_reports_count",
    },
]
