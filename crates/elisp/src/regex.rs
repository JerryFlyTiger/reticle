//! Regular expressions, Emacs-Lisp dialect (M11 item 1).
//!
//! The dialect difference that matters: in elisp regexps, grouping,
//! alternation, and counted repetition are the BACKSLASHED forms —
//! `\(...\)`, `\|`, `\{n,m\}` — while bare `( ) { } |` are literal
//! characters. (In source code those backslashes are doubled — `"\\("`
//! — but by the time the string reaches us the reader has already
//! reduced them, so this module sees `\(`.)
//!
//! Supported: literals, `.`, `*` `+` `?` (with `*?` `+?` `??` lazy
//! variants), `[...]` classes (ranges, negation, `[:alpha:]`-style
//! named classes), `^` `$` line anchors, `` \` `` `\'` string anchors,
//! `\(...\)` capture groups, `\(?:...\)` shy groups, `\|`, `\{n,m\}`,
//! `\w` `\W` word chars, `\s-` `\sw` syntax classes, `\b` `\B` `\<`
//! `\>` word boundaries, `\=` (point — treated as string start).
//! Not supported (v1, documented): backreferences in PATTERNS (`\1`),
//! case-folding. `\N` in *replacement templates* IS supported — that's
//! the ubiquitous use (see `replace_all`).
//!
//! Implementation: pattern → AST → compact instruction program → a
//! backtracking VM (the classic regex-VM construction).
//!
//! **Behavior contract for pathological patterns (M81).** Backtracking
//! can still be exponential (or, with the fast literal-prefix skip in
//! `search`, quadratic even on *linear-looking* patterns like `[^;]+x`
//! with no `;` in the haystack — see below) — same as GNU Emacs's own
//! matcher. What's different from GNU Emacs, and from this module
//! before M81: GNU Emacs's matcher runs on the C stack and depends on
//! the user hitting `C-g` (SIGINT) to escape a runaway match; this
//! module used to do the same via genuine Rust function recursion in
//! `run()`, one stack frame per backtrack choice point (`Split`) *and*
//! — wastefully — one per capture-group boundary (`Save`), which had no
//! bound at all: `[^;]*` against a multi-megabyte haystack with no `;`
//! recursed once per matched byte and reliably **stack-overflowed the
//! process (SIGABRT)** — not a `Flow::Signal` a `condition-case` could
//! catch, the whole editor died.
//!
//! `run()` is now an explicit-stack VM (`Frame`/`Scratch` below,
//! iterative, no Rust recursion) with two independent budgets:
//!
//! - `FRAME_BUDGET_BASE`/`FRAME_BUDGET_PER_BYTE` (M81 R4: scales with
//!   `hay.len()`, was a flat constant) bound the *peak* backtrack-frame
//!   stack depth of a single `match_at` call (memory-bounded:
//!   exponential blowup can no longer overflow the process, it hits
//!   this cap and gives up).
//! - `max_steps` bounds the *cumulative* dispatch-step count across an
//!   entire `search()` call — i.e. across every internal `match_at`
//!   retry at every candidate start position, not just one. This is the
//!   one that matters in practice: `search`'s outer "try the next
//!   position" loop (when the pattern has no literal prefix to skip
//!   with) turns an O(n) per-position backtrack into an O(n²) total,
//!   and that quadratic blowup shows up long before any single
//!   `match_at` call gets anywhere near the frame budget — measured 4.64s
//!   of unresponsiveness for `[^;]+x` on a 30KB haystack in the old
//!   engine, nowhere close to overflowing. A budget scoped to one
//!   `match_at` call cannot see that; only a budget that accumulates
//!   across `search()`'s whole retry loop can.
//!
//! Hitting either budget returns `Err(RegexLimit)`, which the elisp-
//! facing entry points (`string_match`, `replace_all`, and the `core`
//! crate's `re-search-forward`/`re-search-backward`/`looking-at`)
//! convert to the `regexp-too-complex` signal (`Interp::regexp_too_complex`)
//! — a plain `error` child (unlike `elisp-timeout`, see
//! `Interp::define_standard_errors`), because it's the caller's *this
//! one call* that failed, not a "the user wants to cancel everything"
//! event. Silently returning "no match" instead would be worse: it's
//! indistinguishable from an honest non-match, and would make e.g.
//! evil's `:s///` report a flatly false "Pattern not found" for a
//! pattern that isn't actually wrong.
//!
//! The intent, in one line: **linear is fine, quadratic is not** — a
//! typical `re-search-forward` over a real buffer must keep costing
//! what it always did; only patterns/haystacks that are genuinely
//! adversarial should ever hit these budgets.
//!
//! Positions handled by the *engine* (`Regex::search`/`match_at`, and
//! everything below `Inst`) are BYTE offsets into a `&str` (M43 period
//! 2 — see the module's differential-testing `mod tests::legacy_char_engine`
//! for the pre-M43 char-indexed version this replaces). Positions
//! crossing the `Inst`/engine boundary into `MatchData` and the elisp
//! builtins remain CHAR offsets, unchanged — `MatchData.caps` is what
//! `match-beginning`/`match-end`/`match-string` read, and elisp's
//! string/buffer position semantics are (and stay) char-indexed.
//! `MatchData.caps_bytes` carries the engine's native byte positions
//! alongside, so `match-string` can slice `MatchData.target` (now
//! `Rc<str>`) directly instead of re-decoding.

use std::collections::HashMap;
use std::rc::Rc;

use crate::error::Flow;
use crate::interp::Interp;

