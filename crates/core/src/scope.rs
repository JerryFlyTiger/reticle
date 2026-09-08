//! M118: "which constructs enclose this position" — the shared data
//! feeding both the sticky scope header (`redisplay.rs`'s F1) and the
//! mode-line scope breadcrumb (`redisplay.rs`'s F2, `compose_mode_line`).
//! `highlight::Engine::scope_chain` computes and caches this once per
//! completed background parse (mirroring M113's `Engine::matching_pair`
//! -- see that method's doc for the generation-guard reasoning this
//! shares).
//!
//! **Every node kind name below is dump-verified, not remembered.**
//! Verilog was confirmed two ways: (1) `node-types.json` shipped inside
//! the vendored `tree-sitter-systemverilog` 0.4.0 crate source (queried
//! directly, not from memory) for the field each label is pulled from,
//! and (2) `crates/core/lisp/indent.el`'s own `indent--block-node-types`
//! list and header comment, which is itself dump-verified production
//! code (M38/M97) using the same grammar. Rust/Python/C/C++/Java were
//! each confirmed the same way, against their own vendored crate's
//! `node-types.json`. Anything not confirmed by one of those two routes
//! is left out of the tables below, not guessed at.
//!
//! **Bash and Elisp are deliberately empty in v1** -- no dump was done
//! for either grammar, and this project's stated priority is Verilog
//! (see `CLAUDE.md`); adding these tables is straightforward follow-up
//! work for whoever wants breadcrumbs in a shell script or an init.el.
//! Perl is empty for the same reason (not requested, not dumped).
//!
//! **Char offsets, not tree-sitter's `Point`**: like every other place
//! in this codebase (`treesit.rs:419-434`'s module doc explains why),
//! `Scope` never carries a tree-sitter `Point` -- `start_line` is
//! computed directly from the text snapshot by counting newlines, the
//! same way `GapBuffer::line_number` does conceptually (just without
//! its incremental-anchor optimization, since this runs once per parse
//! on a background thread, not once per frame).

use crate::treesit::Lang;

/// One construct enclosing a buffer position. See this module's doc for
/// how `kind`/`label` are derived and verified per language.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    /// 0-based buffer line on which this construct starts.
    pub start_line: usize,
    /// Char offsets of the node, used for the enclosure test
    /// (`start <= pos && pos < end`) in `Engine::scope_chain`.
    pub start: usize,
    pub end: usize,
    /// tree-sitter node kind, e.g. "always_construct".
    pub kind: String,
    /// Compact human label for the breadcrumb/header, e.g. "always_ff".
    pub label: String,
}

/// Whether a node of `kind` counts as a scope for `lang`. See this
/// module's doc for how each kind was verified.
pub fn is_scope_kind(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Verilog => matches!(
            kind,
            "module_declaration"
                | "always_construct"
                | "case_statement"
                | "if_generate_construct"
                | "generate_block"
                | "module_instantiation"
                | "function_declaration"
                | "task_declaration"
                | "package_declaration"
                | "interface_declaration"
                | "class_declaration"
                | "loop_generate_construct"
                // M121: `program`, `covergroup`, `modport`, a class
                // constructor, and the concurrent-assertion wrapper all
                // previously yielded zero breadcrumb -- dump-verified
                // against `scope.rs`'s own probe (see this module's doc
                // comment). `node-types.json` lists FIVE statement kinds
                // that can appear inside `concurrent_assertion_item`
                // (`assert_property_statement`/`cover_property_statement`/
                // `assume_property_statement`/`cover_sequence_statement`/
                // `restrict_property_statement`), not three -- see the
                // `verilog_label` match arm below for the dump that found
                // this.
                | "program_declaration"
                | "covergroup_declaration"
                | "modport_declaration"
                | "class_constructor_declaration"
                | "concurrent_assertion_item"
                // M124: `extern`-declared class members (`extern function
                // new(...);`, `extern task run(...);`, `pure virtual
                // function void step();`) parse as three distinct
                // "prototype" node kinds -- `class_constructor_prototype',
                // `task_prototype', `function_prototype' -- none of which
                // were scope kinds before this, so `M-.'/breadcrumb/
                // `collect_scopes' returned only the enclosing
                // `class_declaration' for any of them. Dump-verified
                // (M124: `class foo;\n  extern function new(int y);\n
                // extern task run(int y);\n  pure virtual function void
                // step();\nendclass\n', `(treesit-node-string ...)'
                // output): `class_constructor_prototype' has no `name:'
                // field of its own (same shape as `class_constructor_
                // declaration' -- SV only ever calls it `new'), but
                // `task_prototype'/`function_prototype' both DO carry
                // `name:' directly on themselves, unlike `task_
                // declaration'/`function_declaration' (whose `name:'
                // lives on a nested `*_body_declaration' child instead --
                // see `verilog_label' below for both shapes). `extern`-
                // declared class members are routine in UVM-style
                // verification code, so this is not a rare-shape cut.
                //
                // M124 fix round: `task_prototype'/`function_prototype'
                // are NOT class-member-only, despite the class-focused
                // framing above. FOUR other productions in `grammar.js'
                // reach them, and the count matters -- the first version
                // of this comment named two and stopped, which is the
                // "when an entry says two, assume at least two" trap
                // `CLAUDE.md' records; the last two were found by the
                // trailing cold read, not by this comment's author:
                //
                //   1. `dpi_function_proto'/`dpi_task_proto' -- a DPI
                //      import, `import "DPI-C" function int c_add(int a,
                //      int b);', at module/package/interface level.
                //   2. `extern_tf_declaration' -- an interface-level
                //      `extern task'/`extern function' prototype,
                //      distinct from the class-member `extern' shape
                //      documented above.
                //   3. `interface_class_method' (`grammar.js:888') --
                //      `pure virtual function void step();' inside an
                //      `interface class', which is NOT a
                //      `class_declaration' node at all.
                //   4. `modport_tf_ports_declaration'
                //      (`grammar.js:1710') -- `modport mp (import task
                //      run(int y));'.
                //
                // This match arm matches by NODE KIND alone with no
                // class-membership check, so all four already produce a
                // correct scope with no additional code -- see
                // `verilog_dpi_import_function_prototype_at_module_level',
                // `verilog_extern_tf_declaration_task_prototype_at_
                // interface_level', `verilog_interface_class_pure_virtual_
                // method_prototype' and `verilog_modport_import_task_
                // prototype' below, one per production, for the proof.
                | "class_constructor_prototype"
                | "task_prototype"
                | "function_prototype"
        ),
        Lang::Rust => matches!(
            kind,
            "function_item" | "impl_item" | "struct_item" | "enum_item" | "trait_item" | "mod_item"
        ),
        Lang::Python => matches!(kind, "function_definition" | "class_definition"),
        Lang::C => matches!(
            kind,
            "function_definition" | "struct_specifier" | "enum_specifier" | "union_specifier"
        ),
        Lang::Cpp => matches!(
            kind,
            "function_definition" | "class_specifier" | "struct_specifier" | "namespace_definition"
        ),
        Lang::Java => matches!(
            kind,
            "method_declaration"
                | "constructor_declaration"
                | "class_declaration"
                | "interface_declaration"
        ),
        // See module doc: deliberately empty, not yet dumped.
        Lang::Bash | Lang::Elisp | Lang::Perl => false,
    }
}

