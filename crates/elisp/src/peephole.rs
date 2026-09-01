//! Bytecode peephole optimizer (M10 item 4): constant folding, dead
//! Dup/Pop elimination, and jump-to-jump threading. A small analog of
//! GNU Emacs's byte-opt.el, run once right after the compiler finishes
//! a chunk, before it's wrapped for the VM/JIT.
//!
//! ## Why folding is restricted to Int literals only
//!
//! Folding `Const(a); Const(b); Add2` into a single `Const(result)` is
//! safe because integers are immutable values — there is no way for
//! "the result of adding these two constants" to differ between compile
//! time and whenever the bytecode actually runs. That is NOT true in
//! general: a quoted list literal in the constant pool is a *shared,
//! mutable* cons cell (`(setcar my-quoted-list ...)` is legal elisp),
//! so folding e.g. `(car '(1 2))` to the literal `1` at compile time
//! would be wrong if something mutates that literal before this
//! particular call runs. So only Add2/Sub2/Mul2/the comparisons/Inc1/
//! Dec1 — which only ever take `Value::Int` operands — are folded;
//! Car1/Cdr1/Eq2/Cons2/Not1 are left alone even when their operand is a
//! literal constant.
//!
//! ## Why the code stays the same length until the very end
//!
//! Jump targets are absolute instruction indices. Rather than deleting
//! instructions (which would require re-deriving every jump target
//! immediately), folded/eliminated instructions are first replaced with
//! `None` in a same-length `Vec<Option<Instr>>` — every index stays
//! where it is while folding and jump-threading run — and only the
//! final `compact` pass removes the `None` holes and rewrites jump
//! targets through an old-index -> new-index map in one step.
//!
//! A fold/elimination is skipped whenever some *other* jump targets the
//! middle of the instructions it would remove — the only way it could
//! be observably wrong (the compiler never actually generates such
//! jumps into our own optimizable windows, but the check costs nothing
//! and keeps this correct even if that ever changes).

use std::collections::HashSet;

use num_bigint::BigInt;

use crate::bytecode::Instr;
use crate::value::{SymId, Value};

pub fn optimize(code: Vec<Instr>, consts: &mut Vec<Value>, t_sym: SymId) -> Vec<Instr> {
    let targets = jump_targets(&code);
    let mut slots: Vec<Option<Instr>> = code.into_iter().map(Some).collect();
    fold_constants(&mut slots, consts, &targets, t_sym);
    eliminate_dup_pop(&mut slots, &targets);
    thread_jumps(&mut slots);
    compact(slots)
}

fn jump_targets(code: &[Instr]) -> HashSet<usize> {
    let mut s = HashSet::new();
    for instr in code {
        match instr {
            Instr::Jump(t) | Instr::JumpIfNil(t) | Instr::JumpIfNonNil(t) => {
                s.insert(*t);
            }
            _ => {}
        }
    }
    s
}

fn next_present(slots: &[Option<Instr>], from: usize) -> Option<usize> {
    (from..slots.len()).find(|&i| slots[i].is_some())
}

