#!/usr/bin/env python3
"""A fake LSP server with configurable behavior: purpose-built to reproduce
what the editor does when "the server misbehaves".

    dev/fake-lsp.py <mode> [--delay SECS]

A real server can only ever reproduce "everything works fine". What M65
needed to prove (connected but silent -> editor freezes, even `C-g` can't
save it) can't be produced by any real server, so this script exists. It
has already caught:

* **M65's bug itself**: run `deaf` mode through `dev/tui-drive.py` and
  confirm that after `M-x lsp` the screen freezes, keystrokes have no
  effect, and `C-g` does nothing — and confirm that once fixed, the same
  script recovers interactivity after the timeout.
* **A test that was proving something else**: M65's kill-on-drop test
  originally used `cat >/dev/null` as a "deaf server". It **does** react to
  stdin being closed (it exits on its own once it reads EOF), so the "the
  child process is gone" the test observed was the effect of closing the
  pipe, not the effect of `kill()` — removing the `kill()` in `Drop` still
  left the test green. The failure mode that needed guarding against is
  **ignoring stdin EOF**, which is the `mute` mode.
  **"Deaf" and "not reading" are two different things, don't conflate them.**

## Usage: hooking it up to the editor

Point `lsp-server-alist` at it with `M-:` (no spaces in the path,
`dev/tui-drive.py`'s script splits on whitespace):

    (setq lsp-server-alist '((verilog-mode . ("/path/to/dev/fake-lsp.py" "deaf"))))

Then `M-x lsp`. Paired with `dev/tui-drive.py` this gives a repeatable
end-to-end repro.

## Modes

* `deaf`      -- reads stdin fine (so no backpressure from a full pipe),
                 but **never replies to anything**. M65's main material:
                 initialize goes out and vanishes without a trace.
* `mute`      -- **doesn't read stdin at all**, and doesn't reply either.
                 Closing stdin doesn't make it exit; only kill can. Used to
                 verify "the destructor/cleanup path actually kills the
                 child process" -- `deaf` would give a false pass here.
* `stop-read` -- answers `initialize` normally, **then stops reading
                 stdin**. Left here for the next candidate: `lsp-send`'s
                 `write_all` blocks synchronously when the pipe is full,
                 and `lsp--sync-buffer-now` is called once per buffer on
                 every idle tick, so the symptom matches M65 (screen
                 freezes) but the cause is on the write side, **and
                 requires no LSP command from the user at all**. To
                 reproduce it, use this mode with a file large enough to
                 fill the OS pipe buffer (typically 64KB).
* `chatty`    -- keeps sending `$/progress` notifications after answering
                 initialize, but **never replies to what you actually
                 asked**. Verifies "the wait budget is a total, not reset
                 on every incoming message": if the budget got reset by
                 every message, it would never time out.
* `slow`      -- replies to initialize only after `--delay` seconds.
                 Verifies behavior on both sides of the timeout boundary.
* `dies`      -- exits immediately. Verifies that "connection died" and
                 "timed out" are handled separately (they're distinct
                 values on the elisp side).

## Which modes have actually been run (2026-08-16)

`deaf` has been through the full end-to-end path: `dev/tui-drive.py` opens
a file -> points at this script -> `M-x lsp` -> after the timeout the
editor is still operable (the cursor still moves), and the mode line has
**no** `LSP` segment (proving it's connected to this script and not real
verible -- a real server would connect). **The other five modes only have
code, they have not been run** -- before using one for the first time,
verify yourself that it actually does what you think it does; don't treat
"the script exists" as "the behavior is correct".

## Pitfalls hit in practice

1. **Message length must be counted in bytes, not characters**, and the
   header uses `\\r\\n\\r\\n`. Writing this kind of fake server in shell and
   forking `wc -c` once per message would make the send rate
   load-dependent -- one M65 mutation "survived" on a busy machine because
   of exactly this (the code path with the guard removed only hangs when
   the server consistently wins the race against the budget). Doing the
   count directly in Python avoids this problem.
2. **`chatty`'s interval must be clearly smaller than the editor side's
   budget**, for the same reason as above.
3. stdout must be flushed, or messages get stuck in Python's own buffer,
   which looks exactly like the server not replying.
"""

import argparse
import json
import sys
import threading
import time


def send(obj):
    body = json.dumps(obj).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def read_message():
    """Read one LSP message, return the parsed dict; returns None on EOF."""
    header = b""
    while b"\r\n\r\n" not in header:
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        header += ch
    length = 0
    for line in header.decode("latin-1").split("\r\n"):
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1])
    body = sys.stdin.buffer.read(length)
    if not body:
        return None
    return json.loads(body)


def initialize_result(req_id):
    return {"jsonrpc": "2.0", "id": req_id, "result": {"capabilities": {}}}


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["deaf", "mute", "stop-read", "chatty", "slow", "dies"])
    ap.add_argument("--delay", type=float, default=3.0,
                    help="seconds to wait before replying to initialize in slow mode (default 3)")
    ap.add_argument("--interval", type=float, default=0.05,
                    help="notification interval in seconds for chatty mode (default 0.05)")
    args = ap.parse_args()

    # stderr gets captured by the editor into *background-output*, which
    # doubles as confirmation that it actually started. Note: the
    # "Background output captured" echo message **only appears once per
    # session**, and it can cover up a failure message from that same
    # instant (confirmed in M65). Don't rely on the first appearance to
    # read the error message clearly.
    print(f"fake-lsp: {args.mode}", file=sys.stderr, flush=True)

    if args.mode == "dies":
        return

    if args.mode == "mute":
        # Don't read a single byte: stdin closing won't make it exit, only
        # a signal can kill it.
        while True:
            time.sleep(3600)

    if args.mode == "deaf":
        # Read everything (no backpressure), but never reply.
        while sys.stdin.buffer.read(1):
            pass
        return

    # All other modes wait to see initialize before doing anything.
    while True:
        msg = read_message()
        if msg is None:
            return
        if msg.get("method") != "initialize":
            continue
        if args.mode == "slow":
            time.sleep(args.delay)
        send(initialize_result(msg.get("id")))
        break

    if args.mode == "stop-read":
        # Stop reading stdin once we've replied: subsequent
        # didOpen/didChange will fill the pipe, and the block happens on
        # the editor's **write side**.
        while True:
            time.sleep(3600)

    if args.mode == "chatty":
        # Keep talking but never reply to any id. Spin up a separate
        # thread to drain stdin, so backpressure doesn't become another
        # variable (which would make the repro's cause non-unique).
        threading.Thread(target=lambda: [None for _ in iter(
            lambda: sys.stdin.buffer.read(4096), b"")], daemon=True).start()
        while True:
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {}})
            time.sleep(args.interval)

    # slow mode: after replying to initialize, act as a normal but silent server.
    while read_message() is not None:
        pass


if __name__ == "__main__":
    main()
