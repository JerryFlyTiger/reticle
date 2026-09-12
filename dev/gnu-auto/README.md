# GNU `verilog-auto` reference fixtures

Fixtures and a runner for settling **what GNU verilog-mode actually does**, by
running it, for the AUTO family (`AUTOINST`, `AUTOOUTPUT`, `AUTORESET`,
`AUTOUNUSED`, …).

This project's rule is that GNU-parity behaviour is settled by **running the
reference implementation**, never by writing a spec from memory. The rule was
written after it was broken: a spec written from memory asserted a rule about
end-of-buffer behaviour that GNU does not have, the implementation matched the
spec, the tests were written to match too and passed, and the result was a
data-corruption bug that ate a newline belonging to a different line. It was
found by someone running the real thing. The same shape has since recurred for
language-server capabilities and for indentation parity.

The second reason is narrower: **a measurement nobody can reproduce is barely
better than no measurement.** These fixtures produced a milestone's worth of
spec; keeping them means the next one can re-run them instead of re-typing
them, and can tell a genuine behaviour change from a mis-remembered one.

## Running

```sh
dev/gnu-auto/run.sh fixtures/autoreset-basic-two-flops.v
dev/gnu-auto/run.sh fixtures/autoreset-parameterized-widths.v \
    '(setq verilog-auto-reset-widths (quote unbased))'
```

The optional second argument is spliced in as `setq` forms before
`verilog-auto` runs — that is how the knobs get exercised.

Requires GNU Emacs 30.2 at `/opt/homebrew/bin/emacs`.

## Comparing against reticle

`dev/gnu-diffcheck.py` replays the same fixture through the **real reticle
binary** and prints what it generated:

```sh
cargo build --workspace            # gnu-diffcheck.py does NOT build
dev/gnu-diffcheck.py dev/gnu-auto/fixtures/autoreset-basic-two-flops.v
dev/gnu-auto/run.sh  dev/gnu-auto/fixtures/autoreset-basic-two-flops.v
```

**Run both, and run them alongside a cold read of the diff — none of the three
subsumes the others.** When `AUTORESET` was built, the differential replay
found a defect that twenty-one green tests and a full cold review had both missed: `/*AUTORESET*/`
expanded where the marker was not inside a conditional branch, and because the
last assignment wins in a procedural block, that made every signal permanently
zero — valid Verilog, clean lint, dead circuit. In the same round, cold reading
found two defects the replay cannot see: a discarded skip-reason emitting
`arr <= ;`, and a whole-branch exclusion where GNU's is positional.

## What each fixture pins

Every name says what the fixture is for. Run `run.sh` on one to see the
expected GNU output — that is the point of keeping them, rather than
transcribing the outputs into a table that can drift away from the tool.

### AUTORESET — the rule

| fixture | pins |
|---|---|
| `autoreset-basic-two-flops.v` | header/footer text, `<=`, widths from the declaration |
| `autoreset-ordering-and-signal-types.v` | **alphabetical** order; `signed` → `4'sh0`; an unpacked array (where GNU emits illegal code) |
| `autoreset-scoped-to-its-own-always-block.v` | a signal assigned in a *different* `always` is not reset |
| `autoreset-excludes-signal-already-reset.v` | a signal already reset in the marker's branch is excluded |
| `autoreset-assignment-before-marker-excluded.v` | the exclusion is **positional** — before the marker |
| `autoreset-assignment-after-marker-not-excluded.v` | …and an assignment *after* the marker is still reset |
| `autoreset-nested-before-vs-after-marker.v` | nesting depth is irrelevant; position is everything |
| `autoreset-marker-in-else-if-catches-sibling.v` | a **sibling** branch's assignment is never excluded, even when textually earlier |

### AUTORESET — where the marker may sit

| fixture | pins |
|---|---|
| `autoreset-marker-first-no-conditional.v` | no enclosing `if` at all, marker first → GNU **does** reset |
| `autoreset-marker-last-after-case-no-if.v` | marker last after a `case` → nothing, because everything is before it |
| `autoreset-marker-last-after-if-else.v` | marker last at block level → same reason |
| `autoreset-single-statement-reset-branch.v` | a one-statement reset branch |
| `autoreset-always-without-begin-end.v` | an `always` with no outer `begin`/`end` |
| `autoreset-two-markers-two-always-blocks.v` | **every** marker expands, one per `always` — not one per module |

### AUTORESET — what gets reset, and to what

| fixture | pins |
|---|---|
| `autoreset-blocking-assignments-in-comb.v` | `=` mirrored in a combinational block |
| `autoreset-blocking-in-non-blocking-block.v` | the `verilog-auto-reset-blocking-in-non` knob, both ways |
| `autoreset-always-comb-blocking.v` | `always_comb` behaves like `always` |
| `autoreset-case-partselect-forloop-lhs.v` | `case` branches, `c[3:0]`, and `d[i]` in a `for` all reduce to the base identifier |
| `autoreset-concatenation-lvalue.v` | `{a,b} <= …` drives both |
| `autoreset-hierarchical-dotted-lvalue.v` | `top.inner.sig` resets the **full dotted path**, not `top` |
| `autoreset-undeclared-signal-is-one-bit.v` | an undeclared signal is reset as 1 bit |
| `autoreset-parameterized-widths.v` | `{WIDTH{1'b0}}`, `{(1+(WIDTH/2-1)){1'b0}}`, and all three width modes |
| `autoreset-symbolic-multidim-packed-range.v` | GNU emits `arr <= 8'h0;` here; reticle deliberately skips |
| `autoreset-systemverilog-always-ff.sv` | `always_ff` + `logic` — `demo/rtl/`'s own style |

### AUTOUNUSED — the measurement that disqualified it

These four are the evidence that copying GNU's `AUTOUNUSED` would be **wrong for
this project's users**, recorded so the conclusion can be re-checked rather than
taken on trust. GNU judges "unused" purely by whether an **AUTOINST-expanded**
instance port consumed the signal:

| fixture | GNU lists as "unused" | correct? |
|---|---|---|
| `autounused-autoinst-connected-excluded.v` | `spare_i` only | yes |
| `autounused-handwritten-connections-ignored.v` | `a_i`, `clk`, `spare_i` | no — they are connected by hand |
| `autounused-leaf-module-inputs-used-in-assign.v` | `a_i`, `clk`, `spare_i` | no — both are used in an `assign` |
| `autounused-no-instance-lists-every-input.v` | every input, incl. `clk`/`rst_n` | no |
| `autounused-inout-and-ignore-regexp.v` | inouts are included; `verilog-auto-unused-ignore-regexp` filters by name | — |

The `wire _unused_ok = &{…}` idiom exists to tell a linter "I know these are
unused", so emitting used signals into it defeats the lint and hides the
genuinely unused ones. Ten of `demo/rtl/`'s eleven modules are leaf modules.

`sub.v` is the shared submodule the AUTOINST/AUTOUNUSED fixtures instantiate.
