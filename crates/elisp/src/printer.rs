use crate::interp::Interp;
use crate::value::{Function, Value};

const MAX_DEPTH: usize = 200;
const MAX_LENGTH: usize = 10_000;

/// prin1: machine-readable, strings escaped.
pub fn prin1_to_string(interp: &Interp, v: &Value) -> String {
    let mut s = String::new();
    write_value(interp, v, true, 0, &mut s);
    s
}

/// princ: human-readable, strings raw.
pub fn princ_to_string(interp: &Interp, v: &Value) -> String {
    let mut s = String::new();
    write_value(interp, v, false, 0, &mut s);
    s
}

fn write_value(interp: &Interp, v: &Value, escape: bool, depth: usize, out: &mut String) {
    if depth > MAX_DEPTH || out.len() > MAX_LENGTH {
        out.push_str("...");
        return;
    }
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Big(b) => out.push_str(&b.to_string()),
        Value::Float(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                out.push_str(&format!("{:.1}", f));
            } else {
                out.push_str(&f.to_string());
            }
        }
        Value::Str(s) => {
            if escape {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        _ => out.push(c),
                    }
                }
                out.push('"');
            } else {
                out.push_str(s);
            }
        }
        Value::Sym(id) => out.push_str(interp.sym_name(*id)),
        Value::Cons(_) => write_list(interp, v, escape, depth, out),
        Value::Vector(items) => {
            out.push('[');
            for (i, item) in items.borrow().iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_value(interp, item, escape, depth + 1, out);
            }
            out.push(']');
        }
        Value::HashTable(h) => {
            out.push_str(&format!("#<hash-table :count {}>", h.borrow().len()));
        }
        Value::Ext(e) => out.push_str(&format!("#<{}>", e.tag)),
        Value::Func(f) => match f.as_ref() {
            Function::Builtin { name, .. } => out.push_str(&format!("#<subr {}>", name)),
            Function::Lambda(l) => {
                let name = l
                    .name
                    .borrow()
                    .map(|id| interp.sym_name(id).to_string())
                    .unwrap_or_else(|| "anonymous".into());
                let kind = if l.is_macro { "macro" } else { "lambda" };
                out.push_str(&format!("#<{} {}>", kind, name));
            }
            Function::Compiled(c) => {
                let name = c
                    .name
                    .borrow()
                    .map(|id| interp.sym_name(id).to_string())
                    .unwrap_or_else(|| "anonymous".into());
                out.push_str(&format!("#<compiled-function {}>", name));
            }
            Function::Native(n) => {
                let name = n
                    .fallback
                    .name
                    .borrow()
                    .map(|id| interp.sym_name(id).to_string())
                    .unwrap_or_else(|| "anonymous".into());
                out.push_str(&format!("#<native-function {}>", name));
            }
            Function::Module(m) => {
                if m.name.is_empty() {
                    out.push_str("#<module-function>");
                } else {
                    out.push_str(&format!("#<module-function {}>", m.name));
                }
            }
        },
    }
}

fn write_list(interp: &Interp, v: &Value, escape: bool, depth: usize, out: &mut String) {
    // Sugar for (quote x) and (function x).
    if let Value::Cons(c) = v {
        let b = c.borrow();
        if let Value::Sym(id) = b.car {
            if let Value::Cons(rest) = &b.cdr {
                let rb = rest.borrow();
                if rb.cdr.is_nil() {
                    let prefix = if id == interp.syms.quote {
                        Some("'")
                    } else if id == interp.syms.function {
                        Some("#'")
                    } else if id == interp.syms.backquote {
                        Some("`")
                    } else if id == interp.syms.unquote {
                        Some(",")
                    } else if id == interp.syms.unquote_splicing {
                        Some(",@")
                    } else {
                        None
                    };
                    if let Some(p) = prefix {
                        out.push_str(p);
                        write_value(interp, &rb.car, escape, depth + 1, out);
                        return;
                    }
                }
            }
        }
    }
    out.push('(');
    let mut cur = v.clone();
    let mut count = 0usize;
    loop {
        match cur {
            Value::Cons(c) => {
                if count > 0 {
                    out.push(' ');
                }
                if count > MAX_LENGTH {
                    out.push_str("...");
                    break;
                }
                let b = c.borrow();
                write_value(interp, &b.car, escape, depth + 1, out);
                cur = b.cdr.clone();
                count += 1;
            }
            Value::Nil => break,
            other => {
                out.push_str(" . ");
                write_value(interp, &other, escape, depth + 1, out);
                break;
            }
        }
    }
    out.push(')');
}
