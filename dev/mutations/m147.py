# M147 -- the library-order contract, and the completion popup's
# unactionable duplicates.
#
# Designed from the cold reviewer's checklist (entries A1/A2/A3/B1 and the two
# declared survivors) plus three the main conversation added: N1 and C1 pin the
# SAME first-match-wins contract on the two paths the reviewer's list did not
# reach (`M-.' and AUTOINST), and D1 is the whole-effect deletion entry this
# project requires per shipped feature. Executed by the main conversation.
#
#     python3 dev/mutate.py --config dev/mutations/m147.py
#
# Why the two `"expect": "survived"` entries are here rather than left out:
# M56 left thirteen SURVIVED nav/complete mutations on the books, and the
# 2026-09-18 handoff note read them as a coverage gap. Two of the four loops
# that note named (`--any-module-name-matches-p' and `--module-found-p') are
# pure existence checks whose answer cannot depend on scan order at all, so a
# first->last mutation there is semantically a no-op and SURVIVED is the only
# possible result -- not a gap. Recording that as an executed fact is the point;
# leaving it out would be indistinguishable, to anyone reading this file later,
# from nobody having thought about it.
#
# A declared survivor is only worth recording if the mutated line actually RUNS
# with data the mutation actually changes, and the first version of this file
# failed both halves of that -- caught by the trailing cold read, not by the
# run, because "SURVIVED (as expected)" looks identical either way:
#
#   - SURV-1 was bound to `keyword_prefixed_typing_still_completes_a_matching_
#     module_name', whose three cases declare the module in the SAME buffer.
#     `--any-module-name-matches-p' is `(or <buffer scan> <library scan>)', so
#     the buffer arm returned t and the mutated library loop was never
#     evaluated at all.
#   - SURV-2 was bound to `empty_port_list_in_a_library_file_exercises_the_
#     module_found_p_library_branch', which does reach the loop but builds ONE
#     library directory; the buffer's own file is excluded from
#     `verilog-auto--library-files', so the list has a single element and
#     `reverse' on it is the identity. The mutation landed and could not
#     possibly change anything.
#
# Both are now bound to tests written for this purpose (M147 fix round), which
# keep the target module out of the buffer so the `or' cannot short-circuit,
# and set `verilog-library-directories' to two directories so the reversed list
# is genuinely a different list. What SURVIVED then shows, precisely: the
# mutated line is reached, over a two-element list in the opposite order, and
# the answer does not change. It still does not show more than that -- these
# loops return a boolean, so there is no order-dependent result for a test to
# catch even in principle.
#
# Cross-directory completion fixtures now: B2 (`library_entry_resolves_by_
# verilog_library_directories_order'), B3 (`all_modules_dedupes_same_named_
# library_files_first_occurrence_wins'), and the two below.

PACKAGE = "core"
TEST_TARGET = "verilog_complete_tests"

_C = "crates/core/lisp/verilog-complete.el"
_N = "crates/core/lisp/verilog-nav.el"
_A = "crates/core/lisp/verilog-auto.el"


