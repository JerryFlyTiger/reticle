# M87 stage 1 -- the half the golden master structurally cannot see.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-width.py \
#         -p core --test-target lib --only W1
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m87-width.py \
#         -p core --test-target core_tests --only W2
#
# Two runs, two targets, because the harness takes one package/target per
# invocation and these two entries are defended by different suites.
#
# Why this file exists at all: the refactor's claim is "nothing changed."
# The golden master proves that for *rendering* -- it captures the grid
# and nothing else. It never calls `string-width`, so it is structurally
# blind to the one place where the five unified width implementations
# genuinely disagreed with each other.
#
# `string-width` answers 3 for "a\tb" while the buffer grid draws 9
# columns for the same text. GNU Emacs's `string-width` *does* expand
# tabs, so unifying the five implementations naively -- routing
# `string-width` through `char_width` because they look like the same
# question -- would have been a silent, elisp-visible behaviour change
# smuggled into a refactor. W1 is the entry that proves someone would
# notice.
#
# Note: `string-width`, `frame-width` and `current-column` are all
# unbound in batch mode (`--eval` / `--script` do not load `ui.rs`'s
# registrations), so this cannot be checked from the command line. That
# is pre-existing behaviour, not something the refactor caused.
#
# All entries are replacement-style.

PACKAGE = "core"

MUTATIONS = [
    {
        "label": "W1 `string-width` starts expanding tabs (the plausible wrong 'fix')",
        "file": "crates/core/src/redisplay/display_width.rs",
        "old": "pub fn string_width_elisp(s: &str) -> usize {\n    s.chars().map(wide_char_width).sum()\n}",
        "new": "pub fn string_width_elisp(s: &str) -> usize {\n    let mut col = 0;\n    for c in s.chars() {\n        col += char_width(c, col);\n    }\n    col\n}",
        "test": "string_width_does_not_expand_tabs",
        "note": (
            "Run against `--test-target lib`. This is not a nonsense "
            "mutation -- it is what 'unify the five width implementations' "
            "looks like if you do it without noticing that the fifth one "
            "deliberately answers differently. It is arguably closer to GNU "
            "Emacs. It is still a behaviour change, and this entry is the "
            "reason it cannot be made by accident."
        ),
    },
    {
        "label": "W2 the `string-width` builtin is rewired to the buffer-drawing width",
        "file": "crates/core/src/builtins/ui.rs",
        "old": "        let w = crate::redisplay::display_width::string_width_elisp(&s);",
        "new": "        let mut w = 0;\n        for c in s.chars() {\n            w += crate::redisplay::display_width::char_width(c, w);\n        }",
        "test": "string_width_builtin_does_not_expand_tabs",
        "note": (
            "Run against `--test-target core_tests`. W1 defends the "
            "function; this entry defends the *wiring* from the builtin to "
            "it. On its first run, against `org_tests`, it SURVIVED -- "
            "`org.el` is the only real consumer (table alignment, "
            "org.el:234,276-277) and none of its tests put a tab inside a "
            "table cell, so nothing in the suite noticed the builtin "
            "changing its answer. `string_width_builtin_does_not_expand_tabs` "
            "was added to close that, and this entry is what proves it did."
        ),
    },
]
