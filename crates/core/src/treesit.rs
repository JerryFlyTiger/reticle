//! tree-sitter integration (M12). Our editor core is already Rust, so
//! unlike GNU Emacs (which needs the emacs-module.h FFI boundary to reach
//! a C library from elisp) we just link tree-sitter as an ordinary crate
//! dependency: zero marshalling cost between the parser and the buffer.
//!
//! v1 deliberately bundled exactly one grammar (Rust) -- adding another
//! is one more `Lang` variant plus a Cargo dependency, not an
//! architectural change, so the scope cut cost nothing later. M25 cashes
//! that in: seven more grammars (C, C++, Python, Bash, Java, Perl,
//! Elisp), each just another match arm here, plus a bump from
//! tree-sitter 0.24 to 0.25 (0.24 only accepts grammars up to ABI 14;
//! the c/python/bash/perl/elisp crates that were current at M25 ship
//! ABI 15, and 0.25 still reads ABI 13-14 too, so this is a pure
//! widening with no observed regression -- see PLAN.md). M38 adds a
//! ninth grammar the same way (Verilog/SystemVerilog, via
//! tree-sitter-systemverilog 0.4 -- ABI 15, already inside the 0.25
//! window opened at M25, so no further widening needed).
//!
//! v1 deliberately reparsed the whole buffer from scratch (rather than
//! using tree-sitter's `Tree::edit`-based incremental reuse) every time
//! the text had actually changed. The concern at M12 was exact
//! byte-accounted edits threaded through every mutation path (insert,
//! delete, *and* undo/redo, which bypasses the normal
//! `Buffer::insert`/`delete` entry points -- see buffer.rs's
//! `undo_step_from`): getting that subtly wrong doesn't just cost speed,
//! it can hand tree-sitter's reuse algorithm a tree that silently
//! disagrees with the text, corrupting node ranges. M108 added a much
//! narrower, much safer cache on top: `parse` below skips even the
//! from-scratch reparse when nothing has changed since the last call for
//! the same buffer and language (see `Buffer::ts_tree`'s doc) -- before
//! that, every call reparsed unconditionally even when called
//! back-to-back on an unmodified buffer, which made every keystroke's
//! indent/highlight pass O(buffer size) regardless of whether that
//! keystroke touched the buffer being parsed.
//!
//! M109 finally does real incremental reuse, but not the way M12's
//! reasoning warned against. The rejected design (call sites reporting
//! an `InputEdit` themselves) is exactly the "silently disagrees with
//! the text" failure mode: an experiment that fed tree-sitter a
//! deliberately wrong `InputEdit` (claiming a 40-byte deletion that
//! hadn't happened) produced a tree with the same node count, same node
//! kinds, same total length, and `has_error() == false` -- but with
//! every node from roughly the 72,000th one onward at the wrong byte
//! range. None of the cheap sanity checks (`has_error`, node count,
//! total length) can see that. Myers diffing (`textdiff`) was also
//! rejected: it reports character offsets, not bytes or `Point`s, so
//! recovering an `InputEdit` from it needs another linear scan, and its
//! own cost has a cliff -- an edit scattered across 200 lines pushed the
//! diff itself past the parse it was trying to avoid.
//!
//! `derive_edit` below sidesteps M12's exact worry by construction: it
//! diffs the *previous* full buffer text (which `TsTreeData::text`
//! already retains) against the *current* one to find the common prefix
//! and common suffix, and builds the one `InputEdit` consistent with
//! `new == old[..start] + new[start..new_end] + old[old_end..]`. It is
//! structurally impossible for this to disagree with the text the way a
//! hand-reported edit can, because it's derived from comparing the two
//! texts directly rather than from trusting a caller's bookkeeping. That
//! also means it doesn't need to hook `Buffer::insert`/`delete`/
//! `undo_step_from` at all -- it only ever looks at text before and
//! after, so undo, redo, `replace-region-contents`, and `erase-buffer`
//! are all covered automatically, with no separate code path for any of
//! them.
//!
//! Measured (controlled A/B against a `git worktree` checkout of the
//! M108 commit as the baseline, both binaries confirmed distinct before
//! timing, three alternating rounds, variance under 1%, this machine,
//! release profile, per keystroke, mean of 20):
//!
//! | workload | edit pattern | M108 | M109 |
//! |---|---|---|---|
//! | 570 small modules, 7980 lines, 190840 B | 1-char edits scattered through real code | 21.420 ms | 0.425 ms |
//! | same | TAB producing byte-identical text | 21.005 ms | 0.076 ms |
//! | one 8000-arm case, 8011 lines | scattered 1-char edits | 38.527 ms | 24.043 ms |
//!
//! All three rows are printed by `measure_incremental_vs_full_on_two_buffer_shapes`
//! in `crates/core/tests/indent_perf_tests.rs`, so they can be re-run or
//! refuted; the M108 column comes from running that same file inside the
//! worktree.
//!
//! Two things this table must be read with, not just next to. First, the
//! realistic many-small-modules shape gains fifty times, but the
//! adversarial one-huge-construct shape gains only 1.6x -- one enormous
//! node leaves tree-sitter almost nothing to reuse, and a generated
//! netlist will look much more like the second row than the first.
//! Second, an earlier version of this table also quoted a
//! "RET repeatedly at one point" pattern at 73x. That pattern flatters
//! the result and was dropped: after the first press the edit lands
//! inside a run of newlines between two top-level modules, which
//! tree-sitter reuses wholesale. Scattered edits through real code are
//! what typing actually does.
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use elisp::error::Flow;
use elisp::value::ExtRef;
use elisp::{Interp, Value};

