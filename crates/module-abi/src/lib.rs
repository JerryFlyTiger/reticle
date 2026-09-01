// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! Pure ABI definitions for reticle dynamic modules (M13) -- no
//! logic, just `#[repr(C)]` types shared between the host (implemented
//! in `crates/elisp/src/module.rs`) and any module compiled against this
//! crate. Kept dependency-free and minimal on purpose: this crate *is*
//! the ABI contract, so a change here is a breaking change for every
//! already-compiled module, same as `emacs-module.h` is for real GNU
//! Emacs modules.
//!
//! Modeled on GNU Emacs 25's emacs-module.h: a versioned table of plain
//! `extern "C" fn` pointers, values exchanged as small opaque handles
//! rather than raw pointers into the host's own `Value`/`Rc` internals.
//! That's a deliberate choice, not an oversight -- it means a module can
//! never forge or corrupt a real host value, and the host's internal
//! value representation is never part of this ABI and can change freely.

use std::os::raw::{c_char, c_void};

/// Opaque handle to a host elisp value. Just a `u32` index into the
/// host's per-call value table (see `ModuleEnv::ctx`) -- deliberately not
/// a pointer, so it round-trips through the FFI boundary with no aliasing
/// or lifetime hazards at the ABI level.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ModuleValue(pub u32);

/// Reserved, always-valid handles every `ModuleCallCtx` seeds up front.
pub const NIL: ModuleValue = ModuleValue(0);
pub const T: ModuleValue = ModuleValue(1);

/// The signature a module's own native function must have to be
/// registered via `ModuleEnv::make_function`. `data` is whatever raw
/// pointer the module passed to `make_function`, handed back unchanged
/// on every call -- the module's own way to carry state, same as GNU
/// Emacs's `emacs_module_function`.
pub type ModuleCallback = extern "C" fn(
    env: *mut ModuleEnv,
    nargs: isize,
    args: *const ModuleValue,
    data: *mut c_void,
) -> ModuleValue;

/// The versioned function table a module receives. `size` is
/// `size_of::<ModuleEnv>()` as the *host* compiled it; a module should
/// compare that against its own compiled `size_of::<ModuleEnv>()` before
/// trusting any field, so a host/module version mismatch fails loudly
/// instead of reading garbage past the end of a shorter/longer struct
/// (the same trick GNU Emacs's own `emacs_runtime`/`emacs_env` use).
#[repr(C)]
pub struct ModuleEnv {
    pub size: usize,
    /// Host-private context for the current call. Modules must treat
    /// this as opaque and never dereference it themselves -- it's only
    /// meaningful to the function pointers below, which the host
    /// implements with access to its own internals.
    pub ctx: *mut c_void,

    pub make_integer: extern "C" fn(*mut ModuleEnv, i64) -> ModuleValue,
    pub extract_integer: extern "C" fn(*mut ModuleEnv, ModuleValue) -> i64,
    pub make_float: extern "C" fn(*mut ModuleEnv, f64) -> ModuleValue,
    pub extract_float: extern "C" fn(*mut ModuleEnv, ModuleValue) -> f64,
    /// `data`/`len` are a UTF-8 byte slice (not necessarily NUL-terminated).
    pub make_string: extern "C" fn(*mut ModuleEnv, data: *const u8, len: usize) -> ModuleValue,
    /// Two-phase, same convention as real Emacs: call with `buf = NULL`
    /// to get the required length written to `*len`, then call again
    /// with a buffer of at least that size. Returns `false` on a bad
    /// value or a too-small buffer.
    pub copy_string_contents:
        extern "C" fn(*mut ModuleEnv, ModuleValue, buf: *mut u8, len: *mut usize) -> bool,
    /// `name` is a NUL-terminated C string.
    pub intern: extern "C" fn(*mut ModuleEnv, name: *const c_char) -> ModuleValue,
    pub is_not_nil: extern "C" fn(*mut ModuleEnv, ModuleValue) -> bool,
    pub eq: extern "C" fn(*mut ModuleEnv, ModuleValue, ModuleValue) -> bool,
    pub funcall: extern "C" fn(
        *mut ModuleEnv,
        func: ModuleValue,
        nargs: isize,
        args: *const ModuleValue,
    ) -> ModuleValue,
    /// `max_arity < 0` means variadic (no upper bound), matching real
    /// Emacs's `emacs_variadic_function` sentinel. `doc`/may be NULL.
    pub make_function: extern "C" fn(
        *mut ModuleEnv,
        min_arity: isize,
        max_arity: isize,
        func: ModuleCallback,
        doc: *const c_char,
        data: *mut c_void,
    ) -> ModuleValue,
    /// v1 simplification vs. real Emacs's `non_local_exit_signal` (which
    /// takes a condition symbol + a data list): just a NUL-terminated
    /// message, surfaced to elisp as a plain `error`. Documented in
    /// PLAN.md M13 as a deliberate v1 scope cut.
    pub signal_error: extern "C" fn(*mut ModuleEnv, msg: *const c_char),
}

/// Every module must export a function with exactly this C signature,
/// named `emacs_module_init`, returning 0 on success.
pub type InitFn = unsafe extern "C" fn(env: *mut ModuleEnv) -> i32;
pub const INIT_FN_NAME: &[u8] = b"emacs_module_init\0";
