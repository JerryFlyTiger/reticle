// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

//! Real, loadable proof-of-concept module for M13's dynamic module API.
//! Compiled as a cdylib and loaded at runtime via `(module-load ...)` --
//! this crate depends on nothing from `elisp`/`core`, only on the plain
//! `#[repr(C)]` ABI in `module-abi`, exactly as a third-party module
//! would.
//!
//! Registers two functions to demonstrate both directions of the
//! boundary: `mymod-double` only touches values passed in (no callback
//! into elisp), `mymod-greet` calls back into elisp (`message`) from
//! native code, proving the "native code drives the interpreter" path
//! actually works, not just "interpreter drives native code".

use std::os::raw::c_void;

use module_abi::{ModuleEnv, ModuleValue};

/// # Safety
///
/// `env` must be a valid, non-null pointer to a `ModuleEnv` the host just
/// built for this exact call, per the `module-abi` ABI contract -- true
/// of every call the host itself makes into `emacs_module_init`.
#[no_mangle]
pub unsafe extern "C" fn emacs_module_init(env: *mut ModuleEnv) -> i32 {
    let e = unsafe { &*env };

    let sym_double = (e.intern)(env, c"mymod-double".as_ptr());
    let fn_double = (e.make_function)(
        env,
        1,
        1,
        mymod_double,
        c"Double an integer.".as_ptr(),
        std::ptr::null_mut(),
    );
    bind(env, sym_double, fn_double);

    let sym_greet = (e.intern)(env, c"mymod-greet".as_ptr());
    let fn_greet = (e.make_function)(
        env,
        1,
        1,
        mymod_greet,
        c"Greet NAME via (message ...).".as_ptr(),
        std::ptr::null_mut(),
    );
    bind(env, sym_greet, fn_greet);

    0
}

unsafe fn bind(env: *mut ModuleEnv, sym: ModuleValue, func: ModuleValue) {
    let e = unsafe { &*env };
    let fset = (e.intern)(env, c"fset".as_ptr());
    let args = [sym, func];
    (e.funcall)(env, fset, 2, args.as_ptr());
}

extern "C" fn mymod_double(
    env: *mut ModuleEnv,
    _nargs: isize,
    args: *const ModuleValue,
    _data: *mut c_void,
) -> ModuleValue {
    let e = unsafe { &*env };
    let arg = unsafe { *args };
    let n = (e.extract_integer)(env, arg);
    (e.make_integer)(env, n * 2)
}

extern "C" fn mymod_greet(
    env: *mut ModuleEnv,
    _nargs: isize,
    args: *const ModuleValue,
    _data: *mut c_void,
) -> ModuleValue {
    let e = unsafe { &*env };
    let name_val = unsafe { *args };

    // Two-phase copy_string_contents: first call asks for the length.
    let mut len: usize = 0;
    (e.copy_string_contents)(env, name_val, std::ptr::null_mut(), &mut len);
    let mut buf = vec![0u8; len];
    (e.copy_string_contents)(env, name_val, buf.as_mut_ptr(), &mut len);
    // `len` includes the NUL the host writes; drop it before decoding.
    let name = String::from_utf8_lossy(&buf[..len.saturating_sub(1)]).into_owned();

    let greeting = format!("Hello from a native module, {}!", name);
    let greeting_val = (e.make_string)(env, greeting.as_ptr(), greeting.len());

    let message_sym = (e.intern)(env, c"message".as_ptr());
    let args = [greeting_val];
    (e.funcall)(env, message_sym, 1, args.as_ptr())
}
