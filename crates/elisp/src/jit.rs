//! Level-2 acceleration: native machine code via Cranelift, generated
//! straight from an already-compiled `bytecode::Chunk` (reusing the
//! bytecode compiler's macro expansion, control-flow flattening, and
//! backpatched jump targets — the JIT only has to add a new backend).
//!
//! Eligibility is intentionally narrow and safety-first: only functions
//! that touch nothing but their own local variables and integer
//! arithmetic/comparisons qualify. No free variables, no dynamic
//! binding, no closures, no calls to anything except a small whitelist
//! of arithmetic builtins, no `Interpret` fallback instructions. This
//! guarantees every eligible function is **pure** (no observable side
//! effect), which matters a lot at the boundary: on arithmetic overflow
//! or division by zero, the native call simply reports failure and the
//! caller re-runs the same call through the (always-correct) bytecode
//! VM from scratch. If the function could have side effects, that retry
//! would double them; because it can't, the retry is always safe.
//!
//! Comparisons (`<`, `=`, ...) return an elisp boolean (`t`/`nil`), which
//! this representation has no room for (everything here is a bare i64).
//! So a comparison's result may only ever be consumed by a branch
//! (`JumpIfNil`/`JumpIfNonNil`), a `Dup`, or a `Pop` — never stored,
//! passed to arithmetic, or returned. Anything else about a function
//! that doesn't fit this mold just isn't compiled; it keeps running on
//! the bytecode VM, with no error and no behavior change.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, Block as ClifBlock, InstBuilder, Value as ClifValue};
use cranelift_codegen::settings;
use cranelift_codegen::settings::Configurable;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};

use crate::bytecode::{Chunk, Instr};
use crate::value::{CompiledFn, SymId};
use crate::Interp;

/// A natively compiled function. Keeps the `JITModule` that owns the
/// executable memory alive for as long as `ptr` might be called (v1:
/// one module per function — compilation isn't hot, so the extra
/// modules cost nothing that matters).
pub struct NativeFn {
    ptr: extern "C" fn(*const i64, *mut u8) -> i64,
    arity: usize,
    pub fallback: Rc<CompiledFn>,
    _module: JITModule,
}

impl NativeFn {
    /// Call with plain-integer args (caller already verified arity and
    /// that every arg is a bare integer). `Ok` on success; `Err(())`
    /// means overflow/division-by-zero — the caller should re-run via
    /// `fallback` instead, safe because eligible functions are pure.
    /// A bare `()` error is deliberate: there's exactly one failure mode
    /// and no detail to report, callers only ever check `is_err()`.
    #[allow(clippy::result_unit_err)]
    pub fn call(&self, args: &[i64]) -> Result<i64, ()> {
        debug_assert_eq!(args.len(), self.arity);
        let mut ok: u8 = 0;
        let result = (self.ptr)(args.as_ptr(), &mut ok as *mut u8);
        if ok == 1 {
            Ok(result)
        } else {
            Err(())
        }
    }

    pub fn arity(&self) -> usize {
        self.arity
    }
}

/// Try to natively compile `fallback` (already-compiled bytecode).
/// Returns `None` (not an error) whenever the function falls outside
/// the eligible subset — the caller just keeps using `fallback` as-is.
fn debug_fail(stage: &str, e: &impl std::fmt::Debug) {
    if std::env::var("JIT_DEBUG").is_ok() {
        eprintln!("jit: {} failed: {:?}", stage, e);
    }
}

