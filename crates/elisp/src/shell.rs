//! M23: raw external-process transport for shell buffers (eshell).
//! Unlike lsp.rs (Content-Length JSON-RPC) and worker.rs (length-
//! prefixed sexps), this transport is unframed: reader threads forward
//! arbitrary output chunks as they arrive — a progress bar with no
//! newline still streams. The exit status is captured (both framed
//! transports discard it).
//!
//! M79 (Rust primitives half): `start-shell-process` grew two optional
//! arguments, STDIN and STREAMS.
//!
//! - STDIN: eshell's only caller (`crates/core/lisp/eshell.el`) never
//!   passes it, so `Stdio::null()` remains the default — interactive
//!   programs still see immediate EOF. Callers that want to feed data
//!   in (M-|, the second half of M79) pass a string, written on its own
//!   thread and then dropped to close the pipe. Unlike `run()` in
//!   `crates/core/src/remote.rs` (where the SAME thread writes stdin
//!   and then drains stdout, so a big write really does deadlock against
//!   a full stdout pipe), `spawn()` here starts the stdout/stderr reader
//!   threads BEFORE the stdin writer, so they're already independently
//!   draining the child's output — a synchronous write on the calling
//!   thread would not deadlock. The reason for the writer thread is
//!   different: `spawn()` is called from the editor's event loop, and a
//!   synchronous write would block that loop for as long as the child
//!   takes to read all of stdin (which, for a large payload or a slow
//!   consumer, is not bounded).
//! - STREAMS: `merged` (the default, and the only behavior that existed
//!   before M79) keeps stdout and stderr interleaved into one string, as
//!   `shell-process-poll` always returned. `separate` splits them so
//!   `shell-process-poll` returns `(stdout . STR)` / `(stderr . STR)`
//!   cons cells instead — needed so `M-|` can substitute a selection with
//!   exactly the tool's stdout, without a stderr warning corrupting the
//!   result. eshell keeps using `merged` (unchanged call site, unchanged
//!   behavior): it's an interactive REPL where seeing stderr interleaved
//!   with stdout is the point, and there was no reason to touch a
//!   working call site while widening this module for a different
//!   caller. It can migrate to `separate` later if that turns out to be
//!   useful there too.
//!
//! M141: `Merged` mode gives the child ONE pipe (`std::io::pipe()`) and
//! passes it as both stdout and stderr (the writer half for stdout, a
//! `try_clone()` of it for stderr), with a single reader thread draining
//! it. Order in that mode is
//! therefore exactly the order the child wrote to its (now-shared)
//! stdout/stderr file descriptor — the same guarantee a terminal or GNU
//! Emacs's own process connection gives, and one that two independent
//! pipes with two independent reader threads structurally cannot give (see the
//! false claim this replaced, and the measured failure counts, in the
//! M141 milestone record). The remaining caveat is the child's own stdio
//! buffering, which this module has no control over: a program whose
//! stdout is fully buffered because it isn't talking to a tty can still
//! reorder its own writes relative to unbuffered stderr writes, same as
//! it would on a real terminal.
//!
//! Deliberately NOT here: any timeout or output-size cap. Both belong to
//! the M79 elisp pump (the second half of this milestone, not yet
//! written) because only it knows which buffer output is landing in and
//! how to tell the user it got truncated. This module stays a thin,
//! unbounded process primitive, same as before M79.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use crate::builtins::{defun, need_str};
use crate::interp::Interp;
use crate::printer::prin1_to_string;
use crate::value::{ExtRef, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Stdout,
    Stderr,
}

/// Whether `shell-process-poll` interleaves stdout/stderr into one
/// string (`Merged`, the historical and still-default behavior) or
/// reports them as separately-tagged cons cells (`Separate`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Streams {
    Merged,
    Separate,
}

enum ShellEvent {
    /// A chunk of output, tagged with which pipe it came from. In
    /// `Merged` mode there is only one pipe and one reader thread (both
    /// stdout and stderr are dup'd ends of the same `std::io::pipe()`),
    /// so the tag is redundant there and chunks arrive in the child's own
    /// write order. In `Separate` mode there are two independent pipes
    /// and two independent reader threads, and the tag is what tells
    /// `poll()` which accumulator to append to.
    Chunk(StreamKind, String),
    /// One reader stream reached EOF (internal bookkeeping).
    Eof,
}

