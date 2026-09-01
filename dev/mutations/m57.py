# M57 -- User-facing explanation when LSP formatting capability is absent
# (capability gate + connect-time report). List designed by the reviewer from
# a cold read of the diff, executed by the main conversation (implementers
# don't verify their own fix).
#
# The gist of this one is the same shape as M56: at handoff, the reviewer
# already stated outright that **M4 / M7 / M9 were unobservable at the
# time** -- whether the region gate checks its own key, whether the two
# `lsp--gated-features-alist` entries point at the same key, and the note's
# string format, all of which left tests unaffected after tampering. The
# fix-up round added the corresponding tests; this file runs to check whether
# these three entries really flip to FAIL now that the tests are in place.
# **The entries that survive are the point**, not "every entry FAILs as
# expected".
#
# How to run (everything lives in the same test binary, so `test` fields are
# deliberately omitted and each entry runs the whole target -- following the
# lesson from M55/M56: when a test-name filter string matches 0 tests, cargo
# returns 0, and it gets recorded as SURVIVED, which is a false negative,
# worse than not running it at all):
#
#     dev/mutate.py --config dev/mutations/m57.py
#
# M8 can only be observed with `slang-server` on PATH (it's a PATH-gated
# e2e; when it's not on PATH, the test prints skip and returns immediately,
# and the mutation gets recorded as SURVIVED). Both servers are installed on
# this machine; if SURVIVED shows up on M8, check PATH before drawing a
# conclusion.
#
# Items that are black-box unobservable and **deliberately left out of the
# list**:
#
# 1. The two unfixed gaps noted in the M57 note at the top of `lsp.el`
#    (async requests have no timeout, `lsp--client-callbacks` has no expiry
#    cleanup): that's a description of "what this project didn't do", and
#    nothing in the test suite would turn red if that prose were wrong.
# 2. The two reasons given in the docstring of `lsp--gated-features-alist`
#    for "completionProvider deliberately not listed": pure design rationale,
#    mutation has no leverage point. Its correctness rests on the main
#    conversation having actually run a probe against verible (confirming the
#    key doesn't exist), not on a test.
# 3. All the comments in `demo/editor/init-example.el`: no test reads that
#    file at all.
#
# Padding the list with these three items just to hit a count would only
# produce permanent SURVIVED noise, which would make the list less trustworthy.

PACKAGE = "core"
TEST_TARGET = "lsp_format_tests"

MUTATIONS = [
    {
        "label": "M1 remove the capability gate from lsp-format-buffer (sends even when the key is absent)",
        "file": "crates/core/lisp/lsp.el",
        "old": """     ((not (lsp--capability-supported-p client "documentFormattingProvider"))
      (message "lsp: server does not advertise documentFormattingProvider -- formatting unavailable"))
""",
        "new": "",
    },
    {
        "label": "M2 remove the capability gate from lsp-format-region",
        "file": "crates/core/lisp/lsp.el",
        "old": """     ((not (lsp--capability-supported-p client "documentRangeFormattingProvider"))
      (message "lsp: server does not advertise documentRangeFormattingProvider -- region formatting unavailable"))
""",
        "new": "",
    },
    {
        "label": "M3 lsp-format-buffer checks the region key instead (message text unchanged)",
        "file": "crates/core/lisp/lsp.el",
        "old": '((not (lsp--capability-supported-p client "documentFormattingProvider"))\n      (message "lsp: server does not advertise documentFormattingProvider',
        "new": '((not (lsp--capability-supported-p client "documentRangeFormattingProvider"))\n      (message "lsp: server does not advertise documentFormattingProvider',
    },
    {
        # First entry the reviewer flagged as "undetectable by existing tests";
        # the fix-up round added
        # format_buffer_and_format_region_use_independent_capability_keys
        # and the region-side present-but-false test, and this entry verifies
        # that batch of fixes.
        "label": "M4 lsp-format-region checks the buffer key instead (originally black-box)",
        "file": "crates/core/lisp/lsp.el",
        "old": '((not (lsp--capability-supported-p client "documentRangeFormattingProvider"))\n      (message "lsp: server does not advertise documentRangeFormattingProvider',
        "new": '((not (lsp--capability-supported-p client "documentFormattingProvider"))\n      (message "lsp: server does not advertise documentRangeFormattingProvider',
    },
    {
        # Tested 2026-08-11, confirmed SURVIVED, and **this is correct**:
        # `crates/elisp/src/json.rs:7` decodes JSON `false` as `:false` and
        # `null` as `:null`, and both are truthy in elisp, so the capabilities
        # hash decoded from a real `initialize` response can **never** make an
        # existing key resolve to elisp `nil`. The difference between
        # `(gethash key caps)` and the original expression only shows up for
        # inputs unreachable over the wire -- a textbook **equivalent
        # mutation**, not a coverage gap. It's kept in the list and marked
        # expect=SURVIVED so the next person to re-run this doesn't have to
        # re-derive this conclusion. The mutation on the same guard that
        # actually hits real wire behavior is M5b.
        "label": "M5 capability predicate changed to check the *value* (equivalent mutation: unreachable over the wire)",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (not (eq (gethash key caps 'lsp--capability-absent) 'lsp--capability-absent)))))",
        "new": "      (gethash key caps))))",
        "expect": "SURVIVED",
    },
    {
        # The exact pitfall M46 hit: verible declares `hoverProvider: false`
        # while hover actually works, so "key present but value false" must
        # count as supported. This entry makes `:false` count as unsupported,
        # which is a way this could actually break over the wire.
        "label": "M5b :false reclassified as unsupported (breaks M46's asymmetric rule)",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (not (eq (gethash key caps 'lsp--capability-absent) 'lsp--capability-absent)))))",
        "new": "      (not (memq (gethash key caps 'lsp--capability-absent)\n                 '(lsp--capability-absent :false))))))",
    },
    {
        "label": "M6 unknown capabilities reclassified as unsupported (t to nil, regresses to before M46)",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (let ((caps (and (lsp--client-p client) (lsp--client-capabilities client))))
    (if (not (hash-table-p caps))
        t""",
        "new": """  (let ((caps (and (lsp--client-p client) (lsp--client-capabilities client))))
    (if (not (hash-table-p caps))
        nil""",
    },
    {
        # Second entry the reviewer flagged as "undetectable by existing
        # tests"; the fix-up round added two asymmetric note tests (missing
        # only one key), and this entry verifies that batch of fixes.
        "label": "M7 both gated-features entries point at the same key (originally black-box)",
        "file": "crates/core/lisp/lsp.el",
        "old": '    ("documentRangeFormattingProvider" . "lsp-format-region"))',
        "new": '    ("documentFormattingProvider" . "lsp-format-region"))',
    },
    {
        "label": "M8 M-x lsp connect message reverted to omit the note (requires slang-server on PATH)",
        "file": "crates/core/lisp/lsp.el",
        "old": """                (let ((note (lsp--unsupported-features-note client)))
                  (if note
                      (message "LSP: connected to %s (%s)" command note)
                    (message "LSP: connected to %s" command)))""",
        "new": '                (message "LSP: connected to %s" command)',
    },
    {
        # Third entry the reviewer flagged as "undetectable by existing
        # tests"; the fix-up round changed the note test from a contains
        # check to a full-string assert_eq, and this entry verifies that
        # change.
        "label": "M9 note's fixed prefix wording changed (originally black-box)",
        "file": "crates/core/lisp/lsp.el",
        "old": '      (format "unsupported: %s" (string-join (nreverse missing) ", ")))))',
        "new": '      (format "missing: %s" (string-join (nreverse missing) ", ")))))',
    },
]
