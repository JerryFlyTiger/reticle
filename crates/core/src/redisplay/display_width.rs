//! One place that states how a single character expands into display
//! cells -- before this module existed, that rule was written out five
//! times (`redisplay.rs`'s `char_width`, `ml_width`, the inline copy in
//! `draw_ml_segment`, the inline copy in `echo_cells`, and the
//! `string-width` builtin in `builtins/ui.rs`), with overlapping but
//! *not* identical case coverage. See each call site below for why the
//! coverage differs; the differences are load-bearing, not accidental,
//! so this module states them explicitly instead of quietly picking one.

use unicode_width::UnicodeWidthChar;

/// How a single character `c` expands into display cells.
pub enum Expansion {
    /// `\t`: the only case whose width depends on the column it starts
    /// at (`8 - col % 8`), because a tab pads out to the next 8-column
    /// stop rather than having a fixed width of its own.
    Tab { width: usize },
    /// A C0 control character (`< 0x20`) or DEL (`0x7f`): rendered as
    /// `^X` and therefore occupies 2 cells.
    Control(char),
    /// A double-width character (CJK and friends, per
    /// `UnicodeWidthChar`): occupies 2 cells.
    Wide(char),
    /// Everything else: one cell.
    Plain(char),
}

impl Expansion {
    /// Classify `c` as it would be drawn starting at column `col`.
    /// `col` only matters for `Tab`; callers working on text that's
    /// already known not to contain a tab (e.g. `ml_sanitize`'s output)
    /// can pass any value, since `Tab` will never be produced for them.
    pub fn classify(c: char, col: usize) -> Expansion {
        match c {
            '\t' => Expansion::Tab {
                width: 8 - (col % 8),
            },
            c if (c as u32) < 32 => Expansion::Control(c),
            '\u{7f}' => Expansion::Control(c),
            c => {
                if wide_char_width(c) == 2 {
                    Expansion::Wide(c)
                } else {
                    Expansion::Plain(c)
                }
            }
        }
    }

    /// The number of display cells this expansion occupies.
    pub fn width(&self) -> usize {
        match self {
            Expansion::Tab { width } => *width,
            Expansion::Control(_) => 2,
            Expansion::Wide(_) => 2,
            Expansion::Plain(_) => 1,
        }
    }
}

/// The full five-way rule, tab- and control-aware: matches what the
/// buffer-drawing loop (`render_window`) and the wrap scanner
/// (`next_row_start`) both actually put on screen. `col` is needed
/// because a tab's width depends on it.
///
/// This is `redisplay.rs`'s former `char_width`, unchanged in every
/// answer -- only the case analysis now lives in `Expansion::classify`.
pub fn char_width(c: char, col: usize) -> usize {
    Expansion::classify(c, col).width()
}

/// Wide-char-only width. Column-independent, since there is no `Tab`
/// case to need a column.
///
/// Two kinds of caller, and they are not the same kind, so do not read
/// this as one rule:
///
/// - Mode-line segments, which have been through `ml_sanitize` — tabs
///   and control characters are physically gone by the time this runs,
///   so the missing cases genuinely cannot arise.
/// - `string_width_elisp` (below), whose input is a raw elisp string
///   with the tab character still in it. Its 1-column answer for a tab
///   is not because the tab was removed; it is because
///   `UnicodeWidthChar::width('\t')` returns `None` (tab is in the
///   control category) and `.unwrap_or(1)` defaults it to 1. Same
///   number, entirely different reason.
pub fn wide_char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(1).max(1)
}

/// The width policy behind the `string-width` elisp builtin
/// (`builtins/ui.rs`).
///
/// **Deliberately not the same answer as `char_width`.** GNU Emacs's
/// own `string-width` *does* account for tabs and control characters,
/// but reticle's `string-width` never has: `(string-width "a\tb")`
/// returns 3 today (one column per character, via `wide_char_width`),
/// while the grid this same text would occupy in a buffer is 9 columns
/// (`char_width`'s tab rule: a tab at column 1 pads to column 8).
///
/// Making `string-width` match `char_width` would arguably be *more*
/// correct against GNU Emacs, but doing so here would be an
/// elisp-visible behaviour change smuggled into a refactor whose whole
/// point is that nothing user-visible changes — the golden-master test
/// this refactor is held to can't see it, because it never calls
/// `string-width`. So this function preserves reticle's current
/// (narrower) answer exactly, under its own name, so a reader sees the
/// divergence from `char_width` is deliberate rather than an oversight.
/// Reconciling the two, if it's ever worth doing, is a separate,
/// tracked decision — not something to slip in here.
pub fn string_width_elisp(s: &str) -> usize {
    s.chars().map(wide_char_width).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_width_is_column_dependent() {
        assert_eq!(char_width('\t', 0), 8);
        assert_eq!(char_width('\t', 1), 7);
        assert_eq!(char_width('\t', 7), 1);
        assert_eq!(char_width('\t', 8), 8);
    }

    #[test]
    fn control_and_del_are_two_cells() {
        assert_eq!(char_width('\r', 0), 2);
        assert_eq!(char_width('\u{7f}', 0), 2);
    }

    #[test]
    fn wide_char_is_two_cells_either_way() {
        assert_eq!(char_width('中', 0), 2);
        assert_eq!(wide_char_width('中'), 2);
    }

    #[test]
    fn plain_ascii_is_one_cell() {
        assert_eq!(char_width('a', 0), 1);
        assert_eq!(wide_char_width('a'), 1);
    }

    /// The documented divergence: `string-width` ignores tab expansion
    /// entirely, unlike `char_width`.
    #[test]
    fn string_width_does_not_expand_tabs() {
        assert_eq!(string_width_elisp("a\tb"), 3);
        assert_eq!(char_width('\t', 1), 7); // same tab, buffer-drawing answer
    }
}
