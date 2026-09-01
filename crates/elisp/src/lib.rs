// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

pub mod bglog;
pub mod builtins;
pub mod bytecode;
pub mod compiler;
pub mod error;
pub mod eval;
pub mod gc;
pub mod interp;
pub mod jit;
pub mod json;
pub mod lsp;
pub mod module;
pub mod peephole;
pub mod printer;
pub mod reader;
pub mod regex;
pub mod shell;
pub mod value;
pub mod vm;
pub mod worker;

pub use error::{EvalResult, Flow};
pub use interp::Interp;
pub use value::Value;

pub const PRELUDE: &str = include_str!("../lisp/prelude.el");

/// A fully initialized interpreter: builtins registered, prelude loaded
/// and byte-compiled (shipped lisp always runs compiled, like GNU
/// Emacs's .elc files).
pub fn new_interp() -> Interp {
    let mut interp = Interp::new();
    builtins::register_all(&mut interp);
    if let Err(flow) = interp.eval_source(PRELUDE) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading prelude: {}", msg);
    }
    compiler::compile_all_defined(&mut interp);
    interp
}
