# M71's ielm pass. The main list and header explanation are in
# dev/mutations/m71.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m71-ielm.py -p core \
#         --test-target ielm_tests

PACKAGE = "core"
TEST_TARGET = "ielm_tests"

MUTATIONS = [
    {
        "label": "M4 revert the clearing in ielm--insert-prompt",
        "file": "crates/core/lisp/ielm.el",
        "old": "  ;; would require a Rust-level change, out of scope here.\n"
               "  (set-buffer-modified-p nil))",
        "new": "  ;; *ielm* is not read-only -- so undo can re-dirty this buffer too.\n"
               "  ;; elisp has no primitive to suppress undo recording; fixing that\n"
               "  ;; would require a Rust-level change, out of scope here.\n"
               "  (when nil (set-buffer-modified-p nil)))",
        "expect_fail": [
            "a_freshly_opened_ielm_buffer_is_not_marked_modified",
            "ielm_stays_unmodified_after_evaluating",
        ],
    },
    {
        "label": "M7 revert the clearing in ielm's incomplete branch (only added during the fix-up round)",
        "file": "crates/core/lisp/ielm.el",
        "old": "      ((eq parse 'incomplete)\n       (insert \"\\n\")\n       (set-buffer-modified-p nil))",
        "new": "      ((eq parse 'incomplete)\n       (insert \"\\n\"))",
        "expect_fail": ["ielm_stays_unmodified_after_incomplete_input"],
        "note": "The reviewer found by reading the code that ielm-return has "
                "four branches (its comment lists three), and the "
                "incomplete one doesn't call insert-prompt. This entry "
                "guards that finding.",
    },
]
