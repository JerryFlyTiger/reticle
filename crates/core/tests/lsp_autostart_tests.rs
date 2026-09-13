//! M88: automatic asynchronous startup of an LSP server on first open --
//! `lsp-autostart`, `lsp-autostart-timeout`, `lsp--autostart-pending`,
//! `lsp--autostart-tried`, `lsp--autostart-begin`, and the idle-tick step
//! `lsp--autostart-tick`, all in `lsp.el`.
//!
//! M123 fix round: `cat' used to stand in as the fake server, on the
//! theory that echoing the `initialize' request's raw bytes straight
//! back would look enough like a reply for the handshake to complete.
//! It "worked" only because a message with BOTH `id' AND `method' --
//! exactly what an echoed REQUEST carries -- fell into the old
//! dispatcher's `id' branch, got stashed as if it were a reply, and
//! `lsp--await'/the autostart completion callback read `(gethash
//! "result" ...)' off it, which is nil for a request and was silently
//! accepted as an empty result. M123 Part A's dispatcher now correctly
//! reads `id'+`method' together as a REQUEST (answered, never
//! stashed), so `cat' no longer produces anything the handshake
//! recognizes as a reply and every one of these tests hung. Replaced
//! throughout this file with `ECHO_INIT_SCRIPT'/`DELAYED_ECHO_INIT_
//! SCRIPT' (below, both `register_cat' calls through `write_script'): a
//! small `python3' script that reads and discards the real
//! `initialize' request, answers it with an actual, well-formed
//! JSON-RPC response, and only THEN echoes everything else verbatim
//! (like the original `cat') -- see `ECHO_INIT_SCRIPT''s own doc
//! comment for why a plain "one reply then `cat'" script is not enough
//! on its own: this file's tests still need the client's OWN outgoing
//! `didOpen'/`didChange' notifications echoed back for `drain_frames'/
//! `lsp-poll' to observe. `register_cat' keeps its pre-existing NAME
//! (every call site already reads naturally as "give me a fake server
//! that answers") even though it no longer spawns literal `cat' as the
//! server command.
//!
//! For the "deaf server" tests (spawns, reads stdin, never writes
//! anything back), `sh -c "cat > /dev/null"' stands in: a real,
//! genuinely alive child process that will never answer -- unaffected
//! by the above, since these tests never expect a reply at all.
//!
//! `lsp--autostart-tick' no-ops entirely until `lsp--frontend-started' is
//! set -- every test below sets it explicitly with `(setq lsp--frontend-
//! started t)' unless the test is specifically about that gate (test 7).
//!
//! Fix-round note (F review; G4 fix round 3 -- do not restate a count of
//! call sites here, it will go stale again the next time a test is
//! added or removed; grep for `dispatch_one_pending' instead): every
//! test below that needs to observe an outgoing frame (a `didOpen', or
//! a still-pending client) right after completing a handshake calls the
//! `dispatch_one_pending' helper instead of `lsp-process-pending-all'.
//! That workaround exists because `cat' echoes bytes back essentially
//! instantly: `lsp-process-pending-all''s ordinary drain-until-empty
//! loop, mid-dispatch of the `initialize' reply, would otherwise
//! immediately re-poll and silently swallow the `didOpen' the
//! completion callback sends synchronously from inside that same
//! dispatch -- an artifact of this specific fake transport, not
//! something a real server (which never echoes its own notifications
//! back) can trigger. The consequence: this file does NOT end-to-end
//! exercise the production drain-and-re-poll path (`lsp-process-
//! pending-all') completing an autostart handshake and sending its
//! `didOpen' in the same call -- the tests here that drive
//! `core::idle_tick' directly (grep for it) DO go through the real
//! `lsp-process-pending-all' end to end, but none of them asserts on
//! `didOpen' framing: they assert on which of "attached" vs "reaped"
//! won. (M135: this named one specific test and called it "the only"
//! one, which F10 had already falsified. The trailing cold read then
//! pointed out that replacing "one" with "two" repeats the same
//! mistake one test later -- and that this file's own note below
//! already says not to restate call-site counts here. So the count is
//! gone rather than corrected.)

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

fn ok(interp: &mut Interp, src: &str) -> String {
    let r = run(interp, src);
    assert!(!r.starts_with("ERROR"), "{:?} failed: {}", src, r);
    r
}

/// A scratch directory that deletes itself on drop -- same shape as
/// `lsp_auto_attach_tests.rs`'s `Scratch` (see its own doc comment for
/// why cleanup lives in `Drop`, not the last line of each test body).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "reticle_lsp_autostart_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn scratch_dir(tag: &str) -> Scratch {
    Scratch::new(tag)
}

