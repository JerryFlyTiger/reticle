//! M25: seven more tree-sitter-backed major modes (c, c++, python, sh,
//! java, perl, emacs-lisp) layered onto M24's `auto-mode-alist` dispatch
//! (see modes_tests.rs for that machinery's own tests), plus the
//! `interpreter-mode-alist` / #! shebang fallback for extensionless
//! scripts and background highlighting (crate::highlight) for a sample
//! of the new languages. See crates/core/lisp/modes.el.

use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

fn setup() -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
}

/// Process-wide monotonic counter (see modes_tests.rs's `unique_seq`):
/// parallel tests in the same process share a pid and can land on the
/// same nanosecond, so the timestamp alone occasionally collides.
fn unique_seq() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// A scratch directory that deletes itself on drop.
///
/// The old shape put `std::fs::remove_dir_all` as the LAST line of each
/// test body -- exactly the line a panicking test never reaches, so
/// cleanup ran on success and leaked on failure, backwards from what you
/// want. By 2026-08-14 that had left 298 stale directories under
/// $TMPDIR, the oldest three days old. `Drop` runs during unwind too.
struct Scratch(std::path::PathBuf);

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn temp_dir(tag: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!(
        "reticle_langmodes_{}_{}_{}_{}",
        tag,
        std::process::id(),
        rand_suffix(),
        unique_seq()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn write_and_open(
    interp: &mut Interp,
    dir: &std::path::Path,
    name: &str,
    contents: &str,
) -> String {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    run(
        interp,
        &format!("(find-file-internal {:?})", path.to_str().unwrap()),
    )
}

// --- 1. extension -> mode, `treesit-language-available-p`, and a real parse ----

/// Shared body: open FILENAME (containing SRC) via `find-file`, check it
/// landed in EXPECTED_MODE, that `treesit-language-available-p` is t for
/// LANG_SYM, and that `treesit-parser-create` on that language actually
/// parses SRC into a tree whose root is EXPECTED_ROOT_KIND (elisp string
/// syntax, e.g. "\"translation_unit\"") with real children -- not just
/// an empty or all-ERROR tree.
fn check_language(
    tag: &str,
    filename: &str,
    src: &str,
    expected_mode: &str,
    lang_sym: &str,
    expected_root_kind: &str,
) {
    let (mut i, _ed) = setup();
    let dir = temp_dir(tag);
    let r = write_and_open(&mut i, &dir, filename, src);
    assert!(!r.starts_with("ERROR"), "{}: find-file failed: {}", tag, r);
    assert_eq!(
        run(&mut i, "(major-mode-internal-get)"),
        expected_mode,
        "{}: wrong major mode",
        tag
    );

    assert_eq!(
        run(
            &mut i,
            &format!("(treesit-language-available-p '{})", lang_sym)
        ),
        "t",
        "{}: treesit-language-available-p should be t for '{}",
        tag,
        lang_sym
    );

    run(
        &mut i,
        &format!("(setq p (treesit-parser-create '{}))", lang_sym),
    );
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    assert_eq!(
        run(&mut i, "(treesit-node-type root)"),
        expected_root_kind,
        "{}: unexpected root node type",
        tag
    );
    let child_count: i64 = run(&mut i, "(treesit-node-child-count root)")
        .parse()
        .unwrap();
    assert!(
        child_count > 0,
        "{}: root node parsed with no children",
        tag
    );
}

const C_SRC: &str = "// compute the sum of two ints\n#include <stdio.h>\n\nint add(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    const char *msg = \"hello\";\n    printf(\"%s %d\\n\", msg, add(1, 2));\n    return 0;\n}\n";

const CPP_SRC: &str = "// greeter\n#include <iostream>\n#include <string>\n\nstd::string greet(const std::string& name) {\n    return \"Hello, \" + name;\n}\n\nint main() {\n    std::cout << greet(\"world\") << std::endl;\n    return 0;\n}\n";

/// Dedicated C++ sample for `cpp_background_highlight_finds_faces_only_the_cpp_half_of_the_query_can_produce`:
/// unlike `CPP_SRC` above, this one has a `class` with an access
/// specifier and a method, needed to exercise captures that only the
/// cpp *half* of the concatenated highlight query can produce (see that
/// test). Deliberately worded so "public"/"greet" each occur exactly
/// once and don't collide as substrings of anything else in the buffer
/// (e.g. the comment avoids the word "greet*" so it can't shadow the
/// method name "greet", and "greet" itself never appears as a prefix of
/// a longer identifier).
const CPP_CLASS_SRC: &str = "// example\n#include <string>\n\nclass Greeter {\npublic:\n    std::string greet() const { return \"hello\"; }\nprivate:\n    int count_ = 0;\n};\n";

const PYTHON_SRC: &str = "# compute the sum of two ints\ndef add(a, b):\n    return a + b\n\ndef main():\n    msg = \"hello\"\n    print(msg, add(1, 2))\n\nmain()\n";

const BASH_SRC: &str = "#!/usr/bin/env bash\n# greet the caller\ngreet() {\n    local name=\"${1:-world}\"\n    echo \"Hello, ${name}!\"\n}\ngreet \"reticle\"\n";

const JAVA_SRC: &str = "// greeter\npublic class Greeter {\n    static String greet(String name) {\n        return \"Hello, \" + name;\n    }\n    public static void main(String[] args) {\n        System.out.println(greet(\"world\"));\n    }\n}\n";

const PERL_SRC: &str = "# greet the caller\nsub greet {\n    my ($name) = @_;\n    return \"Hello, $name!\";\n}\nprint greet(\"world\"), \"\\n\";\n";

const ELISP_SRC: &str = ";; compute the sum of two numbers\n(defun add (a b)\n  \"Add A and B.\"\n  (+ a b))\n\n(message \"%d\" (add 1 2))\n";

/// M38: mirrors the go/no-go survey's own representative snippet -- ANSI
/// module ports, a parameter, `always_ff`, `assign`, a function, a task,
/// `initial`, `case`, and wire/reg/logic declarations -- confirmed
/// `has_error: false` by the same one-off dump tool used throughout this
/// milestone (see indent.el/verilog-highlights.scm's own headers).
const VERILOG_SRC: &str = "// synchronous up-counter\nmodule counter #(parameter WIDTH = 8) (\n  input  logic clk,\n  input  logic rst_n,\n  output logic [WIDTH-1:0] count\n);\n  logic [WIDTH-1:0] next;\n  wire enable;\n  reg old_style;\n\n  always_ff @(posedge clk or negedge rst_n) begin\n    if (!rst_n) count <= '0;\n    else count <= next;\n  end\n\n  assign next = count + 1'b1;\n\n  function automatic int add_one(int x);\n    return x + 1;\n  endfunction\n\n  task automatic do_thing(input int v);\n    old_style <= v[0];\n  endtask\n\n  initial begin\n    case (count)\n      8'd0: old_style = 1'b0;\n      default: old_style = 1'b1;\n    endcase\n  end\nendmodule\n";

#[test]
fn c_file_gets_c_mode_and_treesit_parses_real_c_source() {
    check_language("c", "prog.c", C_SRC, "c-mode", "c", "\"translation_unit\"");
}

#[test]
fn c_header_also_gets_c_mode() {
    check_language(
        "c-header",
        "prog.h",
        C_SRC,
        "c-mode",
        "c",
        "\"translation_unit\"",
    );
}

#[test]
fn cpp_file_gets_cxx_mode_and_treesit_parses_real_cpp_source() {
    check_language(
        "cpp",
        "prog.cpp",
        CPP_SRC,
        "c++-mode",
        "cpp",
        "\"translation_unit\"",
    );
    check_language(
        "cpp-cc",
        "prog.cc",
        CPP_SRC,
        "c++-mode",
        "cpp",
        "\"translation_unit\"",
    );
    check_language(
        "cpp-hpp",
        "prog.hpp",
        CPP_SRC,
        "c++-mode",
        "cpp",
        "\"translation_unit\"",
    );
}

#[test]
fn python_file_gets_python_mode_and_treesit_parses_real_python_source() {
    check_language(
        "python",
        "prog.py",
        PYTHON_SRC,
        "python-mode",
        "python",
        "\"module\"",
    );
}

#[test]
fn sh_file_gets_sh_mode_and_treesit_parses_real_bash_source() {
    check_language("sh", "prog.sh", BASH_SRC, "sh-mode", "bash", "\"program\"");
    check_language(
        "bash-ext",
        "prog.bash",
        BASH_SRC,
        "sh-mode",
        "bash",
        "\"program\"",
    );
}

#[test]
fn java_file_gets_java_mode_and_treesit_parses_real_java_source() {
    check_language(
        "java",
        "Greeter.java",
        JAVA_SRC,
        "java-mode",
        "java",
        "\"program\"",
    );
}

#[test]
fn perl_file_gets_perl_mode_and_treesit_parses_real_perl_source() {
    check_language(
        "perl",
        "prog.pl",
        PERL_SRC,
        "perl-mode",
        "perl",
        "\"source_file\"",
    );
    check_language(
        "perl-pm",
        "Prog.pm",
        PERL_SRC,
        "perl-mode",
        "perl",
        "\"source_file\"",
    );
}

#[test]
fn elisp_file_gets_emacs_lisp_mode_and_treesit_parses_real_elisp_source() {
    check_language(
        "elisp",
        "prog.el",
        ELISP_SRC,
        "emacs-lisp-mode",
        "elisp",
        "\"source_file\"",
    );
}

#[test]
fn verilog_file_gets_verilog_mode_and_treesit_parses_real_verilog_source() {
    check_language(
        "verilog",
        "counter.v",
        VERILOG_SRC,
        "verilog-mode",
        "verilog",
        "\"source_file\"",
    );
    check_language(
        "verilog-sv",
        "counter.sv",
        VERILOG_SRC,
        "verilog-mode",
        "verilog",
        "\"source_file\"",
    );
    check_language(
        "verilog-vh",
        "counter.vh",
        VERILOG_SRC,
        "verilog-mode",
        "verilog",
        "\"source_file\"",
    );
    check_language(
        "verilog-svh",
        "counter.svh",
        VERILOG_SRC,
        "verilog-mode",
        "verilog",
        "\"source_file\"",
    );
}

#[test]
fn treesit_language_available_p_is_t_for_all_seven_new_languages() {
    let (mut i, _ed) = setup();
    for lang in ["c", "cpp", "python", "bash", "java", "perl", "elisp"] {
        assert_eq!(
            run(&mut i, &format!("(treesit-language-available-p '{})", lang)),
            "t",
            "expected '{} to be available",
            lang
        );
    }
}

// --- 2. shebang fallback (normal-mode--shebang-mode / interpreter-mode-alist) ----

#[test]
fn shebang_bash_dispatches_to_sh_mode_with_no_extension() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-bash");
    let r = write_and_open(&mut i, &dir, "myscript", "#!/bin/bash\necho hi\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "sh-mode");
}

#[test]
fn shebang_env_python3_dispatches_to_python_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-envpy3");
    let r = write_and_open(
        &mut i,
        &dir,
        "myscript",
        "#!/usr/bin/env python3\nprint(\"hi\")\n",
    );
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "python-mode");
}

