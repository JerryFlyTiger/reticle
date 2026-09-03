use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use elisp::value::SymId;
use elisp::{Interp, Value};

use crate::buffer::{walk_overlay_props, AppliedEdit, Buffer};
use crate::commands::{Key, PendingArgs};
use crate::redisplay::Style;

pub const BUFFER_TAG: &str = "buffer";

/// GC tracer (M52): walk a buffer's keymap/major-mode/locals/overlay-props
/// so a buffer reachable only through an elisp `Value` (e.g. one still
/// held by a variable after `kill-buffer`, or referenced by an
/// `emulation-keymap`) marks everything it in turn holds. Mirrors the
/// `editor.buffers` root-provider walk in `lib.rs` — that one covers
/// buffers reachable via the editor itself, this one covers buffers
/// reachable only via an elisp value; re-emitting the same values from
/// both paths is harmless since marking is idempotent (`visited`-gated).
fn trace_buffer(obj: &Rc<dyn std::any::Any>, sink: &mut dyn FnMut(&Value)) -> bool {
    let Some(buf) = obj.clone().downcast::<RefCell<Buffer>>().ok() else {
        return true;
    };
    let Ok(b) = buf.try_borrow() else {
        return false;
    };
    sink(&b.keymap);
    sink(&b.major_mode);
    for v in b.locals.values() {
        sink(v);
    }
    for ov in &b.overlays {
        let Ok(o) = ov.try_borrow() else {
            return false;
        };
        walk_overlay_props(&o, sink);
    }
    true
}

pub struct Minibuffer {
    pub prompt: String,
    pub input: String,
    /// 0-based char offset into input.
    pub cursor: usize,
    pub pending: PendingArgs,
    /// Transient status shown after the input (" [No match]" etc.),
    /// cleared on the next key.
    pub note: Option<String>,
    /// Open completion popup (M18), if any. F3 correction (M84 fix
    /// round): this comment used to say "still used for M-x
    /// (command/function/symbol sources)" -- that stopped being true
    /// once M84 routed Command/Function/Symbol onto `panel` below
    /// instead (the M-x type-as-you-filter milestone). No `Source`
    /// variant reaches the code that populates this field anymore
    /// (`commands::minibuffer_tab`'s popup-building tail is dead code
    /// for the same reason -- see its own doc comment); the field and
    /// `CompletionState` are kept rather than removed since that's a
    /// bigger cleanup than the fix that caught this comment being wrong.
    pub completion: Option<CompletionState>,
    /// Bottom selector panel (M21) for file and buffer sources —
    /// opens with the minibuffer, occupies the bottom third.
    pub panel: Option<PanelState>,
    /// M47 Part D: index into `Editor::
    /// minibuffer_history[spec.history_key]` currently showing, or
    /// `None` at the live edge (no M-p yet, or M-n walked back past the
    /// newest entry) — see `commands::minibuffer_history_prev`/`_next`.
    pub hist_pos: Option<usize>,
    /// M47 Part D: `input` as it stood the moment M-p first left the
    /// live edge — what M-n restores when it walks back past the most
    /// recent history entry, so a partially-typed prompt isn't lost by
    /// browsing history and coming back.
    pub hist_stash: String,
}

