//! M33: the eight background-highlight queries (`crates/core/queries/
//! *.scm`) were rewritten from the "tree-sitter ecosystem" philosophy
//! upstream grammars ship (call sites tagged like definitions, naming
//! heuristics like "ALL_CAPS is a constant", a catch-all `@variable` on
//! every identifier, numbers/operators tagged) to GNU font-lock's own:
//! only a definition/declaration's own name gets a face, nothing is
//! guessed from spelling, and numeric/operator tokens stay plain. This
//! file is the "philosophy" test suite the M33 plan asks for: one test
//! per language, each checking (where it applies to that language) --
//!   - a function/method/sub definition's own name gets function face;
//!   - a *call site* referencing that exact same name gets no face at
//!     all (the M25 bug this milestone exists to fix: upstream grammars
//!     routinely tag a call's callee the same way as a definition);
//!   - a short ALL-CAPS identifier used as a plain value (never as a
//!     constant/definition) -- the user-reported case (`foo BAR A`
//!     under the old `#match? "^[A-Z][A-Z0-9_]*$"` heuristic only
//!     colored BAR/A) -- gets no face, just for being spelled that way;
//!   - keywords/strings/comments still get their usual faces;
//!   - a couple of language-specific GNU conventions the M33 plan calls
//!     out by name (elisp's `:keyword` -> builtin and `t`/`nil` ->
//!     constant, a C `enum` member -> constant, a bash assignment vs. a
//!     `$VAR` reference, ...).
//!
//! Numeric literals being uncolored (also part of the M33 plan) is
//! checked on three representative languages (Rust, C, Python) rather
//! than all eight, since the rule and the risk of it silently
//! regressing are identical everywhere it applies.
//!
//! Mirrors highlight_tests.rs/lang_modes_tests.rs's own `setup`/`run`/
//! `tick_until` pattern (duplicated per file rather than shared, the
//! existing convention in this test suite) for driving the real
//! worker-thread parse-and-highlight pipeline through the elisp-visible
//! `overlays-in`/`overlay-get` surface, exactly as a real buffer would
//! see it.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(&mut interp, &format!("(insert {:?})", src));
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

/// Pump ticks (10ms apart, max 2s) until `pred` returns "t". Mirrors
/// highlight_tests.rs/lang_modes_tests.rs's helper of the same name.
fn tick_until(interp: &mut Interp, pred: &str) -> bool {
    for _ in 0..200 {
        core::idle_tick(interp, std::time::Duration::ZERO);
        if run(interp, pred) == "t" {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

const COUNT_HL: &str = "(let ((n 0))
   (dolist (ov (overlays-in (point-min) (point-max)) nil)
     (when (overlay-get ov 'treesit-hl) (setq n (1+ n))))
   n)";

/// Enable highlighting for LANG and block until at least one
/// `treesit-hl` overlay has landed.
fn enable_and_wait(interp: &mut Interp, lang: &str) {
    run(interp, &format!("(treesit-highlight-mode '{lang})"));
    assert!(
        tick_until(interp, &format!("(> {COUNT_HL} 0)")),
        "{lang}: no treesit-hl overlays ever arrived"
    );
}

/// Byte range of NEEDLE's `occurrence`-th (0-based) appearance in SRC as
/// a *whole token* -- i.e. not merely as a substring of a longer
/// identifier. Plain substring search alone isn't safe for the short,
/// common needles this suite needs (`"A"`, `"t"`, `"r"`, ...): it would
/// also match inside `"AA"`, `"const"`, `"Greeter"`. A run of
/// alphanumeric/`_`/`-` characters on either side of a match counts as
/// "still part of a longer word"; anything else (whitespace,
/// punctuation, sigils, start/end of the buffer) counts as a clean
/// boundary. That's a coarse rule, but it's enough to disambiguate
/// every needle actually used below, including multi-character ones
/// like a whole string literal or comment line (their *internal*
/// characters are irrelevant to this check -- only what's immediately
/// outside the match matters).
fn nth_token(src: &str, needle: &str, occurrence: usize) -> (usize, usize) {
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '-'
    }
    let mut seen = 0;
    let mut search_from = 0;
    loop {
        let rel = src[search_from..].find(needle).unwrap_or_else(|| {
            panic!(
                "{needle:?} occurrence {occurrence} not found as a whole token \
                 (matched {seen} time(s) before giving up, searching from byte {search_from})"
            )
        });
        let start = search_from + rel;
        let end = start + needle.len();
        let before_ok = src[..start]
            .chars()
            .next_back()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
        let after_ok = src[end..]
            .chars()
            .next()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
        search_from = start + 1;
        if before_ok && after_ok {
            if seen == occurrence {
                let start_ch = src[..start].chars().count() + 1; // 1-based, like point-min
                let end_ch = start_ch + needle.chars().count();
                return (start_ch, end_ch);
            }
            seen += 1;
        }
    }
}

/// Whether a `treesit-hl` overlay with *exactly* `[start_ch, end_ch)`
/// bounds exists; when `face` is `Some`, it must also carry that face.
/// Exact bounds (rather than "any overlay overlapping this range") is
/// deliberate: a token nested inside a wider already-colored span (e.g.
/// a `$NAME` reference inside an already-string-colored shell string)
/// must not look "colored" here just because something bigger around it
/// is.
fn exact_hl(interp: &mut Interp, start_ch: usize, end_ch: usize, face: Option<&str>) -> bool {
    let face_check = match face {
        Some(f) => format!("(eq (overlay-get ov 'face) '{f})"),
        None => "t".to_string(),
    };
    run(
        interp,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {start_ch} {end_ch}) hit)
                 (when (and (overlay-get ov 'treesit-hl)
                            {face_check}
                            (= (overlay-start ov) {start_ch})
                            (= (overlay-end ov) {end_ch}))
                   (setq hit t))))"
        ),
    ) == "t"
}

