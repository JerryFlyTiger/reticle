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
//!
//! ## Third failure mode added (M96): pure bare-symbol prose
//!
//! The bare-STRING check above (first failure mode) has a shape it
//! cannot see, and this file's own header called the shot in writing
//! before it happened: "a bare SYMBOL is deliberately NOT flagged...
//! A FUTURE premature-docstring-closure whose leftover prose does NOT
//! happen to contain a balanced quote pair... would pass this check
//! completely." M95 (`lsp-definition-at-point`, via a reconstruction of
//! the `"server doesn't advertise this"` shape in `lsp.el`) is exactly
//! that: when a docstring's premature close is followed by prose that
//! contains no further `"` at all before the defun's own closing paren,
//! EVERY leftover word reads as a bare SYMBOL, never a STRING -- no
//! `ParseFailure` (the quote count stays even: one open, one early
//! close, nothing else), and `check_form`'s bare-STRING scan finds
//! nothing past the docstring slot. The function only fails at CALL
//! time, with an error naming a symbol that appears nowhere in the
//! function's actual source.
//!
//! The rule this second check enforces: a `defun`/`defmacro` body
//! containing a RUN of `PROSE_RUN_THRESHOLD` (2) or more CONSECUTIVE
//! ATOM-LIKE forms is flagged. A single atom-like form as a body form
//! is completely ordinary lisp (`(defun f () x)`, returning a local
//! variable) and must never be flagged on its own -- what leftover
//! English prose produces, and ordinary code does not, is a RUN of
//! several atom-like forms in a row as SIBLING body forms.
//!
//! **Fix round (DD1): "atom-like", not "bare symbol"**. The first cut
//! of this rule counted only `Value::Sym`, and an independent cold
//! read built a harness against the real reader and found that misses
//! its own target for realistic prose: `nil` reads as `Value::Nil`,
//! not `Value::Sym` (`crates/elisp/src/reader.rs`'s `parse_atom`); a
//! bare number reads as `Value::Int`/`Float`/`Big`; and `'` is a reader
//! break character (`reader.rs`'s `read_atom`), so an ordinary English
//! contraction like `doesn't` reads as the symbol `doesn` followed by
//! the two-element list `(quote t)` -- a `Value::Cons`. Any of these
//! appearing mid-leak splits a run into pieces below the threshold,
//! and this is not hypothetical: the docstring that actually broke
//! during M95 read "server doesn't advertise this" -- a contraction.
//! `is_atom_like` (below) widens the run to also cover `Value::Nil`,
//! numbers, and the reader's own quote-form shapes (`quote`,
//! `backquote`, `unquote`, `unquote-splicing`, `function` --
//! specifically a two-element list headed by one of those five
//! symbols, so an ordinary short function call like `(foo bar)` is
//! never mistaken for one).
//!
//! **Second fix round (EE1): two more splitting shapes, both real
//! house conventions in this project's own docstrings**. A vector
//! literal (`Value::Vector`, e.g. `[b]`) and an IMPROPER (dotted) cons
//! (a `Value::Cons` whose tail is not `Nil`, e.g. `(b . c)` --
//! distinguished from a PROPER list / ordinary function call via
//! `Value::list_to_vec()` returning `None`) both split a run the same
//! way `nil` and a contraction did in the first fix round. Neither is
//! hypothetical here: bracket-range notation appears in this project's
//! own docstrings (`evil.el`'s "Snap \[BEG, END) to whole lines",
//! `compile.el`'s "FILE:LINE\[:COL\[-END\]\]"), and so does
//! dotted-pair notation (`dabbrev.el`'s "(STRING . POS)", `dired.el`'s
//! "(NAME . DIRP)", `indent.el`'s "(LANG-SYMBOL . ...)") -- if either
//! kind of docstring closes early near its own tail, the leaked
//! fragment plausibly contains one of these two shapes and the
//! detector goes blind again, same class it was built to close.
//! `is_atom_like` (below) now also covers both. Deliberately NOT every
//! `Cons`: a PROPER list (`list_to_vec()` returns `Some`, and it is
//! not one of the quote-family two-element shapes) is an ordinary
//! function call like `(+ 1 2)` and must stay a run-breaker, or every
//! real multi-statement defun body would trip this rule.
//!
//! **Why 2, empirically, not by taste (re-derived twice)**: every
//! `.el` file actually compiled into this editor was scanned (same two
//! directories as the bare-STRING check above, walked recursively --
//! see EE3 below) for the longest run of consecutive ATOM-LIKE body
//! forms inside any real `defun`/`defmacro`, under the CURRENT widened
//! definition (bare symbols, `nil`, numbers, quote-family forms,
//! vectors, and dotted conses). The longest run found anywhere in the
//! corpus is still exactly 1 (the same single trailing `nil` return in
//! `eshell-process-pending-all', `eshell.el', found under all three
//! definitions of "atom-like" tried across both fix rounds) -- there
//! is not one genuine occurrence of two or more atom-like forms back
//! to back anywhere in the shipped lisp, under any definition tried so
//! far. A threshold of 1 would flag that ordinary, legitimate pattern
//! on every function that returns a local variable or constant, which
//! is useless noise; a threshold of 2 is therefore still both the
//! SMALLEST value that produces zero violations on the real corpus,
//! and has a full run of headroom below it (the worst real case is 1,
//! the threshold is 2 -- doubling before it trips).
//! `prose_run_yields_zero_violations_on_real_corpus` (below) pins this
//! against the corpus directly so a future addition that crosses the
//! line is caught immediately rather than by hand.
//!
//! Two shapes the reviewer who found this gap also reconstructed turn
//! out to already be caught by the FIRST check above (bare STRING),
//! confirmed here with their own pinned fixtures so a future refactor
//! cannot quietly lose either path: unescaping ALL the quotes around an
//! embedded phrase (`quote_hygiene_all_quotes_unescaped_is_caught`,
//! producing an even quote count and a bare-STRING leftover), and
//! unescaping only ONE of the pair (`quote_hygiene_one_quote_unescaped_
//! is_caught`, producing an odd quote count and a `ParseFailure`).

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