/// Drain every frame `conn` has echoed back whose "method" is METHOD --
/// same technique as `lsp_auto_attach_tests.rs`'s helper of the same name.
fn drain_frames(i: &mut Interp, conn_expr: &str, method: &str) -> usize {
    let src = format!(
        "(let ((frames nil) (msg t))
           (while msg
             (setq msg (lsp-poll {conn}))
             (when (and msg (equal (gethash \"method\" msg nil) {method:?}))
               (setq frames (cons msg frames))))
           (setq test--frames (nreverse frames))
           (length test--frames))",
        conn = conn_expr,
        method = method,
    );
    ok(i, &src)
        .parse()
        .expect("frame count should print as an integer")
}

/// Writes an executable script into DIR and returns its path as a
/// string -- same shape as `lsp_tests.rs''s own `write_script' (not
/// shared across the two test binaries: each is `cfg(test)'-local to
/// its own crate-external integration-test file, and there is no
/// common home for a helper to live in that both could `include!').
fn write_script(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    let mut perm = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
    std::fs::set_permissions(&path, perm).unwrap();
    path.to_str().unwrap().to_string()
}

/// A `python3' fake server (not a `sh' one-liner, unlike `lsp_tests.rs''s
/// own `ECHO_ONE_SCRIPT'): this file's own tests, unlike that one, still
/// need everything AFTER the handshake echoed back too --
/// `drain_frames'/`lsp-poll' read the client's own outgoing `didOpen'/
/// `didChange' notifications back off this same fake connection as
/// their only way to observe what the client sent (see `dispatch_one_
/// pending''s own doc comment). That rules out the naive fix of just
/// swapping `cat' for a script that prints one reply and exits into
/// `cat >/dev/null' (discards everything, so those notifications would
/// never come back) -- and it rules out KEEPING plain `cat' with
/// nothing in front of it (the ORIGINAL initialize REQUEST, still
/// sitting unread on stdin, would then also get echoed back after this
/// script's own crafted reply, landing back at the client as a message
/// with both `id' AND `method' -- exactly the shape M123 Part A's
/// dispatcher now treats as an incoming REQUEST, which it would answer,
/// producing a reply that itself gets echoed straight back into an
/// infinite request/response ping-pong). So this script actively READS
/// and DISCARDS exactly one framed message (the real `initialize'
/// request) before answering it itself with a well-formed response and
/// only THEN switching into raw echo mode for everything that follows
/// -- a plain `sh' one-liner cannot parse a `Content-Length' header and
/// read exactly that many body bytes without forking a subprocess per
/// byte (see `dev/fake-lsp.py''s own header, pitfall 1, on why that
/// path was rejected there too), so this reuses `dev/fake-lsp.py''s own
/// `read_message'/`send' shape in miniature rather than fighting `sh'.
///
/// Recorded, not fixed (trailing cold review, M123 fix round):
/// `read_message' uses buffered `sys.stdin.buffer.read()', the echo
/// loop below it uses raw `os.read()' on the same fd -- mixing the two
/// is only safe here because the client never pipelines a second
/// message ahead of the `initialize' reply. This fixture also now
/// requires `python3' on PATH and an executable temp directory, unlike
/// the plain `cat'/`sh' it replaced.
const ECHO_INIT_SCRIPT: &str = "#!/usr/bin/env python3
import os
import sys

def read_message():
    header = b''
    while not header.endswith(b'\\r\\n\\r\\n'):
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        header += ch
    length = 0
    for line in header.decode('latin-1').split('\\r\\n'):
        if line.lower().startswith('content-length:'):
            length = int(line.split(':', 1)[1])
    return sys.stdin.buffer.read(length)

def send(body):
    sys.stdout.buffer.write(b'Content-Length: %d\\r\\n\\r\\n' % len(body) + body)
    sys.stdout.buffer.flush()

read_message()  # discard the real `initialize' request
# No `capabilities' key (matching `lsp_tests.rs''s own `ECHO_ONE_SCRIPT',
# `\"result\":{}}') -- an ABSENT capabilities hash is what this client's
# M46 policy trusts as \"everything supported\"; a PRESENT-but-empty one
# instead reads as every individual capability being unsupported, which
# would make `(lsp)''s own connect message grow an unwanted \"(unsupported:
# ...)\" suffix these tests don't expect.
send(b'{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}')
# Echo everything else verbatim -- `didOpen'/`didChange'/`initialized'
# notifications the client sends next, which this file's own
# `drain_frames'/`lsp-poll' helpers read back as their only observation
# point. None of them carry an `id', so echoing them back is never
# misread as a request. `os.read'/`os.write' on the raw fds, NOT
# `sys.stdin.buffer.read(4096)' -- a `BufferedReader.read(N)' on a pipe
# blocks until N bytes actually arrive (or EOF), so a single small
# notification well under 4096 bytes would sit forever waiting for more
# input that never comes, and this fake server would silently never
# echo anything back at all. `os.read' returns as soon as WHATEVER is
# currently available, exactly the single-`read(2)'-syscall semantics an
# actual byte-stream relay needs.
fd_in = sys.stdin.fileno()
fd_out = sys.stdout.fileno()
while True:
    chunk = os.read(fd_in, 4096)
    if not chunk:
        break
    os.write(fd_out, chunk)
";

/// Same reply as `ECHO_INIT_SCRIPT', but only after a 1-second sleep --
/// `did_change_cannot_precede_did_open' (below) depends on the fake
/// server's reply NOT arriving within the first few, sleep-free
/// `lsp-process-pending-all' pumps it runs; see that test's own comment
/// for why (a race this delay forecloses by construction, not a
/// tuned-to-pass timing coincidence).
const DELAYED_ECHO_INIT_SCRIPT: &str = "#!/usr/bin/env python3
import os
import sys
import time

def read_message():
    header = b''
    while not header.endswith(b'\\r\\n\\r\\n'):
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        header += ch
    length = 0
    for line in header.decode('latin-1').split('\\r\\n'):
        if line.lower().startswith('content-length:'):
            length = int(line.split(':', 1)[1])
    return sys.stdin.buffer.read(length)

def send(body):
    sys.stdout.buffer.write(b'Content-Length: %d\\r\\n\\r\\n' % len(body) + body)
    sys.stdout.buffer.flush()

read_message()
time.sleep(1)
# See `ECHO_INIT_SCRIPT''s own comment for why no `capabilities' key and
# why the echo loop below uses `os.read'/`os.write' rather than
# `sys.stdin.buffer.read(4096)'.
send(b'{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}')
fd_in = sys.stdin.fileno()
fd_out = sys.stdout.fileno()
while True:
    chunk = os.read(fd_in, 4096)
    if not chunk:
        break
    os.write(fd_out, chunk)
";

/// M135: reads exactly one full Content-Length-framed message (the
/// `initialize` request) and then exits with no reply -- for
/// `dead_process_before_deadline_is_reaped_via_liveness_not_deadline`
/// (F10), which needs the spawn itself to unconditionally succeed
/// (`lsp-send` must never hit EPIPE) and the process to die only
/// AFTERWARD.
///
/// This is NOT the same shape as `sh -c "read line; exit 1"`, which was
/// tried first and rejected: `sh`'s `read` builtin stops at the first
/// `\n`, and `write_message` (`crates/elisp/src/lsp.rs`) issues the
/// header and the body as TWO SEPARATE `write` calls -- a shell `read
/// line` only consumes the header line, so the child can still exit
/// (closing its stdin) in the gap between those two writes, leaving the
/// BODY write to hit EPIPE exactly as before, just less often. Reading
/// the full framed message the same way `ECHO_INIT_SCRIPT` does --
/// blocking on `sys.stdin.buffer.read(length)` for the whole body, not
/// just a line -- means this script's own `read_message` call cannot
/// return (and therefore the process cannot exit) until every byte of
/// both writes has actually arrived, closing the race by construction
/// rather than narrowing its window.
const DIES_AFTER_READING_INITIALIZE_SCRIPT: &str = "#!/usr/bin/env python3
import sys

def read_message():
    header = b''
    while not header.endswith(b'\\r\\n\\r\\n'):
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        header += ch
    length = 0
    for line in header.decode('latin-1').split('\\r\\n'):
        if line.lower().startswith('content-length:'):
            length = int(line.split(':', 1)[1])
    return sys.stdin.buffer.read(length)

read_message()  # consume the whole `initialize` request, then just die
sys.exit(1)
";

/// Returns the fake server's own script path -- most callers ignore it
/// (the pre-M123 `register_cat` returned nothing, since `"cat"` was a
/// fixed, known literal every assertion could just spell out), but a
/// few tests need the ACTUAL command string `lsp-server-alist` now
/// holds (a per-test scratch-directory path, not a fixed name) to
/// build an exact-match expectation against it.
fn register_cat(i: &mut Interp, dir: &std::path::Path) -> String {
    let script = write_script(dir, "echo_init.sh", ECHO_INIT_SCRIPT);
    ok(
        i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'rust-mode (list {:?})))",
            script
        ),
    );
    script
}

fn set_frontend_started(i: &mut Interp) {
    ok(i, "(setq lsp--frontend-started t)");
}

/// Poll exactly ONE message off the (single) pending autostart's
/// connection and dispatch it -- unlike `lsp-process-pending-all`, this
/// does not loop until the queue is empty, so it can't self-consume a
/// message the completion callback it triggers goes on to send. That
/// self-consumption is a real risk ONLY because the fake transport
/// (`ECHO_INIT_SCRIPT`, formerly plain `cat`) echoes bytes back
/// essentially instantly -- a real server never echoes its own
/// `didOpen` back at all, so `lsp-process-pending-all`'s draining loop
/// is exactly right in production; it's just the wrong tool for
/// observing one specific outgoing frame against this particular test
/// double.
///
/// M123 fix round (trailing cold review): this used to be "sleep a
/// fixed amount, then poll exactly once" at every call site -- a sleep
/// is not a synchronisation primitive (this project's own rule), so
/// raising the fixed sleep (300ms -> 1500ms, an earlier fix-round
/// round) only ever LOWERED the probability of a race under load,
/// never removed the race itself. Now a bounded POLLING loop, entirely
/// inside this one function: try `lsp-poll` immediately, and only
/// sleep a SHORT interval between retries when nothing has arrived
/// yet, up to a generous ceiling -- deterministic (retries until the
/// real message actually shows up, never gives up before it does,
/// short of the ceiling) and typically much faster than the fixed
/// sleep it replaces (the common case answers within a few
/// milliseconds, not 1.5 real seconds).
///
/// M135 fix round (F3, trailing cold review): the previous paragraph's
/// last sentence -- that this function was "the only place in this file
/// that waits for a reply to physically arrive before dispatching it" --
/// went stale the moment M135 added `pump_until` (this file, above), and
/// was never updated; a cold reviewer caught the resulting lie. The
/// actual division of labor as of M135: this function (`dispatch_one_
/// pending`) polls and dispatches EXACTLY ONE message, which is what
/// makes it safe against self-consumption (see this doc comment's first
/// paragraph) -- callers use it when they need to catch one specific
/// completion without risking `lsp-process-pending-all`'s drain-until-
/// empty loop swallowing a frame the completion callback sends
/// synchronously. `pump_until` instead calls `(lsp-process-pending-all)`
/// -- the real, unbounded drain -- on every iteration and only checks an
/// elisp predicate afterward; callers use it when they're waiting on a
/// STATE (e.g. `lsp--buffer-client` becoming non-nil) rather than
/// catching one specific frame, where the self-consumption risk this
/// function exists to avoid doesn't apply.
fn dispatch_one_pending(i: &mut Interp) {
    ok(
        i,
        "(setq test--pending-client (nth 1 (cdr (car lsp--autostart-pending))))",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let dispatched = ok(
            i,
            "(let ((msg (lsp-poll (lsp--client-conn test--pending-client))))
               (if (hash-table-p msg)
                   (progn (lsp--dispatch test--pending-client msg) t)
                 nil))",
        );
        if dispatched == "t" {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "dispatch_one_pending: no message arrived on the pending autostart's \
                 own connection within the 10s polling ceiling -- either the fake \
                 server never answered, or the poll loop has a real bug"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// M135: polls `(lsp-process-pending-all)` (which drives the whole
/// pending queue, unlike `dispatch_one_pending`'s single-message-at-a-
/// time care) followed by evaluating PRED_ELISP, until PRED_ELISP
/// evaluates to non-nil -- 5ms between retries, up to a 30-second
/// ceiling. Exists to replace the "sleep a fixed 1500ms, then check
/// once" shape that made this file's tests flaky under load (see PLAN.md
/// M135): a fixed sleep is not a synchronisation primitive, and this
/// project's own fake `python3` echo server was measured taking anywhere
/// from 404ms (idle machine) to 942ms (26 tests under load), leaving as
/// little as ~1.6x headroom over a fixed 1500ms wait.
///
/// Panics (never `return`s -- see this file's other helpers for why a
/// `return` here would be indistinguishable from a real pass under this
/// project's own gate) naming WHAT it was waiting for if the ceiling is
/// reached, so a genuine regression in the code under test still reads
/// as a clear failure rather than an indefinite hang.
fn pump_until(i: &mut Interp, pred_elisp: &str, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        ok(i, "(lsp-process-pending-all)");
        let r = run(i, pred_elisp);
        if r != "nil" {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "pump_until: condition {:?} never became true within 30s while \
                 waiting for: {} -- either the fake server never answered, or \
                 this is a real bug",
                pred_elisp, what
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// M135: like `drain_frames`, but repeatedly re-runs the drain and
/// ACCUMULATES the count across calls (rather than trusting a single
/// pass to have caught everything) until the running total reaches
/// WANT, or the 30-second ceiling elapses. `drain_frames` itself does a
/// single pass that stops the moment `lsp-poll` returns nil -- which is
/// exactly wrong when the fake server hasn't finished echoing everything
/// back yet: a single pass can legitimately see fewer than WANT frames
/// and then simply stop, because at that instant nothing more had
/// arrived. This helper keeps calling `drain_frames` until enough have
/// shown up in total.
///
/// M135 fix round (F2, trailing cold review): reaching `total >= want`
/// used to return immediately -- which only proves "at least WANT
/// arrived", not "exactly WANT arrived", the moment there's any risk of
/// MORE than WANT showing up. The original fixed `sleep(1500)` this
/// milestone replaced did a single FULL drain after its wait, so it
/// would have caught a bug that sends one extra frame right after the
/// wanted ones (e.g. a completion callback that double-sends `didOpen`)
/// -- this accumulating loop, as first written, would NOT have: it
/// stops checking the instant it has counted enough, so a later, unasked
/// -for frame simply never gets looked at. That's a coverage regression
/// this milestone must not introduce. Fixed by settling briefly (50ms,
/// under the guard's own 500ms threshold) once WANT is reached, then
/// doing one more full drain pass and folding it into the total before
/// returning -- if extra frames were queued right behind the wanted
/// ones, this pass catches them and the caller's `== want` assertion
/// goes red. The 50ms is a SETTLE, not a synchronization primitive: by
/// this point the wanted frame has already physically arrived, so any
/// extra frame from the SAME synchronous callback that sent it is
/// already in flight over the same pipe, not waiting on some future
/// decision by the other side -- this is just giving those bytes time to
/// finish traversing the pipe and land in this process's own read
/// buffer, not waiting on an event that might not happen yet.
///
/// M135 trailing cold review raised the obvious objection: this same
/// file measures the fake server's reply latency at 404-942ms under
/// load, so how can 50ms be enough? Because those are two different
/// latencies. The 942ms figure is a `python3' interpreter start plus the
/// first request/response round trip -- it is the cost of the OTHER SIDE
/// producing a first answer. What this settle waits for is the gap
/// between two frames the other side already wrote back-to-back, before
/// either of them had been read: once the first has traversed the pipe
/// and been picked up by the reader thread, the second is sitting in the
/// same buffer behind it, and what remains is a reader-thread hop, not a
/// round trip. Raising 50ms toward a second would re-introduce exactly
/// the cost this milestone removed, and would still not be a bound -- so
/// the honest statement is that this is a generous window for the
/// mechanism actually in play, not a proof. Nothing in the tree can turn
/// it red today (`dev/mutations/m135.py' D8 measures that), which is why
/// it is labelled defensive coverage rather than a tested guarantee.
fn drain_frames_until(i: &mut Interp, conn_expr: &str, method: &str, want: usize) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut total = 0usize;
    loop {
        total += drain_frames(i, conn_expr, method);
        if total >= want {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "drain_frames_until: only accumulated {} of {} wanted {:?} frames \
                 within 30s -- either the fake server never sent the rest, or this \
                 is a real bug",
                total, want, method
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    total += drain_frames(i, conn_expr, method);
    total
}

/// Same message-capturing shape used by `lsp_action_tests.rs`,
/// `lsp_format_tests.rs`, `lsp_highlight_tests.rs`, `lsp_references_tests.rs`.
fn capture_messages(i: &mut Interp) {
    ok(i, "(setq test--messages nil)");
    ok(
        i,
        "(defun message (fmt &rest args) (push (apply 'format fmt args) test--messages) fmt)",
    );
}

// ============================================================
// 1. Happy path: idle tick spawns, completion attaches the current
//    buffer, and backfill also attaches another already-open buffer in
//    the same project.
// ============================================================

#[test]
fn happy_path_spawns_attaches_and_backfills() {
    let mut i = setup();
    let dir = scratch_dir("happy_path");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i, &dir);

    // b.rs opened first -- no connection exists yet, so it just sits
    // there unattached.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    // a.rs opened second and left current -- this is the buffer whose
    // idle tick triggers the autostart.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    set_frontend_started(&mut i);
    ok(&mut i, "(lsp--autostart-tick)");

    // Spawned, but NOT yet attached (I2): the handshake is still
    // in-flight, so lsp--buffer-client stays nil until the completion
    // callback runs.
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Let cat's echoed initialize reply arrive, which fires the
    // completion closure synchronously from inside `dispatch_one_pending`.
    dispatch_one_pending(&mut i);

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );
    ok(&mut i, "(setq test--client lsp--buffer-client)");

    // b.rs -- never itself ticked -- must have been backfilled too.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--client)"), "t");

    // M135: was a fixed 1500ms sleep then a single-pass drain -- replaced
    // with an accumulating drain (see `drain_frames_until`'s own doc
    // comment) so this doesn't depend on both didOpens having already
    // arrived within an arbitrary fixed window.
    assert_eq!(
        drain_frames_until(&mut i, "test--conn", "textDocument/didOpen", 2),
        2,
        "both a.rs and b.rs should have been didOpen'd"
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

#[test]
fn autostart_records_the_root_it_connected_with_on_the_client() {
    // M131 fix round (FIX-1): `lsp--autostart-begin' is the REAL
    // call site a live GUI session actually connects through (the GUI
    // never runs `M-x lsp' itself) -- unlike `lsp-connect', which
    // `lsp_mode_tests.rs''s own `lsp_connect_records_its_own_root_on_
    // the_client' already guards, nothing here stood watch on
    // `(make-lsp--client :conn conn :command command :root root)''s
    // own `:root root' actually landing. Same shape as `happy_path_
    // spawns_attaches_and_backfills' above, trimmed to the one
    // assertion this test exists for.
    let mut i = setup();
    let dir = scratch_dir("autostart_records_root");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);
    ok(&mut i, "(lsp--autostart-tick)");
    dispatch_one_pending(&mut i);

    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(
        run(&mut i, "(lsp--client-root lsp--buffer-client)"),
        format!("{:?}", dir.to_str().unwrap()),
        "the client's own stored root must be the ROOT this connection was actually made with"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 2. Deaf server: spawns, reads, never writes. The editor stays
//    responsive across many idle ticks; after the deadline the pending
//    entry is reaped, lsp-kill runs, one message is emitted, the entry
//    lands in lsp--autostart-tried, and no second spawn happens.
// ============================================================

#[test]
fn deaf_server_is_reaped_after_the_deadline_with_no_retry() {
    let mut i = setup();
    let dir = scratch_dir("deaf_server");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist
             (cons 'rust-mode (list \"sh\" \"-c\" \"cat > /dev/null\")))",
    );
    // M135: was `1` -- under load, the 5x100ms "stays responsive" pacing
    // loop below can itself take over 1s wall-clock, which would let the
    // entry get reaped EARLY, mid-loop, turning "still pending after the
    // loop" red for reasons that have nothing to do with deadline
    // handling. `30` makes early reaping structurally impossible during
    // the loop; see below for how a real deadline still gets exercised
    // afterward without depending on how long the loop above took.
    ok(&mut i, "(setq lsp-autostart-timeout 30)");
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Several idle ticks well inside the 1s deadline: the entry must
    // stay pending, and ordinary editing must keep working -- nothing on
    // this path blocks. F8 review fix: bound the NON-sleep work of each
    // iteration (the sleep itself is the test's own pacing, not part of
    // what "responsive" claims) so the "stays responsive" comment is an
    // observation, not just a description of which state variables get
    // checked afterward. 2s of slack per iteration is generous -- a
    // genuinely blocked call here would mean `lsp-kill`'s `child.wait()`
    // (or something worse) hanging on a deaf server that hasn't even
    // been killed yet, not a plausible slow-but-fine case.
    for _ in 0..5 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let tick_start = std::time::Instant::now();
        ok(&mut i, "(lsp-process-pending-all)");
        ok(&mut i, "(lsp--autostart-tick)");
        ok(&mut i, "(insert \"x\")");
        let tick_elapsed = tick_start.elapsed();
        assert!(
            tick_elapsed < std::time::Duration::from_secs(2),
            "one idle-tick-shaped iteration took {:?} against a deaf \
             server well before its deadline -- the editor is supposed \
             to stay responsive here",
            tick_elapsed
        );
    }
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "lsp--autostart-tried"), "nil");

    // M135: was `sleep(1200)` against a deadline `lsp-autostart-timeout`
    // set at the top of this test -- with the timeout now `30` (so the
    // pacing loop above can never reap early, see that assignment's own
    // comment), that deadline is no longer something a short fixed sleep
    // could reach at all. Instead: reset THIS ONE pending entry's own
    // deadline to a fresh, real, short one, then poll the REAL clock
    // (`float-time`) until it has genuinely elapsed. This still proves
    // "reaping only happens once a real deadline has passed" against an
    // actual wall-clock deadline -- it's just a freshly-set one, not the
    // original `30`-second one, so the test's own runtime doesn't depend
    // on how much load happened to exist during the preceding loop.
    //
    // Two alternatives considered and rejected:
    //   - Setting the deadline into the PAST (e.g. `(- (float-time) 1)`)
    //     would make the reap-timing assertions below vacuous: nothing in
    //     this file would then ever observe "reaping waits for a REAL
    //     deadline to actually pass" (every past-deadline reap fires
    //     immediately regardless of whether deadline handling works at
    //     all).
    //   - Just raising the fixed sleep from 1s to 3s (matching the
    //     spec's suggestion) only moves the safety margin from 2x to
    //     6x -- still a fixed sleep racing an external process's
    //     scheduling latency under load, the exact shape this whole
    //     milestone exists to remove.
    ok(
        &mut i,
        "(setf (nth 2 (cdr (car lsp--autostart-pending))) (+ (float-time) 0.2))",
    );
    ok(
        &mut i,
        "(setq test--reap-deadline (nth 2 (cdr (car lsp--autostart-pending))))",
    );
    let poll_deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let past = ok(&mut i, "(> (float-time) test--reap-deadline)");
        if past == "t" {
            break;
        }
        if std::time::Instant::now() >= poll_deadline {
            panic!(
                "deaf_server_is_reaped_after_the_deadline_with_no_retry: the real \
                 clock never passed test--reap-deadline within 30s -- this is a \
                 real bug, not a timing fluke"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // This is the call that reaps the entry, i.e. the one that reaches
    // `lsp-kill`'s blocking `child.wait()` (documented at this file's
    // own M88 section header as the one honest exception to "nothing on
    // this path blocks"). Bounded generously: `sh -c "cat > /dev/null"`
    // is a real, well-behaved child that responds to SIGKILL
    // immediately, so this reap should be on the order of milliseconds,
    // not seconds.
    let reap_start = std::time::Instant::now();
    ok(&mut i, "(lsp-process-pending-all)");
    ok(&mut i, "(lsp--autostart-tick)");
    let reap_elapsed = reap_start.elapsed();
    assert!(
        reap_elapsed < std::time::Duration::from_secs(5),
        "reaping the deaf server took {:?} -- lsp-kill's child.wait() \
         may be stuck",
        reap_elapsed
    );

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(lsp-live-p test--pending-conn)"), "nil");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("gave up"),
        "expected a give-up message, got: {}",
        messages
    );
    assert_eq!(run(&mut i, "(length test--messages)"), "1");

    // No second spawn: another tick with the same buffer current must
    // not touch lsp--autostart-pending or spawn a new client.
    let clients_before = run(&mut i, "(length lsp--clients)");
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), clients_before);
}

// ============================================================
// 3. Missing binary: one message, no panic, entry in the give-up list,
//    never retried.
// ============================================================

#[test]
fn missing_binary_is_recorded_and_never_retried() {
    let mut i = setup();
    let dir = scratch_dir("missing_binary");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    ok(
        &mut i,
        "(add-to-list 'lsp-server-alist
             (cons 'rust-mode (list \"reticle-lsp-autostart-test-no-such-binary-88\")))",
    );
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    let r = run(&mut i, "(lsp--autostart-tick)");
    assert!(!r.starts_with("ERROR"), "tick signaled: {}", r);
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length test--messages)"), "1");

    // A second tick with the same buffer current must not retry.
    let r2 = run(&mut i, "(lsp--autostart-tick)");
    assert!(!r2.starts_with("ERROR"), "second tick signaled: {}", r2);
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    assert_eq!(run(&mut i, "(length test--messages)"), "1");
}

// ============================================================
// 4. Two buffers in one project produce exactly one spawn (I3).
// ============================================================

#[test]
fn two_buffers_in_one_project_produce_exactly_one_spawn() {
    let mut i = setup();
    let dir = scratch_dir("one_spawn");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Tick with a.rs current: begins the one and only spawn.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), "1");

    // Switch to b.rs (already open -- find-file-hook does not re-run)
    // and tick again: same project/command, so it must find the
    // existing pending entry rather than spawning its own.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    assert_eq!(run(&mut i, "(length lsp--clients)"), "1");

    // Let it complete; both buffers must end up on the SAME client.
    pump_until(
        &mut i,
        "lsp--buffer-client",
        "b.rs's lsp--buffer-client to become non-nil",
    );
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil"); // b.rs
    ok(&mut i, "(setq test--client-b lsp--buffer-client)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    assert_eq!(run(&mut i, "(eq lsp--buffer-client test--client-b)"), "t");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--client-b))");
}

// ============================================================
// 5. lsp-autostart nil: never spawns.
// ============================================================

#[test]
fn lsp_autostart_nil_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("autostart_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);
    ok(&mut i, "(setq lsp-autostart nil)");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 6. lsp-auto-attach nil: never spawns (D6's coupling).
// ============================================================

#[test]
fn lsp_auto_attach_nil_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("auto_attach_off");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);
    ok(&mut i, "(setq lsp-auto-attach nil)");

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 7. Frontend flag never set: never spawns. This documents why every
//    other integration test file in this crate -- which calls
//    `find-file-internal`/`eval_source` directly and never goes through
//    a real `run_tui`/`run_gui` loop -- can never trigger an autostart
//    spawn no matter how many idle-tick-shaped calls it happens to make.
// ============================================================

#[test]
fn frontend_flag_never_set_never_spawns() {
    let mut i = setup();
    let dir = scratch_dir("no_frontend_flag");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    assert_eq!(run(&mut i, "lsp--frontend-started"), "nil");

    // Several ticks, exactly as the deadline-driven tests above do --
    // none of them may do anything at all.
    for _ in 0..5 {
        ok(&mut i, "(lsp--autostart-tick)");
    }
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--clients"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
}

// ============================================================
// 8. didChange cannot precede didOpen (I2): edit the buffer while the
//    autostart is pending, then let it complete, and assert the
//    didOpen carries the edited text and that no didChange was sent
//    before it -- one connection drain, one pass, since `lsp-poll` is
//    destructive and a second `drain_frames` call on the same
//    connection would find nothing left.
// ============================================================

#[test]
fn did_change_cannot_precede_did_open() {
    let mut i = setup();
    let dir = scratch_dir("no_early_did_change");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    // A deliberately DELAYED reply, not the ordinary `register_cat`
    // helper -- forecloses a real race by construction rather than
    // papering over its symptom. An immediate reply, and under default
    // (parallel) thread count, several spawned test processes
    // contending for CPU at once, that reply can occasionally land
    // fast enough for the THREE bare `lsp-process-pending-all` pumps
    // below (no sleep before them, by design -- see their own comment)
    // to already drain and dispatch it, completing the handshake from
    // INSIDE that same drain-until-empty loop -- which then
    // self-consumes the `didOpen` the completion callback sends, the
    // exact failure mode `dispatch_one_pending` exists to avoid
    // elsewhere in this file. A 1s delay before the fake server replies
    // at all guarantees those three quick pumps see nothing, so this
    // test's own subsequent sleep + `dispatch_one_pending` (not
    // `lsp-process-pending-all`) is deterministically what completes
    // the handshake, every time.
    let script = write_script(&dir, "delayed_echo_init.sh", DELAYED_ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'rust-mode (list {:?})))",
            script
        ),
    );

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    // Edit while the handshake is still in flight, and run several
    // pumps -- lsp--sync-buffer-now's own buffer walk runs on every one
    // of them, and I2 says it must be a no-op here (lsp--buffer-client
    // is still nil), so no didChange can be sent for this edit. The
    // delayed transport (above) guarantees these three can't
    // accidentally also complete the handshake.
    ok(&mut i, "(goto-char (point-max))");
    ok(&mut i, "(insert \"\\n// edited while pending\\n\")");
    for _ in 0..3 {
        ok(&mut i, "(lsp-process-pending-all)");
    }
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--conn (lsp--client-conn lsp--buffer-client))",
    );

    // M135: was a fixed 1500ms sleep then a single pass over the queue.
    // Replaced with a loop that APPENDS every pass's classified frames
    // onto `test--opens'/`test--changes' (never overwrites), stopping
    // once at least one didOpen has shown up. Correctness argument for
    // why stopping at exactly one didOpen is still enough to prove "no
    // didChange snuck out early" (must go in a comment, not just here --
    // the next reader will otherwise mistake this for a hole): the edit
    // above happened BEFORE the handshake completed, so if there were a
    // bug that sent a didChange early, it would have entered this same
    // pipe EARLIER than the didOpen -- and the fake server's echo
    // preserves order, so seeing the didOpen already proves anything
    // that could have preceded it has arrived too.
    //
    // M135 fix round (F2, trailing cold review): that argument only
    // covers a didChange sent BEFORE the didOpen. It does NOT cover a bug
    // that sends a didChange RIGHT AFTER the didOpen (e.g. the same
    // completion callback that sends the didOpen also mistakenly fires
    // an immediate didChange) -- FIFO guarantees that arrives LATER, and
    // the loop above breaks the instant it sees the didOpen, so it would
    // never even look for it. The original fixed `sleep(1500)` this
    // milestone replaced DID cover that case (a single full drain after
    // a generous wait would have picked up any such trailing frame), so
    // leaving this loop as "stop at the first didOpen" would have been a
    // coverage regression. Closed the same way as `drain_frames_until`'s
    // own fix for the identical hole (see that function's own comment
    // for why 50ms here is a settle, not a synchronization primitive):
    // settle briefly once the didOpen has been seen, then run one more
    // full classification pass before trusting the counts.
    ok(&mut i, "(setq test--opens nil) (setq test--changes nil)");
    let classify_one_pass = "(let ((msg t))
           (while msg
             (setq msg (lsp-poll test--conn))
             (when msg
               (let ((m (gethash \"method\" msg nil)))
                 (cond
                  ((equal m \"textDocument/didOpen\")
                   (setq test--opens (append test--opens (list msg))))
                  ((equal m \"textDocument/didChange\")
                   (setq test--changes (append test--changes (list msg))))))))
           (length test--opens))";
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let n: usize = ok(&mut i, classify_one_pass).parse().unwrap_or(0);
        if n >= 1 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "did_change_cannot_precede_did_open: no didOpen arrived within 30s \
                 -- either the fake server never answered, or this is a real bug"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    ok(&mut i, classify_one_pass);
    let counts = ok(&mut i, "(list (length test--opens) (length test--changes))");
    assert_eq!(
        counts, "(1 0)",
        "expected exactly one didOpen and no didChange"
    );

    let text = ok(
        &mut i,
        "(gethash \"text\" (gethash \"textDocument\"
             (gethash \"params\" (car test--opens))))",
    );
    assert!(
        text.contains("edited while pending"),
        "didOpen text missing the edit made while autostart was pending: {}",
        text
    );

    ok(&mut i, "(lsp-kill test--conn)");
}

// ============================================================
// 9. A publishDiagnostics arriving BEFORE our initialize response
//    decorates the buffer without panicking -- a state that has never
//    existed before this milestone.
// ============================================================

#[test]
fn publish_diagnostics_before_initialize_response_does_not_panic() {
    let mut i = setup();
    let dir = scratch_dir("early_diagnostics");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-client (nth 1 (cdr (car lsp--autostart-pending))))",
    );

    let uri_str = format!("file://{}", file.to_str().unwrap());
    let json_body = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":{:?},\"diagnostics\":[]}}}}",
        uri_str
    );
    let msg_expr = format!("(json-parse-string {:?})", json_body);
    let r = run(
        &mut i,
        &format!("(lsp--dispatch test--pending-client {})", msg_expr),
    );
    assert!(!r.starts_with("ERROR"), "dispatch signaled: {}", r);

    // Diagnostics landed on the still-pending client despite no
    // completed handshake yet.
    assert_ne!(
        run(
            &mut i,
            "(lsp-diagnostics test--pending-client (buffer-file-name))"
        ),
        ""
    );

    // The rest of the handshake must still complete normally afterward.
    pump_until(
        &mut i,
        "lsp--buffer-client",
        "lsp--buffer-client to become non-nil once the handshake finishes",
    );
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 10. The mode line shows the pending state and then the attached
//     state (D7): `lsp--autostart-pending-here` -- the buffer-local
//     signal `redisplay.rs` reads via the same `buffer_var_on` it
//     already uses for `lsp--buffer-client` -- is non-nil (keyed
//     `(COMMAND . ROOT)`) while pending, and back to nil once
//     `lsp--buffer-client` itself is set.
// ============================================================

#[test]
fn mode_line_signal_goes_pending_then_attached() {
    let mut i = setup();
    let dir = scratch_dir("mode_line_signal");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let script = register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");
    ok(&mut i, "(lsp--autostart-tick)");

    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    let pending_here = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        pending_here, "nil",
        "expected a pending marker while in flight"
    );
    assert!(
        pending_here.contains(&script),
        "pending marker should carry the command: {}",
        pending_here
    );

    pump_until(
        &mut i,
        "lsp--buffer-client",
        "lsp--buffer-client to become non-nil once attached",
    );

    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 11. F1 (high): an explicit `M-x lsp' racing an in-flight autostart
//     for the SAME (COMMAND . ROOT) must cancel the pending autostart
//     and win deterministically -- not spawn a second, leaked server
//     that the autostart's later completion would silently shadow.
// ============================================================

#[test]
fn m_x_lsp_cancels_an_in_flight_autostart_and_wins() {
    let mut i = setup();
    let dir = scratch_dir("f1_manual_wins");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let script = register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Autostart begins first -- pending, not yet attached.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--autostart-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );
    assert_eq!(run(&mut i, "(lsp-live-p test--autostart-conn)"), "t");

    // `M-x lsp' races it and must win: cancel the pending autostart,
    // spawn its own (synchronous) connection, and attach the buffer.
    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "(lsp) signaled: {}", r);
    assert_eq!(r, format!("\"LSP: connected to {}\"", script));

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(
        run(&mut i, "(lsp-live-p test--autostart-conn)"),
        "nil",
        "the preempted autostart connection must have been killed"
    );
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    ok(&mut i, "(setq test--manual-client lsp--buffer-client)");

    // No zombie completion sneaks a second entry in later: pump/tick a
    // few more times (any reply the killed connection's channel might
    // still have buffered must not resurrect it -- see the completion
    // closure's own defensive branches).
    for _ in 0..3 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        ok(&mut i, "(lsp-process-pending-all)");
        ok(&mut i, "(lsp--autostart-tick)");
    }
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    assert_eq!(
        run(&mut i, "(eq lsp--buffer-client test--manual-client)"),
        "t"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--manual-client))");
}