use crate::buffer::Buffer;

pub const PARSER_TAG: &str = "treesit-parser";
pub const NODE_TAG: &str = "treesit-node";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Rust,
    C,
    Cpp,
    Python,
    Bash,
    Java,
    Perl,
    Elisp,
    Verilog,
}

impl Lang {
    pub fn from_name(name: &str) -> Option<Lang> {
        match name {
            "rust" => Some(Lang::Rust),
            "c" => Some(Lang::C),
            "cpp" => Some(Lang::Cpp),
            "python" => Some(Lang::Python),
            "bash" => Some(Lang::Bash),
            "java" => Some(Lang::Java),
            "perl" => Some(Lang::Perl),
            "elisp" => Some(Lang::Elisp),
            "verilog" => Some(Lang::Verilog),
            _ => None,
        }
    }

    fn ts_language(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::C => tree_sitter_c::LANGUAGE.into(),
            Lang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::Bash => tree_sitter_bash::LANGUAGE.into(),
            Lang::Java => tree_sitter_java::LANGUAGE.into(),
            Lang::Perl => ts_parser_perl::LANGUAGE.into(),
            Lang::Elisp => tree_sitter_elisp::LANGUAGE.into(),
            Lang::Verilog => tree_sitter_systemverilog::LANGUAGE.into(),
        }
    }
}

pub fn language_available(name: &str) -> bool {
    Lang::from_name(name).is_some()
}

pub struct TsParserState {
    pub lang: Lang,
    pub buffer: Weak<RefCell<Buffer>>,
}

/// A parsed tree plus the exact source text it was parsed from. A
/// `tree_sitter::Node` borrows from its `Tree`, and node text extraction
/// needs the original source again, so both must outlive every `TsNode`
/// built from them -- bundling them in one `Rc` is what keeps that true.
pub struct TsTreeData {
    pub tree: tree_sitter::Tree,
    /// `Rc<str>` (M108, was `String`) so the buffer-backed `parse` path
    /// can share the exact same allocation `Buffer::search_text` already
    /// produces (see `Buffer::ts_tree`'s doc) instead of paying a second
    /// O(N) copy on top of it. `parse_string` (no buffer involved) just
    /// wraps its owned `String` in an `Rc` once; every existing reader
    /// of this field only ever calls `&str`-compatible methods
    /// (`.as_bytes()`, indexing via `byte_to_char`/`char_to_byte`,
    /// `Node::utf8_text`), all of which work identically through
    /// `Rc<str>`'s `Deref<Target = str>`, so this is not a breaking
    /// change for any caller.
    pub text: Rc<str>,
}

