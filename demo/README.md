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
  rtl-verilog2001/  plain Verilog-2001 — a FIFO, a Gray-code counter, a non-ANSI
                    top wrapping both, and testbenches
  verif/            SystemVerilog — testbenches and verification-only material
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
`u_arbiter` is left unexpanded so there is something to try. The
`AUTO_TEMPLATE` block above `u_arbiter` is required, not decorative —
the arbiter's own port names don't match this module's signal names,
and AUTOINST without a template still wires up same-named nets, which
here means wiring to names that don't exist in this module. Try
deleting the template and re-expanding: the result doesn't compile —
the language server reports undeclared identifiers where the connected
signals should be, plus a batch of implicit nets papering over them.

This is checkable by eye through the mode-line, not just by reading the
comment: open `rtl/top/soc_top.sv` and the language server reports 17
warnings, 0 errors, 14 of them from `u_arbiter`'s still-unexpanded
ports. Run `M-x verilog-auto` (or `C-c C-a`) and the server's count
drops to 3 — all three honest (an intentional empty connection and two
signals that are assigned but never read), none suppressed. What the
mode-line itself shows is a different, smaller pair of numbers: its
`!N` counts *lines that carry a diagnostic*, not diagnostics, and one
line can carry several (u_arbiter's own instantiation line accounts
for 9 of the 17 by itself). So the 17 land on 9 lines and the 3 land
on 3 lines — the mode-line reads `!9`, then `!3` after the expansion.
Both number pairs are correct; they're just counting different things.

`rtl/verible.filelist` is the file list `verible-verilog-ls` reads to
learn what the design consists of. Reticle reads the same file for
its own module lookup, so the editor and the language server agree on
one list instead of each guessing.

## Verification material: `verif/`

`interface`, `modport`, `generate`/`genvar`, `class`, `program`,
`covergroup` and concurrent assertions are all real SystemVerilog
constructs this editor has code paths for (highlight queries, scope/
breadcrumb kinds, indent rules) but that, until now, `rtl/` had no real
example of — every one of them was validated only against hand-written
snippets inside the Rust test suite. `verif/` is where that material
lives: `bus/axi4_lite_if.sv` under `rtl/` is the `interface` (three
`modport`s, plus two labelled concurrent assertions checking AW/AR
handshake stability), `core/clk_gate.sv` is the `always_latch`,
`mem/sram_bank.sv` is the `generate`/`genvar` (a nested `if` generate
inside a `for` generate) — those three stay under `rtl/` because they
are genuinely synthesizable design, not testbench. `verif/` itself
holds the parts that only make sense in simulation:
`soc_verif_pkg.sv` (a `class`-based read/write checker),
`sram_bank_tb.sv` (a `program` block driving the stimulus, plus a
guarded `covergroup`), and `axi4_lite_monitor.sv` (a passive protocol
checker whose own port is interface-typed — see "Checked by Verible,
never simulated" below for why it lives here rather than being wired
into the testbench). `verif/verible.filelist` is a second file list,
separate from `rtl/verible.filelist` on purpose: the RTL list stays a
clean statement of "what synthesizes," and the verification list adds
the testbench files plus everything they depend on
(`../rtl/pkg/soc_pkg.sv` and friends), each path resolved relative to
`verif/` itself, the same convention `rtl/verible.filelist` already
uses relative to `rtl/`.

`rtl/.slang/server.json` is `slang-server`'s own per-project config
file format — `slang-server` is the other Verilog language server
Reticle talks to, and this is what you'd hand-author if you pointed a
plain `slang-server` (e.g. from a VS Code workspace opened at `rtl/`
directly) at this design without going through Reticle at all:
`{"flags": "-I include"}` tells it where `rtl/include/soc_defs.svh`
lives, so `rtl/top/soc_top.sv`'s `` `include `` resolves instead of
reporting a spurious severity-1 `'soc_defs.svh': No such file or
directory`.

Reticle itself no longer depends on this file (M133): its own project
root for this design is `demo/` (M132's filelist connected-component
computation), so a config that only slang reads from `<rootUri>/.slang/`
is never seen by the server Reticle actually starts. Instead, Reticle
computes the include directories itself — from the very `+incdir+
include` line in `rtl/verible.filelist` you can see above — and pushes
them to a connecting slang-server via its `slang.setBuildFile` command
after the handshake completes. `rtl/.slang/server.json` stays in the
tree anyway, as a correct example of the format for anyone pointing
their own slang-server at `rtl/` by hand. Two details worth knowing if
you do that: the file name must be exactly `server.json` (`config.json`
or anything else under `.slang/` is silently ignored), and the paths
inside `flags` are resolved relative to the language server process's
own working directory, not the workspace root or the config file's
location.

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
| `tools/run_sim.sh` | shell | actually simulates the FIFO and the AXI4-Lite testbench with Icarus |
| `editor/init-example.el` | Emacs Lisp | a real init.el for RTL work |
| `docs/design-notes.org` | Org | design decisions for the SoC |