/// M96 fix round (DD5): recursively collect every `.el` file under
/// `dir`. Both `el_source_dirs()` directories are flat today, so a
/// non-recursive `read_dir` has never missed anything -- but nothing
/// enforces that they STAY flat, and a future reorganisation into
/// subdirectories would silently stop scanning the files inside it,
/// with no guarantee the `scanned_files > 10` floor below would notice
/// (a partial miss can still clear a floor set for "a substantial
/// number").
///
/// M96 second fix round (EE3): `path.is_dir()` follows symlinks, so a
/// symlink loop under either source directory would recurse without
/// bound instead of merely missing files -- a stack overflow, not a
/// silent gap. Both directories are flat with no symlinks today (this
/// is dormant), but bounded here with a simple depth cap rather than
/// left available: `MAX_DEPTH` is generous enough that no real,
/// non-cyclic reorganisation of these two directories would ever come
/// close to it, while a genuine symlink loop still terminates instead
/// of overflowing the stack.
const FIND_EL_FILES_MAX_DEPTH: usize = 32;

fn find_el_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    find_el_files_bounded(dir, out, 0);
}

fn find_el_files_bounded(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    if depth > FIND_EL_FILES_MAX_DEPTH {
        panic!(
            "find_el_files recursed past {} levels under {:?} -- likely a              symlink loop; not descending further",
            FIND_EL_FILES_MAX_DEPTH, dir
        );
    }
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("failed to read dir {:?}: {}", dir, e));
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            find_el_files_bounded(&path, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("el") {
            out.push(path);
        }
    }
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

/// A run of `PROSE_RUN_THRESHOLD` or more CONSECUTIVE bare-symbol body
/// forms in a `defun`/`defmacro` -- see this file's header, "Third
/// failure mode added (M96)".
struct ProseRun {
    file: std::path::PathBuf,
    function: String,
    line: usize,
    len: usize,
    words: String,
}