/// `&src[node.start_byte()..node.end_byte()]` -- tree-sitter byte
/// offsets always land on UTF-8 char boundaries for a node it produced,
/// so this slice is always valid.
fn text<'a>(node: &tree_sitter::Node, src: &'a str) -> &'a str {
    src.get(node.start_byte()..node.end_byte()).unwrap_or("")
}

/// First direct child of `node` with kind `kind`, if any.
fn find_child_kind<'a>(node: &tree_sitter::Node<'a>, kind: &str) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        if cursor.node().kind() == kind {
            return Some(cursor.node());
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

/// First descendant of `node` (any depth) with kind `kind`, if any --
/// used to reach through a wrapper node (e.g. `module_instantiation` ->
/// `hierarchical_instance` -> `name_of_instance`) without hand-coding
/// every intermediate hop.
fn find_descendant_kind<'a>(
    node: &tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    // Explicit stack, but pushed in REVERSE child order so popping (LIFO)
    // still visits children left-to-right -- a plain forward push+pop
    // here would visit the LAST child first, which for "first identifier
    // in source order" is exactly backwards (caught by this function's
    // own test: it originally returned a parameter's identifier instead
    // of the function's own name, since a C function's parameters sit
    // after its declarator in the tree).
    let mut stack: Vec<tree_sitter::Node<'a>> = vec![*node];
    while let Some(n) = stack.pop() {
        if n.kind() == kind {
            return Some(n);
        }
        let children: Vec<tree_sitter::Node<'a>> = n.children(&mut cursor).collect();
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// First `identifier`-ish descendant of `node` (any depth) -- the
/// fallback used for C/C++ `function_definition`, whose declarator
/// nesting (plain / pointer / array return type) has no single fixed
/// field path to a name the way Rust/Python/Java's grammars do
/// (dump-verified against `tree-sitter-c`/`tree-sitter-cpp`'s
/// `node-types.json`: `function_definition.declarator` is a
/// `function_declarator`, whose own `declarator` field is itself one of
/// several possible wrapper kinds). A tree walk for the first
/// `identifier` node is robust to all of them.
fn first_identifier<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    find_descendant_kind(node, "identifier")
}

const CASE_LABEL_MAX: usize = 24;

fn truncate_label(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The declared name of a `module_declaration`/`package_declaration`/
/// `interface_declaration`/`class_declaration` node. **Runtime-dump-
/// verified, not node-types.json-only**: the static grammar schema
/// claims all four have a direct `name` field, but a real parse of
/// `demo/rtl/top/soc_top.sv` shows `module_declaration` does NOT --
/// its `module_ansi_header` child carries the field instead (same
/// split `package_declaration`'s sibling entry in `indent.el`'s header
/// comment already knows about for a different reason: the header/body
/// split behind the "one level deeper" indent quirk). A second dump of
/// a bare `interface foo_if; ... endinterface` snippet shows
/// `interface_declaration` has the exact same split, via
/// `interface_ansi_header`; `package_declaration` and `class_declaration`
/// really do carry `name` directly. Rather than hardcode which two of
/// the four need the extra hop, this checks the node itself first, then
/// falls back to searching direct children for one that itself has a
/// `name` field -- covers both shapes without needing to name the
/// `_ansi_header`/`_nonansi_header` wrapper kind explicitly.
fn declaration_name(node: &tree_sitter::Node, src: &str) -> String {
    if let Some(n) = node.child_by_field_name("name") {
        return text(&n, src).to_string();
    }
    let mut cursor = node.walk();
    let children: Vec<tree_sitter::Node> = node.children(&mut cursor).collect();
    for child in children {
        if let Some(n) = child.child_by_field_name("name") {
            return text(&n, src).to_string();
        }
    }
    node.kind().to_string()
}

fn verilog_label(node: &tree_sitter::Node, src: &str) -> String {
    match node.kind() {
        "module_declaration"
        | "package_declaration"
        | "interface_declaration"
        | "class_declaration"
        // M121: `program_declaration` has the exact same header-carries-the-
        // name split as `module_declaration`/`interface_declaration` above
        // (dump-verified: `program_ansi_header` carries `name:`, not
        // `program_declaration` itself) -- `declaration_name`'s
        // node-then-children fallback already handles that shape.
        | "program_declaration" => declaration_name(node, src),
        // M121: `covergroup_declaration` DOES carry `name:` directly on
        // itself (dump-verified), unlike the four above. `node-types.json`
        // marks that field `"required": false`, and the query file's own
        // M89 comment repeats "an unnamed covergroup is legal SV" -- but
        // dump-verifying that claim here (`covergroup ;\nendgroup\n`, both
        // at top level and nested inside a class) produced a parse ERROR
        // both times, not a valid `covergroup_declaration` with an absent
        // name. IEEE 1800's own grammar for `covergroup_declaration` does
        // NOT mark the identifier optional either, so this looks like the
        // same "node-types.json schema says optional, the real grammar
        // does not accept it" gap this file's own header (M119 finding)
        // already warns about for `module_declaration.name` -- the fallback
        // below is kept anyway (defensive, matching `scope_label`'s own
        // "kept total rather than partial" doc), but no test exercises it:
        // no valid SV snippet was found that reaches it.
        "covergroup_declaration" => node
            .child_by_field_name("name")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| "covergroup".to_string()),
        // M121: `modport_declaration` has no `name:` field of its own --
        // the name is its (first) `modport_item` child's bare, unlabeled
        // first positional child (dump-verified against both a single-name
        // and a comma-separated multi-name `modport mst(...), slv(...);`).
        // A `modport_declaration` node's whole span covers every
        // comma-separated name together (the same one-`Scope`-per-node
        // limitation `module_instantiation`'s comment below documents for
        // a multi-instance statement), so this always labels with the
        // FIRST name, never a later one.
        "modport_declaration" => find_child_kind(node, "modport_item")
            .and_then(|item| find_child_kind(&item, "simple_identifier"))
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        // M121: a class constructor has no name field at all -- SV only
        // ever calls it `new`, so the label is that fixed literal rather
        // than anything pulled from the parse. `node-types.json` lists
        // FIVE possible children, not one: `function_statement_or_null`
        // (the body) is only one of them, and it is present only when
        // the constructor HAS a body statement -- a completely empty
        // `function new(); endfunction` produces no named children at
        // all (dump-verified in the trailing cold read; the first
        // version of this comment said "present for the argument-less
        // shape", which was true only of the test fixture, which does
        // have a body statement) -- `function new(int y);` (routine in real constructors)
        // instead produces a `class_constructor_arg_list` child and NO
        // `function_statement_or_null` at all (dump-verified). The label
        // doesn't change either way (`"new"` is fixed regardless of
        // which children are present), so this is a correction to the
        // comment, not a behavior change.
        //
        // M124: `extern function new(...);` (an out-of-line prototype,
        // distinct from the in-class definition above) parses as its own
        // node kind, `class_constructor_prototype` -- same "SV only ever
        // calls it `new`" reasoning as the in-class shape above, and
        // dump-verified (M124) to likewise carry no `name:` field of its
        // own, so the label is the same fixed literal.
        "class_constructor_declaration" | "class_constructor_prototype" => "new".to_string(),
        // M121: `concurrent_assertion_item` is the SVA statement's outer
        // wrapper -- it optionally carries a leading label as a bare
        // `simple_identifier` positional child (`my_check: assert property
        // (...)`, dump-verified), then exactly one of FIVE statement
        // kinds as its other child (`node-types.json`'s own children list
        // for `concurrent_assertion_item`, cross-checked against a dump
        // of each): `assert_property_statement`, `cover_property_statement`,
        // `assume_property_statement`, `cover_sequence_statement`
        // (`cover sequence (...)`), and `restrict_property_statement`
        // (`restrict property (...)`) -- the first review round only
        // tested three of the five and mislabelled the other two as
        // `"assert"`. This file marks ONLY the wrapper as a scope kind
        // (not the statement kinds too) precisely so there is one `Scope`
        // per assertion, not two nested ones for the same span.
        "concurrent_assertion_item" => {
            let kind_word = if find_child_kind(node, "assert_property_statement").is_some() {
                "assert property".to_string()
            } else if find_child_kind(node, "cover_property_statement").is_some() {
                "cover property".to_string()
            } else if find_child_kind(node, "assume_property_statement").is_some() {
                "assume property".to_string()
            } else if find_child_kind(node, "cover_sequence_statement").is_some() {
                "cover sequence".to_string()
            } else if find_child_kind(node, "restrict_property_statement").is_some() {
                "restrict property".to_string()
            } else {
                // Not expected to be reachable given the five kinds
                // above are `node-types.json`'s complete list, but kept
                // honest rather than mislabelled: falls back to the
                // child's own kind string instead of guessing.
                node.kind().to_string()
            };
            // An SVA label may be either identifier form -- `my_check :`
            // and `\\my_check :` are both legal, and `node-types.json`
            // lists BOTH `simple_identifier` and `escaped_identifier` in
            // this wrapper's child alternatives. Looking only for the
            // plain form silently dropped an escaped label's breadcrumb
            // (found by the trailing cold read; the same "the list is
            // longer than the count you were told" shape that produced
            // this milestone's own `cover sequence` defect).
            match find_child_kind(node, "simple_identifier")
                .or_else(|| find_child_kind(node, "escaped_identifier"))
            {
                Some(n) => format!("{}: {}", text(&n, src).trim_end(), kind_word),
                None => kind_word,
            }
        }
        "always_construct" => find_child_kind(node, "always_keyword")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| "always".to_string()),
        "case_statement" => {
            let expr = find_child_kind(node, "case_expression")
                .map(|n| text(&n, src))
                .unwrap_or("");
            truncate_label(&format!("case {}", expr), CASE_LABEL_MAX)
        }
        "if_generate_construct" => "if generate".to_string(),
        "generate_block" => node
            .child_by_field_name("name")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| "generate".to_string()),
        // `find_descendant_kind` returns the FIRST `name_of_instance` in
        // source order -- for a `module_instantiation` with several
        // comma-separated instances (`sub u1(...), u2(...);`, real but
        // rare Verilog, uncommon in idiomatic RTL), the label is always
        // the FIRST instance's name (`u1` here), never a later one.
        // Known, not fixed in v1: `Scope` carries one `label` per node,
        // and this whole node's `start`/`end` span covers every comma-
        // separated instance together, so there is no single "this
        // instance's own name" to pick without splitting one grammar
        // node into several `Scope`s -- a bigger change than this
        // review round's scope.
        "module_instantiation" => find_descendant_kind(node, "name_of_instance")
            .and_then(|inst| inst.child_by_field_name("instance_name"))
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        "function_declaration" => find_child_kind(node, "function_body_declaration")
            .and_then(|n| n.child_by_field_name("name"))
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        "task_declaration" => find_child_kind(node, "task_body_declaration")
            .and_then(|n| n.child_by_field_name("name"))
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        // M124: UNLIKE `function_declaration'/`task_declaration' above,
        // these two prototype kinds carry `name:' directly on
        // THEMSELVES, not on a nested `*_body_declaration' child --
        // dump-verified (M124: `extern task run(int y);'/`pure virtual
        // function void step();', real `(treesit-node-string ...)'
        // output shows `task_prototype name: (simple_identifier) ...'/
        // `function_prototype (data_type_or_void) name: (simple_
        // identifier)' with no intervening body-declaration wrapper at
        // all, since a prototype has no body).
        "task_prototype" | "function_prototype" => node
            .child_by_field_name("name")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        "loop_generate_construct" => "for generate".to_string(),
        other => other.to_string(),
    }
}

