# Mutation list for M112 (chrome visibility across five themes).
# Run by the main conversation.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m112.py \
#         -p core --test-target gui_features_tests
#
# The milestone's claim is that every theme's chrome now clears a stated
# contrast band and that the bands are enforced rather than merely written
# down. Most of these entries came from the cold reviewer's checklist; the
# last one covers a test the main conversation added afterwards, for the one
# value the reviewer could not design a mutation for because nothing read it.

PACKAGE = "core"
TEST_TARGET = "gui_features_tests"

MUTATIONS = [
    {
        "label": "V1 dark's inactive mode line outshines its active one again",
        "file": "crates/core/lisp/themes.el",
        "old": '(set-face \'mode-line-inactive :foreground "#6b7280" :background "#25272d")',
        "new": '(set-face \'mode-line-inactive :foreground "#6b7280" :background "#282b30")',
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
    {
        "label": "V2 xcode's inactive mode line rises above the band's ceiling",
        "file": "crates/core/lisp/themes.el",
        "old": '(set-face \'mode-line-inactive :foreground "#6c7986" :background "#2f2f33")',
        "new": '(set-face \'mode-line-inactive :foreground "#6c7986" :background "#333338")',
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
    {
        "label": "V3 dracula's inactive mode line falls back to the editor background",
        "file": "crates/core/lisp/themes.el",
        "old": '(set-face \'mode-line-inactive :foreground "#6272a4" :background "#21222c")',
        "new": '(set-face \'mode-line-inactive :foreground "#6272a4" :background "#282a36")',
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
    {
        "label": "V4 light's selection reverts to the weak value that capped its chrome",
        "file": "crates/core/lisp/themes.el",
        "old": "(set-face 'region :background \"#a3c5ed\")",
        "new": "(set-face 'region :background \"#cfe0f5\")",
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
    {
        "label": "V5 light's indent guide reverts below the universal band",
        "file": "crates/core/lisp/themes.el",
        "old": "(set-face 'indent-guide :foreground \"#d7d8db\")",
        "new": "(set-face 'indent-guide :foreground \"#e2e6ee\")",
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
    {
        "label": "V6 an org colour is fat-fingered off its own theme's palette",
        "file": "crates/core/lisp/themes.el",
        "old": "(set-face 'org-level-1 :foreground \"#6f9df0\" :weight 'bold)",
        "new": "(set-face 'org-level-1 :foreground \"#6f9df1\" :weight 'bold)",
        "test": "org_face_colors_are_drawn_from_their_own_theme",
    },
    {
        "label": "V7 a selected row stops mirroring the selection",
        "file": "crates/core/lisp/themes.el",
        "old": "(set-face 'completions-selected :foreground \"#2c313a\" :background \"#a3c5ed\")",
        "new": "(set-face 'completions-selected :foreground \"#2c313a\" :background \"#c9dcf4\")",
        "test": "selected_row_backgrounds_mirror_the_selection",
    },
    {
        "label": "V8 the theme-body parser truncates at the argument list",
        "file": "crates/core/tests/gui_features_tests.rs",
        "old": "        if depth == 0 {",
        "new": "        if depth == 1 {",
        "test": "theme_chrome_faces_clear_the_visibility_bar",
    },
]
