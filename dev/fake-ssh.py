#!/usr/bin/env python3
"""Fake ssh: makes `/ssh:` remote path behavior reproducibly measurable,
without needing a real ssh host.

It logs every call, sleeps for a configurable round-trip time (RTT), and
then performs the same operation against a local fakeroot. Exists for the
same reason as `dev/fake-lsp.py`: **the real thing can only reproduce
"everything's fine"**, and what needs measuring is exactly "what does the
editor do when things aren't fine" -- a slow remote host, unreachable, or
deep paths.

## What it has caught (M66, directly decided that milestone)

`crates/core/lisp/lsp.el`'s `lsp--auto-attach-client` has had an `/ssh:`
guard since M63, but the interactive command `lsp` did not. Using this
script plus `dev/tui-drive.py` on a real TUI, opening
`/ssh:buildhost:/proj/rtl/core/alu.sv` and pressing `M-x lsp`, measured:

* **32 synchronous ssh calls** (4 directory levels x 8
  `lsp--project-root-markers`), with ssh.log's timestamps showing a
  **6.15 second** freeze (RTT 150ms). Single-threaded event loop,
  completely unresponsive during that time. When the host is unreachable
  it's 32 x `ConnectTimeout=5` = **160 seconds**.
* And then it **actually connects** -- attaching a local server to
  `file:///ssh:buildhost:/proj/...`.

Without this script, both of these could only be argued from inference.
**ssh.log's per-call timestamps are the key evidence**: they turn "32
calls" and "6.15 seconds" into something you can paste, not a claim.

## How to hook it up

The editor uses the `RETICLE_SSH_BIN` hook to decide which ssh
executable to use (`ssh_bin()` in `crates/core/src/remote.rs`), so:

    mkdir -p /tmp/fakeroot/proj/rtl/core
    cp demo/rtl/core/alu.sv /tmp/fakeroot/proj/rtl/core/
    RETICLE_SSH_BIN=$PWD/dev/fake-ssh.py FAKE_SSH_RTT=0.15 \\
        python3 dev/tui-drive.py script.txt ./target/release/reticle

The fakeroot and log locations default to under **this file's own
directory** (`fakeroot/`, `ssh.log`), overridable via `FAKE_SSH_ROOT` /
`FAKE_SSH_LOG` -- `dev/` is version-controlled, so redirect to /tmp when
actually testing.

Environment variables:

* `FAKE_SSH_RTT`  -- how many seconds to sleep per call, default 0.15.
  **This is the dial for measuring freeze time**: a real LAN is roughly
  0.01-0.05, cross-country roughly 0.15-0.4, over a VPN it can be longer.
* `FAKE_SSH_ROOT` -- which local directory a remote absolute path maps to
  (default `<this file's dir>/fakeroot`).
* `FAKE_SSH_LOG`  -- where per-call records go (default
  `<this file's dir>/ssh.log`), format is
  `<epoch seconds> <remote command>`, one entry per line. **When the
  command itself contains a newline it spans multiple lines** (true of
  `write_file` since M76) -- to correlate timestamps, only take lines
  starting with an epoch value.

**Two failure modes, don't mix them up** (their observable results are
opposite; picking the wrong one measures a backwards conclusion):

* `FAKE_SSH_TRUNCATE_STREAM=<n>` -- **the connection dropped**: only feed
  the first n bytes of stdin, then a clean EOF; the remote command still
  runs to completion and returns its own exit code. M76 uses this.
* `FAKE_SSH_HANG_RE=<regex>` -- **the connection is fine, the remote side
  just never replies**: matching commands never finish, never print
  anything. `ConnectTimeout=5` doesn't apply to it (the connection was
  already established), so this is the only mode that can observe "`run()`
  has no wall-clock timeout". M77 uses this. The regex is matched against
  the full command string, **including the `LC_ALL=C ` prefix**.

## Pitfalls hit in practice (read before touching this file)

1. **The last argument `remote.rs` sends is `LC_ALL=C <cmd>`. Before it
   there's also `-o BatchMode=yes -o ConnectTimeout=5 <host>`, which this
   script always ignores, looking only at `argv[-1]`.**

   This point used to say "the prefix needs to be stripped before it will
   parse", and **stripping it was wrong** (corrected 2026-08-23, the fifth
   time "the model was wrong"). Real ssh hands the whole string to the
   remote shell, and `LC_ALL=C` takes effect there; stripping it means the
   fake environment runs under **the user's own locale**, while the real
   environment is always C. This had a measurable consequence: `ls -al`
   under a Chinese locale prints `8月 23 06:08` (multi-byte), and after
   switching directories dired's `..` shows up on screen as `. .` --
   **at first glance this looks like an editor redraw bug**, and under real
   ssh this screen could never occur. The stripping was a leftover from the
   v1 "shlex.split + execv" era: execv doesn't understand `VAR=v cmd`, so
   stripping was mandatory back then; once v3 switched to handing it to
   `/bin/sh -c` (see pitfall 3), the stripping should have been removed
   along with it, and wasn't. **The first four model errors were all "the
   fake does something the real one doesn't"; this one is the opposite:
   the fake failed to do something the real one does. Check both
   directions.**
2. **`true` is a liveness probe and must return 0.** `read_file`
   (`remote.rs`) sends a `true` after `test -e` fails, to distinguish
   "file doesn't exist" from "can't connect". If this script makes `true`
   return non-zero, the editor will misreport "new file" as "connection
   failed".
3. **The first version was written in shell, using `${CMD//\\//$ROOT/}` to
   do path substitution -- which also rewrote the command name itself**
   (`test` became `$ROOT/test`), breaking on its own self-test. Switched
   to Python using `shlex.split`, prefixing only **arguments starting with
   `/`**, leaving the command name untouched.
4. **The `fakeroot` directory structure has to already exist.** The script
   only does path mapping and won't create directories for you;
   `lsp--project-root` keeps trying upward all the way to `/`, so
   `<root>/` itself must exist too.

## Which remote operations have actually been run (2026-08-16)

M66's end-to-end run only exercised opening one remote file with
`find-file` (`test -d` -> `test -e` -> `cat`) and `lsp--project-root`'s
marker scan (`test -e` x 32). **The rest have not been run**:

* ~~`write_file` goes through `cat > path` and **feeds content via
  stdin**~~ **verified on 2026-08-22, and it was broken at the time**,
  broken twice in the same day, same root cause both times:
  1. `>` wasn't being treated as a redirection (with no shell involved,
     execv handed it to `cat` directly as a literal argument), so remote
     saves failed unconditionally. **The failure message
     `Remote write failed ... cat: >:` showed up on the editor's echo
     line, and at first glance looked like the editor's remote save was
     broken.**
  2. After patching in a special case for "a single redirection", M76
     changed writes to a **compound command** like
     `cp -p ... ; cat > "$t" && mv ...`, and the special case stopped
     being enough again.
  **Both times the model was wrong, not that one special case was
  missing**: real ssh hands the command to the remote shell. This script
  now instead "maps quoted absolute paths into fakeroot, then genuinely
  hands it to `/bin/sh -c`", so compound commands, `&&`, and `$$` just
  work naturally. **The rule stated at the end of this section paid off
  twice within a single day.**
* `list_dir`'s `ls -al`, `remove_file`'s `rm`, `copy_file`/`rename_file`'s
  `cp`/`mv`, `is_dir`'s `test -d` (on a bare path) -- same as above.

**Before using any of the above for the first time, verify yourself that it
actually does what you think it does** -- don't treat "the script exists"
as "the behavior is correct". This is the same rule stated in
`dev/fake-lsp.py`'s header, and it was earned the same way, by a real
incident.
"""