fn rust_label(node: &tree_sitter::Node, src: &str) -> String {
    match node.kind() {
        "impl_item" => {
            // No `name` field (dump-verified: `impl_item`'s fields are
            // `body`/`trait`/`type`/`type_parameters`) -- the `type`
            // field is the struct/trait being implemented, which is
            // what a reader actually wants to see for "which impl am I
            // in".
            let ty = node
                .child_by_field_name("type")
                .map(|n| text(&n, src))
                .unwrap_or("");
            format!("impl {}", ty)
        }
        _ => node
            .child_by_field_name("name")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
    }
}

fn c_cpp_label(node: &tree_sitter::Node, src: &str) -> String {
    match node.kind() {
        "function_definition" => first_identifier(node)
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
        _ => node
            .child_by_field_name("name")
            .map(|n| text(&n, src).to_string())
            .unwrap_or_else(|| node.kind().to_string()),
    }
}

fn field_name_label(node: &tree_sitter::Node, src: &str) -> String {
    node.child_by_field_name("name")
        .map(|n| text(&n, src).to_string())
        .unwrap_or_else(|| node.kind().to_string())
}

/// Compact human label for `node`, e.g. `"always_ff"` for an
/// `always_construct`. Fallback for any kind with no special rule
/// (should not happen for a kind `is_scope_kind` accepted, but kept
/// total rather than partial): the kind string itself.
pub fn scope_label(lang: Lang, node: &tree_sitter::Node, src: &str) -> String {
    match lang {
        Lang::Verilog => verilog_label(node, src),
        Lang::Rust => rust_label(node, src),
        Lang::Python | Lang::Java => field_name_label(node, src),
        Lang::C | Lang::Cpp => c_cpp_label(node, src),
        Lang::Bash | Lang::Elisp | Lang::Perl => node.kind().to_string(),
    }
}