/// What `ShellProc::poll` handed back for one call.
pub enum PollOutput {
    /// Nothing pending right now.
    None,
    /// `Merged` mode: interleaved stdout+stderr chunk.
    Merged(String),
    /// `Separate` mode: a chunk from just one stream. If both streams
    /// have pending data, one is returned now and the other on a
    /// subsequent poll.
    Stdout(String),
    Stderr(String),
    /// The process exited with this code. Delivered exactly once.
    Exit(i32),
}

pub struct ShellProc {
    child: Child,
    events: Receiver<ShellEvent>,
    /// Streams still open; when it hits zero the child gets waited.
    open_streams: u32,
    exit_code: Option<i32>,
    /// The (exit . CODE) event is delivered to elisp exactly once.
    exit_reported: bool,
    streams: Streams,
    /// Buffered output not yet handed to elisp. In `Merged` mode only
    /// `merged_buf` is used; in `Separate` mode `stdout_buf`/`stderr_buf`
    /// hold data waiting for a future poll() call once one of them has
    /// already been returned this round.
    merged_buf: String,
    stdout_buf: String,
    stderr_buf: String,
    /// Joined so the process can't outlive its writer thread's access to
    /// the child's stdin; also lets us swallow a writer panic instead of
    /// letting `kill()`/`Drop` orphan a stuck thread. `None` when no
    /// stdin was supplied (the historical Stdio::null() path).
    stdin_thread: Option<std::thread::JoinHandle<()>>,
}

/// Read `stream` in chunks and forward them, tagged with `kind`. A
/// trailing incomplete UTF-8 sequence is carried over to the next chunk
/// so multi-byte characters split across read() calls survive.
fn reader_thread(mut stream: impl Read + Send + 'static, kind: StreamKind, tx: Sender<ShellEvent>) {
    std::thread::spawn(move || {
        let mut carry: Vec<u8> = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if !carry.is_empty() {
                        let _ = tx.send(ShellEvent::Chunk(
                            kind,
                            String::from_utf8_lossy(&carry).to_string(),
                        ));
                    }
                    let _ = tx.send(ShellEvent::Eof);
                    return;
                }
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    match std::str::from_utf8(&carry) {
                        Ok(s) => {
                            if tx.send(ShellEvent::Chunk(kind, s.to_string())).is_err() {
                                return;
                            }
                            carry.clear();
                        }
                        Err(e) => {
                            let valid = e.valid_up_to();
                            let s = String::from_utf8_lossy(&carry[..valid]).to_string();
                            if !s.is_empty() && tx.send(ShellEvent::Chunk(kind, s)).is_err() {
                                return;
                            }
                            carry.drain(..valid);
                            // Pathological non-UTF-8 output: don't let
                            // the carry grow without bound.
                            if carry.len() > 8 {
                                let s = String::from_utf8_lossy(&carry).to_string();
                                if tx.send(ShellEvent::Chunk(kind, s)).is_err() {
                                    return;
                                }
                                carry.clear();
                            }
                        }
                    }
                }
            }
        }
    });
}

