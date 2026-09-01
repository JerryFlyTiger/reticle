//! JSON <-> elisp value conversion (M14, first building block the LSP
//! client needs). Mapping mirrors modern GNU Emacs's own `json-parse-
//! string`/`json-serialize` defaults exactly, to sidestep elisp's nil/
//! false/empty-list ambiguity the same way real Emacs does:
//!
//!   JSON null  <-> the keyword `:null`
//!   JSON false <-> the keyword `:false`
//!   JSON true  <-> `t`
//!   JSON number -> `Int` if it fits an i64 exactly, `Float` otherwise
//!   JSON string <-> `Str`
//!   JSON array  <-> `Vector` (a list also serializes as an array, for
//!                   convenience building payloads with `list`)
//!   JSON object <-> `HashTable` keyed by string (matches real Emacs's
//!                   `:object-type 'hash-table`, the modern default)
//!
//! v1 scope cut, documented rather than silent: bare `nil` serializes as
//! `null` (not real Emacs's stricter, version-dependent handling of nil
//! as a list/object/false depending on context) -- callers that mean
//! JSON false or an empty object must say so explicitly with `:false` or
//! an empty hash-table. Every JSON value this crate's own LSP client
//! constructs does so explicitly, so this never bites us in practice.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value as Json;

use crate::interp::Interp;
use crate::value::{HKey, Value};

fn null_sym(interp: &mut Interp) -> u32 {
    interp.intern(":null")
}

fn false_sym(interp: &mut Interp) -> u32 {
    interp.intern(":false")
}

pub fn to_json(interp: &mut Interp, v: &Value) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Int(n) => Json::Number((*n).into()),
        // No arbitrary-precision JSON numbers without pulling in serde_json's
        // arbitrary_precision feature; a string keeps this lossless and
        // clearly distinguishable, at the cost of not being a JSON number.
        // LSP itself never sends/receives values this large (positions,
        // ids, line/column counts are always small integers).
        Value::Big(b) => Json::String(b.to_string()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Str(s) => Json::String(s.to_string()),
        Value::Sym(id) => {
            if *id == interp.syms.t {
                Json::Bool(true)
            } else if *id == null_sym(interp) {
                Json::Null
            } else if *id == false_sym(interp) {
                Json::Bool(false)
            } else {
                // Fallback for an ordinary symbol handed to json-serialize
                // by mistake: its print name, rather than silently
                // dropping data.
                Json::String(interp.sym_name(*id).to_string())
            }
        }
        Value::Cons(_) => {
            let items = v.list_to_vec().unwrap_or_default();
            Json::Array(items.iter().map(|x| to_json(interp, x)).collect())
        }
        Value::Vector(vec) => {
            Json::Array(vec.borrow().iter().map(|x| to_json(interp, x)).collect())
        }
        Value::HashTable(h) => {
            let mut map = serde_json::Map::new();
            for (k, val) in h.borrow().iter() {
                let key = match &k.0 {
                    Value::Str(s) => s.to_string(),
                    Value::Sym(id) => interp.sym_name(*id).to_string(),
                    other => crate::printer::princ_to_string(interp, other),
                };
                map.insert(key, to_json(interp, val));
            }
            Json::Object(map)
        }
        // Functions and editor-native handles have no JSON representation.
        Value::Func(_) | Value::Ext(_) => Json::Null,
    }
}

pub fn from_json(interp: &mut Interp, j: &Json) -> Value {
    match j {
        Json::Null => Value::Sym(null_sym(interp)),
        Json::Bool(true) => Value::Sym(interp.syms.t),
        Json::Bool(false) => Value::Sym(false_sym(interp)),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::string(s.clone()),
        Json::Array(items) => {
            let v: Vec<Value> = items.iter().map(|x| from_json(interp, x)).collect();
            Value::Vector(Rc::new(RefCell::new(v)))
        }
        Json::Object(map) => {
            let mut table = indexmap::IndexMap::new();
            for (k, val) in map {
                table.insert(HKey(Value::string(k.clone())), from_json(interp, val));
            }
            Value::HashTable(Rc::new(RefCell::new(table)))
        }
    }
}

pub fn register(interp: &mut Interp) {
    crate::builtins::defun(interp, "json-serialize", 1, Some(1), |i, a| {
        let j = to_json(i, &a[0]);
        Ok(Value::string(j.to_string()))
    });
    crate::builtins::defun(interp, "json-parse-string", 1, Some(1), |i, a| {
        let s = crate::builtins::need_str(i, &a[0])?;
        let j: Json =
            serde_json::from_str(&s).map_err(|e| i.error(format!("json-parse-string: {}", e)))?;
        Ok(from_json(i, &j))
    });
}
