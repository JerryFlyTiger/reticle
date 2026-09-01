use std::any::Any;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::error::Flow;
use crate::interp::Interp;

pub type SymId = u32;

/// Argument list for a function call (M10 item 6). Most elisp calls
/// pass a handful of arguments, so this stores up to 4 inline on the
/// Rust stack — the overwhelmingly common case pays no heap allocation
/// at all — and spills to the heap transparently (same as `Vec`) for
/// anything longer, so `&mut args` still derefs to a plain `&mut
/// [Value]` for the `Function::Builtin` call convention unchanged.
pub type Args = smallvec::SmallVec<[Value; 4]>;

/// Walk every `Value` that this Ext's payload holds internally, invoking
/// `sink` on each. Returns `false` if the payload can't be safely
/// inspected right now (e.g. a `RefCell` already borrowed elsewhere) —
/// same convention as `gc::RootProvider`: an incomplete walk means the
/// whole collection must abort rather than risk clearing something live.
pub type ExtTracer = fn(&Rc<dyn Any>, &mut dyn FnMut(&Value)) -> bool;

/// Reference to an editor-defined object (buffer, marker, keymap, ...).
/// Keeps the elisp crate independent of the editor: it only knows the tag
/// (and, via `trace`, how to enumerate any `Value`s the payload holds —
/// the elisp crate never downcasts `obj` itself, just calls the function
/// pointer the editor supplied).
#[derive(Clone)]
pub struct ExtRef {
    pub tag: &'static str,
    pub obj: Rc<dyn Any>,
    pub trace: Option<ExtTracer>,
}

impl ExtRef {
    /// `trace` must be `Some` for any payload that stores a `Value`
    /// (directly or via a container), or the GC mark phase will never
    /// see those values and can clear them out from under a live
    /// reference. Callers must state `None` explicitly (with a comment
    /// explaining why the payload holds no `Value`) rather than this
    /// function defaulting one in, so every new Ext type is forced to
    /// make that call deliberately instead of silently inheriting a gap.
    pub fn new<T: Any>(tag: &'static str, obj: T, trace: Option<ExtTracer>) -> ExtRef {
        ExtRef {
            tag,
            obj: Rc::new(obj),
            trace,
        }
    }

    pub fn downcast<T: Any>(&self) -> Option<Rc<T>> {
        self.obj.clone().downcast::<T>().ok()
    }
}

#[derive(Clone)]
pub enum Value {
    /// nil is both the symbol `nil`, the empty list, and false.
    Nil,
    Int(i64),
    /// Arbitrary-precision integer (Emacs 27+ bignum semantics:
    /// arithmetic overflow promotes instead of erroring). Invariant:
    /// always canonical — a value that fits in i64 is ALWAYS an `Int`,
    /// never a `Big` (`Value::big` enforces it), so fixnum comparisons
    /// and the VM/JIT Int fast paths stay exact.
    Big(Rc<num_bigint::BigInt>),
    Float(f64),
    Str(Rc<String>),
    Sym(SymId),
    Cons(Rc<RefCell<ConsCell>>),
    Vector(Rc<RefCell<Vec<Value>>>),
    /// Real hash table (M10 item 5): O(1) average lookup by `equal`,
    /// insertion order preserved for `maphash` (matches the previous
    /// assoc-vector's iteration order, which some elisp relies on even
    /// though Emacs itself doesn't guarantee an order).
    HashTable(Rc<RefCell<indexmap::IndexMap<HKey, Value>>>),
    Func(Rc<Function>),
    Ext(ExtRef),
}

pub struct ConsCell {
    pub car: Value,
    pub cdr: Value,
}

pub struct Lambda {
    pub params: ParamSpec,
    /// List of body forms.
    pub body: Value,
    /// Captured lexical environment (None at top level).
    pub env: Option<Rc<LexEnv>>,
    /// Binding mode at definition site: true = params bind lexically.
    pub lexical: bool,
    pub is_macro: bool,
    pub name: RefCell<Option<SymId>>,
    pub interactive: RefCell<Option<Value>>,
    /// Call counter for tiered auto-compilation: hot interpreted
    /// functions get byte-compiled automatically (see eval::apply_lambda).
    pub calls: std::cell::Cell<u32>,
}

