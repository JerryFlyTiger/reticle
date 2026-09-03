# M91 (module-name and parameter-name completion for Verilog) mutation list.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m91-verilog-complete.py \
#         -p core --test-target verilog_complete_tests
#
# R1 spans two guards on purpose. The fall-through when nothing matches is
# defended twice over -- an outer existence check and an inner empty-candidate
# check -- and EACH alone produces the fall-through, so mutating either one on
# its own SURVIVES while the other quietly does the job. That is defence in
# depth working, not missing coverage, but it means the only honest mutation
# for this behaviour disables both in one edit. R5 records the outer guard
# separately as a deliberate survivor, since its real job is cost, not result.
#
# History worth keeping next to this file: the first fix round added a
# keyword-prefix veto to stop the detector firing at every statement start. The
# trailing cold read then showed the veto also blocked real modules (`re' never
# offering `reset_ctrl'), and that no test could tell the veto from its own
# absence, because every fixture lacked a matching module and the second
# measure masked it. The veto is gone; the match check decides.

PACKAGE = "core"
TEST_TARGET = "verilog_complete_tests"

MUTATIONS = [
    {
        "label": "R1 both fall-through guards are neutralised at once",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": (
            "    (when (verilog-complete--any-module-name-matches-p typed)\n"
            "      (let* ((all (verilog-complete--all-modules))\n"
            "             (candidates (verilog-auto--filter\n"
            "                          (lambda (e) (string-prefix-p typed (car e)))\n"
            "                          all))\n"
            "             (items (mapcar (lambda (e) (verilog-complete--module-item e prefix-start))\n"
            "                             candidates)))\n"
            "        (when items"
        ),
        "new": (
            "    (when t\n"
            "      (let* ((all (verilog-complete--all-modules))\n"
            "             (candidates (verilog-auto--filter\n"
            "                          (lambda (e) (string-prefix-p typed (car e)))\n"
            "                          all))\n"
            "             (items (mapcar (lambda (e) (verilog-complete--module-item e prefix-start))\n"
            "                             candidates)))\n"
            "        (when t"
        ),
        "test": "module_name_no_matching_prefix_falls_through_to_dabbrev",
    },
    {
        "label": "R2 the child-count guard is removed (a second bare identifier is treated as a type name)",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": "                 (= (treesit-node-child-count parent) 1))",
        "new": "                 t)",
        "test": "instantiation_type_context_child_count_guard_excludes_a_second_bare_identifier",
    },
    {
        "label": "R3 the non-empty-prefix guard is removed (an empty prefix matches every module)",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": "  (when (> point prefix-start)",
        "new": "  (when t",
        "test": "instantiation_type_context_returns_nil_for_an_empty_prefix",
    },
    {
        "label": "R4 the nested-ERROR unwrap stops recursing (a second broken override loses its type name)",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": '   ((string= (treesit-node-type node) "ERROR")',
        "new": '   ((string= (treesit-node-type node) "ERROR-disabled")',
        "test": "parameter_completion_recovers_the_type_name_when_two_broken_overrides_stack",
    },
    {
        "label": "R5 (expected SURVIVED, recorded) the cheap pre-check that keeps the keystroke fast",
        "file": "crates/core/lisp/verilog-complete.el",
        "old": "    (when (verilog-complete--any-module-name-matches-p typed)",
        "new": "    (when t",
        "test": "module_name_no_matching_prefix_falls_through_to_dabbrev",
        "note": (
            "Deliberate survivor. This gate exists so the common no-match "
            "keystroke does not pay for the full candidate build -- measured "
            "5-6ms with it against 13.8ms without. Removing it changes no "
            "result, only cost, and this project does not gate on timing, so "
            "no test can observe it. Recorded rather than omitted, so the "
            "line does not look covered."
        ),
    },
]