#[derive(Debug)]
enum Node {
    Char(char),
    Any,
    Class(usize),
    LineStart,
    LineEnd,
    StrStart,
    StrEnd,
    WordBoundary(bool),
    WordStart,
    WordEnd,
    WordChar(bool),
    Space(bool),
    Group(Option<usize>, Vec<Vec<Node>>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
}

struct ClassSpec {
    negated: bool,
    singles: Vec<char>,
    ranges: Vec<(char, char)>,
    named: Vec<Named>,
}

#[derive(Clone, Copy)]
enum Named {
    Alpha,
    Digit,
    Alnum,
    Space,
    Upper,
    Lower,
    Word,
    Punct,
}

impl ClassSpec {
    fn matches(&self, c: char) -> bool {
        let hit = self.singles.contains(&c)
            || self.ranges.iter().any(|&(a, b)| c >= a && c <= b)
            || self.named.iter().any(|n| match n {
                Named::Alpha => c.is_alphabetic(),
                Named::Digit => c.is_ascii_digit(),
                Named::Alnum => c.is_alphanumeric(),
                Named::Space => c.is_whitespace(),
                Named::Upper => c.is_uppercase(),
                Named::Lower => c.is_lowercase(),
                Named::Word => is_word(c),
                Named::Punct => c.is_ascii_punctuation(),
            });
        hit != self.negated
    }
}

// Also backs `\b`/`\<`/`\>` (word boundary/start/end), not just the
// `[[:word:]]'/`\w` class above: GNU's own syntax-table notion of
// "word constituent" is configurable per mode and doesn't universally
// include `_` (many modes make it a symbol constituent instead), but
// this is a fixed, syntax-table-free classifier, so `_` counts as a
// word character unconditionally here. core's verilog-auto.el
// (`verilog-auto--substitute-params`) relies on exactly this: Verilog
// identifiers routinely contain `_`, and its whole-word parameter
// substitution (`\<WIDTH\>`, so it doesn't also match inside `WIDTH2`)
// needs `_` to behave as a word character for that boundary to land
// in the right place around such names.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

// --- Compiled program ---

/// `pos` throughout the VM (`run`, and every `Inst` variant below) is a
/// BYTE offset into the `&str` haystack (M43 period 2) — see the
/// module doc. `Inst::Byte`/`Inst::Chars` are the byte-native
/// replacement for the pre-M43 `Inst::Char(char)`: a pattern char is
/// pre-encoded to UTF-8 at compile time (`compile_node`), so matching
/// it at runtime is a plain byte (or short byte-array) comparison —
/// no per-step decode, and ASCII patterns (the overwhelming common
/// case) compile to single-byte comparisons exactly as cheap as the
/// pre-M43 `Vec<char>` engine's `Inst::Char` was.
#[derive(Clone, Copy, Debug)]
enum Inst {
    /// ASCII pattern char (`c.is_ascii()`): compare one byte.
    Byte(u8),
    /// Multi-byte pattern char: compare `len` bytes of the pre-encoded
    /// UTF-8 sequence (`len` is 2..=4; the trailing `4 - len` bytes of
    /// the array are unused padding).
    Chars([u8; 4], u8),
    Any,
    Class(u16),
    /// Try `a` first; on failure resume at `b`. Order encodes greed.
    Split(u32, u32),
    Jump(u32),
    Save(u8),
    LineStart,
    LineEnd,
    StrStart,
    StrEnd,
    WordBoundary(bool),
    WordStart,
    WordEnd,
    WordChar(bool),
    Space(bool),
    Match,
}

pub struct Regex {
    prog: Vec<Inst>,
    classes: Vec<ClassSpec>,
    pub n_groups: usize,
    /// Literal bytes (pre-encoded UTF-8) that any match must begin
    /// with, extracted at compile time (see `extract_literal_prefix`).
    /// Empty means "no usable prefix" — `search` then falls back to
    /// trying every char position, exactly as before this optimization
    /// existed. A `String` (not `Vec<char>`, M43 period 2): `search`'s
    /// fast path hands this straight to `str::find`, which is what
    /// buys the SIMD/Two-Way substring search speedup over the
    /// pre-M43 per-char scan (see the module doc and the M43 design
    /// doc §3.4/§1.4 — `str::find` measured ~7.3x faster than the
    /// `Vec<char>` equivalent on realistic buffer sizes).
    literal_prefix: String,
    /// `Some(c)` iff `literal_prefix` is exactly one **char** long (a
    /// char count, not a byte count — a single-CJK-char prefix is 1
    /// char but 3 bytes and still belongs here), `None` for an empty or
    /// multi-char prefix. Precomputed once at construction time (M43
    /// period 3) rather than re-checked on every `search()` call, since
    /// `search` can run many times per compiled pattern (once per
    /// isearch keystroke, once per loop iteration of `re-search-
    /// forward`, ...). Lets `search`'s prefix-skip loop dispatch to
    /// `str::find` with a `char` pattern instead of a length-1 `&str`
    /// one — see `gapbuffer::single_char`'s doc comment for why that's
    /// measurably faster (~15-20x on a 1-char needle).
    literal_prefix_char: Option<char>,
}

/// Returned by `Regex::search`/`Regex::match_at` when the match attempt
/// exceeded its work budget (M81 — see the module doc's "Behavior
/// contract for pathological patterns" section). Carries no data; the
/// call sites that need a user-facing message convert this to the
/// `regexp-too-complex` elisp signal via `Interp::regexp_too_complex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegexLimit;

/// One match's capture slots: index 0 is the whole match, `1..=n_groups`
/// are the `\(...\)` groups, and `None` means "that group never
/// participated in this match". Byte offsets into the haystack.
pub type Captures = Vec<Option<(usize, usize)>>;

/// What the matcher hands back: `Ok(Some(caps))` matched, `Ok(None)` did
/// not match, `Err(RegexLimit)` means the M81 step/frame budget ran out
/// before either answer was reached — the caller must turn that into the
/// `regexp-too-complex` signal, never into a silent "no match" (see this
/// file's header: conflating the two makes the editor tell the user a
/// lie about their own pattern).
pub type MatchOutcome = Result<Option<Captures>, RegexLimit>;

/// Fixed part of the peak backtrack-frame stack depth allowed for a
/// single `match_at` call — see `FRAME_BUDGET_PER_BYTE` for the
/// length-proportional part.
const FRAME_BUDGET_BASE: usize = 1_000;

/// Length-proportional multiplier for the frame budget:
/// `max_frames = FRAME_BUDGET_BASE + FRAME_BUDGET_PER_BYTE * hay.len()`.
///
/// M81 R4 fix: `MAX_FRAMES` used to be a FLAT `1_000_000`, independent
/// of `hay.len()` — but on the success path a frame is only ever
/// PUSHED, never popped (popping only happens on backtrack failure —
/// see `backtrack`), so a purely linear match with an unbounded greedy
/// quantifier (e.g. `[^;]*`, one `Split` push per char consumed, same
/// shape as the pre-M81 crash repro) needs `frames.len()` to reach
/// roughly the number of chars it consumes. A flat 1,000,000 meant a
/// perfectly linear, non-backtracking match over ~1M+ chars was
/// misclassified as "too complex" — reviewer measured `[^;]*` against
/// 999,000 chars succeeding and the SAME pattern against 1,500,000
/// chars failing with `RegexLimit`, purely from crossing the flat cap,
/// which directly contradicts this module's own "linear is fine,
/// quadratic is not" contract (see the module doc's "Behavior
/// contract" section). Scaling the frame budget with `hay.len()` (same
/// shape as `max_steps`) fixes that: a fully linear consuming match
/// stays comfortably under `FRAME_BUDGET_BASE + FRAME_BUDGET_PER_BYTE *
/// hay.len()` regardless of haystack size, while genuine backtracking
/// explosion is still caught — primarily by `max_steps` (exponential
/// blowup multiplies total STEPS far faster than peak stack DEPTH; see
/// `fixed_width_repeat_without_split_or_jump_still_bounded_by_budget`'s
/// sibling pathological-pattern tests), with `MAX_FRAMES` remaining as
/// the memory-bounded backstop it always was (so a single `match_at`
/// call's frame stack can't grow unboundedly even in pathological
/// shapes that manage to keep pushing without popping).
const FRAME_BUDGET_PER_BYTE: usize = 2;

/// Fixed part of the peak `Scratch::journal` length allowed for a
/// single `match_at` call — see `JOURNAL_BUDGET_PER_BYTE` for the
/// length-proportional part and why the journal needs its OWN budget
/// (M81 R10, tail review finding: `frames`/`max_steps` don't bound it).
const JOURNAL_BUDGET_BASE: usize = 10_000;

/// Length-proportional multiplier for the journal budget:
/// `max_journal = JOURNAL_BUDGET_BASE + JOURNAL_BUDGET_PER_BYTE *
/// hay.len()`.
///
/// M81 R10 fix. The bug this closes: `Inst::Save` only journals
/// (`(slot, old_value)`) when `frames` is non-empty — but on the
/// SUCCESS path a frame, once pushed, is never popped (only backtrack-
/// on-failure pops), so ANY unbounded quantifier wrapping a capture
/// group leaves `frames` non-empty for the rest of the match the moment
/// its own `Split` is pushed (which happens before the body — including
/// its first iteration — even runs), meaning EVERY `Save` from then on
/// gets journaled even though that frame will never actually be
/// backtracked into. Reviewer's repro: `\(\(x\|y\)\(a\)\{500\}\)*`
/// against a haystack that matches it in one successful, zero-
/// backtracking pass still built a `journal` 262MB in size for a 4MB
/// haystack — neither `max_frames` (peak *frame* depth stays tiny here,
/// ~2, since it doesn't track how many *Save*s happened while a frame
/// sat live) nor `max_steps` (loose: 512 steps/byte gives headroom for
/// up to ~10KB of journal per byte before it would even notice) sees
/// this growth.
///
/// Calibrated (release build, measuring `Scratch::journal.len()` at
/// successful match completion, instrumented then removed — see the
/// R10 completion report for the full run) against the shapes the
/// milestone spec asked for:
///
/// - `\(a\)*` (single group, unbounded quantifier): journal/byte → 2
///   asymptotically (`(2n+1)/n`, measured 2,000,001 / 1,000,000 and
///   200,001 / 100,000) — a save-open + save-close pair every byte,
///   the theoretical FLOOR for any pattern that uses a capture group at
///   all (one group active per byte, never more than one at a time).
/// - `\(\(a\)*b\)*` (nested — outer group amortized over the whole
///   11-byte unit, inner group saturated per-byte like the case above):
///   also → 2/byte asymptotically (measured 2,200,001 / 1,100,000) —
///   nesting alone doesn't push past the floor as long as only ONE
///   group is actively wrapping any given byte.
/// - Reviewer's repro `\(\(x\|y\)\(a\)\{500\}\)*` (TWO extra
///   groups — the outer wrapper and the `x|y` alternation — each
///   re-entered once per 501-byte iteration, on top of an already-
///   saturated inner `\(a\)`): measured 8,032,001 / 4,008,000 ≈
///   2.00399/byte — barely above the floor in RATIO terms, but that
///   small constant excess (~16,001 total at this haystack size) is
///   what a linear budget is for: it's genuinely extra, non-amortized
///   per-iteration cost that the floor-ratio patterns above don't pay.
///
/// `JOURNAL_BUDGET_PER_BYTE = 2` is deliberately set to the EXACT
/// measured floor (not a multiple of it, unlike `FRAME_BUDGET_PER_BYTE`
/// doubling its own 1/byte floor) — this is a provable, not just
/// generous, choice: for any pattern whose asymptotic ratio is <= 2/byte
/// (which covers both calibration shapes above, and by construction any
/// pattern where at most one capture group is ever active around a
/// given byte — the overwhelmingly common real-world shape, including
/// font-lock/org-mode/Verilog patterns that don't nest groups around
/// the same span), `journal.len() - PER_BYTE*hay.len()` is a CONSTANT
/// (independent of `hay.len()`, from the one-time group-0 save pair —
/// see `ProgBuilder`'s `Save(0)`/`Save(1)` bookends in `Regex::new`),
/// so `JOURNAL_BUDGET_BASE` alone (not a per-byte margin) is what has
/// to absorb it, and it does so at ANY haystack size — confirmed with
/// `\(a\)*` up to 1,000,000 bytes (margin stayed a constant ~9,999
/// regardless of size). `JOURNAL_BUDGET_BASE = 10_000` leaves ~9,999 of
/// that margin spare (comfortable slack above the 1-per-call constant
/// overhead) while staying safely under reviewer's repro's ~16,001
/// excess at 4,008,000 bytes, so it still trips there. A pattern with
/// TWO OR MORE groups genuinely re-entered on nearly every byte (not
/// amortized over a large per-iteration span) is exactly the shape this
/// budget is meant to catch once the haystack is large enough — that's
/// a real, if modest, per-byte memory multiplier on top of the 2/byte
/// floor, and the whole point of a per-byte budget is that a small
/// per-byte excess becomes real at scale. (Not separately calibrated:
/// an even more extreme shape — 20 capture groups, the parser's own
/// cap, all nested around a single repeated char — measured ~40/byte;
/// that's far above this budget and would already fail on a
/// haystack of a few hundred bytes. Not asked for by the spec's
/// calibration list, and not a shape any real editing pattern uses, so
/// no attempt was made to keep it under budget — noted here for the
/// record, not defended as "must succeed".)
const JOURNAL_BUDGET_PER_BYTE: usize = 2;

/// Fixed part of the cumulative step budget for one `search()` call —
/// see `STEP_BUDGET_PER_BYTE` for the length-proportional part and the
/// module doc for the two-budget design.
const STEP_BUDGET_BASE: u64 = 1_000_000;

/// Length-proportional multiplier for `search()`'s cumulative step
/// budget: `max_steps = STEP_BUDGET_BASE + STEP_BUDGET_PER_BYTE *
/// hay.len()`. `steps` is only incremented in the `Inst::Split`/
/// `Inst::Jump` arms of `run` (see `Scratch::steps`'s doc comment for
/// why straight-line instructions don't need their own counting) —
/// M81: measured 360_378_566 total counted steps for `.*199999` (no
/// literal prefix — every one of the 8,488,890-byte haystack's
/// positions gets a `match_at` retry, each one backtracking over the
/// rest of the line) over the 200k-line haystack
/// (`regex_prefix_skip_perf_tests.rs`'s
/// `measure_no_prefix_pattern_is_unaffected`, instrumented with a
/// temporary `eprintln!` of `Scratch::steps`, release build; an earlier
/// design that counted every dispatch step measured 739_315_589 steps
/// for the same case but ran ~1.9x *slower* overall — pure counting
/// overhead with no budget-accuracy benefit, which is why counting was
/// narrowed to just the two back-edge instructions). 512 gives
/// `1_000_000 + 512 * 8_488_890 ≈ 4.35e9`, ~12x headroom over that
/// measured legitimate-but-expensive case, while still catching
/// genuinely pathological (quadratic-or-worse) patterns well before
/// they cost user-visible multi-second stalls.
const STEP_BUDGET_PER_BYTE: u64 = 512;

/// One backtrack choice point, pushed by `Inst::Split` and popped on
/// failure. `undo_len` is the `Scratch::journal` length at push time —
/// backtracking truncates the journal back to it, replaying entries in
/// reverse to undo exactly the `Save`s performed since this frame was
/// pushed (see `Regex::backtrack`'s doc comment for the equivalence
/// argument with the old recursive engine).
struct Frame {
    pc: u32,
    pos: u32,
    undo_len: u32,
}

/// Per-`match_at`-call (or, for `search`/`search_with`, per-whole-
/// search — and, for a caller that itself loops over multiple
/// `search_with` calls against the same haystack, per-whole-OUTER-LOOP:
/// see `search`'s doc comment and M81 R5) scratch state.
///
/// M81 R7 correction: an earlier version of this comment credited M43
/// with already having eliminated the per-candidate-position `saves`
/// allocation. That's wrong — M43's `search` (before M81) still called
/// the PUBLIC `match_at`, whose body was `let mut saves = vec![None;
/// ...]` — a fresh heap allocation at every single candidate position
/// `search` tried. What M43 eliminated was the NUMBER of candidate
/// positions tried (the literal-prefix `str::find` skip), not the
/// per-position allocation itself; a pattern with no usable literal
/// prefix still paid one `saves` allocation per position tried, same as
/// before M43. **M81 (this milestone) is what actually eliminates the
/// per-call `saves` allocation** — by introducing this `Scratch` and
/// threading it through every internal `match_at_with` retry instead of
/// each retry allocating its own `saves` Vec — and, for R5, extends
/// that same reuse across an OUTER caller's own retry loop
/// (`replace_all`, `re-search-backward`) via `search_with`, not just
/// within one `search` call.
#[derive(Default)]
pub struct Scratch {
    saves: Vec<Option<usize>>,
    frames: Vec<Frame>,
    journal: Vec<(u8, Option<usize>)>,
    /// Cumulative dispatch-step count. For a standalone `match_at` call
    /// this covers just that one call; for `search`/`search_with`, the
    /// same `Scratch` (and so the same counter) is threaded through
    /// every internal `match_at` retry — and, when a caller shares one
    /// `Scratch` across its own outer loop of `search_with` calls (M81
    /// R5), across THOSE too — so the budget is enforced against the
    /// whole logical operation, not any single retry (see module doc).
    steps: u64,
    max_steps: u64,
    /// Peak `frames.len()` allowed for a single `match_at` call (M81
    /// R4 — see `FRAME_BUDGET_PER_BYTE`'s doc comment for why this must
    /// scale with `hay.len()` rather than being a flat constant).
    max_frames: usize,
    /// Peak `journal.len()` allowed for a single `match_at` call (M81
    /// R10 — see `JOURNAL_BUDGET_PER_BYTE`'s doc comment). Scoped PER
    /// CALL, same as `max_frames` and for the same reason: `run` clears
    /// both `frames` and `journal` at the start of every `match_at`
    /// attempt (see `run`'s doc comment), so nothing carries over
    /// between candidate positions the way `steps` does.
    max_journal: usize,
}

impl Scratch {
    /// `hay_len` should be the length of the haystack this `Scratch`
    /// will be used against — for a caller sharing one `Scratch` across
    /// multiple `search_with` calls (M81 R5), that's the length of the
    /// (unchanging, across the loop) haystack all of those calls search,
    /// not any one match's size.
    pub fn new(hay_len: usize) -> Scratch {
        Scratch {
            saves: Vec::new(),
            frames: Vec::new(),
            journal: Vec::new(),
            steps: 0,
            max_steps: STEP_BUDGET_BASE + STEP_BUDGET_PER_BYTE * hay_len as u64,
            max_frames: FRAME_BUDGET_BASE + FRAME_BUDGET_PER_BYTE * hay_len,
            max_journal: JOURNAL_BUDGET_BASE + JOURNAL_BUDGET_PER_BYTE * hay_len,
        }
    }
}

// --- Parser ---

struct Parser<'a> {
    chars: &'a [char],
    pos: usize,
    next_group: usize,
    classes: Vec<ClassSpec>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    /// True when the next two chars are `\` followed by `c`.
    fn at_escaped(&self, c: char) -> bool {
        self.peek() == Some('\\') && self.peek2() == Some(c)
    }

