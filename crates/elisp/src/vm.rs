//! Executes `bytecode::Chunk`s produced by the compiler.
//!
//! Calling convention: `Instr::Call(n)` expects the stack, from bottom to
//! top, to hold `[function, arg1, .., argn]`; it pops all of them and
//! pushes the single result. Locals are a flat `Vec<Value>` addressed by
//! slot index — no alist scan. Free variables (closure captures, globals,
//! dynamic vars) go through the exact same resolution the tree-walker
//! uses (`eval::eval_symbol` / `eval::set_symbol`), so both evaluation
//! strategies agree on every lookup.

use std::rc::Rc;

use crate::bytecode::{Chunk, Instr};
use crate::error::{EvalResult, Flow};
use crate::eval::{apply_function, eval, eval_symbol, set_symbol, unbind_dynamics};
use crate::interp::Interp;
use crate::value::{Args, CompiledFn, Function, LexEnv, SymId, Value};

/// Bytecode calls before a hot compiled function gets its one
/// native-compilation attempt (tier 2). Ineligible functions just stay
/// on bytecode — `jit_tried` ensures the attempt happens once.
pub const AUTO_NATIVE_THRESHOLD: u32 = 1024;

fn maybe_tier_up_native(interp: &mut Interp, f: &Rc<CompiledFn>) {
    let Some(id) = *f.name.borrow() else { return };
    let holds = matches!(&interp.symbols[id as usize].function,
        Some(Value::Func(v)) if matches!(v.as_ref(), Function::Compiled(c2) if Rc::ptr_eq(c2, f)));
    if !holds {
        return;
    }
    if let Some(native) = crate::jit::try_compile(interp, f) {
        interp.symbols[id as usize].function = Some(Value::Func(Rc::new(Function::Native(native))));
    }
}

pub fn run(interp: &mut Interp, f: &Rc<CompiledFn>, args: Args) -> EvalResult {
    // Per-call M15 interruption point, symmetric with apply_lambda: a
    // builtin looping over compiled callbacks (mapcar over a huge or
    // circular list) must stay interruptible even when each callback's
    // chunk is straight-line code that never hits the jump/call checks
    // inside `execute`.
    interp.check_deadline()?;
    if !f.jit_tried.get() {
        let n = f.calls.get().wrapping_add(1);
        f.calls.set(n);
        if n >= AUTO_NATIVE_THRESHOLD {
            f.jit_tried.set(true);
            maybe_tier_up_native(interp, f);
        }
    }
    interp.depth += 1;
    if interp.depth > interp.max_depth {
        interp.depth -= 1;
        let e = interp.syms.excessive_depth;
        return Err(interp.signal(e, vec![]));
    }
    let result = run_inner(interp, f, args);
    interp.depth -= 1;
    result
}

fn run_inner(interp: &mut Interp, f: &Rc<CompiledFn>, args: Args) -> EvalResult {
    let n = args.len();
    let min = f.params.required.len();
    let max = if f.params.rest.is_some() {
        usize::MAX
    } else {
        min + f.params.optional.len()
    };
    if n < min || n > max {
        let e = interp.syms.wrong_number_of_arguments;
        let name = f
            .name
            .borrow()
            .map(Value::Sym)
            .unwrap_or_else(|| Value::string("lambda"));
        return Err(interp.signal(e, vec![name, Value::Int(n as i64)]));
    }

    // Inline storage for the overwhelmingly common small-frame case:
    // one heap allocation per call was measurable in call-dense code
    // (fib-style recursion) — P2.3. `execute` sees a plain `&mut [Value]`
    // either way, so per-access cost is unchanged.
    let mut locals: smallvec::SmallVec<[Value; 8]> =
        smallvec::smallvec![Value::Nil; f.chunk.num_locals];
    let mut dyn_saves: Vec<(SymId, Option<Value>)> = Vec::new();
    let mut argv = args.into_iter();
    let mut next_slot = 0usize;

    // The compiler allocates parameter slots in call order for lexical
    // params and skips a slot for dynamic ones (see Compiler::bind_incoming),
    // so params_lexical below just re-derives, per parameter, whether it
    // has a slot the same way the compiler decided.
    let dynamic =
        |interp: &Interp, id: SymId| -> bool { !f.lexical || interp.symbols[id as usize].special };
    for &id in f.params.required.iter().chain(f.params.optional.iter()) {
        let v = argv.next().unwrap_or(Value::Nil);
        if dynamic(interp, id) {
            dyn_saves.push((id, interp.symbols[id as usize].value.take()));
            interp.set_sym_value(id, v);
        } else {
            locals[next_slot] = v;
            next_slot += 1;
        }
    }
    if let Some(id) = f.params.rest {
        let rest: Vec<Value> = argv.collect();
        let v = Value::list(rest);
        if dynamic(interp, id) {
            dyn_saves.push((id, interp.symbols[id as usize].value.take()));
            interp.set_sym_value(id, v);
        } else {
            locals[next_slot] = v;
        }
    }

    let vm_result = execute(interp, &f.chunk, &f.env, &mut locals, &mut dyn_saves);
    unbind_dynamics(interp, dyn_saves);
    vm_result
}