/// NEEDLE's `occurrence`-th whole-token appearance in SRC carries FACE,
/// with exactly NEEDLE's own span as the overlay bounds.
fn face_at(interp: &mut Interp, src: &str, needle: &str, occurrence: usize, face: &str) -> bool {
    let (s, e) = nth_token(src, needle, occurrence);
    exact_hl(interp, s, e, Some(face))
}

/// NEEDLE's `occurrence`-th whole-token appearance in SRC carries no
/// face at all (checked as "no exact-bounds `treesit-hl` overlay",
/// which correctly still allows a *wider* enclosing overlay -- e.g. the
/// surrounding string literal -- to exist).
fn not_colored_at(interp: &mut Interp, src: &str, needle: &str, occurrence: usize) -> bool {
    let (s, e) = nth_token(src, needle, occurrence);
    !exact_hl(interp, s, e, None)
}

// --- Rust ---------------------------------------------------------------

const RUST_SRC: &str = "const MAX_RETRY: i32 = 3;
static GREETING: &str = \"hello\";

fn call_it() -> i32 {
    add(1, 2)
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn uses_caps() {
    let x = add(A, AA);
    let _ = x;
}

fn destructure() {
    let (ta, tb) = (1, 2);
    let mut mx = 5;
    let Pair(pa, pb) = make_pair();
    let _ = (ta, tb, mx, pa, pb);
}

// trailing line comment
";

#[test]
fn rust_font_lock_philosophy() {
    let (mut i, _ed) = setup(RUST_SRC);
    enable_and_wait(&mut i, "rust");

    assert!(
        not_colored_at(&mut i, RUST_SRC, "add", 0),
        "rust: call site `add(1, 2)` must not be colored like a definition"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "add", 1, "font-lock-function-name-face"),
        "rust: `fn add` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, RUST_SRC, "A", 0),
        "rust: bare all-caps identifier `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, RUST_SRC, "AA", 0),
        "rust: bare all-caps identifier `AA` must not be colored"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "MAX_RETRY", 0, "font-lock-constant-face"),
        "rust: `const MAX_RETRY` name must get constant face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "GREETING", 0, "font-lock-constant-face"),
        "rust: `static GREETING` name must get constant face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "\"hello\"", 0, "font-lock-string-face"),
        "rust: string literal must get string face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "fn", 0, "font-lock-keyword-face"),
        "rust: `fn` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            RUST_SRC,
            "// trailing line comment",
            0,
            "font-lock-comment-face"
        ),
        "rust: line comment must get comment face"
    );
    assert!(
        not_colored_at(&mut i, RUST_SRC, "3", 0),
        "rust: numeric literal must not be colored"
    );
    // M33 review gap: the parameter/let declaration rules were verified
    // statically against node-types.json but never exercised end to
    // end — pin them here. `a` occurrence 0 is the parameter in
    // `fn add(a: ...)`; `x` occurrence 0 is `let x = ...`.
    assert!(
        face_at(&mut i, RUST_SRC, "a", 0, "font-lock-variable-name-face"),
        "rust: parameter `a` at its declaration must get variable face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "x", 0, "font-lock-variable-name-face"),
        "rust: `let x` binding name must get variable face"
    );

    // M34: `let`'s own destructuring patterns (see rust-highlights.scm's
    // header for the shape-by-shape reasoning, including why the
    // struct-pattern shapes aren't separately pinned here -- they're
    // exercised structurally the same way as the tuple/slice ones).
    assert!(
        face_at(&mut i, RUST_SRC, "ta", 0, "font-lock-variable-name-face"),
        "rust: `let (ta, tb) = ...` tuple-pattern binding must get variable face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "tb", 0, "font-lock-variable-name-face"),
        "rust: `let (ta, tb) = ...` tuple-pattern binding must get variable face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "mx", 0, "font-lock-variable-name-face"),
        "rust: `let mut mx` binding must get variable face (already covered pre-M34 by the \
         plain bare-identifier rule -- `mut` is a sibling token, not a pattern wrapper -- \
         pinned here alongside the new destructuring shapes for the same fixture)"
    );
    // M34 review finding: tuple_struct_pattern is a FOURTH pattern node
    // kind the first cut missed entirely (let Pair(a, b), let Some(x)).
    assert!(
        face_at(&mut i, RUST_SRC, "pa", 0, "font-lock-variable-name-face"),
        "rust: `let Pair(pa, pb)` tuple-struct binding must get variable face"
    );
    assert!(
        face_at(&mut i, RUST_SRC, "pb", 0, "font-lock-variable-name-face"),
        "rust: `let Pair(pa, pb)` tuple-struct binding must get variable face"
    );
    assert!(
        not_colored_at(&mut i, RUST_SRC, "Pair", 0),
        "rust: the tuple-struct TYPE name in the pattern must not be captured as a variable \
         (the type: (_) child is consumed before the identifier sibling pattern can bind)"
    );
}