#[derive(Clone)]
pub struct ParamSpec {
    pub required: Vec<SymId>,
    pub optional: Vec<SymId>,
    pub rest: Option<SymId>,
}

/// A byte-compiled function: shares its `chunk` (the compiled template)
/// across every closure made from the same lambda literal, with `env`
/// distinguishing each closure instance's captured variables.
pub struct CompiledFn {
    pub params: ParamSpec,
    pub chunk: Rc<crate::bytecode::Chunk>,
    pub env: Option<Rc<LexEnv>>,
    pub lexical: bool,
    pub name: RefCell<Option<SymId>>,
    pub interactive: RefCell<Option<Value>>,
    /// Call counter + one-shot flag for the second auto-compilation
    /// tier: hot bytecode functions get one native-compile attempt
    /// (see vm::run); ineligible ones just stay bytecode.
    pub calls: std::cell::Cell<u32>,
    pub jit_tried: std::cell::Cell<bool>,
}

pub enum Function {
    Builtin {
        name: &'static str,
        min_args: usize,
        max_args: Option<usize>,
        f: fn(&mut Interp, &mut [Value]) -> Result<Value, Flow>,
    },
    Lambda(Rc<Lambda>),
    Compiled(Rc<CompiledFn>),
    /// A native-compiled function (level-2 JIT, see `crate::jit`). Only
    /// pure integer functions qualify; `fallback` is the bytecode
    /// twin `native_compile` compiled it from, always safe to re-run
    /// wholesale on bailout since eligible functions have no side effects.
    Native(Rc<crate::jit::NativeFn>),
    /// A function registered by a dynamically loaded module (M13, see
    /// `crate::module`). Distinct from `Native` above -- that's our own
    /// JIT output, this is arbitrary external native code loaded via
    /// `dlopen`.
    Module(Rc<crate::module::ModuleFunction>),
}

/// Lexical environment: a chain of frames, innermost first.
pub struct LexEnv {
    pub vars: RefCell<Vec<(SymId, Value)>>,
    pub parent: Option<Rc<LexEnv>>,
}

impl LexEnv {
    pub fn new(parent: Option<Rc<LexEnv>>) -> Rc<LexEnv> {
        Rc::new(LexEnv {
            vars: RefCell::new(Vec::new()),
            parent,
        })
    }

    pub fn lookup(env: &Rc<LexEnv>, id: SymId) -> Option<Value> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            if let Some((_, v)) = e.vars.borrow().iter().rev().find(|(s, _)| *s == id) {
                return Some(v.clone());
            }
            cur = e.parent.clone();
        }
        None
    }

    /// Assign `id` in the innermost frame that binds it. Returns the
    /// mutated frame on success — the caller registers it with the GC,
    /// since a captured-variable assignment is the one way a closure
    /// can come to (indirectly) reference itself.
    pub fn set(env: &Rc<LexEnv>, id: SymId, val: Value) -> Option<Rc<LexEnv>> {
        let mut cur = Some(env.clone());
        while let Some(e) = cur {
            let found = {
                let mut vars = e.vars.borrow_mut();
                match vars.iter_mut().rev().find(|(s, _)| *s == id) {
                    Some((_, slot)) => {
                        *slot = val.clone();
                        true
                    }
                    None => false,
                }
            };
            if found {
                return Some(e);
            }
            cur = e.parent.clone();
        }
        None
    }
}

impl Value {
    pub fn cons(car: Value, cdr: Value) -> Value {
        Value::Cons(Rc::new(RefCell::new(ConsCell { car, cdr })))
    }

    pub fn string(s: impl Into<String>) -> Value {
        Value::Str(Rc::new(s.into()))
    }

