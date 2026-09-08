//! Background syntax highlighting (M15 item 4) — the VS Code model,
//! possible here in true parallel because tree-sitter is pure Rust and
//! the input is a `String` snapshot (Send, zero unsafe): the buffer text
//! is snapshotted after a short debounce, parsed + queried on a
//! persistent worker thread, and the resulting `(range, face)` spans
//! come back as plain data for the main thread to materialize as
//! overlays at the next tick. Typing while a parse is in flight costs
//! nothing; the highlight arrives a beat later instead of the keystroke
//! stalling — exactly the "semantic tokens" behavior VS Code has.
//!
//! Staleness is handled by generation counting against
//! `Buffer::edit_ticks` (bumped by every mutation entry point including
//! undo): a result whose generation no longer matches is simply
//! replaced by the next parse. There is no byte-level edit accounting
//! anywhere, so there is nothing to get subtly wrong — the same
//! reasoning that made M12 choose full reparse over `Tree::edit`.
//!
//! Overlay volume is bounded by materializing only the visible region
//! plus one screen of margin on each side (the overlay store is a
//! linear-scan Vec); scrolling outside the margin re-materializes from
//! the cached span list without reparsing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use elisp::{Interp, Value};

use crate::buffer::{Buffer, OverlayData};
use crate::scope::Scope;
use crate::treesit::Lang;

/// Snapshot age before it is sent to the parser thread. Continuous
/// typing keeps deferring the parse; a natural pause triggers it.
const DEBOUNCE: Duration = Duration::from_millis(30);

/// Extra chars materialized above/below the visible region.
const MARGIN_CHARS: usize = 4000;

pub struct Span {
    pub start: usize,
    pub end: usize,
    pub face: &'static str,
    /// M37: true for a rainbow-delimiters depth-coloring span (from
    /// `collect_rainbow_spans`), false for an ordinary query-capture
    /// span (`face_for_capture`). Lets `apply_visible` gate just this
    /// category on the buffer-local `rainbow-delimiters-mode' toggle
    /// without a second, separately cached/parsed span list -- the
    /// worker always computes both categories every parse; only
    /// materialization is conditional (see `apply_visible`).
    pub rainbow: bool,
}

struct Job {
    key: usize,
    gen: u64,
    lang: Lang,
    text: String,
}

struct Res {
    key: usize,
    gen: u64,
    spans: Vec<Span>,
    /// M113 (show-paren-mode): (open_char_pos, close_char_pos) for every
    /// bracket pair `collect_rainbow_spans` matched during this same
    /// tree walk -- both positions are always exactly one char wide (a
    /// literal `(`/`[`/`{`/`)`/`]`/`}` token), so there is no separate
    /// end to carry. Riding the same worker pass as `spans` means point
    /// never has to trigger a reparse: this list only changes when the
    /// text does, and the per-frame part (which pair, if any, is
    /// adjacent to point) is decided on the main thread by
    /// `Engine::matching_pair`.
    pairs: Vec<(usize, usize)>,
    /// M118 (sticky scope header / breadcrumb): every scope-kind node in
    /// this parse (`scope::is_scope_kind`), collected in the same
    /// background-thread walk as `spans`/`pairs` -- see
    /// `Engine::scope_chain`'s doc for the per-frame query this feeds
    /// and its generation-guard contract.
    scopes: Vec<Scope>,
}

/// A completed parse's generation, highlight spans, bracket pairs
/// (M113), and scope-chain nodes (M118), bundled since they're always
/// cached/replaced together.
type Cached = (u64, Vec<Span>, Vec<(usize, usize)>, Vec<Scope>);

struct BufState {
    buffer: Weak<RefCell<Buffer>>,
    lang: Lang,
    /// Latest edit_ticks observed on the buffer.
    seen: u64,
    /// When `seen` last changed (debounce clock).
    last_change: Instant,
    /// Generation of the job currently in flight, if any.
    in_flight: Option<u64>,
    /// Cached spans + bracket pairs from the last completed parse, plus
    /// their generation (M113: the pairs are `Res::pairs`, cached
    /// alongside spans so `Engine::matching_pair` never has to reparse
    /// just because point moved).
    cached: Option<Cached>,
    /// What's currently materialized: (generation, char range, whether
    /// `rainbow-delimiters-mode` was on for that materialization -- M37,
    /// see `apply_visible`).
    applied: Option<(u64, (usize, usize), bool)>,
}

pub struct Engine {
    job_tx: mpsc::Sender<Job>,
    res_rx: mpsc::Receiver<Res>,
    states: HashMap<usize, BufState>,
}

impl Engine {
    pub fn new() -> Engine {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (res_tx, res_rx) = mpsc::channel::<Res>();
        std::thread::spawn(move || worker(job_rx, res_tx));
        Engine {
            job_tx,
            res_rx,
            states: HashMap::new(),
        }
    }

    pub fn enable(&mut self, buffer: &Rc<RefCell<Buffer>>, lang: Lang) {
        let key = Rc::as_ptr(buffer) as usize;
        let seen = buffer.borrow().edit_ticks;
        self.states.insert(
            key,
            BufState {
                buffer: Rc::downgrade(buffer),
                lang,
                seen,
                // Backdated so the very first tick passes the debounce
                // and kicks off the initial parse immediately.
                last_change: Instant::now() - DEBOUNCE,
                in_flight: None,
                cached: None,
                applied: None,
            },
        );
    }

    pub fn is_enabled(&self, buffer: &Rc<RefCell<Buffer>>) -> bool {
        self.states.contains_key(&(Rc::as_ptr(buffer) as usize))
    }

