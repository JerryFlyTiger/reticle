;;; probe.el --- how GNU Emacs resolves compile error paths under make -*- lexical-binding: t -*-
;;
;; Run by run.sh: emacs -Q --batch -l probe.el LOGDIR PROJDIR LOG...
;; For every error GNU's compilation-mode finds in each LOG, prints the error
;; line, the directory GNU attached to it (nil = none, i.e. the buffer's
;; default-directory), and whether the resolved file exists.

(require 'compile)

(let* ((args command-line-args-left)
       (logdir (file-name-as-directory (pop args)))
       (proj (file-name-as-directory (pop args))))
  (setq command-line-args-left nil)
  (princ (format "compilation-directory-matcher=%S\n" compilation-directory-matcher))
  (dolist (log args)
    (with-temp-buffer
      (insert-file-contents (expand-file-name log logdir))
      (setq default-directory proj)
      (compilation-mode)
      (goto-char (point-min))
      (princ (format "== %s\n" log))
      (condition-case err
          (while t
            (compilation-next-error 1)
            (let* ((loc (compilation--message->loc
                         (get-text-property (point) 'compilation-message)))
                   (spec (car (compilation--loc->file-struct loc)))
                   (full (expand-file-name (car spec) (or (nth 1 spec) proj))))
              (princ (format "%s  dir=%S exists=%S\n"
                             (buffer-substring (line-beginning-position)
                                               (line-end-position))
                             (nth 1 spec) (file-exists-p full)))))
        (error (princ (format "END: %S\n" err)))))))
