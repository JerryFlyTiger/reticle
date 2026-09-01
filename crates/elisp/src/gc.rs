//! Cycle collector (M9).
//!
//! Memory management is hybrid, CPython-style: `Rc` reference counting
//! keeps reclaiming acyclic garbage immediately (the vast majority — so
//! memory does not balloon between collections), and this module exists
//! solely to reclaim *reference cycles*, which refcounting can never
//! free. Because cycles accumulate slowly, collection can run rarely:
//! at idle, past a threshold, or on explicit `(garbage-collect)` —
//! never in the middle of typing.
//!
//! ## Why registration happens on *mutation*, not allocation
//!
//! A strong-`Rc` cycle cannot be built bottom-up out of fresh cells: a
//! newly allocated cons can only point at already-existing values, none
//! of which can point back at it yet. Every cycle therefore contains at
//! least one link that was *mutated* into place after allocation —
//! `setcar`/`setcdr`, `aset`, `puthash`, or a captured lexical variable
//! assigned via `setq` (`LexEnv::set`). Those five places are the only
//! registration hooks, which buys two big things:
//!
//!  * The allocation fast path (and every read path) is completely
//!    untouched — zero overhead while the user types.
//!  * The registry stays small: it holds only objects that were ever
//!    mutated, not every object in the heap. And breaking *one* link of
//!    a dead cycle is enough — the rest unwinds through normal `Rc`
//!    drops — so "at least one member registered" is sufficient.
//!
//! ## The sweep is cycle-breaking, not freeing
//!
//! An object that is still alive (its `Weak` upgrades) but unreachable
//! from the roots must be part of an orphaned cycle: unreachable means
//! no live code holds it, refcount > 0 means *something* does, and the
//! only candidates left are other unreachable objects — the cycle
//! itself. Clearing such an object's contents (car/cdr to nil, vector
//! emptied, captured environment dropped) severs the cycle; refcounts
//! then fall to zero and `Rc` finishes the job. No reachable code can
//! ever observe the cleared object, by construction.
//!
//! ## Quiescent points only
//!
//! Collection runs only when the evaluator is idle (`interp.depth == 0`
//! — between keystrokes / top-level forms). At that moment no `Value`s
//! live on the Rust call stack, so the root set is exactly: the symbol
//! table (every value/function/plist cell) plus whatever persistent
//! holders were registered as root providers (the editor registers its
//! buffers, keymaps, pending commands, ...). This is what makes the
//! whole design tractable — no conservative stack scanning.
//!
//! ## Known gap: a cycle closed entirely through `Ext` objects can be
//! marked reachable but never registered for sweep (M52)
//!
//! `mark` (below) can now walk *into* a `Value::Ext` payload via its
//! `ExtTracer`, so an Ext-mediated reference no longer defeats
//! reachability analysis — a keymap's key bindings, an overlay's props,
//! a buffer's locals are all correctly seen as reachable (or not) from
//! the roots. But reachability is only half of what the collector needs:
//! `stored_can_cycle` below, which gates every *registration* hook, does
//! not consider `Ext` a cycle-capable value, and *none* of the Rust-side
//! mutators that write a `Value` into an Ext payload — `Keymap::set` /
//! `define_sequence` for a keymap's `entries`, `OverlayData::put` for an
//! overlay's props, `Buffer.locals.insert` for a buffer-local — calls
//! `register_value` or `register_env` the way
//! `setcar`/`setcdr`/`aset`/`puthash`/`LexEnv::set` do. Keymaps are just
//! the easiest example to state; the gap is the whole Ext family. So a cycle
//! closed entirely through `Ext` nodes — two keymaps each bound directly
//! into the other's `entries`, or a keymap `setq`'d into a lexical
//! variable that a closure bound *inside that same keymap* captures —
//! has no member in `interp.gc.registry` at all. `mark` will correctly
//! determine such a cycle is unreachable once nothing else points into
//! it, but `collect`'s sweep only ever inspects the registry, so nothing
//! is ever a sweep candidate and the cycle leaks for the life of the
//! process. Fixing this needs a `GcObj::Ext` registry variant plus a
//! per-tag definition of what "clearing" an Ext's contents means (an
//! `ExtRef` has no generic notion of "empty" the way a cons or vector
//! does) — an architectural change, deliberately out of scope for M52,
//! which only had to make marking correct, not registration.

use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use crate::interp::Interp;
use crate::value::{ConsCell, Function, LexEnv, Value};

