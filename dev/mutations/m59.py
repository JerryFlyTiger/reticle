# M59 -- textDocument/references (`lsp-references-at-point`, `M-?`).
# List designed by the reviewer from a cold read of the diff, executed by the
# main conversation (implementers don't verify their own fix).
#
# The 12 entries here correspond to the code as handed off in the reviewer's
# first round. The four blocks added by the fix-up round (verible command
# gate, `.vh`, skipping malformed Location entries, out-of-range line number)
# were designed separately during the trailing re-review and appended below
# in section F.
#
# One item from the reviewer's original list was deliberately not included:
# "make `lsp--reference-line-text` read the line text via `find-file`" would
# collide with M3 (removing the same-file cache) on the same assertion (the
# `test--fc-calls` counter); the failure reason couldn't be told apart, so one
# entry is enough.
#
# How to run:
#     dev/mutate.py --config dev/mutations/m59.py
#
# Items that are black-box unobservable, honestly listed but **left out of
# the list**:
#
# 1. The two known gaps noted at the top of the file (preview text reads from
#    disk vs. jumping lands in the live buffer; URI is not percent-decoded)
#    are both "missing logic" -- there's no line to revert, so no mutation
#    can be designed for them.
# 2. The **exact boundary** of `(>= line (length cached))` in
#    `lsp--reference-line-text` (where `line` equals the line count exactly)
#    cannot be proven by existing tests. When the trailing re-review designed
#    this entry, it worked out that it can't be hit: the out-of-range test
#    uses line=5 against an array of length 2, and both `>=` and `>` classify
#    that as out of range -- the off-by-one only shows up when
#    `line == length`, and the test set has no case for that. Honestly
#    recorded as a dead zone, not claimed as covered.
# 3. The two entries F8/F9 for fix A (all-malformed -> print a message
#    instead of opening an empty picker) were **designed by the main
#    conversation itself**, not by the reviewer -- that code was written
#    after the trailing re-review, and nobody has cold-read it (see the "one
#    round limit" in step 7 of the milestone loop). Recorded as required by
#    the rules.
#
# -- Section F: mutations for the trailing re-review (the fix-up round's four
#    blocks + a second fix-up round) --------------------------------------
# F1..F6 were designed by the trailing reviewer. F7 (the exact boundary of
# the out-of-range line number) is dead zone 2 above and is not listed.
# F8/F9 were designed by the main conversation, reasons given in item 3 above.

PACKAGE = "core"
TEST_TARGET = "lsp_references_tests"