/// One row of the M21 selector panel.
pub struct PanelRow {
    /// Text inserted into the input on accept (files: basename, with a
    /// trailing `/` for directories).
    pub accept: String,
    /// Styled display segments as (text, face-name) — resolved via
    /// face_or at render time so themes recolor the panel live.
    pub segments: Vec<(String, &'static str)>,
}

/// The M21 bottom selector panel.
pub struct PanelState {
    pub rows: Vec<PanelRow>,
    /// Index of the highlighted row.
    pub selected: usize,
    /// True once the user has moved the selection (arrows / TAB cycle);
    /// for file prompts, RET with an untouched selection and an empty
    /// stem submits the literal input (C-x C-f RET on a directory =
    /// dired) instead of grabbing the first row.
    pub chosen: bool,
    /// Char offset in the minibuffer input where the completed segment
    /// starts (same semantics as CompletionState::stem_start).
    pub stem_start: usize,
    pub source: crate::complete::Source,
}

/// The candidate popup opened by TAB in the minibuffer (M18).
pub struct CompletionState {
    /// Sorted candidate strings; file candidates are basenames with a
    /// trailing `/` for directories.
    pub candidates: Vec<String>,
    /// Index of the highlighted candidate.
    pub selected: usize,
    /// Char offset in `input` where the completed segment starts (0 for
    /// symbols/buffers, right after the last `/` for files); accepting
    /// a candidate replaces `input[stem_start..]`.
    pub stem_start: usize,
}

/// One completion candidate as it reaches the popup (M44-3, `start`'s
/// role narrowed by the M44 review fix #1): `label` is what's drawn;
/// `insert` is what gets typed on accept; `start` is THIS candidate's
/// own buffer char position to delete back to before inserting `insert`
/// — an LSP `textEdit`'s range can start earlier than another candidate
/// in the same list (member completion replacing "obj.", say), so this
/// is per-item rather than the popup-wide `start` M40-4 originally had.
/// `start` is used ONLY by `accept_completion`'s deletion range now —
/// as-you-type filtering (`commands::refilter_completion_popup`) matches
/// every item's `filter` against the POPUP-WIDE `CompletionPopup::
/// prefix_start`..point span instead (see its doc comment for why:
/// `filterText` usually excludes whatever a `textEdit` replaces ahead of
/// the ordinary identifier prefix). `filter` is the text (`filterText`,
/// or `label` when absent — `lsp.el`'s `lsp--completion-item-filter-
/// text`) that span is prefix-matched against.
#[derive(Clone)]
pub struct PopupItem {
    pub label: String,
    pub insert: String,
    pub start: usize,
    pub filter: String,
}

/// The cursor-anchored LSP completion popup (M40-4, extended M44-3),
/// opened by `show-completion-popup` from `lsp.el`'s
/// `lsp-completion-at-point` callback. Unlike `CompletionState` above (a
/// minibuffer input's TAB popup, keyed off a char offset into
/// `Minibuffer::input`), this one lives against the CURRENT BUFFER's
/// own text — each item carries its OWN buffer char position to replace
/// (see `PopupItem`), not a single popup-wide one, and accepting a
/// candidate edits the buffer directly.
pub struct CompletionPopup {
    /// Candidates, already sorted (`lsp.el`'s own `sortText` sort, see
    /// its file header) and filtered as of the last time this popup was
    /// opened or refiltered — see `commands::refilter_completion_popup`,
    /// which narrows this further as the user keeps typing.
    pub items: Vec<PopupItem>,
    /// The identifier-run prefix start (M44 review fix #1) -- what
    /// `refilter_completion_popup` slices `buffer[prefix_start..point)`
    /// against to get the "typed" span every item's `filter` is
    /// prefix-matched against, and what a `point < prefix_start` check
    /// closes the popup on. Deliberately POPUP-WIDE, not per-item: a
    /// `filterText` (or `label` fallback) from the server almost never
    /// includes whatever a `textEdit`-bearing item's own `start` sits
    /// before (e.g. rust-analyzer's postfix completions replace "obj."
    /// but filter on just "if"/"match"/"unwrap" -- see the file's own
    /// `lsp--completion-item-filter-text` in lsp.el) -- comparing typed
    /// text anchored at each item's OWN `start` against that filter, as
    /// M44-3 originally did, made a whole class of real-world candidates
    /// fail to prefix-match and silently vanish. `PopupItem::start`
    /// keeps its per-item meaning for `accept_completion`'s deletion
    /// range (a `textEdit` can legitimately replace more than the
    /// popup's own prefix) -- only the FILTER anchor moved back to being
    /// popup-wide, matching `lsp.el`'s own pre-M44-3 semantics.
    pub prefix_start: usize,
    /// Index of the highlighted candidate.
    pub selected: usize,
    /// From the server's `CompletionList.isIncomplete` (M44-3): true
    /// means the server only answered with a partial candidate set for
    /// the prefix it saw and expects a fresh request once more is
    /// typed — `refilter_completion_popup` re-requests instead of
    /// narrowing this list further when this is set and the buffer has
    /// changed since.
    pub incomplete: bool,
    /// `Buffer::edit_ticks` as of when this popup was last opened or
    /// refiltered (M44-3) — `refilter_completion_popup`'s baseline for
    /// telling "the buffer changed, refilter" apart from "only point
    /// moved, close" (vim/VS Code convention).
    pub tick: u64,
}

/// One window pane. `window_start` and `point` are the saved values used
/// while the window is not selected (the selected window's point lives in
/// its buffer) -- both are kept in sync with every edit to `buffer` via
/// `adjust_windows_for_edit`, whichever window is selected at the time.
pub struct Window {
    pub buffer: Rc<RefCell<Buffer>>,
    pub point: usize,
    pub window_start: usize,
    /// Fix 3 (mouse-support milestone review): the point value observed at
    /// the moment this window was last scrolled explicitly (mouse wheel,
    /// `scroll_window_start`), or `None` if it hasn't been / the pin has
    /// been consumed. While `Some(p)` and the window's current point is
    /// still exactly `p`, `render_window` skips `ensure_point_visible`'s
    /// recentre for this window -- matching GNU Emacs: scrolling moves the
    /// view without moving point, and point is allowed to sit outside the
    /// visible region until a command that actually moves point runs. The
    /// pin is per-window and self-clearing: it is read and compared at
    /// render time (not written by every point-mutating call site), so
    /// scrolling window A and then typing in window B leaves A's pin
    /// intact (A's point hasn't changed), while typing in A itself changes
    /// A's point away from the pinned value and the very next render sees
    /// the mismatch, drops the pin, and lets `ensure_point_visible`
    /// recentre A normally.
    pub scroll_pin: Option<usize>,
}

/// Binary window layout tree; leaves index into `Editor::windows`.
pub enum Layout {
    Leaf(usize),
    Split {
        horizontal: bool,
        a: Box<Layout>,
        b: Box<Layout>,
    },
}

impl Layout {
    pub fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Layout::Leaf(id) => out.push(*id),
            Layout::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    /// Replace leaf `id` with `new`; returns true if found.
    pub fn replace_leaf(&mut self, id: usize, new: Layout) -> bool {
        self.replace_leaf_opt(id, &mut Some(new))
    }

    fn replace_leaf_opt(&mut self, id: usize, new: &mut Option<Layout>) -> bool {
        match self {
            Layout::Leaf(l) if *l == id => {
                if let Some(n) = new.take() {
                    *self = n;
                }
                true
            }
            Layout::Leaf(_) => false,
            Layout::Split { a, b, .. } => {
                a.replace_leaf_opt(id, new) || b.replace_leaf_opt(id, new)
            }
        }
    }