// ============================================================
// 12. F1 (high): the completion closure's own defense -- if a
//     connection for (COMMAND . ROOT) already exists by the time it
//     fires, it must kill its own connection and attach nothing rather
//     than pushing a duplicate onto lsp--connections.
// ============================================================

#[test]
fn autostart_completion_defers_to_an_already_registered_connection() {
    let mut i = setup();
    let dir = scratch_dir("f1_completion_defensive");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let script = register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Simulate a connection for the same key having already been
    // registered through some other path (this is exactly the state
    // `lsp--get-connection' would see), WITHOUT going through
    // `lsp--autostart-cancel-pending' -- so the pending entry this
    // closure is about to look up is still technically present, and
    // the defense under test is the SECOND cond branch (`lsp--get-
    // connection' already answers), not the first. Must be a REAL,
    // live connection: `lsp--get-connection' self-heals a dead/fake
    // one (prunes it and returns nil), which would silently defeat
    // this test by falling through to the ordinary success path
    // instead of exercising the defensive branch at all. The KEY
    // (COMMAND . ROOT) must match the PRIMARY's own -- COMMAND is now
    // this test's own script path (M123 fix round), not a fixed "cat"
    // literal, so the stand-in's own key has to use the SAME variable
    // or `lsp--get-connection' would never find it and this test would
    // silently stop testing the defensive branch at all. The spawned
    // stand-in PROCESS itself can still be plain `cat' -- it only needs
    // to be genuinely alive, never speak the LSP handshake.
    ok(&mut i, "(setq test--other-conn (lsp-start \"cat\" nil))");
    ok(
        &mut i,
        &format!(
            "(setq test--other-client (make-lsp--client :conn test--other-conn :command {:?}))",
            script
        ),
    );
    ok(
        &mut i,
        &format!(
            "(push (cons (cons {:?} (lsp--project-root (buffer-file-name))) \
                    test--other-client)
                  lsp--connections)",
            script
        ),
    );

    // Now let the pending handshake's reply arrive and dispatch.
    dispatch_one_pending(&mut i);

    // The pending entry must be gone, the buffer must NOT be attached
    // to the autostart's own client (nothing should have attached it
    // at all -- the already-registered connection is a stand-in with a
    // nil conn, unusable for a real attach), and lsp--connections must
    // still hold exactly the one (stand-in) entry -- no duplicate.
    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(run(&mut i, "(length lsp--connections)"), "1");
    assert_eq!(
        run(
            &mut i,
            "(eq (cdr (car lsp--connections)) test--other-client)"
        ),
        "t",
        "the pre-existing connection must not have been replaced"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--pending-conn)"),
        "nil",
        "the autostart's own (now-redundant) connection must have been killed"
    );

    ok(&mut i, "(lsp-kill test--other-conn)");
}

