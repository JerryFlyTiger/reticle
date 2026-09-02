use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use elisp::value::SymId;
use elisp::{Interp, Value};

use crate::buffer::Buffer;
use crate::editor::Editor;

pub mod display_width;
use display_width::wide_char_width;

pub type Color = (u8, u8, u8);

/// Underline style (M16): `Wave` is the diagnostics squiggle. The TUI
/// degrades Wave to a straight underline; the GUI draws the real thing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Straight,
    Wave,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    /// Underline drawn in its own color (diagnostics severity); falls
    /// back to the foreground when None.
    pub underline_color: Option<Color>,
    pub reverse: bool,
}

impl Style {
    /// Fill unset colors from `base` (the `default` face) — frontends
    /// call this so themes control the frame's ground colors.
    pub fn or_default(mut self, base: &Style) -> Style {
        if self.fg.is_none() {
            self.fg = base.fg;
        }
        if self.bg.is_none() {
            self.bg = base.bg;
        }
        self
    }
}

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    /// True for the second column of a double-width char.
    pub continuation: bool,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Cell {
        Cell {
            ch: ' ',
            continuation: false,
            style: Style::default(),
        }
    }
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<Vec<Cell>>,
    /// (row, col) of the hardware cursor.
    pub cursor: (usize, usize),
    /// Per-window layout (GUI fix 2): the GUI has no access to
    /// `window_rects`/the gutter-width math above (both `pub(crate)`,
    /// evil's direction picker and `render_window` respectively are the
    /// only other readers), so without this it can only *guess* window
    /// geometry -- correct for one full-height, ungutlered window and
    /// wrong the moment a split or a line-number gutter is on screen.
    /// This is additive metadata about the layout the grid was already
    /// built from, not a change to any cell: the TUI reads `lines` only
    /// and ignores this field, so its rendering is unaffected.
    pub windows: Vec<WindowLayout>,
    /// Mouse-support metadata (additive, same precedent as `windows`
    /// above): every screen cell this frame painted, grouped into
    /// contiguous same-style runs, each tagged with the buffer byte
    /// range it came from (`None` for chrome -- see `PaintRun::src`).
    /// Built alongside `lines`/`windows` by `render_window` (buffer
    /// text, byte-accurate) and a post-pass over the finished grid
    /// (everything else, `src: None`) -- see `PaintRun`'s own doc for
    /// why those need different treatment. The TUI ignores this field
    /// entirely, same as `windows`.
    pub runs: Vec<PaintRun>,
}

/// One contiguous, same-style run of painted screen cells, published
/// alongside the `Grid` (task 1 of the mouse-support milestone) so a
/// frontend can map a click's `(row, col)` back to a buffer position
/// without re-deriving the paint loop's own row/col bookkeeping --
/// `Grid::buffer_pos_at` is that inverse mapping.
///
/// **Byte ranges, not char positions**: `src`, when `Some`, is a range
/// of buffer *bytes* (`GapBuffer`'s `char_to_byte`/`byte_to_char`
/// convert to/from the char positions `Buffer::point` etc. use). This
/// is a deliberate departure from the char-offset convention every
/// other buffer position in this codebase uses (see `gapbuffer.rs`'s
/// module doc) -- picked because a future text-shaping consumer (the
/// next milestone) wants byte spans into `text` directly, and asking it
/// to re-derive byte offsets from char offsets on every shaped run
/// would be exactly the kind of "second source of truth" this field
/// exists to avoid. A GUI event handler that wants a char position (to
/// assign `Buffer::point`) calls `byte_to_char` once, at the very end,
/// on the single resolved position -- not per run.
///
/// **Tab and control-character expansion**: a `\t` or a C0/DEL control
/// character occupies exactly one source byte but is rendered as
/// several display columns (a tab's spaces, or a control char's `^X`
/// escape) -- more rendered columns/bytes than source bytes, so there
/// is no proportional column-to-byte mapping inside such a run. These
/// runs are therefore never merged with anything else (`RunBuilder::
/// push_atomic`, always exactly one run per escape) and are marked
/// `atomic: true` by `push_atomic`: every column inside such a run maps
/// to `src.start`. The invisible-region "..." indicator (`redisplay.rs`'s
/// `invisible_end` handling) is the same shape -- three rendered bytes
/// standing in for an arbitrarily large hidden byte span -- and is
/// pushed the same way.
///
/// **Why `atomic` is an explicit field, not inferred**: an earlier
/// version of this type had no such field and `Grid::map_col_in_run`
/// instead guessed "atomic" from `r.text.len() != byte_len`. That broke
/// for the "..." indicator specifically: it is always the 3-byte ASCII
/// literal `"..."`, so whenever the hidden span it stands in for is
/// *also* exactly 3 source bytes (one CJK character, which is 1 char but
/// 3 UTF-8 bytes -- not exotic in Verilog identifiers or comments), the
/// lengths coincide, the guess says "proportional", and the run gets
/// walked character-by-character over `"..."`'s own three ASCII chars --
/// returning `src.start + 1` and `src.start + 2` as byte offsets that
/// land on UTF-8 continuation bytes, not character boundaries. That
/// panics `GapBuffer::byte_to_char` in debug builds and silently
/// misplaces point in release. A tab landing at `col % 8 == 7` triggers
/// the same coincidence (renders as exactly one column and one byte) but
/// is harmless there, because a one-column run has only one possible
/// answer regardless of which branch runs. An explicit flag, set at the
/// one place (`push_atomic`) that already knows a run is non-proportional,
/// makes the coincidence irrelevant instead of merely rare.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintRun {
    pub row: usize,
    /// Starting display column.
    pub col: usize,
    /// Display columns this run occupies.
    pub cols: usize,
    pub text: String,
    pub style: Style,
    /// Buffer byte range this run came from. `None` for chrome that has
    /// no buffer behind it: the mode line, the line-number gutter, the
    /// echo area, panels and popups.
    pub src: Option<Range<usize>>,
    /// Set by `RunBuilder::push_atomic` for a tab, a C0/DEL control-char
    /// escape, or the invisible-region "..." indicator -- see this
    /// struct's doc for why `map_col_in_run` must read this flag rather
    /// than infer it from a length comparison.
    pub atomic: bool,
}

/// Incrementally builds `Grid::runs` for buffer text as `render_window`
/// walks characters left to right. A run merges consecutive characters
/// when three things all hold: same row, same style (an empty pending
/// run -- see `start_marker` -- adopts the first character's style
/// rather than gating on it), and byte-contiguous `src` (so a run's
/// `src` stays one unbroken `Range`). The moment any of those breaks,
/// the open run is flushed and a new one starts -- which also covers
/// "window boundary": each `render_window` call owns its own
/// `RunBuilder`, so nothing from one window's runs can merge into
/// another's even when two windows abut with no separator column.
///
/// See `PaintRun`'s doc for why tab/control-char expansion and the
/// invisible-region ellipsis must never merge into a `push`ed run --
/// `push_atomic` is their dedicated, always-standalone path.
struct RunBuilder {
    open: Option<PaintRun>,
}

impl RunBuilder {
    fn new() -> RunBuilder {
        RunBuilder { open: None }
    }

    /// Seed (or re-seed) a zero-width pending run at the start of a row:
    /// `col`/`src` mark where this row's buffer text begins, `text` is
    /// empty and `style` is a placeholder the first real `push` adopts.
    /// This is what lets `Grid::buffer_pos_at` answer a click on a
    /// completely empty line (its only run stays zero-width, flushed
    /// as-is) or before the first character of a non-empty one (the
    /// marker gets absorbed into that character's run instead, leaving
    /// no separate entry) -- either way, every row on screen has at
    /// least one `Some`-`src` run recording where it starts.
    fn start_marker(&mut self, grid: &mut Grid, row: usize, col: usize, byte_pos: usize) {
        self.flush(grid);
        self.open = Some(PaintRun {
            row,
            col,
            cols: 0,
            text: String::new(),
            style: Style::default(),
            src: Some(byte_pos..byte_pos),
            atomic: false,
        });
    }

    /// Append one plain (non-expanding) character: `text` grows by
    /// exactly this one char and, when `src` is present, `src.end`
    /// grows by exactly this char's UTF-8 length -- so summing
    /// `len_utf8()` over a prefix of `text.chars()` reconstructs the
    /// matching byte offset, which is exactly what `Grid::
    /// buffer_pos_at`'s proportional-scan branch relies on.
    fn push(
        &mut self,
        grid: &mut Grid,
        at: RunPos,
        ch: char,
        style: Style,
        src: Option<Range<usize>>,
    ) {
        let RunPos { row, col, cols } = at;
        let mergeable = self.open.as_ref().is_some_and(|r| {
            r.row == row
                && r.col + r.cols == col
                && (r.text.is_empty() || r.style == style)
                && match (&r.src, &src) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.end == b.start,
                    _ => false,
                }
        });
        if mergeable {
            let r = self.open.as_mut().unwrap();
            r.text.push(ch);
            r.cols += cols;
            r.style = style;
            if let Some(a) = &mut r.src {
                a.end = src.expect("src.is_some() checked by `mergeable` above").end;
            }
        } else {
            self.flush(grid);
            self.open = Some(PaintRun {
                row,
                col,
                cols,
                text: ch.to_string(),
                style,
                src,
                atomic: false,
            });
        }
    }

    /// One-off run for a tab, a C0/DEL control-char escape, or the
    /// invisible-region "..." indicator: always flushes whatever was
    /// open first and never merges in either direction (the *next*
    /// `push`/`push_atomic` call always starts fresh, since this
    /// doesn't leave anything in `self.open`). `text` is the full
    /// rendered expansion, `cols` its full display width, `src` the
    /// source byte range it stands in for.
    fn push_atomic(
        &mut self,
        grid: &mut Grid,
        at: RunPos,
        text: String,
        style: Style,
        src: Option<Range<usize>>,
    ) {
        self.flush(grid);
        let RunPos { row, col, cols } = at;
        grid.runs.push(PaintRun {
            row,
            col,
            cols,
            text,
            style,
            src,
            atomic: true,
        });
    }

    fn flush(&mut self, grid: &mut Grid) {
        if let Some(r) = self.open.take() {
            grid.runs.push(r);
        }
    }
}

/// `(row, col, cols)` bundled into one value -- `RunBuilder::push`/
/// `push_atomic` each need all three alongside a `ch`/`text`, `style`,
/// and `src`; bundling keeps their arg count under clippy's
/// `too_many_arguments` threshold instead of passing five-plus loose
/// `usize`s.
#[derive(Clone, Copy)]
struct RunPos {
    row: usize,
    col: usize,
    cols: usize,
}

/// One window pane's screen geometry, published alongside the `Grid` it
/// was rendered into. `row`/`col`/`rows`/`cols` are the same rectangle
/// `window_rects` computed (row/col/height/width) -- `rows`/`cols`
/// include the window's own mode-line row and its gutter columns, so a
/// consumer that wants just the text area subtracts `gutter_cols` off
/// the left and treats `mode_line_row` as the bottom-exclusive bound.
#[derive(Clone, Copy, Debug)]
pub struct WindowLayout {
    /// The window ID this entry describes -- lets a consumer match it
    /// against `Editor::selected_window` (the scrollbar needs to know
    /// which entry is the *selected* window's, not just iterate all of
    /// them).
    pub win_id: usize,
    pub row: usize,
    pub col: usize,
    pub rows: usize,
    pub cols: usize,
    /// Line-number gutter width in columns (0 when `display-line-numbers`
    /// is off, or the pane is too narrow to show one).
    pub gutter_cols: usize,
    /// Grid row index of this window's mode line.
    pub mode_line_row: usize,
}

impl Grid {
    fn new(cols: usize, rows: usize) -> Grid {
        Grid {
            cols,
            rows,
            lines: vec![vec![Cell::default(); cols]; rows],
            cursor: (0, 0),
            windows: Vec::new(),
            runs: Vec::new(),
        }
    }

    /// Inverse of the paint loop (task 1 of the mouse-support
    /// milestone): the buffer byte offset that painted screen cell
    /// `(row, col)`, or `None` when there is no buffer text behind that
    /// cell at all -- outside every run's row, or the only runs on that
    /// row have `src: None` (chrome: mode line, gutter, echo, panel,
    /// popup). Self-contained -- everything it needs is already in
    /// `self.runs`, no buffer access required, so a frontend can call it
    /// straight from a resize/click handler holding only the `Grid`.
    ///
    /// Deliberate decisions on the two "which sub-position" cases:
    /// - **Tab / control-char / invisible-ellipsis runs** (pushed via
    ///   `RunBuilder::push_atomic`, identified here by `r.atomic` -- see
    ///   `PaintRun`'s doc for why this is an explicit flag rather than a
    ///   length comparison): every column inside the run maps to
    ///   `src.start`, the one source byte/range it stands in for.
    /// - **Wide (double-width) characters**: both display columns of a
    ///   wide char map to the same byte, that character's own start --
    ///   its continuation cell (`Cell::continuation`) has no byte of its
    ///   own to map to.
    /// - **Past the end of a row's text** (including a wholly empty
    ///   line, whose only run is a zero-width `start_marker`): clamps to
    ///   that row's last run's `src.end` -- landing at end-of-line,
    ///   never spilling onto the next row's first character. This is
    ///   also the fallback for any `col` that doesn't land inside a
    ///   positive-width run for another reason.
    pub fn buffer_pos_at(&self, row: usize, col: usize) -> Option<usize> {
        let mut row_runs: Vec<&PaintRun> = self
            .runs
            .iter()
            .filter(|r| r.row == row && r.src.is_some())
            .collect();
        if row_runs.is_empty() {
            return None;
        }
        row_runs.sort_by_key(|r| r.col);
        for r in &row_runs {
            if r.cols > 0 && col >= r.col && col < r.col + r.cols {
                return Some(Self::map_col_in_run(r, col));
            }
        }
        let last = row_runs.last().unwrap();
        Some(last.src.clone().unwrap().end)
    }

    /// `col`, known to fall inside `r`'s positive-width span, mapped to a
    /// buffer byte offset. See `buffer_pos_at`'s doc for the two
    /// deliberate sub-position decisions this implements.
    fn map_col_in_run(r: &PaintRun, col: usize) -> usize {
        let src = r.src.clone().expect("caller filters to src.is_some()");
        if r.atomic {
            // Non-proportional expansion (tab, C0/DEL escape, or the
            // invisible-region "..." indicator) -- see `PaintRun`'s doc.
            return src.start;
        }
        let mut c = r.col;
        let mut byte = src.start;
        for ch in r.text.chars() {
            let w = wide_char_width(ch).max(1);
            if col < c + w {
                return byte;
            }
            c += w;
            byte += ch.len_utf8();
        }
        src.end
    }