    /// Remove leaf `id`, collapsing its parent split into the sibling.
    pub fn remove_leaf(&mut self, id: usize) -> bool {
        if let Layout::Split { a, b, .. } = self {
            if matches!(**a, Layout::Leaf(l) if l == id) {
                *self = std::mem::replace(b, Layout::Leaf(0));
                return true;
            }
            if matches!(**b, Layout::Leaf(l) if l == id) {
                *self = std::mem::replace(a, Layout::Leaf(0));
                return true;
            }
            return a.remove_leaf(id) || b.remove_leaf(id);
        }
        false
    }
}

/// Incremental search state (C-s / C-r).
pub struct Isearch {
    pub query: String,
    /// The buffer this session was started against. `start`/`origin`/
    /// `origin_byte` below are char/byte offsets into THIS buffer, not
    /// necessarily whatever buffer is current when a later keystroke
    /// arrives (M53: elisp can `switch-to-buffer`/`kill-buffer` mid-
    /// session). `Weak`, not `Rc`, mirroring `MarkerData`/`OverlayData`
    /// in `buffer.rs`: a live search session shouldn't keep an otherwise
    /// dead (killed) buffer alive.
    pub buffer: Weak<RefCell<Buffer>>,
    /// Point when the whole search session started; C-g reverts here.
    pub start: usize,
    /// Current search anchor: every keystroke re-searches from here.
    /// Equals `start` until a repeat (C-s/C-r) advances past a match.
    pub origin: usize,
    /// Byte offset of `origin` in the buffer text, cached alongside it
    /// (M43 period 2). `origin` only changes on session start, an
    /// empty-query reset, or a repeat (`isearch_start`/`isearch_step`'s
    /// empty branch/`isearch_repeat` — never on ordinary typing), so
    /// this is recomputed via `GapBuffer::char_to_byte` only there.
    ///
    /// This matters more than it looks: `isearch_step`'s per-keystroke
    /// search does `find_forward(origin_byte) then byte_to_char(match)`.
    /// If `origin_byte` were instead recomputed fresh every keystroke
    /// (`char_to_byte(origin)`), that call and the `byte_to_char` call
    /// on the match position would fight over the buffer's single
    /// `GapBuffer` anchor every single keystroke: `origin` sits at the
    /// search's fixed starting point while the match can be far away
    /// (e.g. near the end of a 10MB buffer) — so each keystroke would
    /// yank the anchor from "near origin" to "near the match" and back,
    /// both potentially O(buffer size), defeating the "isearch stays
    /// O(delta) per keystroke" amortization the M43 design counts on
    /// (§3.4). Caching `origin_byte` removes the "back to origin" half
    /// of that ping-pong: once the anchor settles near the match after
    /// the first keystroke, later keystrokes' `byte_to_char` calls find
    /// it already there (the match position is stable or moves only
    /// slightly as the query grows), so they're cheap.
    pub origin_byte: usize,
    pub forward: bool,
    pub failed: bool,
}

pub struct Editor {
    pub buffers: Vec<Rc<RefCell<Buffer>>>,
    pub current: Rc<RefCell<Buffer>>,
    /// Window panes by id, arranged by `layout`.
    pub windows: HashMap<usize, Window>,
    pub layout: Layout,
    pub selected_window: usize,
    pub next_window_id: usize,
    pub isearch: Option<Isearch>,
    /// M30: the most recent NON-EMPTY isearch query to commit (RET, or any
    /// key that ends the search keeping the match -- see `isearch_exit`),
    /// paired with its direction -- survives past the session itself
    /// (unlike `isearch`, which is dropped when the session ends) so
    /// evil's `n`/`N` can repeat it. Not updated by `isearch_cancel`
    /// (C-g): an aborted search was never really "performed". Exposed to
    /// elisp via the `isearch-last-string`/`isearch-last-forward-p`
    /// builtins (see builtins/ui.rs) -- there is no other way for elisp
    /// to learn the query, active or ended (only `isearch-start`/
    /// `isearch-active-p` existed before M30).
    pub last_search: Option<(String, bool)>,
    pub kill_ring: Vec<String>,
    pub kill_ring_yank: usize,
    /// Position and length of the last yank, for yank-pop.
    pub last_yank: Option<(usize, usize)>,
    pub last_command: Value,
    pub this_command: Value,
    /// Global values saved while the current buffer's locals are installed.
    pub swapped_globals: HashMap<SymId, Option<Value>>,
    pub global_keymap: Value,
    /// Frame size in character cells (cols, rows).
    pub frame: (usize, usize),
    pub pending_keys: Vec<Key>,
    pub minibuffer: Option<Minibuffer>,
    pub echo: Option<String>,
    /// Consecutive self-insert count, for undo-boundary grouping.
    pub consec_inserts: usize,
    /// M30: one-shot suppression of the boundary `self_insert` would
    /// otherwise insert before the FIRST self-inserted character
    /// (`consec_inserts` having just been reset to 0 by the previous
    /// command's `execute_command` preamble) -- set by the
    /// `undo-amalgamate-boundary` builtin, consumed (cleared
    /// unconditionally) the next time either `self_insert` checks it or
    /// `execute_command` runs, whichever comes first. This is what lets
    /// evil's c-family operators and `o`/`O` merge their initial
    /// delete/newline with the text subsequently typed into ONE undo
    /// group instead of two -- see evil.el's `evil--operator-apply' and
    /// `evil-open-below'/`evil-open-above'.
    pub suppress_next_undo_boundary: bool,
    /// M28 `capture-next-key`: when set, the very next key event skips all
    /// keymap dispatch and is handed straight to this function as its
    /// elisp representation (see `commands::key_to_value`). Cleared right
    /// before the function runs (not after), so the function can re-arm
    /// it for the following key — the basis for evil's `f`/`t`-then-`;`
    /// character search.
    pub key_capture: Option<Value>,
    /// M42-II: `Some((REG, KEYS))` while a keyboard macro is being
    /// recorded (`start-kbd-macro`, evil's `q` "arm" branch) -- REG is
    /// the register character (raw i64, matching `key_to_value`'s own
    /// `Key::Char` representation) it will be stored under once
    /// recording ends; KEYS accumulates every key `handle_key` sees
    /// while this is armed and `macro_replay_depth` is 0 (see the
    /// recording tap in `handle_key` itself) -- replayed keys are never
    /// re-recorded, matching vim's own semantics (`qa...q` records the
    /// two keystrokes that invoke `@a` later, never `@a`'s own
    /// expansion). `None` when nothing is currently recording.
    pub kbd_macro_recording: Option<(i64, Vec<Key>)>,
    /// M42-II: completed keyboard macros by register (a-z, stored as
    /// the same raw i64 char value as `kbd_macro_recording`'s REG).
    /// Written by the `end-kbd-macro` builtin, read by
    /// `execute-kbd-macro` (both builtins/ui.rs).
    pub kbd_macros: HashMap<i64, Vec<Key>>,
    /// M42-II: re-entrancy depth for `execute-kbd-macro` -- 0 outside
    /// any replay; incremented before feeding a macro's keys back
    /// through `handle_key` and restored via a drop-guard (so an elisp
    /// error mid-replay can't leave it stuck above 0 and silently
    /// disable further recording/nested replay for the rest of the
    /// session), capped at 32 as the `@@` self-recursion backstop. Also
    /// gates the recording tap above: a macro replaying INSIDE another
    /// macro's own recording must not double-record the inner macro's
    /// expansion, only the `@x` keystrokes that invoked it.
    pub macro_replay_depth: u32,
    /// Named face styles (face symbol → style).
    pub faces: HashMap<SymId, Style>,
    pub quit: bool,
    /// M15 hook watchdog: timeout strikes per (hook, function), keyed by
    /// the function's printed representation. Three strikes and the
    /// function is removed from the hook — a slow package degrades its
    /// own feature instead of everyone's typing latency.
    pub hook_offenses: HashMap<String, u32>,
    /// Background highlight engine (M15, see `crate::highlight`).
    /// Spawned lazily on the first `treesit-highlight-mode`.
    pub hl: Option<crate::highlight::Engine>,
    /// LSP diagnostics per buffer (M16): buffer pointer → (0-based line,
    /// severity 1=error/2=warning/3+=info, message text). Feeds the
    /// gutter dots, the modeline count, and (M87 stage 3) the inline
    /// diagnostic block rows the renderer draws under the offending
    /// line; the squiggle overlays live on the buffer itself (set from
    /// lsp.el). Removed on buffer kill (`builtins::buffers::kill-buffer`)
    /// -- this map is keyed by `Rc::as_ptr`, which a later unrelated `Rc`
    /// can reuse once the old buffer is freed.
    pub diagnostics: HashMap<usize, Vec<(usize, u8, String)>>,
    /// Floating hover text (M16): set by `show-hover-popup` (the async
    /// LSP hover callback uses it), drawn by the GUI as a popup at the
    /// cursor and echoed in the TUI. Cleared on the next keystroke.
    pub hover_popup: Option<String>,
    /// Open LSP completion popup (M40-4), if any — see `CompletionPopup`.
    /// Unlike `hover_popup`, NOT cleared by `handle_key`'s blanket clear:
    /// it has its own key-routing branch in `commands::handle_key` (a
    /// dedicated close on ESC/C-g/any other key), since a blanket wipe
    /// every keystroke would defeat C-n/C-p navigating it at all.
    pub completion_popup: Option<CompletionPopup>,
    /// M47 Part D: minibuffer input history, keyed by `ArgSpec::
    /// history_key` so unrelated prompts (a search string vs. a symbol
    /// name, say) don't share a ring. Most-recent entry last; capped at
    /// 100 per key (oldest dropped) by `commands::push_minibuffer_history`,
    /// the sole writer. M-p/M-n (`commands::minibuffer_key`) walk this
    /// via `Minibuffer::hist_pos`.
    pub minibuffer_history: HashMap<String, Vec<String>>,
    /// M60: whether the one-time "Background output captured in
    /// *background-output*" echo-area message has already fired this
    /// session. `crate::idle_tick` sets this the first time it drains a
    /// non-empty batch from `elisp::bglog` and never messages again, so
    /// a chatty background source (an LSP server logging steadily) does
    /// not spam the echo area on every idle tick.
    pub background_output_notified: bool,
}

impl Editor {
    pub fn new() -> Editor {
        let scratch = Rc::new(RefCell::new(Buffer::new("*scratch*", "")));
        let mut windows = HashMap::new();
        windows.insert(
            0,
            Window {
                buffer: scratch.clone(),
                point: 0,
                window_start: 0,
                scroll_pin: None,
            },
        );
        Editor {
            buffers: vec![scratch.clone()],
            current: scratch,
            windows,
            layout: Layout::Leaf(0),
            selected_window: 0,
            next_window_id: 1,
            isearch: None,
            last_search: None,
            kill_ring: Vec::new(),
            kill_ring_yank: 0,
            last_yank: None,
            last_command: Value::Nil,
            this_command: Value::Nil,
            swapped_globals: HashMap::new(),
            global_keymap: Value::Nil,
            frame: (80, 24),
            pending_keys: Vec::new(),
            minibuffer: None,
            echo: None,
            consec_inserts: 0,
            suppress_next_undo_boundary: false,
            key_capture: None,
            kbd_macro_recording: None,
            kbd_macros: HashMap::new(),
            macro_replay_depth: 0,
            faces: HashMap::new(),
            quit: false,
            hook_offenses: HashMap::new(),
            hl: None,
            diagnostics: HashMap::new(),
            hover_popup: None,
            completion_popup: None,
            minibuffer_history: HashMap::new(),
            background_output_notified: false,
        }
    }

