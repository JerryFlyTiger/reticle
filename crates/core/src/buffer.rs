use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use elisp::value::SymId;
use elisp::Value;

use crate::gapbuffer::GapBuffer;
use crate::treesit::{Lang, TsTreeData};

pub struct MarkerData {
    pub buffer: Weak<RefCell<Buffer>>,
    pub pos: usize,
}

pub struct OverlayData {
    pub buffer: Weak<RefCell<Buffer>>,
    pub start: usize,
    pub end: usize,
    pub props: Vec<(SymId, Value)>,
    /// Monotonic creation-order tiebreak, assigned by
    /// `Buffer::alloc_overlay_seq` when the overlay is made (P1.3).
    /// `overlays` is kept sorted by `start` rather than simply
    /// appended-to, so the vec's index can no longer stand in for
    /// creation order the way it used to; `seq` is what callers that
    /// need "later-created overlay wins" (real Emacs overlay-stacking
    /// semantics — see `OverlayStyleScan` and `style_at_naive` in
    /// `redisplay.rs`, and `get-text-property` in `builtins/ui.rs`)
    /// compare instead.
    pub seq: u64,
}

/// Walk an already-borrowed overlay's `props` (M52). Factored out so the
/// two call sites that need this — `trace_overlay` below (an overlay
/// reached directly as a `Value::Ext`) and `editor::trace_buffer` (an
/// overlay reached indirectly, via `Buffer.overlays`) — share one copy
/// instead of two independently-maintained copies of the same loop.
pub(crate) fn walk_overlay_props(o: &OverlayData, sink: &mut dyn FnMut(&Value)) {
    for (_, v) in &o.props {
        sink(v);
    }
}

/// GC tracer (M52): walk an overlay's `props`, so an overlay that
/// outlives `delete-overlay` (removed from `Buffer.overlays` but still
/// held by an elisp variable) keeps the values in its props alive.
pub fn trace_overlay(obj: &Rc<dyn std::any::Any>, sink: &mut dyn FnMut(&Value)) -> bool {
    let Some(ov) = obj.clone().downcast::<RefCell<OverlayData>>().ok() else {
        return true;
    };
    let Ok(o) = ov.try_borrow() else {
        return false;
    };
    walk_overlay_props(&o, sink);
    true
}

impl OverlayData {
    pub fn get(&self, prop: SymId) -> Value {
        self.props
            .iter()
            .find(|(p, _)| *p == prop)
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Nil)
    }

    pub fn put(&mut self, prop: SymId, val: Value) {
        if let Some(slot) = self.props.iter_mut().find(|(p, _)| *p == prop) {
            slot.1 = val;
        } else {
            self.props.push((prop, val));
        }
    }
}

/// One text change as it was actually applied, in the coordinates that
/// were current at the moment it happened. Emitted so callers above
/// `Buffer` can shift the positions they own -- `Buffer` adjusts its own
/// point/mark/markers/overlays itself (`adjust_positions_insert`/
/// `adjust_positions_delete` below), but `Window.point`/`window_start`
/// live on the `Editor`, which `Buffer` cannot see (see
/// `editor::adjust_windows_for_edit`, the sole consumer). Constructed by
/// three producers -- `editor::edit_insert`, `editor::edit_delete` (both
/// build one `AppliedEdit` from the char count `Buffer::insert`/`Buffer::
/// delete` return; `Buffer::insert`/`Buffer::delete` themselves never
/// construct one), and `Buffer::undo_step_from` below (one entry per
/// elementary edit it replays, M72 period 2) -- plus `erase-buffer`
/// (`builtins/buffers.rs`), which goes through `editor::edit_delete`
/// like any other buffer-clearing edit rather than calling `Buffer::
/// delete` directly. So there is exactly one shape for "what changed",
/// not a second copy hand-rolled at the window layer.
#[derive(Clone, Copy)]
pub enum AppliedEdit {
    Insert { pos: usize, n: usize },
    Delete { start: usize, n: usize },
}

#[derive(Clone)]
pub enum UndoEntry {
    /// Text was inserted at pos with this char length; undo deletes it.
    Insert {
        pos: usize,
        len: usize,
    },
    /// Text was deleted at pos; undo re-inserts it.
    Delete {
        pos: usize,
        text: String,
    },
    Boundary,
}

