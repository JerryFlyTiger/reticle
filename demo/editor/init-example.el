;;; init-example.el --- a working init.el for RTL work -*- lexical-binding: t -*-

;; Copy to ~/.config/reticle/init.el (or wherever this build reads
;; its init file from) and edit to taste. Everything below uses
;; variables and commands that ship with the editor -- nothing here
;; needs a plugin, because the Verilog support is built in.
;;
;; This file is also the ninth language in demo/: Emacs Lisp is what the
;; editor is configured and extended in, and this is what real
;; configuration looks like.

;; --- Verilog library search ------------------------------------------
;;
;; Where to look for a module that is instantiated here but declared
;; somewhere else. This is what backs `M-.' (jump to module) and `C-M-i'
;; (port-name completion) when no language server is running.
;;
;; Entries are relative to the directory of the file you are editing.
;; Since M56 each entry is searched RECURSIVELY, breadth-first, so one
;; entry usually covers a whole tree.
(setq verilog-library-directories '("." ".." "../.."))

;; Guards on that recursion. They are not performance knobs -- a 500-file
;; scan costs single-digit milliseconds. They exist because this
;; interpreter has no `file-symlink-p', so a symlink loop can only be
;; stopped by bounding the walk.
(setq verilog-library-max-depth 8)
(setq verilog-library-max-files 2000)

;; If the project root holds a `verible.filelist', use it as well as the
;; directory scan. This is the same list verible-verilog-ls reads, so
;; the editor and the language server agree on one set of files instead
;; of each guessing. Set to nil to ignore it entirely.
(setq verilog-library-use-filelist t)

;; --- AUTOINST --------------------------------------------------------
;;
;; `C-c C-a' expands /*AUTOINST*/, /*AUTOWIRE*/ and /*AUTOARG*/ the way
;; GNU verilog-mode's AUTO macros do; `C-c C-k' deletes the expansion
;; again. Column the generated `.NAME' fields are padded to:
(setq verilog-auto-inst-column 40)

;; Opt in to re-expanding on every save, so the expanded text is what
;; reaches disk. Off by default because it rewrites the buffer for you.
(setq verilog-auto-on-save t)

;; --- Language server -------------------------------------------------
;;
;; `M-x lsp' once is enough for a whole project: after that, opening
;; another file under the same project root (walking up for a marker
;; like `verible.filelist' -- see `lsp--project-root-markers') attaches
;; it to the same connection automatically, no second `M-x lsp'. Files
;; already open at the moment you run `M-x lsp' get swept in too, so it
;; doesn't matter whether you connect before or after opening the rest
;; of the design. Controlled by `lsp-auto-attach' (on by default).
;;
;; verible-verilog-ls is the default: a parser plus a style linter.
;; Good formatting, good lint, no completion at all, no cross-file
;; elaboration.
;;
;; slang-server (https://github.com/hudson-trading/slang-server) is the
;; other one worth knowing. It is NOT a strict upgrade -- both halves of
;; this were measured with dev/lsp-probe.py against slang-server 0.2.9
;; on demo/rtl/:
;;
;;   slang-server wins on semantics. Hover on a signal answers with the
;;   resolved type and width (`logic[31:0]', `Width: 32', `Driver:
;;   Continuous'); cross-file definition works straight off its own
;;   workspace index; it has a real completionProvider.
;;
;;   verible wins on formatting. slang-server declares no
;;   documentFormattingProvider at all, and a real formatting request
;;   sent to it anyway gets no reply, ever -- not even an error. As of
;;   M57, reticle checks that capability before sending, so the
;;   before-save hook below no longer sends a doomed request; it just
;;   does nothing when you switch. That's still worth knowing about
;;   rather than assuming: a `message' from inside before-save-hook
;;   alone would be invisible anyway, since save-buffer overwrites the
;;   echo area with "Wrote ..." right after the hook runs -- what
;;   actually tells you is `M-x lsp' itself, which reports the gap once
;;   at connect time ("... (unsupported: lsp-format-buffer,
;;   lsp-format-region)").
;;
;; (add-to-list 'lsp-server-alist '(verilog-mode . ("slang-server")))
;;
;; If you do switch: slang-server indexes everything under the project
;; ROOT it is handed and answers from that index, so the root has to be
;; right or every cross-file request comes back empty while the server
;; still claims full capabilities. reticle finds that root by
;; walking up for `.slang' (among other markers), which is the directory
;; slang-server keeps its own config in -- so a project with a `.slang/'
;; at its top needs no configuration here at all.

;; The local tiers above run BEFORE any language server, so `M-.' and
;; `C-M-i' keep working in a buffer with no server attached at all.

;; --- Per-buffer setup ------------------------------------------------

(add-hook 'verilog-mode-hook
          (lambda ()
            ;; Most RTL style guides say two spaces; the built-in
            ;; default is four.
            (setq-local tab-width 2)
            ;; A design with a deep `rtl/' tree usually wants the search
            ;; anchored at the project root rather than at the file, so
            ;; a module in a sibling directory is still found.
            (setq-local verilog-library-directories '("." ".."))))

;; Format the buffer through the language server before saving. When no
;; server is attached, `lsp-format-buffer' does report that ("No LSP
;; server connected in this buffer (M-x lsp first)") -- but that message
;; is invisible in practice, because `save-buffer' runs this hook and
;; then immediately overwrites the echo area with "Wrote ...". Same
;; blind spot as the slang-server capability gap noted above: what
;; actually surfaces the problem is `M-x lsp' itself (either "No LSP
;; server registered for ..." if you never connected, or the
;; unsupported-feature note at connect time if you did).
;;
;; Note `(major-mode-internal-get)' rather than a bare `major-mode'
;; variable: in this editor the current mode lives on the buffer and is
;; read through that accessor, which is what the shipped Verilog code
;; itself uses (see `verilog-auto--maybe-on-save').
(add-hook 'before-save-hook
          (lambda ()
            (when (eq (major-mode-internal-get) 'verilog-mode)
              (lsp-format-buffer))))

;; --- Keys ------------------------------------------------------------
;;
;; `M-.', `C-M-i', `M-/' and `C-x d' are bound out of the box. These are
;; the two that come up often enough in RTL work to deserve a key:

;; Drop the cached parse of every library file -- use after a submodule's
;; ports change on disk while the editor is running.
(global-set-key "C-c C-r" 'verilog-complete-clear-library-cache)

;; Re-run AUTO expansion from anywhere, not only a Verilog buffer's own
;; local map.
(global-set-key "C-c a" 'verilog-auto)

;; --- Editing style ---------------------------------------------------

;; evil-mode is on by default; set this before it loads to start in
;; ordinary Emacs bindings instead.
;; (setq evil-auto-enable nil)

;;; init-example.el ends here