    pub fn find_buffer(&self, name: &str) -> Option<Rc<RefCell<Buffer>>> {
        self.buffers
            .iter()
            .find(|b| b.borrow().name == name)
            .cloned()
    }

    /// Whether there's an isearch session AND it's still valid for the
    /// current buffer (M53). A session becomes stale -- but is not
    /// eagerly torn down -- when elisp swaps the current buffer out from
    /// under it (`switch-to-buffer`, `kill-buffer`, ...); see the long
    /// comment at the call site in `commands::handle_key` for why
    /// invalidation is checked lazily, here, rather than at every buffer-
    /// switching call site. Every reader of `self.isearch` that cares
    /// whether the session is actually live for the CURRENT buffer
    /// (`handle_key`'s own interception point, `isearch-active-p`, the
    /// completion-popup modal-session guard) must go through this rather
    /// than reading `self.isearch.is_some()` directly, or it'll see a
    /// stale session during the window between the buffer swap and the
    /// next key that would otherwise clear it out.
    ///
    /// Read-only: does not clear a stale `self.isearch` itself (`&self`,
    /// not `&mut self`) -- `handle_key` is the one place responsible for
    /// actually clearing it out, right before dispatching the key.
    pub fn isearch_live(&self) -> bool {
        self.isearch
            .as_ref()
            .is_some_and(|s| matches!(s.buffer.upgrade(), Some(b) if Rc::ptr_eq(&b, &self.current)))
    }

