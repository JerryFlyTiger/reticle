use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};

use super::{defun, need_int};
use crate::error::Flow;
use crate::interp::Interp;
use crate::value::Value;

/// The numeric tower (M11 bignum): fixnum → bignum → float. Integer
/// overflow PROMOTES to `Big` instead of signaling arith-error (Emacs
/// 27+ semantics); mixing with a float contaminates to float.
enum Num {
    Int(i64),
    Big(BigInt),
    Float(f64),
}

fn need_num(interp: &mut Interp, v: &Value) -> Result<Num, Flow> {
    match v {
        Value::Int(i) => Ok(Num::Int(*i)),
        Value::Big(b) => Ok(Num::Big((**b).clone())),
        Value::Float(f) => Ok(Num::Float(*f)),
        _ => Err(interp.wrong_type("numberp", v)),
    }
}

fn num_value(n: Num) -> Value {
    match n {
        Num::Int(i) => Value::Int(i),
        Num::Big(b) => Value::big(b), // canonicalizes back to Int if it fits
        Num::Float(f) => Value::Float(f),
    }
}

fn as_f64(n: &Num) -> f64 {
    match n {
        Num::Int(i) => *i as f64,
        Num::Big(b) => b.to_f64().unwrap_or(f64::INFINITY),
        Num::Float(f) => *f,
    }
}

fn fold(
    interp: &mut Interp,
    args: &[Value],
    init: i64,
    fi: fn(i64, i64) -> Option<i64>,
    fb: fn(&BigInt, &BigInt) -> BigInt,
    ff: fn(f64, f64) -> f64,
) -> Result<Value, Flow> {
    let mut acc = Num::Int(init);
    let mut first = true;
    for a in args {
        let n = need_num(interp, a)?;
        if first && args.len() > 1 {
            acc = n;
            first = false;
            continue;
        }
        first = false;
        acc = match (acc, n) {
            (Num::Int(x), Num::Int(y)) => match fi(x, y) {
                Some(r) => Num::Int(r),
                // Overflow: promote to bignum and redo this step there.
                None => Num::Big(fb(&BigInt::from(x), &BigInt::from(y))),
            },
            (Num::Big(x), Num::Big(y)) => Num::Big(fb(&x, &y)),
            (Num::Big(x), Num::Int(y)) => Num::Big(fb(&x, &BigInt::from(y))),
            (Num::Int(x), Num::Big(y)) => Num::Big(fb(&BigInt::from(x), &y)),
            (x @ Num::Big(_), Num::Float(y)) => Num::Float(ff(as_f64(&x), y)),
            (Num::Float(x), y @ Num::Big(_)) => Num::Float(ff(x, as_f64(&y))),
            (Num::Int(x), Num::Float(y)) => Num::Float(ff(x as f64, y)),
            (Num::Float(x), Num::Int(y)) => Num::Float(ff(x, y as f64)),
            (Num::Float(x), Num::Float(y)) => Num::Float(ff(x, y)),
        };
    }
    Ok(num_value(acc))
}

/// Exact ordering for the tower: integer×integer compares exactly
/// (never through f64, which silently equates giants past 2^53);
/// anything involving a float compares as f64.
fn cmp_nums(a: &Num, b: &Num) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(y)),
        (Num::Big(x), Num::Big(y)) => Some(x.cmp(y)),
        (Num::Big(x), Num::Int(y)) => Some(x.cmp(&BigInt::from(*y))),
        (Num::Int(x), Num::Big(y)) => Some(BigInt::from(*x).cmp(y)),
        _ => as_f64(a).partial_cmp(&as_f64(b)),
    }
}

fn compare_chain(
    interp: &mut Interp,
    args: &[Value],
    ok: fn(std::cmp::Ordering) -> bool,
) -> Result<Value, Flow> {
    // Two-fixnum fast path (P2.3): the overwhelmingly common shape in
    // interpreted loop conditions skips the Num-tower round trip.
    if let [Value::Int(x), Value::Int(y)] = args {
        return Ok(Value::bool(ok(x.cmp(y)), interp.syms.t));
    }
    for w in args.windows(2) {
        let a = need_num(interp, &w[0])?;
        let b = need_num(interp, &w[1])?;
        let ord = cmp_nums(&a, &b).ok_or_else(|| {
            let e = interp.syms.arith_error;
            interp.signal(e, vec![])
        })?;
        if !ok(ord) {
            return Ok(Value::Nil);
        }
    }
    Ok(Value::Sym(interp.syms.t))
}

/// Integer (fixnum or bignum) as BigInt, for %/mod.
fn need_integer(interp: &mut Interp, v: &Value) -> Result<BigInt, Flow> {
    match v {
        Value::Int(i) => Ok(BigInt::from(*i)),
        Value::Big(b) => Ok((**b).clone()),
        _ => Err(interp.wrong_type("integerp", v)),
    }
}

