//! R4 (M83 fix round R3): a lisp-source hygiene check for the exact
//! failure shape that burned an entire fix round chasing a phantom
//! "compiler bug" -- three UNESCAPED `"` characters inside a `defun`
//! docstring (search.el, M83) closed that docstring early. The reader
//! accepted the file silently (the overall paren/string count stayed
//! balanced -- an EVEN number of stray quotes does that), and the
//! leftover prose past the premature close became ordinary BODY forms:
//! bare symbols (a lone `\n` outside any string reads as the SYMBOL
//! `n`, not the two-character escape) and, in this specific case, TWO
//! more bare STRING literals (the "works today"/"silently ..." prose
//! happened to itself contain a balanced quote pair). The function only
//! ever failed at CALL time, with an error ("void: n") that named
//! nothing in the function's actual source -- seven rounds were spent
//! diagnosing a compiler defect that never existed.
//!
//! The rule this test enforces: a `defun`/`defmacro` body containing a
//! bare string literal at any position OTHER than the first (the
//! legitimate docstring slot) is flagged as a violation -- a second,
//! later top-level string in a function body is dead code by
//! construction (its value is simply discarded), and in every case
//! actually found in this tree so far, was exactly this failure mode:
//! prose from a docstring that closed early, happening to also contain
//! its own balanced quote pair. Scanning every `.el` file actually
//! compiled into this editor (`crates/core/lisp/`, `crates/elisp/lisp/`)
//! with this rule found exactly the three occurrences fixed in
//! search.el and zero false positives anywhere else -- as of THIS
//! writing. Both boundaries of that claim, corrected here (fix round
//! R3, tail review) rather than left overstated:
//!
//! - FALSE POSITIVES this rule WILL flag if they're ever written: a
//!   `defun`/`defmacro' whose body is a docstring followed by an
//!   explicit literal string return value, e.g.
//!   `(defun f () "docstring" "literal-return-value")' -- entirely
//!   legitimate lisp (returning a constant string is not dead code),
//!   but structurally indistinguishable from this rule's own trigger
//!   shape. None exist in this tree today; one being added in the
//!   future would need an explicit exception here, not a same-shape
//!   violation silently ignored.
//! - WHAT THIS RULE CANNOT SEE: only a bare STRING literal past the
//!   docstring slot is checked -- a bare SYMBOL is deliberately NOT
//!   flagged, because "return a local variable" is completely ordinary
//!   lisp. This incident's own leftover prose happened to close early
//!   AND land on its own balanced quote pair, producing a bare STRING
//!   (this rule's trigger) in addition to the bare SYMBOL `n` (from an
//!   out-of-string backslash-n) that actually caused the runtime failure. A
//!   FUTURE premature-docstring-closure whose leftover prose does NOT
//!   happen to contain a balanced quote pair -- pure prose, symbols
//!   only, no second string -- would pass this check completely. This
//!   test catches THIS incident's specific shape and any future one
//!   that happens to share it; it is not a general docstring-closed-
//!   early detector.
//!
//! Deliberately NOT implemented as a `crates/elisp` compiler warning
//! (out of scope for this milestone, much larger blast radius) -- this
//! is a source-hygiene test over the actual shipped `.el` files, using
//! the project's own reader (`elisp::reader::Reader`) to parse each file
//! into forms without evaluating any of them.
//!
//! ## Second failure mode added (M85 tail fix): unbalanced quotes
//!
//! The docstring-hygiene incident above (search.el, M83) happened to
//! land on an EVEN number of stray unescaped `"` -- the file's overall
//! quote count stayed balanced, so the reader accepted it silently and
//! the failure only ever showed up at CALL time. That's the shape the
//! check above exists for.
//!
//! An ODD (unbalanced) number of stray quotes is a DIFFERENT failure
//! mode with the opposite visibility problem. It is NOT silent: the
//! reader itself fails to parse the file (typically surfacing as
//! `Invalid read syntax: unexpected )` once the mismatched string
//! swallows a later `)` or vice versa), which means `eval_source`
//! aborts partway through the file at editor startup and nearly every
//! integration test goes red at once. Loud, but useless on its own --
//! the failure is reported by whatever test happened to trip over the
//! broken prelude first, and its error carries no file name or line
//! number, only "something in lisp init failed." M85's implementer lost
//! a full round to a binary-search-by-hand just to find WHICH `.el` and
//! WHERE. `scan_file` below used to swallow this case entirely (`Err(_)
//! => break`, on the reasoning "some other test already catches this" --
//! true, but that other test can't say where). It no longer does: a
//! parse failure is now itself a `Violation` carrying the file path, the
//! reader's own error message, and a line number derived from
//! `Reader::pos()` (the char offset the reader had reached when it gave
//! up). This does not replace the "some other test already catches
//! this" backstop -- it exists so that when that backstop trips, this
//! test's failure message is the one that actually says where to look.
//!
//! **Precision limitation, stated plainly (H4, fix-round tail
//! re-review correction)**: the reported line is where the READER GAVE
//! UP, not where the actual mistake was TYPED. An unbalanced quote
//! makes the reader treat everything after it as still being inside a
//! string literal -- it keeps consuming characters (including any `)`
//! that would otherwise have closed a form) until it either hits a
//! LATER stray quote that happens to close the runaway string, or
//! reaches EOF. Either way, `Reader::pos()` at the moment of failure
//! can sit an ARBITRARY distance past the real mistake -- not a fixed
//! offset, but one that grows with however much source text the
//! swallowed span happens to contain (this project's own real M85
//! incident -- an unescaped quote inside a long docstring paragraph --
//! is exactly this shape: the reported failure point was several lines
//! past the actual typo, not on top of it). `scan_file_detects_and_
//! reports_a_genuinely_unparseable_file' (this file's own test) pins a
//! SMALL, one-line drift because its synthetic mistake is deliberately
//! minimal (a single stray top-level `)` right after a short, complete
//! form) -- it demonstrates the MECHANISM fires and reports SOME line,
//! not that the drift is always this small. The reported line is a
//! STARTING point for a human to search backward from, not a promise
//! that the mistake is AT that line or even close to it.

