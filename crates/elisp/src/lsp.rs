//! Low-level LSP transport (M14): spawn an arbitrary external command
//! (e.g. `rust-analyzer`) and speak JSON-RPC over its stdin/stdout using
//! the Content-Length-prefixed framing the Language Server Protocol
//! specifies. Protocol semantics -- the `initialize` handshake, document
//! sync, hover, go-to-definition, diagnostics tracking -- deliberately
//! are NOT here; they're pure elisp in `crates/core/lisp/lsp.el` calling
//! these primitives, the same division of labor as M11's library layer
//! and M12's `treesit.el`.
//!
//! This mirrors `worker.rs`'s shape (spawn, background reader thread
//! moving only `String`s, mpsc channel, poll/wait) but for an arbitrary
//! external program speaking a different wire format (Content-Length
//! headers + JSON instead of our own 4-byte length prefix + printed
//! sexps), so it's its own module rather than a forced generalization of
//! `worker.rs` over two genuinely different protocols.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crate::builtins::{defun, need_list, need_str, opt};
use crate::error::Flow;
use crate::interp::Interp;
use crate::value::{ExtRef, Value};

pub const LSP_TAG: &str = "lsp-connection";

/// Guards against a hostile/corrupt Content-Length asking for an absurd
/// allocation, same purpose as `worker.rs`'s `MAX_FRAME`.
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

fn write_message(w: &mut impl Write, json_text: &str) -> std::io::Result<()> {
    let bytes = json_text.as_bytes();
    write!(w, "Content-Length: {}\r\n\r\n", bytes.len())?;
    w.write_all(bytes)?;
    w.flush()
}

/// Reads one Content-Length-framed message. `Ok(None)` on a clean EOF
/// before any header arrives (the server exited).
fn read_message(r: &mut impl BufRead) -> std::io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
        // Other headers (e.g. Content-Type) are ignored: exactly one
        // appears in practice among real servers, and it's always utf-8
        // JSON.
    }
    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "lsp message missing Content-Length",
        )
    })?;
    if len > MAX_MESSAGE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "lsp message too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "lsp message not utf-8"))
}

enum LspEvent {
    Message(String),
    Died,
}

pub struct LspConnection {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<LspEvent>,
    alive: bool,
}

impl LspConnection {
    pub fn spawn(cmd: &str, args: &[String]) -> std::io::Result<LspConnection> {
        let mut child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // M60: the server's own log output used to be inherited
            // straight through to the terminal so it stayed visible --
            // but the TUI's redisplay is diff-based against a `Grid` it
            // owns fully (crates/core/src/redisplay.rs), so anything
            // written outside that grid is never revisited and can
            // permanently corrupt the screen (this is exactly how a
            // verible-verilog-ls version banner used to bleed through
            // and stick around forever). Piped and drained into
            // `crate::bglog` instead, same shape as worker.rs's own
            // stderr handling below.
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        // Only ever moves Strings (Send) across the thread boundary,
        // never a Value -- same safety argument as worker.rs's reader.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match read_message(&mut r) {
                    Ok(Some(s)) => {
                        if tx.send(LspEvent::Message(s)).is_err() {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => {
                        let _ = tx.send(LspEvent::Died);
                        break;
                    }
                }
            }
        });

        // M60: forwards the server's stderr into the process-global
        // background log instead of the terminal. Tagged with CMD so
        // multiple LSP connections (or an LSP connection alongside a
        // worker) stay distinguishable in `*background-output*`. Exits
        // on its own once the child's stderr hits EOF -- no join handle
        // needed, nothing to leak.
        let tag = cmd.to_string();
        std::thread::spawn(move || {
            crate::bglog::pump_lines(BufReader::new(stderr), &tag);
        });

        Ok(LspConnection {
            child,
            stdin: Some(stdin),
            events: rx,
            alive: true,
        })
    }

    pub fn send(&mut self, json_text: &str) -> std::io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "lsp connection stdin closed",
            )
        })?;
        write_message(stdin, json_text)
    }

    fn try_recv(&mut self) -> Option<LspEvent> {
        match self.events.try_recv() {
            Ok(ev) => {
                if matches!(ev, LspEvent::Died) {
                    self.alive = false;
                }
                Some(ev)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.alive = false;
                None
            }
        }
    }

    fn recv_blocking(&mut self) -> Option<LspEvent> {
        match self.events.recv() {
            Ok(ev) => {
                if matches!(ev, LspEvent::Died) {
                    self.alive = false;
                }
                Some(ev)
            }
            Err(_) => {
                self.alive = false;
                None
            }
        }
    }

    /// Bounded wait for the next message: distinguishes "a message
    /// arrived", "the timeout elapsed with the connection still alive"
    /// (`Ok(None)`), and "the connection is dead" (`Err(())`, same
    /// `alive = false` bookkeeping as `try_recv`/`recv_blocking`). The
    /// caller (`lsp-wait`) needs all three kept apart: a timeout must NOT
    /// be reported as a dead server, or a slow-but-healthy server would
    /// look identical to a crashed one.
    fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<LspEvent>, ()> {
        match self.events.recv_timeout(timeout) {
            Ok(ev) => {
                if matches!(ev, LspEvent::Died) {
                    self.alive = false;
                }
                Ok(Some(ev))
            }
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                self.alive = false;
                Err(())
            }
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stdin = None;
        self.alive = false;
    }

    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// The OS process id `Command::spawn` assigned this connection's
    /// child, straight from `Child::id()` -- a field already recorded at
    /// spawn time, not a query that has to wait for the child to run,
    /// answer anything, or even be scheduled at all. Exists (M65 review
    /// round 2) so tests needing OS-level proof a process is really dead
    /// (`kill -0`) don't have to get that PID out of the child itself
    /// (e.g. a fake server writing its own `$$` to a file) -- under
    /// parallel test-suite load a starved shell can be SIGKILLed before
    /// it ever reaches its first line, so a file it was supposed to
    /// write may simply never appear, no matter how long a test polls
    /// for it. Valid for the lifetime of `LspConnection` regardless of
    /// `alive`: a `Child`'s recorded pid doesn't change or become
    /// invalid just because the process has since exited or been
    /// killed.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for LspConnection {
    fn drop(&mut self) {
        // M65: dropping this connection means nobody holds it anymore --
        // there can be no future reader for its output or sender of
        // requests to it. A server that ignores its stdin closing (the
        // deaf-server case this milestone is about) would otherwise make
        // `child.wait()` block forever right here, in a destructor, with
        // no way for the caller to even notice. `kill()` first guarantees
        // this returns instead of leaving an orphaned server process that
        // nothing can ever talk to again (PLAN.md M63: "connection and
        // child process live until the process exits").
        let _ = self.child.kill();
        self.stdin = None;
        let _ = self.child.wait();
    }
}

