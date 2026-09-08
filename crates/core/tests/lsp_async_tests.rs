//! M15 item 3: async LSP dispatch logic. Deterministic — fabricated
//! JSON-RPC messages are fed straight to `lsp--dispatch`, no external
//! server involved (the ignored test in lsp_tests.rs covers the real
//! rust-analyzer round trip).

use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> Interp {
    let mut interp = elisp::new_interp();
    core::init_editor(&mut interp);
    interp
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

#[test]
fn async_callback_fires_on_matching_response_and_is_removed() {
    let mut i = setup();
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(&mut i, "(setq got nil)");
    // Register the callback by hand (lsp-request-async would try to
    // send over the nil connection; registration + delivery is the
    // logic under test).
    run(
        &mut i,
        "(setf (lsp--client-callbacks client)
               (list (cons 7 (lambda (result) (setq got result)))))",
    );
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string \"{\\\"id\\\":7,\\\"result\\\":42}\"))",
    );
    assert_eq!(run(&mut i, "got"), "42");
    assert_eq!(run(&mut i, "(lsp--client-callbacks client)"), "nil");
    // And it did NOT get stashed for the sync path too.
    assert_eq!(run(&mut i, "(lsp--client-pending client)"), "nil");
}

#[test]
fn response_without_callback_is_stashed_for_the_sync_path() {
    let mut i = setup();
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string \"{\\\"id\\\":3,\\\"result\\\":\\\"x\\\"}\"))",
    );
    assert_eq!(run(&mut i, "(length (lsp--client-pending client))"), "1");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"result\" (cdr (assq 3 (lsp--client-pending client))))"
        ),
        "\"x\""
    );
}

#[test]
fn mismatched_id_does_not_fire_the_callback() {
    let mut i = setup();
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(&mut i, "(setq got 'untouched)");
    run(
        &mut i,
        "(setf (lsp--client-callbacks client)
               (list (cons 7 (lambda (r) (setq got r)))))",
    );
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string \"{\\\"id\\\":8,\\\"result\\\":1}\"))",
    );
    assert_eq!(run(&mut i, "got"), "untouched");
    assert_eq!(run(&mut i, "(length (lsp--client-callbacks client))"), "1");
}

#[test]
fn publish_diagnostics_notification_updates_diagnostics() {
    let mut i = setup();
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"method\\\":\\\"textDocument/publishDiagnostics\\\",\\\"params\\\":{\\\"uri\\\":\\\"file:///tmp/x.rs\\\",\\\"diagnostics\\\":[{\\\"message\\\":\\\"bad\\\"}]}}\"))",
    );
    assert_eq!(
        run(&mut i, "(length (lsp-diagnostics client \"/tmp/x.rs\"))"),
        "1"
    );
    assert_eq!(
        run(
            &mut i,
            "(gethash \"message\" (aref (lsp-diagnostics client \"/tmp/x.rs\") 0))"
        ),
        "\"bad\""
    );
}

#[test]
fn hover_text_extraction_handles_all_shapes() {
    let mut i = setup();
    assert_eq!(
        run(&mut i, "(lsp--hover-text (json-parse-string \"{\\\"contents\\\":{\\\"kind\\\":\\\"markdown\\\",\\\"value\\\":\\\"fn add\\\"}}\"))"),
        "\"fn add\""
    );
    assert_eq!(
        run(
            &mut i,
            "(lsp--hover-text (json-parse-string \"{\\\"contents\\\":\\\"plain\\\"}\"))"
        ),
        "\"plain\""
    );
    assert_eq!(run(&mut i, "(lsp--hover-text :null)"), "nil");
}

#[test]
fn idle_tick_is_safe_with_no_clients_and_pumps_are_reentrant() {
    let mut i = setup();
    // No clients: must be a cheap no-op, never an error.
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    core::idle_tick(&mut i, std::time::Duration::from_secs(3));
    assert!(!core::has_async_work(&mut i));
    // A registered (dead-conn) client is pruned by the pump rather than
    // looping forever on a dead handle... conn nil is not live.
    run(
        &mut i,
        "(setq lsp--clients (list (make-lsp--client :conn nil)))",
    );
    assert!(core::has_async_work(&mut i));
    let r = run(&mut i, "(lsp-process-pending-all)");
    assert!(!r.starts_with("ERROR"), "pump errored: {}", r);
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert!(!core::has_async_work(&mut i));
}

