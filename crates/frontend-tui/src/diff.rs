//! Frame-to-frame `Grid` diffing for the TUI backend (M23a).
//!
//! `redisplay::render` is cheap enough to call on every idle poll (post
//! P1.1), so we don't try to skip it. What used to be expensive is the
//! *terminal write*: the old `draw` re-wrote every cell of every frame,
//! even when nothing changed (idle poll timeout) or only a single
//! keystroke moved the point. This module compares the freshly rendered
//! `Grid` against the previous frame and emits crossterm output only for
//! the cells that actually changed, merging consecutive changed columns
//! on a row into a single `MoveTo` + run of writes.
//!
//! `diff_draw` is a pure function over `impl Write` so it can be unit
//! tested without a real terminal — see the `tests` module below.

use std::io::Write;

use crossterm::style::{Attribute, Color, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::{cursor, queue};

use core::redisplay::{Grid, Style};

fn to_crossterm_color(c: core::redisplay::Color) -> Color {
    Color::Rgb {
        r: c.0,
        g: c.1,
        b: c.2,
    }
}

fn apply_style(out: &mut impl Write, style: &Style) -> std::io::Result<()> {
    queue!(out, SetAttribute(Attribute::Reset))?;
    if let Some(fg) = style.fg {
        queue!(out, SetForegroundColor(to_crossterm_color(fg)))?;
    }
    if let Some(bg) = style.bg {
        queue!(out, SetBackgroundColor(to_crossterm_color(bg)))?;
    }
    if style.bold {
        queue!(out, SetAttribute(Attribute::Bold))?;
    }
    // Wave degrades to a straight underline in a terminal.
    if style.underline != core::redisplay::Underline::None {
        queue!(out, SetAttribute(Attribute::Underlined))?;
    }
    if style.italic {
        queue!(out, SetAttribute(Attribute::Italic))?;
    }
    if style.reverse {
        queue!(out, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

/// A maximal run of consecutive changed columns on one row: `[start, end)`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Run {
    row: usize,
    start: usize,
    end: usize,
}

/// Whether `(row, col)` differs between `prev` and `new`. `full` forces
/// every cell to be reported as changed (first frame, or a resize that
/// changed the grid's dimensions — the previous frame's coordinates
/// aren't even comparable anymore).
fn cell_changed(prev: Option<&Grid>, new: &Grid, row: usize, col: usize, full: bool) -> bool {
    if full {
        return true;
    }
    // `full` is only false when `prev` is `Some` and same-sized as `new`,
    // so this index is always in bounds.
    let p = &prev.expect("full is false only when prev is Some").lines[row][col];
    let n = &new.lines[row][col];
    p.ch != n.ch || p.continuation != n.continuation || p.style != n.style
}

/// Scans `new` against `prev` and returns the maximal per-row runs of
/// changed columns. Empty when the two grids render identically.
fn changed_runs(prev: Option<&Grid>, new: &Grid) -> Vec<Run> {
    let full = prev.is_none_or(|p| p.cols != new.cols || p.rows != new.rows);
    let mut runs = Vec::new();
    for row in 0..new.rows {
        let mut col = 0;
        while col < new.cols {
            if !cell_changed(prev, new, row, col, full) {
                col += 1;
                continue;
            }
            let start = col;
            while col < new.cols && cell_changed(prev, new, row, col, full) {
                col += 1;
            }
            runs.push(Run {
                row,
                start,
                end: col,
            });
        }
    }
    runs
}

/// Writes the minimal crossterm output needed to bring the terminal
/// display from `prev` (or a forced full redraw when `None`, e.g. the
/// first frame or after a resize) to `new`.
///
/// Returns `true` if anything — cell content or the hardware cursor
/// position — actually changed. Callers should skip flushing the
/// underlying writer when this returns `false`: on an idle poll timeout
/// with an unchanged editor state, that means zero bytes touch the
/// terminal at all.
pub fn diff_draw(out: &mut impl Write, prev: Option<&Grid>, new: &Grid) -> std::io::Result<bool> {
    let full = prev.is_none_or(|p| p.cols != new.cols || p.rows != new.rows);
    let runs = changed_runs(prev, new);
    let cursor_changed = full || prev.is_none_or(|p| p.cursor != new.cursor);
    if runs.is_empty() && !cursor_changed {
        return Ok(false);
    }

    queue!(out, cursor::Hide)?;
    queue!(out, SetAttribute(Attribute::Reset))?;
    let mut current_style = Style::default();
    for run in &runs {
        queue!(out, cursor::MoveTo(run.start as u16, run.row as u16))?;
        for col in run.start..run.end {
            let cell = &new.lines[run.row][col];
            if cell.continuation {
                continue; // the wide char before it already covers this column
            }
            if cell.style != current_style {
                apply_style(out, &cell.style)?;
                current_style = cell.style;
            }
            write!(out, "{}", cell.ch)?;
        }
    }
    queue!(out, SetAttribute(Attribute::Reset))?;
    let (crow, ccol) = new.cursor;
    queue!(out, cursor::MoveTo(ccol as u16, crow as u16), cursor::Show)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::redisplay::Cell;

    fn blank_grid(cols: usize, rows: usize) -> Grid {
        Grid {
            cols,
            rows,
            lines: vec![vec![Cell::default(); cols]; rows],
            cursor: (0, 0),
            windows: Vec::new(),
            runs: Vec::new(),
        }
    }

    // --- byte-count acceptance checks -------------------------------

    #[test]
    fn no_change_writes_nothing() {
        let a = blank_grid(80, 24);
        let b = blank_grid(80, 24);
        let mut buf = Vec::new();
        let wrote = diff_draw(&mut buf, Some(&a), &b).unwrap();
        assert!(!wrote);
        assert!(buf.is_empty());
    }

    #[test]
    fn single_cell_change_is_small() {
        let prev = blank_grid(80, 24);
        let mut new = blank_grid(80, 24);
        new.lines[5][10] = Cell {
            ch: 'X',
            continuation: false,
            style: Style::default(),
        };
        let mut buf = Vec::new();
        let wrote = diff_draw(&mut buf, Some(&prev), &new).unwrap();
        assert!(wrote);
        assert!(
            buf.len() < 200,
            "expected a small diff, got {} bytes: {:?}",
            buf.len(),
            buf
        );
    }

    #[test]
    fn cursor_only_move_still_writes_but_stays_small() {
        let mut prev = blank_grid(80, 24);
        prev.cursor = (0, 0);
        let mut new = blank_grid(80, 24);
        new.cursor = (3, 7);
        let mut buf = Vec::new();
        let wrote = diff_draw(&mut buf, Some(&prev), &new).unwrap();
        assert!(wrote);
        assert!(buf.len() < 200);
    }

    #[test]
    fn full_redraw_when_no_previous_frame_covers_every_cell() {
        let mut new = blank_grid(10, 4);
        new.lines[2][3] = Cell {
            ch: 'Z',
            continuation: false,
            style: Style::default(),
        };
        let runs = changed_runs(None, &new);
        // Every row is one contiguous run spanning the whole width.
        assert_eq!(runs.len(), new.rows);
        for (row, run) in runs.iter().enumerate() {
            assert_eq!(
                *run,
                Run {
                    row,
                    start: 0,
                    end: new.cols
                }
            );
        }
        let mut buf = Vec::new();
        let wrote = diff_draw(&mut buf, None, &new).unwrap();
        assert!(wrote);
        // A full 10x4 (40-cell) redraw writes at least one byte per
        // cell, plus per-row MoveTo overhead — comfortably bigger than
        // the single-cell diff case above.
        assert!(
            buf.len() >= new.cols * new.rows + new.rows * 5,
            "got {} bytes",
            buf.len()
        );
    }

    #[test]
    fn resize_forces_full_redraw_even_with_a_previous_frame() {
        let prev = blank_grid(80, 24);
        let new = blank_grid(40, 12);
        let runs = changed_runs(Some(&prev), &new);
        assert_eq!(runs.len(), new.rows);
        for run in &runs {
            assert_eq!(run.start, 0);
            assert_eq!(run.end, new.cols);
        }
    }

    // --- tiny deterministic PRNG for property-style tests -----------

    struct Xorshift(u32);
    impl Xorshift {
        fn new(seed: u32) -> Self {
            Xorshift(seed | 1)
        }
        fn next_u32(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next_u32() % n
        }
    }

    /// Random grid with occasional wide (double-width) characters, used
    /// to exercise the continuation-cell handling in `changed_runs`.
    fn random_grid_with_wide_chars(cols: usize, rows: usize, rng: &mut Xorshift) -> Grid {
        let mut g = blank_grid(cols, rows);
        for row in 0..rows {
            let mut col = 0;
            while col < cols {
                let style = Style {
                    fg: if rng.below(2) == 0 {
                        Some((rng.below(256) as u8, 0, 0))
                    } else {
                        None
                    },
                    bold: rng.below(2) == 0,
                    ..Style::default()
                };
                if col + 1 < cols && rng.below(4) == 0 {
                    // A wide char, e.g. CJK — occupies two columns.
                    g.lines[row][col] = Cell {
                        ch: '\u{4e2d}',
                        continuation: false,
                        style,
                    };
                    g.lines[row][col + 1] = Cell {
                        ch: ' ',
                        continuation: true,
                        style,
                    };
                    col += 2;
                } else {
                    let ch = char::from_u32(b'a' as u32 + rng.below(5)).unwrap();
                    g.lines[row][col] = Cell {
                        ch,
                        continuation: false,
                        style,
                    };
                    col += 1;
                }
            }
        }
        g
    }

    /// Every cell the diff reports as unchanged must genuinely be
    /// identical, and every cell that genuinely differs must fall inside
    /// exactly one reported run — for a batch of random grid pairs
    /// (fixed seed, so this is deterministic and reproducible).
    #[test]
    #[allow(clippy::needless_range_loop)] // indexes `prev`, `new` and `covered` in lockstep
    fn changed_runs_cover_exactly_the_differing_cells() {
        let mut rng = Xorshift::new(0xC0FFEE);
        for _ in 0..50 {
            let cols = 4 + rng.below(20) as usize;
            let rows = 2 + rng.below(10) as usize;
            let prev = random_grid_with_wide_chars(cols, rows, &mut rng);
            let new = random_grid_with_wide_chars(cols, rows, &mut rng);
            let runs = changed_runs(Some(&prev), &new);

            let mut covered = vec![vec![false; cols]; rows];
            for run in &runs {
                assert!(run.start < run.end, "run must be non-empty: {run:?}");
                for c in run.start..run.end {
                    assert!(!covered[run.row][c], "cell double-covered: {run:?} col {c}");
                    covered[run.row][c] = true;
                }
            }

            for row in 0..rows {
                for col in 0..cols {
                    let p = &prev.lines[row][col];
                    let n = &new.lines[row][col];
                    let differs =
                        p.ch != n.ch || p.continuation != n.continuation || p.style != n.style;
                    assert_eq!(
                        covered[row][col], differs,
                        "row {row} col {col}: differs={differs} but covered={}",
                        covered[row][col]
                    );
                }
            }
        }
    }

    // --- simulated-terminal correctness (diff path == full path) ----

    /// A minimal ANSI interpreter that understands exactly the commands
    /// `diff_draw` emits: CSI cursor-position (`H`), everything else CSI
    /// (colors, attributes, show/hide) is consumed and ignored, and any
    /// other byte is a character written at the current cursor cell.
    /// Restricted to width-1 (ASCII) content by the caller so it doesn't
    /// need to reproduce a real terminal's wide-char column accounting.
    struct FakeTerm {
        cells: Vec<Vec<char>>,
        cols: usize,
        rows: usize,
        cur: (usize, usize),
    }

    impl FakeTerm {
        fn new(cols: usize, rows: usize) -> Self {
            FakeTerm {
                cells: vec![vec![' '; cols]; rows],
                cols,
                rows,
                cur: (0, 0),
            }
        }

        fn apply(&mut self, bytes: &[u8]) {
            let s = std::str::from_utf8(bytes).expect("ascii-only test content");
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\u{1b}' {
                    if chars.peek() == Some(&'[') {
                        chars.next();
                    }
                    let mut param = String::new();
                    let mut final_byte = None;
                    for c2 in chars.by_ref() {
                        if c2.is_ascii_alphabetic() {
                            final_byte = Some(c2);
                            break;
                        }
                        param.push(c2);
                    }
                    if final_byte == Some('H') {
                        let parts: Vec<usize> = param
                            .trim_start_matches('?')
                            .split(';')
                            .map(|p| p.parse().unwrap_or(1))
                            .collect();
                        let row = parts.first().copied().unwrap_or(1).saturating_sub(1);
                        let col = parts.get(1).copied().unwrap_or(1).saturating_sub(1);
                        self.cur = (row, col);
                    }
                    // Any other final byte (colors/attrs/show/hide) doesn't
                    // touch the content grid — nothing to do.
                } else {
                    let (row, col) = self.cur;
                    if row < self.rows && col < self.cols {
                        self.cells[row][col] = c;
                    }
                    self.cur = (row, col + 1);
                }
            }
        }
    }

    fn random_ascii_grid(cols: usize, rows: usize, rng: &mut Xorshift) -> Grid {
        let mut g = blank_grid(cols, rows);
        for row in 0..rows {
            for col in 0..cols {
                let ch = char::from_u32(b'a' as u32 + rng.below(6)).unwrap();
                let style = Style {
                    fg: if rng.below(2) == 0 {
                        Some((rng.below(256) as u8, 10, 20))
                    } else {
                        None
                    },
                    bold: rng.below(3) == 0,
                    ..Style::default()
                };
                g.lines[row][col] = Cell {
                    ch,
                    continuation: false,
                    style,
                };
            }
        }
        g.cursor = (
            rng.below(rows as u32) as usize,
            rng.below(cols as u32) as usize,
        );
        g
    }

    #[test]
    fn diff_path_and_full_path_render_the_same_terminal_content() {
        let (cols, rows) = (12, 6);
        let mut rng = Xorshift::new(12345);
        for _ in 0..20 {
            let prev = random_ascii_grid(cols, rows, &mut rng);
            let new = random_ascii_grid(cols, rows, &mut rng);

            // Path A: a single full redraw straight to `new`.
            let mut full_buf = Vec::new();
            diff_draw(&mut full_buf, None, &new).unwrap();
            let mut term_full = FakeTerm::new(cols, rows);
            term_full.apply(&full_buf);

            // Path B: full redraw to `prev`, then an incremental diff to `new`.
            let mut step1 = Vec::new();
            diff_draw(&mut step1, None, &prev).unwrap();
            let mut term_diff = FakeTerm::new(cols, rows);
            term_diff.apply(&step1);
            let mut step2 = Vec::new();
            diff_draw(&mut step2, Some(&prev), &new).unwrap();
            term_diff.apply(&step2);

            for row in 0..rows {
                for col in 0..cols {
                    let expected = new.lines[row][col].ch;
                    assert_eq!(
                        term_full.cells[row][col], expected,
                        "full path row {row} col {col}"
                    );
                    assert_eq!(
                        term_diff.cells[row][col], expected,
                        "diff path row {row} col {col}"
                    );
                }
            }
        }
    }
}