pub struct Buffer {
    pub name: String,
    pub text: GapBuffer,
    /// 0-based char offset.
    pub point: usize,
    pub mark: Option<usize>,
    pub mark_active: bool,
    pub modified: bool,
    pub file: Option<String>,
    pub undo: Vec<UndoEntry>,
    /// Where the next consecutive undo should consume from (index into
    /// `undo`); reset when a non-undo command intervenes. This is what
    /// makes consecutive undos keep going back instead of redoing.
    pub pending_undo: Option<usize>,
    /// True while an undo command is running (undo entries then act as redo).
    pub in_undo: bool,
    pub markers: Vec<Weak<RefCell<MarkerData>>>,
    /// Kept sorted by `start` at all times, ties broken by `seq` (i.e.
    /// creation order) — see `insert_overlay`/`delete_overlay`/
    /// `overlays_in` below. This is what lets `make-overlay`/
    /// `delete-overlay`/`overlays-in` binary-search instead of scanning
    /// every overlay in the buffer (P1.3): a single org-mode fontify
    /// pass calls the equivalent of `overlays-in` + `delete-overlay`
    /// once per line, and used to make each of those O(overlays in the
    /// buffer) — the overlay count itself grows with line count, so a
    /// whole-buffer fontify was O(lines) calls x O(overlays), trending
    /// toward O(lines²) on large files.
    ///
    /// `adjust_positions_insert`/`adjust_positions_delete` mutate every
    /// overlay's `start`/`end` in place without re-sorting — safe
    /// because both position-shift functions are monotonic
    /// non-decreasing maps (p1 <= p2 implies shifted(p1) <=
    /// shifted(p2)), so applying either to every overlay's `start` can
    /// never invert their relative order. See the comment on each.
    pub overlays: Vec<Rc<RefCell<OverlayData>>>,
    /// Source of the next `OverlayData::seq` value (see its doc).
    next_overlay_seq: u64,
    /// Prefix-max-end cache for `overlays_in`'s early-stop backward
    /// scan (P1.3 stage 2 — plain sorting plus the tail-cut in
    /// `overlays_in` alone measured as almost no improvement for
    /// org-mode's actual access pattern: fontifying line by line queries
    /// `overlays_in(bol, eol)` with `eol` growing monotonically, so
    /// nearly every existing overlay always has `start < eol` and the
    /// tail-cut discards almost nothing).
    ///
    /// `overlay_pme[i]` is the *index into `overlays`* (not the `end`
    /// value itself) of whichever entry among `overlays[0..=i]` has the
    /// largest `end`. Storing the index rather than the value is what
    /// makes this cheap to keep valid: `adjust_positions_insert`/
    /// `adjust_positions_delete` change every overlay's `end` value but
    /// never their order or count, and (by the same monotonic-shift
    /// argument as the sort invariant above) a monotonic map can't
    /// change *which* entry in a prefix holds the max — so those two
    /// functions don't need to touch this cache at all.
    ///
    /// `insert_overlay`/`delete_overlay` extend or pop this in O(1)
    /// when the change is at the tail (the common case — see their doc
    /// comments); anywhere else in the vector, maintaining it exactly
    /// would cost O(n) anyway, so they instead just flag
    /// `overlay_pme_dirty` and `overlays_in` rebuilds the whole thing
    /// (also O(n)) lazily the next time it's actually needed — so a
    /// burst of non-tail edits between two queries pays for one
    /// rebuild, not one per edit.
    overlay_pme: RefCell<Vec<usize>>,
    /// See `overlay_pme`.
    overlay_pme_dirty: Cell<bool>,
    /// Buffer-local variable values (swapped into symbol slots while current).
    pub locals: HashMap<SymId, Value>,
    pub keymap: Value,
    pub major_mode: Value,
    /// Column to keep during consecutive next-line/previous-line.
    pub goal_column: Option<usize>,
    /// M19: interp-visible mutations are refused while set (unless
    /// `inhibit-read-only` is bound non-nil). Internal machinery
    /// (undo replay, highlighting) is deliberately not affected.
    pub read_only: bool,
    /// M19: the directory for relative file operations started from
    /// this buffer (dired, eshell, C-x C-f prefill). Trailing slash.
    pub default_directory: Option<String>,
    /// Monotonic edit counter (M15): bumped by EVERY text mutation
    /// entry point — insert, delete, AND undo (which bypasses the first
    /// two, see `undo_step_from`). The background highlighter compares
    /// generations against this; a counter can't suffer the
    /// byte-accounting bugs that made us reject incremental
    /// `Tree::edit` reuse in M12 — any bump simply means "reparse".
    pub edit_ticks: u64,
    /// P1.2: cached full-buffer-text snapshot for search/regex builtins,
    /// keyed by the `edit_ticks` generation it was taken at — same
    /// staleness pattern as `highlight.rs`'s span cache. Search callers
    /// used to materialize the whole buffer with `text.slice(0,
    /// text.len())` on *every* call (an O(N) gap-buffer walk + UTF-8
    /// re-encode each time), which made line-at-a-time scans like
    /// org-mode fontify or isearch O(N) per line/keystroke, i.e. O(N^2)
    /// overall. `Rc<str>` (M43 period 2; was `Rc<Vec<char>>`) because
    /// that's what `elisp::regex`'s byte-indexed matcher now consumes —
    /// `text.to_string()` builds it via two gap-split `memcpy`s (no
    /// per-char re-encoding), and callers convert the byte positions the
    /// engine reports back to char positions via `GapBuffer::
    /// byte_to_char` (anchor-amortized) at the point they're stored into
    /// `MatchData`/`point`. A `RefCell` because `search_text` is called
    /// from `&self` (buffers are usually only reachable through
    /// `Rc<RefCell<Buffer>>` anyway, but builtins here take `&Buffer`).
    pub search_snapshot: RefCell<Option<(u64, Rc<str>)>>,
    /// M108: cached tree-sitter parse, keyed the same way as
    /// `search_snapshot` — `(edit_ticks generation, language, tree)`.
    /// This is NOT the incremental `Tree::edit` reuse `treesit.rs`'s
    /// module doc explains was rejected in M12 for correctness risk
    /// (byte-accounted edits threaded through insert/delete/undo, easy
    /// to get subtly wrong and corrupt node ranges without any visible
    /// symptom). This is the much narrower, much safer claim "the text
    /// hasn't changed since the last parse, so re-parsing from scratch
    /// would produce byte-for-byte the same tree" — same shape and same
    /// staleness key as `search_snapshot`, which already established
    /// the pattern for a different O(N)-per-call cost (there, every
    /// search call re-materializing the whole buffer; here, every
    /// highlight/indent/`treesit-*` call re-running the parser). Keying
    /// on `edit_ticks` means every text-mutation entry point counts as
    /// invalidating -- insert, delete, AND undo (which bypasses the
    /// first two; see `undo_step_from`) -- so there's no separate
    /// invalidation path to keep in sync by hand. The language is part
    /// of the key (not just an assumption) because nothing stops two
    /// different `treesit-parser-create` calls against the same buffer
    /// from naming different languages (e.g. an embedded-language
    /// experiment, or simply a stale parser object left over from
    /// before a major-mode change) -- caching only by generation would
    /// silently hand back a tree parsed under the wrong grammar.
    pub ts_tree: RefCell<Option<(u64, Lang, Rc<TsTreeData>)>>,
    /// M62 (local)/M75 (`/ssh:` remote): what's known about the on-disk
    /// file the last time it's known this buffer's contents matched it
    /// (or didn't exist yet) -- set when the file is read
    /// (`find-file-internal`) and refreshed after a successful write
    /// (`save-buffer`). `save-buffer` compares this against a fresh
    /// on-disk observation right before writing (a `metadata` call for
    /// local files, a `remote::read_file` reread for `/ssh:` ones) to
    /// detect an external change since the buffer was last known to
    /// agree with disk. This is independent of `modified`,
    /// which instead tracks buffer-vs-last-save (i.e. unsaved
    /// *in-editor* edits) -- the two can disagree in either direction
    /// (an unmodified buffer can still be stale if the file changed
    /// externally, and a modified buffer's `disk_state` still reflects
    /// the last save/read, not the pending edits).
    pub disk_state: DiskState,
    /// M62: the path AND on-disk state observed at the moment the last
    /// `save-buffer` was refused due to a conflict, if any. A subsequent
    /// `save-buffer` call on THIS buffer for the SAME path is treated as
    /// the user having already seen and confirmed past the warning --
    /// but ONLY if the disk is still in that exact observed state; if it
    /// changed again since the refusal (a second, unrelated external
    /// edit landing before the user retries), the ack no longer applies
    /// and the save is refused again with a fresh warning. Cleared on
    /// any successful write (of either kind). Deliberately per-buffer
    /// state, not global: `Editor::last_command` (the mechanism
    /// `save-buffers-kill-terminal`'s "invoke again" uses) is set by
    /// `finish_command` and only reflects the most recent command loop
    /// dispatch, but `save-buffer` can also run from a path that never
    /// touches the command loop (e.g. `M-.`'s async LSP callback
    /// switching buffers via `find-file`, or `idle_tick`'s
    /// `lsp-process-pending-all` -- see `lib.rs`/`lsp.el`) -- keying the
    /// "already confirmed" state off `last_command` would let a save
    /// confirmation on buffer A silently authorize an unrelated
    /// first-time save on buffer B if B's `save-buffer` happened to be
    /// the very next command dispatched. Keying it to this buffer's own
    /// path instead makes that cross-buffer leak structurally
    /// impossible.
    pub save_conflict_ack: Option<(String, DiskState)>,
}