/// M46 Part F: a JSON-RPC error response must surface via `message`
/// (previously indistinguishable from "succeeded with an empty
/// result", so a rejected request produced no user-visible feedback at
/// all), while the callback/pending contract is unchanged -- the
/// callback still gets nil, since an error response has no `result`.
/// `message` is shadowed here to capture what `lsp--dispatch` sends it,
/// rather than adding any new test-only capture hook to the Interp.
#[test]
fn error_response_messages_and_still_delivers_nil_to_the_callback() {
    let mut i = setup();
    run(
        &mut i,
        "(setq lsp--test-messages nil)
         (defun message (fmt &rest args) (push (apply 'format fmt args) lsp--test-messages) fmt)",
    );
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(&mut i, "(setq got 'unset)");
    run(
        &mut i,
        "(setf (lsp--client-callbacks client)
               (list (cons 9 (lambda (result) (setq got result)))))",
    );
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"id\\\":9,\\\"error\\\":{\\\"code\\\":-32601,\\\"message\\\":\\\"method not found\\\"}}\"))",
    );
    assert_eq!(run(&mut i, "got"), "nil");
    assert_eq!(run(&mut i, "(lsp--client-callbacks client)"), "nil");
    assert_eq!(
        run(&mut i, "(car lsp--test-messages)"),
        "\"lsp: method not found\""
    );
}

/// M46 Part F, the pending (no-callback) path: an error response with
/// no registered callback still gets stashed for `lsp--await`
/// (`result` reads as nil from it, same as before) AND still messages.
#[test]
fn error_response_with_no_callback_is_still_messaged_and_stashed() {
    let mut i = setup();
    run(
        &mut i,
        "(setq lsp--test-messages nil)
         (defun message (fmt &rest args) (push (apply 'format fmt args) lsp--test-messages) fmt)",
    );
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"id\\\":4,\\\"error\\\":{\\\"code\\\":-32601,\\\"message\\\":\\\"nope\\\"}}\"))",
    );
    assert_eq!(run(&mut i, "(length (lsp--client-pending client))"), "1");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"result\" (cdr (assq 4 (lsp--client-pending client))))"
        ),
        "nil"
    );
    assert_eq!(run(&mut i, "(car lsp--test-messages)"), "\"lsp: nope\"");
}

/// M46 Part E: `lsp--document-symbol-positions` recursively flattens a
/// nested `DocumentSymbol[]` response (the real shape
/// `verible-verilog-ls` returns, per the M46 spec's captured payload --
/// a top-level `module` symbol with `logic`/`function` children) into
/// an ascending-by-position list covering every level, not just the
/// top-level array.
#[test]
fn document_symbol_flattens_nested_children_in_position_order() {
    let mut i = setup();
    run(
        &mut i,
        "(insert \"module top;\\n  logic [3:0] a;\\n  function int f(int x); return x+1; endfunction\\nendmodule\\n\")",
    );
    // selectionRange.start for each symbol, hand-computed against the
    // buffer text above: `top` at line 0 col 7, `a` at line 1 col 14,
    // `f` at line 2 col 15 (all ASCII, so UTF-16 offsets equal char
    // offsets here).
    let payload = "[{\"name\":\"top\",\"kind\":6,\
                       \"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":3,\"character\":9}},\
                       \"selectionRange\":{\"start\":{\"line\":0,\"character\":7},\"end\":{\"line\":0,\"character\":10}},\
                       \"children\":[\
                         {\"name\":\"a\",\"kind\":13,\
                          \"range\":{\"start\":{\"line\":1,\"character\":2},\"end\":{\"line\":1,\"character\":16}},\
                          \"selectionRange\":{\"start\":{\"line\":1,\"character\":14},\"end\":{\"line\":1,\"character\":15}}},\
                         {\"name\":\"f\",\"kind\":12,\
                          \"range\":{\"start\":{\"line\":2,\"character\":2},\"end\":{\"line\":2,\"character\":47}},\
                          \"selectionRange\":{\"start\":{\"line\":2,\"character\":15},\"end\":{\"line\":2,\"character\":16}}}\
                       ]}]";
    run(
        &mut i,
        &format!("(setq syms (lsp--document-symbol-positions (json-parse-string {payload:?})))"),
    );
    assert_eq!(run(&mut i, "(length syms)"), "3");
    assert_eq!(run(&mut i, "(nth 1 (nth 0 syms))"), "\"top\"");
    assert_eq!(run(&mut i, "(nth 1 (nth 1 syms))"), "\"a\"");
    assert_eq!(run(&mut i, "(nth 1 (nth 2 syms))"), "\"f\"");
    // Strictly ascending positions (the sort actually did something,
    // not just happened to preserve tree order).
    assert_eq!(
        run(
            &mut i,
            "(and (< (nth 0 (nth 0 syms)) (nth 0 (nth 1 syms)))
                  (< (nth 0 (nth 1 syms)) (nth 0 (nth 2 syms))))"
        ),
        "t"
    );
}