    fn put(&mut self, row: usize, col: usize, ch: char, style: Style) {
        if row < self.rows && col < self.cols {
            self.lines[row][col] = Cell {
                ch,
                continuation: false,
                style,
            };
        }
    }

    fn put_wide(&mut self, row: usize, col: usize, ch: char, style: Style) {
        self.put(row, col, ch, style);
        if row < self.rows && col + 1 < self.cols {
            self.lines[row][col + 1] = Cell {
                ch: ' ',
                continuation: true,
                style,
            };
        }
    }
}

/// Width of `c` when drawn at column `col` (tabs are column-dependent).
/// See `display_width` for the shared five-way rule this wraps.
fn char_width(c: char, col: usize) -> usize {
    display_width::char_width(c, col)
}

/// Whether a character of width `w`, drawn starting at column `col`,
/// needs the current row to wrap before it — i.e. it would land on or
/// past the last column, which is reserved for the `\` continuation
/// marker rather than for text.
///
/// Both the wrap lookahead (`next_row_start`, used by scrolling) and
/// the actual paint loop (`render_window`'s draw loop) must agree on
/// this test, or scrolling and painting drift apart — exactly the
/// failure `frame_layout` was extracted to prevent for `render` vs.
/// `window_rects` (see its doc comment). This is the equivalent
/// extraction for the wrap test.
///
/// Three other boundary checks in this file are deliberately *not*
/// routed through here, and the reason is not the one you would guess.
/// `render_panel` and `render_completion_popup` test `col + w >= cols`,
/// which for any `cols >= 1` is the same arithmetic as this function —
/// there is no integer between the two. What differs is the reaction:
/// they `break` and truncate the row, never advancing to a new row and
/// never drawing a continuation marker, so sharing a predicate named
/// "wraps before" would misdescribe both call sites. Only
/// `render_lsp_completion_popup`'s `col + w > width` is a genuinely
/// different formula (no reserved last column). Merging any of the
/// three would be a behaviour question, not a tidying one.
fn wraps_before(col: usize, w: usize, cols: usize) -> bool {
    col + w > cols.saturating_sub(1)
}

// M69: mode-line layout. The old code composed the left/right strings
// and drew them in the same breath, which is how the "right segment gets
// mashed into garbled characters" bugs (see PLAN.md M69) went unnoticed for
// so long — nothing about that shape was testable without a full
// `render()` + grid to inspect.
// `compose_mode_line` below is the pure half: given the raw pieces, it
// returns exactly the two strings that get drawn and where the bold
// "name" style ends, with no editor/grid/env dependency, so the layout
// policy itself has unit tests.

/// Pure inputs to mode-line layout. Every user-controllable string
/// arrives pre-split so `compose_mode_line` never has to reach into the
/// buffer, the editor, or the environment — `home` in particular is
/// injected rather than read from `$HOME` inside the function, because
/// process-global env vars are a race under parallel test execution
/// (`complete.rs`'s `abbreviate_home_with` and `panel.rs`'s `place_for`
/// already use this shape; this follows the same precedent).
#[derive(Default)]
struct ModeLineParts<'a> {
    /// `mode-line-prefix` (M28), buffer-local elisp variable — an
    /// arbitrary string (evil-mode's "<N> " state tag is the only
    /// current user).
    prefix: &'a str,
    /// "dir/name" for file buffers, plain buffer name otherwise.
    title: &'a str,
    modified: bool,
    mode_name: &'a str,
    /// The buffer's *un-abbreviated* `default_directory`; `Some` only
    /// for buffers with no backing file (dired, eshell) — file buffers
    /// show their directory as part of `title` instead.
    dir: Option<&'a str>,
    home: Option<&'a str>,
    lsp: bool,
    diags: usize,
    line: usize,
    col: usize,
}

/// Laid-out mode line, ready to draw. `left` and `right` are already
/// sanitized (no `< 0x20` or `0x7f` byte reaches either string) and,
/// except in degenerate cases narrower than a single space plus the
/// truncation ellipsis, together fit in the `width` columns passed to
/// `compose_mode_line`. `name_cols` is how many *display columns* (not
/// `chars().count()` — see `ml_width`) at the start of `left` should be
/// drawn in the bold "name" face.
struct ModeLine {
    left: String,
    right: String,
    name_cols: usize,
}

/// Minimum columns kept between the left and right segments.
const ML_GAP: usize = 2;
/// `UnicodeWidthChar::width('…') == Some(1)`.
const ML_ELLIPSIS: char = '…';
/// Floor `compose_mode_line` aims to keep a title at *while there is
/// still room for it* — below this a filename stops being recognizable
/// at all, so there's no point trading more columns for it over there
/// being no title at all. This is a target, not a hard guarantee: at
/// widths too small to fit `ML_GAP` plus a bare `L:C`, the final
/// gap-reservation step (see the end of `compose_mode_line`) will
/// shrink the title further, or drop it to nothing, to protect the gap
/// and `L:C` instead.
const ML_MIN_TITLE_COLS: usize = 8;

/// Mode-line-safe rendering of `s`: any `< 0x20` byte becomes `^` +
/// `(byte + 64)` (so `\r` -> `^M`), `0x7f` becomes `^?`, everything else
/// passes through unchanged.
///
/// This is deliberately *not* the same three-way split the buffer-text
/// and echo-area paths use (`redisplay.rs`'s buffer-drawing loop and the
/// echo-area loop below): those two treat `\t` as text layout and expand
/// it to the next 8-column tab stop, because in a buffer or a typed
/// command tab *is* the content. The mode line is a fixed-width status
/// bar assembled from program-chosen fields (`L:C`, mode name) plus a
/// handful of user-controllable strings (buffer name, `mode-line-prefix`,
/// directory); a tab reaching any of those is stray/hostile input, not
/// layout, so it's folded into the same "^X" treatment as any other C0
/// byte — `^I` — rather than eating up to 8 columns of a budget that's
/// already tight enough to be the reason this milestone exists.
fn ml_sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\u{7f}' {
            out.push('^');
            out.push('?');
        } else if (c as u32) < 0x20 {
            out.push('^');
            out.push(char::from_u32(c as u32 + 64).unwrap_or('?'));
        } else {
            out.push(c);
        }
    }
    out
}

/// Display columns of an already-sanitized string (no control chars
/// left, so this is just a `wide_char_width` sum — the column-dependent
/// tab case `char_width` has to handle doesn't apply here).
fn ml_width(s: &str) -> usize {
    s.chars().map(wide_char_width).sum()
}