import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("FAKE_SSH_ROOT", os.path.join(HERE, "fakeroot"))
LOG = os.environ.get("FAKE_SSH_LOG", os.path.join(HERE, "ssh.log"))


def main():
    # remote.rs's call shape: ssh -o BatchMode=yes -o ConnectTimeout=5 <host> "LC_ALL=C <cmd>"
    # Everything before that is always ignored; the remote command is always the last argument.
    cmd = sys.argv[-1] if len(sys.argv) > 1 else ""

    with open(LOG, "a") as f:
        f.write(f"{time.time():.3f} {cmd}\n")

    time.sleep(float(os.environ.get("FAKE_SSH_RTT", "0.15")))

    # **The `LC_ALL=C ` prefix is deliberately not stripped** (fixed
    # 2026-08-23, see header pitfall 1). Real ssh hands the whole string to
    # the remote shell, where the prefix takes effect; stripping it would
    # remove something the editor **depends on** from the model. Stripping
    # was a leftover from the v1 "shlex.split + execv" era -- execv doesn't
    # understand `VAR=v cmd`, so it was mandatory back then; once v3
    # switched to handing it to `/bin/sh -c`, stripping went from
    # "necessary" to "wrong". Handing it to a shell also happens to
    # preserve the subtlety documented in `remote.rs::write_file`: for a
    # **compound** command like `LC_ALL=C n=...; ...; cat ...`, the prefix
    # only sets the current shell's environment, and the `wc`/`cat` spawned
    # afterward don't see it -- that's exactly how real ssh behaves, so the
    # model has to match it.

    if not cmd.strip():
        return 0

    # Real ssh hands the remote command to **the remote shell**, so `>`,
    # `&&`, `;`, `$$` are all interpreted by the shell. This script used to
    # do `shlex.split` + direct execv, so:
    #   - 2026-08-22: `cat > path`'s `>` became a literal argument to cat,
    #     remote saves failed unconditionally, and the error message showed
    #     up on the editor's echo line, looking like the editor itself was broken.
    #   - Later the same day: once M76 changed writes to a compound command
    #     like `cp -p ... ; cat > "$t" && mv ...`, the special-case handling
    #     for a single redirection wasn't enough either.
    # Both times traced to the same root: **the model was wrong, not that a
    # special case was missing**. Switched to "map paths into fakeroot,
    # then genuinely hand it to /bin/sh", matching real ssh's semantics, so
    # compound commands and shell variables just work naturally.
    #
    # Path mapping relies on one fact: absolute paths sent by `remote.rs`
    # always go through `shell_quote`, i.e. they're wrapped in single quotes
    # (`'/proj/alu.sv'`). So replacing `'/` with `'<ROOT>/` rewrites the
    # whole thing without having to parse the command's structure. Command
    # names (cat/mv/cp/test) don't contain single quotes, so they're unaffected.
    mapped = cmd.replace("'/", "'" + ROOT + "/")

    # Liveness probe (see header pitfall 2): read_file uses it to
    # distinguish "file doesn't exist" from "can't connect", and it must
    # return 0, or new files get misreported as connection failures.
    # (The prefix is no longer stripped, so `LC_ALL=C true` needs to be
    # recognized here too.)
    if mapped.strip() in ("true", "LC_ALL=C true"):
        return 0

    # --- simulating "the connection drops before the data finishes sending" ---
    #
    # **This was gotten wrong twice, and the second time nearly built an
    # entire milestone on a false conclusion.**
    #
    # Wrong way #1 (regex-find `> 'path'` and truncate it): once M76 changed
    # writes to redirect into a shell variable `"$t"` instead, the regex no
    # longer matched, so the mode silently failed to trigger, the command
    # succeeded as usual, and it looked like the fix was working.
    #
    # Wrong way #2 (actually run it, then kill the whole process group):
    # looks faithful, but **is not what real ssh does**. When real ssh
    # disconnects: the ssh client dies / TCP drops -> sshd closes the
    # channel -> the remote command's stdin gets a **clean EOF**. And
    # without a tty (this codebase never passes `-t`), sshd **does not**
    # send SIGHUP, so the remote command is not killed -- it finishes
    # whatever work remains. For `cat > tmp && mv tmp target`, `cat` reads
    # EOF and returns 0, and `mv` still runs -- **a truncated file gets
    # atomically installed, with exit code 0**. Simulating this by killing
    # the process instead makes the remote command exit non-zero, so the
    # commit step never runs, measuring "the original file is intact",
    # a conclusion that is **the opposite of reality**.
    #
    # So the correct model is `FAKE_SSH_TRUNCATE_STREAM=<n>`: feed only the
    # first n bytes of stdin to the command, then close normally, and
    # return the command's own exit code.
    trunc = os.environ.get("FAKE_SSH_TRUNCATE_STREAM")
    if trunc is not None:
        n = int(trunc)
        data = sys.stdin.buffer.read()
        proc = subprocess.Popen(["/bin/sh", "-c", mapped], stdin=subprocess.PIPE)
        try:
            proc.stdin.write(data[:n])
        except BrokenPipeError:
            pass
        proc.stdin.close()          # a clean EOF, not a killed process
        return proc.wait()

    # --- simulating "connected fine, but the remote side never replies" ---
    #
    # `FAKE_SSH_HANG_RE=<regex>`: when the remote command (**including the
    # `LC_ALL=C ` prefix**, since that's what `argv[-1]` actually contains)
    # matches this regex, this script **never finishes**: it still reads
    # stdin (otherwise the writer would get EPIPE, a different failure
    # mode), keeps stdout/stderr open, prints nothing, and never exits.
    #
    # This is a **different** failure mode from `FAKE_SSH_TRUNCATE_STREAM`,
    # don't mix them up: the former is "the connection dropped" (clean EOF,
    # the command still runs to completion, there's an exit code); this one
    # is "the connection is perfectly fine, the remote side just doesn't
    # reply" -- the ssh client is alive, and `wait_with_output()` never
    # returns. `ConnectTimeout=5` has no effect on it at all, because the
    # connection **was already established**.
    #
    # Basis for the model (confirmed in M76): killing the local ssh client
    # does **not** stop the remote command; the remote side finishes
    # whatever work remains. So the "timeout" semantics measured by this
    # mode mean "**I gave up waiting**", not "I canceled it" -- the remote
    # file's final state is unknown at the moment of timeout.
    hang_re = os.environ.get("FAKE_SSH_HANG_RE")
    if hang_re and re.search(hang_re, cmd):
        with open(LOG, "a") as f:
            f.write(f"{time.time():.3f} HANG (never responds): {cmd}\n")
        try:
            sys.stdin.buffer.read()   # accept the writer's data, don't let it EPIPE
        except OSError:
            pass
        while True:
            time.sleep(3600)

    return subprocess.call(["/bin/sh", "-c", mapped])


if __name__ == "__main__":
    sys.exit(main())