    pub fn list(items: Vec<Value>) -> Value {
        let mut acc = Value::Nil;
        for v in items.into_iter().rev() {
            acc = Value::cons(v, acc);
        }
        acc
    }

    pub fn bool(b: bool, t: SymId) -> Value {
        if b {
            Value::Sym(t)
        } else {
            Value::Nil
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    pub fn truthy(&self) -> bool {
        !self.is_nil()
    }

    pub fn car(&self) -> Value {
        match self {
            Value::Cons(c) => c.borrow().car.clone(),
            _ => Value::Nil,
        }
    }

    pub fn cdr(&self) -> Value {
        match self {
            Value::Cons(c) => c.borrow().cdr.clone(),
            _ => Value::Nil,
        }
    }

    /// Collect a proper list into a Vec. Returns None on dotted/circular tails.
    pub fn list_to_vec(&self) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        let mut cur = self.clone();
        let mut steps = 0usize;
        loop {
            match cur {
                Value::Nil => return Some(out),
                Value::Cons(c) => {
                    let b = c.borrow();
                    out.push(b.car.clone());
                    cur = b.cdr.clone();
                }
                _ => return None,
            }
            steps += 1;
            if steps > 1_000_000 {
                return None;
            }
        }
    }

    /// eq: identity (pointer) equality; ints/symbols compare by value.
    /// Named after the elisp predicate; not the PartialEq trait on purpose.
    #[allow(clippy::should_implement_trait)]
    /// Canonicalizing bignum constructor: values that fit in i64 become
    /// fixnums (the `Big` invariant — see the enum docs).
    pub fn big(n: num_bigint::BigInt) -> Value {
        use num_traits::ToPrimitive;
        match n.to_i64() {
            Some(i) => Value::Int(i),
            None => Value::Big(Rc::new(n)),
        }
    }

    /// elisp `eq` — deliberately not the `PartialEq` trait (identity
    /// semantics differ from structural equality; see also `equal`).
    #[allow(clippy::should_implement_trait)]
    pub fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Int(a), Value::Int(b)) => a == b,
            // Bignums compare by value even under eq (like Emacs, where
            // eq on bignums is unreliable — we make it just work).
            (Value::Big(a), Value::Big(b)) => a == b,
            (Value::Sym(a), Value::Sym(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => Rc::ptr_eq(a, b),
            (Value::Cons(a), Value::Cons(b)) => Rc::ptr_eq(a, b),
            (Value::Vector(a), Value::Vector(b)) => Rc::ptr_eq(a, b),
            (Value::HashTable(a), Value::HashTable(b)) => Rc::ptr_eq(a, b),
            (Value::Func(a), Value::Func(b)) => Rc::ptr_eq(a, b),
            (Value::Ext(a), Value::Ext(b)) => Rc::ptr_eq(&a.obj, &b.obj),
            _ => false,
        }
    }