/// Truncate an already-sanitized `s` to at most `budget` columns,
/// dropping from the *head* and prefixing `ML_ELLIPSIS`. Keeps the tail
/// rather than the head because RTL module/file names share long common
/// prefixes (`axi4_lite_*`, `dma_*` and the like) — the part that tells
/// two names apart is at the end, not the front. (The pre-M69 code
/// truncated from the tail instead; that's the direct cause of the
/// mode line eating the *last* directory component of a deep dired
/// path — the component with the most information in it.) Counts by
/// display column (`ml_width`), not `chars().count()`, so CJK text
/// truncates to the right width instead of running long.
fn ml_truncate_head(s: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    if ml_width(s) <= budget {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let widths: Vec<usize> = chars.iter().map(|c| wide_char_width(*c)).collect();
    let mut tail_w: usize = widths.iter().sum();
    let mut start = 0;
    let keep = budget - 1; // one column reserved for ML_ELLIPSIS
    while start < chars.len() && tail_w > keep {
        tail_w -= widths[start];
        start += 1;
    }
    let mut out = String::new();
    out.push(ML_ELLIPSIS);
    out.extend(&chars[start..]);
    out
}

/// Assemble the mode line's left (`" " + prefix + title + " *"?`) and
/// right (`dir  !N  LSP  L:C`) segments so the two never collide,
/// regardless of `width` — this is the fix for PLAN.md M69 (a narrow
/// split window used to lose its line/column entirely, and a deep dired
/// path would run the two segments together with zero separation).
///
/// Priority order for the right segment's optional prefixes, and for
/// the left segment's mode-name suffix — checked highest priority
/// first, i.e. the first one checked is the *least* likely to get
/// dropped when `width` gets tight, and the last one checked is the
/// *most* likely:
///
/// 1. `!{diags}` — a nonzero diagnostic count is a compile error for the
///    RTL workflow this project targets; that outranks everything else
///    that could be showing, so it's checked (and kept) first.
/// 2. mode name (left) — useful, but the file extension already hints
///    at it, so it ranks below diagnostics.
/// 3. `LSP` — just a connection status light.
/// 4. `dir` — the buffers that show this (dired, eshell) already spell
///    out their full path in the buffer's own first line or prompt
///    string, so this is the one genuinely redundant field; it's
///    checked last and is therefore the first one dropped once `width`
///    can't fit everything.
///
/// `L{line}:{col}` itself is never dropped or reordered away as an
/// optional segment — it's the one field this milestone exists to
/// protect, so it's simply part of `right`'s required prefix. In the
/// narrowest frames it can still be shortened by the gap-reservation
/// step at the end of this function (see `ML_GAP`'s doc comment and
/// PLAN.md M69's F3 fix) — that's a last resort, not a priority tier.
fn compose_mode_line(p: &ModeLineParts, width: usize) -> ModeLine {
    let prefix = ml_sanitize(p.prefix);
    let mode_name = ml_sanitize(p.mode_name);
    let modified_marker = if p.modified { " *" } else { "" };
    let dir = p
        .dir
        .map(|d| crate::complete::abbreviate_home_with(d, p.home))
        .map(|d| ml_sanitize(&d));

    let right_base = format!("L{}:{}", p.line, p.col);

    let fixed_left = ml_width(" ") + ml_width(&prefix) + ml_width(modified_marker);
    let budget = width.saturating_sub(fixed_left + ml_width(&right_base) + ML_GAP);
    let title = ml_sanitize(p.title);
    let title = if ml_width(&title) > budget {
        ml_truncate_head(&title, budget.max(ML_MIN_TITLE_COLS))
    } else {
        title
    };

    // The bold "name" run is everything up to the modified-buffer " *"
    // marker -- the pre-M69 code's `name_chars` didn't count that
    // marker either, and it isn't part of the buffer's name (M69 review
    // F4). The column count itself is computed at the very end, from
    // the far end of the finished string; see there.
    let mut left = format!(" {}{}{}", prefix, title, modified_marker);

    let mut used = ml_width(&left) + ml_width(&right_base) + ML_GAP;
    let mut show_diags = false;
    let mut show_lsp = false;
    let mut show_dir = false;

    if p.diags > 0 {
        let seg_w = ml_width(&format!("!{}  ", p.diags));
        if used + seg_w <= width {
            used += seg_w;
            show_diags = true;
        }
    }
    // Everything appended to `left` after the name -- the modified
    // marker and (if it fits) the mode name. Tracked as a width so
    // `name_cols` can be derived from the *end* of the final string
    // further down; see the comment there for why counting from the
    // front stops working once `left` is truncated.
    let mut left_suffix_w = ml_width(modified_marker);
    {
        let seg = format!("  {}", mode_name);
        let seg_w = ml_width(&seg);
        if used + seg_w <= width {
            used += seg_w;
            left.push_str(&seg);
            left_suffix_w += seg_w;
        }
    }
    if p.lsp {
        let seg_w = ml_width("LSP  ");
        if used + seg_w <= width {
            used += seg_w;
            show_lsp = true;
        }
    }
    if let Some(d) = &dir {
        let seg_w = ml_width(&format!("{}  ", d));
        if used + seg_w <= width {
            // `dir` is the last of the four priority checks, so nothing
            // reads `used` again after this -- the assignment is
            // genuinely dead by clippy's lights. Kept anyway (with the
            // `#[allow]` scoped to just this line) so the four branches
            // stay symmetric: a future fifth priority tier inserted
            // here would silently read a stale `used` if this branch
            // were the odd one out. M69 review F5.
            #[allow(unused_assignments)]
            {
                used += seg_w;
            }
            show_dir = true;
        }
    }

    // Assembled in the fixed `dir  !N  LSP  L:C` order regardless of the
    // priority order they were *tested* in above — the two orders are
    // different questions (which gets dropped first vs. where it sits).
    let mut right = String::new();
    if show_dir {
        right.push_str(dir.as_deref().unwrap_or(""));
        right.push_str("  ");
    }
    if show_diags {
        right.push_str(&format!("!{}  ", p.diags));
    }
    if show_lsp {
        right.push_str("LSP  ");
    }
    right.push_str(&right_base);

    if ml_width(&right) > width {
        right = ml_truncate_head(&right, width);
    }
    // Gap-reservation fallback: everything above budgets `ML_GAP` once,
    // up front, as part of the initial `used` -- but that only holds
    // the line if `left` (with its `ML_MIN_TITLE_COLS` floor) and
    // `right` (its own truncation just above) both actually fit inside
    // that budget. Narrow splits reach this in practice, not just in
    // theory: `compute_rects`'s horizontal split (`let aw =
    // rect.width.saturating_sub(sep) / 2`) has no minimum, so three
    // `C-x 3`s on an 80-column terminal (80 -> 39 -> 19 -> 9) gets here
    // for real, and was observed doing so -- two panes' title and
    // `L:C` running together with zero gap, title cut to 5 columns
    // (under the `ML_MIN_TITLE_COLS` floor `ml_truncate_head` aimed
    // for), before this fallback accounted for `ML_GAP` (M69 review
    // F3). `ml_truncate_head` is idempotent when `left` already fits,
    // so this always runs rather than only under a size check.
    let right_w = ml_width(&right);
    let avail_left = width.saturating_sub(right_w + ML_GAP);
    left = ml_truncate_head(&left, avail_left);

    // Derived from the *end*, not the front. `ml_truncate_head` keeps
    // the tail, so once the line above actually truncates, the columns
    // that survive are the tail of the name plus the whole suffix --
    // and a `name_cols` counted from the front (clamped with `min`)
    // then spills the bold styling onto the " *" marker and into the
    // mode name. Measuring back from the end works in both cases,
    // because truncation only ever eats the front: when nothing was
    // truncated this equals `ml_width(&name_only)` exactly, and when
    // the truncation ate into the suffix itself `saturating_sub`
    // bottoms out at zero (nothing bold), which is the right answer.
    // M69 tail review, finding 1.
    let name_cols = ml_width(&left).saturating_sub(left_suffix_w);

    ModeLine {
        left,
        right,
        name_cols,
    }
}

/// Draw an already-sanitized mode-line segment starting at column
/// `mcol`, never splitting a wide character across `limit`. `style_of`
/// is called with each character's starting column, so the left
/// segment can bold its leading "name" columns while the right segment
/// just returns one style throughout. Returns the column just past the
/// last cell written (`>= mcol`, `<= limit`).
fn draw_ml_segment(
    grid: &mut Grid,
    row: usize,
    col0: usize,
    mut mcol: usize,
    limit: usize,
    text: &str,
    style_of: impl Fn(usize) -> Style,
) -> usize {
    for c in text.chars() {
        let w = wide_char_width(c);
        if mcol + w > limit {
            break;
        }
        let st = style_of(mcol);
        if w == 2 {
            grid.put_wide(row, col0 + mcol, c, st);
        } else {
            grid.put(row, col0 + mcol, c, st);
        }
        mcol += w;
    }
    mcol
}

/// Merged invisible ranges from the buffer's overlays.
fn invisible_ranges(interp: &Interp, buffer: &Buffer) -> Vec<(usize, usize)> {
    let Some(inv_sym) = interp.intern_soft("invisible") else {
        return Vec::new();
    };
    let mut ranges: Vec<(usize, usize)> = buffer
        .overlays
        .iter()
        .filter_map(|ov| {
            let ov = ov.borrow();
            if ov.get(inv_sym).truthy() && ov.start < ov.end {
                Some((ov.start, ov.end))
            } else {
                None
            }
        })
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in ranges {
        match merged.last_mut() {
            Some((_, le)) if s <= *le => *le = (*le).max(e),
            _ => merged.push((s, e)),
        }
    }
    merged
}

fn invisible_end(pos: usize, ranges: &[(usize, usize)]) -> Option<usize> {
    ranges
        .iter()
        .find(|(s, e)| *s <= pos && pos < *e)
        .map(|(_, e)| *e)
}

/// Resolve the face style at `pos` by merging overlay face properties.
///
/// O(overlays) per call — kept only as the reference implementation for
/// `render_window`'s scanline replacement (`OverlayStyleScan` below) to
/// be cross-checked against in tests; render itself no longer calls
/// this, since a render pass calls the per-position lookup once per
/// visible character, and tree-sitter highlighting can materialize
/// hundreds to thousands of overlays across the visible margin
/// (`highlight.rs`'s `MARGIN_CHARS`), making the naive O(chars ×
/// overlays) cost tens of milliseconds per frame.
///
/// Folds every overlay covering `pos` in creation order (`seq`), later
/// ones winning ties on `fg`/`bg`/`underline` — real Emacs overlay
/// stacking semantics. `buffer.overlays` itself is kept sorted by
/// `start` rather than creation order (P1.3), so that order has to be
/// recovered explicitly via `seq` instead of assumed from iteration —
/// same as `OverlayStyleScan::new` below.
pub fn style_at_naive(interp: &Interp, ed: &Editor, buffer: &Buffer, pos: usize) -> Style {
    let Some(face_sym) = interp.intern_soft("face") else {
        return Style::default();
    };
    let mut covering: Vec<(u64, Value)> = buffer
        .overlays
        .iter()
        .filter_map(|ov| {
            let ov = ov.borrow();
            if ov.start <= pos && pos < ov.end {
                Some((ov.seq, ov.get(face_sym)))
            } else {
                None
            }
        })
        .collect();
    covering.sort_by_key(|(seq, _)| *seq);
    let mut style = Style::default();
    for (_, face) in covering {
        merge_face(interp, ed, &face, &mut style);
    }
    style
}

/// Scanline replacement for [`style_at_naive`]: a single O(overlays)
/// preprocessing pass at the start of `render_window`, then O(1)
/// amortized per query as `pos` sweeps forward.
///
/// Semantics preserved exactly: [`style_at_naive`] folds every overlay
/// covering `pos` in creation order (`seq`) via `merge_styles`, so
/// later-created overlays win ties on `fg`/`bg`/`underline` while
/// `bold`/`italic`/`reverse` OR together (order-independent for those).
/// This struct keeps each overlay's resolved per-overlay style, sorted
/// by `seq` once up front (`entries`, built by walking `buffer.overlays`
/// — which is itself kept sorted by `start`, not creation order, see
/// P1.3 — then re-sorted here), and `at(pos)` folds the *active* subset
/// in that same creation order — so the result is identical to
/// `style_at_naive`, just without re-borrowing and re-resolving every
/// overlay's face on every call.
///
/// Requires callers to query positions in non-decreasing order: this is
/// what makes the two-pointer sweep valid instead of a re-scan.
/// `render_window`'s main loop only ever moves `pos` forward — one
/// character at a time, or in one jump to the end of an invisible
/// region — so that holds.
pub struct OverlayStyleScan {
    /// (start, end, resolved style) for overlays whose face resolves to
    /// a non-default style (others can never change any query's
    /// result), sorted by `seq` (creation order) — that order is this
    /// struct's merge priority.
    entries: Vec<(usize, usize, Style)>,
    /// Indices into `entries`, sorted by start ascending.
    by_start: Vec<usize>,
    /// Indices into `entries`, sorted by end ascending.
    by_end: Vec<usize>,
    start_ptr: usize,
    end_ptr: usize,
    /// Entries currently covering the last-queried `pos`, keyed by
    /// `entries` index so iterating this set visits them in merge-
    /// priority order.
    active: std::collections::BTreeSet<usize>,
}

impl OverlayStyleScan {
    pub fn new(interp: &Interp, ed: &Editor, buffer: &Buffer) -> OverlayStyleScan {
        let mut entries: Vec<(usize, usize, Style, u64)> = Vec::new();
        if let Some(face_sym) = interp.intern_soft("face") {
            for ov in &buffer.overlays {
                let ov = ov.borrow();
                if ov.start >= ov.end {
                    continue; // zero/negative-length: never covers any pos
                }
                let mut style = Style::default();
                merge_face(interp, ed, &ov.get(face_sym), &mut style);
                if style != Style::default() {
                    entries.push((ov.start, ov.end, style, ov.seq));
                }
            }
        }
        // `buffer.overlays` is walked above in its current start-sorted
        // order (P1.3), not creation order — restore creation order
        // (this struct's merge priority, see its doc) explicitly here.
        entries.sort_by_key(|e| e.3);
        let entries: Vec<(usize, usize, Style)> = entries
            .into_iter()
            .map(|(s, e, st, _)| (s, e, st))
            .collect();
        let mut by_start: Vec<usize> = (0..entries.len()).collect();
        by_start.sort_by_key(|&i| entries[i].0);
        let mut by_end: Vec<usize> = (0..entries.len()).collect();
        by_end.sort_by_key(|&i| entries[i].1);
        OverlayStyleScan {
            entries,
            by_start,
            by_end,
            start_ptr: 0,
            end_ptr: 0,
            active: std::collections::BTreeSet::new(),
        }
    }

    /// Style at `pos`, which must be >= the `pos` passed to every prior
    /// call on this scan.
    pub fn at(&mut self, pos: usize) -> Style {
        while self.start_ptr < self.by_start.len()
            && self.entries[self.by_start[self.start_ptr]].0 <= pos
        {
            self.active.insert(self.by_start[self.start_ptr]);
            self.start_ptr += 1;
        }
        while self.end_ptr < self.by_end.len() && self.entries[self.by_end[self.end_ptr]].1 <= pos {
            self.active.remove(&self.by_end[self.end_ptr]);
            self.end_ptr += 1;
        }
        let mut style = Style::default();
        for &i in &self.active {
            merge_styles(&mut style, &self.entries[i].2);
        }
        style
    }
}

fn merge_face(interp: &Interp, ed: &Editor, face: &Value, style: &mut Style) {
    match face {
        Value::Nil => {}
        Value::Sym(id) => {
            if let Some(named) = ed.faces.get(id) {
                merge_styles(style, named);
            }
        }
        Value::Cons(_) => {
            if let Some(items) = face.list_to_vec() {
                // Either a plist (:foreground "#ff0000" ...) or a list of faces.
                if items
                    .first()
                    .map(|v| matches!(v, Value::Sym(id) if interp.is_keyword(*id)))
                    .unwrap_or(false)
                {
                    let plist_style = parse_face_plist(interp, &items);
                    merge_styles(style, &plist_style);
                } else {
                    for item in &items {
                        merge_face(interp, ed, item, style);
                    }
                }
            }
        }
        _ => {}
    }
}

fn merge_styles(base: &mut Style, over: &Style) {
    if over.fg.is_some() {
        base.fg = over.fg;
    }
    if over.bg.is_some() {
        base.bg = over.bg;
    }
    base.bold |= over.bold;
    base.italic |= over.italic;
    if over.underline != Underline::None {
        base.underline = over.underline;
        base.underline_color = over.underline_color;
    }
    base.reverse |= over.reverse;
}

pub fn parse_face_plist(interp: &Interp, items: &[Value]) -> Style {
    let mut style = Style::default();
    let mut i = 0;
    while i + 1 < items.len() {
        let key = match &items[i] {
            Value::Sym(id) => interp.sym_name(*id),
            _ => {
                i += 2;
                continue;
            }
        };
        let val = &items[i + 1];
        match key {
            ":foreground" => style.fg = parse_color(val),
            ":background" => style.bg = parse_color(val),
            ":weight" => {
                if let Value::Sym(id) = val {
                    style.bold = interp.sym_name(*id) == "bold";
                }
            }
            ":slant" => {
                if let Value::Sym(id) = val {
                    style.italic = interp.sym_name(*id) == "italic";
                }
            }
            // `:underline` accepts t, nil, or real Emacs's plist form
            // `(:style wave :color "#ff0000")`.
            ":underline" => match val {
                Value::Cons(_) => {
                    if let Some(uitems) = val.list_to_vec() {
                        style.underline = Underline::Straight;
                        let mut j = 0;
                        while j + 1 < uitems.len() {
                            if let Value::Sym(id) = &uitems[j] {
                                match interp.sym_name(*id) {
                                    ":style" => {
                                        if let Value::Sym(s) = &uitems[j + 1] {
                                            if interp.sym_name(*s) == "wave" {
                                                style.underline = Underline::Wave;
                                            }
                                        }
                                    }
                                    ":color" => {
                                        style.underline_color = parse_color(&uitems[j + 1]);
                                    }
                                    _ => {}
                                }
                            }
                            j += 2;
                        }
                    }
                }
                v if v.truthy() => style.underline = Underline::Straight,
                _ => {}
            },
            ":inverse-video" => style.reverse = val.truthy(),
            _ => {}
        }
        i += 2;
    }
    style
}

pub fn parse_color(v: &Value) -> Option<Color> {
    let Value::Str(s) = v else { return None };
    let s = s.as_str();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some((r, g, b));
        }
    }
    match s {
        "black" => Some((0, 0, 0)),
        "red" => Some((205, 49, 49)),
        "green" => Some((13, 188, 121)),
        "yellow" => Some((229, 229, 16)),
        "blue" => Some((36, 114, 200)),
        "magenta" => Some((188, 63, 188)),
        "cyan" => Some((17, 168, 205)),
        "white" => Some((229, 229, 229)),
        "gray" | "grey" => Some((128, 128, 128)),
        "orange" => Some((230, 126, 34)),
        "purple" => Some((155, 89, 182)),
        _ => None,
    }
}

/// The position that starts the next visual row after the one containing
/// `pos` (which must itself be a row start), or None at end of buffer.
fn next_row_start(
    buffer: &Buffer,
    pos: usize,
    cols: usize,
    inv: &[(usize, usize)],
) -> Option<usize> {
    let len = buffer.text.len();
    if pos >= len {
        return None;
    }
    let mut p = pos;
    let mut col = 0usize;
    let mut chars = buffer.text.chars_from(p);
    while p < len {
        if let Some(end) = invisible_end(p, inv) {
            p = end;
            col += 3; // the "..." indicator
            chars = buffer.text.chars_from(p);
            continue;
        }
        let c = chars.next().unwrap();
        if c == '\n' {
            return Some(p + 1);
        }
        let w = char_width(c, col);
        if wraps_before(col, w, cols) {
            return Some(p);
        }
        col += w;
        p += 1;
    }
    None
}

/// Scroll `window_start` so that `point` is visible in text_rows.
///
/// Point near `window_start` (a page or two either way) keeps the old
/// row-by-row smooth-scroll behavior. A big jump (`M->`, `M-<`, a large
/// `goto-char`, ...) instead recenters directly — GNU Emacs's own
/// redisplay does the same rather than walking every intervening row.
/// The "is this near or far" test has to be O(1): scanning to find out
/// would just reintroduce the cost this is meant to avoid. It uses
/// character distance as a proxy for row distance, which is safe in both
/// directions because a visual row can show at most `cols` characters:
/// `char_distance / cols <= row_distance <= char_distance`. So a large
/// char distance guarantees a large row distance (safe to recenter
/// outright), and a small char distance guarantees a small row distance
/// (safe to use the smooth path). The remaining middle ground — long
/// char distance made of many short/blank lines, where row distance
/// could still be large — is caught by `scan_forward_for_point`'s own
/// bounded guard, which falls back to recenter if it runs out of budget.
fn ensure_point_visible(
    buffer: &Buffer,
    point: usize,
    window_start: &mut usize,
    cols: usize,
    text_rows: usize,
    inv: &[(usize, usize)],
) {
    const NEAR_PAGES: usize = 3;
    let far_chars = (NEAR_PAGES * text_rows).saturating_mul(cols.max(1)).max(1);

    if point < *window_start {
        if *window_start - point > far_chars {
            recenter(buffer, point, window_start, cols, text_rows, inv);
        } else {
            *window_start = buffer.text.line_start(point);
        }
        return;
    }
    if point - *window_start > far_chars {
        recenter(buffer, point, window_start, cols, text_rows, inv);
        return;
    }
    let near_guard = NEAR_PAGES * text_rows + 4;
    if !scan_forward_for_point(
        buffer,
        point,
        window_start,
        cols,
        text_rows,
        inv,
        near_guard,
    ) {
        recenter(buffer, point, window_start, cols, text_rows, inv);
    }
}