pub fn try_compile(interp: &Interp, fallback: &Rc<CompiledFn>) -> Option<Rc<NativeFn>> {
    // v1 restriction: no &optional/&rest — every call site always
    // supplies exactly `required.len()` plain-integer arguments.
    if !fallback.params.optional.is_empty() || fallback.params.rest.is_some() {
        return None;
    }
    let arity = fallback.params.required.len();
    let chunk = &fallback.chunk;

    // Eligibility + "which leaders need a Cranelift block param" both
    // come from one static analysis over the bytecode, before touching
    // Cranelift at all — see `analyze_shapes`.
    let leaders = compute_leaders(&chunk.code);
    let shapes = analyze_shapes(chunk, interp, &leaders)?;

    let mut flag_builder = settings::builder();
    flag_builder.set("use_colocated_libcalls", "false").ok()?;
    flag_builder.set("is_pic", "false").ok()?;
    let isa_builder = cranelift_native::builder().ok()?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .ok()?;
    let call_conv = isa.default_call_conv();
    let jit_builder = JITBuilder::with_isa(isa, default_libcall_names());
    let mut module = JITModule::new(jit_builder);

    let mut ctx = module.make_context();
    // (args: *const i64, out_ok: *mut i8) -> i64
    ctx.func.signature.call_conv = call_conv;
    ctx.func.signature.params.push(AbiParam::new(types::I64));
    ctx.func.signature.params.push(AbiParam::new(types::I64));
    ctx.func.signature.returns.push(AbiParam::new(types::I64));

    let mut fb_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
    let built = {
        let mut tc = TranslateCtx::new(interp, &mut builder, chunk, arity, shapes);
        tc.run()
    };
    if !built {
        return None;
    }
    builder.finalize();
    if std::env::var("JIT_DEBUG").is_ok() {
        eprintln!("jit: IR:\n{}", ctx.func);
    }

    let name = format!("reticle_native_{:p}", Rc::as_ptr(fallback));
    let id = match module.declare_function(&name, Linkage::Export, &ctx.func.signature) {
        Ok(id) => id,
        Err(e) => {
            debug_fail("declare_function", &e);
            return None;
        }
    };
    if let Err(e) = module.define_function(id, &mut ctx) {
        debug_fail("define_function", &e);
        return None;
    }
    module.clear_context(&mut ctx);
    if let Err(e) = module.finalize_definitions() {
        debug_fail("finalize_definitions", &e);
        return None;
    }
    let code_ptr = module.get_finalized_function(id);
    // SAFETY: `ptr` was just JIT-compiled from `ctx.func`, whose
    // signature we built above to exactly match this Rust fn-pointer
    // type (two i64 params, i64 return — the "out_ok" byte is written
    // through the second param, a raw pointer smuggled as i64). The
    // owning `module` is stored alongside `ptr` in `NativeFn` and never
    // dropped while `ptr` might still be called.
    let ptr: extern "C" fn(*const i64, *mut u8) -> i64 = unsafe { std::mem::transmute(code_ptr) };

    Some(Rc::new(NativeFn {
        ptr,
        arity,
        fallback: fallback.clone(),
        _module: module,
    }))
}

/// One value on the abstract compile-time stack: either a real native
/// value, or a boolean-flavored comparison result that may only be
/// consumed by a branch/Dup/Pop (see module docs), or an as-yet-unread
/// callee symbol waiting to be consumed by a `Call`.
#[derive(Clone, Copy)]
enum Slot {
    Int(ClifValue),
    Bool(ClifValue),
    Callee(SymId),
    /// A literal `nil` (carries no runtime payload at all). Control-flow
    /// constructs like `while`/`cond` push one of these for "no value";
    /// it's fine as long as it's only ever discarded (`Pop`/`Dup`) —
    /// `pop_int`/`pop_bool_or_int` already reject it everywhere else
    /// (StoreLocal, arithmetic operands, Return, branch conditions),
    /// which is exactly right since nil has no i64 representation here.
    NilConst,
}

/// Type-only shadow of `Slot`, used by the shape pre-pass (see
/// `analyze_shapes`) — no Cranelift values involved, just enough to
/// determine, for every point two control-flow edges can meet, whether
/// they agree on what's live there and at what Cranelift type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int,
    Bool,
    Nil,
    /// A pending callee marker. Never valid to carry across a block
    /// boundary (see `analyze_shapes`) — its presence in any recorded
    /// leader shape makes the whole function ineligible, regardless of
    /// which symbol (so comparing two different callees unequal here
    /// never hides a real match).
    Callee(SymId),
}

