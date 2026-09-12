#!/bin/bash
# Run real GNU Emacs 30.2's `verilog-auto' over a fixture and print the result.
#
# This project's standing rule is that GNU-parity behaviour is settled by
# running the reference implementation, never written from memory (see
# CLAUDE.md, and PLAN.md's M110 record, where a spec written from memory
# produced a data-corruption bug that ate a newline belonging to a different
# line). This script and the fixtures beside it are the AUTO family's half of
# that rule, kept so the measurements can be reproduced rather than retyped.
#
#     dev/gnu-auto/run.sh fixtures/autoreset-basic-two-flops.v
#     dev/gnu-auto/run.sh fixtures/autounused-autoinst-connected-excluded.v \
#         '(setq verilog-auto-reset-widths (quote unbased))'
#
# The second argument is spliced in as extra `setq' forms before
# `verilog-auto' runs, which is how the knobs get exercised
# (`verilog-auto-reset-widths', `verilog-auto-reset-blocking-in-non',
# `verilog-auto-unused-ignore-regexp', ...).
#
# Two things this script does deliberately, both of which cost a wrong
# measurement before they were fixed:
#
# 1. **It copies the fixture next to itself and opens the copy**, so that
#    `verilog-library-directories' -- which defaults to `(".")' -- resolves
#    sibling modules like `sub.v'. Opening a fixture from some other
#    directory makes `verilog-auto' find no submodule, produce nothing, and
#    say nothing about why. The first AUTOUNUSED measurement in M134 hit
#    exactly this and read as "the command does nothing".
#
# 2. **It does not swallow stderr.** GNU reports a missing module as an elisp
#    error, and an earlier version of this runner filtered that away, leaving
#    an empty output that looked like a deliberate empty result. Only
#    `Loading`/`Wrote` chatter is filtered.
#
# Requires /opt/homebrew/bin/emacs (GNU Emacs 30.2, installed on this
# machine for exactly this purpose).
set -u
f="${1:?usage: run.sh FIXTURE [extra-setq-forms]}"
extra="${2:-}"
here="$(cd "$(dirname "$0")" && pwd)"
# Absolute, because `find-file' sets `default-directory' to the file's own
# directory -- a relative output path written afterwards resolves against THAT
# and silently doubles the prefix (cost one debugging round when this script
# was first collected out of the scratchpad).
tmp="$(cd "$(dirname "$f")" && pwd)/.gnu-$$-$(basename "$f")"
cp "$f" "$tmp"
trap 'rm -f "$tmp" "$tmp.out"' EXIT
/opt/homebrew/bin/emacs -Q --batch \
  --eval "(progn $extra
            (find-file \"$tmp\") (verilog-mode) (verilog-auto)
            (write-region (point-min) (point-max) \"$tmp.out\"))" 2>&1 \
  | grep -v '^Loading\|^Wrote'
if [ -f "$tmp.out" ]; then
  cat "$tmp.out"
else
  echo "!! no output produced -- read the stderr above, it is not filtered." >&2
  echo "!! The usual cause is a submodule GNU could not find from this" >&2
  echo '!! directory; verilog-auto reports that as an elisp error.' >&2
  exit 1
fi
