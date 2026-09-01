//! P2 #8 differential tests for the literal-prefix fast-skip added to
//! `Regex::search` (`crates/elisp/src/regex.rs`), and (M43 period 2)
//! for the byte-indexed rewrite of the engine itself.
//!
//! The optimization: when a pattern's AST has no top-level `\|`
//! alternation and starts with a run of plain characters (optionally
//! interleaved with zero-width assertions like `^` `\b`), `Regex::new`
//! extracts that run into `literal_prefix`. `search` then skips straight
//! to positions where the prefix literally occurs (via `str::find`,
//! M43 period 2) instead of invoking `match_at` (heap-allocating a
//! `saves` vector and running the backtracking VM) at every single
//! position — the fix for `re-search-forward` being ~70x slower than
//! GNU Emacs on a large buffer where every line happens to share a
//! prefix with the search pattern.
//!
//! `match_at` itself is untouched by the fast-skip (only its *unit*
//! changed, char → byte, in M43 period 2), so the "before" behavior
//! isn't a reimplementation of the engine — it's exactly `search_naive`
//! below: the same try-every-char-position loop `search` used to be,
//! calling the same public `match_at`. Every differential case compares
//! `Regex::search` (new, prefix-skipping) against `search_naive` (old,
//! exhaustive) and requires bit-for-bit identical results: same overall
//! match Option-ness, same (start, end) BYTE offsets for every capture
//! group.
//!
//! M43 period 2 additionally widens this file's alphabet to include
//! multi-byte (CJK) characters — see `ALPHABET` — so the fuzz corpus
//! actually exercises byte != char divergence, and adds
//! `prefix_skip_never_matches_mid_character`, a direct construction of
//! the design's UTF-8 self-synchronization argument (a literal-prefix
//! pattern can never spuriously "match" starting inside a multi-byte
//! character's encoded bytes).

use elisp::regex::Regex;

/// The pre-optimization `search`: try `match_at` at every CHAR boundary
/// (not every byte — a byte-indexed loop would probe positions inside
/// multi-byte characters, which `match_at` explicitly rejects via
/// `debug_assert!(hay.is_char_boundary(..))`) in order and return the
/// first hit. Kept here, not in production code, purely as a
/// differential oracle.
fn search_naive(re: &Regex, hay: &str, start: usize) -> Option<Vec<Option<(usize, usize)>>> {
    for at in char_boundaries(hay) {
        if at < start {
            continue;
        }
        if let Some(caps) = re
            .match_at(hay, at)
            .expect("test haystacks are short — must not hit the M81 step/frame budget")
        {
            return Some(caps);
        }
    }
    None
}

/// Every char-boundary byte offset in `s`, 0..=s.len() inclusive.
fn char_boundaries(s: &str) -> Vec<usize> {
    s.char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(s.len()))
        .collect()
}

/// Tiny deterministic PRNG (xorshift64*) so the fuzz cases below are
/// reproducible without pulling in the `rand` crate (not currently a
/// dependency anywhere in this workspace).
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

/// Assert `search` and `search_naive` agree at every char-boundary start
/// position from 0 to `hay.len()` inclusive (not just position 0) — a
/// fast-skip bug that only shows up when resuming a search mid-buffer
/// (as `re-search-forward` does on every subsequent call) would
/// otherwise go unnoticed.
fn assert_matches_naive(pattern: &str, haystack: &str) {
    let re = Regex::new(pattern)
        .unwrap_or_else(|e| panic!("pattern {pattern:?} failed to compile: {e}"));
    for start in char_boundaries(haystack) {
        let got = re
            .search(haystack, start)
            .expect("test haystacks are short — must not hit the M81 step/frame budget");
        let want = search_naive(&re, haystack, start);
        assert_eq!(
            got, want,
            "pattern {pattern:?} haystack {haystack:?} start(byte) {start}: search={got:?} naive={want:?}"
        );
    }
}

// --- Patterns with a genuine literal prefix ---

#[test]
fn literal_prefix_plain_word() {
    assert_matches_naive("line 199999", "line 1\nline 199999 filler\nline 2\n");
    assert_matches_naive("line 199999", "no match anywhere in this text at all");
    assert_matches_naive("line 199999", "");
}