/// Walks the chunk once, in the same program order the real codegen
/// pass will, tracking only *kinds* (no Cranelift IR at all) to work out
/// what Cranelift block parameters each leader needs — i.e. this is the
/// "which values need a phi node" analysis for turning our flat,
/// stack-based bytecode into Cranelift's block-based SSA form. Also
/// serves as a full eligibility pre-check: anything that would make
/// native compilation impossible is caught here, before touching
/// Cranelift at all, so `None` doesn't leak any partial JIT state.
///
/// Returns the per-leader shape (bottom-to-top `Kind`s live at entry)
/// on success.
fn analyze_shapes(
    chunk: &Chunk,
    interp: &Interp,
    leaders: &[usize],
) -> Option<HashMap<usize, Vec<Kind>>> {
    let leader_set: HashSet<usize> = leaders.iter().copied().collect();
    // Shapes come *only* from actual control-flow edges, never from
    // ambient stack contents at a leader boundary. That ambient state is
    // meaningless (garbage) whenever the leader is reached by falling
    // past an unconditional Jump/Return with no other predecessor —
    // exactly the position `compute_leaders` still marks as a leader
    // (dead code immediately after a terminator), and using it as a
    // stand-in for the real shape once produced a canonical-but-wrong
    // entry that a legitimate edge later mismatched against.
    let mut shapes: HashMap<usize, Vec<Kind>> = HashMap::new();

    let mut stack: Vec<Kind> = Vec::new();
    let mut pc = 0usize;
    let mut terminated = false;
    while pc < chunk.code.len() {
        if leader_set.contains(&pc) && pc != 0 {
            if let Some(known) = shapes.get(&pc) {
                // A real edge into this leader was already processed
                // (always true for backward-reachable-only or
                // already-visited-source forward targets) — trust it
                // over whatever `stack` currently holds.
                stack = known.clone();
                terminated = false;
            } else if !terminated {
                // Reached via genuine live fallthrough with no prior
                // edge recorded yet (e.g. a while-loop header, first
                // seen via straight-line code before its back-edge is
                // processed) — `stack` is legitimately accurate here.
            }
            // else: still inside dead code with no known shape yet —
            // stay `terminated`, keep skipping below.
        }
        if terminated {
            pc += 1;
            continue;
        }
        macro_rules! record_edge {
            ($target:expr, $carried:expr) => {{
                let target = $target;
                let carried = $carried;
                match shapes.get(&target) {
                    Some(existing) if *existing == carried => {}
                    Some(_) => return None,
                    None => {
                        shapes.insert(target, carried);
                    }
                }
            }};
        }
        match &chunk.code[pc] {
            Instr::Const(idx) => match &chunk.consts[*idx as usize] {
                crate::value::Value::Int(_) => stack.push(Kind::Int),
                crate::value::Value::Sym(sym) => stack.push(Kind::Callee(*sym)),
                crate::value::Value::Nil => stack.push(Kind::Nil),
                _ => return None,
            },
            Instr::LoadLocal(_) => stack.push(Kind::Int),
            Instr::StoreLocal(_) => {
                if stack.pop() != Some(Kind::Int) {
                    return None;
                }
            }
            Instr::LoadFree(_)
            | Instr::StoreFree(_)
            | Instr::DynBind(_)
            | Instr::DynUnbind(_)
            | Instr::MakeClosure(_)
            | Instr::Interpret(_)
            | Instr::PushFrame
            | Instr::PopFrame
            | Instr::EnvDefine(_)
            | Instr::Car1(_)
            | Instr::Cdr1(_)
            | Instr::Eq2
            | Instr::Not1
            | Instr::Cons2 => return None,
            // Specialized arithmetic opcodes: same typing as the
            // whitelisted builtin calls they replace.
            Instr::Add2(_) | Instr::Sub2(_) | Instr::Mul2(_) => {
                if stack.pop() != Some(Kind::Int) || stack.pop() != Some(Kind::Int) {
                    return None;
                }
                stack.push(Kind::Int);
            }
            Instr::Inc1(_) | Instr::Dec1(_) => {
                if stack.pop() != Some(Kind::Int) {
                    return None;
                }
                stack.push(Kind::Int);
            }
            Instr::Lt2(_) | Instr::Gt2(_) | Instr::Le2(_) | Instr::Ge2(_) | Instr::NumEq2(_) => {
                if stack.pop() != Some(Kind::Int) || stack.pop() != Some(Kind::Int) {
                    return None;
                }
                stack.push(Kind::Bool);
            }
            Instr::Call(argc) => {
                let argc = *argc as usize;
                if stack.len() < argc + 1 {
                    return None;
                }
                let callee_idx = stack.len() - argc - 1;
                let Kind::Callee(sym) = stack[callee_idx] else {
                    return None;
                };
                let name = interp.sym_name(sym);
                for k in &stack[callee_idx + 1..] {
                    if *k != Kind::Int {
                        return None;
                    }
                }
                let result = whitelist_result_kind(name, argc)?;
                stack.truncate(callee_idx);
                stack.push(result);
            }
            Instr::Jump(target) => {
                record_edge!(*target, stack.clone());
                terminated = true;
            }
            Instr::JumpIfNil(target) | Instr::JumpIfNonNil(target) => {
                let cond = stack.pop();
                if cond != Some(Kind::Int) && cond != Some(Kind::Bool) {
                    return None;
                }
                record_edge!(*target, stack.clone());
                record_edge!(pc + 1, stack.clone());
                terminated = true;
            }
            Instr::Pop => {
                stack.pop()?;
            }
            Instr::Dup => {
                let &top = stack.last()?;
                stack.push(top);
            }
            Instr::Return => {
                if stack.pop() != Some(Kind::Int) {
                    return None;
                }
                terminated = true;
            }
        }
        pc += 1;
    }

    // A callee marker must never be "live" across a control-flow edge —
    // it only has meaning as a compile-time tag right next to its Call.
    if shapes
        .values()
        .any(|s| s.iter().any(|k| matches!(k, Kind::Callee(_))))
    {
        return None;
    }
    Some(shapes)
}

