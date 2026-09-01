//! Multi-process parallelism (the "no shared mutable state, message
//! passing" model — like a Web Worker or an Erlang actor, and exactly
//! how the real Emacs ecosystem does async: async.el, LSP servers, etc).
//!
//! A worker is a child process running this same executable in
//! `--worker` mode: it reads length-prefixed serialized S-expressions on
//! stdin, evaluates each in its own fresh, persistent interpreter, and
//! writes back a length-prefixed serialized result. The only thing that
//! ever crosses the process boundary is text (serialized sexps), so the
//! parent's `Rc<RefCell>`-based `Value` graph — which is deliberately
//! `!Send`/`!Sync` — never has to move or be shared. Nothing here needs
//! `unsafe`, and the core memory model is untouched.
//!
//! Why processes and not threads:
//!   * A worker runs user-written elisp, which *will* eventually contain
//!     an infinite loop. Rust has no safe way to force-terminate a
//!     thread, but `kill -9` on a process is guaranteed by the OS — so
//!     `worker-kill` is a promise we can actually keep.
//!   * Crash isolation: a worker that corrupts memory (e.g. a JIT
//!     codegen bug in the `unsafe` native path) takes down only itself;
//!     the parent just sees the pipe close and reports an error.
//!   * A worker's entire `Rc` heap (leaked cycles and all) is reclaimed
//!     by the OS when it exits — free, thorough GC as a side effect.
//!
//! A background reader thread per worker moves *strings* (never
//! `Value`s) from the child's stdout into an mpsc channel the main
//! thread polls; strings are `Send`, so this stays safe.

use std::cell::RefCell;
use std::io::{BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::builtins::defun;
use crate::error::Flow;
use crate::interp::Interp;
use crate::printer::prin1_to_string;
use crate::reader::Reader;
use crate::value::{ExtRef, Value};

pub const WORKER_TAG: &str = "worker";

/// Max serialized message size (64 MiB). Guards the parent against a
/// corrupt/hostile length prefix asking it to allocate absurd buffers.
const MAX_FRAME: usize = 64 * 1024 * 1024;

// --- Length-prefixed framing (4-byte big-endian length, then payload) ---
// Newlines can appear inside string values, so line-delimited framing
// would corrupt the stream; an explicit length prefix cannot.

fn write_frame(w: &mut impl Write, payload: &str) -> std::io::Result<()> {
    let bytes = payload.as_bytes();
    w.write_all(&(bytes.len() as u32).to_be_bytes())?;
    w.write_all(bytes)?;
    w.flush()
}

/// Reads one frame. `Ok(None)` on a clean EOF (peer closed the pipe).
fn read_frame(r: &mut impl Read) -> std::io::Result<Option<String>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "worker frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "worker frame not utf-8"))
}

// --- Worker side: the `--worker` process main loop ---

/// Run as a worker: evaluate framed requests from stdin, write framed
/// responses to stdout, until stdin closes. The interpreter persists
/// across requests, so a worker can `defun` something and then use it.
pub fn run_worker_loop(interp: &mut Interp) {
    // stdout is the protocol channel — any stray `message`/`princ`
    // output there would corrupt it, so redirect all interpreter output
    // to stderr.
    interp.output = Some(Box::new(|s: &str| eprint!("{}", s)));

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    loop {
        let msg = match read_frame(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => break, // parent closed stdin: exit cleanly
            Err(_) => break,
        };
        let response = eval_request(interp, &msg);
        if write_frame(&mut writer, &response).is_err() {
            break; // parent went away
        }
    }
}

/// Evaluate one request string, return the serialized response:
/// `(ok . VALUE)` on success, `(error . "message")` on any failure.
fn eval_request(interp: &mut Interp, msg: &str) -> String {
    let ok = interp.intern("ok");
    let response = match parse_and_eval(interp, msg) {
        Ok(v) => Value::cons(Value::Sym(ok), v),
        Err(text) => Value::cons(Value::Sym(interp.syms.error), Value::string(text)),
    };
    let s = prin1_to_string(interp, &response);
    // Guarantee the parent can always parse what we send: some values
    // (closures, hash tables, ...) print as `#<...>`, which the reader
    // can't round-trip. Verify here and downgrade to an error response
    // rather than handing the parent an unparseable frame.
    let mut verify = Reader::new(&s);
    match verify.read(interp) {
        Ok(Some(_)) => s,
        _ => {
            let err = Value::cons(
                Value::Sym(interp.syms.error),
                Value::string("worker result is not serializable"),
            );
            prin1_to_string(interp, &err)
        }
    }
}

