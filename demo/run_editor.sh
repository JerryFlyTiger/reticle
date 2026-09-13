#!/usr/bin/env bash
#
# run_editor.sh -- build reticle and open this demo design in it.
#
# This is the "how do I actually run this thing" script. Everything else
# under demo/ shows that the sample design is real (lint_rtl.sh) or that
# the sample tools run (run_sim.sh); this one opens the editor itself.
#
#     ./run_editor.sh               # GUI window on the SoC top level
#     ./run_editor.sh --tui         # run in this terminal instead
#     ./run_editor.sh --unused      # open the module AUTOUNUSED fills in
#     ./run_editor.sh path/to.sv    # open some other file
#     ./run_editor.sh -q            # ignore your ~/.reticle/init.el
#     ./run_editor.sh --dry-run     # build and print the command, run nothing
#
# It always builds first. That is not politeness: the GUI has no
# pre-built binary in the repo, and building guarantees the window you
# get is the code in your working tree rather than whatever was last
# compiled. That is not a hypothetical: a fix was once built in release
# mode while the screenshot tool picked up a 73-minute-old debug binary,
# and the resulting "the fix does nothing" reading cost five wrong
# hypotheses before anyone compared the timestamps.
#
# Requires: a Rust toolchain. Nothing else -- the Verible/Icarus tools
# that lint_rtl.sh and run_sim.sh need are NOT needed to open the editor.
# The LSP features light up only if verible-verilog-ls or slang-server is
# on PATH; without them the editor still opens and edits normally.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)

mode=gui
file="$repo_root/demo/rtl/top/soc_top.sv"
extra_args=()
dry_run=0

while [ $# -gt 0 ]; do
    case "$1" in
        --tui | -nw)
            mode=tui
            ;;
        --gui)
            mode=gui
            ;;
        --unused)
            # The module whose two deliberately-unused stub inputs are what
            # /*AUTOUNUSED*/ lists (M136). slang-server warns about exactly
            # those two ports when the idiom is absent.
            file="$repo_root/demo/rtl/core/status_regs_stub.sv"
            ;;
        --dry-run)
            dry_run=1
            ;;
        -q)
            extra_args+=("-q")
            ;;
        -h | --help)
            sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^#//; s/^ //'
            exit 0
            ;;
        -*)
            echo "run_editor.sh: unknown option '$1' (try --help)" >&2
            exit 2
            ;;
        *)
            file="$1"
            ;;
    esac
    shift
done

if [ ! -f "$file" ]; then
    echo "run_editor.sh: no such file: $file" >&2
    exit 2
fi

echo "==> building (cargo build --workspace)"
if ! (cd "$repo_root" && cargo build --workspace); then
    echo "run_editor.sh: build failed -- nothing was launched." >&2
    exit 1
fi

bin="$repo_root/target/debug/reticle"
if [ ! -x "$bin" ]; then
    echo "run_editor.sh: built, but no executable at $bin" >&2
    exit 1
fi

cmd=("$bin" "$file")
if [ "$mode" = tui ]; then
    cmd+=("--tui")
fi
if [ ${#extra_args[@]} -gt 0 ]; then
    cmd+=("${extra_args[@]}")
fi

cat <<'KEYS'

==> things to try once it opens

  C-x C-c      quit (C- means Control)
  M-x          run a command by name
  C-g          cancel whatever you started

  C-c C-a      verilog-auto -- expand every /*AUTO...*/ marker in the
               buffer. In soc_top.sv those are the AUTOINST port lists
               of the four instances; in status_regs_stub.sv it is the
               /*AUTOUNUSED*/ list of never-read inputs.
  C-c C-k      verilog-delete-auto -- remove what C-c C-a generated.
               Press C-c C-k then C-c C-a and the file comes back
               byte-for-byte: that round trip is the point.

  C-s          incremental search        M-.   jump to definition (LSP)
  M-?          find references (LSP)     C-=   expand region

  The mode line shows `LSP' once a language server has attached, with
  an `!N' in front of it when that server has reported N diagnostics
  (nothing to report, nothing in front). No server on PATH means no LSP
  features; the editor itself opens and edits exactly the same.

KEYS

printf '==> %s\n' "${cmd[*]}"

if [ "$dry_run" = 1 ]; then
    echo "(--dry-run: stopping here, nothing launched)"
    exit 0
fi

exec "${cmd[@]}"
