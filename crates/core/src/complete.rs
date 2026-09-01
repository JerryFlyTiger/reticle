//! M18: minibuffer completion — candidate sources, prefix logic, and
//! file-name helpers shared by the command loop and the builtins.
//!
//! M84: the matcher is orderless-style (space-separated tokens, each
//! must appear as a substring anywhere in the candidate, any order,
//! smart-case per token) rather than a single ordered prefix/substring
//! check. v1 does NOT do fuzzy/flex matching (no skipped-letter
//! matching — each token is a plain substring check), does not
//! highlight the matched fragments in candidates, and does not rank by
//! usage frequency or recency — see `orderless_rank`'s doc comment for
//! the exact contract.
//!
//! Known gaps (M84 fix round, record-only — not fixed here):
//!  - The M-x/Function/Symbol candidate PANEL builds one `PanelRow` per
//!    matching candidate with no cap (`panel::name_rows`), while
//!    `redisplay`'s panel rendering only ever draws a handful of rows
//!    at the bottom of the frame. Filtering the full `Symbol` source
//!    (every interned symbol) down to a handful of characters can still
//!    leave hundreds of rows built and thrown away unseen every
//!    keystroke. Pre-existing architecture (the M21 panel itself never
//!    capped row counts for File/Buffer either), not introduced by M84
//!    — flagged here because M84 is what put Symbol (the single
//!    largest candidate source in the editor) behind this per-keystroke
//!    path for the first time.
//!  - `cached_or_compute`'s session cache (`SESSION_CACHE`) is a single
//!    slot keyed only by `kind` (Command/Function/Symbol), not by any
//!    session identity. This is safe today because the architecture
//!    guarantees one live minibuffer session maps to exactly one fixed
//!    `Source` for its whole lifetime (`ArgSpec::code`/`collection`
//!    never change after the spec is created) — but that invariant
//!    lives in `commands.rs`'s `PendingArgs`/`ArgSpec` design, not
//!    enforced anywhere near this cache. A future change that let a
//!    single session's source change mid-flight (unlikely, but nothing
//!    here would catch it) would silently serve the wrong cached list.
//!  - TAB's longest-common-prefix expansion (`commands.rs`'s
//!    `minibuffer_tab`, `longest_common_prefix` below) is computed over
//!    the WHOLE filtered candidate list, rank 0 and rank 1 mixed
//!    together, not per-rank. When a substring-only (rank 1) candidate
//!    is mixed in with prefix (rank 0) ones, the common prefix across
//!    all of them is shorter than the prefix-only candidates would have
//!    given -- sometimes down to nothing at all, in which case TAB
//!    (correctly, as of H1's fix, M84 fix round) does not expand rather
//!    than expanding to something unrelated to what was typed. Before
//!    H1, the worse failure mode was possible: a `>` length-only check
//!    with no relationship check between the common prefix and the
//!    typed stem could expand the input to a string sharing NO
//!    characters with what the user typed (`commands.rs`'s
//!    `minibuffer_tab`, both call sites now guarded by `lcp.starts_with
//!    (&stem)`). That silent-replacement shape is fixed; the "TAB
//!    sometimes doesn't expand as far as you'd like, or at all, once a
//!    substring match dilutes the group" shape is not, and isn't
//!    planned for this milestone — see `commands.rs`'s
//!    `meta_x_tab_lcp_is_diluted_by_a_mixed_in_substring_match` in
//!    `completing_read_tests.rs` for the pinned current behavior.

use std::cell::RefCell;
use std::rc::Rc;

use elisp::Interp;

use crate::commands::ArgSpec;
use crate::editor::Editor;

/// What a minibuffer argument completes over, derived from the
/// interactive spec code that opened it (or, for M47's
/// `completing-read`, the caller-supplied candidate list directly).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// f / F / D — file names in the directory being typed.
    File,
    /// C — interactive commands (M-x).
    Command,
    /// a — any fbound symbol.
    Function,
    /// S — any interned symbol.
    Symbol,
    /// b / B — live buffer names.
    Buffer,
    /// M47: an explicit candidate list from `completing-read`, carried
    /// along so filtering doesn't need a side channel back to the
    /// `ArgSpec` that produced it.
    Custom(Rc<Vec<String>>),
    /// Everything else (free-form strings, numbers, expressions).
    None,
}