/// Advance `window_start` forward row-by-row (the pre-existing
/// smooth-scroll loop) until `point` fits within `text_rows`, trying at
/// most `max_steps` advances. Returns false — leaving `window_start`
/// wherever it got to — when the budget runs out, so the caller can fall
/// back to a cheaper strategy instead of scanning indefinitely.
fn scan_forward_for_point(
    buffer: &Buffer,
    point: usize,
    window_start: &mut usize,
    cols: usize,
    text_rows: usize,
    inv: &[(usize, usize)],
    max_steps: usize,
) -> bool {
    let mut guard = 0usize;
    loop {
        let mut row = 0usize;
        let mut p = *window_start;
        let mut fits = false;
        while row < text_rows {
            let next = next_row_start(buffer, p, cols, inv);
            let row_end = next.unwrap_or(buffer.text.len() + 1);
            if point < row_end || (next.is_none() && point <= buffer.text.len()) {
                fits = true;
                break;
            }
            match next {
                Some(n) => {
                    p = n;
                    row += 1;
                }
                None => break,
            }
        }
        if fits {
            return true;
        }
        guard += 1;
        if guard > max_steps {
            return false;
        }
        match next_row_start(buffer, *window_start, cols, inv) {
            Some(n) => *window_start = n,
            None => return true, // ran off the end of the buffer; nothing more to do
        }
    }
}

/// Recenter: jump `window_start` straight to roughly `text_rows / 2`
/// *logical* lines above `point` (GNU Emacs's recenter semantics), then
/// run the smooth-scroll scan as a cheap correction pass. Logical lines
/// only approximate visual rows — line-wrap and invisible (org-folded)
/// regions can shift the count — so the correction pass is what actually
/// guarantees point ends up on screen; it's cheap here because point and
/// window_start are normally only a page or two apart.
///
/// That correction pass has its own small step budget, which can run out
/// when point sits deep inside one enormous wrapped logical line (a
/// minified-JS/CSS-style file, a giant log line, ...) — backing up whole
/// *logical* lines doesn't help there, since point's line is still the
/// same one huge line. Falling back to merely `line_start(point)` in
/// that case would be wrong: it points window_start at the right line,
/// but point itself can still be hundreds of wrapped rows further down
/// and off-screen. So the fallback instead finds the exact answer with
/// `exact_rows_before_point` — a single linear pass, cost proportional to
/// how deep point is into that one line rather than to the whole buffer,
/// which is the "pay for it only when it's actually pathological" cost
/// the reviewer signed off on.
fn recenter(
    buffer: &Buffer,
    point: usize,
    window_start: &mut usize,
    cols: usize,
    text_rows: usize,
    inv: &[(usize, usize)],
) {
    let half = (text_rows / 2).max(1);
    *window_start = backward_n_lines(buffer, point, half);
    let guard = 2 * text_rows + 4;
    if scan_forward_for_point(buffer, point, window_start, cols, text_rows, inv, guard) {
        return;
    }
    let line_start = buffer.text.line_start(point);
    *window_start = exact_rows_before_point(buffer, line_start, point, cols, inv, half);
}

/// The start of the logical line `n` lines above the one containing
/// `pos` (clamped at the start of the buffer). Char-based, like Emacs's
/// `backward-line` — not visual rows. Each step is one `line_start` scan
/// back to the previous newline, so the cost is O(n × average line
/// length) — independent of how far `pos` is from the start of the
/// buffer, *unless* `pos`'s own line is itself huge, in which case that
/// first `line_start` scan is O(pos's offset into that line). That edge
/// case is what `exact_rows_before_point` exists to handle correctly.
fn backward_n_lines(buffer: &Buffer, pos: usize, n: usize) -> usize {
    let mut p = buffer.text.line_start(pos);
    for _ in 0..n {
        if p == 0 {
            break;
        }
        p = buffer.text.line_start(p - 1);
    }
    p
}

/// The exact `window_start` that puts `point` `rows_before`-many visual
/// rows below it (or as close to that as the buffer allows), found with
/// a single forward pass from `start` — a small ring buffer remembers
/// only the last `rows_before + 1` row-start positions seen, so this is
/// O(rows from `start` to point) with no per-step rescanning, unlike
/// `scan_forward_for_point` (which re-walks up to `text_rows` rows from
/// scratch on every step and is only cheap because its caller bounds the
/// number of steps). That makes this safe to run with no step budget at
/// all: correct in every case, including point sitting deep inside one
/// giant wrapped logical line, and its cost scales with how far into
/// that line point is — not with the size of the rest of the buffer.
fn exact_rows_before_point(
    buffer: &Buffer,
    start: usize,
    point: usize,
    cols: usize,
    inv: &[(usize, usize)],
    rows_before: usize,
) -> usize {
    let keep = rows_before.max(1);
    let mut ring: std::collections::VecDeque<usize> =
        std::collections::VecDeque::with_capacity(keep + 1);
    ring.push_back(start);
    let mut p = start;
    loop {
        let next = next_row_start(buffer, p, cols, inv);
        let row_end = next.unwrap_or(buffer.text.len() + 1);
        if point < row_end || (next.is_none() && point <= buffer.text.len()) {
            break; // [p, row_end) is point's own row.
        }
        match next {
            Some(n) => {
                p = n;
                if ring.len() == keep {
                    ring.pop_front();
                }
                ring.push_back(p);
            }
            None => break,
        }
    }
    // The oldest entry still in the ring is exactly `keep` rows before
    // point's row, or `start` itself if point's row is closer than that.
    *ring.front().unwrap_or(&start)
}