#[test]
fn literal_prefix_with_leading_anchor() {
    assert_matches_naive("^foo", "bar\nfoo\nfoobar\n");
    assert_matches_naive("^foo", "foo at the very start");
    assert_matches_naive("^foo", "nofoo on this line\nfoo on this one\n");
}

#[test]
fn literal_prefix_with_trailing_word_boundary() {
    assert_matches_naive("foobar\\b", "foobar baz foobarbaz foobar!");
    assert_matches_naive("foobar\\b", "foobarbaz only, no boundary hit");
}

#[test]
fn literal_prefix_with_string_anchors_and_boundary() {
    assert_matches_naive("\\`abc", "abcdef");
    assert_matches_naive("abc\\'", "xxabc");
    assert_matches_naive("\\<word\\>", "a word here, wordy words, word.");
}

#[test]
fn literal_prefix_overlapping_occurrences() {
    // Regression target called out explicitly in the design: the +1
    // char (not +prefix.len()) step must find overlapping occurrences
    // and the single leftmost match, exactly like the naive loop.
    assert_matches_naive("aa", "aaaa");
    assert_matches_naive("aa", "aaaaa");
    assert_matches_naive("aba", "ababababa");
    assert_matches_naive("aa", "a");
    assert_matches_naive("aa", "b");
}

// --- Patterns with no usable prefix (the untouched fallback path) ---

#[test]
fn no_prefix_dot_star() {
    assert_matches_naive(".*foo", "xxxfooyyyfoo");
    assert_matches_naive(".*199999", "line 1\nline 199999 filler\nline 2\n");
}

#[test]
fn no_prefix_class_start() {
    assert_matches_naive("[a-z]+bar", "123 foobar 456 zzbar");
    assert_matches_naive("[0-9][0-9]*x", "ab12x cd34x");
}

#[test]
fn no_prefix_shy_group_start() {
    assert_matches_naive("\\(?:ab\\)cd", "xxabcdyyabcd");
    assert_matches_naive("\\(?:ab\\)cd", "no match here");
}

#[test]
fn no_prefix_capturing_group_start() {
    assert_matches_naive("\\(foo\\)bar", "xxfoobaryyfoobar");
}

#[test]
fn no_prefix_top_level_alternation() {
    assert_matches_naive("a\\|b", "xxxaxxxbxxx");
    assert_matches_naive("cat\\|dog", "the dog chased the cat");
}

#[test]
fn no_prefix_leading_repeat() {
    // A pattern that is itself a repeat at the top level (no fixed
    // leading literal char to collect before the Repeat node).
    assert_matches_naive("a*b", "aaab");
    assert_matches_naive("a+b", "xaaabx");
}

// --- Boundary cases from the spec ---

#[test]
fn boundary_pattern_longer_than_haystack() {
    assert_matches_naive("this pattern is way too long", "short");
    assert_matches_naive("this pattern is way too long", "");
}

#[test]
fn boundary_match_at_very_last_char() {
    assert_matches_naive("z", "abcxyz");
    assert_matches_naive("xyz", "abcwxyz");
}

#[test]
fn boundary_empty_haystack() {
    assert_matches_naive("anything", "");
    assert_matches_naive("^$", "");
    assert_matches_naive(".*", "");
}

#[test]
fn boundary_no_occurrence() {
    assert_matches_naive("zzz", "aaa bbb ccc ddd eee");
}

// --- M43 period 2: multi-byte / self-synchronization cases ---