impl ShellProc {
    /// Run CMDLINE via `sh -c` in DIR. The shell owns pipes, globs,
    /// `&&` etc. — eshell doesn't reimplement them (v1, documented).
    ///
    /// `stdin_data`: `None` means the child's stdin is closed
    /// immediately (`Stdio::null()`, the pre-M79 behavior — interactive
    /// programs see EOF and exit). `Some(data)` pipes `data` in on a
    /// dedicated writer thread, then drops the pipe to send EOF. The
    /// writer thread exists so `spawn()` (called from the editor's event
    /// loop) doesn't block on `write_all` for as long as the child takes
    /// to consume stdin — NOT to avoid a deadlock: the stdout/stderr
    /// reader threads below are started first and drain independently of
    /// this write, so a synchronous write here would just be slow, never
    /// stuck (see the file-level doc comment for how this differs from
    /// `remote.rs`'s `run()`, where that distinction doesn't hold).
    fn spawn(
        cmdline: &str,
        dir: &str,
        stdin_data: Option<&str>,
        streams: Streams,
    ) -> std::io::Result<ShellProc> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(cmdline)
            .current_dir(dir)
            .stdin(if stdin_data.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        // `Merged`: one OS pipe, given to the child as BOTH stdout and
        // stderr (via a `try_clone()`'d writer), so the child's own
        // writes to what it sees as two file descriptors land on one
        // pipe in the order it made them -- the ordering guarantee
        // described in the file-level doc comment above. `Separate`:
        // two independent pipes, unchanged from before M141.
        let merged_pipe = match streams {
            Streams::Merged => {
                let (reader, writer) = std::io::pipe()?;
                let writer_clone = writer.try_clone()?;
                cmd.stdout(Stdio::from(writer));
                cmd.stderr(Stdio::from(writer_clone));
                Some(reader)
            }
            Streams::Separate => {
                cmd.stdout(Stdio::piped());
                cmd.stderr(Stdio::piped());
                None
            }
        };
        let mut child = cmd.spawn()?;
        // `spawn` borrows `cmd`, so `cmd` (and, in `Merged` mode, the two
        // `PipeWriter` handles it holds) stays alive until this function
        // returns -- a `let` binding drops at the end of its block, not
        // after its last use. That drop is load-bearing: the pipe's read
        // end sees EOF only once ALL writer-end descriptors, in every
        // process, are closed. It is safe here only because nothing in
        // this function reads the pipe: the reader is handed to its own
        // thread below, which reads output as the child writes it and
        // whose final read (the one returning EOF) cannot complete until
        // `cmd` has dropped at the return.
        // A synchronous read on `reader` added inside this function would
        // deadlock (measured by the M141 cold read with a standalone
        // probe); drop `cmd` explicitly first if one is ever needed.
        let (tx, rx) = std::sync::mpsc::channel();
        let open_streams = match merged_pipe {
            Some(reader) => {
                // One reader thread: `Merged` mode has exactly one pipe.
                // The `StreamKind` tag is irrelevant in this mode (see
                // `ShellEvent::Chunk`'s doc comment) but `reader_thread`
                // needs one; `Stdout` is as good as `Stderr` here.
                reader_thread(reader, StreamKind::Stdout, tx);
                1
            }
            None => {
                let stdout = child.stdout.take().expect("piped stdout");
                let stderr = child.stderr.take().expect("piped stderr");
                reader_thread(stdout, StreamKind::Stdout, tx.clone());
                reader_thread(stderr, StreamKind::Stderr, tx);
                2
            }
        };

        let stdin_thread = stdin_data.map(|data| {
            let data = data.as_bytes().to_vec();
            let mut stdin = child.stdin.take().expect("piped stdin");
            std::thread::spawn(move || {
                // Swallow write errors (e.g. EPIPE if the child never
                // reads stdin and exits early, like `true`) instead of
                // panicking -- an early-exiting child tearing the pipe
                // out from under this write is expected, not exceptional
                // (same EPIPE-swallowing reasoning as remote.rs's
                // `run()`, though unlike `run()` this thread's reason
                // for existing at all is not deadlock avoidance -- see
                // the doc comment on `spawn()` above).
                let _ = stdin.write_all(&data);
                // `stdin` drops here, closing the pipe -- this is what
                // sends the child's stdin its EOF. Load-bearing: without
                // it, a reader like `cat` blocks forever waiting for
                // more input.
            })
        });

        Ok(ShellProc {
            child,
            events: rx,
            open_streams,
            exit_code: None,
            exit_reported: false,
            streams,
            merged_buf: String::new(),
            stdout_buf: String::new(),
            stderr_buf: String::new(),
            stdin_thread,
        })
    }