/// Fold `Const(int); Const(int); <Add2|Sub2|Mul2|Lt2|...>` to a single
/// `Const`, and `Const(int); <Inc1|Dec1>` likewise. Runs to a fixed
/// point (via `next_present`, which skips already-folded holes) so a
/// chain like `(+ 1 2 3)` collapses all the way down in one call.
fn fold_constants(
    slots: &mut [Option<Instr>],
    consts: &mut Vec<Value>,
    targets: &HashSet<usize>,
    t_sym: SymId,
) {
    loop {
        let mut changed = false;
        let mut i = 0;
        while let Some(p1) = next_present(slots, i) {
            i = p1 + 1;
            // Try the 2-wide window first (Inc1/Dec1 on p1's successor).
            if let Some(p2) = next_present(slots, p1 + 1) {
                if !targets.contains(&p2) {
                    if let (Some(Instr::Const(c1)), Some(op2)) = (slots[p1], slots[p2]) {
                        if let Value::Int(x) = consts[c1 as usize] {
                            if let Some(v) = fold_unary(op2, x) {
                                slots[p1] = Some(Instr::Const(push_const(consts, v)));
                                slots[p2] = None;
                                changed = true;
                                continue;
                            }
                        }
                    }
                }
                // 3-wide window: Const, Const, BinOp.
                if let Some(p3) = next_present(slots, p2 + 1) {
                    if !targets.contains(&p2) && !targets.contains(&p3) {
                        if let (Some(Instr::Const(c1)), Some(Instr::Const(c2)), Some(op3)) =
                            (slots[p1], slots[p2], slots[p3])
                        {
                            if let (Value::Int(x), Value::Int(y)) =
                                (&consts[c1 as usize], &consts[c2 as usize])
                            {
                                if let Some(v) = fold_binary(op3, *x, *y, t_sym) {
                                    slots[p1] = Some(Instr::Const(push_const(consts, v)));
                                    slots[p2] = None;
                                    slots[p3] = None;
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn push_const(consts: &mut Vec<Value>, v: Value) -> u32 {
    consts.push(v);
    (consts.len() - 1) as u32
}

fn fold_unary(op: Instr, x: i64) -> Option<Value> {
    match op {
        Instr::Inc1(_) => Some(match x.checked_add(1) {
            Some(r) => Value::Int(r),
            None => Value::big(BigInt::from(x) + 1),
        }),
        Instr::Dec1(_) => Some(match x.checked_sub(1) {
            Some(r) => Value::Int(r),
            None => Value::big(BigInt::from(x) - 1),
        }),
        _ => None,
    }
}

/// Mirrors the VM's Add2/Sub2/Mul2 overflow-promotes-to-bignum
/// semantics and the comparison opcodes' exact-integer semantics
/// exactly — folding must never be observably different from running
/// the unfolded instructions would have been.
fn fold_binary(op: Instr, x: i64, y: i64, t_sym: SymId) -> Option<Value> {
    match op {
        Instr::Add2(_) => Some(match x.checked_add(y) {
            Some(r) => Value::Int(r),
            None => Value::big(BigInt::from(x) + BigInt::from(y)),
        }),
        Instr::Sub2(_) => Some(match x.checked_sub(y) {
            Some(r) => Value::Int(r),
            None => Value::big(BigInt::from(x) - BigInt::from(y)),
        }),
        Instr::Mul2(_) => Some(match x.checked_mul(y) {
            Some(r) => Value::Int(r),
            None => Value::big(BigInt::from(x) * BigInt::from(y)),
        }),
        Instr::Lt2(_) => Some(bool_val(x < y, t_sym)),
        Instr::Gt2(_) => Some(bool_val(x > y, t_sym)),
        Instr::Le2(_) => Some(bool_val(x <= y, t_sym)),
        Instr::Ge2(_) => Some(bool_val(x >= y, t_sym)),
        Instr::NumEq2(_) => Some(bool_val(x == y, t_sym)),
        _ => None,
    }
}

fn bool_val(b: bool, t_sym: SymId) -> Value {
    if b {
        Value::Sym(t_sym)
    } else {
        Value::Nil
    }
}

/// Eliminates two shapes of "produce a value nobody wanted":
///
///  * `Dup; Pop` directly adjacent — a net no-op.
///  * `Dup; StoreLocal(_)|StoreFree(_); Pop` — every `setq` emits a Dup
///    so it has a return value (`(setq x 1)` evaluates to `1`), but
///    when the `setq` isn't the last form in its `progn`/`let` body,
///    `compile_progn` immediately poops that return value right back
///    off — this is the single most common source of dead Dup/Pop
///    pairs in real code, and unlike the first shape it is NOT directly
///    adjacent (the Store sits in between), so it needs its own check.
///    Collapses to just the Store instruction.
fn eliminate_dup_pop(slots: &mut [Option<Instr>], targets: &HashSet<usize>) {
    let mut i = 0;
    while let Some(p1) = next_present(slots, i) {
        i = p1 + 1;
        if !matches!(slots[p1], Some(Instr::Dup)) {
            continue;
        }
        let Some(p2) = next_present(slots, p1 + 1) else {
            continue;
        };
        if targets.contains(&p2) {
            continue;
        }
        if matches!(slots[p2], Some(Instr::Pop)) {
            slots[p1] = None;
            slots[p2] = None;
            continue;
        }
        if matches!(
            slots[p2],
            Some(Instr::StoreLocal(_)) | Some(Instr::StoreFree(_))
        ) {
            let Some(p3) = next_present(slots, p2 + 1) else {
                continue;
            };
            if targets.contains(&p3) {
                continue;
            }
            if matches!(slots[p3], Some(Instr::Pop)) {
                slots[p1] = None;
                slots[p3] = None;
            }
        }
    }
}

/// If a jump's target is itself an unconditional `Jump`, redirect
/// straight to its final destination — avoids bouncing through a
/// trampoline instruction on every taken branch.
fn thread_jumps(slots: &mut [Option<Instr>]) {
    for i in 0..slots.len() {
        match slots[i] {
            Some(Instr::Jump(t)) => slots[i] = Some(Instr::Jump(chase(slots, t))),
            Some(Instr::JumpIfNil(t)) => slots[i] = Some(Instr::JumpIfNil(chase(slots, t))),
            Some(Instr::JumpIfNonNil(t)) => slots[i] = Some(Instr::JumpIfNonNil(chase(slots, t))),
            _ => {}
        }
    }
}

fn chase(slots: &[Option<Instr>], mut t: usize) -> usize {
    let mut seen = HashSet::new();
    loop {
        if !seen.insert(t) {
            return t; // degenerate cycle guard
        }
        match slots.get(t) {
            Some(Some(Instr::Jump(t2))) if *t2 != t => t = *t2,
            _ => return t,
        }
    }
}

/// Remove the `None` holes and rewrite every remaining jump's target
/// through the resulting old-index -> new-index map.
fn compact(mut slots: Vec<Option<Instr>>) -> Vec<Instr> {
    let n = slots.len();
    let mut new_index = vec![0usize; n + 1];
    let mut next = 0usize;
    for (i, slot) in slots.iter().enumerate() {
        new_index[i] = next;
        if slot.is_some() {
            next += 1;
        }
    }
    new_index[n] = next;
    for slot in slots.iter_mut() {
        match slot {
            Some(Instr::Jump(t)) => *t = new_index[*t],
            Some(Instr::JumpIfNil(t)) => *t = new_index[*t],
            Some(Instr::JumpIfNonNil(t)) => *t = new_index[*t],
            _ => {}
        }
    }
    slots.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Direct unit tests of the mechanics, independent of whether the
    // compiler happens to produce these exact shapes today — jump
    // chains in particular are rare in this compiler's current output
    // (every construct tends to end in a value-producing instruction
    // rather than a bare trailing Jump), so this is the reliable way
    // to pin down that threading and compaction are correct.

    #[test]
    fn jump_chain_threads_to_final_target() {
        // 0: Jump(1)   1: Jump(2)   2: Jump(3)   3: Return
        let code = vec![
            Instr::Jump(1),
            Instr::Jump(2),
            Instr::Jump(3),
            Instr::Return,
        ];
        let mut slots: Vec<Option<Instr>> = code.into_iter().map(Some).collect();
        thread_jumps(&mut slots);
        assert!(matches!(slots[0], Some(Instr::Jump(3))));
        assert!(matches!(slots[1], Some(Instr::Jump(3))));
    }

    #[test]
    fn jump_self_cycle_does_not_hang() {
        let code = vec![Instr::Jump(0)];
        let mut slots: Vec<Option<Instr>> = code.into_iter().map(Some).collect();
        thread_jumps(&mut slots); // must terminate
        assert!(matches!(slots[0], Some(Instr::Jump(0))));
    }

    #[test]
    fn compact_removes_holes_and_remaps_targets() {
        // Const, [hole], [hole], JumpIfNil(0), Return
        let slots: Vec<Option<Instr>> = vec![
            Some(Instr::Const(0)),
            None,
            None,
            Some(Instr::JumpIfNil(0)),
            Some(Instr::Return),
        ];
        let out = compact(slots);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Instr::Const(0)));
        // The jump's target (old index 0) must remap to new index 0.
        assert!(matches!(out[1], Instr::JumpIfNil(0)));
        assert!(matches!(out[2], Instr::Return));
    }

    #[test]
    fn fold_binary_matches_runtime_overflow_semantics() {
        let t = 7; // arbitrary SymId stand-in
        assert!(matches!(
            fold_binary(Instr::Add2(0), 2, 3, t),
            Some(Value::Int(5))
        ));
        // i64::MAX + 1 must promote to a bignum, matching the VM's
        // checked_add-then-promote path exactly.
        let r = fold_binary(Instr::Add2(0), i64::MAX, 1, t);
        assert!(matches!(r, Some(Value::Big(_))));
        assert!(matches!(
            fold_binary(Instr::Lt2(0), 2, 3, t),
            Some(Value::Sym(7))
        ));
        assert!(matches!(
            fold_binary(Instr::Lt2(0), 3, 2, t),
            Some(Value::Nil)
        ));
    }
}