// --- C --------------------------------------------------------------------

const C_PHIL_SRC: &str = "#define MAX_RETRY 3
enum Color { RED, GREEN, BLUE };
const char *greeting = \"hi\";
int proto_only(int px);

int call_it(void) {
    return add(1, 2);
}

int add(int a, int b) {
    return a + b;
}

int uses_caps(void) {
    return add(A, AA);
}

int arr[10];
int *p[3];
char **pp;
void (*cb)(int);
int (*ap)[5];
int mat[2][3];
struct Point { int coords[3]; int *label; };

// trailing comment
";

#[test]
fn c_font_lock_philosophy() {
    let (mut i, _ed) = setup(C_PHIL_SRC);
    enable_and_wait(&mut i, "c");

    assert!(
        not_colored_at(&mut i, C_PHIL_SRC, "add", 0),
        "c: call site `add(1, 2)` must not be colored like a definition"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "add", 1, "font-lock-function-name-face"),
        "c: `int add(...)` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, C_PHIL_SRC, "A", 0),
        "c: bare all-caps identifier `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, C_PHIL_SRC, "AA", 0),
        "c: bare all-caps identifier `AA` must not be colored"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "RED", 0, "font-lock-constant-face"),
        "c: enum member `RED` must get constant face"
    );
    // M33 review gap: the deliberately-unanchored function_declarator
    // rule means a body-less prototype (header style) is colored too —
    // matching real cc-mode — but no test exercised that shape.
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "proto_only",
            0,
            "font-lock-function-name-face"
        ),
        "c: a body-less prototype declaration's name must get function face"
    );
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "#define",
            0,
            "font-lock-preprocessor-face"
        ),
        "c: `#define` must get preprocessor face, not keyword face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "return", 0, "font-lock-keyword-face"),
        "c: `return` must get keyword face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "\"hi\"", 0, "font-lock-string-face"),
        "c: string literal must get string face"
    );
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "// trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "c: line comment must get comment face"
    );
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "greeting",
            0,
            "font-lock-variable-name-face"
        ),
        "c: pointer-typed initialized declaration `greeting` must get variable face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "a", 0, "font-lock-variable-name-face"),
        "c: parameter `a` must get variable face"
    );
    assert!(
        not_colored_at(&mut i, C_PHIL_SRC, "1", 0),
        "c: numeric literal must not be colored"
    );

    // M34: declaration-site coverage completion -- every declarator shape
    // the plan named, plus the struct-member (field_identifier) variant,
    // now reaches the declared name at any nesting depth via the two new
    // unanchored `array_declarator`/`pointer_declarator` rules (see
    // c-highlights.scm's header for the per-shape reasoning).
    assert!(
        face_at(&mut i, C_PHIL_SRC, "arr", 0, "font-lock-variable-name-face"),
        "c: plain array declarator `int arr[10]` name must get variable face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "p", 0, "font-lock-variable-name-face"),
        "c: array-of-pointers declarator `int *p[3]` name must get variable face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "pp", 0, "font-lock-variable-name-face"),
        "c: double-pointer declarator `char **pp` name must get variable face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "cb", 0, "font-lock-variable-name-face"),
        "c: function-pointer VARIABLE `void (*cb)(int)` name must get variable face -- not \
         function face, since `cb` is a variable being declared, not a function being defined"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "ap", 0, "font-lock-variable-name-face"),
        "c: pointer-to-array declarator `int (*ap)[5]` name must get variable face"
    );
    assert!(
        face_at(&mut i, C_PHIL_SRC, "mat", 0, "font-lock-variable-name-face"),
        "c: multi-dimensional array declarator `int mat[2][3]` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "coords",
            0,
            "font-lock-variable-name-face"
        ),
        "c: struct member array declarator `int coords[3]` (a field_identifier, not a plain \
         identifier) must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            C_PHIL_SRC,
            "label",
            0,
            "font-lock-variable-name-face"
        ),
        "c: struct member pointer declarator `int *label` (a field_identifier) must get \
         variable face"
    );
}

// --- C++ --------------------------------------------------------------------

const CPP_PHIL_SRC: &str = "class Greeter {
public:
    std::string greet() const { return name_; }
private:
    std::string name_ = \"anon\";
};

void call_it() {
    Greeter g;
    g.greet();
}