    /// M113 (show-paren-mode): the bracket pair GNU Emacs would show for
    /// `point` in `buffer`, or `None`. Matches GNU's default adjacency
    /// rule (verified against real `emacs -Q --batch`, see the M113
    /// report): triggers when the char immediately AFTER point is an
    /// opener (`point == open`), or the char immediately BEFORE point is
    /// a closer (`point == close + 1`) -- never when point merely sits
    /// next to a bracket from the "inside" (just after an opener, or
    /// just before a closer). An unmatched bracket is not in `pairs` at
    /// all (`collect_rainbow_spans` only ever records a pair once its
    /// closer is actually found), so this naturally returns `None` for
    /// it -- deliberately NOT GNU's own behavior (real Emacs highlights
    /// an unmatched bracket alone in a "mismatch" face); this project
    /// has no mismatch face and the milestone spec calls for "nothing"
    /// here, so this is a documented, intentional divergence.
    ///
    /// A linear scan of every pair in the buffer, not a hash lookup:
    /// this runs once per frame per selected window (not once per
    /// character), so its cost is O(pairs in the buffer) once per
    /// render, not O(pairs) per cell.
    pub fn matching_pair(
        &self,
        buffer: &Rc<RefCell<Buffer>>,
        point: usize,
    ) -> Option<(usize, usize)> {
        let key = Rc::as_ptr(buffer) as usize;
        let (gen, _, pairs, _) = self.states.get(&key)?.cached.as_ref()?;
        // M113 review fix round: `cached` is the last COMPLETED parse,
        // but the buffer's live `edit_ticks` can already have moved past
        // it -- an edit bumps `edit_ticks` immediately; the worker only
        // catches up a debounce-interval later. Between those two
        // moments, `pairs`' char offsets describe text that no longer
        // exists, and comparing them against the buffer's CURRENT point
        // can coincidentally "match" a position that isn't a bracket at
        // all anymore (reproduced: edit elsewhere in the buffer while
        // point rests on an already-matched bracket -- the edit doesn't
        // move point, so the stale coordinates keep "matching" until the
        // reparse lands). Same generation check `treesit.rs`'s `parse`
        // already uses for its own tree cache (`cached_gen == gen`);
        // deliberately does NOT try to shift `pairs`' offsets to follow
        // the edit -- see this method's test coverage
        // (`show_paren_tests.rs`) for why "no highlight for one frame"
        // is the correct answer, not "guess where the pair moved to".
        if *gen != buffer.borrow().edit_ticks {
            return None;
        }
        // `.find()` returns the FIRST match in `pairs`' own order, which
        // is close-position ascending (see `collect_rainbow_spans`'
        // ORDERING INVARIANT comment) -- load-bearing at a
        // `"()()"`-style boundary where point sits exactly between one
        // pair's closer and the next pair's opener, satisfying BOTH
        // pairs' adjacency condition at once. Real GNU Emacs picks the
        // earlier-closing pair there (verified against `emacs -Q
        // --batch`); returning the first match is what reproduces that,
        // and only continues to as long as `pairs` keeps coming out in
        // that order.
        pairs
            .iter()
            .copied()
            .find(|&(open, close)| open == point || close + 1 == point)
    }