/// M47 fix round: `lsp--symbol-alist` disambiguates same-named symbols
/// (overloaded functions, same-named members on different classes, ...)
/// with a " (N)" suffix so every DISPLAY string maps back to exactly
/// one POS -- without this, `completing-read` REQUIRE-MATCH would still
/// let the user pick a name, but `assoc' could only ever return the
/// first of several matching positions. Pure function: fed a
/// hand-built (POS NAME KIND) list directly, no JSON round trip needed.
#[test]
fn symbol_alist_disambiguates_repeated_names_and_each_resolves_to_its_own_position() {
    let mut i = setup();
    run(
        &mut i,
        "(setq syms (list (list 10 \"foo\" 12) (list 20 \"foo\" 12) (list 30 \"bar\" 6)))",
    );
    run(&mut i, "(setq alist (lsp--symbol-alist syms))");
    // Two "foo" entries get distinct, disambiguated display names; the
    // lone "bar" is left alone.
    assert_eq!(
        run(&mut i, "(mapcar #'car alist)"),
        "(\"foo (1)\" \"foo (2)\" \"bar\")"
    );
    // Every DISPLAY is unique (no accidental collision after
    // disambiguation) -- the exact-list check above already pins down
    // all three names, so this is a direct pairwise check rather than
    // relying on a dedup helper this build doesn't provide.
    assert_eq!(
        run(
            &mut i,
            "(let ((names (mapcar #'car alist)))
               (and (not (equal (nth 0 names) (nth 1 names)))
                    (not (equal (nth 0 names) (nth 2 names)))
                    (not (equal (nth 1 names) (nth 2 names)))))"
        ),
        "t"
    );
    // Each disambiguated name resolves back to the position it came
    // from, not just the first "foo".
    assert_eq!(run(&mut i, "(cdr (assoc \"foo (1)\" alist))"), "10");
    assert_eq!(run(&mut i, "(cdr (assoc \"foo (2)\" alist))"), "20");
    assert_eq!(run(&mut i, "(cdr (assoc \"bar\" alist))"), "30");
}

/// M123 Part A: a message carrying BOTH `id` and `method` is a
/// server-initiated REQUEST (slang-server's real
/// `client/registerCapability`, id 0, per the spec's own measurement),
/// and must be answered inline with `{"result": null}` rather than
/// stashed into `lsp--client-pending` (the old defect: it would have
/// sat there forever, since only `lsp--await` ever removes an entry,
/// and only for an id THIS client itself allocated). `lsp-send` is
/// shadowed to capture what would have gone over the wire, same
/// convention as `message` above -- CLIENT's `conn` is nil here so the
/// real builtin would error trying to write to it.
#[test]
fn server_request_with_registered_method_is_answered_null_and_never_stashed() {
    let mut i = setup();
    run(
        &mut i,
        "(setq lsp--test-sent nil)
         (defun lsp-send (conn msg) (push msg lsp--test-sent) t)",
    );
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"id\\\":0,\\\"method\\\":\\\"client/registerCapability\\\",\\\"params\\\":{}}\"))",
    );
    // Never stashed: the whole point of distinguishing REQUEST from
    // RESPONSE is that a request never reaches `lsp--client-pending`.
    assert_eq!(run(&mut i, "(lsp--client-pending client)"), "nil");
    assert_eq!(run(&mut i, "(length lsp--test-sent)"), "1");
    assert_eq!(
        run(&mut i, "(gethash \"jsonrpc\" (car lsp--test-sent))"),
        "\"2.0\""
    );
    assert_eq!(run(&mut i, "(gethash \"id\" (car lsp--test-sent))"), "0");
    assert_eq!(
        run(&mut i, "(gethash \"result\" (car lsp--test-sent))"),
        ":null"
    );
    assert_eq!(
        run(&mut i, "(gethash \"method\" (car lsp--test-sent) 'absent)"),
        "absent"
    );
}