    fn parse_alt(&mut self) -> Result<Vec<Vec<Node>>, String> {
        let mut alts = vec![self.parse_seq()?];
        while self.at_escaped('|') {
            self.pos += 2;
            alts.push(self.parse_seq()?);
        }
        Ok(alts)
    }

    fn parse_seq(&mut self) -> Result<Vec<Node>, String> {
        let mut seq = Vec::new();
        loop {
            if self.pos >= self.chars.len() || self.at_escaped('|') || self.at_escaped(')') {
                return Ok(seq);
            }
            let node = self.parse_atom()?;
            let node = self.parse_postfix(node)?;
            seq.push(node);
        }
    }

    fn parse_postfix(&mut self, node: Node) -> Result<Node, String> {
        let (min, max) = if self.eat('*') {
            (0, None)
        } else if self.eat('+') {
            (1, None)
        } else if self.eat('?') {
            (0, Some(1))
        } else if self.at_escaped('{') {
            self.pos += 2;
            let (min, max) = self.parse_counts()?;
            if !self.at_escaped('}') {
                return Err("unterminated \\{".into());
            }
            self.pos += 2;
            (min, max)
        } else {
            return Ok(node);
        };
        let greedy = !self.eat('?');
        Ok(Node::Repeat {
            node: Box::new(node),
            min,
            max,
            greedy,
        })
    }

    fn parse_counts(&mut self) -> Result<(u32, Option<u32>), String> {
        // M81 R11 fix: `min` itself must be checked against the same
        // 1000 cap as `max` -- before this fix, ONLY the `\{n,m\}` arm
        // (explicit min AND max) ever reached the `max > 1000` check
        // below; `\{n\}` (no comma at all) returned at the first
        // early-out, and `\{n,\}` (open-ended max) returned at the
        // second, both BEFORE any bound check ran. Either form lets
        // `Regex::new` itself (parse + compile, before any M81 runtime
        // budget ever gets a chance to run) try to build a
        // multi-hundred-million-instruction program from a single
        // elisp `string-match` call -- reviewer measured `.\{20000000\}`
        // compiling successfully in ~48ms and `.\{5000000,\}` in ~12ms
        // on this machine; extrapolated toward `u32::MAX` this tries to
        // allocate tens of GB at PARSE time. Every return path below
        // now goes through the same `check_count` bound, so the 1000
        // cap (unchanged value, already has its own test coverage)
        // applies uniformly to `min` in all three forms
        // (`\{n\}`/`\{n,\}`/`\{n,m\}`), not just to `max` in the last one.
        fn check_count(n: u32) -> Result<u32, String> {
            if n > 1000 {
                Err("\\{n,m\\} repetition too large".to_string())
            } else {
                Ok(n)
            }
        }
        let mut min_s = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            min_s.push(self.bump().unwrap());
        }
        let min: u32 = min_s.parse().map_err(|_| "bad \\{n,m\\}".to_string())?;
        let min = check_count(min)?;
        if !self.eat(',') {
            return Ok((min, Some(min))); // \{n\}
        }
        let mut max_s = String::new();
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            max_s.push(self.bump().unwrap());
        }
        if max_s.is_empty() {
            Ok((min, None)) // \{n,\} -- `min` already checked above
        } else {
            let max: u32 = max_s.parse().map_err(|_| "bad \\{n,m\\}".to_string())?;
            if max < min {
                return Err("\\{n,m\\} with m < n".into());
            }
            let max = check_count(max)?;
            Ok((min, Some(max)))
        }
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        let c = self.bump().ok_or("unexpected end of regexp")?;
        match c {
            '.' => Ok(Node::Any),
            '^' => Ok(Node::LineStart),
            '$' => Ok(Node::LineEnd),
            '[' => self.parse_class(),
            '\\' => {
                let e = self.bump().ok_or("trailing backslash")?;
                match e {
                    '(' => {
                        // \(?: is a shy (non-capturing) group.
                        let shy = if self.peek() == Some('?') && self.peek2() == Some(':') {
                            self.pos += 2;
                            true
                        } else {
                            false
                        };
                        let num = if shy {
                            None
                        } else {
                            self.next_group += 1;
                            Some(self.next_group)
                        };
                        let alts = self.parse_alt()?;
                        if !self.at_escaped(')') {
                            return Err("unterminated \\(".into());
                        }
                        self.pos += 2;
                        Ok(Node::Group(num, alts))
                    }
                    'w' => Ok(Node::WordChar(true)),
                    'W' => Ok(Node::WordChar(false)),
                    's' => {
                        // Syntax classes; we support the two common ones.
                        match self.bump() {
                            Some('-') | Some(' ') => Ok(Node::Space(true)),
                            Some('w') => Ok(Node::WordChar(true)),
                            other => Err(format!("unsupported syntax class \\s{:?}", other)),
                        }
                    }
                    'S' => match self.bump() {
                        Some('-') | Some(' ') => Ok(Node::Space(false)),
                        Some('w') => Ok(Node::WordChar(false)),
                        other => Err(format!("unsupported syntax class \\S{:?}", other)),
                    },
                    'b' => Ok(Node::WordBoundary(true)),
                    'B' => Ok(Node::WordBoundary(false)),
                    '<' => Ok(Node::WordStart),
                    '>' => Ok(Node::WordEnd),
                    '`' => Ok(Node::StrStart),
                    '\'' => Ok(Node::StrEnd),
                    '=' => Ok(Node::StrStart), // point-anchor: string start here
                    'n' => Ok(Node::Char('\n')),
                    't' => Ok(Node::Char('\t')),
                    'r' => Ok(Node::Char('\r')),
                    '1'..='9' => Err("pattern backreferences (\\N) are not supported".into()),
                    // Any other escaped char is itself (covers \. \* \[ \\ ...).
                    other => Ok(Node::Char(other)),
                }
            }
            other => Ok(Node::Char(other)),
        }
    }

    fn parse_class(&mut self) -> Result<Node, String> {
        let negated = self.eat('^');
        let mut spec = ClassSpec {
            negated,
            singles: Vec::new(),
            ranges: Vec::new(),
            named: Vec::new(),
        };
        // A `]` immediately after `[` or `[^` is a literal.
        if self.peek() == Some(']') {
            self.bump();
            spec.singles.push(']');
        }
        loop {
            let c = self.bump().ok_or("unterminated character class")?;
            if c == ']' {
                break;
            }
            // [:name:]
            if c == '[' && self.peek() == Some(':') {
                let save = self.pos;
                self.bump(); // ':'
                let mut name = String::new();
                while matches!(self.peek(), Some(ch) if ch.is_ascii_alphabetic()) {
                    name.push(self.bump().unwrap());
                }
                if self.eat(':') && self.eat(']') {
                    let named = match name.as_str() {
                        "alpha" => Named::Alpha,
                        "digit" => Named::Digit,
                        "alnum" => Named::Alnum,
                        "space" => Named::Space,
                        "upper" => Named::Upper,
                        "lower" => Named::Lower,
                        "word" => Named::Word,
                        "punct" => Named::Punct,
                        _ => return Err(format!("unknown class [:{}:]", name)),
                    };
                    spec.named.push(named);
                    continue;
                }
                self.pos = save;
                spec.singles.push('[');
                continue;
            }
            if self.peek() == Some('-') && self.peek2().is_some() && self.peek2() != Some(']') {
                self.bump(); // '-'
                let hi = self.bump().unwrap();
                if hi < c {
                    return Err("inverted range in character class".into());
                }
                spec.ranges.push((c, hi));
            } else {
                spec.singles.push(c);
            }
        }
        self.classes.push(spec);
        Ok(Node::Class(self.classes.len() - 1))
    }
}

/// Pulls out the leading run of literal characters a pattern must start
/// with, so `search` can skip straight to candidate positions instead of
/// invoking the backtracking VM at every offset (P2 #8: on a large
/// buffer, patterns like `"line 199999"` were re-entering `match_at` —
/// heap-allocating a fresh `saves` vector and running the VM — at every
/// single character position, even though the vast majority fail on the
/// very first `Inst::Byte`/`Inst::Chars`).
///
/// Only handles the simple, common case: no top-level `\|` alternation
/// (`alts.len() == 1`), and the sequence starts with zero or more
/// zero-width assertions (`^` `$` `` \` `` `\'` `\b` `\B` `\<` `\>`,
/// which consume no characters and so don't break up the literal run)
/// followed by a run of plain `Node::Char`. The first `Any`/`Class`/
/// `Group`/`Repeat` node stops collection. A pattern that never yields
/// any literal chars (e.g. it starts with `.`, a class, a group, or has
/// top-level alternation) returns an empty `String`, which callers treat
/// as "no prefix available" and fall back to the unoptimized path — same
/// behavior as before this function existed, just not faster.
///
/// Returns a `String` (M43 period 2; was `Vec<char>`) so `search` can
/// hand it directly to `str::find`.
fn extract_literal_prefix(alts: &[Vec<Node>]) -> String {
    if alts.len() != 1 {
        return String::new();
    }
    let mut prefix = String::new();
    for node in &alts[0] {
        match node {
            Node::Char(c) => prefix.push(*c),
            Node::LineStart
            | Node::LineEnd
            | Node::StrStart
            | Node::StrEnd
            | Node::WordBoundary(_)
            | Node::WordStart
            | Node::WordEnd => {
                // Zero-width: consumes no characters, doesn't interrupt
                // the literal run being collected.
            }
            _ => break,
        }
    }
    prefix
}

// --- Byte/char decoding helpers (M43 period 2) ---
//
// The engine works over `&str` + byte positions rather than `&[char]`,
// so a handful of small helpers stand in for what direct `char`
// indexing used to give for free. Each has an ASCII-byte fast path
// (a single byte compare/cast, no decode) since ASCII patterns and
// text are the overwhelming common case — see the M43 design doc §3.3.

