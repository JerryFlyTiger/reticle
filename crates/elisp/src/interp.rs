use std::collections::HashMap;

use crate::error::{EvalResult, Flow};
use crate::value::{SymId, Value};

pub struct SymbolData {
    pub name: String,
    pub value: Option<Value>,
    pub function: Option<Value>,
    pub plist: Value,
    /// defvar'd: `let` binds it dynamically even under lexical-binding.
    pub special: bool,
    /// Set iff this symbol names a special form: the evaluator
    /// dispatches on this tag with one array access instead of
    /// allocating + string-comparing the symbol name per form.
    pub special_form: Option<SpecialForm>,
    /// Name starts with ':' — computed once at intern time so the
    /// per-variable-read check in `eval_symbol` is a flag load instead
    /// of a string prefix comparison (P2.3).
    pub keyword: bool,
}

/// Every special form, tagged on its symbol at startup (M10 layer-0
/// dispatch optimization).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpecialForm {
    Quote,
    Function,
    Lambda,
    If,
    Cond,
    While,
    Progn,
    Prog1,
    Prog2,
    And,
    Or,
    Let,
    LetStar,
    Setq,
    Defvar,
    Defconst,
    Defun,
    Defmacro,
    ConditionCase,
    UnwindProtect,
    Catch,
    Interactive,
    Backquote,
}

pub const SPECIAL_FORMS: &[(&str, SpecialForm)] = &[
    ("quote", SpecialForm::Quote),
    ("function", SpecialForm::Function),
    ("lambda", SpecialForm::Lambda),
    ("if", SpecialForm::If),
    ("cond", SpecialForm::Cond),
    ("while", SpecialForm::While),
    ("progn", SpecialForm::Progn),
    ("prog1", SpecialForm::Prog1),
    ("prog2", SpecialForm::Prog2),
    ("and", SpecialForm::And),
    ("or", SpecialForm::Or),
    ("let", SpecialForm::Let),
    ("let*", SpecialForm::LetStar),
    ("setq", SpecialForm::Setq),
    ("defvar", SpecialForm::Defvar),
    ("defconst", SpecialForm::Defconst),
    ("defun", SpecialForm::Defun),
    ("defmacro", SpecialForm::Defmacro),
    ("condition-case", SpecialForm::ConditionCase),
    ("unwind-protect", SpecialForm::UnwindProtect),
    ("catch", SpecialForm::Catch),
    ("interactive", SpecialForm::Interactive),
    ("`", SpecialForm::Backquote),
];

/// Frequently used symbols, interned once at startup.
pub struct Syms {
    pub t: SymId,
    pub quote: SymId,
    pub function: SymId,
    pub lambda: SymId,
    pub macro_: SymId,
    pub backquote: SymId,
    pub unquote: SymId,
    pub unquote_splicing: SymId,
    pub optional: SymId,
    pub rest: SymId,
    pub error: SymId,
    pub error_conditions: SymId,
    pub error_message: SymId,
    pub wrong_type_argument: SymId,
    pub wrong_number_of_arguments: SymId,
    pub void_variable: SymId,
    pub void_function: SymId,
    pub setting_constant: SymId,
    pub arith_error: SymId,
    pub args_out_of_range: SymId,
    pub end_of_file: SymId,
    pub invalid_read_syntax: SymId,
    pub user_error: SymId,
    pub interactive: SymId,
    pub declare: SymId,
    pub excessive_depth: SymId,
    pub elisp_timeout: SymId,
    pub regexp_too_complex: SymId,
}

