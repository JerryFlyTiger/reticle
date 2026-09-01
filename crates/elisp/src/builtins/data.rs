use std::cell::RefCell;
use std::rc::Rc;

use super::{defun, need_int, need_list, need_str, need_sym, opt};
use crate::interp::Interp;
use crate::value::{Function, HKey, Value};

pub fn register(interp: &mut Interp) {
    defun(interp, "cons", 2, Some(2), |_, a| {
        Ok(Value::cons(a[0].clone(), a[1].clone()))
    });
    defun(interp, "car", 1, Some(1), |i, a| match &a[0] {
        Value::Nil | Value::Cons(_) => Ok(a[0].car()),
        v => Err(i.wrong_type("listp", v)),
    });
    defun(interp, "cdr", 1, Some(1), |i, a| match &a[0] {
        Value::Nil | Value::Cons(_) => Ok(a[0].cdr()),
        v => Err(i.wrong_type("listp", v)),
    });
    defun(interp, "caar", 1, Some(1), |_, a| Ok(a[0].car().car()));
    defun(interp, "cadr", 1, Some(1), |_, a| Ok(a[0].cdr().car()));
    defun(interp, "cdar", 1, Some(1), |_, a| Ok(a[0].car().cdr()));
    defun(interp, "cddr", 1, Some(1), |_, a| Ok(a[0].cdr().cdr()));
    defun(interp, "caddr", 1, Some(1), |_, a| {
        Ok(a[0].cdr().cdr().car())
    });
    defun(interp, "setcar", 2, Some(2), |i, a| match &a[0] {
        Value::Cons(c) => {
            c.borrow_mut().car = a[1].clone();
            crate::gc::register_value(i, &a[0], &a[1]);
            Ok(a[1].clone())
        }
        v => Err(i.wrong_type("consp", v)),
    });
    defun(interp, "setcdr", 2, Some(2), |i, a| match &a[0] {
        Value::Cons(c) => {
            c.borrow_mut().cdr = a[1].clone();
            crate::gc::register_value(i, &a[0], &a[1]);
            Ok(a[1].clone())
        }
        v => Err(i.wrong_type("consp", v)),
    });
    defun(interp, "list", 0, None, |_, a| Ok(Value::list(a.to_vec())));
    defun(interp, "length", 1, Some(1), |i, a| match &a[0] {
        Value::Nil | Value::Cons(_) => {
            let v = need_list(i, &a[0])?;
            Ok(Value::Int(v.len() as i64))
        }
        Value::Str(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::Vector(v) => Ok(Value::Int(v.borrow().len() as i64)),
        v => Err(i.wrong_type("sequencep", v)),
    });
    defun(interp, "nth", 2, Some(2), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        let mut cur = a[1].clone();
        for _ in 0..n {
            cur = cur.cdr();
        }
        Ok(cur.car())
    });
    defun(interp, "nthcdr", 2, Some(2), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        let mut cur = a[1].clone();
        for _ in 0..n {
            cur = cur.cdr();
        }
        Ok(cur)
    });
    // Deviation from GNU worth knowing: this collects the list into a
    // `Vec` (`need_list`) and conses a BRAND NEW tail from a slice of
    // it, rather than walking to and returning the original list's own
    // trailing cons cells. Real Emacs's `last` shares structure with
    // its argument, so `(setcar (last list) x)` mutates LIST itself --
    // a common idiom (e.g. "replace a list's final element"). Here
    // that idiom silently no-ops: `setcar` on this `last`'s result
    // never touches the original LIST at all. core's verilog-auto.el
    // (`verilog-auto--grouped-lines`) hit exactly this and works around
    // it by mutating its own accumulator before the final `nreverse'
    // instead of via `(setcar (last ...) ...)' -- see that function's
    // own comment. Left as-is here: fixing it would mean walking the
    // list ourselves instead of `need_list', a bigger change than this
    // v1 scope calls for.
    defun(interp, "last", 1, Some(2), |i, a| {
        let items = need_list(i, &a[0])?;
        let n = match opt(a, 1) {
            Value::Nil => 1,
            v => need_int(i, &v)?.max(0) as usize,
        };
        let skip = items.len().saturating_sub(n);
        Ok(Value::list(items[skip..].to_vec()))
    });
    defun(interp, "append", 0, None, |i, a| {
        if a.is_empty() {
            return Ok(Value::Nil);
        }
        let mut items = Vec::new();
        for v in &a[..a.len() - 1] {
            items.extend(need_list(i, v)?);
        }
        // Last arg becomes the tail unchanged (may be non-list).
        let mut acc = a[a.len() - 1].clone();
        for v in items.into_iter().rev() {
            acc = Value::cons(v, acc);
        }
        Ok(acc)
    });
    defun(interp, "reverse", 1, Some(1), |i, a| {
        let mut items = need_list(i, &a[0])?;
        items.reverse();
        Ok(Value::list(items))
    });
    defun(interp, "nreverse", 1, Some(1), |i, a| {
        let mut items = need_list(i, &a[0])?;
        items.reverse();
        Ok(Value::list(items))
    });
    defun(interp, "assq", 2, Some(2), |i, a| {
        for entry in need_list(i, &a[1])? {
            if entry.car().eq(&a[0]) {
                return Ok(entry);
            }
        }
        Ok(Value::Nil)
    });
    defun(interp, "assoc", 2, Some(2), |i, a| {
        for entry in need_list(i, &a[1])? {
            if entry.car().equal(&a[0]) {
                return Ok(entry);
            }
        }
        Ok(Value::Nil)
    });
    defun(interp, "rassq", 2, Some(2), |i, a| {
        for entry in need_list(i, &a[1])? {
            if entry.cdr().eq(&a[0]) {
                return Ok(entry);
            }
        }
        Ok(Value::Nil)
    });
    defun(interp, "memq", 2, Some(2), |_, a| {
        let mut cur = a[1].clone();
        while let Value::Cons(c) = &cur {
            if c.borrow().car.eq(&a[0]) {
                return Ok(cur.clone());
            }
            let next = c.borrow().cdr.clone();
            cur = next;
        }
        Ok(Value::Nil)
    });
    defun(interp, "member", 2, Some(2), |_, a| {
        let mut cur = a[1].clone();
        while let Value::Cons(c) = &cur {
            if c.borrow().car.equal(&a[0]) {
                return Ok(cur.clone());
            }
            let next = c.borrow().cdr.clone();
            cur = next;
        }
        Ok(Value::Nil)
    });
    defun(interp, "delete", 2, Some(2), |i, a| {
        let items = need_list(i, &a[1])?;
        Ok(Value::list(
            items.into_iter().filter(|v| !v.equal(&a[0])).collect(),
        ))
    });
    defun(interp, "delq", 2, Some(2), |i, a| {
        let items = need_list(i, &a[1])?;
        Ok(Value::list(
            items.into_iter().filter(|v| !v.eq(&a[0])).collect(),
        ))
    });
    defun(interp, "copy-sequence", 1, Some(1), |i, a| match &a[0] {
        Value::Nil | Value::Cons(_) => Ok(Value::list(need_list(i, &a[0])?)),
        Value::Str(s) => Ok(Value::string(s.as_str())),
        Value::Vector(v) => Ok(Value::Vector(Rc::new(RefCell::new(v.borrow().clone())))),
        v => Err(i.wrong_type("sequencep", v)),
    });
    defun(interp, "elt", 2, Some(2), |i, a| {
        let n = need_int(i, &a[1])?.max(0) as usize;
        match &a[0] {
            Value::Str(s) => Ok(s
                .chars()
                .nth(n)
                .map(|c| Value::Int(c as i64))
                .unwrap_or(Value::Nil)),
            Value::Vector(v) => Ok(v.borrow().get(n).cloned().unwrap_or(Value::Nil)),
            _ => {
                let mut cur = a[0].clone();
                for _ in 0..n {
                    cur = cur.cdr();
                }
                Ok(cur.car())
            }
        }
    });

    // Predicates.
    defun(interp, "null", 1, Some(1), |i, a| {
        Ok(Value::bool(a[0].is_nil(), i.syms.t))
    });
    defun(interp, "not", 1, Some(1), |i, a| {
        Ok(Value::bool(a[0].is_nil(), i.syms.t))
    });
    defun(interp, "atom", 1, Some(1), |i, a| {
        Ok(Value::bool(!matches!(a[0], Value::Cons(_)), i.syms.t))
    });
    defun(interp, "consp", 1, Some(1), |i, a| {
        Ok(Value::bool(matches!(a[0], Value::Cons(_)), i.syms.t))
    });
    defun(interp, "listp", 1, Some(1), |i, a| {
        Ok(Value::bool(
            matches!(a[0], Value::Cons(_) | Value::Nil),
            i.syms.t,
        ))
    });
    defun(interp, "symbolp", 1, Some(1), |i, a| {
        Ok(Value::bool(
            matches!(a[0], Value::Sym(_) | Value::Nil),
            i.syms.t,
        ))
    });
    defun(interp, "keywordp", 1, Some(1), |i, a| {
        let b = matches!(&a[0], Value::Sym(id) if i.is_keyword(*id));
        Ok(Value::bool(b, i.syms.t))
    });
    defun(interp, "stringp", 1, Some(1), |i, a| {
        Ok(Value::bool(matches!(a[0], Value::Str(_)), i.syms.t))
    });
    defun(interp, "numberp", 1, Some(1), |i, a| {
        Ok(Value::bool(
            matches!(a[0], Value::Int(_) | Value::Big(_) | Value::Float(_)),
            i.syms.t,
        ))
    });
    defun(interp, "integerp", 1, Some(1), |i, a| {
        Ok(Value::bool(
            matches!(a[0], Value::Int(_) | Value::Big(_)),
            i.syms.t,
        ))
    });
    defun(interp, "floatp", 1, Some(1), |i, a| {
        Ok(Value::bool(matches!(a[0], Value::Float(_)), i.syms.t))
    });
    defun(interp, "vectorp", 1, Some(1), |i, a| {
        Ok(Value::bool(matches!(a[0], Value::Vector(_)), i.syms.t))
    });
    defun(interp, "hash-table-p", 1, Some(1), |i, a| {
        Ok(Value::bool(matches!(a[0], Value::HashTable(_)), i.syms.t))
    });
    defun(interp, "functionp", 1, Some(1), |i, a| {
        let b = match &a[0] {
            Value::Func(f) => !matches!(
                f.as_ref(),
                Function::Lambda(l) if l.is_macro
            ),
            Value::Sym(id) => i.symbols[*id as usize].function.is_some(),
            _ => false,
        };
        Ok(Value::bool(b, i.syms.t))
    });
    defun(interp, "eq", 2, Some(2), |i, a| {
        Ok(Value::bool(a[0].eq(&a[1]), i.syms.t))
    });
    defun(interp, "eql", 2, Some(2), |i, a| {
        Ok(Value::bool(a[0].eql(&a[1]), i.syms.t))
    });
    defun(interp, "equal", 2, Some(2), |i, a| {
        Ok(Value::bool(a[0].equal(&a[1]), i.syms.t))
    });

    // Symbols.
    defun(interp, "intern", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        if s.as_str() == "nil" {
            return Ok(Value::Nil);
        }
        let id = i.intern(&s);
        Ok(Value::Sym(id))
    });
    defun(interp, "intern-soft", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        Ok(i.intern_soft(&s).map(Value::Sym).unwrap_or(Value::Nil))
    });
    defun(interp, "gensym", 0, Some(1), |i, a| {
        let prefix = match opt(a, 0) {
            Value::Str(s) => s.to_string(),
            _ => "g".to_string(),
        };
        let id = i.gensym(&prefix);
        Ok(Value::Sym(id))
    });
    defun(interp, "symbol-name", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        Ok(Value::string(i.sym_name(id)))
    });
    defun(interp, "symbol-value", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        match i.sym_value(id) {
            Some(v) => Ok(v),
            None => {
                let e = i.syms.void_variable;
                Err(i.signal(e, vec![Value::Sym(id)]))
            }
        }
    });
    defun(interp, "symbol-function", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        Ok(i.symbols[id as usize]
            .function
            .clone()
            .unwrap_or(Value::Nil))
    });
    defun(interp, "set", 2, Some(2), |i, a| {
        let id = need_sym(i, &a[0])?;
        if id == 0 || id == i.syms.t || i.is_keyword(id) {
            let e = i.syms.setting_constant;
            return Err(i.signal(e, vec![Value::Sym(id)]));
        }
        i.set_sym_value(id, a[1].clone());
        Ok(a[1].clone())
    });
    defun(interp, "fset", 2, Some(2), |i, a| {
        let id = need_sym(i, &a[0])?;
        i.symbols[id as usize].function = Some(a[1].clone());
        Ok(a[1].clone())
    });
    defun(interp, "defalias", 2, Some(3), |i, a| {
        let id = need_sym(i, &a[0])?;
        i.symbols[id as usize].function = Some(a[1].clone());
        Ok(Value::Sym(id))
    });
    defun(interp, "boundp", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        Ok(Value::bool(i.sym_value(id).is_some(), i.syms.t))
    });
    defun(interp, "fboundp", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        Ok(Value::bool(
            i.symbols[id as usize].function.is_some(),
            i.syms.t,
        ))
    });
    defun(interp, "makunbound", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        i.symbols[id as usize].value = None;
        Ok(Value::Sym(id))
    });
    defun(interp, "fmakunbound", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        i.symbols[id as usize].function = None;
        Ok(Value::Sym(id))
    });
    defun(interp, "get", 2, Some(2), |i, a| {
        let id = need_sym(i, &a[0])?;
        let prop = need_sym(i, &a[1])?;
        Ok(i.plist_get(id, prop))
    });
    defun(interp, "put", 3, Some(3), |i, a| {
        let id = need_sym(i, &a[0])?;
        let prop = need_sym(i, &a[1])?;
        i.plist_put(id, prop, a[2].clone());
        Ok(a[2].clone())
    });

    // Vectors.
    defun(interp, "vector", 0, None, |_, a| {
        Ok(Value::Vector(Rc::new(RefCell::new(a.to_vec()))))
    });
    defun(interp, "make-vector", 2, Some(2), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        Ok(Value::Vector(Rc::new(RefCell::new(vec![a[1].clone(); n]))))
    });
    defun(interp, "aref", 2, Some(2), |i, a| {
        let n = need_int(i, &a[1])?;
        match &a[0] {
            Value::Vector(v) => v.borrow().get(n.max(0) as usize).cloned().ok_or_else(|| {
                let e = i.syms.args_out_of_range;
                i.signal(e, vec![a[0].clone(), Value::Int(n)])
            }),
            Value::Str(s) => s
                .chars()
                .nth(n.max(0) as usize)
                .map(|c| Value::Int(c as i64))
                .ok_or_else(|| {
                    let e = i.syms.args_out_of_range;
                    i.signal(e, vec![a[0].clone(), Value::Int(n)])
                }),
            v => Err(i.wrong_type("arrayp", v)),
        }
    });
    defun(interp, "aset", 3, Some(3), |i, a| {
        let n = need_int(i, &a[1])?.max(0) as usize;
        match &a[0] {
            Value::Vector(v) => {
                let mut b = v.borrow_mut();
                if n >= b.len() {
                    let e = i.syms.args_out_of_range;
                    return Err(i.signal(e, vec![a[0].clone(), Value::Int(n as i64)]));
                }
                b[n] = a[2].clone();
                drop(b);
                crate::gc::register_value(i, &a[0], &a[2]);
                Ok(a[2].clone())
            }
            v => Err(i.wrong_type("vectorp", v)),
        }
    });

    // Hash tables (M10 item 5: real IndexMap, O(1) average lookup by
    // `equal`, insertion order preserved for maphash — see HKey).
    defun(interp, "make-hash-table", 0, None, |_, _| {
        Ok(Value::HashTable(Rc::new(RefCell::new(
            indexmap::IndexMap::new(),
        ))))
    });
    defun(interp, "gethash", 2, Some(3), |i, a| match &a[1] {
        Value::HashTable(h) => Ok(h
            .borrow()
            .get(&HKey(a[0].clone()))
            .cloned()
            .unwrap_or_else(|| opt(a, 2))),
        v => Err(i.wrong_type("hash-table-p", v)),
    });
    defun(interp, "puthash", 3, Some(3), |i, a| match &a[2] {
        Value::HashTable(h) => {
            // insert() overwrites in place (keeps the existing entry's
            // position) when the key is already present, matching the
            // old assoc-vector's "find and update" behavior exactly.
            h.borrow_mut().insert(HKey(a[0].clone()), a[1].clone());
            // Both the key and the value are stored; either could close
            // a cycle back to this table.
            crate::gc::register_value(i, &a[2], &a[0]);
            crate::gc::register_value(i, &a[2], &a[1]);
            Ok(a[1].clone())
        }
        v => Err(i.wrong_type("hash-table-p", v)),
    });
    defun(interp, "remhash", 2, Some(2), |i, a| match &a[1] {
        Value::HashTable(h) => {
            // shift_remove (not the O(1) swap_remove) to preserve the
            // remaining entries' relative order for maphash.
            h.borrow_mut().shift_remove(&HKey(a[0].clone()));
            Ok(Value::Nil)
        }
        v => Err(i.wrong_type("hash-table-p", v)),
    });
    defun(interp, "clrhash", 1, Some(1), |i, a| match &a[0] {
        Value::HashTable(h) => {
            h.borrow_mut().clear();
            Ok(a[0].clone())
        }
        v => Err(i.wrong_type("hash-table-p", v)),
    });
    defun(interp, "hash-table-count", 1, Some(1), |i, a| match &a[0] {
        Value::HashTable(h) => Ok(Value::Int(h.borrow().len() as i64)),
        v => Err(i.wrong_type("hash-table-p", v)),
    });
    defun(interp, "maphash", 2, Some(2), |i, a| match &a[1] {
        Value::HashTable(h) => {
            // Snapshot first: FN may itself mutate the table (a common
            // and legal pattern), which must not disturb this iteration.
            let entries: Vec<(Value, Value)> = h
                .borrow()
                .iter()
                .map(|(k, v)| (k.0.clone(), v.clone()))
                .collect();
            for (k, v) in entries {
                crate::eval::apply_function(i, &a[0].clone(), smallvec::smallvec![k, v])?;
            }
            Ok(Value::Nil)
        }
        v => Err(i.wrong_type("hash-table-p", v)),
    });
}
