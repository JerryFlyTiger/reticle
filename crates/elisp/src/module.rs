//! Dynamic module API (M13): load a compiled `.dylib`/`.so` at runtime
//! and let it register native elisp functions and call back into the
//! interpreter. The ABI itself (`module_abi::ModuleEnv` and friends) is
//! modeled on GNU Emacs 25's emacs-module.h -- a mature, already
//! implementation-agnostic design: a versioned table of plain
//! `extern "C" fn` pointers, values exchanged as small opaque handles
//! rather than raw pointers into our own `Value`/`Rc` internals.
//!
//! Known, accepted, documented tradeoff (GNU Emacs accepts the same
//! one): native code loaded this way runs in-process with no memory
//! safety fence around it. A bug in a loaded module can corrupt or crash
//! the whole editor -- unlike M8's worker subprocesses, there is no
//! crash isolation here. That's inherent to "run arbitrary native code
//! in your own address space," not something a function-pointer-table
//! design can paper over; loading a module means trusting it as much as
//! you'd trust any other native code you run.
//!
//! v1 scope cuts from real emacs-module, documented rather than silent:
//! - Only one grammar of error reporting (`signal_error` takes a plain
//!   message, not a condition symbol + data list), and only one pending
//!   error slot -- a module that keeps calling host functions after
//!   triggering an error can clobber that first error with a later one.
//!   Never a memory-safety issue, just a quality-of-message one, and our
//!   own demo module (the only module that exists) never does this.
//! - No `make_global_ref`/`free_global_ref`: every `ModuleValue` a
//!   module receives or creates is only valid for the one call it was
//!   created during (loading the module, or one invocation of a
//!   module-registered function). A module cannot persist an elisp
//!   value across separate calls in v1.
//! - No module unload: a loaded library, and every function pointer a
//!   module registered from it, stays valid for the rest of the process
//!   -- there is no `(module-unload ...)`. This matches how real Emacs's
//!   own module unloading is unreliable in practice, so it costs
//!   nothing to just not offer it.

use std::os::raw::{c_char, c_void};
use std::rc::Rc;

use crate::error::Flow;
use crate::eval::apply_function;
use crate::interp::Interp;
use crate::value::{Args, Function, Value};

pub struct ModuleFunction {
    pub name: String,
    pub min_arity: usize,
    pub max_arity: Option<usize>,
    pub callback: module_abi::ModuleCallback,
    pub data: *mut c_void,
}

/// Per-entry scratch space: every `ModuleValue` created or received
/// during one call into module-land lives here, indexed by
/// `ModuleValue.0`. Dropped when that call returns (see the module doc
/// comment on why there's no global-ref mechanism to outlive it).
struct ModuleCallCtx {
    interp: *mut Interp,
    locals: Vec<Value>,
    pending_error: Option<Flow>,
}

impl ModuleCallCtx {
    fn new(interp: *mut Interp) -> ModuleCallCtx {
        let t = unsafe { (*interp).syms.t };
        ModuleCallCtx {
            interp,
            locals: vec![Value::Nil, Value::Sym(t)],
            pending_error: None,
        }
    }

    fn store(&mut self, v: Value) -> module_abi::ModuleValue {
        let idx = self.locals.len() as u32;
        self.locals.push(v);
        module_abi::ModuleValue(idx)
    }

    fn get(&self, v: module_abi::ModuleValue) -> Value {
        self.locals.get(v.0 as usize).cloned().unwrap_or(Value::Nil)
    }

    /// SAFETY: valid only while this `ModuleCallCtx` is alive on the Rust
    /// stack, which spans exactly one entry into module-land -- every
    /// trampoline function below only ever calls this on a `ctx` pointer
    /// that a currently-executing `call_via_env`/`load` frame owns.
    unsafe fn from_env(env: *mut module_abi::ModuleEnv) -> &'static mut ModuleCallCtx {
        unsafe { &mut *((*env).ctx as *mut ModuleCallCtx) }
    }
}

extern "C" fn t_make_integer(env: *mut module_abi::ModuleEnv, v: i64) -> module_abi::ModuleValue {
    unsafe { ModuleCallCtx::from_env(env) }.store(Value::Int(v))
}

