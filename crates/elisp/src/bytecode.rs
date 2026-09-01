//! Bytecode data types shared by the compiler and the VM.
//!
//! The instruction set is stack-based. Jump targets are absolute indices
//! into `Chunk::code`, resolved by the compiler via backpatching.

use std::rc::Rc;

use crate::value::{CompiledFn, SymId, Value};

#[derive(Clone, Copy, Debug)]
pub enum Instr {
    /// Push `consts[idx]`.
    Const(u32),
    /// Push the value of local slot `idx`.
    LoadLocal(u16),
    /// Pop and store into local slot `idx` (leaves nothing on the stack).
    StoreLocal(u16),
    /// Push the value of a symbol that isn't a compile-time local: walks
    /// the closure's captured environment, then the global/dynamic cell.
    LoadFree(SymId),
    /// Pop and store into a symbol via the same free-variable path.
    StoreFree(SymId),
    /// Save the symbol's current dynamic value and bind it to the value
    /// popped off the stack (used for `let`-bound special variables).
    DynBind(SymId),
    /// Restore the `count` most recently pushed dynamic bindings.
    DynUnbind(u16),
    /// Build a closure from `chunk.closures[idx]`, capturing the current
    /// lexical frame chain directly (shared, not snapshotted — closures
    /// capture variables, not values), and push it.
    MakeClosure(u32),
    /// Enter a fresh lexical frame (child of the current chain). Emitted
    /// at function entry and at each `let`/`let*` that binds variables
    /// some nested lambda (or interpreted fallback form) references.
    PushFrame,
    /// Leave the innermost lexical frame at normal scope exit. Abnormal
    /// exits (errors/throws) skip it safely: the whole call unwinds and
    /// the chain is discarded, exactly like the tree-walker dropping
    /// frames.
    PopFrame,
    /// Pop a value and bind it as `sym` in the current (innermost)
    /// frame. Used for captured variables instead of a flat slot, so
    /// closures and the defining function share one binding.
    EnvDefine(SymId),
    /// Pop the function and `argc` arguments (function pushed first, then
    /// args in order), call, push the result.
    Call(u16),
    /// Fast-path arithmetic/comparison/list primitives (M10): pop the
    /// operands, run the common case inline (Int×Int arithmetic, cons
    /// access), and fall back to calling the real builtin — stored in
    /// `consts[idx]` at compile time, so a later redefinition of `+`
    /// doesn't change already-compiled code (same rule as GNU Emacs
    /// bytecode) — for every other type combination or error case.
    Add2(u32),
    Sub2(u32),
    Mul2(u32),
    Lt2(u32),
    Gt2(u32),
    Le2(u32),
    Ge2(u32),
    NumEq2(u32),
    /// (1+ x) / (1- x)
    Inc1(u32),
    Dec1(u32),
    Car1(u32),
    Cdr1(u32),
    /// eq / not / cons never need a fallback (total on all values).
    Eq2,
    Not1,
    Cons2,
    /// Unconditional jump to an absolute instruction index.
    Jump(usize),
    /// Pop; if nil, jump to the absolute instruction index.
    JumpIfNil(usize),
    /// Pop; if non-nil, jump to the absolute instruction index.
    JumpIfNonNil(usize),
    Pop,
    /// Duplicate the top of the stack.
    Dup,
    /// Fall back to the tree-walking evaluator for forms the compiler
    /// doesn't handle (condition-case, catch, unwind-protect, backquote,
    /// defmacro, defvar, ...). `interprets[idx]` carries the original
    /// form plus a snapshot map of which locals are currently in scope.
    Interpret(u32),
    /// End the function, returning the top of the stack.
    Return,
}

/// A form to run through `eval()` at a specific compiled program point,
/// with enough information to expose the compiled function's current
/// local variables to it.
pub struct InterpretInfo {
    pub form: Value,
    /// Locals visible at this point: symbol -> slot index.
    pub scope: Vec<(SymId, u16)>,
}

/// An uninstantiated nested lambda: `template.env` is always `None` here;
/// `MakeClosure` clones it and fills in `env` with the current frame
/// chain (shared — captured variables live in frames, never in copies).
pub struct ClosureInfo {
    pub template: Rc<CompiledFn>,
}

pub struct Chunk {
    pub code: Vec<Instr>,
    pub consts: Vec<Value>,
    pub interprets: Vec<InterpretInfo>,
    pub closures: Vec<ClosureInfo>,
    /// Total local variable slots this frame needs (params + all `let`
    /// bindings anywhere in the body; slots are never reused).
    pub num_locals: usize,
}