/// See this file's header, "Third failure mode added (M96)", for the
/// empirical derivation: the longest run of consecutive bare-symbol
/// body forms found anywhere in the real `.el` corpus is 1, so 2 is
/// both the smallest threshold that produces zero corpus violations
/// and carries a full run of headroom below it.
const PROSE_RUN_THRESHOLD: usize = 2;

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
    prose_runs: &mut Vec<ProseRun>,
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
        check_form(&form, &interp, path, before, &src, violations, prose_runs);
    }
}

/// M96 fix round (DD1): "atom-like" body form -- widened past bare
/// symbols to also cover the other token shapes ordinary English prose
/// splits across once a docstring closes early. `nil` reads as
/// `Value::Nil`, not `Value::Sym` (see `crates/elisp/src/reader.rs`'s
/// own `parse_atom`); a bare number reads as `Value::Int`/`Float`/
/// `Big`; and `'` is a reader break character (`reader.rs`'s
/// `read_atom`), so a contraction like `doesn't` reads as the symbol
/// `doesn` followed by the two-element list `(quote t)` -- a
/// `Value::Cons`, not a `Value::Sym`. All four are still exactly as
/// "atom-like" as a bare symbol for this rule's purposes: none of them
/// is a meaningful, intentional body form on its own, and prose leaks
/// this shape routinely (a docstring that closes early near ITS OWN
/// end typically leaks only a handful of trailing words, and words
/// like "nil", numbers, and contractions are ordinary English, not
/// something a leak has to avoid to slip past a symbols-only rule).
/// Detecting the quote-form case specifically (rather than any
/// two-element list) keeps this from also matching genuine short
/// function-call forms like `(foo bar)`, which are not atom-like at
/// all.
fn is_atom_like(v: &Value, interp: &Interp) -> bool {
    match v {
        Value::Sym(_) | Value::Nil | Value::Int(_) | Value::Float(_) | Value::Big(_) => true,
        // M96 fix round (EE1): a vector literal splits a run too --
        // this project's own docstrings routinely use bracket-range
        // notation (`evil.el`'s "[BEG, END)", `compile.el`'s
        // "FILE:LINE[:COL[-END]]"), and a leaked fragment of one reads
        // as a `Value::Vector`, not a `Value::Sym`. A vector literal as
        // an intentional body form (`(defun f () [1 2 3])`) is rare
        // enough, and indistinguishable from the leaked-fragment case
        // by shape alone, that treating it as atom-like carries the
        // same "prose, not code" bias as every other branch here.
        Value::Vector(_) => true,
        Value::Cons(_) => {
            let Some(elems) = v.list_to_vec() else {
                // M96 fix round (EE1): `list_to_vec()` returns `None`
                // for an IMPROPER (dotted) cons -- one whose tail is
                // not `Nil`, e.g. `(b . c)`. This project's own
                // docstrings routinely use dotted-pair notation
                // (`dabbrev.el`'s "(STRING . POS)", `dired.el`'s
                // "(NAME . DIRP)", `indent.el`'s "(LANG-SYMBOL . ...)"),
                // and a leaked fragment of one reads as exactly this
                // shape. Deliberately NOT the same as a PROPER list
                // (`elems` `Some`, below): `(+ 1 2)` is an ordinary
                // function call and must stay a run-breaker, or every
                // real defun body with more than one statement would
                // trip this rule.
                return true;
            };
            if elems.len() != 2 {
                return false;
            }
            let Value::Sym(head_id) = elems[0] else {
                return false;
            };
            head_id == interp.syms.quote
                || head_id == interp.syms.backquote
                || head_id == interp.syms.unquote
                || head_id == interp.syms.unquote_splicing
                || head_id == interp.syms.function
        }
        _ => false,
    }
}