extern "C" fn t_extract_integer(
    env: *mut module_abi::ModuleEnv,
    v: module_abi::ModuleValue,
) -> i64 {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    match ctx.get(v) {
        Value::Int(n) => n,
        Value::Float(f) => f as i64,
        other => {
            let interp = unsafe { &mut *ctx.interp };
            ctx.pending_error = Some(interp.wrong_type("integerp", &other));
            0
        }
    }
}

extern "C" fn t_make_float(env: *mut module_abi::ModuleEnv, v: f64) -> module_abi::ModuleValue {
    unsafe { ModuleCallCtx::from_env(env) }.store(Value::Float(v))
}

extern "C" fn t_extract_float(env: *mut module_abi::ModuleEnv, v: module_abi::ModuleValue) -> f64 {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    match ctx.get(v) {
        Value::Float(f) => f,
        Value::Int(n) => n as f64,
        other => {
            let interp = unsafe { &mut *ctx.interp };
            ctx.pending_error = Some(interp.wrong_type("numberp", &other));
            0.0
        }
    }
}

extern "C" fn t_make_string(
    env: *mut module_abi::ModuleEnv,
    data: *const u8,
    len: usize,
) -> module_abi::ModuleValue {
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    let s = String::from_utf8_lossy(bytes).into_owned();
    unsafe { ModuleCallCtx::from_env(env) }.store(Value::string(s))
}

extern "C" fn t_copy_string_contents(
    env: *mut module_abi::ModuleEnv,
    v: module_abi::ModuleValue,
    buf: *mut u8,
    len: *mut usize,
) -> bool {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    let Value::Str(s) = ctx.get(v) else {
        return false;
    };
    // +1 for the NUL, matching real Emacs's copy_string_contents so a
    // module can treat the buffer as a C string directly.
    let needed = s.len() + 1;
    if buf.is_null() {
        unsafe { *len = needed };
        return true;
    }
    let available = unsafe { *len };
    if available < needed {
        unsafe { *len = needed };
        return false;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
        *buf.add(s.len()) = 0;
        *len = needed;
    }
    true
}

extern "C" fn t_intern(
    env: *mut module_abi::ModuleEnv,
    name: *const c_char,
) -> module_abi::ModuleValue {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    let name = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    let id = unsafe { &mut *ctx.interp }.intern(&name);
    ctx.store(Value::Sym(id))
}

extern "C" fn t_is_not_nil(env: *mut module_abi::ModuleEnv, v: module_abi::ModuleValue) -> bool {
    unsafe { ModuleCallCtx::from_env(env) }.get(v).truthy()
}

extern "C" fn t_eq(
    env: *mut module_abi::ModuleEnv,
    a: module_abi::ModuleValue,
    b: module_abi::ModuleValue,
) -> bool {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    ctx.get(a).eq(&ctx.get(b))
}

extern "C" fn t_funcall(
    env: *mut module_abi::ModuleEnv,
    func: module_abi::ModuleValue,
    nargs: isize,
    args: *const module_abi::ModuleValue,
) -> module_abi::ModuleValue {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    if ctx.pending_error.is_some() {
        return module_abi::NIL;
    }
    let func_val = ctx.get(func);
    let n = nargs.max(0) as usize;
    let handles: &[module_abi::ModuleValue] = if n == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args, n) }
    };
    let call_args: Args = handles.iter().map(|h| ctx.get(*h)).collect();
    let interp = unsafe { &mut *ctx.interp };
    match apply_function(interp, &func_val, call_args) {
        Ok(v) => ctx.store(v),
        Err(flow) => {
            ctx.pending_error = Some(flow);
            module_abi::NIL
        }
    }
}

extern "C" fn t_make_function(
    env: *mut module_abi::ModuleEnv,
    min_arity: isize,
    max_arity: isize,
    func: module_abi::ModuleCallback,
    _doc: *const c_char,
    data: *mut c_void,
) -> module_abi::ModuleValue {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    let mf = ModuleFunction {
        name: String::new(),
        min_arity: min_arity.max(0) as usize,
        max_arity: if max_arity < 0 {
            None
        } else {
            Some(max_arity as usize)
        },
        callback: func,
        data,
    };
    ctx.store(Value::Func(Rc::new(Function::Module(Rc::new(mf)))))
}

