# Mutation list for M143 (the `.*` wildcard port connection).
# Run by the main conversation, never by the implementer.
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m143.py -p core
#
# M143 Part A fixes one defect: a SystemVerilog `.*' wildcard connection parses
# as a `named_port_connection' with NO `port_name' field, and three sites read
# that field unconditionally. `treesit-node-text' on nil signals
# `wrong-type-argument', which aborted the WHOLE `verilog-auto' command and left
# the buffer untouched -- 7 of 10 marker configurations reproduced it.
#
# Per M117's rule a deletion entry is required per FEATURE, not per file. The
# feature here is "a wildcard contributes no port name anywhere", and it is
# reachable from `cargo test' at every site, so all three deletion entries are
# real rather than declared:
#
#   W1  deletes the whole effect (the helper reverts to the naive mapcar)
#   W2  deletes it at the port-propagation site   (5 AUTO commands)
#   W3  deletes it at the AUTOWIRE site           (2 tests)
#   W4  isolates WHICH guard is load-bearing at the AUTOWIRE site
#
# The X block covers a SECOND, independent shape the cold review turned up:
# `.foo()', a legal explicit disconnect, where `connection' is nil but
# `port_name' is present. M143 fixed an older AUTOWIRE crash on it as a side
# effect. See the X block's own header.
#
# R1/R2/R3 are declared survivors. The cold review proved each of these three
# guards is redundant *given today's grammar*, and the main conversation kept
# them anyway as cheap insurance against the one black box the review could not
# probe read-only: whether tree-sitter's error recovery can synthesize a MISSING
# `port_identifier' mid-edit, which would be a non-nil node and slip past an
# `(and pn-node ...)' test. Without `expect`, every run of this list would print
# a `!!' line for an outcome that is by design -- which is exactly how an
# operator learns to ignore `!!' lines.
#
# Reading a SURVIVED result: confirm the mutation actually LANDED and actually
# changed semantics before reading it as a coverage gap. Two milestones in a row
# (M133, M134) hit a SURVIVED that was the mutation's own fault.

PACKAGE = "core"
TEST_TARGET = "verilog_auto_tests"

_EL = "crates/core/lisp/verilog-auto.el"
_ELC = "crates/core/lisp/verilog-complete.el"

