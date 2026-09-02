# M87 stage 1 -- the layout golden master's mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-golden.py \
#         -p core --test-target layout_golden_tests
#
# This list answers one question: **does the golden master actually
# bite?** The refactor it protects (five "how wide is this text"
# implementations collapsed into one, two wrap implementations collapsed
# into one) has now happened, and every entry below has been re-anchored
# to where the code moved to.
#
# That re-anchoring is itself the record of something worth keeping. On
# the first run after the refactor, all five original entries reported
# SKIP -- not SURVIVED. GM4 and GM5 were *expected* to, because they
# deliberately targeted the two wrap implementations separately and one
# of them ceases to exist once they merge; SKIP was the designed signal
# that the merge really happened. But GM1-GM3 skipped too, because
# `char_width`'s body moved out of `redisplay.rs` into
# `display_width.rs`. For that one run the golden master had **zero**
# mutation coverage, and a stale list would have gone on reporting a
# clean "5 SKIP" forever while defending nothing.
#
# The lesson, recorded here rather than in a commit message nobody
# re-reads: **a refactor invalidates its own mutation list, and SKIP is
# not a pass.** Re-anchor immediately, in the same milestone.
#
# All entries are replacement-style. Insertion-style mutations
# repeatedly miss their target on this project.

PACKAGE = "core"
TEST_TARGET = "layout_golden_tests"

MUTATIONS = [
    {
        "label": "GM1 tab stops move from 8 to 4",
        "file": "crates/core/src/redisplay/display_width.rs",
        "old": "            '\\t' => Expansion::Tab {\n                width: 8 - (col % 8),\n            },",
        "new": "            '\\t' => Expansion::Tab {\n                width: 4 - (col % 4),\n            },",
        "test": "layout_golden_master",
        "note": (
            "Shifts every column after a tab. The fixture's `tabs` scenario "
            "puts a tab exactly at a wrap boundary, so this changes both the "
            "glyph columns and where the line breaks. Re-anchored from "
            "redisplay.rs:146 to display_width.rs after the merge."
        ),
    },
    {
        "label": "GM2 wide characters stop being double-width",
        "file": "crates/core/src/redisplay/display_width.rs",
        "old": "pub fn wide_char_width(c: char) -> usize {\n    UnicodeWidthChar::width(c).unwrap_or(1).max(1)\n}",
        "new": "pub fn wide_char_width(c: char) -> usize {\n    let _ = c;\n    1\n}",
        "test": "layout_golden_master",
        "note": (
            "Kills CJK column accounting: nothing sets `continuation`, so the "
            "fixture's CONT_MARK cells vanish and everything after a wide "
            "char shifts left. Covered by the `cjk` scenario and the echo-area "
            "scroll-edge clip. `let _ = c;` keeps the unused-parameter warning "
            "from turning this into a compile error instead of a test failure."
        ),
    },
    {
        "label": "GM3 control characters render one column wide instead of two",
        "file": "crates/core/src/redisplay/display_width.rs",
        "old": "            Expansion::Control(_) => 2,",
        "new": "            Expansion::Control(_) => 1,",
        "test": "layout_golden_master",
        "note": (
            "The `control_chars` scenario places a control character where "
            "`col + w == cols` exactly -- the one value at which this "
            "one-cell difference changes where the line wraps. Positioned "
            "there deliberately: an earlier version of the scenario put "
            "control characters mid-line in a short row and this mutation "
            "SURVIVED."
        ),
    },
    {
        "label": "GM4 the merged wrap test stops reserving the continuation column",
        "file": "crates/core/src/redisplay.rs",
        "old": "fn wraps_before(col: usize, w: usize, cols: usize) -> bool {\n    col + w > cols.saturating_sub(1)\n}",
        "new": "fn wraps_before(col: usize, w: usize, cols: usize) -> bool {\n    col + w > cols\n}",
        "test": "layout_golden_master",
        "note": (
            "This single entry replaces the old GM4 and GM5, which targeted "
            "`next_row_start` and `render_window`'s draw loop separately "
            "because each had its own copy of this test. They now share one "
            "function, so one mutation covers both paths -- which is exactly "
            "the property the merge was for. The scanner path is only "
            "reachable through the fixture's scrolling scenario; without it "
            "this mutation SURVIVED."
        ),
    },
    {
        "label": "GM5 the wrap scanner's call site gets the wrong column bound",
        "file": "crates/core/src/redisplay.rs",
        "old": "        if wraps_before(col, w, cols) {\n            return Some(p);\n        }",
        "new": "        if wraps_before(col, w, cols + 1) {\n            return Some(p);\n        }",
        "test": "layout_golden_master",
        "note": (
            "Mutates one *call site* rather than the shared function, so "
            "unlike GM4 it isolates the lookahead scanner's path on its own. "
            "The scanner is only reachable through the fixture's scrolling "
            "scenario.\n"
            "\n"
            "The cold reviewer originally designed this entry as an argument "
            "swap -- `wraps_before(w, col, cols)` -- to catch a call site "
            "wired in the wrong order. It SURVIVED, and the reason is worth "
            "keeping: the function computes `col + w`, and **addition is "
            "commutative**, so swapping those two arguments is a no-op. That "
            "was a defective mutation, not a coverage gap, and the difference "
            "matters: reading it as a gap would have sent someone writing a "
            "test for behaviour that cannot vary. On SURVIVED, confirming the "
            "mutation landed is only half the check -- confirm it actually "
            "changed the semantics too."
        ),
    },
    {
        "label": "GM6 `Expansion::classify` stops dispatching to `Wide`",
        "file": "crates/core/src/redisplay/display_width.rs",
        "old": "                if wide_char_width(c) == 2 {\n                    Expansion::Wide(c)\n                } else {\n                    Expansion::Plain(c)\n                }",
        "new": "                Expansion::Plain(c)",
        "test": "layout_golden_master",
        "note": (
            "Also from the cold reviewer. Distinct from GM2, which mutates "
            "`wide_char_width` itself and therefore also changes `ml_width` "
            "and the `string-width` builtin. This one leaves the width "
            "function correct and breaks only `classify`'s dispatch, "
            "isolating whether the new enum is wired up rather than whether "
            "the underlying rule is right."
        ),
    },
]