fn parse_and_eval(interp: &mut Interp, msg: &str) -> Result<Value, String> {
    let mut reader = Reader::new(msg);
    let form = match reader.read(interp) {
        Ok(Some(f)) => f,
        Ok(None) => return Err("empty worker request".to_string()),
        Err(_) => return Err("worker request parse error".to_string()),
    };
    match crate::eval::eval(interp, &form, &None) {
        Ok(v) => Ok(v),
        Err(flow) => Err(interp.describe_flow(&flow)),
    }
}

// --- Parent side: worker handle + management ---

enum WorkerEvent {
    /// One serialized response frame from the child.
    Response(String),
    /// The child's stdout closed (exited or was killed).
    Died,
}

pub struct WorkerHandle {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<WorkerEvent>,
    pending: usize,
    alive: bool,
}

impl WorkerHandle {
    pub fn spawn() -> std::io::Result<WorkerHandle> {
        // M60 testability: a worker is normally re-exec'ing THIS process
        // in `--worker` mode, so `current_exe()` is correct in
        // production -- but under `cargo test`, `current_exe()` resolves
        // to the *test harness* binary, not `reticle`, and there's
        // no `--worker` mode for it to enter. `crates/core/tests/
        // background_output_tests.rs` sets this env var to the real
        // `reticle` binary it built itself so `worker-start` is
        // actually exercisable from an integration test; unset in every
        // other context, where behavior is unchanged.
        let exe = match std::env::var_os("RETICLE_WORKER_EXE") {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_exe()?,
        };
        let mut child = Command::new(exe)
            .arg("--worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // M60: was inherited so the child's stderr (which also
            // carries every `(message ...)` a worker evaluates --
            // `run_worker_loop` below redirects `interp.output` there so
            // stdout stays a clean protocol channel) stayed visible.
            // Piped and drained into `crate::bglog` instead: an
            // inherited child stderr writes straight past the TUI's
            // diff-based `Grid` redisplay and never gets overwritten
            // (see the parallel comment in lsp.rs's `spawn`).
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        // Reader thread: only ever moves `String`s (Send) across the
        // thread boundary — never a Value — so this needs no unsafe and
        // touches none of the parent's Rc graph.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            loop {
                match read_frame(&mut r) {
                    Ok(Some(s)) => {
                        if tx.send(WorkerEvent::Response(s)).is_err() {
                            break; // parent dropped the handle
                        }
                    }
                    Ok(None) | Err(_) => {
                        let _ = tx.send(WorkerEvent::Died);
                        break;
                    }
                }
            }
        });

        // M60: forwards the worker's stderr into the process-global
        // background log instead of the terminal. Exits on its own once
        // the child's stderr hits EOF -- no join handle needed.
        std::thread::spawn(move || {
            crate::bglog::pump_lines(BufReader::new(stderr), "worker");
        });

        Ok(WorkerHandle {
            child,
            stdin: Some(stdin),
            events: rx,
            pending: 0,
            alive: true,
        })
    }

    pub fn send(&mut self, form_str: &str) -> std::io::Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "worker stdin closed")
        })?;
        write_frame(stdin, form_str)?;
        self.pending += 1;
        Ok(())
    }

    fn note(&mut self, ev: &WorkerEvent) {
        match ev {
            WorkerEvent::Response(_) => self.pending = self.pending.saturating_sub(1),
            WorkerEvent::Died => self.alive = false,
        }
    }

    fn try_recv(&mut self) -> Option<WorkerEvent> {
        match self.events.try_recv() {
            Ok(ev) => {
                self.note(&ev);
                Some(ev)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.alive = false;
                None
            }
        }
    }

    fn recv_blocking(&mut self) -> Option<WorkerEvent> {
        match self.events.recv() {
            Ok(ev) => {
                self.note(&ev);
                Some(ev)
            }
            Err(_) => {
                self.alive = false;
                None
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

    pub fn pending(&self) -> usize {
        self.pending
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        // Closing stdin makes the worker see EOF and exit cleanly; then
        // reap it so we don't leave a zombie.
        self.stdin = None;
        let _ = self.child.wait();
    }
}

// --- Elisp builtins ---

fn worker_arg(interp: &mut Interp, v: &Value) -> Result<Rc<RefCell<WorkerHandle>>, Flow> {
    v.as_ext::<RefCell<WorkerHandle>>(WORKER_TAG)
        .ok_or_else(|| interp.wrong_type("worker-p", v))
}

/// Parse a response frame back into a Value; unreadable frames become an
/// `(error . "...")` cons so the caller always gets a well-formed result.
fn parse_response(interp: &mut Interp, s: &str) -> Value {
    let owned = s.to_string();
    let mut r = Reader::new(&owned);
    match r.read(interp) {
        Ok(Some(v)) => v,
        _ => Value::cons(
            Value::Sym(interp.syms.error),
            Value::string(format!("unreadable worker result: {}", s)),
        ),
    }
}

fn dead_value(interp: &mut Interp) -> Value {
    Value::cons(
        Value::Sym(interp.syms.error),
        Value::string("worker process died"),
    )
}

pub fn register(interp: &mut Interp) {
    defun(
        interp,
        "worker-start",
        0,
        Some(0),
        |i, _| match WorkerHandle::spawn() {
            // WorkerHandle: thread join handle / channel endpoints, no Value.
            Ok(h) => Ok(Value::Ext(ExtRef::new(WORKER_TAG, RefCell::new(h), None))),
            Err(e) => Err(i.error(format!("cannot start worker: {}", e))),
        },
    );
    defun(interp, "worker-p", 1, Some(1), |i, a| {
        let is = a[0].as_ext::<RefCell<WorkerHandle>>(WORKER_TAG).is_some();
        Ok(Value::bool(is, i.syms.t))
    });
    // Send a form for asynchronous evaluation. Returns t; the result is
    // collected later via worker-poll / worker-wait (results come back
    // in order, since a worker evaluates its queue sequentially).
    defun(interp, "worker-eval", 2, Some(2), |i, a| {
        let h = worker_arg(i, &a[0])?;
        let form_str = prin1_to_string(i, &a[1]);
        h.borrow_mut()
            .send(&form_str)
            .map_err(|e| i.error(format!("worker send failed: {}", e)))?;
        Ok(Value::Sym(i.syms.t))
    });
    // Non-blocking: the next completed result as (ok . VALUE) /
    // (error . MESSAGE), or nil if nothing is ready yet. Use this from
    // the editor event loop so editing never blocks on a worker.
    defun(interp, "worker-poll", 1, Some(1), |i, a| {
        let h = worker_arg(i, &a[0])?;
        let ev = h.borrow_mut().try_recv();
        match ev {
            Some(WorkerEvent::Response(s)) => Ok(parse_response(i, &s)),
            Some(WorkerEvent::Died) => Ok(dead_value(i)),
            None => Ok(Value::Nil),
        }
    });
    // Blocking wait for the next result. Convenient for scripts; the
    // interactive editor should prefer worker-poll.
    defun(interp, "worker-wait", 1, Some(1), |i, a| {
        let h = worker_arg(i, &a[0])?;
        let ev = h.borrow_mut().recv_blocking();
        match ev {
            Some(WorkerEvent::Response(s)) => Ok(parse_response(i, &s)),
            _ => Ok(dead_value(i)),
        }
    });
    defun(interp, "worker-kill", 1, Some(1), |i, a| {
        let h = worker_arg(i, &a[0])?;
        h.borrow_mut().kill();
        Ok(Value::Sym(i.syms.t))
    });
    defun(interp, "worker-live-p", 1, Some(1), |i, a| {
        let h = worker_arg(i, &a[0])?;
        let alive = h.borrow().is_alive();
        Ok(Value::bool(alive, i.syms.t))
    });
    defun(interp, "worker-pending", 1, Some(1), |i, a| {
        let h = worker_arg(i, &a[0])?;
        let n = h.borrow().pending();
        Ok(Value::Int(n as i64))
    });
}