    /// Non-blocking drain: pending output (shape depends on `streams`
    /// mode), plus the exit code once both streams have closed and the
    /// child has been reaped. Genuinely non-blocking -- it is called
    /// from the editor's idle tick, and this project treats any blocking
    /// primitive reachable from there as a bug (M77) -- which is why
    /// reaping below uses `try_wait()`/`JoinHandle::is_finished()`
    /// instead of `wait()`/`join()` on an handle that might not have
    /// finished yet: either could stall an arbitrary amount of time if
    /// the child closed its pipes without exiting, or hasn't been
    /// scheduled off the reader/writer threads yet.
    ///
    /// Timing note (kept honest rather than papered over): output is
    /// returned as soon as it's buffered, which means the reap step
    /// below can be delayed to a LATER `poll()` call than in the
    /// pre-M79 version of this function (which drained the channel,
    /// unconditionally attempted the reap if both streams had closed,
    /// and only THEN decided what to return). That's fine here: the
    /// shape and order elisp sees is unchanged (output strings/cons
    /// always precede `(exit . CODE)`), and delaying the reap by a call
    /// or two just means `exit_code` becomes `Some` slightly later --
    /// nothing reads `exit_code` except this function and `live()`,
    /// which already tolerates `try_wait()` returning "not yet" by
    /// reporting the process as still live.
    pub fn poll(&mut self) -> PollOutput {
        loop {
            match self.events.try_recv() {
                Ok(ShellEvent::Chunk(kind, s)) => match self.streams {
                    Streams::Merged => self.merged_buf.push_str(&s),
                    Streams::Separate => match kind {
                        StreamKind::Stdout => self.stdout_buf.push_str(&s),
                        StreamKind::Stderr => self.stderr_buf.push_str(&s),
                    },
                },
                Ok(ShellEvent::Eof) => {
                    self.open_streams = self.open_streams.saturating_sub(1);
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        match self.streams {
            Streams::Merged => {
                if !self.merged_buf.is_empty() {
                    return PollOutput::Merged(std::mem::take(&mut self.merged_buf));
                }
            }
            Streams::Separate => {
                if !self.stdout_buf.is_empty() {
                    return PollOutput::Stdout(std::mem::take(&mut self.stdout_buf));
                }
                if !self.stderr_buf.is_empty() {
                    return PollOutput::Stderr(std::mem::take(&mut self.stderr_buf));
                }
            }
        }
        if self.exit_code.is_none() && self.open_streams == 0 {
            // Both streams closed: the child has probably finished, but
            // NOT necessarily been reaped yet (it could have closed
            // stdout/stderr without exiting, or the OS may not have
            // updated wait status yet). `try_wait()` never blocks; if it
            // says "not yet", just leave exit_code unset and retry on
            // the next poll() instead of calling the blocking `wait()`.
            match self.child.try_wait() {
                Ok(Some(status)) => self.exit_code = Some(status.code().unwrap_or(-1)),
                Ok(None) => {}
                Err(_) => self.exit_code = Some(-1),
            }
        }
        // Opportunistically reap the stdin writer thread once it has
        // actually finished. `is_finished()` never blocks; `join()` on a
        // thread that has already finished doesn't block either (the
        // thread is done, join() just picks up its result) -- so this
        // can't stall `poll()` regardless of when the writer finishes
        // relative to the child exiting.
        if self.stdin_thread.as_ref().is_some_and(|t| t.is_finished()) {
            if let Some(t) = self.stdin_thread.take() {
                let _ = t.join();
            }
        }
        if let Some(code) = self.exit_code {
            if !self.exit_reported {
                self.exit_reported = true;
                return PollOutput::Exit(code);
            }
        }
        PollOutput::None
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(t) = self.stdin_thread.take() {
            let _ = t.join();
        }
        self.exit_code = Some(-1);
        self.open_streams = 0;
    }

    pub fn live(&mut self) -> bool {
        if self.exit_code.is_some() {
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(_)) | Err(_) => {
                // Exited but poll() hasn't drained yet; still "live"
                // until poll reports the exit so no output is lost.
                true
            }
            Ok(None) => true,
        }
    }
}

/// M104: synchronous "feed stdin, collect stdout/stderr" process
/// execution, for `call-process-string`.
///
/// DIR (M104 fix round): the working directory to spawn PROGRAM in, or
/// `None` to inherit the editor's own current directory (the previous,
/// only behavior). This matters because a formatter invoked without it
/// runs in whatever directory the EDITOR happened to start in, not the
/// directory of the file actually being formatted -- concretely,
/// `clang-format -style=file` searches for a `.clang-format` starting
/// from ITS OWN working directory and upward, and silently falls back
/// to LLVM style (exit 0, no diagnostic at all) if it doesn't find one,
/// so a wrong `dir` doesn't error, it just quietly uses the wrong style.
///
/// Runs PROGRAM with ARGS, writes INPUT to its stdin and then closes
/// the pipe (load-bearing -- a reader like a formatter blocks forever
/// on an unclosed stdin, same reasoning as the writer thread in
/// `ShellProc::spawn` above), and BLOCKS the calling thread until the
/// child exits or TIMEOUT_MS elapses. Because elisp is single-threaded
/// and driven off the editor's event loop, this call blocks the whole
/// editor for up to TIMEOUT_MS -- which is exactly why it exists as a
/// separate primitive from `start-shell-process` (async, unbounded):
/// this one is for short, well-behaved external tools invoked at a
/// predictable moment (formatting a buffer on save), where a bounded
/// synchronous wait is an acceptable trade for not needing an elisp-side
/// poll loop. It is NOT a general process-execution primitive.
///
/// On timeout, or if `try_wait` itself errors, the child's entire
/// process GROUP is killed via `kill(-pid, SIGKILL)`, not just the
/// direct child (`process_group(0)` at spawn makes the child its own
/// group leader so this reaches any of ITS children too) -- killing
/// only the direct child has left an orphaned grandchild running for 18
/// minutes elsewhere in this project (see `dev/mutate.py`'s header,
/// point 5). Non-Unix targets fall back to killing just the child
/// process, since process groups are a Unix concept.
///
/// If PROGRAM can't even be spawned (not found, not executable, ...),
/// this returns a non-zero exit code with the OS error message in
/// stderr, rather than propagating an `Err` -- the elisp caller (M104's
/// `format.el`) treats "formatter missing" and "formatter failed" the
/// same way (message the user, don't touch the buffer), and forcing it
/// to handle two different call shapes for that would buy nothing.
fn call_process_string(
    program: &str,
    args: &[String],
    input: &str,
    timeout_ms: u64,
    dir: Option<&str>,
) -> (i32, String, String) {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return (
                127,
                String::new(),
                format!("call-process-string: {}: {}", program, e),
            )
        }
    };

