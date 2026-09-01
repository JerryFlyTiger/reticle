//! Compiles elisp forms into `bytecode::Chunk`s.
//!
//! Scope resolution: each compiled function tracks its OWN locals (params
//! plus every `let`/`let*` binding anywhere in its body) as a flat,
//! never-reused slot array, addressed by index — no alist scan at
//! runtime. A symbol reference that isn't one of this function's own
//! locals compiles to `LoadFree`/`StoreFree`, which walks the closure's
//! captured environment and then the global/dynamic cell — identical
//! logic to the tree-walker's `eval_symbol`/`set_symbol` (literally the
//! same functions), so the two evaluation strategies never disagree on
//! what a free variable resolves to.
//!
//! Forms the compiler doesn't specialize (condition-case, catch,
//! unwind-protect, defvar, defmacro, backquote, ...) compile to a single
//! `Interpret` instruction that re-enters the tree-walking `eval()` with
//! a synthetic environment exposing this function's current locals.

use std::rc::Rc;

use crate::bytecode::{Chunk, ClosureInfo, Instr, InterpretInfo};
use crate::error::Flow;
use crate::eval::{apply_lambda, parse_lambda_form};
use crate::interp::Interp;
use crate::value::{CompiledFn, Function, LexEnv, ParamSpec, SymId, Value};

pub struct Compiler<'a> {
    interp: &'a mut Interp,
    code: Vec<Instr>,
    consts: Vec<Value>,
    interprets: Vec<InterpretInfo>,
    closures: Vec<ClosureInfo>,
    num_locals: usize,
    /// Compile-time scope of THIS function's locals, innermost last.
    /// Flat-slot locals resolve to indexed slots; env-resident locals
    /// (captured by some nested lambda or interpreted fallback form)
    /// live in real `LexEnv` frames and resolve through the chain, so a
    /// closure and its defining function share one binding.
    scope: Vec<(SymId, Binding)>,
    /// Symbols that may be referenced from a nested lambda / fallback
    /// form (conservative over-approximation, see `analyze_captured`):
    /// any local with one of these names must be env-resident.
    captured: std::collections::HashSet<SymId>,
    /// This function's binding mode, captured once (matches `Lambda.lexical`).
    lexical: bool,
}

#[derive(Clone, Copy)]
enum Binding {
    Slot(u16),
    Env,
}

/// Compile `(lambda (params...) body...)`-shaped `form` into a callable
/// compiled function. `env` is the captured environment to install
/// (`None` for a fresh top-level `defun`; the original closure's `env`
/// when re-compiling an existing `Lambda` via `byte-compile`).
pub fn compile_lambda(
    interp: &mut Interp,
    form: &Value,
    env: Option<Rc<LexEnv>>,
) -> Result<Rc<CompiledFn>, Flow> {
    let (params, body, interactive) = parse_lambda_form(interp, form)?;
    let lexical = interp.lexical_binding;
    compile_parsed(interp, params, body, interactive, lexical, env)
}