void uses_caps() {
    int r = add(A, AA);
}

void takes_ref(int &rr) {
    rr = 0;
}

int add(int a, int b) {
    return a + b;
}

// trailing comment
";

#[test]
fn cpp_font_lock_philosophy() {
    let (mut i, _ed) = setup(CPP_PHIL_SRC);
    enable_and_wait(&mut i, "cpp");

    assert!(
        face_at(
            &mut i,
            CPP_PHIL_SRC,
            "greet",
            0,
            "font-lock-function-name-face"
        ),
        "cpp: inline method definition `greet` must get function face"
    );
    assert!(
        not_colored_at(&mut i, CPP_PHIL_SRC, "greet", 1),
        "cpp: `g.greet()` call site must not be colored, even though it's the exact same \
         spelling (and the exact same `field_identifier` node kind) as the definition"
    );
    assert!(
        not_colored_at(&mut i, CPP_PHIL_SRC, "add", 0),
        "cpp: call site `add(A, AA)` must not be colored like a definition"
    );
    assert!(
        face_at(
            &mut i,
            CPP_PHIL_SRC,
            "add",
            1,
            "font-lock-function-name-face"
        ),
        "cpp: `int add(...)` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, CPP_PHIL_SRC, "A", 0),
        "cpp: bare all-caps identifier `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, CPP_PHIL_SRC, "AA", 0),
        "cpp: bare all-caps identifier `AA` must not be colored"
    );
    assert!(
        face_at(&mut i, CPP_PHIL_SRC, "Greeter", 0, "font-lock-type-face"),
        "cpp: class name `Greeter` must get type face (falls out of the blanket \
         `type_identifier` rule -- see cpp-highlights.scm's header)"
    );
    assert!(
        face_at(&mut i, CPP_PHIL_SRC, "public", 0, "font-lock-keyword-face"),
        "cpp: `public` access specifier must get keyword face"
    );
    assert!(
        face_at(&mut i, CPP_PHIL_SRC, "private", 0, "font-lock-keyword-face"),
        "cpp: `private` access specifier must get keyword face"
    );
    assert!(
        face_at(&mut i, CPP_PHIL_SRC, "\"anon\"", 0, "font-lock-string-face"),
        "cpp: string literal must get string face"
    );
    assert!(
        face_at(
            &mut i,
            CPP_PHIL_SRC,
            "// trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "cpp: line comment must get comment face"
    );
    // M34 review: pin the DOCUMENTED gap so a future "harmless" generic
    // rule can't silently start (mis)coloring it without this test
    // forcing the header's gap list to be revisited: reference
    // declarators (`int &rr`) carry their name as a positional child
    // with no declarator: field, so no current rule can reach it.
    assert!(
        not_colored_at(&mut i, CPP_PHIL_SRC, "rr", 0),
        "cpp: `int &rr` reference parameter is a documented coverage gap — \
         if this starts coloring, update cpp-highlights.scm's header gap list"
    );
}

// --- Python -----------------------------------------------------------------

const PYTHON_PHIL_SRC: &str = "def call_it():
    return add(1, 2)

def add(a, b):
    return a + b

@staticmethod
def decorated():
    pass

class Point:
    def __init__(self, x, y):
        self.x = x

def uses_caps():
    return add(A, AA)

True
False
None

# trailing comment
";

#[test]
fn python_font_lock_philosophy() {
    let (mut i, _ed) = setup(PYTHON_PHIL_SRC);
    enable_and_wait(&mut i, "python");

    assert!(
        not_colored_at(&mut i, PYTHON_PHIL_SRC, "add", 0),
        "python: call site `add(1, 2)` must not be colored like a definition"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "add",
            1,
            "font-lock-function-name-face"
        ),
        "python: `def add` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, PYTHON_PHIL_SRC, "A", 0),
        "python: bare all-caps identifier `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, PYTHON_PHIL_SRC, "AA", 0),
        "python: bare all-caps identifier `AA` must not be colored"
    );
    assert!(
        face_at(&mut i, PYTHON_PHIL_SRC, "Point", 0, "font-lock-type-face"),
        "python: `class Point` name must get type face"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "staticmethod",
            0,
            "font-lock-type-face"
        ),
        "python: decorator target must get type face, matching real python.el (not function \
         face -- a decorator isn't a call or a definition)"
    );
    assert!(
        not_colored_at(&mut i, PYTHON_PHIL_SRC, "self", 0),
        "python: `self` parameter must not be colored (python.el's own long-standing \
         special case)"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "x",
            0,
            "font-lock-variable-name-face"
        ),
        "python: an ordinary (non-`self`) parameter must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "True",
            0,
            "font-lock-constant-face"
        ),
        "python: `True` must get constant face"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "False",
            0,
            "font-lock-constant-face"
        ),
        "python: `False` must get constant face"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "None",
            0,
            "font-lock-constant-face"
        ),
        "python: `None` must get constant face"
    );
    assert!(
        face_at(&mut i, PYTHON_PHIL_SRC, "def", 0, "font-lock-keyword-face"),
        "python: `def` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            PYTHON_PHIL_SRC,
            "# trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "python: comment must get comment face"
    );
    assert!(
        not_colored_at(&mut i, PYTHON_PHIL_SRC, "1", 0),
        "python: numeric literal must not be colored"
    );
}