#[test]
fn shebang_plain_perl_dispatches_to_perl_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-perl");
    let r = write_and_open(
        &mut i,
        &dir,
        "myscript",
        "#!/usr/bin/perl\nprint \"hi\\n\";\n",
    );
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "perl-mode");
}

/// A file extension match always wins over a shebang: `auto-mode-alist`
/// is consulted first (see `normal-mode`), so a `.py` file with a bash
/// shebang (a real-world occurrence, e.g. polyglot build scripts) still
/// gets python-mode.
#[test]
fn file_extension_takes_priority_over_shebang() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("ext-over-shebang");
    let r = write_and_open(&mut i, &dir, "prog.py", "#!/bin/bash\nprint(\"hi\")\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "python-mode");
}

/// An unrecognized interpreter (or no shebang at all, already covered by
/// modes_tests.rs's plain-text case) falls all the way back to
/// fundamental-mode rather than erroring.
#[test]
fn shebang_with_unknown_interpreter_falls_back_to_fundamental_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-unknown");
    let r = write_and_open(&mut i, &dir, "myscript", "#!/usr/bin/ruby\nputs 'hi'\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "fundamental-mode");
}

/// A version-suffixed interpreter (pyenv/homebrew commonly install
/// "python3.11", not a bare "python3") must still match: this is exactly
/// why `interpreter-mode-alist` keys are regexps matched with
/// `string-match` rather than literal strings compared with `equal`.
#[test]
fn shebang_versioned_python_dispatches_to_python_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-py311");
    let r = write_and_open(
        &mut i,
        &dir,
        "myscript",
        "#!/usr/bin/env python3.11\nprint(\"hi\")\n",
    );
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "python-mode");
}

