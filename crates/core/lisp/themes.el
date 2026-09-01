;;; themes.el --- built-in dark/light themes (M16)

;; VS Code-style semantic tokens expressed as faces. `default' is the
;; frame's ground colors — both frontends fill every unstyled cell from
;; it, so (load-theme 'light) genuinely relights the whole frame, TUI
;; included. Org's own faces are set in org.el and deliberately left
;; alone here.

(defun theme--dark ()
  (set-face 'default :foreground "#c5cad3" :background "#191b20")
  (set-face 'region :background "#2c4463")
  (set-face 'hl-line :background "#20232a")
  ;; M49: textDocument/documentHighlight -- distinct from `region'/`hl-line'
  ;; so a highlighted symbol reads clearly without competing with an
  ;; active selection. M-visual-quality: mapped to the nearest listed
  ;; role, mode-line-inactive's background, since both are a subtle
  ;; "raised surface" tone a step off the default background.
  (set-face 'lsp-highlight :background "#1e2128")
  ;; M-visual-quality fix 5: raised from #454b57 (contrast ratio 1.97
  ;; against the #191b20 background) to #646c7d (3.27) -- below the
  ;; readable threshold measured by the review.
  (set-face 'line-number :foreground "#646c7d")
  (set-face 'line-number-current-line :foreground "#98a1b0")
  (set-face 'mode-line :foreground "#c5cad3" :background "#262b34")
  (set-face 'mode-line-inactive :foreground "#6b7280" :background "#1e2128")
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
  (set-face 'indent-guide :foreground "#2b2f38")
  (set-face 'scroll-bar :foreground "#3a404b")
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
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#6cb6ff"))

(defun theme--light ()
  (set-face 'default :foreground "#2c313a" :background "#fbfbfd")
  (set-face 'region :background "#cfe0f5")
  (set-face 'hl-line :background "#f0f2f7")
  ;; M49: see the dark theme's own comment above.
  (set-face 'lsp-highlight :background "#f1f3f7")
  ;; M-visual-quality fix 5: raised from #b0b6c0 (contrast ratio 1.97
  ;; against the #fbfbfd background) to #878e9a (3.19).
  (set-face 'line-number :foreground "#878e9a")
  (set-face 'line-number-current-line :foreground "#5a6270")
  (set-face 'mode-line :foreground "#2c313a" :background "#e8ebf1")
  (set-face 'mode-line-inactive :foreground "#8a919c" :background "#f1f3f7")
  (set-face 'completions-popup :foreground "#2c313a" :background "#f1f3f7")
  (set-face 'completions-selected :foreground "#2c313a" :background "#cfe0f5")
  (set-face 'panel :foreground "#2c313a" :background "#f1f3f7")
  (set-face 'panel-selected :foreground "#2c313a" :background "#cfe0f5")
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
  (set-face 'indent-guide :foreground "#e2e6ee")
  (set-face 'scroll-bar :foreground "#c8cdd6")
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
  (set-face 'rainbow-delimiters-depth-9-face :foreground "#1a73c7"))

(defvar current-theme nil)

(defun load-theme (name)
  "Switch to theme NAME (`dark' or `light')."
  (interactive "Stheme (dark/light): ")
  (cond
   ((eq name 'dark) (theme--dark))
   ((eq name 'light) (theme--light))
   (t (error "Unknown theme: %s (try dark or light)" name)))
  (setq current-theme name)
  (message "Theme: %s" name)
  name)

;; Dark by default, the modern-editor convention.
(load-theme 'dark)