/// One registered potential-cycle member.
enum GcObj {
    Cons(Weak<std::cell::RefCell<ConsCell>>),
    Vector(Weak<std::cell::RefCell<Vec<Value>>>),
    Hash(Weak<std::cell::RefCell<indexmap::IndexMap<crate::value::HKey, Value>>>),
    Env(Weak<LexEnv>),
}

impl GcObj {
    /// Address of the referent (identity key), or None if already dead.
    fn addr(&self) -> Option<usize> {
        match self {
            GcObj::Cons(w) => w.upgrade().map(|r| Rc::as_ptr(&r) as usize),
            GcObj::Vector(w) => w.upgrade().map(|r| Rc::as_ptr(&r) as usize),
            GcObj::Hash(w) => w.upgrade().map(|r| Rc::as_ptr(&r) as usize),
            GcObj::Env(w) => w.upgrade().map(|r| Rc::as_ptr(&r) as usize),
        }
    }

    /// Break the cycle by clearing contents. The dropped values may
    /// cascade-free other doomed objects; that's fine — their Weaks die
    /// and they're skipped when their turn comes.
    fn clear(&self) {
        match self {
            GcObj::Cons(w) => {
                if let Some(c) = w.upgrade() {
                    let mut b = c.borrow_mut();
                    b.car = Value::Nil;
                    b.cdr = Value::Nil;
                }
            }
            GcObj::Vector(w) => {
                if let Some(v) = w.upgrade() {
                    v.borrow_mut().clear();
                }
            }
            GcObj::Hash(w) => {
                if let Some(h) = w.upgrade() {
                    h.borrow_mut().clear();
                }
            }
            GcObj::Env(w) => {
                if let Some(e) = w.upgrade() {
                    e.vars.borrow_mut().clear();
                }
            }
        }
    }
}

/// Registry of mutated (potential-cycle) objects, keyed by referent
/// address so repeated mutation of the same object registers once.
/// Address reuse after a free is handled at insert: a dead entry at the
/// same key is simply replaced.
#[derive(Default)]
pub struct GcState {
    registry: HashMap<usize, GcObj>,
    /// Registrations since the last collection — the trigger metric
    /// (cycles can only grow when this grows).
    pub registered_since_gc: usize,
}

impl GcState {
    pub fn live_count(&self) -> usize {
        self.registry.len()
    }
}

/// Enumerates the persistent `Value`s of some non-elisp holder (the
/// editor: buffer keymaps/locals/overlays, pending commands, ...).
/// Returns false to abort the whole collection — used when the holder
/// can't be safely inspected right now (e.g. the editor is mutably
/// borrowed by an in-flight command); a collection with an incomplete
/// root set could clear live data, so aborting is the only safe answer.
pub type RootProvider = Box<dyn Fn(&mut dyn FnMut(&Value)) -> bool>;

fn register(interp: &mut Interp, addr: usize, obj: GcObj) {
    use std::collections::hash_map::Entry;
    match interp.gc.registry.entry(addr) {
        Entry::Occupied(mut e) => {
            // Same address: either the same object (already registered)
            // or a dead entry whose address got reused — replace then.
            if e.get().addr() != Some(addr) {
                e.insert(obj);
                interp.gc.registered_since_gc += 1;
            }
        }
        Entry::Vacant(e) => {
            e.insert(obj);
            interp.gc.registered_since_gc += 1;
        }
    }
}

/// Can storing this value into a container possibly create a cycle?
/// Ints/floats/strings/symbols/nil cannot hold references, so a store
/// of one can never close a loop — the (very common) `(setq i (1+ i))`
/// case costs nothing. Ext objects are reference-opaque leaves here.
fn stored_can_cycle(v: &Value) -> bool {
    matches!(
        v,
        Value::Cons(_) | Value::Vector(_) | Value::HashTable(_) | Value::Func(_)
    )
}

/// Register a container whose slot was just mutated (setcar/setcdr/
/// aset/puthash) to hold `stored`. No-op when the stored value can't
/// participate in a cycle.
pub fn register_value(interp: &mut Interp, container: &Value, stored: &Value) {
    if !stored_can_cycle(stored) {
        return;
    }
    match container {
        Value::Cons(c) => register(
            interp,
            Rc::as_ptr(c) as usize,
            GcObj::Cons(Rc::downgrade(c)),
        ),
        Value::Vector(c) => register(
            interp,
            Rc::as_ptr(c) as usize,
            GcObj::Vector(Rc::downgrade(c)),
        ),
        Value::HashTable(c) => register(
            interp,
            Rc::as_ptr(c) as usize,
            GcObj::Hash(Rc::downgrade(c)),
        ),
        _ => {}
    }
}