    pub fn buffer_value(buf: &Rc<RefCell<Buffer>>) -> Value {
        Value::Ext(elisp::value::ExtRef {
            tag: BUFFER_TAG,
            obj: buf.clone() as Rc<dyn std::any::Any>,
            trace: Some(trace_buffer),
        })
    }

    pub fn kill_new(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.kill_ring.push(text);
        if self.kill_ring.len() > 60 {
            self.kill_ring.remove(0);
        }
        self.kill_ring_yank = self.kill_ring.len() - 1;
    }

    /// Append to the most recent kill (consecutive kill-line etc.).
    pub fn kill_append(&mut self, text: String, prepend: bool) {
        match self.kill_ring.last_mut() {
            Some(last) => {
                if prepend {
                    *last = format!("{}{}", text, last);
                } else {
                    last.push_str(&text);
                }
                self.kill_ring_yank = self.kill_ring.len() - 1;
            }
            None => self.kill_new(text),
        }
    }

    pub fn echo(&mut self, msg: impl Into<String>) {
        self.echo = Some(msg.into());
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// Fetch the editor handle stored in the interpreter's extension slot.
pub fn editor(interp: &Interp) -> Rc<RefCell<Editor>> {
    interp
        .ext
        .as_ref()
        .and_then(|e| e.clone().downcast::<RefCell<Editor>>().ok())
        .expect("editor not installed in interpreter")
}

/// Switch the current buffer, swapping buffer-local variable values
/// in and out of the global symbol slots (like classic Emacs).
pub fn set_current_buffer(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, buf: Rc<RefCell<Buffer>>) {
    {
        let editor = ed.borrow();
        if Rc::ptr_eq(&editor.current, &buf) {
            return;
        }
    }
    // Save current buffer's locals back and restore globals.
    let old = ed.borrow().current.clone();
    {
        let mut old_b = old.borrow_mut();
        let ids: Vec<SymId> = old_b.locals.keys().copied().collect();
        let mut editor = ed.borrow_mut();
        for id in ids {
            let live = interp.symbols[id as usize].value.take();
            if let Some(v) = live {
                old_b.locals.insert(id, v);
            }
            interp.symbols[id as usize].value = editor.swapped_globals.remove(&id).flatten();
        }
    }
    // Install the new buffer's locals.
    {
        let new_b = buf.borrow();
        let mut editor = ed.borrow_mut();
        for (id, val) in new_b.locals.iter() {
            let global = interp.symbols[*id as usize].value.take();
            editor.swapped_globals.insert(*id, global);
            interp.symbols[*id as usize].value = Some(val.clone());
        }
    }
    ed.borrow_mut().current = buf;
}

/// Shift every window's saved `window_start`/`point` for an edit that just
/// happened in `buf`. This is the window-layer counterpart of
/// `Buffer::adjust_positions_insert`/`adjust_positions_delete`
/// (buffer.rs): those shift the buffer's OWN `point`/`mark`/`markers`/
/// `overlays`, all of which live on `Buffer` and so are visible to
/// `Buffer::insert`/`delete` directly. `Window.point`/`window_start` live
/// on `Editor` instead (a window is not part of the buffer it displays),
/// so `Buffer::insert`/`delete` cannot see or shift them -- every caller
/// that edits buffer text must call this afterward. Three producers as of
/// M72 period 2: `edit_insert`, `edit_delete`, and `undo-internal`
/// (`builtins/editing.rs`, one call per `AppliedEdit` `Buffer::
/// undo_step_from` reports -- see that function's doc for why order
/// matters there). `AppliedEdit` (buffer.rs) is the shared "what changed"
/// type across all three; this function is the one consumer.
///
/// `point`'s thresholds are copied verbatim from `Buffer`'s treatment of
/// `self.point` in `adjust_positions_insert`/`adjust_positions_delete`: a
/// window's frozen `point` is exactly what the buffer's own `point` would
/// be right now had that window stayed selected throughout, so it has to
/// move the same way -- on insert, a point sitting exactly AT the
/// insertion position moves past the newly typed text (`>=`, not `>`); on
/// delete, a point inside the deleted range collapses to the range's
/// start rather than being left dangling past the end of the now-shorter
/// buffer.
///
/// `window_start` keeps its own, pre-existing insert threshold (`>`, not
/// `>=`), left unchanged by M72: that threshold predates this function
/// (it was already there in `edit_insert`'s hand-rolled loop before M72
/// consolidated the three call sites), with no comment recorded anywhere
/// explaining why `>` rather than `>=` was chosen, and no test pinning
/// down the insert-exactly-at-window_start boundary case either way. The
/// plausible-sounding story -- text inserted exactly at a window's first
/// displayed character scrolls into view rather than pushing the anchor
/// forward, mirroring the "stays put on insertion at this exact spot"
/// choice Emacs markers make by default (`insertion-type` nil), as
/// opposed to `point`'s "moves with text typed at point" (`insertion-type`
/// t) -- is a rationalization offered after the fact, NOT a confirmed
/// original design intent; treat it as such. What IS confirmed: changing
/// it is out of M72's scope -- no repro motivates it, and the boundary
/// case (`x == pos`) is untested: the existing `window_start_stays_in_
/// sync_across_windows` test in lsp_format_tests.rs never lands its edit
/// exactly at the frozen window's `window_start` (that test's edit is on
/// line 5, the frozen `window_start` is on line 150), so `>` vs `>=`
/// makes no difference to it either way -- it would pass unchanged under
/// both. `window_start`'s insert threshold is therefore genuinely
/// uncovered at the boundary; flipping `>` to `>=` would not turn any
/// known test red, which is exactly why it is left alone rather than
/// "fixed" on no evidence. (`window_start`'s delete threshold already
/// matched `Buffer.point`'s exactly, so no divergence there.)
pub(crate) fn adjust_windows_for_edit(
    ed: &Rc<RefCell<Editor>>,
    buf: &Rc<RefCell<Buffer>>,
    edit: AppliedEdit,
) {
    for win in ed.borrow_mut().windows.values_mut() {
        if !Rc::ptr_eq(&win.buffer, buf) {
            continue;
        }
        match edit {
            AppliedEdit::Insert { pos, n } => {
                if win.window_start > pos {
                    win.window_start += n;
                }
                if win.point >= pos {
                    win.point += n;
                }
            }
            AppliedEdit::Delete { start, n } => {
                let end = start + n;
                if win.window_start >= end {
                    win.window_start -= n;
                } else if win.window_start > start {
                    win.window_start = start;
                }
                if win.point >= end {
                    win.point -= n;
                } else if win.point > start {
                    win.point = start;
                }
            }
        }
    }
}

/// Insert text into `buf` and keep every window showing it pointed at
/// sane text (their window_start/point shift the same way the buffer's
/// own point/markers do -- see `adjust_windows_for_edit`).
pub fn edit_insert(
    ed: &Rc<RefCell<Editor>>,
    buf: &Rc<RefCell<Buffer>>,
    pos: usize,
    s: &str,
) -> usize {
    let n = buf.borrow_mut().insert(pos, s);
    if n > 0 {
        adjust_windows_for_edit(ed, buf, AppliedEdit::Insert { pos, n });
    }
    n
}

/// Delete `[start, end)` from `buf`, adjusting other windows' scroll and
/// point the same way buffer markers are adjusted (see
/// `adjust_windows_for_edit`).
pub fn edit_delete(
    ed: &Rc<RefCell<Editor>>,
    buf: &Rc<RefCell<Buffer>>,
    start: usize,
    end: usize,
) -> String {
    let removed = buf.borrow_mut().delete(start, end);
    let n = removed.chars().count();
    if n > 0 {
        adjust_windows_for_edit(ed, buf, AppliedEdit::Delete { start, n });
    }
    removed
}

/// Display `buf` in the selected window (switch-to-buffer semantics —
/// unlike set-buffer, this changes what the window shows).
pub fn show_buffer_in_selected_window(
    interp: &mut Interp,
    ed: &Rc<RefCell<Editor>>,
    buf: Rc<RefCell<Buffer>>,
) {
    {
        let mut editor = ed.borrow_mut();
        let sel = editor.selected_window;
        if let Some(win) = editor.windows.get_mut(&sel) {
            win.buffer = buf.clone();
            win.point = buf.borrow().point;
            win.window_start = 0;
        }
    }
    set_current_buffer(interp, ed, buf);
}

/// Select window `id`: save the old window's point, restore the new one's.
pub fn select_window(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, id: usize) {
    let target_buf = {
        let mut editor = ed.borrow_mut();
        let sel = editor.selected_window;
        let cur_point = editor.current.borrow().point;
        if let Some(win) = editor.windows.get_mut(&sel) {
            win.point = cur_point;
        }
        let Some(win) = editor.windows.get(&id) else {
            return;
        };
        let buf = win.buffer.clone();
        let point = win.point;
        editor.selected_window = id;
        {
            let mut b = buf.borrow_mut();
            b.point = b.clamp(point as i64);
        }
        buf
    };
    set_current_buffer(interp, ed, target_buf);
}

/// After every command the current buffer snaps back to the selected
/// window's buffer (Emacs command-loop invariant).
pub fn sync_current_to_selected(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) {
    let buf = {
        let editor = ed.borrow();
        editor
            .windows
            .get(&editor.selected_window)
            .map(|w| w.buffer.clone())
    };
    if let Some(buf) = buf {
        let differs = !Rc::ptr_eq(&ed.borrow().current, &buf);
        if differs {
            set_current_buffer(interp, ed, buf);
        }
    }
}

/// Split the selected window; the new window shows the same buffer.
/// `horizontal` = side by side (C-x 3), else stacked (C-x 2).
pub fn split_selected(ed: &Rc<RefCell<Editor>>, horizontal: bool) {
    let mut editor = ed.borrow_mut();
    let sel = editor.selected_window;
    let Some(win) = editor.windows.get(&sel) else {
        return;
    };
    let new_win = Window {
        buffer: win.buffer.clone(),
        point: win.buffer.borrow().point,
        window_start: win.window_start,
        scroll_pin: None,
    };
    let new_id = editor.next_window_id;
    editor.next_window_id += 1;
    editor.windows.insert(new_id, new_win);
    editor.layout.replace_leaf(
        sel,
        Layout::Split {
            horizontal,
            a: Box::new(Layout::Leaf(sel)),
            b: Box::new(Layout::Leaf(new_id)),
        },
    );
}

fn isearch_prefix(forward: bool, failed: bool) -> &'static str {
    match (forward, failed) {
        (true, false) => "I-search: ",
        (true, true) => "Failing I-search: ",
        (false, false) => "I-search backward: ",
        (false, true) => "Failing I-search backward: ",
    }
}

pub fn isearch_start(ed: &Rc<RefCell<Editor>>, forward: bool) {
    let buf = ed.borrow().current.clone();
    let start = buf.borrow().point;
    let start_byte = buf.borrow().text.char_to_byte(start);
    ed.borrow_mut().isearch = Some(Isearch {
        query: String::new(),
        buffer: Rc::downgrade(&buf),
        start,
        origin: start,
        origin_byte: start_byte,
        forward,
        failed: false,
    });
    let msg = isearch_prefix(forward, false).to_string();
    ed.borrow_mut().echo(msg);
}

/// Re-run the search for the current query from the isearch origin,
/// updating point (on success) and the echo-area status line.
fn isearch_step(ed: &Rc<RefCell<Editor>>) {
    let (query, start, origin_byte, forward) = {
        let editor = ed.borrow();
        let Some(s) = &editor.isearch else { return };
        (s.query.clone(), s.start, s.origin_byte, s.forward)
    };
    let buf = ed.borrow().current.clone();
    if query.is_empty() {
        buf.borrow_mut().point = start;
        let start_byte = buf.borrow().text.char_to_byte(start);
        let mut editor = ed.borrow_mut();
        if let Some(s) = &mut editor.isearch {
            s.origin = start;
            s.origin_byte = start_byte;
            s.failed = false;
        }
        editor.echo(isearch_prefix(forward, false));
        return;
    }
    // Every keystroke re-searches from the fixed origin (not from wherever
    // the previous, shorter query happened to match) — otherwise a match
    // that only fit the old shorter query strands the search forward of
    // where the longer query could actually be found.
    //
    // Both directions search the buffer's cached `search_text` snapshot
    // (P1.2) instead of materializing a fresh `bb.text.slice(...)` on
    // every keystroke — that per-key O(buffer size) slice was the whole
    // point of the isearch perf probe this change targets. M43 period
    // 2: `search_text()`/`find_forward`/`find_backward` all work in
    // bytes now; `origin_byte` is read from the cached field (see its
    // doc on `Isearch`) rather than recomputed via `char_to_byte` here —
    // that recomputation, done fresh every keystroke, was measured to
    // regress this probe past its *pre-M43* baseline (anchor ping-pong
    // between `origin`'s position and the match position). Only the
    // match position still needs a conversion back to chars
    // (`byte_to_char`), which stays cheap once the anchor settles near
    // it (M43 design §3.4).
    let found = if forward {
        let bb = buf.borrow();
        let hay = bb.search_text();
        crate::gapbuffer::find_forward(&hay, &query, origin_byte)
            .map(|start_byte| bb.text.byte_to_char(start_byte + query.len()))
    } else {
        let bb = buf.borrow();
        let hay = bb.search_text();
        let search_end = origin_byte.min(hay.len());
        crate::gapbuffer::find_backward(&hay, &query, search_end)
            .map(|start_byte| bb.text.byte_to_char(start_byte))
    };
    match found {
        Some(pos) => {
            buf.borrow_mut().point = pos;
            let mut editor = ed.borrow_mut();
            if let Some(s) = &mut editor.isearch {
                s.failed = false;
            }
            editor.echo(format!("{}{}", isearch_prefix(forward, false), query));
        }
        None => {
            let mut editor = ed.borrow_mut();
            if let Some(s) = &mut editor.isearch {
                s.failed = true;
            }
            editor.echo(format!("{}{}", isearch_prefix(forward, true), query));
        }
    }
}

pub fn isearch_push_char(ed: &Rc<RefCell<Editor>>, c: char) {
    if let Some(s) = &mut ed.borrow_mut().isearch {
        s.query.push(c);
    }
    isearch_step(ed);
}

pub fn isearch_pop_char(ed: &Rc<RefCell<Editor>>) {
    if let Some(s) = &mut ed.borrow_mut().isearch {
        s.query.pop();
    }
    isearch_step(ed);
}

/// Repeat the search in its current direction: re-anchor the search
/// origin at the current match (so the next search skips past it and
/// finds the *next* occurrence instead of matching the same spot again).
pub fn isearch_repeat(ed: &Rc<RefCell<Editor>>) {
    let empty = {
        let editor = ed.borrow();
        editor
            .isearch
            .as_ref()
            .map(|s| s.query.is_empty())
            .unwrap_or(true)
    };
    if !empty {
        let (point, point_byte) = {
            let buf = ed.borrow().current.clone();
            let bb = buf.borrow();
            (bb.point, bb.text.char_to_byte(bb.point))
        };
        if let Some(s) = &mut ed.borrow_mut().isearch {
            s.origin = point;
            s.origin_byte = point_byte;
        }
    }
    isearch_step(ed);
}

/// End the search, keeping point at the current match. M30: a non-empty
/// query survives into `last_search` for evil's `n`/`N` to repeat (an
/// empty query -- RET pressed immediately -- leaves whatever was
/// previously recorded alone, matching this editor's existing
/// isearch_step handling of an empty query as "revert, don't search").
pub fn isearch_exit(ed: &Rc<RefCell<Editor>>) {
    let session = ed.borrow_mut().isearch.take();
    if let Some(s) = session {
        if !s.query.is_empty() {
            ed.borrow_mut().last_search = Some((s.query, s.forward));
        }
    }
}

/// Abort the search, restoring point to where the session started.
pub fn isearch_cancel(ed: &Rc<RefCell<Editor>>) {
    let start = ed.borrow_mut().isearch.take().map(|s| s.start);
    if let Some(pos) = start {
        let buf = ed.borrow().current.clone();
        let mut b = buf.borrow_mut();
        b.point = b.clamp(pos as i64);
    }
}

/// Make `sym` buffer-local in the current buffer with the given value.
pub fn make_buffer_local(interp: &mut Interp, ed: &Rc<RefCell<Editor>>, sym: SymId, val: Value) {
    let buf = ed.borrow().current.clone();
    let already = buf.borrow().locals.contains_key(&sym);
    if !already {
        let global = interp.symbols[sym as usize].value.take();
        ed.borrow_mut().swapped_globals.insert(sym, global);
    }
    buf.borrow_mut().locals.insert(sym, val.clone());
    interp.symbols[sym as usize].value = Some(val);
}

/// Read `sym`'s value as seen by `buf`, honoring buffer-local bindings
/// whether or not `buf` is the current buffer. This is the same
/// current/locals/swapped_globals dance `set_current_buffer` performs on
/// a buffer switch, worked out for a single symbol without actually
/// switching:
///
/// - `buf` IS current: its local values (if any) live in the global
///   symbol slot (swapped in by `set_current_buffer`/`make_buffer_local`),
///   so the answer is just the global cell.
/// - `buf` is NOT current but has `sym` in its own `locals` map: that
///   map holds the authoritative value for a non-current buffer.
/// - `buf` is NOT current and has no local binding for `sym`, but the
///   CURRENT buffer does: the global cell then holds the *current*
///   buffer's local value, not the true global — the true global is
///   parked in `swapped_globals` instead.
/// - Otherwise: nothing is buffer-local here, so the global cell holds
///   the ordinary global value.
///
/// `None` means void everywhere. Backs the `buffer-local-value` builtin
/// (`builtins/buffers.rs`) — factored out so other per-buffer readers,
/// e.g. redisplay's buffer-local `display-line-numbers` gutter toggle
/// (M24), don't have to re-derive this.
pub fn buffer_local_value(
    interp: &Interp,
    ed: &Editor,
    sym: SymId,
    buf: &Rc<RefCell<Buffer>>,
) -> Option<Value> {
    if Rc::ptr_eq(&ed.current, buf) {
        return interp.sym_value(sym);
    }
    if let Some(v) = buf.borrow().locals.get(&sym) {
        return Some(v.clone());
    }
    if ed.current.borrow().locals.contains_key(&sym) {
        return ed.swapped_globals.get(&sym).cloned().flatten();
    }
    interp.sym_value(sym)
}
