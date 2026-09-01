// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

use std::io::{BufRead, Write};

use elisp::printer::prin1_to_string;
use elisp::reader::{ReadError, Reader};

/// Single source of truth for the usage synopsis: printed by `usage()` on
/// errors (to stderr) and folded into `--help` (to stdout), so the two can
/// never drift apart.
const SYNOPSIS: &str = "\
usage: reticle [FILE] [-nw|--tui|--gui] [-q]
       reticle --repl
       reticle --eval EXPR
       reticle --script FILE";

fn print_version() {
    println!("reticle {}", env!("CARGO_PKG_VERSION"));
}

fn print_help() {
    println!("{}", SYNOPSIS);
    println!();
    println!("Options:");
    println!("  FILE            file to open on startup (GUI/TUI only)");
    println!("  -nw, --tui      run in the terminal instead of opening a GUI window");
    println!("  --gui           run the GUI window (this is the default; only useful");
    println!("                  to spell out explicitly, since -nw/--tui always wins");
    println!("                  over --gui no matter where it appears on the line)");
    println!("  -q              skip loading the user init file");
    println!("  --repl          start an interactive elisp REPL on stdin/stdout");
    println!("  --eval EXPR     evaluate EXPR and print the result");
    println!("  --script FILE   load and evaluate FILE, then exit");
    println!("  -h, --help      print this help and exit");
    println!("  -v, -V, --version");
    println!("                  print version information and exit");
    println!();
    println!("Notes:");
    println!("  --repl, --eval, and --script only take effect as the very first");
    println!("  argument. Elsewhere on the command line: if it's a GUI/TUI flag");
    println!("  position, a stray --repl/--eval/--script starting with '-' is");
    println!("  rejected as an unknown option; if it's the value slot right after");
    println!("  another --eval/--script (i.e. args[1]), or any trailing argument");
    println!("  after --repl/--eval EXPR/--script FILE, it is silently ignored.");
    println!("  -h/--help and -v/-V/--version take priority over every other");
    println!("  argument in a flag position — even before --eval/--script/--repl,");
    println!("  so they can never be shadowed by an accidental GUI window or REPL");
    println!("  session. The one place they are NOT a flag is the value slot right");
    println!("  after --eval/--script: `--eval --help` evaluates --help as elisp");
    println!("  and fails, exactly as it did before these options existed.");
    println!();
    println!("See \"man reticle\" for the full manual.");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // --help/--version are checked before anything else — including the
    // batch-mode dispatch below — so that e.g. `reticle --eval EXPR
    // --help` or `reticle --repl --help` prints help instead of
    // silently starting a REPL or evaluating EXPR. First match by
    // left-to-right scan wins if both appear.
    //
    // Position rule: index 1 is a VALUE, not a flag, when args[0] is
    // "--eval" or "--script" (that's the only place run_batch ever reads
    // args[1] from — see the batch-flag check right below). Every other
    // index is always a flag position. Without this exclusion, `--eval
    // --help` or `--script -v` would have their argument silently
    // swallowed by this scan instead of reaching the interpreter/loader,
    // which is supposed to error on it.
    let eval_or_script_value_index = match args.first().map(|s| s.as_str()) {
        Some("--eval") | Some("--script") => Some(1),
        _ => None,
    };
    for (i, a) in args.iter().enumerate() {
        if Some(i) == eval_or_script_value_index {
            continue;
        }
        match a.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-v" | "-V" | "--version" => {
                print_version();
                std::process::exit(0);
            }
            _ => {}
        }
    }
    // Batch modes (REPL / script / worker — no editor window) and the
    // TUI run on a thread with a large stack: deep elisp recursion uses
    // the Rust stack.
    let batch = matches!(
        args.first().map(|s| s.as_str()),
        Some("--repl") | Some("--eval") | Some("--script") | Some("--worker")
    );
    let tui = !batch && args.iter().any(|a| a == "-nw" || a == "--tui");
    if batch || tui {
        let child = std::thread::Builder::new()
            .stack_size(512 * 1024 * 1024)
            .spawn(move || if tui { run_tui(args) } else { run_batch(args) })
            .expect("failed to spawn main thread");
        std::process::exit(child.join().expect("main thread panicked"));
    }
    // Like GNU Emacs: no -nw means the GUI editor. Its event loop must
    // stay on the real main thread (AppKit).
    std::process::exit(run_gui(args));
}

/// Options for an editor session (GUI or TUI), GNU Emacs style:
/// `reticle [FILE] [-q]`, plus `-nw` picked off earlier by main.
struct SessionOpts {
    skip_init: bool,
    file: Option<String>,
}

fn parse_session(args: &[String]) -> Result<SessionOpts, String> {
    let mut opts = SessionOpts {
        skip_init: false,
        file: None,
    };
    for a in args {
        match a.as_str() {
            "-q" => opts.skip_init = true,
            // Mode flags, already consumed by main; accepted in any order.
            "-nw" | "--tui" | "--gui" => {}
            s if s.starts_with('-') => return Err(format!("unknown option: {}", s)),
            s => {
                if opts.file.is_none() {
                    opts.file = Some(s.to_string());
                }
            }
        }
    }
    Ok(opts)
}