pub fn source_for_code(code: char) -> Source {
    match code {
        'f' | 'F' | 'D' => Source::File,
        'C' => Source::Command,
        'a' => Source::Function,
        'S' => Source::Symbol,
        'b' | 'B' => Source::Buffer,
        _ => Source::None,
    }
}

/// M47: like `source_for_code`, but a spec's caller-provided
/// `collection` (set by `completing-read`) always wins over its code —
/// `completing-read` builtins pass `code: 's'` (a no-op code on its
/// own) precisely so this is the only thing that decides the source.
pub fn source_for_spec(spec: &ArgSpec) -> Source {
    match &spec.collection {
        Some(v) => Source::Custom(v.clone()),
        None => source_for_code(spec.code),
    }
}

/// Candidates matching `input`, plus the char offset in `input` where
/// the completed segment (the "stem") starts — 0 for symbols/buffers,
/// right after the last `/` for files. Candidates are sorted;
/// directories carry a trailing `/`.
pub fn candidates(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    source: Source,
    input: &str,
) -> (usize, Vec<String>) {
    match source {
        Source::File => file_candidates(input),
        Source::Command => (
            0,
            filter_sorted(&cached_or_compute(interp, &Source::Command), input),
        ),
        Source::Function => (
            0,
            filter_sorted(&cached_or_compute(interp, &Source::Function), input),
        ),
        Source::Symbol => (
            0,
            filter_sorted(&cached_or_compute(interp, &Source::Symbol), input),
        ),
        Source::Buffer => {
            let names: Vec<String> = ed
                .borrow()
                .buffers
                .iter()
                .map(|b| b.borrow().name.clone())
                .collect();
            (0, filter_sorted(&names, input))
        }
        Source::Custom(cands) => (0, custom_filter(&cands, input)),
        Source::None => (0, Vec::new()),
    }
}

/// M84 D1: orderless-style match rank against a single candidate.
/// `input` is split on whitespace into tokens; every token must appear
/// as a substring SOMEWHERE in `cand` (any order, not necessarily
/// contiguous, tokens may overlap) or the candidate doesn't match at
/// all (`None`). Each token is matched smart-case (M82's `rg
/// --smart-case` convention): a token containing an uppercase letter is
/// case-sensitive, an all-lowercase token is case-insensitive. Among
/// matching candidates, rank 0 ("prefix") is a candidate where the
/// FIRST token is a prefix of the whole candidate (same smart-case
/// rule); everything else that matches is rank 1. An EMPTY input OR AN
/// INPUT CONTAINING ONLY WHITESPACE matches every candidate at rank 0
/// (`str::split_whitespace` yields zero tokens for either, so there's
/// nothing to fail against). This is a deliberate divergence from
/// pre-M84 behavior, pinned down as F5 in M84's fix round: the old
/// single-token `starts_with`/`contains` check would have matched
/// almost nothing against a whitespace-only input (a candidate would
/// need that exact run of spaces embedded in it), where this returns
/// "everything matches". Kept as-is rather than special-cased back to
/// the old behavior — showing every candidate reads better than an
/// empty list for a probably-accidental all-space input — see
/// `orderless_whitespace_only_input_matches_everything` in
/// `completing_read_tests.rs`.
pub fn orderless_rank(cand: &str, input: &str) -> Option<u8> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.is_empty() {
        return Some(0);
    }
    if !tokens.iter().all(|t| smart_case_contains(cand, t)) {
        return None;
    }
    Some(if smart_case_starts_with(cand, tokens[0]) {
        0
    } else {
        1
    })
}