/// 0-based line number of byte offset `byte_pos` in `text` -- a count
/// of `\n` bytes strictly before it. Deliberately not
/// `tree_sitter::Point` (see module doc).
fn line_of_byte(text: &str, byte_pos: usize) -> usize {
    text.as_bytes()[..byte_pos.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// Walk the whole tree once, collecting every node whose kind
/// `is_scope_kind(lang, ..)` accepts. Called on the highlight worker's
/// background thread, once per completed parse -- see
/// `highlight::extract_spans`'s call site and `Engine::scope_chain`'s
/// doc for the caching/generation-guard contract this feeds.
///
/// Iterative cursor walk (not recursion), same shape as
/// `highlight::collect_rainbow_spans`, to avoid stack depth tracking
/// tree nesting on adversarial input.
pub fn collect_scopes(tree: &tree_sitter::Tree, src: &str, lang: Lang) -> Vec<Scope> {
    let mut out = Vec::new();
    let mut cursor = tree.root_node().walk();
    loop {
        let node = cursor.node();
        if node.is_named() && is_scope_kind(lang, node.kind()) {
            let start_byte = node.start_byte();
            let end_byte = node.end_byte();
            out.push(Scope {
                start_line: line_of_byte(src, start_byte),
                start: crate::treesit::byte_to_char(src, start_byte),
                end: crate::treesit::byte_to_char(src, end_byte),
                kind: node.kind().to_string(),
                label: scope_label(lang, &node, src),
            });
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(lang: tree_sitter::Language, src: &str) -> tree_sitter::Tree {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&lang).unwrap();
        p.parse(src, None).unwrap()
    }

    #[test]
    fn verilog_soc_top_module_and_instantiation() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../demo/rtl/top/soc_top.sv"
        ))
        .expect("demo/rtl/top/soc_top.sv");
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), &src);
        let scopes = collect_scopes(&tree, &src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "module_declaration" && s.label == "soc_top"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "module_instantiation" && s.label == "u_regfile"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_always_ff_and_case_label_in_alu() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../demo/rtl/core/alu.sv"
        ))
        .expect("demo/rtl/core/alu.sv");
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), &src);
        let scopes = collect_scopes(&tree, &src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "always_construct" && s.label == "always_ff"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "case_statement" && s.label.starts_with("case ")),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rust_function_and_impl() {
        let src = "impl Foo {\n    fn bar(&self) -> i32 {\n        1\n    }\n}\n";
        let tree = parse(tree_sitter_rust::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Rust);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "impl_item" && s.label == "impl Foo"));
        assert!(scopes
            .iter()
            .any(|s| s.kind == "function_item" && s.label == "bar"));
    }

    #[test]
    fn python_function_and_class() {
        let src = "class Foo:\n    def bar(self):\n        return 1\n";
        let tree = parse(tree_sitter_python::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Python);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "class_definition" && s.label == "Foo"));
        assert!(scopes
            .iter()
            .any(|s| s.kind == "function_definition" && s.label == "bar"));
    }

    #[test]
    fn c_function_definition() {
        let src = "int add(int a, int b) {\n    return a + b;\n}\n";
        let tree = parse(tree_sitter_c::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::C);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "function_definition" && s.label == "add"));
    }

    #[test]
    fn cpp_class_specifier_and_function() {
        // An inline method body inside a class isn't its own
        // `function_definition` node in this grammar (dump-verified: it
        // stays a `field_declaration` with a `function_declarator`), so
        // this exercises `class_specifier` and a free `function_definition`
        // separately rather than nesting one inside the other.
        let src = "class Foo {\npublic:\n    int x;\n};\nint bar(int a) {\n    return a;\n}\n";
        let tree = parse(tree_sitter_cpp::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Cpp);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "class_specifier" && s.label == "Foo"));
        assert!(scopes
            .iter()
            .any(|s| s.kind == "function_definition" && s.label == "bar"));
    }

    #[test]
    fn java_class_and_method() {
        let src = "class Foo {\n    int bar() {\n        return 1;\n    }\n}\n";
        let tree = parse(tree_sitter_java::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Java);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "class_declaration" && s.label == "Foo"));
        assert!(scopes
            .iter()
            .any(|s| s.kind == "method_declaration" && s.label == "bar"));
    }

    #[test]
    fn bash_and_elisp_are_empty() {
        assert!(!is_scope_kind(Lang::Bash, "function_definition"));
        assert!(!is_scope_kind(Lang::Elisp, "function_definition"));
    }

    // -----------------------------------------------------------------
    // M118 review fix (FIX-3): 8 of the 12 Verilog scope kinds had no
    // test at all. `demo/rtl/` has real material for two of them
    // (`package_declaration`/`function_declaration`, both in
    // `soc_pkg.sv`); the other six were, at the time this comment was
    // first written, minimal invented snippets because none of them
    // appeared anywhere under `demo/rtl/` (confirmed by grep, not
    // assumed).
    //
    // M122 changed that for four of the six: `interface_declaration`,
    // `generate_block`, `if_generate_construct` and
    // `loop_generate_construct` all now have real material
    // (`demo/rtl/bus/axi4_lite_if.sv`'s `interface`;
    // `demo/rtl/mem/sram_bank.sv`'s `for (genvar b …)` loop generate
    // with a nested `if` generate inside it, which is exactly the
    // `generate_block`/`if_generate_construct` nesting shape) --
    // `crates/core/tests/scope_header_tests.rs` and
    // `crates/core/tests/highlight_tests.rs` exercise those against the
    // real files. The invented-snippet tests below are kept anyway: they
    // pin the grammar's node kinds and labels directly, in isolation,
    // which the real-file tests don't attempt to do (those assert on
    // the breadcrumb/highlight *output*, not on `collect_scopes`'s raw
    // kind/label pairs) -- so the two kinds of test are not redundant.
    // `class_declaration` and `task_declaration` are still invented
    // snippets: `soc_verif_pkg.sv`'s class lives under `demo/verif/`,
    // not `demo/rtl/`, and no task exists anywhere in the demo tree.
    // Every snippet below is dump-verified the same way the tests above
    // already are (this module's own doc comment): `eprintln!`-dumped
    // against the real `tree_sitter_systemverilog` grammar before
    // writing the assertion, not guessed at from the grammar's field
    // names.
    // -----------------------------------------------------------------

    #[test]
    fn verilog_package_and_function_in_soc_pkg() {
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../demo/rtl/pkg/soc_pkg.sv"
        ))
        .expect("demo/rtl/pkg/soc_pkg.sv");
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), &src);
        let scopes = collect_scopes(&tree, &src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "package_declaration" && s.label == "soc_pkg"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "function_declaration" && s.label == "strb_to_mask"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_interface_declaration() {
        // Not present anywhere under `demo/rtl/` (grep-confirmed) --
        // minimal invented snippet, per this module's own doc on when
        // that's acceptable.
        let src = "interface my_if;\n  logic clk;\nendinterface\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "interface_declaration" && s.label == "my_if"));
    }

    #[test]
    fn verilog_class_declaration() {
        let src = "class my_cls;\n  int x;\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "class_declaration" && s.label == "my_cls"));
    }

    #[test]
    fn verilog_task_declaration() {
        let src =
            "module m;\n  task my_task(input int a);\n    $display(a);\n  endtask\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "task_declaration" && s.label == "my_task"));
    }

    #[test]
    fn verilog_if_generate_and_generate_block() {
        // Also exercises `generate_block` (the `if` arm's own `begin :
        // g ... end`), dump-verified to nest inside
        // `if_generate_construct` at this shape -- a BARE `generate
        // begin : blk ... end endgenerate` with no `if`/`for` wrapper
        // does NOT produce a `generate_block` node in this grammar
        // (dump-verified: it parses as an empty-label
        // `always_construct` instead), so `generate_block` is only
        // reachable through one of the two constructs below in
        // practice.
        let src = "module m;\n  generate\n    if (1) begin : g\n      wire w;\n    end\n  endgenerate\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "if_generate_construct" && s.label == "if generate"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "generate_block" && s.label == "g"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_loop_generate_construct() {
        let src = "module m;\n  generate\n    for (genvar i = 0; i < 4; i++) begin : g\n      wire w;\n    end\n  endgenerate\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(scopes
            .iter()
            .any(|s| s.kind == "loop_generate_construct" && s.label == "for generate"));
    }

    // -----------------------------------------------------------------
    // M121: `program`/`covergroup`/`modport`/a class constructor/the SVA
    // concurrent-assertion kinds all previously produced zero scopes.
    // At the time this comment was first written, a literal grep for
    // these DID hit one line, `demo/rtl/include/soc_defs.svh:22`'s
    // `assert property` inside the `SOC_ASSERT` macro body -- but that
    // macro was defined and never invoked anywhere else under
    // `demo/rtl/`, so no `concurrent_assertion_item` was ever actually
    // parsed from it (a `` `define `` body is inert text to the parser
    // until expanded at a call site, and there was no call site).
    //
    // M122 made all five kinds real: `modport` and labelled
    // `concurrent_assertion_item`s (unrelated to `SOC_ASSERT`, written
    // directly in the source) are in `demo/rtl/bus/axi4_lite_if.sv`;
    // `program`, `covergroup` and a class constructor (`function new`)
    // are in `demo/verif/sram_bank_tb.sv` and
    // `demo/verif/soc_verif_pkg.sv`. `crates/core/tests/scope_header_tests.rs`
    // exercises the breadcrumb chain for each against those real files.
    // The invented-snippet tests below are kept for the same reason as
    // the M118 block above: they pin `collect_scopes`'s raw kind/label
    // output in isolation, which the breadcrumb tests don't attempt to
    // do. Every snippet below is still dump-verified the same way as
    // the M118 block above (a throwaway probe, deleted before this
    // milestone finished).
    // -----------------------------------------------------------------

    #[test]
    fn verilog_program_declaration() {
        let src = "program my_prog;\n  initial $display(\"hi\");\nendprogram\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "program_declaration" && s.label == "my_prog"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_covergroup_declaration_named() {
        let src =
            "module m;\n  covergroup my_cg;\n    cp_data: coverpoint x;\n  endgroup\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "covergroup_declaration" && s.label == "my_cg"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_modport_declaration() {
        let src = "interface my_if;\n  logic clk;\n  modport mst(input clk);\nendinterface\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "modport_declaration" && s.label == "mst"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_class_constructor_declaration() {
        // Also exercises `class_declaration` alongside it: the constructor
        // (`function new()`) is a DIFFERENT node kind
        // (`class_constructor_declaration`) from an ordinary method
        // (`function_declaration`, already a scope kind before M121) --
        // dump-verified the two do not share a kind.
        let src =
            "class my_cls;\n  int x;\n  function new();\n    x = 0;\n  endfunction\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "class_constructor_declaration" && s.label == "new"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    /// M124: `extern function new(...);` -- an out-of-line prototype,
    /// distinct from `verilog_class_constructor_declaration' above's
    /// in-class shape -- previously produced NO breadcrumb at all
    /// (`class_constructor_prototype' was not a scope kind), so
    /// `collect_scopes' returned only the enclosing `class_declaration'.
    #[test]
    fn verilog_class_constructor_prototype_extern() {
        let src = "class my_cls;\n  extern function new(int y);\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let class = scopes
            .iter()
            .find(|s| s.kind == "class_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no class_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let ctor = scopes
            .iter()
            .find(|s| s.kind == "class_constructor_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no class_constructor_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(ctor.label, "new");
        assert!(
            class.start <= ctor.start && ctor.end <= class.end,
            "constructor prototype must be nested inside the class: class {:?}, ctor {:?}",
            (class.start, class.end),
            (ctor.start, ctor.end)
        );
    }

    /// M124: `extern task run(...);` -- same undisclosed gap, a
    /// different prototype kind (`task_prototype', which carries its own
    /// `name:' field directly, unlike `task_declaration').
    #[test]
    fn verilog_task_prototype_extern() {
        let src = "class my_cls;\n  extern task run(int y);\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let class = scopes
            .iter()
            .find(|s| s.kind == "class_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no class_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let task = scopes
            .iter()
            .find(|s| s.kind == "task_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no task_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(task.label, "run");
        assert!(
            class.start <= task.start && task.end <= class.end,
            "task prototype must be nested inside the class: class {:?}, task {:?}",
            (class.start, class.end),
            (task.start, task.end)
        );
    }

    /// M124: `pure virtual function void step();` -- a third undisclosed
    /// instance of the same gap (`function_prototype', which also
    /// carries its own `name:' field directly, unlike
    /// `function_declaration'). `function_prototype' never appeared in
    /// `scope.rs' before this milestone (zero grep hits) even though the
    /// M121 record only ever disclosed the constructor variant.
    #[test]
    fn verilog_pure_virtual_function_prototype() {
        let src = "class my_cls;\n  pure virtual function void step();\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let class = scopes
            .iter()
            .find(|s| s.kind == "class_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no class_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let func = scopes
            .iter()
            .find(|s| s.kind == "function_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no function_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(func.label, "step");
        assert!(
            class.start <= func.start && func.end <= class.end,
            "function prototype must be nested inside the class: class {:?}, func {:?}",
            (class.start, class.end),
            (func.start, func.end)
        );
    }

    #[test]
    fn verilog_sva_assert_property_unlabeled() {
        let src = "module m;\n  assert property (@(posedge clk) a |-> b);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item" && s.label == "assert property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_sva_assert_property_labeled() {
        let src = "module m;\n  my_check: assert property (@(posedge clk) a |-> b) else $error(\"x\");\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item"
                    && s.label == "my_check: assert property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_sva_cover_property() {
        let src = "module m;\n  cover property (@(posedge clk) a);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item" && s.label == "cover property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_sva_assume_property() {
        let src = "module m;\n  assume property (@(posedge clk) a);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item" && s.label == "assume property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_sva_inside_ifndef_guard_in_real_axi4_lite_if() {
        // M122 checkpoint: the design puts concurrent assertions behind
        // `` `ifndef SOC_SVA_OFF `` so a real testbench can build with
        // Icarus (which does not support them) by defining that macro.
        // This confirms tree-sitter still sees the guarded assertions as
        // real `concurrent_assertion_item` scopes rather than swallowing
        // them as part of the conditional-compilation directive.
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../demo/rtl/bus/axi4_lite_if.sv"
        ))
        .expect("demo/rtl/bus/axi4_lite_if.sv");
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), &src);
        let scopes = collect_scopes(&tree, &src, Lang::Verilog);
        assert!(
            scopes.iter().any(|s| s.kind == "concurrent_assertion_item"
                && s.label == "a_aw_stable: assert property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
        assert!(
            scopes.iter().any(|s| s.kind == "concurrent_assertion_item"
                && s.label == "a_ar_stable: assert property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    /// M121 trailing round: an SVA label may be an ESCAPED identifier
    /// (`\\my_check : assert property (...)`), which is legal SV and
    /// which `node-types.json` lists alongside `simple_identifier` as a
    /// child alternative of `concurrent_assertion_item`. The first
    /// version of this milestone looked only for the plain form, so an
    /// escaped label silently lost its breadcrumb.
    #[test]
    fn verilog_sva_escaped_identifier_label_is_kept() {
        let src = "module m;\n  \\my$check : assert property (@(posedge clk) a);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let sva: Vec<&Scope> = scopes
            .iter()
            .filter(|s| s.kind == "concurrent_assertion_item")
            .collect();
        assert_eq!(sva.len(), 1, "one assertion scope: {:?}", scopes);
        assert_eq!(
            sva[0].label, "\\my$check: assert property",
            "an escaped-identifier label must reach the breadcrumb, not be dropped"
        );
    }

    #[test]
    fn verilog_sva_cover_sequence() {
        let src = "module m;\n  cover sequence (@(posedge clk) a);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item" && s.label == "cover sequence"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn verilog_sva_restrict_property() {
        let src = "module m;\n  restrict property (@(posedge clk) a);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        assert!(
            scopes
                .iter()
                .any(|s| s.kind == "concurrent_assertion_item" && s.label == "restrict property"),
            "{:?}",
            scopes
                .iter()
                .map(|s| (&s.kind, &s.label))
                .collect::<Vec<_>>()
        );
    }

    /// F3 (M124 fix round): `function_prototype`/`task_prototype` are NOT
    /// class-members-only, contrary to what this file's earlier comments
    /// implied -- `dpi_import_export` (a DPI import, routine in
    /// verification code) wraps a `dpi_function_proto`/`dpi_task_proto`
    /// node whose sole child is, again, a bare `function_prototype`/
    /// `task_prototype`. `collect_scopes` walks by kind with no
    /// class-membership check, so this shape already produces a correct
    /// scope with no code change -- dump-verified (real
    /// `(treesit-node-string ...)`-equivalent output via
    /// `tree.root_node().to_sexp()`):
    /// `(dpi_import_export (dpi_spec_string) (dpi_function_proto
    /// (function_prototype ... name: (simple_identifier) ...)))`.
    #[test]
    fn verilog_dpi_import_function_prototype_at_module_level() {
        let src = "module m;\n  import \"DPI-C\" function int c_add(int a, int b);\nendmodule\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let module = scopes
            .iter()
            .find(|s| s.kind == "module_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no module_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let func = scopes
            .iter()
            .find(|s| s.kind == "function_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no function_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(func.label, "c_add");
        assert!(
            module.start <= func.start && func.end <= module.end,
            "DPI function prototype must be nested inside the module: module {:?}, func {:?}",
            (module.start, module.end),
            (func.start, func.end)
        );
    }

    /// F3 (M124 fix round): the second undisclosed non-class-member
    /// shape -- `extern_tf_declaration`, an interface-level `extern
    /// task`/`extern function` prototype (distinct from
    /// `class_constructor_prototype`/`task_prototype`/`function_
    /// prototype`'s already-tested class-member use, see
    /// `verilog_task_prototype_extern` above). Dump-verified: `(interface_
    /// declaration ... (extern_tf_declaration (task_prototype name:
    /// (simple_identifier) ...)))` -- the same bare `task_prototype`
    /// node kind, so this also already works with no code change.
    #[test]
    fn verilog_extern_tf_declaration_task_prototype_at_interface_level() {
        let src = "interface my_if;\n  extern task run(int y);\nendinterface\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let iface = scopes
            .iter()
            .find(|s| s.kind == "interface_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no interface_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let task = scopes
            .iter()
            .find(|s| s.kind == "task_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no task_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(task.label, "run");
        assert!(
            iface.start <= task.start && task.end <= iface.end,
            "extern task prototype must be nested inside the interface: iface {:?}, task {:?}",
            (iface.start, iface.end),
            (task.start, task.end)
        );
    }

    /// M124 trailing cold read: the THIRD non-class-member producer of a
    /// bare `function_prototype' -- `interface_class_method'
    /// (`grammar.js:888', `seq('pure', 'virtual', $._method_prototype,
    /// ';')'), the body item of an `interface class'. Note the enclosing
    /// scope: an `interface class' parses as `interface_class_
    /// declaration', which `is_scope_kind' does NOT accept, so the
    /// prototype's breadcrumb has no class above it -- this test pins
    /// that observed shape rather than an assumed one.
    #[test]
    fn verilog_interface_class_pure_virtual_method_prototype() {
        let src = "interface class ifc;\n  pure virtual function void step();\nendclass\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let func = scopes
            .iter()
            .find(|s| s.kind == "function_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no function_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(func.label, "step");
        // Observed, not assumed (probe output for this exact fixture:
        // `[("function_prototype", "step")]'): the prototype is the ONLY
        // scope here. An `interface class' does not parse as
        // `class_declaration', and its own kind is not a scope kind, so
        // this breadcrumb has nothing above it. Pinned so that a later
        // milestone adding `interface_class_declaration' has to come
        // back and change this line deliberately.
        assert_eq!(scopes.len(), 1, "scopes = {scopes:?}");
    }

    /// M124 trailing cold read: the FOURTH producer --
    /// `modport_tf_ports_declaration' (`grammar.js:1710'), a modport's
    /// own `import'/`export' task or function prototype. Nested inside
    /// the interface, like `verilog_extern_tf_declaration_task_prototype_
    /// at_interface_level' but reached through `modport_declaration'
    /// (itself a scope kind since M121).
    #[test]
    fn verilog_modport_import_task_prototype() {
        let src = "interface my_if;\n  modport mp (import task run(int y));\nendinterface\n";
        let tree = parse(tree_sitter_systemverilog::LANGUAGE.into(), src);
        let scopes = collect_scopes(&tree, src, Lang::Verilog);
        let iface = scopes
            .iter()
            .find(|s| s.kind == "interface_declaration")
            .unwrap_or_else(|| {
                panic!(
                    "no interface_declaration scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        let task = scopes
            .iter()
            .find(|s| s.kind == "task_prototype")
            .unwrap_or_else(|| {
                panic!(
                    "no task_prototype scope: {:?}",
                    scopes
                        .iter()
                        .map(|s| (&s.kind, &s.label))
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(task.label, "run");
        assert!(
            iface.start <= task.start && task.end <= iface.end,
            "modport import task prototype must be nested inside the interface: iface {:?}, task {:?}",
            (iface.start, iface.end),
            (task.start, task.end)
        );
    }
}