fn check_form(
    form: &Value,
    interp: &Interp,
    path: &std::path::Path,
    form_start: usize,
    src: &str,
    violations: &mut Vec<Violation>,
    prose_runs: &mut Vec<ProseRun>,
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
    // Third failure mode (M96): a RUN of consecutive ATOM-LIKE body
    // forms (M96 fix round, DD1: widened past bare symbols alone -- see
    // `is_atom_like` above) -- see this file's header for why a single
    // atom-like form must never be flagged on its own, and for the
    // threshold's empirical derivation.
    let body = &elems[4..];
    let mut run_start: Option<usize> = None;
    let mut i = 0;
    while i <= body.len() {
        let is_sym = i < body.len() && is_atom_like(&body[i], interp);
        if is_sym {
            if run_start.is_none() {
                run_start = Some(i);
            }
        } else if let Some(start) = run_start.take() {
            let len = i - start;
            if len >= PROSE_RUN_THRESHOLD {
                let words = body[start..i]
                    .iter()
                    .map(|v| elisp::printer::prin1_to_string(interp, v))
                    .collect::<Vec<_>>()
                    .join(" ");
                prose_runs.push(ProseRun {
                    file: path.to_path_buf(),
                    function: function_name.clone(),
                    line: line_at(src, form_start),
                    len,
                    words,
                });
            }
        }
        i += 1;
    }
}

#[test]
fn no_bare_string_in_defun_body_past_the_docstring_slot() {
    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    let mut scanned_files = 0usize;
    for dir in el_source_dirs() {
        let mut files = Vec::new();
        find_el_files(&dir, &mut files);
        for path in files {
            scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);
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
    assert!(
        prose_runs.is_empty(),
        "found {} defun/defmacro body(s) with a run of {} or more \
         CONSECUTIVE bare-symbol forms -- this is the M96 fingerprint: \
         a docstring that closed early on an unescaped `\"' whose \
         leftover prose happens to contain no further `\"' at all, so \
         every leftover word reads as a bare SYMBOL instead of a \
         STRING and the bare-STRING check above never sees it (see \
         this file's own header, \"Third failure mode added (M96)\"). \
         This assertion is the false-positive guard for that new rule \
         -- if it ever fires against genuinely legitimate code rather \
         than a real defect, that is grounds to revisit the threshold, \
         not to silence this test:\n{}",
        prose_runs.len(),
        PROSE_RUN_THRESHOLD,
        prose_runs
            .iter()
            .map(|p| format!(
                "  {}:{} in `{}' ({} consecutive bare symbols: {})",
                p.file.display(),
                p.line,
                p.function,
                p.len,
                p.words
            ))
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
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

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

// ============================================================
// M96: the bare-STRING check's own blind spot (see this file's header,
// "Third failure mode added (M96)") -- a docstring that closes early
// on an unescaped `"' whose leftover prose contains no further `"' at
// all reads as PURE bare symbols, invisible to the bare-STRING check
// and silent at the reader level. Three shapes are pinned here
// together so a future refactor cannot quietly lose any one of them:
// the new blind-spot shape (now caught), and the two shapes the
// reviewer who found the gap also reconstructed, both of which turn
// out to already be caught by one of the two EXISTING checks.
// ============================================================

/// Reproduction, then confirmation: a docstring closes early on an
/// unescaped `"' (right after "the primary "), and everything after it
/// up to the defun's own closing paren is prose that contains no
/// further `"' at all -- exactly the M95 shape (a reconstruction of the
/// `"server doesn't advertise this"' phrase that actually broke
/// `lsp-definition-at-point' in `lsp.el'). Confirmed directly against
/// the real reader (not asserted by inspection) before this rule
/// existed: this produces zero `ParseFailure`s (the stray-quote count
/// stays even -- one legitimate open, one early close, nothing else)
/// and zero bare-STRING `Violation`s (every leftover word is a bare
/// SYMBOL, never a STRING) -- the exact silent gap this milestone
/// exists to close. The new rule below is what now catches it.
#[test]
fn bare_symbol_prose_run_is_flagged() {
    let scratch = Scratch::new("prose_run");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-m95-shape ()\n  \
         \"Falls back to the primary \"when the server does not advertise \
         this capability at all exactly as it always did.\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly (the whole point of the gap is \
         that it's SILENT, not a loud parse failure): got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        violations.is_empty(),
        "this shape must produce NO bare-STRING violation (that's the \
         other check, and this fixture deliberately contains no \
         further `\"' past the early close, so it cannot trip): {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun for the one broken defun: {:?}",
        prose_runs.iter().map(|p| &p.function).collect::<Vec<_>>()
    );
    assert!(
        prose_runs[0].len >= PROSE_RUN_THRESHOLD,
        "the flagged run must be at least the threshold length: got {}",
        prose_runs[0].len
    );
}

/// One of the two shapes the reviewer also reconstructed while finding
/// the gap above: BOTH quotes around an embedded phrase left
/// unescaped. The stray-quote count stays even (open, early-close,
/// reopen, close), so this parses cleanly -- but unlike the shape
/// above, the reopened string's content happens to run all the way to
/// the real closing quote without hitting another stray `"' first, so
/// it becomes a bare STRING body form: already caught by the FIRST
/// check (`no_bare_string_in_defun_body_past_the_docstring_slot`'s own
/// `violations` list), independent of the new rule this milestone
/// adds.
#[test]
fn quote_hygiene_all_quotes_unescaped_is_caught() {
    let scratch = Scratch::new("all_unescaped");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-all-unescaped ()\n  \
         \"Falls back to the primary \"when the server does not advertise\" \
         this capability at all.\"\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape's stray-quote count is even and must parse cleanly: \
         got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        !violations.is_empty(),
        "this shape must be caught by the EXISTING bare-STRING check \
         (a same-named regression here would mean that check silently \
         lost coverage of a shape it used to catch)"
    );
}

/// The other reconstructed shape: only ONE of the pair left unescaped.
/// The stray-quote count is now ODD, so the reader never finds a
/// matching close and the file fails to parse at all -- already caught
/// by the SECOND check (the `ParseFailure` path added in the M85 tail
/// fix), independent of the new rule this milestone adds.
#[test]
fn quote_hygiene_one_quote_unescaped_is_caught() {
    let scratch = Scratch::new("one_unescaped");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-one-unescaped ()\n  \
         \"Falls back to the primary \"when the server does not advertise\\\" \
         this capability at all.\"\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        !parse_failures.is_empty(),
        "this shape's stray-quote count is odd and must be caught by \
         the EXISTING ParseFailure check (a regression here would mean \
         that check silently lost coverage of a shape it used to catch)"
    );
}