/// Compile an already-parsed function (params/body already split out —
/// this is exactly the shape `Lambda` stores, so `byte-compile` can go
/// straight from an existing interpreted closure to bytecode without
/// reconstructing its original source form). `lexical` must match the
/// binding mode the function was originally defined under — callers
/// re-compiling an existing `Lambda` must pass its own `.lexical` flag,
/// not the interpreter's current global setting, since those can differ.
pub fn compile_parsed(
    interp: &mut Interp,
    params: ParamSpec,
    body: Value,
    interactive: Option<Value>,
    lexical: bool,
    env: Option<Rc<LexEnv>>,
) -> Result<Rc<CompiledFn>, Flow> {
    // Which names might be referenced from a nested lambda (or a form
    // that falls back to the tree-walker)? Locals with those names must
    // live in shared LexEnv frames, not flat slots, so that closures
    // capture the *variable* rather than a snapshot of its value.
    let captured = analyze_captured(interp, &body)?;

    let mut c = Compiler {
        interp,
        code: Vec::new(),
        consts: Vec::new(),
        interprets: Vec::new(),
        closures: Vec::new(),
        num_locals: 0,
        scope: Vec::new(),
        captured,
        lexical,
    };

    // Bind params: lexical params get slots (in call order); dynamic
    // params (special vars, or any param when this function itself binds
    // dynamically) are bound at entry via DynBind and never get a slot,
    // so every reference to them naturally falls through to LoadFree.
    let mut param_slots = Vec::new();
    for &id in params.required.iter().chain(params.optional.iter()) {
        param_slots.push((id, c.bind_incoming(id)));
    }
    if let Some(id) = params.rest {
        param_slots.push((id, c.bind_incoming(id)));
    }

    // Captured params: the VM still delivers them into flat slots, but
    // all further access goes through a shared entry frame the closure
    // chain can see. Copy them over in the prologue.
    let captured_params: Vec<(SymId, u16)> = param_slots
        .iter()
        .filter_map(|(id, slot)| match slot {
            Some(s) if c.captured.contains(id) => Some((*id, *s)),
            _ => None,
        })
        .collect();
    if !captured_params.is_empty() {
        c.emit(Instr::PushFrame);
        for (id, slot) in &captured_params {
            c.emit(Instr::LoadLocal(*slot));
            c.emit(Instr::EnvDefine(*id));
            // Re-resolve this name to the frame from here on (pushed
            // after the Slot entry, so it shadows it).
            c.scope.push((*id, Binding::Env));
        }
    }

    let body_forms = body
        .list_to_vec()
        .ok_or_else(|| c.interp.error("malformed function body"))?;
    c.compile_progn(&body_forms)?;
    c.emit(Instr::Return);

    let t_sym = c.interp.syms.t;
    let mut consts = c.consts;
    let code = crate::peephole::optimize(c.code, &mut consts, t_sym);
    let chunk = Chunk {
        code,
        consts,
        interprets: c.interprets,
        closures: c.closures,
        num_locals: c.num_locals,
    };
    Ok(Rc::new(CompiledFn {
        params,
        chunk: Rc::new(chunk),
        env,
        lexical,
        name: std::cell::RefCell::new(None),
        interactive: std::cell::RefCell::new(interactive),
        calls: std::cell::Cell::new(0),
        jit_tried: std::cell::Cell::new(false),
    }))
}

impl<'a> Compiler<'a> {
    /// Register an incoming parameter: allocates a slot for it if it
    /// binds lexically, or just marks that the VM must DynBind it at
    /// call time. Returns the slot for lexical params (VM param-binding
    /// code looks at `param_slots` built by the caller for the rest).
    fn bind_incoming(&mut self, id: SymId) -> Option<u16> {
        if self.lexical && !self.interp.symbols[id as usize].special {
            let slot = self.alloc_local();
            self.scope.push((id, Binding::Slot(slot)));
            Some(slot)
        } else {
            None
        }
    }

    fn alloc_local(&mut self) -> u16 {
        let slot = self.num_locals as u16;
        self.num_locals += 1;
        slot
    }

    /// Innermost binding of `id` in this function, if any. `Env`
    /// bindings compile to chain lookups (LoadFree/StoreFree), which is
    /// what makes them shared with closures.
    fn resolve_binding(&self, id: SymId) -> Option<Binding> {
        self.scope
            .iter()
            .rev()
            .find(|(s, _)| *s == id)
            .map(|(_, b)| *b)
    }

    fn resolve_local(&self, id: SymId) -> Option<u16> {
        match self.resolve_binding(id) {
            Some(Binding::Slot(s)) => Some(s),
            _ => None,
        }
    }

    fn add_const(&mut self, v: Value) -> u32 {
        self.consts.push(v);
        (self.consts.len() - 1) as u32
    }

    fn emit(&mut self, i: Instr) -> usize {
        self.code.push(i);
        self.code.len() - 1
    }