/// A node handle. We deliberately don't store a `tree_sitter::Node<'tree>`
/// directly -- escaping its borrow of `data.tree` would need an unsafe
/// lifetime transmute (a well-known pattern in the tree-sitter ecosystem,
/// but still unsafe). Storing the path of child indices from the root and
/// re-walking it on every access is 100% safe Rust, costs O(depth) --
/// cheap, depth is never more than a few dozen -- and needs no unsafe at
/// all, matching this codebase's "narrow, safety-first" bias elsewhere
/// (JIT eligibility, GC registration gating).
pub struct TsNode {
    pub data: Rc<TsTreeData>,
    pub path: Vec<usize>,
}

pub fn make_parser(lang: Lang, buffer: &Rc<RefCell<Buffer>>) -> Value {
    Value::Ext(ExtRef::new(
        PARSER_TAG,
        TsParserState {
            lang,
            buffer: Rc::downgrade(buffer),
        },
        // TsParserState only holds a Weak<RefCell<Buffer>> — a weak
        // reference must not keep the buffer alive, so it's deliberately
        // not traced (the buffer's own root path, if any, is what marks it).
        None,
    ))
}

pub fn as_parser(v: &Value) -> Option<Rc<TsParserState>> {
    v.as_ext::<TsParserState>(PARSER_TAG)
}

pub fn make_node(data: Rc<TsTreeData>, path: Vec<usize>) -> Value {
    // TsNode holds parsed-tree/source data, no Value.
    Value::Ext(ExtRef::new(NODE_TAG, TsNode { data, path }, None))
}

pub fn as_node(v: &Value) -> Option<Rc<TsNode>> {
    v.as_ext::<TsNode>(NODE_TAG)
}

/// Walk `path` from the tree's root. Only `None` if `path` doesn't fit
/// `data`'s tree shape, which the public builtins never produce (a path
/// is always derived from, and resolved against, the same `data`).
pub fn resolve<'a>(data: &'a TsTreeData, path: &[usize]) -> Option<tree_sitter::Node<'a>> {
    let mut node = data.tree.root_node();
    for &i in path {
        node = node.child(i)?;
    }
    Some(node)
}

pub fn byte_to_char(text: &str, byte_pos: usize) -> usize {
    match text.get(..byte_pos) {
        Some(s) => s.chars().count(),
        None => text.chars().count(),
    }
}

