# Contributing to Reticle

Thank you for your interest. Please read this page before opening a pull
request -- the licensing terms here are not the ones you may be used to from
an MIT or Apache-2.0 project.

## Licensing status: source-available, not open source

Reticle is published under the **Functional Source License 1.1 with an
Apache 2.0 future grant** (`FSL-1.1-ALv2`). See `LICENSE.md` for the full text.

In short:

- You may read, build, run, modify and redistribute the source for any
  **Permitted Purpose**, which includes internal use at your company,
  non-commercial education and research, and professional services you provide
  to another licensee.
- You may **not** make a competing product or service out of it -- anything
  that substitutes for Reticle or offers substantially the same
  functionality.
- Two years after each version is published, that version becomes available
  under the Apache License 2.0 automatically. The future grant is irrevocable.

This is deliberately **not** an OSI-approved open-source license. Please do not
file issues asking for it to be relicensed as MIT, Apache-2.0 or GPL; the
answer is no while the project is commercially supported.

## Contributor license grant (required)

Because the project is commercially licensed, contributions cannot simply be
"inbound = outbound". To keep the ability to license Reticle to paying
customers -- and to honour the Apache-2.0 future grant on your code too -- every
contribution must come with a license grant broad enough to permit that.

**By opening a pull request against this repository, you represent and agree
that:**

1. You are the author of the contribution, or you have the right to submit it
   under these terms, and it is not subject to any third-party license
   (including any copyleft license) that would restrict its use here.
2. You grant the licensor (see the copyright notice in `LICENSE.md`) a
   perpetual, worldwide, non-exclusive, irrevocable, royalty-free, fully
   sublicensable and transferable license to use, reproduce, modify, prepare
   derivative works of, publicly display, publicly perform, distribute and
   relicense your contribution, **under any license terms, including
   proprietary and commercial terms**.
3. You grant every recipient of the software a perpetual, worldwide,
   non-exclusive, irrevocable, royalty-free patent license to make, use, sell
   and otherwise exploit your contribution, to the extent your patent claims
   are necessarily infringed by it.
4. You retain your own copyright in your contribution. This is a license
   grant, not an assignment -- you keep the right to use your own code
   elsewhere.
5. Your contribution is provided "as is", without warranty of any kind.

Add a `Signed-off-by:` line to each commit to record your agreement:

    git commit -s -m "your message"

which certifies the [Developer Certificate of Origin](https://developercertificate.org/)
in addition to the grant above.

If you cannot agree to these terms, please open an issue describing the change
instead of a pull request. A clear bug report is genuinely useful and carries
no licensing complications.

## Before you write code

Open an issue first for anything larger than a bug fix. The project has a
strong opinion about what it is for -- **Verilog and SystemVerilog RTL
development** -- and features are prioritised by how much they help a working
RTL engineer. A well-built feature that does not serve that user is still
likely to be declined, so it is worth agreeing on the goal before spending your
time.

## Development setup

    git clone <repository-url>
    cd reticle
    cargo build --release

A recent stable Rust toolchain is required. The GUI frontend builds on macOS,
Linux and Windows; the TUI frontend needs a terminal that reports its size.

## Completion gates

A change is not finished until all three of these are clean. They are not
optional, and they are checked on every pull request:

    cargo build --workspace
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

Note the non-default clippy flags: `--all-targets` and `-D warnings`. Warnings
are errors here.

The `cargo build --workspace` step is not redundant. `crates/demo-module` is a
`cdylib`, and the module-loading tests in `crates/elisp/tests/module_tests.rs`
load the real build artifact from `target/` rather than linking against it. A
cdylib cannot be a Rust dependency, so nothing makes `cargo test` build it: on
a fresh clone those five tests fail with "demo-module dylib not found" until
the workspace has been built once.

If you touch any `.el` file, also run:

    cargo test -p core --test lisp_hygiene_tests

This catches unescaped double quotes inside docstrings, which silently truncate
the docstring and turn the remaining prose into executable body forms. The
reader accepts it, `defun` succeeds, and it only fails later at call time with
an error naming a symbol that does not appear anywhere in the source. It is a
genuinely awful failure mode and the test finds it in seconds.

If you change any Verilog under `demo/`, run:

    demo/tools/lint_rtl.sh

which checks syntax, lint and formatting with `verible`.

## Testing conventions

- **Integration tests are the default.** Name them `<feature>_tests.rs` in the
  relevant crate's `tests/` directory. Performance tests end in
  `_perf_tests.rs`.
- **Add to the existing file for that feature** rather than creating a new one.
  Only create a new file when the feature genuinely has none yet.
- Unit tests in a `#[cfg(test)]` block are for pure data-structure-level code
  only.
- There are no shared fixtures, snapshots or custom test macros. Each test file
  carries its own helpers. Please keep it that way -- it makes any single test
  file readable on its own.

### Race conditions

Running a whole test binary and running a filtered subset of it can give
opposite answers, because the filtered run changes the thread interleaving. If
you are testing anything concurrent, run the specific tests by name, repeatedly:

    for i in $(seq 30); do cargo test -p core --test <file> -- <test1> <test2> || break; done

Do not explain an intermittent failure as "environment noise" unless you can
produce an observation that distinguishes noise from a real race. In this
project that explanation has been wrong every time it has been offered.

## Code style

- No `unwrap()` in library code. Use `Result` with proper error types. Tests
  may unwrap freely.
- Match the surrounding code's comment density and naming. The codebase has a
  consistent voice; a patch that reads differently is harder to review.
- Comments should explain *why*, especially when the reason is a bug that was
  hit once. Several comments in this codebase name the exact failure they
  prevent, and that is the house style.

## A note on the architecture

Modes and commands have **no Rust type**. They are elisp `Value`s dispatched
through keymaps. To find the implementation of an interactive command, search
for its registered name under `crates/core/src/builtins/` -- do not go looking
for a struct.

This is the one thing about the layout that cannot be inferred from the code,
and it costs every new reader about an hour. The rest of the structure follows
the crate boundaries in each `Cargo.toml`.
