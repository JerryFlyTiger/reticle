mod buffers;
mod editing;
mod files;
mod treesit;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use elisp::error::Flow;
use elisp::{Interp, Value};

use crate::buffer::{Buffer, MarkerData};
use crate::editor::Editor;

pub fn register_all(interp: &mut Interp) {
    buffers::register(interp);
    editing::register(interp);
    files::register(interp);
    treesit::register(interp);
    ui::register(interp);
}

/// M19: refuse interp-visible mutation of a read-only buffer unless
/// `inhibit-read-only` is bound non-nil (the GNU idiom dired uses to
/// write its own listing).
pub(crate) fn check_writable(interp: &mut Interp, b: &Rc<RefCell<Buffer>>) -> Result<(), Flow> {
    if !b.borrow().read_only {
        return Ok(());
    }
    let inhibit = interp
        .intern_soft("inhibit-read-only")
        .and_then(|id| interp.sym_value(id))
        .map(|v| v.truthy())
        .unwrap_or(false);
    if inhibit {
        return Ok(());
    }
    let name = b.borrow().name.clone();
    Err(interp.error(format!("Buffer is read-only: {}", name)))
}

pub(crate) fn ed_handle(interp: &Interp) -> Rc<RefCell<Editor>> {
    crate::editor::editor(interp)
}

pub(crate) fn cur(interp: &Interp) -> Rc<RefCell<Buffer>> {
    ed_handle(interp).borrow().current.clone()
}

pub(crate) const MARKER_TAG: &str = "marker";
pub(crate) const OVERLAY_TAG: &str = "overlay";

/// Convert an elisp position (1-based int, marker, or nil = point) to a
/// clamped 0-based char offset in `b`.
pub(crate) fn get_pos(interp: &mut Interp, b: &Buffer, v: &Value) -> Result<usize, Flow> {
    match v {
        Value::Nil => Ok(b.point),
        Value::Int(i) => Ok(b.clamp(i - 1)),
        Value::Ext(_) => {
            if let Some(m) = v.as_ext::<RefCell<MarkerData>>(MARKER_TAG) {
                Ok(b.clamp(m.borrow().pos as i64))
            } else {
                Err(interp.wrong_type("integer-or-marker-p", v))
            }
        }
        _ => Err(interp.wrong_type("integer-or-marker-p", v)),
    }
}

/// Resolve a buffer designator: nil = current, buffer object, or name string.
pub(crate) fn buffer_arg(interp: &mut Interp, v: &Value) -> Result<Rc<RefCell<Buffer>>, Flow> {
    match v {
        Value::Nil => Ok(cur(interp)),
        Value::Ext(_) => v
            .as_ext::<RefCell<Buffer>>(crate::editor::BUFFER_TAG)
            .ok_or_else(|| interp.wrong_type("bufferp", v)),
        Value::Str(name) => {
            let name = name.to_string();
            ed_handle(interp)
                .borrow()
                .find_buffer(&name)
                .ok_or_else(|| interp.error(format!("No buffer named {}", name)))
        }
        _ => Err(interp.wrong_type("bufferp", v)),
    }
}

pub(crate) fn int_pos(pos: usize) -> Value {
    Value::Int(pos as i64 + 1)
}