// ============================================================
// 13. F5 (low): the pending marker must clear by its STORED key, not
//     by re-deriving "does this buffer still match" -- otherwise a
//     buffer whose major mode changes away from any `lsp-server-alist'
//     entry during the handshake window keeps showing `LSP…' forever.
// ============================================================

#[test]
fn pending_marker_clears_even_after_the_buffer_stops_matching() {
    let mut i = setup();
    let dir = scratch_dir("f5_sticky_marker");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    // The buffer's major mode changes away from any `lsp-server-alist'
    // entry WHILE the handshake is still in flight -- the exact window
    // where the pre-fix "re-derive match at clear time" logic would
    // have silently skipped clearing this buffer's marker forever.
    ok(&mut i, "(major-mode-internal-set 'fundamental-mode)");

    dispatch_one_pending(&mut i);

    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the pending marker must clear on completion even though this \
         buffer no longer matches any lsp-server-alist entry"
    );
    // Consistent with backfill's own exact-mode requirement: a buffer
    // that no longer matches the connecting mode is not attached.
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");

    ok(&mut i, "(lsp-kill (lsp--client-conn test--pending-client))");
}

// ============================================================
// 14. F7 (low-medium): the pending marker must use the SAME exact-mode
//     test `lsp--auto-attach-backfill' uses, not "any mode whose
//     lsp-server-alist entry happens to share the same COMMAND" --
//     otherwise a buffer in a sibling mode (c-mode/c++-mode both -> the
//     same command here) shows `LSP…' as a promise backfill will never
//     keep.
// ============================================================