pub struct Interp {
    names: HashMap<String, SymId>,
    pub symbols: Vec<SymbolData>,
    pub syms: Syms,
    /// Current binding mode for `load`/`eval` context.
    pub lexical_binding: bool,
    pub depth: usize,
    pub max_depth: usize,
    pub features: Vec<SymId>,
    pub load_path: Vec<String>,
    gensym_counter: u64,
    /// Output sink for `message`/`princ` etc.; frontends can capture it.
    pub output: Option<OutputSink>,
    /// Host extension slot: the editor core stores its state here so its
    /// builtins can reach it through &mut Interp.
    pub ext: Option<std::rc::Rc<dyn std::any::Any>>,
    /// Last regexp match (string-match / re-search-forward set it;
    /// match-beginning / match-end / match-string read it).
    pub match_data: crate::regex::MatchData,
    /// P1.2: compiled-pattern cache for `crate::regex::compile` — see
    /// its doc comment. `Regex` is immutable once built (no interior
    /// mutable state; `run`'s capture-group scratch lives on the call
    /// stack, not on `self`), so sharing one compiled instance across
    /// calls via `Rc` is safe.
    pub regex_cache: std::collections::HashMap<String, std::rc::Rc<crate::regex::Regex>>,
    /// Cycle-collector state (see `crate::gc`).
    pub gc: crate::gc::GcState,
    /// Extra GC root enumerators registered by the embedder (the editor
    /// contributes its buffers/keymaps/pending commands through this).
    pub gc_roots: Vec<crate::gc::RootProvider>,
    /// Cooperative-interruption deadline (M15). While set, the eval and
    /// VM dispatch loops periodically consult the clock and signal
    /// `elisp-timeout` once it passes — the JetBrains-style "cancel slow
    /// work when the user is waiting" primitive, with the same unwind
    /// semantics as C-g in real Emacs (unwind-protect cleanups run).
    /// None (the normal editing state) costs one predictable branch per
    /// dispatch step.
    pub deadline: Option<std::time::Instant>,
    /// Dispatch-step counter so the clock is only consulted every 64
    /// steps while a deadline is armed (Instant::now() is cheap but not
    /// free; the mask keeps budgeted-execution overhead well under 1%).
    pub deadline_steps: u32,
}