/// `env`'s own flags (here `-S`, used to pass a multi-word command) must
/// be skipped when hunting for the real interpreter -- otherwise "-S"
/// itself gets misread as the interpreter name.
#[test]
fn shebang_env_dash_s_dispatches_to_python_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-env-dash-s");
    let r = write_and_open(
        &mut i,
        &dir,
        "myscript",
        "#!/usr/bin/env -S python3 -u\nprint(\"hi\")\n",
    );
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "python-mode");
}

/// A direct (non-`env`) interpreter line can carry its own flags too
/// (here bash's `-e`); `normal-mode--shebang-interpreter` only unwraps
/// `env`, so this exercises the plain path once more with an argument
/// present, guarding against a regression that broke on any trailing
/// word at all.
#[test]
fn shebang_bash_dash_e_dispatches_to_sh_mode() {
    let (mut i, _ed) = setup();
    let dir = temp_dir("shebang-bash-dash-e");
    let r = write_and_open(&mut i, &dir, "myscript", "#!/bin/bash -e\necho hi\n");
    assert!(!r.starts_with("ERROR"), "find-file failed: {}", r);
    assert_eq!(run(&mut i, "(major-mode-internal-get)"), "sh-mode");
}

/// `interpreter-mode-alist` is a `defvar` like `auto-mode-alist`, so a
/// user's own regexp entry (prepended with `add-to-list`) is consulted
/// too -- the GNU extensibility idiom already covered for
/// `auto-mode-alist` by modes_tests.rs's
/// `user_can_register_a_custom_auto_mode_alist_entry`.
#[test]
fn user_can_register_a_custom_interpreter_mode_alist_entry() {
    let (mut i, _ed) = setup();
    run(&mut i, "(defvar my-interp-mode-ran nil)");
    run(
        &mut i,
        "(defun my-custom-interp-mode ()
           (major-mode-internal-set 'my-custom-interp-mode)
           (setq my-interp-mode-ran t))",
    );
    let r = run(
        &mut i,
        r#"(add-to-list 'interpreter-mode-alist '("^customscript[0-9.]*$" . my-custom-interp-mode))"#,
    );
    assert!(!r.starts_with("ERROR"), "add-to-list failed: {}", r);

    let dir = temp_dir("shebang-custom-interp");
    let opened = write_and_open(
        &mut i,
        &dir,
        "myscript",
        "#!/usr/bin/env customscript2.0\ndata\n",
    );
    assert!(!opened.starts_with("ERROR"), "find-file failed: {}", opened);
    assert_eq!(
        run(&mut i, "(major-mode-internal-get)"),
        "my-custom-interp-mode"
    );
    assert_eq!(run(&mut i, "my-interp-mode-ran"), "t");
}