MUTATIONS = [
    {
        "label": "M1 UTF-16 positioning degrades to the byte/char version lsp--pos-at",
        "file": "crates/core/lisp/lsp.el",
        "old": "  (goto-char (lsp--pos-at-utf16 line character)))",
        "new": "  (goto-char (lsp--pos-at line character)))",
        "test": "utf16_character_offset_lands_on_the_correct_column",
    },
    {
        "label": "M2 remove candidate sorting (server reply order is not guaranteed)",
        "file": "crates/core/lisp/lsp.el",
        "old": """    (sort entries
          (lambda (a b)
            (let* ((ea (cdr a)) (eb (cdr b))
                   (pa (nth 0 ea)) (pb (nth 0 eb))
                   (la (nth 1 ea)) (lb (nth 1 eb))
                   (ca (nth 2 ea)) (cb (nth 2 eb)))
              (cond ((not (string= pa pb)) (string< pa pb))
                    ((/= la lb) (< la lb))
                    (t (< ca cb))))))))""",
        "new": "    entries))",
        "test": "multiple_results_build_exact_sorted_candidate_list",
    },
    {
        "label": "M3 remove the same-file cache (hundreds of replies would re-read the same file)",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (let ((cached (gethash path cache 'lsp--reference-miss)))
    (when (eq cached 'lsp--reference-miss)
      (setq cached
            (condition-case nil
                (apply #'vector (split-string (file-contents-as-string path) "\\n"))
              (error 'lsp--unreadable)))
      (puthash path cached cache))""",
        "new": """  (let ((cached (condition-case nil
                    (apply #'vector (split-string (file-contents-as-string path) "\\n"))
                  (error 'lsp--unreadable))))""",
        "test": "same_file_multiple_hits_reads_the_file_only_once",
    },
    {
        "label": "M4 remove the \"exactly one result jumps directly\" shortcut (forces the picker open every time)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                 (if (= (length alist) 1)",
        "new": "                 (if nil",
        "test": "single_result_jumps_directly_and_m_dot_comma_returns",
    },
    {
        "label": "M5 do not send context.includeDeclaration",
        "file": "crates/core/lisp/lsp.el",
        "old": '        (puthash "context" (lsp--references-context) p)\n',
        "new": "",
        "test": "sends_references_request_with_uri_position_and_include_declaration",
    },
    {
        "label": "M6 empty-result message: with/without-filelist branches swapped",
        "file": "crates/core/lisp/lsp.el",
        "old": """          (if (file-exists-p (expand-file-name "verible.filelist" root))
              base
            (format "%s (no verible.filelist in %s; verible only answers for files listed there)"
                    base root)))""",
        "new": """          (if (file-exists-p (expand-file-name "verible.filelist" root))
              (format "%s (no verible.filelist in %s; verible only answers for files listed there)"
                      base root)
            base))""",
        "test": "empty_result_verilog_file_no_filelist_verible_command_names_verible_filelist_and_root",
    },
    {
        "label": "M7 remove the second staleness check (user switches buffers while the picker is open)",
        "file": "crates/core/lisp/lsp.el",
        "old": """                    (when (eq (current-buffer) buf)
                      (let ((entry (cdr (assoc name alist))))
                        (lsp-push-definition-marker origin)
                        (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry))))""",
        "new": """                    (let ((entry (cdr (assoc name alist))))
                      (lsp-push-definition-marker origin)
                      (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry)))""",
        "test": "stale_buffer_switch_while_picker_open_does_not_jump_on_pick",
    },
    {
        "label": "M8 remove the first staleness check (user has already switched away by the time the reply lands)",
        "file": "crates/core/lisp/lsp.el",
        # The context needs to extend to the line unique to references:
        # `(when (eq (current-buffer) buf)` appears 9 times in lsp.el and
        # `(if (not (and (vectorp result) ...` appears 3 times, both being
        # idioms shared with other LSP commands. The first version of the
        # list only wrote the first two lines, and the harness correctly
        # blocked application by reporting "original string appears 2 times"
        # -- that block was correct, not a false alarm.
        "old": """           (when (eq (current-buffer) buf)
             (if (not (and (vectorp result) (> (length result) 0)))
                 (message "%s" (lsp--references-empty-message file (lsp--client-command client)))""",
        "new": """           (progn
             (if (not (and (vectorp result) (> (length result) 0)))
                 (message "%s" (lsp--references-empty-message file (lsp--client-command client)))""",
        "test": "stale_buffer_switch_before_reply_lands_does_nothing",
    },
    {
        "label": "M9 the single-result branch does not push a marker (M-, can't get back)",
        "file": "crates/core/lisp/lsp.el",
        "old": """                       (lsp-push-definition-marker origin)
                       (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry)))""",
        "new": "                       (lsp--goto-reference (nth 0 entry) (nth 1 entry) (nth 2 entry)))",
        "test": "single_result_jumps_directly_and_m_dot_comma_returns",
    },
    {
        "label": "M10 candidate strings always use the absolute path (never converted to relative)",
        "file": "crates/core/lisp/lsp.el",
        "old": """  (let ((prefix (concat root "/")))
    (if (string-prefix-p prefix file)
        (substring file (length prefix))
      file)))""",
        "new": "  (ignore root)\n  file)",
        "test": "multiple_results_build_exact_sorted_candidate_list",
    },
    {
        "label": "M11 empty-reply check loosened to >=0 (an empty vector counts as having results)",
        "file": "crates/core/lisp/lsp.el",
        # Same as M8: the check itself appears 3 times in lsp.el, needs to be
        # matched together with the next line's references-specific message
        # to be unique.
        "old": """             (if (not (and (vectorp result) (> (length result) 0)))
                 (message "%s" (lsp--references-empty-message file (lsp--client-command client)))""",
        "new": """             (if (not (and (vectorp result) (>= (length result) 0)))
                 (message "%s" (lsp--references-empty-message file (lsp--client-command client)))""",
        "test": "empty_result_non_verilog_file_shows_plain_message_no_picker",
    },
    {
        "label": "M12 read failure/out-of-range line no longer degrades, returns the raw line text directly",
        "file": "crates/core/lisp/lsp.el",
        "old": """    (if (or (eq cached 'lsp--unreadable) (>= line (length cached)))
        nil
      (string-trim (aref cached line)))))""",
        "new": "    (string-trim (aref cached line))))",
        "test": "unreadable_target_degrades_to_no_snippet_without_aborting",
    },
    # -- Section F --------------------------------------------------------
    {
        "label": "F1 remove .vh (the extension added by the fix-up round)",
        "file": "crates/core/lisp/lsp.el",
        "old": """       (or (string-suffix-p ".v" file) (string-suffix-p ".vh" file)
           (string-suffix-p ".sv" file) (string-suffix-p ".svh" file))))""",
        "new": """       (or (string-suffix-p ".v" file)
           (string-suffix-p ".sv" file) (string-suffix-p ".svh" file))))""",
        "test": "empty_result_vh_file_no_filelist_verible_command_names_verible_filelist",
    },
    {
        "label": "F2 remove the verible command gate (falls back to extension-only, misleads slang users)",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (if (and (lsp--verilog-buffer-p file) (lsp--verible-command-p command))",
        "new": "    (if (lsp--verilog-buffer-p file)",
        "test": "empty_result_verilog_file_no_filelist_slang_command_shows_plain_message",
    },
    {
        "label": "F3 remove the short-circuit guard for a nil command (file-name-nondirectory would signal)",
        "file": "crates/core/lisp/lsp.el",
        "old": '  (and command (string-match-p "verible" (file-name-nondirectory command)) t))',
        "new": '  (string-match-p "verible" (file-name-nondirectory command)))',
        "test": "empty_result_verilog_file_no_filelist_nil_command_shows_plain_message",
    },
    {
        "label": "F4 caller does not pass through the real command (simulates a missed wire-up after a signature change)",
        "file": "crates/core/lisp/lsp.el",
        "old": '(message "%s" (lsp--references-empty-message file (lsp--client-command client)))',
        "new": '(message "%s" (lsp--references-empty-message file nil))',
        "test": "empty_result_verilog_file_no_filelist_verible_command_names_verible_filelist_and_root",
    },
    {
        "label": "F5 remove the hash-table-p guard on range (a malformed element aborts the whole batch)",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (when (and (stringp uri) (hash-table-p range))",
        "new": "      (when (stringp uri)",
        "test": "a_malformed_location_is_skipped_the_rest_of_the_reply_still_works",
    },
    {
        "label": "F6 do not drop nil entries (falls back to mixing malformed results into the candidates)",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (when entry (push entry entries)))",
        "new": "        (push entry entries))",
        "test": "a_malformed_location_is_skipped_the_rest_of_the_reply_still_works",
    },
    {
        "label": "F8 remove the \"all malformed\" branch (user gets stuck with an unusable empty picker)",
        "file": "crates/core/lisp/lsp.el",
        "old": """                  ((not alist)
                   (message "No usable references here (%d malformed entries in the reply)"
                            (length result)))
""",
        "new": "",
        "test": "all_malformed_reply_messages_instead_of_opening_an_empty_picker",
    },
    {
        "label": "F9 the \"all malformed\" branch loosened to <2 (swallows the single-result direct-jump path)",
        "file": "crates/core/lisp/lsp.el",
        "old": "                  ((not alist)\n",
        "new": "                  ((< (length alist) 2)\n",
        "test": "malformed_plus_one_valid_still_jumps_directly_no_picker",
    },
]
