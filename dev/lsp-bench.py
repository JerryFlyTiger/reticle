#!/usr/bin/env python3
"""Measure real LSP server request latency -- `lsp-probe.py`'s sister tool.

`lsp-probe.py` answers "**what** does the server reply"; this one answers
"**how long** does it take to reply". Kept separate because they catch
different classes of mistake: the probe catches untrustworthy capability
declarations and off-target positions; this one catches **unfounded
performance assumptions**.

One wrong call it has caught (M51, which directly decided the milestone's
scope):

    A candidate item in `PLAN.md` said "auto-triggering means every cursor
    dwell sends a request, and the load on the server for large files is
    unmeasured", and based on that a local "is this even an identifier"
    filter was going to be added before sending the request. Measured
    against real verible-verilog-ls: **0.1 ms** after warm-up on 6009 lines
    of SystemVerilog, 1.3 ms total for 20 requests in a row, and 0.0 ms
    with an empty array returned at non-identifier positions. Load was
    never the constraint; that filter would have added complexity for a
    bottleneck that doesn't exist, and was cut from the spec outright.

**The key to measuring this correctly is separating the "after warm-up" and
"after an edit" states.** verible does lazy re-parse: the parse cost is
only paid on the first request after a didChange (49 ms for 6009 lines),
and subsequent requests in the same generation drop back to 0.1 ms.
Measuring only one of the two would give a conclusion off by 500x. And
cursor movement **does not produce** a didChange, so "only triggers on
cursor dwell" features live in the first state as their steady state.

Usage
    # synthetic Verilog, sweep three sizes (default)
    dev/lsp-bench.py

    # specify sizes (the number is signal count, roughly 3 lines each)
    dev/lsp-bench.py --sizes 100,1000,5000

    # measure a real file (position via --line/--char; if omitted, finds
    # the first identifier)
    dev/lsp-bench.py --file foo.sv --line 12 --char 9

    # switch method / server
    dev/lsp-bench.py --method textDocument/hover --server svls
"""

import argparse
import importlib.util
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))


