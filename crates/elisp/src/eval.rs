use std::cell::RefCell;
use std::rc::Rc;

use crate::error::{EvalResult, Flow};
use crate::interp::Interp;
use crate::value::{Args, Function, Lambda, LexEnv, ParamSpec, SymId, Value};

pub(crate) type Env = Option<Rc<LexEnv>>;

/// Length of `list` if it is a proper (nil-terminated, acyclic) list;
/// None otherwise. A pure pointer walk — no element value is cloned and
/// no Vec is built. Used where a form must be validated before any of
/// it is evaluated (`let` bindings). The step cap matches
/// `Value::list_to_vec`'s guard.
fn proper_list_len(list: &Value) -> Option<usize> {
    let mut len = 0usize;
    let mut cur = list.clone();
    loop {
        match cur {
            Value::Nil => return Some(len),
            Value::Cons(c) => {
                len += 1;
                if len > 1_000_000 {
                    return None;
                }
                let next = c.borrow().cdr.clone();
                cur = next;
            }
            _ => return None,
        }
    }
}

/// Streaming walk over a list's elements: yields each car in order with
/// no backing Vec (P2.3 — the old `list_to_vec` collect allocated a
/// heap Vec on every progn/arg-list/setq/and/or/cond evaluation, which
/// profiling showed dominated tree-walker cost in tight loops).
///
/// The walk ends at the first non-cons tail; callers check `at_nil`
/// afterwards and report a malformed form if the tail wasn't nil. This
/// means a statically dotted form (`(progn 1 . 2)`) now errors AFTER
/// the forms before the bad tail were evaluated, where the collect-
/// first versions errored before evaluating anything — the trade
/// documented for P2.3, matching how GNU Emacs's eval_sub streams
/// special-form bodies. Walking the live cells (not a snapshot) is
/// likewise GNU behavior.
struct FormIter {
    cur: Value,
}

impl FormIter {
    #[inline]
    fn new(list: &Value) -> FormIter {
        FormIter { cur: list.clone() }
    }

    /// True when the walk consumed the whole list down to nil (callers
    /// use this to distinguish "done" from "stopped at a dotted tail").
    #[inline]
    fn at_nil(&self) -> bool {
        self.cur.is_nil()
    }
}

impl Iterator for FormIter {
    type Item = Value;
    #[inline]
    fn next(&mut self) -> Option<Value> {
        match &self.cur {
            Value::Cons(c) => {
                let (car, cdr) = {
                    let b = c.borrow();
                    (b.car.clone(), b.cdr.clone())
                };
                self.cur = cdr;
                Some(car)
            }
            _ => None,
        }
    }
}

/// The tree-walking evaluator. Takes the environment by reference
/// (P2.3): an owned `Env` parameter forced every caller — including
/// every subform evaluation inside progn/args/setq walks — to bump and
/// later drop an `Rc` refcount pair, which profiling showed was a top
/// cost of interpreted hot loops. Only code that *stores* the env
/// (closure capture, frame creation) clones it now.
pub fn eval(interp: &mut Interp, form: &Value, env: &Env) -> EvalResult {
    interp.depth += 1;
    if interp.depth > interp.max_depth {
        interp.depth -= 1;
        let e = interp.syms.excessive_depth;
        return Err(interp.signal(e, vec![]));
    }
    let result = eval_inner(interp, form, env);
    interp.depth -= 1;
    result
}

/// Evaluate a form that is statistically often a symbol or constant
/// (arguments, loop conditions): non-cons forms are handled inline —
/// they cannot recurse, so the depth-guard ceremony of `eval` buys
/// nothing for them — and only a compound form pays the full `eval`
/// entry (P2.3).
#[inline]
fn eval_form(interp: &mut Interp, form: &Value, env: &Env) -> EvalResult {
    match form {
        Value::Cons(_) => eval(interp, form, env),
        Value::Sym(id) => eval_symbol(interp, *id, env),
        other => Ok(other.clone()),
    }
}