pub fn char_to_byte(text: &str, char_pos: usize) -> usize {
    text.char_indices()
        .nth(char_pos)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

/// Return `parser`'s buffer's current tree, reparsing only when the
/// buffer has actually changed since the last call for this exact
/// `(buffer, language)` pair -- see `Buffer::ts_tree`'s doc for why
/// keying the O(1) early-exit on `edit_ticks` is safe. Within one
/// unchanged generation, repeated calls (e.g. `indent-for-tab-command`
/// and syntax highlighting both asking for a fresh root node on every
/// keystroke, even when the keystroke didn't touch this buffer) are
/// O(1): an `Rc` clone. When the generation *has* moved but the language
/// matches the cached entry, this reparses incrementally off the old
/// tree via `derive_edit` (see the module doc); only a language switch
/// or a first-ever parse falls all the way through to `parse_text`.
pub fn parse(interp: &mut Interp, parser: &TsParserState) -> Result<Rc<TsTreeData>, Flow> {
    let buf = parser
        .buffer
        .upgrade()
        .ok_or_else(|| interp.error("treesit: parser's buffer no longer exists"))?;
    let bb = buf.borrow();
    let gen = bb.edit_ticks;
    let cached = bb
        .ts_tree
        .borrow()
        .as_ref()
        .map(|(g, l, d)| (*g, *l, Rc::clone(d)));
    if let Some((cached_gen, cached_lang, data)) = &cached {
        if *cached_gen == gen && *cached_lang == parser.lang {
            return Ok(Rc::clone(data));
        }
    }
    // Reuse the same full-buffer-text snapshot `search_text` already
    // maintains (keyed on the same `edit_ticks` generation) instead of a
    // second independent `to_string()` materialization of the gap buffer.
    let text = bb.search_text();
    drop(bb);
    let data = match &cached {
        Some((_, cached_lang, old_data)) if *cached_lang == parser.lang => {
            incremental_parse(interp, parser.lang, old_data, text)?
        }
        _ => parse_text(interp, parser.lang, text)?,
    };
    buf.borrow()
        .ts_tree
        .replace(Some((gen, parser.lang, Rc::clone(&data))));
    Ok(data)
}

/// Reparse `new_text` incrementally off `old_data`'s tree, using
/// `derive_edit` to recover the one `InputEdit` consistent with the two
/// texts (see the module doc for why this is safe where a hand-reported
/// edit wouldn't be). `old_data.tree` is cloned (refcount-cheap) before
/// `edit`, never mutated in place, so any `TsNode` handles still
/// pointing at `old_data` through their own `Rc` keep seeing a
/// self-consistent tree+text pair.
fn incremental_parse(
    interp: &mut Interp,
    lang: Lang,
    old_data: &Rc<TsTreeData>,
    new_text: Rc<str>,
) -> Result<Rc<TsTreeData>, Flow> {
    let edit = match derive_edit(&old_data.text, &new_text) {
        // Generation moved but the text is byte-for-byte identical (e.g.
        // an undo/redo or replace-region-contents that round-tripped to
        // the same content) -- share the existing Rc rather than parsing
        // again.
        None => return Ok(Rc::clone(old_data)),
        Some(edit) => edit,
    };
    let mut tree = old_data.tree.clone();
    tree.edit(&edit);
    let mut p = tree_sitter::Parser::new();
    p.set_language(&lang.ts_language())
        .map_err(|e| interp.error(format!("treesit: {}", e)))?;
    let new_tree = p
        .parse(new_text.as_bytes(), Some(&tree))
        .ok_or_else(|| interp.error("treesit: parse was cancelled"))?;
    // Cheap sanity check: a tree that no longer spans the whole new text
    // cannot be right. This only catches a length mismatch, not the
    // range-corruption failure mode the module doc describes (that one
    // reproduced with matching length, node count, and `has_error() ==
    // false` -- it is silent to every check available here) -- the real
    // defense against that is `derive_edit`'s construction, not this
    // check. This exists as a last-ditch fallback in case tree-sitter's
    // own incremental reuse ever hits a case it can't handle.
    //
    // Honesty note: inverting this comparison is not observable by any
    // test in this workspace. Recorded as mutation U6 in
    // `dev/mutations/m109.py`, it SURVIVES the whole suite -- not from an
    // oversight a better test could close, but because falling through to
    // `parse_text` on a length match still produces a correct tree; the
    // only thing an inverted check changes is which path (fast vs.
    // fallback) gets taken on already-correct output. There is no known
    // way to make this line's correctness observable from outside without
    // instrumenting which branch ran.
    if new_tree.root_node().end_byte() != new_text.len() {
        return parse_text(interp, lang, new_text);
    }
    Ok(Rc::new(TsTreeData {
        tree: new_tree,
        text: new_text,
    }))
}

/// Compute the one `InputEdit` consistent with `new == old[..start] +
/// new[start..new_end] + old[old_end..]`, by finding the common prefix
/// and common suffix of `old` and `new`. Returns `None` if the two texts
/// are identical (no edit to report). Boundaries are pulled back to the
/// nearest `char` boundary in *both* strings (a multi-byte character
/// straddling the raw byte-diff point would otherwise slice through it),
/// which is why this needs `str` inputs rather than raw bytes.
///
/// `pub` (rather than private to this module) purely so
/// `treesit_tests.rs` can property-test it directly against the
/// `new == old[..start] + new[start..new_end] + old[old_end..]`
/// invariant, the same way `byte_to_char`/`char_to_byte` above are
/// already exposed for their own callers.
///
/// **This is O(buffer length), not O(edit size)** -- despite the name
/// "incremental," the common-prefix and common-suffix scans each walk
/// from an end of the buffer toward the edit, so together they cover the
/// whole buffer regardless of how small the edit is or where it lands.
/// The M109 speedup this feeds into is real and measured (see the module
/// doc's table), and it comes from two things that have nothing to do
/// with the diff being cheap by size: a raw byte scan is orders of
/// magnitude cheaper per byte than grammar-driven parsing, and the
/// `InputEdit` it produces lets tree-sitter reuse subtrees instead of
/// rebuilding them. "Incremental" describes the *parse*, not this scan --
/// don't read it as promising the diff cost scales with edit size. The
/// practical consequence: on a large enough file, this byte scan itself
/// becomes the floor on how fast a single-keystroke reparse can be (see
/// the 8000-arm-case rows of the table, where the gain shrinks to 1.6x).
pub fn derive_edit(old: &str, new: &str) -> Option<tree_sitter::InputEdit> {
    if old == new {
        return None;
    }
    let old_b = old.as_bytes();
    let new_b = new.as_bytes();

    let max_common = old_b.len().min(new_b.len());
    let mut prefix = 0;
    while prefix < max_common && old_b[prefix] == new_b[prefix] {
        prefix += 1;
    }
    // Bytes 0..prefix are identical in both strings, so shrinking prefix
    // preserves that; pull back until `prefix` lands on a char boundary
    // in both (they can differ, since the byte *at* `prefix` is where
    // they first diverge, or where the shorter one ends).
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }

    let old_rem = old_b.len() - prefix;
    let new_rem = new_b.len() - prefix;
    let max_suffix = old_rem.min(new_rem);
    let mut suffix = 0;
    while suffix < max_suffix && old_b[old_b.len() - 1 - suffix] == new_b[new_b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!old.is_char_boundary(old_b.len() - suffix)
            || !new.is_char_boundary(new_b.len() - suffix))
    {
        suffix -= 1;
    }

    let start_byte = prefix;
    let old_end_byte = old_b.len() - suffix;
    let new_end_byte = new_b.len() - suffix;

    Some(tree_sitter::InputEdit {
        start_byte,
        old_end_byte,
        new_end_byte,
        start_position: point_at(old, start_byte),
        old_end_position: point_at(old, old_end_byte),
        new_end_position: point_at(new, new_end_byte),
    })
}