fn whitelist_result_kind(name: &str, argc: usize) -> Option<Kind> {
    match (name, argc) {
        ("+", _) | ("*", _) => Some(Kind::Int),
        ("-", n) if n >= 1 => Some(Kind::Int),
        ("/", n) if n >= 2 => Some(Kind::Int),
        ("%", 2) => Some(Kind::Int),
        ("1+", 1) | ("1-", 1) => Some(Kind::Int),
        ("<", n) | (">", n) | ("<=", n) | (">=", n) | ("=", n) if n >= 1 => Some(Kind::Bool),
        ("/=", 2) => Some(Kind::Bool),
        _ => None,
    }
}

/// Cranelift type used to represent a given `Kind` as an i64-sized slot
/// (comparisons produce a narrower boolean-ish value from `icmp`, widened
/// immediately to keep every Int/Bool slot uniformly `I64` — simplest
/// way to avoid ever needing a second width for block parameters).
fn clif_type_for(kind: Kind) -> cranelift_codegen::ir::Type {
    match kind {
        Kind::Int => types::I64,
        // `icmp` produces I8, not I64 — match it exactly rather than
        // trying to widen every comparison result to keep this simple.
        Kind::Bool => types::I8,
        // Nil never actually flows through as a real value (see Slot::
        // NilConst); block params for it are dummy zeros.
        Kind::Nil => types::I64,
        Kind::Callee(_) => unreachable!("Callee never survives analyze_shapes"),
    }
}

struct TranslateCtx<'a, 'b> {
    interp: &'a Interp,
    b: &'a mut FunctionBuilder<'b>,
    chunk: &'a Chunk,
    vars: Vec<Variable>,
    blocks: HashMap<usize, ClifBlock>,
    shapes: HashMap<usize, Vec<Kind>>,
    bail_block: ClifBlock,
}

impl<'a, 'b> TranslateCtx<'a, 'b> {
    fn new(
        interp: &'a Interp,
        b: &'a mut FunctionBuilder<'b>,
        chunk: &'a Chunk,
        arity: usize,
        shapes: HashMap<usize, Vec<Kind>>,
    ) -> Self {
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);

        let args_ptr = b.block_params(entry)[0];
        let out_ok_ptr = b.block_params(entry)[1];

        let mut vars = Vec::with_capacity(chunk.num_locals);
        for i in 0..chunk.num_locals {
            let var = b.declare_var(types::I64);
            vars.push(var);
            if i < arity {
                let off = (i as i32) * 8;
                let v = b.ins().load(
                    types::I64,
                    cranelift_codegen::ir::MemFlagsData::trusted(),
                    args_ptr,
                    off,
                );
                b.def_var(var, v);
            } else {
                let zero = b.ins().iconst(types::I64, 0);
                b.def_var(var, zero);
            }
        }

        let bail_block = b.create_block();