fn eval_inner(interp: &mut Interp, form: &Value, env: &Env) -> EvalResult {
    match form {
        Value::Nil
        | Value::Int(_)
        | Value::Big(_)
        | Value::Float(_)
        | Value::Str(_)
        | Value::Vector(_)
        | Value::HashTable(_)
        | Value::Func(_)
        | Value::Ext(_) => Ok(form.clone()),
        Value::Sym(id) => eval_symbol(interp, *id, env),
        Value::Cons(c) => {
            let (head, args) = {
                let b = c.borrow();
                (b.car.clone(), b.cdr.clone())
            };
            match &head {
                Value::Sym(id) => {
                    // Special forms dispatch on a pre-tagged enum: one
                    // array access, no allocation, no string compares.
                    if let Some(sf) = interp.symbols[*id as usize].special_form {
                        return eval_special_form(interp, sf, &args, env);
                    }
                    let func = resolve_function_of_symbol(interp, *id)?;
                    if let Value::Func(f) = &func {
                        if let Function::Lambda(l) = f.as_ref() {
                            if l.is_macro {
                                let unevaled = args
                                    .list_to_vec()
                                    .ok_or_else(|| interp.error("malformed macro call"))?;
                                let expansion = apply_lambda(interp, l.clone(), unevaled.into())?;
                                return eval(interp, &expansion, env);
                            }
                        }
                    }
                    let argv = eval_args(interp, &args, env)?;
                    apply_function(interp, &func, argv)
                }
                Value::Cons(_) => {
                    // ((lambda ...) args...)
                    let func = eval(interp, &head, env)?;
                    let argv = eval_args(interp, &args, env)?;
                    apply_function(interp, &func, argv)
                }
                _ => Err(interp.wrong_type("functionp", &head)),
            }
        }
    }
}

/// Shared by the tree-walker and the VM's `LoadFree`: resolve a symbol
/// that isn't a compile-time local, via the captured env then the global
/// dynamic cell.
pub(crate) fn eval_symbol(interp: &mut Interp, id: SymId, env: &Env) -> EvalResult {
    if interp.is_keyword(id) {
        return Ok(Value::Sym(id));
    }
    if !interp.symbols[id as usize].special {
        if let Some(e) = env {
            if let Some(v) = LexEnv::lookup(e, id) {
                return Ok(v);
            }
        }
    }
    match interp.sym_value(id) {
        Some(v) => Ok(v),
        None => {
            let e = interp.syms.void_variable;
            Err(interp.signal(e, vec![Value::Sym(id)]))
        }
    }
}

fn eval_args(interp: &mut Interp, args: &Value, env: &Env) -> Result<Args, Flow> {
    let mut out = Args::new();
    let mut it = FormIter::new(args);
    for f in it.by_ref() {
        out.push(eval_form(interp, &f, env)?);
    }
    if !it.at_nil() {
        return Err(interp.error("malformed argument list"));
    }
    Ok(out)
}

/// Follow symbol function cells (aliases) to a callable value.
pub fn resolve_function_of_symbol(interp: &mut Interp, id: SymId) -> EvalResult {
    let mut cur = id;
    for _ in 0..16 {
        match &interp.symbols[cur as usize].function {
            Some(Value::Sym(next)) => cur = *next,
            Some(v) => return Ok(v.clone()),
            None => break,
        }
    }
    let e = interp.syms.void_function;
    Err(interp.signal(e, vec![Value::Sym(id)]))
}

/// Coerce a value to something callable (symbol → function cell, lambda list → closure).
pub fn resolve_function(interp: &mut Interp, v: &Value) -> EvalResult {
    match v {
        Value::Func(_) => Ok(v.clone()),
        Value::Sym(id) => resolve_function_of_symbol(interp, *id),
        Value::Cons(c) => {
            let head = c.borrow().car.clone();
            if let Value::Sym(id) = head {
                if id == interp.syms.lambda {
                    let body = c.borrow().cdr.clone();
                    return make_lambda(interp, &body, None, false);
                }
            }
            Err(interp.wrong_type("functionp", v))
        }
        _ => Err(interp.wrong_type("functionp", v)),
    }
}