pub fn register(interp: &mut Interp) {
    // The two-fixnum fast paths below (P2.3) skip the Num-tower fold for
    // the dominant case; overflow or any non-Int operand falls through
    // to the general path, which recomputes from scratch (safe: `fold`
    // is pure over its inputs).
    defun(interp, "+", 0, None, |i, a| {
        if let [Value::Int(x), Value::Int(y)] = a {
            if let Some(r) = x.checked_add(*y) {
                return Ok(Value::Int(r));
            }
        }
        fold(i, a, 0, i64::checked_add, |x, y| x + y, |x, y| x + y)
    });
    defun(interp, "*", 0, None, |i, a| {
        if let [Value::Int(x), Value::Int(y)] = a {
            if let Some(r) = x.checked_mul(*y) {
                return Ok(Value::Int(r));
            }
        }
        fold(i, a, 1, i64::checked_mul, |x, y| x * y, |x, y| x * y)
    });
    defun(interp, "-", 1, None, |i, a| {
        if let [Value::Int(x), Value::Int(y)] = a {
            if let Some(r) = x.checked_sub(*y) {
                return Ok(Value::Int(r));
            }
        }
        if a.len() == 1 {
            return match need_num(i, &a[0])? {
                // checked_neg: -(i64::MIN) doesn't fit — promote.
                Num::Int(x) => Ok(match x.checked_neg() {
                    Some(r) => Value::Int(r),
                    None => Value::big(-BigInt::from(x)),
                }),
                Num::Big(x) => Ok(Value::big(-x)),
                Num::Float(x) => Ok(Value::Float(-x)),
            };
        }
        fold(i, a, 0, i64::checked_sub, |x, y| x - y, |x, y| x - y)
    });
    defun(interp, "/", 1, None, |i, a| {
        if a.len() == 1 {
            return match need_num(i, &a[0])? {
                Num::Int(1) => Ok(Value::Int(1)),
                Num::Int(0) => {
                    let e = i.syms.arith_error;
                    Err(i.signal(e, vec![]))
                }
                Num::Int(x) => Ok(Value::Int(1 / x)),
                // Canonicalization: a Big is never 0/±1, so 1/big = 0.
                Num::Big(_) => Ok(Value::Int(0)),
                Num::Float(x) => Ok(Value::Float(1.0 / x)),
            };
        }
        for d in &a[1..] {
            // Only exact-integer zero signals; float 0.0 divides to
            // inf, like Emacs.
            if matches!(need_num(i, d)?, Num::Int(0)) {
                let e = i.syms.arith_error;
                return Err(i.signal(e, vec![]));
            }
        }
        fold(i, a, 1, i64::checked_div, |x, y| x / y, |x, y| x / y)
    });
    defun(interp, "%", 2, Some(2), |i, a| {
        let x = need_integer(i, &a[0])?;
        let y = need_integer(i, &a[1])?;
        if y.is_zero() {
            let e = i.syms.arith_error;
            return Err(i.signal(e, vec![]));
        }
        Ok(Value::big(x % y))
    });
    defun(interp, "mod", 2, Some(2), |i, a| {
        let x = need_integer(i, &a[0])?;
        let y = need_integer(i, &a[1])?;
        if y.is_zero() {
            let e = i.syms.arith_error;
            return Err(i.signal(e, vec![]));
        }
        let mut r = x % &y;
        if !r.is_zero() && (r.is_negative() != y.is_negative()) {
            r += &y;
        }
        Ok(Value::big(r))
    });
    defun(interp, "1+", 1, Some(1), |i, a| {
        if let Value::Int(x) = &a[0] {
            if let Some(r) = x.checked_add(1) {
                return Ok(Value::Int(r));
            }
        }
        match need_num(i, &a[0])? {
            Num::Int(x) => Ok(match x.checked_add(1) {
                Some(r) => Value::Int(r),
                None => Value::big(BigInt::from(x) + 1),
            }),
            Num::Big(x) => Ok(Value::big(x + 1)),
            Num::Float(x) => Ok(Value::Float(x + 1.0)),
        }
    });
    defun(interp, "1-", 1, Some(1), |i, a| {
        if let Value::Int(x) = &a[0] {
            if let Some(r) = x.checked_sub(1) {
                return Ok(Value::Int(r));
            }
        }
        match need_num(i, &a[0])? {
            Num::Int(x) => Ok(match x.checked_sub(1) {
                Some(r) => Value::Int(r),
                None => Value::big(BigInt::from(x) - 1),
            }),
            Num::Big(x) => Ok(Value::big(x - 1)),
            Num::Float(x) => Ok(Value::Float(x - 1.0)),
        }
    });
    defun(interp, "=", 1, None, |i, a| {
        compare_chain(i, a, |o| o.is_eq())
    });
    defun(interp, "<", 1, None, |i, a| {
        compare_chain(i, a, |o| o.is_lt())
    });
    defun(interp, ">", 1, None, |i, a| {
        compare_chain(i, a, |o| o.is_gt())
    });
    defun(interp, "<=", 1, None, |i, a| {
        compare_chain(i, a, |o| o.is_le())
    });
    defun(interp, ">=", 1, None, |i, a| {
        compare_chain(i, a, |o| o.is_ge())
    });
    defun(interp, "/=", 2, Some(2), |i, a| {
        compare_chain(i, a, |o| o.is_ne())
    });
    defun(interp, "min", 1, None, |i, a| {
        let mut best = need_num(i, &a[0])?;
        for v in &a[1..] {
            let n = need_num(i, v)?;
            if cmp_nums(&n, &best) == Some(std::cmp::Ordering::Less) {
                best = n;
            }
        }
        Ok(num_value(best))
    });
    defun(interp, "max", 1, None, |i, a| {
        let mut best = need_num(i, &a[0])?;
        for v in &a[1..] {
            let n = need_num(i, v)?;
            if cmp_nums(&n, &best) == Some(std::cmp::Ordering::Greater) {
                best = n;
            }
        }
        Ok(num_value(best))
    });
    defun(interp, "abs", 1, Some(1), |i, a| {
        match need_num(i, &a[0])? {
            Num::Int(x) => Ok(match x.checked_abs() {
                Some(r) => Value::Int(r),
                None => Value::big(BigInt::from(x).abs()),
            }),
            Num::Big(x) => Ok(Value::big(x.abs())),
            Num::Float(x) => Ok(Value::Float(x.abs())),
        }
    });
    defun(interp, "float", 1, Some(1), |i, a| {
        Ok(Value::Float(as_f64(&need_num(i, &a[0])?)))
    });
    defun(interp, "floor", 1, Some(1), |i, a| {
        match need_num(i, &a[0])? {
            n @ (Num::Int(_) | Num::Big(_)) => Ok(num_value(n)),
            Num::Float(x) => Ok(Value::Int(x.floor() as i64)),
        }
    });
    defun(interp, "ceiling", 1, Some(1), |i, a| {
        match need_num(i, &a[0])? {
            n @ (Num::Int(_) | Num::Big(_)) => Ok(num_value(n)),
            Num::Float(x) => Ok(Value::Int(x.ceil() as i64)),
        }
    });
    defun(interp, "round", 1, Some(1), |i, a| {
        match need_num(i, &a[0])? {
            n @ (Num::Int(_) | Num::Big(_)) => Ok(num_value(n)),
            Num::Float(x) => Ok(Value::Int(x.round() as i64)),
        }
    });
    defun(interp, "truncate", 1, Some(1), |i, a| {
        match need_num(i, &a[0])? {
            n @ (Num::Int(_) | Num::Big(_)) => Ok(num_value(n)),
            Num::Float(x) => Ok(Value::Int(x.trunc() as i64)),
        }
    });
    defun(interp, "logand", 0, None, |i, a| {
        let mut acc: i64 = -1;
        for v in a.iter() {
            acc &= need_int(i, v)?;
        }
        Ok(Value::Int(acc))
    });
    defun(interp, "logior", 0, None, |i, a| {
        let mut acc: i64 = 0;
        for v in a.iter() {
            acc |= need_int(i, v)?;
        }
        Ok(Value::Int(acc))
    });
    defun(interp, "logxor", 0, None, |i, a| {
        let mut acc: i64 = 0;
        for v in a.iter() {
            acc ^= need_int(i, v)?;
        }
        Ok(Value::Int(acc))
    });
    defun(interp, "ash", 2, Some(2), |i, a| {
        let x = need_int(i, &a[0])?;
        let n = need_int(i, &a[1])?;
        Ok(Value::Int(if n >= 0 {
            x << n.min(62)
        } else {
            x >> (-n).min(62)
        }))
    });
    defun(interp, "number-to-string", 1, Some(1), |i, a| {
        let n = need_num(i, &a[0])?;
        Ok(Value::string(match n {
            Num::Int(x) => x.to_string(),
            Num::Big(x) => x.to_string(),
            Num::Float(x) => crate::printer::prin1_to_string(i, &Value::Float(x)),
        }))
    });
    defun(interp, "string-to-number", 1, Some(1), |i, a| {
        let s = super::need_str(i, &a[0])?;
        let t = s.trim();
        if let Ok(x) = t.parse::<i64>() {
            Ok(Value::Int(x))
        } else if let Ok(b) = t.parse::<BigInt>() {
            // Must try BigInt before f64: f64 parses huge integer
            // strings "successfully" but lossily.
            Ok(Value::big(b))
        } else if let Ok(f) = t.parse::<f64>() {
            Ok(Value::Float(f))
        } else {
            Ok(Value::Int(0))
        }
    });
}
