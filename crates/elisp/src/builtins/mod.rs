mod data;
mod math;
mod misc;

use std::rc::Rc;

use crate::error::Flow;
use crate::interp::Interp;
use crate::value::{Function, SymId, Value};

pub fn register_all(interp: &mut Interp) {
    data::register(interp);
    math::register(interp);
    misc::register(interp);
    crate::worker::register(interp);
    crate::module::register(interp);
    crate::json::register(interp);
    crate::lsp::register(interp);
    crate::shell::register(interp);
    crate::bglog::register(interp);
}

pub fn defun(
    interp: &mut Interp,
    name: &'static str,
    min_args: usize,
    max_args: Option<usize>,
    f: fn(&mut Interp, &mut [Value]) -> Result<Value, Flow>,
) {
    let id = interp.intern(name);
    interp.symbols[id as usize].function = Some(Value::Func(Rc::new(Function::Builtin {
        name,
        min_args,
        max_args,
        f,
    })));
}

pub fn need_int(interp: &mut Interp, v: &Value) -> Result<i64, Flow> {
    match v {
        Value::Int(i) => Ok(*i),
        _ => Err(interp.wrong_type("integerp", v)),
    }
}

pub fn need_str(interp: &mut Interp, v: &Value) -> Result<Rc<String>, Flow> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(interp.wrong_type("stringp", v)),
    }
}

pub fn need_sym(interp: &mut Interp, v: &Value) -> Result<SymId, Flow> {
    interp
        .as_sym(v)
        .ok_or_else(|| interp.wrong_type("symbolp", v))
}

pub fn need_list(interp: &mut Interp, v: &Value) -> Result<Vec<Value>, Flow> {
    v.list_to_vec().ok_or_else(|| interp.wrong_type("listp", v))
}

pub fn opt(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Nil)
}
