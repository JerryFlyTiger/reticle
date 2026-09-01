//! M22: TRAMP-style remote file access over the ssh binary. GNU
//! Emacs's TRAMP shells out to ssh too — this is the same model,
//! synchronous and blocking (v1, documented). Key-based auth only:
//! BatchMode=yes makes missing keys fail fast instead of hanging on a
//! password prompt.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A parsed `/ssh:user@host:/path` remote path.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct RemotePath {
    /// "user@host" or bare "host" — passed to ssh verbatim.
    pub host: String,
    /// The path on the remote (may be relative or ~-relative).
    pub path: String,
}

/// Parse `/ssh:HOST:PATH`. Returns None for local paths.
pub fn parse(p: &str) -> Option<RemotePath> {
    let rest = p.strip_prefix("/ssh:")?;
    let colon = rest.find(':')?;
    let host = &rest[..colon];
    let path = &rest[colon + 1..];
    if host.is_empty() {
        return None;
    }
    Some(RemotePath {
        host: host.to_string(),
        path: if path.is_empty() {
            ".".to_string()
        } else {
            path.to_string()
        },
    })
}

/// Rebuild the `/ssh:host:path` display form.
pub fn format_path(host: &str, path: &str) -> String {
    format!("/ssh:{}:{}", host, path)
}

/// Single-quote `s` for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// The ssh binary — overridable for tests (a local shim) via env.
fn ssh_bin() -> String {
    std::env::var("RETICLE_SSH_BIN").unwrap_or_else(|_| "ssh".to_string())
}

/// How long `run()` waits for a remote command before giving up on it.
///
/// Read from `RETICLE_REMOTE_TIMEOUT` (seconds, `f64` so tests can use
/// sub-second values like `0.3`) rather than an elisp `defvar`: every other
/// knob this module has (`ssh_bin()` above) is an env var, and this module
/// never touches `Interp` at all — threading `&mut Interp` through all nine
/// `pub fn`s here just to read one number would be an architecture change
/// out of scope for the milestone that added this timeout.
///
/// `0` or negative disables the timeout entirely (back to unbounded wait —
/// a deliberate escape hatch, not an oversight). Unset or unparseable
/// falls back to 30 seconds.
///
/// Why 30s: ssh's own `ConnectTimeout=5` above only bounds establishing the
/// TCP connection; once connected, 30 seconds of total silence in practice
/// means the remote command is never coming back. But this can't be made
/// smarter by watching for "no output for N seconds" instead of a flat
/// total — `copy_file`'s remote `cp` and `remove_dir`'s `rm -r` can
/// legitimately run for a long time on a big file/tree while producing
/// zero output the whole way, so any timeout here WILL occasionally kill
/// an operation that was still alive and working. That's an accepted,
/// documented false positive: the remote process the killed ssh spawned
/// keeps running to completion regardless (see `run()`'s doc comment), so
/// this can misreport "unknown" but can't corrupt anything.
///
/// The value is clamped to `[MIN_REMOTE_TIMEOUT_SECS, MAX_REMOTE_TIMEOUT_SECS]`
/// (M77 review): `f64::from_secs_f64` panics on non-finite input, and
/// `Instant::now() + Duration` panics on overflow for a `Duration` built
/// from a merely-huge-but-finite value (measured on macOS/aarch64 by the
/// coordinator: `1e18` is safe, `1e19` overflows the add in `run()`'s wait
/// loop below, and `>= 2e19` panics one step earlier in `from_secs_f64`
/// itself) — both would turn a typo'd env var (`inf`, `Infinity`, `3e400`
/// overflowing to infinity, or just a slipped digit) into a crash in
/// library code reachable from every single remote operation, which this
/// project's no-panic rule forbids.
/// Non-finite or unparseable input is treated the same as unset (falls
/// back to 30s) rather than clamped, since "not a valid number" isn't
/// closer to one clamp bound than the other. The upper bound, 86400s (24h),
/// is "no timeout" in every practical sense while still being a `Duration`
/// no `Instant` arithmetic can overflow on; the lower bound, 0.001s, keeps
/// a typo'd near-zero-but-positive value from being indistinguishable from
/// the `0`/negative "disabled" escape hatch it's adjacent to.
const MIN_REMOTE_TIMEOUT_SECS: f64 = 0.001;
const MAX_REMOTE_TIMEOUT_SECS: f64 = 86_400.0;