/// M123 Part A: a server-initiated request whose method this client
/// does not recognize gets a JSON-RPC `MethodNotFound` (-32601) error
/// response, per spec, rather than either silence or a bare `nil`
/// result that would misrepresent success.
#[test]
fn server_request_with_unknown_method_gets_method_not_found_error() {
    let mut i = setup();
    run(
        &mut i,
        "(setq lsp--test-sent nil)
         (defun lsp-send (conn msg) (push msg lsp--test-sent) t)",
    );
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"id\\\":5,\\\"method\\\":\\\"workspace/nonexistentThing\\\",\\\"params\\\":{}}\"))",
    );
    assert_eq!(run(&mut i, "(lsp--client-pending client)"), "nil");
    assert_eq!(run(&mut i, "(length lsp--test-sent)"), "1");
    assert_eq!(
        run(
            &mut i,
            "(gethash \"code\" (gethash \"error\" (car lsp--test-sent)))"
        ),
        "-32601"
    );
    assert_eq!(run(&mut i, "(gethash \"id\" (car lsp--test-sent))"), "5");
}

/// M123 Part A: an ordinary notification (`method`, no `id`, not
/// `publishDiagnostics`) is UNCHANGED by this milestone -- still
/// silently dropped, no send, no pending entry, no error. This pins
/// the "deliberately not fixed" half of the file header's own claim.
#[test]
fn plain_notification_without_id_is_still_silently_dropped() {
    let mut i = setup();
    run(
        &mut i,
        "(setq lsp--test-sent nil)
         (defun lsp-send (conn msg) (push msg lsp--test-sent) t)",
    );
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    let r = run(
        &mut i,
        "(lsp--dispatch client (json-parse-string
           \"{\\\"method\\\":\\\"$/progress\\\",\\\"params\\\":{}}\"))",
    );
    assert!(!r.starts_with("ERROR"), "dispatch errored: {}", r);
    assert_eq!(run(&mut i, "lsp--test-sent"), "nil");
    assert_eq!(run(&mut i, "(lsp--client-pending client)"), "nil");
}

/// M123 Part A: `lsp--client-pending` is bounded at
/// `lsp--client-pending-limit` -- past the cap the OLDEST entry (the
/// last id ever dispatched, since new entries are consed onto the
/// front) is dropped rather than retained forever. Fires
/// `lsp--client-pending-limit` + 5 distinct unclaimed responses through
/// dispatch and checks the length never exceeds the cap, and that the
/// very first id sent (the oldest) is the one that fell off.
#[test]
fn pending_list_is_capped_and_drops_the_oldest_entry() {
    let mut i = setup();
    run(&mut i, "(setq client (make-lsp--client :conn nil))");
    let total = 205; // lsp--client-pending-limit (200) + 5
    for id in 0..total {
        run(
            &mut i,
            &format!(
                "(lsp--dispatch client (json-parse-string \"{{\\\"id\\\":{id},\\\"result\\\":1}}\"))"
            ),
        );
    }
    assert_eq!(run(&mut i, "(length (lsp--client-pending client))"), "200");
    // id 0 (the very first, oldest) must have been dropped.
    assert_eq!(run(&mut i, "(assq 0 (lsp--client-pending client))"), "nil");
    // The most recent id (204) must still be present.
    assert_eq!(
        run(&mut i, "(if (assq 204 (lsp--client-pending client)) t nil)"),
        "t"
    );
}