#[test]
fn pending_marker_requires_exact_mode_like_backfill_does() {
    let mut i = setup();
    let dir = scratch_dir("f7_exact_mode");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_c = dir.join("a.c");
    let file_cpp = dir.join("b.cpp");
    std::fs::write(&file_c, "int main(void) { return 0; }\n").unwrap();
    std::fs::write(&file_cpp, "int main() { return 0; }\n").unwrap();
    // Two DIFFERENT modes sharing the SAME command -- the exact shape
    // of the default `lsp-server-alist' (c-mode/c++-mode both -> clangd)
    // the reviewer's finding was about.
    let script = write_script(&dir, "echo_init.sh", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'c-mode (list {:?})))",
            script
        ),
    );
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'c++-mode (list {:?})))",
            script
        ),
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'c++-mode)");
    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'c-mode)");
    set_frontend_started(&mut i);

    // c-mode buffer (current) triggers the autostart.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the triggering c-mode buffer itself must be marked"
    );

    // The c++-mode buffer, same project/command, must NOT be marked --
    // backfill will never attach it (exact-mode mismatch), so the
    // pending marker must not promise that it will.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "a same-command, different-mode buffer must not show LSP… \
         pending -- backfill will never attach it"
    );

    // Let the handshake complete and confirm backfill's own behavior
    // matches: c-mode attached, c++-mode buffer left alone.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_c.to_str().unwrap()),
    );
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--f7-conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(
        &mut i,
        &format!("(find-file {:?})", file_cpp.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "backfill must not attach the c++-mode buffer to a c-mode connection"
    );
    assert_eq!(run(&mut i, "lsp--autostart-pending-here"), "nil");

    ok(&mut i, "(lsp-kill test--f7-conn)");
}

// ============================================================
// 15. F9 (medium): drive the REAL `core::idle_tick` (not manual elisp
//     calls in a test-chosen order) through the near-deadline race the
//     design depends on -- the pump (`lsp-process-pending-all`) runs
//     before the reap step (`lsp--autostart-tick`) inside `idle_tick`,
//     so a handshake whose deadline has already technically passed by
//     the time its reply is sitting in the connection's channel is
//     still completed, not reaped out from under itself.
//
//     The deadline is forced into the past directly (rather than raced
//     against real wall-clock timing) so this is deterministic, not
//     flaky: what's under test is ORDERING WITHIN ONE `idle_tick` CALL,
//     not real-time scheduling.
// ============================================================

#[test]
fn idle_tick_completes_a_near_deadline_handshake_before_reaping_it() {
    let mut i = setup();
    let dir = scratch_dir("f9_idle_tick_ordering");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");

    // The real Rust entry point, not `(setq lsp--frontend-started t)`
    // by hand.
    core::frontend_started(&mut i);

    // First real idle_tick: no pending entry yet, so this begins the
    // autostart (lsp--autostart-tick runs as part of it).
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");

    // M135: was `sleep(1500)`. This test's whole point is that a reply
    // sitting in the connection's channel but NOT YET DISPATCHED still
    // gets dispatched (not reaped) inside the same `idle_tick`; ANY call
    // to `lsp-poll`/`lsp-process-pending-all` here (the ordinary way
    // this file waits, in `pump_until`) would itself consume that reply,
    // deleting the very state this test exists to observe -- the
    // `(setf (nth 2 ...) ...)` immediately below would then be mutating
    // a `nil` pending entry, and the whole test would silently prove
    // nothing. So this waits on the reader thread's own delivery count
    // (`lsp-events-delivered`, M135 Part A) instead, which observes
    // "an event has arrived" without taking it. Do NOT "clean this up"
    // into `pump_until` -- that would remove exactly what this test is
    // for.
    ok(
        &mut i,
        "(setq test--f9-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let n: i64 = ok(&mut i, "(lsp-events-delivered test--f9-conn)")
            .parse()
            .unwrap_or(-1);
        if n >= 1 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "idle_tick_completes_a_near_deadline_handshake_before_reaping_it: \
                 lsp-events-delivered never reached 1 within 30s -- the fake \
                 server never replied, or this is a real bug"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    // Force the deadline into the past -- "just barely made its
    // deadline" without depending on real-time scheduling to land the
    // race exactly right.
    ok(
        &mut i,
        "(setf (nth 2 (cdr (car lsp--autostart-pending))) (- (float-time) 5))",
    );

    // One more real idle_tick: `lsp-process-pending-all` (called first,
    // inside `idle_tick`) must drain and dispatch the already-buffered
    // reply -- completing the handshake -- BEFORE `lsp--autostart-tick`
    // (called after it, also inside `idle_tick`) ever gets a chance to
    // reap the (by-then-already-past-deadline) entry.
    core::idle_tick(&mut i, std::time::Duration::ZERO);

    assert_eq!(
        run(&mut i, "lsp--autostart-pending"),
        "nil",
        "the handshake must have completed (removing the pending entry)"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-tried"),
        "nil",
        "it must NOT have been reaped -- if the ordering were reversed, \
         this would show one give-up entry instead"
    );
    assert_ne!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the buffer must have been attached by the completion's backfill"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// 16. F10 (low): the "spawned fine, then died before the deadline"
//     reap branch (`(not (lsp-live-p conn))`) is distinct from both
//     "spawn failed outright" (test 3) and "deadline passed while
//     alive" (test 2) -- exercise it on its own, with a deadline set
//     far enough out that only the liveness check can be what reaps it.
// ============================================================

#[test]
fn dead_process_before_deadline_is_reaped_via_liveness_not_deadline() {
    let mut i = setup();
    let dir = scratch_dir("f10_dead_not_deadline");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    // Spawns successfully, then exits immediately -- distinct from a
    // missing binary (test 3, lsp-start itself fails) and distinct from
    // a deaf server that stays alive (test 2, only the deadline reaps
    // it).
    //
    // M135 fix round: was `"sh" "-c" "exit 1"` alone. Reproduced under
    // 4x parallel load (see PLAN.md M135): `lsp--autostart-begin` writes
    // the `initialize` request to the child's stdin BEFORE any pending
    // entry is recorded, and under load `sh -c "exit 1"` can exit --
    // closing its read end -- before that write happens, so the write
    // hits EPIPE, `lsp--autostart-begin`'s own `condition-case` (`lsp.el`)
    // catches it as a spawn failure, and the site goes straight into
    // `lsp--autostart-tried` with NO pending entry ever created --
    // `(length lsp--autostart-pending)` reads 0, not 1, at the assertion
    // right below. Observed directly: `test--messages` held `"LSP
    // autostart: failed to start sh: lsp-send: Broken pipe (os error
    // 32)"` on reproduction.
    //
    // `DIES_AFTER_READING_INITIALIZE_SCRIPT` (this file, above)
    // eliminates the race by construction: it blocks on reading the
    // WHOLE framed `initialize` message before exiting, so both of
    // `write_message`'s writes (header, then body) are guaranteed to
    // have already landed by the time the child can die. See that
    // const's own doc comment for why a plain `sh -c "read line; exit
    // 1"` was tried first and is NOT enough (it only narrows the race,
    // since a shell `read line` consumes just the header line, not the
    // body that arrives in a second, separate `write` call).
    let script = write_script(
        &dir,
        "dies_after_reading.py",
        DIES_AFTER_READING_INITIALIZE_SCRIPT,
    );
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'rust-mode (list {:?})))",
            script
        ),
    );
    // Deliberately huge: if this test passes, the deadline branch is
    // structurally incapable of having fired -- only liveness could
    // have reaped the entry.
    ok(&mut i, "(setq lsp-autostart-timeout 100)");
    capture_messages(&mut i);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    core::frontend_started(&mut i);

    // Through the real `core::idle_tick`, not manual elisp calls: the
    // `lsp-live-p` check in `lsp--autostart-tick`'s reap loop only
    // reflects reality once the connection has actually been POLLED at
    // least once since the child died -- `LspConnection::is_alive` is a
    // cached flag flipped by the reader thread's `Died` event, observed
    // only via `lsp-poll`/`lsp-wait`, not by any synchronous OS-level
    // check. `idle_tick` calls `lsp-process-pending-all` (which polls
    // every live client, this one included) before `lsp--autostart-
    // tick` runs its reap loop -- driving this test through `idle_tick`
    // rather than `(lsp--autostart-tick)` alone is what makes the
    // liveness check actually see the death, exactly as production
    // does every tick.
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");

    // M135: was `sleep(1500)`. Waits for `lsp-events-delivered` (Part A)
    // instead of `lsp-live-p`, and this distinction is load-bearing, not
    // a style choice: `LspConnection::is_alive` is a cached flag that
    // only flips when something has actually CONSUMED the `Died` event
    // via `try_recv`/`recv_timeout` (i.e. `lsp-poll`/`lsp-wait`) -- so
    // polling `lsp-live-p` in a bare loop here would never see it change
    // (nothing in this loop ever polls the connection) and would spin
    // until the 30s ceiling and panic even though the child is long
    // dead. Worse, a poll loop that DID call `lsp-poll` to make
    // `lsp-live-p` become observable would itself consume the `Died`
    // event ahead of schedule -- pulling forward the very thing this
    // test's own comment above documents as happening on the SECOND
    // `idle_tick` (`lsp-process-pending-all` inside it is what first
    // observes the death, at production-realistic timing). `lsp-events-
    // delivered` sidesteps both problems: it counts `Died` too, without
    // consuming it, so waiting for it to reach 1 proves the child has
    // actually exited without touching when `is_alive` itself flips.
    ok(
        &mut i,
        "(setq test--f10-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let n: i64 = ok(&mut i, "(lsp-events-delivered test--f10-conn)")
            .parse()
            .unwrap_or(-1);
        if n >= 1 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "dead_process_before_deadline_is_reaped_via_liveness_not_deadline: \
                 lsp-events-delivered never reached 1 within 30s -- the child \
                 never died, or this is a real bug"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let before = std::time::Instant::now();
    core::idle_tick(&mut i, std::time::Duration::ZERO);
    let elapsed = before.elapsed();

    assert_eq!(run(&mut i, "lsp--autostart-pending"), "nil");
    assert_eq!(run(&mut i, "(length lsp--autostart-tried)"), "1");
    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("gave up"),
        "expected a give-up message, got: {}",
        messages
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "reap took {:?}, nowhere near instant -- suspicious given a \
         100s deadline that structurally cannot have been the cause",
        elapsed
    );
}