pub fn apply_function(interp: &mut Interp, func: &Value, args: Args) -> EvalResult {
    // Fast path: an already-resolved function object (what eval hands us
    // after resolve_function_of_symbol, and what the VM usually pushes)
    // skips resolve_function's re-match + clone (P2.3).
    if let Value::Func(f) = func {
        return apply_func_object(interp, f, args);
    }
    let func = resolve_function(interp, func)?;
    match &func {
        Value::Func(f) => apply_func_object(interp, f, args),
        other => Err(interp.wrong_type("functionp", other)),
    }
}

fn apply_func_object(interp: &mut Interp, f: &Rc<Function>, mut args: Args) -> EvalResult {
    match f.as_ref() {
        Function::Builtin {
            name,
            min_args,
            max_args,
            f,
        } => {
            let n = args.len();
            if n < *min_args || max_args.map(|m| n > m).unwrap_or(false) {
                let e = interp.syms.wrong_number_of_arguments;
                let ns = Value::string(*name);
                return Err(interp.signal(e, vec![ns, Value::Int(n as i64)]));
            }
            f(interp, &mut args)
        }
        Function::Lambda(l) => apply_lambda(interp, l.clone(), args),
        Function::Compiled(c) => crate::vm::run(interp, c, args),
        Function::Native(n) => {
            if args.len() != n.arity() {
                return crate::vm::run(interp, &n.fallback, args);
            }
            let mut raw = Vec::with_capacity(args.len());
            for a in &args {
                match a {
                    Value::Int(x) => raw.push(*x),
                    _ => return crate::vm::run(interp, &n.fallback, args),
                }
            }
            match n.call(&raw) {
                Ok(result) => Ok(Value::Int(result)),
                // Overflow/div-by-zero: safe to fully re-run since
                // native-eligible functions are pure (see jit.rs).
                Err(()) => crate::vm::run(interp, &n.fallback, args),
            }
        }
        Function::Module(mf) => crate::module::call_module_function(interp, mf, args),
    }
}

/// Interpreted calls before a named function is automatically
/// byte-compiled (tier 1 of tiered compilation, HotSpot/V8-style).
pub const AUTO_BYTE_THRESHOLD: u32 = 64;

/// If this lambda is hot, named, and still installed in its symbol's
/// function cell, swap in a byte-compiled version. The in-flight call
/// proceeds interpreted; every later call (including recursive ones,
/// which re-resolve through the symbol) gets the compiled tier.
fn maybe_tier_up_lambda(interp: &mut Interp, l: &Rc<Lambda>) {
    let Some(id) = *l.name.borrow() else { return };
    let holds = matches!(&interp.symbols[id as usize].function,
        Some(Value::Func(f)) if matches!(f.as_ref(), Function::Lambda(l2) if Rc::ptr_eq(l2, l)));
    if !holds {
        return;
    }
    if let Ok(compiled) = crate::compiler::compile_parsed(
        interp,
        l.params.clone(),
        l.body.clone(),
        l.interactive.borrow().clone(),
        l.lexical,
        l.env.clone(),
    ) {
        *compiled.name.borrow_mut() = Some(id);
        interp.symbols[id as usize].function =
            Some(Value::Func(Rc::new(Function::Compiled(compiled))));
    }
}

