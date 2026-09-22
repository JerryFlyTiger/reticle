# M150 -- AUTO directives sharing a line (part 1) and AUTOOUTPUT/AUTOINPUT/
# AUTOINOUT inside a module port list (part 2).
#
# Designed from three cold-review rounds' checklists; anchors were cut
# programmatically from the reviewed tree and each checked to occur exactly
# once. Executed by the main conversation:
#
#     python3 dev/mutate.py --config dev/mutations/m150.py
#
# Per-FEATURE deletion entries (M117 rule): A1 (Defect A, line prefix),
# D1-D6 (Defect D, one per call site -- each site is independent, so one
# entry could not see a site that lost its call), P1 (the port-list comma
# form as a whole), P5 (the non-ANSI refusal), P6 (the comma terminator).
#
# P8 below: the port-list paren picker choosing the FIRST top-level pair
# instead of the last -- added after the fourth review round.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_A = "crates/core/lisp/verilog-auto.el"

MUTATIONS = [
    {
        "label": "A1 DELETION line-indent copies the whole prefix again (Defect A undone)",
        "file": _A,
        "old": '(if idx (substring text 0 idx) text)',
        "new": 'text',
        "test": "autowire_directive_after_code_on_same_line_does_not_copy_the_code",
    },
    {
        "label": "D1 DELETION trailing-text split removed at the port-propagation site",
        "file": _A,
        "old": '          sorted "")\n         "\\n" indent "// End of automatics")\n        (verilog-auto--split-trailing-directive-text indent))',
        "new": '          sorted "")\n         "\\n" indent "// End of automatics")\n        nil)',
        "test": "autooutput_body_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D2 DELETION trailing-text split removed at the AUTOWIRE site",
        "file": _A,
        "old": '         "\\n" indent "// End of automatics")\n        (verilog-auto--split-trailing-directive-text indent)))',
        "new": '         "\\n" indent "// End of automatics")\n        nil))',
        "test": "autowire_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D3 DELETION trailing-text split removed at the AUTOTIEOFF site",
        "file": _A,
        "old": '              resolved "")\n             "\\n" indent "// End of automatics")\n            (verilog-auto--split-trailing-directive-text indent)))',
        "new": '              resolved "")\n             "\\n" indent "// End of automatics")\n            nil))',
        "test": "autotieoff_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D4 DELETION trailing-text split removed at the AUTOREG site",
        "file": _A,
        "old": '                  candidates "")\n                 "\\n" indent "// End of automatics")\n                (verilog-auto--split-trailing-directive-text indent)))',
        "new": '                  candidates "")\n                 "\\n" indent "// End of automatics")\n                nil))',
        "test": "autoreg_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D5 DELETION trailing-text split removed at the AUTORESET site",
        "file": _A,
        "old": '                  resolved "")\n                 "\\n" indent "// End of automatics")\n                (verilog-auto--split-trailing-directive-text indent)))',
        "new": '                  resolved "")\n                 "\\n" indent "// End of automatics")\n                nil))',
        "test": "autoreset_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D6 DELETION trailing-text split removed at the AUTOUNUSED site",
        "file": _A,
        "old": '             (mapconcat (lambda (nm) (concat "\\n" indent nm ",")) unread "")\n             "\\n" indent "// End of automatics")\n            (verilog-auto--split-trailing-directive-text indent)))',
        "new": '             (mapconcat (lambda (nm) (concat "\\n" indent nm ",")) unread "")\n             "\\n" indent "// End of automatics")\n            nil))',
        "test": "autounused_code_after_directive_on_same_line_survives",
    },
    {
        "label": "D7 CRLF: a lone \\r counts as trailing content again",
        "file": _A,
        "old": '(string-match "[^ \\t\\r]" text)',
        "new": '(string-match "[^ \\t]" text)',
        "test": "autowire_crlf_directive_alone_on_its_line_is_not_spuriously_split",
    },
    {
        "label": "D8 split helper always splits (no-op guard removed)",
        "file": _A,
        "old": '(when idx\n      (delete-region (point) (+ (point) idx))',
        "new": '(progn\n      (delete-region (point) (+ (point) (or idx 0)))',
        "test": "auto_directive_alone_on_its_line_is_byte_identical_to_before",
    },
    {
        "label": "P1 DELETION port-list branch never taken (comma form undone wholesale)",
        "file": _A,
        "old": '(in-port-list (verilog-auto--comment-in-header-port-list-p header comment)))',
        "new": '(in-port-list nil))',
        "test": "autooutput_ansi_port_list_last_before_close_paren_omits_trailing_comma",
    },
    {
        "label": "P2 close repair removed (last decl before ) keeps its comma)",
        "file": _A,
        "old": '((and close-needed (= i n)) "")',
        "new": '((and nil (= i n)) "")',
        "test": "autooutput_ansi_port_list_last_before_close_paren_omits_trailing_comma",
    },
    {
        "label": "P3 close repair on the nothing-inserted path removed",
        "file": _A,
        "old": '(when (and comma-p (not sorted) close-needed prev (eq (char-after prev) ?\\,))',
        "new": '(when (and nil (not sorted) close-needed prev (eq (char-after prev) ?\\,))',
        "test": "autooutput_close_repair_removes_dangling_comma_when_nothing_is_inserted",
    },
    {
        "label": "P4 open repair removed",
        "file": _A,
        "old": "(when (and comma-p prev (not (memq (char-after prev) '(?\\( ?\\,))))",
        "new": "(when (and nil prev (not (memq (char-after prev) '(?\\( ?\\,))))",
        "test": "autooutput_ansi_port_list_open_repair_adds_comma_to_previous_decl",
    },
    {
        "label": "P4b open repair removed, seen from the OUTPUT-then-INPUT order",
        "file": _A,
        "old": "(when (and comma-p prev (not (memq (char-after prev) '(?\\( ?\\,))))",
        "new": "(when (and nil prev (not (memq (char-after prev) '(?\\( ?\\,))))",
        "test": "autooutput_then_autoinput_output_then_input_order_also_repairs_correctly",
    },
    {
        "label": "P5 DELETION non-ANSI refusal removed (bare names never detected)",
        "file": _A,
        "old": '(and (verilog-auto--find-all-of-type header "port") t))',
        "new": 'nil)',
        "test": "autooutput_non_ansi_port_list_with_bare_name_is_refused_and_untouched",
    },
    {
        "label": "P6 DELETION comma terminator never used (always ;)",
        "file": _A,
        "old": 'name (or term ";")))',
        "new": 'name ";"))',
        "test": "autooutput_ansi_port_list_middle_both_lines_keep_comma",
    },
    {
        "label": "P7 open-repair comma no longer preserves the comment column",
        "file": _A,
        "old": '(when (looking-at "  +//")',
        "new": '(when nil',
        "test": "autooutput_then_autoinput_output_then_input_order_also_repairs_correctly",
    },
    {
        "label": "P8 port-list paren picker takes the FIRST top-level pair (the #(...) parameter list)",
        "file": _A,
        "old": '      (let ((last (car (last sorted))))',
        "new": '      (let ((last (car sorted)))',
        "test": "autooutput_ansi_port_list_after_parameter_list_with_nested_parens",
    },
]