pub type OutputSink = Box<dyn FnMut(&str)>;

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    pub fn new() -> Interp {
        let mut names = HashMap::new();
        let mut symbols = Vec::new();
        let mut intern = |name: &str| -> SymId {
            let id = symbols.len() as SymId;
            names.insert(name.to_string(), id);
            symbols.push(SymbolData {
                name: name.to_string(),
                value: None,
                function: None,
                plist: Value::Nil,
                special: false,
                special_form: None,
                keyword: name.starts_with(':'),
            });
            id
        };
        // "nil" gets id 0 so the reader can map it; Value::Nil is its canonical form.
        intern("nil");
        let syms = Syms {
            t: intern("t"),
            quote: intern("quote"),
            function: intern("function"),
            lambda: intern("lambda"),
            macro_: intern("macro"),
            backquote: intern("`"),
            unquote: intern(","),
            unquote_splicing: intern(",@"),
            optional: intern("&optional"),
            rest: intern("&rest"),
            error: intern("error"),
            error_conditions: intern("error-conditions"),
            error_message: intern("error-message"),
            wrong_type_argument: intern("wrong-type-argument"),
            wrong_number_of_arguments: intern("wrong-number-of-arguments"),
            void_variable: intern("void-variable"),
            void_function: intern("void-function"),
            setting_constant: intern("setting-constant"),
            arith_error: intern("arith-error"),
            args_out_of_range: intern("args-out-of-range"),
            end_of_file: intern("end-of-file"),
            invalid_read_syntax: intern("invalid-read-syntax"),
            user_error: intern("user-error"),
            interactive: intern("interactive"),
            declare: intern("declare"),
            excessive_depth: intern("excessive-lisp-nesting"),
            elisp_timeout: intern("elisp-timeout"),
            regexp_too_complex: intern("regexp-too-complex"),
        };
        let mut interp = Interp {
            names,
            symbols,
            syms,
            lexical_binding: true,
            depth: 0,
            max_depth: 4000,
            features: Vec::new(),
            load_path: Vec::new(),
            gensym_counter: 0,
            output: None,
            ext: None,
            match_data: crate::regex::MatchData::default(),
            regex_cache: HashMap::new(),
            gc: crate::gc::GcState::default(),
            gc_roots: Vec::new(),
            deadline: None,
            deadline_steps: 0,
        };
        let t = interp.syms.t;
        interp.symbols[t as usize].value = Some(Value::Sym(t));
        interp.symbols[t as usize].special = true;
        for (name, sf) in SPECIAL_FORMS {
            let id = interp.intern(name);
            interp.symbols[id as usize].special_form = Some(*sf);
        }
        interp.define_standard_errors();
        interp
    }

    pub fn intern(&mut self, name: &str) -> SymId {
        if let Some(&id) = self.names.get(name) {
            return id;
        }
        let id = self.symbols.len() as SymId;
        self.names.insert(name.to_string(), id);
        self.symbols.push(SymbolData {
            name: name.to_string(),
            value: None,
            function: None,
            plist: Value::Nil,
            special: false,
            special_form: None,
            keyword: name.starts_with(':'),
        });
        id
    }

    pub fn intern_soft(&self, name: &str) -> Option<SymId> {
        self.names.get(name).copied()
    }

    pub fn sym_name(&self, id: SymId) -> &str {
        &self.symbols[id as usize].name
    }

    pub fn gensym(&mut self, prefix: &str) -> SymId {
        loop {
            self.gensym_counter += 1;
            let name = format!("{}{}", prefix, self.gensym_counter);
            if !self.names.contains_key(&name) {
                return self.intern(&name);
            }
        }
    }

    #[inline]
    pub fn is_keyword(&self, id: SymId) -> bool {
        self.symbols[id as usize].keyword
    }

    /// The symbol a Value refers to, treating Nil as symbol `nil` (id 0).
    pub fn as_sym(&self, v: &Value) -> Option<SymId> {
        match v {
            Value::Nil => Some(0),
            Value::Sym(id) => Some(*id),
            _ => None,
        }
    }

    pub fn sym_value(&self, id: SymId) -> Option<Value> {
        if id == 0 {
            return Some(Value::Nil);
        }
        self.symbols[id as usize].value.clone()
    }

    pub fn set_sym_value(&mut self, id: SymId, v: Value) {
        self.symbols[id as usize].value = Some(v);
    }

    pub fn plist_get(&self, id: SymId, prop: SymId) -> Value {
        let mut cur = self.symbols[id as usize].plist.clone();
        while let Value::Cons(c) = cur {
            let b = c.borrow();
            if let Value::Sym(p) = b.car {
                if p == prop {
                    return b.cdr.car();
                }
            }
            cur = b.cdr.cdr();
        }
        Value::Nil
    }

    pub fn plist_put(&mut self, id: SymId, prop: SymId, val: Value) {
        let mut cur = self.symbols[id as usize].plist.clone();
        while let Value::Cons(c) = cur {
            let b = c.borrow();
            if let Value::Sym(p) = b.car {
                if p == prop {
                    if let Value::Cons(vc) = &b.cdr {
                        vc.borrow_mut().car = val;
                        return;
                    }
                }
            }
            cur = b.cdr.cdr();
        }
        let old = self.symbols[id as usize].plist.clone();
        self.symbols[id as usize].plist = Value::cons(Value::Sym(prop), Value::cons(val, old));
    }

    fn define_standard_errors(&mut self) {
        let error = self.syms.error;
        self.define_error(error, "error", &[]);
        for (sym, msg) in [
            (self.syms.wrong_type_argument, "Wrong type argument"),
            (
                self.syms.wrong_number_of_arguments,
                "Wrong number of arguments",
            ),
            (
                self.syms.void_variable,
                "Symbol's value as variable is void",
            ),
            (
                self.syms.void_function,
                "Symbol's function definition is void",
            ),
            (
                self.syms.setting_constant,
                "Attempt to set a constant symbol",
            ),
            (self.syms.arith_error, "Arithmetic error"),
            (self.syms.args_out_of_range, "Args out of range"),
            (self.syms.end_of_file, "End of file during parsing"),
            (self.syms.invalid_read_syntax, "Invalid read syntax"),
            (self.syms.user_error, ""),
            (self.syms.excessive_depth, "Lisp nesting exceeds max depth"),
            (
                self.syms.regexp_too_complex,
                "Regexp match gave up: pattern is too expensive on this text",
            ),
        ] {
            self.define_error(sym, msg, &[error]);
        }
        // Deliberately NOT a child of `error`: like `quit` in real Emacs,
        // a time-budget interruption must not be swallowed by the
        // `ignore-errors` / `(condition-case ... (error ...))` wrappers
        // that ordinary code legitimately uses — only an explicit
        // `elisp-timeout` handler may catch it, or the hook watchdog's
        // guard rail (M15) would be defeated by any hook that wraps its
        // body in ignore-errors.
        //
        // `regexp-too-complex` (M81) is the deliberate opposite: IS a
        // child of `error` (registered above, in the loop). Where
        // `elisp-timeout` means "the user wants to cancel", a regex
        // engine giving up on one pathological pattern means "this
        // particular call failed" — the caller has legitimate reason to
        // catch it with a plain `(condition-case nil ... (error ...))`
        // and fall back (e.g. to a plain-string search), the same way it
        // would handle any other single-call failure. Silently returning
        // "no match" instead would be worse: it would be indistinguishable
        // from a real non-match, and callers like `evil.el`'s `:s///`
        // would report a flatly false "Pattern not found".
        let timeout = self.syms.elisp_timeout;
        self.define_error(timeout, "Elisp execution exceeded its time budget", &[]);
    }

    pub fn define_error(&mut self, sym: SymId, message: &str, parents: &[SymId]) {
        let mut conditions = vec![Value::Sym(sym)];
        for &p in parents {
            let pc = self.plist_get(p, self.syms.error_conditions);
            if let Some(items) = pc.list_to_vec() {
                for item in items {
                    if !conditions.iter().any(|c| c.eq(&item)) {
                        conditions.push(item);
                    }
                }
            } else {
                conditions.push(Value::Sym(p));
            }
        }
        let conds = Value::list(conditions);
        self.plist_put(sym, self.syms.error_conditions, conds);
        let msg = Value::string(message);
        self.plist_put(sym, self.syms.error_message, msg);
    }

    pub fn signal(&mut self, error_symbol: SymId, data: Vec<Value>) -> Flow {
        Flow::Signal {
            error_symbol: Value::Sym(error_symbol),
            data: Value::list(data),
        }
    }

    pub fn error(&mut self, msg: impl Into<String>) -> Flow {
        let e = self.syms.error;
        self.signal(e, vec![Value::string(msg.into())])
    }

    /// Signal `regexp-too-complex` (M81) — see `define_standard_errors`
    /// for why it, unlike `elisp-timeout`, IS a plain `error` child.
    ///
    /// M81 R1: empty data (`vec![]`), matching `elisp-timeout`'s own
    /// `signal(e, vec![])` (see `define_standard_errors`) — NOT
    /// `vec![msg]` with the error's own `error-message` re-passed as
    /// signal data. `describe_flow` formats as "message: data", so
    /// passing the message as data a second time printed the same
    /// sentence twice, and — because the echo area truncates to 72
    /// chars (M70/M77) — what the user actually saw was that duplicate
    /// getting cut off mid-repeat, not a clean message.
    pub fn regexp_too_complex(&mut self) -> Flow {
        let e = self.syms.regexp_too_complex;
        self.signal(e, vec![])
    }

    pub fn wrong_type(&mut self, expected: &str, got: &Value) -> Flow {
        let pred = self.intern(expected);
        let e = self.syms.wrong_type_argument;
        self.signal(e, vec![Value::Sym(pred), got.clone()])
    }

    pub fn out(&mut self, text: &str) {
        match &mut self.output {
            Some(f) => f(text),
            None => print!("{}", text),
        }
    }

    /// The M15 interruption check, called from the eval entry and the VM
    /// dispatch loop. With no deadline armed (the normal case) this is a
    /// single predictable branch. While armed, the clock is consulted
    /// every 64 dispatch steps; once exceeded, the deadline is CLEARED
    /// first and then `elisp-timeout` is signaled — clearing first means
    /// `unwind-protect` cleanup forms run unbudgeted instead of being
    /// immediately re-interrupted mid-cleanup (the same pragmatic choice
    /// real Emacs makes by clearing quit-flag when the quit fires).
    #[inline]
    pub fn check_deadline(&mut self) -> Result<(), Flow> {
        if let Some(dl) = self.deadline {
            self.deadline_steps = self.deadline_steps.wrapping_add(1);
            if self.deadline_steps & 0x3F == 0 && std::time::Instant::now() >= dl {
                self.deadline = None;
                let e = self.syms.elisp_timeout;
                return Err(self.signal(e, vec![]));
            }
        }
        Ok(())
    }

    /// Human-readable description of a signal, for REPL/minibuffer display.
    pub fn describe_flow(&mut self, flow: &Flow) -> String {
        match flow {
            Flow::Signal { error_symbol, data } => {
                let msg = if let Some(id) = self.as_sym(error_symbol) {
                    match self.plist_get(id, self.syms.error_message) {
                        Value::Str(s) => s.to_string(),
                        _ => self.sym_name(id).to_string(),
                    }
                } else {
                    crate::printer::prin1_to_string(self, error_symbol)
                };
                let parts: Vec<String> = data
                    .list_to_vec()
                    .unwrap_or_default()
                    .iter()
                    .map(|v| match v {
                        Value::Str(s) => s.to_string(),
                        other => crate::printer::prin1_to_string(self, other),
                    })
                    .collect();
                if parts.is_empty() {
                    msg
                } else if msg.is_empty() {
                    parts.join(", ")
                } else {
                    format!("{}: {}", msg, parts.join(", "))
                }
            }
            Flow::Throw { .. } => "No catch for throw".to_string(),
        }
    }

    /// Evaluate all forms in `src`, honoring a `lexical-binding` file cookie.
    pub fn eval_source(&mut self, src: &str) -> EvalResult {
        let saved = self.lexical_binding;
        self.lexical_binding = detect_lexical_cookie(src).unwrap_or(saved);
        let result = self.eval_source_inner(src);
        self.lexical_binding = saved;
        result
    }

    fn eval_source_inner(&mut self, src: &str) -> EvalResult {
        let mut reader = crate::reader::Reader::new(src);
        let mut last = Value::Nil;
        loop {
            match reader.read(self) {
                Ok(Some(form)) => last = crate::eval::eval(self, &form, &None)?,
                Ok(None) => return Ok(last),
                Err(e) => return Err(e.into_flow(self)),
            }
        }
    }

    pub fn load_file(&mut self, path: &str) -> EvalResult {
        let src = std::fs::read_to_string(path)
            .map_err(|e| self.error(format!("Cannot open load file: {}: {}", path, e)))?;
        self.eval_source(&src)
    }
}

/// Look for `lexical-binding: t/nil` in the first line, Emacs file-variable style.
fn detect_lexical_cookie(src: &str) -> Option<bool> {
    let first = src.lines().next()?;
    let idx = first.find("lexical-binding:")?;
    let rest = first[idx + "lexical-binding:".len()..].trim_start();
    if rest.starts_with("nil") {
        Some(false)
    } else if !rest.is_empty() {
        Some(true)
    } else {
        None
    }
}