/// Slow path for a specialized 2-arg opcode: call the compile-time-
/// frozen builtin stored in the constant pool (handles floats, type
/// errors, overflow promotion — whatever the real builtin does).
fn slow_call2(interp: &mut Interp, chunk: &Rc<Chunk>, cidx: u32, a: Value, b: Value) -> EvalResult {
    let f = chunk.consts[cidx as usize].clone();
    apply_function(interp, &f, smallvec::smallvec![a, b])
}

fn slow_call1(interp: &mut Interp, chunk: &Rc<Chunk>, cidx: u32, a: Value) -> EvalResult {
    let f = chunk.consts[cidx as usize].clone();
    apply_function(interp, &f, smallvec::smallvec![a])
}

/// Shared body of the five comparison opcodes.
fn cmp2(
    interp: &mut Interp,
    chunk: &Rc<Chunk>,
    cidx: u32,
    stack: &mut Vec<Value>,
    op: impl Fn(i64, i64) -> bool,
) -> Result<(), Flow> {
    let b = stack.pop().expect("stack underflow");
    let a = stack.pop().expect("stack underflow");
    match (&a, &b) {
        (Value::Int(x), Value::Int(y)) => {
            stack.push(Value::bool(op(*x, *y), interp.syms.t));
            Ok(())
        }
        _ => {
            let v = slow_call2(interp, chunk, cidx, a, b)?;
            stack.push(v);
            Ok(())
        }
    }
}

