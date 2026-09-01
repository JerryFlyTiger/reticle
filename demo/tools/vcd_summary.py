#!/usr/bin/env python3
"""Summarise a VCD waveform dump: per-signal toggle counts and activity.

A stuck signal in a long simulation is usually a bug -- a clock enable
that never asserts, a reset that never releases -- and it is much faster
to spot in a toggle table than in a waveform viewer.

    ./vcd_summary.py sim.vcd
    ./vcd_summary.py sim.vcd --top-n 10 --stuck-only
"""

from __future__ import annotations

import argparse
import sys
from collections import defaultdict
from dataclasses import dataclass, field


@dataclass
class Signal:
    """One VCD identifier, which may be visible under several names."""

    ident: str
    width: int
    names: list[str] = field(default_factory=list)
    toggles: int = 0
    last_value: str | None = None

    @property
    def display_name(self) -> str:
        return min(self.names, key=len) if self.names else self.ident


def parse_vcd(stream) -> tuple[dict[str, Signal], int]:
    """Return (signals by identifier, final timestamp).

    Only the parts of the VCD grammar that carry information we use are
    handled; unknown commands are skipped rather than rejected, since
    every simulator emits a slightly different dialect of the optional
    sections.
    """
    signals: dict[str, Signal] = {}
    scope: list[str] = []
    time = 0

    for raw in stream:
        line = raw.strip()
        if not line:
            continue

        if line.startswith("$scope"):
            parts = line.split()
            if len(parts) >= 3:
                scope.append(parts[2])
            continue

        if line.startswith("$upscope"):
            if scope:
                scope.pop()
            continue

        if line.startswith("$var"):
            # $var wire 8 ! data_o $end
            parts = line.split()
            if len(parts) < 5:
                continue
            width, ident, name = int(parts[2]), parts[3], parts[4]
            sig = signals.setdefault(ident, Signal(ident=ident, width=width))
            sig.names.append(".".join(scope + [name]))
            continue

        if line.startswith("#"):
            try:
                time = int(line[1:])
            except ValueError:
                pass
            continue

        # Value changes: scalars are "0!", vectors are "b1010 !".
        if line[0] in "01xXzZ" and len(line) > 1:
            value, ident = line[0], line[1:]
        elif line[0] in "bBrR":
            parts = line.split()
            if len(parts) != 2:
                continue
            value, ident = parts[0][1:], parts[1]
        else:
            continue

        sig = signals.get(ident)
        if sig is None:
            continue
        if sig.last_value is not None and sig.last_value != value:
            sig.toggles += 1
        sig.last_value = value

    return signals, time


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Summarise a VCD waveform dump: per-signal toggle counts."
    )
    ap.add_argument("vcd", help="VCD file, or - for stdin")
    ap.add_argument("--top-n", type=int, default=20, help="how many rows to print")
    ap.add_argument(
        "--stuck-only",
        action="store_true",
        help="only list signals that never changed",
    )
    args = ap.parse_args(argv)

    if args.vcd == "-":
        signals, end_time = parse_vcd(sys.stdin)
    else:
        with open(args.vcd, encoding="utf-8", errors="replace") as fh:
            signals, end_time = parse_vcd(fh)

    if not signals:
        print("no signals found -- is this a VCD file?", file=sys.stderr)
        return 1

    rows = sorted(signals.values(), key=lambda s: (-s.toggles, s.display_name))
    if args.stuck_only:
        rows = [s for s in rows if s.toggles == 0]

    by_width: dict[int, int] = defaultdict(int)
    for sig in signals.values():
        by_width[sig.width] += 1

    print(f"{len(signals)} signals, simulation ends at t={end_time}")
    print(f"widths: " + ", ".join(f"{w}b x{n}" for w, n in sorted(by_width.items())))
    print()
    print(f"{'signal':<48} {'width':>5} {'toggles':>9}")
    print("-" * 64)
    for sig in rows[: args.top_n]:
        print(f"{sig.display_name:<48} {sig.width:>5} {sig.toggles:>9}")

    stuck = sum(1 for s in signals.values() if s.toggles == 0)
    if stuck and not args.stuck_only:
        print(f"\n{stuck} signal(s) never toggled -- rerun with --stuck-only")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