// --- 3. background highlighting end-to-end for a sample of the new languages ----

/// Pump ticks (10ms apart, max 2s) until `pred` returns "t". Mirrors
/// highlight_tests.rs's helper of the same name.
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

fn has_face_anywhere(interp: &mut Interp, face: &str) -> bool {
    run(
        interp,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in (point-min) (point-max)) hit)
                 (when (eq (overlay-get ov 'face) '{}) (setq hit t))))",
            face
        ),
    ) == "t"
}

/// Whether a `treesit-hl` overlay covering *exactly* the first
/// occurrence of NEEDLE in SRC (1-based char positions, matching this
/// editor's buffer conventions) carries FACE. Sharper than
/// `has_face_anywhere`: it pins the face to one specific token instead
/// of "this face showed up somewhere", which matters when the point is
/// to prove a *particular* capture rule fired rather than just that
/// highlighting ran at all. Panics if NEEDLE doesn't occur in SRC (a
/// test-fixture bug, not something a real run should hit).
fn has_face_at(interp: &mut Interp, src: &str, needle: &str, face: &str) -> bool {
    let byte_start = src
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} must occur in src"));
    let start_ch = src[..byte_start].chars().count() + 1; // 1-based, like point-min
    let end_ch = start_ch + needle.chars().count();
    run(
        interp,
        &format!(
            "(let (hit)
               (dolist (ov (overlays-in {start_ch} {end_ch}) hit)
                 (when (and (overlay-get ov 'treesit-hl)
                            (eq (overlay-get ov 'face) '{face})
                            (= (overlay-start ov) {start_ch})
                            (= (overlay-end ov) {end_ch}))
                   (setq hit t))))"
        ),
    ) == "t"
}