MUTATIONS = [
    # ---- W: the wildcard guard, at each of the three sites ----------------
    {
        "label": "W1 DELETION: the helper reverts to the naive unguarded mapcar",
        "file": _EL,
        "old": """  (verilog-auto--filter
   #'identity
   (mapcar (lambda (c)
             (let ((pn (treesit-node-child-by-field-name c "port_name")))
               (and pn (treesit-node-text pn))))
           (verilog-auto--find-all-of-type hier "named_port_connection"))))""",
        "new": """  (mapcar (lambda (c) (treesit-node-text (treesit-node-child-by-field-name c "port_name")))
          (verilog-auto--find-all-of-type hier "named_port_connection")))""",
        "test": "autoinst_wildcard_present_matches_no_wildcard_control",
    },
    {
        "label": "W1b same deletion, seen by the test that pins the GNU rule",
        "file": _EL,
        "old": """  (verilog-auto--filter
   #'identity
   (mapcar (lambda (c)
             (let ((pn (treesit-node-child-by-field-name c "port_name")))
               (and pn (treesit-node-text pn))))
           (verilog-auto--find-all-of-type hier "named_port_connection"))))""",
        "new": """  (mapcar (lambda (c) (treesit-node-text (treesit-node-child-by-field-name c "port_name")))
          (verilog-auto--find-all-of-type hier "named_port_connection")))""",
        "test": "wildcard_pins_gnu_rule_explicit_connection_excluded_wildcard_left_in_place",
    },
    {
        "label": "W1c same deletion, seen by the delete/expand round trip",
        "file": _EL,
        "old": """  (verilog-auto--filter
   #'identity
   (mapcar (lambda (c)
             (let ((pn (treesit-node-child-by-field-name c "port_name")))
               (and pn (treesit-node-text pn))))
           (verilog-auto--find-all-of-type hier "named_port_connection"))))""",
        "new": """  (mapcar (lambda (c) (treesit-node-text (treesit-node-child-by-field-name c "port_name")))
          (verilog-auto--find-all-of-type hier "named_port_connection")))""",
        "test": "wildcard_survives_delete_expand_round_trip",
    },
    {
        # Feeds AUTOOUTPUT/AUTOINPUT/AUTOINOUT directly and AUTOREG/AUTOTIEOFF
        # indirectly via `verilog-auto--driven-output-names'. One revert, five
        # commands -- run it against each of the five named tests in turn.
        "label": "W2 DELETION: the port-propagation site reads port_name unguarded",
        "file": _EL,
        "old": """                  (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                         ;; M143: a `.*' wildcard connection is its own
                         ;; `named_port_connection' with NO `port_name' field
                         ;; -- `pn-node' is nil and this connection contributes
                         ;; no candidate at all (`.*' connects nothing, see
                         ;; `verilog-auto--explicitly-connected-port-names').
                         (pname (and pn-node (treesit-node-text pn-node)))
                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (and pname (assoc pname ports)))""",
        "new": """                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))""",
        "test": "autooutput_wildcard_present_matches_control",
    },
    {
        "label": "W2b same revert, seen by AUTOINPUT",
        "file": _EL,
        "old": """                  (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                         ;; M143: a `.*' wildcard connection is its own
                         ;; `named_port_connection' with NO `port_name' field
                         ;; -- `pn-node' is nil and this connection contributes
                         ;; no candidate at all (`.*' connects nothing, see
                         ;; `verilog-auto--explicitly-connected-port-names').
                         (pname (and pn-node (treesit-node-text pn-node)))
                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (and pname (assoc pname ports)))""",
        "new": """                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))""",
        "test": "autoinput_wildcard_present_matches_control",
    },
    {
        "label": "W2c same revert, seen by AUTOINOUT",
        "file": _EL,
        "old": """                  (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                         ;; M143: a `.*' wildcard connection is its own
                         ;; `named_port_connection' with NO `port_name' field
                         ;; -- `pn-node' is nil and this connection contributes
                         ;; no candidate at all (`.*' connects nothing, see
                         ;; `verilog-auto--explicitly-connected-port-names').
                         (pname (and pn-node (treesit-node-text pn-node)))
                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (and pname (assoc pname ports)))""",
        "new": """                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))""",
        "test": "autoinout_wildcard_present_matches_control",
    },
    {
        "label": "W2d same revert, seen by AUTOREG (indirect, via driven-output-names)",
        "file": _EL,
        "old": """                  (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                         ;; M143: a `.*' wildcard connection is its own
                         ;; `named_port_connection' with NO `port_name' field
                         ;; -- `pn-node' is nil and this connection contributes
                         ;; no candidate at all (`.*' connects nothing, see
                         ;; `verilog-auto--explicitly-connected-port-names').
                         (pname (and pn-node (treesit-node-text pn-node)))
                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (and pname (assoc pname ports)))""",
        "new": """                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))""",
        "test": "autoreg_wildcard_present_matches_control",
    },
    {
        "label": "W2e same revert, seen by AUTOTIEOFF (indirect)",
        "file": _EL,
        "old": """                  (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                         ;; M143: a `.*' wildcard connection is its own
                         ;; `named_port_connection' with NO `port_name' field
                         ;; -- `pn-node' is nil and this connection contributes
                         ;; no candidate at all (`.*' connects nothing, see
                         ;; `verilog-auto--explicitly-connected-port-names').
                         (pname (and pn-node (treesit-node-text pn-node)))
                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (and pname (assoc pname ports)))""",
        "new": """                  (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                         (cnode (treesit-node-child-by-field-name conn "connection"))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))
                         (pinfo (assoc pname ports))""",
        "test": "autotieoff_wildcard_present_matches_control",
    },
    {
        "label": "W3 DELETION: the AUTOWIRE site reads port_name and connection unguarded",
        "file": _EL,
        "old": """            (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                   (pname (and pn-node (treesit-node-text pn-node)))
                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (and cnode (string-trim (treesit-node-text cnode))))
                   (pinfo (and pname (assoc pname ports))))
              (when (and pinfo
                         ctext
                         (eq (nth 1 pinfo) 'output)""",
        "new": """            (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                   (cnode (treesit-node-child-by-field-name conn "connection"))
                   (ctext (string-trim (treesit-node-text cnode)))
                   (pinfo (assoc pname ports)))
              (when (and pinfo
                         (eq (nth 1 pinfo) 'output)""",
        "test": "autowire_wildcard_present_matches_control",
    },
    {
        "label": "W3b same revert, seen by the two-instances-one-statement test",
        "file": _EL,
        "old": """            (let* ((pn-node (treesit-node-child-by-field-name conn "port_name"))
                   (pname (and pn-node (treesit-node-text pn-node)))
                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (and cnode (string-trim (treesit-node-text cnode))))
                   (pinfo (and pname (assoc pname ports))))
              (when (and pinfo
                         ctext
                         (eq (nth 1 pinfo) 'output)""",
        "new": """            (let* ((pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                   (cnode (treesit-node-child-by-field-name conn "connection"))
                   (ctext (string-trim (treesit-node-text cnode)))
                   (pinfo (assoc pname ports)))
              (when (and pinfo
                         (eq (nth 1 pinfo) 'output)""",
        "test": "wildcard_in_one_of_two_instances_does_not_change_sibling_expansion",
    },
    {
        # Isolates which of the AUTOWIRE site's guards is load-bearing. Only
        # `pname' is restored to an unconditional read; cnode/ctext/pinfo/when
        # stay fixed. If this FAILs, the port_name guard is the one doing the
        # work -- which is what the review predicted, and what distinguishes
        # this site's fix from the redundant extras R1/R2 below.
        # The 19-space indentation is what makes this snippet unique to the
        # AUTOWIRE site; the port-propagation site is indented 25.
        "label": "W4 only the AUTOWIRE port_name guard is reverted (isolation probe)",
        "file": _EL,
        "old": """                   (pname (and pn-node (treesit-node-text pn-node)))
                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))""",
        "new": """                   (pname (treesit-node-text (treesit-node-child-by-field-name conn "port_name")))
                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))""",
        "test": "autowire_wildcard_present_matches_control",
    },
    # ---- R: declared survivors (redundant-by-grammar guards) --------------
    {
        # `verilog-auto--bare-identifier-p' is
        # `(and (> (length text) 0) (string-match-p ...))', and `(length nil)'
        # is 0 in Elisp, so it already returns nil safely for a nil argument.
        # The `ctext' term in the `when' therefore changes nothing observable
        # on its own. Kept as defence-in-depth, declared here so the SURVIVED
        # verdict is a fact that was executed rather than an assumption.
        "expect": "survived",
        "label": "R1 the redundant `ctext' term is dropped from the AUTOWIRE `when' (expected to SURVIVE)",
        "file": _EL,
        "old": """              (when (and pinfo
                         ctext
                         (eq (nth 1 pinfo) 'output)""",
        "new": """              (when (and pinfo
                         (eq (nth 1 pinfo) 'output)""",
        "test": "autowire_wildcard_present_matches_control",
    },
    {
        # Per tree-sitter-systemverilog 0.4.0, `named_port_connection' has
        # exactly two shapes: `.NAME(...)' (which always carries `port_name',
        # and only carries `connection' inside that same branch) and bare `.*'
        # (no fields at all). So `connection' can never be present while
        # `port_name' is absent -- gating cnode on pn-node is an indirection,
        # not a guard. Declared, not deleted: see this file's header on the
        # MISSING-node black box.
        #
        # The preceding `pname' line is carried along purely as an ANCHOR. The
        # cnode line alone is NOT unique: it is indented 19 spaces here and 25
        # at the port-propagation site, and the 19-space form is a substring of
        # the 25-space one, so it matched twice. Including the line above
        # anchors the match on a newline, which is what makes it unique -- the
        # same reason W4's two-line form is unique.
        "expect": "survived",
        "label": "R2 cnode stops being gated on pn-node at the AUTOWIRE site (expected to SURVIVE)",
        "file": _EL,
        "old": """                   (pname (and pn-node (treesit-node-text pn-node)))
                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))""",
        "new": """                   (pname (and pn-node (treesit-node-text pn-node)))
                   (cnode (treesit-node-child-by-field-name conn "connection"))""",
        "test": "autowire_wildcard_present_matches_control",
    },
    {
        # The helper's sole consumer is `(member (nth 0 p) connected)', and a
        # stray nil element is inert against `member' -- never `equal' to a real
        # port-name string. So the `#'identity' filter is cleanliness, not
        # correctness. `(progn (mapcar ...))' keeps the paren balance identical.
        "expect": "survived",
        "label": "R3 the helper stops filtering nils out of its result (expected to SURVIVE)",
        "file": _EL,
        "old": """  (verilog-auto--filter
   #'identity
   (mapcar (lambda (c)""",
        "new": """  (progn
   (mapcar (lambda (c)""",
        "test": "autoinst_wildcard_present_matches_no_wildcard_control",
    },
    # ---- X: the `.foo()` explicit-disconnect guards ----------------------
    #
    # `.foo()' is a legal SystemVerilog explicit disconnect: the port IS named,
    # so `port_name' is present, but the parens are empty so the `connection'
    # field is absent. It is the mirror image of `.*' and an INDEPENDENT second
    # shape in which a field can be nil.
    #
    # The cold review found it: before M143 the AUTOWIRE site read
    # `(string-trim (treesit-node-text cnode))' unconditionally, so a `.foo()'
    # anywhere in the buffer already crashed AUTOWIRE with the same
    # `wrong-type-argument', entirely independently of `.*'. M143's guards fixed
    # that as a side effect -- an unclaimed drive-by fix that had zero test
    # coverage until these tests were added.
    #
    # NOTE on anchors: each `old' below carries the preceding `cnode' line
    # purely as an anchor. The `ctext' line ALONE is not unique -- it appears at
    # both sites, indented 19 spaces at AUTOWIRE and 25 at port-propagation, and
    # the 19-space form is a substring of the 25-space one. The two-line form is
    # unique because the newline pins the indentation exactly.
    {
        "label": "X1 DELETION: the AUTOWIRE ctext guard is reverted (`.foo()' crashes again)",
        "file": _EL,
        "old": """                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (and cnode (string-trim (treesit-node-text cnode))))""",
        "new": """                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (string-trim (treesit-node-text cnode)))""",
        "test": "autowire_explicit_disconnect_present_matches_control",
    },
    {
        "label": "X1b same revert, seen by the combined `.*' + `.foo()' test",
        "file": _EL,
        "old": """                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (and cnode (string-trim (treesit-node-text cnode))))""",
        "new": """                   (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                   (ctext (string-trim (treesit-node-text cnode)))""",
        "test": "autowire_wildcard_and_explicit_disconnect_together",
    },
    {
        # This one mutates a guard that PREDATES M143 -- the port-propagation
        # site was already written `(and cnode ...)', which is why `.foo()'
        # never crashed AUTOOUTPUT the way it crashed AUTOWIRE. It is here
        # because the implementer that added the test verified X1 by direct
        # mutation but reported this site as reasoned-not-observed, and an
        # unverified deletion claim is exactly what this list exists to settle.
        "label": "X2 the pre-existing port-propagation ctext guard is reverted",
        "file": _EL,
        "old": """                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (and cnode (string-trim (treesit-node-text cnode))))""",
        "new": """                         (cnode (and pn-node (treesit-node-child-by-field-name conn "connection")))
                         (ctext (string-trim (treesit-node-text cnode)))""",
        "test": "autooutput_explicit_disconnect_present_matches_control",
    },
    # ---- B: Part B, port completion excludes already-connected ports ------
    #
    # Per M117's rule the unit is the FEATURE, not the file. Part B ships four
    # separable effects and each gets its own deletion-style entry, because a
    # single "turn exclusion off" mutation does NOT reach three of them:
    #
    #   B1   the exclusion itself
    #   B2   the exclusion is scoped to THIS `hierarchical_instance'
    #   B2c  ... and no wider than the instance either
    #   B3   the connection under the cursor is never excluded (self-skip)
    #   B4   a `.*' wildcard contributes no name (and must not crash)
    #   B5   the distinct "every port already connected" message
    #
    # Why B2/B2c exist at all: the implementer answered the deletion question
    # by disabling exclusion wholesale, and reported that
    # `two_instances_in_one_statement_do_not_leak_connections_between_them' and
    # `a_sibling_instance_elsewhere_in_the_module_does_not_narrow_this_one'
    # STAYED GREEN under it. That is the expected result for that particular
    # mutation -- exclude nothing and nothing can leak -- but it means the
    # scope rule those two tests exist to pin had not been shown to be pinned
    # by anything. B2 widens the scope to the whole `module_instantiation'
    # (sibling instance in the SAME statement) and B2c widens it to the whole
    # `module_declaration' (sibling instance elsewhere), which is what those
    # two tests actually claim to catch. If either SURVIVES, the test does not
    # reach what its name says, and that is a finding worth recording.
    {
        "label": "B1 DELETION: exclusion removed, every port offered again",
        "file": _ELC,
        "old": """             (remaining (verilog-auto--filter
                         (lambda (p) (not (member (nth 0 p) excluded)))
                         ports))""",
        "new": """             (remaining ports)""",
        "test": "already_connected_ports_absent_from_popup_with_empty_prefix",
        "test_target": "verilog_complete_tests",
    },
    {
        "label": "B1b same deletion, seen by the rewritten M54 test",
        "file": _ELC,
        "old": """             (remaining (verilog-auto--filter
                         (lambda (p) (not (member (nth 0 p) excluded)))
                         ports))""",
        "new": """             (remaining ports)""",
        "test": "bare_second_dot_after_comma_offers_all_ports",
        "test_target": "verilog_complete_tests",
    },
    {
        "label": "B1c same deletion, seen by the typed-prefix case",
        "file": _ELC,
        "old": """             (remaining (verilog-auto--filter
                         (lambda (p) (not (member (nth 0 p) excluded)))
                         ports))""",
        "new": """             (remaining ports)""",
        "test": "already_connected_ports_absent_from_popup_with_typed_prefix",
        "test_target": "verilog_complete_tests",
    },
    {
        # Scope widened from the `hierarchical_instance' to the enclosing
        # `module_instantiation'. `sram_bank u_a (.clk_i(c)), u_b (.);' is ONE
        # module_instantiation holding two hierarchical_instances, so this makes
        # u_a's connections leak into u_b's popup -- the M92 trap.
        "label": "B2 the exclusion set is scoped to the whole module_instantiation (named-connection branch)",
        "file": _ELC,
        "old": """                        (list mi hier parent))))))""",
        "new": """                        (list mi mi parent))))))""",
        "test": "two_instances_in_one_statement_named_shape_do_not_leak_connections",
        "test_target": "verilog_complete_tests",
    },
    {
        # `verilog-complete--port-context' has TWO branches that each build the
        # scope independently: the dot's parent is a `named_port_connection'
        # (B2 above), or it is an ERROR node for a bare `.' (this one). The
        # first run of this list had only B2, aimed at the first branch, and
        # pointed it at the BARE-DOT test -- which goes through the second
        # branch and therefore SURVIVED. The mutation landed and changed
        # semantics; it simply never ran on that test's path. Two branches need
        # two entries and two tests.
        "label": "B2b the exclusion set is scoped to the whole module_instantiation (bare-dot ERROR branch)",
        "file": _ELC,
        "old": """                        (list mi grandparent nil))))))""",
        "new": """                        (list mi mi nil))))))""",
        "test": "two_instances_in_one_statement_do_not_leak_connections_between_them",
        "test_target": "verilog_complete_tests",
    },
    {
        # Scope widened all the way to the enclosing module, so EVERY port
        # connection anywhere in the module joins the exclusion set.
        "label": "B2c the exclusion set is scoped to the whole module_declaration",
        "file": _ELC,
        "old": """           (let ((conns (verilog-auto--find-all-of-type hier "named_port_connection")))""",
        "new": """           (let ((conns (verilog-auto--find-all-of-type (verilog-auto--enclosing-of-type hier "module_declaration") "named_port_connection")))""",
        "test": "a_sibling_instance_elsewhere_in_the_module_does_not_narrow_this_one",
        "test_target": "verilog_complete_tests",
    },
    {
        # Self-skip removed: the connection the user is re-editing is excluded
        # from its own popup, so `.clk_i|(c)' stops offering `clk_i'.
        "label": "B3 DELETION: the self-skip is removed (a port vanishes from its own popup)",
        "file": _ELC,
        "old": """             (if self-conn
                 (verilog-auto--filter (lambda (c) (not (treesit-node-eq c self-conn))) conns)
               conns)""",
        "new": """             (if nil
                 (verilog-auto--filter (lambda (c) (not (treesit-node-eq c self-conn))) conns)
               conns)""",
        "test": "reediting_an_existing_connection_still_offers_its_own_port_name",
        "test_target": "verilog_complete_tests",
    },
    {
        # The `.*' guard in Part B's OWN walk (verilog-complete.el's copy, not
        # verilog-auto.el's). Without it, `treesit-node-text' gets nil and the
        # completion command signals on any instance containing a wildcard.
        "label": "B4 DELETION: the wildcard guard is removed from the exclusion walk",
        "file": _ELC,
        "old": """             (let ((pn (treesit-node-child-by-field-name c "port_name")))
               (and pn (treesit-node-text pn))))""",
        "new": """             (treesit-node-text (treesit-node-child-by-field-name c "port_name")))""",
        "test": "wildcard_connection_does_not_suppress_unconnected_ports",
        "test_target": "verilog_complete_tests",
    },
    {
        # The new branch's message text. Reverting it to the older wording makes
        # the two cases indistinguishable to the user again.
        "label": "B5 the distinct all-connected message reverts to the old wording",
        "file": _ELC,
        "old": """          (message "Verilog port completion: every port of `%s' is already connected on this instance" type-name))""",
        "new": """          (message "Verilog port completion: no port of `%s' starts with `%s'" type-name typed))""",
        "test": "every_port_already_connected_produces_a_distinct_message_naming_the_module",
        "test_target": "verilog_complete_tests",
    },
]
