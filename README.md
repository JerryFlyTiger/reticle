# Reticle

An Emacs-like editor written from scratch in Rust, built for **Verilog and
SystemVerilog RTL development**.

It is not a wrapper around GNU Emacs and it does not run GNU Emacs packages. The
Emacs Lisp interpreter, the editor core, the syntax highlighting, the LSP client
and the vim-key layer are all original implementations. What it borrows from
Emacs is the *model*: a small core plus a Lisp you can reconfigure at runtime,
with a keymap you already know.

> **Status: pre-1.0.** Everything documented below is implemented and tested,
> but the project has not had a tagged release yet, and the "Known limitations"
> section is deliberately long and honest. Read it before deciding whether this
> fits your workflow.

---

## Why another editor

RTL work has a specific shape that general-purpose editors handle poorly:

- Module names are long and numerous, and the file that declares a module is
  rarely the file you are looking at.
- A 40-port instantiation is normal, and maintaining it by hand is where the
  bugs come from.
- The design is split across many small files tied together by a filelist, not
  by an import graph a language server can walk on its own.
- Parameterised modules are the default, not an advanced case.

Reticle is organised around those facts. Cross-file module jump, filelist-
driven library search, AUTOINST/AUTOWIRE/AUTOARG expansion, per-file indent
width detection and a Verilog-aware search buffer are core features here, not
plugins someone else has to maintain.

---

## Features

### Emacs Lisp engine

A complete Elisp implementation with three execution tiers:

| Tier | What it is | Tight integer loop | Recursive `fib` |
| --- | --- | --- | --- |
| Tree-walking evaluator | Default. Full language semantics. | baseline | baseline |
| Bytecode compiler + VM | Opt in with `(byte-compile 'sym)`. | ~8x | 1.0x |
| Native JIT (Cranelift) | Opt in with `(native-compile 'sym)`. | ~575x | 1.0x |

Measure it yourself -- `dev/bench-tiers.el` is what produced those numbers
(eight runs on one machine spanned 8.01x-8.20x and 573x-577x, hence the
approximations):

```sh
cargo build --release --workspace
./target/release/reticle --script dev/bench-tiers.el
```

The second column is the point. `fib` is the same integer arithmetic, but its
self-call puts it outside the JIT's eligible subset, so `native-compile` leaves
it byte-compiled -- and byte-compiling buys nothing either, because the call
overhead dominates. Use a release build: on a debug build the tree-walking
evaluator is itself an order of magnitude slower, which inflates every ratio.

The JIT covers a deliberately narrow subset -- pure-integer functions touching
only locals and a whitelisted set of arithmetic operations. That restriction is
the point: it makes the "bail out and rerun as bytecode on overflow" fallback
provably safe, rather than approximately safe.

Also included: a hand-written reader, macros, `condition-case` / `catch` /
`unwind-protect`, bignums, hash tables, a self-implemented regex engine with an
explicit backtracking stack and a work budget, and JSON parsing that matches
current GNU defaults.

### Editor core

Gap buffer with UTF-8 aware operations, windows, keymaps (global, buffer-local
and an emulation layer), undo, overlays, and a single screen-grid redisplay
model shared by both front ends.

On top of that: minibuffer with completion, incremental search, Dired,
`compile` / `recompile` / `next-error`, one-shot shell commands
(`M-!` / `M-&` / `M-|`), Eshell, IELM, a help system
(`describe-key` / `describe-bindings` / `describe-function`), an org-mode
implementation that is file-format compatible with real org files, and
TRAMP-style remote editing over SSH (`/ssh:user@host:/path`).

### Typing never blocks

This is architectural rather than a feature you can point at, and it is the
reason the rest works. An interrupt mechanism preempts both the tree-walker and
the bytecode VM every 64 steps. Hooks on the keystroke path run under a 50ms
budget, and a hook that blows that budget three times is removed automatically
with a named message telling you which one it was. Tree-sitter parsing runs on a
background thread with a 30ms debounce. LSP is fully asynchronous.

The practical consequence: a slow language server, a large file or a badly
written hook in your own init file degrades a feature, not the editor.