// ============================================================
// M96 fix round (DD1): the first cut of the prose-run check counted
// only `Value::Sym`; an independent cold read built a harness against
// the real reader and found two ordinary-English shapes that split a
// run into pieces below the threshold and slip past silently. Both are
// reproduced here as their own named fixtures, confirmed failing
// against the PRE-fix-round code (bare-symbol-only run counting) before
// `is_atom_like` was written, and now confirmed caught.
// ============================================================

/// DD1, shape 1: `nil` splits a run. Reproduced against the PRE-fix
/// code (bare-`Value::Sym`-only run counting): the docstring closes
/// early after "Fetch the value, or ", and the leftover prose "returns
/// nil eventually." reads as `Sym(returns) Nil Sym(eventually.)` --
/// under the OLD rule, `nil` is `Value::Nil`, not `Value::Sym`, so the
/// run breaks into `[1, 1]`, below the threshold of 2, and nothing was
/// flagged. `is_atom_like` now counts `Nil` too, restoring the run to
/// length 3 and flagging it.
#[test]
fn atomlike_prose_run_nil_split_is_flagged() {
    let scratch = Scratch::new("nil_split");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-nil-split ()\n  \
         \"Fetch the value, or \"returns nil eventually.\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly: got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        violations.is_empty(),
        "this shape must produce no bare-STRING violation: {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun (returns/nil/eventually. is a          run of 3 atom-like forms, >= threshold): {:?}",
        prose_runs.iter().map(|p| (&p.function, p.len)).collect::<Vec<_>>()
    );
    assert_eq!(prose_runs[0].len, 3, "expected the full 3-token run");
}