fn conn_arg(interp: &mut Interp, v: &Value) -> Result<Rc<RefCell<LspConnection>>, Flow> {
    v.as_ext::<RefCell<LspConnection>>(LSP_TAG)
        .ok_or_else(|| interp.wrong_type("lsp-connection-p", v))
}

/// `Duration` from a `lsp-wait` TIMEOUT argument: accepts an integer, a
/// bignum, or a float, matching the numeric types `float-time`/
/// arithmetic already produce so callers doing `(- deadline
/// (float-time))` don't need to coerce anything themselves.
///
/// M65 review fix: this used to go through `f64` and hand the result
/// straight to `Duration::from_secs_f64`, which PANICS -- taking down
/// the whole process, unsaved edits included -- on any non-finite or
/// out-of-range value (`f64::MAX`, `1e20`, and the bignum-overflow
/// fallback used to deliberately produce exactly `f64::MAX`, so that
/// fallback was guaranteed to crash despite its own comment claiming it
/// was harmless). Every case below is defined to NEVER panic:
///
///   - NaN -> `Duration::ZERO` (one non-blocking check), spelled out
///     explicitly rather than relying on a comparison's behavior on NaN
///     to fall out the right way by accident.
///   - <= 0 (including negative) -> `Duration::ZERO`, same as before.
///   - Finite and representable -> that many seconds, via
///     `Duration::try_from_secs_f64` (never panics; returns `Err` for
///     what it can't represent instead).
///   - Infinite, or finite but too large for `Duration` to represent
///     (`Duration::try_from_secs_f64` erroring), or a bignum too large
///     for `f64` at all -> `Duration::MAX` (~584 billion years):
///     effectively unbounded, but still routed through the bounded
///     `recv_timeout` call, NOT a fallback to the old unconditionally-
///     blocking `recv_blocking` path.
fn need_wait_duration(interp: &mut Interp, v: &Value) -> Result<Duration, Flow> {
    let secs = match v {
        Value::Int(i) => *i as f64,
        Value::Big(b) => num_traits::ToPrimitive::to_f64(b.as_ref()).unwrap_or(f64::INFINITY),
        Value::Float(f) => *f,
        _ => return Err(interp.wrong_type("numberp", v)),
    };
    if secs.is_nan() || secs <= 0.0 {
        // NaN is spelled out explicitly (`is_nan`) rather than leaned
        // on via a negated `>' comparison -- clippy's
        // `neg_cmp_op_on_partial_ord' flags that shape as easy to
        // misread on a partially-ordered type, and this is exactly the
        // NaN-handling case the lint exists for. Zero and negative
        // values fall out of the plain `<=' half.
        return Ok(Duration::ZERO);
    }
    Ok(Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX))
}

fn dead_value(interp: &mut Interp) -> Value {
    Value::cons(
        Value::Sym(interp.syms.error),
        Value::string("lsp server process died"),
    )
}