fn smart_case_contains(hay: &str, needle: &str) -> bool {
    if needle.chars().any(|c| c.is_uppercase()) {
        hay.contains(needle)
    } else {
        hay.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn smart_case_starts_with(hay: &str, needle: &str) -> bool {
    if needle.chars().any(|c| c.is_uppercase()) {
        hay.starts_with(needle)
    } else {
        hay.to_lowercase().starts_with(&needle.to_lowercase())
    }
}

/// M47 `completing-read` filtering, M84: now orderless (`orderless_rank`)
/// instead of a single ordered prefix/substring check, but still split
/// into two ranked groups — prefix-rank matches first (in the
/// collection's own order), then substring-rank matches (also in
/// collection order). Deliberately NOT sorted or deduplicated like every
/// other `Source` here — the collection's order is the caller's own
/// semantics (e.g. LSP symbols in document order, most-recent-first
/// history), and re-sorting it would throw that away. The
/// prefix-before-substring split is also new relative to the other
/// sources: they're all `filter_sorted` orderless filters, but a symbol
/// picker where "substring only, nothing prefix-matches" still turns up
/// results reads far better than an empty list, so substring matches are
/// appended after (never instead of) the prefix ones rather than the two
/// being unioned and re-sorted.
pub fn custom_filter(cands: &[String], input: &str) -> Vec<String> {
    let mut prefix = Vec::new();
    let mut substr = Vec::new();
    for c in cands {
        match orderless_rank(c, input) {
            Some(0) => prefix.push(c.clone()),
            Some(_) => substr.push(c.clone()),
            None => {}
        }
    }
    prefix.extend(substr);
    prefix
}

/// Command / Function filtering (and, in principle, Buffer — see below):
/// orderless (`orderless_rank`), then EACH rank group is sorted and
/// deduplicated independently (rank 0 group first) — unlike
/// `custom_filter`, these sources have no caller-supplied order worth
/// preserving.
///
/// F2 correction (M84 fix round): the doc comment here used to claim
/// this serves "Command/Function/Symbol/Buffer", but `candidates()`'s
/// `Source::Buffer` arm that calls this is DEAD CODE in practice —
/// `Source::Buffer` has had its own panel (`panel::buffer_rows`,
/// current-buffer-first ordering that a rank-based re-sort here would
/// break) since M21, so `minibuffer_tab`'s `complete::candidates`
/// fallback (the only other caller) never runs for it: every non-`None`
/// `Source` opens a panel before M84's fix round ends, so that fallback
/// is unreachable full stop (also flagged in `commands.rs`'s
/// `minibuffer_tab` doc comment). The `Source::Buffer` arm is left as
/// dead code rather than removed here — actually reconciling the two
/// Buffer filtering paths (this one and `buffer_rows`'s) is a bigger
/// change than this fix round's scope.
fn filter_sorted(names: &[String], input: &str) -> Vec<String> {
    let mut prefix = Vec::new();
    let mut substr = Vec::new();
    for n in names {
        match orderless_rank(n, input) {
            Some(0) => prefix.push(n.clone()),
            Some(_) => substr.push(n.clone()),
            None => {}
        }
    }
    prefix.sort();
    prefix.dedup();
    substr.sort();
    substr.dedup();
    prefix.extend(substr);
    prefix
}

/// M84 D6: `command_names`/`function_names`/`symbol_names` each scan the
/// whole symbol table (`command_names` additionally runs
/// `commands::is_command`, itself an `interactive-spec` lookup, per
/// symbol). Before M84 that cost was paid once per TAB press; the M-x
/// candidate panel now refreshes on every keystroke, so without this
/// cache it would be paid on every keystroke instead. Cached for the
/// lifetime of one minibuffer session (a thread-local, since `Source`
/// carries no session id) and cleared by `reset_session_cache`
/// (`commands.rs`, called each time a fresh minibuffer opens) so a
/// `defun`/`intern` between sessions is picked up by the next one.
/// Panics if called with a `Source` other than Command/Function/Symbol —
/// callers only ever reach this from `candidates()`'s arms for those
/// three.
fn cached_or_compute(interp: &mut Interp, source: &Source) -> Rc<Vec<String>> {
    let kind = match source {
        Source::Command => 0u8,
        Source::Function => 1,
        Source::Symbol => 2,
        _ => unreachable!("cached_or_compute only serves Command/Function/Symbol"),
    };
    if let Some(names) = SESSION_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(k, _)| *k == kind)
            .map(|(_, v)| v.clone())
    }) {
        return names;
    }
    COMPUTE_COUNT.with(|c| c.set(c.get() + 1));
    let names = Rc::new(match source {
        Source::Command => command_names(interp),
        Source::Function => function_names(interp),
        Source::Symbol => symbol_names(interp),
        _ => unreachable!(),
    });
    SESSION_CACHE.with(|c| *c.borrow_mut() = Some((kind, names.clone())));
    names
}

