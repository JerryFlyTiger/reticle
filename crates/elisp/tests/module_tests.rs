//! Dynamic module API (M13): loads the real, separately compiled
//! `demo-module` cdylib via `(module-load ...)` and exercises both
//! directions of the boundary -- native code only touching values passed
//! in, and native code calling back into elisp (`message`).

use elisp::printer::prin1_to_string;

fn run(src: &str) -> String {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut interp = elisp::new_interp();
            match interp.eval_source(&src) {
                Ok(v) => prin1_to_string(&interp, &v),
                Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
            }
        })
        .expect("spawn failed")
        .join()
        .expect("eval thread panicked")
}

fn demo_module_path() -> String {
    let manifest = env!("CARGO_MANIFEST_DIR"); // .../crates/elisp
    let target = std::path::Path::new(manifest).join("../../target");
    let lib_name = if cfg!(target_os = "macos") {
        "libdemo_module.dylib"
    } else if cfg!(target_os = "windows") {
        "demo_module.dll"
    } else {
        "libdemo_module.so"
    };
    for profile in ["debug", "release"] {
        let p = target.join(profile).join(lib_name);
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    panic!(
        "demo-module dylib not found in target/{{debug,release}}; \
         run `cargo build -p demo-module` first"
    );
}

#[test]
fn module_load_registers_a_native_function() {
    let path = demo_module_path();
    let src = format!("(module-load {:?}) (mymod-double 21)", path);
    assert_eq!(run(&src), "42");
}

#[test]
fn module_function_calls_back_into_elisp() {
    let path = demo_module_path();
    let src = format!("(module-load {:?}) (mymod-greet \"World\")", path);
    assert_eq!(run(&src), "\"Hello from a native module, World!\"");
}

#[test]
fn module_function_is_a_real_function_value() {
    let path = demo_module_path();
    let src = format!(
        "(module-load {:?}) (functionp (symbol-function 'mymod-double))",
        path
    );
    assert_eq!(run(&src), "t");
}

#[test]
fn wrong_type_argument_is_signaled_not_silently_coerced() {
    let path = demo_module_path();
    let src = format!("(module-load {:?}) (mymod-double \"not a number\")", path);
    assert!(run(&src).starts_with("ERROR"));
}

#[test]
fn wrong_arity_is_signaled() {
    let path = demo_module_path();
    let src = format!("(module-load {:?}) (mymod-double 1 2)", path);
    assert!(run(&src).starts_with("ERROR"));
}

#[test]
fn loading_a_nonexistent_module_errors_cleanly() {
    let r = run("(module-load \"/no/such/module.dylib\")");
    assert!(r.starts_with("ERROR"), "expected an error, got {}", r);
}
