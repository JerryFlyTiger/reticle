# M63 -- LSP auto-attach to new buffers.
#
# Source of this list: the reviewer designed 10 entries from a cold read of
# the diff (it only designs, doesn't execute). After the fix-up round, the
# main conversation added 4 more entries following the same principle --
# because the fix-up round's four new fix points (the scope of the hook's
# condition-case, backfill switching to `lsp--live-buffer-client`, and two
# new guards) didn't exist yet when the reviewer read the diff, so nobody had
# designed observation points for them. **M9/M11/M13/M14 were designed by the
# main conversation itself**, recorded honestly here.
#
# One entry from the reviewer's original list (removing backfill's "skip if
# already attached" guard) was **unobservable by any test at the time** -- it
# flagged this itself, that gap got a test added in the fix-up round, and is
# now M8. "The reviewer, while designing mutations, discovers its own list
# has an entry with no target" is itself the value of this step: reading
# code alone can't reveal which fixes are unguarded, only listing mutations
# makes it surface.
#
# How to run:
#     dev/mutate.py --config dev/mutations/m63.py
#
# TEST_TARGET left empty: the list spans three integration test binaries --
# lsp_auto_attach_tests / lsp_mode_tests / gui_features_tests -- and the
# shared TEST_TARGET can only point at one.
#
# ## A pitfall deliberately avoided when writing this list
#
# The "natural" way to write several of these mutations would be to replace
# a whole `let*`/`condition-case` block with a different shape, which
# changes the parenthesis count. **Once parentheses become unbalanced, the
# whole `lsp.el` fails to load, and of course the designated test goes red
# -- but not because the fix was reverted, because the file is broken.**
# That kind of mutation "FAILs as expected" while verifying nothing at all,
# and is the easiest false positive this harness produces. So every mutation
# in this file is deliberately written as a paren-balanced minimal change:
# deleting a whole cond clause, changing `(if X` to `(if nil`, or **inserting
# a complete extra form** (M12 uses this trick to reproduce "an unguarded
# call").
#
# ## Live run results: 14 entries, 13 FAILed as expected, 1 honestly recorded
#    as structurally unobservable
#
# First full run: 12 matched expectations, 2 survived (M11, M14) -- both
# were the two entries already labeled "may survive" ahead of time in this
# file, with no after-the-fact change of expectation. Afterward:
#
# * **M14 is a genuine gap**, and points at a potential bug: what
#   `lsp--auto-attach-backfill` passes to `lsp--auto-attach-client` is **the
#   caller's** mode, not the buffer's own mode, so the line
#   `(eq (major-mode-internal-get) mode)` was the only thing blocking "a
#   fundamental-mode buffer gets attached as if it were rust-mode", and no
#   test was guarding it. The main conversation added
#   `backfill_skips_buffers_whose_own_major_mode_has_no_server`, and after
#   re-running it FAILs as expected.
# * **M11 was judged structurally unobservable, no test forced into
#   existence for it**: `lsp--buffer-diagnostic-positions` has only two
#   callers (`next-diagnostic`/`previous-diagnostic`), **both of which
#   already blocked once via `lsp--live-buffer-client`**, so by the time
#   execution reaches inside the helper, "raw" and "live" must be equal --
#   the difference is structurally unreachable. It's defense in depth. The
#   trailing re-review independently traced both call sites and agrees with
#   this verdict, stating explicitly that "no counterexample was found".
#
# ## The harness got killed externally twice, leaving the source code in a
#    mutated state
#
# `finally` didn't run, and `git status` **couldn't show it** (these files
# were already modified anyway). The two kills stopped at M3 and M6
# respectively, each leaving one line of product code deleted/swapped. It
# was caught by taking this list and comparing "does each `old` string
# appear in the file exactly once" against the current state on disk --
# **this check is worth doing as standard practice before and after every
# harness run**; it's more effective than `git diff` because it asks "is the
# source code the shape I think it is" rather than "has anything changed".
# Reverting was always done via targeted `Edit` + `touch` to bump mtime
# (**never `git checkout --`**).
#
# One more new observation: `dev/mutate.py` **only prints the table after
# finishing**, so being killed mid-run doesn't just lose the current entry,
# it also throws away the results of all the entries already completed. On
# top of that, the first attempt also piped output into `tail` (a second
# buffering point; `python3 -u` only fixes buffering on the Python side),
# so a full 41-minute first attempt left behind 0 bytes. The correct
# approach is to run in batches **with `--test-target`**: leaving
# `TEST_TARGET` empty makes every single mutation relink all of core's test
# binaries, turning the per-entry cost from about 40 seconds into about 5
# minutes.
#
# ## Touched once more after the trailing re-review (this round wasn't
#    cold-read by anyone)
#
# The trailing re-review pointed out that `lsp--auto-attach-backfill`'s four
# guards get evaluated outside the `condition-case`, asymmetric with the
# just-fixed `lsp--maybe-auto-attach`. After fixing that, the indentation of
# the `old` strings for backfill's three entries (M8/M13/M14) all shifted;
# this file has been updated accordingly and re-run, and all three still
# FAIL as expected. **That fix itself currently has no mutation observation
# point** -- the trailing re-review checked all four guards one by one, and
# today no input can make them signal, so that's "the guard has a hole but
# the hole can't be reached", not "nobody is guarding it". No mutation was
# forced into existence for something that can't be triggered naturally.