fn execute(
    interp: &mut Interp,
    chunk: &Rc<Chunk>,
    captured_env: &Option<Rc<LexEnv>>,
    locals: &mut [Value],
    dyn_saves: &mut Vec<(SymId, Option<Value>)>,
) -> EvalResult {
    let mut stack: Vec<Value> = Vec::with_capacity(8);
    let mut pc = 0usize;
    // Hoisted out of the dispatch loop: one Rc deref instead of one per
    // instruction (P2.3).
    let code: &[Instr] = &chunk.code;
    // The live lexical chain: starts at the closure's captured env and
    // grows/shrinks as PushFrame/PopFrame execute. Closures capture this
    // chain directly (variables, not values), and free-variable access
    // resolves through it — one shared binding for everyone. On abnormal
    // exit the whole chain is simply discarded, exactly like the
    // tree-walker dropping its frames.
    let mut cur_env: Option<Rc<LexEnv>> = captured_env.clone();
    loop {
        // The M15 interruption check moved from here (once per
        // instruction) to the jump and call arms below (P2.3): any
        // cycle in the control-flow graph must execute a jump per
        // traversal, and straight-line code is bounded by chunk length,
        // so interruptibility is preserved — a long-running chunk still
        // consults the deadline at least once per loop iteration —
        // while the ~10 straight-line instructions of a typical loop
        // body no longer each pay the check.
        let instr = code[pc];
        pc += 1;
        match instr {
            Instr::Const(idx) => stack.push(chunk.consts[idx as usize].clone()),
            Instr::LoadLocal(slot) => stack.push(locals[slot as usize].clone()),
            Instr::StoreLocal(slot) => {
                locals[slot as usize] = stack.pop().expect("stack underflow: StoreLocal");
            }
            Instr::LoadFree(sym) => {
                let v = eval_symbol(interp, sym, &cur_env)?;
                stack.push(v);
            }
            Instr::StoreFree(sym) => {
                let v = stack.pop().expect("stack underflow: StoreFree");
                set_symbol(interp, sym, v, &cur_env)?;
            }
            Instr::PushFrame => {
                cur_env = Some(LexEnv::new(cur_env.clone()));
            }
            Instr::PopFrame => {
                cur_env = cur_env.as_ref().and_then(|e| e.parent.clone());
            }
            Instr::EnvDefine(sym) => {
                let v = stack.pop().expect("stack underflow: EnvDefine");
                let frame = cur_env.clone().expect("EnvDefine without a frame");
                frame.vars.borrow_mut().push((sym, v.clone()));
                // Defining a closure into a frame that closure's own
                // chain contains creates a cycle with zero mutation —
                // the GC must know this frame (same rule as bind_one).
                crate::gc::register_env(interp, &frame, &v);
            }
            Instr::DynBind(sym) => {
                let v = stack.pop().expect("stack underflow: DynBind");
                dyn_saves.push((sym, interp.symbols[sym as usize].value.take()));
                interp.set_sym_value(sym, v);
            }
            Instr::DynUnbind(count) => {
                let at = dyn_saves.len() - count as usize;
                let group: Vec<_> = dyn_saves.split_off(at);
                unbind_dynamics(interp, group);
            }
            Instr::MakeClosure(idx) => {
                // Capture the live chain itself — no snapshot. Any
                // variable a closure can see is env-resident (the
                // compiler guarantees it), so closure and creator share
                // the same bindings, matching the tree-walker exactly.
                let info = &chunk.closures[idx as usize];
                let t = &info.template;
                let closure = crate::value::CompiledFn {
                    params: t.params.clone(),
                    chunk: t.chunk.clone(),
                    env: cur_env.clone(),
                    lexical: t.lexical,
                    name: std::cell::RefCell::new(None),
                    interactive: std::cell::RefCell::new(t.interactive.borrow().clone()),
                    calls: std::cell::Cell::new(0),
                    jit_tried: std::cell::Cell::new(false),
                };
                stack.push(Value::Func(Rc::new(Function::Compiled(Rc::new(closure)))));
            }
            Instr::Call(argc) => {
                interp.check_deadline()?;
                let n = argc as usize;
                let mut call_args = Args::with_capacity(n);
                for _ in 0..n {
                    call_args.push(stack.pop().expect("stack underflow: Call arg"));
                }
                call_args.reverse();
                let func = stack.pop().expect("stack underflow: Call fn");
                let result = apply_function(interp, &func, call_args)?;
                stack.push(result);
            }
            // Dedicated primitive opcodes: the Int×Int (or cons) common
            // case runs inline with zero allocation; anything else —
            // floats, wrong types, overflow — takes the compile-time-
            // frozen builtin from the constant pool, which raises
            // exactly the errors the ordinary call path would.
            Instr::Add2(cidx) => {
                let b = stack.pop().expect("stack underflow");
                let a = stack.pop().expect("stack underflow");
                match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => match x.checked_add(*y) {
                        Some(r) => stack.push(Value::Int(r)),
                        None => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                    },
                    _ => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                }
            }
            Instr::Sub2(cidx) => {
                let b = stack.pop().expect("stack underflow");
                let a = stack.pop().expect("stack underflow");
                match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => match x.checked_sub(*y) {
                        Some(r) => stack.push(Value::Int(r)),
                        None => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                    },
                    _ => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                }
            }
            Instr::Mul2(cidx) => {
                let b = stack.pop().expect("stack underflow");
                let a = stack.pop().expect("stack underflow");
                match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => match x.checked_mul(*y) {
                        Some(r) => stack.push(Value::Int(r)),
                        None => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                    },
                    _ => stack.push(slow_call2(interp, chunk, cidx, a, b)?),
                }
            }
            Instr::Lt2(cidx) => cmp2(interp, chunk, cidx, &mut stack, |x, y| x < y)?,
            Instr::Gt2(cidx) => cmp2(interp, chunk, cidx, &mut stack, |x, y| x > y)?,
            Instr::Le2(cidx) => cmp2(interp, chunk, cidx, &mut stack, |x, y| x <= y)?,
            Instr::Ge2(cidx) => cmp2(interp, chunk, cidx, &mut stack, |x, y| x >= y)?,
            Instr::NumEq2(cidx) => cmp2(interp, chunk, cidx, &mut stack, |x, y| x == y)?,
            Instr::Inc1(cidx) => {
                let a = stack.pop().expect("stack underflow");
                match &a {
                    Value::Int(x) => match x.checked_add(1) {
                        Some(r) => stack.push(Value::Int(r)),
                        None => stack.push(slow_call1(interp, chunk, cidx, a)?),
                    },
                    _ => stack.push(slow_call1(interp, chunk, cidx, a)?),
                }
            }
            Instr::Dec1(cidx) => {
                let a = stack.pop().expect("stack underflow");
                match &a {
                    Value::Int(x) => match x.checked_sub(1) {
                        Some(r) => stack.push(Value::Int(r)),
                        None => stack.push(slow_call1(interp, chunk, cidx, a)?),
                    },
                    _ => stack.push(slow_call1(interp, chunk, cidx, a)?),
                }
            }
            Instr::Car1(cidx) => {
                let a = stack.pop().expect("stack underflow");
                match &a {
                    Value::Nil => stack.push(Value::Nil),
                    Value::Cons(c) => {
                        let v = c.borrow().car.clone();
                        stack.push(v);
                    }
                    _ => stack.push(slow_call1(interp, chunk, cidx, a)?),
                }
            }
            Instr::Cdr1(cidx) => {
                let a = stack.pop().expect("stack underflow");
                match &a {
                    Value::Nil => stack.push(Value::Nil),
                    Value::Cons(c) => {
                        let v = c.borrow().cdr.clone();
                        stack.push(v);
                    }
                    _ => stack.push(slow_call1(interp, chunk, cidx, a)?),
                }
            }
            Instr::Eq2 => {
                let b = stack.pop().expect("stack underflow");
                let a = stack.pop().expect("stack underflow");
                stack.push(Value::bool(a.eq(&b), interp.syms.t));
            }
            Instr::Not1 => {
                let a = stack.pop().expect("stack underflow");
                stack.push(Value::bool(a.is_nil(), interp.syms.t));
            }
            Instr::Cons2 => {
                let b = stack.pop().expect("stack underflow");
                let a = stack.pop().expect("stack underflow");
                stack.push(Value::cons(a, b));
            }
            Instr::Jump(target) => {
                interp.check_deadline()?;
                pc = target;
            }
            Instr::JumpIfNil(target) => {
                interp.check_deadline()?;
                let v = stack.pop().expect("stack underflow: JumpIfNil");
                if v.is_nil() {
                    pc = target;
                }
            }
            Instr::JumpIfNonNil(target) => {
                interp.check_deadline()?;
                let v = stack.pop().expect("stack underflow: JumpIfNonNil");
                if v.truthy() {
                    pc = target;
                }
            }
            Instr::Pop => {
                stack.pop();
            }
            Instr::Dup => {
                let top = stack.last().expect("stack underflow: Dup").clone();
                stack.push(top);
            }
            Instr::Interpret(idx) => {
                interp.check_deadline()?;
                let info = &chunk.interprets[idx as usize];
                let frame = LexEnv::new(cur_env.clone());
                {
                    let mut vars = frame.vars.borrow_mut();
                    for &(sym, slot) in &info.scope {
                        vars.push((sym, locals[slot as usize].clone()));
                    }
                }
                // Copy any updates back after eval, in case the
                // interpreted form (e.g. condition-case, setq inside a
                // catch) mutated a variable that's also a compiled local.
                let form = info.form.clone();
                let scope = info.scope.clone();
                let result = eval(interp, &form, &Some(frame.clone()))?;
                for (sym, slot) in scope {
                    if let Some(v) = LexEnv::lookup(&frame, sym) {
                        locals[slot as usize] = v;
                    }
                }
                stack.push(result);
            }
            Instr::Return => {
                return Ok(stack.pop().unwrap_or(Value::Nil));
            }
        }
    }
}
