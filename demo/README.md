# demo/ — real code in every language Reticle supports

This directory exists to answer one question honestly: **is this editor
something you could actually write RTL in, or is it a toy?**

Nothing here is a snippet. The SystemVerilog design parses, lints and
format-checks clean under Google's Verible toolchain; the Rust file's
unit tests pass; the C file's self-test passes against published CRC-32
vectors; the C++ tool emits a waveform that the Python tool in the same
directory then reads back. Every claim on this page has a command next
to it so you can check it rather than believe it.

## Why `demo/` and not `examples/`

Cargo owns `tests/`, `benches/` and `examples/` — a `.rs` file dropped
into `examples/` becomes a build target. This directory holds source in
ten languages that cargo must not touch, so it gets a name cargo has no
opinion about, next to the existing `dev/` tooling directory.

## Layout

```
demo/
  rtl/              SystemVerilog — a small SoC, split across subdirectories
  rtl-verilog2001/  plain Verilog-2001 — a FIFO and its testbench
  tools/            Rust, C, C++, Java, Python, Perl, shell
  editor/           Emacs Lisp — a working init.el for RTL work
  docs/             Org — design notes for the SoC
```

Directories are named for the **role** the code plays, not the language
it is written in — which is why `tools/` holds seven languages at once.
That is what a real project looks like: an RTL repo has `rtl/` and
`scripts/`, not `python/` and `perl/`. `rtl-verilog2001/` is the one
name carrying a language, because being a different dialect is the
entire reason that directory exists; the `rtl-` prefix keeps it visibly
in the same family as `rtl/`.

## The SystemVerilog design

`rtl/` is deliberately spread over `pkg/`, `core/`, `mem/`, `bus/` and
`top/`, because that is the shape of a real RTL tree and it is the shape
that breaks naive tooling. Open `rtl/top/soc_top.sv` and:

| Key | What happens | Needs a language server? |
|---|---|---|
| `M-.` on `alu` | jumps to `rtl/core/alu.sv` | no |
| `M-,` | jumps back | no |
| `C-M-i` after a `.` in a port list | completes that module's port names | no |
| `C-c C-a` | expands the `/*AUTOINST*/` in `u_arbiter` | no |
| `C-c C-k` | deletes the expansion again | no |

`u_alu` is shown already expanded so the file reads as finished RTL;
`u_arbiter` is left unexpanded so there is something to try.

`rtl/verible.filelist` is the file list `verible-verilog-ls` reads to
learn what the design consists of. Reticle reads the same file for
its own module lookup, so the editor and the language server agree on
one list instead of each guessing.

## Verilog-2001 vs SystemVerilog

`rtl-verilog2001/` is plain IEEE 1364 — `reg`/`wire`, `always @(posedge)`,
no `logic`, no packed structs. It is here because plenty of production
RTL still looks exactly like that, and because it demonstrates a
subtlety worth knowing:

**IEEE 1364 was folded into IEEE 1800 in 2009.** There is one language
and one parser; "SystemVerilog support" always includes Verilog. What
does still differ is *style*: Verible's default rule set encodes a
SystemVerilog style guide and applies it to `.v` files too, asking for
`parameter int unsigned` and `task automatic` — constructs that do not
exist in Verilog-2001. `rtl-verilog2001/.rules.verible_lint` waives exactly
those four rules and explains each one. The SystemVerilog under `rtl/`
passes the full default rule set with **no waivers at all**.

## The other languages

| File | Language | What it is |
|---|---|---|
| `tools/bitvec.rs` | Rust | fixed-width bit vector with RTL wrapping semantics, 5 unit tests |
| `tools/crc32.c` | C | CRC-32 (IEEE 802.3) with a runtime-built table and a self-test |
| `tools/vcd_writer.cpp` | C++ | RAII waveform writer that emits a valid VCD |
| `tools/vcd_summary.py` | Python | reads a VCD, reports per-signal toggle counts and stuck signals |
| `tools/simlog_report.pl` | Perl | collapses a simulator log into distinct messages with counts |
| `tools/TimingReport.java` | Java | pulls the worst paths out of a static timing report |
| `tools/lint_rtl.sh` | shell | runs every check on this page |
| `editor/init-example.el` | Emacs Lisp | a real init.el for RTL work |
| `docs/design-notes.org` | Org | design decisions for the SoC |

They share a theme on purpose: these are the small tools that pile up
around an RTL project, which makes the set cohesive rather than ten
unrelated "hello, world"s.

## Checking the claims

```sh
# SystemVerilog + Verilog: parse, lint, format-check
./tools/lint_rtl.sh

# Rust: unit tests
rustc --test tools/bitvec.rs -o /tmp/bitvec && /tmp/bitvec

# C: CRC self-test against published vectors
cc -O2 -Wall -Wextra -o /tmp/crc32 tools/crc32.c && /tmp/crc32 --selftest

# C++ writes a waveform, Python reads it back
c++ -std=c++17 -O2 -Wall -Wextra -o /tmp/vcd_writer tools/vcd_writer.cpp
/tmp/vcd_writer > /tmp/demo.vcd
./tools/vcd_summary.py /tmp/demo.vcd

# Perl: collapse the bundled sample log
./tools/simlog_report.pl tools/sample_sim.log

# Python and Perl syntax
python3 -m py_compile tools/vcd_summary.py
perl -c tools/simlog_report.pl
```

