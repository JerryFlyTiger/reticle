;;; prelude.el --- reticle bootstrap library -*- lexical-binding: t -*-

;; Standard macros and functions implemented in our own elisp,
;; loaded into every interpreter at startup.

(defmacro when (cond &rest body)
  `(if ,cond (progn ,@body)))

(defmacro unless (cond &rest body)
  `(if ,cond nil ,@body))

(defmacro dolist (spec &rest body)
  (let ((var (car spec))
        (listform (cadr spec))
        (result (caddr spec))
        (tail (gensym "dolist-tail-")))
    `(let ((,tail ,listform))
       (while ,tail
         (let ((,var (car ,tail)))
           ,@body)
         (setq ,tail (cdr ,tail)))
       (let ((,var nil))
         ,result))))

(defmacro dotimes (spec &rest body)
  (let ((var (car spec))
        (count (cadr spec))
        (result (caddr spec))
        (n (gensym "dotimes-n-")))
    `(let ((,n ,count)
           (,var 0))
       (while (< ,var ,n)
         ,@body
         (setq ,var (1+ ,var)))
       ,result)))

(defmacro push (val place)
  `(setq ,place (cons ,val ,place)))

(defmacro pop (place)
  `(prog1 (car ,place)
     (setq ,place (cdr ,place))))

(defmacro ignore-errors (&rest body)
  `(condition-case nil (progn ,@body) (error nil)))

(defmacro defcustom (symbol value &rest _args)
  `(defvar ,symbol ,value))

(defun zerop (n) (= n 0))
(defun natnump (n) (and (integerp n) (>= n 0)))

(defun alist-get (key alist &optional default)
  (let ((entry (assq key alist)))
    (if entry (cdr entry) default)))

;; Hooks.
(defun add-hook (hook function &optional append _local)
  (unless (boundp hook) (set hook nil))
  (let ((val (symbol-value hook)))
    (unless (member function val)
      (set hook (if append
                    (append val (list function))
                  (cons function val))))))

(defun remove-hook (hook function)
  (when (boundp hook)
    (set hook (delete function (symbol-value hook)))))

(defun run-hooks (&rest hooks)
  (dolist (h hooks)
    (when (boundp h)
      (dolist (f (symbol-value h))
        (funcall f)))))

;; M15 hook watchdog: per-function time budget (milliseconds) for hooks
;; on the keystroke path (post-command-hook, post-self-insert-hook). A
;; hook function exceeding it is interrupted with `elisp-timeout', named
;; in the echo area, and removed after three strikes. nil disables.
(defvar hook-time-budget 50)

;; M40: save/kill hooks. Unbudgeted (see commands.rs's KEYSTROKE_HOOKS
;; doc comment) -- these run off the keystroke path, same as
;; find-file-hook.
(defvar before-save-hook nil
  "Functions run by `save-buffer', with no arguments, after it has