/// tree-sitter's `Point` (row/column) bookkeeping. Nothing in this
/// codebase actually reads `start_point`/`end_point` off a node (`rg`
/// confirms zero hits under `crates/`) -- tree-sitter just wants these
/// for its own internal accounting -- but they still have to be computed
/// correctly rather than filled in with placeholders, since an
/// incremental-reuse bug in tree-sitter's own row/column tracking isn't
/// this codebase's to rely on being forgiving.
fn point_at(text: &str, byte: usize) -> tree_sitter::Point {
    let up_to = &text.as_bytes()[..byte];
    let row = up_to.iter().filter(|&&b| b == b'\n').count();
    let column = match up_to.iter().rposition(|&b| b == b'\n') {
        Some(idx) => byte - idx - 1,
        None => byte,
    };
    tree_sitter::Point { row, column }
}

/// Parse `text` fresh under `lang`, with no buffer or parser handle
/// involved at all (M39: `treesit-parse-string`, used to look up module
/// definitions in library files that are never opened as buffers -- see
/// verilog-auto.el's header for why a one-shot string parse was chosen
/// over the alternative of stuffing the text into a hidden temp buffer
/// just to reuse `parse` above). A "parser" object wouldn't mean
/// anything here anyway -- there's nothing to ever re-parse -- so this
/// returns the tree data directly rather than something shaped like
/// `TsParserState`.
pub fn parse_string(interp: &mut Interp, lang: Lang, text: String) -> Result<Rc<TsTreeData>, Flow> {
    parse_text(interp, lang, Rc::from(text))
}

fn parse_text(interp: &mut Interp, lang: Lang, text: Rc<str>) -> Result<Rc<TsTreeData>, Flow> {
    let mut p = tree_sitter::Parser::new();
    p.set_language(&lang.ts_language())
        .map_err(|e| interp.error(format!("treesit: {}", e)))?;
    let tree = p
        .parse(text.as_bytes(), None)
        .ok_or_else(|| interp.error("treesit: parse was cancelled"))?;
    Ok(Rc::new(TsTreeData { tree, text }))
}