/// Register a lexical frame whose variable was just assigned `stored`
/// (the only way a closure can come to reference itself). Same
/// can-cycle gate, so integer-accumulator loops register nothing.
pub fn register_env(interp: &mut Interp, env: &Rc<LexEnv>, stored: &Value) {
    if !stored_can_cycle(stored) {
        return;
    }
    register(
        interp,
        Rc::as_ptr(env) as usize,
        GcObj::Env(Rc::downgrade(env)),
    );
}

/// What to traverse next during marking. An explicit worklist (not
/// recursion) so million-element lists can't overflow the Rust stack.
enum Work {
    V(Value),
    Env(Rc<LexEnv>),
    CFn(Rc<crate::value::CompiledFn>),
}

fn mark(roots: Vec<Value>) -> Option<HashSet<usize>> {
    let mut visited: HashSet<usize> = HashSet::new();
    let mut work: Vec<Work> = roots.into_iter().map(Work::V).collect();

    while let Some(item) = work.pop() {
        match item {
            Work::V(v) => match &v {
                Value::Nil | Value::Int(_) | Value::Big(_) | Value::Float(_) | Value::Str(_) => {}
                // Symbol cells are all enumerated as roots directly, so
                // a symbol reference needs no traversal of its own.
                Value::Sym(_) => {}
                // Ext objects (buffers, keymaps, overlays, ...) are opaque
                // leaves unless the editor supplied a tracer: `trace`
                // enumerates any Values the payload holds internally (a
                // keymap's bindings, a buffer's locals/overlay props, ...),
                // which root providers can't reach on their own because
                // those values are only reachable *through* the Ext, not
                // held directly by the editor's root structures.
                Value::Ext(e) => {
                    if let Some(trace) = e.trace {
                        // Insert into `visited` before recursing. A
                        // cycle that closes through a Cons/Func/Env node
                        // along the way is already caught by that node's
                        // own visited-gate (e.g. a keymap binding a
                        // closure that captures the keymap variable
                        // itself loops back through `Value::Func`, which
                        // gates on its own). The case this guard alone
                        // has to catch is a cycle that closes entirely
                        // through Ext nodes -- e.g. two keymaps each
                        // bound directly as the other's key definition
                        // (core's `mutually_referential_keymaps_do_not_
                        // hang_gc` test) -- where nothing else on the
                        // path would ever stop the recursion.
                        if visited.insert(Rc::as_ptr(&e.obj) as *const () as usize) {
                            let mut children: Vec<Value> = Vec::new();
                            if !trace(&e.obj, &mut |v: &Value| children.push(v.clone())) {
                                return None;
                            }
                            for c in children {
                                work.push(Work::V(c));
                            }
                        }
                    }
                }
                Value::Cons(c) => {
                    if visited.insert(Rc::as_ptr(c) as usize) {
                        let b = c.borrow();
                        work.push(Work::V(b.car.clone()));
                        work.push(Work::V(b.cdr.clone()));
                    }
                }
                Value::Vector(c) => {
                    if visited.insert(Rc::as_ptr(c) as usize) {
                        for x in c.borrow().iter() {
                            work.push(Work::V(x.clone()));
                        }
                    }
                }
                Value::HashTable(c) => {
                    if visited.insert(Rc::as_ptr(c) as usize) {
                        for (k, val) in c.borrow().iter() {
                            work.push(Work::V(k.0.clone()));
                            work.push(Work::V(val.clone()));
                        }
                    }
                }
                Value::Func(f) => {
                    if visited.insert(Rc::as_ptr(f) as usize) {
                        match f.as_ref() {
                            Function::Builtin { .. } => {}
                            Function::Lambda(l) => {
                                work.push(Work::V(l.body.clone()));
                                if let Some(i) = l.interactive.borrow().clone() {
                                    work.push(Work::V(i));
                                }
                                if let Some(env) = &l.env {
                                    work.push(Work::Env(env.clone()));
                                }
                            }
                            Function::Compiled(c) => work.push(Work::CFn(c.clone())),
                            Function::Native(n) => work.push(Work::CFn(n.fallback.clone())),
                            // A module's `data` pointer is opaque, module-owned
                            // memory, never one of our `Value`s -- nothing to mark.
                            Function::Module(_) => {}
                        }
                    }
                }
            },
            Work::Env(env) => {
                let mut cur = Some(env);
                while let Some(e) = cur {
                    if !visited.insert(Rc::as_ptr(&e) as usize) {
                        break; // chain tail already marked
                    }
                    for (_, v) in e.vars.borrow().iter() {
                        work.push(Work::V(v.clone()));
                    }
                    cur = e.parent.clone();
                }
            }
            Work::CFn(c) => {
                if visited.insert(Rc::as_ptr(&c) as usize) {
                    // A compiled function's constants can hold mutable
                    // conses (quoted literals the user may setcar), and
                    // its Interpret fallbacks / nested closure templates
                    // hold live forms — all of it must be marked or the
                    // sweep would corrupt function internals.
                    for v in &c.chunk.consts {
                        work.push(Work::V(v.clone()));
                    }
                    for info in &c.chunk.interprets {
                        work.push(Work::V(info.form.clone()));
                    }
                    for cl in &c.chunk.closures {
                        work.push(Work::CFn(cl.template.clone()));
                    }
                    if let Some(i) = c.interactive.borrow().clone() {
                        work.push(Work::V(i));
                    }
                    if let Some(env) = &c.env {
                        work.push(Work::Env(env.clone()));
                    }
                }
            }
        }
    }
    Some(visited)
}