def _load_probe_class():
    """Borrow LspProbe from lsp-probe.py (the filename has a hyphen, can't
    import it directly).

    Deliberately not duplicating the JSON-RPC framing -- reading and
    writing Content-Length should only have one implementation.
    """
    path = os.path.join(HERE, "lsp-probe.py")
    spec = importlib.util.spec_from_file_location("lsp_probe", path)
    if spec is None or spec.loader is None:
        sys.exit(f"failed to load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.LspProbe


LspProbe = _load_probe_class()


def gen_verilog(n_signals):
    """Synthesize a module with n_signals wires, each signal declared once
    plus referenced three times.

    The shape deliberately resembles real RTL (a declaration block,
    continuous assigns, an always_comb branch), not random characters --
    the server's parse cost is sensitive to structure.
    """
    lines = ["module bench (", "  input clk,", "  input rst_n,", "  output done", ");"]
    for i in range(n_signals):
        lines.append(f"  wire [7:0] sig_{i};")
    for i in range(n_signals):
        lines.append(f"  assign sig_{i} = sig_{(i + 1) % n_signals} + 8'd{i % 256};")
    lines.append("  always_comb begin")
    for i in range(n_signals):
        lines.append(f"    if (sig_{i} == 8'h00) $display(\"sig_{i}\");")
    lines.append("  end")
    tail = ", ".join(f"sig_{i}" for i in range(min(n_signals, 64)))
    lines.append("  assign done = |{" + tail + "};")
    lines.append("endmodule")
    return "\n".join(lines) + "\n"


def find_pos(text, needle, occurrence=0):
    """The (line, char) of needle's occurrence-th occurrence, 0-based. Returns None if not found."""
    seen = 0
    for li, src in enumerate(text.split("\n")):
        start = 0
        while True:
            idx = src.find(needle, start)
            if idx < 0:
                break
            if seen == occurrence:
                return li, idx
            seen += 1
            start = idx + 1
    return None


def timed(probe, method, uri, line, char):
    t0 = time.perf_counter()
    resp = probe.request(method, {
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": char},
    })
    ms = (time.perf_counter() - t0) * 1000
    result = resp.get("result") if resp else None
    n = len(result) if isinstance(result, list) else (-1 if result is None else 1)
    return ms, n


def median(xs):
    s = sorted(xs)
    return s[len(s) // 2]


def bench(server, text, path, method, pos, burst, label):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    uri = "file://" + os.path.abspath(path)
    probe = LspProbe([server], os.path.dirname(os.path.abspath(path)))
    try:
        probe.request("initialize", {
            "processId": os.getpid(),
            "rootUri": "file://" + os.path.dirname(os.path.abspath(path)),
            "capabilities": {},
        })
        probe.notify("initialized", {})

        print(f"\n=== {label}: {text.count(chr(10))} lines / {len(text)} bytes ===")

        t0 = time.perf_counter()
        probe.notify("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "systemverilog", "version": 1, "text": text}})
        first_ms, first_n = timed(probe, method, uri, *pos)
        print(f"  didOpen -> first response: {(time.perf_counter() - t0) * 1000:.1f} ms "
              f"(that request took {first_ms:.1f} ms, {first_n} item(s))")

        warm = [timed(probe, method, uri, *pos)[0] for _ in range(5)]
        print(f"  same position after warm-up x5: {[f'{m:.1f}' for m in warm]} ms  "
              f"(median {median(warm):.1f})")

        t0 = time.perf_counter()
        shots = [timed(probe, method, uri, *pos)[0] for _ in range(burst)]
        total = (time.perf_counter() - t0) * 1000
        print(f"  {burst} in a row (no gaps): total {total:.1f} ms, "
              f"avg {total / burst:.1f} ms/req, slowest {max(shots):.1f} ms")

        # After an edit: didChange invalidates the parse, and the next
        # request pays the re-parse cost.
        edited = []
        ver = 1
        for k in range(3):
            ver += 1
            probe.notify("textDocument/didChange", {
                "textDocument": {"uri": uri, "version": ver},
                "contentChanges": [{"text": text.replace(
                    "output done", f"output done /* e{k} */", 1)}],
            })
            edited.append(timed(probe, method, uri, *pos)[0])
        print(f"  first request after didChange (whole doc) x3: {[f'{m:.1f}' for m in edited]} ms  "
              f"(median {median(edited):.1f}) <- re-parse cost")

        # Non-identifier positions: decides whether a local filter is
        # needed before sending the request.
        spots = []
        decl = find_pos(text, "  wire [7:0] sig_0;", 0)
        if decl:
            spots += [("leading whitespace", decl[0], 0), ("keyword wire", decl[0], 2)]
        for needle, name, off in ((";", "semicolon", 0), ("endmodule", "endmodule", 0)):
            q = find_pos(text, needle, 0)
            if q:
                spots.append((name, q[0], q[1] + off))
        for name, li, ch in spots:
            ms, n = timed(probe, method, uri, li, ch)
            shape = "null" if n == -1 else f"{n} item(s)"
            print(f"  non-identifier \"{name}\" (line {li} char {ch}): {ms:.1f} ms -> {shape}")
    finally:
        probe.close()


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", default="verible-verilog-ls")
    ap.add_argument("--method", default="textDocument/documentHighlight")
    ap.add_argument("--sizes", default="50,500,2000",
                    help="signal counts for synthetic mode, comma-separated (default 50,500,2000)")
    ap.add_argument("--burst", type=int, default=20, help="number of requests in a row (default 20)")
    ap.add_argument("--file", help="measure a real file instead of a synthetic one")
    ap.add_argument("--line", type=int, help="0-based line for --file mode")
    ap.add_argument("--char", type=int, help="0-based char for --file mode")
    ap.add_argument("--workdir", default=None,
                    help="directory to write synthetic files into (default: system temp dir)")
    args = ap.parse_args()

    if args.file:
        text = open(args.file, encoding="utf-8").read()
        if args.line is None or args.char is None:
            sys.exit("--file mode requires --line and --char "
                     "(use dev/lsp-probe.py first to confirm the position really points at an identifier)")
        bench(args.server, text, os.path.abspath(args.file), args.method,
              (args.line, args.char), args.burst, os.path.basename(args.file))
        return

    import tempfile
    workdir = args.workdir or tempfile.mkdtemp(prefix="lsp-bench-")
    os.makedirs(workdir, exist_ok=True)
    for n in [int(s) for s in args.sizes.split(",")]:
        text = gen_verilog(n)
        # take sig_0's second occurrence (skip the declaration, land on a use)
        pos = find_pos(text, "sig_0", 1)
        if pos is None:
            sys.exit("could not find a use of sig_0 in the synthetic file")
        bench(args.server, text, os.path.join(workdir, f"bench_{n}.sv"),
              args.method, pos, args.burst, f"synthetic {n} signals")
    print(f"\nsynthetic files left in {workdir} (not inside the repo)")


if __name__ == "__main__":
    main()