    let mut stdin = child.stdin.take().expect("piped stdin");
    let input_bytes = input.as_bytes().to_vec();
    let stdin_thread = std::thread::spawn(move || {
        // Swallow write errors (EPIPE if the child exits/dies before
        // reading all of stdin) -- expected, not exceptional, same as
        // the writer thread in `ShellProc::spawn` above.
        let _ = stdin.write_all(&input_bytes);
        // `stdin` drops here, closing the pipe -- this is what sends
        // the child's stdin its EOF.
    });

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    });

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };

    match status {
        Some(status) => {
            // Child already exited: its pipes are closed, so the
            // writer/reader threads finish (or already have) promptly.
            let _ = stdin_thread.join();
            let stdout = stdout_thread.join().unwrap_or_default();
            let stderr = stderr_thread.join().unwrap_or_default();
            (status.code().unwrap_or(-1), stdout, stderr)
        }
        None => {
            // Timed out (or try_wait errored): kill the whole process
            // group FIRST, then join -- joining the writer/reader
            // threads before killing could itself hang (e.g. a writer
            // thread blocked on a full stdin pipe that a wedged child
            // never drains).
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdin_thread.join();
            let stdout = stdout_thread.join().unwrap_or_default();
            let stderr = stderr_thread.join().unwrap_or_default();
            (-1, stdout, stderr)
        }
    }
}

const SHELL_TAG: &str = "shell-process";

fn proc_arg(
    interp: &mut Interp,
    v: &Value,
) -> Result<std::rc::Rc<RefCell<ShellProc>>, crate::error::Flow> {
    v.as_ext::<RefCell<ShellProc>>(SHELL_TAG)
        .ok_or_else(|| interp.wrong_type("shell-process-p", v))
}

