//! M60: process-global sink for output that would otherwise be written
//! directly to the terminal by a background thread/process (LSP server
//! stderr, worker stderr, the highlight worker's own diagnostics). In
//! the TUI, redisplay is diff-based against a `Grid` it owns fully (see
//! `crates/core/src/redisplay.rs`) — any byte written straight to the
//! terminal outside that grid is never revisited, so it can permanently
//! corrupt the screen (M60's motivating bug: an LSP server's stderr
//! banner bleeding through and never getting overwritten). Every such
//! writer must instead call `push` here; `crates/core::idle_tick` drains
//! it each tick into a `*background-output*` buffer the user can look at
//! on demand.
//!
//! Lives in `elisp` (not `core`) because both `elisp::lsp` and
//! `elisp::worker` need it, and `core` depends on `elisp` — not the
//! other way around (see `crates/core/Cargo.toml`).

use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::Mutex;

use crate::builtins::defun;
use crate::interp::Interp;
use crate::value::Value;

/// Bound on buffered lines. A runaway background writer (e.g. an LSP
/// server stuck logging in a loop) must not grow this without bound;
/// old lines are dropped in favor of new ones, and the drop count is
/// surfaced once at the front of the next `drain`.
const MAX_LINES: usize = 500;

struct State {
    lines: VecDeque<String>,
    dropped: usize,
}

static STATE: Mutex<State> = Mutex::new(State {
    lines: VecDeque::new(),
    dropped: 0,
});

/// Record one line of background output, tagged with its source (e.g.
/// `"worker"`, or an LSP server's command name). Cheap and non-blocking:
/// callers may be background threads that must not stall on I/O while
/// holding the lock, so this only ever touches the in-memory queue.
///
/// If the lock is poisoned (a panic elsewhere while holding it), the
/// line is silently dropped rather than propagating the panic into an
/// unrelated background reader thread.
pub fn push(tag: &str, line: &str) {
    let Ok(mut st) = STATE.lock() else {
        return;
    };
    if st.lines.len() >= MAX_LINES {
        st.lines.pop_front();
        st.dropped += 1;
    }
    st.lines.push_back(format!("[{}] {}", tag, line));
}

/// Take everything buffered so far, clearing it. If any lines were
/// dropped for exceeding `MAX_LINES` since the last drain, the returned
/// vec's first element is a note saying how many, and the drop counter
/// is reset to zero.
///
/// If the lock is poisoned, returns an empty vec (same "prefer silent
/// degradation over panicking a caller" stance as `push`).
pub fn drain() -> Vec<String> {
    let Ok(mut st) = STATE.lock() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(st.lines.len() + 1);
    if st.dropped > 0 {
        out.push(format!(
            "[bglog] {} line(s) dropped (buffer full)",
            st.dropped
        ));
        st.dropped = 0;
    }
    out.extend(st.lines.drain(..));
    out
}

/// Read newline-delimited text from `r` (a child process's stderr) and
/// `push` each line under `tag`, until EOF. Byte-oriented rather than
/// `BufRead::lines()` so a non-UTF-8 byte in one line degrades that line
/// to lossy replacement characters instead of aborting the whole reader
/// (a real LSP server or worker child is not a hostile input, but a
/// stray non-UTF-8 byte in a log line shouldn't cost every line after
/// it). Meant to be the entire body of a dedicated reader thread; blocks
/// until the underlying stream closes, then returns.
pub fn pump_lines(mut r: impl BufRead, tag: &str) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) => return, // EOF
            Ok(_) => {
                while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
                    buf.pop();
                }
                push(tag, &String::from_utf8_lossy(&buf));
            }
            Err(_) => return,
        }
    }
}

/// `background-log-drain`: batch-mode escape hatch. `--repl`/`--eval`/
/// `--script` have no editor, so `crates/core::idle_tick` (the normal
/// drain path, feeding `*background-output*`) never runs and this
/// output would otherwise just vanish. Returns the drained lines as a
/// list of strings (each already `"[tag] ..."`-prefixed, same as what
/// the buffer would show); also used directly by tests, which have no
/// running editor either.
pub fn register(interp: &mut Interp) {
    defun(interp, "background-log-drain", 0, Some(0), |_i, _a| {
        let lines = drain();
        Ok(lines
            .into_iter()
            .rev()
            .fold(Value::Nil, |acc, l| Value::cons(Value::string(l), acc)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests share process-global state (the `STATE` static) not
    // just with each other but with any other test in this crate's test
    // binary that touches bglog -- and `cargo test` runs tests in one
    // binary concurrently by default. Draining before asserting clears
    // whatever an unrelated test left behind, but does NOT stop a
    // concurrently-running sibling test in *this* module from pushing or
    // draining mid-test, so every test also holds `TEST_LOCK` for its
    // duration to serialize against its siblings here.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn push_and_drain_round_trips() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = drain(); // clear whatever earlier tests left behind
        push("t1", "hello");
        push("t1", "world");
        let out = drain();
        assert_eq!(
            out,
            vec!["[t1] hello".to_string(), "[t1] world".to_string()]
        );
    }

    #[test]
    fn drain_after_drain_is_empty() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = drain();
        push("t2", "one");
        let _ = drain();
        assert_eq!(drain(), Vec::<String>::new());
    }

    #[test]
    fn overflow_drops_oldest_and_reports_count() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = drain();
        for n in 0..(MAX_LINES + 3) {
            push("t3", &format!("line{n}"));
        }
        let out = drain();
        // 3 lines were dropped (oldest: line0, line1, line2).
        assert_eq!(out[0], "[bglog] 3 line(s) dropped (buffer full)");
        assert_eq!(out.len(), 1 + MAX_LINES);
        assert_eq!(out[1], "[t3] line3");
        assert_eq!(*out.last().unwrap(), format!("[t3] line{}", MAX_LINES + 2));

        // The drop counter was reset: a second overflow-free round
        // reports no drops.
        push("t3", "again");
        let out2 = drain();
        assert_eq!(out2, vec!["[t3] again".to_string()]);
    }
}