/// pub(crate) so the compiler can expand macros at compile time.
pub(crate) fn apply_lambda(interp: &mut Interp, l: Rc<Lambda>, args: Args) -> EvalResult {
    // See eval_while: unbounded interpreted execution is only reachable
    // through `while` or recursion, so both check the M15 deadline.
    interp.check_deadline()?;
    if !l.is_macro {
        let n = l.calls.get().wrapping_add(1);
        l.calls.set(n);
        // Exactly-at-threshold so a failed compile is never retried.
        if n == AUTO_BYTE_THRESHOLD {
            maybe_tier_up_lambda(interp, &l);
        }
    }
    let n = args.len();
    let min = l.params.required.len();
    let max = if l.params.rest.is_some() {
        usize::MAX
    } else {
        min + l.params.optional.len()
    };
    if n < min || n > max {
        let e = interp.syms.wrong_number_of_arguments;
        let name = l
            .name
            .borrow()
            .map(Value::Sym)
            .unwrap_or_else(|| Value::string("lambda"));
        return Err(interp.signal(e, vec![name, Value::Int(n as i64)]));
    }

    let frame = if l.lexical {
        Some(LexEnv::new(l.env.clone()))
    } else {
        None
    };
    let mut dyn_saves: Vec<(SymId, Option<Value>)> = Vec::new();
    let mut argv = args.into_iter();
    let mut bind = |interp: &mut Interp, id: SymId, val: Value| {
        bind_one(interp, id, val, &frame, l.lexical, &mut dyn_saves);
    };
    for &id in &l.params.required {
        let v = argv.next().unwrap();
        bind(interp, id, v);
    }
    for &id in &l.params.optional {
        let v = argv.next().unwrap_or(Value::Nil);
        bind(interp, id, v);
    }
    if let Some(id) = l.params.rest {
        let rest: Vec<Value> = argv.collect();
        bind(interp, id, Value::list(rest));
    }

    let body_env = frame.map(|f| f as Rc<LexEnv>).or_else(|| l.env.clone());
    let result = eval_progn(interp, &l.body, &body_env);
    unbind_dynamics(interp, dyn_saves);
    result
}

fn bind_one(
    interp: &mut Interp,
    id: SymId,
    val: Value,
    frame: &Option<Rc<LexEnv>>,
    lexical: bool,
    dyn_saves: &mut Vec<(SymId, Option<Value>)>,
) {
    let special = interp.symbols[id as usize].special;
    if lexical && !special {
        if let Some(f) = frame {
            f.vars.borrow_mut().push((id, val.clone()));
            // `(let* ((f (lambda () f))))`-style: pushing a closure into
            // a frame that closure's own chain contains forms a cycle
            // with no setq anywhere — the GC must see this frame.
            crate::gc::register_env(interp, f, &val);
            return;
        }
    }
    dyn_saves.push((id, interp.symbols[id as usize].value.take()));
    interp.symbols[id as usize].value = Some(val);
}

pub(crate) fn unbind_dynamics(interp: &mut Interp, saves: Vec<(SymId, Option<Value>)>) {
    for (id, old) in saves.into_iter().rev() {
        interp.symbols[id as usize].value = old;
    }
}

pub fn eval_progn(interp: &mut Interp, body: &Value, env: &Env) -> EvalResult {
    let mut last = Value::Nil;
    let mut it = FormIter::new(body);
    for f in it.by_ref() {
        last = eval_form(interp, &f, env)?;
    }
    if !it.at_nil() {
        return Err(interp.error("malformed body"));
    }
    Ok(last)
}