/// Basic end-to-end smoke test that `Lang::C` highlighting is wired up
/// at all: `lang_query` loads `crates/core/queries/c-highlights.scm` via
/// `include_str!` and the worker thread actually runs it over real C
/// source. Before M33 this also doubled as a check against a
/// copy-paste of the wrong constant name (tree-sitter-c's own
/// `HIGHLIGHT_QUERY` is singular, unlike most other grammars here); M33
/// replaced every grammar-crate constant with our own trimmed file, so
/// that particular risk is gone, but "does this language's file
/// actually load and produce the basic categories" is still worth a
/// dedicated smoke test. Sharper, GNU-font-lock-*philosophy* assertions
/// (definition vs. call site, no naming heuristics, ...) live in
/// font_lock_philosophy_tests.rs instead of piling more of them on here.
#[test]
fn c_background_highlight_finds_keyword_string_and_comment_faces() {
    let (mut i, _ed) = setup();
    run(&mut i, &format!("(insert {:?})", C_SRC));
    run(&mut i, "(treesit-highlight-mode 'c)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    assert!(
        has_face_anywhere(&mut i, "font-lock-keyword-face"),
        "no keyword face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-string-face"),
        "no string face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-comment-face"),
        "no comment face found"
    );
}

/// Same basic end-to-end smoke test as the C one above, for
/// `crates/core/queries/python-highlights.scm`; see that test's doc
/// comment for what this level of test does and doesn't try to prove.
#[test]
fn python_background_highlight_finds_keyword_string_and_comment_faces() {
    let (mut i, _ed) = setup();
    run(&mut i, &format!("(insert {:?})", PYTHON_SRC));
    run(&mut i, "(treesit-highlight-mode 'python)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    assert!(
        has_face_anywhere(&mut i, "font-lock-keyword-face"),
        "no keyword face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-string-face"),
        "no string face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-comment-face"),
        "no comment face found"
    );
}

/// Elisp was already the one grammar whose highlight query isn't a
/// crate-provided constant at all even before M33 -- tree-sitter-elisp
/// 1.6's published crate comments its own `HIGHLIGHTS_QUERY` constant
/// out, so this was always our own vendored/trimmed copy loaded with
/// `include_str!` (see crates/core/queries/elisp-highlights.scm). Same
/// basic end-to-end smoke test as the C one above otherwise; see that
/// test's doc comment for what this level of test does and doesn't try
/// to prove.
#[test]
fn elisp_background_highlight_finds_keyword_string_and_comment_faces() {
    let (mut i, _ed) = setup();
    run(&mut i, &format!("(insert {:?})", ELISP_SRC));
    run(&mut i, "(treesit-highlight-mode 'elisp)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    assert!(
        has_face_anywhere(&mut i, "font-lock-keyword-face"),
        "no keyword face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-string-face"),
        "no string face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-comment-face"),
        "no comment face found"
    );
}

