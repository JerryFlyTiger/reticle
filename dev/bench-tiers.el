;;; bench-tiers.el --- measure the three execution tiers  -*- lexical-binding: t -*-
;;
;;     cargo build --release --workspace
;;     ./target/release/reticle --script dev/bench-tiers.el
;;
;; Prints the speedup of the bytecode VM and the native JIT over the
;; tree-walking evaluator, on two workloads that behave very differently.
;;
;; Why this file exists: the README used to quote tier speedups that nothing
;; in the repository reproduced. A number no one can re-derive is a number no
;; one should trust, so the claim and the thing that produces it now ship
;; together. Re-run this after touching the compiler, the VM, or the JIT.
;;
;; Two workloads, because a single number would be misleading:
;;
;;   loop  -- a tight integer `while' loop. Locals and whitelisted arithmetic
;;            only, so it is inside the JIT's eligible subset.
;;   fib   -- naive recursive Fibonacci. Same integer arithmetic, but the
;;            self-call puts it outside that subset, so `native-compile'
;;            leaves it byte-compiled. This is the honest counterexample: it
;;            shows what the tiers do NOT buy you.
;;
;; Use a release build. On a debug build the tree-walking evaluator is itself
;; slowed by roughly an order of magnitude, which inflates every ratio here.

(defun bench--loop-tw (n) (let ((i 0) (s 0)) (while (< i n) (setq s (+ s i)) (setq i (+ i 1))) s))
(defun bench--loop-bc (n) (let ((i 0) (s 0)) (while (< i n) (setq s (+ s i)) (setq i (+ i 1))) s))
(defun bench--loop-jit (n) (let ((i 0) (s 0)) (while (< i n) (setq s (+ s i)) (setq i (+ i 1))) s))

(defun bench--fib-tw (n) (if (< n 2) n (+ (bench--fib-tw (- n 1)) (bench--fib-tw (- n 2)))))
(defun bench--fib-bc (n) (if (< n 2) n (+ (bench--fib-bc (- n 1)) (bench--fib-bc (- n 2)))))
(defun bench--fib-jit (n) (if (< n 2) n (+ (bench--fib-jit (- n 1)) (bench--fib-jit (- n 2)))))

(byte-compile 'bench--loop-bc)
(native-compile 'bench--loop-jit)
(byte-compile 'bench--fib-bc)
(native-compile 'bench--fib-jit)

;; `native-compile' returns its argument whether or not the function was
;; eligible -- "not eligible" is an expected outcome, not an error. Ask the
;; predicate instead, and note it wants the function object, not the symbol.
(defun bench--native-p (sym) (native-compiled-function-p (symbol-function sym)))

(defun bench--time (f n)
  (let ((t0 (float-time)))
    (funcall f n)
    (- (float-time) t0)))

;; Take the best of several runs rather than one sample: the first run pays
;; for cold caches, and anything else on the machine only ever makes a run
;; slower, never faster.
(defun bench--best (f n rounds)
  (let ((best (bench--time f n)) (i 1))
    (while (< i rounds)
      (let ((this (bench--time f n)))
        (when (< this best) (setq best this)))
      (setq i (+ i 1)))
    best))

(defun bench--report (label tw bc jit native-p)
  ;; This editor's `format' has no field-width or precision specifiers (see the
  ;; known gaps in README.md), so lay the line out with plain %s and let the
  ;; float print itself.
  (message "%s: tree-walk %ss | bytecode %ss (%sx) | native %ss (%sx)%s"
           label tw bc (/ tw bc) jit (/ tw jit)
           (if native-p "" "  [not JIT-eligible: still bytecode]")))

(let ((n 300000) (rounds 5))
  (bench--report "loop"
                 (bench--best 'bench--loop-tw n rounds)
                 (bench--best 'bench--loop-bc n rounds)
                 (bench--best 'bench--loop-jit n rounds)
                 (bench--native-p 'bench--loop-jit)))

(let ((n 22) (rounds 5))
  (bench--report "fib"
                 (bench--best 'bench--fib-tw n rounds)
                 (bench--best 'bench--fib-bc n rounds)
                 (bench--best 'bench--fib-jit n rounds)
                 (bench--native-p 'bench--fib-jit)))