pub struct GcOutcome {
    /// Cyclic objects found unreachable (and cleared, unless dry run).
    pub freed: usize,
    /// Registry entries still live after the collection.
    pub remaining: usize,
}

/// Run a full cycle collection. Returns None (refuses) if the evaluator
/// is mid-eval — the root set is only complete at a quiescent point.
pub fn collect(interp: &mut Interp, dry_run: bool) -> Option<GcOutcome> {
    if interp.depth != 0 {
        return None;
    }

    // Roots: every symbol cell, then everything the embedder registered.
    let mut roots: Vec<Value> = Vec::new();
    for s in &interp.symbols {
        if let Some(v) = &s.value {
            roots.push(v.clone());
        }
        if let Some(f) = &s.function {
            roots.push(f.clone());
        }
        if !s.plist.is_nil() {
            roots.push(s.plist.clone());
        }
    }
    // Providers are moved out during the calls so they can be handed a
    // &mut collector closure without aliasing interp.
    let providers = std::mem::take(&mut interp.gc_roots);
    let mut all_ok = true;
    for p in &providers {
        if !p(&mut |v: &Value| roots.push(v.clone())) {
            all_ok = false;
            break;
        }
    }
    interp.gc_roots = providers;
    if !all_ok {
        return None; // incomplete roots: collecting now would be unsafe
    }

    // An Ext tracer refusing to walk (e.g. a borrowed RefCell) is the
    // same kind of incomplete-root-set hazard as a root provider
    // refusing: better to skip this collection than sweep with a
    // partial view of what's reachable.
    let visited = mark(roots)?;

    // Sweep: snapshot the doomed set first, then clear, so cascade
    // drops during clearing can't invalidate the iteration.
    let mut doomed: Vec<usize> = Vec::new();
    let mut dead: Vec<usize> = Vec::new();
    for (addr, obj) in &interp.gc.registry {
        match obj.addr() {
            None => dead.push(*addr),
            Some(a) => {
                if !visited.contains(&a) {
                    doomed.push(*addr);
                }
            }
        }
    }
    let freed = doomed.len();
    if !dry_run {
        for addr in &doomed {
            if let Some(obj) = interp.gc.registry.get(addr) {
                obj.clear();
            }
        }
        for addr in doomed.iter().chain(dead.iter()) {
            interp.gc.registry.remove(addr);
        }
        interp.gc.registered_since_gc = 0;
    }
    Some(GcOutcome {
        freed,
        remaining: interp.gc.registry.len(),
    })
}

/// Auto-collection hook for editor loops. `idle` marks a genuine idle
/// tick (user hasn't typed for a while): collect if anything meaningful
/// accumulated. Non-idle calls (after each key) collect only past the
/// `gc-cons-threshold` pressure limit, so typing stays collection-free.
pub fn maybe_auto_collect(interp: &mut Interp, idle: bool) -> Option<GcOutcome> {
    if interp.depth != 0 {
        return None;
    }
    let n = interp.gc.registered_since_gc;
    if n == 0 {
        return None;
    }
    let threshold = {
        let id = interp.intern("gc-cons-threshold");
        match interp.sym_value(id) {
            Some(Value::Int(t)) if t > 0 => t as usize,
            _ => usize::MAX, // unset/broken: never force mid-use
        }
    };
    // Idle floor: don't bother walking the whole heap for a handful of
    // registrations; they can wait for the next idle window.
    const IDLE_FLOOR: usize = 64;
    if (idle && n >= IDLE_FLOOR) || n >= threshold {
        collect(interp, false)
    } else {
        None
    }
}