// --- Bash -------------------------------------------------------------------

const BASH_PHIL_SRC: &str = "NAME=world
GREETING=\"hello\"
arr=(1 2 3)

greet() {
    echo \"Hello, $NAME!\"
}

greet

for i in 1 2 3; do
    echo \"$i\"
done

call_bare() {
    echo A AA
}

# trailing comment
";

#[test]
fn bash_font_lock_philosophy() {
    let (mut i, _ed) = setup(BASH_PHIL_SRC);
    enable_and_wait(&mut i, "bash");

    assert!(
        face_at(
            &mut i,
            BASH_PHIL_SRC,
            "NAME",
            0,
            "font-lock-variable-name-face"
        ),
        "bash: `NAME=world` assignment must get variable face"
    );
    assert!(
        not_colored_at(&mut i, BASH_PHIL_SRC, "NAME", 1),
        "bash: `$NAME` reference must not itself carry variable face (only the whole \
         enclosing string, a different span, is colored)"
    );
    assert!(
        face_at(
            &mut i,
            BASH_PHIL_SRC,
            "greet",
            0,
            "font-lock-function-name-face"
        ),
        "bash: `greet()` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, BASH_PHIL_SRC, "greet", 1),
        "bash: the bare `greet` invocation (a call site) must not be colored"
    );
    assert!(
        not_colored_at(&mut i, BASH_PHIL_SRC, "A", 0),
        "bash: bare all-caps command argument `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, BASH_PHIL_SRC, "AA", 0),
        "bash: bare all-caps command argument `AA` must not be colored"
    );
    assert!(
        face_at(
            &mut i,
            BASH_PHIL_SRC,
            "\"hello\"",
            0,
            "font-lock-string-face"
        ),
        "bash: string literal must get string face"
    );
    assert!(
        face_at(&mut i, BASH_PHIL_SRC, "for", 0, "font-lock-keyword-face"),
        "bash: `for` must get keyword face"
    );
    assert!(
        face_at(&mut i, BASH_PHIL_SRC, "do", 0, "font-lock-keyword-face"),
        "bash: `do` must get keyword face"
    );
    assert!(
        face_at(&mut i, BASH_PHIL_SRC, "done", 0, "font-lock-keyword-face"),
        "bash: `done` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            BASH_PHIL_SRC,
            "# trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "bash: comment must get comment face"
    );

    // M34: an array assignment's name must get variable face exactly
    // like a plain scalar assignment -- confirmed (not a query change;
    // see bash-highlights.scm's existing `variable_assignment name:`
    // rule) that the rule never inspected the `value:` field's own node
    // kind (`word` for a scalar, `array` for `arr=(1 2 3)`) to begin
    // with.
    assert!(
        face_at(
            &mut i,
            BASH_PHIL_SRC,
            "arr",
            0,
            "font-lock-variable-name-face"
        ),
        "bash: array assignment `arr=(1 2 3)` must get variable face on the name"
    );
}

// --- Java -------------------------------------------------------------------

const JAVA_PHIL_SRC: &str = "class Greeter {
    enum Level { LOW, HIGH }

    static String greet(String name) {
        return name;
    }

    static String callIt() {
        return greet(\"world\");
    }

    static String usesCaps() {
        String outcome = greet(A);
        return outcome;
    }

    static void arrays() {
        int[] nums = null;
        int nums2[] = null;
    }
}

// trailing comment
";

#[test]
fn java_font_lock_philosophy() {
    let (mut i, _ed) = setup(JAVA_PHIL_SRC);
    enable_and_wait(&mut i, "java");

    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "greet",
            0,
            "font-lock-function-name-face"
        ),
        "java: method declaration name `greet` must get function face"
    );
    assert!(
        not_colored_at(&mut i, JAVA_PHIL_SRC, "greet", 1),
        "java: `greet(\"world\")` call site must not be colored"
    );
    assert!(
        not_colored_at(&mut i, JAVA_PHIL_SRC, "A", 0),
        "java: bare all-caps identifier `A` must not be colored"
    );
    assert!(
        face_at(&mut i, JAVA_PHIL_SRC, "Greeter", 0, "font-lock-type-face"),
        "java: class name `Greeter` must get type face"
    );
    assert!(
        face_at(&mut i, JAVA_PHIL_SRC, "Level", 0, "font-lock-type-face"),
        "java: enum name `Level` must get type face"
    );
    assert!(
        face_at(&mut i, JAVA_PHIL_SRC, "LOW", 0, "font-lock-constant-face"),
        "java: enum constant `LOW` must get constant face"
    );
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "name",
            0,
            "font-lock-variable-name-face"
        ),
        "java: method parameter `name` must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "outcome",
            0,
            "font-lock-variable-name-face"
        ),
        "java: local variable declaration `outcome` must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "\"world\"",
            0,
            "font-lock-string-face"
        ),
        "java: string literal must get string face"
    );
    assert!(
        face_at(&mut i, JAVA_PHIL_SRC, "return", 0, "font-lock-keyword-face"),
        "java: `return` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "// trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "java: line comment must get comment face"
    );

    // M34: both of Java's array-declaration spellings must get variable
    // face on the declared name -- confirmed (not a query change; see
    // java-highlights.scm's existing `variable_declarator name:` rule)
    // that neither spelling changes where the `name:` field's own
    // identifier sits, only whether `dimensions:` shows up on the type or
    // as a sibling field on the same `variable_declarator`.
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "nums",
            0,
            "font-lock-variable-name-face"
        ),
        "java: `int[] nums` (array-type-position spelling) must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            JAVA_PHIL_SRC,
            "nums2",
            0,
            "font-lock-variable-name-face"
        ),
        "java: `int nums2[]` (C-style array-after-name spelling) must get variable face"
    );
}

