#!/usr/bin/env python3
"""Drive reticle's TUI inside a pty, feed it keystrokes, and print the
computed screen.

Purpose: find bugs by walking a "real editing workflow". M60's three bugs
(server stderr contaminating the screen, opening a file by relative path
creating a duplicate buffer, LSP disconnecting after jumping across files)
all came from a single run of this tool against `demo/rtl`, and none of
them appeared on the candidate list at the time. See PLAN.md's M60 record
for details.

    dev/tui-drive.py <script.txt> <binary> [raw-dump.bin]

The script has one command per line:

    OPEN <args...>   arguments to pass to the binary (only the last OPEN in
                     the whole script is used)
    KEY <spec>       send a keystroke, see KEYMAP below; `C-x`, `M-x`,
                     `'literal` are all supported
                     (**whitespace gets dropped, `'` is not elisp quote**,
                     see pitfalls 5 and 6)
    WAIT <secs>      only read output, don't send keys
    SHELL <cmd>      run a shell command **synchronously** at this point
                     (for modifying files externally)
    SHOT <label>     print the currently computed screen to stdout
    MARK <label>     insert a marker into the raw dump (used with the third argument)

When the third argument is given, every byte received is written verbatim
into that file. **This is the single most valuable trick**: M60 used it to
prove that leftover characters weren't drawn by the editor -- the frame
where C-g was pressed only emitted 105 bytes, and every other cell it
considered already blank. When the screen looks wrong, check the raw dump
first, don't suspect the editor first.

Seven pitfalls hit in practice (read before changing this file). **The
first three are about the editor and VT parsing; the last four are about
this tool's own script language -- and all four of those belong to the
category of "won't error, will just give you the wrong result":**

1. **The default is evil normal mode.** `/` triggers a search instead of
   inserting a slash, RET moves to the next line, and typing requires
   sending `i` first. `demo/README.md`'s usage instructions don't mention this.
2. **Escape sequences that span a read boundary must be buffered.**
   `os.read` can cut a sequence in the middle; without buffering, the
   screen shows fake leftover characters, easy to misdiagnose as an editor
   bug.
3. **`ESC[6 q` (cursor shape) contains an intermediate byte**, so the CSI
   regex must consume `[ -/]*`, or the whole sequence leaks through as
   visible characters.
4. **`OPEN`'s argument must be `-nw` (or `--tui`), not a bare `tui`.**
   `src/main.rs` only recognizes those two spellings; getting it wrong
   **doesn't error, it silently falls through to GUI mode**, and the GUI
   draws nothing at all in a pty -> the screen looks like keystrokes being
   echoed verbatim by the tty, easy to misdiagnose as the editor never
   having started. Took two rounds of investigation on 2026-08-17 (M67).
5. **`keybytes` uses `spec.split()`, so whitespace is dropped entirely.**
   `KEY '(list a b)` actually sends `(listab)`. To send elisp containing
   spaces (most commonly hit with `M-:` eval), each token has to be split
   into its own `KEY '<token>`, with `KEY SPC` in between. **Easy to miss
   by hand; generating the script with Python is more reliable.**
6. **`'` is only a marker meaning "what follows is literal text", not
   elisp's quote.** `KEY '(list 'foo)` eats both `'`s as markers and
   glues it into `listfoo`, reporting `void function: listfoo` -- looks
   like the editor is broken, but the script actually got mangled. To send
   an elisp quote, rewrite it in a form that doesn't need one.
7. **`ROWS, COLS = 40, 120` are hardcoded, and 120 columns hides an entire
   class of bug.** The industry-standard terminal is 80 columns, and
   narrower still once split into two columns. Confirmed in practice
   during M68: mode line right-segment overflow doesn't show at 120
   columns, but blows up immediately at 80 columns with an
   industry-typical RTL path length. **To test other widths, copy the file
   and change the constants** -- don't keep flipping them back and forth
   in the original (forgetting to revert makes every later measurement wrong).

The VT model only implements CUP / ED / EL / CUU-CUD-CUF-CUB / visible
characters / CR / LF / BS; SGR is always ignored (color doesn't affect bug
hunting). Before verifying anything finer-grained, confirm the model is
good enough for it -- don't just trust the screen at face value.
"""

import fcntl
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time

ROWS, COLS = 40, 120

CSI = re.compile(rb"\x1b\[([0-9;?]*)[ -/]*([@-~])")
OSC = re.compile(rb"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")
ESCX = re.compile(rb"\x1b[()][AB0]|\x1b[=><MND78]")

KEYMAP = {
    "ESC": b"\x1b",
    "RET": b"\r",
    "TAB": b"\t",
    "SPC": b" ",
    "BS": b"\x7f",
    "DOWN": b"\x1b[B",
    "UP": b"\x1b[A",
    "RIGHT": b"\x1b[C",
    "LEFT": b"\x1b[D",
}