pub fn register(interp: &mut Interp) {
    defun(interp, "start-shell-process", 2, Some(4), |i, a| {
        let cmdline = need_str(i, &a[0])?.to_string();
        let dir = need_str(i, &a[1])?.to_string();
        let stdin_data = match a.get(2) {
            None | Some(Value::Nil) => None,
            Some(v) => Some(need_str(i, v)?.to_string()),
        };
        let streams = match a.get(3) {
            None | Some(Value::Nil) => Streams::Merged,
            Some(v) => {
                let name = i.as_sym(v).map(|id| i.sym_name(id).to_string());
                match name.as_deref() {
                    Some("merged") => Streams::Merged,
                    Some("separate") => Streams::Separate,
                    _ => {
                        let got = prin1_to_string(i, v);
                        return Err(i.error(format!(
                            "start-shell-process: STREAMS must be `merged' or `separate', got {}",
                            got
                        )));
                    }
                }
            }
        };
        match ShellProc::spawn(&cmdline, &dir, stdin_data.as_deref(), streams) {
            // ShellProc: Child process handle / pipes, no Value.
            Ok(p) => Ok(Value::Ext(ExtRef::new(SHELL_TAG, RefCell::new(p), None))),
            Err(e) => Err(i.error(format!("start-shell-process: {}", e))),
        }
    });
    // nil = nothing pending; STRING = a merged output chunk (`Merged`
    // mode, the default); (stdout . STR) / (stderr . STR) = a chunk from
    // just that stream (`Separate` mode); (exit . CODE) = the process
    // finished (delivered exactly once, after all output).
    defun(interp, "shell-process-poll", 1, Some(1), |i, a| {
        let p = proc_arg(i, &a[0])?;
        let mut p = p.borrow_mut();
        match p.poll() {
            PollOutput::None => Ok(Value::Nil),
            PollOutput::Merged(s) => Ok(Value::string(s)),
            PollOutput::Stdout(s) => Ok(Value::cons(
                Value::Sym(i.intern("stdout")),
                Value::string(s),
            )),
            PollOutput::Stderr(s) => Ok(Value::cons(
                Value::Sym(i.intern("stderr")),
                Value::string(s),
            )),
            PollOutput::Exit(code) => Ok(Value::cons(
                Value::Sym(i.intern("exit")),
                Value::Int(code as i64),
            )),
        }
    });
    defun(interp, "shell-process-kill", 1, Some(1), |i, a| {
        let p = proc_arg(i, &a[0])?;
        p.borrow_mut().kill();
        Ok(Value::Sym(i.syms.t))
    });
    defun(interp, "shell-process-live-p", 1, Some(1), |i, a| {
        let p = proc_arg(i, &a[0])?;
        let live = p.borrow_mut().live();
        Ok(Value::bool(live, i.syms.t))
    });
    // M104: `(call-process-string PROGRAM ARGS INPUT &optional
    // TIMEOUT-MS DIR)` -> `(EXIT-CODE STDOUT STDERR)`. Synchronous --
    // see `call_process_string`'s doc comment above for why this blocks
    // the editor and why that's an acceptable trade for its intended
    // use (save-time formatting), unlike the async `start-shell-process`
    // family above. DIR (M104 fix round) is the directory to spawn
    // PROGRAM in; omitted or nil inherits the editor's own current
    // directory, the pre-fix-round behavior -- see `call_process_string`'s
    // own doc comment for why a caller like `format--clang-args' needs
    // to pass this explicitly rather than relying on the default.
    defun(interp, "call-process-string", 3, Some(5), |i, a| {
        let program = need_str(i, &a[0])?.to_string();
        let args: Vec<String> = match &a[1] {
            Value::Nil => Vec::new(),
            v => match v.list_to_vec() {
                Some(items) => items
                    .iter()
                    .map(|item| need_str(i, item).map(|s| s.to_string()))
                    .collect::<Result<Vec<String>, crate::error::Flow>>()?,
                None => return Err(i.wrong_type("listp", v)),
            },
        };
        let input = need_str(i, &a[2])?.to_string();
        let timeout_ms = match a.get(3) {
            None | Some(Value::Nil) => 5000,
            Some(v) => {
                let n = crate::builtins::need_int(i, v)?;
                if n < 0 {
                    0
                } else {
                    n as u64
                }
            }
        };
        let dir = match a.get(4) {
            None | Some(Value::Nil) => None,
            Some(v) => Some(need_str(i, v)?.to_string()),
        };
        let (code, stdout, stderr) =
            call_process_string(&program, &args, &input, timeout_ms, dir.as_deref());
        Ok(Value::list(vec![
            Value::Int(code as i64),
            Value::string(stdout),
            Value::string(stderr),
        ]))
    });
}