/// See `Buffer::disk_state`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DiskState {
    /// (mtime, size) of the on-disk file the last time this buffer's
    /// contents are known to have matched it.
    Known(std::time::SystemTime, u64),
    /// The path didn't exist on disk when this buffer was created (a
    /// new file via `find-file-internal`'s NotFound branch). If the
    /// path exists by the time of a `save-buffer`, someone else created
    /// it first -- a conflict.
    Absent,
    /// No trustworthy baseline: `metadata` failed when it should have
    /// been recorded. `save-buffer` never treats this as a conflict -- a
    /// baseline-less save that sometimes false-positives would just
    /// train the user to reflexively force past the warning, defeating
    /// the whole point.
    Unknown,
    /// M75: content digest of a `/ssh:` remote file the last time this
    /// buffer's contents are known to have matched it (or `Absent` if it
    /// didn't exist remotely yet -- see the `Absent` variant above, same
    /// semantics). `remote.rs` has no single-file stat primitive (no
    /// portable mtime/size across whatever the remote host happens to
    /// run), so `Known`'s (mtime, size) pair isn't available for remote
    /// files. Three designs were tried before this one (recorded here so
    /// the next person doesn't re-derive and re-reject the same two
    /// dead ends):
    /// - Opaque `ls -ld` line as a fingerprint: dead. Measured on macOS,
    ///   `ls -l` timestamps only have minute granularity, so an edit that
    ///   changes content but not length within the same minute produces
    ///   a byte-identical `ls -ld` line -- silently defeats the exact
    ///   class of change this guard exists to catch, and a test written
    ///   without noticing the minute granularity would look green.
    /// - `stat` for mtime/size: dead. `stat -f ...` works on macOS,
    ///   `stat -c ...` on GNU/Linux, and they're mutually incompatible;
    ///   the remote host's flavor can't be inferred from the client's
    ///   (client is typically macOS, a Verilog build host is typically
    ///   Linux), so there's no single invocation that works everywhere.
    /// - `cksum` (POSIX, content-addressed): technically works and was
    ///   verified not to false-positive on `touch`/`chmod`, but rejected
    ///   anyway -- if the remote host lacks `cksum`, the guard would
    ///   silently fall back to no protection at all, which is exactly
    ///   the "looks fixed but isn't" failure mode this codebase treats
    ///   as worse than the original gap.
    ///
    /// So instead: reread the whole file via the existing
    /// `remote::read_file` (already has correct error semantics for a
    /// dead connection) and compare a same-process digest of its bytes.
    /// No assumption about the remote toolchain. Cost: one extra
    /// `read_file` call per remote save, which is exactly 2 more ssh
    /// round trips (its `test -e` probe, plus one of `cat`/a `true`
    /// liveness check depending on whether the probe found the file).
    ///
    /// Caveat inherited from `remote.rs`, not introduced here: `run()`
    /// decodes the remote stdout with `String::from_utf8_lossy`, so
    /// `read_file` (and therefore this digest) operates on a
    /// lossy-converted string, not the file's raw bytes. Two different
    /// invalid-UTF-8 byte sequences that get replaced by the same number
    /// of U+FFFD characters would digest identically -- a false
    /// negative (a real external change goes undetected). This is a
    /// pre-existing limitation of `remote.rs` (it was already lossy for
    /// display purposes before M75), but M75 is the first place that
    /// leans on it for content comparison rather than just showing text
    /// on screen, so the stakes are higher here. Low real-world impact
    /// for this project's primary use case (Verilog source is
    /// overwhelmingly ASCII), but worth knowing about rather than
    /// silently assuming byte-exact comparison.
    RemoteContent { digest: u64, len: u64 },
}

