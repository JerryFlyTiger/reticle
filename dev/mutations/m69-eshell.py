# M69's eshell_tests pass. The main list and header explanation are in
# dev/mutations/m69.py.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m69-eshell.py -p core \
#         --test-target eshell_tests
#
# All of `crates/core/lisp/*.el` is compiled into the binary via
# include_str!, so even though this entry is elisp, it still needs a
# recompile to take effect.

PACKAGE = "core"
TEST_TARGET = "eshell_tests"

MUTATIONS = [
    {
        "label": "M4 eshell--abbrev-dir reverted to raw string-prefix-p (reverts defect 4)",
        "file": "crates/core/lisp/eshell.el",
        "old": """    (cond
     ((string= dir home) "~")
     ((string-prefix-p (concat home "/") dir)
      (concat "~/" (substring dir (1+ (length home)))))
     (t dir))))""",
        "new": """    (if (string-prefix-p home dir)
        (concat "~" (substring dir (length home)))
      dir)))""",
        "expect_fail": ["eshell_abbrev_dir_sibling_of_home_not_mangled"],
        "note": "This is the elisp counterpart of the redisplay.rs bug, turned up incidentally during reconnaissance, not on any checklist entry.",
    },
]