PACKAGE = "core"
TEST_TARGET = ""

MUTATIONS = [
    # --- 10 entries designed by the reviewer ---
    {
        "label": "M1 attach failure does not roll back",
        "file": "crates/core/lisp/lsp.el",
        "old": """    (error
     (setq-local lsp--buffer-client nil)
     (setq-local lsp--last-synced-tick nil)
     (signal (car err) (cdr err)))))""",
        "new": """    (error
     (signal (car err) (cdr err)))))""",
        "test": "did_open_failure_rolls_back_buffer_client_and_synced_tick",
    },
    {
        "label": "M2 remove the /ssh: guard (order regresses)",
        "file": "crates/core/lisp/lsp.el",
        "old": """   ((string-prefix-p "/ssh:" file) nil)
""",
        "new": "",
        # Only run the counter-stub test: the other test asserting nil would,
        # under this mutation, spawn a real ssh process (ConnectTimeout=5 x 8
        # markers x several ancestor levels), dragging the harness out to
        # tens of seconds.
        "test": "ssh_guard_short_circuits_before_project_root_is_consulted",
    },
    {
        "label": "M3 remove the lsp--connections short-circuit guard",
        "file": "crates/core/lisp/lsp.el",
        "old": """   ((not lsp--connections) nil)
""",
        "new": "",
        "test": "no_connections_short_circuits_before_project_root_is_consulted",
    },
    {
        "label": "M4 (lsp)'s return value is no longer the message string",
        "file": "crates/core/lisp/lsp.el",
        # The regression described by the reviewer is "backfill becomes the
        # last form, and the return value becomes dolist's nil". Directly
        # moving that form would touch the parenthesis count (see file
        # header), so an equivalent, paren-balanced form is used instead:
        # make the last form return nil. What's observed is exactly the
        # same -- (lsp) no longer returns the sentence it printed.
        "old": """                    (message "LSP: connected to %s" command)))))""",
        "new": """                    (progn (message "LSP: connected to %s" command) nil)))))""",
        "test": "lsp_command_connects_reuses_and_did_opens_via_a_real_cat_subprocess",
    },
    {
        "label": "M5 does not register find-file-hook",
        "file": "crates/core/lisp/lsp.el",
        "old": """(add-hook 'find-file-hook 'lsp--maybe-auto-attach)""",
        "new": """;; (add-hook 'find-file-hook 'lsp--maybe-auto-attach)  ; mutated""",
        "test": "find_file_in_same_project_auto_attaches_the_new_buffer",
    },
    {
        "label": "M6 does not backfill after lsp succeeds",
        "file": "crates/core/lisp/lsp.el",
        "old": """                (lsp--auto-attach-backfill mode client)""",
        "new": """                nil""",
        "test": "lsp_backfills_buffers_already_open_before_it_was_run",
    },
    {
        "label": "M7 modeline indicator reverts to the global lsp--clients",
        "file": "crates/core/src/redisplay.rs",
        "old": """    if buffer_var_on(interp, editor, &buf, "lsp--buffer-client") {""",
        "new": """    if var_on(interp, "lsp--clients") {""",
        "test": "segmented_modeline_shows_name_position_and_lsp_state",
    },
    {
        "label": "M8 backfill does not skip an already-attached buffer",
        "file": "crates/core/lisp/lsp.el",
        # In the reviewer's original list, this entry was **unobservable by
        # any test at the time**, a gap it flagged itself; the fix-up round
        # added
        # backfill_does_not_redundantly_did_open_an_already_attached_buffer.
        "old": """                       (not (lsp--live-buffer-client))
                       (eq (lsp--auto-attach-client file mode) client))""",
        "new": """                   (eq (lsp--auto-attach-client file mode) client))""",
        "test": "backfill_does_not_redundantly_did_open_an_already_attached_buffer",
    },
    {
        "label": "M9 next-diagnostic does not distinguish no-connection from no-diagnostics",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (if (not (lsp--live-buffer-client))
      (message "No LSP server connected in this buffer (M-x lsp first)")
    (let ((positions (lsp--buffer-diagnostic-positions)))
      (if (not positions)
          (message "No diagnostics")
        (let ((here (point)) (next nil))""",
        "new": """  (if nil
      (message "No LSP server connected in this buffer (M-x lsp first)")
    (let ((positions (lsp--buffer-diagnostic-positions)))
      (if (not positions)
          (message "No diagnostics")
        (let ((here (point)) (next nil))""",
        "test": "diagnostic_navigation_reports_no_diagnostics_when_none_published",
    },
    {
        "label": "M10 lsp-auto-attach switch stops working",
        "file": "crates/core/lisp/lsp.el",
        "old": """   ((not lsp-auto-attach) nil)
""",
        "new": "",
        "test": "lsp_auto_attach_nil",
    },
    # --- 4 entries added by the main conversation after the fix-up round
    #     (these fixes did not exist when the reviewer read the diff) ---
    {
        "label": "M11 diagnostics helper reverts to reading the raw buffer-client",
        "file": "crates/core/lisp/lsp.el",
        # Expected to possibly survive: no test constructs the state of "client
        # still present but dead" and then queries diagnostics. If it
        # survives, that's an honestly recorded gap; do not change the
        # expectation after the fact.
        "old": """  (let ((client (lsp--live-buffer-client))
        (file (buffer-file-name)))""",
        "new": """  (let ((client lsp--buffer-client)
        (file (buffer-file-name)))""",
        "test": "diagnostic",
    },
    {
        "label": "M12 the hook's check call moves back outside condition-case",
        "file": "crates/core/lisp/lsp.el",
        # Fix 1 from the fix-up round. Inserts a **complete extra form** that
        # runs lsp--auto-attach-client once outside the protected scope --
        # exactly the pre-fix failure shape (a malformed alist entry's
        # signal escapes the hook, run_hook_by_name echoes it), and it's
        # paren-balanced.
        "old": """  (condition-case nil
      (let* ((file (buffer-file-name))
             (mode (major-mode-internal-get))
             (client (lsp--auto-attach-client file mode)))""",
        "new": """  (lsp--auto-attach-client (buffer-file-name) (major-mode-internal-get))
  (condition-case nil
      (let* ((file (buffer-file-name))
             (mode (major-mode-internal-get))
             (client (lsp--auto-attach-client file mode)))""",
        "test": "hook_swallows_a_malformed_server_alist_entry_without_signaling_or_attaching",
    },
    {
        "label": "M13 backfill's already-attached check reverts to not checking liveness",
        "file": "crates/core/lisp/lsp.el",
        # Fix 2 from the fix-up round: a stale dead client would make
        # backfill skip that buffer forever.
        "old": """                       (not (lsp--live-buffer-client))
                       (eq (lsp--auto-attach-client file mode) client))""",
        "new": """                       (not lsp--buffer-client)
                       (eq (lsp--auto-attach-client file mode) client))""",
        "test": "backfill_reattaches_a_buffer_whose_old_connection_died",
    },
    {
        "label": "M14 backfill does not compare the major mode",
        "file": "crates/core/lisp/lsp.el",
        # The expectation for this entry is **uncertain**. What
        # `lsp--auto-attach-client` receives is the caller's mode, not the
        # buffer's own mode, so removing this line means a fundamental-mode
        # buffer would get attached as if it were rust-mode. If no test goes
        # red, that's a genuine coverage gap.
        "old": """                       (eq (major-mode-internal-get) mode)
""",
        "new": "",
        "test": "backfill",
    },
]