use elisp::value::Value;
use elisp::Interp;

/// Every `.el` file actually compiled into this editor (see
/// `crates/core/src/lib.rs`'s `include_str!` constants and
/// `crates/elisp/src/interp.rs`'s own prelude load) -- listed by
/// directory rather than by individual file so a future new `.el` file
/// is covered automatically without this test needing an update.
fn el_source_dirs() -> Vec<std::path::PathBuf> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest_dir.join("lisp"),
        manifest_dir.join("../elisp/lisp"),
    ]
}

/// A `defun`/`defmacro` body's string literal at BODY index >= 1 (index
/// 0 is the legitimate docstring slot, if present) -- see this file's
/// header for why this is never meaningful lisp.
struct Violation {
    file: std::path::PathBuf,
    function: String,
    line: usize,
}

/// A file the reader failed to parse at all -- see this file's header,
/// "Second failure mode added (M85 tail fix)".
struct ParseFailure {
    file: std::path::PathBuf,
    line: usize,
    message: String,
}

/// 1-based line number of BYTE offset `pos` within `src` -- used only to
/// give a violation a human-findable location, not for parsing itself.
fn line_at(src: &str, pos: usize) -> usize {
    src[..pos.min(src.len())].matches('\n').count() + 1
}

/// 1-based line number of CHAR offset `pos` within `src` -- `Reader`
/// indexes by `char`, not by byte (it collects `src.chars()` into a
/// `Vec<char>` and its `pos()` is an index into that vec), so a parse
/// failure's position needs its own char-counting variant of `line_at`
/// rather than being fed straight into the byte-offset one above.
fn line_at_char(src: &str, char_pos: usize) -> usize {
    src.chars().take(char_pos).filter(|&c| c == '\n').count() + 1
}

fn scan_file(
    path: &std::path::Path,
    violations: &mut Vec<Violation>,
    parse_failures: &mut Vec<ParseFailure>,
) {
    let src = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {:?}: {}", path, e));
    let mut interp = elisp::new_interp();
    let mut reader = elisp::reader::Reader::new(&src);
    loop {
        let before = reader.pos();
        let form = match reader.read(&mut interp) {
            Ok(Some(form)) => form,
            Ok(None) => break,
            // A genuine parse error would already fail this editor's own
            // startup (every one of these files is `eval_source`'d at
            // init) -- some OTHER test already catches THAT it broke,
            // but not WHERE (see this file's header, "Second failure
            // mode"). Record it here with a location and stop scanning
            // this file -- anything past a parse failure isn't
            // trustworthy `Value` data anyway.
            Err(e) => {
                let msg = match &e {
                    elisp::reader::ReadError::Incomplete => "incomplete form at EOF".to_string(),
                    elisp::reader::ReadError::Syntax(s) => s.clone(),
                };
                parse_failures.push(ParseFailure {
                    file: path.to_path_buf(),
                    line: line_at_char(&src, reader.pos()),
                    message: msg,
                });
                break;
            }
        };
        check_form(&form, &interp, path, before, &src, violations);
    }
}

fn check_form(
    form: &Value,
    interp: &Interp,
    path: &std::path::Path,
    form_start: usize,
    src: &str,
    violations: &mut Vec<Violation>,
) {
    let Some(elems) = form.list_to_vec() else {
        return;
    };
    if elems.len() < 2 {
        return;
    }
    let Value::Sym(head_id) = elems[0] else {
        return;
    };
    let head_name = interp.sym_name(head_id);
    if head_name != "defun" && head_name != "defmacro" {
        return;
    }
    // elems: [defun/defmacro, NAME, ARGLIST, BODY...] -- body starts at
    // index 3; index 3 itself (BODY's own index 0) is the legitimate
    // docstring slot.
    if elems.len() < 4 {
        return;
    }
    let function_name = match &elems[1] {
        Value::Sym(id) => interp.sym_name(*id).to_string(),
        other => elisp::printer::prin1_to_string(interp, other),
    };
    for elem in &elems[4..] {
        if matches!(elem, Value::Str(_)) {
            violations.push(Violation {
                file: path.to_path_buf(),
                function: function_name.clone(),
                line: line_at(src, form_start),
            });
        }
    }
}