They share a theme on purpose: these are the small tools that pile up
around an RTL project, which makes the set cohesive rather than ten
unrelated "hello, world"s.

## Opening this design in the editor

If you only run one thing here, run this:

```sh
./run_editor.sh              # GUI window on the SoC top level
./run_editor.sh --tui        # or in this terminal instead
./run_editor.sh --unused     # the stub module `/*AUTOUNUSED*/` fills in
./run_editor.sh --help       # every option, and the keys worth trying
```

It builds first (`cargo build --workspace`) and then opens the file, so the
window you get is the code in your working tree. A Rust toolchain is the only
requirement: Verible and Icarus are needed by the scripts below, not by the
editor. If `verible-verilog-ls` or `slang-server` happens to be on `PATH` the
editor attaches to it and the mode line grows an `LSP` indicator; without one,
everything else still works.

The two keys this design is built to show off are `C-c C-a` (`verilog-auto` —
expand every `/*AUTO...*/` marker) and `C-c C-k` (`verilog-delete-auto` —
remove what it generated). Pressing `C-c C-k` then `C-c C-a` returns the file
byte-for-byte; that round trip is the point. `C-x C-c` quits.

## Checking the claims

```sh
# SystemVerilog + Verilog: parse, lint, format-check
./tools/lint_rtl.sh

# SystemVerilog + Verilog: actually simulate (Icarus Verilog)
./tools/run_sim.sh

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

- `./run_editor.sh` — `--help`, both error paths (unknown option, missing
  file) and `--dry-run` (which does perform the build) were run and behave as
  documented. The interactive launch itself cannot be exercised from a script,
  so it was verified the way this project verifies every GUI claim — by
  screenshot: `dev/gui-shot.sh demo/rtl/core/status_regs_stub.sv` opens the
  same binary on the same file and shows the file, the expanded
  `/*AUTOUNUSED*/` block and `LSP: autostarted slang-server`. **What has not
  been exercised: pressing the keys.** The `C-c C-a` / `C-c C-k` round trip is
  covered by the test suite, not by this script.
- `./tools/lint_rtl.sh` — **all three checks pass** across all 20 Verilog
  and SystemVerilog files (`verible-verilog-syntax`, `-lint`, `-format
  --verify`).
- `./tools/run_sim.sh` — **all four simulations actually run and pass**:
  `rtl-verilog2001/fifo_sync` prints `PASS: fifo_sync 8 x 32`,
  `rtl-verilog2001/gray_ctr` prints `PASS: gray_ctr WIDTH=4` (M124: a
  non-ANSI-header Gray-code counter, checked against both the expected
  binary sequence and the one-bit-per-step Gray property across two full
  wraps), `rtl-verilog2001/fifo_gray_top` prints `PASS: fifo_gray_top 8 x
  32, GRAY_WIDTH=4` (M125: a non-ANSI top wrapping both `fifo_sync` and
  `gray_ctr` — real material for `/*AUTOOUTPUT*/`, `/*AUTOINPUT*/` and
  `/*AUTOINOUT*/`, checked in fully expanded and pinned byte-for-byte
  against the editor's own output), and `verif/sram_bank_tb`
  prints `PASS: sram_bank 4 banks x 3 words (hits=12 misses=0)` — 4
  banks, 3 words each, written and read back through the real
  `axi4_lite_if` interface and `sram_bank`'s generate-instantiated
  `sram_wrapper`/`clk_gate` hierarchy.
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

- **All 38 files open in the correct major mode** — 32 in a
  language-specific mode (`verilog-mode`, `rust-mode`, `c-mode`,
  `c++-mode`, `python-mode`, `perl-mode`, `sh-mode`, `java-mode`,
  `emacs-lisp-mode`, `org-mode`) and 6 in `fundamental-mode`
  (`README.md`, `rtl/verible.filelist`, `verif/verible.filelist`,
  `rtl/.slang/server.json`, `rtl-verilog2001/.rules.verible_lint`,
  `tools/sample_sim.log`). (M124: `gray_ctr.v`/`gray_ctr_tb.v` added to
  `rtl-verilog2001/`, both dump-verified to open in `verilog-mode`. M125:
  `fifo_gray_top.v`/`fifo_gray_top_tb.v` added the same way. M127:
  `rtl/mem/sram_dual_channel.sv` added the same way.)
- From `rtl/top/soc_top.sv`, **`M-.` on `alu` lands in
  `rtl/core/alu.sv`** and port completion engages inside `u_regfile`'s
  port list — the two table rows above are measured, not asserted.
- That file sees **9 library files** even though `rtl/top/` contains
  only `soc_top.sv` itself: all nine arrive via `rtl/verible.filelist`.
- Open any of the 20 files under `rtl/`, `rtl-verilog2001/` and
  `verif/` that have enough indented lines to go on, and the editor's
  indent step
  **follows that file's own 2-space style** instead of `verilog-mode`'s
  4-space default: a new statement line opened in a module body or
  inside a `begin`/`end` block lands where `verible-verilog-format`
  already wants it, so the indentation no longer diffs on save.
  Measured on `rtl/core/alu.sv`, which is what the test asserts.

  One honest exception, checked rather than assumed.
  `rtl/include/soc_defs.svh` has only 2 indented lines out of 28 —
  below the detector's 5-sample confidence floor, so it keeps the
  4-space mode default; the detector declines to guess rather than
  guessing from two samples.

  A line opened *inside* an instantiation's port-connection list, a
  `#(...)` parameter list, or a wrapped call's argument list — a
  **hanging** list, where the opening paren is immediately followed by
  a newline, which is what every instantiation and port list in this
  directory looks like — lands at the enclosing block's depth **plus a
  fixed 4-column wrap step** (column 6 in `rtl/top/soc_top.sv`, matching
  where the file's own `.clk_i (clk_i),` lines are already aligned),
  not at the opening paren's own column. That fixed step is deliberate,
  not a gap: it mirrors `verible-verilog-format`'s own two independent
  flags, `--wrap_spaces` (default 4) versus `--indentation_spaces`
  (default 2, what this file's own detected 2-column body width
  matches) — measured by running the formatter at `--indentation_spaces`
  2/3/4/8, where the continuation delta stayed 4 every time for hanging
  lists. `indent-wrap-width` (default 4, see `indent.el`) implements
  this as a second axis alongside the block-depth multiply, decoupled
  from the buffer's own detected step.

  This is narrower than "verible never aligns to the paren column,"
  which is false in general: when a wrapped call's own argument is
  itself a call whose paren is followed by more text on the same line
  (not a shape this directory happens to contain), verible switches to
  paren-column alignment instead of the fixed step — a decision that
  needs line-length lookahead this engine doesn't have, so this editor
  keeps computing the fixed-step answer there. See `indent.el`'s M90
  section for the exact shape and the two diverging columns.

The four editor claims above, plus the `C-c C-a` / `C-c C-k` rows of
the keybinding table, are the ones a test now re-checks on every run:
`crates/core/tests/demo_smoke_tests.rs`. The "9 library files" count is
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

Not verified, for lack of a toolchain on this machine:

- `tools/TimingReport.java` — **no JDK installed**: `/usr/bin/java` on
  this machine is the macOS stub (`java -version` reports "Unable to
  locate a Java Runtime"), and there is no `javac` that actually
  compiles anything. It is the one file on this page whose claims rest
  on reading rather than running.

`rtl-verilog2001/fifo_sync_tb.v` is **no longer** in this list — it now
actually runs (`./tools/run_sim.sh`, above): `PASS: fifo_sync 8 x 32` is
an observed result, not intent. Same for `rtl-verilog2001/gray_ctr_tb.v`
(M124), added together with `gray_ctr.v` and wired into the same script
from the start — `PASS: gray_ctr WIDTH=4` is likewise observed. Same
again for `rtl-verilog2001/fifo_gray_top_tb.v` (M125), added together
with `fifo_gray_top.v` and wired into the same script from the start —
`PASS: fifo_gray_top 8 x 32, GRAY_WIDTH=4` is likewise observed.

M126 changed `rtl-verilog2001/gray_ctr.v` itself: `bin_count`'s port
declaration was changed from `output reg [WIDTH-1:0]` to a bare, untyped
`output`, and a `/*AUTOREG*/` marker (checked in fully expanded) now
supplies the `reg` declaration that used to be hand-written. The same
`PASS: gray_ctr WIDTH=4` and `PASS: fifo_gray_top 8 x 32, GRAY_WIDTH=4`
lines above were re-observed after that change (`./tools/run_sim.sh`),
and `./tools/lint_rtl.sh` re-run clean across all 20 files. `rtl/core/
status_regs_stub.sv` is new material for `/*AUTOTIEOFF*/` on a real ANSI
SystemVerilog module (a bring-up stub whose outputs are declared but not
yet driven, ordinary early-stage RTL practice) — lint/format-clean under
the same zero-waiver rule set as every other file under `rtl/`, but
**not** run through `./tools/run_sim.sh`: it is not instantiated
anywhere, so it has no testbench to run. It is deliberately not wired
into `rtl/top/soc_top.sv`, because several tests pin that file's exact
line numbers and instance count and this module's own job is AUTOTIEOFF
rather than integration.

Both of those checked-in expanded artifacts are pinned byte-for-byte
against what this editor itself generates, so neither can drift from the
command that is supposed to produce it: `demo_verilog2001_gray_ctr_
autoreg_matches_editor_output` and `demo_rtl_status_regs_stub_autotieoff_
matches_editor_output`, both in `crates/core/tests/demo_smoke_tests.rs`.
Each opens the real file, runs delete-auto → verilog-auto → format-buffer,
and asserts byte-equality with what is on disk.

M127 added `rtl/mem/sram_dual_channel.sv`: a third `/*AUTOINST*/` demo,
checked in fully expanded, exercising AUTO_TEMPLATE's `@` instance-number
substitution and `[]` bit-range tokens (neither had any exercise anywhere
under `demo/` before this milestone) — one AUTO_TEMPLATE body applied to
two `sram_wrapper` instances (`u_ch0`/`u_ch1`), an exact rule for the
shared `clk_i`/`rst_ni` winning over a wildcard rule that would otherwise
mis-rename them, and every templated connection carrying its own
`// Templated` annotation. Same pinning discipline as the other two:
`demo_rtl_sram_dual_channel_autoinst_matches_editor_output` in
`crates/core/tests/demo_smoke_tests.rs` runs delete-auto → verilog-auto →
format-buffer and asserts byte-equality with what is on disk. It is
deliberately not instantiated anywhere in `rtl/top/soc_top.sv` for the
same reason `status_regs_stub.sv` isn't — that file's exact line numbers
and instance count are pinned by other tests, and this module's own job
is demonstrating AUTO_TEMPLATE, not integration.

### Checked by Verible, never simulated

Icarus Verilog 13.0 (the simulator `./tools/run_sim.sh` uses) does not
support every construct `verible-verilog-lint`/`-format` accept, so
three pieces of real material in this tree are checked by Verible with
the full default rule set and zero waivers, but **never actually
simulated** — named explicitly here per this project's own rule that
anything not yet run must be called out by name:

- **Concurrent assertions** (`property`/`endproperty`, `assert
  property`) in `rtl/bus/axi4_lite_if.sv` and `rtl/mem/sram_bank.sv`.
  Compiling either file without `-DSOC_SVA_OFF` fails:
  `concurrent_assertion_item not supported. Try -gno-assertions or
  -gsupported-assertions to turn this message off.`, followed by a
  cascade of `syntax error` / `Invalid module item.` for the rest of
  the property body — Icarus's own error, not a defect in the SVA
  itself (it is written directly in the source, never hidden inside a
  macro, specifically so a real parser sees it). `./tools/run_sim.sh`
  always passes `-DSOC_SVA_OFF`, so the `` `ifndef `` guard around each
  property strips it out of every simulation run.
- **The `covergroup`** in `verif/sram_bank_tb.sv`. Compiling without
  `-DSOC_COVERAGE_OFF` fails the same way, on the `covergroup
  cg_axi_bank @(posedge clk_i);` line: `syntax error` / `Invalid module
  item.`. `./tools/run_sim.sh` always passes `-DSOC_COVERAGE_OFF`.
- **`verif/axi4_lite_monitor.sv`** in its entirety — this is an Icarus
  limitation, not a language or design one; the material is IEEE
  1800-2017 legal and Verible accepts it. Its own module port is
  interface-typed (`axi4_lite_if.monitor bus`), and Icarus 13.0 cannot
  parse a module whose own port is interface-typed at all, independent
  of the assertions inside it: `axi4_lite_monitor.sv:20: syntax error`
  / `axi4_lite_monitor.sv:20: Errors in port declarations.` (measured
  directly, isolating this file from the concurrent-assertion issue
  above by compiling it with `-DSOC_SVA_OFF` set and getting the exact
  same port-declaration error regardless). `./tools/run_sim.sh` never
  compiles this file at all — `sram_bank_tb.sv` instead wires an
  `axi4_lite_if` instance directly to `sram_bank`'s flat ports, so the
  interface itself is still exercised end to end in real simulation
  even though this passive monitor cannot be instantiated alongside it.

## Editing this directory

The files here are not wired into `cargo test`; changing one cannot
break the build. If you add a language, add a file that does something
real in it — the point of this directory is entirely lost on a sample
that would embarrass you in a code review.