/// The effective timeout in seconds, `None` meaning "disabled" — shared by
/// `remote_timeout()` (which turns it into a `Duration` for the wait loop)
/// and `remote_timeout_secs_for_display()` (which formats the SAME value
/// into the messages callers see), so the two can never drift apart.
fn effective_timeout_secs() -> Option<f64> {
    let secs = std::env::var("RETICLE_REMOTE_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite())
        .unwrap_or(30.0);
    if secs <= 0.0 {
        None
    } else {
        Some(secs.clamp(MIN_REMOTE_TIMEOUT_SECS, MAX_REMOTE_TIMEOUT_SECS))
    }
}

fn remote_timeout() -> Option<Duration> {
    effective_timeout_secs().map(Duration::from_secs_f64)
}

/// Run `cmd` on the remote; returns (stdout, stderr, exit-code).
///
/// ## Wall-clock bound, and why it needs reader threads
///
/// Before this milestone `run()` only bounded TCP connection setup
/// (`ConnectTimeout=5` below) — once connected, a remote command that
/// never returns (a stuck NFS mount, a hung build, a genuinely wedged
/// shell) hung this call, and by extension the whole editor, forever: the
/// TUI's main loop is a synchronous call into `handle_key`
/// (`crates/frontend-tui/src/lib.rs`), so it never gets back to
/// `crossterm::event::poll` to notice a keypress while stuck in here.
///
/// The fix can't be "poll `try_wait()` and read stdout/stderr only after
/// it exits" — if the remote writes more than a pipe buffer's worth of
/// output (64 KB is typical) before anyone drains it, the remote process
/// blocks on ITS write, `try_wait()` never returns Ready, and the timeout
/// never fires (this is not theoretical: `cat`-ing a multi-hundred-KB
/// file hits it). So stdout and stderr are drained on their own threads
/// unconditionally, joined once the child is known to be finished (either
/// it exited on its own, or we killed it).
///
/// stdin has the same problem in reverse: writing `stdin_data` on the
/// calling thread blocks just as long if the remote command never reads
/// it, and it blocks BEFORE the child has been through any wait/kill
/// logic at all. So it also gets a thread; its only job, on top of
/// `write_all`, is to let `ChildStdin` drop when the thread ends, because
/// dropping is what sends the remote `cat` its EOF (this is the same
/// invariant the pre-existing single-threaded code relied on — losing it
/// would hang every remote write forever, timeout or not).
///
/// All three threads swallow I/O errors instead of panicking: killing the
/// child (the timeout path) tears the pipes out from under them (EPIPE on
/// the writer, EOF on the readers), which is expected, not exceptional.
fn run(host: &str, cmd: &str, stdin_data: Option<&[u8]>) -> std::io::Result<(String, String, i32)> {
    let mut child = Command::new(ssh_bin())
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=5")
        .arg(host)
        .arg(format!("LC_ALL=C {}", cmd))
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout_pipe = child.stdout.take().expect("piped stdout");
    let mut stderr_pipe = child.stderr.take().expect("piped stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf);
        buf
    });
    let stdin_thread = stdin_data.map(|data| {
        // Borrowed from the caller; owned so it can move into the thread.
        let data = data.to_vec();
        let mut stdin = child.stdin.take().expect("piped stdin");
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
            // `stdin` drops here, closing the pipe -- the remote `cat`
            // sees EOF and finishes. Load-bearing: see the doc comment
            // above.
        })
    });

    let deadline = remote_timeout().map(|d| Instant::now() + d);
    let status = loop {
        // M77 review, finding 6: this `?` is the one cleanup
        // asymmetry in this function -- if `try_wait()` itself errors
        // (not "not exited yet", an actual OS-level error), this returns
        // immediately without `kill()`ing the child or joining the three
        // reader/writer threads, unlike the timeout branch just below,
        // which is careful to do both. Left as-is rather than sharing
        // cleanup with the timeout branch: on Unix, `waitpid(WNOHANG)` on
        // a child this process itself spawned and hasn't reaped yet does
        // not fail in practice (the documented failure modes are ECHILD,
        // which needs either no such child or a prior reap -- neither
        // applies here -- and EINTR, which the standard library already
        // retries internally), so this path is not known to be
        // reachable, and adding shared cleanup machinery for an
        // unreachable path would cost more readability than the
        // asymmetry it removes is worth. If this ever turns out to be
        // reachable (a future platform, a `Command` change upstream),
        // the fix is to route this `Err` through the same
        // kill-wait-join sequence the timeout branch below uses instead
        // of returning it directly.
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Some(deadline) = deadline {
            if Instant::now() >= deadline {
                // Kill before wait: waiting on a still-running child would
                // just reproduce the hang we're trying to end. Wait AFTER
                // kill so the child doesn't linger as a zombie.
                child.kill()?;
                child.wait()?;
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                if let Some(t) = stdin_thread {
                    let _ = t.join();
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "remote command timed out",
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    if let Some(t) = stdin_thread {
        let _ = t.join();
    }
    Ok((
        String::from_utf8_lossy(&stdout).to_string(),
        String::from_utf8_lossy(&stderr).to_string(),
        status.code().unwrap_or(-1),
    ))
}

/// Turn a `run()` I/O error into user-facing text, front-loading the one
/// fact that matters (deliberately no remote path here: see the callers'
/// doc notes -- an 80-column terminal's echo area is `cols - 1` = 79
/// characters with no truncation ellipsis, and a `/ssh:host:/long/path`
/// prefix alone can eat that whole budget, pushing the actually-useful
/// part off the visible row; the user just typed the path and it's on
/// the mode line, so repeating it here isn't worth the space).
///
/// The REAL budget for the message text built here is 72, not 79: every
/// error surfaced through `Interp::error` gets an `"error: "` prefix
/// (`crates/elisp/src/interp.rs`'s `describe_flow`) before it ever reaches
/// the echo area, and that's 7 characters this function doesn't control
/// but has to leave room for (measured on a real TUI: `error: Remote save
/// timed out after 3s: NOT saved, remote state unknown`). See
/// `ssh_tests.rs`'s message-length-budget test, which is what actually
/// pins this number down for every string built here.
fn run_err(e: &std::io::Error, verb: &str) -> String {
    if e.kind() == std::io::ErrorKind::TimedOut {
        format!(
            "Remote {} timed out after {}s -- it may still be running there",
            verb,
            remote_timeout_secs_for_display()
        )
    } else {
        format!("Cannot run ssh: {}", e)
    }
}

/// The timeout value to show in a message, matching whatever `run()`
/// actually used (so the two never drift apart -- both read through
/// `effective_timeout_secs()`). Only called from the TimedOut branch
/// above, where `effective_timeout_secs()` is necessarily `Some` -- `run()`
/// can't return `TimedOut` with the escape hatch (`0`/negative) active,
/// since that disables the deadline check entirely; the `unwrap_or(30.0)`
/// below is unreachable in practice and exists only so this function
/// can't itself become a second place that panics.
///
/// Rounds to 3 decimal places before formatting: `RETICLE_REMOTE_TIMEOUT`
/// is a raw env var a test harness's own float math can hand back as
/// `0.30000000000000004` (`0.1 + 0.2`-style binary-float noise), and that
/// full 17-digit tail landing in a user-facing message was never intended
/// -- 3 decimals is more precision than a wall-clock timeout needs to be
/// meaningful.
///
/// Values of 100s or more additionally drop the fraction entirely, which
/// is what actually bounds this string's width. Rounding alone does NOT:
/// `12345.6789` rounds to `"12345.679"`, 9 characters, and that pushes
/// `run_err`'s longest message (verb `delete`/`rename`) to 73 characters
/// against a 72-character budget -- one character of the trailing "it may
/// still be running there" silently cut off in an 80-column terminal (M77
/// tail review, finding 1; the earlier claim that rounding capped this at
/// `"86400"`/5 characters was only true for integer-valued bounds). With
/// the fraction dropped above 100s the widest possible output is
/// `"99.999"` (6 characters), which every message here fits around. Sub-
/// second precision is meaningless at that scale anyway: nobody needs to
/// be told a timeout fired at 12345.679 seconds rather than 12346.
fn remote_timeout_secs_for_display() -> String {
    let secs = effective_timeout_secs().unwrap_or(30.0);
    let rounded = (secs * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 || rounded >= 100.0 {
        format!("{}", rounded as i64)
    } else {
        // `{:.3}` always emits exactly 3 decimals; trim the trailing
        // zeros (and a bare trailing "." if all 3 were zero) `rounded`
        // may have introduced back off.
        let s = format!("{:.3}", rounded);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// `write_file`'s own timeout message — deliberately NOT `run_err`'s
/// generic one: this is the most dangerous timeout in the module (the
/// data may already be safely committed and we're the only one who
/// doesn't know it), so it has to say BOTH "we didn't confirm a save"
/// and "we don't know what's really there", not just one or the other.
///
/// A function rather than an inline `format!` at its one call site so
/// the echo-area budget test measures the string production code
/// actually emits (M77 tail review, finding 5: it used to re-type the
/// literal, so a reworded message could have blown the budget with that
/// test still green — a guard that only guarded its own copy).
fn write_timeout_err() -> String {
    format!(
        "Remote save timed out after {}s: NOT saved, remote state unknown",
        remote_timeout_secs_for_display()
    )
}

/// Shared error formatting for every remote op, keyed by host/path
/// rather than a `RemotePath` so ops that operate on two paths on the
/// same host (`copy_file`/`rename_file` below) don't need to fabricate
/// one just to report an error.
fn err_text(op: &str, host: &str, path: &str, stderr: &str, code: i32) -> String {
    let detail = stderr.lines().next().unwrap_or("").trim();
    format!(
        "Remote {} failed on {} ({}): {}",
        op,
        format_path(host, path),
        code,
        if detail.is_empty() {
            "no error output"
        } else {
            detail
        }
    )
}

/// Read a remote file. Ok(None) = the file doesn't exist (new file).
pub fn read_file(rp: &RemotePath) -> Result<Option<String>, String> {
    let q = shell_quote(&rp.path);
    let probe = run(&rp.host, &format!("test -e {}", q), None).map_err(|e| run_err(&e, "read"))?;
    if probe.2 != 0 {
        // Distinguish "no file" from "no connection": a dead connection
        // also fails `true`.
        let alive = run(&rp.host, "true", None).map_err(|e| run_err(&e, "read"))?;
        if alive.2 != 0 {
            return Err(err_text("connect", &rp.host, &rp.path, &alive.1, alive.2));
        }
        return Ok(None);
    }
    let (out, errs, code) =
        run(&rp.host, &format!("cat {}", q), None).map_err(|e| run_err(&e, "read"))?;
    if code != 0 {
        return Err(err_text("read", &rp.host, &rp.path, &errs, code));
    }
    Ok(Some(out))
}

/// Split PATH into (dir, basename) without shelling out to
/// `dirname`/`basename` — plain string surgery on the Rust side so the
/// remote command doesn't need extra subprocesses to build a temp name.
/// No leading slash means "relative to the remote cwd", matching `sh`.
fn split_dir_base(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => (".", path),
    }
}

/// Write a remote file with a length-gated, write-through commit.
///
/// ## Why not `cat > path` (the pre-M76 shape)
///
/// `cat > path` truncates PATH the instant the remote shell *opens*
/// it, before a single byte of TEXT arrives. A dropped connection mid-
/// transfer (a stalled VPN to a build host) leaves PATH at 0 bytes
/// with the old content gone for good.
///
/// ## Why not "temp file + `mv`" either (M76's first attempt, reverted)
///
/// The first fix here staged into a temp file and `mv`'d it over PATH,
/// betting that a dropped connection kills the remote command before
/// `mv` runs. That bet is wrong for *real* ssh: without a pty (this
/// codebase never passes `-t`), a dropped connection does not send
/// SIGHUP — sshd just closes the command's stdin with a clean EOF.
/// `cat` reading a closed pipe returns 0 (success), so `cat > "$t" &&
/// mv "$t" path` cheerfully commits a truncated temp file. Measured on
/// a real TUI against a real ssh round-trip cut off after 800 of 79
/// lines: editor said `Wrote`, remote file was a clean-looking half a
/// file. That is worse than the original bug — the old bug was loud
/// (0 bytes, a reported error); this one is silent and looks like a
/// normal edit in `git diff`.
///
/// ## This version: verify length before committing, write through (not `mv`) to commit
///
/// 1. Stage the new content into a temp file.
/// 2. **Length-gate**: compare the byte count landed on the remote
///    against `n`, the length TEXT actually has on our side. `wc` is
///    POSIX and always present; if it were somehow missing, `[ ... -eq
///    ... ]` fails on `wc`'s empty output with "integer expression
///    expected" and the shell takes the `||` branch — i.e. missing
///    `wc` fails *closed* (reject), not open (silently skip the
///    check), unlike the `cksum`-availability worry M75 raised for a
///    different guard. No checksum is needed: ssh's transport already
///    guarantees per-packet integrity, so the only failure shape left
///    to catch is truncation, and byte count alone catches that.
/// 3. **Commit by write-through** (`cat "$t" > path`), not `mv`. `mv`
///    was measured to have four real regressions: it replaces a
///    symlink with a plain file (breaking a shared symlink other
///    things point at, and turning a one-line `git diff` into
///    "deleted symlink, added huge file"); it breaks hardlinks; on a
///    non-root connection it can flip the file's owner to whoever's
///    saving; and a path that starts with `-` (e.g. `-rf.v`, a real
///    RTL naming pattern) gets parsed as an option by `mv`/`cp`/`rm`'s
///    getopt and every save — including the cleanup `rm`! — fails.
///    Write-through targets PATH via a shell redirect, never passing
///    it as a `mv`/`cp` *argument*, so none of the four apply: inode,
///    mode, owner, and any symlink/hardlink structure survive exactly
///    as GNU Emacs's own default does (`file-precious-flag` is nil by
///    default, i.e. write-through, and no setting of it turns a
///    symlink into a plain file either). Local saves already go
///    through `fs::write`, which is also write-through — this keeps
///    remote and local symmetric.
///
///    The cost: the commit step is no longer atomic. Measured window
///    for a 2 KB file (`demo/rtl/`'s average size): ~40 microseconds.
///    Today's `cat > path` has a truncation window spanning the whole
///    transfer (tens of ms to seconds on a slow link) — so this
///    *shrinks* an existing risk by roughly 3-4 orders of magnitude,
///    it doesn't introduce a new one.
///
/// The temp file is created **in PATH's own directory first**
/// (`( : > "$t" ) 2>/dev/null` — see the note on the parens further
/// down) so a full disk or over-quota filesystem is caught before
/// we'd otherwise touch PATH at all,
/// which matters more than a dropped connection on a shared build
/// host. If that directory refuses writes (read-only directory, file
/// itself still writable — GNU's own `file-precious-flag` docstring
/// describes exactly this fallback), we retry under
/// `${TMPDIR:-/tmp}`. The temp name is built from PATH's basename but
/// deliberately does *not* keep PATH's extension (`.reticle-save-<base>`,
/// not `<base>.reticle-save`) — a plain `<base>.tmp`-style name was
/// measured to get picked up by `find -name '*.v*'`-style filelist
/// generators that are common in real RTL trees.
///
/// The subshell parens around `( : > "$t" )` are load-bearing, not
/// decoration: `: > "$t" 2>/dev/null` (no parens) redirects stderr of
/// the whole `:`-with-redirect command, but a failure to OPEN `"$t"`
/// for that FIRST redirect is reported by the shell before the second
/// redirect (`2>/dev/null`) has taken effect — so the "cannot create"
/// diagnostic still lands on the real stderr, and `err_text`'s
/// first-line pick grabs it instead of the actual write's error.
/// Wrapping the whole thing in `( ... )` makes `2>/dev/null` apply to
/// the *subshell's* stderr as a unit, silencing that diagnostic no
/// matter which redirect inside it fails first. Verified against both
/// `bash` and `dash`.
///
/// If the commit step itself fails (target is actually a directory,
/// disk fills between the length check and the write-through, a
/// symlink target got revoked, etc.) the temp file is deliberately
/// **not** cleaned up (`exit 3`, with its path echoed to stderr) —
/// that's the user's unsaved work and dropping it silently would be
/// worse than a stray file. Every other failure path removes the temp
/// file.
///
/// M77 added a wall-clock timeout around `run()` (killing a stuck write is
/// only safe once killing it can no longer land a truncated commit — this
/// gate is exactly what makes that safe). Three facts about that timeout
/// specific to writes, documented rather than fixed here:
///
/// - **"data fully sent, remote committed, but the ack never came back"
///   reads identically to "we don't know".** If the remote finishes the
///   whole script (including the `cat "$t" > path` commit) and only the
///   final exit-status round trip is what's slow, killing the local ssh
///   at the timeout still severs the connection before that status
///   arrives: `status.code()` comes back `None`, `run()` maps that to
///   `-1`, and this function's existing "unknown" branch below reports it
///   as such. The save actually succeeded and we call it unknown anyway —
///   an honest false negative, not data loss, and deliberately accepted
///   rather than trying to distinguish it (there's no reliable way to,
///   short of re-reading the file over another `run()` call, which has
///   the same timeout problem one level up).
/// - **`disk_state` is deliberately left untouched on a write timeout.**
///   `crate::buffer::DiskState::Unknown` means "no baseline, so
///   `save-buffer` never treats the next save as a conflict" — i.e. it
///   *relaxes* the M75 guard. Adopting it here on a timeout would make an
///   uncertain outcome permissive in exactly the direction that's unsafe.
///   Leaving the OLD baseline in place instead is fail-closed the same
///   way the `exit 3` gap below already is: if this write actually landed
///   remotely, the next save rereads PATH, finds it doesn't match the
///   stale baseline, and reports "changed on disk ... save again to
///   overwrite" — one extra keystroke, not silent corruption.
/// - **The timeout is per-`run()` call, not per user-visible operation.**
///   A single remote save is measured at 3 `run()` calls (the M75 `test
///   -e` probe, `read_file`'s pre-existing-content check where relevant,
///   and the write itself), so the worst case for one `(save-buffer)` is
///   3x the configured timeout, not 1x. Opening a remote file is the same
///   shape (test -e / cat, plus the is_dir probe before that). This
///   module doesn't budget across calls; each `run()` gets the full
///   timeout independently.
///
/// M77 also does NOT make the wait interruptible: a hung remote command
/// now hangs for a bounded time instead of forever, but `C-g` still can't
/// break out of it early — that needs the event loop itself to notice
/// keypresses while a `run()` call is in flight, out of scope here.
///
/// Known gaps, documented rather than fixed here:
///
/// - **A stale save-conflict baseline after `exit 3`.** If the commit
///   step fails partway (PATH ends up partially written), the M75
///   save-conflict guard's cached digest still reflects the content
///   from BEFORE this attempt. The next save then rereads PATH, sees
///   it doesn't match that stale digest, and reports "changed on disk
///   by someone else" — even though the only writer was us, moments
///   ago, failing. This is fail-closed (the user gets a refusal, not
///   a silent overwrite) so it's not a safety bug, but it can be a
///   confusing one; a future milestone could special-case this by
///   invalidating the baseline entirely (falling back to "unknown")
///   on an `exit 3` instead of leaving it stale.
/// - **The `chmod 0o555` read-only-directory test is meaningless when
///   run as root** (see `ssh_tests.rs`) — root ignores permission
///   bits, so the primary same-directory temp file creation would
///   succeed and the test would pass for the wrong reason (never
///   actually exercising the `${TMPDIR:-/tmp}` fallback). This is a
///   generic Unix-testing limitation, not specific to this command.
/// - **`LC_ALL=C` (set by `run()`, see its call site) does not reach
///   `wc`/`cat` here.** `run()` sends the whole compound command as
///   `LC_ALL=C {cmd}`, but a bare `VAR=value` prefix without a
///   trailing simple command only sets `VAR` for the CURRENT shell,
///   not for subprocesses `cmd` itself later spawns — since `cmd` here
///   is `n=...; t=...; cat ...`, none of `wc`/`cat` see `LC_ALL` in
///   their environment. This happens not to matter: `wc -c` counts
///   bytes, which is locale-independent, so there's no correctness
///   gap from it — just don't assume this command's `wc`/`cat` are
///   running under a pinned-`C` locale, because they aren't.
pub fn write_file(rp: &RemotePath, text: &str) -> Result<(), String> {
    let q = shell_quote(&rp.path);
    let (dir, base) = split_dir_base(&rp.path);
    let stem_q = shell_quote(&format!("{}/.reticle-save-{}", dir, base));
    let n = text.len(); // str::len() is already the byte length
    let cmd = format!(
        "n={n}; t={stem_q}.$$; ( : > \"$t\" ) 2>/dev/null || t=${{TMPDIR:-/tmp}}/reticle-save.$$\n\
         cat > \"$t\" && [ \"$(wc -c < \"$t\")\" -eq \"$n\" ] || {{ rm -f \"$t\"; exit 1; }}\n\
         if cat \"$t\" > {q}; then rm -f \"$t\"; exit 0; \
         elif [ -r \"$t\" ]; then echo \"staged copy kept at $t\" >&2; exit 3; \
         else echo \"staged copy lost\" >&2; exit 3; fi"
    );
    let (_, errs, code) = run(&rp.host, &cmd, Some(text.as_bytes())).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            // See `write_timeout_err`'s doc comment for why this one
            // timeout gets its own wording instead of `run_err`'s, and
            // why it lives in a function rather than inline here.
            write_timeout_err()
        } else {
            run_err(&e, "write")
        }
    })?;
    if code == 3 {
        // The length gate passed but the write-through commit itself
        // failed. PATH may now be partially written (the ~40us window
        // described above) — unlike the length-gate failure below, we
        // can't promise PATH is untouched here.
        //
        // We ALSO can't unconditionally promise the staged copy at `$t`
        // survives: `cat "$t" > path` truncates PATH first and only
        // then reads `$t`, so if `$t` itself vanished between the
        // length gate passing and the commit running (an external tmp-
        // cleanup sweep, say), the result is PATH truncated AND the
        // staged copy gone — a narrow window, but claiming "your edit
        // is kept" there would be a flat lie. So the shell script
        // itself checks `[ -r "$t" ]` before deciding what to say
        // (`elif`/`else` above), and we surface exactly that line
        // rather than asserting anything ourselves.
        //
        // Deliberately use the LAST stderr line here, not `err_text`'s
        // usual first-line pick: when `cat "$t" > path` fails because
        // the shell can't even open PATH for the redirect (e.g. PATH
        // is a directory), the shell prints its OWN diagnostic first
        // ("/bin/sh: ...: Is a directory") before our `echo` ever runs
        // — so the first line is the generic shell error and the last
        // line is always our own "staged copy kept/lost" message.
        let staged = errs.lines().next_back().unwrap_or("").trim();
        return Err(format!(
            "Remote write to {} did not commit ({}): {}",
            format_path(&rp.host, &rp.path),
            code,
            if staged.is_empty() {
                "commit failed and no diagnostic was captured — check the remote manually"
            } else {
                staged
            }
        ));
    }
    if code != 0 && code != 1 {
        // Some exit status the script itself never produces: the remote
        // was interrupted from OUTSIDE (ssh's own 255 on a connection
        // failure, `run`'s -1 for a signal death, an OOM kill). The
        // script's three exits are 0/1/3 and each is reached only at a
        // point where we know what happened; an unknown code means the
        // remote may have been killed anywhere -- including partway
        // through the commit -- so we know nothing about PATH.
        //
        // M76 tail review caught this: the `untouched` wording below was
        // originally applied to EVERY non-zero code, which promised the
        // file was safe in exactly the scenario this milestone exists
        // for (a connection that drops mid-write). Saying "unknown" is
        // the honest answer and it is still actionable -- it tells the
        // user to go look, which "untouched" actively discourages.
        return Err(format!(
            "Remote write status unknown — {} (the remote was interrupted before it \
             could report back; the file there may be the old content or the new one)",
            err_text("write", &rp.host, &rp.path, &errs, code)
        ));
    }
    if code != 0 {
        // The length gate rejected the transfer (or the temp file
        // couldn't even be created) before the commit step ever ran,
        // so PATH was never touched: it's still exactly the old
        // content there was before this save attempt.
        // Front-loaded on purpose: the echo area is one terminal row and
        // an 80-column terminal (the industry default, and what this
        // project measures against) cuts a `Remote write failed on
        // /ssh:host:/long/path/...` prefix off well before any trailing
        // reassurance. Measured: with the invariant at the END, all the
        // user saw was `... (1): no error output`. The single fact that
        // matters -- their file on the build host is intact -- has to be
        // in the first few words or it may as well not be there.
        return Err(format!(
            "Remote file untouched — {} (new content is committed only after its byte \
             length is confirmed on the remote; this save didn't get that far)",
            err_text("write", &rp.host, &rp.path, &errs, code)
        ));
    }
    Ok(())
}

/// Delete a remote file via `rm` (M41: `(delete-file "/ssh:...")`).
pub fn remove_file(rp: &RemotePath) -> Result<(), String> {
    let (_, errs, code) = run(&rp.host, &format!("rm {}", shell_quote(&rp.path)), None)
        .map_err(|e| run_err(&e, "delete"))?;
    if code != 0 {
        return Err(err_text("delete", &rp.host, &rp.path, &errs, code));
    }
    Ok(())
}

/// Delete a remote directory via `rmdir`, or `rm -r` when RECURSIVE
/// (M41: `(delete-directory "/ssh:..." t)`).
pub fn remove_dir(rp: &RemotePath, recursive: bool) -> Result<(), String> {
    let cmd = if recursive {
        format!("rm -r {}", shell_quote(&rp.path))
    } else {
        format!("rmdir {}", shell_quote(&rp.path))
    };
    let (_, errs, code) = run(&rp.host, &cmd, None).map_err(|e| run_err(&e, "delete"))?;
    if code != 0 {
        return Err(err_text("delete", &rp.host, &rp.path, &errs, code));
    }
    Ok(())
}

/// Copy SRC_PATH to DST_PATH on the same remote HOST via `cp` (M41:
/// `(copy-file "/ssh:h:a" "/ssh:h:b")`). Callers (files.rs's
/// `copy-file`) have already checked both paths share a host.
pub fn copy_file(host: &str, src_path: &str, dst_path: &str) -> Result<(), String> {
    let cmd = format!("cp {} {}", shell_quote(src_path), shell_quote(dst_path));
    let (_, errs, code) = run(host, &cmd, None).map_err(|e| run_err(&e, "copy"))?;
    if code != 0 {
        return Err(err_text("copy", host, src_path, &errs, code));
    }
    Ok(())
}

/// Rename/move SRC_PATH to DST_PATH on the same remote HOST via `mv`
/// (M41: `(rename-file "/ssh:h:a" "/ssh:h:b")`) — files and
/// directories alike, matching GNU's `mv`.
pub fn rename_file(host: &str, src_path: &str, dst_path: &str) -> Result<(), String> {
    let cmd = format!("mv {} {}", shell_quote(src_path), shell_quote(dst_path));
    let (_, errs, code) = run(host, &cmd, None).map_err(|e| run_err(&e, "rename"))?;
    if code != 0 {
        return Err(err_text("rename", host, src_path, &errs, code));
    }
    Ok(())
}

/// `test -d` — is PATH an existing directory on the remote (M41:
/// `file-directory-p` on a `/ssh:` path, files.rs, and dired's
/// `find-file`/`C`/`R` dest checks below).
///
/// M77: distinguishes exit-code-level "no" from io-level "couldn't tell"
/// (a `run()` timeout, or ssh itself failing to start). Before M77 both
/// collapsed into `false` via `.unwrap_or(false)` — harmless when the
/// only possible `Err` was "ssh binary missing", but a timeout turning
/// into a silent `false` here would make `find-file` treat a merely-slow
/// remote as "definitely not a directory, must be a new/plain file",
/// which is a much worse lie than "I don't know". The exit-code case
/// (`Ok((_, _, code))`) is unchanged on purpose: a dead connection still
/// reads as `Ok(false)` exactly as before (ssh's own exit 255 on connect
/// failure), matching GNU's own `file-directory-p`, which has no
/// separate "can't tell" return either — only the NEW io-level failure
/// mode (timeout, or "ssh" not found) is now surfaced as `Err`.
pub fn is_dir(rp: &RemotePath) -> Result<bool, String> {
    run(
        &rp.host,
        &format!("test -d {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .map_err(|e| run_err(&e, "stat"))
}

/// `test -e` — does PATH exist at all on the remote, file or directory
/// alike (M41: `file-exists-p` on a `/ssh:` path, files.rs). A dead
/// connection reads the same as "doesn't exist" at the exit-code level,
/// matching `is_dir` above and GNU's own `file-exists-p` (no separate
/// "can't tell" return there either). M77: an io-level failure (a
/// `run()` timeout, or ssh itself failing to start) is a different case
/// from "doesn't exist" and is now `Err`, not folded into `false` — see
/// `is_dir`'s doc comment just above for why that distinction matters.
pub fn exists(rp: &RemotePath) -> Result<bool, String> {
    run(
        &rp.host,
        &format!("test -e {}", shell_quote(&rp.path)),
        None,
    )
    .map(|(_, _, code)| code == 0)
    .map_err(|e| run_err(&e, "stat"))
}

/// List a remote directory via `ls -al`, parsed into the shared
/// FileInfo shape (uid/gid columns carry the remote's names).
pub fn list_dir(rp: &RemotePath) -> Result<Vec<crate::dired::FileInfo>, String> {
    let (out, errs, code) = run(&rp.host, &format!("ls -al {}", shell_quote(&rp.path)), None)
        .map_err(|e| run_err(&e, "ls"))?;
    if code != 0 {
        return Err(err_text("ls", &rp.host, &rp.path, &errs, code));
    }
    Ok(parse_ls_output(&out))
}

/// Parse POSIX `ls -al` output lines into FileInfo. Lines that don't
/// look like entries (the `total N` header) are skipped.
pub fn parse_ls_output(out: &str) -> Vec<crate::dired::FileInfo> {
    let mut files = Vec::new();
    for line in out.lines() {
        let Some(fi) = parse_ls_line(line) else {
            continue;
        };
        files.push(fi);
    }
    files
}

fn parse_ls_line(line: &str) -> Option<crate::dired::FileInfo> {
    // perms nlink user group size month day time-or-year name [-> target]
    let mut it = line.split_whitespace();
    let perms = it.next()?;
    if perms.len() < 10
        || !matches!(
            perms.chars().next()?,
            '-' | 'd' | 'l' | 'c' | 'b' | 'p' | 's'
        )
    {
        return None; // "total 12" header etc.
    }
    let nlink: u64 = it.next()?.parse().ok()?;
    let user = it.next()?.to_string();
    let group = it.next()?.to_string();
    let size: u64 = it.next()?.parse().ok()?;
    let month = it.next()?;
    let day = it.next()?;
    let time_or_year = it.next()?;
    // The name is everything after the 8 metadata columns — recovered
    // by scanning the raw line so names with spaces survive.
    let mut rest = line;
    for _ in 0..8 {
        rest = rest.trim_start();
        let cut = rest.find(char::is_whitespace)?;
        rest = &rest[cut..];
    }
    let name_part = rest.trim_start();
    let is_symlink = perms.starts_with('l');
    let (name, link_target) = if is_symlink {
        match name_part.split_once(" -> ") {
            Some((n, t)) => (n.to_string(), Some(t.to_string())),
            None => (name_part.to_string(), None),
        }
    } else {
        (name_part.to_string(), None)
    };
    if name.is_empty() {
        return None;
    }
    let is_dir = perms.starts_with('d');
    let is_exec = !is_dir && perms.chars().skip(1).any(|c| c == 'x');
    Some(crate::dired::FileInfo {
        name,
        is_dir,
        is_symlink,
        link_target,
        is_exec,
        perms: perms.to_string(),
        nlink,
        user,
        group,
        size,
        mtime: format!("{} {:>2} {:>5}", month, day, time_or_year),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_paths() {
        assert_eq!(
            parse("/ssh:alice@server:/etc/hosts"),
            Some(RemotePath {
                host: "alice@server".into(),
                path: "/etc/hosts".into()
            })
        );
        assert_eq!(
            parse("/ssh:server:~/notes.txt"),
            Some(RemotePath {
                host: "server".into(),
                path: "~/notes.txt".into()
            })
        );
        assert_eq!(
            parse("/ssh:h:"),
            Some(RemotePath {
                host: "h".into(),
                path: ".".into()
            })
        );
        assert_eq!(parse("/etc/hosts"), None);
        assert_eq!(parse("/ssh:nopath"), None);
        assert_eq!(parse("/ssh::/x"), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("plain.txt"), "'plain.txt'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn ls_parsing() {
        let out = "\
total 16
drwxr-xr-x  3 alice staff   96 Jul 20 10:00 .
drwxr-xr-x 10 alice staff  320 Jul 19 09:00 ..
-rw-r--r--  1 alice staff  123 Jul 20 10:01 with space.txt
-rwxr-xr-x  1 alice staff   45 Jan  2  2024 run.sh
lrwxrwxrwx  1 alice staff    9 Jul 20 10:02 link.txt -> plain.txt
";
        let files = parse_ls_output(out);
        assert_eq!(files.len(), 5);
        assert_eq!(files[2].name, "with space.txt");
        assert_eq!(files[2].size, 123);
        assert!(files[3].is_exec);
        assert_eq!(files[3].mtime, "Jan  2  2024");
        assert!(files[4].is_symlink);
        assert_eq!(files[4].link_target.as_deref(), Some("plain.txt"));
        assert!(files[0].is_dir);
    }

    // -------------------------------------------------------------
    // M77 review: RETICLE_REMOTE_TIMEOUT is process-global
    // state, same reasoning as ssh_tests.rs's own ENV_LOCK -- these
    // tests serialize against each other (no other test in this file
    // touches the env var, but a future one might).
    // -------------------------------------------------------------
    static TIMEOUT_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Sets (or, with `None`, unsets) RETICLE_REMOTE_TIMEOUT for the
    /// duration of `f`, restoring whatever was there before -- even if
    /// `f` panics, since this holds the lock across the whole call and
    /// the restore happens on the way out regardless.
    fn with_timeout_env<T>(val: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = TIMEOUT_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("RETICLE_REMOTE_TIMEOUT");
        match val {
            Some(v) => std::env::set_var("RETICLE_REMOTE_TIMEOUT", v),
            None => std::env::remove_var("RETICLE_REMOTE_TIMEOUT"),
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        match prev {
            Some(v) => std::env::set_var("RETICLE_REMOTE_TIMEOUT", v),
            None => std::env::remove_var("RETICLE_REMOTE_TIMEOUT"),
        }
        match r {
            Ok(v) => v,
            Err(e) => std::panic::resume_unwind(e),
        }
    }

    /// M77 review, finding 1: a malformed `RETICLE_REMOTE_TIMEOUT`
    /// must never panic -- this is on the hot path of every remote
    /// operation, in library code, with no `catch_unwind` anywhere above
    /// it. Before the clamp, `inf`/`Infinity`/`3e400` (parses to infinity)
    /// panicked in `Duration::from_secs_f64`, and `1e19` panicked in the
    /// `Instant + Duration` addition inside `run()`'s wait loop instead
    /// (a merely-huge-but-finite value that `from_secs_f64` itself
    /// accepts). The two mechanisms have different thresholds, measured
    /// on macOS/aarch64: `1e18` trips neither, `1e19`-`1.8e19` trips the
    /// addition, `>= 2e19` trips `from_secs_f64`. The values below cover
    /// both sides of both thresholds.
    #[test]
    fn remote_timeout_env_var_edge_cases_do_not_panic() {
        // Non-finite (or unparseable) input falls back to the default,
        // same as unset -- not clamped to a bound, since "not a number"
        // isn't closer to one bound than the other.
        for bad in ["inf", "Infinity", "-inf", "3e400", "NaN", "abc", ""] {
            assert_eq!(
                with_timeout_env(Some(bad), effective_timeout_secs),
                Some(30.0),
                "input {:?} should fall back to the 30s default",
                bad
            );
        }
        // `0` and negative: the documented escape hatch, disabled.
        assert_eq!(with_timeout_env(Some("0"), effective_timeout_secs), None);
        assert_eq!(with_timeout_env(Some("-1"), effective_timeout_secs), None);
        assert_eq!(with_timeout_env(Some("-0.5"), effective_timeout_secs), None);
        // Huge-but-finite: clamped, not left to overflow `Instant + Duration`
        // later. `1e18` (below both panic thresholds), `1e19` (overflows
        // the addition), and `5e19` (panics inside `from_secs_f64` itself)
        // all land on the same clamped ceiling now.
        for huge in ["1e15", "1e18", "1e19", "5e19"] {
            assert_eq!(
                with_timeout_env(Some(huge), effective_timeout_secs),
                Some(MAX_REMOTE_TIMEOUT_SECS),
                "input {:?} should clamp to the ceiling",
                huge
            );
        }
        // Tiny-but-positive: clamped up to the floor, not treated as
        // "disabled" (only `<= 0.0` is).
        assert_eq!(
            with_timeout_env(Some("0.0000001"), effective_timeout_secs),
            Some(MIN_REMOTE_TIMEOUT_SECS)
        );
        // The actual panic sites: building the Duration, and doing the
        // Instant arithmetic `run()`'s wait loop performs every time
        // around. Both must survive every one of the inputs above once
        // routed through the real clamp.
        for val in ["inf", "3e400", "NaN", "abc", "1e18", "1e15", "0.0000001"] {
            with_timeout_env(Some(val), || {
                if let Some(d) = remote_timeout() {
                    let _ = std::time::Instant::now() + d;
                }
            });
        }
    }

    /// M77 review, finding 3: the echo area's real budget for a
    /// message text is 72, not 79 -- `Interp::error`'s `"error: "` prefix
    /// (7 chars) eats into the 79-character `cols - 1` row before this
    /// text ever gets there. Exercises every verb `run_err` is called
    /// with, plus `write_file`'s own timeout string, against the clamp
    /// bounds, an integer value, and a decimal value (including the kind
    /// of float noise `0.1 + 0.2`-style arithmetic can hand back) -- this
    /// is the guard against the next string edit silently blowing the
    /// budget, not a spot check.
    #[test]
    fn timeout_message_text_fits_the_echo_area_budget() {
        const PREFIX_LEN: usize = "error: ".len();
        const ECHO_AREA_WIDTH: usize = 79;
        let budget = ECHO_AREA_WIDTH - PREFIX_LEN;
        let verbs = ["read", "write", "stat", "delete", "copy", "rename", "ls"];
        let timeout_values = [
            "0.001",               // MIN_REMOTE_TIMEOUT_SECS itself
            "86400",               // MAX_REMOTE_TIMEOUT_SECS itself
            "5",                   // a plain integer value
            "0.30000000000000004", // float noise (0.1 + 0.2-style)
            // The actual worst case, and the one this list originally
            // missed (M77 tail review, finding 1): a big integer part
            // WITH a surviving fraction. Before the display function
            // dropped fractions above 100s these formatted as
            // "12345.679"/"86399.999" (9 characters) and pushed the
            // delete/rename messages to 73 against a 72 budget.
            "12345.6789",
            "86399.9994",
            "99.9994", // just under the drop-the-fraction threshold
        ];
        for tv in timeout_values {
            with_timeout_env(Some(tv), || {
                for verb in verbs {
                    let e = std::io::Error::new(std::io::ErrorKind::TimedOut, "x");
                    let msg = run_err(&e, verb);
                    assert!(
                        msg.len() <= budget,
                        "timeout={} verb={}: {:?} is {} chars, budget is {}",
                        tv,
                        verb,
                        msg,
                        msg.len(),
                        budget
                    );
                }
                // write_file's own message, through the SAME function
                // write_file calls -- not a re-typed copy of the literal
                // (M77 tail review, finding 5).
                let write_msg = write_timeout_err();
                assert!(
                    write_msg.len() <= budget,
                    "timeout={}: {:?} is {} chars, budget is {}",
                    tv,
                    write_msg,
                    write_msg.len(),
                    budget
                );
            });
        }
    }

    /// M77 review, finding 5 (precise version): `run()`'s doc comment
    /// argues stdin needs its OWN thread, not just stdout/stderr -- but
    /// `write_file`'s actual remote script produces only a few bytes of
    /// output, so a large-payload test that goes through `write_file`
    /// (see `ssh_tests.rs`) can't exercise the specific failure mode a
    /// synchronous stdin write creates: the classic two-way pipe deadlock,
    /// where OUR write blocks because the child's stdout buffer is full
    /// (nobody's reading it -- we're stuck writing stdin), and the child's
    /// write blocks because it's still waiting to read more stdin.
    /// `cat` -- which echoes everything it reads straight back out --
    /// reproduces that shape exactly: with a payload well past one pipe
    /// buffer (64 KB) in each direction, only threaded stdin AND threaded
    /// stdout draining lets this return at all.
    #[test]
    fn run_stdin_and_stdout_both_large_does_not_deadlock() {
        let _guard = TIMEOUT_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "se_remote_run_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("fake-ssh");
        std::fs::write(
            &shim,
            "#!/bin/sh\nwhile [ \"$1\" = \"-o\" ]; do shift 2; done\nshift\nexec /bin/sh -c \"$1\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let prev_ssh_bin = std::env::var_os("RETICLE_SSH_BIN");
        std::env::set_var("RETICLE_SSH_BIN", shim.to_str().unwrap());

        // 600 KB in each direction -- past the 64 KB pipe buffer both ways.
        let payload = vec![b'x'; 600_000];
        let start = std::time::Instant::now();
        let result = run("fake@host", "cat", Some(&payload));
        let elapsed = start.elapsed();

        match prev_ssh_bin {
            Some(v) => std::env::set_var("RETICLE_SSH_BIN", v),
            None => std::env::remove_var("RETICLE_SSH_BIN"),
        }
        std::fs::remove_dir_all(&dir).ok();

        let (out, _err, code) = result.expect("cat must not error");
        assert_eq!(code, 0);
        assert_eq!(
            out.len(),
            payload.len(),
            "everything written to stdin must come back out stdout"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "600 KB round-tripped through `cat` took {:?}, should be milliseconds -- a regression back to a synchronous stdin write would deadlock this instead",
            elapsed
        );
    }
}