### Vim keys

An evil-mode work-alike, written for this editor, **on by default**. Five
states, motions, operators, text objects, `:` ex commands, `/` and `?` bridged
to incremental search, dot-repeat, marks, named registers, `:s` substitution and
`q` macros.

Turn it off with `(setq evil-auto-enable nil)` in your init file.

### Syntax highlighting

tree-sitter grammars for **SystemVerilog/Verilog, Rust, C, C++, Python, Bash,
Java, Perl and Emacs Lisp** -- nine languages.

The highlighting philosophy follows GNU font-lock rather than the upstream
tree-sitter defaults: declaration and definition sites get coloured, and bare
identifiers, operators and ALL-CAPS guesses do not. The result is closer to what
an Emacs user expects and considerably less noisy on real RTL.

### LSP client

Transport and JSON-RPC framing in Rust; all protocol semantics in Elisp, so you
can read and change them.

Implemented: `initialize` handshake, `didOpen` / `didChange` / `didSave` /
`didClose`, `hover`, `definition`, `references`, `formatting`,
`rangeFormatting`, `documentSymbol`, `codeAction`, `rename`, `completion`,
`documentHighlight`, and incoming `publishDiagnostics`.

Eight major modes are pre-registered against seven server binaries
(rust-analyzer, clangd for both C and C++, pyright-langserver,
bash-language-server, jdtls, pls, verible-verilog-ls). Servers verified end-to-end against real binaries:
**verible-verilog-ls** (the primary target), **rust-analyzer**, and
**slang-server**.

Every LSP milestone in this project was validated by probing a real server
before any code was written, using `dev/lsp-probe.py`. That rule exists because
it caught three separate cases where the capability declarations and the actual
behaviour disagreed.

### Verilog tooling

- **Cross-file module jump** -- `M-.` on an instantiation goes to the module
  declaration, `M-,` comes back, across files.
- **Library search** -- recursive BFS plus `verible.filelist` support, so the
  jump works on a real multi-directory design.
- **AUTO expansion** -- `AUTOINST`, `AUTOWIRE`, `AUTOARG`, expanded with
  `C-c C-a` and removed again with `C-c C-k`. Set `verilog-auto-on-save` to
  `t` to run the expansion before every save instead; it is off by default.
- **Port name completion** -- local, from the module being instantiated.
- **Per-file indent width detection** -- someone else's RTL keeps its own
  indentation instead of being silently reformatted.
- **Search buffer** -- background project search streaming into a `*search*`
  buffer, with wgrep-style **editable results**, unordered ("orderless")
  filtering, and live filter-as-you-type.

### Two front ends, one core

- **GUI** (`egui`): character-cell rendering, hover popups, squiggly
  diagnostic underlines, indent guides, an overlay scrollbar, a fading block
  cursor, and hairline separators above each window's mode line. Filled
  rectangles are snapped to whole device pixels, so cell backgrounds meet
  exactly instead of leaving seams. Padding, line spacing, font family, font
  size, indent-guide width and cursor-blink count are elisp variables
  (`gui-padding-x`, `gui-padding-y`, `gui-line-spacing`, `gui-font-family`,
  `gui-font-size`, `gui-indent-guide-step`, `gui-cursor-blinks`). As in GNU
  Emacs, the cursor stops blinking after `gui-cursor-blinks` blinks with no
  input (default 10, `0` to blink forever) and the window drops back to a slow
  repaint until you type again. Real bold and italic font faces are loaded from
  known macOS system font paths; on other platforms bold degrades to a
  double-draw and italic is unavailable.
- **TUI** (`crossterm`): the same grid model in a terminal.

Both render the identical redisplay output -- the GUI's extras above are pixels
drawn around and behind that grid, never changes to a cell. There is no "the
terminal version is the lesser one" split.

Two built-in themes, dark and light, switched with `(load-theme 'light)`. Every
colour lives in `crates/core/lisp/themes.el`, including the diagnostic severity
colours and the cursor; both themes are checked against each other by a test, so
a face cannot be added to one and forgotten in the other.