/// DD1, shape 2: a contraction splits a run -- and this is the SAME
/// wording family as the real M95 incident ("server doesn't advertise
/// this" in `lsp.el`). Reproduced against the PRE-fix code: the
/// docstring closes early after "Falls back to the default ", and the
/// leftover prose "doesn't apply." reads as `Sym(doesn) Cons(quote t)
/// Sym(apply.)` -- `'` is a reader break character, so under the OLD
/// rule the middle token is `(quote t)`, a `Value::Cons`, not a
/// `Value::Sym`, splitting the run into `[1, 1]`, below threshold, and
/// nothing was flagged. `is_atom_like` now recognizes the reader's own
/// quote-form shape too, restoring the run to length 3.
#[test]
fn atomlike_prose_run_contraction_split_is_flagged() {
    let scratch = Scratch::new("contraction_split");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-contraction-split ()\n  \
         \"Falls back to the default \"doesn't apply.\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly: got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        violations.is_empty(),
        "this shape must produce no bare-STRING violation: {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun (doesn/'t/apply. is a run of 3          atom-like forms, >= threshold): {:?}",
        prose_runs.iter().map(|p| (&p.function, p.len)).collect::<Vec<_>>()
    );
    assert_eq!(prose_runs[0].len, 3, "expected the full 3-token run");
}

// ============================================================
// M96 fix round (DD2): the threshold BOUNDARY itself was never pinned
// -- the original fixtures covered a run of 1 (not flagged) and a run
// of 15 (flagged), so changing `>=` to `>` in `check_form` would fail
// nothing. These two fixtures pin exactly `PROSE_RUN_THRESHOLD` (2,
// flagged) and exactly one below it (1, not flagged).
// ============================================================

/// A run of EXACTLY `PROSE_RUN_THRESHOLD` (2) atom-like body forms
/// must be flagged.
#[test]
fn prose_run_of_exactly_threshold_is_flagged() {
    let scratch = Scratch::new("exact_threshold");
    let path = scratch.join("legit.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-exact-threshold (x y)\n  \"A harmless docstring.\"\n  x y)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "legitimate file must parse cleanly"
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "a run of exactly PROSE_RUN_THRESHOLD (2) atom-like body forms          must be flagged: {:?}",
        prose_runs.iter().map(|p| (&p.function, p.len)).collect::<Vec<_>>()
    );
    assert_eq!(prose_runs[0].len, PROSE_RUN_THRESHOLD);
}

// ============================================================
// M96 fix round (DD3): the run RESET was never exercised -- no
// fixture contained two separate runs, so dropping the `.take()` that
// clears `run_start` after a flush (silently merging two runs
// separated by a real form into a report of only one, or under-
// counting) would go undetected.
// ============================================================

/// A body shaped `sym sym (call) sym sym` -- two separate runs of 2,
/// split by a genuine intervening form -- must be reported as TWO
/// `ProseRun`s, not one merged run and not zero.
#[test]
fn prose_run_resets_between_separate_runs() {
    let scratch = Scratch::new("two_runs");
    let path = scratch.join("legit.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-two-runs (a b)\n  \"A harmless docstring.\"\n  a b (+ 1 2) a b)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "legitimate file must parse cleanly"
    );
    assert_eq!(
        prose_runs.len(),
        2,
        "expected TWO separate ProseRuns (a regression in the          `run_start.take()` reset would merge these into one, or drop          one): {:?}",
        prose_runs.iter().map(|p| (&p.function, p.len)).collect::<Vec<_>>()
    );
    assert_eq!(prose_runs[0].len, 2);
    assert_eq!(prose_runs[1].len, 2);
}

// ============================================================
// M96 fix round (DD4): every synthetic fixture in this file used
// `defun`; the gate covers `defun` and `defmacro` identically
// (`check_form`'s own `head_name != "defun" && head_name != "defmacro"`
// guard), but nothing exercised the `defmacro` half of that guard, so
// removing it from the gate entirely would fail nothing here.
// ============================================================

/// `defmacro` must be covered by the prose-run check, not just `defun`.
#[test]
fn defmacro_prose_run_is_flagged() {
    let scratch = Scratch::new("defmacro_prose_run");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defmacro se-hygiene-defmacro-prose-run ()\n  \
         \"Falls back to the primary \"when the server does not advertise \
         this capability.\n  \
         (list 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly: got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        !prose_runs.is_empty(),
        "a `defmacro` with the exact same leaked-prose shape as a          `defun` must be flagged too"
    );
}

