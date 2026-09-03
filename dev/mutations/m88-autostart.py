# M88 (LSP autostart on first open) mutation list -- the `core` integration half.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m88-autostart.py \
#         -p core --test-target lsp_autostart_tests
#
# The mode-line label half lives in dev/mutations/m88-autostart-lib.py and runs
# against `--test-target lib`, because the harness takes one target per run.
#
# All entries are replacement-style except A8, which cannot be: the invariant it
# guards (`lsp--buffer-client` stays nil until the completion callback) is
# enforced by the ABSENCE of a line, so the only way to break it is to add one.
# Insertion-style mutations have missed their target repeatedly on this project;
# this one is safe from that failure mode because it does not try to shadow a
# definition -- it adds two `setq-local`s inside a function body, which nothing
# later rebuilds.

PACKAGE = "core"
TEST_TARGET = "lsp_autostart_tests"

MUTATIONS = [
    {
        "label": "A1 the per-project dedup guard is gone (two buffers, two servers)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                         (not (assoc key lsp--autostart-pending))",
        "new": "                         t",
        "test": "two_buffers_in_one_project_produce_exactly_one_spawn",
    },
    {
        "label": "A2 the never-retry-after-give-up guard is gone",
        "file": "crates/core/lisp/lsp.el",
        "old": "                         (not (member key lsp--autostart-tried)))",
        "new": "                         t)",
        "test": "missing_binary_is_recorded_and_never_retried",
    },
    {
        "label": "A3 `lsp-autostart' is ignored (autostart cannot be turned off)",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (when (and lsp-autostart lsp-auto-attach)",
        "new": "  (when lsp-auto-attach",
        "test": "lsp_autostart_nil_never_spawns",
    },
    {
        "label": "A4 the `lsp-auto-attach' coupling is gone (spawn a server nobody talks to)",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (when (and lsp-autostart lsp-auto-attach)",
        "new": "  (when lsp-autostart",
        "test": "lsp_auto_attach_nil_never_spawns",
    },
    {
        "label": "A5 the frontend-started gate is gone (this is what keeps the test suite from spawning)",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (when lsp--frontend-started",
        "new": "  (when t",
        "test": "frontend_flag_never_set_never_spawns",
    },
    {
        "label": "A6 reaping no longer records the give-up (a deaf server is retried forever)",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (push key lsp--autostart-tried)",
        "new": "    (ignore key)",
        "test": "deaf_server_is_reaped_after_the_deadline_with_no_retry",
    },
    {
        "label": "A7 the deadline disjunct is gone (a live but deaf server is never reaped)",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (when (or (not (lsp-live-p conn)) (> (float-time) deadline))",
        "new": "        (when (not (lsp-live-p conn))",
        "test": "deaf_server_is_reaped_after_the_deadline_with_no_retry",
    },
    {
        "label": "A8 (F1) `M-x lsp' no longer cancels an in-flight autostart -- the leaked-server race returns",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (lsp--autostart-cancel-pending command root)",
        "new": "        (ignore command root)",
        "test": "m_x_lsp_cancels_an_in_flight_autostart_and_wins",
    },
    {
        "label": "A9 (F1) the completion callback stops deferring to an existing connection",
        "file": "crates/core/lisp/lsp.el",
        # First attempt aimed at the `(not entry)` branch instead. That is the
        # *other* half of the same `cond` -- picking the nearest line is not the
        # same as picking the line the behaviour depends on, and it reported
        # SURVIVED rather than a false pass.
        "old": "              ((lsp--get-connection command root)",
        "new": "              ((and nil (lsp--get-connection command root))",
        "test": "autostart_completion_defers_to_an_already_registered_connection",
    },
    {
        "label": "A10 (F1) the `own pending entry is gone' branch is disabled",
        "file": "crates/core/lisp/lsp.el",
        "old": "              ((not entry) (lsp-kill conn))",
        "new": "              ((and nil entry) (lsp-kill conn))",
        "test": "m_x_lsp_cancels_an_in_flight_autostart_and_wins",
        "note": (
            "Expected to SURVIVE, and listed for that reason rather than "
            "omitted: when this branch is disabled the next one -- `a "
            "connection for this key already exists' -- catches the same "
            "case and also kills the connection, because `M-x lsp' has by "
            "then registered its own. The two branches are deliberate "
            "belt-and-suspenders, so no test can distinguish them; recording "
            "that is honest, deleting the entry would hide that the line is "
            "unwatched."
        ),
    },
    {
        "label": "A11 (F9) `idle_tick' stops pumping replies before running the autostart tick",
        "file": "crates/core/src/lib.rs",
        "old": '    let _ = interp.eval_source("(lsp-process-pending-all)");',
        "new": '    let _ = interp.eval_source("(ignore)");',
        "test": "idle_tick_completes_a_near_deadline_handshake_before_reaping_it",
        "note": (
            "The design's ordering claim is that the pump runs first, so a "
            "handshake that just made its deadline is completed rather than "
            "reaped out from under itself. Swapping two statements is not a "
            "string replacement, so this removes the pump from `idle_tick` "
            "entirely -- a strictly stronger mutation that the same test "
            "must catch."
        ),
    },
    {
        "label": "A12 (G2) the marker loop walks only the current buffer",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (dolist (buf (buffer-list))\n      (condition-case nil\n          (with-current-buffer buf\n            (if marked",
        "new": "    (dolist (buf (list (current-buffer)))\n      (condition-case nil\n          (with-current-buffer buf\n            (if marked",
        "test": "pending_marker_is_set_and_cleared_on_the_non_current_buffer_too",
        "note": (
            "The reviewer's own design for this was to add `(eq buf "
            "(current-buffer))' to the clear condition -- but inside "
            "`with-current-buffer buf' that is trivially true, so it is a "
            "no-op, and biting would need a second edit capturing the outer "
            "buffer first. The runner applies one string per entry, so this "
            "single-string form goes at the loop instead: restrict it to "
            "the current buffer and both the set and the clear halves stop "
            "reaching any other buffer."
        ),
    },
    {
        "label": "A14 (G3) cancelling takes whichever autostart is first, not the matching one",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (let ((entry (assoc (cons command root) lsp--autostart-pending)))\n    (when entry\n      (setq lsp--autostart-pending (delq entry lsp--autostart-pending))",
        "new": "  (let ((entry (car lsp--autostart-pending)))\n    (when entry\n      (setq lsp--autostart-pending (delq entry lsp--autostart-pending))",
        "test": "cancelling_one_projects_autostart_leaves_a_different_projects_alone",
    },
]
