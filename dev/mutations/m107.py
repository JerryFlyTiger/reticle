# Mutation list for M107 (three dark themes; Dracula becomes the default).
# Designed by the two review rounds, run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m107.py \
#         -p core --test-target gui_features_tests
#
# T1/T2 are the interesting pair: the OLD version of the face-set test
# hardcoded `theme--dark`/`theme--light` only, so a face missing from any of
# the three new themes would have passed silently. T2 targets the last theme
# in the file, which the old test could never have reached.

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "T1 a face goes missing from theme--dracula",
        "file": "crates/core/lisp/themes.el",
        "old": "  (set-face 'scroll-bar :foreground \"#323443\")",
        "new": "  (ignore)",
        "test": "every_theme_defines_the_same_set_of_faces",
    },
    {
        "label": "T2 a face goes missing from theme--vscode (the last theme in the file)",
        "file": "crates/core/lisp/themes.el",
        "old": "  (set-face 'scroll-bar :foreground \"#323232\")",
        "new": "  (ignore)",
        "test": "every_theme_defines_the_same_set_of_faces",
    },
    {
        "label": "T3 the error message stops being built from theme--names",
        "file": "crates/core/lisp/themes.el",
        "old": "     (t (error \"Unknown theme: %s (try %s)\" name",
        "new": "     (t (error \"Unknown theme: %s (try dark or light)\" name",
        "test": "load_theme_rejects_unknown_names_and_lists_all_five",
    },
]
