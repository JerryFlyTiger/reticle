# M70's lib unit-test pass. The main list and header explanation are in
# dev/mutations/m70.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m70-lib.py -p core \
#         --test-target lib

PACKAGE = "core"
TEST_TARGET = "lib"

MUTATIONS = [
    {
        "label": "M6 the fits check gets an extra equals sign (sanity check for the int_plus_one rewrite)",
        "file": "crates/core/src/redisplay.rs",
        "old": "    if total < win {",
        "new": "    if total <= win {",
        "expect_fail": ["e7_echo_scroll_invariant_scan"],
        "note": "To satisfy clippy's int_plus_one, the implementer rewrote the "
                "spec's `total + 1 <= win` as `total < win`. This mutation "
                "verifies the boundary of that rewrite: one extra equals "
                "sign returns off=0 when total == win, and caret==total "
                "then lands outside the window.",
    },
]
