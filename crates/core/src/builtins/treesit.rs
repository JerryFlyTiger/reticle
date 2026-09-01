//! elisp-facing `treesit-*` builtins (M12), naming and shape modeled on
//! GNU Emacs 29's `treesit.el`/C primitives. See `crate::treesit` for the
//! Rust-side data model and the reasoning behind its scope cuts.

use std::rc::Rc;

use elisp::builtins::{defun, need_int, need_str, need_sym, opt};
use elisp::error::Flow;
use elisp::{Interp, Value};

use super::{buffer_arg, cur, get_pos};
use crate::treesit::{self, Lang, TsNode};

pub fn register(interp: &mut Interp) {
    defun(
        interp,
        "treesit-language-available-p",
        1,
        Some(1),
        |i, a| {
            let id = need_sym(i, &a[0])?;
            let name = i.sym_name(id).to_string();
            Ok(Value::bool(treesit::language_available(&name), i.syms.t))
        },
    );

    // (treesit-highlight-mode LANG) — enable background syntax
    // highlighting (M15, see crate::highlight) for the current buffer.
    // The parse thread spawns lazily on first use; highlighting arrives
    // asynchronously via the editor idle tick, never on the keystroke.
    defun(interp, "treesit-highlight-mode", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        let name = i.sym_name(id).to_string();
        let lang = Lang::from_name(&name)
            .ok_or_else(|| i.error(format!("treesit: unsupported language `{}`", name)))?;
        let ed = super::ed_handle(i);
        let buf = cur(i);
        let mut editor = ed.borrow_mut();
        editor
            .hl
            .get_or_insert_with(crate::highlight::Engine::new)
            .enable(&buf, lang);
        Ok(Value::Sym(i.syms.t))
    });

    defun(interp, "treesit-parser-create", 1, Some(2), |i, a| {
        let id = need_sym(i, &a[0])?;
        let name = i.sym_name(id).to_string();
        let lang = Lang::from_name(&name)
            .ok_or_else(|| i.error(format!("treesit: unsupported language `{}`", name)))?;
        let buf = buffer_arg(i, &opt(a, 1))?;
        Ok(treesit::make_parser(lang, &buf))
    });

    defun(interp, "treesit-parser-buffer", 1, Some(1), |i, a| {
        let parser = parser_arg(i, &a[0])?;
        let buf = parser
            .buffer
            .upgrade()
            .ok_or_else(|| i.error("treesit: parser's buffer no longer exists"))?;
        Ok(crate::editor::Editor::buffer_value(&buf))
    });

    defun(interp, "treesit-parser-root-node", 1, Some(1), |i, a| {
        let parser = parser_arg(i, &a[0])?;
        let data = treesit::parse(i, &parser)?;
        Ok(treesit::make_node(data, Vec::new()))
    });

    // M39: parse a plain string, no buffer or parser handle involved
    // (verilog-auto.el uses this to look up module definitions in
    // library files it never opens as buffers). LANG first, matching
    // `treesit-parser-create'/`treesit-highlight-mode' above.
    defun(interp, "treesit-parse-string", 2, Some(2), |i, a| {
        let id = need_sym(i, &a[0])?;
        let name = i.sym_name(id).to_string();
        let lang = Lang::from_name(&name)
            .ok_or_else(|| i.error(format!("treesit: unsupported language `{}`", name)))?;
        let text = need_str(i, &a[1])?.to_string();
        let data = treesit::parse_string(i, lang, text)?;
        Ok(treesit::make_node(data, Vec::new()))
    });

    defun(interp, "treesit-node-type", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        Ok(Value::string(node.kind()))
    });

    defun(interp, "treesit-node-start", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        let ch = treesit::byte_to_char(&n.data.text, node.start_byte());
        Ok(Value::Int((ch + 1) as i64))
    });

    defun(interp, "treesit-node-end", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        let ch = treesit::byte_to_char(&n.data.text, node.end_byte());
        Ok(Value::Int((ch + 1) as i64))
    });

    defun(interp, "treesit-node-string", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        Ok(Value::string(node.to_sexp()))
    });

    defun(interp, "treesit-node-text", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        let text = node.utf8_text(n.data.text.as_bytes()).unwrap_or("");
        Ok(Value::string(text))
    });

    defun(interp, "treesit-node-child-count", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        Ok(Value::Int(node.child_count() as i64))
    });

    defun(interp, "treesit-node-child", 2, Some(2), |i, a| {
        let n = node_arg(i, &a[0])?;
        let idx = need_int(i, &a[1])?;
        if idx < 0 {
            return Ok(Value::Nil);
        }
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        if node.child(idx as usize).is_none() {
            return Ok(Value::Nil);
        }
        let mut path = n.path.clone();
        path.push(idx as usize);
        Ok(treesit::make_node(n.data.clone(), path))
    });

    // M39: field-based child access (GNU treesit.el's own name/shape).
    // Needed once query-capture-only field access (M12) stopped being
    // enough -- verilog-auto.el walks the SystemVerilog grammar's
    // `instance_type'/`port_name'/`connection'/`name' fields directly.
    defun(
        interp,
        "treesit-node-child-by-field-name",
        2,
        Some(2),
        |i, a| {
            let n = node_arg(i, &a[0])?;
            let field = need_str(i, &a[1])?.to_string();
            let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
            match node.child_by_field_name(&field) {
                Some(child) => {
                    let path = path_to(child);
                    Ok(treesit::make_node(n.data.clone(), path))
                }
                None => Ok(Value::Nil),
            }
        },
    );

    defun(interp, "treesit-node-parent", 1, Some(1), |i, a| {
        let n = node_arg(i, &a[0])?;
        if n.path.is_empty() {
            return Ok(Value::Nil);
        }
        let mut path = n.path.clone();
        path.pop();
        Ok(treesit::make_node(n.data.clone(), path))
    });

    defun(interp, "treesit-node-eq", 2, Some(2), |i, a| {
        let n1 = node_arg(i, &a[0])?;
        let n2 = node_arg(i, &a[1])?;
        let eq = Rc::ptr_eq(&n1.data, &n2.data) && n1.path == n2.path;
        Ok(Value::bool(eq, i.syms.t))
    });

    // v1 requires PARSER explicitly (real Emacs can infer it from
    // `treesit-parser-list`, which we don't implement -- see PLAN.md M12).
    defun(interp, "treesit-node-at", 2, Some(2), |i, a| {
        let parser = parser_arg(i, &a[1])?;
        let buf = cur(i);
        let char_pos = get_pos(i, &buf.borrow(), &a[0])?;
        let data = treesit::parse(i, &parser)?;
        let byte_pos = treesit::char_to_byte(&data.text, char_pos);
        let root = data.tree.root_node();
        let target = root
            .descendant_for_byte_range(byte_pos, byte_pos)
            .unwrap_or(root);
        let path = path_to(target);
        Ok(treesit::make_node(data, path))
    });

    defun(interp, "treesit-query-capture", 2, Some(2), |i, a| {
        let n = node_arg(i, &a[0])?;
        let query_src = need_str(i, &a[1])?;
        let node = treesit::resolve(&n.data, &n.path).expect("valid node path");
        let query = tree_sitter::Query::new(&node.language(), &query_src)
            .map_err(|e| i.error(format!("treesit-query-capture: {}", e)))?;
        let mut cursor = tree_sitter::QueryCursor::new();
        let text_bytes = n.data.text.as_bytes();
        let mut captures = cursor.captures(&query, node, text_bytes);
        let mut out = Vec::new();
        {
            use streaming_iterator::StreamingIterator;
            while let Some((m, capture_idx)) = captures.next() {
                let cap = m.captures[*capture_idx];
                let name = query.capture_names()[cap.index as usize];
                let path = path_to(cap.node);
                let sym = i.intern(name);
                out.push(Value::cons(
                    Value::Sym(sym),
                    treesit::make_node(n.data.clone(), path),
                ));
            }
        }
        Ok(Value::list(out))
    });
}

fn parser_arg(interp: &mut Interp, v: &Value) -> Result<Rc<crate::treesit::TsParserState>, Flow> {
    treesit::as_parser(v).ok_or_else(|| interp.wrong_type("treesit-parser-p", v))
}

fn node_arg(interp: &mut Interp, v: &Value) -> Result<Rc<TsNode>, Flow> {
    treesit::as_node(v).ok_or_else(|| interp.wrong_type("treesit-node-p", v))
}

/// Reconstruct the child-index path from root to `target`. Used wherever
/// tree-sitter hands us a borrowed `Node` (query captures,
/// `descendant_for_byte_range`) that we need to store as a long-lived
/// path-based handle instead -- see `crate::treesit::TsNode`.
fn path_to(target: tree_sitter::Node) -> Vec<usize> {
    let mut path = Vec::new();
    let mut node = target;
    while let Some(parent) = node.parent() {
        let idx = (0..parent.child_count())
            .find(|&i| parent.child(i).map(|c| c.id()) == Some(node.id()))
            .expect("node must be a child of its own parent");
        path.push(idx);
        node = parent;
    }
    path.reverse();
    path
}