        let mut blocks = HashMap::new();
        blocks.insert(0, entry);
        let _ = out_ok_ptr;
        TranslateCtx {
            interp,
            b,
            chunk,
            vars,
            blocks,
            shapes,
            bail_block,
        }
    }

    /// Convert the top `shape.len()` shadow-stack entries into block
    /// arguments for a jump/branch into a leader with this recorded
    /// shape — `Kind::Nil` slots (which carry no real `Slot::Int`/`Bool`
    /// value) pass a dummy zero, since the receiving side never actually
    /// reads them back as data (see `Slot::NilConst`).
    fn block_args(
        &mut self,
        stack: &[Slot],
        shape: &[Kind],
    ) -> Vec<cranelift_codegen::ir::BlockArg> {
        let start = stack.len() - shape.len();
        stack[start..]
            .iter()
            .map(|s| match s {
                Slot::Int(v) | Slot::Bool(v) => (*v).into(),
                Slot::NilConst | Slot::Callee(_) => self.b.ins().iconst(types::I64, 0).into(),
            })
            .collect()
    }

    /// Returns true if the whole chunk translated successfully. On
    /// failure (ineligible), the caller discards everything built so
    /// far — no partial state escapes since we never call
    /// `finalize_definitions` unless this returns true.
    fn run(&mut self) -> bool {
        let r = self.run_inner();
        if !r && std::env::var("JIT_DEBUG").is_ok() {
            eprintln!("jit: ineligible");
        }
        r
    }

    fn run_inner(&mut self) -> bool {
        let out_ok_ptr = self.b.block_params(self.entry_block())[1];
        let leaders = compute_leaders(&self.chunk.code);
        for &idx in &leaders {
            if idx != 0 {
                let blk = self.b.create_block();
                if let Some(shape) = self.shapes.get(&idx) {
                    for &kind in shape {
                        self.b.append_block_param(blk, clif_type_for(kind));
                    }
                }
                self.blocks.insert(idx, blk);
            }
        }

        let mut stack: Vec<Slot> = Vec::new();
        let mut pc = 0usize;
        let mut terminated = false;
        while pc < self.chunk.code.len() {
            if let Some(&blk) = self.blocks.get(&pc) {
                if pc != 0 {
                    // Every leader's shape was fixed by `analyze_shapes`
                    // (block params were declared for it above); pass
                    // the current top-of-stack as arguments — Cranelift
                    // then hands them back via `block_params(blk)` once
                    // we switch in, which correctly dominates everything
                    // in and after this block regardless of which edge
                    // was actually taken at runtime (this is exactly how
                    // a stack-machine-to-SSA translation is supposed to
                    // turn "whatever's on top of the stack" into a
                    // proper phi node at merge points).
                    let shape = self.shapes.get(&pc).cloned().unwrap_or_default();
                    if stack.len() < shape.len() {
                        return false;
                    }
                    let args = self.block_args(&stack, &shape);
                    if !terminated {
                        self.b.ins().jump(blk, &args);
                    }
                    stack.truncate(stack.len() - shape.len());
                    self.b.switch_to_block(blk);
                    let params = self.b.block_params(blk).to_vec();
                    for (kind, val) in shape.iter().zip(params) {
                        stack.push(match kind {
                            Kind::Int => Slot::Int(val),
                            Kind::Bool => Slot::Bool(val),
                            Kind::Nil => Slot::NilConst,
                            Kind::Callee(_) => unreachable!("rejected by analyze_shapes"),
                        });
                    }
                    terminated = false;
                }
            }
            if terminated {
                // Dead code between a terminator and the next leader
                // (shouldn't happen with our own compiler's output, but
                // skip safely rather than emit into a finished block).
                pc += 1;
                continue;
            }
            if std::env::var("JIT_DEBUG").is_ok() {
                eprintln!("jit: pc={} instr={:?}", pc, self.chunk.code[pc]);
            }
            match self.chunk.code[pc] {
                Instr::Const(idx) => match &self.chunk.consts[idx as usize] {
                    crate::value::Value::Int(n) => {
                        let v = self.b.ins().iconst(types::I64, *n);
                        stack.push(Slot::Int(v));
                    }
                    crate::value::Value::Sym(id) => stack.push(Slot::Callee(*id)),
                    crate::value::Value::Nil => stack.push(Slot::NilConst),
                    _ => return false,
                },
                Instr::LoadLocal(slot) => {
                    let v = self.b.use_var(self.vars[slot as usize]);
                    stack.push(Slot::Int(v));
                }
                Instr::StoreLocal(slot) => {
                    let Some(v) = pop_int(&mut stack) else {
                        return false;
                    };
                    self.b.def_var(self.vars[slot as usize], v);
                }
                Instr::LoadFree(_)
                | Instr::StoreFree(_)
                | Instr::DynBind(_)
                | Instr::DynUnbind(_)
                | Instr::MakeClosure(_)
                | Instr::Interpret(_)
                | Instr::PushFrame
                | Instr::PopFrame
                | Instr::EnvDefine(_)
                | Instr::Car1(_)
                | Instr::Cdr1(_)
                | Instr::Eq2
                | Instr::Not1
                | Instr::Cons2 => return false,
                Instr::Add2(_) => {
                    let Some(y) = pop_int(&mut stack) else {
                        return false;
                    };
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let r = self.checked_binop(x, y, |b, a, c| b.ins().sadd_overflow(a, c));
                    stack.push(Slot::Int(r));
                }
                Instr::Sub2(_) => {
                    let Some(y) = pop_int(&mut stack) else {
                        return false;
                    };
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let r = self.checked_binop(x, y, |b, a, c| b.ins().ssub_overflow(a, c));
                    stack.push(Slot::Int(r));
                }
                Instr::Mul2(_) => {
                    let Some(y) = pop_int(&mut stack) else {
                        return false;
                    };
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let r = self.checked_binop(x, y, |b, a, c| b.ins().smul_overflow(a, c));
                    stack.push(Slot::Int(r));
                }
                Instr::Inc1(_) => {
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let one = self.b.ins().iconst(types::I64, 1);
                    let r = self.checked_binop(x, one, |b, a, c| b.ins().sadd_overflow(a, c));
                    stack.push(Slot::Int(r));
                }
                Instr::Dec1(_) => {
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let one = self.b.ins().iconst(types::I64, 1);
                    let r = self.checked_binop(x, one, |b, a, c| b.ins().ssub_overflow(a, c));
                    stack.push(Slot::Int(r));
                }
                Instr::Lt2(_)
                | Instr::Gt2(_)
                | Instr::Le2(_)
                | Instr::Ge2(_)
                | Instr::NumEq2(_) => {
                    let Some(y) = pop_int(&mut stack) else {
                        return false;
                    };
                    let Some(x) = pop_int(&mut stack) else {
                        return false;
                    };
                    let cc = match self.chunk.code[pc] {
                        Instr::Lt2(_) => IntCC::SignedLessThan,
                        Instr::Gt2(_) => IntCC::SignedGreaterThan,
                        Instr::Le2(_) => IntCC::SignedLessThanOrEqual,
                        Instr::Ge2(_) => IntCC::SignedGreaterThanOrEqual,
                        _ => IntCC::Equal,
                    };
                    let r = self.b.ins().icmp(cc, x, y);
                    stack.push(Slot::Bool(r));
                }
                Instr::Call(argc) => {
                    if !self.translate_call(&mut stack, argc as usize) {
                        return false;
                    }
                }
                Instr::Jump(target) => {
                    let Some(&blk) = self.blocks.get(&target) else {
                        return false;
                    };
                    let Some(shape) = self.shapes.get(&target).cloned() else {
                        return false;
                    };
                    if stack.len() < shape.len() {
                        return false;
                    }
                    let args = self.block_args(&stack, &shape);
                    self.b.ins().jump(blk, &args);
                    terminated = true;
                }
                Instr::JumpIfNil(target) => {
                    let Some(&blk) = self.blocks.get(&target) else {
                        return false;
                    };
                    let Some(&fall) = self.blocks.get(&(pc + 1)) else {
                        return false;
                    };
                    let Some(cond) = pop_bool_or_int(&mut stack) else {
                        return false;
                    };
                    let Some(target_shape) = self.shapes.get(&target).cloned() else {
                        return false;
                    };
                    let Some(fall_shape) = self.shapes.get(&(pc + 1)).cloned() else {
                        return false;
                    };
                    if stack.len() < target_shape.len() || stack.len() < fall_shape.len() {
                        return false;
                    }
                    let target_args = self.block_args(&stack, &target_shape);
                    let fall_args = self.block_args(&stack, &fall_shape);
                    self.b.ins().brif(cond, fall, &fall_args, blk, &target_args);
                    terminated = true;
                }
                Instr::JumpIfNonNil(target) => {
                    let Some(&blk) = self.blocks.get(&target) else {
                        return false;
                    };
                    let Some(&fall) = self.blocks.get(&(pc + 1)) else {
                        return false;
                    };
                    let Some(cond) = pop_bool_or_int(&mut stack) else {
                        return false;
                    };
                    let Some(target_shape) = self.shapes.get(&target).cloned() else {
                        return false;
                    };
                    let Some(fall_shape) = self.shapes.get(&(pc + 1)).cloned() else {
                        return false;
                    };
                    if stack.len() < target_shape.len() || stack.len() < fall_shape.len() {
                        return false;
                    }
                    let target_args = self.block_args(&stack, &target_shape);
                    let fall_args = self.block_args(&stack, &fall_shape);
                    self.b.ins().brif(cond, blk, &target_args, fall, &fall_args);
                    terminated = true;
                }
                Instr::Pop => {
                    if stack.pop().is_none() {
                        return false;
                    }
                }
                Instr::Dup => {
                    let Some(&top) = stack.last() else {
                        return false;
                    };
                    stack.push(top);
                }
                Instr::Return => {
                    let Some(v) = pop_int(&mut stack) else {
                        return false;
                    };
                    let one = self.b.ins().iconst(types::I8, 1);
                    self.b.ins().store(
                        cranelift_codegen::ir::MemFlagsData::trusted(),
                        one,
                        out_ok_ptr,
                        0,
                    );
                    self.b.ins().return_(&[v]);
                    terminated = true;
                }
            }
            pc += 1;
        }
        if !terminated {
            // Chunks always end with Return (the compiler guarantees
            // this), but bail out defensively rather than build an
            // unterminated function if that ever changes.
            return false;
        }

        // Bail-out block: any branch here (overflow / div-by-zero)
        // reports failure and returns a dummy value.
        self.b.switch_to_block(self.bail_block);
        let zero8 = self.b.ins().iconst(types::I8, 0);
        self.b.ins().store(
            cranelift_codegen::ir::MemFlagsData::trusted(),
            zero8,
            out_ok_ptr,
            0,
        );
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.ins().return_(&[zero]);

        self.b.seal_all_blocks();
        true
    }

    fn entry_block(&self) -> ClifBlock {
        self.blocks[&0]
    }

    fn translate_call(&mut self, stack: &mut Vec<Slot>, argc: usize) -> bool {
        if stack.len() < argc + 1 {
            return false;
        }
        let args_start = stack.len() - argc;
        let callee_idx = args_start - 1;
        let Slot::Callee(id) = stack[callee_idx] else {
            return false;
        };
        let name = self.interp.sym_name(id);
        let args: Vec<Slot> = stack.split_off(args_start);
        stack.pop(); // the callee marker itself

        let mut ints = Vec::with_capacity(args.len());
        for a in &args {
            match a {
                Slot::Int(v) => ints.push(*v),
                _ => return false,
            }
        }

        let result = match (name, ints.len()) {
            ("+", 0) => Slot::Int(self.b.ins().iconst(types::I64, 0)),
            ("+", _) => Slot::Int(self.fold_checked(&ints, |b, x, y| b.ins().sadd_overflow(x, y))),
            ("*", 0) => Slot::Int(self.b.ins().iconst(types::I64, 1)),
            ("*", _) => Slot::Int(self.fold_checked(&ints, |b, x, y| b.ins().smul_overflow(x, y))),
            ("-", 1) => {
                let zero = self.b.ins().iconst(types::I64, 0);
                Slot::Int(self.checked_binop(zero, ints[0], |b, x, y| b.ins().ssub_overflow(x, y)))
            }
            ("-", _) => Slot::Int(self.fold_checked(&ints, |b, x, y| b.ins().ssub_overflow(x, y))),
            ("/", n) if n >= 2 => Slot::Int(self.fold_div(&ints)),
            ("%", 2) => Slot::Int(self.checked_rem(ints[0], ints[1])),
            ("1+", 1) => {
                let one = self.b.ins().iconst(types::I64, 1);
                Slot::Int(self.checked_binop(ints[0], one, |b, x, y| b.ins().sadd_overflow(x, y)))
            }
            ("1-", 1) => {
                let one = self.b.ins().iconst(types::I64, 1);
                Slot::Int(self.checked_binop(ints[0], one, |b, x, y| b.ins().ssub_overflow(x, y)))
            }
            ("<", n) if n >= 1 => Slot::Bool(self.chain_cmp(&ints, IntCC::SignedLessThan)),
            (">", n) if n >= 1 => Slot::Bool(self.chain_cmp(&ints, IntCC::SignedGreaterThan)),
            ("<=", n) if n >= 1 => Slot::Bool(self.chain_cmp(&ints, IntCC::SignedLessThanOrEqual)),
            (">=", n) if n >= 1 => {
                Slot::Bool(self.chain_cmp(&ints, IntCC::SignedGreaterThanOrEqual))
            }
            ("=", n) if n >= 1 => Slot::Bool(self.chain_cmp(&ints, IntCC::Equal)),
            ("/=", 2) => Slot::Bool(self.b.ins().icmp(IntCC::NotEqual, ints[0], ints[1])),
            _ => return false,
        };
        stack.push(result);
        true
    }

    /// `iadd_overflow`-style fold: left to right, bailing on the first
    /// overflow.
    fn fold_checked(
        &mut self,
        vals: &[ClifValue],
        op: impl Fn(&mut FunctionBuilder<'b>, ClifValue, ClifValue) -> (ClifValue, ClifValue) + Copy,
    ) -> ClifValue {
        let mut acc = vals[0];
        for &v in &vals[1..] {
            acc = self.checked_binop(acc, v, op);
        }
        acc
    }

    fn checked_binop(
        &mut self,
        x: ClifValue,
        y: ClifValue,
        op: impl Fn(&mut FunctionBuilder<'b>, ClifValue, ClifValue) -> (ClifValue, ClifValue),
    ) -> ClifValue {
        let (result, overflow) = op(self.b, x, y);
        let cont = self.b.create_block();
        self.b.append_block_param(cont, types::I64);
        self.b
            .ins()
            .brif(overflow, self.bail_block, &[], cont, &[result.into()]);
        self.b.switch_to_block(cont);
        self.b.block_params(cont)[0]
    }

    fn fold_div(&mut self, vals: &[ClifValue]) -> ClifValue {
        let mut acc = vals[0];
        for &v in &vals[1..] {
            acc = self.checked_div(acc, v);
        }
        acc
    }

    fn checked_div(&mut self, x: ClifValue, y: ClifValue) -> ClifValue {
        let zero = self.b.ins().iconst(types::I64, 0);
        let is_zero = self.b.ins().icmp(IntCC::Equal, y, zero);
        let cont = self.b.create_block();
        self.b.append_block_param(cont, types::I64);
        let safe = self.b.create_block();
        self.b.ins().brif(is_zero, self.bail_block, &[], safe, &[]);
        self.b.switch_to_block(safe);
        let q = self.b.ins().sdiv(x, y);
        self.b.ins().jump(cont, &[q.into()]);
        self.b.switch_to_block(cont);
        self.b.block_params(cont)[0]
    }

    fn checked_rem(&mut self, x: ClifValue, y: ClifValue) -> ClifValue {
        let zero = self.b.ins().iconst(types::I64, 0);
        let is_zero = self.b.ins().icmp(IntCC::Equal, y, zero);
        let cont = self.b.create_block();
        self.b.append_block_param(cont, types::I64);
        let safe = self.b.create_block();
        self.b.ins().brif(is_zero, self.bail_block, &[], safe, &[]);
        self.b.switch_to_block(safe);
        let r = self.b.ins().srem(x, y);
        self.b.ins().jump(cont, &[r.into()]);
        self.b.switch_to_block(cont);
        self.b.block_params(cont)[0]
    }

    /// Emacs-style chained comparison: `(< a b c)` means `a<b && b<c`.
    fn chain_cmp(&mut self, vals: &[ClifValue], cc: IntCC) -> ClifValue {
        if vals.len() == 1 {
            return self.b.ins().iconst(types::I8, 1);
        }
        let mut acc = self.b.ins().icmp(cc, vals[0], vals[1]);
        for w in vals.windows(2).skip(1) {
            let step = self.b.ins().icmp(cc, w[0], w[1]);
            acc = self.b.ins().band(acc, step);
        }
        acc
    }
}

fn pop_int(stack: &mut Vec<Slot>) -> Option<ClifValue> {
    match stack.pop() {
        Some(Slot::Int(v)) => Some(v),
        _ => None,
    }
}

fn pop_bool_or_int(stack: &mut Vec<Slot>) -> Option<ClifValue> {
    match stack.pop() {
        Some(Slot::Int(v)) | Some(Slot::Bool(v)) => Some(v),
        _ => None,
    }
}

/// Standard "leader" computation for turning flat, jump-based bytecode
/// into basic blocks: a new block starts at index 0, at every jump
/// target, and right after every branch/jump instruction.
fn compute_leaders(code: &[Instr]) -> Vec<usize> {
    let mut set = HashSet::new();
    set.insert(0);
    for (i, instr) in code.iter().enumerate() {
        match instr {
            Instr::Jump(t) => {
                set.insert(*t);
                set.insert(i + 1);
            }
            Instr::JumpIfNil(t) | Instr::JumpIfNonNil(t) => {
                set.insert(*t);
                set.insert(i + 1);
            }
            _ => {}
        }
    }
    let mut v: Vec<usize> = set.into_iter().filter(|&i| i < code.len()).collect();
    v.sort_unstable();
    v
}