/// `defmacro` must be covered by the pre-existing bare-STRING check
/// too, not just `defun`.
#[test]
fn defmacro_bare_string_is_caught() {
    let scratch = Scratch::new("defmacro_bare_string");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defmacro se-hygiene-defmacro-bare-string ()\n  \
         \"Falls back to the primary \"when the server does not advertise\" \
         this capability.\"\n  \
         (list 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape's stray-quote count is even and must parse cleanly:          got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        !violations.is_empty(),
        "a `defmacro` with the exact same unescaped-quote shape as a          `defun` must be caught by the bare-STRING check too"
    );
}

/// A legitimate single atom-like body form (`(defun f () x)`, returning
/// a local variable) is completely ordinary lisp and must never be
/// flagged on its own -- only a RUN of `PROSE_RUN_THRESHOLD` or more in
/// a row is prose's fingerprint. This is also the DD2 "just below the
/// threshold" boundary pin: a run of 1 (`PROSE_RUN_THRESHOLD` - 1) must
/// NOT be flagged. See this file's header for the empirical derivation
/// of the threshold.
#[test]
fn single_bare_symbol_body_form_is_not_flagged() {
    let scratch = Scratch::new("single_symbol");
    let path = scratch.join("legit.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-legit-fn (x)\n  \"A harmless docstring.\"\n  x)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "legitimate file must parse cleanly"
    );
    assert!(
        violations.is_empty(),
        "a single bare symbol body form is not a bare-STRING violation"
    );
    assert!(
        prose_runs.is_empty(),
        "a single bare symbol body form must NOT be flagged as a prose \
         run -- returning a local variable is completely ordinary lisp: \
         got {:?}",
        prose_runs
            .iter()
            .map(|p| (&p.function, p.len))
            .collect::<Vec<_>>()
    );
}

// ============================================================
// M96 second fix round (EE1): two more atom-like shapes, both real
// house conventions in this project's own docstrings -- see this
// file's header, "Second fix round (EE1)". A vector literal and an
// IMPROPER (dotted) cons both split a run the same way `nil` and a
// contraction did in the first fix round.
// ============================================================

/// EE1, shape 1: a vector literal (bracket-range notation, e.g.
/// `evil.el`'s "[BEG, END)") splits a run. Reproduced against the
/// PRE-EE1 code (`is_atom_like` without the `Value::Vector` branch):
/// `(defun f () "d" a [b] c)` parses to body `[Sym(a), Vector([b]),
/// Sym(c)]` -- under the OLD rule a `Value::Vector` was not atom-like,
/// splitting the run into `[1, 1]`, below threshold, nothing flagged.
#[test]
fn atomlike_prose_run_vector_split_is_flagged() {
    let scratch = Scratch::new("vector_split");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-vector-split ()\n  \
         \"Snap the region \"[BEG END] here.\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly: got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        violations.is_empty(),
        "this shape must produce no bare-STRING violation: {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun (a vector literal mid-leak must \
         not break the run): {:?}",
        prose_runs
            .iter()
            .map(|p| (&p.function, p.len))
            .collect::<Vec<_>>()
    );
    assert!(
        prose_runs[0].len >= PROSE_RUN_THRESHOLD,
        "the flagged run must be at least the threshold length: got {}",
        prose_runs[0].len
    );
}

