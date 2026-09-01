# M71's eshell pass. The main list and header explanation are in
# dev/mutations/m71.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-eshell.py -p core \
#         --test-target eshell_tests

PACKAGE = "core"
TEST_TARGET = "eshell_tests"

MUTATIONS = [
    {
        "label": "M3 revert the clearing in eshell--insert-prompt",
        "file": "crates/core/lisp/eshell.el",
        "old": "  ;; would require a Rust-level change, out of scope here.\n"
               "  (set-buffer-modified-p nil))",
        "new": "  ;; *eshell* is not read-only -- so undo can re-dirty this buffer too.\n"
               "  ;; elisp has no primitive to suppress undo recording; fixing that\n"
               "  ;; would require a Rust-level change, out of scope here.\n"
               "  (when nil (set-buffer-modified-p nil)))",
        "expect_fail": [
            "a_freshly_opened_eshell_buffer_is_not_marked_modified",
            "eshell_stays_unmodified_after_a_command_and_new_prompt",
        ],
    },
    {
        "label": "M6 revert the clearing in eshell's incomplete branch (only added during the fix-up round)",
        "file": "crates/core/lisp/eshell.el",
        "old": "      ((eq parse 'incomplete)\n       (insert \"\\n\")\n       (set-buffer-modified-p nil))",
        "new": "      ((eq parse 'incomplete)\n       (insert \"\\n\"))",
        "expect_fail": ["eshell_stays_unmodified_after_incomplete_elisp_input"],
        "note": "These two lines didn't exist yet when the reviewer read the "
                "diff (they were added after it found the incomplete branch "
                "was missing), so its list couldn't possibly cover this entry.",
    },
    {
        "label": "M8 revert the clearing in the streaming-output branch (trailing re-review finding 2)",
        "file": "crates/core/lisp/eshell.el",
        "old": "                     ;; that list (M71 tail review, finding 2).\n"
               "                     (set-buffer-modified-p nil)))",
        "new": "                     ;; that list (M71 tail review, finding 2).\n"
               "                     ))",
        "expect_fail": ["eshell_stays_unmodified_while_a_command_is_still_streaming"],
        "note": "Found by the trailing re-review: **while** running an "
                "external command that produces output, *eshell* would keep "
                "showing `*`, and that's neither \"typing\" nor \"undo\", so "
                "it doesn't fall into either exception listed in "
                "insert-prompt's comment. The first version of the test "
                "added by the main conversation was fake -- `pump_until` "
                "looking for \"chunk\" matched immediately, because the "
                "user's own command line already contained that word, "
                "without ever waiting for the process's output. Fixed by "
                "counting to two instead.",
    },
]