class Screen:
    def __init__(self):
        self.buf = [[" "] * COLS for _ in range(ROWS)]
        self.r = 0
        self.c = 0
        self.pending = b""

    def put(self, ch):
        if self.r < ROWS and self.c < COLS:
            self.buf[self.r][self.c] = ch
        self.c = min(self.c + 1, COLS - 1)

    def feed(self, data):
        data = self.pending + data
        self.pending = b""
        # Pitfall 2: if the tail is a not-yet-complete escape sequence,
        # hold onto it until the next read.
        cut = data.rfind(b"\x1b")
        if cut != -1 and not CSI.match(data, cut) and len(data) - cut < 24:
            self.pending = data[cut:]
            data = data[:cut]
        data = ESCX.sub(b"", OSC.sub(b"", data))
        i = 0
        while i < len(data):
            b = data[i : i + 1]
            if b == b"\x1b":
                m = CSI.match(data, i)
                if not m:
                    i += 1
                    continue
                params, final = m.group(1).decode(), m.group(2)
                nums = [int(x) for x in params.replace("?", "").split(";") if x.isdigit()]
                self._csi(final, nums)
                i = m.end()
                continue
            if b == b"\r":
                self.c = 0
            elif b == b"\n":
                self.r = min(ROWS - 1, self.r + 1)
            elif b == b"\x08":
                self.c = max(0, self.c - 1)
            elif b >= b" ":
                x = data[i]
                n = 4 if x >= 0xF0 else 3 if x >= 0xE0 else 2 if x >= 0xC0 else 1
                try:
                    self.put(data[i : i + n].decode("utf-8"))
                except UnicodeDecodeError:
                    self.put("?")
                i += n
                continue
            i += 1

    def _csi(self, final, nums):
        if final in (b"H", b"f"):
            self.r = max(0, min(ROWS - 1, (nums[0] - 1) if nums else 0))
            self.c = max(0, min(COLS - 1, (nums[1] - 1) if len(nums) > 1 else 0))
        elif final == b"J":
            n = nums[0] if nums else 0
            if n == 2:
                self.buf = [[" "] * COLS for _ in range(ROWS)]
            elif n == 0:
                for cc in range(self.c, COLS):
                    self.buf[self.r][cc] = " "
                for rr in range(self.r + 1, ROWS):
                    self.buf[rr] = [" "] * COLS
        elif final == b"K":
            n = nums[0] if nums else 0
            if n == 0:
                for cc in range(self.c, COLS):
                    self.buf[self.r][cc] = " "
            elif n == 2:
                self.buf[self.r] = [" "] * COLS
        elif final == b"A":
            self.r = max(0, self.r - (nums[0] if nums else 1))
        elif final == b"B":
            self.r = min(ROWS - 1, self.r + (nums[0] if nums else 1))
        elif final == b"C":
            self.c = min(COLS - 1, self.c + (nums[0] if nums else 1))
        elif final == b"D":
            self.c = max(0, self.c - (nums[0] if nums else 1))

    def dump(self):
        return "\n".join("".join(row).rstrip() for row in self.buf)


def keybytes(spec):
    out = b""
    for tok in spec.split():
        if tok in KEYMAP:
            out += KEYMAP[tok]
        elif tok.startswith("C-") and len(tok) == 3:
            out += bytes([ord(tok[2].lower()) & 0x1F])
        elif tok.startswith("M-"):
            out += b"\x1b" + keybytes(tok[2:])
        elif tok.startswith("'"):
            out += tok[1:].encode()
        else:
            out += tok.encode()
    return out


def main():
    if len(sys.argv) < 3:
        sys.exit("usage: dev/tui-drive.py <script.txt> <binary> [raw-dump.bin]")
    script = open(sys.argv[1]).read().splitlines()
    binary = sys.argv[2]
    raw = open(sys.argv[3], "wb") if len(sys.argv) > 3 else open(os.devnull, "wb")

    openargs = ["-nw"]
    for line in script:
        if line.startswith("OPEN "):
            openargs = line[5:].split()

    env = dict(os.environ, TERM="xterm-256color", LINES=str(ROWS), COLUMNS=str(COLS))
    pid, fd = pty.fork()
    if pid == 0:
        os.execvpe(binary, [binary] + openargs, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    scr = Screen()

    def pump(secs):
        end = time.time() + secs
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.05)
            if not r:
                continue
            try:
                data = os.read(fd, 65536)
            except OSError:
                return
            if not data:
                return
            raw.write(data)
            scr.feed(data)

    pump(2.0)
    for line in script:
        line = line.strip()
        if not line or line.startswith("#") or line.startswith("OPEN"):
            continue
        cmd, _, rest = line.partition(" ")
        if cmd == "KEY":
            os.write(fd, keybytes(rest))
            pump(0.35)
        elif cmd == "WAIT":
            pump(float(rest))
        elif cmd == "SHELL":
            # Run a shell command between two keystrokes. Exists for
            # scenarios like "a file on disk got changed externally":
            # opening a window with WAIT and betting that a background
            # process finishes inside it is **a timing gamble**, and on
            # failure it looks exactly like "the editor failed to detect
            # the change" -- precisely the conclusion under test -- so a
            # false negative and a true positive look identical. Running
            # it synchronously removes timing as a variable.
            print(f"\n----- SHELL: {rest}")
            out = subprocess.run(rest, shell=True, capture_output=True, text=True)
            for stream in (out.stdout, out.stderr):
                if stream.strip():
                    print("      " + stream.strip().replace("\n", "\n      "))
            if out.returncode != 0:
                print(f"      (exit {out.returncode})")
        elif cmd == "MARK":
            raw.write(b"\x00MARK:" + rest.encode() + b"\x00")
        elif cmd == "SHOT":
            print(f"\n===== SHOT: {rest} =====")
            print(scr.dump())

    # Cleanup must always run: a leftover reticle would hold onto the
    # terminal settings, and any language server it spawned would become
    # an orphan along with it (PLAN.md records a similar incident).
    os.write(fd, keybytes("C-x C-c"))
    pump(1.0)
    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)


main()