MUTATIONS = [
    # ---- D: the whole effect, deleted ------------------------------------
    # The dedupe IS the milestone. With the `unless' gone, every occurrence is
    # pushed again and the popup is back to one item per library file.
    {
        "label": "D1 DELETION dedupe guard removed -- every occurrence flows through again",
        "file": _C,
        "old": "      (unless (gethash (car entry) seen)\n"
               "        (puthash (car entry) t seen)\n"
               "        (push entry result)))",
        "new": "      (puthash (car entry) t seen)\n"
               "      (push entry result))",
        "test": "all_modules_dedupes_same_named_library_files_first_occurrence_wins",
    },
    {
        "label": "D1b same deletion, seen from the buffer-vs-library side",
        "file": _C,
        "old": "      (unless (gethash (car entry) seen)\n"
               "        (puthash (car entry) t seen)\n"
               "        (push entry result)))",
        "new": "      (puthash (car entry) t seen)\n"
               "      (push entry result))",
        "test": "all_modules_dedupe_prefers_the_buffer_over_a_library_file",
    },
    # ---- A: which occurrence survives the dedupe -------------------------
    # Swapping the two `append' arguments makes the library files visited
    # first, so a "keep the last occurrence" implementation would pass B4 just
    # as happily as "keep the first" does. This is the entry that proves B4
    # discriminates between the two rather than matching by luck.
    {
        "label": "A1 traversal order swapped -- library visited before the buffer",
        "file": _C,
        "old": "    (dolist (entry (append\n"
               "                    (mapcar (lambda (m) (cons (verilog-auto--module-name m) nil))\n"
               "                            (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer)))\n"
               "                    (apply #'append\n"
               "                           (mapcar (lambda (path)\n"
               "                                     (let ((alist (verilog-complete--library-file-modules path)))\n"
               "                                       (mapcar (lambda (e) (cons (car e) (file-name-nondirectory path))) alist)))\n"
               "                                   (verilog-auto--library-files)))))",
        "new": "    (dolist (entry (append\n"
               "                    (apply #'append\n"
               "                           (mapcar (lambda (path)\n"
               "                                     (let ((alist (verilog-complete--library-file-modules path)))\n"
               "                                       (mapcar (lambda (e) (cons (car e) (file-name-nondirectory path))) alist)))\n"
               "                                   (verilog-auto--library-files)))\n"
               "                    (mapcar (lambda (m) (cons (verilog-auto--module-name m) nil))\n"
               "                            (verilog-auto--top-level-modules (verilog-auto--parse-current-buffer)))))",
        "test": "all_modules_dedupe_prefers_the_buffer_over_a_library_file",
    },
    # ---- B: first-match-wins, completion path ----------------------------
    # `--library-entry' is the order-sensitive completion site M147's B2 was
    # written for; the function itself is untouched by this milestone.
    {
        "label": "B1 library-entry scan order reversed -- last library file would win",
        "file": _C,
        "old": "  (let ((files (verilog-auto--library-files)) (found 'verilog-complete--miss))",
        "new": "  (let ((files (reverse (verilog-auto--library-files))) (found 'verilog-complete--miss))",
        "test": "library_entry_resolves_by_verilog_library_directories_order",
    },
    # ---- N: first-match-wins, `M-.' path ---------------------------------
    # Before M147 the test named below asserted only "landed in one of the
    # two", so this mutation SURVIVED by construction. It is the entry that
    # says the rename-the-contract half of B1 actually bought something.
    {
        "label": "N1 nav library scan reversed -- M-. would jump to the last match",
        "file": _N,
        "old": "  (let ((files (verilog-auto--library-files)) (found nil))",
        "new": "  (let ((files (reverse (verilog-auto--library-files))) (found nil))",
        "test": "duplicate_module_name_across_library_files_picks_the_first_one",
        "test_target": "verilog_nav_tests",
    },
    # ---- C: first-match-wins, AUTOINST path ------------------------------
    # The third consumer of the same contract. Included so that all three
    # order-sensitive sites are answered by one executed run rather than by
    # three separate readings of the code.
    {
        "label": "C1 AUTOINST library scan reversed -- last match would win",
        "file": _A,
        "old": "  (let ((files (verilog-auto--library-files)) (found nil) (found-path nil))",
        "new": "  (let ((files (reverse (verilog-auto--library-files))) (found nil) (found-path nil))",
        "test": "multiple_library_directories_first_directorys_whole_tree_wins",
        "test_target": "verilog_auto_tests",
    },
    # ---- R: the two survivors' tests really do reach those loops ---------
    # Two-part check. SURV-1/SURV-2 below say "the mutation landed and nothing
    # changed"; on their own that is also what you get when the test never
    # executes the line (which is exactly how the first version of this file
    # was wrong -- see the header). R1/R2 close the other half: they break the
    # loop's own answer rather than its order, so the bound test can only stay
    # green if it never looks at that loop. Both must FAIL.
    {
        "label": "R1 any-module-name-matches-p library branch never reports a hit",
        "file": _C,
        "old": "         (when (verilog-auto--filter (lambda (e) (string-prefix-p typed (car e))) alist)\n"
               "           (setq found t)))",
        "new": "         (when (verilog-auto--filter (lambda (e) (string-prefix-p typed (car e))) alist)\n"
               "           (setq found nil)))",
        "test": "any_module_name_matches_p_reaches_the_library_loop_with_two_library_files",
    },
    {
        "label": "R2 module-found-p library branch never reports a hit",
        "file": _C,
        "old": "            (when (and alist (assoc name alist))\n"
               "              (setq found t)))",
        "new": "            (when (and alist (assoc name alist))\n"
               "              (setq found nil)))",
        "test": "module_found_p_reaches_the_library_loop_with_two_library_files",
    },
    # ---- SURV: the two order-insensitive loops ---------------------------
    {
        "expect": "survived",
        "label": "SURV-1 any-module-name-matches-p scan reversed -- pure existence check, order cannot change the boolean",
        "file": _C,
        "old": "   (let ((files (verilog-auto--library-files)) (found nil))",
        "new": "   (let ((files (reverse (verilog-auto--library-files))) (found nil))",
        "test": "any_module_name_matches_p_reaches_the_library_loop_with_two_library_files",
    },
    {
        "expect": "survived",
        "label": "SURV-2 module-found-p scan reversed -- pure existence check, order cannot change the boolean",
        "file": _C,
        "old": "      (let ((files (verilog-auto--library-files)) found)",
        "new": "      (let ((files (reverse (verilog-auto--library-files))) found)",
        "test": "module_found_p_reaches_the_library_loop_with_two_library_files",
    },
]
