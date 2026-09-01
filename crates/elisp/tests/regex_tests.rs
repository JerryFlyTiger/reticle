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

#[test]
fn literals_and_dot() {
    assert_eq!(run(r#"(string-match "world" "hello world")"#), "6");
    assert_eq!(run(r#"(string-match "w.rld" "hello world")"#), "6");
    assert_eq!(run(r#"(string-match "xyz" "hello world")"#), "nil");
    // `.` must not match a newline.
    assert_eq!(run(r#"(string-match "a.b" "a\nb")"#), "nil");
}

#[test]
fn star_plus_opt() {
    assert_eq!(run(r#"(string-match "ab*c" "ac")"#), "0");
    assert_eq!(run(r#"(string-match "ab*c" "abbbc")"#), "0");
    assert_eq!(run(r#"(string-match "ab+c" "ac")"#), "nil");
    assert_eq!(run(r#"(string-match "ab+c" "abbc")"#), "0");
    assert_eq!(run(r#"(string-match "colou?r" "color")"#), "0");
    assert_eq!(run(r#"(string-match "colou?r" "colour")"#), "0");
}

#[test]
fn greedy_vs_lazy() {
    // Greedy * takes the longest match, lazy *? the shortest.
    assert_eq!(
        run(r#"(progn (string-match "<.*>" "<a><b>") (match-string 0))"#),
        "\"<a><b>\""
    );
    assert_eq!(
        run(r#"(progn (string-match "<.*?>" "<a><b>") (match-string 0))"#),
        "\"<a>\""
    );
}

#[test]
fn char_classes() {
    assert_eq!(run(r#"(string-match "[0-9]+" "abc123def")"#), "3");
    assert_eq!(run(r#"(string-match "[^a-z]" "abc!def")"#), "3");
    assert_eq!(run(r#"(string-match "[]x]" "x")"#), "0"); // literal ] first
    assert_eq!(run(r#"(string-match "[[:digit:]]+" "ab42")"#), "2");
    assert_eq!(run(r#"(string-match "[[:space:]]" "ab cd")"#), "2");
}

#[test]
fn anchors() {
    assert_eq!(run(r#"(string-match "^abc" "abcdef")"#), "0");
    assert_eq!(run(r#"(string-match "^def" "abcdef")"#), "nil");
    // ^ matches after a newline (line anchor, not string anchor).
    assert_eq!(run(r#"(string-match "^def" "abc\ndef")"#), "4");
    assert_eq!(run(r#"(string-match "def$" "abcdef")"#), "3");
    assert_eq!(run(r#"(string-match "abc$" "abc\ndef")"#), "0");
    // \` and \' are hard string anchors.
    assert_eq!(run(r#"(string-match "\\`abc" "abc\nabc")"#), "0");
    assert_eq!(run(r#"(string-match "\\'" "ab")"#), "2");
}

#[test]
fn groups_and_match_data() {
    let src = r#"(progn
        (string-match "\\([0-9]+\\)-\\([0-9]+\\)" "range 10-25 here")
        (list (match-beginning 0) (match-end 0)
              (match-string 0) (match-string 1) (match-string 2)))"#;
    assert_eq!(run(src), "(6 11 \"10-25\" \"10\" \"25\")");

    // Unmatched optional group → nil.
    let src2 = r#"(progn
        (string-match "a\\(x\\)?b" "ab")
        (list (match-string 0) (match-string 1)))"#;
    assert_eq!(run(src2), "(\"ab\" nil)");
}

#[test]
fn alternation() {
    assert_eq!(run(r#"(string-match "cat\\|dog" "hotdog")"#), "3");
    assert_eq!(
        run(r#"(progn (string-match "\\(cat\\|dog\\)food" "dogfood") (match-string 1))"#),
        "\"dog\""
    );
    // Backtracking across alternation and the rest of the pattern.
    assert_eq!(run(r#"(string-match "\\(a\\|ab\\)c" "abc")"#), "0");
}

#[test]
fn shy_groups_do_not_capture() {
    let src = r#"(progn
        (string-match "\\(?:ab\\)+\\([0-9]\\)" "ababab7")
        (list (match-string 0) (match-string 1)))"#;
    assert_eq!(run(src), "(\"ababab7\" \"7\")");
}

#[test]
fn counted_repetition() {
    assert_eq!(run(r#"(string-match "a\\{3\\}" "aa")"#), "nil");
    assert_eq!(run(r#"(string-match "a\\{3\\}" "aaaa")"#), "0");
    assert_eq!(run(r#"(string-match "a\\{2,3\\}b" "aaab")"#), "0");
    assert_eq!(run(r#"(string-match "a\\{2,\\}b" "ab")"#), "nil");
    assert_eq!(run(r#"(string-match "a\\{2,\\}b" "aaaaab")"#), "0");
}

#[test]
fn word_classes_and_boundaries() {
    assert_eq!(run(r#"(string-match "\\w+" "  hello")"#), "2");
    assert_eq!(run(r#"(string-match "\\W" "ab cd")"#), "2");
    assert_eq!(run(r#"(string-match "\\s-+" "ab   cd")"#), "2");
    assert_eq!(run(r#"(string-match "\\bcat\\b" "concat cat")"#), "7");
    assert_eq!(run(r#"(string-match "\\<dog" "hotdog dog")"#), "7");
    assert_eq!(run(r#"(string-match "cat\\>" "catalog cat")"#), "8");
}

#[test]
fn escaped_specials_are_literal() {
    assert_eq!(run(r#"(string-match "3\\.14" "pi=3.14")"#), "3");
    assert_eq!(run(r#"(string-match "3\\.14" "pi=3x14")"#), "nil");
    // Bare parens/braces are literal in the elisp dialect.
    assert_eq!(run(r#"(string-match "f(x)" "call f(x) now")"#), "5");
    assert_eq!(run(r#"(string-match "a{2}" "a{2}")"#), "0");
}

#[test]
fn string_match_p_leaves_match_data_alone() {
    let src = r#"(progn
        (string-match "b\\(c\\)" "abc")
        (string-match-p "x\\(y\\)" "xy")
        (list (match-beginning 0) (match-string 1)))"#;
    assert_eq!(run(src), "(1 \"c\")");
}

#[test]
fn start_argument() {
    assert_eq!(run(r#"(string-match "a" "banana" 2)"#), "3");
    assert_eq!(run(r#"(string-match "^" "ab" 1)"#), "nil");
}

#[test]
fn replace_regexp_in_string() {
    assert_eq!(
        run(r#"(replace-regexp-in-string "[0-9]+" "N" "a1b22c333")"#),
        "\"aNbNcN\""
    );
    // \& = whole match, \1 = group 1.
    assert_eq!(
        run(r#"(replace-regexp-in-string "[0-9]+" "<\\&>" "a12b")"#),
        "\"a<12>b\""
    );
    assert_eq!(
        run(r#"(replace-regexp-in-string "\\([a-z]\\)\\([0-9]\\)" "\\2\\1" "a1 b2")"#),
        "\"1a 2b\""
    );
    // No match: unchanged.
    assert_eq!(
        run(r#"(replace-regexp-in-string "x" "y" "abc")"#),
        "\"abc\""
    );
}

#[test]
fn regexp_quote_makes_literals() {
    assert_eq!(
        run(r#"(string-match (regexp-quote "3.14 (approx)") "pi is 3.14 (approx)!")"#),
        "6"
    );
    // The result string is a\.b\*c — prin1 doubles the backslashes.
    assert_eq!(run(r#"(regexp-quote "a.b*c")"#), r#""a\\.b\\*c""#);
}

#[test]
fn split_string() {
    assert_eq!(
        run(r#"(split-string "one two  three")"#),
        "(\"one\" \"two\" \"three\")"
    );
    assert_eq!(
        run(r#"(split-string "a,b,,c" ",")"#),
        "(\"a\" \"b\" \"\" \"c\")"
    );
    assert_eq!(
        run(r#"(split-string "a,b,,c" "," t)"#),
        "(\"a\" \"b\" \"c\")"
    );
}

/// Like `run`, but with a caller-chosen stack size — M81 test 2 below
/// needs this to prove the result is identical regardless of stack size
/// (the load-bearing property of the explicit-stack rewrite: it no
/// longer uses the Rust call stack for backtracking at all, so unlike
/// the pre-M81 engine, shrinking the thread stack must not change
/// whether/how a match gives up).
fn run_with_stack(src: &str, stack_size: usize) -> String {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(stack_size)
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

/// M81 test 1 (pathological-backtracking regression): `\(a+\)+z`
/// against 30 `a`s with no trailing `z` is the textbook nested-
/// quantifier catastrophic-backtracking shape — exponential in the
/// number of ways to partition the run of `a`s between the outer and
/// inner `+`, so total VM dispatch steps blow up long before the
/// haystack is even "large" (see the module doc's `max_steps` budget).
/// Must NOT assert "didn't crash" or "didn't hang": `run`'s helper
/// thread has a 512MB stack (see its doc comment on the crate's other
/// test files) and no wall-clock bound, so a bare "the call returned"
/// check would eventually pass even on a badly broken engine — a false
/// pass. The only assertion that actually exercises the M81 budget is
/// that the call comes back as the `regexp-too-complex` signal
/// specifically.
///
/// M81 R4: this test used to run `[^;]*` against a 2,000,000-char
/// haystack instead — a PURELY LINEAR match (one `Split` push per
/// consumed char, never backtracking) that only "gave up" because it
/// crossed the old flat `MAX_FRAMES = 1_000_000` cap, not because of
/// any actual complexity. That made this regression test itself pin
/// the R4 bug as "expected behavior". See
/// `long_linear_match_is_not_misclassified_as_too_complex` below for
/// the fixed version of that scenario (must now SUCCEED).
#[test]
fn giving_up_on_pathological_pattern_signals_regexp_too_complex_not_crash() {
    let src = r#"(condition-case e
        (string-match "\\(a+\\)+z" (make-string 30 ?a))
      (error (car e)))"#;
    assert_eq!(run(src), "regexp-too-complex");
}

/// M81 R4: the exact scenario reviewer used to demonstrate the flat-
/// `MAX_FRAMES` bug — `[^;]*` (unbounded greedy class, no backtracking
/// needed at all) against a long haystack with no `;` must SUCCEED, no
/// matter how long the haystack is, as long as it's a genuinely linear
/// match. 1,500,000 chars is reviewer's own repro size (pre-fix: this
/// specific length failed with `RegexLimit` purely from crossing the
/// old flat 1,000,000-frame cap; 999,000 chars, just under it,
/// succeeded — a completely arbitrary success/failure boundary with no
/// relationship to the pattern's actual complexity). Asserts more than
/// "no error": also checks the match spans the whole string, so a
/// regression that silently truncated the match wouldn't slip through
/// as a false pass.
#[test]
fn long_linear_match_is_not_misclassified_as_too_complex() {
    let src = r#"(progn
        (string-match "[^;]*" (make-string 1500000 ?a))
        (match-end 0))"#;
    assert_eq!(run(src), "1500000");
}

/// M81 R1 regression: the coordinator's repro (`:s/module[^;]*zzz/...`)
/// showed the echo area printing the message TWICE (and, since the echo
/// area truncates to 72 chars — M70/M77 — getting cut off mid-repeat).
/// Root cause: `Interp::regexp_too_complex` used to pass the error's own
/// `error-message` AS the signal's data (`vec![msg]`), and
/// `describe_flow` formats as "message: data" — so the message got
/// printed once as the base message and once again as "data". Fixed to
/// `vec![]` (empty data), matching `elisp-timeout`'s own
/// `signal(e, vec![])`. Deliberately asserts the FULL uncaught-error
/// string (not just "contains the sentence" — that assertion would pass
/// whether or not the duplication bug was fixed, see M78) AND that the
/// sentence appears exactly once.
#[test]
fn regexp_too_complex_message_is_not_duplicated() {
    // M81 R4: switched from the linear `[^;]*`/2,000,000-char shape
    // (which now correctly SUCCEEDS, see `long_linear_match_is_not_
    // misclassified_as_too_complex`) to the genuinely pathological
    // `\(a+\)+z` shape used by the R4 tests above.
    let src = r#"(string-match "\\(a+\\)+z" (make-string 30 ?a))"#;
    let out = run(src);
    assert_eq!(
        out,
        "ERROR: Regexp match gave up: pattern is too expensive on this text"
    );
    let sentence = "Regexp match gave up: pattern is too expensive on this text";
    assert_eq!(
        out.matches(sentence).count(),
        1,
        "message duplicated: {out}"
    );
}

/// M81 test 2: the same pathological match (same `\(a+\)+z` shape as
/// the test above — see R4's note there on why the ORIGINAL `[^;]*`
/// shape can't be used here anymore, it's linear and must succeed) on
/// 1MB / 8MB / 512MB thread stacks must give the IDENTICAL result — the
/// direct proof that the explicit-stack rewrite no longer depends on
/// the Rust call stack at all.
#[test]
fn giving_up_is_independent_of_thread_stack_size() {
    let src = r#"(condition-case e
        (string-match "\\(a+\\)+z" (make-string 30 ?a))
      (error (car e)))"#;
    let small = run_with_stack(src, 1024 * 1024);
    let medium = run_with_stack(src, 8 * 1024 * 1024);
    let large = run_with_stack(src, 512 * 1024 * 1024);
    assert_eq!(small, "regexp-too-complex");
    assert_eq!(small, medium);
    assert_eq!(medium, large);
}

/// M81 test 3 (freeze regression): `[^;]+x` against a 30KB haystack
/// with no `;` and no trailing `x` — no literal prefix, so `search`'s
/// outer "try every position" loop turns the per-position O(remaining
/// length) backtrack into O(n²) overall (the exact shape architect
/// measured at 4.64s of unresponsiveness pre-M81, nowhere near
/// stack-overflowing — see the module doc's "Behavior contract"
/// section on why a per-`match_at` budget alone cannot catch this).
/// Deliberately does NOT assert a millisecond bound (that would be
/// flaky under load) — only that the cumulative step budget in
/// `search()` catches the blow-up and signals cleanly instead of
/// hanging.
#[test]
fn quadratic_no_prefix_pattern_signals_regexp_too_complex() {
    let src = r#"(condition-case e
        (string-match "[^;]+x" (make-string 30000 ?a))
      (error (car e)))"#;
    assert_eq!(run(src), "regexp-too-complex");
}

/// M81 R3 regression: `\{n\}`'s MANDATORY `min` copies compile to a
/// flat straight-line instruction sequence (`compile_node`'s `Repeat`
/// arm, `for _ in 0..*min { self.compile_node(node); }`) — no `Split`,
/// no `Jump`. The step-budget design (as first shipped) only counted at
/// those two "back edge" instructions, so a huge mandatory run like
/// `.\{45000\}` racks up zero counted steps and zero frames while still
/// costing O(min) real work PER starting position `search`'s outer
/// no-literal-prefix loop tries — reviewer measured 1.56s of
/// unresponsiveness on a 60KB haystack with `scratch.steps` and
/// `scratch.frames` both stuck at 0 the entire time. Fixed by counting
/// on EVERY dispatch step (not just Split/Jump) and only comparing
/// against the budget at back-edges and at `match_at` return — so the
/// accumulated cost across a single `match_at` call, and across
/// `search`'s retries, is what gets checked, without paying a compare
/// on every straight-line instruction.
///
/// M81 R11: `\{45000\}` as a single counted-repetition operator is no
/// longer legal (R11 capped `\{n\}`'s own count at 1000, same as
/// `\{n,m\}` already was) — the 45,000-long flat, Split/Jump-free
/// instruction run this test needs is now built by CHAINING 45 separate
/// `.\{1000\}` blocks back to back instead of one `.\{45000\}`. Each
/// block is individually within the R11 cap; concatenating many of them
/// is a different (and unrestricted) axis — overall pattern length, not
/// any single counted-repetition operator's own count — so the
/// resulting program is structurally identical (still one long flat
/// run, no `Split`/`Jump` anywhere in it) to what `.\{45000\}` used to
/// compile to.
#[test]
fn fixed_width_repeat_without_split_or_jump_still_bounded_by_budget() {
    let pattern = format!("{}zzz", ".\\\\{1000\\\\}".repeat(45));
    let src = format!(
        r#"(condition-case e
        (string-match "{}" (make-string 60000 ?a))
      (error (car e)))"#,
        pattern
    );
    assert_eq!(run(&src), "regexp-too-complex");
}

/// M81 R8: `split-string`'s own `.map_err(|_| i.regexp_too_complex())?`
/// (`crates/elisp/src/builtins/misc.rs`) was never exercised by any
/// test — this call site converts `RegexLimit` to `regexp-too-complex`
/// independently of `string-match`/`replace-regexp-in-string`'s own
/// conversions, so a broken conversion there wouldn't show up in any
/// other test in this file. Uses the SEPARATOR argument as the
/// pathological pattern (same `\(a+\)+z` shape as the R4 tests above)
/// — a single call exceeding budget is enough to exercise this
/// conversion path; no need for the R5 shared-budget-across-a-loop
/// construction here.
#[test]
fn split_string_signals_regexp_too_complex_not_swallowed() {
    let src = r#"(condition-case e
        (split-string (make-string 30 ?a) "\\(a+\\)+z")
      (error (car e)))"#;
    assert_eq!(run(src), "regexp-too-complex");
}

/// M81 R9: `split-string`'s own loop (`crates/elisp/src/builtins/misc.rs`,
/// `while pos <= s.len() { re.search(&s, pos) ... }`) is the THIRD place
/// (after `replace_all` and `re-search-backward`, R5) that retries the
/// PUBLIC `Regex::search` in an outer loop against the same string — so
/// it has the exact same "individually cheap, cumulatively unbounded"
/// gap R5 fixed, before this test: each iteration got its own fresh
/// `Scratch` (and so a fresh budget), because `search`'s budget is
/// always derived from the WHOLE `s.len()`, never the shrinking
/// remainder.
///
/// Different from `split_string_signals_regexp_too_complex_not_swallowed`
/// above: that test trips the budget WITHIN A SINGLE separator match
/// (`\(a+\)+z`'s nested-quantifier blowup all by itself, on one call).
/// THIS test's separator (`.\{2000\}z`, same shape as R5's own
/// verification construction) is comfortably within budget for any ONE
/// match on its own — it's only the SUM across many matches in the same
/// call that must exceed the shared budget. The two tests exercise
/// different code paths (one call vs. many calls sharing one budget)
/// and neither subsumes the other.
#[test]
fn split_string_shares_budget_across_its_whole_loop_not_per_separator_match() {
    // M81 R11: `w`/`d` swapped from (2000, 1000) to (1000, 2000) —
    // `.\{2000\}z` is no longer a legal single counted-repetition
    // operator (R11 caps it at 1000). `.\{1000\}z` with a larger `d`
    // keeps the same "individually cheap, cumulatively over budget"
    // shape (see the STEP_BUDGET_PER_BYTE math in the M81 R5/R9
    // completion report).
    let src = r#"(let* ((w 1000) (d 2000) (n 8)
                   (unit (concat (make-string (+ w d -1) ?a) "z"))
                   (target ""))
        (dotimes (_ n) (setq target (concat target unit)))
        (condition-case e
            (split-string target ".\\{1000\\}z")
          (error (car e))))"#;
    assert_eq!(run(src), "regexp-too-complex");
}

/// M81: `regexp-too-complex` IS a plain `error` child (the deliberate
/// opposite of `elisp-timeout` — see `timeout_tests.rs`'s
/// `ignore_errors_cannot_swallow_the_timeout`/`explicit_handler_can_catch_it`
/// pair and `Interp::define_standard_errors`'s doc comment), so an
/// ordinary `(condition-case nil ... (error ...))` — the same handler
/// shape callers already use for every other single-call failure —
/// must catch it without needing to name `regexp-too-complex`
/// specifically.
#[test]
fn regexp_too_complex_is_caught_by_a_plain_error_handler() {
    // M81 R4: same pathological-pattern switch as the tests above.
    let src = r#"(condition-case nil
        (string-match "\\(a+\\)+z" (make-string 30 ?a))
      (error 'caught))"#;
    assert_eq!(run(src), "caught");
}

/// M81: capture positions must come out correct after the new
/// explicit-stack engine backtracks through a `Save` more than once —
/// the direct behavioral check on the undo-journal fix (`Regex::backtrack`'s
/// doc comment argues for this by construction; this test is the
/// "trust but verify" companion). `\(a+\)ab` against "aaab": the
/// capture group's greedy `a+` first grabs all three `a`s (group 1 =
/// "aaa"), leaving only "b" for the mandatory trailing "ab" — fails,
/// so the engine must backtrack the `a+` down to two `a`s (re-writing
/// group 1's end position via the journal), after which "ab" matches
/// the remaining "ab". If the undo journal ever failed to replay (or
/// replayed against the wrong frame), group 1 would come out as "aaa"
/// (never backtracked) or something else entirely, not the correct
/// "aa".
#[test]
fn capture_position_correct_after_backtracking_through_save() {
    let src = r#"(progn
        (string-match "\\(a+\\)ab" "aaab")
        (list (match-string 0) (match-string 1)))"#;
    assert_eq!(run(src), "(\"aaab\" \"aa\")");
}

/// The single-group case above CANNOT distinguish "truncate the journal
/// back to this frame's `undo_len`" from "wipe the whole journal": with
/// only one group the journal never holds entries from more than one
/// frame's span, so both do the same thing. Mutation M5 (over-restore)
/// survived against it. This two-group case is the reviewer's own
/// adversarial shape -- it needs the journal unwound across TWO
/// separately-popped frames, so over-restoring destroys group 1's still
/// valid capture and the assertion moves.
#[test]
fn capture_positions_correct_across_two_separately_popped_frames() {
    let src = r#"(progn
        (string-match "\\(a*\\)\\(a*\\)ab" "aaab")
        (list (match-string 0) (match-string 1) (match-string 2)))"#;
    assert_eq!(run(src), "(\"aaab\" \"aa\" \"\")");
}

#[test]
fn invalid_patterns_error_cleanly() {
    assert!(run(r#"(string-match "\\(abc" "abc")"#).starts_with("ERROR:"));
    assert!(run(r#"(string-match "[abc" "abc")"#).starts_with("ERROR:"));
    assert!(run(r#"(string-match "a\\{5,2\\}" "aaa")"#).starts_with("ERROR:"));
    assert!(run(r#"(string-match "\\1" "x")"#).starts_with("ERROR:"));
}

#[test]
fn define_error_hierarchy() {
    let src = r#"(progn
        (define-error 'my-app-error "App failure")
        (define-error 'my-net-error "Network failure" 'my-app-error)
        (list
          ;; A child error is caught by a handler for its parent.
          (condition-case e (signal 'my-net-error '(42))
            (my-app-error (list 'via-parent (cdr e))))
          ;; ... and by the root `error`.
          (condition-case e (signal 'my-net-error nil) (error 'via-root))
          ;; A sibling handler does not catch it; the exact one does.
          (condition-case e (signal 'my-app-error nil)
            (my-net-error 'wrong)
            (my-app-error 'right))))"#;
    assert_eq!(run(src), "((via-parent (42)) via-root right)");
}

#[test]
fn docstrings_are_retained_and_survive_compilation() {
    let src = r#"(progn
        (defun greet (name) "Say hello to NAME." (concat "hi " name))
        (list (documentation 'greet)
              (progn (byte-compile 'greet) (documentation 'greet))
              (greet "ada")
              (documentation 'no-such-doc)))"#;
    assert_eq!(
        run(src),
        "(\"Say hello to NAME.\" \"Say hello to NAME.\" \"hi ada\" nil)"
    );

    // A sole string body is still the return value.
    let src2 = r#"(progn (defun just-str () "only me") (just-str))"#;
    assert_eq!(run(src2), "\"only me\"");
}

#[test]
fn org_style_patterns() {
    // The kinds of patterns org.el actually needs.
    assert_eq!(run(r#"(string-match "^\\*+ " "** Heading")"#), "0");
    assert_eq!(
        run(
            r#"(progn (string-match "^\\(\\*+\\) \\(TODO \\)?\\(.*\\)$" "** TODO Buy milk")
               (list (match-string 1) (match-string 2) (match-string 3)))"#
        ),
        "(\"**\" \"TODO \" \"Buy milk\")"
    );
    assert_eq!(
        run(
            r#"(progn (string-match "\\[\\[\\([^]]+\\)\\]\\[\\([^]]+\\)\\]\\]"
                                    "see [[http://x.org][my site]] ok")
               (list (match-string 1) (match-string 2)))"#
        ),
        "(\"http://x.org\" \"my site\")"
    );
}

/// M81 R5 regression: `replace-regexp-in-string` (`crate::regex::replace_all`)
/// loops calling `Regex::search` once per match — reviewer found each
/// of those calls used to get its OWN fresh `Scratch` (and so a fresh
/// budget), because `search` always derives its budget from the WHOLE
/// (unchanging) `target.len()`, not the shrinking remainder — so a
/// caller retrying `search` in a loop against the same haystack got
/// `(number of calls) x (1,000,000 + 512 x target.len())` total budget
/// instead of the ONE such budget the module doc promises for "an
/// entire operation". Fixed by threading one shared `Scratch` through
/// `replace_all`'s loop via the new `search_with` entry point (see
/// `Scratch`'s and `search`'s doc comments).
///
/// Constructed so each INDIVIDUAL match is comfortably within budget on
/// its own (a standalone `search` call for one such match would never
/// come close to `1,000,000 + 512 * target.len()`), but repeating that
/// same "cheap-but-not-free" cost across 8 real matches in the same
/// string accumulates past the shared budget. Confirmed directly
/// (before writing this test) with a throwaway comparison: looping
/// `Regex::search` (fresh `Scratch` every call, the pre-R5 shape)
/// completes all 8 matches with no error; looping `Regex::search_with`
/// with one shared `Scratch` (the post-fix shape `replace_all` now
/// uses) fails partway through with `RegexLimit` — target_len=24000,
/// pattern `.\{2000\}z`: OLD shape "ok=true calls=9", NEW shape
/// "ok=false calls=7".
#[test]
fn replace_all_shares_budget_across_its_whole_loop_not_per_match() {
    // M81 R11: same (w, d) swap as the `split-string` sibling test —
    // `.\{2000\}z` is no longer legal (R11 caps `\{n\}` at 1000).
    let src = r#"(let* ((w 1000) (d 2000) (n 8)
                   (unit (concat (make-string (+ w d -1) ?a) "z"))
                   (target ""))
        (dotimes (_ n) (setq target (concat target unit)))
        (condition-case e
            (replace-regexp-in-string ".\\{1000\\}z" "X" target)
          (error (car e))))"#;
    assert_eq!(run(src), "regexp-too-complex");
}

/// M81 R10 regression (tail-review finding): `Inst::Save` only journals
/// when there's a live choice point above it (`!scratch.frames.is_empty()`)
/// — but on the SUCCESS path a `Split`-pushed frame is never popped, so
/// an unbounded quantifier wrapping a capture group leaves `frames`
/// non-empty for the rest of the match from its very first iteration
/// onward, meaning EVERY subsequent `Save` gets journaled even though
/// that frame will never actually be backtracked into. Reviewer's
/// repro: this pattern against a haystack it matches in ONE successful,
/// zero-backtracking pass still built a `journal` measured at 262MB (a
/// different run of this same repro, on this machine, measured ~191MB
/// via `/usr/bin/time -l` — see the M81 R10 completion report) for a
/// ~4MB haystack, because neither `max_frames` (peak frame depth stays
/// tiny here — the outer quantifier's frame and the `x|y` alternation's
/// frame, ~2 total, never more) nor `max_steps` (512 steps/byte gives
/// enough slack to not even notice tens of KB of journal per byte) sees
/// unbounded `journal` growth.
///
/// Uses the direct `Regex::search` API (not `string-match` through the
/// elisp interpreter): building this ~4MB haystack via elisp-level
/// `concat` in a loop would itself be O(n^2) (each `concat` re-copies
/// the whole growing string) — tens of billions of byte copies for
/// 8,000 iterations, far too slow for a unit test. `Regex::new`/
/// `.search()` exercise exactly the same engine `string-match` calls
/// into (see this file's other direct-API tests, e.g.
/// `search_out_of_range_start_is_none_not_panic`).
#[test]
fn journal_growth_on_a_successful_zero_backtrack_match_is_bounded() {
    let re = elisp::regex::Regex::new(r"\(\(x\|y\)\(a\)\{500\}\)*").unwrap();
    let unit = format!("x{}", "a".repeat(500));
    let hay = unit.repeat(8000);
    assert_eq!(hay.len(), 4_008_000);
    assert_eq!(re.search(&hay, 0), Err(elisp::regex::RegexLimit));
}

/// M81 R10 reverse test: a genuinely ordinary quantifier-wrapped
/// capture group (`\(a\)*`, no nesting, no extra alternation) must
/// still succeed at hundreds-of-thousands-of-chars scale — the R10 fix
/// must not misfire on the common case just because it now bounds
/// `journal` growth. See `JOURNAL_BUDGET_PER_BYTE`'s doc comment for
/// why this specific shape (exactly one capture group ever active
/// around any given byte) is provably unaffected by the new budget at
/// ANY haystack size, not just the 500,000 used here.
#[test]
fn quantifier_wrapped_capture_group_linear_match_still_succeeds() {
    let re = elisp::regex::Regex::new(r"\(a\)*").unwrap();
    let hay = "a".repeat(500_000);
    let result = re.search(&hay, 0).expect("must not hit the journal budget");
    let caps = result.expect("must find a match");
    assert_eq!(caps[0], Some((0, 500_000)));
}

/// M81 R11 (existing gap, explicitly brought into scope by the
/// coordinator — not introduced by this milestone's diff, `parse_counts`
/// itself wasn't touched by M81 before this fix): `\{n\}` (no comma at
/// all) and `\{n,\}` (open-ended max) both used to return before the
/// `max > 1000` check ever ran — only `\{n,m\}` (explicit min AND max)
/// was actually bounded. `.\{20000000\}` used to compile successfully
/// in ~48ms (`Regex::new`, this machine) and `.\{5000000,\}` in ~12ms —
/// a single elisp `string-match` call reaching `Regex::new` with a huge
/// bare `\{n\}` or open-ended `\{n,\}` could make the PARSER itself try
/// to build a program with tens of millions (or, near `u32::MAX`,
/// billions) of flat instructions, before any M81 runtime budget is
/// even in the picture. Both forms must now be rejected at compile
/// time, same as `\{n,m\}` already was.
#[test]
fn counted_repetition_bare_and_open_ended_forms_respect_the_1000_cap() {
    assert!(elisp::regex::Regex::new(r".\{2000000\}").is_err());
    assert!(elisp::regex::Regex::new(r".\{5000000,\}").is_err());
    // Sanity: right at and just under the cap must still compile.
    assert!(elisp::regex::Regex::new(r".\{1000\}").is_ok());
    assert!(elisp::regex::Regex::new(r".\{1000,\}").is_ok());
    assert!(elisp::regex::Regex::new(r".\{999,1000\}").is_ok());
}

/// M81 R11: the pre-existing `\{n,m\}` (explicit min AND max) cap was
/// already enforced before this fix and has its own `m < n` coverage
/// (`invalid_patterns_error_cleanly`'s `a\{5,2\}` case) — but, until
/// this test, nothing in the suite actually exercised `max > 1000`
/// specifically for ANY of the three forms. Pins the unchanged 1000
/// value and the error message across all three forms post-fix.
#[test]
fn counted_repetition_over_1000_errors_in_all_three_forms() {
    assert!(run(r#"(string-match "a\\{1001\\}" "a")"#).starts_with("ERROR:"));
    assert!(run(r#"(string-match "a\\{1,1001\\}" "a")"#).starts_with("ERROR:"));
    assert!(run(r#"(string-match "a\\{1001,\\}" "a")"#).starts_with("ERROR:"));
    // And the existing `\{n,m\}` boundary case (exactly at 1000) still
    // compiles and matches correctly post-fix.
    assert_eq!(
        run(r#"(string-match "a\\{1,1000\\}b" (concat (make-string 1000 ?a) "b"))"#),
        "0"
    );
}