// ============================================================
// 17. G2 (fix round 3): the pending marker must be set AND cleared on
//     the non-current buffer, not just the one that happens to be
//     current when `lsp--autostart-mark-pending` runs. Every earlier
//     test with two buffers sharing a pending entry (1, 4, 14) only
//     ever inspected `lsp--autostart-pending-here` in whichever buffer
//     was current at the time -- this one switches buffers specifically
//     to read the OTHER one, both while pending and after it clears.
//     Mutation this must catch: restricting the clear (or the mark) to
//     `(eq buf (current-buffer))'.
// ============================================================

#[test]
fn pending_marker_is_set_and_cleared_on_the_non_current_buffer_too() {
    let mut i = setup();
    let dir = scratch_dir("g2_non_current_marker");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file_a = dir.join("a.rs");
    let file_b = dir.join("b.rs");
    std::fs::write(&file_a, "fn a() {}\n").unwrap();
    std::fs::write(&file_b, "fn b() {}\n").unwrap();
    let script = register_cat(&mut i, &dir);

    // b.rs opened first -- it will be the NON-current buffer for the
    // rest of this test.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    // a.rs opened second and left current -- triggers the autostart.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_ne!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the current (triggering) buffer must be marked"
    );

    // Switch to b.rs (already open, no hook re-run) and read ITS marker
    // while the handshake is still pending.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    let b_pending_marker = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        b_pending_marker, "nil",
        "the NON-current buffer (b.rs) must also be marked while pending"
    );
    assert!(
        b_pending_marker.contains(&script),
        "b.rs's pending marker should carry the command: {}",
        b_pending_marker
    );

    // Switch back to a.rs (current, so dispatch_one_pending's own
    // (nth 1 (cdr (car lsp--autostart-pending))) lookup is unaffected by
    // which buffer is current -- it reads the pending alist, not any
    // buffer-local state) and let the handshake complete.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_a.to_str().unwrap()),
    );
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "a.rs's own marker must have cleared"
    );
    ok(
        &mut i,
        "(setq test--g2-conn (lsp--client-conn lsp--buffer-client))",
    );

    // Switch to b.rs again and confirm ITS marker cleared too -- this is
    // the assertion the "restrict clear to (eq buf (current-buffer))"
    // mutation would falsify: b.rs was never current during the clear
    // call above, so a buggy clear would leave its marker stuck at
    // `("cat" . ROOT)' forever.
    ok(
        &mut i,
        &format!("(find-file {:?})", file_b.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "the NON-current buffer's marker must have cleared too"
    );

    ok(&mut i, "(lsp-kill test--g2-conn)");
}

// ============================================================
// 18. G3 (fix round 3): cancelling an in-flight autostart for ONE
//     project must not disturb a simultaneously pending autostart for a
//     DIFFERENT project. `lsp--autostart-cancel-pending' looks up an
//     exact `(command . root)' key; a mutation that widened this to
//     "just take the first pending entry" (or otherwise ignored ROOT)
//     would pass every other test here, since none of them have two
//     live pending entries under different keys at once.
// ============================================================

#[test]
fn cancelling_one_projects_autostart_leaves_a_different_projects_alone() {
    let mut i = setup();
    let root = scratch_dir("g3_two_projects");
    let dir1 = root.join("proj1");
    let dir2 = root.join("proj2");
    std::fs::create_dir_all(dir1.join(".git")).unwrap();
    std::fs::create_dir_all(dir2.join(".git")).unwrap();
    let file1 = dir1.join("a.rs");
    let file2 = dir2.join("b.rs");
    std::fs::write(&file1, "fn a() {}\n").unwrap();
    std::fs::write(&file2, "fn b() {}\n").unwrap();
    let script = register_cat(&mut i, &root);

    // Buffer 1: project 1, autostart begins FIRST -- its entry ends up
    // at the TAIL of `lsp--autostart-pending' (each new entry is
    // `push'ed onto the front).
    ok(
        &mut i,
        &format!("(find-file {:?})", file1.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    ok(
        &mut i,
        "(setq test--proj1-pending-conn (nth 0 (cdr (car lsp--autostart-pending))))",
    );

    // Buffer 2: a DIFFERENT project (different root), autostart begins
    // SECOND -- its entry ends up at the HEAD. Deliberately: this is
    // what makes the "just take the first pending entry" mutation
    // observable below -- cancelling project 1 (the one at the TAIL)
    // must not let a "take the head instead" bug pass by coincidentally
    // hitting the right entry anyway.
    ok(
        &mut i,
        &format!("(find-file {:?})", file2.to_str().unwrap()),
    );
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "2");
    let proj2_marker_before = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(proj2_marker_before, "nil");

    // `M-x lsp' in buffer 1's own project (the TAIL entry) must cancel
    // ONLY buffer 1's pending entry, not buffer 2's -- even though
    // buffer 2's entry is the one `car'/`(first ...)' would hand back.
    ok(
        &mut i,
        &format!("(find-file {:?})", file1.to_str().unwrap()),
    );
    let r = run(&mut i, "(lsp)");
    assert!(!r.starts_with("ERROR"), "(lsp) signaled: {}", r);
    assert_eq!(r, format!("\"LSP: connected to {}\"", script));

    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "exactly buffer 1's own pending entry must have been cancelled"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "buffer 1's own marker must be cleared (it's attached now)"
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--proj1-pending-conn)"),
        "nil",
        "buffer 1's preempted autostart connection must be dead"
    );
    ok(
        &mut i,
        "(setq test--proj1-conn (lsp--client-conn lsp--buffer-client))",
    );

    // Buffer 2's pending entry, marker, and connection must be
    // untouched -- this is the assertion a "take the first entry"
    // mutation falsifies: it would have killed buffer 2's connection
    // (the one actually at the head) instead of buffer 1's.
    ok(
        &mut i,
        &format!("(find-file {:?})", file2.to_str().unwrap()),
    );
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "project 2's pending entry must still be there"
    );
    let proj2_marker_after = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        proj2_marker_after, "nil",
        "project 2's own marker must survive project 1's cancellation"
    );
    assert_eq!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        &format!(
            "(setq test--proj2-pending-conn (nth 0 (cdr (assoc (cons {:?} (lsp--project-root (buffer-file-name))) lsp--autostart-pending))))",
            script
        ),
    );
    assert_eq!(
        run(&mut i, "(lsp-live-p test--proj2-pending-conn)"),
        "t",
        "project 2's autostart connection must still be alive"
    );

    // Let project 2's own handshake complete too, for cleanup and as a
    // final sanity check that it was never disturbed.
    dispatch_one_pending(&mut i);
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(
        &mut i,
        "(setq test--proj2-conn (lsp--client-conn lsp--buffer-client))",
    );

    ok(&mut i, "(lsp-kill test--proj1-conn)");
    ok(&mut i, "(lsp-kill test--proj2-conn)");
}

// ============================================================
// M94: a second server per buffer, routed by capability -- the
// autostart-side half. This is the test the silent-sink bug needed:
// before M94, `lsp--autostart-maybe-begin' short-circuited on `(not
// (lsp--live-buffer-client))', so the instant a primary attached, a
// SECONDARY (`lsp-secondary-server-alist') could never autostart for
// that buffer again, for the whole session, with no error and no
// message.
// ============================================================

#[test]
fn secondary_autostart_is_not_blocked_by_an_already_attached_primary() {
    let mut i = setup();
    let dir = scratch_dir("m94_secondary_not_blocked");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let primary_script = register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // The primary autostarts and completes first -- no secondary is
    // registered for rust-mode yet.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "1");
    dispatch_one_pending(&mut i);
    assert_eq!(
        run(&mut i, "(and (lsp--live-buffer-client) t)"),
        "t",
        "the primary must now be live-attached"
    );
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "1");

    // Register a SECONDARY for the same mode, a DIFFERENT command, and
    // tick again. The old `(not (lsp--live-buffer-client))' guard would
    // have skipped this call entirely from here on, forever, since the
    // primary is already live.
    let secondary_script = write_script(&dir, "echo_init_secondary.sh", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-secondary-server-alist (cons 'rust-mode (list {:?})))",
            secondary_script
        ),
    );
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "1",
        "the secondary's own autostart must have begun despite the primary already being live"
    );
    assert_eq!(
        run(&mut i, "(car (car (car lsp--autostart-pending)))"),
        format!("{:?}", secondary_script),
        "the pending entry must be keyed on the SECONDARY's own command"
    );

    // Let it complete too: both clients end up attached, and the
    // primary is untouched.
    dispatch_one_pending(&mut i);
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(equal (lsp--client-command lsp--buffer-client) {:?})",
                primary_script
            )
        ),
        "t",
        "the primary must still be the original cat client, not the secondary"
    );

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}

#[test]
fn secondary_autostart_short_circuits_immediately_for_a_mode_with_no_secondary_registered() {
    // The perf-sensitive fast path (M93): a mode with NO secondary
    // entry must still skip `lsp--project-root' entirely once the
    // primary is already live-attached -- M94 must not have regressed
    // that for every OTHER mode just to fix Verilog's.
    let mut i = setup();
    let dir = scratch_dir("m94_no_secondary_fast_path");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    register_cat(&mut i, &dir);

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    dispatch_one_pending(&mut i);
    assert_eq!(run(&mut i, "(and (lsp--live-buffer-client) t)"), "t");

    ok(&mut i, "(setq test--project-root-calls 0)");
    ok(
        &mut i,
        "(setq test--orig-project-root (symbol-function 'lsp--project-root))",
    );
    ok(
        &mut i,
        "(fset 'lsp--project-root
               (lambda (f) (setq test--project-root-calls (1+ test--project-root-calls))
                 (funcall test--orig-project-root f)))",
    );
    ok(&mut i, "(lsp--autostart-tick)");
    ok(&mut i, "(fset 'lsp--project-root test--orig-project-root)");
    assert_eq!(
        run(&mut i, "test--project-root-calls"),
        "0",
        "no secondary registered for rust-mode -- the fast path must still skip \
         lsp--project-root entirely once the primary is already live"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn lsp--buffer-client))");
}