### What was actually run

Verified on macOS (arm64) when this directory was written:

- `./tools/lint_rtl.sh` — **all three checks pass** across all 9 Verilog
  and SystemVerilog files (`verible-verilog-syntax`, `-lint`, `-format
  --verify`).
- `tools/bitvec.rs` — **5 tests passed, 0 failed**.
- `tools/crc32.c` — **5 vectors + streaming pass**.
- `tools/vcd_writer.cpp` — builds clean with `-Wall -Wextra`, produced a
  248-line VCD that `vcd_summary.py` parsed into 4 signals.
- `tools/simlog_report.pl` — collapsed the 31-line sample log to 5
  distinct messages, exit status 1 as designed.
- `tools/lint_rtl.sh`, `tools/vcd_summary.py`, `tools/simlog_report.pl`
  — pass `bash -n`, `py_compile` and `perl -c`.

Those six are one-time manual records: they need external toolchains, so
no test in this repo re-runs them.

And, driving the editor itself rather than the external toolchains:

- **All 22 files open in the correct major mode** — 18 in a
  language-specific mode (`verilog-mode`, `rust-mode`, `c-mode`,
  `c++-mode`, `python-mode`, `perl-mode`, `sh-mode`, `java-mode`,
  `emacs-lisp-mode`, `org-mode`) and 4 in `fundamental-mode`
  (`README.md`, `rtl/verible.filelist`,
  `rtl-verilog2001/.rules.verible_lint`, `tools/sample_sim.log`).
- From `rtl/top/soc_top.sv`, **`M-.` on `alu` lands in
  `rtl/core/alu.sv`** and port completion engages inside `u_regfile`'s
  port list — the two table rows above are measured, not asserted.
- That file sees **5 library files** even though `rtl/top/` contains
  only `soc_top.sv` itself: all five arrive via `rtl/verible.filelist`.
- Open any of the 8 files under `rtl/` and `rtl-verilog2001/` that have
  enough indented lines to go on, and the editor's indent step
  **follows that file's own 2-space style** instead of `verilog-mode`'s
  4-space default: a new statement line opened in a module body or
  inside a `begin`/`end` block lands where `verible-verilog-format`
  already wants it, so the indentation no longer diffs on save.
  Measured on `rtl/core/alu.sv`, which is what the test asserts.

  Two honest exceptions, both checked rather than assumed.
  `rtl/include/soc_defs.svh` has only 2 indented lines out of 28 —
  below the detector's 5-sample confidence floor, so it keeps the
  4-space mode default; the detector declines to guess rather than
  guessing from two samples. And a line opened *inside* an
  instantiation's port-connection list still lands at the enclosing
  block's depth (column 2 in `rtl/top/soc_top.sv`, where the
  surrounding `.clk_i (clk_i),` lines are aligned at 6): matching
  continuation alignment needs the paren-column rule this indent
  engine has never had (a documented M36 limit, see `indent.el`'s
  header), and detecting the file's step does not change that.

The four editor claims above, plus the `C-c C-a` / `C-c C-k` rows of
the keybinding table, are the ones a test now re-checks on every run:
`crates/core/tests/demo_smoke_tests.rs`. The "5 library files" count is
not asserted directly — the cross-file jump and completion tests only
prove that resolution reaches other directories at all. Add or remove a
file under `demo/` and that test fails until its expected-mode table and
the count above are updated; that is the point of it. Everything in this
section was a hand-checked one-off before it existed, and the file count
had already drifted (it read "15") by the time anyone looked again.

That run also caught a real bug in `editor/init-example.el` — it used a
bare `major-mode` variable, which does not exist in this editor (the
current mode is read with `(major-mode-internal-get)`). Worth saying out
loud, because it is the whole argument for this directory: sample code
nobody executes drifts into being wrong.

Not verified, for lack of a toolchain on that machine:

- `tools/TimingReport.java` — **no JDK installed**, so it has never been
  compiled. It is the one file on this page whose claims rest on reading
  rather than running.
- `rtl-verilog2001/fifo_sync_tb.v` — **no simulator installed**. The
  testbench is written for Icarus Verilog
  (`iverilog -o /tmp/fifo_tb fifo_sync.v fifo_sync_tb.v && /tmp/fifo_tb`)
  but has not been run, so treat "PASS" in its output as intent, not
  evidence.

## Editing this directory

The files here are not wired into `cargo test`; changing one cannot
break the build. If you add a language, add a file that does something
real in it — the point of this directory is entirely lost on a sample
that would embarrass you in a code review.