fn usage() {
    eprintln!("{}", SYNOPSIS);
}

/// Shared editor-session setup: interpreter, editor, init file, and the
/// file named on the command line.
fn start_session(
    opts: &SessionOpts,
) -> (
    elisp::Interp,
    std::rc::Rc<std::cell::RefCell<core::editor::Editor>>,
) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    if !opts.skip_init {
        core::load_user_init(&mut interp, &ed);
    }
    // M29: evil-mode defaults to on for an interactive session; init.el
    // can opt out with (setq evil-auto-enable nil). Batch modes
    // (--repl/--eval/--script/--worker) never call start_session, so
    // they're untouched. Errors must not abort startup, same guarantee
    // load_user_init already gives init.el itself — echo instead of
    // unwrapping.
    if let Err(flow) =
        interp.eval_source("(when (and (fboundp 'evil-mode) evil-auto-enable) (evil-mode 1))")
    {
        let msg = interp.describe_flow(&flow);
        ed.borrow_mut().echo = Some(format!("Error enabling evil-mode: {}", msg));
    }
    if let Some(path) = &opts.file {
        let quoted = format!("(find-file-internal {:?})", path);
        if let Err(flow) = interp.eval_source(&quoted) {
            eprintln!("error: {}", interp.describe_flow(&flow));
        }
    }
    (interp, ed)
}

fn run_gui(args: Vec<String>) -> i32 {
    let opts = match parse_session(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e);
            usage();
            return 2;
        }
    };
    let (interp, ed) = start_session(&opts);
    match frontend_gui::run_gui(interp, ed) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("GUI error: {}", e);
            1
        }
    }
}

fn run_tui(args: Vec<String>) -> i32 {
    let opts = match parse_session(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", e);
            usage();
            return 2;
        }
    };
    let (mut interp, ed) = start_session(&opts);
    match frontend_tui::run_tui(&mut interp, &ed) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("terminal error: {}", e);
            1
        }
    }
}

fn run_batch(args: Vec<String>) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("--repl") => repl(),
        Some("--eval") => {
            let Some(src) = args.get(1) else {
                eprintln!("usage: reticle --eval EXPR");
                return 2;
            };
            let mut interp = elisp::new_interp();
            match interp.eval_source(src) {
                Ok(v) => {
                    println!("{}", prin1_to_string(&interp, &v));
                    0
                }
                Err(flow) => {
                    eprintln!("error: {}", interp.describe_flow(&flow));
                    1
                }
            }
        }
        Some("--script") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: reticle --script FILE");
                return 2;
            };
            let mut interp = elisp::new_interp();
            match interp.load_file(path) {
                Ok(_) => 0,
                Err(flow) => {
                    eprintln!("error: {}", interp.describe_flow(&flow));
                    1
                }
            }
        }
        Some("--worker") => {
            // Child worker process: evaluate framed sexps from stdin,
            // write framed results to stdout, until the parent closes
            // stdin. Not meant to be run interactively by hand.
            let mut interp = elisp::new_interp();
            elisp::worker::run_worker_loop(&mut interp);
            0
        }
        _ => unreachable!("run_batch called for a non-batch mode"),
    }
}

fn repl() -> i32 {
    let mut interp = elisp::new_interp();
    println!("reticle elisp REPL — Ctrl-D to exit");
    let stdin = std::io::stdin();
    let mut pending = String::new();
    loop {
        if pending.is_empty() {
            print!("elisp> ");
        } else {
            print!("  ...> ");
        }
        std::io::stdout().flush().ok();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => {
                println!();
                return 0;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {}", e);
                return 1;
            }
        }
        pending.push_str(&line);
        if pending.trim().is_empty() {
            pending.clear();
            continue;
        }

        // Re-parse the whole pending buffer; ask for more input if incomplete.
        let src = pending.clone();
        let mut reader = Reader::new(&src);
        let mut forms = Vec::new();
        let mut incomplete = false;
        loop {
            match reader.read(&mut interp) {
                Ok(Some(f)) => forms.push(f),
                Ok(None) => break,
                Err(ReadError::Incomplete) => {
                    incomplete = true;
                    break;
                }
                Err(ReadError::Syntax(msg)) => {
                    eprintln!("read error: {}", msg);
                    forms.clear();
                    incomplete = false;
                    pending.clear();
                    break;
                }
            }
        }
        if incomplete {
            continue;
        }
        pending.clear();
        for form in &forms {
            match elisp::eval::eval(&mut interp, form, &None) {
                Ok(v) => println!("{}", prin1_to_string(&interp, &v)),
                Err(flow) => eprintln!("error: {}", interp.describe_flow(&flow)),
            }
        }
    }
}