// ============================================================
// M94 review fix round 2 (AA1/AA4): `lsp--autostart-pending-here` is a
// LIST now, not a single slot -- two independent handshakes (primary
// and secondary) pending on the SAME buffer at once must not have one
// clobber the other's marker, and the secondary completing FIRST must
// not clear the marker while the primary is still in flight. This also
// covers AA4's second request (a test that reads the marker's actual
// VALUE after a secondary-only handshake, not just `lsp--autostart-
// pending'/`lsp--buffer-clients' the way the round-1 M94 tests did --
// see this test's own assertions on `lsp--autostart-pending-here'
// below; no separate test is added for that half of AA4).
// ============================================================

#[test]
fn pending_marker_holds_both_keys_and_only_fully_clears_once_both_complete() {
    let mut i = setup();
    let dir = scratch_dir("aa1_two_pending_markers");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let file = dir.join("a.rs");
    std::fs::write(&file, "fn a() {}\n").unwrap();
    let primary_script = register_cat(&mut i, &dir);
    // The secondary needs its OWN answering fake server too -- it used
    // to be raw `sh -c "cat"` (the same echo defect `register_cat`
    // fixes for the primary, see this file's own header), and this
    // test's own assertions below (the secondary's pending key clears
    // once its handshake completes) depend on that handshake actually
    // completing.
    let secondary_script = write_script(&dir, "echo_init_secondary.sh", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-secondary-server-alist (cons 'rust-mode (list {:?})))",
            secondary_script
        ),
    );

    ok(&mut i, &format!("(find-file {:?})", file.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'rust-mode)");
    set_frontend_started(&mut i);

    // Both the primary and the secondary autostart in the SAME tick.
    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(run(&mut i, "(length lsp--autostart-pending)"), "2");

    // Both keys must be present in the marker at once -- the AA1 bug
    // was the second `lsp--autostart-mark-pending' call unconditionally
    // overwriting the first's key.
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending-here)"),
        "2",
        "both the primary's and the secondary's pending keys must coexist"
    );
    let commands = run(
        &mut i,
        "(sort (mapcar 'car lsp--autostart-pending-here) 'string<)",
    );
    let mut expected_commands = [primary_script.clone(), secondary_script.clone()];
    expected_commands.sort();
    assert_eq!(
        commands,
        format!("({:?} {:?})", expected_commands[0], expected_commands[1])
    );

    // The secondary was BEGUN second, so it's at the HEAD of
    // `lsp--autostart-pending' (each new entry is `push'ed onto the
    // front) -- dispatching the head completes the SECONDARY first,
    // exactly the ordering AA1 is about: nothing orders two independent
    // subprocess startups, and this project's own echoing fake-server
    // transport makes the second-begun one answer first
    // deterministically.
    dispatch_one_pending(&mut i);

    assert_eq!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the secondary must never take the primary slot"
    );
    let after_secondary = run(&mut i, "lsp--autostart-pending-here");
    assert_ne!(
        after_secondary, "nil",
        "the marker must still report pending -- the primary's own handshake \
         is still in flight -- not go fully dark the instant the secondary \
         (whichever completed first) finishes"
    );
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending-here)"),
        "1",
        "only the secondary's own key must have been removed"
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(equal (car (car lsp--autostart-pending-here)) {:?})",
                primary_script
            )
        ),
        "t",
        "the surviving key must be the primary's"
    );

    // Let the primary complete too.
    dispatch_one_pending(&mut i);
    assert_ne!(
        run(&mut i, "lsp--buffer-client"),
        "nil",
        "the primary must now be attached"
    );
    assert_eq!(
        run(
            &mut i,
            &format!(
                "(equal (lsp--client-command lsp--buffer-client) {:?})",
                primary_script
            )
        ),
        "t"
    );
    assert_eq!(
        run(&mut i, "lsp--autostart-pending-here"),
        "nil",
        "both handshakes are done -- the marker must be fully clear now"
    );
    assert_eq!(run(&mut i, "(length lsp--buffer-clients)"), "2");

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}

// ============================================================
// M132: per-server rootUri -- primary and secondary autostart handshakes
// must be keyed at DIFFERENT roots when their commands have different
// `lsp-server-root-style-alist' styles, and a live connection at a
// widened root must be found by the auto-attach path for a buffer
// resolving to the same connected component through a different
// (narrower) file list.
// ============================================================

#[test]
fn autostart_gives_primary_and_secondary_different_roots() {
    // proj/.git, proj/rtl/verible.filelist and proj/verif/verible.filelist
    // share `core/alu.sv' -- one connected component whose smallest
    // covering directory is `proj' itself. Buffer at proj/rtl/core/alu.sv:
    // a `filelist'-style primary must stay at `proj/rtl' (the nearest
    // filelist ancestor); a `workspace'-style secondary (basename
    // `slang-server', matching the default `lsp-server-root-style-alist'
    // entry) must widen to `proj'. This is the deletion test for F1e:
    // reverting `lsp--autostart-try-one'/`lsp--autostart-maybe-begin'
    // back to a single shared ROOT computed once makes both keys carry
    // the SAME root, and this test goes red.
    let mut i = setup();
    let dir = scratch_dir("m132_primary_secondary_roots");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/core")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/top")).unwrap();
    std::fs::create_dir_all(dir.join("verif")).unwrap();
    std::fs::write(
        dir.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = dir.join("rtl/core/alu.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        dir.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(dir.join("verif/tb.sv"), "module tb; endmodule\n").unwrap();

    let primary_script = write_script(&dir, "verible_primary.sh", ECHO_INIT_SCRIPT);
    let secondary_script = write_script(&dir, "slang-server", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list {:?})))",
            primary_script
        ),
    );
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-secondary-server-alist (cons 'verilog-mode (list {:?})))",
            secondary_script
        ),
    );

    ok(&mut i, &format!("(find-file {:?})", alu.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "2",
        "both the primary's and the secondary's autostart must be pending"
    );

    let primary_root = run(
        &mut i,
        &format!(
            "(cdr (assoc {:?} (mapcar 'car lsp--autostart-pending)))",
            primary_script
        ),
    );
    let secondary_root = run(
        &mut i,
        &format!(
            "(cdr (assoc {:?} (mapcar 'car lsp--autostart-pending)))",
            secondary_script
        ),
    );
    assert_eq!(
        primary_root,
        format!("{:?}", dir.join("rtl").to_str().unwrap()),
        "the filelist-style primary must stay at the nearest filelist ancestor"
    );
    assert_eq!(
        secondary_root,
        format!("{:?}", dir.to_str().unwrap()),
        "the workspace-style secondary must widen to the filelist component's root"
    );
    assert_ne!(
        primary_root, secondary_root,
        "primary and secondary must be keyed at different roots"
    );

    // FIX 5 (M132 fix round, review #6): every existing assertion here
    // reads the GLOBAL `lsp--autostart-pending', never the buffer-local
    // `lsp--autostart-pending-here' that `lsp--autostart-buffer-
    // matches-p' actually fills (lsp.el:4317) -- the mode-line
    // indicator's own data source. Reverting that predicate's ROOT
    // comparison back to `(equal (lsp--project-root file) root)' would
    // make it wrongly say "no match" for the workspace-style
    // secondary's widened root (`dir', not the buffer's own plain
    // `lsp--project-root' of `dir/rtl'), so this buffer's own pending
    // marker for the secondary key would never be set -- caught here
    // because this same buffer is the one that started BOTH handshakes.
    let secondary_key_pending = run(
        &mut i,
        &format!(
            "(and (member (cons {:?} {}) lsp--autostart-pending-here) t)",
            secondary_script, secondary_root
        ),
    );
    assert_eq!(
        secondary_key_pending, "t",
        "this buffer's own lsp--autostart-pending-here must contain the \
         workspace-style secondary's (command . widened-root) key"
    );

    // Let both complete.
    dispatch_one_pending(&mut i);
    dispatch_one_pending(&mut i);

    // FIX 4 (M132 fix round, review #5): before this assertion, this
    // test only ever iterated `lsp--buffer-clients' to KILL every
    // connection -- it never checked what, if anything, they actually
    // are. Per the reviewer's reading, `lsp--autostart-begin''s
    // completion closure calling `lsp--auto-attach-backfill' is the
    // ONLY mechanism that attaches THIS buffer (the one whose idle tick
    // started both handshakes) to its own new connections, and
    // `lsp--auto-attach-backfill-matches-p' (lsp.el:3567-3568) is what
    // decides the match using `lsp--project-root-for-command' (the
    // per-command root) rather than plain `lsp--project-root'.
    // Reverting that one line to `(lsp--project-root file)' would make
    // the `workspace'-style secondary's root ("proj") mismatch the
    // buffer's own plain project root ("proj/rtl"), so the secondary
    // would silently never attach here -- and this is the first
    // assertion in this file that would notice.
    let has_primary = run(
        &mut i,
        &format!(
            "(and (member {:?} (mapcar #'lsp--client-command (lsp--effective-buffer-clients))) t)",
            primary_script
        ),
    );
    let has_secondary = run(
        &mut i,
        &format!(
            "(and (member {:?} (mapcar #'lsp--client-command (lsp--effective-buffer-clients))) t)",
            secondary_script
        ),
    );
    assert_eq!(
        has_primary, "t",
        "the buffer that started autostart must end up attached to the \
         filelist-style primary's connection"
    );
    assert_eq!(
        has_secondary, "t",
        "the buffer that started autostart must end up attached to the \
         workspace-style secondary's connection"
    );

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}