/// Before M33, C++'s highlight query was our own runtime *composition*
/// -- `tree_sitter_c::HIGHLIGHT_QUERY` concatenated with
/// `tree_sitter_cpp::HIGHLIGHT_QUERY` via a `OnceLock` (see the removed
/// `cpp_highlight_query` in highlight.rs's history) -- because upstream
/// cpp's own query alone is nearly empty (it assumes C's is layered
/// underneath; see the M25 report). M33 deleted that runtime
/// concatenation: crates/core/queries/cpp-highlights.scm is now a single
/// self-contained file that deliberately *duplicates* the C-derived
/// rules by hand instead of composing them from two separate crate
/// constants at load time (see that file's own header). This test
/// exercises the same risk the old one did -- a regression that
/// silently lost the C-derived half, or the cpp-specific half, of that
/// one file -- just against the new architecture: checking for
/// keyword/string/comment faces alone wouldn't catch losing the
/// cpp-specific half (a plain C file already has all three), so beyond
/// that baseline this also asserts two things only the cpp-specific
/// rules can produce:
///   - "public"/"private" access specifiers as keyword face -- neither
///     word is in the C-derived keyword list at all.
///   - the method name "greet" (a `field_identifier` used as a
///     `function_declarator`'s own name) getting
///     font-lock-function-name-face -- the C-derived rules only ever
///     match a plain `identifier` in that position, never a
///     `field_identifier`; only cpp-highlights.scm's own added rule
///     reaches this node kind at all.
#[test]
fn cpp_background_highlight_is_self_contained_and_finds_both_c_derived_and_cpp_specific_faces() {
    let (mut i, _ed) = setup();
    run(&mut i, &format!("(insert {:?})", CPP_CLASS_SRC));
    run(&mut i, "(treesit-highlight-mode 'cpp)");
    assert!(tick_until(&mut i, &format!("(> {} 0)", COUNT_HL)));
    // C-derived baseline: present because cpp-highlights.scm duplicates
    // these rules by hand, not because anything C-specific was loaded.
    assert!(
        has_face_anywhere(&mut i, "font-lock-keyword-face"),
        "no keyword face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-string-face"),
        "no string face found"
    );
    assert!(
        has_face_anywhere(&mut i, "font-lock-comment-face"),
        "no comment face found"
    );
    // cpp-specific evidence: neither word is in the C-derived keyword list.
    assert!(
        has_face_at(&mut i, CPP_CLASS_SRC, "public", "font-lock-keyword-face"),
        "cpp-specific keyword `public` didn't get keyword face -- cpp-highlights.scm's own \
         keyword list may not be active"
    );
    assert!(
        has_face_at(&mut i, CPP_CLASS_SRC, "private", "font-lock-keyword-face"),
        "cpp-specific keyword `private` didn't get keyword face -- cpp-highlights.scm's own \
         keyword list may not be active"
    );
    // cpp-specific evidence: only cpp-highlights.scm's own
    // function_declarator/field_identifier rule can attach
    // function-name face to a method name.
    assert!(
        has_face_at(
            &mut i,
            CPP_CLASS_SRC,
            "greet",
            "font-lock-function-name-face"
        ),
        "method name `greet` didn't get function-name face -- cpp-highlights.scm's own \
         method-definition rule may not be active"
    );
}

// --- perf probe -------------------------------------------------------

/// M25 perf probe (run deliberately: `cargo test --release -p core
/// --test lang_modes_tests -- --ignored --nocapture
/// measure_full_parse_time_per_new_language`): full-buffer reparse cost
/// for each of the seven languages M25 added, on a synthetic >=50k-char
/// buffer built by repeating a real snippet of that language (Rust
/// itself already has its own dedicated 35k-char/~2ms benchmark from
/// M12, see PLAN.md, so it's not repeated here). Timed the same way
/// M12's was: through the elisp-visible `treesit-parser-root-node` round
/// trip (same call `treesit-highlight-mode`/tests drive), after one
/// warmup call, averaged over 100 iterations, release build. Target:
/// <10ms/parse per language (M12's own bar, scaled from 35k to 50k+
/// chars, was ~2ms/parse).
#[test]
#[ignore]
fn measure_full_parse_time_per_new_language() {
    let cases: &[(&str, &str)] = &[
        ("c", C_SRC),
        ("cpp", CPP_SRC),
        ("python", PYTHON_SRC),
        ("bash", BASH_SRC),
        ("java", JAVA_SRC),
        ("perl", PERL_SRC),
        ("elisp", ELISP_SRC),
    ];
    for &(lang_sym, snippet) in cases {
        let mut src = String::new();
        while src.chars().count() < 50_000 {
            src.push_str(snippet);
        }
        let total_chars = src.chars().count();
        let (mut i, _ed) = setup();
        run(&mut i, &format!("(insert {:?})", src));
        run(
            &mut i,
            &format!("(setq p (treesit-parser-create '{}))", lang_sym),
        );
        run(&mut i, "(treesit-parser-root-node p)"); // warm up
        let iters = 100u32;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            run(&mut i, "(treesit-parser-root-node p)");
        }
        let per = start.elapsed() / iters;
        eprintln!("{lang_sym}: {total_chars} chars, {per:?}/parse ({iters} iters)");
        assert!(
            per < std::time::Duration::from_millis(10),
            "{lang_sym}: {per:?}/parse exceeds the 10ms target"
        );
    }
}
