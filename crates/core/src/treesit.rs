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
//! v1 also deliberately reparses the whole buffer from scratch every time
//! a fresh root node is requested, rather than using tree-sitter's
//! `Tree::edit`-based incremental reuse. Real incremental reuse needs
//! exact byte-accounted edits threaded through every mutation path
//! (insert, delete, *and* undo/redo, which bypasses the normal
//! `Buffer::insert`/`delete` entry points -- see buffer.rs's
//! `undo_step_from`). Getting that subtly wrong doesn't just cost speed,
//! it can hand tree-sitter's reuse algorithm a tree that silently
//! disagrees with the text, corrupting node ranges. A from-scratch parse
//! of realistic file sizes is already low-millisecond (measured; see
//! PLAN.md), so this is a correctness-first scope cut, not a missing
//! optimization users will feel.
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
    pub text: String,
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

/// Parse `parser`'s buffer's current full text fresh. See the module doc
/// for why this is a full reparse rather than an incremental one.
pub fn parse(interp: &mut Interp, parser: &TsParserState) -> Result<Rc<TsTreeData>, Flow> {
    let buf = parser
        .buffer
        .upgrade()
        .ok_or_else(|| interp.error("treesit: parser's buffer no longer exists"))?;
    let text = buf.borrow().text.to_string();
    parse_text(interp, parser.lang, text)
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
    parse_text(interp, lang, text)
}

fn parse_text(interp: &mut Interp, lang: Lang, text: String) -> Result<Rc<TsTreeData>, Flow> {
    let mut p = tree_sitter::Parser::new();
    p.set_language(&lang.ts_language())
        .map_err(|e| interp.error(format!("treesit: {}", e)))?;
    let tree = p
        .parse(&text, None)
        .ok_or_else(|| interp.error("treesit: parse was cancelled"))?;
    Ok(Rc::new(TsTreeData { tree, text }))
}