    /// eql: eq, plus floats compare by value.
    pub fn eql(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
            _ => self.eq(other),
        }
    }

    /// equal: structural equality.
    pub fn equal(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Cons(a), Value::Cons(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let (x, y) = (a.borrow(), b.borrow());
                x.car.equal(&y.car) && x.cdr.equal(&y.cdr)
            }
            (Value::Vector(a), Value::Vector(b)) => {
                if Rc::ptr_eq(a, b) {
                    return true;
                }
                let (x, y) = (a.borrow(), b.borrow());
                x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| p.equal(q))
            }
            _ => self.eql(other),
        }
    }

    /// Hash consistent with `equal()` (same partition: content-based for
    /// Str/Int/Big/Float/Sym, structural-recursive for Cons/Vector,
    /// identity-based for HashTable/Func/Ext, matching exactly which
    /// arms `equal()` special-cases versus falls through to `eql`/`eq`
    /// for). Used to key hash tables by `equal` in O(1) instead of the
    /// old O(n) linear scan — see `HKey` below.
    ///
    /// Depth-limited like the printer, so a self-referential list used
    /// as a hash key can't hang hashing forever. Note this is a
    /// narrower guarantee than `equal()` itself has: `equal()`'s own
    /// Cons/Vector recursion has no such guard and could in principle
    /// loop forever on a circular structure passed to it directly
    /// (pre-existing, not introduced here) — hashing terminating just
    /// means `puthash`/`gethash` on a circular key won't hang on the
    /// hash computation itself.
    pub fn hash_for_equal<H: std::hash::Hasher>(&self, state: &mut H) {
        hash_value(self, state, 0);
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Int(_) => "integer",
            Value::Big(_) => "integer",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Sym(_) => "symbol",
            Value::Cons(_) => "cons",
            Value::Vector(_) => "vector",
            Value::HashTable(_) => "hash-table",
            Value::Func(_) => "function",
            Value::Ext(e) => e.tag,
        }
    }

    pub fn as_ext<T: Any>(&self, tag: &str) -> Option<Rc<T>> {
        match self {
            Value::Ext(e) if e.tag == tag => e.downcast::<T>(),
            _ => None,
        }
    }
}

const HASH_MAX_DEPTH: usize = 200;

fn hash_value<H: Hasher>(v: &Value, state: &mut H, depth: usize) {
    if depth > HASH_MAX_DEPTH {
        return; // stop contributing further structure; never infinite-loops
    }
    match v {
        Value::Nil => 0u8.hash(state),
        Value::Int(i) => {
            1u8.hash(state);
            i.hash(state);
        }
        // A separate tag from Int: the canonicalization invariant
        // (Value::big demotes anything that fits i64) means a Big and
        // an Int can never actually hold the same mathematical value,
        // so they never compare `equal` — no need to hash them alike.
        Value::Big(b) => {
            2u8.hash(state);
            b.hash(state);
        }
        Value::Float(f) => {
            3u8.hash(state);
            // eql (which equal falls through to for non-string/cons/
            // vector) compares by bit pattern, not by `==` — match it.
            f.to_bits().hash(state);
        }
        Value::Str(s) => {
            4u8.hash(state);
            s.hash(state);
        }
        Value::Sym(id) => {
            5u8.hash(state);
            id.hash(state);
        }
        Value::Cons(c) => {
            6u8.hash(state);
            let b = c.borrow();
            hash_value(&b.car, state, depth + 1);
            hash_value(&b.cdr, state, depth + 1);
        }
        Value::Vector(items) => {
            7u8.hash(state);
            for item in items.borrow().iter() {
                hash_value(item, state, depth + 1);
            }
        }
        // equal() falls through to eq() for these three (identity, via
        // Rc::ptr_eq) rather than comparing contents — hash by identity
        // to match, or two `equal` (by this table's actual rule) values
        // could still land in different buckets, which is merely slow,
        // never wrong, but pointer-identity hashing keeps it consistent
        // and fast.
        Value::HashTable(h) => {
            8u8.hash(state);
            (Rc::as_ptr(h) as usize).hash(state);
        }
        Value::Func(f) => {
            9u8.hash(state);
            (Rc::as_ptr(f) as usize).hash(state);
        }
        Value::Ext(e) => {
            10u8.hash(state);
            (Rc::as_ptr(&e.obj) as *const () as usize).hash(state);
        }
    }
}

/// Newtype key wrapper so hash tables can use a real `HashMap`/
/// `IndexMap` keyed by elisp's `equal` semantics specifically, without
/// giving raw `Value` a general-purpose `Eq`/`Hash` impl (which would
/// force a single, sitewide choice among eq/eql/equal — `equal` is the
/// only one that makes sense as a hash-table default, so it's scoped to
/// this wrapper rather than claimed for `Value` as a whole).
#[derive(Clone)]
pub struct HKey(pub Value);

impl PartialEq for HKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.equal(&other.0)
    }
}

impl std::cmp::Eq for HKey {}

impl Hash for HKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash_for_equal(state);
    }
}
