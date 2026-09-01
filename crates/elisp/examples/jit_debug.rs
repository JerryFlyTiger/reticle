//! Diagnostic tool for the JIT: prints a function's compiled bytecode,
//! whether `jit::try_compile` accepts it, and (if native args are
//! given) calls the resulting native code directly.
//!
//! Usage: jit_debug '(defun f (n) ...)' f [arg1 arg2 ...]
//! Set JIT_DEBUG=1 in the environment for step-by-step trace output
//! from the JIT itself (leader shapes, generated Cranelift IR, etc).
use elisp::value::Function;
use elisp::Value;

fn main() {
    let mut interp = elisp::new_interp();
    let src = std::env::args().nth(1).unwrap_or_else(|| {
        "(defun loop-sum (n)
           (let ((acc 0) (i 0))
             (while (< i n)
               (setq acc (+ acc i))
               (setq i (1+ i)))
             acc))"
            .to_string()
    });
    let name = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "loop-sum".to_string());
    let native_args: Vec<i64> = std::env::args()
        .skip(3)
        .map(|s| s.parse().unwrap())
        .collect();

    interp.eval_source(&src).map_err(|_| ()).unwrap();
    let id = interp.intern(&name);
    let func = interp.symbols[id as usize].function.clone().unwrap();
    let Value::Func(f) = &func else {
        panic!("{} is not a function", name)
    };
    let Function::Lambda(l) = f.as_ref() else {
        panic!("{} is not interpreted", name)
    };
    let compiled = elisp::compiler::compile_parsed(
        &mut interp,
        l.params.clone(),
        l.body.clone(),
        l.interactive.borrow().clone(),
        l.lexical,
        l.env.clone(),
    )
    .map_err(|_| ())
    .unwrap();

    println!("chunk instrs:");
    for (i, instr) in compiled.chunk.code.iter().enumerate() {
        println!("  {}: {:?}", i, instr);
    }

    let native = elisp::jit::try_compile(&interp, &compiled);
    println!("native eligible: {}", native.is_some());
    if let Some(n) = native {
        if native_args.len() == n.arity() {
            println!("native result = {:?}", n.call(&native_args));
        } else if !native_args.is_empty() {
            println!(
                "(skipped call: expected {} args, got {})",
                n.arity(),
                native_args.len()
            );
        }
    }
}