thread_local! {
    static SESSION_CACHE: RefCell<Option<(u8, Rc<Vec<String>>)>> = const { RefCell::new(None) };
    // F1 (M84 fix round): this used to be a process-global `AtomicUsize`,
    // but what it measures ("did a recompute happen in THIS session")
    // only makes sense at the same scope as `SESSION_CACHE` itself. A
    // global counter meant any OTHER test in the same test binary that
    // also happened to miss the cache on a different thread bumped this
    // count in the middle of a targeted, filtered `cargo test` run --
    // T11's `before == mid == after` equality assertions would then
    // observe a bump that had nothing to do with the session under test.
    // Reproduced: `cargo test -p core --test completing_read_tests --
    // symbol_source_candidate_list_is_computed_once_per_session meta_x
    // orderless` looped 20x red a large fraction of the time before this
    // fix; a full-binary run never reproduced it (the race needs
    // multiple cache-missing tests actually overlapping in wall time,
    // which `--test-threads` default parallelism gives a filtered subset
    // but a full run's much larger thread pool dilutes).
    static COMPUTE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// M84 D6 / T11 test hook: number of Command/Function/Symbol
/// candidate-list (re)computations THIS THREAD has done (H2 correction,
/// M84 fix round: F1 changed the backing counter from a process-global
/// `AtomicUsize` to a thread-local `Cell` — this doc comment said "since
/// process start" from when it still was one; it's per-thread now, same
/// scope as `SESSION_CACHE` itself). There's no other public signal
/// that distinguishes "recomputed" from "served from cache" — the
/// returned candidate list looks identical either way — so tests that
/// want to observe the cache actually firing read this counter's delta
/// across keystrokes instead (on the SAME thread — `cargo test`'s
/// default runner gives each test its own thread, so this is safe to
/// read across keystrokes within one test without another test's
/// unrelated cache miss bleeding in).
///
/// This is a TEST-ONLY observability window, not a product API — it's
/// `pub` (rather than `#[cfg(test)]`) only because `completing_read_
/// tests.rs` is a separate integration-test binary that can't see the
/// lib crate's own `cfg(test)` items. If a cleaner mechanism shows up
/// later (e.g. exposing this count through an elisp builtin so it's
/// reachable without a Rust-side `pub fn` at all), prefer that over
/// this one.
pub fn debug_compute_count() -> usize {
    COMPUTE_COUNT.with(|c| c.get())
}

/// M84 D6: called each time a fresh minibuffer opens (`commands.rs`,
/// `process_pending`) so a stale Command/Function/Symbol list from a
/// PREVIOUS session (which may predate a `defun`/`intern` the user just
/// ran) is never served to a new one. Session-scoped, not
/// keystroke-scoped: repeated keys within the same session keep hitting
/// the same cached list, which is the whole point of caching.
pub fn reset_session_cache() {
    SESSION_CACHE.with(|c| *c.borrow_mut() = None);
}

fn command_names(interp: &mut Interp) -> Vec<String> {
    let ids: Vec<u32> = (0..interp.symbols.len() as u32)
        .filter(|&id| interp.symbols[id as usize].function.is_some())
        .collect();
    let mut out = Vec::new();
    for id in ids {
        if crate::commands::is_command(interp, id) {
            out.push(interp.symbols[id as usize].name.clone());
        }
    }
    out
}

fn function_names(interp: &Interp) -> Vec<String> {
    interp
        .symbols
        .iter()
        .filter(|s| s.function.is_some())
        .map(|s| s.name.clone())
        .collect()
}

fn symbol_names(interp: &Interp) -> Vec<String> {
    interp.symbols.iter().map(|s| s.name.clone()).collect()
}

/// File candidates for a partially typed path: list the directory part,
/// keep entries matching the basename stem. Returns (stem start in
/// chars, candidates).
fn file_candidates(input: &str) -> (usize, Vec<String>) {
    let (dir_part, stem) = match input.rfind('/') {
        Some(i) => (&input[..i + 1], &input[i + 1..]),
        None => ("", input),
    };
    let stem_start = dir_part.chars().count();
    let dir = expand_file_input(if dir_part.is_empty() { "." } else { dir_part });
    // M22: no remote completion — an ssh round trip per TAB (or per
    // panel refresh keystroke) would block typing (documented).
    if crate::remote::parse(&dir).is_some() {
        return (stem_start, Vec::new());
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (stem_start, Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let mut name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(stem) {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            name.push('/');
        }
        out.push(name);
    }
    out.sort();
    (stem_start, out)
}

/// Longest common prefix of all candidates (chars, so multi-byte safe).
pub fn longest_common_prefix(cands: &[String]) -> String {
    let Some(first) = cands.first() else {
        return String::new();
    };
    let mut lcp: Vec<char> = first.chars().collect();
    for c in &cands[1..] {
        let mut n = 0;
        for (a, b) in lcp.iter().zip(c.chars()) {
            if *a != b {
                break;
            }
            n += 1;
        }
        lcp.truncate(n);
        if lcp.is_empty() {
            break;
        }
    }
    lcp.into_iter().collect()
}

/// The directory `C-x C-f` starts from: the current buffer's file's
/// directory (HOME abbreviated to `~`, like GNU Emacs), else the
/// process working directory. Always ends with `/`.
pub fn default_directory(ed: &Rc<RefCell<Editor>>) -> String {
    // The buffer's own default-directory (set by find-file/dired/eshell)
    // wins; fall back to the visited file's parent, then the cwd.
    let (dd, file) = {
        let b = ed.borrow().current.clone();
        let b = b.borrow();
        (b.default_directory.clone(), b.file.clone())
    };
    let dir = dd
        .or_else(|| {
            file.and_then(|f| {
                std::path::Path::new(&f)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
            })
        })
        .filter(|d| !d.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "/".to_string());
    let mut dir = abbreviate_home(&dir);
    if !dir.ends_with('/') {
        dir.push('/');
    }
    dir
}

fn abbreviate_home(path: &str) -> String {
    abbreviate_home_with(path, std::env::var("HOME").ok().as_deref())
}

/// M61: the pure half of `abbreviate_home`, injectable for testing
/// without touching the (process-global) `HOME` env var. `panel.rs`'s
/// own `place_for` helper delegates here too, for the same reason.
pub(crate) fn abbreviate_home_with(path: &str, home: Option<&str>) -> String {
    if let Some(home) = home {
        if path == home {
            return "~".to_string();
        }
        if let Some(rest) = path.strip_prefix(&format!("{}/", home)) {
            return format!("~/{}", rest);
        }
    }
    path.to_string()
}

/// Expand a user-typed file path the way GNU Emacs's interactive file
/// readers do: with the minibuffer prefilled with the default
/// directory, typing a fresh absolute path just shadows the prefix —
/// everything before the last `//` or `/~` is discarded
/// (substitute-in-file-name) — then `~/` expands to $HOME.
pub fn expand_file_input(p: &str) -> String {
    let p = match p.rfind("//") {
        Some(i) => &p[i + 1..],
        None => p,
    };
    let p = match p.rfind("/~") {
        Some(i) => &p[i + 1..],
        None => p,
    };
    // M22: remote paths pass through verbatim after shadowing — the
    // remote shell owns any ~ in them.
    if p.starts_with("/ssh:") {
        return p.to_string();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    if p == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcp_basic() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            longest_common_prefix(&c(&["alpha.txt", "alphabet.txt"])),
            "alpha"
        );
        assert_eq!(longest_common_prefix(&c(&["beta"])), "beta");
        assert_eq!(longest_common_prefix(&c(&["x", "y"])), "");
        assert_eq!(longest_common_prefix(&[]), "");
        // Multi-byte safety.
        assert_eq!(
            longest_common_prefix(&c(&["中文檔.txt", "中文件.txt"])),
            "中文"
        );
    }

    #[test]
    fn shadowing_rules() {
        assert_eq!(expand_file_input("/a/b//tmp/x"), "/tmp/x");
        assert_eq!(expand_file_input("/a//b//c"), "/c");
        assert_eq!(expand_file_input("/plain/path"), "/plain/path");
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_file_input("/a/b/~/x"), format!("{}/x", home));
        assert_eq!(expand_file_input("~"), home);
    }

    // M61 T9: the pure half, not touching the (process-global) HOME env
    // var so this is safe under parallel test execution.
    #[test]
    fn abbreviate_home_with_cases() {
        assert_eq!(
            abbreviate_home_with("/home/u/rtl/top/x.sv", Some("/home/u")),
            "~/rtl/top/x.sv"
        );
        assert_eq!(abbreviate_home_with("/home/u", Some("/home/u")), "~");
        assert_eq!(
            abbreviate_home_with("/other/rtl/x.sv", Some("/home/u")),
            "/other/rtl/x.sv"
        );
        assert_eq!(abbreviate_home_with("/home/u/x.sv", None), "/home/u/x.sv");
    }
}