/// FNV-1a 64-bit over `bytes`. Deliberately NOT `std::hash::DefaultHasher`
/// (or `DefaultHasher`'s siblings): its exact algorithm and whether it's
/// seeded/randomized is an implementation detail with no cross-version
/// stability guarantee documented anywhere reachable without reading the
/// standard library source -- unsuitable for a value that gets stored and
/// compared later. FNV-1a is fixed by spec, so what it computes today is
/// what it computes tomorrow.
///
/// This is a change-detection digest, not a cryptographic hash: it's
/// sized and used only to notice "did the bytes change since I last read
/// them", not to resist a deliberate adversary constructing a collision.
pub fn content_digest(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

impl Buffer {
    pub fn new(name: &str, contents: &str) -> Buffer {
        Buffer {
            name: name.to_string(),
            text: GapBuffer::from_str(contents),
            point: 0,
            mark: None,
            mark_active: false,
            modified: false,
            file: None,
            undo: Vec::new(),
            pending_undo: None,
            in_undo: false,
            markers: Vec::new(),
            overlays: Vec::new(),
            next_overlay_seq: 0,
            overlay_pme: RefCell::new(Vec::new()),
            overlay_pme_dirty: Cell::new(false),
            locals: HashMap::new(),
            keymap: Value::Nil,
            major_mode: Value::Nil,
            goal_column: None,
            read_only: false,
            default_directory: None,
            edit_ticks: 0,
            search_snapshot: RefCell::new(None),
            ts_tree: RefCell::new(None),
            disk_state: DiskState::Unknown,
            save_conflict_ack: None,
        }
    }

    pub fn clamp(&self, pos: i64) -> usize {
        pos.max(0).min(self.text.len() as i64) as usize
    }

    /// Full-buffer-text snapshot for search/regex builtins, memoized
    /// against `edit_ticks` (see the field doc on `search_snapshot`).
    /// O(1) on a cache hit (an `Rc` clone); O(buffer size) to rebuild on
    /// the first call after an edit (two gap-split `memcpy`s via
    /// `GapBuffer::to_string`, M43 period 2 — no per-char UTF-8
    /// re-encoding).
    pub fn search_text(&self) -> Rc<str> {
        let mut cache = self.search_snapshot.borrow_mut();
        if let Some((gen, text)) = cache.as_ref() {
            if *gen == self.edit_ticks {
                return Rc::clone(text);
            }
        }
        let text: Rc<str> = Rc::from(self.text.to_string());
        *cache = Some((self.edit_ticks, Rc::clone(&text)));
        text
    }

    pub fn insert(&mut self, pos: usize, s: &str) -> usize {
        let n = self.text.insert(pos, s);
        if n == 0 {
            return 0;
        }
        self.record_undo(UndoEntry::Insert { pos, len: n });
        self.modified = true;
        self.edit_ticks += 1;
        self.adjust_positions_insert(pos, n);
        n
    }

    pub fn delete(&mut self, start: usize, end: usize) -> String {
        let removed = self.text.delete(start, end);
        if removed.is_empty() {
            return removed;
        }
        self.record_undo(UndoEntry::Delete {
            pos: start,
            text: removed.clone(),
        });
        self.modified = true;
        self.edit_ticks += 1;
        self.adjust_positions_delete(start, removed.chars().count());
        removed
    }

    fn record_undo(&mut self, entry: UndoEntry) {
        // Cap undo history so long sessions don't grow without bound.
        if self.undo.len() > 10_000 {
            self.undo.drain(..5_000);
        }
        self.undo.push(entry);
    }

    pub fn undo_boundary(&mut self) {
        if !matches!(self.undo.last(), Some(UndoEntry::Boundary) | None) {
            self.undo.push(UndoEntry::Boundary);
        }
    }

    /// Allocate the next creation-order tiebreak for a new overlay (see
    /// `OverlayData::seq`).
    pub fn alloc_overlay_seq(&mut self) -> u64 {
        let seq = self.next_overlay_seq;
        self.next_overlay_seq += 1;
        seq
    }

    /// Insert `ov` into `overlays`, maintaining the sort-by-`start`
    /// invariant. Ties (equal `start`) land after all existing entries
    /// with that `start` — i.e. in `seq`/creation order, matching what
    /// a plain `push` used to give for free.
    ///
    /// `partition_point` finds the insertion index in O(log n); the
    /// `Vec::insert` shift that follows is O(n) worst case, but O(1)
    /// amortized for the access pattern that actually matters here — a
    /// single org-mode fontify pass or tree-sitter re-highlight creates
    /// overlays at strictly increasing positions, so each new entry
    /// lands at (or very near) the tail.
    pub fn insert_overlay(&mut self, ov: Rc<RefCell<OverlayData>>) {
        let start = ov.borrow().start;
        let idx = self.overlays.partition_point(|o| o.borrow().start <= start);
        let is_append = idx == self.overlays.len();
        self.overlays.insert(idx, ov);
        if !is_append {
            self.overlay_pme_dirty.set(true);
        } else if !self.overlay_pme_dirty.get() {
            let mut pme = self.overlay_pme.borrow_mut();
            let end = self.overlays[idx].borrow().end;
            let prev_max = pme.last().copied();
            let carries_forward = prev_max
                .map(|p| self.overlays[p].borrow().end > end)
                .unwrap_or(false);
            pme.push(if carries_forward {
                prev_max.unwrap()
            } else {
                idx
            });
        }
    }

    /// Remove `ov` from `overlays` (a no-op if it's not there — mirrors
    /// the old `retain`-based `delete-overlay`, which silently ignored
    /// an overlay that was already gone). Binary-searches the run of
    /// entries sharing `ov`'s current `start` (O(log n)), then does the
    /// identity (`Rc::ptr_eq`) scan only within that run — O(log n + k)
    /// with k = overlays sharing that start, rather than O(n) across
    /// the whole buffer.
    pub fn delete_overlay(&mut self, ov: &Rc<RefCell<OverlayData>>) {
        let start = ov.borrow().start;
        let lo = self.overlays.partition_point(|o| o.borrow().start < start);
        let hi = self.overlays.partition_point(|o| o.borrow().start <= start);
        if let Some(rel) = self.overlays[lo..hi].iter().position(|o| Rc::ptr_eq(o, ov)) {
            let idx = lo + rel;
            let is_tail = idx + 1 == self.overlays.len();
            self.overlays.remove(idx);
            if is_tail {
                // The prefix up to (but not including) the removed tail
                // entry is exactly what it was before — just shorter by
                // the one entry that's gone.
                if !self.overlay_pme_dirty.get() {
                    self.overlay_pme.borrow_mut().pop();
                }
            } else {
                self.overlay_pme_dirty.set(true);
            }
        }
    }

    /// Drop every overlay matching a predicate — `remove-text-properties`
    /// (dropping ones whose props became empty) and the background
    /// highlighter's re-materialization pass (dropping its own stale
    /// spans) both need this "remove several, anywhere in the vector"
    /// shape, which plain `delete_overlay` doesn't cover.
    ///
    /// Bypassing `overlays.retain` directly here (instead of going
    /// through this method) would silently desync `overlay_pme` — it
    /// only knows to invalidate itself from `insert_overlay`/
    /// `delete_overlay`/`clear_overlays` — and the next `overlays_in`
    /// call would then index into it with stale, possibly out-of-bounds
    /// positions. Always flags dirty when anything was actually removed
    /// rather than trying to patch the cache up in place: an arbitrary
    /// subset can vanish from anywhere in the vector at once, so
    /// there's no cheap partial fix-up to do — this is not expected to
    /// be a hot path the way `make-overlay`/`delete-overlay`/
    /// `overlays-in` are (see the P1.3 report).
    ///
    /// `f` must only decide keep-or-drop, not mutate a surviving
    /// overlay's `start`/`end` — dirty-detection here only compares
    /// vec length before/after, so a predicate that both keeps an entry
    /// *and* changes its `end` would desync `overlay_pme`'s cached
    /// values without tripping this method's own staleness check.
    /// Neither current caller (`remove-text-properties`'s prop-emptiness
    /// check, the background highlighter's own-overlay filter) does
    /// that.
    pub fn retain_overlays<F>(&mut self, mut f: F)
    where
        F: FnMut(&Rc<RefCell<OverlayData>>) -> bool,
    {
        let before = self.overlays.len();
        self.overlays.retain(|ov| f(ov));
        if self.overlays.len() != before {
            self.overlay_pme_dirty.set(true);
        }
    }

    /// Drop every overlay in the buffer (`remove-overlays`). Resets
    /// `overlay_pme` directly rather than merely flagging it dirty:
    /// cheaper than a lazy rebuild since we already know the answer for
    /// an empty vector (empty), and it avoids leaving stale indices
    /// around if something inspects the cache before the next
    /// `overlays_in` call.
    pub fn clear_overlays(&mut self) {
        self.overlays.clear();
        self.overlay_pme.borrow_mut().clear();
        self.overlay_pme_dirty.set(false);
    }

    /// Rebuild `overlay_pme` from scratch if `insert_overlay`/
    /// `delete_overlay` flagged it stale. O(n); see `overlay_pme`'s doc
    /// for why this is only ever needed after a non-tail change.
    fn ensure_overlay_pme(&self) {
        if !self.overlay_pme_dirty.get() {
            // A plain (always-on, not debug_assert-only) check: this is
            // one `usize` comparison, immaterial next to the O(log n +
            // k) `overlays_in` call it guards, and it's exactly the
            // invariant a future bypass of insert_overlay/delete_overlay/
            // retain_overlays/clear_overlays would violate — panicking
            // here with a clear message beats indexing `overlay_pme`
            // with a stale, possibly out-of-bounds position further
            // down (see the P1.3 report for a real instance of this
            // exact bug, caught before it shipped).
            assert_eq!(
                self.overlay_pme.borrow().len(),
                self.overlays.len(),
                "overlay_pme out of sync with overlays while not marked dirty — some \
                 write path is mutating `overlays`' length without going through \
                 insert_overlay/delete_overlay/retain_overlays/clear_overlays"
            );
            return;
        }
        let mut pme = self.overlay_pme.borrow_mut();
        pme.clear();
        pme.reserve(self.overlays.len());
        let mut best = 0usize;
        let mut best_end = 0usize;
        for (i, ov) in self.overlays.iter().enumerate() {
            let end = ov.borrow().end;
            if i == 0 || end >= best_end {
                best = i;
                best_end = end;
            }
            pme.push(best);
        }
        self.overlay_pme_dirty.set(false);
    }

    /// Overlays intersecting `[s, e)` — Emacs's `overlays-in` semantics
    /// (`start < e && end > s`) — in `overlays`' current start-sorted
    /// order, which is NOT creation order (see `OverlayData::seq` for
    /// the one reader, `OverlayStyleScan`'s merge priority, that still
    /// needs creation order instead).
    ///
    /// `partition_point` first discards the tail of entries starting at
    /// or after `e` in O(log n). What's left is scanned *backward* from
    /// that cut point rather than forward from 0, using `overlay_pme`
    /// (see its doc) to stop as soon as the running max `end` over
    /// everything not yet visited drops to `<= s` — nothing further
    /// back could possibly satisfy `end > s` at that point. Worst case
    /// (many long, overlapping overlays) this still visits every
    /// candidate, same as the plain tail-cut; the case it actually
    /// targets is org-mode-shaped usage, where at any point in a
    /// forward fontify pass only a handful of recently-created overlays
    /// near the tail can possibly extend past `s` and everything older
    /// is already entirely behind it.
    pub fn overlays_in(&self, s: usize, e: usize) -> Vec<Rc<RefCell<OverlayData>>> {
        let cut = self.overlays.partition_point(|o| o.borrow().start < e);
        if cut == 0 {
            return Vec::new();
        }
        self.ensure_overlay_pme();
        let pme = self.overlay_pme.borrow();
        let mut result = Vec::new();
        let mut i = cut;
        loop {
            i -= 1;
            if self.overlays[i].borrow().end > s {
                result.push(self.overlays[i].clone());
            }
            if self.overlays[pme[i]].borrow().end <= s || i == 0 {
                break;
            }
        }
        result
    }

    /// Shift `point`/`mark`/`markers`/`overlays` positions for an
    /// `n`-char insertion at `pos`. Overlays keep their relative
    /// `start` order across this: the per-position shift here (`p ->
    /// p + n if p >= pos else p`) is monotonic non-decreasing — p1 <=
    /// p2 implies shifted(p1) <= shifted(p2) — so applying it to every
    /// overlay's `start` can only ever preserve or collapse their
    /// relative order, never invert it. That's what lets `overlays`
    /// stay sorted by `start` (see its doc on the `Buffer` struct)
    /// through this loop with no re-sort needed.
    fn adjust_positions_insert(&mut self, pos: usize, n: usize) {
        let shift = |p: &mut usize| {
            if *p >= pos {
                *p += n;
            }
        };
        shift(&mut self.point);
        if let Some(m) = &mut self.mark {
            shift(m);
        }
        self.markers.retain(|w| {
            if let Some(m) = w.upgrade() {
                let mut m = m.borrow_mut();
                if m.pos >= pos {
                    m.pos += n;
                }
                true
            } else {
                false
            }
        });
        for ov in &self.overlays {
            let mut ov = ov.borrow_mut();
            if ov.start >= pos {
                ov.start += n;
            }
            if ov.end > pos {
                ov.end += n;
            }
        }
    }

    /// Shift/collapse `point`/`mark`/`markers`/`overlays` positions for
    /// deleting `n` chars at `start`. Same sortedness argument as
    /// `adjust_positions_insert`: `shift` here (identity below `start`,
    /// collapsed to `start` inside the deleted range, `p - n` above it)
    /// is also monotonic non-decreasing, so it cannot invert the
    /// relative `start` order of any two overlays — `overlays` stays
    /// sorted through this loop with no re-sort needed.
    fn adjust_positions_delete(&mut self, start: usize, n: usize) {
        let end = start + n;
        let shift = |p: &mut usize| {
            if *p >= end {
                *p -= n;
            } else if *p > start {
                *p = start;
            }
        };
        shift(&mut self.point);
        if let Some(m) = &mut self.mark {
            shift(m);
        }
        self.markers.retain(|w| {
            if let Some(m) = w.upgrade() {
                let mut m = m.borrow_mut();
                if m.pos >= end {
                    m.pos -= n;
                } else if m.pos > start {
                    m.pos = start;
                }
                true
            } else {
                false
            }
        });
        self.overlays.retain(|ov| {
            let mut ov = ov.borrow_mut();
            let mut s = ov.start;
            let mut e = ov.end;
            shift(&mut s);
            shift(&mut e);
            ov.start = s;
            ov.end = e;
            true
        });
    }

    /// Apply one undo group ending at index `from` (exclusive), leaving the
    /// consumed entries in place and pushing the inverse operations on top
    /// as a new group (so re-doing works after an intervening command).
    ///
    /// Returns `Some((start, edits))` on success, or `None` if there was
    /// nothing left to undo. `start` is the index of the consumed group,
    /// for the next consecutive undo (unchanged from before M72 period 2).
    /// `edits` is every `AppliedEdit` this call actually applied, in the
    /// exact order it applied them (bundled into the same `Option` as
    /// `start`, rather than a separate out-param, since there is nothing
    /// meaningful to report in either half when the other is empty/None --
    /// a group is either undone as a whole, applying at least one edit,
    /// producing a `start`, or nothing happens at all).
    ///
    /// **Order matters and callers must preserve it.** A group is undone
    /// newest-entry-first (the `.rev()` below undoes it in the reverse of
    /// how it was originally typed/applied), and each entry's `pos`/
    /// `start` is a coordinate in the buffer as it stood immediately
    /// after the previous entry in `edits` was applied -- exactly the
    /// same coordinate space `self.point`/`self.mark`/`self.markers`/
    /// `self.overlays` get shifted through one `adjust_positions_*` call
    /// per entry, below. A caller shifting its OWN positions (window
    /// point/window_start -- see `editor::adjust_windows_for_edit`) must
    /// therefore also apply `edits` one at a time, in this order, not
    /// batch them or apply them out of order -- doing so would shift a
    /// later entry's window positions using an earlier entry's
    /// now-stale coordinates.
    pub fn undo_step_from(&mut self, from: usize) -> Option<(usize, Vec<AppliedEdit>)> {
        let mut idx = from.min(self.undo.len());
        while idx > 0 && matches!(self.undo[idx - 1], UndoEntry::Boundary) {
            idx -= 1;
        }
        if idx == 0 {
            return None;
        }
        let mut start = idx;
        while start > 0 && !matches!(self.undo[start - 1], UndoEntry::Boundary) {
            start -= 1;
        }
        self.in_undo = true;
        let group: Vec<UndoEntry> = self.undo[start..idx].to_vec();
        let mut redo = Vec::new();
        let mut applied = Vec::new();
        for entry in group.into_iter().rev() {
            match entry {
                UndoEntry::Insert { pos, len } => {
                    let removed = self.text.delete(pos, pos + len);
                    self.adjust_positions_delete(pos, len);
                    self.point = self.clamp(pos as i64);
                    applied.push(AppliedEdit::Delete { start: pos, n: len });
                    redo.push(UndoEntry::Delete { pos, text: removed });
                }
                UndoEntry::Delete { pos, text } => {
                    let n = self.text.insert(pos, &text);
                    self.adjust_positions_insert(pos, n);
                    self.point = self.clamp((pos + n) as i64);
                    applied.push(AppliedEdit::Insert { pos, n });
                    redo.push(UndoEntry::Insert { pos, len: n });
                }
                UndoEntry::Boundary => {}
            }
        }
        self.in_undo = false;
        if !matches!(self.undo.last(), Some(UndoEntry::Boundary) | None) {
            self.undo.push(UndoEntry::Boundary);
        }
        self.undo.extend(redo);
        self.modified = true;
        self.edit_ticks += 1;
        Some((start, applied))
    }
}