### C ABI dynamic modules

A `#[repr(C)]` module ABI with a working example `cdylib`. `unsafe` in this
project is concentrated rather than absent: `crates/elisp/src/module.rs` and
`crates/demo-module` hold most of it, since an FFI boundary cannot be written
without it. The rest is a small number of isolated uses -- the JIT's call into
generated code, one UTF-8 conversion in the gap buffer, and a few platform
calls.

---

## Installation

### Requirements

- A recent stable Rust toolchain (install via [rustup](https://rustup.rs/)).
- A C compiler at **build time only** -- the tree-sitter grammar crates compile
  C sources. On macOS the Xcode Command Line Tools are enough; on Linux, `gcc`
  or `clang`.

There is no runtime toolchain dependency. The JIT is Cranelift, which is pure
Rust -- no LLVM, no gcc at runtime.

Optional, for the Verilog features:

- [`verible`](https://github.com/chipsalliance/verible) for
  `verible-verilog-ls` (language server), plus its lint and format tools.

### Build

```sh
git clone <repository-url>
cd reticle
cargo build --release
```

The binary lands at `target/release/reticle`. Copy it somewhere on your
`PATH`:

```sh
install -m 755 target/release/reticle ~/.local/bin/
```

A man page is provided at `doc/reticle.1`:

```sh
install -m 644 doc/reticle.1 ~/.local/share/man/man1/
man reticle
```

---

## Running

```sh
reticle                    # GUI (default)
reticle top.sv             # GUI, opening a file
reticle -nw top.sv         # terminal UI
reticle -q                 # skip ~/.reticle/init.el
```

Batch and scripting modes:

```sh
reticle --eval '(+ 1 2)'   # evaluate an expression and print the result
reticle --script build.el  # load and evaluate a file, then exit
reticle --repl             # interactive Elisp REPL on stdin/stdout
```

| Flag | Effect |
| --- | --- |
| *(none)* | GUI |
| `-nw`, `--tui` | Terminal UI. Wins over `--gui` regardless of argument order. |
| `--gui` | Explicit GUI. Same as the default. |
| `-q` | Do not load the user init file. |
| `--eval EXPR` | Evaluate `EXPR`, print the result, exit. |
| `--script FILE` | Load `FILE`, evaluate it, exit. |
| `--repl` | Interactive Elisp REPL. |
| `-h`, `--help` | Usage. Takes priority over every other argument. |
| `-v`, `--version` | Version. Same priority. |

**The three batch flags only take effect as the very first argument.**
`reticle --repl`, `reticle --eval EXPR` and `reticle --script FILE`
work; `reticle foo.sv --repl` does not, and is rejected as an unknown
option. `-nw` / `--tui` / `--gui` / `-q`, by contrast, are recognised anywhere
on the line.

`-h` / `--help` and `-v` / `--version` win over everything else, so they cannot
be shadowed by an accidental REPL or GUI session. The single exception is the
value slot immediately after `--eval` or `--script`: `--eval --help` evaluates
`--help` as Elisp and fails, which is the correct behaviour.

`--worker` also exists; it is an internal child-process mode and is not meant to
be invoked by hand.

---

## Configuration

Your init file is **`~/.reticle/init.el`**. Its directory is added to
`load-path` automatically, so you can split your configuration across files and
`require` them.

```elisp
;; ~/.reticle/init.el

;; Turn off the vim layer if you want plain Emacs keys.
;; (setq evil-auto-enable nil)

(setq indent-width 2)

(defun my-verilog-setup ()
  (setq indent-width 2))
(add-hook 'verilog-mode-hook 'my-verilog-setup)

(global-set-key (kbd "C-c c") 'compile)
```

An error while loading your init file lands in the echo area. It does not abort
startup, so a typo cannot lock you out of the editor.

See `examples/init.el` for a general starting point and
`demo/editor/init-example.el` for one oriented at RTL work.

One gotcha worth knowing early: there is **no `major-mode` variable**. Use
`(major-mode-internal-get)`.

---

## The `demo/` directory

`demo/` contains real code in ten languages, not toy snippets. It exists to
answer one question honestly: is this an editor you could actually write RTL in?

- `demo/rtl/` -- a small SystemVerilog SoC, deliberately split across
  `pkg/ core/ mem/ bus/ top/` with a `verible.filelist`, so the cross-file
  features have somewhere real to operate.
- `demo/rtl-verilog2001/` -- plain Verilog-2001, with a narrowly scoped lint
  waiver file that documents each waived rule and why.
- `demo/tools/` -- EDA-adjacent utilities in seven languages.
- `demo/editor/` -- a working RTL-oriented init file.
- `demo/docs/` -- Org design notes for the SoC.

`demo/rtl/` passes verible's full default rule set with no waivers at all.

Everything claimed in `demo/README.md` has actually been run, and the few items
that have not are named explicitly on that page. The editor-behaviour claims --
major mode per file, cross-file jump, port completion, AUTO expansion, indent
width detection -- are re-checked by `crates/core/tests/demo_smoke_tests.rs` on
every test run; the claims that depend on external toolchains are one-time
manual records.

---

## Project layout

```
crates/elisp/          Elisp reader, evaluator, bytecode VM, JIT, regex, GC
crates/core/           Editor core: buffers, windows, keymaps, redisplay, LSP
crates/core/lisp/      The Elisp standard library shipped with the editor
crates/core/queries/   tree-sitter highlight queries, one per language
crates/frontend-tui/   Terminal front end (crossterm)
crates/frontend-gui/   GUI front end (egui / eframe)
crates/module-abi/     The C ABI contract for dynamic modules
crates/demo-module/    A working example dynamic module
src/main.rs            Thin entry point and argument dispatch
dev/                   Developer tooling (LSP probe, mutation harness, fakes)
```

**One thing you cannot infer from the code:** modes and commands have no Rust
type. They are Elisp values dispatched through keymaps. To find the
implementation of an interactive command, search for its registered name under
`crates/core/src/builtins/` rather than looking for a struct.

---

## Project status

86 milestones completed as of 2026-09-01, each with a written design record
kept with the project.

The test suite is 1,655 tests across 83 test binaries, weighted toward
integration tests rather than unit tests (26 further tests are marked
`#[ignore]` by design -- they depend on external language servers or measure
performance). There are no doc-tests: the code carries doc comments but no
runnable examples in them. Every change must pass three gates
before it is considered done: `cargo fmt` clean, `cargo clippy --workspace
--all-targets -- -D warnings` with zero warnings, and the full suite green.
(Run `cargo build --workspace` first -- the module-loading tests load a real
`cdylib` artifact that `cargo test` does not build on its own.)
Important fixes are additionally verified by mutation testing -- reverting the
fix must turn a named test red.

Where a defence cannot be observed by a test, that is recorded in the test
comments rather than papered over.

---

## Known limitations

This list is maintained deliberately. It is not a list of everything missing
relative to GNU Emacs -- that would be endless -- but of the gaps most likely to
matter to someone evaluating the editor.

**Elisp**

- Lexical binding is the default, which is the opposite of GNU Emacs's
  historical default. A `lexical-binding: nil` file cookie switches a file
  back to dynamic scope.
- `format` supports `%s` `%S` `%d` `%x` `%o` `%c` `%f` `%%`, but has no field
  width, no flags and no precision -- `%-6s`, `%5d` and `%.2f` are all errors,
  and `%e`/`%g` are missing.
- No `emacs-module.h` ABI compatibility, so real GNU Emacs dynamic modules will
  not load. Modules cannot be unloaded.
- Reference cycles held entirely through external objects (some keymap and
  overlay graphs) can leak.
- JIT-compiled code is not interruptible. This is accepted because the
  qualifying subset cannot loop forever.
- `push` / `pop` work on plain variable places only. Use `setf` for accessor
  places.

**Editor**

- No `define-derived-mode` and no syntax-table system.
- Minibuffer keys are a hardcoded Rust dispatch and are not rebindable from
  Elisp.
- `completing-read` does not accept alist, hash-table or dynamic collections.
- Closing the minibuffer with ESC has no Elisp-observable hook, so background
  work tied to minibuffer lifetime can be orphaned.
- No grammar loading at runtime; the nine languages are compiled in.

**LSP**

- `codeAction` does not execute `Command`-typed actions.
- `WorkspaceEdit` only applies edits belonging to the current file's URI.
- Position conversion is UTF-16 correct for `rename`, `references`, formatting
  edits, `documentSymbol`, `documentHighlight`, and diagnostics-at-point, but is
  still Unicode-scalar based for hover, go-to-definition, completion, and
  diagnostic decoration. In those four, a file containing astral-plane
  characters is off by one per such character.
- `workspace/symbol` is not implemented.

**Remote editing over SSH**

- Fully synchronous. Remote operations block the UI.
- Key-based authentication only -- there is no password or passphrase prompt.
- No multi-hop, no sudo, no other TRAMP methods, no remote completion, no
  ControlMaster connection reuse, no remote backup or locking.

**Search**

- `C-g` during filtering does not restore the prior state.
- Result navigation re-ranks the whole result set on each keypress, which is
  O(N) per `n` / `p`.

**GUI**

- No minimap, no smooth scrolling, no font ligatures, no tab bar.
- The cursor's fade is the only animation; it repaints at ~33ms while the
  window is focused, and drops back to 500ms when it is not.
- Rendering correctness depends partly on manual confirmation, because egui has
  no headless screenshot pipeline to assert against. `dev/gui-shot.sh` captures
  the window to a PNG so a change can at least be reviewed from an artifact; on
  macOS it needs Screen Recording permission.

---

## Roadmap

Candidates under consideration, in rough priority order. These are recorded
development candidates, not commitments:

1. **Development-loop speed.** The Elisp standard library is currently embedded
   into the binary at compile time, so a one-line `.el` change rebuilds a whole
   crate. Loading it from disk in development builds is the highest-value fix
   available.
2. **Semantic search filtering for RTL** -- classifying search hits by whether
   the signal is driven or read.
3. **Minimal git integration** -- `C-x g` status, hunk-level stage and unstage,
   commit. Driving the `git` CLI; there is no plan for a reimplemented git
   backend.
4. **`re-search-forward` `BOUND` argument.**

Requested, recorded, not yet scoped:

- **A todo list.** The kind of task pane most IDEs and editors carry.
- **Code formatting with a selectable style.**
- **Multiple cursors**, in the shape of Magnar Sveen's `multiple-cursors.el`.
- **`expand-region`**, in the shape of Magnar Sveen's package of that name.
- **A new app icon** -- a handwritten capital **R**, in GNU Emacs's colours.
- **Visual design of the GUI**, promoted to second only to Verilog and
  SystemVerilog in priority.
- **User-selectable font and theme**; a transparent window with a background
  image that can be faded and scaled, as iTerm2, VS Code and the JetBrains
  IDEs offer.
- **Dracula Official as the default theme**, with Xcode and VS Code themes
  offered alongside it.
- **JetBrains Mono as the default font**, with Fira Code and SF Mono offered
  alongside it.
- **GNU Emacs's window-splitting behaviour.**

---

## License

Reticle is **source-available, not open source**. It is published under the
[Functional Source License 1.1 with an Apache 2.0 future
grant](https://fsl.software/) (`FSL-1.1-ALv2`). The full text is in
`LICENSE.md`.

What that means in practice:

- **You may** read the source, build it, run it, modify it and redistribute it
  for any Permitted Purpose -- which explicitly includes internal use at your
  company, non-commercial education, and non-commercial research.
- **You may not** use it to build a competing product or service.
- **Two years after each version is published, that version becomes Apache-2.0
  automatically.** The future grant is irrevocable.

Third-party dependency attribution is in `THIRD_PARTY_LICENSES.md`. Every
dependency linked into the shipped binaries is under a permissive license; there
is no copyleft code in the build.

## Contributing

Bug reports are welcome and carry no licensing complications. Pull requests
require agreement to a contributor license grant, because the project is
commercially licensed -- see `CONTRIBUTING.md` before you start writing.