fn parse_message(interp: &mut Interp, s: &str) -> Result<Value, Flow> {
    let j: serde_json::Value = serde_json::from_str(s)
        .map_err(|e| interp.error(format!("lsp: malformed message from server: {}", e)))?;
    Ok(crate::json::from_json(interp, &j))
}

pub fn register(interp: &mut Interp) {
    defun(interp, "lsp-start", 1, Some(2), |i, a| {
        let cmd = need_str(i, &a[0])?;
        let args: Vec<String> = match opt(a, 1) {
            Value::Nil => Vec::new(),
            v => need_list(i, &v)?
                .iter()
                .map(|x| need_str(i, x).map(|s| s.to_string()))
                .collect::<Result<_, _>>()?,
        };
        match LspConnection::spawn(&cmd, &args) {
            // LspConnection: process handle / streams / pending-request
            // bookkeeping, no Value.
            Ok(conn) => Ok(Value::Ext(ExtRef::new(LSP_TAG, RefCell::new(conn), None))),
            Err(e) => Err(i.error(format!("lsp-start: cannot spawn {}: {}", cmd, e))),
        }
    });
    defun(interp, "lsp-connection-p", 1, Some(1), |i, a| {
        let is = a[0].as_ext::<RefCell<LspConnection>>(LSP_TAG).is_some();
        Ok(Value::bool(is, i.syms.t))
    });
    // Serializes VALUE to JSON and Content-Length-frames it to the
    // server. Callers build VALUE with hash-tables/vectors/:null/:false,
    // matching json-serialize's own mapping (see crate::json).
    defun(interp, "lsp-send", 2, Some(2), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        let json_text = crate::json::to_json(i, &a[1]).to_string();
        conn.borrow_mut()
            .send(&json_text)
            .map_err(|e| i.error(format!("lsp-send: {}", e)))?;
        Ok(Value::Sym(i.syms.t))
    });
    // Non-blocking: the next message already received, parsed from JSON,
    // or nil if nothing is ready. Use this from the editor's idle loop.
    defun(interp, "lsp-poll", 1, Some(1), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        let ev = conn.borrow_mut().try_recv();
        match ev {
            Some(LspEvent::Message(s)) => parse_message(i, &s),
            Some(LspEvent::Died) => Ok(dead_value(i)),
            None => Ok(Value::Nil),
        }
    });
    // Wait for the next message. lsp.el uses this to wait out the
    // one-time `initialize` handshake and synchronous requests.
    //
    // TIMEOUT (M65) is optional and in seconds (int, bignum, or float):
    // nil/omitted keeps the old unbounded-block behavior unchanged, so
    // every pre-M65 caller and test is unaffected. A non-nil TIMEOUT
    // that elapses with no message available returns nil -- naturally
    // distinct from both a parsed message (a hash table) and a dead
    // server (`dead_value`'s `(error . ...)` cons), so callers can tell
    // all three apart with `hash-table-p'/`consp' checks. A zero or
    // negative TIMEOUT is treated as "one non-blocking check": it maps
    // to `Duration::ZERO`, which `recv_timeout` treats the same as any
    // other already-elapsed deadline (returns immediately with whatever
    // is or isn't already queued) rather than being special-cased or
    // rejected.
    defun(interp, "lsp-wait", 1, Some(2), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        let timeout = match opt(a, 1) {
            Value::Nil => None,
            v => Some(need_wait_duration(i, &v)?),
        };
        match timeout {
            None => {
                let ev = conn.borrow_mut().recv_blocking();
                match ev {
                    Some(LspEvent::Message(s)) => parse_message(i, &s),
                    _ => Ok(dead_value(i)),
                }
            }
            Some(dur) => match conn.borrow_mut().recv_timeout(dur) {
                Ok(Some(LspEvent::Message(s))) => parse_message(i, &s),
                Ok(Some(LspEvent::Died)) => Ok(dead_value(i)),
                Ok(None) => Ok(Value::Nil),
                Err(()) => Ok(dead_value(i)),
            },
        }
    });
    defun(interp, "lsp-kill", 1, Some(1), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        conn.borrow_mut().kill();
        Ok(Value::Sym(i.syms.t))
    });
    defun(interp, "lsp-live-p", 1, Some(1), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        let alive = conn.borrow().is_alive();
        Ok(Value::bool(alive, i.syms.t))
    });
    // M65 review round 2: the OS pid of CONN's child process, for tests
    // that need to independently confirm (`kill -0`) a process is
    // really gone rather than trusting this crate's own `alive`
    // bookkeeping. See `LspConnection::pid`'s doc comment for why this
    // is race-free where asking the child itself to report its own pid
    // (e.g. writing `$$` to a file) is not.
    defun(interp, "lsp-connection-pid", 1, Some(1), |i, a| {
        let conn = conn_arg(i, &a[0])?;
        let pid = conn.borrow().pid();
        Ok(Value::Int(pid as i64))
    });
}
