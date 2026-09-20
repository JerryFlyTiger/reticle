#!/bin/sh
# M141: what GNU Emacs 30.2 does with compile errors printed from a
# subdirectory. Builds a throwaway project, captures three real make logs
# (macOS make 3.81, make 3.81 -w, gmake 4.x) plus one synthetic nested log,
# and runs probe.el over them. The project path is printed as <P>.
# Usage: dev/gnu-compile/run.sh > dev/gnu-compile/run.out
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
P=$(cd "$tmp" && pwd -P)/proj
mkdir -p "$P/sim/deep"
printf 'module a;\nwire x\nendmodule\n' > "$P/sim/bad.v"
printf 'x\n' > "$P/top.v"; printf 'x\n' > "$P/sim/s.v"; printf 'x\n' > "$P/sim/deep/d.v"
printf 'all:\n\t$(MAKE) -C sim\n' > "$P/Makefile"
printf 'all:\n\tiverilog -o /dev/null bad.v\n' > "$P/sim/Makefile"

(cd "$P" && make > "$tmp/make.out" 2>&1 || true)
(cd "$P" && make -w > "$tmp/makew.out" 2>&1 || true)
logs="make.out makew.out"
if command -v gmake >/dev/null 2>&1; then
  (cd "$P" && gmake > "$tmp/gmake.out" 2>&1 || true)
  logs="$logs gmake.out"
fi

cat > "$tmp/nested.out" <<EOF
make: Entering directory '$P'
top.v:1: error: A
make[1]: Entering directory '$P/sim'
s.v:1: error: B
make[2]: Entering directory 'deep'
d.v:1: error: C
make[2]: Leaving directory 'deep'
s.v:1: error: D
make[1]: Leaving directory '$P/sim'
top.v:1: error: E
make: Leaving directory '$P'
top.v:1: error: F
EOF
logs="$logs nested.out"

{
  make --version | head -1
  command -v gmake >/dev/null 2>&1 && gmake --version | head -1
  emacs --version | head -1
  for f in $logs; do echo "---- $f"; cat "$tmp/$f"; done
  # shellcheck disable=SC2086
  emacs -Q --batch -l "$here/probe.el" "$tmp" "$P" $logs 2>&1
} | sed "s#$P#<P>#g; s#$tmp#<TMP>#g"