    /// Rewrite a previously emitted jump's target to `target`.
    fn patch_jump(&mut self, at: usize, target: usize) {
        self.code[at] = match self.code[at] {
            Instr::Jump(_) => Instr::Jump(target),
            Instr::JumpIfNil(_) => Instr::JumpIfNil(target),
            Instr::JumpIfNonNil(_) => Instr::JumpIfNonNil(target),
            other => other,
        };
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    /// Compile a sequence of forms, discarding all but the last value.
    /// An empty sequence compiles to nil.
    fn compile_progn(&mut self, forms: &[Value]) -> Result<(), Flow> {
        if forms.is_empty() {
            let idx = self.add_const(Value::Nil);
            self.emit(Instr::Const(idx));
            return Ok(());
        }
        for (i, f) in forms.iter().enumerate() {
            self.compile_form(f)?;
            if i + 1 < forms.len() {
                self.emit(Instr::Pop);
            }
        }
        Ok(())
    }

    /// Compile one form, leaving its value on the stack.
    fn compile_form(&mut self, form: &Value) -> Result<(), Flow> {
        match form {
            Value::Nil
            | Value::Int(_)
            | Value::Big(_)
            | Value::Float(_)
            | Value::Str(_)
            | Value::Vector(_)
            | Value::HashTable(_)
            | Value::Func(_)
            | Value::Ext(_) => {
                let idx = self.add_const(form.clone());
                self.emit(Instr::Const(idx));
                Ok(())
            }
            Value::Sym(id) => {
                if let Some(slot) = self.resolve_local(*id) {
                    self.emit(Instr::LoadLocal(slot));
                } else {
                    self.emit(Instr::LoadFree(*id));
                }
                Ok(())
            }
            Value::Cons(_) => self.compile_compound(form),
        }
    }

    fn compile_compound(&mut self, form: &Value) -> Result<(), Flow> {
        let head = form.car();
        let args = form.cdr();
        if let Some(id) = self.interp.as_sym(&head) {
            let name = self.interp.sym_name(id).to_string();
            if let Some(()) = self.try_compile_special(&name, id, &args)? {
                return Ok(());
            }
            // Special forms we don't specialize (rare in hot loops, or
            // needing non-local-exit machinery the VM doesn't have):
            // fall back to the tree-walker rather than misreading their
            // unevaluated argument syntax as an ordinary call.
            if matches!(
                name.as_str(),
                "defvar"
                    | "defconst"
                    | "defun"
                    | "defmacro"
                    | "condition-case"
                    | "unwind-protect"
                    | "catch"
                    | "interactive"
                    | "`"
            ) {
                return self.fallback_interpret(form);
            }
            // Macro? Expand at compile time and recurse on the result.
            if let Some(Value::Func(f)) = self.interp.symbols[id as usize].function.clone() {
                if let Function::Lambda(l) = f.as_ref() {
                    if l.is_macro {
                        let unevaled = args
                            .list_to_vec()
                            .ok_or_else(|| self.interp.error("malformed macro call"))?;
                        let expansion = apply_lambda(self.interp, l.clone(), unevaled.into())?;
                        return self.compile_form(&expansion);
                    }
                }
            }
            // Core primitive with a dedicated opcode? Only when the
            // symbol currently resolves to the builtin (a user
            // redefinition compiles as an ordinary call instead), and
            // semantics freeze at compile time like GNU Emacs bytecode.
            if let Some(()) = self.try_compile_fast_op(id, &name, &args)? {
                return Ok(());
            }
            // Ordinary call: push the callee symbol (resolved at call
            // time, so redefinition is picked up exactly like today),
            // then args, then Call.
            let cidx = self.add_const(Value::Sym(id));
            self.emit(Instr::Const(cidx));
            self.compile_call_args(&args)
        } else if matches!(head, Value::Cons(_)) {
            // ((lambda ...) args...) or any other computed-function call.
            self.compile_form(&head)?;
            self.compile_call_args(&args)
        } else {
            // Malformed call head; let the tree-walker produce the
            // correct wrong-type-argument error.
            self.fallback_interpret(form)
        }
    }

    /// Emit a dedicated opcode for a core primitive call, if `id`
    /// still resolves to the expected builtin and the arity matches.
    /// Ok(Some(())) when handled.
    fn try_compile_fast_op(
        &mut self,
        id: SymId,
        name: &str,
        args: &Value,
    ) -> Result<Option<()>, Flow> {
        // Only these names, at these arities, with the ORIGINAL builtin
        // still installed.
        let items = match args.list_to_vec() {
            Some(v) => v,
            None => return Ok(None),
        };
        let n = items.len();
        let wanted = matches!(
            (name, n),
            ("+", 2)
                | ("-", 2)
                | ("*", 2)
                | ("<", 2)
                | (">", 2)
                | ("<=", 2)
                | (">=", 2)
                | ("=", 2)
                | ("1+", 1)
                | ("1-", 1)
                | ("car", 1)
                | ("cdr", 1)
                | ("eq", 2)
                | ("not", 1)
                | ("null", 1)
                | ("cons", 2)
        );
        if !wanted {
            return Ok(None);
        }
        let builtin = match &self.interp.symbols[id as usize].function {
            Some(v @ Value::Func(f)) if matches!(f.as_ref(), Function::Builtin { .. }) => v.clone(),
            _ => return Ok(None),
        };
        for a in &items {
            self.compile_form(a)?;
        }
        let instr = match name {
            "+" => Instr::Add2(self.add_const(builtin)),
            "-" => Instr::Sub2(self.add_const(builtin)),
            "*" => Instr::Mul2(self.add_const(builtin)),
            "<" => Instr::Lt2(self.add_const(builtin)),
            ">" => Instr::Gt2(self.add_const(builtin)),
            "<=" => Instr::Le2(self.add_const(builtin)),
            ">=" => Instr::Ge2(self.add_const(builtin)),
            "=" => Instr::NumEq2(self.add_const(builtin)),
            "1+" => Instr::Inc1(self.add_const(builtin)),
            "1-" => Instr::Dec1(self.add_const(builtin)),
            "car" => Instr::Car1(self.add_const(builtin)),
            "cdr" => Instr::Cdr1(self.add_const(builtin)),
            "eq" => Instr::Eq2,
            "not" | "null" => Instr::Not1,
            "cons" => Instr::Cons2,
            _ => unreachable!(),
        };
        self.emit(instr);
        Ok(Some(()))
    }

    fn compile_call_args(&mut self, args: &Value) -> Result<(), Flow> {
        let items = args
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed argument list"))?;
        let n = items.len();
        for a in &items {
            self.compile_form(a)?;
        }
        self.emit(Instr::Call(n as u16));
        Ok(())
    }

    /// Try to compile `name` as a special form. Ok(Some(())) if handled.
    fn try_compile_special(
        &mut self,
        name: &str,
        _id: SymId,
        args: &Value,
    ) -> Result<Option<()>, Flow> {
        match name {
            "quote" => {
                let idx = self.add_const(args.car());
                self.emit(Instr::Const(idx));
            }
            "function" => {
                let arg = args.car();
                if let Value::Cons(c) = &arg {
                    let ihead = c.borrow().car.clone();
                    if self.interp.as_sym(&ihead) == Some(self.interp.syms.lambda) {
                        let lform = c.borrow().cdr.clone();
                        self.compile_lambda_literal(&lform)?;
                        return Ok(Some(()));
                    }
                }
                let idx = self.add_const(arg);
                self.emit(Instr::Const(idx));
            }
            "lambda" => self.compile_lambda_literal(args)?,
            "if" => self.compile_if(args)?,
            "cond" => self.compile_cond(args)?,
            "while" => self.compile_while(args)?,
            "progn" => {
                let forms = args
                    .list_to_vec()
                    .ok_or_else(|| self.interp.error("malformed progn"))?;
                self.compile_progn(&forms)?;
            }
            "prog1" => {
                self.compile_form(&args.car())?;
                let rest = args
                    .cdr()
                    .list_to_vec()
                    .ok_or_else(|| self.interp.error("malformed prog1"))?;
                for f in &rest {
                    self.compile_form(f)?;
                    self.emit(Instr::Pop);
                }
            }
            "prog2" => {
                self.compile_form(&args.car())?;
                self.emit(Instr::Pop);
                self.compile_form(&args.cdr().car())?;
                let rest = args
                    .cdr()
                    .cdr()
                    .list_to_vec()
                    .ok_or_else(|| self.interp.error("malformed prog2"))?;
                for f in &rest {
                    self.compile_form(f)?;
                    self.emit(Instr::Pop);
                }
            }
            "and" => self.compile_and(args)?,
            "or" => self.compile_or(args)?,
            "let" => self.compile_let(args, false)?,
            "let*" => self.compile_let(args, true)?,
            "setq" => self.compile_setq(args)?,
            _ => return Ok(None),
        }
        Ok(Some(()))
    }

    fn compile_if(&mut self, args: &Value) -> Result<(), Flow> {
        self.compile_form(&args.car())?;
        let jump_to_else = self.emit(Instr::JumpIfNil(0));
        self.compile_form(&args.cdr().car())?;
        let jump_to_end = self.emit(Instr::Jump(0));
        let else_start = self.here();
        let else_forms = args
            .cdr()
            .cdr()
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed if"))?;
        self.compile_progn(&else_forms)?;
        let end = self.here();
        self.patch_jump(jump_to_else, else_start);
        self.patch_jump(jump_to_end, end);
        Ok(())
    }

    fn compile_cond(&mut self, args: &Value) -> Result<(), Flow> {
        let clauses = args
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed cond"))?;
        let mut end_jumps = Vec::new();
        for clause in &clauses {
            self.compile_form(&clause.car())?;
            self.emit(Instr::Dup);
            let to_next = self.emit(Instr::JumpIfNil(0));
            // Non-nil path: stack = [test].
            let body = clause
                .cdr()
                .list_to_vec()
                .ok_or_else(|| self.interp.error("malformed cond clause"))?;
            if body.is_empty() {
                end_jumps.push(self.emit(Instr::Jump(0)));
            } else {
                self.emit(Instr::Pop);
                self.compile_progn(&body)?;
                end_jumps.push(self.emit(Instr::Jump(0)));
            }
            let next = self.here();
            self.patch_jump(to_next, next);
            self.emit(Instr::Pop); // discard the leftover nil test value
        }
        let idx = self.add_const(Value::Nil);
        self.emit(Instr::Const(idx));
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Ok(())
    }

    fn compile_while(&mut self, args: &Value) -> Result<(), Flow> {
        let loop_start = self.here();
        self.compile_form(&args.car())?;
        let exit = self.emit(Instr::JumpIfNil(0));
        let body = args
            .cdr()
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed while"))?;
        self.compile_progn(&body)?;
        self.emit(Instr::Pop);
        self.emit(Instr::Jump(loop_start));
        let end = self.here();
        self.patch_jump(exit, end);
        let idx = self.add_const(Value::Nil);
        self.emit(Instr::Const(idx));
        Ok(())
    }

    fn compile_and(&mut self, args: &Value) -> Result<(), Flow> {
        let forms = args
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed and"))?;
        if forms.is_empty() {
            let idx = self.add_const(Value::Sym(self.interp.syms.t));
            self.emit(Instr::Const(idx));
            return Ok(());
        }
        let mut end_jumps = Vec::new();
        for (i, f) in forms.iter().enumerate() {
            self.compile_form(f)?;
            if i + 1 < forms.len() {
                self.emit(Instr::Dup);
                end_jumps.push(self.emit(Instr::JumpIfNil(0)));
                self.emit(Instr::Pop);
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Ok(())
    }

    fn compile_or(&mut self, args: &Value) -> Result<(), Flow> {
        let forms = args
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed or"))?;
        if forms.is_empty() {
            let idx = self.add_const(Value::Nil);
            self.emit(Instr::Const(idx));
            return Ok(());
        }
        let mut end_jumps = Vec::new();
        for (i, f) in forms.iter().enumerate() {
            self.compile_form(f)?;
            if i + 1 < forms.len() {
                self.emit(Instr::Dup);
                end_jumps.push(self.emit(Instr::JumpIfNonNil(0)));
                self.emit(Instr::Pop);
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch_jump(j, end);
        }
        Ok(())
    }

    /// Whether `id` should be a fast lexical slot here, or must go
    /// through the dynamic path (special variable, or this whole
    /// function binds dynamically).
    fn binds_lexically(&self, id: SymId) -> bool {
        self.lexical && !self.interp.symbols[id as usize].special
    }

    fn compile_let(&mut self, args: &Value, star: bool) -> Result<(), Flow> {
        let bindings = args
            .car()
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed let bindings"))?;
        let saved_scope_len = self.scope.len();
        let mut dyn_count: u16 = 0;

        // A binding some nested lambda (or fallback form) may reference
        // must live in a real frame the closure chain shares. One fresh
        // frame per let that needs it — mirroring the tree-walker's
        // frame-per-let, so scoping/shadowing semantics line up exactly.
        let needs_frame = bindings.iter().any(|b| {
            matches!(self.binding_id(b), Some(id)
                if self.binds_lexically(id) && self.captured.contains(&id))
        });
        let mut frame_pushed = false;

        if star {
            if needs_frame {
                self.emit(Instr::PushFrame);
                frame_pushed = true;
            }
            for b in &bindings {
                let (id, val_form) = self.binding_parts(b)?;
                self.compile_form(&val_form)?;
                if self.binds_lexically(id) {
                    if self.captured.contains(&id) {
                        self.emit(Instr::EnvDefine(id));
                        self.scope.push((id, Binding::Env));
                    } else {
                        let slot = self.alloc_local();
                        self.emit(Instr::StoreLocal(slot));
                        self.scope.push((id, Binding::Slot(slot)));
                    }
                } else {
                    self.emit(Instr::DynBind(id));
                    dyn_count += 1;
                }
            }
        } else {
            // Evaluate all values first (in the outer scope), then bind.
            let mut ids = Vec::with_capacity(bindings.len());
            for b in &bindings {
                let (id, val_form) = self.binding_parts(b)?;
                self.compile_form(&val_form)?;
                ids.push(id);
            }
            // The frame appears only after all values are computed, so
            // closures made in the value forms don't see these bindings
            // (matching plain-let semantics).
            if needs_frame {
                self.emit(Instr::PushFrame);
                frame_pushed = true;
            }
            for id in ids.into_iter().rev() {
                if self.binds_lexically(id) {
                    if self.captured.contains(&id) {
                        self.emit(Instr::EnvDefine(id));
                        self.scope.push((id, Binding::Env));
                    } else {
                        let slot = self.alloc_local();
                        self.emit(Instr::StoreLocal(slot));
                        self.scope.push((id, Binding::Slot(slot)));
                    }
                } else {
                    self.emit(Instr::DynBind(id));
                    dyn_count += 1;
                }
            }
        }

        let body = args
            .cdr()
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed let body"))?;
        self.compile_progn(&body)?;

        if dyn_count > 0 {
            self.emit(Instr::DynUnbind(dyn_count));
        }
        if frame_pushed {
            self.emit(Instr::PopFrame);
        }
        self.scope.truncate(saved_scope_len);
        Ok(())
    }

    /// Just the symbol of a let binding form (for the pre-scan), or
    /// None if malformed — the real error surfaces in binding_parts.
    fn binding_id(&self, b: &Value) -> Option<SymId> {
        match b {
            Value::Sym(id) => Some(*id),
            Value::Cons(c) => match &c.borrow().car {
                Value::Sym(id) => Some(*id),
                _ => None,
            },
            _ => None,
        }
    }

    fn binding_parts(&mut self, b: &Value) -> Result<(SymId, Value), Flow> {
        match b {
            Value::Sym(id) => Ok((*id, Value::Nil)),
            Value::Cons(_) => {
                let sym = b.car();
                let id = self
                    .interp
                    .as_sym(&sym)
                    .ok_or_else(|| self.interp.wrong_type("symbolp", &sym))?;
                Ok((id, b.cdr().car()))
            }
            other => Err(self.interp.wrong_type("symbolp", other)),
        }
    }

    fn compile_setq(&mut self, args: &Value) -> Result<(), Flow> {
        let items = args
            .list_to_vec()
            .ok_or_else(|| self.interp.error("malformed setq"))?;
        if items.len() % 2 != 0 {
            return Err(self.interp.error("setq: odd number of arguments"));
        }
        if items.is_empty() {
            let idx = self.add_const(Value::Nil);
            self.emit(Instr::Const(idx));
            return Ok(());
        }
        let mut i = 0;
        while i < items.len() {
            let id = self
                .interp
                .as_sym(&items[i])
                .ok_or_else(|| self.interp.wrong_type("symbolp", &items[i]))?;
            self.compile_form(&items[i + 1])?;
            let is_last = i + 2 >= items.len();
            if is_last {
                self.emit(Instr::Dup);
            }
            if let Some(slot) = self.resolve_local(id) {
                self.emit(Instr::StoreLocal(slot));
            } else {
                self.emit(Instr::StoreFree(id));
            }
            i += 2;
        }
        Ok(())
    }

    fn compile_lambda_literal(&mut self, form: &Value) -> Result<(), Flow> {
        // A nested `lambda` always inherits ITS enclosing function's
        // binding mode (lexical-binding is a per-file setting, not
        // something that can vary within one compiled function) — use
        // `self.lexical` directly rather than re-reading the global,
        // which may have moved on if we're compiling long after the
        // defining file's `eval_source` call returned (e.g. via a
        // later, interactive `byte-compile`).
        let (params, body, interactive) = parse_lambda_form(self.interp, form)?;
        let template = compile_parsed(self.interp, params, body, interactive, self.lexical, None)?;
        let idx = self.closures.len() as u32;
        self.closures.push(ClosureInfo { template });
        self.emit(Instr::MakeClosure(idx));
        Ok(())
    }

    fn fallback_interpret(&mut self, form: &Value) -> Result<(), Flow> {
        let idx = self.interprets.len() as u32;
        // Only flat-slot locals go into the interpret scope snapshot;
        // env-resident locals are reachable through the live frame chain
        // the interpreted form evaluates under, which is also what makes
        // their mutations shared instead of write-back copies.
        let flat_scope: Vec<(SymId, u16)> = self
            .scope
            .iter()
            .filter_map(|(id, b)| match b {
                Binding::Slot(s) => Some((*id, *s)),
                Binding::Env => None,
            })
            .collect();
        self.interprets.push(InterpretInfo {
            form: form.clone(),
            scope: flat_scope,
        });
        self.emit(Instr::Interpret(idx));
        Ok(())
    }
}

/// Byte-compile every named, non-macro interpreted function currently
/// defined — used at startup so the shipped lisp layer (prelude,
/// simple.el, org.el) runs compiled out of the box, like GNU Emacs
/// shipping .elc. Individual failures are skipped silently (the
/// function just stays interpreted; the tree-walker is always correct).
pub fn compile_all_defined(interp: &mut Interp) {
    use crate::value::SymId as Id;
    let candidates: Vec<Id> = (0..interp.symbols.len() as Id)
        .filter(|&id| {
            matches!(&interp.symbols[id as usize].function,
                Some(Value::Func(f)) if matches!(f.as_ref(), Function::Lambda(l) if !l.is_macro))
        })
        .collect();
    for id in candidates {
        let l = match &interp.symbols[id as usize].function {
            Some(Value::Func(f)) => match f.as_ref() {
                Function::Lambda(l) => Rc::clone(l),
                _ => continue,
            },
            _ => continue,
        };
        let interactive = l.interactive.borrow().clone();
        if let Ok(compiled) = compile_parsed(
            interp,
            l.params.clone(),
            l.body.clone(),
            interactive,
            l.lexical,
            l.env.clone(),
        ) {
            *compiled.name.borrow_mut() = Some(id);
            interp.symbols[id as usize].function =
                Some(Value::Func(Rc::new(Function::Compiled(compiled))));
        }
    }
}

/// Conservative over-approximation of "names a nested lambda or an
/// interpreted fallback form might reference": every symbol occurring
/// anywhere inside such regions (macros expanded along the way, since
/// e.g. a `dotimes` body only reveals its lambdas/setqs after
/// expansion). Locals with these names become env-resident. Symbols in
/// `quote`d data are skipped; over-collecting merely costs a variable
/// its fast slot, never correctness.
fn analyze_captured(
    interp: &mut Interp,
    body: &Value,
) -> Result<std::collections::HashSet<SymId>, Flow> {
    let mut captured = std::collections::HashSet::new();
    let mut cursor = body.clone();
    while let Value::Cons(c) = &cursor {
        let (form, next) = {
            let b = c.borrow();
            (b.car.clone(), b.cdr.clone())
        };
        walk_for_captures(interp, &form, false, &mut captured)?;
        cursor = next;
    }
    Ok(captured)
}

/// `collecting` = we're inside a capture region (a nested lambda or a
/// form the compiler will hand to the tree-walker), where every symbol
/// counts.
fn walk_for_captures(
    interp: &mut Interp,
    form: &Value,
    collecting: bool,
    out: &mut std::collections::HashSet<SymId>,
) -> Result<(), Flow> {
    match form {
        Value::Sym(id) => {
            if collecting {
                out.insert(*id);
            }
            Ok(())
        }
        Value::Cons(_) => {
            let head = form.car();
            let args = form.cdr();
            if let Value::Sym(id) = &head {
                if collecting {
                    out.insert(*id);
                }
                let name = interp.sym_name(*id).to_string();
                match name.as_str() {
                    // Quoted data can't reference variables.
                    "quote" => return Ok(()),
                    // Capture regions: everything inside counts.
                    "lambda" => return walk_all(interp, &args, true, out),
                    "function" => return walk_all(interp, &args, collecting, out),
                    // Forms the compiler sends to the tree-walker run
                    // under the live frame chain — treat like lambdas.
                    "condition-case" | "unwind-protect" | "catch" | "defvar" | "defconst"
                    | "defun" | "defmacro" | "interactive" | "`" => {
                        return walk_all(interp, &args, true, out);
                    }
                    _ => {}
                }
                // Macros hide lambdas/setqs until expanded; expand with
                // the same expander the compiler itself uses. (Expansion
                // happens again during codegen — macro expanders are
                // assumed pure, as everywhere else in elisp.)
                if let Some(Value::Func(f)) = interp.symbols[*id as usize].function.clone() {
                    if let Function::Lambda(l) = f.as_ref() {
                        if l.is_macro {
                            let unevaled = args
                                .list_to_vec()
                                .ok_or_else(|| interp.error("malformed macro call"))?;
                            let expansion = apply_lambda(interp, l.clone(), unevaled.into())?;
                            return walk_for_captures(interp, &expansion, collecting, out);
                        }
                    }
                }
                walk_all(interp, &args, collecting, out)
            } else {
                walk_for_captures(interp, &head, collecting, out)?;
                walk_all(interp, &args, collecting, out)
            }
        }
        _ => Ok(()),
    }
}

fn walk_all(
    interp: &mut Interp,
    list: &Value,
    collecting: bool,
    out: &mut std::collections::HashSet<SymId>,
) -> Result<(), Flow> {
    let mut cursor = list.clone();
    while let Value::Cons(c) = &cursor {
        let (car, cdr) = {
            let b = c.borrow();
            (b.car.clone(), b.cdr.clone())
        };
        walk_for_captures(interp, &car, collecting, out)?;
        cursor = cdr;
    }
    Ok(())
}
