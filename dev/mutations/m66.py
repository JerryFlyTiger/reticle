# M66 -- `M-x lsp` on a remote (`/ssh:`) buffer must exit early, instead of
# spending 32 synchronous ssh calls before connecting to the wrong place.
#
# Source of this list: the reviewer designed 5 entries (MUT-1..MUT-5) from a
# cold read of the diff; this file adopts them as-is, plus M6 added by the
# main conversation -- the reviewer's list can prove "the guard exists", but
# none of its entries can prove "the guard runs **before**
# `lsp--project-root`", and that's exactly where this milestone's entire
# value lies.
#
# How to run (two passes, since mutate.py's `--test-target` applies to the
# whole config):
#     dev/mutate.py --config dev/mutations/m66.py -p core --test-target lsp_mode_tests
#     dev/mutate.py --config dev/mutations/m66-auto-attach.py -p core \
#         --test-target lsp_auto_attach_tests
#
# **Must be run as a background task** (lesson from the file header: hitting
# the 10-minute limit in the foreground gets SIGKILLed, `finally` doesn't
# run, and the source code is left in a mutated state). This list is all
# elisp and test-layer changes, but `crates/core/lisp/*.el` is compiled into
# the binary via `include_str!`, so every single entry still requires
# recompiling the whole thing.
#
# ## Why this list can be run safely
#
# The reviewer raised a concrete danger: after M1 removes the guard, the
# test falls through to the real `lsp--project-root`, issuing **real ssh**
# calls (`remote.rs`'s `ssh_bin()`, and the test never sets
# `RETICLE_SSH_BIN`) against every ancestor directory of
# `/ssh:buildhost:...` times 8 markers, and since `buildhost` doesn't exist,
# `ConnectTimeout=5` times dozens of calls turns into a network-dependent,
# long-running hang, not a quick assertion failure.
#
# Mitigation (a test change the main conversation made during wrap-up): T1
# and T3 got `(fset 'lsp--project-root (lambda (f) f))` added. **That stub
# isn't for what they assert** -- it's for hermeticity: if the guard breaks,
# it should go red on the assertion, not go red because it hung on the
# network. T2 already had this stub (it relies on it for its counting).
# All three tests therefore never make outbound connections, so this list
# can be run as a whole batch.
#
# ## Timing conditions of the observation points
#
# All five entries are **deterministic**: what's asserted is string
# equality, a counter's value, or whether `lsp--buffer-client` is nil, and
# none of them requires a "race" to hold (contrast M65's M6, whose
# observation point was load-dependent and would silently produce a false
# survival on a busy machine). So this batch doesn't need a widened time
# budget.
#
# ## A claim the reviewer explicitly stated "cannot be verified by mutation",
#    honestly recorded here
#
# The newly added v1-gap description at the top of the file claims that
# `lsp` and `lsp--auto-attach-client` are the **only two** paths to obtain a
# live `lsp--buffer-client`. That's an exhaustive negative proposition,
# established by searching every write site of `setq-local
# lsp--buffer-client` (independently re-verified by the reviewer), and
# **there's no single line that can be reverted to turn any test FAIL**. Not
# claimed to have mutation coverage.

PACKAGE = "core"
TEST_TARGET = "lsp_mode_tests"

MUTATIONS = [
    {
        "label": "M1 remove `lsp`'s remote guard (whole cond branch)",
        "file": "crates/core/lisp/lsp.el",
        "old": """     ((lsp--remote-path-p file)
      (message "LSP: remote (/ssh:) files are not supported"))
""",
        "new": "",
        # Guard gone -> falls through to the t branch -> calls the (stubbed)
        # lsp--project-root, counter becomes 1. This is the one thing this
        # milestone is actually guarding against: an ssh storm.
        "test": "lsp_command_on_a_remote_buffer_never_calls_project_root",
    },
    {
        "label": "M1b the same mutation, but observed against all three of T1/T2/T3",
        "file": "crates/core/lisp/lsp.el",
        "old": """     ((lsp--remote-path-p file)
      (message "LSP: remote (/ssh:) files are not supported"))
""",
        "new": "",
        # A question raised by the trailing re-review: the main conversation
        # added the `lsp--project-root` stub to T1/T3 (for hermeticity -- if
        # the guard breaks, it should go red on the assertion, not hang from
        # issuing real ssh calls against a nonexistent host). Could that stub
        # **instead cause T1/T3 to lose their target**? This entry answers
        # it: the filter string `a_remote` matches T1/T2/T3 at once, all
        # three should FAIL. If only T2 goes red, that means the stub really
        # did turn T1/T3 into targetless tests. M1 and this entry are the
        # same mutation, split into two only to observe them separately.
        "test": "a_remote",
    },
    {
        "label": "M2 prefix missing a colon (`/ssh` instead of `/ssh:`), becomes an overly broad match",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (and file (string-prefix-p "/ssh:" file)))""",
        "new": """  (and file (string-prefix-p "/ssh" file)))""",
        # In T5, "/sshfoo/a.sv" should be nil, becomes t after the mutation.
        "test": "remote_path_p_unit",
    },
    {
        "label": "M3 remove the nil guard (the file in `(and file ...)`)",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (and file (string-prefix-p "/ssh:" file)))""",
        "new": """  (string-prefix-p "/ssh:" file))""",
        # nil passed into string-prefix-p -> wrong-type-argument. T5's nil case
        # gets "ERROR: ..." instead of "nil".
        "test": "remote_path_p_unit",
    },
    {
        "label": "M5 message text changed by one word",
        "file": "crates/core/lisp/lsp.el",
        "old": """      (message "LSP: remote (/ssh:) files are not supported"))""",
        "new": """      (message "LSP: remote (/ssh:) file is not supported"))""",
        "test": "lsp_command_reports_and_declines_a_remote_ssh_buffer",
    },
    {
        "label": "M6 guard still present, but checked **after** lsp--project-root",
        "file": "crates/core/lisp/lsp.el",
        "old": """     ((lsp--remote-path-p file)
      (message "LSP: remote (/ssh:) files are not supported"))
     (t
      (let* ((command (car entry))
             (args (cdr entry))
             (root (lsp--project-root file)))""",
        "new": """     (t
      (let* ((command (car entry))
             (args (cdr entry))
             (root (lsp--project-root file)))
        (when (lsp--remote-path-p file)
          (message "LSP: remote (/ssh:) files are not supported"))""",
        # Entry added by the main conversation. The reviewer's five entries can
        # only prove "the guard exists": M1 removes it, M5 changes its
        # wording, M2/M3 change the predicate itself. But what this milestone
        # fixes is **not** "is there a message", it's "does it pay for 32
        # synchronous ssh calls before the message". This entry keeps the
        # guard but moves it to run after `lsp--project-root` -- the message
        # is word-for-word identical, T1/T5 stay green, and only T2's counter
        # goes from 0 to 1. It's the only entry in the list that can
        # distinguish "the guard exists" from "the guard runs early enough".
        #
        # Note that the code after this mutation is deliberately broken (it
        # keeps running the connection flow after the `when`), so T1 may also
        # go red alongside it -- that doesn't affect the conclusion, the
        # designated observation point is T2.
        "test": "lsp_command_on_a_remote_buffer_never_calls_project_root",
    },
]