/// Parse `((params...) body...)` into a param spec plus the remaining
/// body with any leading docstring / (declare ...) / (interactive ...)
/// forms stripped out. Shared by the tree-walker and the bytecode
/// compiler so both build closures with identical parameter semantics.
pub(crate) fn parse_lambda_form(
    interp: &mut Interp,
    form: &Value,
) -> Result<(ParamSpec, Value, Option<Value>), Flow> {
    let param_form = form.car();
    let mut body = form.cdr();
    let param_syms = param_form
        .list_to_vec()
        .ok_or_else(|| interp.error("malformed parameter list"))?;
    let mut params = ParamSpec {
        required: Vec::new(),
        optional: Vec::new(),
        rest: None,
    };
    let mut mode = 0; // 0=required 1=optional 2=rest
    for p in &param_syms {
        let id = interp
            .as_sym(p)
            .ok_or_else(|| interp.wrong_type("symbolp", p))?;
        if id == interp.syms.optional {
            mode = 1;
        } else if id == interp.syms.rest {
            mode = 2;
        } else {
            match mode {
                0 => params.required.push(id),
                1 => params.optional.push(id),
                _ => {
                    if params.rest.is_some() {
                        return Err(interp.error("multiple &rest parameters"));
                    }
                    params.rest = Some(id);
                }
            }
        }
    }

    // Strip docstring / (declare ...) / (interactive ...) prefix forms.
    let mut interactive = None;
    loop {
        let first = body.car();
        let rest = body.cdr();
        match &first {
            Value::Str(_) if !rest.is_nil() => body = rest,
            Value::Cons(c) => {
                let head = c.borrow().car.clone();
                match interp.as_sym(&head) {
                    Some(id) if id == interp.syms.declare => body = rest,
                    Some(id) if id == interp.syms.interactive => {
                        interactive = Some(c.borrow().cdr.clone());
                        body = rest;
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    Ok((params, body, interactive))
}

/// Build a Lambda from `((params...) body...)`.
pub fn make_lambda(interp: &mut Interp, form: &Value, env: Env, is_macro: bool) -> EvalResult {
    let (params, body, interactive) = parse_lambda_form(interp, form)?;
    let lexical = interp.lexical_binding;
    Ok(Value::Func(Rc::new(Function::Lambda(Rc::new(Lambda {
        params,
        body,
        env: if lexical { env } else { None },
        lexical,
        is_macro,
        name: RefCell::new(None),
        interactive: RefCell::new(interactive),
        calls: std::cell::Cell::new(0),
    })))))
}

fn eval_special_form(
    interp: &mut Interp,
    sf: crate::interp::SpecialForm,
    args: &Value,
    env: &Env,
) -> EvalResult {
    use crate::interp::SpecialForm as SF;
    match sf {
        SF::Quote => Ok(args.car()),
        SF::Function => {
            let arg = args.car();
            match &arg {
                Value::Cons(c) => {
                    let head = c.borrow().car.clone();
                    if interp.as_sym(&head) == Some(interp.syms.lambda) {
                        let body = c.borrow().cdr.clone();
                        make_lambda(interp, &body, env.clone(), false)
                    } else {
                        Ok(arg)
                    }
                }
                _ => Ok(arg),
            }
        }
        SF::Lambda => make_lambda(interp, args, env.clone(), false),
        SF::If => {
            let cond = eval(interp, &args.car(), env);
            match cond {
                Err(e) => Err(e),
                Ok(c) => {
                    if c.truthy() {
                        eval(interp, &args.cdr().car(), env)
                    } else {
                        eval_progn(interp, &args.cdr().cdr(), env)
                    }
                }
            }
        }
        SF::Cond => eval_cond(interp, args, env),
        SF::While => eval_while(interp, args, env),
        SF::Progn => eval_progn(interp, args, env),
        SF::Prog1 => {
            let first = eval(interp, &args.car(), env);
            match first {
                Err(e) => Err(e),
                Ok(v) => eval_progn(interp, &args.cdr(), env).map(|_| v),
            }
        }
        SF::Prog2 => {
            let r =
                eval(interp, &args.car(), env).and_then(|_| eval(interp, &args.cdr().car(), env));
            match r {
                Err(e) => Err(e),
                Ok(v) => eval_progn(interp, &args.cdr().cdr(), env).map(|_| v),
            }
        }
        SF::And => eval_and(interp, args, env),
        SF::Or => eval_or(interp, args, env),
        SF::Let => eval_let(interp, args, env, false),
        SF::LetStar => eval_let(interp, args, env, true),
        SF::Setq => eval_setq(interp, args, env),
        SF::Defvar => eval_defvar(interp, args, env, false),
        SF::Defconst => eval_defvar(interp, args, env, true),
        SF::Defun => eval_defun(interp, args, false),
        SF::Defmacro => eval_defun(interp, args, true),
        SF::ConditionCase => eval_condition_case(interp, args, env),
        SF::UnwindProtect => eval_unwind_protect(interp, args, env),
        SF::Catch => eval_catch(interp, args, env),
        SF::Interactive => Ok(Value::Nil),
        SF::Backquote => eval_backquote(interp, &args.car(), env),
    }
}

fn eval_cond(interp: &mut Interp, clauses: &Value, env: &Env) -> EvalResult {
    let mut it = FormIter::new(clauses);
    for clause in it.by_ref() {
        let test = eval_form(interp, &clause.car(), env)?;
        if test.truthy() {
            let body = clause.cdr();
            if body.is_nil() {
                return Ok(test);
            }
            return eval_progn(interp, &body, env);
        }
    }
    if !it.at_nil() {
        return Err(interp.error("malformed cond"));
    }
    Ok(Value::Nil)
}

fn eval_while(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let cond = args.car();
    let body = args.cdr();
    loop {
        // The M15 interruption point for interpreted loops: `eval` no
        // longer checks the deadline on every entry (P2.3), so the two
        // constructs that can run unboundedly — this loop and function
        // application — check it themselves.
        interp.check_deadline()?;
        let c = eval_form(interp, &cond, env)?;
        if !c.truthy() {
            return Ok(Value::Nil);
        }
        eval_progn(interp, &body, env)?;
    }
}

fn eval_and(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let mut last = Value::Sym(interp.syms.t);
    let mut it = FormIter::new(args);
    for f in it.by_ref() {
        last = eval_form(interp, &f, env)?;
        if !last.truthy() {
            return Ok(Value::Nil);
        }
    }
    if !it.at_nil() {
        return Err(interp.error("malformed and"));
    }
    Ok(last)
}

fn eval_or(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let mut it = FormIter::new(args);
    for f in it.by_ref() {
        let v = eval_form(interp, &f, env)?;
        if v.truthy() {
            return Ok(v);
        }
    }
    if !it.at_nil() {
        return Err(interp.error("malformed or"));
    }
    Ok(Value::Nil)
}

fn eval_let(interp: &mut Interp, args: &Value, env: &Env, star: bool) -> EvalResult {
    let binding_list = args.car();
    if proper_list_len(&binding_list).is_none() {
        return Err(interp.error("malformed let bindings"));
    }
    let body = args.cdr();
    let lexical = interp.lexical_binding;
    let frame = if lexical {
        Some(LexEnv::new(env.clone()))
    } else {
        None
    };
    let mut dyn_saves: Vec<(SymId, Option<Value>)> = Vec::new();

    // let evaluates all values in the outer env first; let* in the growing env.
    let value_env = |frame: &Option<Rc<LexEnv>>| -> Env {
        if star {
            frame
                .clone()
                .map(|f| f as Rc<LexEnv>)
                .or_else(|| env.clone())
        } else {
            env.clone()
        }
    };

    let mut pending: smallvec::SmallVec<[(SymId, Value); 4]> = smallvec::SmallVec::new();
    let result: EvalResult = 'outer: {
        let mut it = FormIter::new(&binding_list);
        for b in it.by_ref() {
            let (id, val_form) = match &b {
                Value::Sym(id) => (*id, Value::Nil),
                Value::Cons(_) => {
                    let sym = b.car();
                    let id = match interp.as_sym(&sym) {
                        Some(id) => id,
                        None => break 'outer Err(interp.wrong_type("symbolp", &sym)),
                    };
                    (id, b.cdr().car())
                }
                other => break 'outer Err(interp.wrong_type("symbolp", other)),
            };
            let ve = value_env(&frame);
            let val = match eval(interp, &val_form, &ve) {
                Ok(v) => v,
                Err(e) => break 'outer Err(e),
            };
            if star {
                bind_one(interp, id, val, &frame, lexical, &mut dyn_saves);
            } else {
                pending.push((id, val));
            }
        }
        if !star {
            for (id, val) in pending {
                bind_one(interp, id, val, &frame, lexical, &mut dyn_saves);
            }
        }
        let body_env = frame
            .clone()
            .map(|f| f as Rc<LexEnv>)
            .or_else(|| env.clone());
        eval_progn(interp, &body, &body_env)
    };
    unbind_dynamics(interp, dyn_saves);
    result
}

/// Shared by the tree-walker's `setq` and the VM's `StoreFree`: set a
/// symbol's value, preferring a live lexical binding in `env` when the
/// symbol isn't dynamically special.
pub(crate) fn set_symbol(
    interp: &mut Interp,
    id: SymId,
    val: Value,
    env: &Env,
) -> Result<(), Flow> {
    if id == 0 || id == interp.syms.t || interp.is_keyword(id) {
        let e = interp.syms.setting_constant;
        return Err(interp.signal(e, vec![Value::Sym(id)]));
    }
    let special = interp.symbols[id as usize].special;
    let mut set_lexically = false;
    if !special {
        if let Some(e) = env {
            if let Some(frame) = LexEnv::set(e, id, val.clone()) {
                // A captured-variable assignment is how closures become
                // self-referential; the GC needs to know this frame.
                crate::gc::register_env(interp, &frame, &val);
                set_lexically = true;
            }
        }
    }
    if !set_lexically {
        interp.set_sym_value(id, val);
    }
    Ok(())
}

fn eval_setq(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let mut last = Value::Nil;
    let mut it = FormIter::new(args);
    // Explicit loop (not `for`): each pass consumes TWO elements.
    loop {
        let Some(sym) = it.next() else {
            if !it.at_nil() {
                return Err(interp.error("malformed setq"));
            }
            return Ok(last);
        };
        let id = interp
            .as_sym(&sym)
            .ok_or_else(|| interp.wrong_type("symbolp", &sym))?;
        let Some(val_form) = it.next() else {
            return Err(if it.at_nil() {
                interp.error("setq: odd number of arguments")
            } else {
                interp.error("malformed setq")
            });
        };
        let val = eval_form(interp, &val_form, env)?;
        last = val.clone();
        set_symbol(interp, id, val, env)?;
    }
}

fn eval_defvar(interp: &mut Interp, args: &Value, env: &Env, is_const: bool) -> EvalResult {
    let sym = args.car();
    let id = interp
        .as_sym(&sym)
        .ok_or_else(|| interp.wrong_type("symbolp", &sym))?;
    interp.symbols[id as usize].special = true;
    let has_value = matches!(args.cdr(), Value::Cons(_));
    if has_value {
        let already_bound = interp.symbols[id as usize].value.is_some();
        if is_const || !already_bound {
            let val = eval(interp, &args.cdr().car(), env)?;
            interp.set_sym_value(id, val);
        }
    }
    Ok(Value::Sym(id))
}

fn eval_defun(interp: &mut Interp, args: &Value, is_macro: bool) -> EvalResult {
    let sym = args.car();
    let id = interp
        .as_sym(&sym)
        .ok_or_else(|| interp.wrong_type("symbolp", &sym))?;
    // Retain the docstring on the symbol's plist (self-documentation is
    // an Emacs hallmark). Stored on the SYMBOL rather than the function
    // object, so it survives byte-/native-compilation untouched.
    if let Value::Str(doc) = args.cdr().cdr().car() {
        let prop = interp.intern("function-documentation");
        interp.plist_put(id, prop, Value::Str(doc));
    }
    // defun does not capture the enclosing lexical env (matches Emacs).
    let func = make_lambda(interp, &args.cdr(), None, is_macro)?;
    if let Value::Func(f) = &func {
        if let Function::Lambda(l) = f.as_ref() {
            *l.name.borrow_mut() = Some(id);
        }
    }
    interp.symbols[id as usize].function = Some(func);
    Ok(Value::Sym(id))
}

fn eval_condition_case(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let var = args.car();
    let body = args.cdr().car();
    let handlers = args.cdr().cdr();
    match eval(interp, &body, env) {
        Ok(v) => Ok(v),
        Err(Flow::Throw { tag, value }) => Err(Flow::Throw { tag, value }),
        Err(Flow::Signal { error_symbol, data }) => {
            let handler_list = handlers
                .list_to_vec()
                .ok_or_else(|| interp.error("malformed condition-case"))?;
            let err_conditions = match interp.as_sym(&error_symbol) {
                Some(id) => interp.plist_get(id, interp.syms.error_conditions),
                None => Value::Nil,
            };
            for h in &handler_list {
                let cond_spec = h.car();
                if condition_matches(interp, &cond_spec, &err_conditions) {
                    let err_val = Value::cons(error_symbol.clone(), data.clone());
                    let lexical = interp.lexical_binding;
                    let frame = if lexical {
                        Some(LexEnv::new(env.clone()))
                    } else {
                        None
                    };
                    let mut dyn_saves = Vec::new();
                    if let Some(vid) = interp.as_sym(&var) {
                        if vid != 0 {
                            bind_one(interp, vid, err_val, &frame, lexical, &mut dyn_saves);
                        }
                    }
                    let body_env = frame.map(|f| f as Rc<LexEnv>).or_else(|| env.clone());
                    let r = eval_progn(interp, &h.cdr(), &body_env);
                    unbind_dynamics(interp, dyn_saves);
                    return r;
                }
            }
            Err(Flow::Signal { error_symbol, data })
        }
    }
}

fn condition_matches(interp: &Interp, spec: &Value, err_conditions: &Value) -> bool {
    let matches_one = |s: &Value| -> bool {
        if let Value::Sym(id) = s {
            if *id == interp.syms.t {
                return true;
            }
        }
        let mut cur = err_conditions.clone();
        while let Value::Cons(c) = cur {
            let b = c.borrow();
            if b.car.eq(s) {
                return true;
            }
            cur = b.cdr.clone();
        }
        false
    };
    match spec {
        Value::Cons(_) => spec
            .list_to_vec()
            .map(|v| v.iter().any(&matches_one))
            .unwrap_or(false),
        other => matches_one(other),
    }
}

fn eval_unwind_protect(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let result = eval(interp, &args.car(), env);
    let cleanup = eval_progn(interp, &args.cdr(), env);
    match cleanup {
        Err(e) => Err(e),
        Ok(_) => result,
    }
}

fn eval_catch(interp: &mut Interp, args: &Value, env: &Env) -> EvalResult {
    let tag = eval(interp, &args.car(), env)?;
    match eval_progn(interp, &args.cdr(), env) {
        Err(Flow::Throw { tag: t, value }) if t.eq(&tag) => Ok(value),
        other => other,
    }
}

fn eval_backquote(interp: &mut Interp, form: &Value, env: &Env) -> EvalResult {
    match form {
        Value::Cons(c) => {
            let (head, rest) = {
                let b = c.borrow();
                (b.car.clone(), b.cdr.clone())
            };
            if let Some(id) = interp.as_sym(&head) {
                if id == interp.syms.unquote {
                    return eval(interp, &rest.car(), env);
                }
                if id == interp.syms.backquote {
                    return Err(interp.error("nested backquote not supported"));
                }
            }
            // Rebuild the list, splicing ,@ elements.
            let mut items: Vec<Value> = Vec::new();
            let mut cur = form.clone();
            loop {
                match cur {
                    Value::Nil => return Ok(Value::list(items)),
                    Value::Cons(cell) => {
                        let (car, cdr) = {
                            let b = cell.borrow();
                            (b.car.clone(), b.cdr.clone())
                        };
                        // Dotted unquote tail: `(a . ,b)
                        if let Some(id) = interp.as_sym(&car) {
                            if id == interp.syms.unquote {
                                let tail = eval(interp, &cdr.car(), env)?;
                                return Ok(build_dotted(items, tail));
                            }
                        }
                        if let Value::Cons(inner) = &car {
                            let ihead = inner.borrow().car.clone();
                            if interp.as_sym(&ihead) == Some(interp.syms.unquote_splicing) {
                                let spliced = eval(interp, &inner.borrow().cdr.car(), env)?;
                                match spliced.list_to_vec() {
                                    Some(vs) => items.extend(vs),
                                    None => return Err(interp.wrong_type("listp", &spliced)),
                                }
                                cur = cdr;
                                continue;
                            }
                        }
                        items.push(eval_backquote(interp, &car, env)?);
                        cur = cdr;
                    }
                    other => {
                        let tail = eval_backquote(interp, &other, env)?;
                        return Ok(build_dotted(items, tail));
                    }
                }
            }
        }
        Value::Vector(items) => {
            let mut out = Vec::new();
            for item in items.borrow().iter() {
                out.push(eval_backquote(interp, item, env)?);
            }
            Ok(Value::Vector(Rc::new(RefCell::new(out))))
        }
        _ => Ok(form.clone()),
    }
}

fn build_dotted(items: Vec<Value>, tail: Value) -> Value {
    let mut acc = tail;
    for v in items.into_iter().rev() {
        acc = Value::cons(v, acc);
    }
    acc
}
