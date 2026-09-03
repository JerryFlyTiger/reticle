# Mutation list for M99's merged-diagnostics path (`lsp--diagnostics-for-uri'
# and `lsp--decorate-buffer's gate).
#
#     PYTHONUNBUFFERED=1 dev/mutate.py --config dev/mutations/m99-merge.py \
#         -p core --test-target lsp_highlight_tests
#
# Designed by the reviewer that cold-read the M99 diff plus the fix round;
# run from the main conversation, because an implementer must not verify
# its own fix.
#
# M99 changed decoration from "paint the authoritative client's own publish"
# to "paint the union of every attached client's stored diagnostics", because
# the two Verilog servers this project talks to are complementary rather than
# redundant (verible: style lint; slang: elaboration). Each mutation below
# breaks exactly one link in that chain and names the test that must go red.
#
# M4 is the one worth reading twice: it restores the *first* draft's dedup,
# which keyed on (line, character, severity, message) and so silently dropped
# a second diagnostic that started at the same place with the same wording but
# covered a different range. That defect shipped through the first
# implementation round and was caught only by the cold read.

PACKAGE = "core"
TEST_TARGET = "lsp_highlight_tests"

MUTATIONS = [
    {
        "label": "M1 the union collapses back to the publishing client alone",
        "file": "crates/core/lisp/lsp.el",
        "old": "    (let ((raw-clients (lsp--effective-buffer-clients)))",
        "new": "    (let ((raw-clients (list client)))",
        "test": "merge_diagnostics_default_paints_the_union_of_two_attached_clients",
    },
    {
        "label": "M2 the M94 authority gate is re-armed even in merge mode",
        "file": "crates/core/lisp/lsp.el",
        "old": "          (when (or lsp-merge-diagnostics-from-all-clients\n                    (lsp--diagnostics-authoritative-p client))",
        "new": "          (when (and lsp-merge-diagnostics-from-all-clients\n                     (lsp--diagnostics-authoritative-p client))",
        "test": "merge_diagnostics_default_paints_the_union_of_two_attached_clients",
    },
    {
        "label": "M3 client-level dedup removed (a client listed twice paints twice)",
        "file": "crates/core/lisp/lsp.el",
        "old": "        (dolist (c raw-clients)\n          (unless (memq c clients)\n            (push c clients)))",
        "new": "        (dolist (c raw-clients)\n          (push c clients))",
        "test": "merge_diagnostics_does_not_double_paint_a_client_listed_twice",
    },
    {
        "label": "M4 the pre-fix content-keyed dedup is restored (drops a real diagnostic)",
        "file": "crates/core/lisp/lsp.el",
        "old": (
            "        (let ((out nil))\n"
            "          (dolist (c clients)\n"
            "            (let ((diags (cdr (assoc uri (lsp--client-diagnostics c)))))\n"
            "              (when diags\n"
            "                (let ((n (length diags)) (i 0))\n"
            "                  (while (< i n)\n"
            "                    (push (aref diags i) out)\n"
            "                    (setq i (1+ i)))))))\n"
            "          (nreverse out))"
        ),
        "new": (
            "        (let ((out nil) (seen nil))\n"
            "          (dolist (c clients)\n"
            "            (let ((diags (cdr (assoc uri (lsp--client-diagnostics c)))))\n"
            "              (when diags\n"
            "                (let ((n (length diags)) (i 0))\n"
            "                  (while (< i n)\n"
            "                    (let* ((d (aref diags i))\n"
            "                           (start (gethash \"start\" (gethash \"range\" d)))\n"
            "                           (key (list (gethash \"line\" start)\n"
            "                                      (gethash \"character\" start)\n"
            "                                      (or (gethash \"severity\" d) 1)\n"
            "                                      (or (gethash \"message\" d) \"\"))))\n"
            "                      (unless (member key seen)\n"
            "                        (push key seen)\n"
            "                        (push d out)))\n"
            "                    (setq i (1+ i)))))))\n"
            "          (nreverse out))"
        ),
        "test": "same_client_two_diagnostics_sharing_start_severity_and_message_are_both_painted",
    },
    {
        "label": "M5 the nil (M94-compatible) branch stops returning anything",
        "file": "crates/core/lisp/lsp.el",
        "old": "      (let ((diags (cdr (assoc uri (lsp--client-diagnostics client))))\n            (out nil))",
        "new": "      (let ((diags nil)\n            (out nil))",
        "test": "merge_diagnostics_set_to_nil_restores_authoritative_only_painting",
    },
]