// --- Perl -------------------------------------------------------------------

const PERL_PHIL_SRC: &str = "sub greet {
    my ($name) = @_;
    return \"Hello, $name!\";
}

greet(\"world\");

sub uses_caps {
    my $RESULT = greet(\"x\");
    return $RESULT;
}

# trailing comment
";

#[test]
fn perl_font_lock_philosophy() {
    let (mut i, _ed) = setup(PERL_PHIL_SRC);
    enable_and_wait(&mut i, "perl");

    assert!(
        face_at(
            &mut i,
            PERL_PHIL_SRC,
            "greet",
            0,
            "font-lock-function-name-face"
        ),
        "perl: `sub greet` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, PERL_PHIL_SRC, "greet", 1),
        "perl: bare `greet(\"world\")` call site must not be colored"
    );
    assert!(
        face_at(
            &mut i,
            PERL_PHIL_SRC,
            "name",
            0,
            "font-lock-variable-name-face"
        ),
        "perl: `my ($name)` declared scalar must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            PERL_PHIL_SRC,
            "RESULT",
            0,
            "font-lock-variable-name-face"
        ),
        "perl: `my $RESULT` declared scalar must get variable face -- Perl has no bare \
         `identifier` node the way other languages do, so this (a declaration vs. a later \
         reference to the same all-caps name) is this suite's Perl-shaped analogue of the \
         cross-language \"bare ALL-CAPS not colored\" check"
    );
    assert!(
        not_colored_at(&mut i, PERL_PHIL_SRC, "RESULT", 1),
        "perl: `return $RESULT` is a reference, not a fresh declaration, and must not be \
         colored"
    );
    assert!(
        face_at(
            &mut i,
            PERL_PHIL_SRC,
            "\"world\"",
            0,
            "font-lock-string-face"
        ),
        "perl: string literal must get string face"
    );
    assert!(
        face_at(&mut i, PERL_PHIL_SRC, "sub", 0, "font-lock-keyword-face"),
        "perl: `sub` must get keyword face"
    );
    assert!(
        face_at(&mut i, PERL_PHIL_SRC, "my", 0, "font-lock-keyword-face"),
        "perl: `my` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            PERL_PHIL_SRC,
            "# trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "perl: comment must get comment face"
    );
}

// --- Elisp ------------------------------------------------------------------

const ELISP_PHIL_SRC: &str = "(defvar my-var 1)
(defconst my-const 2)

(defun greet (name)
  (message \"Hello, %s\" name))

(defun call-it ()
  (greet \"world\"))

(defmacro my-macro (x) x)

(defun uses-caps ()
  (list A AA :keyword t nil))

;; trailing comment
";

#[test]
fn elisp_font_lock_philosophy() {
    let (mut i, _ed) = setup(ELISP_PHIL_SRC);
    enable_and_wait(&mut i, "elisp");

    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            "greet",
            0,
            "font-lock-function-name-face"
        ),
        "elisp: `defun greet` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, ELISP_PHIL_SRC, "greet", 1),
        "elisp: `(greet \"world\")` call site must not be colored"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            "my-var",
            0,
            "font-lock-variable-name-face"
        ),
        "elisp: `defvar` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            "my-const",
            0,
            "font-lock-variable-name-face"
        ),
        "elisp: `defconst` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            "name",
            0,
            "font-lock-variable-name-face"
        ),
        "elisp: `defun` parameter must get variable face"
    );
    assert!(
        not_colored_at(&mut i, ELISP_PHIL_SRC, "A", 0),
        "elisp: bare all-caps symbol `A` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, ELISP_PHIL_SRC, "AA", 0),
        "elisp: bare all-caps symbol `AA` must not be colored"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            ":keyword",
            0,
            "font-lock-builtin-face"
        ),
        "elisp: a `:keyword`-style symbol must get builtin face"
    );
    assert!(
        face_at(&mut i, ELISP_PHIL_SRC, "t", 0, "font-lock-constant-face"),
        "elisp: `t` must get constant face"
    );
    assert!(
        face_at(&mut i, ELISP_PHIL_SRC, "nil", 0, "font-lock-constant-face"),
        "elisp: `nil` must get constant face"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            "\"world\"",
            0,
            "font-lock-string-face"
        ),
        "elisp: string literal must get string face"
    );
    assert!(
        face_at(&mut i, ELISP_PHIL_SRC, "defun", 0, "font-lock-keyword-face"),
        "elisp: `defun` must get keyword face"
    );
    assert!(
        face_at(
            &mut i,
            ELISP_PHIL_SRC,
            ";; trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "elisp: comment must get comment face"
    );
    assert!(
        not_colored_at(&mut i, ELISP_PHIL_SRC, "1", 0),
        "elisp: numeric literal must not be colored"
    );
}

