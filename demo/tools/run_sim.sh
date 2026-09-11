#!/usr/bin/env bash
#
# run_sim.sh -- actually simulate the demo's Verilog and SystemVerilog,
# with Icarus Verilog.
#
# This is the script that backs the claim in demo/README.md that the
# testbenches don't just parse and lint clean -- they run and pass.
#
#     ./run_sim.sh              # run both simulations
#     ./run_sim.sh --help
#
# Requires Icarus Verilog (iverilog, vvp) on PATH. Concurrent SVA and
# covergroups are not compiled in (Icarus 13.0 does not support either;
# demo/README.md names the exact tool output), so the demo/verif/
# simulation runs with `-DSOC_SVA_OFF -DSOC_COVERAGE_OFF`.
# demo/verif/axi4_lite_monitor.sv is not part of either simulation at
# all: it is a module whose own port is interface-typed, which Icarus
# 13.0 also does not support ("Errors in port declarations.").
#
# Exits non-zero if any simulation fails to build, fails to print its
# PASS line, or exits non-zero, and non-zero if fewer than 4
# simulations actually ran -- a discovery mechanism that silently runs
# nothing must not be able to report success.
#
# Set DEMO_SIM_ALLOW_MISSING=1 (or yes/true, case-insensitive) to treat
# a missing Icarus Verilog installation as "skip, don't fail" instead
# of the default hard error. Any other value, including 0, means "not
# allowed to skip".

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
demo_root="$(dirname "$here")"

for arg in "$@"; do
  case "$arg" in
    -h | --help)
      sed -n '2,24p' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

is_affirmative() {
  case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
    1 | yes | true) return 0 ;;
    *) return 1 ;;
  esac
}

for tool in iverilog vvp; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    if is_affirmative "${DEMO_SIM_ALLOW_MISSING:-}"; then
      echo "missing required tool: $tool -- skipping (DEMO_SIM_ALLOW_MISSING set)" >&2
      exit 0
    fi
    echo "missing required tool: $tool" >&2
    echo "install Icarus Verilog: https://steveicarus.github.io/iverilog/" >&2
    echo "or set DEMO_SIM_ALLOW_MISSING=1 to skip this check deliberately" >&2
    exit 127
  fi
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/demo-run-sim.XXXXXX")"
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT

ran=0
failures=0

run_one() {
  local label="$1" out="$2"
  shift 2
  local safe_label="${label//\//_}"
  local log="$work_dir/${safe_label}.log"
  printf '%-24s ' "$label"
  if "$@" >"$log" 2>&1 && vvp "$out" >>"$log" 2>&1; then
    if grep -q '^PASS:' "$log"; then
      echo "ok ($(grep '^PASS:' "$log"))"
      ran=$((ran + 1))
      return 0
    fi
    echo "FAILED (no PASS: line)"
  else
    echo "FAILED (build or run error)"
  fi
  sed 's/^/    /' "$log"
  failures=$((failures + 1))
  return 1
}

run_one "rtl-verilog2001/fifo_sync" "$work_dir/fifo_sync.vvp" \
  iverilog -o "$work_dir/fifo_sync.vvp" \
  "$demo_root/rtl-verilog2001/fifo_sync.v" \
  "$demo_root/rtl-verilog2001/fifo_sync_tb.v" || true

run_one "rtl-verilog2001/gray_ctr" "$work_dir/gray_ctr.vvp" \
  iverilog -o "$work_dir/gray_ctr.vvp" \
  "$demo_root/rtl-verilog2001/gray_ctr.v" \
  "$demo_root/rtl-verilog2001/gray_ctr_tb.v" || true

run_one "rtl-verilog2001/fifo_gray_top" "$work_dir/fifo_gray_top.vvp" \
  iverilog -o "$work_dir/fifo_gray_top.vvp" \
  "$demo_root/rtl-verilog2001/fifo_sync.v" \
  "$demo_root/rtl-verilog2001/gray_ctr.v" \
  "$demo_root/rtl-verilog2001/fifo_gray_top.v" \
  "$demo_root/rtl-verilog2001/fifo_gray_top_tb.v" || true

run_one "verif/sram_bank_tb" "$work_dir/sram_bank_tb.vvp" \
  iverilog -g2012 -DSOC_SVA_OFF -DSOC_COVERAGE_OFF \
  -I "$demo_root/rtl/include" \
  -o "$work_dir/sram_bank_tb.vvp" \
  "$demo_root/rtl/pkg/soc_pkg.sv" \
  "$demo_root/rtl/core/clk_gate.sv" \
  "$demo_root/rtl/mem/sram_wrapper.sv" \
  "$demo_root/rtl/mem/sram_bank.sv" \
  "$demo_root/rtl/bus/axi4_lite_if.sv" \
  "$demo_root/verif/soc_verif_pkg.sv" \
  "$demo_root/verif/sram_bank_tb.sv" || true

if [[ $ran -lt 4 ]]; then
  echo
  echo "only $ran/4 simulation(s) actually ran and passed -- treating as failure" >&2
  exit 1
fi

if [[ $failures -gt 0 ]]; then
  echo
  echo "$failures simulation(s) failed"
  exit 1
fi

echo
echo "all simulations passed"