/// Byte length (1..=4) of the UTF-8-encoded char whose leading byte is
/// `lead`.
fn utf8_lead_len(lead: u8) -> usize {
    if lead & 0x80 == 0 {
        1
    } else if lead & 0xE0 == 0xC0 {
        2
    } else if lead & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

/// Decode the char starting at byte offset `pos` in `hay`, returning it
/// with its UTF-8 byte length. `None` at end of string. ASCII fast
/// path: a byte < 0x80 IS its own char, one byte, no decode needed.
fn char_at_byte(hay: &str, pos: usize) -> Option<(char, usize)> {
    let b = *hay.as_bytes().get(pos)?;
    if b < 0x80 {
        Some((b as char, 1))
    } else {
        let c = hay.get(pos..)?.chars().next()?;
        Some((c, c.len_utf8()))
    }
}

/// Decode the char immediately before byte offset `pos` in `hay` (i.e.
/// ending at `pos`). `None` at the start of string. Used by the word-
/// boundary instructions, which need to classify the char on both sides
/// of `pos`. ASCII fast path as `char_at_byte`; the multi-byte case
/// defers to `str`'s own backward char decode (`Chars::next_back`)
/// rather than hand-walking continuation bytes.
fn char_before_byte(hay: &str, pos: usize) -> Option<char> {
    if pos == 0 {
        return None;
    }
    let bytes = hay.as_bytes();
    if bytes[pos - 1] < 0x80 {
        return Some(bytes[pos - 1] as char);
    }
    hay.get(..pos)?.chars().next_back()
}

/// UTF-8 byte length of the char starting at `pos` (1 if `pos` is at or
/// past the end — callers only use this to compute a *next* position to
/// resume scanning from, where any positive step is safe).
fn char_len_at(hay: &str, pos: usize) -> usize {
    char_at_byte(hay, pos).map(|(_, len)| len).unwrap_or(1)
}

// --- Compiler (AST → program) ---

struct ProgBuilder {
    prog: Vec<Inst>,
}

impl ProgBuilder {
    fn emit(&mut self, i: Inst) -> usize {
        self.prog.push(i);
        self.prog.len() - 1
    }
    fn here(&self) -> u32 {
        self.prog.len() as u32
    }
    fn patch(&mut self, at: usize, to: u32) {
        match &mut self.prog[at] {
            Inst::Split(_, b) if *b == u32::MAX => {
                if let Inst::Split(_, b) = &mut self.prog[at] {
                    *b = to;
                }
            }
            Inst::Jump(t) => *t = to,
            Inst::Split(a, _) if *a == u32::MAX => {
                if let Inst::Split(a, _) = &mut self.prog[at] {
                    *a = to;
                }
            }
            _ => unreachable!("patch target is not a jump"),
        }
    }

    fn compile_alts(&mut self, alts: &[Vec<Node>]) {
        // alt1 \| alt2 \| alt3 — chain of Splits, each preferring its
        // own branch, falling through to the next alternative.
        let mut end_jumps = Vec::new();
        for (i, seq) in alts.iter().enumerate() {
            if i + 1 < alts.len() {
                let split = self.emit(Inst::Split(0, u32::MAX));
                let body = self.here();
                if let Inst::Split(a, _) = &mut self.prog[split] {
                    *a = body;
                }
                self.compile_seq(seq);
                end_jumps.push(self.emit(Inst::Jump(u32::MAX)));
                let next = self.here();
                self.patch(split, next);
            } else {
                self.compile_seq(seq);
            }
        }
        let end = self.here();
        for j in end_jumps {
            self.patch(j, end);
        }
    }

    fn compile_seq(&mut self, seq: &[Node]) {
        for n in seq {
            self.compile_node(n);
        }
    }

    fn compile_node(&mut self, node: &Node) {
        match node {
            Node::Char(c) => {
                if c.is_ascii() {
                    self.emit(Inst::Byte(*c as u8));
                } else {
                    let mut buf = [0u8; 4];
                    let len = c.encode_utf8(&mut buf).len() as u8;
                    self.emit(Inst::Chars(buf, len));
                }
            }
            Node::Any => {
                self.emit(Inst::Any);
            }
            Node::Class(i) => {
                self.emit(Inst::Class(*i as u16));
            }
            Node::LineStart => {
                self.emit(Inst::LineStart);
            }
            Node::LineEnd => {
                self.emit(Inst::LineEnd);
            }
            Node::StrStart => {
                self.emit(Inst::StrStart);
            }
            Node::StrEnd => {
                self.emit(Inst::StrEnd);
            }
            Node::WordBoundary(b) => {
                self.emit(Inst::WordBoundary(*b));
            }
            Node::WordStart => {
                self.emit(Inst::WordStart);
            }
            Node::WordEnd => {
                self.emit(Inst::WordEnd);
            }
            Node::WordChar(b) => {
                self.emit(Inst::WordChar(*b));
            }
            Node::Space(b) => {
                self.emit(Inst::Space(*b));
            }
            Node::Group(num, alts) => {
                if let Some(n) = num {
                    self.emit(Inst::Save((n * 2) as u8));
                    self.compile_alts(alts);
                    self.emit(Inst::Save((n * 2 + 1) as u8));
                } else {
                    self.compile_alts(alts);
                }
            }
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => {
                // Mandatory prefix: min copies.
                for _ in 0..*min {
                    self.compile_node(node);
                }
                match max {
                    None => {
                        // Unbounded tail: L: Split(body, out); body; Jump L
                        let l = self.here();
                        let split = self.emit(Inst::Split(u32::MAX, u32::MAX));
                        let body = self.here();
                        self.compile_node(node);
                        self.emit(Inst::Jump(l));
                        let out = self.here();
                        if let Inst::Split(a, b) = &mut self.prog[split] {
                            if *greedy {
                                (*a, *b) = (body, out);
                            } else {
                                (*a, *b) = (out, body);
                            }
                        }
                    }
                    Some(m) => {
                        // (m - min) optional copies.
                        let mut splits = Vec::new();
                        for _ in *min..*m {
                            let s = self.emit(Inst::Split(u32::MAX, u32::MAX));
                            let body = self.here();
                            if let Inst::Split(a, b) = &mut self.prog[s] {
                                if *greedy {
                                    *a = body;
                                } else {
                                    *b = body;
                                }
                            }
                            splits.push(s);
                            self.compile_node(node);
                        }
                        let out = self.here();
                        for s in splits {
                            if let Inst::Split(a, b) = &mut self.prog[s] {
                                if *greedy {
                                    *b = out;
                                } else {
                                    *a = out;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser {
            chars: &chars,
            pos: 0,
            next_group: 0,
            classes: Vec::new(),
        };
        let alts = p.parse_alt()?;
        if p.pos < p.chars.len() {
            return Err("unmatched \\) in regexp".into());
        }
        if p.next_group > 20 {
            return Err("too many groups (max 20)".into());
        }
        let n_groups = p.next_group;
        let literal_prefix = extract_literal_prefix(&alts);
        // Computed once here, not per-`search()`-call — see the field
        // doc comment.
        let mut lp_chars = literal_prefix.chars();
        let literal_prefix_char = match (lp_chars.next(), lp_chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        let mut b = ProgBuilder { prog: Vec::new() };
        b.emit(Inst::Save(0));
        b.compile_alts(&alts);
        b.emit(Inst::Save(1));
        b.emit(Inst::Match);
        Ok(Regex {
            prog: b.prog,
            classes: p.classes,
            n_groups,
            literal_prefix,
            literal_prefix_char,
        })
    }

    /// Leftmost match at or after byte offset `start`. Returns capture
    /// positions in BYTES: `caps[0]` is the whole match, `caps[n]` group
    /// n. `start` must be on a char boundary of `hay` (callers always
    /// derive it from a char position via `char_to_byte` or a previous
    /// match's own byte position).
    ///
    /// When the pattern has a usable literal prefix (see
    /// `extract_literal_prefix`), skip straight to positions where that
    /// prefix actually occurs (via `str::find`, i.e. libstd's Two-Way
    /// substring search) instead of invoking `match_at` — and its
    /// per-call heap allocation plus full VM run — at every single char
    /// position. Any real match's start position must have the prefix
    /// bytes immediately following it (zero-width assertions interleaved
    /// among the prefix's characters consume no bytes, so the byte
    /// sequence itself is contiguous), so skipping non-prefix positions
    /// never skips a real match.
    ///
    /// Self-synchronization argument (M43 design §3.4, why this never
    /// produces a "false candidate" mid-character): `literal_prefix` is
    /// built by pushing real `char`s (`extract_literal_prefix`), so its
    /// encoded first byte is either an ASCII byte (< 0x80) or a UTF-8
    /// lead byte (0xC2..=0xF4) — by construction, never a continuation
    /// byte (0x80..=0xBF). `hay` is also valid UTF-8, so *any* position
    /// in it whose byte equals the prefix's first byte cannot itself be
    /// a continuation byte's value — meaning that position cannot be a
    /// continuation-byte position, and (since `hay` is valid UTF-8) is
    /// therefore necessarily a char boundary. So every candidate
    /// `str::find` returns is already char-boundary-aligned; no explicit
    /// boundary check is needed here (differential-tested against a
    /// boundary-respecting naive scan — see `mod tests` below, and the
    /// dedicated `prefix_skip_never_matches_mid_character` test).
    ///
    /// `match_at` is unchanged and still does the full, authoritative
    /// match (including the assertions) once a candidate position is
    /// found.
    pub fn search(&self, hay: &str, start: usize) -> MatchOutcome {
        // One `Scratch` for the whole call — reused (not reallocated)
        // across every internal `match_at_with` retry below, both for
        // performance (M43 already established the "no per-candidate
        // heap allocation" rule for `saves`; M81 extends it to
        // `frames`/`journal`) and because `steps` MUST accumulate across
        // retries for the cumulative step budget to mean anything (see
        // module doc). A caller that itself loops over MULTIPLE
        // `search` calls against the SAME haystack (`replace_all`
        // below, and `core`'s `re-search-backward` — M81 R5) must NOT
        // go through this wrapper: a fresh `Scratch` per call means a
        // fresh budget per call, defeating the whole "bounded across
        // the whole operation" point for exactly the `:s///g` shape
        // that motivated this milestone. Those callers use
        // `search_with` directly, sharing one `Scratch` (and so one
        // cumulative step counter) across their own outer loop.
        let mut scratch = Scratch::new(hay.len());
        self.search_with(hay, start, &mut scratch)
    }

    /// Same as `search`, but the caller supplies (and keeps reusing)
    /// the `Scratch` — see `search`'s doc comment on why a caller that
    /// itself retries `search` in an outer loop against the same
    /// haystack (M81 R5: `replace_all`'s `:s///g`-style loop below, and
    /// `crates/core/src/builtins/editing.rs`'s `re-search-backward`)
    /// MUST call this instead of `search`, sharing one `Scratch` (and
    /// its cumulative `steps` counter) across every outer-loop
    /// iteration, not just within a single call.
    pub fn search_with(&self, hay: &str, start: usize, scratch: &mut Scratch) -> MatchOutcome {
        if start > hay.len() {
            return Ok(None);
        }
        if self.literal_prefix.is_empty() {
            let mut at = start;
            loop {
                if let Some(caps) = self.match_at_with(hay, at, scratch)? {
                    return Ok(Some(caps));
                }
                if at >= hay.len() {
                    return Ok(None);
                }
                at += char_len_at(hay, at);
            }
        }
        if let Some(c) = self.literal_prefix_char {
            // Single-char prefix (M43 period 3): `str::find` with a
            // `char` pattern is measurably faster (~15-20x) than the
            // same search with a length-1 `&str` pattern — see
            // `literal_prefix_char`'s doc comment. Same loop shape as
            // the general case below, just a `char` needle throughout.
            let mut at = start;
            loop {
                let Some(off) = hay.get(at..).and_then(|s| s.find(c)) else {
                    return Ok(None);
                };
                let cand = at + off;
                if let Some(caps) = self.match_at_with(hay, cand, scratch)? {
                    return Ok(Some(caps));
                }
                at = cand + char_len_at(hay, cand);
            }
        }
        let prefix = self.literal_prefix.as_str();
        let mut at = start;
        loop {
            let Some(off) = hay.get(at..).and_then(|s| s.find(prefix)) else {
                return Ok(None);
            };
            let cand = at + off;
            if let Some(caps) = self.match_at_with(hay, cand, scratch)? {
                return Ok(Some(caps));
            }
            // Advance past exactly one CHAR (not one byte, and not the
            // whole prefix): the prefix may occur again starting one
            // character later (e.g. pattern "aa" against "aaaa") — same
            // rationale as the pre-M43 char engine's `+1`.
            at = cand + char_len_at(hay, cand);
        }
    }

    /// Anchored match attempt at exactly byte offset `at`, which must be
    /// on a char boundary of `hay`. Public entry point: allocates its
    /// own single-call `Scratch` (budgeted against just this one call —
    /// there's no outer retry loop to accumulate across, unlike
    /// `search`).
    pub fn match_at(&self, hay: &str, at: usize) -> MatchOutcome {
        let mut scratch = Scratch::new(hay.len());
        self.match_at_with(hay, at, &mut scratch)
    }

    /// Same as `match_at`, but takes caller-owned scratch state so
    /// `search` can reuse one allocation (and one cumulative step
    /// counter) across every candidate position it tries.
    fn match_at_with(&self, hay: &str, at: usize, scratch: &mut Scratch) -> MatchOutcome {
        debug_assert!(
            hay.is_char_boundary(at),
            "Regex::match_at: `at` {at} is not on a char boundary of the {}-byte haystack",
            hay.len()
        );
        scratch.saves.clear();
        scratch.saves.resize((self.n_groups + 1) * 2, None::<usize>);
        let matched = self.run(0, at, hay, scratch)?;
        // M81 R3 fix: the budget is only ever COMPARED at the VM's two
        // back edges (`Split`/`Jump`) and here, right before returning
        // to the caller — NOT on every dispatch step (see `run`'s
        // top-of-loop comment). This is the check that catches a huge
        // Split/Jump-free straight-line run (`\{n\}`'s mandatory `min`
        // copies, `compile_node`'s `Repeat` arm) that never touched a
        // back edge at all: without it, `scratch.steps` would have
        // accumulated real cost this whole call but never been
        // compared against `max_steps` once.
        if scratch.steps > scratch.max_steps {
            return Err(RegexLimit);
        }
        if matched {
            let caps = (0..=self.n_groups)
                .map(|g| match (scratch.saves[g * 2], scratch.saves[g * 2 + 1]) {
                    (Some(s), Some(e)) => Some((s, e)),
                    _ => None,
                })
                .collect();
            Ok(Some(caps))
        } else {
            Ok(None)
        }
    }

    /// Pop the top backtrack frame (if any), undoing every `Save` the
    /// journal recorded since it was pushed, and return the `(pc, pos)`
    /// to resume at. `None` means the frame stack is empty — the whole
    /// `match_at` attempt has failed.
    ///
    /// Equivalence with the old recursive engine (see module doc for why
    /// this replaced it): the old `run()` had `Save` recurse
    /// unconditionally, so failure unwound back through every `Save`
    /// frame between the failure point and the nearest enclosing `Split`
    /// frame, restoring each one's `saves[slot]` on the way — i.e. it
    /// undid exactly the `Save`s performed since that `Split` was
    /// entered. Here, every `Save` executed while `frames` is non-empty
    /// pushes its old value onto `journal` (skipped when `frames` is
    /// empty, since with no choice point above it that `Save` can never
    /// be backtracked past — see `run`'s `Inst::Save` arm), so
    /// `journal[frame.undo_len..]` is by construction exactly the set of
    /// `Save`s performed after `frame` was pushed, in execution order.
    /// Replaying it in reverse (`pop`, most-recent-first) undoes them in
    /// the same order the old engine's unwind would have, restoring the
    /// same final `saves` contents.
    fn backtrack(scratch: &mut Scratch) -> Option<(usize, usize)> {
        let frame = scratch.frames.pop()?;
        while scratch.journal.len() > frame.undo_len as usize {
            let (slot, old) = scratch.journal.pop().expect("just checked len > undo_len");
            scratch.saves[slot as usize] = old;
        }
        Some((frame.pc as usize, frame.pos as usize))
    }

    /// Explicit-stack backtracking VM (M81 — see module doc; was genuine
    /// Rust recursion before this milestone). `scratch.frames`/`.journal`
    /// are cleared at the start of each call (this function is always
    /// entered fresh, at `pc == 0`, from `match_at_with`) — only
    /// `scratch.saves` (cleared by the caller) and `scratch.steps`
    /// (intentionally NOT reset here — see `Scratch::steps`'s doc
    /// comment) cross calls.
    fn run(
        &self,
        pc: usize,
        pos: usize,
        hay: &str,
        scratch: &mut Scratch,
    ) -> Result<bool, RegexLimit> {
        scratch.frames.clear();
        scratch.journal.clear();
        let bytes = hay.as_bytes();
        let mut pc = pc;
        let mut pos = pos;
        macro_rules! fail {
            () => {{
                match Self::backtrack(scratch) {
                    Some((npc, npos)) => {
                        pc = npc;
                        pos = npos;
                        continue;
                    }
                    None => return Ok(false),
                }
            }};
        }
        loop {
            // M81 R3 fix: count EVERY dispatch step (plain `+= 1`, no
            // comparison here) — see `Scratch::steps`'s doc comment for
            // why counting only at Split/Jump (the first cut of this
            // design) let `\{n\}`'s MANDATORY `min` copies (compiled to
            // a flat, Split/Jump-free straight-line run by
            // `compile_node`'s `Repeat` arm) rack up unbounded real cost
            // while the counter stayed at 0. The budget is still only
            // ever COMPARED against at the two back edges
            // (`Inst::Split`/`Inst::Jump`) and once more at
            // `match_at_with`'s return (see there) — not on every
            // instruction — so the hot straight-line path pays one
            // `u64` increment per step and nothing else.
            scratch.steps += 1;
            // M43-2 review hardening: the VM's load-bearing invariant is
            // that `pos` only ever rests on a char boundary (every
            // advancing instruction moves by a whole character). A future
            // edit to any one arm's step size must panic here in debug
            // builds instead of silently misaligning downstream caps.
            debug_assert!(hay.is_char_boundary(pos));
            match self.prog[pc] {
                Inst::Byte(b) => {
                    if bytes.get(pos) == Some(&b) {
                        pos += 1;
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::Chars(buf, len) => {
                    let len = len as usize;
                    if bytes.get(pos..pos + len) == Some(&buf[..len]) {
                        pos += len;
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::Any => {
                    // `.` matches anything except newline (elisp rule).
                    // Newline is single-byte ASCII, so this is a plain
                    // byte compare — no decode needed to reject it.
                    match bytes.get(pos) {
                        Some(&b) if b != b'\n' => {
                            pos += if b < 0x80 { 1 } else { utf8_lead_len(b) };
                            pc += 1;
                        }
                        _ => fail!(),
                    }
                }
                Inst::Class(i) => match char_at_byte(hay, pos) {
                    Some((c, len)) if self.classes[i as usize].matches(c) => {
                        pos += len;
                        pc += 1;
                    }
                    _ => fail!(),
                },
                Inst::WordChar(want) => match char_at_byte(hay, pos) {
                    Some((c, len)) if is_word(c) == want => {
                        pos += len;
                        pc += 1;
                    }
                    _ => fail!(),
                },
                Inst::Space(want) => match char_at_byte(hay, pos) {
                    Some((c, len)) if c.is_whitespace() == want => {
                        pos += len;
                        pc += 1;
                    }
                    _ => fail!(),
                },
                Inst::LineStart => {
                    // Newline is single-byte ASCII, so checking the raw
                    // byte at `pos - 1` is safe regardless of char-
                    // boundary alignment concerns: a byte can only equal
                    // b'\n' by being that exact complete ASCII char (see
                    // the module doc's self-sync argument).
                    if pos == 0 || bytes.get(pos - 1) == Some(&b'\n') {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::LineEnd => {
                    if pos == bytes.len() || bytes.get(pos) == Some(&b'\n') {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::StrStart => {
                    if pos == 0 {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::StrEnd => {
                    if pos == bytes.len() {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::WordBoundary(want) => {
                    let before = char_before_byte(hay, pos).map(is_word).unwrap_or(false);
                    let after = char_at_byte(hay, pos)
                        .map(|(c, _)| is_word(c))
                        .unwrap_or(false);
                    if (before != after) == want {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::WordStart => {
                    let before = char_before_byte(hay, pos).map(is_word).unwrap_or(false);
                    let after = char_at_byte(hay, pos)
                        .map(|(c, _)| is_word(c))
                        .unwrap_or(false);
                    if !before && after {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::WordEnd => {
                    let before = char_before_byte(hay, pos).map(is_word).unwrap_or(false);
                    let after = char_at_byte(hay, pos)
                        .map(|(c, _)| is_word(c))
                        .unwrap_or(false);
                    if before && !after {
                        pc += 1;
                    } else {
                        fail!();
                    }
                }
                Inst::Save(slot) => {
                    // Only journal (and thus only ever restore) this
                    // write when there's a choice point above it to
                    // backtrack to — with `frames` empty, nothing can
                    // ever unwind past this point, so recording its old
                    // value would be pure waste (see module doc: this is
                    // exactly the "Save is not a choice point" fix).
                    if !scratch.frames.is_empty() {
                        // M81 R10 fix: an unbounded quantifier wrapping
                        // a capture group leaves `frames` non-empty for
                        // the rest of a successful match (its own
                        // `Split` is pushed and never popped on the
                        // success path — see `JOURNAL_BUDGET_PER_BYTE`'s
                        // doc comment) — so EVERY `Save` from then on
                        // lands in this branch and journals, even though
                        // that frame will never actually be backtracked
                        // into. Unlike `frames`/`steps`, nothing else
                        // bounds how large `journal` can grow, so it
                        // needs its own check here, right before the
                        // push that would grow it further.
                        if scratch.journal.len() >= scratch.max_journal {
                            return Err(RegexLimit);
                        }
                        scratch.journal.push((slot, scratch.saves[slot as usize]));
                    }
                    scratch.saves[slot as usize] = Some(pos);
                    pc += 1;
                }
                Inst::Split(a, b) => {
                    // Budget COMPARED here and at `Jump` (M81 R3 fix —
                    // see the top-of-loop comment for why counting
                    // happens on every instruction now): these two are
                    // the VM's "back edges", the only instructions that
                    // can make a `match_at` call visit more than
                    // `prog.len()` instructions net-forward, i.e. the
                    // only ones that can turn a bounded-by-construction
                    // straight-line run into unbounded work. Comparing
                    // on every dispatch step (not just here) measured
                    // ~1.9x slower on the adversarial-but-legitimate
                    // `.*199999`/8.49MB case for no extra safety this
                    // milestone needs: a straight-line stretch between
                    // back edges is itself bounded by `prog.len()` +
                    // `hay.len()` (structurally can't loop without a
                    // Split/Jump), so the ADDITIONAL check at
                    // `match_at_with`'s return (see there) is enough to
                    // catch a huge straight-line stretch (M81 R3's
                    // `\{n\}` mandatory-copies case) without comparing
                    // on every single instruction.
                    if scratch.steps > scratch.max_steps
                        || scratch.frames.len() >= scratch.max_frames
                    {
                        return Err(RegexLimit);
                    }
                    scratch.frames.push(Frame {
                        pc: b,
                        pos: pos as u32,
                        undo_len: scratch.journal.len() as u32,
                    });
                    pc = a as usize;
                }
                Inst::Jump(t) => {
                    if scratch.steps > scratch.max_steps {
                        return Err(RegexLimit);
                    }
                    pc = t as usize;
                }
                Inst::Match => return Ok(true),
            }
        }
    }
}

// --- Interp-level match data + operations used by the builtins ---

/// Last successful match: capture positions plus the text that was
/// searched (so `match-string` works without re-supplying the string —
/// buffer searches store a buffer snapshot here too).
///
/// `caps` stays CHAR-indexed (M43 period 2 does not change elisp's
/// position semantics: `match-beginning`/`match-end` must keep
/// reporting the same numbers as before). `caps_bytes` carries the
/// engine's native byte positions into the *same* `target` alongside
/// it, so `match-string` can slice `target` directly (O(1) plus the
/// copy) instead of re-walking `target` char by char — see
/// `builtins::misc::match-string`.
///
/// `target` is `Rc<str>` (M43 period 2; was `Rc<Vec<char>>`): the
/// engine now works over `&str` directly (buffer searches hand in
/// `Buffer::search_text`'s cached snapshot, itself now an `Rc<str>` —
/// two gap-split `memcpy`s to build, no per-char re-encoding).
#[derive(Default)]
pub struct MatchData {
    pub caps: Vec<Option<(usize, usize)>>,
    pub caps_bytes: Vec<Option<(usize, usize)>>,
    pub target: Option<Rc<str>>,
    /// Offset added to capture positions (buffer searches report
    /// buffer positions; string searches use 0).
    pub offset: i64,
}

/// Convert byte-offset captures into char-offset captures with a single
/// pass over `hay` — the same "collect the distinct offsets, sort them,
/// walk once" shape `highlight.rs::extract_spans` already uses for its
/// overlay byte→char conversion. Used by `string_match`/`replace_all`,
/// which (unlike the buffer-search builtins in `core`) have no
/// persistent `GapBuffer` anchor to amortize a conversion against —
/// buffer searches instead call `GapBuffer::byte_to_char` per capture,
/// which *does* have that amortization (see M43 design §3.4).
fn caps_bytes_to_chars(
    hay: &str,
    caps_bytes: &[Option<(usize, usize)>],
) -> Vec<Option<(usize, usize)>> {
    let mut offsets: Vec<usize> = Vec::with_capacity(caps_bytes.len() * 2);
    for (s, e) in caps_bytes.iter().flatten() {
        offsets.push(*s);
        offsets.push(*e);
    }
    offsets.sort_unstable();
    offsets.dedup();
    let mut map: HashMap<usize, usize> = HashMap::with_capacity(offsets.len());
    let mut oi = 0usize;
    let mut chars = 0usize;
    for (byte_pos, _) in hay.char_indices() {
        while oi < offsets.len() && offsets[oi] == byte_pos {
            map.insert(offsets[oi], chars);
            oi += 1;
        }
        chars += 1;
    }
    // Any remaining offsets must equal hay.len() (a capture ending at
    // the very end of the string — char_indices only yields the start
    // of each char, never one-past-the-end).
    while oi < offsets.len() {
        debug_assert_eq!(
            offsets[oi],
            hay.len(),
            "leftover capture offset is not end-of-string: a byte cap \
             landed off every char boundary"
        );
        map.insert(offsets[oi], chars);
        oi += 1;
    }
    caps_bytes
        .iter()
        .map(|c| c.map(|(s, e)| (map[&s], map[&e])))
        .collect()
}

/// Run REGEXP against STRING from START (a CHAR offset — elisp's
/// `string-match` START argument, like all elisp string positions, is
/// char-indexed); set match data on success.
pub fn string_match(
    interp: &mut Interp,
    pattern: &str,
    target: &str,
    start: usize,
    set_data: bool,
) -> Result<Option<usize>, Flow> {
    let re = compile(interp, pattern)?;
    let char_len = target.chars().count();
    if start > char_len {
        return Err(interp.error(format!(
            "args out of range: start {} > length {}",
            start, char_len
        )));
    }
    // Char start → byte start: a single bounded scan (at most `start`
    // chars), not a full-string collect — cheaper than the pre-M43
    // `target.chars().collect::<Vec<char>>()` this replaces, not just
    // different (M43 design §3.4: "strictly cheaper, never worse").
    let byte_start = if start == char_len {
        target.len()
    } else {
        target
            .char_indices()
            .nth(start)
            .map(|(b, _)| b)
            .unwrap_or(target.len())
    };
    let hay: Rc<str> = Rc::from(target);
    let result = re
        .search(&hay, byte_start)
        .map_err(|RegexLimit| interp.regexp_too_complex())?;
    match result {
        Some(caps_bytes) => {
            let caps = caps_bytes_to_chars(&hay, &caps_bytes);
            let begin = caps[0].map(|(s, _)| s);
            if set_data {
                interp.match_data = MatchData {
                    caps,
                    caps_bytes,
                    target: Some(hay),
                    offset: 0,
                };
            }
            Ok(begin)
        }
        None => Ok(None),
    }
}

/// Compiled-pattern cache: patterns like org-mode's fontify regexps get
/// recompiled on every call site's every invocation otherwise, which
/// dominates cost on large buffers (recompiling is O(pattern length) but
/// happens once per line/keystroke rather than once per pattern). Keyed
/// on the raw pattern string; capped so a caller that generates lots of
/// one-off patterns can't grow this unboundedly — full-clear on overflow
/// rather than real LRU, which is enough for the realistic access
/// pattern (a handful of hot, repeated patterns from font-lock/isearch).
const REGEX_CACHE_CAP: usize = 256;

pub fn compile(interp: &mut Interp, pattern: &str) -> Result<Rc<Regex>, Flow> {
    if let Some(re) = interp.regex_cache.get(pattern) {
        return Ok(Rc::clone(re));
    }
    let re =
        Rc::new(Regex::new(pattern).map_err(|e| interp.error(format!("invalid regexp: {}", e)))?);
    if interp.regex_cache.len() >= REGEX_CACHE_CAP {
        interp.regex_cache.clear();
    }
    interp
        .regex_cache
        .insert(pattern.to_string(), Rc::clone(&re));
    Ok(re)
}

/// `replace-regexp-in-string`: replace every match of `re` in `target`
/// with the template `rep`, where `\N` inserts group N, `\&` the whole
/// match, and `\\` a literal backslash.
pub fn replace_all(
    interp: &mut Interp,
    pattern: &str,
    rep: &str,
    target: &str,
) -> Result<String, Flow> {
    let re = compile(interp, pattern)?;
    let mut out = String::new();
    let mut pos = 0usize; // byte offset into `target`
                          // M81 R5: ONE `Scratch` shared across the whole `:s///g`-shaped
                          // loop below, via `search_with` — NOT the public `search` (which
                          // would allocate a fresh `Scratch`, and so a fresh budget, on every
                          // iteration). Reviewer's point: `:s///g` replacing many individually
                          // cheap matches is exactly the shape this milestone exists to
                          // protect, and a per-iteration-reset budget can never see the
                          // cumulative cost across a match-heavy replace-all, only ever one
                          // match's worth at a time.
    let mut scratch = Scratch::new(target.len());
    while pos <= target.len() {
        let found = re
            .search_with(target, pos, &mut scratch)
            .map_err(|RegexLimit| interp.regexp_too_complex())?;
        let Some(caps) = found else {
            break;
        };
        let (ms, me) = caps[0].expect("match without group 0");
        out.push_str(&target[pos..ms]);
        // Expand the template.
        let repc: Vec<char> = rep.chars().collect();
        let mut i = 0;
        while i < repc.len() {
            if repc[i] == '\\' && i + 1 < repc.len() {
                let d = repc[i + 1];
                i += 2;
                match d {
                    '&' => {
                        out.push_str(&target[ms..me]);
                    }
                    '1'..='9' => {
                        let g = d.to_digit(10).unwrap() as usize;
                        if let Some(Some((gs, ge))) = caps.get(g) {
                            out.push_str(&target[*gs..*ge]);
                        }
                    }
                    '\\' => out.push('\\'),
                    other => out.push(other),
                }
            } else {
                out.push(repc[i]);
                i += 1;
            }
        }
        // Zero-width match: emit one char (not byte) and advance past
        // it to avoid looping.
        if me == ms {
            if ms < target.len() {
                let ch = target[ms..].chars().next().expect("ms < target.len()");
                out.push(ch);
                pos = ms + ch.len_utf8();
            } else {
                pos = ms + 1; // ends the loop: pos > target.len()
            }
        } else {
            pos = me;
        }
    }
    if pos < target.len() {
        out.push_str(&target[pos..]);
    }
    Ok(out)
}

/// `regexp-quote`: escape everything special in the elisp dialect.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '.' | '*' | '+' | '?' | '[' | ']' | '^' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self-contained copy of the pre-M43 `&[char]`-indexed engine,
    /// kept only as a differential-testing oracle for the byte-indexed
    /// `Regex` above (house convention: `gapbuffer.rs`'s `CharGapBuffer`
    /// and P2.2's naive oracle used the same "freeze the old
    /// implementation as an oracle" trick). Reuses the (representation-
    /// agnostic) `Node`/`Parser`/`ClassSpec`/`is_word` from the outer
    /// module — parsing didn't change in M43, only the compiled
    /// instruction set and the VM that walks it — so only the compiler
    /// and VM are duplicated here.
    mod legacy_char_engine {
        use super::*;

        #[derive(Clone, Copy, Debug)]
        enum Inst {
            Char(char),
            Any,
            Class(u16),
            Split(u32, u32),
            Jump(u32),
            Save(u8),
            LineStart,
            LineEnd,
            StrStart,
            StrEnd,
            WordBoundary(bool),
            WordStart,
            WordEnd,
            WordChar(bool),
            Space(bool),
            Match,
        }

        struct ProgBuilder {
            prog: Vec<Inst>,
        }

        impl ProgBuilder {
            fn emit(&mut self, i: Inst) -> usize {
                self.prog.push(i);
                self.prog.len() - 1
            }
            fn here(&self) -> u32 {
                self.prog.len() as u32
            }
            fn patch(&mut self, at: usize, to: u32) {
                match &mut self.prog[at] {
                    Inst::Split(_, b) if *b == u32::MAX => {
                        if let Inst::Split(_, b) = &mut self.prog[at] {
                            *b = to;
                        }
                    }
                    Inst::Jump(t) => *t = to,
                    Inst::Split(a, _) if *a == u32::MAX => {
                        if let Inst::Split(a, _) = &mut self.prog[at] {
                            *a = to;
                        }
                    }
                    _ => unreachable!("patch target is not a jump"),
                }
            }

            fn compile_alts(&mut self, alts: &[Vec<Node>]) {
                let mut end_jumps = Vec::new();
                for (i, seq) in alts.iter().enumerate() {
                    if i + 1 < alts.len() {
                        let split = self.emit(Inst::Split(0, u32::MAX));
                        let body = self.here();
                        if let Inst::Split(a, _) = &mut self.prog[split] {
                            *a = body;
                        }
                        self.compile_seq(seq);
                        end_jumps.push(self.emit(Inst::Jump(u32::MAX)));
                        let next = self.here();
                        self.patch(split, next);
                    } else {
                        self.compile_seq(seq);
                    }
                }
                let end = self.here();
                for j in end_jumps {
                    self.patch(j, end);
                }
            }

            fn compile_seq(&mut self, seq: &[Node]) {
                for n in seq {
                    self.compile_node(n);
                }
            }

            fn compile_node(&mut self, node: &Node) {
                match node {
                    Node::Char(c) => {
                        self.emit(Inst::Char(*c));
                    }
                    Node::Any => {
                        self.emit(Inst::Any);
                    }
                    Node::Class(i) => {
                        self.emit(Inst::Class(*i as u16));
                    }
                    Node::LineStart => {
                        self.emit(Inst::LineStart);
                    }
                    Node::LineEnd => {
                        self.emit(Inst::LineEnd);
                    }
                    Node::StrStart => {
                        self.emit(Inst::StrStart);
                    }
                    Node::StrEnd => {
                        self.emit(Inst::StrEnd);
                    }
                    Node::WordBoundary(b) => {
                        self.emit(Inst::WordBoundary(*b));
                    }
                    Node::WordStart => {
                        self.emit(Inst::WordStart);
                    }
                    Node::WordEnd => {
                        self.emit(Inst::WordEnd);
                    }
                    Node::WordChar(b) => {
                        self.emit(Inst::WordChar(*b));
                    }
                    Node::Space(b) => {
                        self.emit(Inst::Space(*b));
                    }
                    Node::Group(num, alts) => {
                        if let Some(n) = num {
                            self.emit(Inst::Save((n * 2) as u8));
                            self.compile_alts(alts);
                            self.emit(Inst::Save((n * 2 + 1) as u8));
                        } else {
                            self.compile_alts(alts);
                        }
                    }
                    Node::Repeat {
                        node,
                        min,
                        max,
                        greedy,
                    } => {
                        for _ in 0..*min {
                            self.compile_node(node);
                        }
                        match max {
                            None => {
                                let l = self.here();
                                let split = self.emit(Inst::Split(u32::MAX, u32::MAX));
                                let body = self.here();
                                self.compile_node(node);
                                self.emit(Inst::Jump(l));
                                let out = self.here();
                                if let Inst::Split(a, b) = &mut self.prog[split] {
                                    if *greedy {
                                        (*a, *b) = (body, out);
                                    } else {
                                        (*a, *b) = (out, body);
                                    }
                                }
                            }
                            Some(m) => {
                                let mut splits = Vec::new();
                                for _ in *min..*m {
                                    let s = self.emit(Inst::Split(u32::MAX, u32::MAX));
                                    let body = self.here();
                                    if let Inst::Split(a, b) = &mut self.prog[s] {
                                        if *greedy {
                                            *a = body;
                                        } else {
                                            *b = body;
                                        }
                                    }
                                    splits.push(s);
                                    self.compile_node(node);
                                }
                                let out = self.here();
                                for s in splits {
                                    if let Inst::Split(a, b) = &mut self.prog[s] {
                                        if *greedy {
                                            *b = out;
                                        } else {
                                            *a = out;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        fn extract_literal_prefix_chars(alts: &[Vec<Node>]) -> Vec<char> {
            if alts.len() != 1 {
                return Vec::new();
            }
            let mut prefix = Vec::new();
            for node in &alts[0] {
                match node {
                    Node::Char(c) => prefix.push(*c),
                    Node::LineStart
                    | Node::LineEnd
                    | Node::StrStart
                    | Node::StrEnd
                    | Node::WordBoundary(_)
                    | Node::WordStart
                    | Node::WordEnd => {}
                    _ => break,
                }
            }
            prefix
        }

        fn prefix_matches_at(hay: &[char], at: usize, prefix: &[char]) -> bool {
            if at + prefix.len() > hay.len() {
                return false;
            }
            for (i, &pc) in prefix.iter().enumerate() {
                if hay[at + i] != pc {
                    return false;
                }
            }
            true
        }

        pub struct LegacyRegex {
            prog: Vec<Inst>,
            classes: Vec<ClassSpec>,
            n_groups: usize,
            literal_prefix: Vec<char>,
        }

        impl LegacyRegex {
            pub fn new(pattern: &str) -> Result<LegacyRegex, String> {
                let chars: Vec<char> = pattern.chars().collect();
                let mut p = Parser {
                    chars: &chars,
                    pos: 0,
                    next_group: 0,
                    classes: Vec::new(),
                };
                let alts = p.parse_alt()?;
                if p.pos < p.chars.len() {
                    return Err("unmatched \\) in regexp".into());
                }
                if p.next_group > 20 {
                    return Err("too many groups (max 20)".into());
                }
                let n_groups = p.next_group;
                let literal_prefix = extract_literal_prefix_chars(&alts);
                let mut b = ProgBuilder { prog: Vec::new() };
                b.emit(Inst::Save(0));
                b.compile_alts(&alts);
                b.emit(Inst::Save(1));
                b.emit(Inst::Match);
                Ok(LegacyRegex {
                    prog: b.prog,
                    classes: p.classes,
                    n_groups,
                    literal_prefix,
                })
            }

            pub fn search(
                &self,
                hay: &[char],
                start: usize,
            ) -> Option<Vec<Option<(usize, usize)>>> {
                if self.literal_prefix.is_empty() {
                    for at in start..=hay.len() {
                        if let Some(caps) = self.match_at(hay, at) {
                            return Some(caps);
                        }
                    }
                    return None;
                }
                if start > hay.len() {
                    return None;
                }
                let prefix = &self.literal_prefix[..];
                let first = prefix[0];
                let mut at = start;
                while let Some(off) = hay[at..].iter().position(|&c| c == first) {
                    let cand = at + off;
                    if prefix_matches_at(hay, cand, prefix) {
                        if let Some(caps) = self.match_at(hay, cand) {
                            return Some(caps);
                        }
                    }
                    at = cand + 1;
                }
                None
            }

            pub fn match_at(&self, hay: &[char], at: usize) -> Option<Vec<Option<(usize, usize)>>> {
                let mut saves: Vec<Option<usize>> = vec![None; (self.n_groups + 1) * 2];
                if self.run(0, at, hay, &mut saves) {
                    let caps = (0..=self.n_groups)
                        .map(|g| match (saves[g * 2], saves[g * 2 + 1]) {
                            (Some(s), Some(e)) => Some((s, e)),
                            _ => None,
                        })
                        .collect();
                    Some(caps)
                } else {
                    None
                }
            }

            fn run(
                &self,
                mut pc: usize,
                mut pos: usize,
                hay: &[char],
                saves: &mut Vec<Option<usize>>,
            ) -> bool {
                loop {
                    match self.prog[pc] {
                        Inst::Char(c) => {
                            if hay.get(pos) == Some(&c) {
                                pos += 1;
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::Any => match hay.get(pos) {
                            Some(&c) if c != '\n' => {
                                pos += 1;
                                pc += 1;
                            }
                            _ => return false,
                        },
                        Inst::Class(i) => match hay.get(pos) {
                            Some(&c) if self.classes[i as usize].matches(c) => {
                                pos += 1;
                                pc += 1;
                            }
                            _ => return false,
                        },
                        Inst::WordChar(want) => match hay.get(pos) {
                            Some(&c) if is_word(c) == want => {
                                pos += 1;
                                pc += 1;
                            }
                            _ => return false,
                        },
                        Inst::Space(want) => match hay.get(pos) {
                            Some(&c) if c.is_whitespace() == want => {
                                pos += 1;
                                pc += 1;
                            }
                            _ => return false,
                        },
                        Inst::LineStart => {
                            if pos == 0 || hay.get(pos - 1) == Some(&'\n') {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::LineEnd => {
                            if pos == hay.len() || hay.get(pos) == Some(&'\n') {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::StrStart => {
                            if pos == 0 {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::StrEnd => {
                            if pos == hay.len() {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::WordBoundary(want) => {
                            let before = pos > 0 && is_word(hay[pos - 1]);
                            let after = pos < hay.len() && is_word(hay[pos]);
                            if (before != after) == want {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::WordStart => {
                            let before = pos > 0 && is_word(hay[pos - 1]);
                            let after = pos < hay.len() && is_word(hay[pos]);
                            if !before && after {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::WordEnd => {
                            let before = pos > 0 && is_word(hay[pos - 1]);
                            let after = pos < hay.len() && is_word(hay[pos]);
                            if before && !after {
                                pc += 1;
                            } else {
                                return false;
                            }
                        }
                        Inst::Save(slot) => {
                            let old = saves[slot as usize];
                            saves[slot as usize] = Some(pos);
                            if self.run(pc + 1, pos, hay, saves) {
                                return true;
                            }
                            saves[slot as usize] = old;
                            return false;
                        }
                        Inst::Split(a, b) => {
                            if self.run(a as usize, pos, hay, saves) {
                                return true;
                            }
                            pc = b as usize;
                        }
                        Inst::Jump(t) => pc = t as usize,
                        Inst::Match => return true,
                    }
                }
            }
        }
    }
    use legacy_char_engine::LegacyRegex;

    /// Tiny deterministic PRNG (xorshift64*), reproducible without a
    /// `rand` dependency — mirrors `regex_prefix_skip_tests.rs`'s.
    struct Rng(u64);
    impl Rng {
        fn new(seed: u64) -> Self {
            Rng(seed | 1)
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn next_range(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// Alphabet mixing ASCII with multi-byte (2/3-byte) CJK/Latin-Extended
    /// characters, so fuzzed haystacks/patterns exercise real byte != char
    /// divergence — the case the M43 byte-engine rewrite has to get right
    /// that a pure-ASCII fuzz corpus can't catch.
    const ALPHABET: &[char] = &['a', 'b', 'c', 'x', '中', '文', 'é', '一'];

    fn random_string(rng: &mut Rng, len: usize) -> String {
        (0..len)
            .map(|_| ALPHABET[rng.next_range(ALPHABET.len())])
            .collect()
    }

    fn random_pattern(rng: &mut Rng) -> String {
        // M81: widened from 9 to 12 choices to add lazy quantifiers,
        // `\{n,m\}`, and quantifiers wrapping a capture group — exactly
        // the shapes the new explicit-stack `run()` (undo journal on
        // `Save`, Split push-order for greedy/lazy) is most likely to
        // get wrong, and which the previous generator never produced.
        let choice = rng.next_range(12);
        match choice {
            0 => {
                let len = 1 + rng.next_range(4);
                random_string(rng, len) // plain literal run
            }
            1 => {
                let len = 1 + rng.next_range(3);
                format!("^{}", random_string(rng, len))
            }
            2 => {
                let len = 1 + rng.next_range(3);
                format!("{}\\b", random_string(rng, len))
            }
            3 => {
                let len = 1 + rng.next_range(3);
                format!(".{}", random_string(rng, len)) // no prefix
            }
            4 => {
                let len = 1 + rng.next_range(2);
                format!("[abc中文]+{}", random_string(rng, len)) // no prefix
            }
            5 => {
                let a = random_string(rng, 2);
                let b = random_string(rng, 1);
                format!("\\({}\\){}", a, b) // no prefix
            }
            6 => {
                let a = random_string(rng, 2);
                let b = random_string(rng, 2);
                format!("{}\\|{}", a, b) // no prefix (top-level alt)
            }
            7 => {
                let a = random_string(rng, 1);
                let b = random_string(rng, 2);
                format!("{}*{}", a, b) // leading repeat, no prefix
            }
            8 => {
                let len = 1 + rng.next_range(3);
                format!("\\<{}\\>", random_string(rng, len)) // word boundaries
            }
            9 => {
                // Lazy quantifiers (`*?`/`+?`/`??`) — exercise the
                // Split(a,b) push-order flip (`compile_node`'s
                // `!*greedy` branch): must try the SHORT match first.
                let a = random_string(rng, 1);
                let b = random_string(rng, 2);
                let q = ["*?", "+?", "??"][rng.next_range(3)];
                format!("{}{}{}", a, q, b) // no prefix
            }
            10 => {
                // `\{n,m\}` counted repetition, greedy or lazy.
                let a = random_string(rng, 1);
                let b = random_string(rng, 2);
                let n = rng.next_range(3);
                let m = n + rng.next_range(3);
                let lazy = if rng.next_range(2) == 0 { "" } else { "?" };
                format!("{}\\{{{},{}\\}}{}{}", a, n, m, lazy, b) // no prefix
            }
            _ => {
                // A quantifier wrapping a capture group — the case that
                // most directly exercises the M81 undo-journal fix
                // (`Save` inside a `Split`-driven loop, undone and
                // replayed on every backtrack iteration).
                let a = random_string(rng, 2);
                let b = random_string(rng, 1);
                let inner = if rng.next_range(2) == 0 {
                    a.clone() // \(ab\)*c
                } else {
                    let c = random_string(rng, 1);
                    format!("{}\\|{}", a, c) // \(a\|b\)+c
                };
                let q = ["*", "+", "*?", "+?"][rng.next_range(4)];
                format!("\\({}\\){}{}", inner, q, b) // no prefix
            }
        }
    }

    /// Convert a `LegacyRegex`-style char-indexed capture vec to bytes
    /// against `hay_str` (the same content as `hay_chars`, re-encoded),
    /// for comparison against the byte engine's native output.
    fn char_caps_to_bytes(
        hay_str: &str,
        caps: &[Option<(usize, usize)>],
    ) -> Vec<Option<(usize, usize)>> {
        let char_byte: Vec<usize> = hay_str
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(hay_str.len()))
            .collect();
        caps.iter()
            .map(|c| c.map(|(s, e)| (char_byte[s], char_byte[e])))
            .collect()
    }

    /// Every char-boundary byte offset in `s`, 0..=s.len() inclusive —
    /// the complete set of positions the byte engine is ever legitimately
    /// asked to start a search from.
    fn char_boundaries(s: &str) -> Vec<usize> {
        s.char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(s.len()))
            .collect()
    }

    /// Core differential check: the new byte-indexed `Regex` and the
    /// frozen `LegacyRegex` oracle must agree (after converting the
    /// oracle's char positions to bytes) at every char-boundary start
    /// position.
    fn assert_engines_agree(pattern: &str, haystack: &str) {
        let new_re = match Regex::new(pattern) {
            Ok(re) => re,
            Err(_) => return, // both engines share the same parser/error surface
        };
        let old_re = LegacyRegex::new(pattern)
            .expect("legacy engine failed on a pattern the new engine accepted");
        let hay_chars: Vec<char> = haystack.chars().collect();
        for start in char_boundaries(haystack) {
            let char_start = haystack[..start].chars().count();
            let got = new_re
                .search(haystack, start)
                .expect("haystack is short (<=20 chars, see module fuzz note) — must not hit the step/frame budget");
            let want_chars = old_re.search(&hay_chars, char_start);
            let want = want_chars.map(|caps| char_caps_to_bytes(haystack, &caps));
            assert_eq!(
                got, want,
                "pattern {pattern:?} haystack {haystack:?} start(byte)={start}: \
                 new={got:?} legacy(converted)={want:?}"
            );
        }
    }

    #[test]
    fn randomized_new_vs_legacy_engine_fixed_seed() {
        let mut rng = Rng::new(0xC0DE_F00D_1234_5678);
        let mut checked = 0;
        // M81: 200 -> 2000 rounds, alongside `random_pattern` widening to
        // cover lazy quantifiers / `\{n,m\}` / quantifier-wrapped capture
        // groups — this is the differential test that actually exercises
        // the new explicit-stack `run()`'s undo journal and Split
        // push-order against the frozen recursive oracle. Haystacks stay
        // <=~16 chars: `LegacyRegex`/`old_re` is itself recursive and
        // will stack-overflow on long inputs (see module doc + the
        // `random_string` length bound below) — that's a property of the
        // *oracle*, not something M81 needs to fix.
        for _ in 0..2000 {
            let pattern = random_pattern(&mut rng);
            let hay_len = rng.next_range(16);
            let haystack = random_string(&mut rng, hay_len);
            if Regex::new(&pattern).is_err() {
                continue;
            }
            assert_engines_agree(&pattern, &haystack);
            checked += 1;
        }
        assert!(
            checked > 1500,
            "expected most of the 2000 random patterns to compile, got {checked}"
        );
    }

    #[test]
    fn cjk_prefix_and_groups_match_legacy_engine() {
        assert_engines_agree("中文", "abc中文def中文");
        assert_engines_agree("^中", "中文\nabc\n中");
        assert_engines_agree("\\(中\\)\\(文\\)", "中文");
        assert_engines_agree("中\\{2\\}", "中中中");
        assert_engines_agree("[中文]+", "中文abc中");
    }

    #[test]
    fn word_boundary_multibyte_matches_legacy_engine() {
        // `is_word` treats any Unicode alphabetic char as a word
        // constituent, so a boundary can land right at a CJK/ASCII
        // seam — exactly the case that needs `char_before_byte`'s
        // backward decode to get right.
        assert_engines_agree("\\<中文\\>", "see 中文 here");
        assert_engines_agree("\\bfoo\\b", "中文foo中文 foo bar");
        assert_engines_agree("\\w+", "中文abc123中文");
        assert_engines_agree("\\W", "中,文");
        assert_engines_agree("é\\b", "café café123");
    }

    /// Direct construction of the M43 design's self-synchronization
    /// argument: a literal-prefix pattern whose ASCII prefix bytes could
    /// only ever spuriously "occur" inside a multi-byte character if
    /// UTF-8 weren't self-synchronizing — i.e. if a continuation byte
    /// could coincide with an ASCII byte value. It can't (continuation
    /// bytes are always in 0x80..=0xBF, ASCII is always < 0x80), so
    /// `search` must find only the genuine, char-boundary-aligned
    /// occurrence, never a false one from inside `中`/`文`'s encoded
    /// bytes. Differential-tested against `LegacyRegex` (which walks
    /// `char`s and structurally cannot ever see a mid-character
    /// position), so any accidental byte-alignment bug here would show
    /// up as a mismatch.
    #[test]
    fn prefix_skip_never_matches_mid_character() {
        // "line" repeated with CJK filler between/around occurrences,
        // including right up against multi-byte chars on both sides —
        // the shape most likely to expose an off-by-one in the
        // "advance past one char, not one byte" prefix-skip step.
        let haystack = "中line文linex中文line 199999文中line";
        assert_engines_agree("line 199999", haystack);
        assert_engines_agree("line", haystack);
        // A prefix pattern where the byte immediately following a CJK
        // char happens to be an ASCII letter that's also the prefix's
        // first char — the exact "does the skip land mid-character"
        // stress case.
        assert_engines_agree("a", "中a文a中中a");
        assert_engines_agree("ab", "中ab文中ab");
        // The case that actually distinguishes "advance past one CHAR"
        // from "advance past one BYTE" after a failed candidate: a
        // MULTI-BYTE literal prefix ("中"/"é") occurring twice, where
        // only the SECOND occurrence satisfies the trailing `\'`
        // string-end anchor — so `search` must fail at the first
        // candidate and retry. A byte-advance bug lands the retry
        // mid-character (not a char boundary), which — via the checked
        // `str::get` in `search` — aborts the whole search with `None`
        // instead of finding the real match. Confirmed as a genuine
        // mutation-catcher (not just by inspection): flipping
        // `search`'s retry step from `char_len_at(hay, cand)` to a
        // hardcoded `1` makes exactly these two assertions fail while
        // every ASCII-prefix case above keeps passing (ASCII chars are
        // 1 byte, so that mutation is a no-op for them) — this is the
        // case that closes that gap.
        assert_engines_agree("中\\'", "中x中");
        // Same shape, 2-byte-encoded char instead of 3-byte (exercises
        // `utf8_lead_len`'s 2-byte branch too).
        assert_engines_agree("é\\'", "éxé");
    }

    #[test]
    fn search_out_of_range_start_is_none_not_panic() {
        let re = Regex::new("abc").unwrap();
        assert_eq!(re.search("hello", 100), Ok(None));
        assert_eq!(re.search("", 5), Ok(None));
        assert_eq!(re.search("中文", 100), Ok(None));
    }

    /// M43 period 3: explicit, hand-computed byte-offset assertions for a
    /// single-char literal-prefix pattern — one ASCII, one CJK — pinning
    /// down that `literal_prefix_char` actually gets set and dispatched
    /// (`search`'s `str::find(char)` branch) for both, independent of
    /// the random-pattern fuzz's RNG luck (`random_pattern`'s `choice ==
    /// 0` branch can produce a 1-char literal, but doesn't guarantee it
    /// on every run).
    #[test]
    fn single_char_literal_prefix_dispatch_matches_expected_positions() {
        // Single ASCII-char prefix ("a"): several 'a's in the haystack,
        // with the greedy ".*z" pulling the match end to the last 'z'.
        let re = Regex::new("a.*z").unwrap();
        let hay = "xa1za2za3z";
        // byte layout: x=0,a=1,1=2,z=3,a=4,2=5,z=6,a=7,3=8,z=9; len=10.
        assert_eq!(re.search(hay, 0).unwrap().unwrap()[0], Some((1, 10)));
        // Starting after the first 'a': the prefix skip must land on
        // the second 'a', not re-match the first.
        assert_eq!(re.search(hay, 2).unwrap().unwrap()[0], Some((4, 10)));

        // Single CJK-char prefix ("中"): 1 char, 3 bytes — must still
        // take the char-count-1 fast path (`literal_prefix_char`), not
        // fall through to the general multi-char `&str::find` path.
        let re_cjk = Regex::new("中.*文").unwrap();
        let hay_cjk = "a中b文c中d文e";
        // byte layout: a=0,中=1..4,b=4,文=5..8,c=8,中=9..12,d=12,
        // 文=13..16,e=16; len=17.
        assert_eq!(
            re_cjk.search(hay_cjk, 0).unwrap().unwrap()[0],
            Some((1, 16))
        );
        assert_eq!(
            re_cjk.search(hay_cjk, 4).unwrap().unwrap()[0],
            Some((9, 16))
        );
    }

    #[test]
    fn caps_bytes_to_chars_handles_end_of_string_and_ascii_fast_path() {
        // Pure ASCII: byte offsets equal char offsets.
        let caps = vec![Some((0usize, 3usize)), None];
        assert_eq!(caps_bytes_to_chars("abc", &caps), vec![Some((0, 3)), None]);
        // Multi-byte: capture ending exactly at the end of the string
        // (byte offset == hay.len(), past the last char_indices entry).
        let hay = "a中b";
        // byte layout: a=0, 中=1..4, b=4; hay.len() = 5
        let caps = vec![Some((0usize, 5usize))];
        assert_eq!(caps_bytes_to_chars(hay, &caps), vec![Some((0, 3))]);
    }
}
