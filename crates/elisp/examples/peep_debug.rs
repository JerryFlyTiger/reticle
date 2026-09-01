fn main() {
    let mut interp = elisp::new_interp();
    interp
        .eval_source("(defun f (x lo hi) (if (and (>= x lo) (<= x hi)) 1 0))")
        .map_err(|_| ())
        .unwrap();
    let id = interp.intern("f");
    let elisp::Value::Func(f) = interp.symbols[id as usize].function.clone().unwrap() else {
        panic!()
    };
    let elisp::value::Function::Lambda(l) = f.as_ref() else {
        panic!()
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
    for (i, instr) in compiled.chunk.code.iter().enumerate() {
        println!("{}: {:?}", i, instr);
    }
}