/// Direct construction of the design's UTF-8 self-synchronization
/// argument (§3.4): `literal_prefix`'s encoded first byte can only ever
/// be an ASCII byte or a UTF-8 lead byte, never a continuation byte, so
/// `str::find` can never report a "candidate" that starts inside a
/// multi-byte character's encoding — every candidate is necessarily
/// char-boundary-aligned. These haystacks deliberately place CJK
/// characters immediately adjacent to (and interleaved with) the
/// pattern's literal text, the shape most likely to expose an
/// off-by-one in the "advance past one char, not one byte" prefix-skip
/// step if that invariant were ever violated.
#[test]
fn prefix_skip_never_matches_mid_character() {
    assert_matches_naive("line", "中line文linex中文line 199999文中line");
    assert_matches_naive("line 199999", "中文line 1中文line 199999文中");
    assert_matches_naive("a", "中a文a中中a");
    assert_matches_naive("ab", "中ab文中ab");
    assert_matches_naive("foo\\b", "中foo文foo 中foo");
    // The case that actually distinguishes "advance past one CHAR" from
    // "advance past one BYTE" after a failed candidate: a MULTI-BYTE
    // literal prefix ("中"/"é") occurring twice, where only the SECOND
    // occurrence satisfies the trailing `\'` string-end anchor — so
    // `search` must fail at the first candidate and retry. A
    // byte-advance bug lands the retry mid-character (not a char
    // boundary), which — via the checked `str::get` in `search` —
    // aborts the whole search with `None` instead of finding the real
    // match. Confirmed as a genuine mutation-catcher: flipping
    // `Regex::search`'s retry step from `char_len_at(hay, cand)` to a
    // hardcoded `1` makes exactly these two assertions fail while every
    // ASCII-prefix case above keeps passing (ASCII chars are 1 byte, so
    // that mutation is a no-op for them).
    assert_matches_naive("中\\'", "中x中");
    assert_matches_naive("é\\'", "éxé");
}

#[test]
fn literal_prefix_itself_multibyte() {
    assert_matches_naive("中文", "abc中文def中文ghi");
    assert_matches_naive("^中", "中文\nabc\n中");
    assert_matches_naive("中\\{2\\}", "中中中abc");
    assert_matches_naive("\\<中文\\>", "see 中文 here 中文者");
}

#[test]
fn word_boundary_multibyte() {
    assert_matches_naive("\\bfoo\\b", "中文foo中文 foo bar");
    assert_matches_naive("\\w+", "中文abc123中文");
    assert_matches_naive("é\\b", "café café123");
}

// --- Randomized fuzz: many small pattern/haystack pairs, fixed seed ---

/// Alphabet mixing ASCII with multi-byte (CJK + Latin-Extended)
/// characters (M43 period 2 widening — was ASCII-only) so
/// collisions/overlaps between pattern and haystack are common, and so
/// byte-vs-char divergence is actually exercised — that's exactly the
/// adversarial case for a byte-indexed prefix-skip loop (lots of
/// partial-prefix false starts to reject correctly, now with multi-byte
/// characters in the mix).
const ALPHABET: &[char] = &['a', 'b', 'c', 'x', '中', '文', 'é'];

fn random_string(rng: &mut Rng, len: usize) -> String {
    (0..len)
        .map(|_| ALPHABET[rng.next_range(ALPHABET.len())])
        .collect()
}

/// Random patterns built from a small grammar mixing literal runs with
/// occasional class/group/repeat/alternation/anchor pieces, so roughly
/// half have a usable literal prefix and half don't.
fn random_pattern(rng: &mut Rng) -> String {
    let choice = rng.next_range(8);
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
            format!("[abc中]+{}", random_string(rng, len)) // no prefix
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
        _ => {
            let a = random_string(rng, 1);
            let b = random_string(rng, 2);
            format!("{}*{}", a, b) // leading repeat, no prefix
        }
    }
}

#[test]
fn randomized_differential_fixed_seed() {
    let mut rng = Rng::new(0x00C0_FFEE_1234_5678);
    let mut checked = 0;
    for _ in 0..200 {
        let pattern = random_pattern(&mut rng);
        let hay_len = rng.next_range(20);
        let haystack = random_string(&mut rng, hay_len);
        let Ok(re) = Regex::new(&pattern) else {
            continue;
        };
        for start in char_boundaries(&haystack) {
            let got = re
                .search(&haystack, start)
                .expect("test haystacks are short — must not hit the M81 step/frame budget");
            let want = search_naive(&re, &haystack, start);
            assert_eq!(
                got, want,
                "pattern {pattern:?} haystack {haystack:?} start(byte) {start}: search={got:?} naive={want:?}"
            );
        }
        checked += 1;
    }
    // Guard against the whole fuzz loop silently compiling zero
    // patterns (e.g. every generated pattern erroring out) and passing
    // vacuously.
    assert!(
        checked > 150,
        "expected most of the 200 random patterns to compile, got {checked}"
    );
}
