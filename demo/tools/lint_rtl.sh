#!/usr/bin/env bash
#
# lint_rtl.sh -- parse, lint and format-check the demo RTL.
#
# This is the script that backs the claim in demo/README.md that the
# sample design is real RTL rather than plausible-looking filler: run it
# and see for yourself.
#
#     ./lint_rtl.sh              # check everything
#     ./lint_rtl.sh --fix        # rewrite files with the formatter
#
# Requires the Verible tools on PATH (verible-verilog-syntax, -lint,
# -format). Exits non-zero if any check fails, so CI can call it.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
demo_root="$(dirname "$here")"

fix=0
for arg in "$@"; do
  case "$arg" in
    --fix) fix=1 ;;
    -h | --help)
      sed -n '2,15p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

for tool in verible-verilog-syntax verible-verilog-lint verible-verilog-format; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "missing required tool: $tool" >&2
    echo "install Verible: https://github.com/chipsalliance/verible" >&2
    exit 127
  fi
done

# Collect sources without relying on `find -print0` support quirks.
sources=()
while IFS= read -r file; do
  sources+=("$file")
done < <(find "$demo_root" -type f \( -name '*.sv' -o -name '*.svh' -o -name '*.v' \) | sort)

if [[ ${#sources[@]} -eq 0 ]]; then
  echo "no Verilog sources found under $demo_root" >&2
  exit 1
fi

echo "checking ${#sources[@]} file(s) under ${demo_root}"

failures=0

step() {
  local label="$1"
  shift
  printf '%-16s ' "$label"
  if "$@" >/tmp/lint_rtl.$$ 2>&1; then
    echo "ok"
  else
    echo "FAILED"
    sed 's/^/    /' /tmp/lint_rtl.$$
    failures=$((failures + 1))
  fi
  rm -f /tmp/lint_rtl.$$
}

step "syntax" verible-verilog-syntax "${sources[@]}"

# --rules_config_search makes the linter walk upward from each file
# looking for `.rules.verible_lint'. Without it the waiver file in
# demo/rtl-verilog2001/ (which turns off three SystemVerilog-only style
# rules that plain Verilog-2001 cannot satisfy) is ignored.
step "lint" verible-verilog-lint --rules_config_search "${sources[@]}"

format_check() {
  # verible-verilog-format refuses --verify with several files at once
  # ("--inplace required for multiple files"), so check one at a time.
  local rc=0 file
  for file in "${sources[@]}"; do
    verible-verilog-format --verify "$file" || rc=1
  done
  return $rc
}

if [[ $fix -eq 1 ]]; then
  step "format (rewrite)" verible-verilog-format --inplace "${sources[@]}"
else
  step "format" format_check
fi

if [[ $failures -gt 0 ]]; then
  echo
  echo "$failures check(s) failed"
  exit 1
fi

echo
echo "all checks passed"