// --- Verilog ------------------------------------------------------------

/// M38: exercises every declaration container verilog-highlights.scm
/// anchors to (module/function/task name, logic/reg/wire/string locals, a
/// port-list parameter and localparam, ANSI ports, function/task
/// parameters), a function AND a task call site (each spelled identically
/// to its own definition, the sharpest version of the M25 call-site bug
/// this whole test suite exists to catch), type keywords, a broad sample
/// of the M38 plan's named keywords, a comment, a string, and a numeric
/// literal -- plus three extra USE-SITE safety pins the dump investigation
/// specifically flagged as risky: a parameter used inside a packed-
/// dimension range (`[WIDTH-1:0]`), a signal read inside an event-control
/// sensitivity list (`posedge clk`), and a localparam used inside an
/// if-generate condition (`if (DEPTH > 0)`) -- none of which are
/// declarations, so none may be colored. The leading comment deliberately
/// avoids every identifier/keyword tested below (mirrors
/// `CPP_CLASS_SRC`'s own comment, above, avoiding "greet*" so it can't
/// shadow the method name "greet") -- an early draft of this fixture
/// worded it "synchronous counter ...", which shadowed the module name
/// `counter`'s own occurrence-0 lookup with the comment's unrelated use of
/// the same word, caught by this test itself failing on its very first
/// assertion.
///
/// Review-round addition: a module INSTANTIATION (`sub_mod u1 (.clk(clk),
/// .data(next));`), placed last (right before `endmodule`) so its own
/// four identifiers only ever ADD a new, later occurrence index to
/// anything already tested above rather than shifting one -- the instance
/// type name `sub_mod`, the instance name `u1`, the port-connection's own
/// `port_name:` field (`clk`/`data`), and the connection expression
/// itself (reusing the module's own real `clk`/`next` signals, the
/// realistic shape) are all asserted uncolored below. verilog-
/// highlights.scm's own header calls out instantiation as a deliberate,
/// call-site-like exclusion; before this addition nothing in this test
/// suite actually exercised that claim.
const VERILOG_PHIL_SRC: &str = "// hardware example with call sites
module counter #(parameter WIDTH = 8, localparam DEPTH = 4) (
  input  logic clk,
  input  logic rst_n,
  output logic [WIDTH-1:0] count
);
  logic [WIDTH-1:0] next;
  wire enable;
  reg old_style;
  string greeting = \"hello world\";

  always_ff @(posedge clk or negedge rst_n) begin
    if (!rst_n) count <= '0;
    else count <= next;
  end

  assign next = add_one(count);

  function automatic int add_one(int x);
    return x + 1;
  endfunction

  task automatic do_thing(input int v);
    old_style <= v[0];
  endtask

  initial begin
    do_thing(1);
    case (count)
      8'd0: old_style = 1'b0;
      default: old_style = 1'b1;
    endcase
  end

  generate
    if (DEPTH > 0) begin : gen_blk
      wire dummy;
    end
  endgenerate

  sub_mod u1 (.clk(clk), .data(next));
endmodule

// trailing comment
";