#[test]
fn no_bare_string_in_defun_body_past_the_docstring_slot() {
    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut scanned_files = 0usize;
    for dir in el_source_dirs() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("failed to read dir {:?}: {}", dir, e));
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("el") {
                continue;
            }
            scan_file(&path, &mut violations, &mut parse_failures);
            scanned_files += 1;
        }
    }
    assert!(
        scanned_files > 10,
        "expected to scan a substantial number of .el files, only found {} -- \
         did `el_source_dirs' stop resolving correctly?",
        scanned_files
    );
    assert!(
        parse_failures.is_empty(),
        "found {} .el file(s) the project's own reader failed to parse -- \
         this is the UNBALANCED-quote failure mode (see this file's own \
         header, \"Second failure mode added\"): loud at startup, but \
         normally silent about WHERE. The line below is where the \
         READER GAVE UP, not necessarily where the mistake was typed -- \
         an unbalanced quote can make the reader swallow an arbitrary \
         amount of source before it notices anything is wrong (see this \
         file's own header, \"Precision limitation\", H4). Start \
         searching BACKWARD from the reported line for the actual \
         unescaped quote:\n{}",
        parse_failures.len(),
        parse_failures
            .iter()
            .map(|f| format!("  {}:{}: {}", f.file.display(), f.line, f.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        violations.is_empty(),
        "found {} defun/defmacro body(s) with a bare string literal past \
         the docstring slot -- this is the exact fingerprint of a \
         docstring that closed early on an unescaped `\"' (see this \
         file's own header for the M83 incident that motivated this \
         check):\n{}",
        violations.len(),
        violations
            .iter()
            .map(|v| format!("  {}:{} in `{}'", v.file.display(), v.line, v.function))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ============================================================
// M85 fix round (tail re-review, H3): the `ParseFailure` detection path
// itself (the `Err(e) => { push; break }` arm in `scan_file`, and
// `line_at_char`) had never been exercised by any test here -- only
// the "clean tree, zero false positives" case was checked. Removing
// the `parse_failures.push(...)` call, or breaking `line_at_char`'s
// own char-offset arithmetic, would have gone completely undetected.
// This test synthesizes a deliberately unparseable `.el` file in a
// scratch directory (never touches `crates/core/lisp/`) and calls
// `scan_file` on it directly.
// ============================================================

/// A scratch directory that deletes itself on `Drop` -- same shape
/// every other test file in this crate already uses (`search_tests.rs`
/// etc.).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!(
            "se_lisp_hygiene_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn scan_file_detects_and_reports_a_genuinely_unparseable_file() {
    let scratch = Scratch::new("parse_failure");
    let path = scratch.join("broken.el");
    // A stray, UNMATCHED trailing `)` on line 4 -- `(defun ...)` on
    // lines 1-3 is a complete, valid form the reader reads
    // successfully; the lone `)` on its own line is what the reader
    // chokes on when it tries to read a SECOND top-level form,
    // producing a `ReadError::Syntax("unexpected )")`-shaped failure
    // (this editor's own reader; exact wording not asserted on, since
    // that's an implementation detail of `elisp::reader::Reader`, not
    // what this test exists to pin).
    std::fs::write(
        &path,
        "(defun se-hygiene-test-fn ()\n  \"A harmless docstring.\"\n  (+ 1 2))\n)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures);

    assert_eq!(
        parse_failures.len(),
        1,
        "a genuinely unparseable file must produce exactly one \
         ParseFailure, not zero (the exact regression this test \
         exists to catch: an earlier version of `scan_file` swallowed \
         this case entirely with `Err(_) => break`) and not more than \
         one (scanning stops at the first parse failure, per \
         `scan_file`'s own comment)"
    );
    let failure = &parse_failures[0];
    assert_eq!(
        failure.file, path,
        "the recorded file path must be the SYNTHETIC file, not some \
         other .el file"
    );
    // The stray `)` is on line 4 of a 4-line file; `Reader::pos()`
    // stops somewhere at or after the point it gave up, which is
    // AT LEAST as far as the successfully-parsed first form (3 lines)
    // -- a "reasonable range" check (line 3 through line 4 inclusive),
    // not an exact pin, since the reader's own stopping position
    // relative to the offending character is an implementation detail
    // (see H4's note elsewhere in this fix round on why an exact line
    // number can't be promised in general).
    assert!(
        (3..=4).contains(&failure.line),
        "expected the reported line to fall within the synthetic \
         file's own 4 lines, at or near the stray `)` on line 4: got {}",
        failure.line
    );
    assert!(
        !failure.message.is_empty(),
        "the reader's own error message must not be silently dropped"
    );
    // No defun-body violations should be reported for a file that
    // never got past its own parse failure -- `scan_file' stops
    // scanning (`break`) the moment a parse failure is recorded, so
    // the ONE well-formed `defun` before the stray `)` is the only
    // form ever handed to `check_form`, and it has no violation of its
    // own (a single, legitimate docstring, nothing past it).
    assert!(
        violations.is_empty(),
        "no defun-body violations expected from this synthetic file: {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
}
