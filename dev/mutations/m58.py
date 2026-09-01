# M58 -- Persistent smoke tests backing the claims made by the demo/ showcase.
# List designed by the reviewer from a cold read of the diff, executed by the
# main conversation (implementers don't verify their own fix).
#
# This one differs in kind from the previous ones: M58 **changes no product
# code at all**, the whole deliverable is tests. So what's being verified here
# isn't "is the fix being watched", but "do these tests actually pin down the
# product behavior they claim to pin down" -- breaking the product code should
# turn the corresponding smoke test red.
#
# Every entry writes a `test` field naming a single test, because what's being
# verified is "which specific test goes red", not "did the whole file go red"
# -- the reviewer's highest-severity finding was exactly that "test A goes red
# so things look fine, but test B is itself green under the very same failure
# mode". Test names are complete and unique, so they never match 0 (matching 0
# makes cargo return 0, which gets recorded as SURVIVED -- a false negative).
#
# How to run:
#     dev/mutate.py --config dev/mutations/m58.py
#
# M1a is the fix-up round's acceptance point. The version handed off by the
# reviewer was **SURVIVED** under this mutation: when M-. is completely
# broken, the buffer never leaves soc_top.sv, `lsp--marker-stack` is never
# pushed, and popping an empty stack is a no-op, so both the "back in
# soc_top.sv" and "point unchanged" assertions coincidentally hold. The fix-up
# round added a check before the pop requiring that alu.sv was genuinely
# reached first, which should flip M1a to FAIL.
#
# Items that are black-box unobservable and **deliberately left out of the
# list**:
#
# 1. The "disk bytes identical before and after" guard at the end of
#    `arbiter_autoinst_expands_then_deletes`. The current call chain
#    (find-file-internal -> verilog-auto -> verilog-delete-auto ->
#    buffer-string) has no line that writes back to disk, so there's no
#    revertible line to make it surface. It guards against a future
#    regression, not a currently observable path. Honestly recorded:
#    black-box, cannot be mutation-verified. As an aside, it's also only
#    reached once all the preceding assertions have already passed.
# 2. The test file's own two self-guards -- the assertion against duplicate
#    keys in the expectation table, and walk_files's panic on symlinks. They
#    guard against **the test itself** regressing; there's no corresponding
#    line in the product code to revert.
# 3. Everything stated in `demo/README.md`: no test reads it. Its correctness
#    rests on the main conversation having checked the numbers directly on
#    the spot (22 entries = 18 language files + 4 fundamental), not on a test.

PACKAGE = "core"
TEST_TARGET = "demo_smoke_tests"

MUTATIONS = [
    {
        "label": "M1a cross-file module lookup always fails -> M-, back-jump test should go red (fix-up round acceptance point)",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "          (let ((hit (verilog-nav--find-module-in-libraries type-name)))",
        "new": "          (let ((hit nil))",
        "test": "m_comma_returns_from_alu_back_to_the_instantiation",
    },
    {
        "label": "M1b same break -> M-. jump test should go red",
        "file": "crates/core/lisp/verilog-nav.el",
        "old": "          (let ((hit (verilog-nav--find-module-in-libraries type-name)))",
        "new": "          (let ((hit nil))",
        "test": "soc_top_m_dot_jumps_to_the_alu_module_definition",
    },
    {
        "label": "M2 lsp-pop-definition-stack does not jump back (only the message branch left)",
        "file": "crates/core/lisp/lsp.el",
        "old": """      (if (not (lsp--buffer-live-p buf))
          (message "Buffer for previous position no longer exists")
        (switch-to-buffer buf)
        (goto-char (marker-position marker))))))""",
        "new": """      (if (not (lsp--buffer-live-p buf))
          (message "Buffer for previous position no longer exists")
        (ignore buf marker)))))""",
        "test": "m_comma_returns_from_alu_back_to_the_instantiation",
    },
    {
        "label": "M3 .sv no longer maps to verilog-mode -> full major-mode scan should go red",
        "file": "crates/core/lisp/modes.el",
        "old": "(add-to-list 'auto-mode-alist '(\"\\\\.sv\\\\'\" . verilog-mode))",
        "new": "(add-to-list 'auto-mode-alist '(\"\\\\.sv\\\\'\" . fundamental-mode))",
        "test": "every_file_under_demo_opens_in_its_expected_major_mode",
    },
    {
        "label": "M4 port candidate filter always empty -> u_regfile completion test should go red",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": "                              (lambda (p) (string-prefix-p typed (nth 0 p)))",
        "new": "                              (lambda (p) (ignore p) nil)",
        "test": "regfile_instantiation_offers_all_nine_port_names",
    },
    {
        "label": "M5 AUTOINST computes ports but does not insert them -> AUTOINST test should go red",
        "file": "crates/core/lisp/verilog-auto.el",
        "old": """        (when lines
          (goto-char (treesit-node-end comment))
          (insert "\\n" (string-join lines "\\n")))
        1))))""",
        "new": """        (ignore lines comment)
        1))))""",
        "test": "arbiter_autoinst_expands_then_deletes",
    },
]