#[test]
fn verilog_font_lock_philosophy() {
    let (mut i, _ed) = setup(VERILOG_PHIL_SRC);
    enable_and_wait(&mut i, "verilog");

    // --- module/function/task name -> function; call sites -> plain -----

    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "counter",
            0,
            "font-lock-function-name-face"
        ),
        "verilog: `module counter` name must get function face"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "add_one", 0),
        "verilog: `add_one(count)` call site must not be colored like a definition"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "add_one",
            1,
            "font-lock-function-name-face"
        ),
        "verilog: `function ... add_one` definition name must get function face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "do_thing",
            0,
            "font-lock-function-name-face"
        ),
        "verilog: `task ... do_thing` definition name must get function face"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "do_thing", 1),
        "verilog: bare `do_thing(1)` call site must not be colored like a definition"
    );

    // --- declared names -> variable (M34: any declaration counts) -------

    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "WIDTH",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `parameter WIDTH` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "DEPTH",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `localparam DEPTH` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "clk",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: ANSI port `input logic clk` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "count",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: ANSI port `output logic [...] count` name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "next",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `logic [...] next` declaration name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "enable",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `wire enable` declaration name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "old_style",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `reg old_style` declaration name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "greeting",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: `string greeting` declaration name must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "x",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: function parameter `x` must get variable face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "v",
            0,
            "font-lock-variable-name-face"
        ),
        "verilog: task parameter `v` must get variable face"
    );

    // --- use sites (never a declaration) -> plain -----------------------

    // `nth_token`'s word-boundary check treats `-' as a word character
    // (needed elsewhere in this file for hyphenated identifiers), so it
    // can never isolate "WIDTH" from the immediately-adjacent `-1:0` in
    // verilog's own `[WIDTH-1:0]` bit-range syntax -- a shape none of the
    // other eight languages' fixtures ever produce. A direct byte-offset
    // computation (bypassing `nth_token`, same idea as lang_modes_tests.rs's
    // own `has_face_at`) is used just for this one use-site instead.
    {
        let bit_range_byte = VERILOG_PHIL_SRC
            .find("[WIDTH-1:0]")
            .expect("[WIDTH-1:0] must occur in VERILOG_PHIL_SRC");
        let width_use_start = bit_range_byte + 1; // skip the leading `[`
        let start_ch = VERILOG_PHIL_SRC[..width_use_start].chars().count() + 1;
        let end_ch = start_ch + "WIDTH".chars().count();
        assert!(
            !exact_hl(&mut i, start_ch, end_ch, None),
            "verilog: `[WIDTH-1:0]` packed-dimension USE must not be colored"
        );
    }
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "DEPTH", 1),
        "verilog: `if (DEPTH > 0)` generate-condition USE must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "clk", 1),
        "verilog: `posedge clk` sensitivity-list USE must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "count", 4),
        "verilog: `case (count)` USE must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "next", 1),
        "verilog: `count <= next` USE must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "x", 1),
        "verilog: `return x + 1` USE of the function parameter must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "v", 1),
        "verilog: `v[0]` USE of the task parameter must not be colored"
    );

    // --- module instantiation: a USE of an existing module type, exactly --
    // like a function/method call site elsewhere in this suite -- never
    // colored. `sub_mod u1 (.clk(clk), .data(next));` is the LAST
    // statement in the fixture, so `clk`/`next` here are occurrences 2/3
    // and 3 respectively, strictly AFTER every occurrence already checked
    // above (which never shift as a result).

    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "sub_mod", 0),
        "verilog: instantiated module TYPE name `sub_mod` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "u1", 0),
        "verilog: instance name `u1` must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "clk", 2),
        "verilog: named port connection's own `.clk` port-name reference must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "clk", 3),
        "verilog: named port connection's `(clk)` connected-signal expression must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "data", 0),
        "verilog: named port connection's own `.data` port-name reference must not be colored"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "next", 3),
        "verilog: named port connection's `(next)` connected-signal expression must not be colored"
    );

    // --- type keywords -> type -------------------------------------------

    assert!(
        face_at(&mut i, VERILOG_PHIL_SRC, "logic", 0, "font-lock-type-face"),
        "verilog: `logic` must get type face"
    );
    assert!(
        face_at(&mut i, VERILOG_PHIL_SRC, "wire", 0, "font-lock-type-face"),
        "verilog: `wire` must get type face"
    );
    assert!(
        face_at(&mut i, VERILOG_PHIL_SRC, "reg", 0, "font-lock-type-face"),
        "verilog: `reg` must get type face"
    );
    assert!(
        face_at(&mut i, VERILOG_PHIL_SRC, "string", 0, "font-lock-type-face"),
        "verilog: `string` must get type face"
    );
    assert!(
        face_at(&mut i, VERILOG_PHIL_SRC, "int", 0, "font-lock-type-face"),
        "verilog: `int` must get type face"
    );

    // --- keywords -> keyword (a broad sample of the M38 plan's list) ----

    for kw in [
        "module",
        "endmodule",
        "begin",
        "end",
        "if",
        "else",
        "case",
        "endcase",
        "default",
        "function",
        "endfunction",
        "task",
        "endtask",
        "initial",
        "assign",
        "always_ff",
        "input",
        "output",
        "parameter",
        "localparam",
        "posedge",
        "negedge",
        "return",
        "generate",
        "endgenerate",
        "automatic",
    ] {
        assert!(
            face_at(&mut i, VERILOG_PHIL_SRC, kw, 0, "font-lock-keyword-face"),
            "verilog: `{kw}` must get keyword face"
        );
    }

    // --- comment / string / number ----------------------------------------

    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "// hardware example with call sites",
            0,
            "font-lock-comment-face"
        ),
        "verilog: leading line comment must get comment face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "// trailing comment",
            0,
            "font-lock-comment-face"
        ),
        "verilog: trailing line comment must get comment face"
    );
    assert!(
        face_at(
            &mut i,
            VERILOG_PHIL_SRC,
            "\"hello world\"",
            0,
            "font-lock-string-face"
        ),
        "verilog: string literal must get string face"
    );
    assert!(
        not_colored_at(&mut i, VERILOG_PHIL_SRC, "8", 0),
        "verilog: numeric literal must not be colored"
    );
}