/// A screen-cell rectangle: one window pane's slice of the frame, or the
/// root area handed to `compute_rects`. `pub(crate)` since M45 -- evil's
/// `C-w h/j/k/l` direction picker (`select-window-in-direction`,
/// builtins/ui.rs) reads these fields straight off `window_rects`' output
/// to compare candidate windows geometrically.
#[derive(Clone, Copy)]
pub(crate) struct Rect {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

/// Flatten the window layout tree into (window_id, screen_rect) pairs.
fn compute_rects(layout: &crate::editor::Layout, rect: Rect, out: &mut Vec<(usize, Rect)>) {
    use crate::editor::Layout;
    match layout {
        Layout::Leaf(id) => out.push((*id, rect)),
        Layout::Split { horizontal, a, b } => {
            if *horizontal {
                let sep = if rect.width > 2 { 1 } else { 0 };
                let aw = (rect.width.saturating_sub(sep)) / 2;
                let bw = rect.width.saturating_sub(aw + sep);
                compute_rects(a, Rect { width: aw, ..rect }, out);
                compute_rects(
                    b,
                    Rect {
                        col: rect.col + aw + sep,
                        width: bw,
                        ..rect
                    },
                    out,
                );
            } else {
                let ah = rect.height / 2;
                let bh = rect.height - ah;
                compute_rects(a, Rect { height: ah, ..rect }, out);
                compute_rects(
                    b,
                    Rect {
                        row: rect.row + ah,
                        height: bh,
                        ..rect
                    },
                    out,
                );
            }
        }
    }
}

/// Terminal size plus the vertical split between the tiled-window area and
/// the reserved panel/echo rows: `(cols, rows, panel_h, windows_height)`.
/// Shared by `render` and `window_rects` (M45) so the two can never drift
/// apart -- `panel_h`/`windows_height` mirror the M21 selector-panel math
/// exactly (an open panel reserves the bottom third of the frame, above
/// the echo row; windows tile into what's left).
fn frame_layout(editor: &Editor) -> (usize, usize, usize, usize) {
    let (cols, rows) = editor.frame;
    // Degenerate terminal sizes (0 during pty startup) must not crash.
    let cols = cols.max(4);
    let rows = rows.max(3);
    let panel_h = editor
        .minibuffer
        .as_ref()
        .and_then(|mb| mb.panel.as_ref())
        .map(|_| (rows / 3).max(4).min(rows.saturating_sub(5)))
        .unwrap_or(0);
    let windows_height = rows - 1 - panel_h; // last row is the shared echo area
    (cols, rows, panel_h, windows_height)
}

/// (window_id, screen_rect) for every window pane currently on screen --
/// the single source of truth for window geometry (M45): `render` uses
/// this same function to lay out the frame, and `select-window-in-
/// direction` (builtins/ui.rs, evil's `C-w h/j/k/l`) uses it to compare
/// windows geometrically without duplicating the split math or the panel/
/// echo-row height deduction above.
pub(crate) fn window_rects(editor: &Editor) -> Vec<(usize, Rect)> {
    let (cols, _rows, _panel_h, windows_height) = frame_layout(editor);
    let root = Rect {
        row: 0,
        col: 0,
        width: cols,
        height: windows_height,
    };
    let mut rects = Vec::new();
    compute_rects(&editor.layout, root, &mut rects);
    rects
}

/// Task 3 (mouse support): scroll window `win_id`'s `window_start` by
/// `delta_lines` *logical* lines -- positive scrolls forward (later text
/// comes into view, matching wheel-down), negative scrolls backward --
/// without touching `point`, the one thing every keyboard scroll path in
/// this editor does NOT offer (`evil-scroll-down`/`evil-scroll-up` both
/// move point; see `evil.el`'s own header comment: "no elisp-level
/// window-start control is exposed"). No-op when `win_id` names no
/// window.
///
/// **Logical lines, not visual rows**: unlike `ensure_point_visible`'s
/// machinery above, this does not reproduce the wrap-aware `cols`/
/// gutter-width computation `render_window` uses to find a window's real
/// text width -- duplicating that just for wheel scrolling isn't worth
/// it for a first cut, and a wheel event's "how many lines" is already
/// an approximation on every platform. The practical effect: a window
/// showing heavily-wrapped long lines scrolls somewhat more per wheel
/// tick than a visual-row-accurate version would. Good enough for
/// Verilog source, which is not typically wrapped at GUI widths.
pub fn scroll_window_start(ed: &Rc<RefCell<Editor>>, win_id: usize, delta_lines: i64) {
    let mut editor = ed.borrow_mut();
    let Some(win) = editor.windows.get(&win_id) else {
        return;
    };
    let buf = win.buffer.clone();
    let start = win.window_start;
    // Fix 3 (mouse-support milestone review): current point, read from
    // `buf` when this is the selected window (its point lives there, not
    // in `win.point`) and from `win.point` otherwise -- same split
    // `render_window` already makes. Recorded as the scroll pin below.
    let is_selected = win_id == editor.selected_window;
    let point = if is_selected {
        buf.borrow().point
    } else {
        win.point
    };
    let new_start = {
        let b = buf.borrow();
        if delta_lines >= 0 {
            forward_n_lines(&b, start, delta_lines as usize)
        } else {
            backward_n_lines(&b, start, (-delta_lines) as usize)
        }
    };
    if let Some(win) = editor.windows.get_mut(&win_id) {
        win.window_start = new_start;
        // Pin: as long as point stays exactly here, `render_window` won't
        // recentre this window out from under the scroll -- see
        // `Window::scroll_pin`'s doc.
        win.scroll_pin = Some(point);
    }
}

/// The start of the logical line `n` lines below the one containing
/// `pos` (clamped at the end of the buffer) -- forward counterpart to
/// `backward_n_lines`, used only by `scroll_window_start` (wheel
/// scrolling has no other caller that needs to walk forward by a raw
/// line count rather than a visual row).
fn forward_n_lines(buffer: &Buffer, pos: usize, n: usize) -> usize {
    let len = buffer.text.len();
    let mut p = pos;
    for _ in 0..n {
        if p >= len {
            break;
        }
        match buffer.text.chars_from(p).position(|c| c == '\n') {
            Some(off) => p = (p + off + 1).min(len),
            None => {
                p = len;
                break;
            }
        }
    }
    p
}

/// Truthy check on a global elisp variable, for render-time feature
/// toggles (hl-line-mode, lsp--clients). Reads the raw global cell —
/// wrong for a variable that may be buffer-local (see `buffer_var_on`),
/// fine for these, which aren't.
fn var_on(interp: &Interp, name: &str) -> bool {
    interp
        .intern_soft(name)
        .and_then(|id| interp.sym_value(id))
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// Truthy check on a variable as seen by `buf` specifically (M24),
/// honoring buffer-local bindings the same way the `buffer-local-value`
/// builtin does — needed for per-window toggles like
/// `display-line-numbers`, where `render_window` may be painting a
/// buffer that isn't the current buffer (a second window on a different
/// buffer) and a plain global-cell read (`var_on`) would answer for the
/// wrong buffer. See `editor::buffer_local_value` for the swap
/// semantics. `pub(crate)` since M37: `highlight.rs`'s `apply_visible`
/// reuses this exact same buffer-local read for `rainbow-delimiters-
/// mode`, the identical need one module over.
pub(crate) fn buffer_var_on(
    interp: &Interp,
    ed: &Editor,
    buf: &Rc<RefCell<Buffer>>,
    name: &str,
) -> bool {
    interp
        .intern_soft(name)
        .and_then(|id| crate::editor::buffer_local_value(interp, ed, id, buf))
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// A buffer-local string variable's value (M28 `mode-line-prefix`), the
/// same buffer-local-aware read as `buffer_var_on`. `None` covers nil,
/// void/never-set, and any non-string value alike — all of which mean
/// "nothing to show", leaving the modeline exactly as it was before M28.
fn buffer_var_str(
    interp: &Interp,
    ed: &Editor,
    buf: &Rc<RefCell<Buffer>>,
    name: &str,
) -> Option<Rc<String>> {
    interp
        .intern_soft(name)
        .and_then(|id| crate::editor::buffer_local_value(interp, ed, id, buf))
        .and_then(|v| match v {
            Value::Str(s) => Some(s),
            _ => None,
        })
}

/// A named face from the editor's face table, or `fallback`.
fn face_or(interp: &Interp, ed: &Editor, name: &str, fallback: Style) -> Style {
    interp
        .intern_soft(name)
        .and_then(|id| ed.faces.get(&id).copied())
        .unwrap_or(fallback)
}

/// Diagnostic severity color, resolved from the `diagnostic-error` /
/// `diagnostic-warning` / `diagnostic-info` faces (M-visual-quality) so
/// it's theme-driven; falls back to the original hard-coded tuples when
/// the face is absent, following `face_or`'s pattern.
fn severity_color(interp: &Interp, ed: &Editor, sev: u8) -> Color {
    let (face_name, fallback) = match sev {
        1 => ("diagnostic-error", (244, 71, 71)),   // error: red
        2 => ("diagnostic-warning", (255, 204, 0)), // warning: yellow
        _ => ("diagnostic-info", (58, 150, 221)),   // info/hint: blue
    };
    face_or(
        interp,
        ed,
        face_name,
        Style {
            fg: Some(fallback),
            ..Style::default()
        },
    )
    .fg
    .unwrap_or(fallback)
}

/// Render one window pane's buffer and modeline into `rect`. Returns this
/// window's published layout (task 2) plus the hardware cursor position
/// when this window is selected -- `None` for two cases: the degenerate
/// panes `frame_layout`'s `.max(...)` floors are meant to guard against
/// (`rect.height < 2 || rect.width == 0`, same as before this function
/// grew a `WindowLayout` return value), and the unrelated case just below,
/// `editor.windows.get(&win_id)?`, when `win_id` has no entry in
/// `editor.windows` at all.
fn render_window(
    interp: &Interp,
    editor: &mut Editor,
    win_id: usize,
    rect: Rect,
    is_selected: bool,
    grid: &mut Grid,
) -> Option<(WindowLayout, Option<(usize, usize)>)> {
    if rect.height < 2 || rect.width == 0 {
        return None;
    }
    let text_rows = rect.height - 1;

    let buf = editor.windows.get(&win_id)?.buffer.clone();
    let point = if is_selected {
        buf.borrow().point
    } else {
        editor.windows[&win_id].point
    };

    // Line-number gutter (M16): `NN │`-style prefix column, giving the
    // text area the remaining width. Diagnostics show as a colored dot
    // in the gutter on their line. `display-line-numbers` is commonly
    // buffer-local (M24: prog-mode-hook turns it on per-buffer), so this
    // must read it as seen by `buf` — the window being painted here may
    // not be the currently-selected one.
    let mut gutter_w = if buffer_var_on(interp, editor, &buf, "display-line-numbers") {
        // total_lines() is O(1) (M24: incrementally maintained newline
        // count), so this is cheap even on a huge buffer — but it's still
        // only computed when the gutter is actually shown.
        let total_lines = buf.borrow().text.total_lines();
        let digits = total_lines.to_string().len().max(2);
        digits + 2 // number, diagnostic-dot column, space
    } else {
        0
    };
    if rect.width < gutter_w + 8 {
        gutter_w = 0; // degenerate pane: give the text every column
    }
    let cols = rect.width - gutter_w;
    let diag_lines: std::collections::HashMap<usize, u8> = editor
        .diagnostics
        .get(&(Rc::as_ptr(&buf) as usize))
        .map(|v| {
            let mut m = std::collections::HashMap::new();
            for &(line, sev) in v {
                let e = m.entry(line).or_insert(sev);
                *e = (*e).min(sev); // keep the most severe (lowest code)
            }
            m
        })
        .unwrap_or_default();
    let ln_face = face_or(
        interp,
        editor,
        "line-number",
        Style {
            fg: Some((110, 110, 110)),
            ..Style::default()
        },
    );
    let ln_cur_face = face_or(
        interp,
        editor,
        "line-number-current-line",
        Style {
            fg: Some((200, 200, 200)),
            ..Style::default()
        },
    );

    let inv = invisible_ranges(interp, &buf.borrow());
    let mut window_start = editor.windows[&win_id].window_start;
    // Fix 3 (mouse-support milestone review): an explicit scroll pins this
    // window against `ensure_point_visible`'s recentre as long as point
    // hasn't moved since the scroll -- see `Window::scroll_pin`'s doc.
    // Otherwise the wheel-scroll flow was self-defeating: `render_window`
    // runs every frame, so the very next frame after a wheel event would
    // recentre right back to point, undoing the scroll a trackpad's burst
    // of wheel events reaches in a single gesture.
    let pinned = editor.windows[&win_id].scroll_pin == Some(point);
    if pinned {
        // Keep `window_start` exactly as the scroll left it; don't call
        // `ensure_point_visible` at all this frame.
    } else {
        editor.windows.get_mut(&win_id).unwrap().scroll_pin = None;
        let b = buf.borrow();
        ensure_point_visible(&b, point, &mut window_start, cols, text_rows, &inv);
    }
    editor.windows.get_mut(&win_id).unwrap().window_start = window_start;

    let b = buf.borrow();
    let len = b.text.len();
    // Preprocess overlay face styles once per window render (P1.5), so
    // the main loop below queries a scanline instead of re-scanning
    // every overlay for every visible character. See `OverlayStyleScan`.
    let mut style_scan = OverlayStyleScan::new(interp, editor, &b);

    // Selection region (M16): mark..point painted with the region face's
    // background, live only in the selected window while the mark is on.
    let region = if is_selected && b.mark_active {
        b.mark.map(|m| (m.min(point), m.max(point)))
    } else {
        None
    };
    let region_bg = face_or(
        interp,
        editor,
        "region",
        Style {
            bg: Some((38, 79, 120)),
            ..Style::default()
        },
    )
    .bg;

    let point_line = b.text.line_number(point);
    let mut line_no = b.text.line_number(window_start);
    let at_line_start =
        window_start == 0 || b.text.char_at(window_start.saturating_sub(1)) == Some('\n');

    let mut row = 0usize;
    let mut col = 0usize;
    let mut pos = window_start;
    let mut cursor: Option<(usize, usize)> = None;
    let dim = Style {
        fg: Some((128, 128, 128)),
        ..Style::default()
    };

    let paint_gutter = |grid: &mut Grid, row: usize, number: Option<usize>, gutter_w: usize| {
        if gutter_w == 0 || row >= text_rows {
            return;
        }
        let digits = gutter_w - 2;
        match number {
            Some(n) => {
                let styled = if n == point_line {
                    ln_cur_face
                } else {
                    ln_face
                };
                let s = format!("{:>width$}", n, width = digits);
                for (i, ch) in s.chars().enumerate() {
                    grid.put(rect.row + row, rect.col + i, ch, styled);
                }
                // Diagnostic dot column (0-based diag lines).
                if let Some(&sev) = diag_lines.get(&(n - 1)) {
                    let dot = Style {
                        fg: Some(severity_color(interp, editor, sev)),
                        ..Style::default()
                    };
                    grid.put(rect.row + row, rect.col + digits, '●', dot);
                }
            }
            None => {
                for i in 0..gutter_w.saturating_sub(1) {
                    grid.put(rect.row + row, rect.col + i, ' ', ln_face);
                }
            }
        }
    };
    paint_gutter(grid, 0, at_line_start.then_some(line_no), gutter_w);

    let tx = rect.col + gutter_w; // text area origin column
    let mut chars = b.text.chars_from(pos);
    // Task 1 (mouse support): accumulates `grid.runs` for the buffer
    // text painted below -- see `RunBuilder`'s doc. Chrome (gutter, the
    // wrap-continuation backslash, mode line, echo, panel, popup) is
    // *not* built here; it's filled in afterward by a post-pass over the
    // finished grid (see `render`'s call to `fill_chrome_runs`), which
    // needs no byte-position bookkeeping since its runs are all `src:
    // None`.
    let mut run = RunBuilder::new();
    run.start_marker(grid, rect.row + row, tx, b.text.char_to_byte(pos));
    while row < text_rows {
        if pos == point && cursor.is_none() {
            cursor = Some((rect.row + row, tx + col.min(cols.saturating_sub(1))));
        }
        if pos >= len {
            break;
        }
        if let Some(end) = invisible_end(pos, &inv) {
            if point > pos && point < end && cursor.is_none() {
                cursor = Some((rect.row + row, tx + col.min(cols.saturating_sub(1))));
            }
            for (i, ch) in "...".chars().enumerate() {
                grid.put(rect.row + row, tx + col + i, ch, dim);
            }
            run.push_atomic(
                grid,
                RunPos {
                    row: rect.row + row,
                    col: tx + col,
                    cols: 3,
                },
                "...".to_string(),
                dim,
                Some(b.text.char_to_byte(pos)..b.text.char_to_byte(end)),
            );
            col += 3;
            pos = end;
            chars = b.text.chars_from(pos);
            continue;
        }
        let c = chars.next().unwrap();
        if c == '\n' {
            row += 1;
            col = 0;
            pos += 1;
            line_no += 1;
            paint_gutter(grid, row, Some(line_no), gutter_w);
            if row < text_rows {
                run.start_marker(grid, rect.row + row, tx, b.text.char_to_byte(pos));
            }
            continue;
        }
        let mut style = style_scan.at(pos);
        if let Some((rs, re)) = region {
            if pos >= rs && pos < re {
                style.bg = region_bg.or(style.bg);
            }
        }
        let w = char_width(c, col);
        if wraps_before(col, w, cols) {
            grid.put(rect.row + row, tx + cols - 1, '\\', dim);
            row += 1;
            col = 0;
            paint_gutter(grid, row, None, gutter_w);
            if row >= text_rows {
                break;
            }
            run.start_marker(grid, rect.row + row, tx, b.text.char_to_byte(pos));
        }
        let byte_start = b.text.char_to_byte(pos);
        let byte_end = byte_start + c.len_utf8();
        match c {
            '\t' => {
                let w = char_width('\t', col);
                for i in 0..w {
                    grid.put(rect.row + row, tx + col + i, ' ', style);
                }
                run.push_atomic(
                    grid,
                    RunPos {
                        row: rect.row + row,
                        col: tx + col,
                        cols: w,
                    },
                    " ".repeat(w),
                    style,
                    Some(byte_start..byte_end),
                );
                col += w;
            }
            c if (c as u32) < 32 => {
                grid.put(rect.row + row, tx + col, '^', style);
                let esc = char::from_u32((c as u32) + 64).unwrap_or('?');
                grid.put(rect.row + row, tx + col + 1, esc, style);
                run.push_atomic(
                    grid,
                    RunPos {
                        row: rect.row + row,
                        col: tx + col,
                        cols: 2,
                    },
                    format!("^{}", esc),
                    style,
                    Some(byte_start..byte_end),
                );
                col += 2;
            }
            '\u{7f}' => {
                grid.put(rect.row + row, tx + col, '^', style);
                grid.put(rect.row + row, tx + col + 1, '?', style);
                run.push_atomic(
                    grid,
                    RunPos {
                        row: rect.row + row,
                        col: tx + col,
                        cols: 2,
                    },
                    "^?".to_string(),
                    style,
                    Some(byte_start..byte_end),
                );
                col += 2;
            }
            c => {
                let w = char_width(c, col);
                if w == 2 {
                    grid.put_wide(rect.row + row, tx + col, c, style);
                } else {
                    grid.put(rect.row + row, tx + col, c, style);
                }
                run.push(
                    grid,
                    RunPos {
                        row: rect.row + row,
                        col: tx + col,
                        cols: w,
                    },
                    c,
                    style,
                    Some(byte_start..byte_end),
                );
                col += w;
            }
        }
        pos += 1;
    }
    run.flush(grid);
    if cursor.is_none() {
        cursor = Some((
            rect.row + row.min(text_rows.saturating_sub(1)),
            tx + col.min(cols.saturating_sub(1)),
        ));
    }

    // Current-line highlight (M16, hl-line-mode): tint the cursor row's
    // background wherever nothing set one — region/selection colors win.
    if is_selected && var_on(interp, "hl-line-mode") {
        if let Some((crow, _)) = cursor {
            if crow < rect.row + text_rows {
                let hl_bg = face_or(
                    interp,
                    editor,
                    "hl-line",
                    Style {
                        bg: Some((42, 45, 46)),
                        ..Style::default()
                    },
                )
                .bg;
                for c in 0..rect.width {
                    let cell = &mut grid.lines[crow][rect.col + c];
                    if cell.style.bg.is_none() {
                        cell.style.bg = hl_bg;
                    }
                }
            }
        }
    }

    // Segmented modeline (M16): "NAME * mode" left, "!N LSP L:C" right,
    // VS Code status-bar style, themable via mode-line faces.
    let mode_row = rect.row + rect.height - 1;
    let col_no = point - b.text.line_start(point);
    let mode_name = match &b.major_mode {
        Value::Sym(id) => interp.sym_name(*id).to_string(),
        _ => "Fundamental".to_string(),
    };
    let base = if is_selected {
        face_or(
            interp,
            editor,
            "mode-line",
            Style {
                reverse: true,
                ..Style::default()
            },
        )
    } else {
        face_or(
            interp,
            editor,
            "mode-line-inactive",
            Style {
                reverse: true,
                fg: Some((100, 100, 100)),
                ..Style::default()
            },
        )
    };
    let name_style = Style { bold: true, ..base };
    let diag_count = diag_lines.len();
    // M63: per-buffer, not global. `lsp--clients' (the old source here) is
    // a single editor-wide list -- once ANY buffer connects to ANY server,
    // that flips true for every window, including buffers that were never
    // attached (measured: jumping from a connected buffer to `M-.'-visited
    // core/alu.sv showed "LSP" lit while `M-?' there reported no
    // connection). `lsp--buffer-client' is the buffer-local slot every LSP
    // command actually checks, so reading it per-`buf' here is what makes
    // "this segment is on" and "M-? works" the same claim.
    //
    // Known residual: this is a truthy check on the variable, not a
    // liveness probe -- redisplay runs every frame and has no business
    // poking a subprocess that often. If the server dies, the segment
    // stays lit until this buffer happens to go through
    // `lsp--live-buffer-client' (the diagnostics/hover/definition path),
    // which clears the stale reference. The bias is toward "stays on too
    // long", not "goes dark too early".
    let lsp_on = buffer_var_on(interp, editor, &buf, "lsp--buffer-client");
    let right_style = if diag_count > 0 {
        Style {
            fg: Some(severity_color(interp, editor, 2)),
            ..base
        }
    } else {
        base
    };
    // M19: title is "dirname/buffername" for file buffers; buffers with
    // a default_directory but no file (dired, eshell) show the plain
    // name and put the ~-abbreviated directory on the right segment.
    let title = match &b.file {
        Some(f) => {
            let dir_last = std::path::Path::new(f)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string());
            match dir_last {
                Some(d) if !d.is_empty() => format!("{}/{}", d, b.name),
                _ => b.name.clone(),
            }
        }
        None => b.name.clone(),
    };
    let dir = if b.file.is_none() {
        b.default_directory.as_deref()
    } else {
        None
    };
    // M28: an optional buffer-local tag ahead of the name — evil-mode
    // uses this for its state indicator ("<N> NAME * mode"). Unset (the
    // default) leaves the modeline exactly as it was before M28.
    let prefix = buffer_var_str(interp, editor, &buf, "mode-line-prefix").unwrap_or_default();
    // M69 review F9: only look this up when `dir` is actually going to
    // be shown -- every other window on every other keystroke has no
    // use for it, and `render_window` runs once per window per frame.
    let home = if dir.is_some() {
        std::env::var("HOME").ok()
    } else {
        None
    };

    let parts = ModeLineParts {
        prefix: prefix.as_str(),
        title: &title,
        modified: b.modified,
        mode_name: &mode_name,
        dir,
        home: home.as_deref(),
        lsp: lsp_on,
        diags: diag_count,
        line: point_line,
        col: col_no,
    };
    let rect_w = rect.width;
    let ml = compose_mode_line(&parts, rect_w);

    // M69: `left_limit` is the structural guarantee that the left and
    // right segments can never collide — the left segment is physically
    // incapable of drawing into the right segment's columns, regardless
    // of what either string contains.
    let right_w = ml_width(&ml.right);
    let left_limit = rect_w.saturating_sub(right_w);
    let mut mcol = draw_ml_segment(grid, mode_row, rect.col, 0, left_limit, &ml.left, |c| {
        if c < ml.name_cols {
            name_style
        } else {
            base
        }
    });
    while mcol < left_limit {
        grid.put(mode_row, rect.col + mcol, ' ', base);
        mcol += 1;
    }
    mcol = draw_ml_segment(grid, mode_row, rect.col, mcol, rect_w, &ml.right, |_| {
        right_style
    });
    while mcol < rect_w {
        grid.put(mode_row, rect.col + mcol, ' ', base);
        mcol += 1;
    }

    let layout = WindowLayout {
        win_id,
        row: rect.row,
        col: rect.col,
        rows: rect.height,
        cols: rect.width,
        gutter_cols: gutter_w,
        mode_line_row: mode_row,
    };
    let cursor = if is_selected { cursor } else { None };
    Some((layout, cursor))
}

/// The `default` face — themes set it; frontends use it as the frame's
/// ground colors (every cell whose style leaves fg/bg unset).
pub fn frame_base_style(interp: &Interp, ed: &Editor) -> Style {
    face_or(interp, ed, "default", Style::default())
}

/// One display column of the echo row, after `echo_cells` expansion.
/// `cont` marks a column that's the placeholder half of the wide
/// character drawn in the previous cell (mirrors `Cell::continuation`
/// on `Grid`, and `put_wide`'s own second-cell write).
struct EchoCell {
    ch: char,
    cont: bool,
}

/// Expand the echo row's raw text into one `EchoCell` per display
/// column, one column at a time, left to right. Same `display_width`
/// rule set the buffer-drawing loop uses (M67): `\t`, C0 controls,
/// DEL, double-width chars, everything else -- deliberately kept
/// identical so M70 only adds scrolling, not a second rendering of
/// the same text.
///
/// This has to be a separate expansion pass, not folded into the
/// drawing loop the way it used to be, because `echo_scroll_off`
/// needs to know how many columns the WHOLE text takes before it can
/// pick a starting offset -- and a `\t`'s width depends on the column
/// it starts at (`8 - col % 8`), so there's no way to sum widths
/// without walking from column 0 first. Expansion therefore always
/// starts at 0 regardless of any later scroll offset; `echo_scroll_off`
/// only slices into this result afterward, it doesn't change how the
/// text expands (so a `\t`'s stop lines up with what you'd see if
/// nothing were scrolled).
fn echo_cells(text: &str) -> Vec<EchoCell> {
    let mut cells = Vec::new();
    let mut col = 0;
    for c in text.chars() {
        match display_width::Expansion::classify(c, col) {
            display_width::Expansion::Tab { width } => {
                for _ in 0..width {
                    cells.push(EchoCell {
                        ch: ' ',
                        cont: false,
                    });
                }
                col += width;
            }
            display_width::Expansion::Control(ctrl) => {
                cells.push(EchoCell {
                    ch: '^',
                    cont: false,
                });
                let second = if ctrl == '\u{7f}' {
                    '?'
                } else {
                    char::from_u32((ctrl as u32) + 64).unwrap_or('?')
                };
                cells.push(EchoCell {
                    ch: second,
                    cont: false,
                });
                col += 2;
            }
            display_width::Expansion::Wide(wc) => {
                cells.push(EchoCell {
                    ch: wc,
                    cont: false,
                });
                cells.push(EchoCell {
                    ch: ' ',
                    cont: true,
                });
                col += 2;
            }
            display_width::Expansion::Plain(pc) => {
                cells.push(EchoCell {
                    ch: pc,
                    cont: false,
                });
                col += 1;
            }
        }
    }
    cells
}

/// Drawn in the leftmost column whenever the echo row is scrolled
/// (`off > 0`), covering up whatever cell would otherwise be there.
const ECHO_ELLIPSIS: char = '…';

/// Pick a starting display column so `caret` stays inside a `win`-wide
/// visible window over `total` columns of content.
///
/// Stateless by design: called fresh every frame from `caret`/`total`/
/// `win`, with no scroll position stored on `Minibuffer` itself.
/// `Minibuffer.cursor` already has on the order of 15 separate write
/// sites across `commands.rs` (the movement keys, history browsing,
/// completion accept, panel selection...); a scroll-position field
/// would be a 16th thing every one of those has to remember to keep in
/// sync, and the cost of missing one is a silently stale scroll
/// position, not a compile error. Recomputing from scratch means
/// there's nothing to miss.
///
/// `total + 1`, not `total`, throughout: `caret` ranges over
/// `0..=total` (the caret can sit one column past the last character,
/// e.g. an empty input or point at end-of-line), so there are
/// `total + 1` caret positions to be able to show, not `total`.
///
/// Precondition: `caret <= total` -- every call site derives `caret`
/// from the SAME text `total` was computed from (a prompt+input
/// expansion, or the isearch query's own cell count), so it can never
/// legitimately exceed the text's own length. Not clamped internally
/// on purpose (review finding 9): silently clamping an out-of-range
/// caret would turn a caller bug into a caret that just quietly
/// doesn't track where it should, instead of failing loudly in debug
/// builds.
fn echo_scroll_off(total: usize, caret: usize, win: usize) -> usize {
    debug_assert!(
        caret <= total,
        "caret {caret} out of range for total {total} columns"
    );
    if win == 0 {
        return 0;
    }
    if total < win {
        return 0; // Even the caret's own column fits -- no need to scroll.
    }
    let max_off = (total + 1).saturating_sub(win);
    caret.saturating_sub(win - 1).min(max_off)
}

pub fn render(interp: &Interp, ed: &Rc<RefCell<Editor>>) -> Grid {
    let mut editor = ed.borrow_mut();
    // M45: `frame_layout`/`window_rects` are the single source of truth
    // for the cols/rows/panel_h/windows_height numbers below and the
    // per-window rects derived from them -- see their own doc comments.
    let (cols, rows, panel_h, windows_height) = frame_layout(&editor);
    let mut grid = Grid::new(cols, rows);
    let rects = window_rects(&editor);

    let mut cursor = (0, 0);
    let mut selected_rect: Option<Rect> = None;
    for (win_id, rect) in rects {
        let is_selected = win_id == editor.selected_window;
        if is_selected {
            selected_rect = Some(rect);
        }
        if let Some((layout, c)) =
            render_window(interp, &mut editor, win_id, rect, is_selected, &mut grid)
        {
            grid.windows.push(layout);
            if let Some(c) = c {
                cursor = c;
            }
        }
    }
    grid.cursor = cursor;

    // LSP completion popup (M40-4): cursor-anchored, drawn straight onto
    // the shared grid (unlike `hover_popup`'s free text, which each
    // frontend presents its own way) so TUI and GUI render it
    // identically. After the window loop (so it sits on top of the
    // buffer text it's anchored to) and before the echo area below (so a
    // pathologically short frame's message row always wins any overlap
    // -- `render_lsp_completion_popup` itself also never targets that
    // row, see its own doc comment, but this ordering is the belt to
    // that suspenders). Needs the selected window's own rect (M40-4
    // review fix #2) to keep the popup inside that window's text area --
    // never missing if `completion_popup` is set, since `selected_window`
    // always names a window `compute_rects` produced a rect for.
    if let (Some(popup), Some(rect)) = (&editor.completion_popup, selected_rect) {
        render_lsp_completion_popup(interp, &editor, popup, cursor, rect, &mut grid);
    }

    // Echo area / minibuffer (M32: three layers, highest priority
    // first — the minibuffer prompt/input; a one-shot `editor.echo'
    // message (e.g. `(message ...)'); and, only when neither of those
    // has anything to show, the CURRENT buffer's `echo-area-fallback'
    // buffer-local variable — a persistent state indicator such as
    // evil-mode's "-- INSERT --" that stays up between messages and
    // reappears on its own once a transient message is cleared (every
    // key clears `editor.echo' first — see commands.rs's `handle_key').
    let echo_row = rows - 1;
    let echo_text = if let Some(mb) = &editor.minibuffer {
        format!(
            "{}{}{}",
            mb.prompt,
            mb.input,
            mb.note.as_deref().unwrap_or("")
        )
    } else if let Some(msg) = &editor.echo {
        msg.clone()
    } else {
        buffer_var_str(interp, &editor, &editor.current, "echo-area-fallback")
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    // M70: horizontal scrolling. Expand the text into display columns
    // once (`echo_cells` -- the same five-way `\t`/C0/DEL/wide-char/
    // plain rules the old inline loop drew directly, see its own doc
    // comment for why this has to be a separate step), then compute a
    // caret column in that SAME coordinate space -- not a `char_width`
    // sum over raw chars, which undercounts a `\t` whenever anything
    // scrolls it off column 0, since a tab's width depends on the
    // column it starts at.
    //
    // Only the minibuffer and isearch have a caret to keep in view.
    // A bare `message`/`echo-area-fallback` has no point at all, so it
    // keeps the pre-M70 always-truncate-from-0 behavior (`off` stays
    // 0) -- mid-string ellipsis for those is a different, undesigned
    // feature (M70 spec 3.5), not this milestone.
    let cells = echo_cells(&echo_text);
    // M70 review fix (finding 2): the caret used to be the SUM of two
    // independently-expanded pieces (`echo_cells(&mb.prompt).len() +
    // echo_cells(&input_prefix).len()`), but `echo_cells` always starts
    // counting tab stops from column 0 -- so a tab inside the input
    // prefix got measured as if it started at column 0, not at the end
    // of the prompt. A single expansion of `prompt + input_prefix`
    // together gives the tab the same column-relative width it gets
    // when the whole `echo_text` is expanded below, which is what the
    // caret actually needs to index into (`cells`).
    let mb_caret = editor.minibuffer.as_ref().map(|mb| {
        let input_prefix: String = mb.input.chars().take(mb.cursor).collect();
        echo_cells(&format!("{}{}", mb.prompt, input_prefix)).len()
    });
    // M70 review fix (finding 6): `isearch_live()`, not a bare
    // `.is_some()` -- a stale session (buffer switched out from under
    // it, see `Editor::isearch_live`'s own doc comment) must not be
    // treated as having a caret; `handle_key` hasn't necessarily
    // cleared `editor.isearch` yet by the time a frame renders.
    let caret = mb_caret.or_else(|| editor.isearch_live().then_some(cells.len()));
    // M70 review fix (finding 1): the echo row is the terminal's LAST
    // row, so its last column is the screen's bottom-right cell.
    // `frontend-tui`'s setup (`frontend-tui/src/lib.rs`) enters the
    // alternate screen and hides the cursor but never disables
    // autowrap, so writing into that cell is a classic "pending wrap"
    // scroll hazard on terminals that haven't disabled it. The pre-M70
    // code never wrote it either -- its `if ecol + w >= cols { break; }`
    // guard used `>=`, one column short of `cols` -- but that was never
    // written down as an invariant anywhere, so it didn't survive M70's
    // rewrite to a per-column loop on the first pass. `win` is
    // therefore the usable width, one less than the grid's actual
    // `cols`: column `cols - 1` is deliberately left untouched below
    // (its `Cell::default()` is a plain space), which also makes the
    // plain `message`/fallback path draw byte-for-byte the same as
    // before M70.
    let win = cols.saturating_sub(1);
    let off = caret
        .map(|c| echo_scroll_off(cells.len(), c, win))
        .unwrap_or(0);
    // M-visual-quality: the echo row used to draw with `Style::default()`
    // regardless of theme; now it picks up the `echo-area` face, falling
    // back to the old plain style when the face is absent.
    let echo_style = face_or(interp, &editor, "echo-area", Style::default());
    for i in 0..win {
        let idx = off + i;
        // Left-edge indicator: tells the reader there's more of the
        // path/text before what's visible -- the thing most likely to
        // cause confusion when a path gets cut off. No right-edge
        // indicator (M70 spec 3.4): the common case is typing at the
        // end, where there's nothing past the visible window anyway,
        // and a second indicator would just be another edge condition
        // fighting the caret for the same column.
        if i == 0 && off > 0 {
            grid.put(echo_row, 0, ECHO_ELLIPSIS, echo_style);
            continue;
        }
        match cells.get(idx) {
            Some(cell) if cell.cont => {
                // This column is the placeholder half of a wide char
                // whose first half got scrolled out of view -- draw
                // blank rather than half a character.
            }
            Some(cell) => {
                let is_wide = cells.get(idx + 1).map(|n| n.cont).unwrap_or(false);
                if is_wide {
                    if idx + 1 < off + win {
                        grid.put_wide(echo_row, i, cell.ch, echo_style);
                    }
                    // else: the wide char's second column would land
                    // past the right edge of the window -- leave this
                    // column blank instead of drawing off the edge.
                } else {
                    grid.put(echo_row, i, cell.ch, echo_style);
                }
            }
            None => grid.put(echo_row, i, ' ', echo_style),
        }
    }
    if let Some(mb) = &editor.minibuffer {
        // Cursor sits in the echo area while the minibuffer is active.
        // `.min(win - 1)` is a safety clamp, not the primary logic --
        // `echo_scroll_off`'s invariant (`off <= caret < off + win`)
        // already keeps `caret - off` in range; this only guards a
        // degenerate frame slipping past `frame_layout`'s `.max(4)`.
        // Clamping to `win - 1` (not `cols - 1`) also keeps the cursor
        // off the last column for the same reason `win`'s own comment
        // above gives for content.
        let mb_caret = mb_caret.expect("mb_caret is Some whenever minibuffer is Some");
        grid.cursor = (echo_row, mb_caret.saturating_sub(off).min(win - 1));
        // Completion popup (M18): candidate rows stacked directly above
        // the minibuffer, shared by both frontends via the grid.
        if let Some(cs) = &mb.completion {
            render_completion_popup(interp, &editor, cs, echo_row, cols, &mut grid);
        }
        // Selector panel (M21): the reserved bottom-third block.
        if let Some(panel) = &mb.panel {
            render_panel(
                interp,
                &editor,
                panel,
                windows_height,
                panel_h,
                cols,
                &mut grid,
            );
        }
    }
    fill_chrome_runs(&mut grid);
    grid
}

/// Task 1 (mouse support), second half: `render_window` already built
/// byte-accurate `PaintRun`s (`src: Some(...)`) for buffer text as it
/// painted; this covers everything else the frame drew directly into
/// `grid.lines` without going through a `RunBuilder` at all -- the
/// wrap-continuation backslash, the line-number gutter, both mode-line
/// segments, the echo area, the completion popup, and the selector
/// panel. All of those are chrome (`src: None`), so rather than thread a
/// `RunBuilder` through `draw_ml_segment`/`paint_gutter`/the echo loop/
/// `render_panel`/the popups -- five call sites with their own closures
/// and loops -- this walks the *finished* grid once, row by row, and
/// turns every screen cell not already claimed by a buffer-text run
/// into a same-style chrome run. Cheap (one pass over `cols * rows`
/// cells, once per frame) and correct by construction: it can't
/// possibly disagree with what `lines` actually shows, because it reads
/// `lines` directly instead of re-deriving what should have been drawn.
fn fill_chrome_runs(grid: &mut Grid) {
    // Buffer-text runs already cover to some columns per row; chrome
    // fills in whatever's left. `covered[row]` is that row's list of
    // (start, end) buffer-run column ranges, used below to skip them.
    let mut covered: Vec<Vec<(usize, usize)>> = vec![Vec::new(); grid.rows];
    for r in &grid.runs {
        if r.src.is_some() && r.row < grid.rows {
            covered[r.row].push((r.col, r.col + r.cols));
        }
    }
    for row in covered.iter_mut() {
        row.sort_unstable();
    }
    let mut chrome: Vec<PaintRun> = Vec::new();
    for (row_idx, line) in grid.lines.iter().enumerate() {
        let is_covered = |col: usize| covered[row_idx].iter().any(|&(s, e)| col >= s && col < e);
        let mut open: Option<PaintRun> = None;
        for (col, cell) in line.iter().enumerate() {
            if cell.continuation {
                // Fix 2 (mouse-support milestone review): a double-width
                // chrome character (a CJK buffer name in the mode line, a
                // CJK message in the echo area, ...) is two grid cells --
                // the glyph's own cell plus this continuation cell, which
                // carries no character of its own (`Cell::put_wide`).
                // `RunBuilder::push` keeps buffer-text runs' `cols`
                // separate from their char count for exactly this reason
                // (see its doc); do the same here instead of leaving the
                // continuation cell claimed by no run at all, which used
                // to make `grid.runs` a column short for that row.
                if let Some(r) = open.as_mut() {
                    if r.col + r.cols == col {
                        r.cols += 1;
                        continue;
                    }
                }
                // No open run ends exactly here -- the lead cell must have
                // been claimed by something else (unexpected shape; chrome
                // painting never draws a continuation cell without its
                // lead beside it). Don't guess: close whatever's open and
                // leave this cell unclaimed, same as the old behavior.
                if let Some(r) = open.take() {
                    chrome.push(r);
                }
                continue;
            }
            if is_covered(col) {
                if let Some(r) = open.take() {
                    chrome.push(r);
                }
                continue;
            }
            let mergeable = open
                .as_ref()
                .is_some_and(|r| r.col + r.cols == col && r.style == cell.style);
            if mergeable {
                let r = open.as_mut().unwrap();
                r.text.push(cell.ch);
                r.cols += 1;
            } else {
                if let Some(r) = open.take() {
                    chrome.push(r);
                }
                open = Some(PaintRun {
                    row: row_idx,
                    col,
                    cols: 1,
                    text: cell.ch.to_string(),
                    style: cell.style,
                    src: None,
                    atomic: false,
                });
            }
        }
        if let Some(r) = open.take() {
            chrome.push(r);
        }
    }
    grid.runs.extend(chrome);
}

/// Draw the M21 selector panel into its reserved rows
/// `[top, top + height)`: a dim separator line, then candidate rows,
/// the selected one highlighted; scroll follows the selection.
fn render_panel(
    interp: &Interp,
    editor: &Editor,
    panel: &crate::editor::PanelState,
    top: usize,
    height: usize,
    cols: usize,
    grid: &mut Grid,
) {
    if height == 0 {
        return;
    }
    let base = face_or(interp, editor, "panel", Style::default());
    let hi = face_or(
        interp,
        editor,
        "panel-selected",
        Style {
            reverse: true,
            ..base
        },
    );
    // Separator line.
    for col in 0..cols {
        grid.put(top, col, '─', base);
    }
    let visible = height.saturating_sub(1);
    if visible == 0 {
        return;
    }
    let n = panel.rows.len();
    let shown = n.min(visible);
    let first = if n <= visible {
        0
    } else {
        panel.selected.saturating_sub(visible - 1).min(n - visible)
    };
    for row_i in 0..visible {
        let row = top + 1 + row_i;
        let idx = first + row_i;
        let selected = idx == panel.selected && idx < n;
        let row_style = if selected { hi } else { base };
        for col in 0..cols {
            grid.put(row, col, ' ', row_style);
        }
        if row_i >= shown {
            continue;
        }
        let mut col = 1; // one cell of left padding
        for (text, face) in &panel.rows[idx].segments {
            // The selected row keeps a uniform highlight; other rows
            // color each segment by its face.
            let seg_style = if selected {
                hi
            } else {
                face_or(interp, editor, face, base)
            };
            for c in text.chars() {
                let w = char_width(c, col);
                if col + w >= cols {
                    break;
                }
                if w == 2 {
                    grid.put_wide(row, col, c, seg_style);
                } else {
                    grid.put(row, col, c, seg_style);
                }
                col += w;
            }
        }
    }
}

/// Rows of the TAB-completion popup drawn above the minibuffer line.
const POPUP_MAX_ROWS: usize = 10;

fn render_completion_popup(
    interp: &Interp,
    editor: &Editor,
    cs: &crate::editor::CompletionState,
    echo_row: usize,
    cols: usize,
    grid: &mut Grid,
) {
    let n = cs.candidates.len();
    if n == 0 || echo_row == 0 {
        return;
    }
    let visible = n.min(POPUP_MAX_ROWS).min(echo_row);
    // Keep the highlighted candidate inside the window.
    let first = cs.selected.saturating_sub(visible - 1).min(n - visible);
    let base = face_or(interp, editor, "completions-popup", Style::default());
    let hi = face_or(interp, editor, "completions-selected", base);
    for row_i in 0..visible {
        let row = echo_row - visible + row_i;
        let cand = &cs.candidates[first + row_i];
        let style = if first + row_i == cs.selected {
            hi
        } else {
            base
        };
        // Paint the full-width row background, then the candidate text.
        for col in 0..cols {
            grid.put(row, col, ' ', style);
        }
        let mut col = 1; // one cell of left padding
        for c in cand.chars() {
            let w = char_width(c, col);
            if col + w >= cols {
                break;
            }
            if w == 2 {
                grid.put_wide(row, col, c, style);
            } else {
                grid.put(row, col, c, style);
            }
            col += w;
        }
    }
}

/// Column cap on the LSP completion popup's width (M40-4) — a label
/// longer than this is truncated, same "don't let one candidate blow up
/// the whole popup" spirit as `POPUP_MAX_ROWS` caps row count.
const LSP_COMPLETION_MAX_WIDTH: usize = 40;

/// Draw the M40-4 LSP completion popup anchored at `cursor`, clamped to
/// `win_rect` (the selected window's own screen rect -- what `render`
/// passes is the SAME `Rect` its window loop just painted that window's
/// buffer text and modeline into): below the cursor's row when every
/// visible row fits before the window's modeline, above it when the
/// space above the cursor is bigger than the space below (clamped to
/// whichever side is actually used, never just assumed to fit), and not
/// drawn at all when neither side has room (e.g. a one-line-tall
/// window). The popup NEVER covers `win_rect`'s modeline row and NEVER
/// covers the cursor's own row -- both are meaningful content, not
/// blank space it can paint over. (Earlier revisions of this function
/// clamped only to the frame's echo row, which let a tall enough
/// candidate list overrun the modeline or the cursor's row itself in a
/// short window -- see the M40-4 review's issue #2.)  Width is the
/// longest label's DISPLAY width (double-width/CJK-aware, matching the
/// draw loop below -- issue #4), capped at `LSP_COMPLETION_MAX_WIDTH`
/// AND at `win_rect`'s own width -- a candidate list can't force a
/// narrow split window's popup wider than the window itself, or it
/// would paint into whatever window sits to its right (M40-4 review's
/// second pass on issue #2: the first pass only fixed the vertical
/// axis). `start_col` is likewise clamped to `win_rect`'s horizontal
/// span: shifted left when anchoring at the cursor's column would run
/// past the window's own right edge, never the frame's. Scrolls to
/// keep the selection visible past `POPUP_MAX_ROWS`, same
/// windowing idea as `render_completion_popup` (the unrelated M18
/// minibuffer-TAB popup), just against the window's rect instead of the
/// echo row. Reuses the `completions-popup`/`completions-selected` faces
/// that popup already established — no new theme entries needed.
fn render_lsp_completion_popup(
    interp: &Interp,
    editor: &Editor,
    popup: &crate::editor::CompletionPopup,
    cursor: (usize, usize),
    win_rect: Rect,
    grid: &mut Grid,
) {
    let n = popup.items.len();
    if n == 0 {
        return;
    }
    let (cursor_row, cursor_col) = cursor;
    // `win_rect`'s horizontal span, `[win_left, win_right)` -- same
    // exclusive-upper-bound shape as `text_top`/`text_bottom` below, and
    // what keeps the popup from painting into a neighboring window when
    // `win_rect` is narrower than the whole frame (a horizontal
    // `split-window`).
    let win_left = win_rect.col;
    let win_right = win_rect.col + win_rect.width;
    if win_right <= win_left {
        return; // degenerate zero-width window: nowhere to draw at all
    }
    // `win_rect`'s text area is `[text_top, text_bottom)` -- `text_bottom`
    // is exactly the window's modeline row (`render_window`'s `mode_row`
    // is `rect.row + rect.height - 1`), so it's an exclusive bound the
    // popup must never reach. The cursor's own row is likewise off
    // limits on both sides, hence `below_room` counting from
    // `cursor_row + 1` and `above_room` stopping short of `cursor_row`
    // itself.
    let text_top = win_rect.row;
    let text_bottom = win_rect.row + win_rect.height.saturating_sub(1);
    let below_room = text_bottom.saturating_sub(cursor_row + 1);
    let above_room = cursor_row.saturating_sub(text_top);
    let wanted = n.min(POPUP_MAX_ROWS);
    let (top, visible) = if below_room >= wanted {
        (cursor_row + 1, wanted)
    } else if above_room >= below_room {
        let v = wanted.min(above_room);
        if v == 0 {
            return;
        }
        (cursor_row - v, v)
    } else {
        let v = wanted.min(below_room);
        if v == 0 {
            return;
        }
        (cursor_row + 1, v)
    };

    let max_label = popup
        .items
        .iter()
        .map(|item| {
            let mut w = 0usize;
            for c in item.label.chars() {
                w += char_width(c, w);
            }
            w
        })
        .max()
        .unwrap_or(0);
    let width = max_label
        .clamp(1, LSP_COMPLETION_MAX_WIDTH)
        .min(win_right - win_left);
    // `width <= win_right - win_left` (just clamped above), so
    // `win_right - width >= win_left` -- this range is always valid,
    // never an inverted `clamp` call.
    let start_col = cursor_col.clamp(win_left, win_right - width);

    let base = face_or(interp, editor, "completions-popup", Style::default());
    let hi = face_or(interp, editor, "completions-selected", base);
    let first = popup
        .selected
        .saturating_sub(visible.saturating_sub(1))
        .min(n.saturating_sub(visible));
    for row_i in 0..visible {
        let row = top + row_i;
        let idx = first + row_i;
        let label = &popup.items[idx].label;
        let style = if idx == popup.selected { hi } else { base };
        for col in 0..width {
            grid.put(row, start_col + col, ' ', style);
        }
        let mut col = 0;
        for c in label.chars() {
            let w = char_width(c, col);
            if col + w > width {
                break;
            }
            if w == 2 {
                grid.put_wide(row, start_col + col, c, style);
            } else {
                grid.put(row, start_col + col, c, style);
            }
            col += w;
        }
    }
}

/// Register a named face style (used by the set-face builtin).
pub fn define_face(ed: &Rc<RefCell<Editor>>, name: SymId, style: Style) {
    ed.borrow_mut().faces.insert(name, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthChar;

    // M69 T*: `compose_mode_line` is the pure half of mode-line layout —
    // same shape as `complete.rs`'s `abbreviate_home_with_cases` and
    // `panel.rs`'s tests around its own `place_for` helper (injected
    // `home` instead of touching the process-global `$HOME`). Each test
    // builds `ModeLineParts` with struct-update syntax off
    // `Default::default()` (M69 review F1: a 10-argument positional
    // helper tripped clippy's `too_many_arguments` and was unreadable
    // at call sites besides) -- only the fields a given test cares
    // about are named, everything else is `""`/`None`/`false`/`0`.

    #[test]
    fn u1_wide_terminal_all_segments() {
        let p = ModeLineParts {
            title: "bus/axi4_lite_arbiter.sv",
            mode_name: "verilog-mode",
            dir: Some("/home/u/rtl"),
            home: Some("/home/u"),
            lsp: true,
            diags: 2,
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 120);
        let dir_pos = ml.right.find("~/rtl").expect("dir segment present");
        let diag_pos = ml.right.find("!2").expect("diag segment present");
        let lsp_pos = ml.right.find("LSP").expect("lsp segment present");
        let lc_pos = ml.right.find("L1:0").expect("L:C segment present");
        assert!(dir_pos < diag_pos && diag_pos < lsp_pos && lsp_pos < lc_pos);
        assert!(ml.right.ends_with("L1:0"));
    }

    #[test]
    fn u2_narrow_window_keeps_line_col() {
        let p = ModeLineParts {
            prefix: "<1> ",
            title: "bus/axi4_lite_arbiter.sv",
            mode_name: "verilog-mode",
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 40);
        assert_eq!(ml.right, "L1:0");
        assert!(!ml.left.contains("verilog-mode"));
        assert!(ml.left.contains("bus/axi4_lite_arbiter.sv"));
    }

    #[test]
    fn u3_deep_dired_path_drops_dir_first() {
        let p = ModeLineParts {
            title: "axi_read_engine",
            mode_name: "dired-mode",
            dir: Some("/home/eng/work/soc_project/hw/rtl/subsystem/dma/axi_read_engine"),
            home: Some("/home/eng"),
            line: 4,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 80);
        assert_eq!(ml.right, "L4:0");
        assert!(ml_width(&ml.left) + ml_width(&ml.right) <= 80);
    }

    #[test]
    fn u4_diags_beat_mode_name() {
        // left w/o mode name: " title" = 6; right base "L1:0" = 4;
        // "!3  " = 4; "  Fundamental" = 15. Pick a width that fits the
        // diag segment but not the mode-name segment.
        let p = ModeLineParts {
            title: "title",
            mode_name: "Fundamental",
            diags: 3,
            line: 1,
            ..Default::default()
        };
        let width = ml_width(" title") + ml_width("L1:0") + ML_GAP + ml_width("!3  ");
        let ml = compose_mode_line(&p, width);
        assert!(ml.right.contains("!3"));
        assert!(!ml.left.contains("Fundamental"));
    }

    #[test]
    fn u5_mode_name_beats_lsp() {
        let p = ModeLineParts {
            title: "title",
            mode_name: "Fundamental",
            lsp: true,
            line: 1,
            ..Default::default()
        };
        let width = ml_width(" title") + ml_width("L1:0") + ML_GAP + ml_width("  Fundamental");
        let ml = compose_mode_line(&p, width);
        assert!(ml.left.contains("Fundamental"));
        assert!(!ml.right.contains("LSP"));
    }

    #[test]
    fn u6_title_truncates_keeping_tail() {
        let long_title = "very_long_common_prefix_axi4_lite_arbiter_unique_tail.sv";
        let p = ModeLineParts {
            title: long_title,
            mode_name: "verilog-mode",
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 30);
        assert!(ml.left.contains(ML_ELLIPSIS));
        // M69 review F6: `ends_with` implies `contains`, so an `||` of
        // the two degenerated the assertion into just the weaker half.
        assert!(ml.left.ends_with("unique_tail.sv"));
        assert!(ml_width(&ml.left) + ml_width(&ml.right) <= 30);
    }

    #[test]
    fn u7_extreme_narrow_no_panic() {
        for width in 1..=12 {
            let p = ModeLineParts {
                title: "x",
                mode_name: "m",
                line: 1,
                ..Default::default()
            };
            let ml = compose_mode_line(&p, width);
            assert!(
                ml_width(&ml.left) + ml_width(&ml.right) <= width,
                "width={} left={:?} right={:?}",
                width,
                ml.left,
                ml.right
            );
        }
    }

    #[test]
    fn u8_sanitizes_control_chars() {
        let p = ModeLineParts {
            prefix: "\t\u{1}\u{7f}",
            title: "title",
            mode_name: "m",
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 120);
        assert!(ml.left.contains("^I"));
        assert!(ml.left.contains("^A"));
        assert!(ml.left.contains("^?"));
        for s in [&ml.left, &ml.right] {
            assert!(!s.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}'));
        }
    }

    #[test]
    fn u9_cjk_width_not_char_count() {
        let title = "專案專案專案"; // 6 chars, 12 columns
        let p = ModeLineParts {
            title,
            mode_name: "m",
            line: 1,
            ..Default::default()
        };
        // Budget wide enough for 6 chars but not 12 columns.
        let fixed = ml_width(" ") + ml_width("L1:0") + ML_GAP;
        let width = fixed + 9; // 9 < 12 (columns) but > 6 (chars)
        let ml = compose_mode_line(&p, width);
        assert!(ml_width(&ml.left) <= 9 + ml_width(" "));
        assert_ne!(ml.left, format!(" {}", title));
    }

    #[test]
    fn u10_name_cols_is_columns_not_chars() {
        let p = ModeLineParts {
            title: "專案",
            mode_name: "m",
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 120);
        assert_eq!(ml.name_cols, 1 + 4);

        // M69 review F4: `name_cols` must not grow to cover the
        // modified-buffer " *" marker -- the pre-M69 code's
        // `name_chars` never counted it either, and it's not part of
        // the buffer's name.
        let pm = ModeLineParts {
            title: "專案",
            mode_name: "m",
            modified: true,
            line: 1,
            ..Default::default()
        };
        let ml_modified = compose_mode_line(&pm, 120);
        assert_eq!(ml_modified.name_cols, 1 + 4);
        assert!(ml_modified.left.contains(" *"));
    }

    /// The first `name_cols` columns of `left` -- the run the caller
    /// draws in the bold `mode-line` name face.
    fn bold_run(ml: &ModeLine) -> String {
        let mut out = String::new();
        let mut col = 0usize;
        for c in ml.left.chars() {
            let w = UnicodeWidthChar::width(c).unwrap_or(1).max(1);
            if col + w > ml.name_cols {
                break;
            }
            out.push(c);
            col += w;
        }
        out
    }

    #[test]
    fn u14_name_cols_excludes_the_marker_after_the_final_truncation() {
        // The width here is narrow enough that the gap-reservation
        // fallback truncates `left` outright. That truncation keeps the
        // *tail*, so a `name_cols` counted from the front of the
        // original string no longer describes the surviving columns --
        // clamping it with `min` isn't enough. Designed by the M69 tail
        // review (finding 1); at that point the bold run really did
        // cover the " *" marker.
        for width in 6..=24 {
            let p = ModeLineParts {
                title: "very_long_title_name_here_unique.sv",
                mode_name: "verilog-mode",
                modified: true,
                line: 1,
                ..Default::default()
            };
            let ml = compose_mode_line(&p, width);
            assert!(
                !bold_run(&ml).contains('*'),
                "width={} left={:?} name_cols={} bold={:?}",
                width,
                ml.left,
                ml.name_cols,
                bold_run(&ml)
            );
        }

        // Same check with the mode name still on the line: the suffix
        // is then " *" plus "  verilog-mode", and none of it is name.
        let p = ModeLineParts {
            title: "alu.sv",
            mode_name: "verilog-mode",
            modified: true,
            line: 1,
            ..Default::default()
        };
        let ml = compose_mode_line(&p, 40);
        assert!(ml.left.ends_with("verilog-mode"), "left={:?}", ml.left);
        assert_eq!(bold_run(&ml), " alu.sv");
    }

    #[test]
    fn u11_home_abbreviation_boundary() {
        let base_width = 60;
        let p1 = ModeLineParts {
            title: "buf",
            mode_name: "m",
            dir: Some("/home/u2/rtl"),
            home: Some("/home/u"),
            line: 1,
            ..Default::default()
        };
        let ml1 = compose_mode_line(&p1, base_width);
        assert!(ml1.right.contains("/home/u2/rtl"));
        assert!(!ml1.right.contains("~2"));

        let p2 = ModeLineParts {
            title: "buf",
            mode_name: "m",
            dir: Some("/home/u/rtl"),
            home: Some("/home/u"),
            line: 1,
            ..Default::default()
        };
        let ml2 = compose_mode_line(&p2, base_width);
        assert!(ml2.right.contains("~/rtl"));

        let p3 = ModeLineParts {
            title: "buf",
            mode_name: "m",
            dir: Some("/home/u"),
            home: Some("/home/u"),
            line: 1,
            ..Default::default()
        };
        let ml3 = compose_mode_line(&p3, base_width);
        assert!(ml3.right.contains('~'));
    }

    #[test]
    fn u12_invariant_scan() {
        let p = ModeLineParts {
            prefix: "<1> ",
            title: "bus/axi4_lite_arbiter.sv",
            modified: true,
            mode_name: "verilog-mode",
            dir: Some("/home/u/rtl/top"),
            home: Some("/home/u"),
            lsp: true,
            diags: 3,
            line: 42,
            col: 7,
        };
        let lc = "L42:7";
        for width in 1..=200 {
            let ml = compose_mode_line(&p, width);
            assert!(
                ml_width(&ml.left) + ml_width(&ml.right) <= width,
                "width={} left={:?} right={:?}",
                width,
                ml.left,
                ml.right
            );
            if width >= ml_width(lc) {
                assert!(
                    ml.right.ends_with(lc),
                    "width={} right={:?}",
                    width,
                    ml.right
                );
            }
        }
    }

    #[test]
    fn u13_gap_reserved_even_in_degenerate_widths() {
        // M69 review F3: u7/u12 only check the *sum* doesn't overrun
        // `width` -- that's satisfiable with `ML_GAP` squeezed to 0,
        // which is exactly the bug (title and `L:C` running together
        // with no separating space at all, observed on a real 9-column
        // TUI pane after three `C-x 3`s). This asserts the gap itself
        // survives, or `left` gives up and goes empty rather than
        // eating into it.
        let p = ModeLineParts {
            prefix: "<1> ",
            title: "bus/axi4_lite_arbiter.sv",
            mode_name: "verilog-mode",
            line: 1,
            ..Default::default()
        };
        for width in 1..=200 {
            let ml = compose_mode_line(&p, width);
            assert!(
                ml.left.is_empty() || ml_width(&ml.left) + ML_GAP + ml_width(&ml.right) <= width,
                "width={} left={:?} right={:?}",
                width,
                ml.left,
                ml.right
            );
        }
    }

    // M70 U*: `echo_cells`/`echo_scroll_off` are the pure halves of the
    // echo row's horizontal scrolling, same shape as `compose_mode_line`
    // above -- no `Editor`/`Grid` dependency, so the character-expansion
    // rules and the offset math can be checked directly.

    // The echo-row unit tests use an `e` prefix, not the `u` the
    // mode-line ones above use: both groups live in this one `mod
    // tests`, and a second `u1`..`u8` would make `cargo test --lib
    // u3` run one test from each group (M70 tail review).
    #[test]
    fn e1_echo_cells_five_way_expansion() {
        let c = echo_cells("a\tb");
        // 'a' (col 0->1), tab stops at col 8, 'b' at col 8.
        assert_eq!(c.len(), 1 + 7 + 1);
        assert_eq!(c[0].ch, 'a');
        assert!(c[1..8].iter().all(|cell| cell.ch == ' ' && !cell.cont));
        assert_eq!(c[8].ch, 'b');

        let c = echo_cells("\u{1}");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].ch, '^');
        assert_eq!(c[1].ch, 'A');

        let c = echo_cells("\u{7f}");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].ch, '^');
        assert_eq!(c[1].ch, '?');

        let c = echo_cells("專");
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].ch, '專');
        assert!(!c[0].cont);
        assert!(c[1].cont);

        let c = echo_cells("abc");
        assert_eq!(c.len(), 3);
        assert!(c.iter().map(|cell| cell.ch).eq("abc".chars()));
        assert!(c.iter().all(|cell| !cell.cont));
    }

    #[test]
    fn e2_tab_stop_is_column_dependent() {
        let short = echo_cells("a\tb");
        let long = echo_cells("abcdefgh\tb");
        // "a" -> tab fills columns 1..8 (7 cells) before "b" at col 8.
        let short_tab_cells = short.len() - 2; // minus 'a' and 'b'
                                               // "abcdefgh" ends exactly on a tab stop (col 8), so the next
                                               // tab fills a full 8 columns before "b" at col 16.
        let long_tab_cells = long.len() - 9; // minus the 8 letters and 'b'
        assert_eq!(short_tab_cells, 7);
        assert_eq!(long_tab_cells, 8);
    }

    #[test]
    fn e3_fits_without_scrolling() {
        assert_eq!(echo_scroll_off(10, 5, 20), 0);
        assert_eq!(echo_scroll_off(19, 19, 20), 0); // total+1 == win
    }

    #[test]
    fn e4_caret_at_end_scrolls_to_show_it() {
        let total = 100;
        let win = 20;
        let off = echo_scroll_off(total, total, win);
        assert_eq!(off, total + 1 - win);
        assert_eq!(total - off, win - 1);
    }

    #[test]
    fn e5_caret_at_start_no_scroll() {
        let off = echo_scroll_off(100, 0, 20);
        assert_eq!(off, 0);
    }

    #[test]
    fn e6_caret_in_middle_stays_in_window() {
        let total = 100;
        let win = 20;
        let caret = 50;
        let off = echo_scroll_off(total, caret, win);
        assert!(off <= caret && caret < off + win);
    }

    #[test]
    fn e7_echo_scroll_invariant_scan() {
        for total in (0..=200).step_by(7) {
            for caret in (0..=total).step_by(5) {
                for win in 1..=40 {
                    let off = echo_scroll_off(total, caret, win);
                    assert!(
                        off <= caret,
                        "total={total} caret={caret} win={win} off={off}"
                    );
                    assert!(
                        caret < off + win,
                        "total={total} caret={caret} win={win} off={off}"
                    );
                    let max_off = (total + 1).saturating_sub(win);
                    assert!(
                        off <= max_off,
                        "total={total} caret={caret} win={win} off={off} max_off={max_off}"
                    );
                }
            }
        }
    }

    #[test]
    fn e8_zero_width_window_does_not_panic() {
        assert_eq!(echo_scroll_off(100, 50, 0), 0);
    }
}