    /// M118 (sticky scope header / breadcrumb): constructs enclosing
    /// `pos`, outermost first. Empty when the buffer has no parse, no
    /// grammar, or a parse older than the buffer's current `edit_ticks`.
    ///
    /// Same shape as `matching_pair` above, including the generation
    /// guard: an edit bumps `edit_ticks` immediately, but the worker only
    /// catches up a debounce interval later, so `cached`'s scopes can
    /// describe text that no longer exists for a short window. Returning
    /// empty rather than trying to shift `Scope::start`/`end` to follow
    /// the edit is the same tradeoff `matching_pair` documents at length
    /// -- "no header/breadcrumb for one frame" is the correct answer,
    /// not "guess where the enclosing construct moved to" (a stale
    /// `Scope` pinning the wrong source line as a header row would be
    /// actively misleading, worse than showing nothing).
    pub fn scope_chain(&self, buffer: &Rc<RefCell<Buffer>>, pos: usize) -> Vec<Scope> {
        let key = Rc::as_ptr(buffer) as usize;
        let Some((gen, _, _, scopes)) = self.states.get(&key).and_then(|st| st.cached.as_ref())
        else {
            return Vec::new();
        };
        if *gen != buffer.borrow().edit_ticks {
            return Vec::new();
        }
        // Half-open enclosure: `start <= pos && pos < end`. Point resting
        // exactly AT a scope's own end (e.g. right on `endmodule`'s
        // final char, or the char immediately after it) is treated as
        // OUTSIDE that scope -- intentional, the same half-open
        // convention this codebase already uses for spans elsewhere
        // (buffer ranges throughout are `[start, end)`), not an
        // oversight.
        let mut chain: Vec<Scope> = scopes
            .iter()
            .filter(|s| s.start <= pos && pos < s.end)
            .cloned()
            .collect();
        // Outermost first: ascending by start, and on a tie descending
        // by end so the wider (outer) one sorts first -- defensive, not
        // exercised by any known input. The comment this replaced cited
        // a `module_declaration`/`module_ansi_header` tie as the
        // motivating case, but `module_ansi_header` is not in
        // `is_scope_kind` (`scope.rs`), so it never becomes a `Scope`
        // and can never reach this comparator. No construct
        // `is_scope_kind` currently accepts, for any language this
        // module supports, is known to nest another accepted construct
        // starting at the exact same char. Same honesty convention
        // `treesit.rs`'s `incremental_parse` uses for its own
        // known-unobservable length check: kept as defence against a
        // grammar shape nobody has found yet, not claimed as covered.
        chain.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        chain
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// The grammar and highlight-query text for `lang`. Also called directly
/// by this file's own `tests` module to audit capture-name coverage
/// against the *exact* query text this engine runs, rather than a copy
/// that could silently drift from it.
///
/// Eight of these nine query texts are our own M33 file under
/// `crates/core/queries/`, trimmed from the matching grammar crate's
/// upstream `HIGHLIGHTS_QUERY`/`HIGHLIGHT_QUERY` constant toward GNU
/// font-lock conventions (definition sites only; no naming-convention
/// heuristics; see each `.scm` file's own header for the per-language
/// rationale) rather than the "editor semantic tokens" style upstream
/// ships. Because each file is now fully self-contained, M25's
/// `cpp_highlight_query()` runtime concatenation of the C and C++
/// upstream queries (`tree_sitter_c::HIGHLIGHT_QUERY` +
/// `tree_sitter_cpp::HIGHLIGHT_QUERY` via a `OnceLock`) is gone: it was
/// only ever needed because upstream's *own* cpp query assumes C's is
/// layered underneath, but crates/core/queries/cpp-highlights.scm
/// deliberately duplicates the C-side rules by hand instead (see that
/// file's header). The ninth, M38's verilog-highlights.scm, was written
/// from scratch (its grammar crate ships no upstream query constant at
/// all to trim from) but follows the exact same GNU font-lock philosophy
/// -- see that file's own header.
pub fn lang_query(lang: Lang) -> (tree_sitter::Language, &'static str) {
    match lang {
        Lang::Rust => (
            tree_sitter_rust::LANGUAGE.into(),
            include_str!("../queries/rust-highlights.scm"),
        ),
        Lang::C => (
            tree_sitter_c::LANGUAGE.into(),
            include_str!("../queries/c-highlights.scm"),
        ),
        Lang::Cpp => (
            tree_sitter_cpp::LANGUAGE.into(),
            include_str!("../queries/cpp-highlights.scm"),
        ),
        Lang::Python => (
            tree_sitter_python::LANGUAGE.into(),
            include_str!("../queries/python-highlights.scm"),
        ),
        Lang::Bash => (
            tree_sitter_bash::LANGUAGE.into(),
            include_str!("../queries/bash-highlights.scm"),
        ),
        Lang::Java => (
            tree_sitter_java::LANGUAGE.into(),
            include_str!("../queries/java-highlights.scm"),
        ),
        // Not `tree-sitter-perl` -- that crates.io name is squatted by an
        // unmaintained package with a broken dependency declaration
        // (won't build) and no bundled query at all. See the M25 grammar
        // survey.
        Lang::Perl => (
            ts_parser_perl::LANGUAGE.into(),
            include_str!("../queries/perl-highlights.scm"),
        ),
        // tree-sitter-elisp 1.6's published crate comments out its
        // HIGHLIGHTS_QUERY constant even though queries/highlights.scm
        // ships inside the package, so we vendor that file ourselves --
        // see crates/core/queries/elisp-highlights.scm for the exact
        // provenance, license, and M33 trimming notes.
        Lang::Elisp => (
            tree_sitter_elisp::LANGUAGE.into(),
            include_str!("../queries/elisp-highlights.scm"),
        ),
        // M38: tree-sitter-systemverilog ships no HIGHLIGHTS_QUERY constant
        // at all (unlike most of the eight M25/M33 grammars), so this file
        // was written from scratch, not trimmed from an upstream query --
        // see its own header for the full dump-verified rationale.
        Lang::Verilog => (
            tree_sitter_systemverilog::LANGUAGE.into(),
            include_str!("../queries/verilog-highlights.scm"),
        ),
    }
}

/// The worker thread: parse + highlight-query + byte→char conversion,
/// all on a text snapshot it owns. Exits when the Engine (and thus the
/// job sender) is dropped.
fn worker(job_rx: mpsc::Receiver<Job>, res_tx: mpsc::Sender<Res>) {
    let mut parsers: HashMap<Lang, Option<(tree_sitter::Parser, tree_sitter::Query)>> =
        HashMap::new();
    while let Ok(job) = job_rx.recv() {
        let (ts_lang, query_src) = lang_query(job.lang);
        // A language whose grammar or query fails to initialize is
        // isolated (logged once, then skipped) instead of panicking the
        // thread: all eight queries are hand-maintained vendored files
        // since M33, so one bad edit or an incompatible grammar bump
        // must cost highlighting for THAT language only — not silently
        // kill this shared worker and with it every language's
        // highlighting for the rest of the session (M33 review).
        let entry = parsers.entry(job.lang).or_insert_with(|| {
            let mut p = tree_sitter::Parser::new();
            if let Err(e) = p.set_language(&ts_lang) {
                // M60: was eprintln! straight to the terminal, which the
                // TUI's diff-based redisplay never revisits. Routed
                // through the background log instead (see
                // `elisp::bglog`'s module doc).
                elisp::bglog::push(
                    "highlight",
                    &format!("{:?} grammar rejected: {e}", job.lang),
                );
                return None;
            }
            match tree_sitter::Query::new(&ts_lang, query_src) {
                Ok(q) => Some((p, q)),
                Err(e) => {
                    elisp::bglog::push(
                        "highlight",
                        &format!("{:?} query failed to compile: {e}", job.lang),
                    );
                    None
                }
            }
        });
        let Some((parser, query)) = entry else {
            continue;
        };
        let Some(tree) = parser.parse(&job.text, None) else {
            continue;
        };
        let (spans, pairs) = extract_spans(&job.text, &tree, query);
        // M118: a separate, simpler walk than `extract_spans`'
        // capture-query pass -- scopes are plain node-kind matches, not
        // query captures, so there is no shared machinery to ride along
        // with. Still only once per completed parse, on this same
        // background thread, never per frame.
        let scopes = crate::scope::collect_scopes(&tree, &job.text, job.lang);
        if res_tx
            .send(Res {
                key: job.key,
                gen: job.gen,
                spans,
                pairs,
                scopes,
            })
            .is_err()
        {
            return; // engine dropped
        }
    }
}

fn extract_spans(
    text: &str,
    tree: &tree_sitter::Tree,
    query: &tree_sitter::Query,
) -> (Vec<Span>, Vec<(usize, usize)>) {
    use streaming_iterator::StreamingIterator;
    let mut cursor = tree_sitter::QueryCursor::new();
    // 4th element: `rainbow` (see `Span::rainbow`) -- false for every
    // capture-based span collected here.
    let mut raw: Vec<(usize, usize, &'static str, bool)> = Vec::new();
    let mut captures = cursor.captures(query, tree.root_node(), text.as_bytes());
    while let Some((m, idx)) = captures.next() {
        let cap = m.captures[*idx];
        let name = query.capture_names()[cap.index as usize];
        if let Some(face) = face_for_capture(name) {
            raw.push((cap.node.start_byte(), cap.node.end_byte(), face, false));
        }
    }
    // M37: rainbow-delimiters -- a second pass, independent of the query
    // above (see `collect_rainbow_spans`), appended into the SAME `raw`
    // vec so it rides the byte->char conversion below for free.
    //
    // M113 (show-paren-mode): the same walk also hands back
    // `raw_pairs`, the (open_start_byte, close_start_byte) of every
    // pair it actually matched. Both positions are always themselves
    // entries `collect_rainbow_spans` already pushed into `raw` above
    // (an opener's span is pushed the moment it's seen; a closer's only
    // when it matches -- exactly the condition a pair is recorded
    // under), so they are guaranteed already present in `boundaries`
    // below with no separate pass needed.
    let mut raw_pairs: Vec<(usize, usize)> = Vec::new();
    collect_rainbow_spans(tree, &mut raw, &mut raw_pairs);
    // Convert all byte boundaries to char positions in one pass over the
    // text (boundaries sorted, walked with a single char_indices scan).
    let mut boundaries: Vec<usize> = raw.iter().flat_map(|&(s, e, _, _)| [s, e]).collect();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut byte_to_char: HashMap<usize, usize> = HashMap::with_capacity(boundaries.len());
    let mut bi = 0;
    for (char_idx, (byte_idx, _)) in text.char_indices().enumerate() {
        while bi < boundaries.len() && boundaries[bi] <= byte_idx {
            byte_to_char.insert(boundaries[bi], char_idx);
            bi += 1;
        }
        if bi == boundaries.len() {
            break;
        }
    }
    let total_chars = text.chars().count();
    while bi < boundaries.len() {
        byte_to_char.insert(boundaries[bi], total_chars);
        bi += 1;
    }
    let pairs: Vec<(usize, usize)> = raw_pairs
        .into_iter()
        .map(|(o, c)| {
            (
                byte_to_char.get(&o).copied().unwrap_or(0),
                byte_to_char.get(&c).copied().unwrap_or(0),
            )
        })
        .collect();
    let spans = raw
        .into_iter()
        .map(|(s, e, face, rainbow)| Span {
            start: byte_to_char.get(&s).copied().unwrap_or(0),
            end: byte_to_char.get(&e).copied().unwrap_or(0),
            face,
            rainbow,
        })
        .collect();
    (spans, pairs)
}

/// `rainbow-delimiters-depth-{1..9}-face`, cycling every 9 nesting
/// levels (mirrors the `rainbow-delimiters` package) -- plain string
/// literals rather than a `format!`-built `String`, since `Span::face`
/// is `&'static str` like every other face this engine ever names.
/// Colors themselves live entirely in themes.el; this file only ever
/// emits the face NAME.
const RAINBOW_DEPTH_FACES: [&str; 9] = [
    "rainbow-delimiters-depth-1-face",
    "rainbow-delimiters-depth-2-face",
    "rainbow-delimiters-depth-3-face",
    "rainbow-delimiters-depth-4-face",
    "rainbow-delimiters-depth-5-face",
    "rainbow-delimiters-depth-6-face",
    "rainbow-delimiters-depth-7-face",
    "rainbow-delimiters-depth-8-face",
    "rainbow-delimiters-depth-9-face",
];

/// `depth` (1 = outermost) -> face name, wrapping every 9 levels: depth
/// 10 reuses depth 1's face, same as `rainbow-delimiters` itself.
fn rainbow_face(depth: i32) -> &'static str {
    RAINBOW_DEPTH_FACES[(depth - 1).rem_euclid(9) as usize]
}

/// M37 second pass for `extract_spans`: walk every ANONYMOUS LEAF token
/// of the parse tree (an iterative cursor walk, not recursion -- see
/// below) looking for a literal `(` `)` `[` `]` `{` `}`, coloring each
/// by nesting depth as it goes (rainbow-delimiters' whole idea: an
/// opener is colored at the depth it INTRODUCES, its matching closer at
/// the SAME depth just before that level closes back down, so a pair
/// always shares one color). Appends `(start_byte, end_byte, face,
/// true)` tuples onto `out`.
///
/// A tree WALK (not a query) is what keeps a bracket CHARACTER embedded
/// inside a string/comment literal's own TEXT from ever being counted:
/// that text lives entirely inside the one string/comment node, never
/// as a separate anonymous token the way a real structural bracket
/// always parses as -- dump-verified precedent already established by
/// indent.el's `indent--closer-token-at-p` (same codebase, M36): an
/// anonymous/literal token's `kind()` IS its own text, e.g. a real `}`
/// token's kind is the string "}", so checking `node.kind()` against
/// the literal bracket strings below can only ever match a REAL
/// structural bracket token, never a character sitting inside a wider
/// string/comment/identifier node's text.
///
/// Iterative (`TreeCursor::goto_first_child`/`goto_next_sibling`/
/// `goto_parent`, not a recursive descent) so a pathologically deep
/// nesting can't blow the worker thread's call stack the way a naive
/// recursive walk could -- this file's own worker thread has no size
/// ceiling on the buffers it parses (unlike indent.el's
/// `indent-treesit-max-chars', which exists precisely because THAT
/// engine reparses synchronously on every keystroke; this one is always
/// async and debounced, so the cost that guard defends against doesn't
/// apply here, but an unbounded call-stack depth still would).
///
/// Depth tracking is a SIBLING-MATCHING STACK, not the bare linear counter
/// this replaces (M37 review fix round). The linear counter's `depth > 0`
/// guard only ever caught an OVER-closed buffer; an unclosed OPENER still
/// inflated `depth` forever with no way back down, so everything after it
/// -- including a later, syntactically-complete, unrelated top-level
/// definition -- inherited the wrong depth all the way to EOF. That
/// contradicted this same comment's old claim of protecting against
/// exactly that.
///
/// The fix leans on one invariant true of all eight grammars: a pair of
/// matching brackets is always a pair of SIBLINGS under the same parent
/// node (an `arguments` node's children are `(` ... `)`; an elisp `list`
/// node's children are `(` ... `)`; etc.). `open` holds `(parent node id,
/// expected closer kind)` for every opener whose parent's subtree hasn't
/// finished walking yet, oldest first. `ancestors` is the current node's
/// ancestor-id chain, maintained by hand across
/// `goto_first_child`/`goto_parent` -- cheaper than calling the O(depth)
/// `Node::parent()` once per leaf.
///
///   - An OPENER (parent id `p`) always pushes `(p, matching closer
///     kind)` and colors at `rainbow_face(open.len())` -- the depth it
///     introduces, same idea as before.
///   - A CLOSER (parent id `p`, kind `k`) colors and pops ONLY when
///     `open`'s TOP entry is exactly `(p, k)` -- parent AND kind both
///     have to match. Anything else (an over-closed buffer, same
///     end-result as the old guard; or a closer whose kind doesn't match
///     what was expected, e.g. an elisp `]` sitting where a `)` was
///     wanted and so parsed into its own `ERROR` node) is left uncolored
///     and `open` is untouched.
///   - EAGER POP: the instant a node's own subtree is fully walked (right
///     before `goto_next_sibling`, the same spot the old counter used to
///     inc/dec), every `open` entry whose parent is THAT node is dropped,
///     matched or not. This is the actual fix: an opener that never found
///     its sibling closer stops existing the moment its enclosing node's
///     subtree ends, so it can no longer inflate the depth of anything
///     that comes after -- including a later sibling defun/fn that
///     tree-sitter's error recovery placed outside the trouble entirely.
///
/// Why checking only the TOP of `open` is ever enough (never a scan): an
/// entry survives exactly as long as its parent's subtree is still open,
/// so at any point during the walk `open`'s parents form a
/// shallow-to-deep subsequence of the CURRENT node's own ancestor chain.
/// A closer's parent is the single deepest ancestor possible, so if a
/// live entry for it exists at all, it has to be the last one pushed --
/// the top. Multiple sibling pairs nested under one parent (`( [ ] )`)
/// resolve correctly for the same reason any stack-based matcher does.
/// And on a tree with no errors at all, this produces byte-for-byte the
/// same spans as the old linear counter (matching brackets being siblings
/// means stack depth and linear depth are the same number at every
/// point) -- which is exactly why every rainbow test already in this
/// file's test suite keeps passing unchanged; the divergence only shows
/// up once error recovery is involved, covered by this codebase's own
/// error-recovery fixtures (elisp and a second, structurally-distinct
/// language, mirroring the shape that motivated this fix).
/// `pairs` (M113, `show-paren-mode`): every (open_start_byte,
/// close_start_byte) this same walk actually matched, pushed at the
/// exact moment `top_matches` fires below -- the identical condition
/// under which this function already colors both members the same
/// rainbow depth. An opener that never finds its sibling closer (or is
/// eagerly popped, see the doc comment above) never appears here, which
/// is what makes `Engine::matching_pair` naturally report "no pair" for
/// an unmatched bracket with no extra bookkeeping.
///
/// ORDERING INVARIANT (review fix round): `pairs` comes out sorted by
/// CLOSE position ascending, because a pair is pushed at the exact
/// moment its closer is visited by this DFS walk, and the walk visits
/// every leaf token in left-to-right DOCUMENT order. `Engine::
/// matching_pair`'s `.find()` relies on this to break the tie at a
/// `"()()"`-style boundary (point sitting exactly between one pair's
/// closer and the next pair's opener, so BOTH pairs' adjacency
/// condition is satisfied) the same way real GNU Emacs does: the
/// earlier-closing pair wins. If this function is ever rewritten to
/// build `pairs` some other way (e.g. sorting by open position, or
/// collecting per-subtree and concatenating), that tie-break silently
/// flips unless the new code re-sorts by close position first --
/// pinned by `show_paren_tests.rs`'s
/// `tie_break_at_a_close_open_boundary_picks_the_earlier_closing_pair`.
fn collect_rainbow_spans(
    tree: &tree_sitter::Tree,
    out: &mut Vec<(usize, usize, &'static str, bool)>,
    pairs: &mut Vec<(usize, usize)>,
) {
    // (parent node id, expected closer kind, opener's own start byte,
    // whether the OPENER itself is a synthesized MISSING node) per
    // pending opener -- see the doc comment above. The third and fourth
    // fields are M113's addition, needed only to emit `pairs` once a
    // closer matches; the matching logic itself still keys off the
    // first two, unchanged from before M113.
    let mut open: Vec<(usize, &'static str, usize, bool)> = Vec::new();
    // Current node's ancestor-id chain (root-to-parent), hand-maintained
    // alongside the cursor walk below instead of calling `Node::parent()`.
    let mut ancestors: Vec<usize> = Vec::new();
    let mut cursor = tree.root_node().walk();
    loop {
        let node = cursor.node();
        if node.child_count() == 0 && !node.is_named() {
            let parent = ancestors.last().copied();
            match node.kind() {
                "(" | "[" | "{" => {
                    if let Some(parent) = parent {
                        open.push((
                            parent,
                            closer_for(node.kind()),
                            node.start_byte(),
                            node.is_missing(),
                        ));
                        out.push((
                            node.start_byte(),
                            node.end_byte(),
                            rainbow_face(open.len() as i32),
                            true,
                        ));
                    }
                }
                ")" | "]" | "}" => {
                    let top_matches = match (parent, open.last()) {
                        (Some(parent), Some(&(open_parent, expected, _, _))) => {
                            parent == open_parent && node.kind() == expected
                        }
                        _ => false,
                    };
                    if top_matches {
                        out.push((
                            node.start_byte(),
                            node.end_byte(),
                            rainbow_face(open.len() as i32),
                            true,
                        ));
                        let (_, _, open_start, open_missing) =
                            open.pop().expect("top_matches checked Some");
                        // M113 fix round: guard BOTH sides against a
                        // synthesized MISSING node, not just the closer.
                        // A zero-width node tree-sitter inserts to
                        // recover from an unbalanced structure still
                        // satisfies `top_matches` above, and rainbow-
                        // delimiters doesn't care which side it's on
                        // (the resulting span has start==end and
                        // `apply_visible` already drops zero-length
                        // spans, so it never renders either way). But
                        // `Engine::matching_pair` cares about `open`/
                        // `close` independently of each other: a real
                        // unmatched "(" with a MISSING ")" synthesized
                        // after it would otherwise get a phantom pair
                        // whose `open` field is its own real position
                        // (wrongly lighting up point-before-that-
                        // opener); symmetrically, a MISSING "(" tree-
                        // sitter might synthesize ahead of a real,
                        // otherwise-unmatched ")" would give that real
                        // closer a phantom partner at the synthesized
                        // opener's (also real-looking, non-empty range
                        // in byte terms, but never actually typed)
                        // position. Tested empirically (elisp, C, Rust,
                        // Verilog, a lone stray closer): tree-sitter
                        // wraps the orphan closer in its own `ERROR`
                        // node instead of synthesizing a MISSING opener
                        // for it, so the MISSING-opener side of this
                        // guard is not known to be reachable today in
                        // any of those four grammars -- kept anyway
                        // because nothing rules it out for the five
                        // grammars not tested this way (Python, Bash,
                        // Java, Perl, C++), and because guarding both
                        // sides is the only way to keep this correct
                        // without re-auditing every grammar's specific
                        // recovery behavior. Excluding either side's
                        // MISSING node from `pairs` (while leaving
                        // `out`/`open.pop()` untouched, so rainbow's own
                        // behavior is bit-for-bit unchanged) is what
                        // makes `matching_pair` return `None` for a real
                        // unmatched bracket, per this milestone's spec.
                        if !node.is_missing() && !open_missing {
                            pairs.push((open_start, node.start_byte()));
                        }
                    }
                }
                _ => {}
            }
        }
        if cursor.goto_first_child() {
            ancestors.push(node.id());
            continue;
        }
        loop {
            // Eager pop: `cursor.node()`'s whole subtree has now been
            // walked, so any opener whose parent is exactly this node can
            // never be matched by anything later -- see the doc comment
            // above for why this is the boundary that keeps a stray
            // opener's damage from spreading past its own enclosing node.
            let finished = cursor.node();
            while open.last().map(|&(p, _, _, _)| p) == Some(finished.id()) {
                open.pop();
            }
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
            ancestors.pop();
        }
    }
}

/// The specific closing-bracket kind an opener of `kind` (one of `(` `[`
/// `{`, the only three arms `collect_rainbow_spans` ever calls this with)
/// expects as its sibling-matched partner.
fn closer_for(kind: &str) -> &'static str {
    match kind {
        "(" => ")",
        "[" => "]",
        "{" => "}",
        _ => unreachable!("caller already matched kind against (, [, or {{"),
    }
}

/// Capture name → face, mirroring how GNU Emacs's treesit font-lock maps
/// standard capture names onto the classic font-lock faces. Unmapped
/// captures deliberately get nothing — highlight should inform, not
/// carnival.
///
/// M25 audited this against the real bundled (upstream) query of all
/// eight supported languages; M33 re-audits it against our own trimmed
/// queries (`crates/core/queries/*.scm`, one per language -- see each
/// file's header for what it kept/deleted/added) that replace upstream's
/// "tree-sitter ecosystem" style with GNU font-lock's own: only
/// definition/declaration sites get a face, and nothing is guessed from
/// a name's spelling. `capture_bases`/
/// `capture_coverage_audit_across_all_languages` in this file's `tests`
/// module enumerates every `@capture` base name that appears in each
/// language's *actual* query text and lists whichever aren't matched
/// here; M33 shrinks every language's unmapped set toward empty, since a
/// query built by hand (rather than inherited wholesale from upstream)
/// has no reason to introduce a capture name this table doesn't already
/// expect. Two names change meaning with this milestone:
///
///   - "variable" now actually appears (parameters and local/`let`/`my`-
///     style declarations across every trimmed query) and is mapped to
///     font-lock-variable-name-face; M25 deliberately left it unmapped
///     because upstream's *only* use of it was the blanket
///     `(identifier) @variable` catch-all this milestone deletes
///     everywhere.
///   - "builtin" is new, for elisp's `:keyword`-style symbols
///     (font-lock-builtin-face, a new face -- see themes.el).
///
/// Several M25-era arms -- bare "constructor", "char", "text", "number"/
/// "boolean"/"escape", "attribute"/"macro", and "property"/"field"/
/// "label" -- are no longer produced by any of the eight trimmed queries
/// (verified against the real `capture_bases` of all eight, not just
/// assumed). Kept anyway as reasonable generic vocabulary a future
/// grammar bump could plausibly reintroduce, rather than deleted and
/// re-added later; see the M33 report for the full per-arm accounting.
/// Bare "method" is the one M25-era synonym still genuinely exercised:
/// ts-parser-perl's own `method_declaration_statement` name capture
/// (Perl's `method` keyword, distinct from `sub`) uses it verbatim.
/// Capture base → face. Deliberately EXACTLY the vocabulary the eight
/// vendored queries use, nothing more (M33 review): a mapping kept
/// wider than the queries would let someone reintroduce, say, @number
/// without the per-language coverage audit forcing an explicit
/// decision — and numbers-stay-plain is font-lock philosophy, the
/// point of M33. New capture names must be added both to a query and
/// here (or to the audit's expected-unmapped set), on purpose.
fn face_for_capture(name: &str) -> Option<&'static str> {
    let base = name.split('.').next().unwrap_or(name);
    Some(match base {
        "function" | "method" | "constructor" => "font-lock-function-name-face",
        "keyword" | "conditional" | "repeat" | "include" | "exception" => "font-lock-keyword-face",
        "string" => "font-lock-string-face",
        "comment" => "font-lock-comment-face",
        "type" => "font-lock-type-face",
        "constant" => "font-lock-constant-face",
        "preproc" => "font-lock-preprocessor-face",
        "variable" => "font-lock-variable-name-face",
        "builtin" => "font-lock-builtin-face",
        _ => return None,
    })
}

/// One pump: harvest finished parses, send fresh snapshots where edits
/// have settled, and (re)materialize overlays for the visible region.
/// Called from `core::idle_tick`; cheap no-op when nothing is enabled.
pub fn tick(interp: &mut Interp, ed: &Rc<RefCell<crate::editor::Editor>>) {
    // take/put so a re-entrant tick (or a buffer borrow inside) can
    // never double-borrow the editor.
    let Some(mut engine) = ed.borrow_mut().hl.take() else {
        return;
    };
    engine_tick(interp, ed, &mut engine);
    ed.borrow_mut().hl = Some(engine);
}

fn engine_tick(interp: &mut Interp, ed: &Rc<RefCell<crate::editor::Editor>>, engine: &mut Engine) {
    // 1. Harvest completed parses.
    while let Ok(res) = engine.res_rx.try_recv() {
        if let Some(st) = engine.states.get_mut(&res.key) {
            if st.in_flight == Some(res.gen) {
                st.in_flight = None;
            }
            let newer = st
                .cached
                .as_ref()
                .map(|(g, _, _, _)| res.gen > *g)
                .unwrap_or(true);
            if newer {
                st.cached = Some((res.gen, res.spans, res.pairs, res.scopes));
            }
        }
    }

    // 2. Per buffer: debounce → snapshot → send; then apply if stale.
    let keys: Vec<usize> = engine.states.keys().copied().collect();
    for key in keys {
        let st = engine.states.get_mut(&key).expect("state exists");
        let Some(buffer) = st.buffer.upgrade() else {
            engine.states.remove(&key);
            continue;
        };
        let ticks = buffer.borrow().edit_ticks;
        if ticks != st.seen {
            st.seen = ticks;
            st.last_change = Instant::now();
        }
        let cached_gen = st.cached.as_ref().map(|(g, _, _, _)| *g);
        let needs_parse = cached_gen != Some(st.seen);
        if needs_parse && st.in_flight.is_none() && st.last_change.elapsed() >= DEBOUNCE {
            let text = buffer.borrow().text.to_string();
            st.in_flight = Some(st.seen);
            let _ = engine.job_tx.send(Job {
                key,
                gen: st.seen,
                lang: st.lang,
                text,
            });
        }
        apply_visible(interp, ed, st, &buffer);
    }
}

/// Materialize the cached spans intersecting the visible region (plus
/// margin) as overlays, replacing whatever this engine applied before.
/// Skipped entirely when generation and region are both unchanged.
fn apply_visible(
    interp: &mut Interp,
    ed: &Rc<RefCell<crate::editor::Editor>>,
    st: &mut BufState,
    buffer: &Rc<RefCell<Buffer>>,
) {
    let Some((gen, spans, _pairs, _scopes)) = &st.cached else {
        return;
    };
    let range = visible_range(ed, buffer);
    // M37: buffer-local `rainbow-delimiters-mode`, read the same
    // buffer-local-aware way `display-line-numbers` is at render time
    // (`redisplay.rs`'s `buffer_var_on`) — the worker thread ALWAYS
    // computes rainbow spans (see `collect_rainbow_spans`); only this
    // materialization step is conditional, so toggling the mode takes
    // effect on the very next tick with no reparse.
    let rainbow_on = {
        let editor = ed.borrow();
        crate::redisplay::buffer_var_on(interp, &editor, buffer, "rainbow-delimiters-mode")
    };
    // The applied range includes a full margin on each side, so scrolls
    // smaller than the margin keep the visible range inside it — free
    // hysteresis: no per-tick re-materialization at a range edge. Also
    // keyed on `rainbow_on` (M37): flipping `rainbow-delimiters-mode`
    // alone changes neither `gen` nor the visible range, so without this
    // the toggle would silently wait for one of those to move instead of
    // taking effect on the next tick as promised above.
    if let Some((applied_gen, (alo, ahi), applied_rainbow)) = st.applied {
        if applied_gen == *gen && range.0 >= alo && range.1 <= ahi && applied_rainbow == rainbow_on
        {
            return;
        }
    }
    let lo = range.0.saturating_sub(MARGIN_CHARS);
    let hi = range.1 + MARGIN_CHARS;

    let face_prop = interp.intern("face");
    let hl_prop = interp.intern("treesit-hl");
    let t = interp.syms.t;

    let mut b = buffer.borrow_mut();
    // Remove only what we own; org's overlays and the user's stay.
    b.retain_overlays(|ov| !ov.borrow().get(hl_prop).truthy());
    let weak = Rc::downgrade(buffer);
    for span in spans {
        if span.rainbow && !rainbow_on {
            continue;
        }
        if span.end < lo || span.start > hi || span.start >= span.end {
            continue;
        }
        let clamped_start = b.clamp(span.start as i64);
        let clamped_end = b.clamp(span.end as i64);
        if clamped_start >= clamped_end {
            continue;
        }
        let face_sym = interp.intern(span.face);
        let seq = b.alloc_overlay_seq();
        let ov = Rc::new(RefCell::new(OverlayData {
            buffer: weak.clone(),
            start: clamped_start,
            end: clamped_end,
            props: vec![(face_prop, Value::Sym(face_sym)), (hl_prop, Value::Sym(t))],
            seq,
        }));
        b.insert_overlay(ov);
    }
    st.applied = Some((*gen, (lo, hi), rainbow_on));
}

/// Union of the char ranges shown by every window displaying `buffer`.
fn visible_range(
    ed: &Rc<RefCell<crate::editor::Editor>>,
    buffer: &Rc<RefCell<Buffer>>,
) -> (usize, usize) {
    let editor = ed.borrow();
    let rows = editor.frame.1.max(5);
    let b = buffer.borrow();
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for win in editor.windows.values() {
        if !Rc::ptr_eq(&win.buffer, buffer) {
            continue;
        }
        let start = win.window_start.min(b.text.len());
        let mut end = start;
        for _ in 0..rows {
            end = b.text.line_end(end);
            if end >= b.text.len() {
                break;
            }
            end += 1;
        }
        lo = lo.min(start);
        hi = hi.max(end);
    }
    if lo == usize::MAX {
        // Not currently displayed: fall back to the whole buffer head so
        // tests (and freshly-opened background buffers) still highlight.
        (0, b.text.len().min(MARGIN_CHARS * 2))
    } else {
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::treesit::Lang;

    const ALL_LANGS: [Lang; 9] = [
        Lang::Rust,
        Lang::C,
        Lang::Cpp,
        Lang::Python,
        Lang::Bash,
        Lang::Java,
        Lang::Perl,
        Lang::Elisp,
        Lang::Verilog,
    ];

    /// Every distinct `@base` capture name (dotted suffixes stripped,
    /// exactly like `face_for_capture`'s own `split('.')`) that appears
    /// anywhere in `lang`'s real bundled highlight query -- the query
    /// text `lang_query` hands the worker thread, not a hand-copied
    /// approximation of it.
    fn capture_bases(lang: Lang) -> Vec<String> {
        let (ts_lang, query_src) = lang_query(lang);
        let query = tree_sitter::Query::new(&ts_lang, query_src)
            .unwrap_or_else(|e| panic!("{lang:?}: bundled query must compile: {e}"));
        let mut bases: Vec<String> = query
            .capture_names()
            .iter()
            .map(|n| n.split('.').next().unwrap_or(n).to_string())
            .collect();
        bases.sort();
        bases.dedup();
        bases
    }

    /// M25 audit, re-pinned at M33, extended at M38 (Verilog, the ninth
    /// language): for every supported language, which of its query's
    /// capture bases get no face at all from `face_for_capture`. Run with
    /// `--nocapture` to see the printed table. Also pinned to the exact
    /// set observed so a future grammar-crate bump (or hand-edit of one of
    /// these `.scm` files) that introduces a brand new capture name fails
    /// this test instead of silently going uncolored forever.
    ///
    /// At M25, every language's bundled (upstream) query left several
    /// capture bases unmapped by design -- operators, punctuation, and
    /// the blanket per-identifier `@variable` catch-all this whole
    /// milestone deletes. At M33, with all eight queries replaced by our
    /// own hand-trimmed files (`crates/core/queries/*.scm`), every base
    /// name each query actually produces has a deliberate
    /// `face_for_capture` mapping -- there was no reason to introduce an
    /// unmapped one when writing the query by hand, unlike inheriting
    /// upstream's wholesale. Hence every set below is empty; the table
    /// (and the per-language loop that checks it) is kept in the same
    /// shape as M25's rather than collapsed to a single assertion, so a
    /// future language-specific regression still prints exactly which
    /// language grew an unmapped capture.
    #[test]
    fn capture_coverage_audit_across_all_languages() {
        // (lang, expected capture bases with no face, sorted)
        let expected_unmapped: &[(Lang, &[&str])] = &[
            (Lang::Rust, &[]),
            (Lang::C, &[]),
            (Lang::Cpp, &[]),
            (Lang::Python, &[]),
            (Lang::Bash, &[]),
            (Lang::Java, &[]),
            (Lang::Perl, &[]),
            (Lang::Elisp, &[]),
            (Lang::Verilog, &[]),
        ];
        assert_eq!(
            expected_unmapped
                .iter()
                .map(|(l, _)| *l)
                .collect::<Vec<_>>(),
            ALL_LANGS,
            "keep `expected_unmapped` in lang_query/ALL_LANGS order"
        );

        let mut any_mismatch = false;
        for &(lang, expected) in expected_unmapped {
            let bases = capture_bases(lang);
            assert!(
                !bases.is_empty(),
                "{lang:?}: query exposed no captures at all"
            );
            let unmapped: Vec<&str> = bases
                .iter()
                .map(|s| s.as_str())
                .filter(|b| face_for_capture(b).is_none())
                .collect();
            println!(
                "{lang:?}: {} total capture bases, unmapped = {:?}",
                bases.len(),
                unmapped
            );
            if unmapped != expected {
                eprintln!(
                    "{lang:?}: unmapped mismatch\n  got:      {:?}\n  expected: {:?}",
                    unmapped, expected
                );
                any_mismatch = true;
            }
        }
        assert!(
            !any_mismatch,
            "capture coverage drifted from the M33 audit -- see stderr above for which \
             language and what changed; a new capture base name needs a deliberate \
             face_for_capture decision, not a silent pass-through"
        );
    }
}