#[test]
fn auto_attach_uses_the_per_command_root_for_a_workspace_style_server() {
    // Same fixture as above, but exercising `lsp--auto-attach-client'
    // (M132's lsp.el:3011 site): connect buffer A (proj/verif/tb.sv) to
    // a `workspace'-style PRIMARY (basename `slang-server') via a real
    // `M-x lsp', then confirm `lsp--auto-attach-client' finds that SAME
    // live connection for buffer B (proj/rtl/core/alu.sv) -- a
    // DIFFERENT file, with a DIFFERENT narrow `lsp--project-root'
    // (`proj/rtl' vs `proj/verif'), but the SAME widened `workspace'
    // root (`proj') since both file lists share `core/alu.sv'.
    //
    // Before M132, `lsp--auto-attach-client' looked up
    // `(lsp--get-connection command (lsp--project-root file))' -- the
    // plain, narrow root -- so this exact case (one server, one widened
    // root, two buffers reaching it through two different narrower file
    // lists) would miss the live connection entirely and return nil.
    let mut i = setup();
    let dir = scratch_dir("m132_auto_attach_workspace_root");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/core")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/top")).unwrap();
    std::fs::create_dir_all(dir.join("verif")).unwrap();
    std::fs::write(
        dir.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = dir.join("rtl/core/alu.sv");
    let tb = dir.join("verif/tb.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        dir.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(&tb, "module tb; endmodule\n").unwrap();

    let script = write_script(&dir, "slang-server", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list {:?})))",
            script
        ),
    );

    // Buffer A: connect for real via `M-x lsp'.
    ok(&mut i, &format!("(find-file {:?})", tb.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    ok(&mut i, "(lsp)");
    assert_ne!(run(&mut i, "lsp--buffer-client"), "nil");
    ok(&mut i, "(setq test--m132-client lsp--buffer-client)");

    // Buffer B: a different file, never connected itself.
    ok(&mut i, &format!("(find-file {:?})", alu.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");

    let found = run(
        &mut i,
        &format!(
            "(eq (lsp--auto-attach-client {:?} 'verilog-mode) test--m132-client)",
            alu.to_str().unwrap()
        ),
    );
    assert_eq!(
        found, "t",
        "lsp--auto-attach-client must find buffer A's connection for buffer B, \
         since both resolve to the same widened workspace root"
    );

    ok(&mut i, "(lsp-kill (lsp--client-conn test--m132-client))");
}

#[test]
fn autostart_warns_about_duplicate_declarations_under_a_widened_root() {
    // FIX 3 (M132 fix round, review #4): the second of
    // `lsp--workspace-maybe-warn-duplicates''s two real call sites is
    // `lsp--autostart-begin''s success path (lsp.el:4547, inside the
    // `lsp-request-async' completion callback), and nothing in this
    // file exercised it before this test -- every M132 test above
    // reads `lsp--autostart-pending'/`lsp--connections' state, never
    // `message' output. Deleting that call line must make THIS test
    // fail.
    //
    // Same widened-root fixture as `lsp_connect_warns_about_duplicate_
    // declarations_under_a_widened_root' (lsp_mode_tests.rs): rtl/ and
    // verif/ share `core/alu.sv' via their own `verible.filelist's, so
    // a `workspace'-style command (`slang-server' basename) widens to
    // the top `dir', capped at `.git' -- plus one duplicate module
    // (`dup_widened') declared under `rtl/top' and again under
    // `verif`, which only the WIDENED root covers.
    let mut i = setup();
    let dir = scratch_dir("m132_fix3_autostart_warns");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/core")).unwrap();
    std::fs::create_dir_all(dir.join("rtl/top")).unwrap();
    std::fs::create_dir_all(dir.join("verif")).unwrap();
    std::fs::write(
        dir.join("rtl/verible.filelist"),
        "core/alu.sv\ntop/soc_top.sv\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/verible.filelist"),
        "../rtl/core/alu.sv\ntb.sv\n",
    )
    .unwrap();
    let alu = dir.join("rtl/core/alu.sv");
    std::fs::write(&alu, "module alu; endmodule\n").unwrap();
    std::fs::write(
        dir.join("rtl/top/soc_top.sv"),
        "module soc_top; endmodule\n",
    )
    .unwrap();
    std::fs::write(dir.join("verif/tb.sv"), "module tb; endmodule\n").unwrap();
    // The actual duplicate the widened root (but neither individual
    // filelist) covers.
    std::fs::write(
        dir.join("rtl/top/extra_a.sv"),
        "module dup_widened; endmodule\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("verif/extra_b.sv"),
        "module dup_widened; endmodule\n",
    )
    .unwrap();

    // `verilog-mode' already has REAL default entries in both
    // `lsp-server-alist' (`verible-verilog-ls') and `lsp-secondary-
    // server-alist' (`slang-server', unqualified) -- and both binaries
    // are actually installed on this machine's PATH, so leaving either
    // default in place would autostart a real server race instead of
    // this test's controlled fake. Override BOTH explicitly (same
    // shape as `autostart_gives_primary_and_secondary_different_roots'
    // above) so `lsp--server-for-mode'/`lsp--secondary-server-for-mode'
    // (both plain `assq', first match wins, and `add-to-list' prepends)
    // resolve to these two fakes instead.
    let primary_script = write_script(&dir, "verible_primary.sh", ECHO_INIT_SCRIPT);
    let secondary_script = write_script(&dir, "slang-server", ECHO_INIT_SCRIPT);
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-server-alist (cons 'verilog-mode (list {:?})))",
            primary_script
        ),
    );
    ok(
        &mut i,
        &format!(
            "(add-to-list 'lsp-secondary-server-alist (cons 'verilog-mode (list {:?})))",
            secondary_script
        ),
    );

    capture_messages(&mut i);
    ok(&mut i, &format!("(find-file {:?})", alu.to_str().unwrap()));
    ok(&mut i, "(major-mode-internal-set 'verilog-mode)");
    set_frontend_started(&mut i);

    ok(&mut i, "(lsp--autostart-tick)");
    assert_eq!(
        run(&mut i, "(length lsp--autostart-pending)"),
        "2",
        "both the filelist-style primary's and the workspace-style \
         secondary's autostart must be pending"
    );
    dispatch_one_pending(&mut i);
    dispatch_one_pending(&mut i);

    let messages = run(&mut i, "test--messages");
    assert!(
        messages.contains("dup_widened"),
        "expected the duplicate-declaration warning to have been \
         messaged via the real lsp--autostart-begin completion path \
         (the workspace-style secondary only -- the filelist-style \
         primary's own narrower root never contains the duplicate): {}",
        messages
    );

    ok(
        &mut i,
        "(dolist (c lsp--buffer-clients) (lsp-kill (lsp--client-conn c)))",
    );
}

// ============================================================
// M135 Part E: a guard against the fixed-`sleep` family (F9/F10 in
// PLAN.md) creeping back into this file. Reads THIS FILE'S OWN SOURCE
// and fails loudly if any thread-sleep call site's argument is >= 500ms
// -- the pacing sleeps this file legitimately keeps (5/20/50/100ms) are
// all well under that, and the fake `python3` echo server's own 1-second
// delay (`DELAYED_ECHO_INIT_SCRIPT`) is a Python string constant, not a
// Rust sleep call, so it is untouched by (and irrelevant to) this guard,
// and it is deliberate (see that const's own doc comment).
//
// This is the deletion-question answer for Part B/C/D as a whole: revert
// any one of those fixes back to a fixed long sleep, and THIS test goes
// red and names the line -- without it, a regression back to `sleep
// (1500)` anywhere in this file would pass silently on an idle machine
// (only failing under load, which is exactly the false-negative shape
// this whole milestone exists to close).
#[test]
fn guard_no_long_thread_sleeps_in_this_file() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/lsp_autostart_tests.rs");
    let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "guard_no_long_thread_sleeps_in_this_file: could not read its own \
             source at {:?}: {}",
            path, e
        )
    });

    // M114 self-check floor: a mechanism that decides what to check must
    // fail loudly if it ends up checking nothing. Built by concatenating
    // parts at runtime (not spelled as one literal) so this floor check
    // itself doesn't shadow-satisfy the very thing it's guarding against.
    assert!(
        !content.is_empty(),
        "guard lost its target: read an empty file"
    );
    let line_count = content.lines().count();
    assert!(
        line_count > 1500,
        "guard lost its target: only {} lines read, expected > 1500 -- \
         wrong path, or this file got drastically smaller",
        line_count
    );
    let sentinel: String = ["fn ", "dispatch_one_pending"].concat();
    assert!(
        content.contains(&sentinel),
        "guard lost its target: sentinel {:?} not found in the file it read",
        sentinel
    );

    // Built from parts, not one literal -- see this test's own doc
    // comment above for why (a literal copy here would make every line
    // of THIS function itself look like a hit).
    //
    // M135 fix round (F4, trailing cold review): the marker used to be
    // built ONLY from the fully-qualified path's own two segments (the
    // module path, then the function name plus its open paren). A cold
    // reviewer pointed out that importing the module first and calling
    // the function unqualified afterward -- valid Rust, and arguably
    // the more idiomatic spelling once the module is already imported
    // -- would silently sail past that marker entirely, since the
    // fully-qualified prefix wouldn't appear at the call site at all.
    // Narrowed the first segment down to just the module name's own
    // trailing `::`, which is a substring of both spellings, so it
    // still catches every existing call site in this file (all fully-
    // qualified) as well as the shorter, unqualified-call spelling.
    //
    // Known remaining bypasses, written down honestly rather than
    // implied to be closed: this is a textual scan, not a parse, so (a)
    // importing the function itself under a different local name and
    // calling it under THAT name never contains this guard's marker
    // text at the call site at all, and is invisible to it, and (b)
    // wrapping the real call inside a locally-defined helper function
    // (so only the helper's own definition contains the marker, and
    // every call site just names the helper with its own duration
    // argument) is likewise invisible once the helper's own definition
    // line no longer spells the marker out verbatim, or if the helper
    // lives in a different file this guard doesn't read. Closing those
    // would need an actual AST-level check, which is out of proportion
    // to what this guard is for -- a slow leak, not a promise that
    // regressions here are structurally impossible.
    let sleep_marker: String = ["thread::", "sleep("].concat();
    let millis_marker = "from_millis(";
    let secs_marker = "from_secs(";

    let mut violations: Vec<(usize, String)> = Vec::new();
    let mut unparseable: Vec<(usize, String)> = Vec::new();
    for (lineno, line) in content.lines().enumerate() {
        let Some(sleep_idx) = line.find(&sleep_marker) else {
            continue;
        };
        // Only look for the duration marker AFTER the sleep call itself
        // starts on this line -- found the hard way (F4 fix round,
        // self-check while verifying this guard): an earlier, unrelated
        // `Duration::from_millis(N)` on the SAME line (e.g. a `let`
        // binding for some other duration, immediately followed on the
        // same line by a bare-variable sleep call using a DIFFERENT
        // variable) would otherwise get picked up as if it were the
        // sleep's own argument, silently treating a genuinely
        // unparseable call (a bare variable, whose real value this
        // guard cannot see) as a harmless, already-small duration
        // instead of failing loudly.
        let after_sleep = &line[sleep_idx..];
        let ms: Option<u64> = if let Some(idx) = after_sleep.find(millis_marker) {
            let rest = &after_sleep[idx + millis_marker.len()..];
            rest.find(')')
                .and_then(|end| rest[..end].trim().parse::<u64>().ok())
        } else if let Some(idx) = after_sleep.find(secs_marker) {
            let rest = &after_sleep[idx + secs_marker.len()..];
            rest.find(')')
                .and_then(|end| rest[..end].trim().parse::<u64>().ok())
                .map(|secs| secs * 1000)
        } else {
            None
        };
        match ms {
            Some(ms) if ms >= 500 => violations.push((lineno + 1, line.to_string())),
            Some(_) => {}
            None => unparseable.push((lineno + 1, line.to_string())),
        }
    }

    assert!(
        unparseable.is_empty(),
        "guard could not determine the duration of a thread-sleep call, \
         so it cannot be sure it's under 500ms -- inspect and fix by hand: {:?}",
        unparseable
    );
    assert!(
        violations.is_empty(),
        "found thread-sleep call(s) with an argument >= 500ms, which this \
         milestone (M135) exists to remove -- replace with a bounded poll \
         (pump_until / drain_frames_until / lsp-events-delivered) instead: {:?}",
        violations
    );
}