/// EE1, shape 2: an IMPROPER (dotted) cons (dotted-pair notation, e.g.
/// `dabbrev.el`'s "(STRING . POS)") splits a run. Reproduced against
/// the PRE-EE1 code: `(defun f () "d" a (b . c) d)` parses to body
/// `[Sym(a), Cons(b . c), Sym(d)]` -- under the OLD rule EVERY
/// `Value::Cons` that wasn't a two-element quote-form was rejected
/// outright (`elems.len() != 2` on a `None` `list_to_vec()` never even
/// evaluated, the function returned `false` at the `let Some(elems) =
/// ... else { return false }` guard), splitting the run into `[1, 1]`.
#[test]
fn atomlike_prose_run_dotted_pair_split_is_flagged() {
    let scratch = Scratch::new("dotted_split");
    let path = scratch.join("broken.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-dotted-split ()\n  \
         \"Returns the pair \"(STRING . POS) for the match here.\n  \
         (+ 1 2))\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "this shape must parse cleanly: got {:?}",
        parse_failures
            .iter()
            .map(|f| &f.message)
            .collect::<Vec<_>>()
    );
    assert!(
        violations.is_empty(),
        "this shape must produce no bare-STRING violation: {:?}",
        violations.iter().map(|v| &v.function).collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun (an improper/dotted cons \
         mid-leak must not break the run): {:?}",
        prose_runs
            .iter()
            .map(|p| (&p.function, p.len))
            .collect::<Vec<_>>()
    );
    assert!(
        prose_runs[0].len >= PROSE_RUN_THRESHOLD,
        "the flagged run must be at least the threshold length: got {}",
        prose_runs[0].len
    );
}

/// A genuine PROPER list -- an ordinary function call like `(+ 1 2)`
/// -- must stay a run-breaker even under the widened EE1 definition;
/// only an IMPROPER (dotted) cons is atom-like. This is also already
/// exercised by `prose_run_resets_between_separate_runs` (its `(+ 1
/// 2)` intervening form splits two runs of 2 apart); pinned again here
/// under its own name so a regression that made ALL conses atom-like
/// (not just dotted ones) is attributable to this specific case rather
/// than inferred from the reset test.
#[test]
fn proper_list_body_form_stays_a_run_breaker() {
    let scratch = Scratch::new("proper_list_breaker");
    let path = scratch.join("legit.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-proper-list-breaker (a b)\n  \
         \"A harmless docstring.\"\n  \
         a (+ 1 2) b)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "legitimate file must parse cleanly"
    );
    assert!(
        prose_runs.is_empty(),
        "a proper-list function call between two lone atom-like forms \
         must break the run on both sides (a regression that made \
         PROPER lists atom-like too would merge `a (+ 1 2) b` into one \
         run of 3 and flag it): {:?}",
        prose_runs
            .iter()
            .map(|p| (&p.function, p.len))
            .collect::<Vec<_>>()
    );
}

// ============================================================
// M96 second fix round (EE2): four of the five quote-family branches
// in `is_atom_like` had no fixture -- deleting `backquote`, `unquote`,
// `unquote_splicing`, or `function` from that match arm would fail
// nothing. One fixture below exercises all four together (backtick,
// comma, comma-at, and `#'`) as four consecutive body forms.
// ============================================================

/// All four of `backquote`, `unquote`, `unquote-splicing`, and
/// `function` (the reader's quote-family symbols besides plain
/// `quote`, already covered by the M95-shape fixtures above) must be
/// recognized as atom-like. Confirmed directly against the real reader
/// before writing this fixture's assertions: `` `x ``, `,y`, `,@z`,
/// and `#'w` parse to four distinct `Value::Cons` shapes, one per
/// symbol.
#[test]
fn atomlike_quote_family_all_four_branches_is_flagged() {
    let scratch = Scratch::new("quote_family");
    let path = scratch.join("legit.el");
    std::fs::write(
        &path,
        "(defun se-hygiene-quote-family ()\n  \"A harmless docstring.\"\n  `x ,y ,@z #'w)\n",
    )
    .unwrap();

    let mut violations = Vec::new();
    let mut parse_failures = Vec::new();
    let mut prose_runs = Vec::new();
    scan_file(&path, &mut violations, &mut parse_failures, &mut prose_runs);

    assert!(
        parse_failures.is_empty(),
        "legitimate file must parse cleanly"
    );
    assert_eq!(
        prose_runs.len(),
        1,
        "expected exactly one ProseRun spanning all four quote-family \
         forms: {:?}",
        prose_runs
            .iter()
            .map(|p| (&p.function, p.len))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        prose_runs[0].len, 4,
        "expected all four quote-family forms in one run (a regression \
         in any one of the `backquote`/`unquote`/`unquote_splicing`/ \
         `function` branches would shorten this run): got {}",
        prose_runs[0].len
    );
}
