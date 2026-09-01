# M55 -- Verilog cross-file module definition jump. List designed by the
# reviewer from a cold read of the diff, executed by the main conversation
# (implementers don't verify their own fix).
#
# The goal is not "every entry FAILs as expected" -- **the entries that
# survive are the point**. At handoff, the reviewer already predicted M4/M5/M6
# would be SURVIVED, for these reasons respectively: no test checks
# `lsp--marker-stack` for the marker push in the same-buffer branch; the
# wiring in `modes.el` (the whole reason this milestone exists) has no
# end-to-end test; the START-inclusive end of `point-in-node-p` has no
# positive test. This file is run once first to confirm these three
# predictions, then a second time after the fix-up round adds the missing
# tests, expecting all three to flip to FAIL.
#
# How to run: **run the same list twice**, with `--test-target` set to
# verilog_nav_tests and lsp_mode_tests respectively; an entry only counts as
# SURVIVED if it fails to turn red in both runs.
#
#     dev/mutate.py --config dev/mutations/m55.py --test-target verilog_nav_tests
#     dev/mutate.py --config dev/mutations/m55.py --test-target lsp_mode_tests
#
# The first version left TEST_TARGET empty to run the whole `-p core`, on the
# theory that "M4/M5 ask whether **any** test is watching, and limiting to a
# single test file would bias the answer optimistic". In practice it didn't
# finish: every mutation touches a `.el` file that's pulled into lib.rs via
# `include_str!`, so after core rebuilds, `-p core` relinks **every** test
# binary; across 8 runs this went way over budget, and the harness was killed
# mid-application of M4 -- the working tree was left with a mutated
# verilog-nav.el (restored via backup + targeted Edit, with mtime bumped).
# Adding `--test <target>` only links one binary, which finishes in time.
# These two test files are the only places that could possibly cover this
# milestone's new code; the union of the two runs is enough to answer
# "is anything watching at all", at the cost that if some other test file
# touches these lines in the future, this list won't see it.
#
# Each entry deliberately **omits the `test` field**: each run exercises the
# whole target binary. If a test-name filter string were written in, running
# it against the other pass (the binary where that name doesn't exist) would
# match 0 tests, cargo would return 0, and it would be recorded as SURVIVED
# -- a false negative, worse than not running it at all.

# Items that are black-box unobservable and **deliberately left out of the
# list** (confirmed one by one during the trailing re-review):
#
# 1. The string content and two format parameters of the staleness message
#    (`verilog-nav.el`'s "Verilog module jump: %s declares `%s' ..."): the
#    test harness has no mechanism for intercepting `message`
#    (`lsp_mode_tests.rs`'s `test--captured` intercepts `lsp-request-async`),
#    so a typo or swapped parameters won't turn any test red. The same point
#    is also recorded in the comment in section 13 of `verilog_nav_tests.rs`.
# 2. The fix itself that makes the same-buffer branch reuse the decl node
#    (the trailing round removed a third reparse): external behavior (return
#    value, point landing, marker stack) is exactly equivalent, so reverting
#    it only makes things slower -- no behavioral test will go red.
# 3. `verilog-nav--goto-module-name-in-buffer` delegating to
#    `--goto-decl-name` to remove duplication: same as above, a pure
#    equivalent refactor.
# 4. The numeric correctness of the "Reparse cost" section in the file
#    header: pure comment, mutation has no leverage point here, only a cold
#    read can catch it -- and in fact that's exactly how the trailing
#    re-review caught it undercounting by one.
#
# Padding the list with these four items just to hit a count would only
# produce permanent SURVIVED noise, which would make the list less trustworthy.

PACKAGE = "core"
TEST_TARGET = None

MUTATIONS = [
    {
        "label": "M1 dispatch tier order: local tier moved after (not live)",
        "file": "crates/core/lisp/lsp.el",
        "old": """    ((and local-definition-function (funcall local-definition-function)))
    ((not live)
     (message "No LSP server connected in this buffer (M-x lsp first)"))""",
        "new": """    ((not live)
     (message "No LSP server connected in this buffer (M-x lsp first)"))
    ((and local-definition-function (funcall local-definition-function)))""",
    },
    {
        "label": "M2 type-name end boundary changed to exclusive",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "  (and node (>= pos (treesit-node-start node)) (<= pos (treesit-node-end node))))",
        "new": "  (and node (>= pos (treesit-node-start node)) (< pos (treesit-node-end node))))",
    },
    {
        "label": "M3 on-ident's or changed to and",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": """         (on-ident (or (verilog-complete--ident-char-p (char-before pos))
                       (verilog-complete--ident-char-p (char-after pos))))""",
        "new": """         (on-ident (and (verilog-complete--ident-char-p (char-before pos))
                       (verilog-complete--ident-char-p (char-after pos))))""",
    },
    {
        "label": "M4 remove marker push from the same-buffer branch (reviewer predicts SURVIVED)",
        "file": "crates/core/lisp/verilog-nav.el",
        # The fix-up round refactored the same-buffer branch to reuse the
        # already-found decl node (removing a third whole-file reparse), so
        # the original string changed accordingly; updated here to match.
        "old": """            (progn
              (lsp-push-definition-marker (lsp--make-definition-marker))
              (verilog-nav--goto-decl-name decl))""",
        "new": """            (progn
              (verilog-nav--goto-decl-name decl))""",
    },
    {
        "label": "M5 remove the wiring in modes.el (reviewer predicts SURVIVED)",
        "file": "crates/core/lisp/modes.el",
        "old": """            (setq-local local-completion-function 'verilog-complete-at-point)
            (setq-local local-definition-function 'verilog-goto-module-at-point)))""",
        "new": """            (setq-local local-completion-function 'verilog-complete-at-point)))""",
    },
    {
        "label": "M6 type-name start boundary changed to exclusive (reviewer predicts SURVIVED)",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "  (and node (>= pos (treesit-node-start node)) (<= pos (treesit-node-end node))))",
        "new": "  (and node (> pos (treesit-node-start node)) (<= pos (treesit-node-end node))))",
    },
    {
        "label": "M7 staleness fallback polarity inverted (control group)",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "                (unless (verilog-nav--goto-module-name-in-buffer type-name)",
        "new": "                (when (verilog-nav--goto-module-name-in-buffer type-name)",
    },
]
