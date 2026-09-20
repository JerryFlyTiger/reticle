# Mutation list for M144 (one port-name accessor; orphaned /*AUTOINST*/ markers).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m144.py -p core
#
# M144 ships these effects, each with a deletion entry below:
#
#   A   `verilog-auto--connection-port-name': a connection with no port name
#       (`.*' has no field; a bare `.' gets a zero-width MISSING node with
#       text "") names nothing. A1 = absent-field check, A2 = empty-text
#       check, A3 = the whole accessor.
#   X   EXCLUDE-NODE on `verilog-auto--explicitly-connected-port-names', and
#       completion calling it with the connection under the cursor (X1, X2).
#       The completion-side duplicate was deleted, so there is no second site.
#   H   `verilog-auto--expand-autoinst-site' skips a marker with no
#       `hierarchical_instance' ancestor instead of signalling (H1).
#   O   Orphaned markers (a `module_instantiation' ancestor, no
#       `hierarchical_instance' ancestor -- tree-sitter's recovery from a
#       user-typed `.(sig),'): classified (O1), kept off the M129
#       text-fallback deletion (O2), counted in the 5th return element (O3),
#       reported by `verilog-delete-auto' (O4) and by `verilog-auto' (O5).
#   R   `verilog-auto--unreachable-autoinst-markers' collects reachable
#       markers per `hierarchical_instance', not per `module_instantiation'
#       (R1).
#
# Not listed: the `.*|' completion test (Part C) and the label-based rewrite
# of `instantiate_item_for_a_real_demo_rtl_module_with_parameters_and_many_ports'
# (Part B) are test-only; the behaviour `.*|' pins predates M144. The round-3
# reviewer noted `.*|' is blocked by two independent checks in
# `verilog-complete--port-context', so a one-line mutation there survives.
#
# Reading a SURVIVED result: confirm the mutation actually LANDED and actually
# changed semantics before reading it as a coverage gap.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_EL = "crates/core/lisp/verilog-auto.el"
_ELC = "crates/core/lisp/verilog-complete.el"

MUTATIONS = [
    # ---- A: the accessor ----------------------------------------------------
    {
        "label": "A1 accessor reads port_name text without checking the field exists",
        "file": _EL,
        "old": "         (text (and pn (treesit-node-text pn))))",
        "new": "         (text (treesit-node-text pn)))",
        "test": "wildcard_pins_gnu_rule_explicit_connection_excluded_wildcard_left_in_place",
    },
    {
        "label": "A2 accessor returns the empty MISSING-node text instead of nil",
        "file": _EL,
        "old": "    (and text (> (length text) 0) text)))",
        "new": "    text))",
        "test": "explicitly_connected_port_names_skips_missing_port_name_and_honours_exclude_node",
    },
    {
        "label": "A3 DELETION: the accessor never names any port",
        "file": _EL,
        "old": """  (let* ((pn (treesit-node-child-by-field-name conn "port_name"))
         (text (and pn (treesit-node-text pn))))
    (and text (> (length text) 0) text)))""",
        "new": """  (ignore conn)
  nil)""",
        "test": "explicitly_connected_port_names_skips_missing_port_name_and_honours_exclude_node",
    },
    # ---- X: EXCLUDE-NODE ----------------------------------------------------
    {
        "label": "X1 DELETION: EXCLUDE-NODE is ignored",
        "file": _EL,
        "old": """             (if exclude-node
                 (verilog-auto--filter
                  (lambda (c) (not (treesit-node-eq c exclude-node))) conns)
               conns)))))""",
        "new": """             conns))))""",
        "test": "explicitly_connected_port_names_skips_missing_port_name_and_honours_exclude_node",
    },
    {
        "label": "X2 completion stops passing the connection under the cursor",
        "file": _ELC,
        "old": "(excluded (verilog-auto--explicitly-connected-port-names hier self-conn))",
        "new": "(excluded (verilog-auto--explicitly-connected-port-names hier))",
        "test_target": "verilog_complete_tests",
        "test": "reediting_an_existing_connection_still_offers_its_own_port_name",
    },
    # ---- H: no abort on a marker outside hierarchical_instance -------------
    {
        "label": "H1 DELETION: expand-autoinst-site no longer skips a nil hier",
        "file": _EL,
        "old": "            (not hier))",
        "new": "            nil)",
        "test": "autoinst_marker_pushed_outside_hierarchical_instance_does_not_abort_verilog_auto",
    },
    # ---- O: orphaned markers ------------------------------------------------
    {
        "label": "O1 DELETION: no marker is ever classified orphaned",
        "file": _EL,
        "old": """  (verilog-auto--filter
   (lambda (c)
     (and (verilog-auto--enclosing-of-type c "module_instantiation")
          (not (verilog-auto--enclosing-of-type c "hierarchical_instance"))))
   (verilog-auto--find-comments root "/*AUTOINST*/")))""",
        "new": """  (ignore root)
  nil)""",
        "test": "autoinst_marker_second_run_does_not_lose_generated_connections",
    },
    {
        "label": "O2 orphaned markers are no longer subtracted from the unreachable set",
        "file": _EL,
        "old": """     (lambda (c) (and (not (member (treesit-node-start c) reachable-starts))
                       (not (member (treesit-node-start c) orphaned-starts))))""",
        "new": """     (lambda (c) (not (member (treesit-node-start c) reachable-starts)))""",
        "test": "second_run",
    },
    {
        "label": "O3 the 5th return element no longer counts orphaned markers",
        "file": _EL,
        "old": "text-recovered (length orphaned)))))",
        "new": "text-recovered 0)))))",
        "test": "delete_auto_on_the_step2_buffer_leaves_the_malformed_site_untouched",
    },
    {
        "label": "O4 DELETION: verilog-delete-auto does not report orphaned markers",
        "file": _EL,
        "old": """      (when orphaned
        (message "verilog-delete-auto: %d /*AUTOINST*/ marker(s) sit past a malformed connection in the same instantiation, left untouched (not deleted, not regenerated)"
                  (length orphaned)))""",
        "new": """      (ignore orphaned)""",
        "test": "delete_auto_on_the_step2_buffer_leaves_the_malformed_site_untouched",
    },
    {
        "label": "O5 DELETION: verilog-auto does not report orphaned markers",
        "file": _EL,
        "old": "(if (> orphaned-connection-autoinst 0)",
        "new": "(if nil",
        "test": "autoinst_marker_second_run_does_not_lose_generated_connections",
    },
    # ---- R: reachable markers per hierarchical_instance --------------------
    {
        "label": "R1 reachable markers collected per module_instantiation again",
        "file": _EL,
        "old": """             (dolist (hier (verilog-auto--find-all-of-type mi "hierarchical_instance"))
               (let ((c (verilog-auto--find-comment hier "/*AUTOINST*/")))
                 (when c (push (treesit-node-start c) acc)))))""",
        "new": """             (let ((c (verilog-auto--find-comment mi "/*AUTOINST*/")))
               (when c (push (treesit-node-start c) acc))))""",
        "test": "multi_instance_statement_both_markers_well_formed_neither_misclassified",
    },
]