extern "C" fn t_signal_error(env: *mut module_abi::ModuleEnv, msg: *const c_char) {
    let ctx = unsafe { ModuleCallCtx::from_env(env) };
    let msg = unsafe { std::ffi::CStr::from_ptr(msg) }
        .to_string_lossy()
        .into_owned();
    let interp = unsafe { &mut *ctx.interp };
    ctx.pending_error = Some(interp.error(msg));
}

fn build_env(ctx: *mut ModuleCallCtx) -> module_abi::ModuleEnv {
    module_abi::ModuleEnv {
        size: std::mem::size_of::<module_abi::ModuleEnv>(),
        ctx: ctx as *mut c_void,
        make_integer: t_make_integer,
        extract_integer: t_extract_integer,
        make_float: t_make_float,
        extract_float: t_extract_float,
        make_string: t_make_string,
        copy_string_contents: t_copy_string_contents,
        intern: t_intern,
        is_not_nil: t_is_not_nil,
        eq: t_eq,
        funcall: t_funcall,
        make_function: t_make_function,
        signal_error: t_signal_error,
    }
}

/// Call a module-registered function (the `Function::Module` arm of
/// `apply_function`). Builds a fresh per-call context, converts `args`
/// into module handles, invokes the module's raw callback, and converts
/// the result (or a signaled error) back.
pub fn call_module_function(
    interp: &mut Interp,
    mf: &ModuleFunction,
    args: Args,
) -> Result<Value, Flow> {
    let n = args.len();
    if n < mf.min_arity || mf.max_arity.is_some_and(|max| n > max) {
        let e = interp.syms.wrong_number_of_arguments;
        let name = Value::string(mf.name.clone());
        return Err(interp.signal(e, vec![name, Value::Int(n as i64)]));
    }
    let mut ctx = ModuleCallCtx::new(interp as *mut Interp);
    let handles: Vec<module_abi::ModuleValue> = args.iter().map(|v| ctx.store(v.clone())).collect();
    let mut env = build_env(&mut ctx as *mut ModuleCallCtx);
    let result = (mf.callback)(
        &mut env as *mut module_abi::ModuleEnv,
        handles.len() as isize,
        handles.as_ptr(),
        mf.data,
    );
    if let Some(flow) = ctx.pending_error.take() {
        return Err(flow);
    }
    Ok(ctx.get(result))
}

fn load_module(interp: &mut Interp, path: &str) -> Result<(), Flow> {
    // SAFETY: dlopen runs the library's static initializers; this is
    // inherent to loading arbitrary native code and is exactly the
    // documented tradeoff of this feature (see the module doc comment).
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| interp.error(format!("module-load: cannot open {}: {}", path, e)))?;
    // SAFETY: we trust the symbol's declared signature matches
    // `module_abi::InitFn` exactly -- the same trust boundary as calling
    // any C function via dlsym.
    let init: libloading::Symbol<module_abi::InitFn> = unsafe { lib.get(module_abi::INIT_FN_NAME) }
        .map_err(|e| {
            interp.error(format!(
                "module-load: {} has no emacs_module_init: {}",
                path, e
            ))
        })?;

    let mut ctx = ModuleCallCtx::new(interp as *mut Interp);
    let mut env = build_env(&mut ctx as *mut ModuleCallCtx);
    let rc = unsafe { init(&mut env as *mut module_abi::ModuleEnv) };
    if let Some(flow) = ctx.pending_error.take() {
        return Err(flow);
    }
    if rc != 0 {
        return Err(interp.error(format!(
            "module-load: {} emacs_module_init returned {}",
            path, rc
        )));
    }

    // No module-unload in v1 (see module doc comment) -- the library,
    // and every function pointer any module-registered Function::Module
    // holds into it, must stay valid for the rest of the process, so we
    // deliberately leak the handle rather than let it drop here.
    std::mem::forget(lib);
    Ok(())
}

pub fn register(interp: &mut Interp) {
    crate::builtins::defun(interp, "module-load", 1, Some(1), |i, a| {
        let path = crate::builtins::need_str(i, &a[0])?;
        load_module(i, &path)?;
        Ok(Value::Nil)
    });
}
