;;; themes.el --- built-in dark/light themes (M16)

;; VS Code-style semantic tokens expressed as faces. `default' is the
;; frame's ground colors — both frontends fill every unstyled cell from
;; it, so (load-theme 'light) genuinely relights the whole frame, TUI
;; included.
;;
;; M112: org's nine faces (org-level-1..4, org-todo, org-done, org-table,
;; org-date, org-link) used to be set by org.el's own `org--set-faces',
;; called only from `org-mode' with one hard-coded palette regardless of
;; the active theme -- this comment used to document that as
;; deliberate, but it wasn't: switching to `light' and opening an org
;; file left headings/links in colors chosen for a dark background.
;; They are now set per-theme below, right alongside every other face,
;; and `org--set-faces' is gone.
;;
;; M112 also raised several chrome faces (hl-line, indent-guide,
;; scroll-bar, mode-line, mode-line-inactive) that used to sit so close
;; to a theme's own background that a screenshot showed no visible
;; guide/highlight/scrollbar at all. The numbers in the per-face
;; comments below are WCAG relative-luminance contrast ratios (the
;; formula from https://www.w3.org/TR/WCAG21/#dfn-relative-luminance),
;; used here only as a consistent way to measure "how far a color sits
;; from the background" -- the target bands chosen (roughly 1.20-1.45)
;; are this project's own design targets for quiet chrome, NOT a claim
;; of WCAG conformance: WCAG's own threshold for non-text UI components
;; is 3:1, well above anything used here, because these faces are
;; deliberately subtle, not body text.

(defun theme--dark ()
  (set-face 'default :foreground "#c5cad3" :background "#191b20")
  (set-face 'region :background "#2c4463")
  ;; M112: hl-line was #20232a, ratio 1.095 against this theme's #191b20
  ;; -- within three units per channel of the background, invisible in a
  ;; screenshot. Raised to 1.288 (target band 1.20-1.35), still strictly
  ;; weaker than region's 1.734 so a highlighted current line never reads
  ;; as a selection.
  (set-face 'hl-line :background "#223042")
  ;; M49: textDocument/documentHighlight -- distinct from `region'/`hl-line'
  ;; so a highlighted symbol reads clearly without competing with an
  ;; active selection. M-visual-quality: mapped to the nearest listed
  ;; role, mode-line-inactive's background, since both are a subtle
  ;; "raised surface" tone a step off the default background.
  (set-face 'lsp-highlight :background "#1e2128")
  ;; M113 (show-paren-mode): DERIVED in every theme, not sourced from
  ;; any upstream palette -- `editorBracketMatch' is, in Dracula's own
  ;; words, "highly contested" and left unset there, so this project
  ;; picks one consistent recipe for all five themes instead of
  ;; matching a value that doesn't exist upstream for one of them: 20%
  ;; `diagnostic-warning' blended into `default''s background, giving a
  ;; warm amber tone that reads clearly apart from `region'/`hl-line'
  ;; (both blue-ish here) and from `lsp-highlight'.
  (set-face 'show-paren-match :background "#62523a")
  ;; M116 (show-trailing-whitespace): DERIVED in every theme, not
  ;; sourced from any upstream palette -- same reasoning as
  ;; `show-paren-match' just above (no upstream "trailing whitespace"
  ;; color exists to match), but blended from a 1:1 mix of this theme's
  ;; own `diagnostic-warning'/`diagnostic-error' into `default''s
  ;; background instead of `diagnostic-warning' alone, so the two
  ;; derived backgrounds read as related but distinguishable "something
  ;; needs attention here" tones rather than identical hues. Contrast
  ;; ratio against `default''s background: 2.303; against `region':
  ;; 1.328 (bar 1.30, same test `show-paren-match' already
  ;; clears); against `hl-line': 1.789 (same bar) -- covered by
  ;; `trailing_whitespace_separates_from_region_and_hl_line' in
  ;; `gui_features_tests.rs', since this face can sit on point's own
  ;; current line (wherever point is not at that line's end) at the
  ;; same time as `hl-line', and inside an active selection at the same
  ;; time as `region'.
  (set-face 'trailing-whitespace :background "#714c3f")
  ;; M-visual-quality fix 5: raised from #454b57 (contrast ratio 1.97
  ;; against the #191b20 background) to #646c7d (3.27) -- below the
  ;; readable threshold measured by the review.
  (set-face 'line-number :foreground "#646c7d")
  (set-face 'line-number-current-line :foreground "#98a1b0")
  (set-face 'mode-line :foreground "#c5cad3" :background "#262b34")
  ;; M112: mode-line-inactive background was #1e2128, ratio 1.069 against
  ;; this theme's #191b20 -- effectively invisible as a distinct surface.
  ;; First pass raised it to #282b30 (1.213), which review caught as
  ;; HIGHER than mode-line's own 1.212 -- backwards, an inactive surface
  ;; outshining the active one -- despite the comment claiming the
  ;; opposite (the two ratios were only 0.0007 apart, closer than this
  ;; project's own rounding in that first comment noticed). Re-picked at
  ;; #25272d, ratio 1.154 -- a real, visible margin (0.058) below
  ;; mode-line's 1.212, kept unchanged; the muted foreground below is
  ;; what still reads as "inactive" on top of that margin.
  (set-face 'mode-line-inactive :foreground "#6b7280" :background "#25272d")
  ;; M118 (`scope-header'/`scope-breadcrumb'): DERIVED, not sourced from
  ;; any upstream palette -- a 50/50 blend of this theme's own
  ;; `lsp-highlight' and `mode-line-inactive' backgrounds, landing
  ;; between the two "raised surface" tones this file already has. A
  ;; pinned header row must read as chrome sitting on top of ordinary
  ;; text (so it needs real separation from `default''s background) but
  ;; must not compete with the mode line for attention (it is a
  ;; permanent-while-scrolled fixture, not a one-off state indicator the
  ;; way the mode line's reverse-video treatment is) -- landing it
  ;; between two EXISTING quiet surfaces, rather than inventing a third
  ;; independent color family, is what keeps it "quieter than the mode
  ;; line" without also being invisible against `default'. Contrast
  ;; ratio against `default''s background: 1.111 (below the 1.20 M112
  ;; established as the floor for a chrome color meant to be clearly
  ;; seen on its own -- accepted here because a header row's TEXT is
  ;; also styled in this face, giving it far more surface area than a
  ;; thin line-number/scrollbar glyph, so the row reads as a distinct
  ;; band even at this ratio); against `mode-line': 1.091, comfortably
  ;; the weaker of the two. Foreground reuses `line-number-current-line'
  ;; (`#98a1b0'), an established "readable but muted" tone -- ratio
  ;; against this background: 5.954.
  (set-face 'scope-header :foreground "#98a1b0" :background "#22242a")
  ;; M-visual-quality: completions-popup/panel are surfaces, mapped to
  ;; the same mode-line-inactive-bg role as lsp-highlight above, with
  ;; `default' foreground for body text.
  (set-face 'completions-popup :foreground "#c5cad3" :background "#1e2128")
  ;; Selected rows: region's selection background, default foreground.
  (set-face 'completions-selected :foreground "#c5cad3" :background "#2c4463")
  (set-face 'panel :foreground "#c5cad3" :background "#1e2128")
  (set-face 'panel-selected :foreground "#c5cad3" :background "#2c4463")
  ;; panel-buffer-name/-mode/-file: nearest roles are function-name,
  ;; type, and line-number (dim/muted) respectively.
  (set-face 'panel-buffer-name :foreground "#74c6f5" :background "#1e2128")
  (set-face 'panel-buffer-mode :foreground "#4bc3d6" :background "#1e2128")
  (set-face 'panel-buffer-file :foreground "#454b57" :background "#1e2128")
  ;; panel-modified: nearest role is error, to keep drawing attention.
  (set-face 'panel-modified :foreground "#f06c6c" :background "#1e2128")
  (set-face 'dired-directory :foreground "#6f9df0" :weight 'bold)
  (set-face 'dired-symlink :foreground "#4bc3d6")
  (set-face 'dired-executable :foreground "#93c96f")
  (set-face 'dired-perms :foreground "#454b57")
  ;; dired-size: nearest role is constant, matching the original theme's
  ;; pairing (dired-size and font-lock-constant-face shared one hex).
  (set-face 'dired-size :foreground "#b18cf0")
  ;; dired-date: nearest role is number/literal (orange family, matches
  ;; the original hue relationship).
  (set-face 'dired-date :foreground "#e8a05c")
  (set-face 'font-lock-keyword-face :foreground "#6f9df0")
  (set-face 'font-lock-string-face :foreground "#93c96f")
  ;; M-visual-quality fix 5: raised from #616b7d (contrast ratio 3.21) to
  ;; #7b8697 (4.68) -- comments carry Verilog design intent, so they
  ;; need to clear a readable threshold.
  (set-face 'font-lock-comment-face :foreground "#7b8697" :slant 'italic)
  (set-face 'font-lock-function-name-face :foreground "#74c6f5")
  (set-face 'font-lock-type-face :foreground "#4bc3d6")
  (set-face 'font-lock-constant-face :foreground "#b18cf0")
  (set-face 'font-lock-preprocessor-face :foreground "#dfae63")
  (set-face 'font-lock-variable-name-face :foreground "#c5cad3")
  ;; font-lock-builtin-face: nearest role is operator/punctuation, the
  ;; remaining light-blue accent not already claimed by keyword/function.
  (set-face 'font-lock-builtin-face :foreground "#8ec9e0")
  ;; New faces (M-visual-quality).
  (set-face 'cursor :background "#6cb6ff" :foreground "#191b20")
  (set-face 'echo-area :foreground "#9aa3b0" :background "#191b20")
  (set-face 'diagnostic-error :foreground "#f06c6c")
  (set-face 'diagnostic-warning :foreground "#e5b567")
  (set-face 'diagnostic-info :foreground "#5aa9e6")
  ;; M112: indent-guide was #2b2f38, ratio 1.285 -- close to the target
  ;; band but this milestone standardizes all five themes on ~1.38 (the
  ;; ratio Dracula's own published `#FFFFFF1A` composite works out to).
  ;; Raised to 1.386.
  (set-face 'indent-guide :foreground "#25354b")
  ;; M116 (display-fill-column-indicator-mode): DERIVED in every theme
  ;; -- blended from `default''s own foreground into its background
  ;; (same recipe `scroll-bar' uses one theme-section up), rather than
  ;; from any accent hue, so the ruler reads as neutral chrome, not a
  ;; semantic color competing with `indent-guide' (blue-tinted in every
  ;; theme here) for the same visual role. Contrast ratio against
  ;; `default''s background: 1.849; against `indent-guide': 1.334
  ;; (bar 1.30, chosen so the two are distinguishable when a guide and
  ;; the ruler land on the same column -- this face is painted AFTER
  ;; indent guides in `frontend-gui/src/lib.rs', so on any such overlap
  ;; this face's pixels are the ones that actually show).
  (set-face 'fill-column-indicator :foreground "#44474d")
  ;; M112: scroll-bar was #3a404b, ratio 1.653 against this theme's
  ;; background -- already inside indent-guide/region bounds before this
  ;; change, but re-centered to 1.471 (strictly between indent-guide's
  ;; new 1.386 and region's 1.734) now that indent-guide moved.
  (set-face 'scroll-bar :foreground "#273952")
  ;; M37: rainbow-delimiters -- spread across the palette's accent
  ;; colours so nesting depth stays visually distinguishable. The base
  ;; six requested for this milestone (operator, preprocessor, constant,
  ;; function, string, number) cover depths 1-6; depths 7-9 extend with
  ;; three more palette accents (type, keyword, cursor) not otherwise
  ;; used by rainbow-delimiters, to keep all nine hues distinct. The
  ;; light theme below uses the same nine roles, just their light-theme
  ;; hex values -- see its own comment.
  (set-face 'rainbow-delimiters-depth-1-face :foreground "#8ec9e0")
  (set-face 'rainbow-delimiters-depth-2-face :foreground "#dfae63")
  (set-face 'rainbow-delimiters-depth-3-face :foreground "#b18cf0")
  (set-face 'rainbow-delimiters-depth-4-face :foreground "#74c6f5")
  (set-face 'rainbow-delimiters-depth-5-face :foreground "#93c96f")
  (set-face 'rainbow-delimiters-depth-6-face :foreground "#e8a05c")
  (set-face 'rainbow-delimiters-depth-7-face :foreground "#4bc3d6")
  (set-face 'rainbow-delimiters-depth-8-face :foreground "#6f9df0")
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#6cb6ff")
  ;; M112: org faces, moved here from org.el's `org--set-faces' (which
  ;; hard-coded one Atom One Dark palette regardless of the active
  ;; theme -- see the top-of-file comment for the history). Reuses this
  ;; theme's own font-lock roles: level-1/link->keyword, level-2->
  ;; constant, level-3/done->string, level-4->preprocessor, todo->the
  ;; diagnostic-error color, table->type, date->dired-date.
  (set-face 'org-level-1 :foreground "#6f9df0" :weight 'bold)
  (set-face 'org-level-2 :foreground "#b18cf0" :weight 'bold)
  (set-face 'org-level-3 :foreground "#93c96f")
  (set-face 'org-level-4 :foreground "#dfae63")
  (set-face 'org-todo :foreground "#f06c6c" :weight 'bold)
  (set-face 'org-done :foreground "#93c96f")
  (set-face 'org-table :foreground "#4bc3d6")
  (set-face 'org-date :foreground "#e8a05c")
  (set-face 'org-link :foreground "#6f9df0" :underline t))

(defun theme--light ()
  (set-face 'default :foreground "#2c313a" :background "#fbfbfd")
  ;; M112: this theme's region used to be #cfe0f5, ratio 1.300 against
  ;; #fbfbfd -- dramatically weaker than every other theme's region
  ;; (dark 1.734, dracula 1.556, xcode 2.407, vscode 1.807), which
  ;; capped indent-guide/scroll-bar below their usual bands (a first
  ;; pass left region alone and special-cased those two faces around
  ;; it; review lifted the "don't touch region" restriction once it was
  ;; established that, unlike every sourced value in the other four
  ;; themes, nothing here is copied from any upstream palette --
  ;; `dark'/`light' are this project's own design in full). Raised
  ;; along the same hue to #a3c5ed, ratio 1.727, in the normal range the
  ;; other four themes occupy; foreground-on-region contrast is now
  ;; 7.32:1 (#2c313a text still reads clearly as a selection).
  (set-face 'region :background "#a3c5ed")
  ;; M112: hl-line was #f0f2f7, ratio 1.084 against this theme's
  ;; #fbfbfd -- invisible. Raised to 1.242 (target band 1.20-1.35),
  ;; comfortably below region's now-1.727.
  (set-face 'hl-line :background "#e2e3e6")
  ;; M49: see the dark theme's own comment above.
  (set-face 'lsp-highlight :background "#f1f3f7")
  ;; M113: see the dark theme's own comment above -- same recipe (20%
  ;; `diagnostic-warning' blended into `default''s background).
  (set-face 'show-paren-match :background "#c8a471")
  ;; M116 (show-trailing-whitespace): DERIVED in every theme, not
  ;; sourced from any upstream palette -- same reasoning as
  ;; `show-paren-match' just above (no upstream "trailing whitespace"
  ;; color exists to match), but blended from a 1:1 mix of this theme's
  ;; own `diagnostic-warning'/`diagnostic-error' into `default''s
  ;; background instead of `diagnostic-warning' alone, so the two
  ;; derived backgrounds read as related but distinguishable "something
  ;; needs attention here" tones rather than identical hues. Contrast
  ;; ratio against `default''s background: 2.296; against `region':
  ;; 1.329 (bar 1.30, same test `show-paren-match' already
  ;; clears); against `hl-line': 1.849 (same bar) -- covered by
  ;; `trailing_whitespace_separates_from_region_and_hl_line' in
  ;; `gui_features_tests.rs', since this face can sit on point's own
  ;; current line (wherever point is not at that line's end) at the
  ;; same time as `hl-line', and inside an active selection at the same
  ;; time as `region'.
  (set-face 'trailing-whitespace :background "#d59b84")
  ;; M-visual-quality fix 5: raised from #b0b6c0 (contrast ratio 1.97
  ;; against the #fbfbfd background) to #878e9a (3.19).
  (set-face 'line-number :foreground "#878e9a")
  (set-face 'line-number-current-line :foreground "#5a6270")
  ;; M112: mode-line was #e8ebf1, ratio 1.156 against this theme's
  ;; #fbfbfd -- below the >=1.20 chrome bar. Raised to 1.253 (darker,
  ;; matching this theme's own "distinct means darker" direction).
  (set-face 'mode-line :foreground "#2c313a" :background "#e1e2e5")
  ;; M112: mode-line-inactive was #f1f3f7, ratio 1.075. Raised to 1.219,
  ;; still below mode-line's own 1.253 so the active/inactive hierarchy
  ;; survives.
  (set-face 'mode-line-inactive :foreground "#8a919c" :background "#e4e5e8")
  ;; M118: see the dark theme's own comment above -- same recipe (50/50
  ;; blend of `lsp-highlight'/`mode-line-inactive'). Contrast against
  ;; `default': 1.144; against `mode-line': 1.095. Foreground reuses
  ;; `line-number-current-line'; ratio 5.198.
  (set-face 'scope-header :foreground "#5a6270" :background "#eaecf0")
  (set-face 'completions-popup :foreground "#2c313a" :background "#f1f3f7")
  ;; M112: kept mirroring `region''s own background (see that face's
  ;; own comment above) so a selected completion row and an active
  ;; selection read as the same "chosen" affordance.
  (set-face 'completions-selected :foreground "#2c313a" :background "#a3c5ed")
  (set-face 'panel :foreground "#2c313a" :background "#f1f3f7")
  ;; M112: same mirroring as `completions-selected' above.
  (set-face 'panel-selected :foreground "#2c313a" :background "#a3c5ed")
  (set-face 'panel-buffer-name :foreground "#1a6fbd" :background "#f1f3f7")
  (set-face 'panel-buffer-mode :foreground "#0d7f92" :background "#f1f3f7")
  (set-face 'panel-buffer-file :foreground "#b0b6c0" :background "#f1f3f7")
  (set-face 'panel-modified :foreground "#c62828" :background "#f1f3f7")
  (set-face 'dired-directory :foreground "#2f5fd0" :weight 'bold)
  (set-face 'dired-symlink :foreground "#0d7f92")
  (set-face 'dired-executable :foreground "#2e7d3a")
  (set-face 'dired-perms :foreground "#b0b6c0")
  (set-face 'dired-size :foreground "#7a3fc4")
  (set-face 'dired-date :foreground "#b25c14")
  (set-face 'font-lock-keyword-face :foreground "#2f5fd0")
  (set-face 'font-lock-string-face :foreground "#2e7d3a")
  ;; M-visual-quality fix 5: raised from #7c8494 (contrast ratio 3.64) to
  ;; #6b7382 (4.62).
  (set-face 'font-lock-comment-face :foreground "#6b7382" :slant 'italic)
  (set-face 'font-lock-function-name-face :foreground "#1a6fbd")
  (set-face 'font-lock-type-face :foreground "#0d7f92")
  (set-face 'font-lock-constant-face :foreground "#7a3fc4")
  (set-face 'font-lock-preprocessor-face :foreground "#9a6a10")
  (set-face 'font-lock-variable-name-face :foreground "#2c313a")
  (set-face 'font-lock-builtin-face :foreground "#2a6a80")
  ;; New faces (M-visual-quality).
  (set-face 'cursor :background "#1a73c7" :foreground "#fbfbfd")
  (set-face 'echo-area :foreground "#5a6270" :background "#fbfbfd")
  (set-face 'diagnostic-error :foreground "#c62828")
  (set-face 'diagnostic-warning :foreground "#a16207")
  (set-face 'diagnostic-info :foreground "#1a6fbd")
  ;; M112: indent-guide was #e2e6ee, ratio 1.210 -- invisible, and a
  ;; first pass here was further capped at 1.289 by this theme's own
  ;; region, which was only 1.300 (see region's own comment above for
  ;; why that restriction is gone). Now that region is 1.727, indent-
  ;; guide targets the same ~1.38 band every other theme uses: raised
  ;; to 1.379.
  (set-face 'indent-guide :foreground "#d7d8db")
  ;; M116 (display-fill-column-indicator-mode): DERIVED in every theme
  ;; -- blended from `default''s own foreground into its background
  ;; (same recipe `scroll-bar' uses one theme-section up), rather than
  ;; from any accent hue, so the ruler reads as neutral chrome, not a
  ;; semantic color competing with `indent-guide' (blue-tinted in every
  ;; theme here) for the same visual role. Contrast ratio against
  ;; `default''s background: 1.797; against `indent-guide': 1.303
  ;; (bar 1.30, chosen so the two are distinguishable when a guide and
  ;; the ruler land on the same column -- this face is painted AFTER
  ;; indent guides in `frontend-gui/src/lib.rs', so on any such overlap
  ;; this face's pixels are the ones that actually show).
  (set-face 'fill-column-indicator :foreground "#bdbec2")
  ;; M112: scroll-bar was #c8cdd6, ratio 1.544. Raised to 1.519,
  ;; strictly between indent-guide's 1.379 and region's 1.727 --
  ;; previously this had to sit in a hair's-width gap between
  ;; indent-guide and a too-weak region; that gap is gone now that
  ;; region moved.
  (set-face 'scroll-bar :foreground "#b6d1f1")
  ;; M37: same nine roles as `theme--dark''s rainbow-delimiters set
  ;; (see its comment), using this theme's light-mode hex values for
  ;; those roles -- e.g. depth-1 is `operator/punctuation' in both
  ;; themes, just #2a6a80 here instead of #8ec9e0.
  (set-face 'rainbow-delimiters-depth-1-face :foreground "#2a6a80")
  (set-face 'rainbow-delimiters-depth-2-face :foreground "#9a6a10")
  (set-face 'rainbow-delimiters-depth-3-face :foreground "#7a3fc4")
  (set-face 'rainbow-delimiters-depth-4-face :foreground "#1a6fbd")
  (set-face 'rainbow-delimiters-depth-5-face :foreground "#2e7d3a")
  (set-face 'rainbow-delimiters-depth-6-face :foreground "#b25c14")
  (set-face 'rainbow-delimiters-depth-7-face :foreground "#0d7f92")
  (set-face 'rainbow-delimiters-depth-8-face :foreground "#2f5fd0")
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#1a73c7")
  ;; M112: org faces -- same role mapping as `theme--dark''s own comment
  ;; above, this theme's light-mode hex values.
  (set-face 'org-level-1 :foreground "#2f5fd0" :weight 'bold)
  (set-face 'org-level-2 :foreground "#7a3fc4" :weight 'bold)
  (set-face 'org-level-3 :foreground "#2e7d3a")
  (set-face 'org-level-4 :foreground "#9a6a10")
  (set-face 'org-todo :foreground "#c62828" :weight 'bold)
  (set-face 'org-done :foreground "#2e7d3a")
  (set-face 'org-table :foreground "#0d7f92")
  (set-face 'org-date :foreground "#b25c14")
  (set-face 'org-link :foreground "#2f5fd0" :underline t))

;; Dracula Official (github.com/dracula/dracula-theme). The 11 colors
;; below are copied verbatim from that project's README color-reference
;; table (Background/Current Line/Selection/Foreground/Comment plus the
;; six accents Cyan/Green/Orange/Pink/Purple/Red/Yellow -- Current Line
;; and Selection are the SAME hex upstream, hence 11 distinct values,
;; not 12). Every one of those 11 official hexes is used somewhere
;; below, UNCHANGED.
;;
;; Font-lock role mapping, M107 second fix round: SEVEN roles --
;; keyword->Pink, string->Yellow, comment->Comment, function->Green,
;; type/class->Cyan, constant->Purple, variable->Foreground -- were
;; checked one by one against the actual authority, the Dracula
;; author's own VS Code theme (github.com/dracula/visual-studio-code,
;; `src/dracula.yml`), and match it exactly. BUILTIN and PREPROCESSOR
;; below do NOT come from that source: the real theme has no single
;; universal color for either role -- it assigns them per-language
;; (e.g. `support.function.magic`->Purple, `support.function.any-
;; method.lua`->Green, `meta.preprocessor.haskell`->Comment, all
;; different colors for what this editor treats as one face each). So
;; builtin->Cyan and preprocessor->Pink below are THIS PROJECT'S OWN
;; CHOICE, not an upstream fact: builtin reuses Cyan because it already
;; carries `type', the nearest existing role; preprocessor reuses Pink
;; because it already carries `keyword', the nearest existing role.
;;
;; NOTE FOR THE NEXT PERSON WHO CHECKS THIS: the two community Emacs
;; ports of Dracula (`dracula-theme.el` on MELPA and doom-themes'
;; `doom-dracula-theme.el`) assign type and constant OPPOSITE to the
;; official VS Code theme used here (i.e. swapped Cyan/Purple). This
;; project deliberately follows the author-maintained official theme,
;; not either community port -- comparing against a port instead of
;; `dracula/visual-studio-code` will make this file look wrong when it
;; isn't.
;;
;; M112 SOURCED two more values from that same repo's `src/dracula.yml`
;; (verified directly, not trusted from an older comment): `mode-line'
;; below is `colors.statusBar.background: *BGDarker' (`&BGDarker:
;; "#191A21"'), used verbatim. `indent-guide' below is
;; `colors.editorIndentGuide.background: *NonText' (`&NonText:
;; "#FFFFFF1A"', white at 10% alpha) -- composited over this theme's
;; `#282a36' background that renders as `#3e404a', the hex actually
;; stored below, since this project's face model has no alpha channel
;; (the same simplification `theme--xcode''s own alpha note below
;; applies to its `default' foreground). M112 also checked
;; `colors.editor.lineHighlightBorder: *SELECTION' (`&SELECTION:
;; "#44475A"' -- the SAME hex as `region': Dracula publishes the
;; current line as a border in that color, not a fill; `hl-line''s own
;; comment below explains why this project uses a fill at a lower
;; strength instead of that value) and `colors.editorLineNumber.
;; foreground: *COMMENT' (`&COMMENT: "#6272A4"', already covered by
;; `line-number''s own comment below).
;;
;; Everything else -- the remaining surfaces (hl-line/lsp-highlight/
;; mode-line-inactive/panel backgrounds), scroll-bar, cursor, the three
;; diagnostic colors, and which of the 9 hues lands on which rainbow-
;; delimiters depth -- has no upstream spec (Dracula's README and
;; `src/dracula.yml' don't cover UI chrome or this editor's face set
;; beyond the four values named just above), so those are DERIVED:
;; most reuse one of the 11 official hexes directly (cursor,
;; diagnostics, rainbow depths), and the rest are new colors mixed from
;; the official palette (blends, since a blend of two in-palette colors
;; is not itself always in the palette) -- #2c2e3b/#2e303d (surfaces,
;; Background blended a small percentage toward Current Line/
;; Selection), #21222c/#363848/#404254 (M112: mode-line-inactive/
;; hl-line/scroll-bar, the same style of blend -- further toward
;; Current Line/Selection, or for mode-line-inactive toward the now-
;; sourced statusBar.background hex above), #6b7aa9 (line-number:
;; Comment blended 6% toward Foreground, see its own comment below),
;; #b4bccf (line-number-current-line: Comment blended 55% toward
;; Foreground), and #adb5cb (echo-area: the same blend at 50%). M107
;; fix round: fixed a prior version of this comment that claimed no
;; color here fell outside the official palette -- that was never true
;; of these blended surface/UI tones, which is expected and fine
;; (mixing two in-palette colors routinely produces an out-of-palette
;; one); the earlier wording was simply inaccurate, not a design
;; problem.
(defun theme--dracula ()
  (set-face 'default :foreground "#f8f8f2" :background "#282a36")
  (set-face 'region :background "#44475a")
  ;; M112: hl-line was #2a2c39, ratio 1.029 against this theme's
  ;; #282a36 -- within three units per channel of the background,
  ;; invisible in a screenshot. Raised to 1.232 (target band
  ;; 1.20-1.35), a blend of the background toward the official Current
  ;; Line/Selection hex (#44475A -- the same 11-color palette this
  ;; theme's header comment already documents as reused for `region').
  ;; Kept well below region's 1.556: Dracula publishes the current line
  ;; as a BORDER (`editor.lineHighlightBorder'), not a fill, with
  ;; `editor.lineHighlightBackground' deliberately unset -- this
  ;; editor's `Style'/`Grid' have no border concept, so a fill is
  ;; substituted, at a strength well under what the border color would
  ;; read as if used for a fill (a fill at 1.556 would be exactly as
  ;; loud as the selection). Do not "correct" this toward the upstream
  ;; border value; that would make hl-line and region indistinguishable.
  (set-face 'hl-line :background "#363848")
  (set-face 'lsp-highlight :background "#2e303d")
  ;; M113: `editorBracketMatch' is unset in Dracula's own upstream
  ;; palette -- their spec literally calls it "highly contested" and
  ;; leaves the choice to the editor. DERIVED like the other four
  ;; themes, not left unset, since this project always wants something
  ;; visible here.
  ;;
  ;; M113 review fix round: the other four themes' 20% `diagnostic-
  ;; warning' blend landed this theme's own value (#534641) at contrast
  ;; ratio 1.011 against `region''s #44475a -- the closest two faces
  ;; this project ships anywhere, against its own measuring stick
  ;; (contrast ratio, not hue), despite differing in hue (warm brown vs
  ;; blue-grey). Re-derived at 35% instead of 20% for this theme only:
  ;; #735c49, measured at 2.28:1 against `default''s background, 1.46:1
  ;; against `region', 1.85:1 against `hl-line'.
  ;;
  ;; **That round also asserted, without measuring, that "the other four
  ;; already clear a reasonable margin at 20%". They did not.** A second
  ;; cold read computed all five: `light''s was 1.051 against `hl-line'
  ;; -- as close as the Dracula value this comment was written to fix,
  ;; and worse in practice, because `hl-line-mode' is on by default so
  ;; point is nearly always on the highlighted line. `dark' was 1.127 and
  ;; `vscode' 1.249/1.196 against `region'/`hl-line'. Three themes were
  ;; re-derived at whatever blend clears **1.30 against both** (dark 36%,
  ;; light 57%, vscode 42%); `xcode' already cleared it at 20%.
  ;;
  ;; The blend fraction therefore varies per theme on purpose -- it is
  ;; whatever that theme's own warning hue needs to separate from that
  ;; theme's own selection and current-line colors. The 1.30 bar is
  ;; enforced by `show_paren_match_separates_from_region_and_hl_line' in
  ;; gui_features_tests.rs, so this is now an assertion rather than a
  ;; claim in prose -- which is exactly what the sentence it replaces
  ;; was, and why it was wrong.
  (set-face 'show-paren-match :background "#735c49")
  ;; M116 (show-trailing-whitespace): DERIVED in every theme, not
  ;; sourced from any upstream palette -- same reasoning as
  ;; `show-paren-match' just above (no upstream "trailing whitespace"
  ;; color exists to match), but blended from a 1:1 mix of this theme's
  ;; own `diagnostic-warning'/`diagnostic-error' into `default''s
  ;; background instead of `diagnostic-warning' alone, so the two
  ;; derived backgrounds read as related but distinguishable "something
  ;; needs attention here" tones rather than identical hues. Contrast
  ;; ratio against `default''s background: 2.100; against `region':
  ;; 1.349 (bar 1.30, same test `show-paren-match' already
  ;; clears); against `hl-line': 1.705 (same bar) -- covered by
  ;; `trailing_whitespace_separates_from_region_and_hl_line' in
  ;; `gui_features_tests.rs', since this face can sit on point's own
  ;; current line (wherever point is not at that line's end) at the
  ;; same time as `hl-line', and inside an active selection at the same
  ;; time as `region'.
  (set-face 'trailing-whitespace :background "#7e4f47")
  ;; M107 fix round: the official Comment color (#6272a4) alone is only
  ;; 3.03:1 against this theme's background -- below the >=3.2 bar this
  ;; project already applies to `dark' (3.27, see its own fix-5 comment
  ;; above) and to the other two new themes below. Blended 6% toward
  ;; Foreground -> #6b7aa9, 3.38:1, while staying in the same blue-grey
  ;; hue family as Comment.
  ;;
  ;; M112 left this unchanged even though that milestone's spec named
  ;; "use editorLineNumber.foreground (#6272A4) verbatim if we do not
  ;; already" -- we do not, on purpose, per the M107 comment immediately
  ;; above, and reverting to the raw 3.03:1 value would undo a
  ;; deliberate readability fix. Flagged rather than actioned.
  (set-face 'line-number :foreground "#6b7aa9")
  (set-face 'line-number-current-line :foreground "#b4bccf")
  ;; M112: mode-line was #303341, LIGHTER than this theme's own #282a36
  ;; background (ratio 1.136) -- the wrong side of a "distinct surface"
  ;; for a dark theme. `dracula/visual-studio-code''s `src/dracula.yml'
  ;; publishes `statusBar.background' as `#191A21', DARKER than the
  ;; editor -- used here verbatim (ratio 1.218 against the background,
  ;; foreground contrast 16.3:1 against #f8f8f2; see this defun's own
  ;; top-of-file comment above for the exact source line).
  (set-face 'mode-line :foreground "#f8f8f2" :background "#191a21")
  ;; M112: mode-line-inactive was #2c2e3b, ratio 1.059. No upstream
  ;; source covers this face; raised to 1.109 (still below mode-line's
  ;; 1.218, keeping the active/inactive hierarchy) by blending further
  ;; toward the same statusBar.background hex used above.
  (set-face 'mode-line-inactive :foreground "#6272a4" :background "#21222c")
  ;; M118: see the dark theme's own comment above -- same recipe (50/50
  ;; blend of `lsp-highlight'/`mode-line-inactive'). Contrast against
  ;; `default': 1.012; against `mode-line': 1.203. Foreground reuses
  ;; `line-number-current-line'; ratio 7.574.
  (set-face 'scope-header :foreground "#b4bccf" :background "#282934")
  (set-face 'completions-popup :foreground "#f8f8f2" :background "#2c2e3b")
  (set-face 'completions-selected :foreground "#f8f8f2" :background "#44475a")
  (set-face 'panel :foreground "#f8f8f2" :background "#2c2e3b")
  (set-face 'panel-selected :foreground "#f8f8f2" :background "#44475a")
  (set-face 'panel-buffer-name :foreground "#50fa7b" :background "#2c2e3b")
  (set-face 'panel-buffer-mode :foreground "#8be9fd" :background "#2c2e3b")
  (set-face 'panel-buffer-file :foreground "#6272a4" :background "#2c2e3b")
  (set-face 'panel-modified :foreground "#ff5555" :background "#2c2e3b")
  (set-face 'dired-directory :foreground "#ff79c6" :weight 'bold)
  (set-face 'dired-symlink :foreground "#8be9fd")
  (set-face 'dired-executable :foreground "#f1fa8c")
  (set-face 'dired-perms :foreground "#6272a4")
  (set-face 'dired-size :foreground "#bd93f9")
  (set-face 'dired-date :foreground "#ffb86c")
  (set-face 'font-lock-keyword-face :foreground "#ff79c6")
  (set-face 'font-lock-string-face :foreground "#f1fa8c")
  ;; M107: this theme's own comment color is only 3.03:1 against the
  ;; background, below the project's usual 4.68 comment-readability bar
  ;; (see the `dark' theme's own fix-5 comment above). Kept anyway,
  ;; unchanged from Dracula's official Comment hex: for "Dracula
  ;; Official" the point is upstream fidelity, and #6272a4 IS Dracula's
  ;; real Comment color, not an oversight. Anyone who wants a higher-
  ;; contrast comment color has `dark'/`xcode'/`vscode' to switch to --
  ;; this is a deliberate tradeoff, not a bug.
  (set-face 'font-lock-comment-face :foreground "#6272a4" :slant 'italic)
  (set-face 'font-lock-function-name-face :foreground "#50fa7b")
  (set-face 'font-lock-type-face :foreground "#8be9fd")
  (set-face 'font-lock-constant-face :foreground "#bd93f9")
  (set-face 'font-lock-preprocessor-face :foreground "#ff79c6")
  (set-face 'font-lock-variable-name-face :foreground "#f8f8f2")
  (set-face 'font-lock-builtin-face :foreground "#8be9fd")
  (set-face 'cursor :background "#8be9fd" :foreground "#282a36")
  (set-face 'echo-area :foreground "#adb5cb" :background "#282a36")
  (set-face 'diagnostic-error :foreground "#ff5555")
  (set-face 'diagnostic-warning :foreground "#ffb86c")
  (set-face 'diagnostic-info :foreground "#8be9fd")
  ;; M112: indent-guide was #2a2c38, ratio 1.028 -- invisible. Dracula
  ;; publishes its indent guide as `editorIndentGuide.background':
  ;; `#FFFFFF1A' (white at 10% alpha, verified directly in
  ;; `dracula/visual-studio-code''s `src/dracula.yml' -- see this
  ;; defun's own top-of-file comment above), which composited over this
  ;; theme's #282a36 background renders as #3e404a, ratio 1.381 -- used
  ;; here directly (this project's face model has no alpha channel, so
  ;; the pre-composited hex is what gets stored, same simplification
  ;; `theme--xcode''s own alpha note below applies to its `default'
  ;; foreground).
  (set-face 'indent-guide :foreground "#3e404a")
  ;; M116 (display-fill-column-indicator-mode): DERIVED in every theme
  ;; -- blended from `default''s own foreground into its background
  ;; (same recipe `scroll-bar' uses one theme-section up), rather than
  ;; from any accent hue, so the ruler reads as neutral chrome, not a
  ;; semantic color competing with `indent-guide' (blue-tinted in every
  ;; theme here) for the same visual role. Contrast ratio against
  ;; `default''s background: 1.808; against `indent-guide': 1.309
  ;; (bar 1.30, chosen so the two are distinguishable when a guide and
  ;; the ruler land on the same column -- this face is painted AFTER
  ;; indent guides in `frontend-gui/src/lib.rs', so on any such overlap
  ;; this face's pixels are the ones that actually show).
  (set-face 'fill-column-indicator :foreground "#50515a")
  ;; M112: scroll-bar was #323443, ratio 1.158. Raised to 1.441,
  ;; strictly between indent-guide's 1.381 and region's 1.556.
  (set-face 'scroll-bar :foreground "#404254")
  (set-face 'rainbow-delimiters-depth-1-face :foreground "#8be9fd")
  (set-face 'rainbow-delimiters-depth-2-face :foreground "#ffb86c")
  (set-face 'rainbow-delimiters-depth-3-face :foreground "#bd93f9")
  (set-face 'rainbow-delimiters-depth-4-face :foreground "#50fa7b")
  (set-face 'rainbow-delimiters-depth-5-face :foreground "#f1fa8c")
  (set-face 'rainbow-delimiters-depth-6-face :foreground "#ff5555")
  (set-face 'rainbow-delimiters-depth-7-face :foreground "#ff79c6")
  (set-face 'rainbow-delimiters-depth-8-face :foreground "#6272a4")
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#f8f8f2")
  ;; M112: org faces. Unlike the seven font-lock roles at the top of
  ;; this defun, org headings/todo/table/date/link have no upstream
  ;; Dracula spec at all (the README/VS Code theme cover syntax tokens
  ;; and UI chrome, not a document-outline mode) -- these are THIS
  ;; PROJECT'S OWN CHOICE, reusing six of the 11 official hexes:
  ;; level-1/link->Pink (keyword), level-2->Purple (constant),
  ;; level-3/done aren't the same role here as in the other four themes
  ;; -- level-3 reuses Yellow (string) and done reuses Green
  ;; (function-name) instead, because Dracula's role mapping (see the
  ;; big comment atop this defun) puts string on Yellow, not Green, so
  ;; "done" borrows Green directly for the usual success connotation.
  ;; level-4/date share Orange; table reuses Cyan (type).
  (set-face 'org-level-1 :foreground "#ff79c6" :weight 'bold)
  (set-face 'org-level-2 :foreground "#bd93f9" :weight 'bold)
  (set-face 'org-level-3 :foreground "#f1fa8c")
  (set-face 'org-level-4 :foreground "#ffb86c")
  (set-face 'org-todo :foreground "#ff5555" :weight 'bold)
  (set-face 'org-done :foreground "#50fa7b")
  (set-face 'org-table :foreground "#8be9fd")
  (set-face 'org-date :foreground "#ffb86c")
  (set-face 'org-link :foreground "#ff79c6" :underline t))

;; Xcode Default (Dark). Values copied verbatim from this machine's
;; /Applications/Xcode.app/.../Default (Dark).xccolortheme: background/
;; current-line/selection, plain text, cursor, comment, keyword,
;; string, number/character (same value upstream), preprocessor,
;; declaration.type, declaration.other, identifier.type/class (same
;; value upstream), identifier.function/variable/constant (same value
;; upstream -- Xcode really does give these three one shared color),
;; and the two diagnostic marker colors. `hl-line' below uses the
;; official Current Line hex (#23252b) verbatim -- M107 fix round 1:
;; an earlier version of this theme put a blended color there instead
;; and the comment wrongly claimed it was verbatim; caught by a cold
;; read, since #23252b appeared nowhere in the file.
;;
;; M107 fix round 2, honest disclosure: `default''s foreground below
;; is NOT actually a verbatim copy of the plain-text color. The real
;; `xcode.syntax.plain' entry in the .xccolortheme file is
;; "1 1 1 0.85" -- opaque white carries an 0.85 ALPHA, which composited
;; over the #1f1f24 background renders as roughly #dddee0, not pure
;; white. This project's face model (crates/core/src/redisplay.rs)
;; has no alpha channel, so #ffffff below is the RGB component taken
;; at face value and rendered fully opaque -- a reasonable
;; simplification, but it does mean this theme's actual on-screen text
;; will read slightly brighter than real Xcode's.
;;
;; Everything else not in that source file -- surfaces other than
;; hl-line (lsp-highlight/mode-line/panel backgrounds), line-number,
;; diagnostic-info (Xcode's file has no "info" marker color),
;; echo-area, indent-guide, scroll-bar, and the rainbow-delimiters
;; depth assignment -- is DERIVED here, either by blending the given
;; colors toward each other or reusing one of the given colors for a
;; second role.
(defun theme--xcode ()
  (set-face 'default :foreground "#ffffff" :background "#1f1f24")
  (set-face 'region :background "#515b70")
  ;; hl-line: the official Current Line color, verbatim -- see the
  ;; comment above this defun.
  ;;
  ;; M112 measured this at ratio 1.071 against this theme's #1f1f24
  ;; background -- inside the "invisible" band this milestone otherwise
  ;; fixes everywhere else, and it left this one alone ON PURPOSE: this
  ;; hex is disclosed above as taken verbatim from Xcode's own
  ;; .xccolortheme file, and M112's own instructions say not to override
  ;; a value the comments mark as upstream-sourced. Flagged, not fixed --
  ;; if this is judged wrong, the fix is to change the sourcing decision
  ;; explicitly, not to quietly recolor it here.
  ;;
  ;; VERIFIED, not just trusted: read directly off this machine's
  ;; installed Xcode, `/Applications/Xcode.app/Contents/
  ;; SharedFrameworks/SourceEditor.framework/Versions/A/Resources/
  ;; Default (Dark).xccolortheme', via `plutil -convert json'.
  ;; `DVTSourceTextBackground' is `0.120543 0.122844 0.141312 1' and
  ;; `DVTSourceTextCurrentLineHighlightColor' is `0.138526 0.146864
  ;; 0.169283 1' -- both ALPHA 1, i.e. not the same alpha-compositing
  ;; situation as `default''s foreground below (that one really does
  ;; carry an 0.85 alpha this project's alpha-less `Style' can't
  ;; represent). Multiplied out, background is #1f1f24 and current-line
  ;; is #23252b -- exactly the two hexes already in this file. So
  ;; #23252b is not a truncated/misread value; Xcode's own current-line
  ;; highlight genuinely renders at this low a contrast in the real
  ;; application. The theme here is faithful, not broken.
  (set-face 'hl-line :background "#23252b")
  ;; lsp-highlight: DERIVED (background blended 80% toward Current
  ;; Line), kept distinct from hl-line above so a highlighted symbol
  ;; still reads apart from the cursor's own line.
  (set-face 'lsp-highlight :background "#22242a")
  ;; M113: DERIVED (20% `diagnostic-warning' blended into `default''s
  ;; background) -- same recipe as every other theme; see the dark
  ;; theme's own comment above. Xcode's real bracket-match UI is a box
  ;; OUTLINE around the pair, not a fill (same fill-substitutes-for-
  ;; border situation as this theme's own `hl-line' above); this
  ;; project's `Style' has no border concept, so a fill stands in here
  ;; too.
  (set-face 'show-paren-match :background "#493d2f")
  ;; M116 (show-trailing-whitespace): DERIVED in every theme, not
  ;; sourced from any upstream palette -- same reasoning as
  ;; `show-paren-match' just above (no upstream "trailing whitespace"
  ;; color exists to match), but blended from a 1:1 mix of this theme's
  ;; own `diagnostic-warning'/`diagnostic-error' into `default''s
  ;; background instead of `diagnostic-warning' alone, so the two
  ;; derived backgrounds read as related but distinguishable "something
  ;; needs attention here" tones rather than identical hues. Contrast
  ;; ratio against `default''s background: 1.478; against `region':
  ;; 1.629 (bar 1.30, same test `show-paren-match' already
  ;; clears); against `hl-line': 1.380 (same bar) -- covered by
  ;; `trailing_whitespace_separates_from_region_and_hl_line' in
  ;; `gui_features_tests.rs', since this face can sit on point's own
  ;; current line (wherever point is not at that line's end) at the
  ;; same time as `hl-line', and inside an active selection at the same
  ;; time as `region'.
  (set-face 'trailing-whitespace :background "#50352f")
  ;; M107 fix round: the official Invisibles color (#424d5b) alone is
  ;; only 1.91:1 against this theme's background -- worse than the
  ;; 1.97 this project already rejected as unreadable for `dark' (see
  ;; its fix-5 comment above). Blended 20% toward the plain-text color
  ;; -> #68717c, 3.31:1, in the same blue-grey family as Invisibles.
  (set-face 'line-number :foreground "#68717c")
  (set-face 'line-number-current-line :foreground "#6c7986")
  ;; mode-line's #313540 is already ratio 1.339 against this theme's
  ;; #1f1f24 -- already above M112's >=1.20 chrome bar, left unchanged.
  (set-face 'mode-line :foreground "#ffffff" :background "#313540")
  ;; M112: mode-line-inactive was #212228, ratio 1.035. Raised to 1.231,
  ;; still below mode-line's own 1.339.
  (set-face 'mode-line-inactive :foreground "#6c7986" :background "#2f2f33")
  ;; M118: see the dark theme's own comment above -- same recipe (50/50
  ;; blend of `lsp-highlight'/`mode-line-inactive'). Contrast against
  ;; `default': 1.142; against `mode-line': 1.173.
  ;;
  ;; M118 review fix (FIX-7): foreground was `line-number-current-line'
  ;; (`#6c7986') verbatim, ratio 3.229 against this face's own
  ;; background -- markedly weaker than every other theme's
  ;; `scope-header' (5.2-7.6). Raised by blending 35% toward white
  ;; (`#9fa8b0'), ratio 5.957 -- in line with the other themes, still
  ;; the same blue-grey family as `line-number-current-line' so the
  ;; theme's own character is kept, not a new hue.
  (set-face 'scope-header :foreground "#9fa8b0" :background "#282a2e")
  (set-face 'completions-popup :foreground "#ffffff" :background "#212228")
  (set-face 'completions-selected :foreground "#ffffff" :background "#515b70")
  (set-face 'panel :foreground "#ffffff" :background "#212228")
  (set-face 'panel-selected :foreground "#ffffff" :background "#515b70")
  (set-face 'panel-buffer-name :foreground "#67b7a4" :background "#212228")
  (set-face 'panel-buffer-mode :foreground "#5dd8ff" :background "#212228")
  (set-face 'panel-buffer-file :foreground "#6c7986" :background "#212228")
  (set-face 'panel-modified :foreground "#f74a4a" :background "#212228")
  (set-face 'dired-directory :foreground "#fc5fa3" :weight 'bold)
  (set-face 'dired-symlink :foreground "#5dd8ff")
  (set-face 'dired-executable :foreground "#fc6a5d")
  (set-face 'dired-perms :foreground "#6c7986")
  (set-face 'dired-size :foreground "#67b7a4")
  (set-face 'dired-date :foreground "#d0bf69")
  (set-face 'font-lock-keyword-face :foreground "#fc5fa3")
  (set-face 'font-lock-string-face :foreground "#fc6a5d")
  (set-face 'font-lock-comment-face :foreground "#6c7986" :slant 'italic)
  (set-face 'font-lock-function-name-face :foreground "#67b7a4")
  (set-face 'font-lock-type-face :foreground "#5dd8ff")
  (set-face 'font-lock-constant-face :foreground "#67b7a4")
  (set-face 'font-lock-preprocessor-face :foreground "#fd8f3f")
  (set-face 'font-lock-variable-name-face :foreground "#67b7a4")
  (set-face 'font-lock-builtin-face :foreground "#41a1c0")
  (set-face 'cursor :background "#ffffff" :foreground "#1f1f24")
  (set-face 'echo-area :foreground "#a7afb6" :background "#1f1f24")
  (set-face 'diagnostic-error :foreground "#f74a4a")
  (set-face 'diagnostic-warning :foreground "#efb759")
  (set-face 'diagnostic-info :foreground "#5dd8ff")
  ;; M112: indent-guide was #202126, ratio 1.021 -- invisible. Raised to
  ;; 1.380 (target band 1.20-1.45), a blend of the background toward
  ;; this theme's own region hex.
  (set-face 'indent-guide :foreground "#333742")
  ;; M116 (display-fill-column-indicator-mode): DERIVED in every theme
  ;; -- blended from `default''s own foreground into its background
  ;; (same recipe `scroll-bar' uses one theme-section up), rather than
  ;; from any accent hue, so the ruler reads as neutral chrome, not a
  ;; semantic color competing with `indent-guide' (blue-tinted in every
  ;; theme here) for the same visual role. Contrast ratio against
  ;; `default''s background: 1.860; against `indent-guide': 1.348
  ;; (bar 1.30, chosen so the two are distinguishable when a guide and
  ;; the ruler land on the same column -- this face is painted AFTER
  ;; indent guides in `frontend-gui/src/lib.rs', so on any such overlap
  ;; this face's pixels are the ones that actually show).
  (set-face 'fill-column-indicator :foreground "#4a4a4e")
  ;; M112: scroll-bar was #2c2e37, ratio 1.214. Raised to 1.631, still
  ;; strictly between indent-guide's 1.380 and region's 2.407.
  (set-face 'scroll-bar :foreground "#3c4250")
  (set-face 'rainbow-delimiters-depth-1-face :foreground "#fc5fa3")
  (set-face 'rainbow-delimiters-depth-2-face :foreground "#fc6a5d")
  (set-face 'rainbow-delimiters-depth-3-face :foreground "#d0bf69")
  (set-face 'rainbow-delimiters-depth-4-face :foreground "#fd8f3f")
  (set-face 'rainbow-delimiters-depth-5-face :foreground "#5dd8ff")
  (set-face 'rainbow-delimiters-depth-6-face :foreground "#41a1c0")
  (set-face 'rainbow-delimiters-depth-7-face :foreground "#9ef1dd")
  (set-face 'rainbow-delimiters-depth-8-face :foreground "#67b7a4")
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#f74a4a")
  ;; M112: org faces, DERIVED (Xcode's .xccolortheme has no org-mode
  ;; concept) by reuse of this theme's own token colors: level-1/link->
  ;; keyword, level-2->type, level-3/done->function (Xcode's real teal-
  ;; green, the closest role to "success"), level-4->preprocessor,
  ;; todo->the diagnostic-error color, table->type (shared with
  ;; level-2), date->dired-date.
  (set-face 'org-level-1 :foreground "#fc5fa3" :weight 'bold)
  (set-face 'org-level-2 :foreground "#5dd8ff" :weight 'bold)
  (set-face 'org-level-3 :foreground "#67b7a4")
  (set-face 'org-level-4 :foreground "#fd8f3f")
  (set-face 'org-todo :foreground "#f74a4a" :weight 'bold)
  (set-face 'org-done :foreground "#67b7a4")
  (set-face 'org-table :foreground "#5dd8ff")
  (set-face 'org-date :foreground "#d0bf69")
  (set-face 'org-link :foreground "#fc5fa3" :underline t))

;; VS Code Dark+, copied verbatim from microsoft/vscode's
;; extensions/theme-defaults/themes/dark_plus.json (which itself mostly
;; points at dark_vs.json's TextMate scopes): editor background/
;; foreground, comment, string, keyword (keyword.control shares it),
;; constant.numeric, constant.language (shares keyword's color),
;; entity.name.function (support.function shares it),
;; entity.name.type/support.type (shared), variable, storage.type/
;; meta.preprocessor (both share keyword's color upstream). Those files
;; define token colors, not workbench chrome, so everything else below
;; -- every background surface, the selection color, both line-number
;; shades, all three diagnostic colors, echo-area, indent-guide,
;; scroll-bar, dired-perms/panel-buffer-file's muted grey, and the
;; rainbow-delimiters depth assignment -- is DERIVED here (blends of
;; the editor background/foreground, or reuses one of the token colors
;; above for a second role), not lifted from any VS Code source file.
(defun theme--vscode ()
  (set-face 'default :foreground "#d4d4d4" :background "#1e1e1e")
  (set-face 'region :background "#324a5e")
  ;; M112: hl-line was #222222, ratio 1.048 against this theme's
  ;; #1e1e1e -- invisible. Raised to 1.211 (target band 1.20-1.35),
  ;; still strictly weaker than region's 1.807.
  (set-face 'hl-line :background "#2d2d2d")
  (set-face 'lsp-highlight :background "#2a2a2a")
  ;; M113: DERIVED (20% `diagnostic-warning' blended into `default''s
  ;; background) -- same recipe as every other theme, not VS Code's own
  ;; `editorBracketMatch.background' (a semi-transparent green this
  ;; project's alpha-less `Style' can't represent, and this milestone's
  ;; design deliberately treats the face as derived everywhere rather
  ;; than sourced for some themes and derived for others).
  (set-face 'show-paren-match :background "#675811")
  ;; M116 (show-trailing-whitespace): DERIVED in every theme, not
  ;; sourced from any upstream palette -- same reasoning as
  ;; `show-paren-match' just above (no upstream "trailing whitespace"
  ;; color exists to match), but blended from a 1:1 mix of this theme's
  ;; own `diagnostic-warning'/`diagnostic-error' into `default''s
  ;; background instead of `diagnostic-warning' alone, so the two
  ;; derived backgrounds read as related but distinguishable "something
  ;; needs attention here" tones rather than identical hues. Contrast
  ;; ratio against `default''s background: 2.431; against `region':
  ;; 1.345 (bar 1.30, same test `show-paren-match' already
  ;; clears); against `hl-line': 2.008 (same bar) -- covered by
  ;; `trailing_whitespace_separates_from_region_and_hl_line' in
  ;; `gui_features_tests.rs', since this face can sit on point's own
  ;; current line (wherever point is not at that line's end) at the
  ;; same time as `hl-line', and inside an active selection at the same
  ;; time as `region'.
  (set-face 'trailing-whitespace :background "#824e22")
  ;; M107 fix round: blend(bg,fg,0.30) alone was only 2.24:1 against
  ;; this theme's background -- below the >=3.2 bar this project uses
  ;; elsewhere (see `dark''s fix-5 comment above). Re-derived at
  ;; blend(bg,fg,0.44) -> #6e6e6e, 3.27:1, same neutral-grey family.
  (set-face 'line-number :foreground "#6e6e6e")
  (set-face 'line-number-current-line :foreground "#828282")
  ;; mode-line's #2e2e2e is already ratio 1.228 against this theme's
  ;; #1e1e1e -- already above M112's >=1.20 chrome bar, left unchanged.
  (set-face 'mode-line :foreground "#d4d4d4" :background "#2e2e2e")
  ;; M112: mode-line-inactive was #242424, ratio 1.074. Raised to 1.194,
  ;; still below mode-line's own 1.228 (kept just under the >=1.20 bar
  ;; on purpose so it stays distinguishable from both hl-line's #2d2d2d
  ;; and mode-line's #2e2e2e -- at this theme's near-monochrome grey
  ;; ramp, the three faces are only 1 RGB unit per channel apart, and
  ;; #2d2d2d/#2e2e2e are both already taken).
  (set-face 'mode-line-inactive :foreground "#707070" :background "#2c2c2c")
  ;; M118: see the dark theme's own comment above -- same recipe (50/50
  ;; blend of `lsp-highlight'/`mode-line-inactive'). Contrast against
  ;; `default': 1.177; against `mode-line': 1.043.
  ;;
  ;; M118 review fix (FIX-7): foreground was `line-number-current-line'
  ;; (`#828282') verbatim, ratio 3.684 against this face's own
  ;; background -- markedly weaker than every other theme's
  ;; `scope-header' (5.2-7.6). Raised by blending 26% toward white
  ;; (`#a3a3a3'), ratio 5.613 -- in line with the other themes, still
  ;; this theme's own monochrome-grey family, not a new hue.
  (set-face 'scope-header :foreground "#a3a3a3" :background "#2b2b2b")
  (set-face 'completions-popup :foreground "#d4d4d4" :background "#242424")
  (set-face 'completions-selected :foreground "#d4d4d4" :background "#324a5e")
  (set-face 'panel :foreground "#d4d4d4" :background "#242424")
  (set-face 'panel-selected :foreground "#d4d4d4" :background "#324a5e")
  (set-face 'panel-buffer-name :foreground "#dcdcaa" :background "#242424")
  (set-face 'panel-buffer-mode :foreground "#4ec9b0" :background "#242424")
  (set-face 'panel-buffer-file :foreground "#5e5e5e" :background "#242424")
  (set-face 'panel-modified :foreground "#f14c4c" :background "#242424")
  (set-face 'dired-directory :foreground "#569cd6" :weight 'bold)
  (set-face 'dired-symlink :foreground "#4ec9b0")
  (set-face 'dired-executable :foreground "#ce9178")
  (set-face 'dired-perms :foreground "#5e5e5e")
  (set-face 'dired-size :foreground "#b5cea8")
  ;; M107 fix round 1: this used to be a blend of Comment and
  ;; constant.numeric -- a third derivation rule this defun's own
  ;; comment never disclosed (it only documents "blend bg/fg" or
  ;; "reuse a token color"). Switched to a straight reuse of
  ;; font-lock-comment-face's color (#6a9955), which fits the
  ;; disclosed rules -- but #6a9955 sits at hue ~101 deg, almost
  ;; exactly on top of dired-size's #b5cea8 at hue ~99 deg (they only
  ;; differ in lightness), and these two columns sit right next to
  ;; each other in the dired listing (crates/core/src/dired.rs:237-240
  ;; renders perms -> size -> date in that order). M107 fix round 2:
  ;; switched instead to font-lock-variable-name-face's color
  ;; (#9cdcfe, hue ~201 deg), a ~100 deg hue difference from dired-size
  ;; -- still a disclosed token-color reuse, just a different token.
  (set-face 'dired-date :foreground "#9cdcfe")
  (set-face 'font-lock-keyword-face :foreground "#569cd6")
  (set-face 'font-lock-string-face :foreground "#ce9178")
  (set-face 'font-lock-comment-face :foreground "#6a9955" :slant 'italic)
  (set-face 'font-lock-function-name-face :foreground "#dcdcaa")
  (set-face 'font-lock-type-face :foreground "#4ec9b0")
  (set-face 'font-lock-constant-face :foreground "#b5cea8")
  (set-face 'font-lock-preprocessor-face :foreground "#569cd6")
  (set-face 'font-lock-variable-name-face :foreground "#9cdcfe")
  (set-face 'font-lock-builtin-face :foreground "#dcdcaa")
  (set-face 'cursor :background "#d4d4d4" :foreground "#1e1e1e")
  (set-face 'echo-area :foreground "#828282" :background "#1e1e1e")
  (set-face 'diagnostic-error :foreground "#f14c4c")
  (set-face 'diagnostic-warning :foreground "#cca700")
  (set-face 'diagnostic-info :foreground "#3794ff")
  ;; M112: indent-guide was #282828, ratio 1.131 -- invisible. Raised to
  ;; 1.400 (target band 1.30-1.45).
  (set-face 'indent-guide :foreground "#373737")
  ;; M116 (display-fill-column-indicator-mode): DERIVED in every theme
  ;; -- blended from `default''s own foreground into its background
  ;; (same recipe `scroll-bar' uses one theme-section up), rather than
  ;; from any accent hue, so the ruler reads as neutral chrome, not a
  ;; semantic color competing with `indent-guide' (blue-tinted in every
  ;; theme here) for the same visual role. Contrast ratio against
  ;; `default''s background: 1.823; against `indent-guide': 1.302
  ;; (bar 1.30, chosen so the two are distinguishable when a guide and
  ;; the ruler land on the same column -- this face is painted AFTER
  ;; indent guides in `frontend-gui/src/lib.rs', so on any such overlap
  ;; this face's pixels are the ones that actually show).
  (set-face 'fill-column-indicator :foreground "#484848")
  ;; M112: scroll-bar was #323232, ratio 1.300. Raised to 1.629, still
  ;; strictly between indent-guide's 1.400 and region's 1.807.
  (set-face 'scroll-bar :foreground "#2f4354")
  (set-face 'rainbow-delimiters-depth-1-face :foreground "#569cd6")
  (set-face 'rainbow-delimiters-depth-2-face :foreground "#ce9178")
  (set-face 'rainbow-delimiters-depth-3-face :foreground "#6a9955")
  (set-face 'rainbow-delimiters-depth-4-face :foreground "#dcdcaa")
  (set-face 'rainbow-delimiters-depth-5-face :foreground "#4ec9b0")
  (set-face 'rainbow-delimiters-depth-6-face :foreground "#9cdcfe")
  (set-face 'rainbow-delimiters-depth-7-face :foreground "#b5cea8")
  (set-face 'rainbow-delimiters-depth-8-face :foreground "#f14c4c")
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#cca700")
  ;; M112: org faces, DERIVED (dark_plus.json defines token colors, not
  ;; an org-mode-equivalent) by reuse of this theme's own token colors:
  ;; level-1/link->keyword, level-2/table->type, level-3->function,
  ;; level-4->string, todo->the diagnostic-error color, done->constant
  ;; (the closest "success green" this theme's token set has),
  ;; date->the same light-blue dired-date already picked in M107.
  (set-face 'org-level-1 :foreground "#569cd6" :weight 'bold)
  (set-face 'org-level-2 :foreground "#4ec9b0" :weight 'bold)
  (set-face 'org-level-3 :foreground "#dcdcaa")
  (set-face 'org-level-4 :foreground "#ce9178")
  (set-face 'org-todo :foreground "#f14c4c" :weight 'bold)
  (set-face 'org-done :foreground "#b5cea8")
  (set-face 'org-table :foreground "#4ec9b0")
  (set-face 'org-date :foreground "#9cdcfe")
  (set-face 'org-link :foreground "#569cd6" :underline t))

(defvar current-theme nil)

(defvar theme--names '("dracula" "xcode" "vscode" "dark" "light")
  "The five theme names `load-theme' knows about, in the order offered
by its `completing-read' prompt and listed in its error message.")

(defun load-theme (&optional name)
  "Switch to theme NAME, one of `dracula' (the default), `xcode',
`vscode', `dark', or `light'. Called interactively with no argument,
prompts for the name via `completing-read' over `theme--names' (the
same pattern as `format-set-style'/`set-font') and recurses with the
chosen symbol; called from Lisp with NAME already bound, applies it
directly."
  (interactive)
  (if (not name)
      (with-completing-read
       (choice "Theme: " theme--names t)
       (load-theme (intern choice)))
    (cond
     ((eq name 'dracula) (theme--dracula))
     ((eq name 'xcode) (theme--xcode))
     ((eq name 'vscode) (theme--vscode))
     ((eq name 'dark) (theme--dark))
     ((eq name 'light) (theme--light))
     (t (error "Unknown theme: %s (try %s)" name
               (string-join theme--names ", "))))
    (setq current-theme name)
    (message "Theme: %s" name)
    name))

;; Dracula by default (M107 -- the owner's explicit call).
(load-theme 'dracula)