/// `point_at` is a pure `byte -> Point` conversion, so per this project's
/// testing conventions it's covered directly here rather than through an
/// integration test.
///
/// Why this matters even though it looks like bookkeeping: a cold-read
/// review of tree-sitter 0.25.10's `ts_subtree_edit` (`subtree.c`)
/// established that `Point` only feeds tree-sitter's internal decisions
/// for grammars with column-dependent tokens (Python-style indentation
/// sensitivity). None of the six grammars this project currently drives
/// through this path -- C, C++, Java, Rust, Elisp and Verilog, the ones
/// whose modes reach `treesit-node-at` from `indent.el`; Python, Bash
/// and Perl indent by textual heuristic and never create a parser --
/// is such a grammar, which means
/// **every existing differential and fuzz test in this workspace would
/// stay green even if `point_at` swapped row and column, or was off by
/// one at every newline.** The property is real -- it becomes
/// load-bearing the day a column-sensitive grammar is added -- but
/// nothing else in this project can see it break. Hence a direct unit
/// test of the function itself.
#[cfg(test)]
mod tests {
    use super::point_at;
    use tree_sitter::Point;

    #[test]
    fn offset_zero() {
        assert_eq!(point_at("hello\nworld", 0), Point { row: 0, column: 0 });
    }

    #[test]
    fn mid_first_line() {
        assert_eq!(point_at("hello\nworld", 3), Point { row: 0, column: 3 });
    }

    #[test]
    fn exactly_on_newline() {
        // Byte 5 is the '\n' itself: still counted as being on row 0 (the
        // newline hasn't been "crossed" yet at this offset).
        assert_eq!(point_at("hello\nworld", 5), Point { row: 0, column: 5 });
    }

    #[test]
    fn immediately_after_newline() {
        // Byte 6 is the 'w' of "world": start of row 1, column 0.
        assert_eq!(point_at("hello\nworld", 6), Point { row: 1, column: 0 });
    }

    #[test]
    fn later_row() {
        assert_eq!(point_at("aa\nbb\ncc\ndd", 9), Point { row: 3, column: 0 });
    }

    #[test]
    fn very_end_of_text() {
        let text = "hello\nworld";
        assert_eq!(point_at(text, text.len()), Point { row: 1, column: 5 });
    }

    #[test]
    fn no_trailing_newline() {
        let text = "abc";
        assert_eq!(point_at(text, text.len()), Point { row: 0, column: 3 });
    }

    #[test]
    fn only_newlines() {
        let text = "\n\n\n";
        assert_eq!(point_at(text, 0), Point { row: 0, column: 0 });
        assert_eq!(point_at(text, 1), Point { row: 1, column: 0 });
        assert_eq!(point_at(text, 2), Point { row: 2, column: 0 });
        assert_eq!(point_at(text, 3), Point { row: 3, column: 0 });
    }

    #[test]
    fn multi_byte_character() {
        // 'é' is 2 bytes in UTF-8. The code walks raw bytes
        // (`text.as_bytes()`), so the column it computes is a *byte*
        // offset within the row, not a character offset -- reading it,
        // the loop bodies index into `as_bytes()` directly with no char
        // decoding at all. Asserting the byte-offset value here, per
        // this task's instruction to test what the code actually
        // computes rather than what one might assume it should compute.
        //
        // This choice looks questionable for a column-sensitive grammar
        // (tree-sitter's own `Point.column` convention is a byte offset,
        // so it happens to line up -- but if a future caller ever treats
        // this `column` as a character count, e.g. for cursor placement,
        // it would be wrong on any line with multi-byte characters before
        // the given offset).
        let text = "é\nworld"; // 'é' = 2 bytes, then '\n' at byte 2.
                               // byte 0..2 is 'é', byte 2 is '\n', byte 3 is 'w'.
        assert_eq!(point_at(text, 3), Point { row: 1, column: 0 });

        let text2 = "aéb\ncd"; // 'a'=1, 'é'=2, 'b'=1 -> row 0 has 4 bytes.
                               // byte offset 4 is the '\n'; column should be the byte offset
                               // within the row (4), not the character offset (3).
        assert_eq!(point_at(text2, 4), Point { row: 0, column: 4 });
    }
}