confirmed the buffer visits a file but before the buffer's text is
written to disk -- edits made here (e.g. `verilog-auto-on-save') are
included in the write.")

(defvar after-save-hook nil
  "Functions run by `save-buffer', with no arguments, after the write
to disk has succeeded and the buffer's modified flag has been
cleared.")

(defvar kill-buffer-hook nil
  "Functions run by `kill-buffer', with no arguments, with the buffer
being killed current, before it is removed.")

(provide 'prelude)

;; M9 cycle collector: how many mutated (potential-cycle) objects may
;; accumulate before a collection is forced at the next quiescent point.
;; Idle-time collection runs regardless once a small floor is passed, so
;; this only matters under sustained mutation-heavy load.
(defvar gc-cons-threshold 100000)

;;; --- M11: property-list helpers -------------------------------------

(defun plist-get (plist prop)
  "Value of PROP in the property list PLIST, or nil."
  (catch 'plist--done
    (while plist
      (when (eq (car plist) prop)
        (throw 'plist--done (cadr plist)))
      (setq plist (cddr plist)))
    nil))

(defun plist-member (plist prop)
  "Tail of PLIST starting at PROP, or nil — distinguishes an explicit
nil value from an absent property."
  (catch 'plist--done
    (while plist
      (when (eq (car plist) prop)
        (throw 'plist--done plist))
      (setq plist (cddr plist)))
    nil))

(defun plist-put (plist prop val)
  "Store PROP -> VAL in PLIST destructively; use as
\(setq l (plist-put l prop val)) like in GNU Emacs."
  (if (null plist)
      (list prop val)
    (let ((tail plist))
      (catch 'plist--done
        (while t
          (when (eq (car tail) prop)
            (setcar (cdr tail) val)
            (throw 'plist--done plist))
          (if (cddr tail)
              (setq tail (cddr tail))
            (progn
              (setcdr (cdr tail) (list prop val))
              (throw 'plist--done plist))))))))

;;; --- M11: setf (generalized variables) ------------------------------
;; A place form (F args...) is settable when F's symbol carries a
;; 'setf-expander property: a function (PLACE VAL) -> replacement form.
;; Subforms may be evaluated more than once (documented v1 limitation).

(defmacro setf (&rest pairs)
  (if (cddr pairs)
      (let ((forms nil))
        (while pairs
          (push (list 'setf (car pairs) (cadr pairs)) forms)
          (setq pairs (cddr pairs)))
        (cons 'progn (nreverse forms)))
    (let ((place (car pairs)) (val (cadr pairs)))
      (if (symbolp place)
          (list 'setq place val)
        (let ((expander (get (car place) 'setf-expander)))
          (unless expander
            (error "No setf expander for %s" (car place)))
          (funcall expander place val))))))

(put 'car 'setf-expander
     (lambda (place val) (list 'setcar (cadr place) val)))
(put 'cdr 'setf-expander
     (lambda (place val) (list 'setcdr (cadr place) val)))
(put 'nth 'setf-expander
     (lambda (place val)
       (list 'setcar (list 'nthcdr (cadr place) (caddr place)) val)))
(put 'aref 'setf-expander
     (lambda (place val) (list 'aset (cadr place) (caddr place) val)))
(put 'gethash 'setf-expander
     (lambda (place val) (list 'puthash (cadr place) val (caddr place))))
(put 'symbol-function 'setf-expander
     (lambda (place val) (list 'fset (cadr place) val)))

;;; --- M11: cl-defstruct ----------------------------------------------
;; v1: (cl-defstruct NAME SLOT...) where SLOT is SYM or (SYM DEFAULT).
;; Representation: a vector [cl-struct-NAME slot...]. Generates
;; make-NAME (keyword arguments, defaults, explicit nil respected),
;; NAME-p, copy-NAME, one accessor per slot, and setf support for the
;; accessors. No :include/:constructor options in v1.

(defmacro cl-defstruct (name &rest slots)
  (let* ((sname (symbol-name name))
         (tag (intern (concat "cl-struct-" sname)))
         (size (+ (length slots) 1))
         (pred (intern (concat sname "-p")))
         (forms nil)
         (slot-inits nil)
         (i 1))
    ;; Constructor slot initializers, in slot order.
    (dolist (slot slots)
      (let* ((sym (if (consp slot) (car slot) slot))
             (default (if (consp slot) (cadr slot) nil))
             (kw (intern (concat ":" (symbol-name sym)))))
        (push (list 'aset 'v i
                    (list 'let (list (list 'm (list 'plist-member 'args (list 'quote kw))))
                          (list 'if 'm (list 'cadr 'm) default)))
              slot-inits)
        (setq i (+ i 1))))
    (push (list 'defun (intern (concat "make-" sname)) '(&rest args)
                (append (list 'let (list (list 'v (list 'make-vector size nil))))
                        (list (list 'aset 'v 0 (list 'quote tag)))
                        (nreverse slot-inits)
                        (list 'v)))
          forms)
    (push (list 'defun pred '(obj)
                (list 'and '(vectorp obj)
                      (list '= '(length obj) size)
                      (list 'eq '(aref obj 0) (list 'quote tag))))
          forms)
    (push (list 'defun (intern (concat "copy-" sname)) '(obj)
                '(copy-sequence obj))
          forms)
    ;; Accessors + their setf expanders.
    (setq i 1)
    (dolist (slot slots)
      (let* ((sym (if (consp slot) (car slot) slot))
             (acc (intern (concat sname "-" (symbol-name sym)))))
        (push (list 'defun acc '(obj)
                    (list 'unless (list pred 'obj)
                          (list 'error "%s: not a %s" (list 'quote acc) sname))
                    (list 'aref 'obj i))
              forms)
        (push (list 'put (list 'quote acc) '(quote setf-expander)
                    (list 'let (list (list 'idx i))
                          '(lambda (place val)
                             (list 'aset (cadr place) idx val))))
              forms)
        (setq i (+ i 1))))
    (cons 'progn (nreverse forms))))

;;; --- M11: advice ----------------------------------------------------
;; (advice-add 'f :before FN) / :after / :around / :override, and
;; (advice-remove 'f FN). Rebuilds the function from the original plus
;; the current advice list, so removal restores cleanly.

(defun advice--build (orig advices)
  (let ((f orig))
    (dolist (a (reverse advices))
      (let ((how (car a)) (fn (cdr a)) (inner f))
        (setq f
              (cond
               ((eq how :before)
                (lambda (&rest args) (apply fn args) (apply inner args)))
               ((eq how :after)
                (lambda (&rest args) (prog1 (apply inner args) (apply fn args))))
               ((eq how :around)
                (lambda (&rest args) (apply fn inner args)))
               ((eq how :override)
                (lambda (&rest args) (apply fn args)))
               (t (error "Unknown advice type %s" how))))))
    f))

(defun advice-add (sym how fn)
  (unless (get sym 'advice--original)
    (put sym 'advice--original (symbol-function sym)))
  (put sym 'advice--list (cons (cons how fn) (get sym 'advice--list)))
  (fset sym (advice--build (get sym 'advice--original)
                           (get sym 'advice--list)))
  nil)

(defun advice-remove (sym fn)
  (let ((kept nil))
    (dolist (a (get sym 'advice--list))
      (unless (eq (cdr a) fn)
        (push a kept)))
    (setq kept (nreverse kept))
    (put sym 'advice--list kept)
    (if kept
        (fset sym (advice--build (get sym 'advice--original) kept))
      (progn
        (fset sym (get sym 'advice--original))
        (put sym 'advice--original nil))))
  nil)

;;; --- M11: pcase -----------------------------------------------------
;; Supported patterns: _  SYMBOL (binds)  literals (integer / string /
;; keyword / t / nil)  'DATUM  (pred FN)  (guard EXPR)  (and P...)
;; (or P...)  and backquote patterns `(...) with ,SUBPATTERN unquoting
;; (dotted tails work; atoms inside compare with eq/equal).

(defvar pcase--backquote (intern "`"))
(defvar pcase--unquote (intern ","))

(defun pcase--expand (pat v succ)
  (cond
   ((eq pat '_) succ)
   ((eq pat t) (list 'when (list 'eq v t) succ))
   ((null pat) (list 'when (list 'null v) succ))
   ((keywordp pat) (list 'when (list 'eq v pat) succ))
   ((symbolp pat) (list 'let (list (list pat v)) succ))
   ((integerp pat) (list 'when (list 'equal v pat) succ))
   ((stringp pat) (list 'when (list 'equal v pat) succ))
   ((consp pat)
    (let ((head (car pat)))
      (cond
       ((eq head 'quote) (list 'when (list 'equal v pat) succ))
       ((eq head 'pred) (list 'when (list (cadr pat) v) succ))
       ((eq head 'guard) (list 'when (cadr pat) succ))
       ((eq head 'and)
        (let ((form succ))
          (dolist (p (reverse (cdr pat)))
            (setq form (pcase--expand p v form)))
          form))
       ((eq head 'or)
        (cons 'progn
              (let ((out nil))
                (dolist (p (cdr pat))
                  (push (pcase--expand p v succ) out))
                (nreverse out))))
       ((eq head pcase--backquote)
        (pcase--expand-bq (cadr pat) v succ))
       (t (error "Unsupported pcase pattern: %s" pat)))))
   (t (error "Unsupported pcase pattern: %s" pat))))

(defun pcase--expand-bq (pat v succ)
  (cond
   ((null pat) (list 'when (list 'null v) succ))
   ((consp pat)
    (if (eq (car pat) pcase--unquote)
        (pcase--expand (cadr pat) v succ)
      (let ((hv (gensym "pcase-h")) (tv (gensym "pcase-t")))
        (list 'when (list 'consp v)
              (list 'let (list (list hv (list 'car v))
                               (list tv (list 'cdr v)))
                    (pcase--expand-bq (car pat) hv
                                      (pcase--expand-bq (cdr pat) tv succ)))))))
   ((symbolp pat) (list 'when (list 'eq v (list 'quote pat)) succ))
   (t (list 'when (list 'equal v pat) succ))))

(defmacro pcase (expr &rest cases)
  (let ((v (gensym "pcase-v")))
    (list 'let (list (list v expr))
          (append (list 'catch '(quote pcase--done))
                  (let ((out nil))
                    (dolist (c cases)
                      (push (pcase--expand
                             (car c) v
                             (list 'throw '(quote pcase--done)
                                   (cons 'progn (cdr c))))
                            out))
                    (nreverse out))
                  (list nil)))))
